//! The timeline's context menu: right-click, a long press on a touchscreen, or
//! the Menu key (through the window's `win.context-menu` action).
//!
//! The menu's contents (a `gio::MenuModel` from the window's Blueprint) are the
//! window's actions; the timeline only decides what is selected first and where
//! the menu appears. A right-click selects what is under the pointer, unless it
//! is inside the time selection: then the menu is about that selection.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gdk, gio};
use transient_core::time::frames_to_f64;

use super::{LANE_HEIGHT, RULER_HEIGHT, TimelineView};

impl TimelineView {
    /// Uses `model` as the context menu. Called once by the window.
    pub(crate) fn set_context_menu(&self, model: &gio::MenuModel) {
        let menu = gtk::PopoverMenu::from_model(Some(model));
        // Context menus in GNOME have no arrow and open at the pointer.
        menu.set_has_arrow(false);
        menu.set_halign(gtk::Align::Start);
        menu.set_parent(self);
        if let Some(old) = self.imp().context_menu.replace(Some(menu)) {
            old.unparent();
        }
    }

    /// Sets up right-click and long press. Called once, from `setup_controllers`.
    pub(super) fn setup_context_menu(&self) {
        let click = gtk::GestureClick::builder()
            .button(gdk::BUTTON_SECONDARY)
            .build();
        let weak = self.downgrade();
        click.connect_pressed(move |gesture, _, x, y| {
            if let Some(view) = weak.upgrade() {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                view.open_context_menu_at(x, y);
            }
        });
        self.add_controller(click);

        // A long press is the touchscreen's right-click.
        let long_press = gtk::GestureLongPress::builder().touch_only(true).build();
        let weak = self.downgrade();
        long_press.connect_pressed(move |gesture, x, y| {
            if let Some(view) = weak.upgrade() {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                // The finger did not drag: drop the drag the press started.
                view.cancel_drag();
                view.open_context_menu_at(x, y);
            }
        });
        self.add_controller(long_press);
    }

    /// Selects what is at (`x`, `y`) (unless that is inside the time selection)
    /// and opens the menu there.
    fn open_context_menu_at(&self, x: f64, y: f64) {
        if !self.is_in_time_selection(x, y) {
            let clip = self.clip_at(x, y).map_or(0, |id| id.0);
            self.emit_by_name::<()>("clip-selected", &[&clip]);
        }
        self.popup_context_menu(x, y);
    }

    /// Opens the menu for the keyboard (Menu key, Shift+F10): at the start of the
    /// time selection, else at the selected clip, else at the playhead.
    pub(crate) fn show_context_menu(&self) {
        let imp = self.imp();
        let start = self.start_frame();
        let frames_per_pixel = imp.frames_per_pixel.get();
        let to_x = |frame| (frames_to_f64(frame) - start) / frames_per_pixel;
        let lane_middle = |lane| super::lane_top_f64(lane) + f64::from(LANE_HEIGHT) / 2.0;
        let selected_clip = imp.selected.get().and_then(|id| self.find_clip(id));
        let (x, y) = if let Some(selection) = imp.time_selection.get() {
            (
                to_x(selection.range.start),
                lane_middle(selection.first_lane),
            )
        } else if let Some((lane, clip)) = selected_clip {
            (to_x(clip.clip.start), lane_middle(lane))
        } else {
            (to_x(imp.playhead.get()), f64::from(RULER_HEIGHT))
        };
        let width = f64::from(self.width());
        self.popup_context_menu(x.clamp(0.0, width), y);
    }

    fn popup_context_menu(&self, x: f64, y: f64) {
        let Some(menu) = self.imp().context_menu.borrow().clone() else {
            return;
        };
        menu.set_pointing_to(Some(&gdk::Rectangle::new(
            whole_pixel(x),
            whole_pixel(y),
            1,
            1,
        )));
        menu.popup();
        tracing::debug!(x, y, open = menu.is_visible(), "context menu");
    }

    /// Whether (`x`, `y`) is inside the time selection (its stretch and lanes).
    fn is_in_time_selection(&self, x: f64, y: f64) -> bool {
        let Some(selection) = self.imp().time_selection.get() else {
            return false;
        };
        let Some(lane) = self.lane_at(y) else {
            return false;
        };
        let frame = self.frame_at(x);
        (selection.first_lane..=selection.last_lane).contains(&lane)
            && (selection.range.start..selection.range.end).contains(&frame)
    }
}

/// A position in the view as a whole pixel, for `gdk::Rectangle`.
fn whole_pixel(value: f64) -> i32 {
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        reason = "a position inside the widget, rounded and clamped to the i32 range"
    )]
    let pixel = value
        .round()
        .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
    pixel
}
