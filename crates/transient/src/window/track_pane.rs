//! The pane's Track tab: one track's effects, next to the Clip tab (F9).
//!
//! It shows the selected clip's track. Effects in a track's menu opens it on
//! that track; that deselects the clips, so the tab keeps showing the chosen
//! track until a clip is selected again.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gettextrs::gettext;
use transient_core::convert::f32_from_f64;
use transient_core::effects::TrackEffects;
use transient_core::effects::compressor::Ratio;
use transient_core::effects::low_cut::{Slope, amount_for_frequency, frequency_for_amount};
use transient_core::effects::noise_gate::Reduction;
use transient_core::project::TrackId;

use super::Window;
use super::mixer::MixerControl;

/// The choices of the Slope row, in the order of its list in window.blp.
const SLOPES: [Slope; 3] = [Slope::Db12, Slope::Db24, Slope::Db48];
/// The choices of the noise gate's Reduction row, likewise.
const REDUCTIONS: [Reduction; 3] = [Reduction::Silence, Reduction::Strong, Reduction::Light];

/// The compressor's Ratio row, likewise.
const RATIOS: [Ratio; 3] = [Ratio::Gentle, Ratio::Medium, Ratio::Strong];

/// A level in whole decibels for a slider's value, for example "−45 dB": with
/// a minus sign (−), not a hyphen, as in the rest of the interface.
fn decibels(db: f32) -> String {
    gettext("{} dB").replace("{}", &format!("{db:.0}").replace('-', "−"))
}

/// The item of a choice row (`ComboRow::selected`) among `choices`.
fn chosen<T: Copy>(choices: &[T], selected: u32) -> Option<T> {
    usize::try_from(selected)
        .ok()
        .and_then(|index| choices.get(index).copied())
}

/// The position of `value` among `choices`, for `ComboRow::set_selected`.
fn position<T: PartialEq>(choices: &[T], value: &T) -> Option<u32> {
    choices
        .iter()
        .position(|choice| choice == value)
        .and_then(|index| u32::try_from(index).ok())
}

impl Window {
    /// Connects the pane's effect controls. Called once, from `constructed`.
    pub(super) fn setup_track_pane(&self) {
        let imp = self.imp();
        let weak = self.downgrade();
        imp.low_cut_switch.connect_active_notify(move |switch| {
            if let Some(window) = weak.upgrade() {
                let enabled = switch.is_active();
                // Switching is its own undo step (`None`), like mute and solo.
                window.change_effects(None, |effects| effects.low_cut.enabled = enabled);
            }
        });
        let weak = self.downgrade();
        imp.low_cut_slider
            .scale()
            .connect_value_changed(move |scale| {
                if let Some(window) = weak.upgrade() {
                    let frequency = frequency_for_amount(scale.value());
                    window.change_effects(Some(MixerControl::LowCut), |effects| {
                        effects.low_cut.frequency_hz = frequency;
                    });
                }
            });
        let weak = self.downgrade();
        imp.low_cut_slope_row.connect_selected_notify(move |row| {
            let slope = chosen(&SLOPES, row.selected());
            if let (Some(window), Some(slope)) = (weak.upgrade(), slope) {
                window.change_effects(None, |effects| effects.low_cut.slope = slope);
            }
        });
        let weak = self.downgrade();
        imp.noise_gate_switch.connect_active_notify(move |switch| {
            if let Some(window) = weak.upgrade() {
                let enabled = switch.is_active();
                window.change_effects(None, |effects| effects.noise_gate.enabled = enabled);
            }
        });
        let weak = self.downgrade();
        imp.noise_gate_slider
            .scale()
            .connect_value_changed(move |scale| {
                if let Some(window) = weak.upgrade() {
                    let threshold = f32_from_f64(scale.value());
                    window.change_effects(Some(MixerControl::NoiseGate), |effects| {
                        effects.noise_gate.threshold_db = threshold;
                    });
                }
            });
        let weak = self.downgrade();
        imp.noise_gate_reduction_row
            .connect_selected_notify(move |row| {
                let reduction = chosen(&REDUCTIONS, row.selected());
                if let (Some(window), Some(reduction)) = (weak.upgrade(), reduction) {
                    window.change_effects(None, |effects| effects.noise_gate.reduction = reduction);
                }
            });
        let weak = self.downgrade();
        imp.compressor_switch.connect_active_notify(move |switch| {
            if let Some(window) = weak.upgrade() {
                let enabled = switch.is_active();
                window.change_effects(None, |effects| effects.compressor.enabled = enabled);
            }
        });
        let weak = self.downgrade();
        imp.compressor_slider
            .scale()
            .connect_value_changed(move |scale| {
                if let Some(window) = weak.upgrade() {
                    let threshold = f32_from_f64(scale.value());
                    window.change_effects(Some(MixerControl::Compressor), |effects| {
                        effects.compressor.threshold_db = threshold;
                    });
                }
            });
        let weak = self.downgrade();
        imp.compressor_ratio_row
            .connect_selected_notify(move |row| {
                let ratio = chosen(&RATIOS, row.selected());
                if let (Some(window), Some(ratio)) = (weak.upgrade(), ratio) {
                    window.change_effects(None, |effects| effects.compressor.ratio = ratio);
                }
            });
    }

    /// Opens the pane on the Track tab with `track`'s effects (Effects in the
    /// track's menu).
    pub(super) fn show_track_effects(&self, track: TrackId) {
        // The Track tab follows the selected clip: deselect, so it stays on
        // the chosen track.
        self.select(None);
        let imp = self.imp();
        imp.effects_track.set(Some(track));
        self.update_pane();
        imp.pane_tabs.set_visible_child_name("track");
        imp.split_view.set_show_sidebar(true);
    }

    /// Opens the pane on the selected track's effects, or the first track's
    /// (`win.track-effects`, for scripted tests).
    pub(super) fn show_selected_track_effects(&self) {
        let track = self.selected_track().or_else(|| {
            self.imp()
                .project
                .borrow()
                .tracks
                .first()
                .map(|track| track.id)
        });
        if let Some(track) = track {
            self.show_track_effects(track);
        }
    }

    /// Shows the effects of the pane's track on the Track tab, or "No Track
    /// Selected" if there is none (or it was removed).
    pub(super) fn update_track_pane(&self) {
        let imp = self.imp();
        let track = imp
            .effects_track
            .get()
            .and_then(|id| imp.project.borrow().track(id).cloned());
        let Some(track) = track else {
            imp.effects_track.set(None);
            imp.track_stack.set_visible_child_name("none");
            return;
        };
        let low_cut = track.effects.low_cut;
        imp.updating_controls.set(true);
        imp.pane_track_group.set_title(&track.name);
        imp.low_cut_switch.set_active(low_cut.enabled);
        imp.low_cut_slider
            .scale()
            .set_value(amount_for_frequency(low_cut.frequency_hz));
        // In whole hertz.
        imp.low_cut_slider.set_value_text(
            gettext("{} Hz").replace("{}", &format!("{:.0}", low_cut.frequency_hz)),
        );
        if let Some(index) = position(&SLOPES, &low_cut.slope) {
            imp.low_cut_slope_row.set_selected(index);
        }
        let gate = track.effects.noise_gate;
        imp.noise_gate_switch.set_active(gate.enabled);
        imp.noise_gate_slider
            .scale()
            .set_value(f64::from(gate.threshold_db));
        imp.noise_gate_slider
            .set_value_text(decibels(gate.threshold_db));
        if let Some(index) = position(&REDUCTIONS, &gate.reduction) {
            imp.noise_gate_reduction_row.set_selected(index);
        }
        let compressor = track.effects.compressor;
        imp.compressor_switch.set_active(compressor.enabled);
        imp.compressor_slider
            .scale()
            .set_value(f64::from(compressor.threshold_db));
        imp.compressor_slider
            .set_value_text(decibels(compressor.threshold_db));
        if let Some(index) = position(&RATIOS, &compressor.ratio) {
            imp.compressor_ratio_row.set_selected(index);
        }
        imp.updating_controls.set(false);
        imp.track_stack.set_visible_child_name("track");
    }

    /// Changes the shown track's effects from the pane. `control` says which
    /// control (it is given the track): its steps coalesce into one undo step
    /// (see `mixer.rs`); `None` always makes a new one.
    fn change_effects(
        &self,
        control: Option<fn(TrackId) -> MixerControl>,
        change: impl FnOnce(&mut TrackEffects),
    ) {
        let imp = self.imp();
        if imp.updating_controls.get() {
            return;
        }
        let Some(id) = imp.effects_track.get() else {
            return;
        };
        self.apply_mixer_change(control.map(|control| control(id)), |project| {
            let Some(track) = project.track_mut(id) else {
                return false;
            };
            let before = track.effects;
            change(&mut track.effects);
            track.effects != before
        });
        // Shows the new value next to the slider, and the header's mark.
        self.update_track_pane();
        self.refresh_effect_marks();
    }
}
