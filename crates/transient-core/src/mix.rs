//! Mixing: adding all clips together into one stereo signal.
//!
//! A [`Mix`] is a ready-to-play snapshot of a project: every clip with its audio.
//! The player asks it for any stretch of the timeline with [`Mix::render`].
//! After every edit the app builds a new `Mix` and hands it to the player, so
//! changes are heard right away. Building one is cheap: the audio is shared
//! (`Arc`), only the list of clips is copied.

use std::collections::HashMap;

use crate::audio::{AudioBuffer, CHANNELS};
use crate::project::{Project, SourceId};
use crate::time::{Frames, SampleRate};

/// Decoded audio for each source, by ID.
pub type AudioPool = HashMap<SourceId, AudioBuffer>;

/// One clip, ready to mix.
#[derive(Debug, Clone)]
struct MixClip {
    audio: AudioBuffer,
    start: Frames,
    offset: Frames,
    length: Frames,
}

/// A project's clips with their audio, ready to render.
#[derive(Debug, Clone)]
pub struct Mix {
    sample_rate: SampleRate,
    length: Frames,
    clips: Vec<MixClip>,
}

impl Mix {
    /// Collects every clip of `project` whose audio is in `pool`.
    /// Clips whose source is missing from the pool are left out (silent).
    #[must_use]
    pub fn new(project: &Project, pool: &AudioPool) -> Self {
        let clips = project
            .tracks
            .iter()
            .flat_map(|track| &track.clips)
            .filter_map(|clip| {
                Some(MixClip {
                    audio: pool.get(&clip.source)?.clone(),
                    start: clip.start,
                    offset: clip.offset,
                    length: clip.length,
                })
            })
            .collect();
        Self {
            sample_rate: project.sample_rate,
            length: project.length(),
            clips,
        }
    }

    /// A mix with nothing in it.
    #[must_use]
    pub fn silent(sample_rate: SampleRate) -> Self {
        Self {
            sample_rate,
            length: 0,
            clips: Vec::new(),
        }
    }

    /// The sample rate of the mix.
    #[must_use]
    pub fn sample_rate(&self) -> SampleRate {
        self.sample_rate
    }

    /// The length of the mix: up to the end of the last clip.
    #[must_use]
    pub fn length(&self) -> Frames {
        self.length
    }

    /// Fills `out` (interleaved stereo) with the mix from frame `start` on.
    /// Places without clips are silent; overlapping clips are added together.
    pub fn render(&self, start: Frames, out: &mut [f32]) {
        out.fill(0.0);
        let frames = Frames::try_from(out.len() / CHANNELS).unwrap_or(0);
        let end = start.saturating_add(frames);

        for clip in &self.clips {
            // The part of this clip that falls inside start..end, in timeline frames.
            let from = start.max(clip.start);
            let to = end.min(clip.start.saturating_add(clip.length));
            if from >= to {
                continue;
            }
            let (Ok(out_first), Ok(count)) =
                (usize::try_from(from - start), usize::try_from(to - from))
            else {
                continue;
            };
            let source_first = clip.offset.saturating_add(from - clip.start);
            let samples = clip.audio.frames_from(source_first, count);
            let Some(target) = out.get_mut(out_first * CHANNELS..) else {
                continue;
            };
            for (out_sample, sample) in target.iter_mut().zip(samples) {
                *out_sample += sample;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A buffer of `frames` frames, every sample `value`.
    fn constant(value: f32, frames: usize) -> AudioBuffer {
        AudioBuffer::new(SampleRate::HZ_48000, vec![value; frames * CHANNELS]).unwrap()
    }

    /// A buffer whose frame n holds the value n (both channels).
    fn ramp(frames: usize) -> AudioBuffer {
        let samples = (0..frames)
            .flat_map(|n| {
                let value = f32::from(u16::try_from(n).unwrap());
                [value, value]
            })
            .collect();
        AudioBuffer::new(SampleRate::HZ_48000, samples).unwrap()
    }

    fn render(mix: &Mix, start: Frames, frames: usize) -> Vec<f32> {
        let mut out = vec![f32::NAN; frames * CHANNELS];
        mix.render(start, &mut out);
        // Keep the left channel only, to make the expectations easy to read.
        out.iter().step_by(CHANNELS).copied().collect()
    }

    #[test]
    fn a_clip_plays_at_its_start_position() {
        let mut project = Project::default();
        let track = project.add_track("t".to_owned());
        let source = project.add_source("ramp.wav".into(), 4);
        project.add_clip(track, source, 2).unwrap();
        let pool = AudioPool::from([(source, ramp(4))]);

        let mix = Mix::new(&project, &pool);
        assert_eq!(mix.length(), 6);
        assert_eq!(render(&mix, 0, 8), [0.0, 0.0, 0.0, 1.0, 2.0, 3.0, 0.0, 0.0]);
    }

    #[test]
    fn offset_and_length_select_part_of_the_source() {
        let mut project = Project::default();
        let track = project.add_track("t".to_owned());
        let source = project.add_source("ramp.wav".into(), 10);
        let clip = project.add_clip(track, source, 0).unwrap();
        let clip = project.tracks[0]
            .clips
            .iter_mut()
            .find(|c| c.id == clip)
            .unwrap();
        clip.offset = 3;
        clip.length = 4;
        let pool = AudioPool::from([(source, ramp(10))]);

        let mix = Mix::new(&project, &pool);
        assert_eq!(render(&mix, 0, 6), [3.0, 4.0, 5.0, 6.0, 0.0, 0.0]);
    }

    #[test]
    fn overlapping_tracks_are_added_together() {
        let mut project = Project::default();
        let a = project.import("a.wav".into(), 4, "a".to_owned());
        let b = project.import("b.wav".into(), 4, "b".to_owned());
        let source_a = project.track(a).unwrap().clips[0].source;
        let source_b = project.track(b).unwrap().clips[0].source;
        let pool = AudioPool::from([(source_a, constant(0.25, 4)), (source_b, constant(0.5, 4))]);

        let mix = Mix::new(&project, &pool);
        assert_eq!(render(&mix, 2, 4), [0.75, 0.75, 0.0, 0.0]);
    }

    #[test]
    fn rendering_starts_anywhere_and_past_the_end_is_silent() {
        let mut project = Project::default();
        let track = project.import("ramp.wav".into(), 100, "r".to_owned());
        let source = project.track(track).unwrap().clips[0].source;
        let pool = AudioPool::from([(source, ramp(100))]);
        let mix = Mix::new(&project, &pool);

        assert_eq!(render(&mix, 97, 5), [97.0, 98.0, 99.0, 0.0, 0.0]);
        assert_eq!(render(&mix, 1000, 2), [0.0, 0.0]);
    }

    #[test]
    fn a_source_without_audio_is_silent() {
        let mut project = Project::default();
        project.import("missing.wav".into(), 10, "m".to_owned());
        let mix = Mix::new(&project, &AudioPool::new());
        assert_eq!(render(&mix, 0, 3), [0.0, 0.0, 0.0]);
    }
}
