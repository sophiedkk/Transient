//! Mixing: adding all tracks together into one stereo signal.
//!
//! A [`Mix`] is a ready-to-play snapshot of a project: every clip with its audio.
//! The player asks it for any stretch of the timeline with [`Mix::render`].
//! After every edit the app builds a new `Mix` and hands it to the player, so
//! changes are heard right away. Building one is cheap: the audio is shared
//! (`Arc`), only the list of clips is copied.

use std::collections::HashMap;

use crate::audio::{AudioBuffer, CHANNELS};
use crate::edit::visible_parts;
use crate::project::{Project, SourceId};
use crate::range::TimeRange;
use crate::time::{Frames, SampleRate, frames_to_f64};

/// Decoded audio for each source, by ID.
pub type AudioPool = HashMap<SourceId, AudioBuffer>;

/// Volumes at or below this many decibels are silent (the bottom of the
/// volume sliders).
pub const SILENT_DB: f32 = -60.0;

/// One clip, ready to mix.
#[derive(Debug, Clone)]
struct MixClip {
    audio: AudioBuffer,
    start: Frames,
    offset: Frames,
    length: Frames,
    /// What to multiply the left and right channel by: the clip's gain, the
    /// track's volume and pan, and the master volume.
    gains: [f32; CHANNELS],
    fade_in: Frames,
    fade_out: Frames,
}

/// The shape of a fade: how loud (0 to 1) a fade is at `t`, the fraction of the
/// fade done (0 = silent end, 1 = full volume end).
///
/// A quarter sine: an *equal-power* curve. When one clip fades out while an
/// overlapping clip fades in, their combined power stays constant, so the
/// crossfade has no dip in loudness.
#[must_use]
pub fn fade_shape(t: f64) -> f64 {
    (t.clamp(0.0, 1.0) * std::f64::consts::FRAC_PI_2).sin()
}

/// The fade multiplier at `position` frames into a clip of `length` frames.
fn fade_gain(position: Frames, length: Frames, fade_in: Frames, fade_out: Frames) -> f32 {
    let mut gain = 1.0;
    if position < fade_in {
        gain *= fade_shape(frames_to_f64(position) / frames_to_f64(fade_in));
    }
    let remaining = length.saturating_sub(position);
    if remaining < fade_out {
        gain *= fade_shape(frames_to_f64(remaining) / frames_to_f64(fade_out));
    }
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        reason = "a gain between 0 and 1 fits an f32"
    )]
    let gain = gain as f32;
    gain
}

/// Decibels to a multiplier: 0 dB → 1.0, −6 dB → about 0.5, +6 dB → about 2.
#[must_use]
pub fn db_to_gain(db: f32) -> f32 {
    if db <= SILENT_DB {
        0.0
    } else {
        10_f32.powf(db / 20.0)
    }
}

/// Left and right multipliers for a balance of `pan` (−1 left … 1 right).
///
/// A *balance* law: in the centre both channels stay unchanged; panning right
/// turns the left channel down (and the other way around). That suits stereo
/// recordings, which already have their own left and right.
#[must_use]
pub fn pan_gains(pan: f32) -> [f32; CHANNELS] {
    let pan = pan.clamp(-1.0, 1.0);
    [(1.0 - pan).min(1.0), (1.0 + pan).min(1.0)]
}

/// A project's clips with their audio, ready to render.
#[derive(Debug, Clone)]
pub struct Mix {
    sample_rate: SampleRate,
    length: Frames,
    clips: Vec<MixClip>,
    /// Where frame 0 of this mix is on the timeline: 0, except for an excerpt
    /// (see [`Mix::excerpt`]).
    offset: Frames,
}

impl Mix {
    /// Collects every audible clip of `project` whose audio is in `pool`, with
    /// its track's mixer settings. Muted tracks, tracks silenced by another
    /// track's solo, and clips whose source is missing are left out.
    #[must_use]
    pub fn new(project: &Project, pool: &AudioPool) -> Self {
        let any_solo = project.tracks.iter().any(|track| track.solo);
        let master = db_to_gain(project.master_volume_db);
        let clips = project
            .tracks
            .iter()
            .filter(|track| !track.mute && (track.solo || !any_solo))
            .flat_map(|track| {
                let volume = db_to_gain(track.volume_db) * master;
                let [left, right] = pan_gains(track.pan);
                // Where clips on a track overlap, only the top one plays.
                visible_parts(&track.clips).into_iter().map(move |clip| {
                    let gain = volume * db_to_gain(clip.gain_db);
                    (clip, [left * gain, right * gain])
                })
            })
            .filter(|(_, gains)| gains.iter().any(|gain| *gain > 0.0))
            .filter_map(|(clip, gains)| {
                Some(MixClip {
                    audio: pool.get(&clip.source)?.clone(),
                    start: clip.start,
                    offset: clip.offset,
                    length: clip.length,
                    gains,
                    fade_in: clip.fade_in,
                    fade_out: clip.fade_out,
                })
            })
            .collect();
        Self {
            sample_rate: project.sample_rate,
            length: project.length(),
            clips,
            offset: 0,
        }
    }

    /// A mix with nothing in it.
    #[must_use]
    pub fn silent(sample_rate: SampleRate) -> Self {
        Self {
            sample_rate,
            length: 0,
            clips: Vec::new(),
            offset: 0,
        }
    }

    /// Only the stretch `range` of this mix (for exporting a selection): its
    /// frame 0 is `range.start`, and it ends at `range.end` (or where the mix
    /// ends, if that is earlier). Fades sound exactly as in the whole mix.
    #[must_use]
    pub fn excerpt(&self, range: TimeRange) -> Self {
        Self {
            length: range.end.min(self.length).saturating_sub(range.start),
            offset: self.offset + range.start,
            ..self.clone()
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
    /// Places without clips are silent; overlapping tracks are added together
    /// (on one track, only the visible parts of clips play).
    pub fn render(&self, start: Frames, out: &mut [f32]) {
        out.fill(0.0);
        // In timeline frames (an excerpt starts later on the timeline).
        let start = start.saturating_add(self.offset);
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
            let first_position = from - clip.start;
            let fades = clip.fade_in > 0 || clip.fade_out > 0;
            for ((out_frame, frame), position) in target
                .chunks_exact_mut(CHANNELS)
                .zip(samples.chunks_exact(CHANNELS))
                .zip(first_position..)
            {
                let fade = if fades {
                    fade_gain(position, clip.length, clip.fade_in, clip.fade_out)
                } else {
                    1.0
                };
                for ((out, sample), gain) in out_frame.iter_mut().zip(frame).zip(clip.gains) {
                    *out += sample * gain * fade;
                }
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
    fn an_excerpt_plays_its_stretch_of_the_mix_from_its_frame_0() {
        let mut project = Project::default();
        let track = project.import("ramp.wav".into(), 100, "r".to_owned());
        let source = project.track(track).unwrap().clips[0].source;
        let pool = AudioPool::from([(source, ramp(100))]);
        let mix = Mix::new(&project, &pool);
        let excerpt = mix.excerpt(TimeRange { start: 40, end: 60 });
        assert_eq!(excerpt.length(), 20);
        assert_eq!(render(&excerpt, 0, 20), render(&mix, 40, 20));
        // Past the end of the mix, an excerpt is cut short.
        assert_eq!(
            mix.excerpt(TimeRange {
                start: 90,
                end: 200
            })
            .length(),
            10
        );
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

    /// Two tracks of constant 0.5, for the mixer setting tests.
    fn two_tracks() -> (Project, AudioPool) {
        let mut project = Project::default();
        let a = project.import("a.wav".into(), 4, "a".to_owned());
        let b = project.import("b.wav".into(), 4, "b".to_owned());
        let pool = [a, b]
            .iter()
            .map(|track| {
                (
                    project.track(*track).unwrap().clips[0].source,
                    constant(0.5, 4),
                )
            })
            .collect();
        (project, pool)
    }

    /// The first frame of the mix, both channels.
    fn first_frame(project: &Project, pool: &AudioPool) -> [f32; 2] {
        let mut out = [0.0; 2];
        Mix::new(project, pool).render(0, &mut out);
        out
    }

    fn assert_close(actual: [f32; 2], expected: [f32; 2]) {
        for (a, e) in actual.iter().zip(expected) {
            assert!(
                (a - e).abs() < 1e-4,
                "expected {expected:?}, got {actual:?}"
            );
        }
    }

    #[test]
    fn decibels_become_multipliers() {
        assert!((db_to_gain(0.0) - 1.0).abs() < f32::EPSILON);
        assert!((db_to_gain(-6.0) - 0.501).abs() < 0.001);
        assert!((db_to_gain(6.0) - 1.995).abs() < 0.001);
        assert!(db_to_gain(SILENT_DB).abs() < f32::EPSILON);
    }

    #[test]
    fn volume_and_master_volume_scale_the_mix() {
        let (mut project, pool) = two_tracks();
        assert_close(first_frame(&project, &pool), [1.0, 1.0]);
        project.tracks[0].volume_db = SILENT_DB;
        assert_close(first_frame(&project, &pool), [0.5, 0.5]);
        project.master_volume_db = -6.0;
        assert_close(first_frame(&project, &pool), [0.2506, 0.2506]);
    }

    #[test]
    fn pan_moves_a_track_between_left_and_right() {
        let (mut project, pool) = two_tracks();
        project.tracks[0].pan = -1.0; // first track: left only
        project.tracks[1].pan = 0.5; // second: right, left at half
        assert_close(first_frame(&project, &pool), [0.5 + 0.25, 0.5]);
    }

    #[test]
    fn mute_silences_a_track() {
        let (mut project, pool) = two_tracks();
        project.tracks[1].mute = true;
        assert_close(first_frame(&project, &pool), [0.5, 0.5]);
    }

    #[test]
    fn solo_silences_every_other_track() {
        let (mut project, pool) = two_tracks();
        project.tracks[1].solo = true;
        assert_close(first_frame(&project, &pool), [0.5, 0.5]);
        // A muted soloed track stays silent.
        project.tracks[1].mute = true;
        assert_close(first_frame(&project, &pool), [0.0, 0.0]);
    }

    #[test]
    fn clip_gain_scales_the_clip() {
        let (mut project, pool) = two_tracks();
        project.tracks[1].clips[0].gain_db = SILENT_DB;
        assert_close(first_frame(&project, &pool), [0.5, 0.5]);
    }

    #[test]
    fn fades_start_and_end_in_silence() {
        let mut project = Project::default();
        let track = project.import("a.wav".into(), 100, "a".to_owned());
        let source = project.track(track).unwrap().clips[0].source;
        project.tracks[0].clips[0] = project.tracks[0].clips[0].with_fades(10, 20);
        let pool = AudioPool::from([(source, constant(1.0, 100))]);
        let mix = Mix::new(&project, &pool);
        let level = render(&mix, 0, 100);

        assert!(level[0].abs() < 1e-6, "silent at the very start");
        assert!(
            (level[5] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-3,
            "halfway through the fade-in"
        );
        assert!((level[50] - 1.0).abs() < 1e-6, "full volume in the middle");
        assert!(
            (level[90] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-3,
            "halfway through the fade-out"
        );
        assert!(level[99] < 0.08, "nearly silent at the end");
    }

    #[test]
    fn an_equal_power_crossfade_keeps_the_power_constant() {
        for step in 0..=10 {
            let t = f64::from(step) / 10.0;
            let fading_in = fade_shape(t);
            let fading_out = fade_shape(1.0 - t);
            let power = fading_in.powi(2) + fading_out.powi(2);
            assert!((power - 1.0).abs() < 1e-9, "at {t}: power {power}");
        }
    }

    #[test]
    fn a_source_without_audio_is_silent() {
        let mut project = Project::default();
        project.import("missing.wav".into(), 10, "m".to_owned());
        let mix = Mix::new(&project, &AudioPool::new());
        assert_eq!(render(&mix, 0, 3), [0.0, 0.0, 0.0]);
    }
}
