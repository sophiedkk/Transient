//! Which waveform a clip is drawn from: an overview as heard (gain, fades,
//! effects; see `window/heard.rs`) wherever that overview has the clip's
//! audio, shifted to where the clip is now, or else its source audio.
//!
//! Looking up the clip's *audio* rather than its place means a clip keeps its
//! look while it is moved (also to another track), split, trimmed shorter or
//! pasted, until its track is rendered again. Only audio no overview has yet
//! (a clip trimmed longer, a new file) is drawn from the source.

use std::ops::Range;

use transient_core::mix::db_to_gain;
use transient_core::project::Clip;
use transient_core::time::{Frames, frames_to_f64};
use transient_core::waveform::{Overview, Peak};

use super::LaneClip;

/// Where a clip's audio is in a track overview: the stretch of the source a
/// rendered clip played, and where that source's frame 0 fell on the
/// overview's timeline.
#[derive(Debug, Clone)]
pub(crate) struct ClipHeard {
    /// The overview of the track the audio was rendered in.
    pub(crate) overview: Overview,
    /// The source frames the rendered clip played.
    pub(crate) source_range: Range<Frames>,
    /// The overview frame of the source's frame 0 (the rendered clip's start
    /// minus its offset; can be below 0 for a clip trimmed at its start).
    pub(crate) anchor: i128,
}

/// Where to read `part` (a clip as drawn: whole, a visible part or a drag
/// preview) in `heard`'s overview: the overview frames of its audio, and how
/// far the overview is ahead of the timeline there (to add to a timeline
/// position). `None` if the overview does not have all of the part's audio.
pub(super) fn heard_frames(heard: &ClipHeard, part: &Clip) -> Option<(Range<Frames>, f64)> {
    let audio_end = part.offset.checked_add(part.length)?;
    if part.offset < heard.source_range.start || audio_end > heard.source_range.end {
        return None;
    }
    let start = Frames::try_from(heard.anchor + i128::from(part.offset)).ok()?;
    let ahead = frames_to_f64(start) - frames_to_f64(part.start);
    Some((start..start.checked_add(part.length)?, ahead))
}

/// The peaks to draw `clip` (a whole clip, a visible part or a drag preview
/// of `drawn`) with, for `columns` pixel columns from timeline frame
/// `first_frame`, and the gain to draw them with. As heard (gain, fades and
/// effects already in it) from wherever an overview has this audio, or else
/// the source audio with the clip's gain applied, so normalizing visibly
/// changes it. A clip whose audio is missing has no peaks.
pub(super) fn clip_peaks(
    drawn: &LaneClip,
    clip: &Clip,
    first_frame: f64,
    frames_per_column: f64,
    columns: usize,
) -> (Vec<Option<Peak>>, f32) {
    let Some(waveform) = drawn.waveform.as_ref() else {
        return (Vec::new(), 1.0);
    };
    if let Some(heard) = drawn.heard.as_ref()
        && let Some((frames, ahead)) = heard_frames(heard, clip)
    {
        let peaks =
            heard
                .overview
                .peaks_within(frames, first_frame + ahead, frames_per_column, columns);
        return (peaks, 1.0);
    }
    let peaks = waveform.peaks_within(
        clip.offset..clip.offset.saturating_add(clip.length),
        first_frame + frames_to_f64(clip.offset) - frames_to_f64(clip.start),
        frames_per_column,
        columns,
    );
    (peaks, db_to_gain(clip.gain_db))
}

#[cfg(test)]
mod tests {
    use transient_core::project::{ClipId, SourceId};
    use transient_core::waveform::OverviewBuilder;

    use super::*;

    /// A clip playing source frames 100..1100 at timeline frames 1000..2000.
    fn rendered() -> Clip {
        Clip {
            id: ClipId(1),
            source: SourceId(1),
            start: 1000,
            offset: 100,
            length: 1000,
            ..Clip::default()
        }
    }

    /// Where `rendered` was in a (silent) overview.
    fn heard() -> ClipHeard {
        let clip = rendered();
        ClipHeard {
            overview: OverviewBuilder::default().finish(),
            source_range: clip.offset..clip.offset + clip.length,
            anchor: i128::from(clip.start) - i128::from(clip.offset),
        }
    }

    #[test]
    fn in_place_it_reads_where_it_is() {
        let faded = rendered().with_fades(300, 200);
        assert_eq!(heard_frames(&heard(), &faded), Some((1000..2000, 0.0)));
    }

    #[test]
    fn moved_it_reads_where_it_was() {
        let moved = Clip {
            start: 5000,
            ..rendered()
        };
        assert_eq!(heard_frames(&heard(), &moved), Some((1000..2000, -4000.0)));
    }

    #[test]
    fn a_shorter_or_split_part_reads_its_own_stretch() {
        // The right half of a split, moved 500 frames later.
        let half = Clip {
            start: 2000,
            offset: 600,
            length: 500,
            ..rendered()
        };
        assert_eq!(heard_frames(&heard(), &half), Some((1500..2000, -500.0)));
    }

    #[test]
    fn audio_the_overview_does_not_have_is_not_read() {
        let longer = Clip {
            length: 1500,
            ..rendered()
        };
        let earlier_audio = Clip {
            offset: 50,
            ..rendered()
        };
        assert_eq!(heard_frames(&heard(), &longer), None);
        assert_eq!(heard_frames(&heard(), &earlier_audio), None);
    }
}
