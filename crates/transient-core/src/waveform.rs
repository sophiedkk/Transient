//! Waveform overviews for drawing.
//!
//! Drawing a waveform means: for every pixel column, find the lowest and highest
//! sample in the frames that column covers. Zoomed out, one column can cover
//! hundreds of thousands of frames, far too many to scan 60 times a second.
//!
//! So [`Waveform`] precomputes a *peak pyramid*: level 0 stores the minimum and
//! maximum of every block of 256 frames, level 1 of every 1024 frames, and so
//! on, ×4 per level. A query uses the coarsest level whose blocks still fit in
//! one column, so it only ever looks at a handful of values per column. When
//! zoomed in further than 256 frames per column, it reads the samples directly.
//!
//! The pyramid costs about 1/100 of the audio's memory.

use std::ops::Range;

use crate::audio::{AudioBuffer, CHANNELS};
use crate::time::{Frames, frames_from_f64};

/// Frames per peak at level 0.
const BASE_BLOCK: usize = 256;
/// How much larger each level's blocks are than the previous level's.
const LEVEL_FACTOR: usize = 4;

/// The lowest and highest sample value in a range, over both channels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Peak {
    /// The lowest sample (−1.0 is full scale).
    pub min: f32,
    /// The highest sample (1.0 is full scale).
    pub max: f32,
}

impl Peak {
    /// A peak that any real sample will replace.
    const EMPTY: Self = Self {
        min: f32::INFINITY,
        max: f32::NEG_INFINITY,
    };

    fn include_sample(self, sample: f32) -> Self {
        Self {
            min: self.min.min(sample),
            max: self.max.max(sample),
        }
    }

    fn include(self, other: Self) -> Self {
        Self {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }

    fn of_samples(samples: &[f32]) -> Option<Self> {
        let peak = samples
            .iter()
            .fold(Self::EMPTY, |peak, &s| peak.include_sample(s));
        (!samples.is_empty()).then_some(peak)
    }
}

/// An audio buffer plus its peak pyramid. Cheap to clone (the data is shared).
#[derive(Debug, Clone)]
pub struct Waveform {
    buffer: AudioBuffer,
    /// `levels[n]` holds one peak per `BASE_BLOCK * LEVEL_FACTOR^n` frames.
    levels: std::sync::Arc<[Vec<Peak>]>,
}

impl Waveform {
    /// Computes the peak pyramid for `buffer`. Takes about 10 ms per minute of
    /// audio, so call it on a background thread together with decoding.
    #[must_use]
    pub fn new(buffer: AudioBuffer) -> Self {
        let mut levels = vec![
            buffer
                .samples()
                .chunks(BASE_BLOCK * CHANNELS)
                .filter_map(Peak::of_samples)
                .collect::<Vec<_>>(),
        ];
        while let Some(previous) = levels.last()
            && previous.len() > 1
        {
            let next = previous
                .chunks(LEVEL_FACTOR)
                .map(|group| group.iter().fold(Peak::EMPTY, |a, &b| a.include(b)))
                .collect();
            levels.push(next);
        }
        Self {
            buffer,
            levels: levels.into(),
        }
    }

    /// The audio this waveform shows.
    #[must_use]
    pub fn buffer(&self) -> &AudioBuffer {
        &self.buffer
    }

    /// The peaks for `columns` pixel columns, the first starting at frame `start`,
    /// each covering `frames_per_column` frames (fractions allowed when zoomed in).
    ///
    /// Columns before the start or after the end of the audio are `None`.
    #[must_use]
    pub fn peaks(&self, start: f64, frames_per_column: f64, columns: usize) -> Vec<Option<Peak>> {
        self.peaks_within(0..self.buffer.frames(), start, frames_per_column, columns)
    }

    /// Like [`Self::peaks`], but only looking at the frames in `range`: a clip
    /// shows only its own part of the source, even when trimmed.
    #[must_use]
    pub fn peaks_within(
        &self,
        range: Range<Frames>,
        start: f64,
        frames_per_column: f64,
        columns: usize,
    ) -> Vec<Option<Peak>> {
        let frames_per_column = frames_per_column.max(f64::MIN_POSITIVE);
        let level = self.level_for(frames_per_column);
        (0..columns)
            .map(|column| {
                let first = start + column_offset(column) * frames_per_column;
                let first_frame = frames_from_f64(first.floor())?;
                // Every column covers at least one frame, even when zoomed in very far.
                let end_frame =
                    frames_from_f64((first + frames_per_column).ceil())?.max(first_frame + 1);
                let clamped_first = first_frame.max(range.start);
                let clamped_end = end_frame.min(range.end);
                if clamped_first >= clamped_end {
                    return None;
                }
                // Whole blocks would include audio outside the range, so columns at
                // the range's edges read the samples themselves.
                let at_edge = clamped_first != first_frame || clamped_end != end_frame;
                match level {
                    Some(level) if !at_edge => {
                        self.peak_from_level(level, clamped_first, clamped_end)
                    }
                    _ => self.peak_from_samples(clamped_first, clamped_end),
                }
            })
            .collect()
    }

    /// The coarsest level whose blocks are no larger than one column,
    /// or `None` to read samples directly.
    fn level_for(&self, frames_per_column: f64) -> Option<usize> {
        let mut block = BASE_BLOCK;
        let mut level = None;
        for index in 0..self.levels.len() {
            if column_offset(block) > frames_per_column {
                break;
            }
            level = Some(index);
            block = block.saturating_mul(LEVEL_FACTOR);
        }
        level
    }

    fn peak_from_samples(&self, first: Frames, end: Frames) -> Option<Peak> {
        let count = usize::try_from(end.saturating_sub(first)).ok()?;
        Peak::of_samples(self.buffer.frames_from(first, count))
    }

    fn peak_from_level(&self, level: usize, first: Frames, end: Frames) -> Option<Peak> {
        let block =
            Frames::try_from(BASE_BLOCK * LEVEL_FACTOR.pow(u32::try_from(level).ok()?)).ok()?;
        let first_block = usize::try_from(first / block).ok()?;
        let end_block = usize::try_from(end.div_ceil(block)).ok()?;
        let peaks = self.levels.get(level)?;
        let blocks = peaks.get(first_block..end_block.min(peaks.len()))?;
        let peak = blocks.iter().fold(Peak::EMPTY, |a, &b| a.include(b));
        (!blocks.is_empty()).then_some(peak)
    }
}

/// A column index or block size as `f64` (exact for any realistic value).
fn column_offset(value: usize) -> f64 {
    #[expect(
        clippy::as_conversions,
        clippy::cast_precision_loss,
        reason = "column counts and block sizes are far below 2^53"
    )]
    let value = value as f64;
    value
}

#[cfg(test)]
#[allow(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    reason = "test inputs are small whole numbers, so these casts are exact"
)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::time::SampleRate;

    fn waveform(samples: Vec<f32>) -> Waveform {
        Waveform::new(AudioBuffer::new(SampleRate::HZ_48000, samples).unwrap())
    }

    /// The exact peak of frames `first..end`, by looking at every sample.
    fn brute_force(waveform: &Waveform, first: u64, end: u64) -> Option<Peak> {
        let count = usize::try_from(end - first).unwrap();
        Peak::of_samples(waveform.buffer().frames_from(first, count))
    }

    #[test]
    fn levels_shrink_by_four_until_one_peak_is_left() {
        let w = waveform(vec![0.0; 2 * 256 * 64]); // 64 base blocks
        let sizes: Vec<usize> = w.levels.iter().map(Vec::len).collect();
        assert_eq!(sizes, vec![64, 16, 4, 1]);
    }

    #[test]
    fn a_single_click_shows_up_at_every_zoom_level() {
        let mut samples = vec![0.0; 2 * 48_000];
        samples[2 * 30_000] = 0.9; // one sample, left channel
        let w = waveform(samples);
        for frames_per_column in [0.5, 1.0, 37.0, 256.0, 1_000.0, 48_000.0] {
            let columns = (48_000.0 / frames_per_column) as usize;
            let loudest = w
                .peaks(0.0, frames_per_column, columns)
                .into_iter()
                .flatten()
                .map(|p| p.max)
                .fold(0.0, f32::max);
            assert!(
                (loudest - 0.9).abs() < f32::EPSILON,
                "{frames_per_column} frames per column: max {loudest}"
            );
        }
    }

    #[test]
    fn columns_outside_the_audio_are_empty() {
        let w = waveform(vec![0.5; 2 * 1000]);
        let peaks = w.peaks(-100.0, 100.0, 12);
        assert_eq!(peaks[0], None, "before the start");
        assert!(peaks[1].is_some());
        assert_eq!(peaks[11], None, "after the end");
    }

    #[test]
    fn a_range_hides_audio_outside_it() {
        // Loud first half, silent second half; show only frames 500..1000.
        let mut samples = vec![0.9; 2 * 500];
        samples.extend(vec![0.0; 2 * 500]);
        let w = waveform(samples);
        for frames_per_column in [1.0, 100.0, 300.0, 1000.0] {
            let peaks = w.peaks_within(500..1000, 0.0, frames_per_column, 20);
            let loudest = peaks.iter().flatten().map(|p| p.max).fold(0.0, f32::max);
            assert!(
                loudest.abs() < f32::EPSILON,
                "{frames_per_column}: {loudest}"
            );
        }
    }

    proptest! {
        /// Zoomed in (below 256 frames per column), every column is exact.
        #[test]
        fn zoomed_in_peaks_are_exact(
            samples in prop::collection::vec(-1.0_f32..1.0, 2..4000),
            start in 0_u64..1000,
            frames_per_column in 1_u64..200,
        ) {
            let mut samples = samples;
            samples.truncate(samples.len() / 2 * 2);
            let w = waveform(samples);
            let peaks = w.peaks(start as f64, frames_per_column as f64, 10);
            for (column, peak) in peaks.into_iter().enumerate() {
                let first = start + column as u64 * frames_per_column;
                let end = (first + frames_per_column).min(w.buffer().frames());
                let expected = if first < end { brute_force(&w, first, end) } else { None };
                prop_assert_eq!(peak, expected);
            }
        }

        /// Zoomed out, a column may include a little audio from its neighbours
        /// (whole blocks), but never misses a peak inside it.
        #[test]
        fn zoomed_out_peaks_contain_every_sample_of_their_column(
            samples in prop::collection::vec(-1.0_f32..1.0, 2_000..40_000),
            frames_per_column in 256_u64..5_000,
        ) {
            let mut samples = samples;
            samples.truncate(samples.len() / 2 * 2);
            let w = waveform(samples);
            let peaks = w.peaks(0.0, frames_per_column as f64, 8);
            for (column, peak) in peaks.into_iter().enumerate() {
                let first = column as u64 * frames_per_column;
                let end = (first + frames_per_column).min(w.buffer().frames());
                if first >= end {
                    continue;
                }
                let exact = brute_force(&w, first, end).unwrap();
                let peak = peak.unwrap();
                prop_assert!(peak.min <= exact.min && peak.max >= exact.max);
            }
        }
    }
}
