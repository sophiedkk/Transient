//! Editing: selection, the edit actions, and undo/redo.
//!
//! Every change goes through [`Window::apply_edit`], which saves the project as it
//! was in the undo history first. So everything is undoable, which is why
//! deleting needs no confirmation dialog: a toast offers "Undo" instead
//! (GNOME HIG, "Be considerate").

use adw::prelude::*;
use adw::subclass::prelude::*;
use gettextrs::gettext;
use transient_core::project::{Clip, ClipId, Project};
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
                window.select(clip);
            }
        });
        let weak = self.downgrade();
        timeline.connect_clip_changed(move |_, clip, lane| {
            if let Some(window) = weak.upgrade() {
                window.apply_edit(|project| {
                    let Some(track) = project.tracks.get(lane).map(|track| track.id) else {
                        return false;
                    };
                    project.update_clip(clip, track)
                });
            }
        });
        self.update_edit_actions();
    }

    /// Changes the project with `edit`, which returns whether it changed anything.
    /// If it did, the old project goes into the undo history.
    pub(super) fn apply_edit(&self, edit: impl FnOnce(&mut Project) -> bool) {
        let imp = self.imp();
        let before = imp.project.borrow().clone();
        let changed = edit(&mut imp.project.borrow_mut());
        if changed {
            imp.history.borrow_mut().record(before);
            self.project_changed();
        }
    }

    /// Selects a clip (or nothing).
    pub(crate) fn select(&self, clip: Option<ClipId>) {
        self.imp().selected.set(clip);
        self.imp().timeline.set_selection(clip);
        self.update_edit_actions();
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

    /// Deletes the selected clip (`win.delete-clip`, Delete), with an Undo toast.
    pub(super) fn delete_selected_clip(&self) {
        let Some(clip) = self.imp().selected.get() else {
            return;
        };
        self.apply_edit(|project| project.remove_clip(clip).is_some());
        self.select(None);
        let toast = adw::Toast::builder()
            .title(gettext("Clip deleted"))
            .button_label(gettext("_Undo"))
            .action_name("win.undo")
            .build();
        self.imp().toast_overlay.add_toast(toast.clone());
        self.imp().undo_toast.replace(Some(toast));
    }

    /// Splits at the playhead (`win.split`, S): the selected clip if the playhead
    /// is inside it, otherwise every clip under the playhead.
    pub(super) fn split_at_playhead(&self) {
        let Some(at) = self.playhead() else {
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
        let Some(at) = self.playhead() else {
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
            start != clip.start && project.update_clip(Clip::moved_to(clip, start), track)
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
        let has_selection = imp.selected.get().is_some();
        let has_clips = imp.project.borrow().length() > 0;
        let history = imp.history.borrow();
        self.action_set_enabled("win.undo", history.can_undo());
        self.action_set_enabled("win.redo", history.can_redo());
        self.action_set_enabled("win.delete-clip", has_selection);
        self.action_set_enabled("win.nudge-left", has_selection);
        self.action_set_enabled("win.nudge-right", has_selection);
        self.action_set_enabled("win.deselect", has_selection);
        self.action_set_enabled("win.split", has_clips);
        self.action_set_enabled("win.select-clip-at-playhead", has_clips);
    }

    /// The playhead position, if something is loaded.
    fn playhead(&self) -> Option<Frames> {
        self.imp()
            .player
            .borrow()
            .as_ref()
            .map(transient_engine::player::Player::position)
    }
}
