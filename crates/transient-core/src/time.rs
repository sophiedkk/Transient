//! Time in Transient is counted in *frames*.
//!
//! A frame is one sample for every channel at a single moment in time: at 48 kHz
//! there are 48 000 frames per second. Counting in whole frames (a `u64`) keeps
//! edits sample-accurate; seconds (`f64`) are only used to show time to people.

use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};

/// A position or a length on the timeline, in frames.
pub type Frames = u64;

/// The largest frame count that an `f64` can represent exactly (2^53).
/// That is more than 5 000 years of audio at 48 kHz, so it is never a real limit.
const MAX_EXACT_FRAMES: f64 = 9_007_199_254_740_992.0;

/// Frames per second. Can never be zero, so dividing by it is always safe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SampleRate(NonZeroU32);

impl SampleRate {
    /// 48 kHz, the standard rate for video and for PipeWire, and Transient's default.
    pub const HZ_48000: Self = Self::from_const(48_000);

    /// Creates a sample rate, or returns `None` for 0 Hz.
    #[must_use]
    pub fn new(hz: u32) -> Option<Self> {
        NonZeroU32::new(hz).map(Self)
    }

    /// Helper for the constants above; a zero falls back to 1 Hz instead of panicking.
    const fn from_const(hz: u32) -> Self {
        match NonZeroU32::new(hz) {
            Some(hz) => Self(hz),
            None => Self(NonZeroU32::MIN),
        }
    }

    /// The rate in frames per second.
    #[must_use]
    pub const fn hz(self) -> u32 {
        self.0.get()
    }

    /// Converts a frame count to seconds.
    #[must_use]
    pub fn frames_to_seconds(self, frames: Frames) -> f64 {
        #[expect(
            clippy::as_conversions,
            clippy::cast_precision_loss,
            reason = "exact for every count below 2^53 frames (see MAX_EXACT_FRAMES)"
        )]
        let frames = frames as f64;
        frames / f64::from(self.hz())
    }

    /// Converts seconds to the nearest frame count.
    ///
    /// Returns `None` for negative, infinite or NaN input, and for durations too
    /// large to count exactly.
    #[must_use]
    pub fn seconds_to_frames(self, seconds: f64) -> Option<Frames> {
        let frames = (seconds * f64::from(self.hz())).round();
        if !(0.0..=MAX_EXACT_FRAMES).contains(&frames) {
            return None;
        }
        #[expect(
            clippy::as_conversions,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "range-checked above: a whole number in 0..=2^53"
        )]
        Some(frames as Frames)
    }
}

/// Formats a timeline position for display, like `1:02.345` (minutes, seconds,
/// milliseconds) or `1:02:03.456` from one hour on.
#[must_use]
pub fn format_position(frames: Frames, rate: SampleRate) -> String {
    let total_ms = frames.saturating_mul(1000) / Frames::from(rate.hz());
    let ms = total_ms % 1000;
    let total_seconds = total_ms / 1000;
    let seconds = total_seconds % 60;
    let minutes = (total_seconds / 60) % 60;
    let hours = total_seconds / 3600;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}.{ms:03}")
    } else {
        format!("{minutes}:{seconds:02}.{ms:03}")
    }
}

impl Default for SampleRate {
    fn default() -> Self {
        Self::HZ_48000
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn one_second_at_48_khz() {
        let rate = SampleRate::HZ_48000;
        assert_eq!(rate.seconds_to_frames(1.0), Some(48_000));
        assert!((rate.frames_to_seconds(24_000) - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn positions_are_formatted_for_display() {
        let rate = SampleRate::HZ_48000;
        assert_eq!(format_position(0, rate), "0:00.000");
        assert_eq!(format_position(62 * 48_000 + 16_560, rate), "1:02.345");
        assert_eq!(
            format_position(3_723 * 48_000 + 21_888, rate),
            "1:02:03.456"
        );
    }

    #[test]
    fn zero_hz_is_rejected() {
        assert_eq!(SampleRate::new(0), None);
    }

    #[test]
    fn invalid_seconds_are_rejected() {
        let rate = SampleRate::HZ_48000;
        assert_eq!(rate.seconds_to_frames(-1.0), None);
        assert_eq!(rate.seconds_to_frames(f64::NAN), None);
        assert_eq!(rate.seconds_to_frames(f64::INFINITY), None);
    }

    proptest! {
        /// Converting frames to seconds and back must give the same frame,
        /// at every common sample rate, for any project up to ~6 hours long.
        #[test]
        fn frames_survive_a_round_trip(
            frames in 0..(1_u64 << 40),
            hz in prop::sample::select(vec![8_000_u32, 22_050, 44_100, 48_000, 96_000, 192_000]),
        ) {
            let rate = SampleRate::new(hz).unwrap();
            let seconds = rate.frames_to_seconds(frames);
            prop_assert_eq!(rate.seconds_to_frames(seconds), Some(frames));
        }
    }
}
