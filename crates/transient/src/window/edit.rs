//! Editing: selection, the edit actions, and undo/redo.
//!
//! Every change goes through [`Window::apply_edit`], which saves the project as it
//! was in the undo history first. So everything is undoable, which is why
//! deleting needs no confirmation dialog: a toast offers "Undo" instead
//! (GNOME HIG, "Be considerate").

use adw::prelude::*;
use adw::subclass::prelude::*;
use gettextrs::{gettext, ngettext};
use transient_core::project::{Clip, ClipId, Project, TrackId};
use transient_core::time::Frames;

use super::Window;
use crate::widgets::track_header::TrackHeader;

/// Ctrl+Left/Right moves the selected clips by this many pixels at the current zoom.
const NUDGE_PIXELS: f64 = 10.0;

impl Window {
    /// Connects the timeline's editing signals. Called once, from `constructed`.
    pub(super) fn setup_editing(&self) {
        let timeline = &self.imp().timeline;
        let weak = self.downgrade();
        timeline.connect_clip_selected(move |_, clip, extend| {
            if let Some(window) = weak.upgrade() {
                // A click selects a clip or nothing; either way, not a stretch of time.
                window.set_time_selection(None);
                match clip {
                    Some(clip) if extend => window.toggle_in_selection(clip),
                    _ => window.select(clip),
                }
            }
        });
        let weak = self.downgrade();
        timeline.connect_empty_clicked(move |_| {
            if let Some(window) = weak.upgrade() {
                // The click deselected the clip (`clip-selected`); the pane
                // closes with it, as double-clicking a clip opens it.
                window.hide_clip_pane();
            }
        });
        let weak = self.downgrade();
        timeline.connect_clip_activated(move |_, clip| {
            if let Some(window) = weak.upgrade() {
                // The first click of the double-click selected it already; with
                // Ctrl or Shift held it may have toggled it off, so select it again.
                window.set_time_selection(None);
                window.select(Some(clip));
                window.show_clip_pane();
            }
        });
        let weak = self.downgrade();
        timeline.connect_clips_moved(move |_, frames, lanes| {
            if let Some(window) = weak.upgrade() {
                window.move_selected_clips(frames, lanes);
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

    /// Selects one clip (or nothing).
    pub(crate) fn select(&self, clip: Option<ClipId>) {
        // A clip selection and a time selection never exist at the same time.
        if clip.is_some() && self.imp().time_selection.take().is_some() {
            self.show_time_selection();
        }
        self.imp().selection.replace(clip.into_iter().collect());
        self.imp().selected.set(clip);
        self.selection_changed();
    }

    /// Ctrl+click or Shift+click: adds `clip` to the selection, or removes it if
    /// it is selected already. An added clip becomes the primary one.
    fn toggle_in_selection(&self, clip: ClipId) {
        let imp = self.imp();
        {
            let mut selection = imp.selection.borrow_mut();
            if let Some(index) = selection.iter().position(|id| *id == clip) {
                selection.remove(index);
                imp.selected.set(selection.last().copied());
            } else {
                selection.push(clip);
                imp.selected.set(Some(clip));
            }
        }
        self.selection_changed();
    }

    /// Selects exactly `clips` (for example what was just pasted); the last one
    /// becomes the primary clip.
    pub(super) fn select_clips(&self, clips: &[ClipId]) {
        let imp = self.imp();
        if !clips.is_empty() && imp.time_selection.take().is_some() {
            self.show_time_selection();
        }
        imp.selection.replace(clips.to_vec());
        imp.selected.set(clips.last().copied());
        self.selection_changed();
    }

    /// Adds every clip playing at `seconds` to the selection (for `--select-clips-at`).
    pub(crate) fn add_clips_at_seconds(&self, seconds: f64) {
        let clips = {
            let project = self.imp().project.borrow();
            project
                .sample_rate
                .seconds_to_frames(seconds.max(0.0))
                .map(|frame| project.clips_at(frame))
                .unwrap_or_default()
        };
        for clip in clips {
            if !self.imp().selection.borrow().contains(&clip) {
                self.toggle_in_selection(clip);
            }
        }
    }

    /// Shows the clip selection in the timeline and updates what depends on it.
    fn selection_changed(&self) {
        let imp = self.imp();
        imp.timeline
            .set_selection(imp.selected.get(), &imp.selection.borrow());
        self.update_edit_actions();
        self.update_pane();
    }

    /// The selected clips (all of them).
    pub(super) fn selected_clips(&self) -> Vec<ClipId> {
        self.imp().selection.borrow().clone()
    }

    /// Moves every selected clip by `frames` and `lanes` (a group drag), as one
    /// undo step. The timeline already kept the group inside the timeline.
    fn move_selected_clips(&self, frames: i64, lanes: i32) {
        let ids = self.selected_clips();
        self.apply_edit(|project| {
            let moves: Vec<(Clip, TrackId)> = ids
                .iter()
                .filter_map(|id| {
                    let (track, clip) = project.clip(*id)?;
                    let index = project.tracks.iter().position(|t| t.id == track)?;
                    let target = index.checked_add_signed(isize::try_from(lanes).ok()?)?;
                    let target = project.tracks.get(target)?.id;
                    Some((
                        clip.moved_to(clip.start.saturating_add_signed(frames)),
                        target,
                    ))
                })
                .collect();
            project.move_clips(&moves)
        });
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
        let clips = self.selected_clips();
        if clips.is_empty() {
            return;
        }
        self.apply_edit(|project| project.delete_clips(&clips));
        self.select(None);
        let count = u32::try_from(clips.len()).unwrap_or(u32::MAX);
        self.show_undo_toast(
            &ngettext("Clip deleted", "{} clips deleted", count)
                .replace("{}", &clips.len().to_string()),
        );
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

    /// The selected track: the topmost track with a selected clip, or else the
    /// first track of the time selection. Paste starts there; F2 and
    /// Alt+Up/Down act on it.
    pub(super) fn selected_track(&self) -> Option<TrackId> {
        let imp = self.imp();
        let selection = imp.selection.borrow();
        if !selection.is_empty() {
            let project = imp.project.borrow();
            return project
                .tracks
                .iter()
                .find(|track| track.clips.iter().any(|clip| selection.contains(&clip.id)))
                .map(|track| track.id);
        }
        imp.time_selection
            .borrow()
            .as_ref()
            .and_then(|selection| selection.tracks.first().copied())
    }

    /// Starts renaming the selected track in its header (`win.rename-track`, F2).
    pub(super) fn rename_selected_track(&self) {
        let Some(track) = self.selected_track() else {
            return;
        };
        let mut child = self.imp().track_headers.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            if let Ok(header) = widget.downcast::<TrackHeader>()
                && header.track_id() == track
            {
                header.start_rename();
            }
        }
    }

    /// Moves the selected track up or down (`win.move-track-up` / `-down`,
    /// Alt+Up / Alt+Down).
    pub(super) fn move_selected_track(&self, up: bool) {
        if let Some(track) = self.selected_track() {
            self.apply_edit(|project| project.move_track(track, up));
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
        let selected = self.selected_clips();
        self.apply_edit(|project| {
            let under_playhead = project.clips_at(at);
            // The selected clips under the playhead; if none, every clip there.
            let chosen: Vec<ClipId> = under_playhead
                .iter()
                .copied()
                .filter(|clip| selected.contains(clip))
                .collect();
            let targets = if chosen.is_empty() {
                under_playhead
            } else {
                chosen
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

    /// Moves the selected clips a little left or right (Ctrl+Left/Right).
    pub(super) fn nudge_selected_clip(&self, direction: i8) {
        let ids = self.selected_clips();
        let step = NUDGE_PIXELS * self.imp().timeline.frames_per_pixel();
        let step = transient_core::time::frames_from_f64(step.max(1.0)).unwrap_or(1);
        self.apply_edit(|project| {
            let clips: Vec<(TrackId, Clip)> = ids
                .iter()
                .filter_map(|id| project.clip(*id).map(|(track, clip)| (track, *clip)))
                .collect();
            // To the left, the group stops when its first clip reaches 0:00.
            let earliest = clips.iter().map(|(_, clip)| clip.start).min().unwrap_or(0);
            let step = if direction < 0 {
                step.min(earliest)
            } else {
                step
            };
            if step == 0 {
                return false;
            }
            let moves: Vec<(Clip, TrackId)> = clips
                .iter()
                .map(|&(track, clip)| {
                    let start = if direction < 0 {
                        clip.start - step
                    } else {
                        clip.start.saturating_add(step)
                    };
                    (clip.moved_to(start), track)
                })
                .collect();
            project.move_clips(&moves)
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
        // Selected clips can disappear through undo; drop them from the selection.
        let gone = {
            let project = imp.project.borrow();
            let mut selection = imp.selection.borrow_mut();
            let before = selection.len();
            selection.retain(|clip| project.clip(*clip).is_some());
            if imp
                .selected
                .get()
                .is_some_and(|clip| !selection.contains(&clip))
            {
                imp.selected.set(selection.last().copied());
            }
            selection.len() != before
        };
        if gone {
            imp.timeline
                .set_selection(imp.selected.get(), &imp.selection.borrow());
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
        self.action_set_enabled("win.cut", has_selection);
        self.action_set_enabled("win.copy", has_selection);
        let can_paste = imp.clipboard.borrow().is_some();
        self.action_set_enabled("win.paste", can_paste);
        self.action_set_enabled("win.paste-insert", can_paste);
        self.action_set_enabled("win.deselect", has_selection);
        self.action_set_enabled("win.select-all", has_clips);
        self.action_set_enabled("win.rename-track", has_selection);
        self.action_set_enabled("win.move-track-up", has_selection);
        self.action_set_enabled("win.move-track-down", has_selection);
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
        crate::widgets::set_button_text(&*imp.split_button, &split);
        let clips = imp.selection.borrow().len();
        let delete = if has_time {
            gettext("Delete Selection")
        } else if clips > 1 {
            gettext("Delete Selected Clips")
        } else if has_clip {
            gettext("Delete Clip")
        } else {
            gettext("Select a clip, or drag over a waveform to select time")
        };
        crate::widgets::set_button_text(&*imp.delete_button, &delete);
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
