//! Changes to a project: moving clips (between tracks too), splitting and
//! removing them, renaming, reordering and removing tracks, and snapping.
//!
//! What happens to a single clip (trimming, splitting, stacking) is in
//! [`crate::clip`]; undo and redo in [`crate::history`].

use crate::project::{Clip, ClipId, Project, Track, TrackId};
use crate::time::Frames;

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

    /// Changes the clip with this ID in place with `change` (its gain, fades, …).
    /// It stays on its track and keeps its place in the stack. Returns whether
    /// anything changed: `false` for an unknown clip, or when `change` returns
    /// the clip as it was.
    pub fn change_clip(&mut self, id: ClipId, change: impl FnOnce(Clip) -> Clip) -> bool {
        let Some((track, clip)) = self.clip(id).map(|(track, clip)| (track, *clip)) else {
            return false;
        };
        let changed = change(clip);
        changed != clip && self.update_clip(changed, track)
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

    /// Moves several clips at once (a group drag or nudge): each goes onto its
    /// track, on top. Tracks left empty by the move are removed afterwards, once
    /// every clip has landed (so a clip can still move onto a track another clip
    /// leaves). Returns whether anything changed.
    pub fn move_clips(&mut self, moves: &[(Clip, TrackId)]) -> bool {
        let sources: Vec<TrackId> = moves
            .iter()
            .filter_map(|(clip, _)| self.clip(clip.id).map(|(track, _)| track))
            .collect();
        let mut changed = false;
        for &(clip, track) in moves {
            changed |= self.place_clip(clip, track);
        }
        for source in sources {
            let emptied = self.track(source).is_some_and(|t| t.clips.is_empty());
            if emptied && moves.iter().all(|(_, target)| *target != source) {
                let _removed = self.remove_track(source);
            }
        }
        changed
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

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::audio::AudioBuffer;
    use crate::clip::visible_parts;
    use crate::mix::AudioPool;
    use crate::test_support::{clips, render};
    use crate::time::SampleRate;

    #[test]
    fn the_playhead_belongs_to_the_clip_that_starts_there() {
        let mut project = Project::default();
        let track = project.import("a.wav".into(), 1000, "a".to_owned(), 0);
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
        let first = project.import("a.wav".into(), 1000, "a".to_owned(), 0);
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
        let track = project.import("under.wav".into(), 5000, "under".to_owned(), 0);
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
        let other = project.import("top.wav".into(), 5000, "top".to_owned(), 0);
        let top = project.track(other).unwrap().clips[0];
        (project, track, under, Clip { length: 400, ..top })
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
        let track = project.import("a.wav".into(), 1000, "a".to_owned(), 0);
        let below = project.track(track).unwrap().clips[0];
        let above = project.add_clip(track, below.source, 0).unwrap();
        assert_eq!(project.clips_at(10), vec![above, below.id]);
    }

    #[test]
    fn dragging_the_last_clip_away_removes_its_track() {
        let mut project = Project::default();
        let first = project.import("a.wav".into(), 1000, "a".to_owned(), 0);
        let second = project.import("b.wav".into(), 1000, "b".to_owned(), 0);
        let clip = project.track(second).unwrap().clips[0];
        assert!(project.move_clip(clip.moved_to(2000), first));
        assert_eq!(project.tracks.len(), 1);
        assert_eq!(project.track(first).unwrap().clips.len(), 2);
    }

    #[test]
    fn moving_a_clip_on_its_own_track_keeps_the_track() {
        let mut project = Project::default();
        let track = project.import("a.wav".into(), 1000, "a".to_owned(), 0);
        let clip = project.track(track).unwrap().clips[0];
        assert!(project.move_clip(clip.moved_to(500), track));
        assert_eq!(project.tracks.len(), 1);
    }

    #[test]
    fn a_clip_can_get_a_track_of_its_own_again() {
        let mut project = Project::default();
        let track = project.import("a.wav".into(), 1000, "a".to_owned(), 0);
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
    fn a_clip_can_be_changed_in_place() {
        let (mut project, track, under, top) = two_clips();
        assert!(project.place_clip(top.moved_to(1500), track));
        assert!(project.change_clip(under.id, |clip| clip.with_fades(100, 0)));
        let clips = clips(&project, track);
        assert_eq!(clips[0].fade_in, 100);
        assert_eq!(clips[1].id, top.id, "the changed clip stays underneath");
        assert!(
            !project.change_clip(under.id, |clip| clip),
            "nothing changed"
        );
        assert!(
            !project.change_clip(ClipId(999), |clip| clip),
            "unknown clip"
        );
    }

    #[test]
    fn a_track_can_be_renamed_but_not_to_nothing() {
        let mut project = Project::default();
        let track = project.import("a.wav".into(), 1000, "a".to_owned(), 0);
        assert!(project.rename_track(track, "  Voice  "));
        assert_eq!(project.track(track).unwrap().name, "Voice");
        assert!(!project.rename_track(track, "Voice"), "same name");
        assert!(!project.rename_track(track, "   "), "empty name");
        assert!(!project.rename_track(TrackId(999), "x"), "unknown track");
    }

    #[test]
    fn tracks_move_up_and_down_but_not_past_the_ends() {
        let mut project = Project::default();
        let a = project.import("a.wav".into(), 1000, "a".to_owned(), 0);
        let b = project.import("b.wav".into(), 1000, "b".to_owned(), 0);
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
        let track = project.import("a.wav".into(), 1000, "a".to_owned(), 4800);
        assert_eq!(project.track(track).unwrap().clips[0].start, 4800);
        assert_eq!(project.length(), 5800);
    }

    #[test]
    fn a_group_moves_together_and_leaves_no_empty_track() {
        let mut project = Project::default();
        let a = project.import("a.wav".into(), 1000, "a".to_owned(), 0);
        let b = project.import("b.wav".into(), 1000, "b".to_owned(), 0);
        let c = project.import("c.wav".into(), 1000, "c".to_owned(), 0);
        let clip_b = project.track(b).unwrap().clips[0];
        let clip_c = project.track(c).unwrap().clips[0];
        // Both clips move up one track: b's onto a, c's onto b. Track b is both
        // left and arrived at, so only track c ends up empty.
        assert!(project.move_clips(&[(clip_b.moved_to(100), a), (clip_c.moved_to(100), b)]));
        assert_eq!(
            project.track_ids(),
            vec![a, b],
            "only the emptied track c goes"
        );
        assert_eq!(project.track(a).unwrap().clips.len(), 2);
        assert_eq!(project.track(b).unwrap().clips, vec![clip_c.moved_to(100)]);
    }

    #[test]
    fn a_track_can_be_removed() {
        let mut project = Project::default();
        let first = project.import("a.wav".into(), 1000, "a".to_owned(), 0);
        let second = project.import("b.wav".into(), 1000, "b".to_owned(), 0);
        assert_eq!(
            project.remove_track(first).map(|track| track.id),
            Some(first)
        );
        assert_eq!(project.tracks.len(), 1);
        assert_eq!(project.tracks[0].id, second);
        assert!(project.remove_track(first).is_none(), "already gone");
    }

    proptest! {
        /// Splitting a clip anywhere must not change what you hear.
        #[test]
        fn splitting_does_not_change_the_mix(at in 0_u64..300) {
            let samples: Vec<f32> = (0..200_u16).flat_map(|n| [f32::from(n), 0.0]).collect();
            let mut project = Project::default();
            let track = project.import("ramp.wav".into(), 200, "r".to_owned(), 0);
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
            let track = project.import("ramp.wav".into(), 2000, "r".to_owned(), 0);
            let other = project.import("silence.wav".into(), 2000, "s".to_owned(), 0);
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

    }
}
