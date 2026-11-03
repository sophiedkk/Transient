//! The open project, and keeping the window in sync with it.
//!
//! The window owns a [`Project`] (what is saved) and the decoded audio for its
//! sources (what is played and drawn). After every change, [`Window::project_changed`]
//! rebuilds what depends on it: the track headers, the timeline lanes, the
//! player's mix and the title.

use std::collections::HashMap;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gettextrs::{gettext, ngettext};
use tracing::warn;
use transient_core::mix::{AudioPool, Mix};
use transient_core::project::{Project, SourceId};
use transient_core::time::format_position;
use transient_core::waveform::Waveform;
use transient_engine::decode::SourceFormat;
use transient_engine::player::Player;

use super::Window;
use crate::widgets::timeline_view::{Lane, LaneClip};
use crate::widgets::track_header::TrackHeader;

/// A source's decoded audio, with what the window shows about it.
#[derive(Debug, Clone)]
pub(super) struct LoadedSource {
    /// The audio and its peak overview.
    pub(super) waveform: Waveform,
    /// The file's original format, if GStreamer reported it.
    pub(super) format: Option<SourceFormat>,
}

/// Decoded audio by source.
pub(super) type LoadedSources = HashMap<SourceId, LoadedSource>;

impl Window {
    /// Updates everything that depends on the project. Call after every change.
    pub(super) fn project_changed(&self) {
        let imp = self.imp();
        self.update_mix();
        self.end_mixer_coalescing();
        self.show_master_volume();
        let project = imp.project.borrow().clone();
        let sources = imp.sources.borrow();
        self.rebuild_track_headers(&project, &sources);
        imp.timeline.set_lanes(
            lanes(&project, &sources),
            project.sample_rate,
            project.length(),
        );

        let length = format_position(project.length(), project.sample_rate);
        let tracks = project.tracks.len();
        imp.header_title.set_title(&gettext("Untitled Project"));
        imp.header_title.set_subtitle(&format!(
            "{} · {length}",
            ngettext(
                "{} track",
                "{} tracks",
                u32::try_from(tracks).unwrap_or(u32::MAX)
            )
            .replace("{}", &tracks.to_string())
        ));
        imp.duration_label.set_label(&format!("/ {length}"));
        imp.stack
            .set_visible_child_name(if tracks == 0 { "empty" } else { "project" });
        let has_audio = project.length() > 0;
        imp.toolbar_view.set_reveal_bottom_bars(has_audio);
        self.set_playback_enabled(has_audio);
        self.update_edit_actions();
        self.update_clip_pane();
    }

    /// Redraws the timeline lanes (for example after mute or solo changed),
    /// without touching the track headers.
    pub(super) fn refresh_lanes(&self) {
        let imp = self.imp();
        let project = imp.project.borrow();
        imp.timeline.set_lanes(
            lanes(&project, &imp.sources.borrow()),
            project.sample_rate,
            project.length(),
        );
    }

    /// Gives the player the project's current mix, creating the player the first
    /// time. Enough after a mixer change; structural changes use `project_changed`.
    pub(super) fn update_mix(&self) {
        let imp = self.imp();
        let project = imp.project.borrow();
        let sources = imp.sources.borrow();
        let pool: AudioPool = sources
            .iter()
            .map(|(id, source)| (*id, source.waveform.buffer().clone()))
            .collect();
        let mix = Mix::new(&project, &pool);
        let mut player = self.imp().player.borrow_mut();
        if let Some(player) = player.as_mut() {
            player.set_mix(mix);
            return;
        }
        if mix.length() == 0 {
            return;
        }
        match Player::new(mix, self.imp().audio_output.get()) {
            Ok(new_player) => *player = Some(new_player),
            Err(error) => {
                warn!("could not create the player: {error}");
                self.show_toast(&gettext("Playback is not available"));
            }
        }
    }

    /// One header per track, next to its lane in the timeline. Existing headers
    /// are reused when the number of tracks is the same, so a slider being
    /// dragged is never replaced under the pointer.
    fn rebuild_track_headers(&self, project: &Project, sources: &LoadedSources) {
        let container = &self.imp().track_headers;
        let mut existing = Vec::new();
        let mut child = container.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            if let Ok(header) = widget.downcast::<TrackHeader>() {
                existing.push(header);
            }
        }
        if existing.len() != project.tracks.len() {
            for header in existing.drain(..) {
                container.remove(&header);
            }
            for _ in &project.tracks {
                let header = TrackHeader::new();
                let weak = self.downgrade();
                header.connect_changed(move |track, settings| {
                    if let Some(window) = weak.upgrade() {
                        window.apply_track_settings(track, settings);
                    }
                });
                container.append(&header);
                existing.push(header);
            }
        }
        for (header, track) in existing.iter().zip(&project.tracks) {
            // The format of the track's first clip, for example "44.1 kHz · Stereo".
            let format = track
                .clips
                .first()
                .and_then(|clip| sources.get(&clip.source))
                .map(|source| super::import::describe_format(source.format))
                .unwrap_or_default();
            header.set_track(track, &format);
        }
    }
}

/// The timeline's view of the project: one lane per track.
fn lanes(project: &Project, sources: &LoadedSources) -> Vec<Lane> {
    let any_solo = project.tracks.iter().any(|track| track.solo);
    project
        .tracks
        .iter()
        .map(|track| Lane {
            audible: !track.mute && (track.solo || !any_solo),
            clips: track
                .clips
                .iter()
                .filter_map(|clip| {
                    Some(LaneClip {
                        clip: *clip,
                        waveform: sources.get(&clip.source)?.waveform.clone(),
                        name: track.name.clone(),
                    })
                })
                .collect(),
        })
        .collect()
}
