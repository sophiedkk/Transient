//! Equalizer: more or less bass, middle and treble.
//!
//! Three bands at fixed places, chosen for voices and music: a shelf below
//! 120 Hz (body, boom), a broad bump around 1 kHz (presence, nasal or
//! boxy sound) and a shelf above 8 kHz (air, hiss, sibilance). Each turns its
//! part up or down by up to 12 dB; at 0 dB a band changes nothing.

use serde::{Deserialize, Serialize};

use super::biquad::Biquad;
use crate::time::SampleRate;

/// The most a band can turn down, in dB.
pub const MIN_GAIN_DB: f32 = -12.0;

/// The most a band can turn up, in dB.
pub const MAX_GAIN_DB: f32 = 12.0;

/// Where the bass shelf bends, in hertz.
const BASS_HZ: f32 = 120.0;
/// The centre of the middle band, in hertz.
const MIDDLE_HZ: f32 = 1000.0;
/// How wide the middle band is: 0.7 is about two octaves.
const MIDDLE_Q: f64 = 0.7;
/// Where the treble shelf bends, in hertz.
const TREBLE_HZ: f32 = 8000.0;

/// The equalizer's settings, as saved in the project file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Equalizer {
    /// Whether the equalizer is switched on.
    pub enabled: bool,
    /// How much the bass is turned up (or down, below 0), in dB.
    pub bass_db: f32,
    /// How much the middle is turned up or down, in dB.
    pub middle_db: f32,
    /// How much the treble is turned up or down, in dB.
    pub treble_db: f32,
}

/// The running equalizer: one filter per band, one after another.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EqualizerFilter {
    bands: [Biquad; 3],
}

impl EqualizerFilter {
    /// An equalizer for `settings`, starting from silence.
    pub(crate) fn new(settings: Equalizer, sample_rate: SampleRate) -> Self {
        let gain = |db: f32| db.clamp(MIN_GAIN_DB, MAX_GAIN_DB);
        Self {
            bands: [
                Biquad::low_shelf(BASS_HZ, gain(settings.bass_db), sample_rate),
                Biquad::peak(MIDDLE_HZ, gain(settings.middle_db), MIDDLE_Q, sample_rate),
                Biquad::high_shelf(TREBLE_HZ, gain(settings.treble_db), sample_rate),
            ],
        }
    }

    /// Forgets earlier audio.
    pub(crate) fn reset(&mut self) {
        for band in &mut self.bands {
            band.reset();
        }
    }

    /// Takes over what `old` remembers, keeping these settings.
    pub(crate) fn continue_from(&mut self, old: &Self) {
        for (band, old) in self.bands.iter_mut().zip(&old.bands) {
            band.continue_from(old);
        }
    }

    /// Filters interleaved stereo `samples` in place.
    pub(crate) fn process(&mut self, samples: &mut [f32]) {
        for band in &mut self.bands {
            band.process(samples);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{settled_rms, sine};

    /// The change in level (dB, by RMS) of a sine of `frequency` Hz through
    /// `settings`.
    fn change_db(settings: Equalizer, frequency: f64) -> f32 {
        let original = sine(frequency);
        let mut samples = original.clone();
        EqualizerFilter::new(settings, SampleRate::HZ_48000).process(&mut samples);
        crate::audio::db_from_amplitude(settled_rms(&samples) / settled_rms(&original))
    }

    fn settings(bass_db: f32, middle_db: f32, treble_db: f32) -> Equalizer {
        Equalizer {
            enabled: true,
            bass_db,
            middle_db,
            treble_db,
        }
    }

    #[test]
    fn each_band_changes_its_own_part() {
        assert!((change_db(settings(6.0, 0.0, 0.0), 40.0) - 6.0).abs() < 0.3);
        assert!((change_db(settings(0.0, -6.0, 0.0), 1000.0) - -6.0).abs() < 0.1);
        // A shelf's frequency is its midpoint: half the gain there, all of
        // it further out.
        assert!((change_db(settings(0.0, 0.0, 6.0), 8000.0) - 3.0).abs() < 0.1);
        assert!(change_db(settings(0.0, 0.0, 6.0), 21_000.0) > 5.5);
        // A voice's middle is barely touched by bass and treble.
        assert!(change_db(settings(12.0, 0.0, -12.0), 1000.0).abs() < 0.5);
    }

    #[test]
    fn all_bands_at_0_db_change_nothing() {
        for frequency in [40.0, 1000.0, 16_000.0] {
            assert!(change_db(settings(0.0, 0.0, 0.0), frequency).abs() < 0.01);
        }
    }

    #[test]
    fn gains_are_held_within_12_db() {
        let wild = change_db(settings(40.0, 0.0, 0.0), 30.0);
        let most = change_db(settings(MAX_GAIN_DB, 0.0, 0.0), 30.0);
        assert!((wild - most).abs() < 1e-3);
    }
}
