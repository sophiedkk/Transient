//! Low cut: removes everything below a frequency.
//!
//! Microphones pick up rumble (traffic, footsteps, wind, handling) far below
//! the lowest note of a voice. A low cut (also called a high-pass filter) takes
//! that out and leaves the voice as it was. Set higher, it thins out a sound
//! on purpose (a "telephone" voice, a lighter kick drum).

use serde::{Deserialize, Serialize};

use super::biquad::Biquad;
use crate::convert::f32_from_f64;
use crate::time::SampleRate;

/// The lowest frequency the low cut can be set to, in hertz.
pub const MIN_FREQUENCY_HZ: f32 = 20.0;

/// The highest frequency the low cut can be set to, in hertz: clearly audible
/// even on laptop speakers, which hardly play anything below about 150 Hz.
pub const MAX_FREQUENCY_HZ: f32 = 500.0;

/// How steeply the low cut turns down what is below its frequency.
///
/// Steeper takes out rumble while touching less above the frequency; gentler
/// sounds more natural when the low cut is set high.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Slope {
    /// 12 dB per octave: at half the frequency, a quarter as loud.
    Db12,
    /// 24 dB per octave: at half the frequency, a sixteenth as loud.
    #[default]
    Db24,
    /// 48 dB per octave: at half the frequency, almost gone (1/256).
    Db48,
}

impl Slope {
    /// The Qs of the biquads that, one after another, make a Butterworth
    /// filter of this steepness (2nd, 4th and 8th order): for an order n,
    /// 1 / (2 cos((2k − 1)π / 2n)) for each k up to n/2.
    fn stage_qs(self) -> &'static [f64] {
        match self {
            Self::Db12 => &[std::f64::consts::FRAC_1_SQRT_2],
            Self::Db24 => &[0.541_196_100_146_197, 1.306_562_964_876_376_5],
            Self::Db48 => &[
                0.509_795_579_104_159_2,
                0.601_344_886_935_045_3,
                0.899_976_223_136_415_7,
                2.562_915_447_741_505_5,
            ],
        }
    }
}

/// Where a new low cut starts, in hertz: below a low male voice, above most
/// rumble.
pub const DEFAULT_FREQUENCY_HZ: f32 = 80.0;

/// The low cut's settings, as saved in the project file.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LowCut {
    /// Whether the low cut is switched on.
    pub enabled: bool,
    /// Sounds below this frequency (in hertz) are turned down; at the
    /// frequency itself by 3 dB.
    pub frequency_hz: f32,
    /// How steeply they are turned down below it.
    pub slope: Slope,
}

impl Default for LowCut {
    /// Off, at [`DEFAULT_FREQUENCY_HZ`].
    fn default() -> Self {
        Self {
            enabled: false,
            frequency_hz: DEFAULT_FREQUENCY_HZ,
            slope: Slope::default(),
        }
    }
}

/// The running low cut: a Butterworth high-pass filter (as flat as possible
/// above the frequency, 3 dB down at it), made of one, two or four biquads in
/// a row depending on the [`Slope`].
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LowCutFilter {
    stages: Vec<Biquad>,
}

impl LowCutFilter {
    /// A filter for `settings`' frequency, starting from silence.
    pub(crate) fn new(settings: LowCut, sample_rate: SampleRate) -> Self {
        Self {
            stages: settings
                .slope
                .stage_qs()
                .iter()
                .map(|q| Biquad::high_pass(settings.frequency_hz, *q, sample_rate))
                .collect(),
        }
    }

    /// Forgets earlier audio.
    pub(crate) fn reset(&mut self) {
        for stage in &mut self.stages {
            stage.reset();
        }
    }

    /// Takes over what `old` remembers, keeping this filter's frequency. When
    /// the slope changed, the filters differ too much to share their memory:
    /// this one starts from silence (the mix's crossfade hides that).
    pub(crate) fn continue_from(&mut self, old: &Self) {
        if self.stages.len() != old.stages.len() {
            return;
        }
        for (stage, old) in self.stages.iter_mut().zip(&old.stages) {
            stage.continue_from(old);
        }
    }

    /// Filters interleaved stereo `samples` in place.
    pub(crate) fn process(&mut self, samples: &mut [f32]) {
        for stage in &mut self.stages {
            stage.process(samples);
        }
    }
}

/// The frequency for a position of the plain-language slider: `amount` 0 is the
/// gentlest low cut ([`MIN_FREQUENCY_HZ`]), 1 the strongest ([`MAX_FREQUENCY_HZ`]).
///
/// The scale is logarithmic, as we hear pitch: each step of the slider moves
/// the frequency by the same musical interval, not by the same number of hertz.
#[must_use]
pub fn frequency_for_amount(amount: f64) -> f32 {
    let ratio = f64::from(MAX_FREQUENCY_HZ) / f64::from(MIN_FREQUENCY_HZ);
    f32_from_f64(f64::from(MIN_FREQUENCY_HZ) * ratio.powf(amount.clamp(0.0, 1.0)))
}

/// Where `frequency_hz` is on the plain-language slider (0 to 1); the inverse of
/// [`frequency_for_amount`].
#[must_use]
pub fn amount_for_frequency(frequency_hz: f32) -> f64 {
    let ratio = f64::from(MAX_FREQUENCY_HZ) / f64::from(MIN_FREQUENCY_HZ);
    let frequency = f64::from(frequency_hz.clamp(MIN_FREQUENCY_HZ, MAX_FREQUENCY_HZ));
    (frequency / f64::from(MIN_FREQUENCY_HZ)).ln() / ratio.ln()
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::test_support::{settled_peak, sine};

    /// The level (peak) of a full-scale sine of `frequency` Hz after a low cut
    /// at `cut_off` Hz.
    fn low_cut(frequency: f64, cut_off: f32, slope: Slope) -> f32 {
        let mut samples = sine(frequency);
        let settings = LowCut {
            enabled: true,
            frequency_hz: cut_off,
            slope,
        };
        LowCutFilter::new(settings, SampleRate::HZ_48000).process(&mut samples);
        settled_peak(&samples)
    }

    const SLOPES: [Slope; 3] = [Slope::Db12, Slope::Db24, Slope::Db48];

    #[test]
    fn every_slope_is_3_db_down_at_its_frequency() {
        for slope in SLOPES {
            let peak = low_cut(200.0, 200.0, slope);
            let expected = std::f32::consts::FRAC_1_SQRT_2;
            assert!((peak - expected).abs() < 0.01, "{slope:?}: {peak}");
        }
    }

    #[test]
    fn an_octave_below_is_turned_down_by_the_slope() {
        // A Butterworth filter of order n gives 1 / sqrt(1 + 2^(2n)) at half
        // its frequency: −12, −24 and −48 dB (and a little more).
        for (slope, order) in SLOPES.into_iter().zip([2, 4, 8]) {
            let peak = low_cut(100.0, 200.0, slope);
            let expected = 1.0 / (1.0 + 2_f32.powi(2 * order)).sqrt();
            assert!((peak - expected).abs() < 0.005, "{slope:?}: {peak}");
        }
    }

    #[test]
    fn every_slope_keeps_what_is_well_above_it() {
        for slope in SLOPES {
            let peak = low_cut(2000.0, 200.0, slope);
            assert!((peak - 1.0).abs() < 0.01, "{slope:?}: {peak}");
        }
    }

    #[test]
    fn the_slider_ends_are_the_frequency_limits() {
        assert!((frequency_for_amount(0.0) - MIN_FREQUENCY_HZ).abs() < 1e-3);
        assert!((frequency_for_amount(1.0) - MAX_FREQUENCY_HZ).abs() < 1e-3);
        // Out of range is held at the ends.
        assert!((frequency_for_amount(2.0) - MAX_FREQUENCY_HZ).abs() < 1e-3);
        assert!(amount_for_frequency(5.0).abs() < 1e-9);
    }

    proptest! {
        #[test]
        fn amount_and_frequency_convert_back_and_forth(amount in 0.0..=1.0_f64) {
            let back = amount_for_frequency(frequency_for_amount(amount));
            prop_assert!((back - amount).abs() < 1e-5);
        }

        #[test]
        fn a_larger_amount_cuts_more(a in 0.0..=1.0_f64, b in 0.0..=1.0_f64) {
            prop_assume!(a < b);
            prop_assert!(frequency_for_amount(a) <= frequency_for_amount(b));
        }
    }
}
