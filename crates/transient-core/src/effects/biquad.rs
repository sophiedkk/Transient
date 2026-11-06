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

impl Biquad {
    /// A high-pass filter: frequencies below `frequency_hz` are turned down,
    /// 12 dB per octave. `q` (sharpness) shapes the bend at the frequency:
    /// with 1/√2 (a 2nd-order Butterworth filter) the response is as flat as
    /// possible above it and 3 dB down at it. Steeper filters chain biquads
    /// with other Qs (see [`super::low_cut`]).
    pub(crate) fn high_pass(frequency_hz: f32, q: f64, sample_rate: SampleRate) -> Self {
        let rate = f64::from(sample_rate.hz());
        // Keep the frequency where the formulas work: above 0, below Nyquist.
        let frequency = f64::from(frequency_hz).clamp(1.0, rate * 0.45);
        let omega = std::f64::consts::TAU * frequency / rate;
        let (sin, cos) = omega.sin_cos();
        let alpha = sin / (2.0 * q);
        let a0 = 1.0 + alpha;
        let b0 = f64::midpoint(1.0, cos) / a0;
        Self {
            b: [b0, -2.0 * b0, b0],
            a: [-2.0 * cos / a0, (1.0 - alpha) / a0],
            state: [[0.0; 2]; CHANNELS],
        }
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
    use crate::test_support::{settled_peak, sine};

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
        let mut samples = vec![0.5; 48_000 * CHANNELS];
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
