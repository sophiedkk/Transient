//! Editing shortcuts: single keys (S, M, Space, Delete, …) and keys that text
//! fields use too (Ctrl+C, Ctrl+Z, Ctrl+Left, …).
//!
//! These are not application-wide accelerators (`set_accels_for_action`):
//! GTK runs those *before* the focused widget, so typing "test" in a track's
//! name field would split clips at every "s". Instead, one shortcut controller
//! on the window runs them in the *bubble* phase: after the focused widget.
//! A text field uses the keys it understands (letters, Delete, Ctrl+C, …), and
//! only the rest reach these shortcuts. While a dialog (such as Export) is open,
//! they do nothing.
//!
//! Commands that never clash with typing (Ctrl+N, Ctrl+S, F9, zoom, …) stay
//! application-wide, in `application.rs`. The shortcuts dialog and the context
//! menu list these shortcuts themselves; a test below keeps them in sync.

use adw::prelude::*;
use gtk::glib;
use tracing::warn;

use super::Window;

/// Each editing action and its keys, in GTK's shortcut syntax ("|" separates
/// alternatives).
pub(crate) const EDITING_SHORTCUTS: &[(&str, &str)] = &[
    ("win.play-pause", "space"),
    ("win.go-to-start", "Home"),
    ("win.undo", "<Control>z"),
    ("win.redo", "<Shift><Control>z"),
    ("win.split", "s"),
    ("win.delete-clip", "Delete"),
    ("win.ripple-delete", "<Control>Delete"),
    ("win.select-all", "<Control>a"),
    ("win.deselect", "Escape"),
    ("win.cut", "<Control>x"),
    ("win.copy", "<Control>c"),
    ("win.paste", "<Control>v"),
    ("win.paste-insert", "<Shift><Control>v"),
    ("win.context-menu", "Menu|<Shift>F10"),
    ("win.nudge-left", "<Control>Left"),
    ("win.nudge-right", "<Control>Right"),
    ("win.move-track-up", "<Alt>Up"),
    ("win.move-track-down", "<Alt>Down"),
    ("win.toggle-mute", "m"),
    ("win.toggle-solo", "<Shift>m"),
];

impl Window {
    /// Adds the editing shortcuts to the window. Called once, from `constructed`.
    pub(super) fn setup_shortcuts(&self) {
        let controller = gtk::ShortcutController::new();
        // After the focused widget, so text fields get their keys first.
        controller.set_propagation_phase(gtk::PropagationPhase::Bubble);
        for &(action, keys) in EDITING_SHORTCUTS {
            let Some(trigger) = gtk::ShortcutTrigger::parse_string(keys) else {
                warn!("could not read the shortcut {keys} for {action}");
                continue;
            };
            let callback = gtk::CallbackAction::new(move |widget, _| {
                let Some(window) = widget.downcast_ref::<Window>() else {
                    return glib::Propagation::Proceed;
                };
                // Keys typed in a dialog are the dialog's, even unused ones.
                if window.visible_dialog().is_some() {
                    return glib::Propagation::Proceed;
                }
                // The focused widget had its chance (bubble phase), so the key is
                // ours. A disabled action (nothing selected, …) simply does nothing.
                if WidgetExt::activate_action(window, action, None).is_ok() {
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            });
            controller.add_shortcut(gtk::Shortcut::new(Some(trigger), Some(callback)));
        }
        self.add_controller(controller);
    }
}

#[cfg(test)]
mod tests {
    use super::EDITING_SHORTCUTS;

    /// The shortcuts dialog lists every editing shortcut with the same keys.
    #[test]
    fn the_shortcuts_dialog_matches_the_table() {
        let dialog = include_str!("../../resources/ui/shortcuts-dialog.blp");
        for (action, keys) in EDITING_SHORTCUTS {
            // The dialog writes alternatives with spaces instead of "|".
            let expected = format!("accelerator: \"{}\";", keys.replace('|', " "));
            assert!(
                dialog.contains(&expected),
                "shortcuts-dialog.blp lacks {expected} (for {action})"
            );
        }
    }

    /// Every context menu item for an editing action shows that action's key.
    #[test]
    fn the_context_menu_shows_the_same_keys() {
        let window = include_str!("../../resources/ui/window.blp");
        let menu = window
            .split("menu timeline_menu")
            .nth(1)
            .expect("window.blp has the timeline menu");
        for (action, keys) in EDITING_SHORTCUTS {
            let item = format!("action: \"{action}\";");
            if !menu.contains(&item) {
                continue;
            }
            let first_key = keys.split('|').next().unwrap_or(keys);
            let expected = format!("{item}\n      accel: \"{first_key}\";");
            assert!(
                menu.contains(&expected),
                "the {action} menu item lacks accel: \"{first_key}\""
            );
        }
    }
}
