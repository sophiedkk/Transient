//! Compressor: evens out the volume of a track.
//!
//! Above a threshold, the compressor turns the sound down: with a ratio of
//! 4:1, every 4 dB louder in comes out only 1 dB louder. Loud words and quiet
//! words end up closer together. Then the whole track is turned up again
//! (*make-up gain*, here automatic), so it is not quieter overall: the quiet
//! parts end up louder.
//!
//! The bend at the threshold is gradual over 6 dB (a *soft knee*), which
//! sounds more natural than switching on abruptly. It measures the level with
//! a detector that jumps to each peak and falls over 150 ms (*release*), so it
//! follows words, not single sound waves, and turns down within 1 ms when it
//! gets louder (*attack*). A sound that jumps from quiet to full scale can
//! still overshoot for a few samples; the master limiter catches that.

use serde::{Deserialize, Serialize};

use super::smoothing;
use crate::audio::CHANNELS;
use crate::time::SampleRate;

/// The lowest threshold, in dBFS: almost everything is compressed.
pub const MIN_THRESHOLD_DB: f32 = -40.0;

/// The highest threshold, in dBFS: only peaks at full scale would be.
pub const MAX_THRESHOLD_DB: f32 = 0.0;

/// The width of the soft knee, in dB.
const KNEE_DB: f32 = 6.0;
/// How fast it turns down a louder sound, in seconds.
const ATTACK_SECONDS: f64 = 0.001;
/// How fast it lets go after it, in seconds.
const RELEASE_SECONDS: f64 = 0.15;
/// Levels below this (dBFS) count as silence, so `log10` never sees 0.
const FLOOR_DB: f32 = -120.0;

/// How strongly sound above the threshold is turned down.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ratio {
    /// 2:1: subtle, keeps the life in a performance.
    Gentle,
    /// 4:1: clearly evener; the usual choice for voices.
    #[default]
    Medium,
    /// 8:1: very even, almost a limiter.
    Strong,
}

impl Ratio {
    /// How many dB louder in make 1 dB louder out.
    fn value(self) -> f32 {
        match self {
            Self::Gentle => 2.0,
            Self::Medium => 4.0,
            Self::Strong => 8.0,
        }
    }
}

/// The compressor's settings, as saved in the project file.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Compressor {
    /// Whether the compressor is switched on.
    pub enabled: bool,
    /// Sound louder than this (in dBFS) is turned down.
    pub threshold_db: f32,
    /// How strongly.
    pub ratio: Ratio,
}

impl Default for Compressor {
    /// Off; at −20 dBFS and 4:1, a common starting point for a voice.
    fn default() -> Self {
        Self {
            enabled: false,
            threshold_db: -20.0,
            ratio: Ratio::default(),
        }
    }
}

/// How many dB a sound at `level_db` is turned down, before make-up gain: 0
/// well below the threshold, `(level − threshold) × (1 − 1/ratio)` well above
/// it, and a smooth curve within the knee between.
#[must_use]
pub fn gain_reduction_db(level_db: f32, threshold_db: f32, ratio: f32) -> f32 {
    let over = level_db - threshold_db;
    let slope = 1.0 - 1.0 / ratio;
    if over <= -KNEE_DB / 2.0 {
        0.0
    } else if over >= KNEE_DB / 2.0 {
        over * slope
    } else {
        let into_knee = over + KNEE_DB / 2.0;
        slope * into_knee * into_knee / (2.0 * KNEE_DB)
    }
}

/// A typical loud word, in dBFS: the automatic make-up gain gives back
/// exactly what it is turned down by, so loud words keep their level.
const MAKE_UP_REFERENCE_DB: f32 = -10.0;

/// The automatic make-up gain, in dB: loud words (at the reference level)
/// come out as loud as they went in, quieter ones louder. Louder peaks stay
/// below full scale: even at −40 dB and 8:1, 0 dBFS comes out at −3.5 dBFS.
fn make_up_db(threshold_db: f32, ratio: f32) -> f32 {
    gain_reduction_db(MAKE_UP_REFERENCE_DB, threshold_db, ratio)
}

/// The running compressor: its settings, and how far it is turning down now.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CompressorFilter {
    threshold_db: f32,
    ratio: f32,
    make_up_db: f32,
    attack: f32,
    release: f32,
    /// The measured level: the peak of both channels, falling slowly.
    level: f32,
    /// The gain reduction applied now, in dB (0 = none).
    reduction_db: f32,
}

impl CompressorFilter {
    /// A compressor for `settings`, starting relaxed (as after silence).
    pub(crate) fn new(settings: Compressor, sample_rate: SampleRate) -> Self {
        let threshold_db = settings
            .threshold_db
            .clamp(MIN_THRESHOLD_DB, MAX_THRESHOLD_DB);
        let ratio = settings.ratio.value();
        Self {
            threshold_db,
            ratio,
            make_up_db: make_up_db(threshold_db, ratio),
            attack: smoothing(ATTACK_SECONDS, sample_rate),
            release: smoothing(RELEASE_SECONDS, sample_rate),
            level: 0.0,
            reduction_db: 0.0,
        }
    }

    /// Forgets earlier audio: relaxed, as after silence.
    pub(crate) fn reset(&mut self) {
        self.level = 0.0;
        self.reduction_db = 0.0;
    }

    /// Takes over how far `old` was turning down, keeping these settings.
    pub(crate) fn continue_from(&mut self, old: &Self) {
        self.level = old.level;
        self.reduction_db = old.reduction_db;
    }

    /// Compresses interleaved stereo `samples` in place.
    pub(crate) fn process(&mut self, samples: &mut [f32]) {
        for frame in samples.chunks_exact_mut(CHANNELS) {
            // Both channels together, so the stereo image never tilts.
            let peak = frame
                .iter()
                .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
            // The detector: up at once to a new peak, down slowly (release).
            if peak > self.level {
                self.level = peak;
            } else {
                self.level += (peak - self.level) * self.release;
            }
            let level_db = (20.0 * self.level.log10()).max(FLOOR_DB);
            let target = gain_reduction_db(level_db, self.threshold_db, self.ratio);
            // Turning down takes the attack time; letting go follows the
            // detector, which already falls slowly.
            if target > self.reduction_db {
                self.reduction_db += (target - self.reduction_db) * self.attack;
            } else {
                self.reduction_db = target;
            }
            let gain = 10_f32.powf((self.make_up_db - self.reduction_db) / 20.0);
            for sample in frame {
                *sample *= gain;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::convert::f32_from_f64;

    const RATE: SampleRate = SampleRate::HZ_48000;

    /// One second of a stereo 200 Hz sine at `db` dBFS.
    fn tone(db: f32) -> Vec<f32> {
        let level = f64::from(10_f32.powf(db / 20.0));
        (0..48_000)
            .flat_map(|n| {
                let t = f64::from(n) / 48_000.0;
                [f32_from_f64(level * (std::f64::consts::TAU * 200.0 * t).sin()); CHANNELS]
            })
            .collect()
    }

    /// The level (peak, in dBFS) of the left channel over the last half second.
    fn settled_db(samples: &[f32]) -> f32 {
        let peak = crate::test_support::settled_peak(samples);
        20.0 * peak.log10()
    }

    fn compressed(db: f32, threshold_db: f32, ratio: Ratio) -> f32 {
        let mut samples = tone(db);
        let settings = Compressor {
            enabled: true,
            threshold_db,
            ratio,
        };
        CompressorFilter::new(settings, RATE).process(&mut samples);
        settled_db(&samples)
    }

    #[test]
    fn below_the_threshold_only_the_make_up_gain_applies() {
        // −20 dB at 4:1: −10 dB is turned down 7.5 dB, so make-up is 7.5 dB.
        let out = compressed(-40.0, -20.0, Ratio::Medium);
        assert!((out - (-40.0 + 7.5)).abs() < 0.1, "{out}");
    }

    #[test]
    fn above_the_threshold_the_ratio_applies() {
        // 14 dB over the threshold at 4:1 comes out 3.5 dB over it, plus
        // 7.5 dB make-up: −20 + 3.5 + 7.5 = −9 dB.
        let out = compressed(-6.0, -20.0, Ratio::Medium);
        assert!((out - -9.0).abs() < 0.2, "{out}");
    }

    #[test]
    fn a_loud_word_keeps_its_level() {
        for ratio in [Ratio::Gentle, Ratio::Medium, Ratio::Strong] {
            let out = compressed(MAKE_UP_REFERENCE_DB, -30.0, ratio);
            assert!((out - MAKE_UP_REFERENCE_DB).abs() < 0.2, "{ratio:?}: {out}");
        }
    }

    #[test]
    fn full_scale_never_comes_out_at_full_scale() {
        for threshold in [MIN_THRESHOLD_DB, -20.0, MAX_THRESHOLD_DB] {
            for ratio in [Ratio::Gentle, Ratio::Medium, Ratio::Strong] {
                let out = compressed(0.0, threshold, ratio);
                assert!(
                    out < -0.5 || threshold > -12.0,
                    "{threshold} {ratio:?}: {out}"
                );
            }
        }
    }

    #[test]
    fn a_stronger_ratio_evens_out_more() {
        // The difference between a loud and a quiet sound shrinks.
        let spread = |ratio| compressed(-3.0, -30.0, ratio) - compressed(-30.0, -30.0, ratio);
        let (gentle, medium, strong) = (
            spread(Ratio::Gentle),
            spread(Ratio::Medium),
            spread(Ratio::Strong),
        );
        assert!(
            gentle > medium && medium > strong,
            "{gentle} {medium} {strong}"
        );
        assert!(strong < 27.0 / 4.0);
    }

    #[test]
    fn compressing_in_pieces_is_the_same_as_in_one_go() {
        let mut whole = tone(-30.0);
        whole.extend(tone(-6.0));
        let mut pieces = whole.clone();
        let settings = Compressor {
            enabled: true,
            ..Compressor::default()
        };
        CompressorFilter::new(settings, RATE).process(&mut whole);
        let mut filter = CompressorFilter::new(settings, RATE);
        for piece in pieces.chunks_mut(777 * CHANNELS) {
            filter.process(piece);
        }
        assert_eq!(whole, pieces);
    }

    proptest! {
        #[test]
        fn louder_in_is_never_quieter_out(
            a in -100.0_f32..10.0,
            b in -100.0_f32..10.0,
            threshold in MIN_THRESHOLD_DB..MAX_THRESHOLD_DB,
            ratio in 1.0_f32..20.0,
        ) {
            prop_assume!(a < b);
            let out = |level: f32| level - gain_reduction_db(level, threshold, ratio);
            prop_assert!(out(a) <= out(b) + 1e-4);
        }

        #[test]
        fn the_knee_has_no_jumps(
            threshold in MIN_THRESHOLD_DB..MAX_THRESHOLD_DB,
            ratio in 1.0_f32..20.0,
            offset in -4.0_f32..4.0,
        ) {
            // A tiny step in level is a tiny step in gain reduction.
            let level = threshold + offset;
            let step = gain_reduction_db(level + 0.01, threshold, ratio)
                - gain_reduction_db(level, threshold, ratio);
            prop_assert!((0.0..=0.011).contains(&step));
        }
    }
}
