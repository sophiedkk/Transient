//! Master limiter: keeps the whole mix from clipping.
//!
//! When tracks add up (or a compressor turns a track up), peaks can pass full
//! scale and would be cut off flat, which sounds harsh. The limiter turns the
//! mix down just enough, just in time, so no sample goes above −1 dBFS.
//!
//! It *looks ahead*: the audio passes through a delay of 1.5 ms, so the
//! limiter sees a peak coming and starts turning down before it arrives. The
//! gain it needs for each frame goes through two steps of the same length as
//! the delay: the lowest gain needed anywhere in the window (so a peak is
//! never missed), then an average (so the turning down is a smooth ramp
//! instead of a step). Together they guarantee the ceiling: every gain that
//! the average includes is at most the gain the peak needs. Afterwards it
//! lets go over 100 ms.
//!
//! The delay is hidden from the rest of Transient: [`crate::mix::Mix`]
//! renders the tracks [`Limiter::delay_frames`] ahead, so the limited audio
//! comes out in time.

use std::collections::VecDeque;

use super::smoothing;
use crate::audio::CHANNELS;
use crate::time::{Frames, SampleRate};

/// The highest level a sample can reach, in dBFS: 1 dB of room below full
/// scale, for converters and lossy formats (MP3, Opus) that overshoot.
pub const CEILING_DB: f32 = -1.0;

/// How far ahead it looks, in seconds.
const LOOKAHEAD_SECONDS: f64 = 0.0015;
/// How fast it lets go after a peak, in seconds.
const RELEASE_SECONDS: f64 = 0.1;

/// The running limiter: the delayed audio and what it computed so far.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Limiter {
    /// The window length in frames (the lookahead plus one).
    window: usize,
    ceiling: f32,
    release: f32,
    /// The audio waiting to come out, oldest first (`window − 1` frames).
    delay: VecDeque<[f32; CHANNELS]>,
    /// The gain each of the last `window` frames needs, oldest first.
    needed: VecDeque<f32>,
    /// The lowest needed gain over the window, for each of the last `window`
    /// frames, and their sum (for the average).
    lowest: VecDeque<f32>,
    lowest_sum: f64,
    /// The gain applied now, after the release.
    gain: f32,
    /// The timeline frame the mix should push next to continue without a
    /// gap; `None` until it has been filled (see [`crate::mix::Mix`]).
    pub(crate) next_input: Option<Frames>,
}

impl Limiter {
    /// A limiter for audio at `sample_rate`, empty (as after silence).
    pub(crate) fn new(sample_rate: SampleRate) -> Self {
        let lookahead = sample_rate
            .seconds_to_frames(LOOKAHEAD_SECONDS)
            .and_then(|frames| usize::try_from(frames).ok())
            .unwrap_or(0);
        let mut limiter = Self {
            window: lookahead + 1,
            ceiling: 10_f32.powf(CEILING_DB / 20.0),
            release: smoothing(RELEASE_SECONDS, sample_rate),
            delay: VecDeque::new(),
            needed: VecDeque::new(),
            lowest: VecDeque::new(),
            lowest_sum: 0.0,
            gain: 1.0,
            next_input: None,
        };
        limiter.reset();
        limiter
    }

    /// How many frames the audio is delayed by.
    pub(crate) fn delay_frames(&self) -> usize {
        self.window - 1
    }

    /// Forgets earlier audio: the delay holds silence, nothing needs turning
    /// down.
    pub(crate) fn reset(&mut self) {
        self.delay = std::iter::repeat_n([0.0; CHANNELS], self.window - 1).collect();
        self.needed = std::iter::repeat_n(1.0, self.window).collect();
        self.lowest = std::iter::repeat_n(1.0, self.window).collect();
        self.lowest_sum = crate::convert::f64_from_usize(self.window);
        self.gain = 1.0;
        self.next_input = None;
    }

    /// Pushes interleaved stereo `input` in and writes what comes out of the
    /// delay, limited, to `output` (as long as `input`).
    pub(crate) fn process(&mut self, input: &[f32], output: &mut [f32]) {
        let window = crate::convert::f64_from_usize(self.window);
        for (frame, out) in input
            .chunks_exact(CHANNELS)
            .zip(output.chunks_exact_mut(CHANNELS))
        {
            let peak = frame
                .iter()
                .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
            let needed = if peak > self.ceiling {
                self.ceiling / peak
            } else {
                1.0
            };
            self.needed.pop_front();
            self.needed.push_back(needed);
            let lowest = self.needed.iter().copied().fold(1.0_f32, f32::min);
            self.lowest_sum -= self.lowest.pop_front().map_or(0.0, f64::from);
            self.lowest.push_back(lowest);
            self.lowest_sum += f64::from(lowest);
            let smooth = crate::convert::f32_from_f64(self.lowest_sum / window).min(1.0);
            // Down at once (the ramp is already smooth), up slowly.
            self.gain = if smooth < self.gain {
                smooth
            } else {
                self.gain + (smooth - self.gain) * self.release
            };
            let mut sample = [0.0; CHANNELS];
            for (slot, value) in sample.iter_mut().zip(frame) {
                *slot = *value;
            }
            self.delay.push_back(sample);
            let delayed = self.delay.pop_front().unwrap_or([0.0; CHANNELS]);
            for (out, value) in out.iter_mut().zip(delayed) {
                *out = value * self.gain;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    const RATE: SampleRate = SampleRate::HZ_48000;

    fn limited(input: &[f32]) -> Vec<f32> {
        let mut limiter = Limiter::new(RATE);
        let mut output = vec![0.0; input.len()];
        limiter.process(input, &mut output);
        output
    }

    #[test]
    fn audio_under_the_ceiling_passes_unchanged_but_delayed() {
        let input: Vec<f32> = (0..2000)
            .map(|n| f32::from(u16::try_from(n % 100).unwrap()) / 200.0)
            .collect();
        let output = limited(&input);
        let delay = Limiter::new(RATE).delay_frames() * CHANNELS;
        assert_eq!(output[delay..], input[..input.len() - delay]);
        assert!(output[..delay].iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn a_sudden_peak_is_caught_before_it_arrives() {
        // Quiet, then one frame at twice full scale.
        let mut input = vec![0.1; 400 * CHANNELS];
        input[300 * CHANNELS] = 2.0;
        input[300 * CHANNELS + 1] = -2.0;
        let output = limited(&input);
        let ceiling = 10_f32.powf(CEILING_DB / 20.0);
        assert!(
            output
                .iter()
                .all(|sample| sample.abs() <= ceiling * 1.000_01)
        );
        // The quiet audio before the peak is turned down only shortly before.
        let delay = Limiter::new(RATE).delay_frames();
        assert!((output[(200 + delay) * CHANNELS] - 0.1).abs() < 1e-6);
    }

    #[test]
    fn it_lets_go_after_a_peak() {
        let mut input = vec![0.5; 40_000 * CHANNELS];
        input[100 * CHANNELS] = 4.0;
        let output = limited(&input);
        // Turned down right after the peak...
        assert!(output[1000 * CHANNELS] < 0.3);
        // ...and 0.6 s later (six release times) back to (almost) 1.
        assert!((output[30_000 * CHANNELS] - 0.5).abs() < 0.005);
    }

    #[test]
    fn limiting_in_pieces_is_the_same_as_in_one_go() {
        let input: Vec<f32> = (0..9000)
            .map(|n| (f32::from(u16::try_from(n).unwrap()) * 0.01).sin() * 1.7)
            .collect();
        let whole = limited(&input);
        let mut limiter = Limiter::new(RATE);
        let mut pieces = vec![0.0; input.len()];
        for (input, output) in input.chunks(334).zip(pieces.chunks_mut(334)) {
            limiter.process(input, output);
        }
        assert_eq!(whole, pieces);
    }

    proptest! {
        #[test]
        fn nothing_ever_comes_out_above_the_ceiling(
            input in prop::collection::vec(-8.0_f32..8.0, 2..4000),
        ) {
            let input = &input[..input.len() / CHANNELS * CHANNELS];
            let ceiling = 10_f32.powf(CEILING_DB / 20.0);
            for sample in limited(input) {
                prop_assert!(sample.abs() <= ceiling * 1.000_01, "{sample}");
            }
        }
    }
}
