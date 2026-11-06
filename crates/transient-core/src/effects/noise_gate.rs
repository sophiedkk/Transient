//! Noise gate: turns a track down while it is quiet.
//!
//! Between phrases a voice recording still has hiss, hum and room noise. A
//! gate listens to the level: while it is below a threshold, the gate is
//! "closed" and turns the sound down; when someone speaks, it opens again.
//!
//! To sound natural it opens fast (1 ms, so the start of a word is not lost),
//! stays open a moment after the level drops (50 ms, the *hold*), and then
//! closes slowly (150 ms, so a word's tail fades instead of being cut). It
//! closes only 6 dB below where it opens (*hysteresis*), so a level hovering
//! around the threshold does not make it flutter.

use serde::{Deserialize, Serialize};

use super::smoothing;
use crate::audio::{CHANNELS, amplitude_from_db};
use crate::time::SampleRate;

/// The lowest threshold the gate can be set to, in dBFS: only near-silence
/// is turned down.
pub const MIN_THRESHOLD_DB: f32 = -80.0;

/// The highest threshold, in dBFS: quiet speech is turned down too.
pub const MAX_THRESHOLD_DB: f32 = -20.0;

/// Where a new gate starts, in dBFS: above typical room noise, below quiet
/// speech.
pub const DEFAULT_THRESHOLD_DB: f32 = -45.0;

/// How far [`Reduction::Strong`] turns the sound down, in dB.
const STRONG_REDUCTION_DB: f32 = -20.0;
/// How far [`Reduction::Light`] turns the sound down, in dB.
const LIGHT_REDUCTION_DB: f32 = -10.0;

/// How much quieter the gate closes than it opens, in dB.
const HYSTERESIS_DB: f32 = 6.0;
/// How fast the gate opens, in seconds.
const ATTACK_SECONDS: f64 = 0.001;
/// How long the gate stays open after the level drops, in seconds.
const HOLD_SECONDS: f64 = 0.05;
/// How fast the gate closes, in seconds.
const RELEASE_SECONDS: f64 = 0.15;
/// How fast the measured level falls after a peak, in seconds: smooths the
/// level between the peaks of a sound wave.
const LEVEL_FALL_SECONDS: f64 = 0.01;

/// How far the gate turns the sound down while closed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reduction {
    /// Fully silent.
    Silence,
    /// 20 dB quieter: noise all but gone, but not an unnatural dead silence.
    #[default]
    Strong,
    /// 10 dB quieter: some room sound stays, the most natural on voices.
    Light,
}

impl Reduction {
    /// What the sound is multiplied by while the gate is closed.
    fn gain(self) -> f32 {
        match self {
            Self::Silence => 0.0,
            Self::Strong => amplitude_from_db(STRONG_REDUCTION_DB),
            Self::Light => amplitude_from_db(LIGHT_REDUCTION_DB),
        }
    }
}

/// The noise gate's settings, as saved in the project file.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NoiseGate {
    /// Whether the gate is switched on.
    pub enabled: bool,
    /// Sound quieter than this (in dBFS) is turned down.
    pub threshold_db: f32,
    /// How far it is turned down.
    pub reduction: Reduction,
}

impl Default for NoiseGate {
    /// Off, at [`DEFAULT_THRESHOLD_DB`].
    fn default() -> Self {
        Self {
            enabled: false,
            threshold_db: DEFAULT_THRESHOLD_DB,
            reduction: Reduction::default(),
        }
    }
}

/// The running gate: its settings in the form it needs, and what it measured
/// so far.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NoiseGateFilter {
    /// The level at which it opens and closes, as sample values.
    open_level: f32,
    close_level: f32,
    /// The gain while closed.
    floor: f32,
    attack: f32,
    release: f32,
    level_fall: f32,
    hold_frames: u32,
    /// The measured level: the peak of both channels, falling slowly.
    level: f32,
    /// The gain applied now, moving towards 1 (open) or `floor` (closed).
    gain: f32,
    open: bool,
    /// Frames left before a gate whose level dropped starts closing.
    hold_left: u32,
}

impl NoiseGateFilter {
    /// A gate for `settings`, starting closed (as after silence).
    pub(crate) fn new(settings: NoiseGate, sample_rate: SampleRate) -> Self {
        let threshold = settings
            .threshold_db
            .clamp(MIN_THRESHOLD_DB, MAX_THRESHOLD_DB);
        let floor = settings.reduction.gain();
        let hold = HOLD_SECONDS * f64::from(sample_rate.hz());
        Self {
            open_level: amplitude_from_db(threshold),
            close_level: amplitude_from_db(threshold - HYSTERESIS_DB),
            floor,
            attack: smoothing(ATTACK_SECONDS, sample_rate),
            release: smoothing(RELEASE_SECONDS, sample_rate),
            level_fall: smoothing(LEVEL_FALL_SECONDS, sample_rate),
            hold_frames: crate::convert::u32_from_f64(hold),
            level: 0.0,
            gain: floor,
            open: false,
            hold_left: 0,
        }
    }

    /// Forgets earlier audio: closed, as after silence.
    pub(crate) fn reset(&mut self) {
        self.level = 0.0;
        self.gain = self.floor;
        self.open = false;
        self.hold_left = 0;
    }

    /// Takes over what `old` measured, keeping this gate's settings.
    pub(crate) fn continue_from(&mut self, old: &Self) {
        self.level = old.level;
        self.gain = old.gain;
        self.open = old.open;
        self.hold_left = old.hold_left.min(self.hold_frames);
    }

    /// Gates interleaved stereo `samples` in place.
    pub(crate) fn process(&mut self, samples: &mut [f32]) {
        for frame in samples.chunks_exact_mut(CHANNELS) {
            // Both channels together, so the stereo image never tilts.
            let peak = frame
                .iter()
                .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
            if peak > self.level {
                self.level = peak;
            } else {
                self.level += (peak - self.level) * self.level_fall;
            }
            if self.level >= self.open_level {
                self.open = true;
                self.hold_left = self.hold_frames;
            } else if self.open && self.level < self.close_level {
                if self.hold_left == 0 {
                    self.open = false;
                } else {
                    self.hold_left -= 1;
                }
            }
            let (target, speed) = if self.open {
                (1.0, self.attack)
            } else {
                (self.floor, self.release)
            };
            self.gain += (target - self.gain) * speed;
            for sample in frame {
                *sample *= self.gain;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::f32_from_f64;

    const RATE: SampleRate = SampleRate::HZ_48000;

    /// `seconds` of a stereo 200 Hz sine at `db` dBFS.
    fn tone(db: f32, seconds: f64) -> Vec<f32> {
        let level = f64::from(amplitude_from_db(db));
        let frames = RATE.seconds_to_frames(seconds).unwrap();
        (0..frames)
            .flat_map(|n| {
                let t = crate::time::frames_to_f64(n) / f64::from(RATE.hz());
                [f32_from_f64(level * (std::f64::consts::TAU * 200.0 * t).sin()); CHANNELS]
            })
            .collect()
    }

    /// The peak of the left channel from `from` to `to` seconds.
    fn peak(samples: &[f32], from: f64, to: f64) -> f32 {
        let frame =
            |seconds: f64| usize::try_from(RATE.seconds_to_frames(seconds).unwrap()).unwrap();
        samples[frame(from) * CHANNELS..frame(to) * CHANNELS]
            .iter()
            .step_by(CHANNELS)
            .fold(0.0, |peak, sample| peak.max(sample.abs()))
    }

    fn gate(reduction: Reduction) -> NoiseGateFilter {
        let settings = NoiseGate {
            enabled: true,
            threshold_db: -45.0,
            reduction,
        };
        NoiseGateFilter::new(settings, RATE)
    }

    #[test]
    fn loud_sound_passes_unchanged() {
        let mut samples = tone(-12.0, 0.5);
        gate(Reduction::Strong).process(&mut samples);
        let expected = amplitude_from_db(-12.0);
        assert!((peak(&samples, 0.1, 0.5) - expected).abs() < 0.001 * expected);
    }

    #[test]
    fn quiet_sound_is_turned_down_by_the_reduction() {
        for (reduction, db) in [
            (Reduction::Silence, None),
            (Reduction::Strong, Some(-20.0)),
            (Reduction::Light, Some(-10.0)),
        ] {
            let mut samples = tone(-60.0, 1.0);
            gate(reduction).process(&mut samples);
            let expected = db.map_or(0.0, |db| amplitude_from_db(-60.0) * amplitude_from_db(db));
            let actual = peak(&samples, 0.5, 1.0);
            assert!((actual - expected).abs() < 1e-5, "{reduction:?}: {actual}");
        }
    }

    #[test]
    fn after_a_phrase_it_holds_then_fades() {
        // Speech, then only room noise.
        let mut samples = tone(-12.0, 0.5);
        samples.extend(tone(-60.0, 1.0));
        gate(Reduction::Silence).process(&mut samples);
        let noise = amplitude_from_db(-60.0);
        // Right after the phrase the noise is still heard (held open)...
        assert!(peak(&samples, 0.52, 0.54) > 0.9 * noise);
        // ...then it fades out, and is gone a second later.
        assert!(peak(&samples, 1.4, 1.5) < 0.01 * noise);
    }

    #[test]
    fn a_level_just_under_the_threshold_keeps_it_open() {
        // Opened by speech, then 3 dB under the threshold: within the 6 dB
        // hysteresis, so it stays open instead of fluttering.
        let mut samples = tone(-12.0, 0.2);
        samples.extend(tone(-48.0, 1.0));
        gate(Reduction::Silence).process(&mut samples);
        let level = amplitude_from_db(-48.0);
        assert!((peak(&samples, 0.8, 1.2) - level).abs() < 0.01 * level);
    }

    #[test]
    fn gating_in_pieces_is_the_same_as_in_one_go() {
        let mut whole = tone(-12.0, 0.3);
        whole.extend(tone(-60.0, 0.5));
        let mut pieces = whole.clone();
        gate(Reduction::Strong).process(&mut whole);
        let mut filter = gate(Reduction::Strong);
        for piece in pieces.chunks_mut(999 * CHANNELS) {
            filter.process(piece);
        }
        assert_eq!(whole, pieces);
    }
}
