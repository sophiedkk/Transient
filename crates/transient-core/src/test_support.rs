//! Helpers shared by the unit tests of several modules (only built for tests).

use crate::mix::{AudioPool, Mix};
use crate::project::{Clip, Project, TrackId};
use crate::range::TimeRange;
use crate::time::Frames;

/// Renders the whole project and keeps the left channel, to keep expectations
/// easy to read.
pub(crate) fn render(project: &Project, pool: &AudioPool) -> Vec<f32> {
    let mix = Mix::new(project, pool);
    let mut out = vec![0.0; usize::try_from(mix.length()).unwrap() * 2];
    mix.render(0, &mut out);
    out.into_iter().step_by(2).collect()
}

/// The clips on `track`, bottom to top.
pub(crate) fn clips(project: &Project, track: TrackId) -> Vec<Clip> {
    project.track(track).unwrap().clips.clone()
}

/// The time range `start..end`.
pub(crate) fn range(start: Frames, end: Frames) -> TimeRange {
    TimeRange { start, end }
}
