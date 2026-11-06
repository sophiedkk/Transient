//! Mixing: adding all tracks together into one stereo signal.
//!
//! A [`Mix`] is a ready-to-play snapshot of a project: every clip with its audio.
//! The player asks it for any stretch of the timeline with [`Mix::render`].
//! After every edit the app builds a new `Mix` and hands it to the player, so
//! changes are heard right away. Building one is cheap: the audio is shared
//! (`Arc`), only the list of clips is copied.
//!
//! Each track is rendered into its own buffer first, so its effects
//! ([`crate::effects`]) hear only that track; then the tracks are added up.

use std::collections::HashMap;

use crate::audio::{AudioBuffer, CHANNELS};
use crate::clip::visible_parts;
use crate::convert::f32_from_f64;
use crate::effects::EffectChain;
use crate::project::{Project, SourceId, TrackId};
use crate::range::TimeRange;
use crate::time::{Frames, SampleRate, frames_to_f64};

/// Decoded audio for each source, by ID.
pub type AudioPool = HashMap<SourceId, AudioBuffer>;

/// Volumes at or below this many decibels are silent (the bottom of the
/// volume sliders).
pub const SILENT_DB: f32 = -60.0;

/// How much audio before the playing position effects hear when playback
/// jumps (a seek, the start of an export of a selection), in seconds. A filter
/// settles within milliseconds; compressors and gates need a little longer.
const WARM_UP_SECONDS: f64 = 0.25;

/// How long the switch from an old mix to a new one takes during playback, in
/// seconds (see [`Mix::continue_from`]): short enough to sound immediate, long
/// enough not to click.
const CROSSFADE_SECONDS: f64 = 0.02;

/// One clip, ready to mix.
#[derive(Debug, Clone)]
struct MixClip {
    audio: AudioBuffer,
    start: Frames,
    offset: Frames,
    length: Frames,
    /// The clip's gain, as a multiplier.
    gain: f32,
    fade_in: Frames,
    fade_out: Frames,
}

impl MixClip {
    /// Adds the part of this clip that falls in the `out.len() / CHANNELS`
    /// frames from timeline frame `start` on to `out`, with gain and fades.
    fn add_to(&self, start: Frames, out: &mut [f32]) {
        let frames = Frames::try_from(out.len() / CHANNELS).unwrap_or(0);
        let end = start.saturating_add(frames);
        // The part of this clip that falls inside start..end, in timeline frames.
        let from = start.max(self.start);
        let to = end.min(self.start.saturating_add(self.length));
        if from >= to {
            return;
        }
        let (Ok(out_first), Ok(count)) =
            (usize::try_from(from - start), usize::try_from(to - from))
        else {
            return;
        };
        let source_first = self.offset.saturating_add(from - self.start);
        let samples = self.audio.frames_from(source_first, count);
        let Some(target) = out.get_mut(out_first * CHANNELS..) else {
            return;
        };
        let first_position = from - self.start;
        let fades = self.fade_in > 0 || self.fade_out > 0;
        for ((out_frame, frame), position) in target
            .chunks_exact_mut(CHANNELS)
            .zip(samples.chunks_exact(CHANNELS))
            .zip(first_position..)
        {
            let fade = if fades {
                fade_gain(position, self.length, self.fade_in, self.fade_out)
            } else {
                1.0
            };
            let gain = self.gain * fade;
            for (out, sample) in out_frame.iter_mut().zip(frame) {
                *out += sample * gain;
            }
        }
    }
}

/// One audible track, ready to mix.
#[derive(Debug, Clone)]
struct MixTrack {
    id: TrackId,
    /// Only the visible parts of its clips (see [`visible_parts`]).
    clips: Vec<MixClip>,
    effects: EffectChain,
    /// What to multiply the left and right channel by after the effects: the
    /// track's volume and pan, and the master volume.
    gains: [f32; CHANNELS],
    /// Room to render the track into before it is added to the mix; kept
    /// between calls so rendering does not allocate every time.
    buffer: Vec<f32>,
}

impl MixTrack {
    /// Renders this track from timeline frame `start` on and adds it to `out`.
    fn add_to(&mut self, start: Frames, out: &mut [f32]) {
        self.buffer.clear();
        self.buffer.resize(out.len(), 0.0);
        for clip in &self.clips {
            clip.add_to(start, &mut self.buffer);
        }
        self.effects.process(&mut self.buffer);
        for (out_frame, frame) in out
            .chunks_exact_mut(CHANNELS)
            .zip(self.buffer.chunks_exact(CHANNELS))
        {
            for ((out, sample), gain) in out_frame.iter_mut().zip(frame).zip(self.gains) {
                *out += sample * gain;
            }
        }
    }
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
    f32_from_f64(gain)
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

/// A project's tracks with their audio and effects, ready to render.
///
/// Rendering changes it (`&mut self`): the effects remember the audio they
/// heard, so a mix plays best from start to end, one stretch after another.
#[derive(Debug, Clone)]
pub struct Mix {
    sample_rate: SampleRate,
    length: Frames,
    tracks: Vec<MixTrack>,
    /// Where frame 0 of this mix is on the timeline: 0, except for an excerpt
    /// (see [`Mix::excerpt`]).
    offset: Frames,
    /// The timeline frame right after the last render, if there was one. A
    /// render that starts anywhere else is a jump: the effects start over.
    next_frame: Option<Frames>,
    /// The mix this one replaced during playback, still fading out.
    crossfade: Option<Crossfade>,
}

/// The switch from an old mix to a new one: for a moment both play, the old
/// one turning down while the new one turns up.
///
/// Without it, a change during playback (a clip's gain, a fade, a track's
/// volume) would jump from one sample to the next, which is heard as a click.
/// Both mixes play the same audio, so a straight-line crossfade keeps the
/// volume steady.
#[derive(Debug, Clone)]
struct Crossfade {
    /// The old mix (boxed: a `Mix` cannot contain a `Mix` directly).
    from: Box<Mix>,
    /// The crossfade's length in frames, and how much of it has been played.
    length: Frames,
    done: Frames,
    /// Room to render the old mix into; kept between calls.
    buffer: Vec<f32>,
}

impl Crossfade {
    /// Mixes the old mix into `out` (the new mix from `start` on, in the mixes'
    /// own frames). Returns whether the crossfade is still going.
    fn apply(&mut self, start: Frames, out: &mut [f32]) -> bool {
        self.buffer.clear();
        self.buffer.resize(out.len(), 0.0);
        self.from.render(start, &mut self.buffer);
        let length = frames_to_f64(self.length);
        for (out_frame, old_frame) in out
            .chunks_exact_mut(CHANNELS)
            .zip(self.buffer.chunks_exact(CHANNELS))
        {
            if self.done >= self.length {
                break;
            }
            // How far the crossfade is: 0 = only the old mix, 1 = only the new.
            let new = f32_from_f64(frames_to_f64(self.done) / length);
            for (out, old) in out_frame.iter_mut().zip(old_frame) {
                *out = *old * (1.0 - new) + *out * new;
            }
            self.done += 1;
        }
        self.done < self.length
    }
}

impl Mix {
    /// Collects every audible track of `project`, with the clips whose audio is
    /// in `pool`, its effects and its mixer settings. Muted tracks, tracks
    /// silenced by another track's solo or volume, and clips whose source is
    /// missing are left out.
    #[must_use]
    pub fn new(project: &Project, pool: &AudioPool) -> Self {
        let any_solo = project.tracks.iter().any(|track| track.solo);
        let master = db_to_gain(project.master_volume_db);
        let tracks = project
            .tracks
            .iter()
            .filter(|track| !track.mute && (track.solo || !any_solo))
            .filter_map(|track| {
                let volume = db_to_gain(track.volume_db) * master;
                let gains = pan_gains(track.pan).map(|pan| pan * volume);
                if gains.iter().all(|gain| *gain <= 0.0) {
                    return None;
                }
                // Where clips on a track overlap, only the top one plays.
                let clips = visible_parts(&track.clips)
                    .into_iter()
                    .filter_map(|clip| {
                        Some(MixClip {
                            audio: pool.get(&clip.source)?.clone(),
                            start: clip.start,
                            offset: clip.offset,
                            length: clip.length,
                            gain: db_to_gain(clip.gain_db),
                            fade_in: clip.fade_in,
                            fade_out: clip.fade_out,
                        })
                    })
                    .filter(|clip| clip.gain > 0.0)
                    .collect();
                Some(MixTrack {
                    id: track.id,
                    clips,
                    effects: EffectChain::new(track.effects, project.sample_rate),
                    gains,
                    buffer: Vec::new(),
                })
            })
            .collect();
        Self {
            sample_rate: project.sample_rate,
            length: project.length(),
            tracks,
            offset: 0,
            next_frame: None,
            crossfade: None,
        }
    }

    /// A mix with nothing in it.
    #[must_use]
    pub fn silent(sample_rate: SampleRate) -> Self {
        Self {
            sample_rate,
            length: 0,
            tracks: Vec::new(),
            offset: 0,
            next_frame: None,
            crossfade: None,
        }
    }

    /// Only the stretch `range` of this mix (for exporting a selection): its
    /// frame 0 is `range.start`, and it ends at `range.end` (or where the mix
    /// ends, if that is earlier). Fades and effects sound exactly as in the
    /// whole mix.
    #[must_use]
    pub fn excerpt(&self, range: TimeRange) -> Self {
        Self {
            length: range.end.min(self.length).saturating_sub(range.start),
            offset: self.offset + range.start,
            ..self.clone()
        }
    }

    /// Takes over from `old` during playback (after an edit) without a click:
    /// the effects keep what they remember of `old`'s audio (for tracks in both
    /// mixes), and for the first moment (20 ms) `old` fades out while this mix
    /// fades in. A jump elsewhere ends the crossfade early.
    pub fn continue_from(&mut self, mut old: Self) {
        let Some(next_frame) = old.next_frame.filter(|_| self.offset == old.offset) else {
            return;
        };
        for track in &mut self.tracks {
            if let Some(old_track) = old.tracks.iter().find(|old| old.id == track.id) {
                track.effects.continue_from(&old_track.effects);
            }
        }
        self.next_frame = Some(next_frame);
        // While a slider is dragged, a new mix arrives before the last
        // crossfade ended: fade from where `old` is now, not from its own old
        // mix too, so crossfades never pile up.
        old.crossfade = None;
        self.crossfade = Some(Crossfade {
            from: Box::new(old),
            length: self
                .sample_rate
                .seconds_to_frames(CROSSFADE_SECONDS)
                .unwrap_or(0),
            done: 0,
            buffer: Vec::new(),
        });
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
    ///
    /// Rendering the stretch right after the previous one continues the
    /// effects; rendering anywhere else first lets them hear a moment of the
    /// audio before `start`, so they start settled.
    pub fn render(&mut self, start: Frames, out: &mut [f32]) {
        out.fill(0.0);
        // In timeline frames (an excerpt starts later on the timeline).
        let mix_start = start;
        let start = start.saturating_add(self.offset);
        if self.next_frame != Some(start) {
            // A jump: nothing to crossfade from, and the effects start over.
            self.crossfade = None;
            self.warm_up(start);
        }
        for track in &mut self.tracks {
            track.add_to(start, out);
        }
        if let Some(crossfade) = &mut self.crossfade
            && !crossfade.apply(mix_start, out)
        {
            self.crossfade = None;
        }
        let frames = Frames::try_from(out.len() / CHANNELS).unwrap_or(0);
        self.next_frame = Some(start.saturating_add(frames));
    }

    /// Starts the effects over, and plays them the audio just before timeline
    /// frame `start` (the result is not heard).
    fn warm_up(&mut self, start: Frames) {
        let warm_up = self
            .sample_rate
            .seconds_to_frames(WARM_UP_SECONDS)
            .unwrap_or(0);
        let from = start.saturating_sub(warm_up);
        let samples = usize::try_from(start - from).unwrap_or(0) * CHANNELS;
        let mut discarded = Vec::new();
        for track in self
            .tracks
            .iter_mut()
            .filter(|track| !track.effects.is_empty())
        {
            track.effects.reset();
            discarded.clear();
            discarded.resize(samples, 0.0);
            track.add_to(from, &mut discarded);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::Clip;

    /// A project with one track playing a 100-frame [`ramp`] from frame 0, and
    /// its audio.
    fn ramp_project() -> (Project, AudioPool) {
        let mut project = Project::default();
        let track = project.import("ramp.wav".into(), 100, "r".to_owned(), 0);
        let source = project.track(track).unwrap().clips[0].source;
        (project, AudioPool::from([(source, ramp(100))]))
    }

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

    /// Renders a copy of `mix`, so each call is a fresh start.
    fn render(mix: &Mix, start: Frames, frames: usize) -> Vec<f32> {
        let mut out = vec![f32::NAN; frames * CHANNELS];
        mix.clone().render(start, &mut out);
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
        let a = project.import("a.wav".into(), 4, "a".to_owned(), 0);
        let b = project.import("b.wav".into(), 4, "b".to_owned(), 0);
        let source_a = project.track(a).unwrap().clips[0].source;
        let source_b = project.track(b).unwrap().clips[0].source;
        let pool = AudioPool::from([(source_a, constant(0.25, 4)), (source_b, constant(0.5, 4))]);

        let mix = Mix::new(&project, &pool);
        assert_eq!(render(&mix, 2, 4), [0.75, 0.75, 0.0, 0.0]);
    }

    #[test]
    fn an_excerpt_plays_its_stretch_of_the_mix_from_its_frame_0() {
        let (project, pool) = ramp_project();
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
        let (project, pool) = ramp_project();
        let mix = Mix::new(&project, &pool);

        assert_eq!(render(&mix, 97, 5), [97.0, 98.0, 99.0, 0.0, 0.0]);
        assert_eq!(render(&mix, 1000, 2), [0.0, 0.0]);
    }

    /// Two tracks of constant 0.5, for the mixer setting tests.
    fn two_tracks() -> (Project, AudioPool) {
        let mut project = Project::default();
        let a = project.import("a.wav".into(), 4, "a".to_owned(), 0);
        let b = project.import("b.wav".into(), 4, "b".to_owned(), 0);
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
        let track = project.import("a.wav".into(), 100, "a".to_owned(), 0);
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
        project.import("missing.wav".into(), 10, "m".to_owned(), 0);
        let mix = Mix::new(&project, &AudioPool::new());
        assert_eq!(render(&mix, 0, 3), [0.0, 0.0, 0.0]);
    }

    /// One second of constant 0.5 on one track, with the clip at `gain_db`.
    fn constant_project(gain_db: f32) -> (Project, AudioPool) {
        let mut project = Project::default();
        let track = project.import("c.wav".into(), 48_000, "c".to_owned(), 0);
        let clip = project.track(track).unwrap().clips[0];
        project.change_clip(clip.id, |clip| Clip { gain_db, ..clip });
        let pool = AudioPool::from([(clip.source, constant(0.5, 48_000))]);
        (project, pool)
    }

    /// Plays 1000 frames at 0 dB, then switches to a mix at `gain_db` as an
    /// edit during playback does, and plays 2000 more (left channel).
    fn switch_during_playback(gain_db: f32) -> Vec<f32> {
        let (project, pool) = constant_project(0.0);
        let mut old = Mix::new(&project, &pool);
        let mut out = vec![0.0; 1000 * CHANNELS];
        old.render(0, &mut out);
        let (project, pool) = constant_project(gain_db);
        let mut new = Mix::new(&project, &pool);
        new.continue_from(old);
        let mut out = vec![0.0; 2000 * CHANNELS];
        new.render(1000, &mut out);
        out.into_iter().step_by(CHANNELS).collect()
    }

    #[test]
    fn a_change_during_playback_fades_over_instead_of_jumping() {
        let after = switch_during_playback(-12.0);
        let target = 0.5 * db_to_gain(-12.0);
        // It starts where the old mix was, and ends at the new level after
        // 20 ms (960 frames at 48 kHz)...
        assert!((after[0] - 0.5).abs() < 1e-3, "{}", after[0]);
        assert!((after[960] - target).abs() < 1e-6, "{}", after[960]);
        assert!((after[1999] - target).abs() < 1e-6);
        // ...in steps far too small to click.
        let largest_step = after
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).abs())
            .fold(0.0, f32::max);
        assert!(largest_step < 0.001, "{largest_step}");
    }

    #[test]
    fn a_crossfade_spans_several_renders() {
        let (project, pool) = constant_project(0.0);
        let mut old = Mix::new(&project, &pool);
        old.render(0, &mut [0.0; 2 * CHANNELS]);
        let (project, pool) = constant_project(SILENT_DB);
        let mut new = Mix::new(&project, &pool);
        new.continue_from(old);
        // Small pieces, as a player with a small buffer asks for them.
        let mut pieces = Vec::new();
        for start in (2..1002).step_by(100) {
            let mut out = [0.0; 100 * CHANNELS];
            new.render(start, &mut out);
            pieces.extend(out.into_iter().step_by(CHANNELS));
        }
        assert!(pieces.windows(2).all(|pair| pair[1] <= pair[0]));
        assert!(pieces[959].abs() < 0.001 && pieces[999] == 0.0);
    }

    #[test]
    fn a_jump_ends_the_crossfade() {
        let (project, pool) = constant_project(0.0);
        let mut old = Mix::new(&project, &pool);
        old.render(0, &mut [0.0; 1000 * CHANNELS]);
        let (project, pool) = constant_project(-12.0);
        let mut new = Mix::new(&project, &pool);
        new.continue_from(old);
        // A seek right after the edit plays only the new mix.
        let mut out = [0.0; 2 * CHANNELS];
        new.render(30_000, &mut out);
        assert!((out[0] - 0.5 * db_to_gain(-12.0)).abs() < 1e-6);
    }

    #[test]
    fn a_mix_that_never_played_has_nothing_to_fade_from() {
        let (project, pool) = constant_project(0.0);
        let old = Mix::new(&project, &pool);
        let (project, pool) = constant_project(-12.0);
        let mut new = Mix::new(&project, &pool);
        new.continue_from(old);
        assert!((first_frame_of(&mut new) - 0.5 * db_to_gain(-12.0)).abs() < 1e-6);
    }

    fn first_frame_of(mix: &mut Mix) -> f32 {
        let mut out = [0.0; CHANNELS];
        mix.render(0, &mut out);
        out[0]
    }
}
