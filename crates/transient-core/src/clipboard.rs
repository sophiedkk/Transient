//! Cut, Copy and Paste: a [`Clipboard`] holds clips (or the parts of clips inside
//! a time selection) together with where they were relative to each other, so
//! pasting puts them back in the same arrangement somewhere else.
//!
//! The clipboard lives in the app, not in the system clipboard: it refers to the
//! project's sources (audio files) by ID, so it only makes sense within the
//! project it was copied from.

use crate::project::{Clip, ClipId, Project, TrackId};
use crate::range::TimeRange;
use crate::time::Frames;

/// One copied clip.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CopiedClip {
    /// How many tracks below the first copied track it was (0: on that track).
    pub track_offset: usize,
    /// The clip, with `start` counted from the start of the copy.
    pub clip: Clip,
}

/// What Copy (or Cut) took: clips and how long the copy is.
#[derive(Debug, Clone, PartialEq)]
pub struct Clipboard {
    /// The clips, in stacking order (later ones on top).
    pub clips: Vec<CopiedClip>,
    /// How long the copy is: for a time selection its length (silence
    /// included), for clips from the first start to the last end. Insert makes
    /// this much room.
    pub length: Frames,
    /// The track the copy started on; where Paste goes when nothing is selected.
    pub first_track: TrackId,
}

impl Clip {
    /// The part of the clip inside `range`, or `None` when they do not overlap.
    /// The audio stays where it was on the timeline. A fade is kept only where
    /// the clip's own start or end is inside the range.
    #[must_use]
    pub fn within(self, range: TimeRange) -> Option<Self> {
        if range.is_empty() || self.end() <= range.start || self.start >= range.end {
            return None;
        }
        let start = self.start.max(range.start);
        let end = self.end().min(range.end);
        let fade_in = if start == self.start { self.fade_in } else { 0 };
        let fade_out = if end == self.end() { self.fade_out } else { 0 };
        Some(
            Self {
                start,
                offset: self.offset + (start - self.start),
                length: end - start,
                ..self
            }
            .with_fades(fade_in, fade_out),
        )
    }
}

impl Project {
    /// Copies the clips `ids` (Copy with clips selected). `None` if none of them
    /// exist.
    #[must_use]
    pub fn copy_clips(&self, ids: &[ClipId]) -> Option<Clipboard> {
        // Track by track and bottom to top on each, so pasting stacks them the same way.
        let found: Vec<(usize, Clip)> = self
            .tracks
            .iter()
            .enumerate()
            .flat_map(|(index, track)| track.clips.iter().map(move |clip| (index, *clip)))
            .filter(|(_, clip)| ids.contains(&clip.id))
            .collect();
        let first_index = found.iter().map(|(index, _)| *index).min()?;
        let earliest = found.iter().map(|(_, clip)| clip.start).min()?;
        let latest = found.iter().map(|(_, clip)| clip.end()).max()?;
        Some(Clipboard {
            clips: found
                .into_iter()
                .map(|(index, clip)| CopiedClip {
                    track_offset: index - first_index,
                    clip: clip.moved_to(clip.start - earliest),
                })
                .collect(),
            length: latest - earliest,
            first_track: self.tracks.get(first_index)?.id,
        })
    }

    /// Copies what is inside `range` on `tracks` (Copy with a time selection).
    /// `None` if none of the tracks exist or the range is empty.
    #[must_use]
    pub fn copy_range(&self, tracks: &[TrackId], range: TimeRange) -> Option<Clipboard> {
        if range.is_empty() {
            return None;
        }
        let selected: Vec<(usize, &crate::project::Track)> = self
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, track)| tracks.contains(&track.id))
            .collect();
        let (first_index, first_track) =
            selected.first().map(|(index, track)| (*index, track.id))?;
        let clips = selected
            .iter()
            .flat_map(|(index, track)| {
                track.clips.iter().filter_map(move |clip| {
                    let part = clip.within(range)?;
                    Some(CopiedClip {
                        track_offset: index - first_index,
                        clip: part.moved_to(part.start - range.start),
                    })
                })
            })
            .collect();
        Some(Clipboard {
            clips,
            length: range.length(),
            first_track,
        })
    }

    /// Pastes `clipboard` at `at`, starting on `first_track` (the copy's lower
    /// tracks go onto the tracks below it; tracks called `new_track_name` are
    /// added at the bottom when there are not enough). Pasted clips go on top.
    /// Clips whose audio is not in this project are skipped. Returns the new
    /// clips' IDs.
    pub fn paste(
        &mut self,
        clipboard: &Clipboard,
        first_track: TrackId,
        at: Frames,
        new_track_name: &str,
    ) -> Vec<ClipId> {
        let base = self
            .tracks
            .iter()
            .position(|track| track.id == first_track)
            .unwrap_or(0);
        let mut pasted = Vec::new();
        for copied in &clipboard.clips {
            if self.source(copied.clip.source).is_none() {
                continue;
            }
            let index = base + copied.track_offset;
            while self.tracks.len() <= index {
                self.add_track(new_track_name.to_owned());
            }
            let id = ClipId(self.next_id);
            self.next_id += 1;
            let clip = Clip {
                id,
                ..copied.clip.moved_to(at + copied.clip.start)
            };
            if let Some(track) = self.tracks.get_mut(index) {
                track.clips.push(clip);
                pasted.push(id);
            }
        }
        pasted
    }

    /// Insert ("Paste and Make Room"): moves everything from `at` onwards to
    /// the right by the clipboard's length on every track (splitting clips that
    /// cross `at`), so tracks stay in sync, then pastes into the gap. The mirror
    /// image of [`Project::ripple_delete`]. Returns the new clips' IDs.
    pub fn insert(
        &mut self,
        clipboard: &Clipboard,
        first_track: TrackId,
        at: Frames,
        new_track_name: &str,
    ) -> Vec<ClipId> {
        let all_tracks = self.track_ids();
        // Split at `at` everywhere (a split with nothing to split is no change).
        let _changed = self.split_range(&all_tracks, TimeRange { start: at, end: at });
        for clip in self.tracks.iter_mut().flat_map(|track| &mut track.clips) {
            if clip.start >= at {
                clip.start += clipboard.length;
            }
        }
        self.paste(clipboard, first_track, at, new_track_name)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::audio::AudioBuffer;
    use crate::mix::{AudioPool, Mix};
    use crate::time::SampleRate;

    fn range(start: Frames, end: Frames) -> TimeRange {
        TimeRange { start, end }
    }

    /// Two tracks: a at 0..1000, b at 500..1500.
    fn two_tracks() -> (Project, TrackId, TrackId) {
        let mut project = Project::default();
        let a = project.import("a.wav".into(), 1000, "a".to_owned());
        let b = project.import_at("b.wav".into(), 1000, "b".to_owned(), 500);
        (project, a, b)
    }

    fn clips(project: &Project, track: TrackId) -> Vec<Clip> {
        project.track(track).unwrap().clips.clone()
    }

    #[test]
    fn copied_clips_remember_where_they_were_relative_to_each_other() {
        let (project, a, b) = two_tracks();
        let ids = [clips(&project, a)[0].id, clips(&project, b)[0].id];
        let copy = project.copy_clips(&ids).unwrap();
        assert_eq!(copy.length, 1500);
        assert_eq!(copy.first_track, a);
        let places: Vec<(usize, Frames)> = copy
            .clips
            .iter()
            .map(|c| (c.track_offset, c.clip.start))
            .collect();
        assert_eq!(places, vec![(0, 0), (1, 500)]);
    }

    #[test]
    fn a_copied_range_holds_only_what_is_inside_it() {
        let (project, a, b) = two_tracks();
        let copy = project.copy_range(&[a, b], range(800, 1200)).unwrap();
        assert_eq!(copy.length, 400);
        let parts: Vec<(usize, Frames, Frames, Frames)> = copy
            .clips
            .iter()
            .map(|c| (c.track_offset, c.clip.start, c.clip.offset, c.clip.length))
            .collect();
        // a: frames 800..1000 of its source; b: 800..1200 on the timeline is
        // frames 300..700 of its source.
        assert_eq!(parts, vec![(0, 0, 800, 200), (1, 0, 300, 400)]);
    }

    #[test]
    fn pasting_onto_the_last_track_adds_tracks_for_the_rest() {
        let (mut project, a, b) = two_tracks();
        let ids = [clips(&project, a)[0].id, clips(&project, b)[0].id];
        let copy = project.copy_clips(&ids).unwrap();
        let pasted = project.paste(&copy, b, 3000, "New");
        assert_eq!(pasted.len(), 2);
        assert_eq!(
            project.tracks.len(),
            3,
            "one track added for the copy's second row"
        );
        let b_clips = clips(&project, b);
        assert_eq!(
            b_clips.last().unwrap().start,
            3000,
            "on top, at the playhead"
        );
        assert_eq!(project.tracks[2].clips[0].start, 3500);
        assert_eq!(project.tracks[2].name, "New");
    }

    #[test]
    fn insert_makes_room_on_every_track() {
        let (mut project, a, b) = two_tracks();
        let copy = project.copy_range(&[a], range(0, 100)).unwrap();
        project.insert(&copy, a, 700, "New");
        // a was split at 700: its second half moved 100 to the right.
        let a_clips = clips(&project, a);
        assert_eq!(
            a_clips
                .iter()
                .map(|c| (c.start, c.end()))
                .collect::<Vec<_>>(),
            vec![(0, 700), (800, 1100), (700, 800)]
        );
        // b crossed 700 too: split, and its second part moved.
        let b_clips = clips(&project, b);
        assert_eq!(
            b_clips
                .iter()
                .map(|c| (c.start, c.end()))
                .collect::<Vec<_>>(),
            vec![(500, 700), (800, 1600)]
        );
    }

    #[test]
    fn a_fade_is_copied_only_with_the_clip_edge_it_belongs_to() {
        let clip = Clip {
            start: 0,
            length: 1000,
            ..Clip::default()
        }
        .with_fades(100, 100);
        let middle = clip.within(range(200, 800)).unwrap();
        assert_eq!((middle.fade_in, middle.fade_out), (0, 0));
        let start = clip.within(range(0, 500)).unwrap();
        assert_eq!((start.fade_in, start.fade_out), (100, 0));
    }

    /// Renders the whole project, left channel only.
    fn render(project: &Project, pool: &AudioPool) -> Vec<f32> {
        let mix = Mix::new(project, pool);
        let mut out = vec![0.0; usize::try_from(mix.length()).unwrap() * 2];
        mix.render(0, &mut out);
        out.into_iter().step_by(2).collect()
    }

    proptest! {
        /// Cutting a range out (ripple delete) and inserting the cut back at the
        /// same place gives exactly the original sound.
        #[test]
        fn cut_and_insert_back_changes_nothing(start in 0_u64..1000, length in 1_u64..600) {
            let mut project = Project::default();
            let track = project.import("ramp.wav".into(), 1000, "r".to_owned());
            let samples: Vec<f32> = (0..1000_u16).flat_map(|n| [f32::from(n) + 1.0, 0.0]).collect();
            let pool = AudioPool::from([(
                clips(&project, track)[0].source,
                AudioBuffer::new(SampleRate::HZ_48000, samples).unwrap(),
            )]);
            let before = render(&project, &pool);

            let cut = range(start, (start + length).min(1000));
            prop_assume!(!cut.is_empty());
            let copy = project.copy_range(&[track], cut).unwrap();
            project.ripple_delete(&[track], cut);
            project.insert(&copy, track, cut.start, "New");

            let after = render(&project, &pool);
            prop_assert_eq!(after.len(), before.len());
            for (frame, (a, b)) in after.iter().zip(&before).enumerate() {
                prop_assert!((a - b).abs() < 1e-6, "frame {} differs", frame);
            }
        }
    }
}
