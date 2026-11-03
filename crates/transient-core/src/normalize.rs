//! Normalizing: working out the clip gain that makes a clip as loud as wanted.
//!
//! Both ways measure the clip's own part of its source (its offset and length,
//! ignoring fades and the current gain) and return the gain to set:
//!
//! - **Peak**: the loudest sample ends up at a target level, by default −1 dBFS.
//!   Simple and never clips, but says little about how loud it *sounds*.
//! - **Loudness**: the perceived loudness (EBU R128 "integrated loudness", in
//!   LUFS) ends up at a target, by default −16 LUFS (common for podcasts). The
//!   gain is lowered if needed so the peak stays below the ceiling.

use ebur128::{EbuR128, Mode};

use crate::audio::{AudioBuffer, CHANNELS, to_dbfs};
use crate::mix::SILENT_DB;
use crate::project::Clip;

/// The default peak target and ceiling, in dBFS: a little headroom below 0.
pub const PEAK_TARGET_DB: f64 = -1.0;
/// The default loudness target, in LUFS.
pub const LOUDNESS_TARGET_LUFS: f64 = -16.0;

/// The result of loudness normalization.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LoudnessGain {
    /// The clip gain to set, in dB.
    pub gain_db: f32,
    /// True if the gain was lowered to keep the peak below the ceiling, so the
    /// clip ends up quieter than the target.
    pub limited: bool,
}

/// The gain that puts the clip's loudest sample at `target_db`.
/// `None` if the clip is silent.
#[must_use]
pub fn peak_gain(audio: &AudioBuffer, clip: &Clip, target_db: f64) -> Option<f32> {
    let peak = clip_samples(audio, clip)
        .iter()
        .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
    let peak_db = to_dbfs(peak)?;
    Some(to_gain_db(target_db - peak_db))
}

/// The gain that brings the clip's loudness to `target_lufs` without its peak
/// passing `ceiling_db`. `None` if the clip is silent or too short to measure
/// (EBU R128 needs at least 0.4 seconds).
#[must_use]
pub fn loudness_gain(
    audio: &AudioBuffer,
    clip: &Clip,
    target_lufs: f64,
    ceiling_db: f64,
) -> Option<LoudnessGain> {
    let channels = u32::try_from(CHANNELS).ok()?;
    let mut meter = EbuR128::new(
        channels,
        audio.sample_rate().hz(),
        Mode::I | Mode::SAMPLE_PEAK,
    )
    .ok()?;
    meter.add_frames_f32(clip_samples(audio, clip)).ok()?;
    let loudness = meter.loudness_global().ok().filter(|l| l.is_finite())?;
    let peak = (0..channels)
        .filter_map(|channel| meter.sample_peak(channel).ok())
        .fold(0.0_f64, f64::max);
    if peak <= 0.0 {
        return None;
    }
    let peak_db = 20.0 * peak.log10();

    let wanted = target_lufs - loudness;
    let allowed = ceiling_db - peak_db;
    Some(LoudnessGain {
        gain_db: to_gain_db(wanted.min(allowed)),
        limited: allowed < wanted,
    })
}

/// The samples a clip plays (before fades and gain).
fn clip_samples<'a>(audio: &'a AudioBuffer, clip: &Clip) -> &'a [f32] {
    let length = usize::try_from(clip.length).unwrap_or(usize::MAX);
    audio.frames_from(clip.offset, length)
}

/// A gain in dB as `f32`, kept within what the mixer can express.
fn to_gain_db(db: f64) -> f32 {
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        reason = "clamped to a small range first"
    )]
    let db = db.clamp(f64::from(SILENT_DB), 60.0) as f32;
    db
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::SampleRate;

    /// `seconds` of a 1 kHz sine at `amplitude` in both channels, as one clip.
    fn sine(amplitude: f32, seconds: u16) -> (AudioBuffer, Clip) {
        let frames = 48_000 * u32::from(seconds);
        let samples: Vec<f32> = (0..frames)
            .flat_map(|n| {
                let phase = f64::from(n) * 1000.0 * std::f64::consts::TAU / 48_000.0;
                #[expect(
                    clippy::as_conversions,
                    clippy::cast_possible_truncation,
                    reason = "test signal"
                )]
                let value = amplitude * phase.sin() as f32;
                [value, value]
            })
            .collect();
        let clip = Clip {
            length: u64::from(frames),
            ..Clip::default()
        };
        (
            AudioBuffer::new(SampleRate::HZ_48000, samples).unwrap(),
            clip,
        )
    }

    /// Applies a gain the way the mixer does.
    fn apply(audio: &AudioBuffer, gain_db: f32) -> AudioBuffer {
        let gain = crate::mix::db_to_gain(gain_db);
        let samples = audio.samples().iter().map(|s| s * gain).collect();
        AudioBuffer::new(audio.sample_rate(), samples).unwrap()
    }

    #[test]
    fn peak_normalizing_puts_the_peak_at_the_target() {
        let (audio, clip) = sine(0.25, 1);
        let gain = peak_gain(&audio, &clip, -1.0).unwrap();
        // 0.25 is −12.04 dBFS, so +11.04 dB brings it to −1 dBFS.
        assert!((gain - 11.04).abs() < 0.01, "gain {gain}");
    }

    #[test]
    fn a_silent_clip_cannot_be_normalized() {
        let (audio, clip) = sine(0.0, 1);
        assert_eq!(peak_gain(&audio, &clip, -1.0), None);
        assert_eq!(loudness_gain(&audio, &clip, -16.0, -1.0), None);
    }

    #[test]
    fn loudness_normalizing_reaches_the_target() {
        let (audio, clip) = sine(0.05, 3);
        let result = loudness_gain(&audio, &clip, -16.0, -1.0).unwrap();
        assert!(!result.limited);
        // Measure again after applying the gain: now at the target.
        let louder = apply(&audio, result.gain_db);
        let again = loudness_gain(&louder, &clip, -16.0, 100.0).unwrap();
        assert!(again.gain_db.abs() < 0.05, "still {} dB off", again.gain_db);
    }

    #[test]
    fn loudness_normalizing_does_not_push_the_peak_past_the_ceiling() {
        let (audio, clip) = sine(0.5, 3);
        // A sine at −6 dBFS peak is far louder than −30 LUFS... ask for very loud.
        let result = loudness_gain(&audio, &clip, 0.0, -1.0).unwrap();
        assert!(result.limited);
        assert!(
            (result.gain_db - 5.02).abs() < 0.05,
            "gain {}",
            result.gain_db
        );
    }
}
