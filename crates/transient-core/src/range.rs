//! Editing a stretch of time instead of whole clips: cutting a range out of some
//! tracks, ripple delete (cut and close the gap), and splitting at both ends
//! of a range.
//!
//! A range is a [`TimeRange`] plus the tracks it covers (the lanes the user
//! dragged over). Like everything in [`crate::edit`], these are plain changes to a
//! [`Project`]; the window records them for undo.

use crate::project::{Clip, ClipId, Project, TrackId};
use crate::time::Frames;

/// A stretch of the timeline: from `start` up to (not including) `end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeRange {
    /// The first frame in the range.
    pub start: Frames,
    /// The first frame after the range.
    pub end: Frames,
}

impl TimeRange {
    /// The range between two frames, in either order (a drag can go left or right).
    #[must_use]
    pub fn between(a: Frames, b: Frames) -> Self {
        Self {
            start: a.min(b),
            end: a.max(b),
        }
    }

    /// How many frames the range covers.
    #[must_use]
    pub fn length(self) -> Frames {
        self.end - self.start
    }

    /// Whether the range covers nothing.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.end <= self.start
    }
}

impl Clip {
    /// What is left of the clip after cutting `range` out of it: the piece
    /// before the range and the piece after it (either can be `None`). The audio
    /// in each piece stays where it was on the timeline. If both are left, the
    /// second piece gets `new_id`. A clip outside the range comes back unchanged.
    ///
    /// Like a split, the fade-in stays with the first piece and the fade-out
    /// with the second.
    #[must_use]
    pub fn without(self, range: TimeRange, new_id: ClipId) -> (Option<Self>, Option<Self>) {
        if range.is_empty() || self.end() <= range.start || self.start >= range.end {
            return (Some(self), None);
        }
        let before = (self.start < range.start).then(|| {
            Self {
                length: range.start - self.start,
                ..self
            }
            .with_fades(self.fade_in, 0)
        });
        let after = (self.end() > range.end).then(|| {
            Self {
                id: if before.is_some() { new_id } else { self.id },
                start: range.end,
                offset: self.offset + (range.end - self.start),
                length: self.end() - range.end,
                ..self
            }
            .with_fades(0, self.fade_out)
        });
        (before, after)
    }
}

impl Project {
    /// Cuts `range` out of every clip on `tracks`, leaving a gap. Returns whether
    /// anything changed.
    pub fn delete_range(&mut self, tracks: &[TrackId], range: TimeRange) -> bool {
        let mut next_id = self.next_id;
        let mut changed = false;
        for track in self
            .tracks
            .iter_mut()
            .filter(|track| tracks.contains(&track.id))
        {
            let clips = std::mem::take(&mut track.clips);
            for clip in clips {
                let (before, after) = clip.without(range, ClipId(next_id));
                if before != Some(clip) {
                    changed = true;
                }
                if before.is_some() && after.is_some() {
                    // The new ID was used for the second piece.
                    next_id += 1;
                }
                // Both pieces take the clip's place in the stack.
                track.clips.extend(before);
                track.clips.extend(after);
            }
        }
        self.next_id = next_id;
        changed
    }

    /// Ripple delete: cuts `range` out of `tracks` and closes the gap, by moving
    /// everything that starts at or after the end of the range to the left, on
    /// *every* track, so tracks stay in sync. Tracks that are not selected are
    /// not cut, only moved; a clip there that starts before the end of the range
    /// stays where it is. Returns whether anything changed.
    pub fn ripple_delete(&mut self, tracks: &[TrackId], range: TimeRange) -> bool {
        if range.is_empty() {
            return false;
        }
        let mut changed = self.delete_range(tracks, range);
        for clip in self.tracks.iter_mut().flat_map(|track| &mut track.clips) {
            if clip.start >= range.end {
                clip.start -= range.length();
                changed = true;
            }
        }
        changed
    }

    /// Splits every clip on `tracks` at both ends of `range`, so the part inside
    /// the range becomes clips of its own. Returns whether anything changed.
    pub fn split_range(&mut self, tracks: &[TrackId], range: TimeRange) -> bool {
        let mut changed = false;
        for at in [range.start, range.end] {
            let to_split: Vec<ClipId> = self
                .tracks
                .iter()
                .filter(|track| tracks.contains(&track.id))
                .flat_map(|track| &track.clips)
                .filter(|clip| clip.start < at && at < clip.end())
                .map(|clip| clip.id)
                .collect();
            for clip in to_split {
                changed |= self.split_clip(clip, at).is_some();
            }
        }
        changed
    }

    /// Every track, for "select all".
    #[must_use]
    pub fn track_ids(&self) -> Vec<TrackId> {
        self.tracks.iter().map(|track| track.id).collect()
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::audio::AudioBuffer;
    use crate::mix::AudioPool;
    use crate::test_support::{clips, range, render};
    use crate::time::SampleRate;

    /// Two tracks, each with one 1000-frame clip at 0..1000.
    fn two_tracks() -> (Project, TrackId, TrackId) {
        let mut project = Project::default();
        let a = project.import("a.wav".into(), 1000, "a".to_owned(), 0);
        let b = project.import("b.wav".into(), 1000, "b".to_owned(), 0);
        (project, a, b)
    }

    #[test]
    fn a_range_can_be_dragged_either_way() {
        assert_eq!(TimeRange::between(500, 200), range(200, 500));
        assert_eq!(TimeRange::between(200, 500).length(), 300);
    }

    #[test]
    fn deleting_a_range_leaves_a_gap_on_the_selected_track_only() {
        let (mut project, a, b) = two_tracks();
        assert!(project.delete_range(&[a], range(300, 600)));
        let pieces = clips(&project, a);
        assert_eq!(pieces.len(), 2);
        assert_eq!((pieces[0].start, pieces[0].length), (0, 300));
        assert_eq!(
            (pieces[1].start, pieces[1].offset, pieces[1].length),
            (600, 600, 400)
        );
        assert_eq!(clips(&project, b).len(), 1, "the other track is untouched");
    }

    #[test]
    fn deleting_a_range_over_a_whole_clip_removes_it() {
        let (mut project, a, _) = two_tracks();
        assert!(project.delete_range(&[a], range(0, 2000)));
        assert!(clips(&project, a).is_empty());
    }

    #[test]
    fn deleting_an_empty_stretch_changes_nothing() {
        let (mut project, a, _) = two_tracks();
        assert!(!project.delete_range(&[a], range(2000, 3000)));
    }

    #[test]
    fn ripple_delete_closes_the_gap_on_every_track() {
        let (mut project, a, b) = two_tracks();
        // A second clip on track b, after the range.
        let source = clips(&project, b)[0].source;
        project.add_clip(b, source, 1500).unwrap();

        assert!(project.ripple_delete(&[a], range(300, 600)));
        let pieces = clips(&project, a);
        assert_eq!(
            (pieces[1].start, pieces[1].offset),
            (300, 600),
            "pulled left"
        );
        let other = clips(&project, b);
        assert_eq!(other[0].start, 0, "starts before the range: stays");
        assert_eq!(other[0].length, 1000, "not selected: not cut");
        assert_eq!(
            other[1].start, 1200,
            "after the range: moved left with the rest"
        );
    }

    #[test]
    fn splitting_a_range_makes_its_middle_a_clip() {
        let (mut project, a, b) = two_tracks();
        assert!(project.split_range(&[a], range(300, 600)));
        let pieces = clips(&project, a);
        assert_eq!(pieces.len(), 3);
        assert_eq!((pieces[1].start, pieces[1].length), (300, 300));
        assert_eq!(clips(&project, b).len(), 1);
    }

    /// A ramp (frame n plays the value n + 1, so silence is easy to tell apart)
    /// on one track.
    fn ramp_project(frames: u16) -> (Project, TrackId, AudioPool) {
        let mut project = Project::default();
        let track = project.import("ramp.wav".into(), u64::from(frames), "r".to_owned(), 0);
        let samples: Vec<f32> = (0..frames)
            .flat_map(|n| [f32::from(n) + 1.0, 0.0])
            .collect();
        let source = clips(&project, track)[0].source;
        let pool = AudioPool::from([(
            source,
            AudioBuffer::new(SampleRate::HZ_48000, samples).unwrap(),
        )]);
        (project, track, pool)
    }

    proptest! {
        /// Deleting a range silences exactly that range and nothing else.
        #[test]
        fn deleting_silences_exactly_the_range(start in 0_u64..1200, length in 1_u64..600) {
            let (mut project, track, pool) = ramp_project(1000);
            let before = render(&project, &pool);
            let cut = range(start, start + length);
            project.delete_range(&[track], cut);
            let after = render(&project, &pool);
            for (frame, original) in before.iter().enumerate() {
                let frame_u64 = u64::try_from(frame).unwrap();
                let now = after.get(frame).copied().unwrap_or(0.0);
                if (cut.start..cut.end).contains(&frame_u64) {
                    prop_assert!(now.abs() < 1e-6, "frame {} should be silent", frame);
                } else {
                    prop_assert!((now - original).abs() < 1e-6, "frame {} changed", frame);
                }
            }
        }

        /// Ripple delete plays what came after the range right where the range
        /// began: the result is the original with the range taken out.
        #[test]
        fn ripple_delete_takes_the_range_out(start in 0_u64..1000, length in 1_u64..600) {
            let (mut project, track, pool) = ramp_project(1000);
            let before = render(&project, &pool);
            let cut = range(start, (start + length).min(1000));
            prop_assume!(!cut.is_empty());
            project.ripple_delete(&[track], cut);
            let after = render(&project, &pool);

            let (start, end) = (usize::try_from(cut.start).unwrap(), usize::try_from(cut.end).unwrap());
            let mut expected = before[..start].to_vec();
            expected.extend_from_slice(&before[end..]);
            prop_assert_eq!(after.len(), expected.len());
            for (frame, (a, b)) in after.iter().zip(&expected).enumerate() {
                prop_assert!((a - b).abs() < 1e-6, "frame {} differs", frame);
            }
        }

        /// Splitting a range never changes what you hear.
        #[test]
        fn splitting_a_range_does_not_change_the_mix(start in 0_u64..1200, length in 0_u64..600) {
            let (mut project, track, pool) = ramp_project(1000);
            let before = render(&project, &pool);
            project.split_range(&[track], range(start, start + length));
            let after = render(&project, &pool);
            for (frame, (a, b)) in before.iter().zip(&after).enumerate() {
                prop_assert!((a - b).abs() < 1e-6, "frame {} changed", frame);
            }
        }
    }
}
