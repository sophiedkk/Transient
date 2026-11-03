//! Mixer controls: each track's volume, pan, mute and solo, the master volume,
//! and the output level meter.
//!
//! Slider changes come in many small steps while dragging. They go into the undo
//! history *coalesced*: steps on the same slider less than a second apart count as
//! one change, so one drag is one undo step. They also only update the mix, not
//! the whole window, so the slider being dragged is never rebuilt under the pointer.

use std::time::{Duration, Instant};

use adw::prelude::*;
use adw::subclass::prelude::*;
use transient_core::project::{ClipId, Project, TrackId};

use super::Window;
use crate::widgets::track_header::TrackSettings;

/// Slider steps closer together than this are one undo step.
const COALESCE_WINDOW: Duration = Duration::from_secs(1);
/// The level meter's range: this many dB below full scale shows as empty.
const METER_FLOOR_DB: f64 = -60.0;

/// Which slider changed, for coalescing undo steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MixerControl {
    /// The master volume.
    Master,
    /// A track's volume.
    Volume(TrackId),
    /// A track's pan.
    Pan(TrackId),
    /// A clip's gain (in the clip pane).
    ClipGain(ClipId),
    /// A clip's fade-in length (in the clip pane).
    FadeIn(ClipId),
    /// A clip's fade-out length (in the clip pane).
    FadeOut(ClipId),
}

impl Window {
    /// Connects the master volume slider. Called once, from `constructed`.
    pub(super) fn setup_mixer(&self) {
        let weak = self.downgrade();
        self.imp().master_scale.connect_value_changed(move |scale| {
            let Some(window) = weak.upgrade() else {
                return;
            };
            if window.imp().updating_controls.get() {
                return;
            }
            let volume = to_f32(scale.value());
            window.apply_mixer_change(Some(MixerControl::Master), |project| {
                let changed = (project.master_volume_db - volume).abs() > f32::EPSILON;
                project.master_volume_db = volume;
                changed
            });
        });
    }

    /// Applies a track header's new settings.
    pub(super) fn apply_track_settings(&self, track: TrackId, settings: TrackSettings) {
        let Some(current) = self.imp().project.borrow().track(track).cloned() else {
            return;
        };
        // Toggling mute or solo is always its own undo step; slider drags coalesce.
        let control = if current.mute != settings.mute || current.solo != settings.solo {
            None
        } else if (current.pan - settings.pan).abs() > f32::EPSILON {
            Some(MixerControl::Pan(track))
        } else {
            Some(MixerControl::Volume(track))
        };
        self.apply_mixer_change(control, |project| {
            let Some(track) = project.track_mut(track) else {
                return false;
            };
            let unchanged = (track.volume_db - settings.volume_db).abs() <= f32::EPSILON
                && (track.pan - settings.pan).abs() <= f32::EPSILON
                && track.mute == settings.mute
                && track.solo == settings.solo;
            track.volume_db = settings.volume_db;
            track.pan = settings.pan;
            track.mute = settings.mute;
            track.solo = settings.solo;
            !unchanged
        });
    }

    /// Changes a mixer setting. `control` says which slider, for coalescing undo
    /// steps; `None` always makes a separate step.
    pub(super) fn apply_mixer_change(
        &self,
        control: Option<MixerControl>,
        change: impl FnOnce(&mut Project) -> bool,
    ) {
        let imp = self.imp();
        let before = imp.project.borrow().clone();
        if !change(&mut imp.project.borrow_mut()) {
            return;
        }
        let now = Instant::now();
        let continues_previous = matches!(
            (control, *imp.last_mixer_change.borrow()),
            (Some(control), Some((previous, at)))
                if control == previous && now.duration_since(at) < COALESCE_WINDOW
        );
        if !continues_previous {
            imp.history.borrow_mut().record(before);
        }
        imp.last_mixer_change
            .replace(control.map(|control| (control, now)));
        self.update_mix();
        self.refresh_lanes();
        self.update_edit_actions();
    }

    /// Ends slider coalescing: the next slider step starts a new undo step.
    /// Called by every other kind of change.
    pub(super) fn end_mixer_coalescing(&self) {
        self.imp().last_mixer_change.replace(None);
    }

    /// Shows the project's master volume on the slider without treating it as a change.
    pub(super) fn show_master_volume(&self) {
        let imp = self.imp();
        imp.updating_controls.set(true);
        imp.master_scale
            .set_value(f64::from(imp.project.borrow().master_volume_db));
        imp.updating_controls.set(false);
    }

    /// Updates the level meter from the player's latest measurement.
    pub(super) fn update_level_meter(&self, levels: Option<[f64; 2]>) {
        let imp = self.imp();
        let [left, right] = levels.unwrap_or([f64::NEG_INFINITY; 2]);
        let fill = |db: f64| ((db - METER_FLOOR_DB) / -METER_FLOOR_DB).clamp(0.0, 1.0);
        imp.left_meter.set_value(fill(left));
        imp.right_meter.set_value(fill(right));
        if left > 0.0 || right > 0.0 {
            imp.level_meter.add_css_class("clipping");
        } else {
            imp.level_meter.remove_css_class("clipping");
        }
    }
}

/// A slider value as `f32` (slider ranges are small).
fn to_f32(value: f64) -> f32 {
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        reason = "slider values are within -60..12, far inside f32's range"
    )]
    let value = value as f32;
    value
}
