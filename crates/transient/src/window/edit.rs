//! Editing: selection, the edit actions, and undo/redo.
//!
//! Every change goes through [`Window::apply_edit`], which saves the project as it
//! was in the undo history first. So everything is undoable, which is why
//! deleting needs no confirmation dialog: a toast offers "Undo" instead
//! (GNOME HIG, "Be considerate").

use adw::prelude::*;
use adw::subclass::prelude::*;
use gettextrs::gettext;
use transient_core::project::{Clip, ClipId, Project, TrackId};
use transient_core::time::Frames;

use super::Window;

/// Ctrl+Left/Right moves the selected clip by this many pixels at the current zoom.
const NUDGE_PIXELS: f64 = 10.0;

impl Window {
    /// Connects the timeline's editing signals. Called once, from `constructed`.
    pub(super) fn setup_editing(&self) {
        let timeline = &self.imp().timeline;
        let weak = self.downgrade();
        timeline.connect_clip_selected(move |_, clip| {
            if let Some(window) = weak.upgrade() {
                // A click selects a clip or nothing; either way, not a stretch of time.
                window.set_time_selection(None);
                window.select(clip);
            }
        });
        let weak = self.downgrade();
        timeline.connect_clip_changed(move |_, clip, lane| {
            if let Some(window) = weak.upgrade() {
                window.apply_edit(|project| {
                    // Dropped below the last track: the clip gets a new track.
                    let Some(track) = project.tracks.get(lane).map(|track| track.id) else {
                        let name = new_track_name(project, &clip);
                        return project.move_clip_to_new_track(clip, name).is_some();
                    };
                    // The moved clip goes on top of the clips it overlaps; a track
                    // left empty goes away (in the same undo step).
                    project.move_clip(clip, track)
                });
            }
        });
        self.update_edit_actions();
    }

    /// Changes the project with `edit`, which returns whether it changed anything.
    /// If it did, the old project goes into the undo history. Returns whether it did.
    pub(super) fn apply_edit(&self, edit: impl FnOnce(&mut Project) -> bool) -> bool {
        let imp = self.imp();
        let before = imp.project.borrow().clone();
        let changed = edit(&mut imp.project.borrow_mut());
        if changed {
            self.end_mixer_coalescing();
            imp.history.borrow_mut().record(before);
            self.project_changed();
        }
        changed
    }

    /// Selects a clip (or nothing).
    pub(crate) fn select(&self, clip: Option<ClipId>) {
        // A clip selection and a time selection never exist at the same time.
        if clip.is_some() && self.imp().time_selection.take().is_some() {
            self.show_time_selection();
        }
        self.imp().selected.set(clip);
        self.imp().timeline.set_selection(clip);
        self.update_edit_actions();
        self.update_clip_pane();
    }

    /// Steps back in the history (`win.undo`, Ctrl+Z).
    pub(super) fn undo(&self) {
        let imp = self.imp();
        let current = imp.project.borrow().clone();
        let previous = imp.history.borrow_mut().undo(current);
        if let Some(previous) = previous {
            imp.project.replace(previous);
            self.project_changed();
        }
        // An "Undo" toast is pointless once the change is undone.
        if let Some(toast) = imp.undo_toast.take() {
            toast.dismiss();
        }
    }

    /// Steps forward again (`win.redo`, Shift+Ctrl+Z).
    pub(super) fn redo(&self) {
        let imp = self.imp();
        let current = imp.project.borrow().clone();
        let next = imp.history.borrow_mut().redo(current);
        if let Some(next) = next {
            imp.project.replace(next);
            self.project_changed();
        }
    }

    /// Deletes the time selection, or else the selected clip (`win.delete-clip`,
    /// Delete), with an Undo toast.
    pub(super) fn delete_selected_clip(&self) {
        if self.delete_time_selection() {
            return;
        }
        let Some(clip) = self.imp().selected.get() else {
            return;
        };
        self.apply_edit(|project| project.remove_clip(clip).is_some());
        self.select(None);
        self.show_undo_toast(&gettext("Clip deleted"));
    }

    /// Clears the clip and the time selection (`win.deselect`, Escape).
    pub(super) fn deselect_all(&self) {
        self.set_time_selection(None);
        self.select(None);
    }

    /// Moves the selected clip to a new track of its own (`win.move-to-new-track`,
    /// also in the clip pane): the keyboard way of dragging it below the last track.
    pub(super) fn move_selected_clip_to_new_track(&self) {
        let Some(id) = self.imp().selected.get() else {
            return;
        };
        self.apply_edit(|project| {
            let Some(clip) = project.clip(id).map(|(_, clip)| *clip) else {
                return false;
            };
            let name = new_track_name(project, &clip);
            project.move_clip_to_new_track(clip, name).is_some()
        });
    }

    /// Removes a track (the header's Remove Track), with an Undo toast.
    pub(super) fn remove_track(&self, track: TrackId) {
        if self.apply_edit(|project| project.remove_track(track).is_some()) {
            self.show_undo_toast(&gettext("Track removed"));
        }
    }

    /// Removes the selected clip's track (`win.remove-track`).
    pub(super) fn remove_selected_track(&self) {
        let Some(clip) = self.imp().selected.get() else {
            return;
        };
        let track = self
            .imp()
            .project
            .borrow()
            .clip(clip)
            .map(|(track, _)| track);
        if let Some(track) = track {
            self.remove_track(track);
        }
    }

    /// Splits at the playhead (`win.split`, S): the selected clip if the playhead
    /// is inside it, otherwise every clip under the playhead.
    pub(super) fn split_at_playhead(&self) {
        // With a time selection, S splits at both of its ends instead.
        if self.split_time_selection() {
            return;
        }
        let Some(at) = self.playhead_position() else {
            return;
        };
        let selected = self.imp().selected.get();
        self.apply_edit(|project| {
            let under_playhead = project.clips_at(at);
            let targets = match selected {
                Some(clip) if under_playhead.contains(&clip) => vec![clip],
                _ => under_playhead,
            };
            let mut changed = false;
            for clip in targets {
                changed |= project.split_clip(clip, at).is_some();
            }
            changed
        });
    }

    /// Selects the topmost clip under the playhead (`win.select-clip-at-playhead`).
    pub(super) fn select_clip_at_playhead(&self) {
        let Some(at) = self.playhead_position() else {
            return;
        };
        let clip = self.imp().project.borrow().clips_at(at).first().copied();
        self.select(clip);
    }

    /// Moves the selected clip a little left or right (Ctrl+Left/Right).
    pub(super) fn nudge_selected_clip(&self, direction: i8) {
        let Some(id) = self.imp().selected.get() else {
            return;
        };
        let step = NUDGE_PIXELS * self.imp().timeline.frames_per_pixel();
        let step = transient_core::time::frames_from_f64(step.max(1.0)).unwrap_or(1);
        self.apply_edit(|project| {
            let Some((track, clip)) = project.clip(id).map(|(track, clip)| (track, *clip)) else {
                return false;
            };
            let start = if direction < 0 {
                clip.start.saturating_sub(step)
            } else {
                clip.start.saturating_add(step)
            };
            start != clip.start && project.place_clip(Clip::moved_to(clip, start), track)
        });
    }

    /// Mutes or unmutes the selected clip's track (`win.toggle-mute`, M).
    pub(super) fn toggle_mute(&self) {
        self.toggle_selected_track(|track| track.mute = !track.mute);
    }

    /// Solos or unsolos the selected clip's track (`win.toggle-solo`, Shift+M).
    pub(super) fn toggle_solo(&self) {
        self.toggle_selected_track(|track| track.solo = !track.solo);
    }

    fn toggle_selected_track(&self, toggle: impl FnOnce(&mut transient_core::project::Track)) {
        let Some(clip) = self.imp().selected.get() else {
            return;
        };
        self.apply_edit(|project| {
            let Some(track) = project.clip(clip).map(|(track, _)| track) else {
                return false;
            };
            project.track_mut(track).map(toggle).is_some()
        });
    }

    /// Enables the edit actions that make sense right now. Called after every
    /// change, selection and undo/redo.
    pub(super) fn update_edit_actions(&self) {
        let imp = self.imp();
        // A selected clip can disappear through undo; drop such a selection.
        if let Some(clip) = imp.selected.get()
            && imp.project.borrow().clip(clip).is_none()
        {
            imp.selected.set(None);
            imp.timeline.set_selection(None);
        }
        let has_clip = imp.selected.get().is_some();
        let has_time = imp.time_selection.borrow().is_some();
        let has_selection = has_clip || has_time;
        let has_clips = imp.project.borrow().length() > 0;
        let history = imp.history.borrow();
        self.action_set_enabled("win.undo", history.can_undo());
        self.action_set_enabled("win.redo", history.can_redo());
        self.action_set_enabled("win.delete-clip", has_selection);
        self.action_set_enabled("win.ripple-delete", has_selection);
        self.action_set_enabled("win.deselect", has_selection);
        self.action_set_enabled("win.select-all", has_clips);
        // These act on the selected clip only.
        self.action_set_enabled("win.remove-track", has_clip);
        self.action_set_enabled("win.nudge-left", has_clip);
        self.action_set_enabled("win.nudge-right", has_clip);
        self.action_set_enabled("win.toggle-mute", has_clip);
        self.action_set_enabled("win.toggle-solo", has_clip);
        for action in [
            "win.move-to-new-track",
            "win.normalize-peak",
            "win.normalize-loudness",
            "win.fade-in-to-playhead",
            "win.fade-out-from-playhead",
        ] {
            self.action_set_enabled(action, has_clip);
        }
        self.action_set_enabled("win.split", has_clips);
        self.update_edit_tooltips(has_clip, has_time, has_clips);
        self.action_set_enabled("win.select-clip-at-playhead", has_clips);
    }

    /// Tooltips for the Split and Delete buttons in the bottom bar: they say what
    /// a click does now, and when it can do nothing, how to select something.
    fn update_edit_tooltips(&self, has_clip: bool, has_time: bool, has_clips: bool) {
        let imp = self.imp();
        let split = if has_time {
            gettext("Split at Selection Edges")
        } else if has_clips {
            gettext("Split at Playhead")
        } else {
            gettext("Import audio to split it")
        };
        imp.split_button.set_tooltip_text(Some(&split));
        let delete = if has_time {
            gettext("Delete Selection")
        } else if has_clip {
            gettext("Delete Clip")
        } else {
            gettext("Select a clip, or drag over a waveform to select time")
        };
        imp.delete_button.set_tooltip_text(Some(&delete));
    }

    /// The playhead position, if something is loaded.
    pub(super) fn playhead_position(&self) -> Option<Frames> {
        self.imp()
            .player
            .borrow()
            .as_ref()
            .map(transient_engine::player::Player::position)
    }
}

/// The name for a new track holding `clip`: the clip's file name, as on import.
fn new_track_name(project: &Project, clip: &Clip) -> String {
    super::project::clip_file_name(project, clip).unwrap_or_else(|| gettext("Track"))
}
