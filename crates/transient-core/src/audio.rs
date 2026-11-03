//! Decoded audio in memory.
//!
//! Every imported file is converted to the same layout: 32-bit float samples,
//! two channels (stereo), *interleaved* — left, right, left, right, … — at the
//! project sample rate. One left+right pair is one *frame*.
//! Having a single layout keeps mixing simple: no conversions while playing.

use std::sync::Arc;

use thiserror::Error;

use crate::time::{Frames, SampleRate};

/// Transient always works in stereo.
pub const CHANNELS: usize = 2;

/// Why an [`AudioBuffer`] could not be created.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum AudioError {
    /// Stereo audio needs an even number of samples (a left and a right for each frame).
    #[error("expected an even number of samples for stereo audio, got {0}")]
    IncompleteFrame(usize),
}

/// Stereo audio, decoded into memory.
///
/// Cloning is cheap: the samples are shared (`Arc`), not copied. That is what lets
/// many clips refer to the same audio, and what makes undo snapshots cheap.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioBuffer {
    sample_rate: SampleRate,
    samples: Arc<[f32]>,
}

impl AudioBuffer {
    /// Wraps interleaved stereo samples.
    ///
    /// # Errors
    ///
    /// Returns [`AudioError::IncompleteFrame`] if `samples` has an odd length.
    pub fn new(sample_rate: SampleRate, samples: Vec<f32>) -> Result<Self, AudioError> {
        if !samples.len().is_multiple_of(CHANNELS) {
            return Err(AudioError::IncompleteFrame(samples.len()));
        }
        Ok(Self {
            sample_rate,
            samples: samples.into(),
        })
    }

    /// The sample rate of the audio.
    #[must_use]
    pub fn sample_rate(&self) -> SampleRate {
        self.sample_rate
    }

    /// All samples, interleaved (left, right, left, right, …).
    #[must_use]
    pub fn samples(&self) -> &[f32] {
        &self.samples
    }

    /// The length in frames.
    #[must_use]
    pub fn frames(&self) -> Frames {
        // A usize always fits in a u64 on the 64-bit platforms GNOME supports.
        Frames::try_from(self.samples.len() / CHANNELS).unwrap_or(Frames::MAX)
    }

    /// The length in seconds.
    #[must_use]
    pub fn duration_seconds(&self) -> f64 {
        self.sample_rate.frames_to_seconds(self.frames())
    }

    /// Up to `count` frames starting at frame `start`, as interleaved samples.
    /// Returns fewer frames near the end, and an empty slice past the end.
    #[must_use]
    pub fn frames_from(&self, start: Frames, count: usize) -> &[f32] {
        let Ok(start) = usize::try_from(start) else {
            return &[];
        };
        let first = start.saturating_mul(CHANNELS).min(self.samples.len());
        let last = first
            .saturating_add(count.saturating_mul(CHANNELS))
            .min(self.samples.len());
        self.samples.get(first..last).unwrap_or_default()
    }

    /// The loudest sample, from 0.0 (silence) to 1.0 (full scale) or above (clipping).
    #[must_use]
    pub fn peak(&self) -> f32 {
        self.samples.iter().fold(0.0, |peak, s| peak.max(s.abs()))
    }

    /// The average loudness (root mean square), on the same scale as [`Self::peak`].
    #[must_use]
    pub fn rms(&self) -> f32 {
        if self.samples.is_empty() {
            return 0.0;
        }
        let sum_of_squares: f64 = self.samples.iter().map(|s| f64::from(*s).powi(2)).sum();
        #[expect(
            clippy::as_conversions,
            clippy::cast_precision_loss,
            reason = "a sample count is far below 2^53, so the conversion is exact"
        )]
        let count = self.samples.len() as f64;
        #[expect(
            clippy::as_conversions,
            clippy::cast_possible_truncation,
            reason = "an RMS of f32 samples always fits in an f32"
        )]
        let rms = (sum_of_squares / count).sqrt() as f32;
        rms
    }
}

/// Converts a level (1.0 = full scale) to decibels relative to full scale (dBFS).
/// Silence has no decibel value and returns `None`.
#[must_use]
pub fn to_dbfs(level: f32) -> Option<f64> {
    (level > 0.0).then(|| 20.0 * f64::from(level).log10())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer(samples: Vec<f32>) -> AudioBuffer {
        AudioBuffer::new(SampleRate::HZ_48000, samples).unwrap()
    }

    #[test]
    fn an_odd_number_of_samples_is_refused() {
        let error = AudioBuffer::new(SampleRate::HZ_48000, vec![0.0; 3]).unwrap_err();
        assert_eq!(error, AudioError::IncompleteFrame(3));
    }

    #[test]
    fn length_is_counted_in_frames() {
        let one_second = buffer(vec![0.0; 2 * 48_000]);
        assert_eq!(one_second.frames(), 48_000);
        assert!((one_second.duration_seconds() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn frames_from_stops_at_the_end() {
        let audio = buffer(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]); // three frames
        assert_eq!(audio.frames_from(1, 1), &[3.0, 4.0]);
        assert_eq!(audio.frames_from(1, 10), &[3.0, 4.0, 5.0, 6.0]);
        assert!(audio.frames_from(3, 1).is_empty());
        assert!(audio.frames_from(Frames::MAX, 1).is_empty());
    }

    #[test]
    fn peak_and_rms_of_a_square_wave() {
        let audio = buffer(vec![0.5, -0.5, -0.5, 0.5]);
        assert!((audio.peak() - 0.5).abs() < f32::EPSILON);
        assert!((audio.rms() - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn silence_has_no_decibel_value() {
        assert_eq!(to_dbfs(0.0), None);
        let half = to_dbfs(0.5).unwrap();
        assert!(
            (half - -6.0206).abs() < 0.001,
            "0.5 is about -6 dBFS, got {half}"
        );
    }
}
