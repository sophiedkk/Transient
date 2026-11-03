//! Editing operations: moving, trimming, splitting and deleting clips, snapping,
//! and the undo history.
//!
//! The clip operations are pure functions (a clip in, a changed clip out), so the
//! timeline can use them to preview a drag before the project changes.

use crate::project::{Clip, ClipId, Project, TrackId};
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
    }

    /// The clip with its right edge dragged to `new_end`, but not past the end of
    /// the source (`source_frames` long).
    #[must_use]
    pub fn trimmed_end(self, new_end: Frames, source_frames: Frames) -> Self {
        let longest = source_frames.saturating_sub(self.offset);
        let length = new_end
            .saturating_sub(self.start)
            .clamp(MIN_CLIP_LENGTH.min(longest), longest);
        Self { length, ..self }
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
        let first = Self {
            length: first_length,
            ..self
        };
        let second = Self {
            id: new_id,
            start: at,
            offset: self.offset + first_length,
            length: self.length - first_length,
            ..self
        };
        Some((first, second))
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

    /// Replaces the clip with the same ID by `clip`, on `track` (moving it there if
    /// it was on another track). Returns `false` if the clip or track does not exist.
    pub fn update_clip(&mut self, clip: Clip, track: TrackId) -> bool {
        if self.track(track).is_none() || self.remove_clip(clip.id).is_none() {
            return false;
        }
        self.track_mut(track)
            .map(|track| track.clips.push(clip))
            .is_some()
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
        if let Some(slot) = clips.iter_mut().find(|c| c.id == id) {
            *slot = first;
        }
        clips.push(second);
        Some(new_id)
    }

    /// Every clip playing at `frame`: from its first frame up to (not including)
    /// its end, so after a split the playhead belongs to the second half.
    #[must_use]
    pub fn clips_at(&self, frame: Frames) -> Vec<ClipId> {
        self.tracks
            .iter()
            .flat_map(|track| &track.clips)
            .filter(|clip| clip.start <= frame && frame < clip.end())
            .map(|clip| clip.id)
            .collect()
    }
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
        }
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
