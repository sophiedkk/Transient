//! Helpers shared by the unit tests of several modules (only built for tests).

use crate::audio::CHANNELS;
use crate::convert::f32_from_f64;
use crate::mix::{AudioPool, Mix};
use crate::project::{Clip, Project, TrackId};
use crate::range::TimeRange;
use crate::time::Frames;

/// Renders the whole project and keeps the left channel, to keep expectations
/// easy to read.
pub(crate) fn render(project: &Project, pool: &AudioPool) -> Vec<f32> {
    // Without the master limiter: these tests use levels above full scale.
    let project = Project {
        master_limiter: false,
        ..project.clone()
    };
    let mut mix = Mix::new(&project, pool);
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

/// A stereo sine of `frequency` Hz, one second long at 48 kHz, at full scale.
pub(crate) fn sine(frequency: f64) -> Vec<f32> {
    (0..48_000)
        .flat_map(|n| {
            let value = (std::f64::consts::TAU * frequency * f64::from(n) / 48_000.0).sin();
            [f32_from_f64(value); CHANNELS]
        })
        .collect()
}

/// The peak of the left channel over the last half second of a one-second
/// signal (after a filter has settled).
pub(crate) fn settled_peak(samples: &[f32]) -> f32 {
    samples
        .iter()
        .step_by(CHANNELS)
        .skip(24_000)
        .fold(0.0, |peak, sample| peak.max(sample.abs()))
}

/// The RMS (average power) of the left channel over the last half second of
/// a one-second signal. Unlike the peak it does not depend on where the
/// samples fall on the wave, which matters at high frequencies (six samples
/// per wave at 8 kHz) after a filter shifted the wave.
pub(crate) fn settled_rms(samples: &[f32]) -> f32 {
    let settled: Vec<f32> = samples
        .iter()
        .step_by(CHANNELS)
        .skip(24_000)
        .copied()
        .collect();
    let power = settled.iter().map(|sample| sample * sample).sum::<f32>();
    (power / f32_from_f64(crate::convert::f64_from_usize(settled.len().max(1)))).sqrt()
}
