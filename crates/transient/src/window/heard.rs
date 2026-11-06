//! Waveforms as heard: the timeline draws each track as it sounds (clip gain,
//! fades, effects), rendered by `transient_core::heard` on a background thread.
//!
//! After every change the window waits a moment ([`RENDER_DELAY`], so a slider
//! drag or a burst of clicks is one render, not hundreds), then renders every
//! track whose clips or effects changed. Only one render runs at a time: a
//! change during it asks for one more afterwards, which then covers every
//! change made meanwhile. Volume, pan and mute are not part of it, so moving those
//! renders nothing. Until the new overview arrives, each clip is drawn from
//! the last overview that has its audio, wherever the clip is now (see
//! `timeline_view::clip_waveform`): toggling an effect, moving, splitting or
//! pasting a clip never flashes back to the plain source audio.

use std::collections::HashMap;
use std::time::Duration;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};
use tracing::{debug, warn};
use transient_core::effects::TrackEffects;
use transient_core::heard::track_overview;
use transient_core::mix::AudioPool;
use transient_core::project::{Clip, SourceId, Track, TrackId};
use transient_core::waveform::Overview;

use super::Window;
use super::project::LoadedSources;
use crate::widgets::timeline_view::clip_waveform::ClipHeard;

/// How long after the last change the tracks are rendered.
const RENDER_DELAY: Duration = Duration::from_millis(250);

/// A track's overview, with what it was rendered from.
#[derive(Debug)]
pub(super) struct HeardTrack {
    clips: Vec<Clip>,
    effects: TrackEffects,
    /// The track's sources whose audio was not loaded (yet) at the time:
    /// once they arrive, the overview is out of date.
    missing: Vec<SourceId>,
    pub(super) overview: Overview,
}

/// The overviews by track.
pub(super) type HeardTracks = HashMap<TrackId, HeardTrack>;

impl HeardTrack {
    /// Whether this overview still shows `track` as it sounds now (if not,
    /// it is rendered again).
    pub(super) fn matches(&self, track: &Track, sources: &LoadedSources) -> bool {
        self.clips == track.clips
            && self.effects == track.effects
            && self.missing == missing_sources(track, sources)
    }
}

/// Where `clip`'s audio is in one of the overviews in `heard`, to draw it
/// from: the rendered clip it came from (the same clip if it is still there,
/// or else any clip that played this stretch of the same file, such as the
/// clip it was split or copied from). `None` if no overview has all of it, or
/// its audio was not loaded when rendered or is not now.
pub(super) fn clip_heard(
    heard: &HeardTracks,
    clip: &Clip,
    sources: &LoadedSources,
) -> Option<ClipHeard> {
    if !sources.contains_key(&clip.source) {
        return None;
    }
    let audio_end = clip.offset.saturating_add(clip.length);
    heard
        .values()
        .filter(|track| !track.missing.contains(&clip.source))
        .flat_map(|track| {
            track
                .clips
                .iter()
                .filter(|then| {
                    then.source == clip.source
                        && then.offset <= clip.offset
                        && then.offset.saturating_add(then.length) >= audio_end
                })
                .map(move |then| (track, then))
        })
        // The clip itself first: `false` sorts before `true`.
        .min_by_key(|(_, then)| then.id != clip.id)
        .map(|(track, then)| ClipHeard {
            overview: track.overview.clone(),
            source_range: then.offset..then.offset.saturating_add(then.length),
            anchor: i128::from(then.start) - i128::from(then.offset),
        })
}

/// The sources of `track`'s clips whose audio is not loaded, in clip order.
fn missing_sources(track: &Track, sources: &LoadedSources) -> Vec<SourceId> {
    track
        .clips
        .iter()
        .map(|clip| clip.source)
        .filter(|source| !sources.contains_key(source))
        .collect()
}

/// The setting that switches waveforms as heard on and off (Preferences).
const SETTING: &str = "heard-waveforms";

impl Window {
    /// Follows the Preferences switch: off drops the overviews (the lanes
    /// show the audio files) and stops rendering; on renders them again.
    /// Called once, from `constructed`.
    pub(super) fn setup_heard_waveforms(&self) {
        let Some(settings) = self.imp().settings.get() else {
            return;
        };
        let weak = self.downgrade();
        settings.connect_changed(Some(SETTING), move |_, _| {
            let Some(window) = weak.upgrade() else {
                return;
            };
            if window.heard_waveforms_on() {
                window.schedule_heard_waveforms();
            } else {
                window.imp().heard.borrow_mut().clear();
                window.refresh_lanes();
            }
        });
    }

    /// Whether waveforms show effects (on unless switched off in Preferences).
    fn heard_waveforms_on(&self) -> bool {
        self.imp()
            .settings
            .get()
            .is_none_or(|settings| settings.boolean(SETTING))
    }

    /// Renders the tracks that changed, a moment from now (see the module
    /// documentation). Call after every change to the project. Does nothing
    /// while switched off in Preferences.
    pub(super) fn schedule_heard_waveforms(&self) {
        let imp = self.imp();
        if let Some(pending) = imp.heard_timer.take() {
            pending.remove();
        }
        if !self.heard_waveforms_on() {
            return;
        }
        let weak = self.downgrade();
        let timer = glib::timeout_add_local_once(RENDER_DELAY, move || {
            if let Some(window) = weak.upgrade() {
                window.imp().heard_timer.take();
                glib::spawn_future_local(async move { window.render_heard_waveforms().await });
            }
        });
        imp.heard_timer.replace(Some(timer));
    }

    /// Renders the out-of-date tracks, or, if a render is already running,
    /// asks it to run once more when it is done.
    async fn render_heard_waveforms(&self) {
        let imp = self.imp();
        if imp.heard_rendering.get() {
            imp.heard_again.set(true);
            return;
        }
        imp.heard_rendering.set(true);
        loop {
            imp.heard_again.set(false);
            self.render_stale_tracks().await;
            if !imp.heard_again.get() {
                break;
            }
        }
        imp.heard_rendering.set(false);
    }

    /// Renders every track whose overview is missing or out of date, on a
    /// background thread, then redraws the lanes.
    async fn render_stale_tracks(&self) {
        let imp = self.imp();
        let project = imp.project.borrow().clone();
        let (stale, pool) = {
            let sources = imp.sources.borrow();
            let mut heard = imp.heard.borrow_mut();
            heard.retain(|id, _| project.track(*id).is_some());
            let stale: Vec<(TrackId, Vec<SourceId>)> = project
                .tracks
                .iter()
                .filter(|track| {
                    !heard
                        .get(&track.id)
                        .is_some_and(|known| known.matches(track, &sources))
                })
                .map(|track| (track.id, missing_sources(track, &sources)))
                .collect();
            let pool: AudioPool = sources
                .iter()
                .map(|(id, source)| (*id, source.waveform.buffer().clone()))
                .collect();
            (stale, pool)
        };
        if stale.is_empty() {
            return;
        }
        let count = stale.len();
        let started = std::time::Instant::now();
        let rendered = gio::spawn_blocking(move || {
            stale
                .into_iter()
                .filter_map(|(id, missing)| {
                    let track = project.track(id)?;
                    let overview = track_overview(&project, &pool, id)?;
                    Some((
                        id,
                        HeardTrack {
                            clips: track.clips.clone(),
                            effects: track.effects,
                            missing,
                            overview,
                        },
                    ))
                })
                .collect::<Vec<_>>()
        })
        .await;
        let Ok(rendered) = rendered else {
            warn!("rendering the tracks' waveforms failed");
            return;
        };
        debug!(
            "rendered {count} track waveforms as heard in {} ms",
            started.elapsed().as_millis()
        );
        {
            let project = imp.project.borrow();
            let sources = imp.sources.borrow();
            let mut heard = imp.heard.borrow_mut();
            for (id, track_heard) in rendered {
                let Some(track) = project.track(id) else {
                    continue;
                };
                // The track may have changed again while this was rendering:
                // the new result is still closer to it than an older one, but
                // never replaces an overview that is up to date.
                let up_to_date = heard
                    .get(&id)
                    .is_some_and(|known| known.matches(track, &sources));
                if !up_to_date {
                    heard.insert(id, track_heard);
                }
            }
        }
        self.refresh_lanes();
    }
}
