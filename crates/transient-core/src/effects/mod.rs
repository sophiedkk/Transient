//! Effects: sound processing on a track, applied while mixing.
//!
//! Effects are non-destructive: the project only stores their settings
//! ([`TrackEffects`]), and the mixer runs them on the fly. Unlike volume or
//! fades, most effects *remember* earlier audio (a filter's last samples, a
//! compressor's current level). That memory is the [`EffectChain`]: one per
//! track, owned by the [`crate::mix::Mix`], which feeds it the track's audio in
//! order and resets it when playback jumps.

mod biquad;
pub mod compressor;
pub mod limiter;
pub mod low_cut;
pub mod noise_gate;

use serde::{Deserialize, Serialize};

use crate::convert::f32_from_f64;
use crate::time::SampleRate;
use compressor::{Compressor, CompressorFilter};
use low_cut::{LowCut, LowCutFilter};
use noise_gate::{NoiseGate, NoiseGateFilter};

/// The effect settings of one track, as saved in the project file.
///
/// Every effect is off by default, so tracks from older project files sound
/// as they did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TrackEffects {
    /// Removes rumble and handling noise below a frequency.
    pub low_cut: LowCut,
    /// Turns the track down while it is quiet (hiss between phrases).
    pub noise_gate: NoiseGate,
    /// Evens out the volume: loud parts down, then everything up.
    pub compressor: Compressor,
}

impl TrackEffects {
    /// Whether any effect is switched on (the track header then shows it).
    #[must_use]
    pub fn any_on(&self) -> bool {
        self.low_cut.enabled || self.noise_gate.enabled || self.compressor.enabled
    }
}

/// How much of the way to its target a smoothed value (a level, a gain) moves
/// per frame, for a time constant of `seconds`: after that time it has gone
/// about two thirds of the way. Used by the gate and the compressor.
fn smoothing(seconds: f64, sample_rate: SampleRate) -> f32 {
    let frames = seconds * f64::from(sample_rate.hz());
    f32_from_f64(1.0 - (-1.0 / frames).exp())
}

/// The running effects of one track: the settings turned into filters, plus
/// what they remember of the audio so far.
#[derive(Debug, Clone, Default)]
pub(crate) struct EffectChain {
    /// `None` while the low cut is off.
    low_cut: Option<LowCutFilter>,
    /// `None` while the noise gate is off.
    noise_gate: Option<NoiseGateFilter>,
    /// `None` while the compressor is off.
    compressor: Option<CompressorFilter>,
}

impl EffectChain {
    /// The effects that are switched on in `settings`, starting from silence.
    pub(crate) fn new(settings: TrackEffects, sample_rate: SampleRate) -> Self {
        Self {
            low_cut: settings
                .low_cut
                .enabled
                .then(|| LowCutFilter::new(settings.low_cut, sample_rate)),
            noise_gate: settings
                .noise_gate
                .enabled
                .then(|| NoiseGateFilter::new(settings.noise_gate, sample_rate)),
            compressor: settings
                .compressor
                .enabled
                .then(|| CompressorFilter::new(settings.compressor, sample_rate)),
        }
    }

    /// Whether no effect is switched on (so the mixer can skip warming up).
    pub(crate) fn is_empty(&self) -> bool {
        self.low_cut.is_none() && self.noise_gate.is_none() && self.compressor.is_none()
    }

    /// Forgets all earlier audio, as if playback started from silence.
    pub(crate) fn reset(&mut self) {
        if let Some(filter) = &mut self.low_cut {
            filter.reset();
        }
        if let Some(gate) = &mut self.noise_gate {
            gate.reset();
        }
        if let Some(compressor) = &mut self.compressor {
            compressor.reset();
        }
    }

    /// Continues where `old` left off: keeps what its effects remember, with
    /// this chain's settings. Used when an edit replaces the mix during
    /// playback, so moving a slider does not click.
    pub(crate) fn continue_from(&mut self, old: &Self) {
        if let (Some(filter), Some(old)) = (&mut self.low_cut, &old.low_cut) {
            filter.continue_from(old);
        }
        if let (Some(gate), Some(old)) = (&mut self.noise_gate, &old.noise_gate) {
            gate.continue_from(old);
        }
        if let (Some(compressor), Some(old)) = (&mut self.compressor, &old.compressor) {
            compressor.continue_from(old);
        }
    }

    /// Runs the effects over `samples` (interleaved stereo), in place: the
    /// low cut first, so rumble cannot hold the gate open; the compressor
    /// last, so it does not bring up the noise the gate took out.
    pub(crate) fn process(&mut self, samples: &mut [f32]) {
        if let Some(filter) = &mut self.low_cut {
            filter.process(samples);
        }
        if let Some(gate) = &mut self.noise_gate {
            gate.process(samples);
        }
        if let Some(compressor) = &mut self.compressor {
            compressor.process(samples);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::audio::{AudioBuffer, CHANNELS};
    use crate::convert::f32_from_f64;
    use crate::mix::{AudioPool, Mix};
    use crate::project::Project;
    use crate::time::{Frames, SampleRate};

    /// One second of a 440 Hz sine plus a constant offset of 0.5 (which a low
    /// cut removes), on one track with the low cut on.
    fn low_cut_project() -> (Project, AudioPool) {
        let mut project = Project::default();
        let track = project.import("tone.wav".into(), 48_000, "tone".to_owned(), 0);
        let source = project.track(track).unwrap().clips[0].source;
        project.tracks[0].effects.low_cut.enabled = true;
        let samples = (0..48_000)
            .flat_map(|n| {
                let t = f64::from(n) / 48_000.0;
                [f32_from_f64(0.5 + 0.25 * (std::f64::consts::TAU * 440.0 * t).sin()); CHANNELS]
            })
            .collect();
        let audio = AudioBuffer::new(SampleRate::HZ_48000, samples).unwrap();
        (project, AudioPool::from([(source, audio)]))
    }

    /// Renders `frames` frames from `start` in one go.
    fn render(mix: &mut Mix, start: Frames, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0; frames * CHANNELS];
        mix.render(start, &mut out);
        out
    }

    fn max_difference(a: &[f32], b: &[f32]) -> f32 {
        a.iter()
            .zip(b)
            .fold(0.0, |max, (a, b)| max.max((a - b).abs()))
    }

    #[test]
    fn effects_are_on_only_when_switched_on() {
        let mut effects = super::TrackEffects::default();
        assert!(!effects.any_on());
        effects.low_cut.enabled = true;
        assert!(effects.any_on());
        effects.low_cut.enabled = false;
        effects.noise_gate.enabled = true;
        assert!(effects.any_on());
        effects.noise_gate.enabled = false;
        effects.compressor.enabled = true;
        assert!(effects.any_on());
    }

    #[test]
    fn a_low_cut_on_a_track_removes_its_offset() {
        let (project, pool) = low_cut_project();
        let out = render(&mut Mix::new(&project, &pool), 0, 48_000);
        // After settling, the offset is gone: the sine swings around 0.
        let settled = out.get(24_000 * CHANNELS..).unwrap();
        let average = settled.iter().sum::<f32>() / f32_from_f64(24_000.0 * 2.0);
        assert!(average.abs() < 1e-3, "{average}");
    }

    #[test]
    fn rendering_in_pieces_is_the_same_as_in_one_go() {
        let (project, pool) = low_cut_project();
        let whole = render(&mut Mix::new(&project, &pool), 0, 48_000);
        let mut mix = Mix::new(&project, &pool);
        let pieces: Vec<f32> = (0..48)
            .flat_map(|piece| render(&mut mix, piece * 1000, 1000))
            .collect();
        assert_eq!(whole, pieces);
    }

    #[test]
    fn after_a_jump_the_effects_start_settled() {
        let (project, pool) = low_cut_project();
        let whole = render(&mut Mix::new(&project, &pool), 0, 48_000);
        // A fresh mix starting halfway, as after a seek, sounds as if it had
        // played all along (the warm-up), not like a filter starting cold.
        let jumped = render(&mut Mix::new(&project, &pool), 24_000, 1000);
        let expected = whole.get(24_000 * CHANNELS..25_000 * CHANNELS).unwrap();
        assert!(max_difference(&jumped, expected) < 1e-4);
    }

    #[test]
    fn a_new_mix_continues_the_effects_of_the_old_one() {
        let (project, pool) = low_cut_project();
        let whole = render(&mut Mix::new(&project, &pool), 0, 2000);
        let mut old = Mix::new(&project, &pool);
        render(&mut old, 0, 1000);
        // An edit (here: none) builds a new mix while playing.
        let mut new = Mix::new(&project, &pool);
        new.continue_from(old);
        let continued = render(&mut new, 1000, 1000);
        let expected = whole.get(1000 * CHANNELS..).unwrap();
        // The same project: the crossfade from old to new changes nothing
        // (but rounding).
        assert!(max_difference(&continued, expected) < 1e-6);
    }
}
