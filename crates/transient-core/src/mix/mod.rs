//! Mixing: adding all tracks together into one stereo signal.
//!
//! A [`Mix`] is a ready-to-play snapshot of a project: every clip with its audio.
//! The player asks it for any stretch of the timeline with [`Mix::render`].
//! After every edit the app builds a new `Mix` and hands it to the player, so
//! changes are heard right away. Building one is cheap: the audio is shared
//! (`Arc`), only the list of clips is copied.
//!
//! Each track is rendered into its own buffer first, so its effects
//! ([`crate::effects`]) hear only that track; then the tracks are added up,
//! and the master limiter keeps the sum from clipping.

use std::collections::HashMap;

use crate::audio::{AudioBuffer, CHANNELS, amplitude_from_db};
use crate::clip::visible_parts;
use crate::convert::f32_from_f64;
use crate::effects::EffectChain;
use crate::effects::limiter::Limiter;
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
        amplitude_from_db(db)
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
    /// The master limiter, if the project has it on. Its audio comes out
    /// delayed, so the tracks are rendered that far ahead.
    limiter: Option<Limiter>,
    /// Room for the tracks' sum before it goes through the limiter.
    sum: Vec<f32>,
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
            limiter: project
                .master_limiter
                .then(|| Limiter::new(project.sample_rate)),
            sum: Vec::new(),
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
            limiter: None,
            sum: Vec::new(),
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
            // A fresh start: the first render warms everything up.
            next_frame: None,
            crossfade: None,
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
        // The limiter's delayed audio is the old mix's: hand it over, so the
        // limited sum continues without a gap.
        if let (Some(limiter), Some(old_limiter)) = (&mut self.limiter, &old.limiter) {
            *limiter = old_limiter.clone();
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
        self.add_tracks(start, out);
        if let Some(crossfade) = &mut self.crossfade
            && !crossfade.apply(mix_start, out)
        {
            self.crossfade = None;
        }
        let frames = Frames::try_from(out.len() / CHANNELS).unwrap_or(0);
        self.next_frame = Some(start.saturating_add(frames));
    }

    /// Adds the tracks from timeline frame `start` on to `out`, through the
    /// limiter if it is on. The limiter delays its audio, so the tracks are
    /// rendered that much ahead; after a jump (or when the limiter was just
    /// switched on) it is first filled with the audio from `start` on.
    fn add_tracks(&mut self, start: Frames, out: &mut [f32]) {
        let Some(limiter) = &mut self.limiter else {
            for track in &mut self.tracks {
                track.add_to(start, out);
            }
            return;
        };
        let delay = limiter.delay_frames();
        let ahead = start.saturating_add(Frames::try_from(delay).unwrap_or(0));
        if limiter.next_input != Some(ahead) {
            limiter.reset();
            let mut first = vec![0.0; delay * CHANNELS];
            for track in &mut self.tracks {
                track.add_to(start, &mut first);
            }
            let mut silence = vec![0.0; first.len()];
            limiter.process(&first, &mut silence);
        }
        self.sum.clear();
        self.sum.resize(out.len(), 0.0);
        for track in &mut self.tracks {
            track.add_to(ahead, &mut self.sum);
        }
        let mut from_limiter = vec![0.0; out.len()];
        limiter.process(&self.sum, &mut from_limiter);
        for (out, sample) in out.iter_mut().zip(from_limiter) {
            *out += sample;
        }
        let frames = Frames::try_from(out.len() / CHANNELS).unwrap_or(0);
        limiter.next_input = Some(ahead.saturating_add(frames));
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
mod tests;
