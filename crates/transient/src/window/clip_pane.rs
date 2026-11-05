//! The clip pane (F9): the selected clip's gain and fades, and normalizing.
//!
//! The pane is a GNOME "utility pane": it sits on the right of the timeline, can
//! be shown and hidden, and slides over the timeline on narrow windows (an
//! `AdwOverlaySplitView` with a breakpoint, see window.blp).

use adw::prelude::*;
use adw::subclass::prelude::*;
use gettextrs::gettext;
use gtk::gio;
use tracing::warn;
use transient_core::convert::f32_from_f64;
use transient_core::normalize::{LOUDNESS_TARGET_LUFS, PEAK_TARGET_DB, loudness_gain, peak_gain};
use transient_core::project::{Clip, ClipId};
use transient_core::time::frames_from_f64;

use super::Window;
use super::mixer::MixerControl;

/// Which normalization to apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Normalize {
    /// The loudest sample to −1 dBFS.
    Peak,
    /// The loudness to −16 LUFS, without the peak passing −1 dBFS.
    Loudness,
}

impl Window {
    /// Connects the pane's controls. Called once, from `constructed`.
    pub(super) fn setup_clip_pane(&self) {
        let imp = self.imp();
        let weak = self.downgrade();
        imp.gain_row.connect_value_notify(move |row| {
            if let Some(window) = weak.upgrade() {
                let gain = f32_from_f64(row.value());
                window.change_selected_clip(MixerControl::ClipGain, |clip| Clip {
                    gain_db: gain,
                    ..clip
                });
            }
        });
        let weak = self.downgrade();
        imp.fade_in_row.connect_value_notify(move |row| {
            if let Some(window) = weak.upgrade() {
                let frames = window.seconds_to_frames(row.value());
                window.change_selected_clip(MixerControl::FadeIn, |clip| {
                    clip.with_fades(frames, clip.fade_out)
                });
            }
        });
        let weak = self.downgrade();
        imp.fade_out_row.connect_value_notify(move |row| {
            if let Some(window) = weak.upgrade() {
                let frames = window.seconds_to_frames(row.value());
                window.change_selected_clip(MixerControl::FadeOut, |clip| {
                    clip.with_fades(clip.fade_in, frames)
                });
            }
        });
        self.update_clip_pane();
    }

    /// Shows the selected clip in the pane, or "No Clip Selected".
    pub(super) fn update_clip_pane(&self) {
        let imp = self.imp();
        let selected = imp.selected.get().and_then(|id| self.clip_with_name(id));
        let Some((clip, name)) = selected else {
            imp.pane_stack.set_visible_child_name("none");
            return;
        };
        let rate = f64::from(imp.project.borrow().sample_rate.hz());
        let length = transient_core::time::frames_to_f64(clip.length) / rate;
        imp.updating_controls.set(true);
        imp.pane_clip_group.set_title(&name);
        imp.gain_row.set_value(f64::from(clip.gain_db));
        imp.fade_in_row.adjustment().set_upper(length);
        imp.fade_out_row.adjustment().set_upper(length);
        imp.fade_in_row
            .set_value(transient_core::time::frames_to_f64(clip.fade_in) / rate);
        imp.fade_out_row
            .set_value(transient_core::time::frames_to_f64(clip.fade_out) / rate);
        imp.updating_controls.set(false);
        imp.pane_stack.set_visible_child_name("clip");
    }

    /// Shows the pane (double-clicking a clip).
    pub(super) fn show_clip_pane(&self) {
        self.imp().split_view.set_show_sidebar(true);
    }

    /// Hides the pane (clicking empty space, which deselects).
    pub(super) fn hide_clip_pane(&self) {
        self.imp().split_view.set_show_sidebar(false);
    }

    /// Shows or hides the pane (`win.toggle-clip-pane`, F9).
    pub(super) fn toggle_clip_pane(&self) {
        let split_view = &self.imp().split_view;
        split_view.set_show_sidebar(!split_view.shows_sidebar());
    }

    /// Normalizes the selected clip (`win.normalize-peak` / `win.normalize-loudness`).
    /// Measuring a long clip takes a moment, so it happens on a background thread.
    pub(super) async fn normalize_selected_clip(&self, how: Normalize) {
        let imp = self.imp();
        let Some(id) = imp.selected.get() else {
            return;
        };
        let Some(clip) = imp.project.borrow().clip(id).map(|(_, clip)| *clip) else {
            return;
        };
        let Some(audio) = imp
            .sources
            .borrow()
            .get(&clip.source)
            .map(|source| source.waveform.buffer().clone())
        else {
            return;
        };
        let measured = gio::spawn_blocking(move || match how {
            Normalize::Peak => peak_gain(&audio, &clip, PEAK_TARGET_DB).map(|gain| (gain, false)),
            Normalize::Loudness => {
                loudness_gain(&audio, &clip, LOUDNESS_TARGET_LUFS, PEAK_TARGET_DB)
                    .map(|result| (result.gain_db, result.limited))
            }
        })
        .await;
        let Ok(Some((gain_db, limited))) = measured else {
            warn!("could not measure clip {id:?} for normalizing");
            self.show_toast(&gettext("The clip is silent or too short to normalize"));
            return;
        };
        self.apply_edit(|project| project.change_clip(id, |clip| Clip { gain_db, ..clip }));
        self.update_clip_pane();
        if limited {
            self.show_toast(&gettext("Kept quieter than the target to avoid clipping"));
        }
    }

    /// Fades the selected clip in from its start to the playhead
    /// (`win.fade-in-to-playhead`).
    pub(super) fn fade_in_to_playhead(&self) {
        self.fade_to_playhead(|clip, at| {
            clip.with_fades(at.saturating_sub(clip.start), clip.fade_out)
        });
    }

    /// Fades the selected clip out from the playhead to its end
    /// (`win.fade-out-from-playhead`).
    pub(super) fn fade_out_from_playhead(&self) {
        self.fade_to_playhead(|clip, at| {
            clip.with_fades(clip.fade_in, clip.end().saturating_sub(at))
        });
    }

    fn fade_to_playhead(&self, fade: impl FnOnce(Clip, u64) -> Clip) {
        let (Some(id), Some(at)) = (self.imp().selected.get(), self.playhead_position()) else {
            return;
        };
        self.apply_edit(|project| project.change_clip(id, |clip| fade(clip, at)));
        self.update_clip_pane();
    }

    /// Changes the selected clip from the pane; coalesced like a mixer slider.
    fn change_selected_clip(
        &self,
        control: fn(ClipId) -> MixerControl,
        change: impl FnOnce(Clip) -> Clip,
    ) {
        let imp = self.imp();
        if imp.updating_controls.get() {
            return;
        }
        let Some(id) = imp.selected.get() else {
            return;
        };
        self.apply_mixer_change(Some(control(id)), |project| project.change_clip(id, change));
    }

    /// A clip and the name of its track.
    fn clip_with_name(&self, id: ClipId) -> Option<(Clip, String)> {
        let project = self.imp().project.borrow();
        let (track, clip) = project.clip(id)?;
        Some((*clip, project.track(track)?.name.clone()))
    }

    fn seconds_to_frames(&self, seconds: f64) -> u64 {
        let rate = f64::from(self.imp().project.borrow().sample_rate.hz());
        frames_from_f64(seconds.max(0.0) * rate).unwrap_or(0)
    }
}
