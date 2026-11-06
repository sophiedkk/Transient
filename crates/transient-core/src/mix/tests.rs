//! Tests of the mixer (`super`).

use super::*;
use crate::effects::limiter::CEILING_DB;
use crate::project::Clip;
use crate::test_support::{ONE_SECOND, frames};

/// The crossfade's length in frames at the tests' 48 kHz.
fn crossfade_frames() -> usize {
    let frames = SampleRate::HZ_48000
        .seconds_to_frames(CROSSFADE_SECONDS)
        .unwrap();
    usize::try_from(frames).unwrap()
}

/// The mix of `project` without the master limiter: these tests check the
/// mixing itself, with levels far above full scale (ramps up to 99).
fn unlimited(project: &Project, pool: &AudioPool) -> Mix {
    let project = Project {
        master_limiter: false,
        ..project.clone()
    };
    Mix::new(&project, pool)
}

/// A project with one track playing a 100-frame [`ramp`] from frame 0, and
/// its audio.
fn ramp_project() -> (Project, AudioPool) {
    let mut project = Project::default();
    let track = project.import("ramp.wav".into(), 100, "r".to_owned(), 0);
    let source = project.track(track).unwrap().clips[0].source;
    (project, AudioPool::from([(source, ramp(100))]))
}

/// A buffer of `frames` frames, every sample `value`.
fn constant(value: f32, frames: usize) -> AudioBuffer {
    AudioBuffer::new(SampleRate::HZ_48000, vec![value; frames * CHANNELS]).unwrap()
}

/// A buffer whose frame n holds the value n (both channels).
fn ramp(frames: usize) -> AudioBuffer {
    let samples = (0..frames)
        .flat_map(|n| {
            let value = f32::from(u16::try_from(n).unwrap());
            [value, value]
        })
        .collect();
    AudioBuffer::new(SampleRate::HZ_48000, samples).unwrap()
}

/// Renders a copy of `mix`, so each call is a fresh start.
fn render(mix: &Mix, start: Frames, frames: usize) -> Vec<f32> {
    let mut out = vec![f32::NAN; frames * CHANNELS];
    mix.clone().render(start, &mut out);
    // Keep the left channel only, to make the expectations easy to read.
    out.iter().step_by(CHANNELS).copied().collect()
}

#[test]
fn a_clip_plays_at_its_start_position() {
    let mut project = Project::default();
    let track = project.add_track("t".to_owned());
    let source = project.add_source("ramp.wav".into(), 4);
    project.add_clip(track, source, 2).unwrap();
    let pool = AudioPool::from([(source, ramp(4))]);

    let mix = unlimited(&project, &pool);
    assert_eq!(mix.length(), 6);
    assert_eq!(render(&mix, 0, 8), [0.0, 0.0, 0.0, 1.0, 2.0, 3.0, 0.0, 0.0]);
}

#[test]
fn offset_and_length_select_part_of_the_source() {
    let mut project = Project::default();
    let track = project.add_track("t".to_owned());
    let source = project.add_source("ramp.wav".into(), 10);
    let clip = project.add_clip(track, source, 0).unwrap();
    let clip = project.tracks[0]
        .clips
        .iter_mut()
        .find(|c| c.id == clip)
        .unwrap();
    clip.offset = 3;
    clip.length = 4;
    let pool = AudioPool::from([(source, ramp(10))]);

    let mix = unlimited(&project, &pool);
    assert_eq!(render(&mix, 0, 6), [3.0, 4.0, 5.0, 6.0, 0.0, 0.0]);
}

#[test]
fn overlapping_tracks_are_added_together() {
    let mut project = Project::default();
    let a = project.import("a.wav".into(), 4, "a".to_owned(), 0);
    let b = project.import("b.wav".into(), 4, "b".to_owned(), 0);
    let source_a = project.track(a).unwrap().clips[0].source;
    let source_b = project.track(b).unwrap().clips[0].source;
    let pool = AudioPool::from([(source_a, constant(0.25, 4)), (source_b, constant(0.5, 4))]);

    let mix = unlimited(&project, &pool);
    assert_eq!(render(&mix, 2, 4), [0.75, 0.75, 0.0, 0.0]);
}

#[test]
fn an_excerpt_plays_its_stretch_of_the_mix_from_its_frame_0() {
    let (project, pool) = ramp_project();
    let mix = unlimited(&project, &pool);
    let excerpt = mix.excerpt(TimeRange { start: 40, end: 60 });
    assert_eq!(excerpt.length(), 20);
    assert_eq!(render(&excerpt, 0, 20), render(&mix, 40, 20));
    // Past the end of the mix, an excerpt is cut short.
    assert_eq!(
        mix.excerpt(TimeRange {
            start: 90,
            end: 200
        })
        .length(),
        10
    );
}

#[test]
fn rendering_starts_anywhere_and_past_the_end_is_silent() {
    let (project, pool) = ramp_project();
    let mix = unlimited(&project, &pool);

    assert_eq!(render(&mix, 97, 5), [97.0, 98.0, 99.0, 0.0, 0.0]);
    assert_eq!(render(&mix, 1000, 2), [0.0, 0.0]);
}

/// Two tracks of constant 0.5, for the mixer setting tests.
fn two_tracks() -> (Project, AudioPool) {
    let mut project = Project::default();
    let a = project.import("a.wav".into(), 4, "a".to_owned(), 0);
    let b = project.import("b.wav".into(), 4, "b".to_owned(), 0);
    let pool = [a, b]
        .iter()
        .map(|track| {
            (
                project.track(*track).unwrap().clips[0].source,
                constant(0.5, 4),
            )
        })
        .collect();
    (project, pool)
}

/// The first frame of the mix, both channels.
fn first_frame(project: &Project, pool: &AudioPool) -> [f32; 2] {
    let mut out = [0.0; 2];
    unlimited(project, pool).render(0, &mut out);
    out
}

fn assert_close(actual: [f32; 2], expected: [f32; 2]) {
    for (a, e) in actual.iter().zip(expected) {
        assert!(
            (a - e).abs() < 1e-4,
            "expected {expected:?}, got {actual:?}"
        );
    }
}

#[test]
fn decibels_become_multipliers() {
    assert!((db_to_gain(0.0) - 1.0).abs() < f32::EPSILON);
    assert!((db_to_gain(-6.0) - 0.501).abs() < 0.001);
    assert!((db_to_gain(6.0) - 1.995).abs() < 0.001);
    assert!(db_to_gain(SILENT_DB).abs() < f32::EPSILON);
}

#[test]
fn volume_and_master_volume_scale_the_mix() {
    let (mut project, pool) = two_tracks();
    assert_close(first_frame(&project, &pool), [1.0, 1.0]);
    project.tracks[0].volume_db = SILENT_DB;
    assert_close(first_frame(&project, &pool), [0.5, 0.5]);
    project.master_volume_db = -6.0;
    assert_close(first_frame(&project, &pool), [0.2506, 0.2506]);
}

#[test]
fn pan_moves_a_track_between_left_and_right() {
    let (mut project, pool) = two_tracks();
    project.tracks[0].pan = -1.0; // first track: left only
    project.tracks[1].pan = 0.5; // second: right, left at half
    assert_close(first_frame(&project, &pool), [0.5 + 0.25, 0.5]);
}

#[test]
fn mute_silences_a_track() {
    let (mut project, pool) = two_tracks();
    project.tracks[1].mute = true;
    assert_close(first_frame(&project, &pool), [0.5, 0.5]);
}

#[test]
fn solo_silences_every_other_track() {
    let (mut project, pool) = two_tracks();
    project.tracks[1].solo = true;
    assert_close(first_frame(&project, &pool), [0.5, 0.5]);
    // A muted soloed track stays silent.
    project.tracks[1].mute = true;
    assert_close(first_frame(&project, &pool), [0.0, 0.0]);
}

#[test]
fn clip_gain_scales_the_clip() {
    let (mut project, pool) = two_tracks();
    project.tracks[1].clips[0].gain_db = SILENT_DB;
    assert_close(first_frame(&project, &pool), [0.5, 0.5]);
}

#[test]
fn fades_start_and_end_in_silence() {
    let mut project = Project::default();
    let track = project.import("a.wav".into(), 100, "a".to_owned(), 0);
    let source = project.track(track).unwrap().clips[0].source;
    project.tracks[0].clips[0] = project.tracks[0].clips[0].with_fades(10, 20);
    let pool = AudioPool::from([(source, constant(1.0, 100))]);
    let mix = unlimited(&project, &pool);
    let level = render(&mix, 0, 100);

    assert!(level[0].abs() < 1e-6, "silent at the very start");
    assert!(
        (level[5] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-3,
        "halfway through the fade-in"
    );
    assert!((level[50] - 1.0).abs() < 1e-6, "full volume in the middle");
    assert!(
        (level[90] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-3,
        "halfway through the fade-out"
    );
    assert!(level[99] < 0.08, "nearly silent at the end");
}

#[test]
fn an_equal_power_crossfade_keeps_the_power_constant() {
    for step in 0..=10 {
        let t = f64::from(step) / 10.0;
        let fading_in = fade_shape(t);
        let fading_out = fade_shape(1.0 - t);
        let power = fading_in.powi(2) + fading_out.powi(2);
        assert!((power - 1.0).abs() < 1e-9, "at {t}: power {power}");
    }
}

#[test]
fn a_source_without_audio_is_silent() {
    let mut project = Project::default();
    project.import("missing.wav".into(), 10, "m".to_owned(), 0);
    let mix = unlimited(&project, &AudioPool::new());
    assert_eq!(render(&mix, 0, 3), [0.0, 0.0, 0.0]);
}

/// One second of constant 0.5 on one track, with the clip at `gain_db`.
fn constant_project(gain_db: f32) -> (Project, AudioPool) {
    let mut project = Project::default();
    let track = project.import("c.wav".into(), frames(ONE_SECOND), "c".to_owned(), 0);
    let clip = project.track(track).unwrap().clips[0];
    project.change_clip(clip.id, |clip| Clip { gain_db, ..clip });
    let pool = AudioPool::from([(clip.source, constant(0.5, ONE_SECOND))]);
    (project, pool)
}

/// Plays 1000 frames at 0 dB, then switches to a mix at `gain_db` as an
/// edit during playback does, and plays 2000 more (left channel).
fn switch_during_playback(gain_db: f32) -> Vec<f32> {
    let (project, pool) = constant_project(0.0);
    let mut old = unlimited(&project, &pool);
    let mut out = vec![0.0; 1000 * CHANNELS];
    old.render(0, &mut out);
    let (project, pool) = constant_project(gain_db);
    let mut new = unlimited(&project, &pool);
    new.continue_from(old);
    let mut out = vec![0.0; 2000 * CHANNELS];
    new.render(1000, &mut out);
    out.into_iter().step_by(CHANNELS).collect()
}

#[test]
fn a_change_during_playback_fades_over_instead_of_jumping() {
    let after = switch_during_playback(-12.0);
    let target = 0.5 * db_to_gain(-12.0);
    // It starts where the old mix was, and ends at the new level after
    // the crossfade (20 ms)...
    assert!((after[0] - 0.5).abs() < 1e-3, "{}", after[0]);
    let done = crossfade_frames();
    assert!((after[done] - target).abs() < 1e-6, "{}", after[done]);
    assert!(
        after[done..]
            .iter()
            .all(|sample| (sample - target).abs() < 1e-6)
    );
    // ...in steps far too small to click.
    let largest_step = after
        .windows(2)
        .map(|pair| (pair[1] - pair[0]).abs())
        .fold(0.0, f32::max);
    assert!(largest_step < 0.001, "{largest_step}");
}

#[test]
fn a_crossfade_spans_several_renders() {
    let (project, pool) = constant_project(0.0);
    let mut old = unlimited(&project, &pool);
    old.render(0, &mut [0.0; 2 * CHANNELS]);
    let (project, pool) = constant_project(SILENT_DB);
    let mut new = unlimited(&project, &pool);
    new.continue_from(old);
    // Small pieces, as a player with a small buffer asks for them.
    let mut pieces = Vec::new();
    for start in (2..1002).step_by(100) {
        let mut out = [0.0; 100 * CHANNELS];
        new.render(start, &mut out);
        pieces.extend(out.into_iter().step_by(CHANNELS));
    }
    assert!(pieces.windows(2).all(|pair| pair[1] <= pair[0]));
    let done = crossfade_frames();
    assert!(pieces[done - 1].abs() < 0.001 && pieces[done..].iter().all(|s| *s == 0.0));
}

#[test]
fn a_jump_ends_the_crossfade() {
    let (project, pool) = constant_project(0.0);
    let mut old = unlimited(&project, &pool);
    old.render(0, &mut [0.0; 1000 * CHANNELS]);
    let (project, pool) = constant_project(-12.0);
    let mut new = unlimited(&project, &pool);
    new.continue_from(old);
    // A seek right after the edit plays only the new mix.
    let mut out = [0.0; 2 * CHANNELS];
    new.render(30_000, &mut out);
    assert!((out[0] - 0.5 * db_to_gain(-12.0)).abs() < 1e-6);
}

#[test]
fn a_mix_that_never_played_has_nothing_to_fade_from() {
    let (project, pool) = constant_project(0.0);
    let old = unlimited(&project, &pool);
    let (project, pool) = constant_project(-12.0);
    let mut new = unlimited(&project, &pool);
    new.continue_from(old);
    assert!((first_frame_of(&mut new) - 0.5 * db_to_gain(-12.0)).abs() < 1e-6);
}

fn first_frame_of(mix: &mut Mix) -> f32 {
    let mut out = [0.0; CHANNELS];
    mix.render(0, &mut out);
    out[0]
}

/// Renders `frames` frames of `mix` from `start` (both channels).
fn render_all(mix: &mut Mix, start: Frames, frames: usize) -> Vec<f32> {
    let mut out = vec![0.0; frames * CHANNELS];
    mix.render(start, &mut out);
    out
}

/// Two tracks of a 1000-frame ramp from 0 to 0.999 each, adding up to almost
/// 2: twice full scale at the end.
fn loud_project() -> (Project, AudioPool) {
    let mut project = Project::default();
    let ramp: Vec<f32> = (0..1000)
        .flat_map(|n| [f32::from(u16::try_from(n).unwrap()) / 1000.0; CHANNELS])
        .collect();
    let audio = AudioBuffer::new(SampleRate::HZ_48000, ramp).unwrap();
    let mut pool = AudioPool::new();
    for name in ["a", "b"] {
        let track = project.import(format!("{name}.wav").into(), 1000, name.to_owned(), 0);
        pool.insert(project.track(track).unwrap().clips[0].source, audio.clone());
    }
    (project, pool)
}

#[test]
fn the_master_limiter_keeps_the_mix_under_its_ceiling() {
    let (project, pool) = loud_project();
    let out = render_all(&mut Mix::new(&project, &pool), 0, 1000);
    let ceiling = amplitude_from_db(CEILING_DB);
    assert!(out.iter().all(|sample| sample.abs() <= ceiling * 1.000_01));
    // Without it, the same mix goes far over.
    let out = render_all(&mut unlimited(&project, &pool), 0, 1000);
    assert!(out.iter().any(|sample| *sample > 1.5));
}

#[test]
fn under_the_ceiling_the_limiter_changes_nothing_and_adds_no_delay() {
    // One track at 0.5, well under the ceiling (two would add up to 1.0).
    let (project, pool) = constant_project(0.0);
    let limited = render_all(&mut Mix::new(&project, &pool), 0, 1000);
    let unlimited = render_all(&mut unlimited(&project, &pool), 0, 1000);
    assert_eq!(limited, unlimited);
}

#[test]
fn with_the_limiter_pieces_and_jumps_match_one_go() {
    let (project, pool) = loud_project();
    let whole = render_all(&mut Mix::new(&project, &pool), 0, 1000);
    let mut mix = Mix::new(&project, &pool);
    let pieces: Vec<f32> = (0..10)
        .flat_map(|piece| render_all(&mut mix, piece * 100, 100))
        .collect();
    assert_eq!(whole, pieces);
    // A jump (a seek) to frame 500 sounds as if it had played all along.
    let jumped = render_all(&mut Mix::new(&project, &pool), 500, 100);
    let expected = whole.get(500 * CHANNELS..600 * CHANNELS).unwrap();
    let difference = jumped
        .iter()
        .zip(expected)
        .fold(0.0_f32, |max, (a, b)| max.max((a - b).abs()));
    assert!(difference < 0.01, "{difference}");
}
