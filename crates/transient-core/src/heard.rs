//! A track as it is heard: its clips with their gain and fades, through its
//! effects, before the track's volume and pan.
//!
//! The timeline draws this instead of the plain source files, so what you see
//! matches what you hear: a fade, a compressor evening out the loud parts, a
//! noise gate silencing the pauses. It is rendered by the same [`Mix`] that
//! plays the project, so the picture cannot disagree with the sound.

use crate::audio::CHANNELS;
use crate::mix::{AudioPool, Mix};
use crate::project::{Project, Track, TrackId};
use crate::time::Frames;
use crate::waveform::{Overview, OverviewBuilder};

/// How many frames to render at a time (about 1.4 s at 48 kHz).
const PIECE_FRAMES: usize = 65_536;

/// The overview of `track` as heard, from timeline frame 0 to the end of its
/// last clip, or `None` if the project has no such track.
///
/// Its volume, pan, mute and solo are left out (the lane shows those in
/// other ways), and so is the master limiter. Rendering takes about as long
/// as exporting the track, so call it on a background thread.
#[must_use]
pub fn track_overview(project: &Project, pool: &AudioPool, track: TrackId) -> Option<Overview> {
    let track = project.track(track)?;
    let alone = Project {
        tracks: vec![Track {
            volume_db: 0.0,
            pan: 0.0,
            mute: false,
            solo: false,
            ..track.clone()
        }],
        master_volume_db: 0.0,
        master_limiter: false,
        ..project.clone()
    };
    let mut mix = Mix::new(&alone, pool);
    let length = mix.length();
    let mut builder = OverviewBuilder::default();
    let mut piece = vec![0.0; PIECE_FRAMES * CHANNELS];
    let mut position: Frames = 0;
    while position < length {
        let frames = (length - position).min(Frames::try_from(PIECE_FRAMES).unwrap_or(1));
        let samples = piece.get_mut(..usize::try_from(frames).ok()? * CHANNELS)?;
        mix.render(position, samples);
        builder.push(samples);
        position += frames;
    }
    Some(builder.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::AudioBuffer;
    use crate::project::Clip;
    use crate::test_support::{ONE_SECOND, frames};
    use crate::time::SampleRate;

    /// The level of the test clip's audio.
    const LEVEL: f32 = 0.8;
    /// The test clip's gain, in dB.
    const CLIP_GAIN_DB: f32 = -6.0;

    /// One track with a clip of constant [`LEVEL`], two seconds long, its clip
    /// gain at [`CLIP_GAIN_DB`], a one-second fade-out, and the track turned
    /// down and muted.
    fn project() -> (Project, AudioPool, TrackId) {
        let mut project = Project::default();
        let second = frames(ONE_SECOND);
        let track = project.import("c.wav".into(), 2 * second, "c".to_owned(), 0);
        let clip = project.track(track).unwrap().clips[0];
        project.change_clip(clip.id, |clip| Clip {
            gain_db: CLIP_GAIN_DB,
            ..clip.with_fades(0, second)
        });
        let settings = project.track_mut(track).unwrap();
        settings.volume_db = -20.0;
        settings.mute = true;
        let samples = vec![LEVEL; 2 * ONE_SECOND * CHANNELS];
        let audio = AudioBuffer::new(SampleRate::HZ_48000, samples).unwrap();
        (project, AudioPool::from([(clip.source, audio)]), track)
    }

    /// The highest value in the overview's column covering `frames`.
    fn level(overview: &Overview, frames: std::ops::Range<Frames>) -> f32 {
        let width = crate::time::frames_to_f64(frames.end - frames.start);
        let start = crate::time::frames_to_f64(frames.start);
        overview.peaks_within(frames, start, width, 1)[0]
            .unwrap()
            .max
    }

    #[test]
    fn it_shows_clip_gain_and_fades_but_not_track_volume_or_mute() {
        let (project, pool, track) = project();
        let overview = track_overview(&project, &pool, track).unwrap();
        let expected = crate::audio::amplitude_from_db(CLIP_GAIN_DB) * LEVEL;
        let second = frames(ONE_SECOND);
        // The first second: the clip gain, not the track's volume or mute.
        assert!((level(&overview, 0..second) - expected).abs() < 1e-4);
        // The last 1000 frames of the fade-out: almost silent.
        assert!(level(&overview, 2 * second - 1000..2 * second) < 0.05 * expected);
    }

    #[test]
    fn an_unknown_track_has_no_overview() {
        let (project, pool, _) = project();
        assert!(track_overview(&project, &pool, TrackId(999)).is_none());
    }
}
