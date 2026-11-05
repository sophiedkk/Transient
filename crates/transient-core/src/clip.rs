//! One clip: moving, trimming, splitting and fading it, and how clips stacked
//! on one track hide each other.
//!
//! Clips on one track are stacked like sheets of paper, as in Ardour: the order
//! of `Track::clips` is bottom to top, and a clip that is moved goes on top
//! ([`Project::place_clip`]). Where clips overlap you hear (and see) only the top
//! one; nothing is cut, so moving the top clip away brings the one underneath
//! back whole. [`visible_parts`] works out what remains of each clip. Where the
//! top clip fades in or out, the clip underneath keeps playing and fades the
//! other way, so the two crossfade.
//!
//! These are pure functions (a clip in, a changed clip out), so the timeline can
//! use them to preview a drag before the project changes.
//!
//! [`Project::place_clip`]: crate::project::Project::place_clip

use crate::project::{Clip, ClipId};
use crate::time::Frames;

/// The shortest a clip can be trimmed to (about 1.3 ms at 48 kHz).
pub const MIN_CLIP_LENGTH: Frames = 64;

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

/// The parts of a track's `clips` (stored bottom to top) that are heard and drawn.
///
/// Each clip is hidden where a clip above it covers it, except while the clip
/// above fades in or out: there both play, crossfading (see
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

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::project::SourceId;

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

    proptest! {
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
