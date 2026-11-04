//! Cut, Copy, Paste and "Paste and Make Room" (`win.cut`, `win.copy`,
//! `win.paste`, `win.paste-insert`; Ctrl+X/C/V and Shift+Ctrl+V).
//!
//! Copy takes the time selection if there is one, otherwise the selected
//! clips. Paste puts the copy at the playhead, starting on the selected track
//! (or the track it was copied from), and selects what it pasted. The maths is
//! in `transient_core::clipboard`.

use adw::subclass::prelude::*;
use gettextrs::gettext;
use transient_core::clipboard::Clipboard;
use transient_core::project::ClipId;

use super::Window;

impl Window {
    /// Copies the time selection, or else the selected clips (`win.copy`, Ctrl+C).
    /// Returns whether something was copied.
    pub(super) fn copy(&self) -> bool {
        let copy = {
            let project = self.imp().project.borrow();
            match self.time_selection() {
                Some(selection) => project.copy_range(&selection.tracks, selection.range),
                None => project.copy_clips(&self.selected_clips()),
            }
        };
        let copied = copy.is_some();
        if copied {
            self.imp().clipboard.replace(copy);
            self.update_edit_actions();
        }
        copied
    }

    /// Copies, then deletes what was copied (`win.cut`, Ctrl+X). No Undo toast:
    /// the cut part is on the clipboard, and Ctrl+Z works as always.
    pub(super) fn cut(&self) {
        if !self.copy() {
            return;
        }
        if let Some(selection) = self.time_selection() {
            self.apply_edit(|project| project.delete_range(&selection.tracks, selection.range));
            self.set_time_selection(None);
        } else {
            let clips = self.selected_clips();
            self.apply_edit(|project| {
                let mut changed = false;
                for clip in &clips {
                    changed |= project.remove_clip(*clip).is_some();
                }
                changed
            });
            self.select(None);
        }
    }

    /// Pastes at the playhead (`win.paste`, Ctrl+V), or with `insert` first makes
    /// room on every track (`win.paste-insert`, Shift+Ctrl+V).
    pub(super) fn paste(&self, insert: bool) {
        let imp = self.imp();
        let Some(clipboard) = imp.clipboard.borrow().clone() else {
            return;
        };
        let at = self.playhead_position().unwrap_or(0);
        let first_track = self.selected_track().unwrap_or(clipboard.first_track);
        let name = gettext("Track");
        let mut pasted: Vec<ClipId> = Vec::new();
        self.apply_edit(|project| {
            pasted = if insert {
                project.insert(&clipboard, first_track, at, &name)
            } else {
                project.paste(&clipboard, first_track, at, &name)
            };
            // Insert also changes things when there was nothing to paste (it
            // makes room); a paste of nothing changes nothing.
            insert || !pasted.is_empty()
        });
        // What was pasted is selected, so it can be moved right away.
        self.set_time_selection(None);
        self.select_clips(&pasted);
    }

    /// Forgets the clipboard: it refers to this project's audio files, so it
    /// means nothing in another project (New, Open).
    pub(super) fn clear_clipboard(&self) {
        self.imp().clipboard.replace(None::<Clipboard>);
    }
}
