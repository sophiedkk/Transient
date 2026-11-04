//! Editing operations: moving, trimming, splitting and deleting clips, snapping,
//! and the undo history.
//!
//! Clips on one track are stacked like sheets of paper, as in Ardour: the order
//! of `Track::clips` is bottom to top, and a clip that is moved goes on top
//! ([`Project::place_clip`]). Where clips overlap you hear (and see) only the top
//! one; nothing is cut, so moving the top clip away brings the one underneath
//! back whole. [`visible_parts`] works out what remains of each clip. Where the
//! top clip fades in or out, the clip underneath keeps playing and fades the
//! other way, so the two crossfade.
//!
//! The clip operations are pure functions (a clip in, a changed clip out), so the
//! timeline can use them to preview a drag before the project changes.

use crate::project::{Clip, ClipId, Project, Track, TrackId};
use crate::time::Frames;

/// The shortest a clip can be trimmed to (about 1.3 ms at 48 kHz).
pub const MIN_CLIP_LENGTH: Frames = 64;

/// How many steps the undo history keeps.
const HISTORY_LIMIT: usize = 100;

impl Clip {
    /// The clip moved so it starts at `start`.
    #[must_use]
    pub fn moved_to(self, start: Frames) -> Self {
        Self { start, ..self }
    }

    /// The clip with its left edge dragged to `new_start`. The audio stays where it
    /// is on the timeline: dragging right hides the beginning, dragging left
    /// reveals more of the source (but not past its start).
    #[must_use]
    pub fn trimmed_start(self, new_start: Frames) -> Self {
        let earliest = self.start.saturating_sub(self.offset);
        let latest = self.end().saturating_sub(MIN_CLIP_LENGTH).max(earliest);
        let new_start = new_start.clamp(earliest, latest);
        let offset = (self.offset + new_start).saturating_sub(self.start);
        Self {
            start: new_start,
            offset,
            length: self.end() - new_start,
            ..self
        }
        .with_fades(self.fade_in, self.fade_out)
    }

    /// The clip with its right edge dragged to `new_end`, but not past the end of
    /// the source (`source_frames` long).
    #[must_use]
    pub fn trimmed_end(self, new_end: Frames, source_frames: Frames) -> Self {
        let longest = source_frames.saturating_sub(self.offset);
        let length = new_end
            .saturating_sub(self.start)
            .clamp(MIN_CLIP_LENGTH.min(longest), longest);
        Self { length, ..self }.with_fades(self.fade_in, self.fade_out)
    }

    /// The clip with these fade lengths, shortened if needed so they fit: the
    /// fade-in at most the whole clip, the fade-out at most what is left.
    #[must_use]
    pub fn with_fades(self, fade_in: Frames, fade_out: Frames) -> Self {
        let fade_in = fade_in.min(self.length);
        let fade_out = fade_out.min(self.length - fade_in);
        Self {
            fade_in,
            fade_out,
            ..self
        }
    }

    /// Splits the clip at timeline frame `at` into two clips that play exactly the
    /// same audio. The second half gets `new_id`. `None` if `at` is not inside
    /// the clip (or would leave a piece shorter than [`MIN_CLIP_LENGTH`]).
    #[must_use]
    pub fn split_at(self, at: Frames, new_id: ClipId) -> Option<(Self, Self)> {
        if at < self.start + MIN_CLIP_LENGTH || at + MIN_CLIP_LENGTH > self.end() {
            return None;
        }
        let first_length = at - self.start;
        // The fade-in stays with the first half, the fade-out with the second.
        let first = Self {
            length: first_length,
            ..self
        }
        .with_fades(self.fade_in, 0);
        let second = Self {
            id: new_id,
            start: at,
            offset: self.offset + first_length,
            length: self.length - first_length,
            ..self
        }
        .with_fades(0, self.fade_out);
        Some((first, second))
    }

    /// What remains visible of `self` (a clip underneath) with `top` over it on
    /// the same track: the piece before `top` and the piece after it. Either can
    /// be `None`; both are `None` when `top` covers all of `self`. If both are
    /// left (`top` lies in the middle), the second piece gets `new_id`.
    ///
    /// `self` keeps playing only while `top` fades in or out, with a matching
    /// fade of its own, so they crossfade. Where `top` plays at full volume,
    /// nothing of `self` is left.
    #[must_use]
    pub fn covered_by(self, top: &Self, new_id: ClipId) -> (Option<Self>, Option<Self>) {
        if self.end() <= top.start || self.start >= top.end() {
            return (Some(self), None);
        }
        // `self` may only play where `top` is fading: up to the end of its fade-in,
        // and again from the start of its fade-out.
        let cut_start = top.start + top.fade_in;
        let cut_end = top.end() - top.fade_out;

        let before = (self.start < top.start).then(|| {
            let end = self.end().min(cut_start);
            let length = end - self.start;
            // Fade out for as long as `top` fades in (keep a longer fade-out of
            // its own if `self` already ended inside that fade).
            let overlap = end - top.start;
            let fade_out = if end < self.end() {
                overlap
            } else {
                self.fade_out.max(overlap)
            };
            Self { length, ..self }.with_fades(self.fade_in, fade_out)
        });
        let after = (self.end() > top.end()).then(|| {
            let start = self.start.max(cut_end);
            let skipped = start - self.start;
            let length = self.end() - start;
            let overlap = top.end() - start;
            let fade_in = if start > self.start {
                overlap
            } else {
                self.fade_in.max(overlap)
            };
            // The fade-out at the far end is kept as it is (so the audio after
            // `top` sounds exactly as before); the new fade-in gets what is left.
            let fade_out = self.fade_out.min(length);
            Self {
                id: if before.is_some() { new_id } else { self.id },
                start,
                offset: self.offset + skipped,
                length,
                ..self
            }
            .with_fades(fade_in.min(length - fade_out), fade_out)
        });
        // Both pieces are at least one frame long: `before` ends after `top`
        // starts, `after` starts before `top` ends. Short pieces are kept: they
        // are still audio that you hear.
        (before, after)
    }
}

impl Project {
    /// The clip with this ID, and the track it is on.
    #[must_use]
    pub fn clip(&self, id: ClipId) -> Option<(TrackId, &Clip)> {
        self.tracks.iter().find_map(|track| {
            let clip = track.clips.iter().find(|clip| clip.id == id)?;
            Some((track.id, clip))
        })
    }

    /// Replaces the clip with the same ID by `clip`, on `track`. On the same
    /// track it keeps its place in the stack (changing a clip's gain must not
    /// change which clip you hear); moved to another track, it goes on top.
    /// Returns `false` if the clip or track does not exist.
    pub fn update_clip(&mut self, clip: Clip, track: TrackId) -> bool {
        if let Some(slot) = self
            .track_mut(track)
            .and_then(|track| track.clips.iter_mut().find(|c| c.id == clip.id))
        {
            *slot = clip;
            return true;
        }
        self.place_clip(clip, track)
    }

    /// Puts `clip` on `track`, on top of the other clips there (where they
    /// overlap, it is the one you hear). Used when a clip is moved. Nothing
    /// underneath changes. Returns `false` if the clip or track does not exist.
    pub fn place_clip(&mut self, clip: Clip, track: TrackId) -> bool {
        if self.track(track).is_none() || self.remove_clip(clip.id).is_none() {
            return false;
        }
        self.track_mut(track)
            .map(|track| track.clips.push(clip))
            .is_some()
    }

    /// Moves `clip` to `track` by dragging: it goes on top there (see
    /// [`Project::place_clip`]). Tracks come from imported files, so a track
    /// whose last clip was dragged to another track has no use left and is
    /// removed too. Returns `false` if the clip or track does not exist.
    pub fn move_clip(&mut self, clip: Clip, track: TrackId) -> bool {
        let from = self.clip(clip.id).map(|(from, _)| from);
        if !self.place_clip(clip, track) {
            return false;
        }
        if let Some(from) = from.filter(|&from| from != track)
            && self.track(from).is_some_and(|from| from.clips.is_empty())
        {
            let _removed = self.remove_track(from);
        }
        true
    }

    /// Moves `clip` to a new track at the bottom, called `name` (the other way
    /// round from dragging a clip onto another track). Like [`Project::move_clip`],
    /// the track it leaves is removed if it is left empty. Returns the new track,
    /// or `None` if the clip does not exist.
    pub fn move_clip_to_new_track(&mut self, clip: Clip, name: String) -> Option<TrackId> {
        self.clip(clip.id)?;
        let track = self.add_track(name);
        self.move_clip(clip, track).then_some(track)
    }

    /// Renames a track. Leading and trailing spaces are dropped; an empty name is
    /// refused. Returns whether the name changed.
    pub fn rename_track(&mut self, id: TrackId, name: &str) -> bool {
        let name = name.trim();
        match self.track_mut(id) {
            Some(track) if !name.is_empty() && track.name != name => {
                name.clone_into(&mut track.name);
                true
            }
            _ => false,
        }
    }

    /// Moves a track one place up (towards the top, `up`) or down. Returns
    /// whether it moved (not past the first or last place).
    pub fn move_track(&mut self, id: TrackId, up: bool) -> bool {
        let Some(index) = self.tracks.iter().position(|track| track.id == id) else {
            return false;
        };
        let other = if up {
            index.checked_sub(1)
        } else {
            Some(index + 1).filter(|&other| other < self.tracks.len())
        };
        let Some(other) = other else {
            return false;
        };
        self.tracks.swap(index, other);
        true
    }

    /// Removes a track and its clips, returning it. The sources stay, so undo
    /// (a snapshot) and other clips of the same file keep working.
    pub fn remove_track(&mut self, id: TrackId) -> Option<Track> {
        let index = self.tracks.iter().position(|track| track.id == id)?;
        Some(self.tracks.remove(index))
    }

    /// Removes a clip, returning it and the track it was on.
    pub fn remove_clip(&mut self, id: ClipId) -> Option<(TrackId, Clip)> {
        self.tracks.iter_mut().find_map(|track| {
            let index = track.clips.iter().position(|clip| clip.id == id)?;
            Some((track.id, track.clips.remove(index)))
        })
    }

    /// Splits a clip at timeline frame `at`. Returns the ID of the new (second) clip.
    pub fn split_clip(&mut self, id: ClipId, at: Frames) -> Option<ClipId> {
        let (track, clip) = self.clip(id).map(|(track, clip)| (track, *clip))?;
        let new_id = ClipId(self.next_id);
        let (first, second) = clip.split_at(at, new_id)?;
        self.next_id += 1;
        let clips = &mut self.track_mut(track)?.clips;
        // The second half goes right above the first, so the stack (which clip
        // you hear where clips overlap) stays the same.
        let index = clips.iter().position(|c| c.id == id)?;
        if let Some(slot) = clips.get_mut(index) {
            *slot = first;
        }
        clips.insert(index + 1, second);
        Some(new_id)
    }

    /// Every clip playing at `frame`: from its first frame up to (not including)
    /// its end, so after a split the playhead belongs to the second half.
    /// Track by track, and within a track the topmost clip (drawn last) first.
    #[must_use]
    pub fn clips_at(&self, frame: Frames) -> Vec<ClipId> {
        self.tracks
            .iter()
            .flat_map(|track| track.clips.iter().rev())
            .filter(|clip| clip.start <= frame && frame < clip.end())
            .map(|clip| clip.id)
            .collect()
    }
}

/// The parts of a track's `clips` (stored bottom to top) that are heard and
/// drawn. Each clip is hidden where a clip above it covers it, except while the
/// clip above fades in or out: there both play, crossfading (see
/// [`Clip::covered_by`]). Parts keep their clip's ID, so one clip can have
/// several parts, or none when it is covered completely. The parts are in
/// stacking order too: later ones are on top.
#[must_use]
pub fn visible_parts(clips: &[Clip]) -> Vec<Clip> {
    let mut parts: Vec<Clip> = Vec::with_capacity(clips.len());
    for clip in clips {
        parts = parts
            .into_iter()
            .flat_map(|part| {
                let (before, after) = part.covered_by(clip, part.id);
                before.into_iter().chain(after)
            })
            .collect();
        parts.push(*clip);
    }
    parts
}

/// The point in `points` nearest to `frame` if it is within `tolerance`,
/// otherwise `frame` itself.
#[must_use]
pub fn snap(frame: Frames, points: &[Frames], tolerance: Frames) -> Frames {
    points
        .iter()
        .copied()
        .filter(|point| point.abs_diff(frame) <= tolerance)
        .min_by_key(|point| point.abs_diff(frame))
        .unwrap_or(frame)
}

/// Undo and redo, as whole-project snapshots.
///
/// Before every change, call [`History::record`] with the project as it was.
/// Snapshots are cheap: a project is only numbers and paths; the audio is
/// stored elsewhere.
#[derive(Debug, Clone, Default)]
pub struct History {
    undo: Vec<Project>,
    redo: Vec<Project>,
}

impl History {
    /// Remembers `before` (the project before a change). Clears the redo steps:
    /// after a new change, the undone changes cannot come back.
    pub fn record(&mut self, before: Project) {
        self.undo.push(before);
        if self.undo.len() > HISTORY_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Steps back: returns the previous project, remembering `current` for redo.
    pub fn undo(&mut self, current: Project) -> Option<Project> {
        let previous = self.undo.pop()?;
        self.redo.push(current);
        Some(previous)
    }

    /// Steps forward again after an undo.
    pub fn redo(&mut self, current: Project) -> Option<Project> {
        let next = self.redo.pop()?;
        self.undo.push(current);
        Some(next)
    }

    /// Whether there is something to undo.
    #[must_use]
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Whether there is something to redo.
    #[must_use]
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::audio::AudioBuffer;
    use crate::mix::{AudioPool, Mix};
    use crate::project::SourceId;
    use crate::time::SampleRate;

    /// A clip playing source frames 100..1100 at timeline frames 1000..2000.
    fn clip() -> Clip {
        Clip {
            id: ClipId(1),
            source: SourceId(1),
            start: 1000,
            offset: 100,
            length: 1000,
            ..Clip::default()
        }
    }

    #[test]
    fn fades_always_fit_in_the_clip() {
        let faded = clip().with_fades(700, 700);
        assert_eq!((faded.fade_in, faded.fade_out), (700, 300));
        // Trimming shorter shrinks the fades with it.
        let trimmed = faded.trimmed_end(1500, 5000);
        assert_eq!(trimmed.length, 500);
        assert_eq!((trimmed.fade_in, trimmed.fade_out), (500, 0));
    }

    #[test]
    fn splitting_keeps_each_fade_with_its_end() {
        let faded = clip().with_fades(100, 200);
        let (first, second) = faded.split_at(1500, ClipId(2)).unwrap();
        assert_eq!((first.fade_in, first.fade_out), (100, 0));
        assert_eq!((second.fade_in, second.fade_out), (0, 200));
    }

    #[test]
    fn trimming_the_start_keeps_the_audio_in_place() {
        let trimmed = clip().trimmed_start(1200);
        assert_eq!(
            (trimmed.start, trimmed.offset, trimmed.length),
            (1200, 300, 800)
        );
        assert_eq!(trimmed.end(), clip().end());
    }

    #[test]
    fn the_start_cannot_go_before_the_source_or_past_the_end() {
        // The source starts 100 frames before the clip's in-point.
        assert_eq!(clip().trimmed_start(0).start, 900);
        assert_eq!(clip().trimmed_start(0).offset, 0);
        assert_eq!(clip().trimmed_start(5000).length, MIN_CLIP_LENGTH);
    }

    #[test]
    fn the_end_cannot_go_past_the_source() {
        // 1500 source frames, offset 100: at most 1400 frames can play.
        assert_eq!(clip().trimmed_end(9999, 1500).length, 1400);
        assert_eq!(clip().trimmed_end(1500, 1500).length, 500);
        assert_eq!(clip().trimmed_end(0, 1500).length, MIN_CLIP_LENGTH);
    }

    #[test]
    fn split_halves_play_the_same_audio() {
        let (first, second) = clip().split_at(1400, ClipId(2)).unwrap();
        assert_eq!((first.start, first.offset, first.length), (1000, 100, 400));
        assert_eq!(
            (second.start, second.offset, second.length),
            (1400, 500, 600)
        );
        assert_eq!(second.id, ClipId(2));
        assert!(
            clip().split_at(1000, ClipId(2)).is_none(),
            "at the very start"
        );
        assert!(
            clip().split_at(2000, ClipId(2)).is_none(),
            "at the very end"
        );
    }

    #[test]
    fn the_playhead_belongs_to_the_clip_that_starts_there() {
        let mut project = Project::default();
        let track = project.import("a.wav".into(), 1000, "a".to_owned());
        let first = project.track(track).unwrap().clips[0].id;
        let second = project.split_clip(first, 400).unwrap();
        assert_eq!(project.clips_at(400), vec![second]);
        assert_eq!(project.clips_at(399), vec![first]);
        assert!(
            project.clips_at(1000).is_empty(),
            "the end is not part of a clip"
        );
    }

    #[test]
    fn snapping_picks_the_nearest_point_within_tolerance() {
        let points = [0, 1000, 2000];
        assert_eq!(snap(1030, &points, 50), 1000);
        assert_eq!(snap(1960, &points, 50), 2000);
        assert_eq!(snap(1500, &points, 50), 1500, "too far from any point");
    }

    #[test]
    fn a_clip_can_move_to_another_track() {
        let mut project = Project::default();
        let first = project.import("a.wav".into(), 1000, "a".to_owned());
        let second = project.add_track("b".to_owned());
        let clip = project.track(first).unwrap().clips[0];

        assert!(project.update_clip(clip.moved_to(500), second));
        assert!(project.track(first).unwrap().clips.is_empty());
        assert_eq!(project.track(second).unwrap().clips[0].start, 500);
        assert!(!project.update_clip(clip, TrackId(999)), "unknown track");
    }

    /// A track with a 1000-frame clip at 1000..2000 (from a 5000-frame source)
    /// and a second clip, on its own track, to drop onto it.
    fn two_clips() -> (Project, TrackId, Clip, Clip) {
        let mut project = Project::default();
        let track = project.import("under.wav".into(), 5000, "under".to_owned());
        let under = project.track(track).unwrap().clips[0];
        let under = Clip {
            offset: 500,
            ..under.moved_to(1000)
        };
        let under = Clip {
            length: 1000,
            ..under
        };
        assert!(project.update_clip(under, track));
        let other = project.import("top.wav".into(), 5000, "top".to_owned());
        let top = project.track(other).unwrap().clips[0];
        (project, track, under, Clip { length: 400, ..top })
    }

    fn clips(project: &Project, track: TrackId) -> Vec<Clip> {
        project.track(track).unwrap().clips.clone()
    }

    /// What is visible on `track`.
    fn visible(project: &Project, track: TrackId) -> Vec<Clip> {
        visible_parts(&clips(project, track))
    }

    #[test]
    fn a_clip_on_top_hides_the_end_it_covers() {
        let (mut project, track, under, top) = two_clips();
        assert!(project.place_clip(top.moved_to(1800), track));
        let parts = visible(&project, track);
        assert_eq!(parts.len(), 2);
        assert_eq!((parts[0].start, parts[0].length), (1000, 800));
        assert_eq!(parts[0].id, under.id);
        assert_eq!(parts[1].id, top.id, "the moved clip is on top");
        assert_eq!(clips(&project, track)[0], under, "nothing is cut");
    }

    #[test]
    fn a_clip_on_top_hides_the_start_it_covers() {
        let (mut project, track, under, top) = two_clips();
        assert!(project.place_clip(top.moved_to(800), track));
        let rest = visible(&project, track)[0];
        // The audio stays where it was: 200 frames later, 200 frames further in.
        assert_eq!(
            (rest.start, rest.offset, rest.length),
            (1200, under.offset + 200, 800)
        );
    }

    #[test]
    fn a_clip_in_the_middle_leaves_two_visible_parts() {
        let (mut project, track, under, top) = two_clips();
        assert!(project.place_clip(top.moved_to(1300), track));
        let parts = visible(&project, track);
        assert_eq!(parts.len(), 3);
        assert_eq!((parts[0].start, parts[0].length), (1000, 300));
        assert_eq!(
            (parts[1].start, parts[1].offset),
            (1700, under.offset + 700)
        );
        assert_eq!(parts[1].end(), under.end());
        assert_eq!(parts[1].id, under.id, "both parts belong to the same clip");
    }

    #[test]
    fn a_clip_that_is_covered_completely_is_hidden_but_kept() {
        let (mut project, track, under, top) = two_clips();
        let top = Clip {
            length: 1500,
            ..top.moved_to(900)
        };
        assert!(project.place_clip(top, track));
        assert_eq!(visible(&project, track), vec![top]);
        assert_eq!(clips(&project, track), vec![under, top]);
    }

    #[test]
    fn a_fade_in_on_the_top_clip_makes_a_crossfade() {
        let (mut project, track, _, top) = two_clips();
        let top = top.moved_to(1500).with_fades(200, 0);
        assert!(project.place_clip(top, track));
        let rest = visible(&project, track)[0];
        // The clip underneath plays on until the fade-in ends, fading out.
        assert_eq!(rest.end(), 1700);
        assert_eq!(rest.fade_out, 200);
    }

    #[test]
    fn clips_elsewhere_are_left_alone() {
        let (mut project, track, under, top) = two_clips();
        assert!(project.place_clip(top.moved_to(3000), track));
        assert_eq!(visible(&project, track)[0], under);
    }

    #[test]
    fn changing_a_clip_keeps_its_place_in_the_stack() {
        let (mut project, track, under, top) = two_clips();
        assert!(project.place_clip(top.moved_to(1500), track));
        // Changing the gain of the clip underneath must not bring it to the top.
        let louder = Clip {
            gain_db: 3.0,
            ..under
        };
        assert!(project.update_clip(louder, track));
        assert_eq!(clips(&project, track)[1].id, top.id);
    }

    #[test]
    fn splitting_keeps_the_stack() {
        let (mut project, track, under, top) = two_clips();
        assert!(project.place_clip(top.moved_to(1500), track));
        let before = visible(&project, track);
        let second = project.split_clip(under.id, 1200).unwrap();
        let ids: Vec<ClipId> = clips(&project, track).iter().map(|c| c.id).collect();
        assert_eq!(ids, vec![under.id, second, top.id]);
        // Splitting changes nothing you hear: same parts, the second half has a new ID.
        assert_eq!(visible(&project, track).len(), before.len() + 1);
    }

    #[test]
    fn the_topmost_clip_under_the_playhead_comes_first() {
        let mut project = Project::default();
        let track = project.import("a.wav".into(), 1000, "a".to_owned());
        let below = project.track(track).unwrap().clips[0];
        let above = project.add_clip(track, below.source, 0).unwrap();
        assert_eq!(project.clips_at(10), vec![above, below.id]);
    }

    #[test]
    fn dragging_the_last_clip_away_removes_its_track() {
        let mut project = Project::default();
        let first = project.import("a.wav".into(), 1000, "a".to_owned());
        let second = project.import("b.wav".into(), 1000, "b".to_owned());
        let clip = project.track(second).unwrap().clips[0];
        assert!(project.move_clip(clip.moved_to(2000), first));
        assert_eq!(project.tracks.len(), 1);
        assert_eq!(project.track(first).unwrap().clips.len(), 2);
    }

    #[test]
    fn moving_a_clip_on_its_own_track_keeps_the_track() {
        let mut project = Project::default();
        let track = project.import("a.wav".into(), 1000, "a".to_owned());
        let clip = project.track(track).unwrap().clips[0];
        assert!(project.move_clip(clip.moved_to(500), track));
        assert_eq!(project.tracks.len(), 1);
    }

    #[test]
    fn a_clip_can_get_a_track_of_its_own_again() {
        let mut project = Project::default();
        let track = project.import("a.wav".into(), 1000, "a".to_owned());
        let source = project.track(track).unwrap().clips[0].source;
        let second = project.add_clip(track, source, 2000).unwrap();
        let clip = *project.clip(second).unwrap().1;

        let new = project
            .move_clip_to_new_track(clip, "b".to_owned())
            .unwrap();
        assert_eq!(project.tracks.len(), 2);
        assert_eq!(project.tracks[1].id, new, "the new track is at the bottom");
        assert_eq!(project.track(new).unwrap().clips, vec![clip]);
        assert_eq!(project.track(track).unwrap().clips.len(), 1);

        // Moving a track's only clip to a new track leaves no empty track behind.
        assert!(
            project
                .move_clip_to_new_track(clip, "c".to_owned())
                .is_some()
        );
        assert_eq!(project.tracks.len(), 2);
        assert!(
            project
                .move_clip_to_new_track(Clip::default(), "x".to_owned())
                .is_none(),
            "unknown clip"
        );
        assert_eq!(project.tracks.len(), 2, "no track for an unknown clip");
    }

    #[test]
    fn a_track_can_be_renamed_but_not_to_nothing() {
        let mut project = Project::default();
        let track = project.import("a.wav".into(), 1000, "a".to_owned());
        assert!(project.rename_track(track, "  Voice  "));
        assert_eq!(project.track(track).unwrap().name, "Voice");
        assert!(!project.rename_track(track, "Voice"), "same name");
        assert!(!project.rename_track(track, "   "), "empty name");
        assert!(!project.rename_track(TrackId(999), "x"), "unknown track");
    }

    #[test]
    fn tracks_move_up_and_down_but_not_past_the_ends() {
        let mut project = Project::default();
        let a = project.import("a.wav".into(), 1000, "a".to_owned());
        let b = project.import("b.wav".into(), 1000, "b".to_owned());
        assert!(!project.move_track(a, true), "already at the top");
        assert!(!project.move_track(b, false), "already at the bottom");
        assert!(project.move_track(b, true));
        assert_eq!(project.track_ids(), vec![b, a]);
        assert!(project.move_track(b, false));
        assert_eq!(project.track_ids(), vec![a, b]);
    }

    #[test]
    fn a_file_can_be_imported_at_the_playhead() {
        let mut project = Project::default();
        let track = project.import_at("a.wav".into(), 1000, "a".to_owned(), 4800);
        assert_eq!(project.track(track).unwrap().clips[0].start, 4800);
        assert_eq!(project.length(), 5800);
    }

    #[test]
    fn a_track_can_be_removed() {
        let mut project = Project::default();
        let first = project.import("a.wav".into(), 1000, "a".to_owned());
        let second = project.import("b.wav".into(), 1000, "b".to_owned());
        assert_eq!(
            project.remove_track(first).map(|track| track.id),
            Some(first)
        );
        assert_eq!(project.tracks.len(), 1);
        assert_eq!(project.tracks[0].id, second);
        assert!(project.remove_track(first).is_none(), "already gone");
    }

    #[test]
    fn undo_and_redo_walk_through_the_history() {
        let mut history = History::default();
        let mut project = Project::default();
        assert!(!history.can_undo());

        history.record(project.clone());
        project.name = "one".to_owned();
        history.record(project.clone());
        project.name = "two".to_owned();

        project = history.undo(project).unwrap();
        assert_eq!(project.name, "one");
        project = history.undo(project).unwrap();
        assert_eq!(project.name, "");
        assert!(history.undo(project.clone()).is_none());

        project = history.redo(project).unwrap();
        assert_eq!(project.name, "one");
        // A new change makes the undone "two" unreachable.
        history.record(project.clone());
        assert!(!history.can_redo());
    }

    /// Renders the whole project, left channel only.
    fn render(project: &Project, pool: &AudioPool) -> Vec<f32> {
        let mix = Mix::new(project, pool);
        let mut out = vec![0.0; usize::try_from(mix.length()).unwrap() * 2];
        mix.render(0, &mut out);
        out.into_iter().step_by(2).collect()
    }

    proptest! {
        /// Splitting a clip anywhere must not change what you hear.
        #[test]
        fn splitting_does_not_change_the_mix(at in 0_u64..300) {
            let samples: Vec<f32> = (0..200_u16).flat_map(|n| [f32::from(n), 0.0]).collect();
            let mut project = Project::default();
            let track = project.import("ramp.wav".into(), 200, "r".to_owned());
            let clip = project.track(track).unwrap().clips[0];
            let pool = AudioPool::from([(
                clip.source,
                AudioBuffer::new(SampleRate::HZ_48000, samples).unwrap(),
            )]);
            let before = render(&project, &pool);

            project.split_clip(clip.id, at);
            prop_assert_eq!(render(&project, &pool), before);
        }

        /// A clip on top changes nothing you hear outside it, hides everything
        /// underneath where it plays at full volume, and moving it away again
        /// brings back exactly what was there: stacking never cuts audio.
        #[test]
        fn a_clip_on_top_only_hides_what_it_covers(
            start in 0_u64..2500,
            length in 64_u64..1500,
            fade_in in 0_u64..600,
            fade_out in 0_u64..600,
        ) {
            let ramp: Vec<f32> = (0..2000_u16).flat_map(|n| [f32::from(n), 0.0]).collect();
            let mut project = Project::default();
            let track = project.import("ramp.wav".into(), 2000, "r".to_owned());
            let other = project.import("silence.wav".into(), 2000, "s".to_owned());
            let top = project.track(other).unwrap().clips[0];
            let pool = AudioPool::from([
                (project.track(track).unwrap().clips[0].source,
                 AudioBuffer::new(SampleRate::HZ_48000, ramp).unwrap()),
                (top.source, AudioBuffer::new(SampleRate::HZ_48000, vec![0.0; 4000]).unwrap()),
            ]);
            // The top clip plays silence, so it only matters through what it hides.
            let before = render(&project, &pool);

            let top = Clip { length, ..top.moved_to(start) }.with_fades(fade_in, fade_out);
            prop_assert!(project.place_clip(top, track));
            let stacked = render(&project, &pool);
            let covered = usize::try_from(top.start).unwrap()..usize::try_from(top.end()).unwrap();
            for (frame, (a, b)) in before.iter().zip(&stacked).enumerate() {
                if !covered.contains(&frame) {
                    prop_assert!((a - b).abs() < 1e-6, "frame {} outside the top clip changed", frame);
                }
            }
            let full_volume = (top.start + top.fade_in)..(top.end() - top.fade_out);
            for part in visible_parts(&project.track(track).unwrap().clips) {
                if part.id != top.id {
                    prop_assert!(
                        part.end() <= full_volume.start || part.start >= full_volume.end,
                        "{:?} still plays under {:?}", part, top
                    );
                }
            }

            // Back to its own track: the ramp sounds exactly as before.
            prop_assert!(project.place_clip(top, other));
            let restored = render(&project, &pool);
            for (frame, (a, b)) in before.iter().zip(&restored).enumerate() {
                prop_assert!((a - b).abs() < 1e-6, "frame {} did not come back", frame);
            }
        }

        /// Trimming the start never moves audio on the timeline: wherever the
        /// trimmed clip still plays, it plays the same source frame as before.
        #[test]
        fn trimming_keeps_audio_aligned(new_start in 0_u64..4000) {
            let original = clip();
            let trimmed = original.trimmed_start(new_start);
            prop_assert_eq!(
                trimmed.start.saturating_sub(trimmed.offset),
                original.start - original.offset
            );
            prop_assert!(trimmed.length >= MIN_CLIP_LENGTH);
        }
    }
}
