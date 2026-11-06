//! A biquad: the standard building block for audio filters (low cut, EQ).
//!
//! Each output sample is a weighted sum of the last three input samples and
//! the last two output samples. The five weights (*coefficients*) decide what
//! the filter does; they come from Robert Bristow-Johnson's "Audio EQ
//! Cookbook". The filter runs in "transposed direct form II", which keeps just
//! two numbers of memory per channel (`state`) and stays accurate at low
//! frequencies when computed in `f64`.

use crate::audio::CHANNELS;
use crate::convert::f32_from_f64;
use crate::time::SampleRate;

/// A two-pole filter for stereo audio.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Biquad {
    /// Weights of the input samples: now, one ago, two ago.
    b: [f64; 3],
    /// Weights of the output samples: one ago, two ago (already divided by
    /// the cookbook's `a0`, like `b`).
    a: [f64; 2],
    /// What the filter remembers, per channel.
    state: [[f64; 2]; CHANNELS],
}

/// The lowest frequency a filter can be set to, in hertz: the formulas need
/// it above 0.
const LOWEST_HZ: f64 = 1.0;

/// The highest frequency a filter can be set to, as a fraction of the sample
/// rate: just under half of it (the Nyquist frequency), where the formulas
/// break down.
const HIGHEST_FRACTION: f64 = 0.45;

/// The cookbook's `A` for shelves and peaks: the square root of the gain as a
/// multiplier (10^(dB/40)), because each of the filter's two halves applies
/// half of it.
fn shelf_a(gain_db: f32) -> f64 {
    f64::from(crate::audio::amplitude_from_db(gain_db)).sqrt()
}

impl Biquad {
    /// A filter from the cookbook's six coefficients, dividing them by `a0`
    /// as the processing loop expects.
    fn from_cookbook(b: [f64; 3], a: [f64; 3]) -> Self {
        let [a0, a1, a2] = a;
        Self {
            b: b.map(|value| value / a0),
            a: [a1 / a0, a2 / a0],
            state: [[0.0; 2]; CHANNELS],
        }
    }

    /// The sine and cosine of `frequency_hz` as an angle per sample (the
    /// cookbook's ω), with the frequency kept where the formulas work: above
    /// 0 and below half the sample rate.
    fn angle(frequency_hz: f32, sample_rate: SampleRate) -> (f64, f64) {
        let rate = f64::from(sample_rate.hz());
        let frequency = f64::from(frequency_hz).clamp(LOWEST_HZ, rate * HIGHEST_FRACTION);
        (std::f64::consts::TAU * frequency / rate).sin_cos()
    }

    /// A high-pass filter: frequencies below `frequency_hz` are turned down,
    /// 12 dB per octave. `q` (sharpness) shapes the bend at the frequency:
    /// with 1/√2 (a 2nd-order Butterworth filter) the response is as flat as
    /// possible above it and 3 dB down at it. Steeper filters chain biquads
    /// with other Qs (see [`super::low_cut`]).
    pub(crate) fn high_pass(frequency_hz: f32, q: f64, sample_rate: SampleRate) -> Self {
        let (sin, cos) = Self::angle(frequency_hz, sample_rate);
        let alpha = sin / (2.0 * q);
        let b0 = f64::midpoint(1.0, cos);
        Self::from_cookbook([b0, -2.0 * b0, b0], [1.0 + alpha, -2.0 * cos, 1.0 - alpha])
    }

    /// A low shelf: everything below about `frequency_hz` turned up or down
    /// by `gain_db`, everything well above it unchanged (a "bass" control).
    pub(crate) fn low_shelf(frequency_hz: f32, gain_db: f32, sample_rate: SampleRate) -> Self {
        let (sin, cos) = Self::angle(frequency_hz, sample_rate);
        let a = shelf_a(gain_db);
        // The cookbook's shelf with slope 1: as steep as it goes without a
        // bump next to the shelf.
        let two_sqrt_a_alpha = 2.0 * a.sqrt() * sin / std::f64::consts::SQRT_2;
        Self::from_cookbook(
            [
                a * ((a + 1.0) - (a - 1.0) * cos + two_sqrt_a_alpha),
                2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
                a * ((a + 1.0) - (a - 1.0) * cos - two_sqrt_a_alpha),
            ],
            [
                (a + 1.0) + (a - 1.0) * cos + two_sqrt_a_alpha,
                -2.0 * ((a - 1.0) + (a + 1.0) * cos),
                (a + 1.0) + (a - 1.0) * cos - two_sqrt_a_alpha,
            ],
        )
    }

    /// A high shelf: everything above about `frequency_hz` turned up or down
    /// by `gain_db` (a "treble" control).
    pub(crate) fn high_shelf(frequency_hz: f32, gain_db: f32, sample_rate: SampleRate) -> Self {
        let (sin, cos) = Self::angle(frequency_hz, sample_rate);
        let a = shelf_a(gain_db);
        let two_sqrt_a_alpha = 2.0 * a.sqrt() * sin / std::f64::consts::SQRT_2;
        Self::from_cookbook(
            [
                a * ((a + 1.0) + (a - 1.0) * cos + two_sqrt_a_alpha),
                -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
                a * ((a + 1.0) + (a - 1.0) * cos - two_sqrt_a_alpha),
            ],
            [
                (a + 1.0) - (a - 1.0) * cos + two_sqrt_a_alpha,
                2.0 * ((a - 1.0) - (a + 1.0) * cos),
                (a + 1.0) - (a - 1.0) * cos - two_sqrt_a_alpha,
            ],
        )
    }

    /// A peak: the frequencies around `frequency_hz` turned up or down by
    /// `gain_db`; `q` sets how wide (0.7 is about two octaves).
    pub(crate) fn peak(frequency_hz: f32, gain_db: f32, q: f64, sample_rate: SampleRate) -> Self {
        let (sin, cos) = Self::angle(frequency_hz, sample_rate);
        let a = shelf_a(gain_db);
        let alpha = sin / (2.0 * q);
        Self::from_cookbook(
            [1.0 + alpha * a, -2.0 * cos, 1.0 - alpha * a],
            [1.0 + alpha / a, -2.0 * cos, 1.0 - alpha / a],
        )
    }

    /// Forgets earlier audio.
    pub(crate) fn reset(&mut self) {
        self.state = [[0.0; 2]; CHANNELS];
    }

    /// Takes over what `old` remembers, keeping this filter's coefficients.
    pub(crate) fn continue_from(&mut self, old: &Self) {
        self.state = old.state;
    }

    /// Filters interleaved stereo `samples` in place.
    pub(crate) fn process(&mut self, samples: &mut [f32]) {
        let [b0, b1, b2] = self.b;
        let [a1, a2] = self.a;
        for frame in samples.chunks_exact_mut(CHANNELS) {
            for (sample, [z1, z2]) in frame.iter_mut().zip(&mut self.state) {
                let input = f64::from(*sample);
                let output = b0 * input + *z1;
                *z1 = b1 * input - a1 * output + *z2;
                *z2 = b2 * input - a2 * output;
                *sample = f32_from_f64(output);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{ONE_SECOND, settled_peak, settled_rms, sine};

    /// The Q of a 2nd-order Butterworth filter.
    const BUTTERWORTH_Q: f64 = std::f64::consts::FRAC_1_SQRT_2;

    const RATE: SampleRate = SampleRate::HZ_48000;

    fn high_passed(frequency: f64, cut_off: f32) -> f32 {
        let mut samples = sine(frequency);
        Biquad::high_pass(cut_off, BUTTERWORTH_Q, RATE).process(&mut samples);
        settled_peak(&samples)
    }

    #[test]
    fn a_high_pass_removes_a_constant_offset() {
        let mut samples = vec![0.5; ONE_SECOND * CHANNELS];
        Biquad::high_pass(80.0, BUTTERWORTH_Q, RATE).process(&mut samples);
        assert!(settled_peak(&samples) < 1e-6);
    }

    #[test]
    fn a_high_pass_keeps_high_frequencies() {
        let peak = high_passed(1000.0, 80.0);
        assert!((peak - 1.0).abs() < 0.01, "{peak}");
    }

    #[test]
    fn a_high_pass_is_3_db_down_at_its_frequency() {
        let peak = high_passed(80.0, 80.0);
        assert!(
            (peak - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.01,
            "{peak}"
        );
    }

    #[test]
    fn a_high_pass_turns_down_an_octave_below_by_12_db() {
        // Well below the cut-off, each halving of the frequency is 12 dB
        // (a quarter); at 20 Hz, two octaves below 80 Hz, about 1/16.
        let peak = high_passed(20.0, 80.0);
        assert!((peak - 1.0 / 16.0).abs() < 0.01, "{peak}");
    }

    /// The change in level (dB, by RMS) of a sine of `frequency` Hz.
    fn change_db(mut filter: Biquad, frequency: f64) -> f32 {
        let original = sine(frequency);
        let mut samples = original.clone();
        filter.process(&mut samples);
        crate::audio::db_from_amplitude(settled_rms(&samples) / settled_rms(&original))
    }

    #[test]
    fn a_low_shelf_changes_only_the_low_end() {
        let shelf = || Biquad::low_shelf(120.0, 6.0, RATE);
        assert!((change_db(shelf(), 30.0) - 6.0).abs() < 0.3);
        assert!(change_db(shelf(), 3000.0).abs() < 0.1);
    }

    #[test]
    fn a_high_shelf_changes_only_the_high_end() {
        let shelf = || Biquad::high_shelf(8000.0, -6.0, RATE);
        assert!((change_db(shelf(), 18_000.0) - -6.0).abs() < 0.3);
        assert!(change_db(shelf(), 300.0).abs() < 0.1);
    }

    #[test]
    fn a_peak_changes_the_middle() {
        let peak = || Biquad::peak(1000.0, 6.0, 0.7, RATE);
        assert!((change_db(peak(), 1000.0) - 6.0).abs() < 0.05);
        assert!(change_db(peak(), 40.0).abs() < 0.3);
        assert!(change_db(peak(), 18_000.0).abs() < 0.3);
    }

    #[test]
    fn at_0_db_the_tone_filters_change_nothing() {
        for filter in [
            Biquad::low_shelf(120.0, 0.0, RATE),
            Biquad::peak(1000.0, 0.0, 0.7, RATE),
            Biquad::high_shelf(8000.0, 0.0, RATE),
        ] {
            let mut samples = sine(440.0);
            let original = samples.clone();
            let mut filter = filter;
            filter.process(&mut samples);
            let difference = samples
                .iter()
                .zip(&original)
                .fold(0.0_f32, |max, (a, b)| max.max((a - b).abs()));
            assert!(difference < 1e-6, "{difference}");
        }
    }

    #[test]
    fn reset_forgets_earlier_audio() {
        let mut filter = Biquad::high_pass(80.0, BUTTERWORTH_Q, RATE);
        filter.process(&mut [1.0; 64]);
        filter.reset();
        assert_eq!(filter, Biquad::high_pass(80.0, BUTTERWORTH_Q, RATE));
    }

    #[test]
    fn filtering_in_pieces_is_the_same_as_in_one_go() {
        let whole = {
            let mut samples = sine(440.0);
            Biquad::high_pass(80.0, BUTTERWORTH_Q, RATE).process(&mut samples);
            samples
        };
        let mut pieces = sine(440.0);
        let mut filter = Biquad::high_pass(80.0, BUTTERWORTH_Q, RATE);
        for piece in pieces.chunks_mut(1000) {
            filter.process(piece);
        }
        assert_eq!(whole, pieces);
    }
}
