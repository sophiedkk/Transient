//! Using the timeline from the keyboard, like a text cursor (Tab or a click
//! gives it focus):
//!
//! | Key               | Does                                                   |
//! |-------------------|--------------------------------------------------------|
//! | Left / Right      | moves the playhead a small step (more when zoomed out)  |
//! | Shift+Left/Right  | selects time from the playhead, on the current track(s) |
//! | Up / Down         | moves to the track above/below; selects the clip there  |
//! | End               | moves the playhead to the end (Home is `win.go-to-start`) |
//!
//! Keys with Ctrl or Alt are left to the window's shortcuts (nudge, move
//! track). The timeline shows a focus outline and a faint band on the current
//! track. For screen readers, its accessible description says the current
//! track, the playhead and the selection, and changes are announced.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gettextrs::gettext;
use gtk::{gdk, glib};
use transient_core::range::TimeRange;
use transient_core::time::{Frames, format_position, frames_from_f64};

use super::{LaneRange, TimelineView};

/// How far one arrow key press moves the playhead, in pixels at the current zoom.
const STEP_PIXELS: f64 = 10.0;

impl TimelineView {
    /// Makes the timeline focusable and handles its keys. Called once.
    pub(super) fn setup_keyboard(&self) {
        self.set_focusable(true);
        self.set_focus_on_click(true);

        let keys = gtk::EventControllerKey::new();
        let weak = self.downgrade();
        keys.connect_key_pressed(move |_, key, _, state| {
            let Some(view) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            view.key_pressed(key, state)
        });
        self.add_controller(keys);

        // The focus outline and the current track's band come and go with focus.
        let focus = gtk::EventControllerFocus::new();
        let weak = self.downgrade();
        focus.connect_enter(move |_| {
            if let Some(view) = weak.upgrade() {
                view.queue_draw();
                view.update_description();
            }
        });
        let weak = self.downgrade();
        focus.connect_leave(move |_| {
            if let Some(view) = weak.upgrade() {
                view.queue_draw();
            }
        });
        self.add_controller(focus);
    }

    fn key_pressed(&self, key: gdk::Key, state: gdk::ModifierType) -> glib::Propagation {
        // Ctrl and Alt combinations belong to the window's shortcuts.
        if state.intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK) {
            return glib::Propagation::Proceed;
        }
        let extend = state.contains(gdk::ModifierType::SHIFT_MASK);
        match key {
            gdk::Key::Left | gdk::Key::KP_Left => self.step(-1, extend),
            gdk::Key::Right | gdk::Key::KP_Right => self.step(1, extend),
            gdk::Key::Up | gdk::Key::KP_Up if !extend => self.change_lane(-1),
            gdk::Key::Down | gdk::Key::KP_Down if !extend => self.change_lane(1),
            gdk::Key::End | gdk::Key::KP_End if !extend => {
                self.emit_by_name::<()>("seek", &[&self.imp().length.get()]);
            }
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    }

    /// Left/Right (`direction` −1 or 1): moves the playhead one step; with
    /// `extend` (Shift), grows or shrinks the time selection instead.
    fn step(&self, direction: i8, extend: bool) {
        let imp = self.imp();
        let step =
            frames_from_f64((STEP_PIXELS * imp.frames_per_pixel.get()).max(1.0)).unwrap_or(1);
        let length = imp.length.get();
        let moved = |from: Frames| {
            if direction < 0 {
                from.saturating_sub(step)
            } else {
                from.saturating_add(step).min(length)
            }
        };
        if !extend {
            // Like a text cursor: moving without Shift ends the selection.
            imp.keyboard_selection.set(None);
            if imp.time_selection.get().is_some() {
                self.report_selection(None);
            }
            self.emit_by_name::<()>("seek", &[&moved(imp.playhead.get())]);
            return;
        }
        // Keep extending the same selection: its fixed end (anchor) and the end
        // that moves (cursor). A selection made some other way starts afresh
        // from the playhead.
        let lane = imp
            .current_lane
            .get()
            .min(imp.lanes.borrow().len().saturating_sub(1));
        let (anchor, cursor, lanes) = match (imp.keyboard_selection.get(), imp.time_selection.get())
        {
            (Some((anchor, cursor)), Some(current))
                if current.range == TimeRange::between(anchor, cursor) =>
            {
                (anchor, cursor, (current.first_lane, current.last_lane))
            }
            _ => (imp.playhead.get(), imp.playhead.get(), (lane, lane)),
        };
        let cursor = moved(cursor);
        imp.keyboard_selection.set(Some((anchor, cursor)));
        let selection = LaneRange {
            range: TimeRange::between(anchor, cursor),
            first_lane: lanes.0,
            last_lane: lanes.1,
        };
        self.report_selection(Some(selection));
        let rate = self.rate_for_text();
        self.announce(
            &gettext("Selected {length}")
                .replace("{length}", &format_position(selection.range.length(), rate)),
            gtk::AccessibleAnnouncementPriority::Medium,
        );
    }

    /// Up/Down (`delta` −1 or 1): makes the track above or below current, and
    /// selects the clip under the playhead there (if any).
    fn change_lane(&self, delta: i8) {
        let imp = self.imp();
        let count = imp.lanes.borrow().len();
        if count == 0 {
            return;
        }
        let current = imp.current_lane.get().min(count - 1);
        let lane = if delta < 0 {
            current.saturating_sub(1)
        } else {
            (current + 1).min(count - 1)
        };
        imp.current_lane.set(lane);
        self.queue_draw();

        let playhead = imp.playhead.get();
        let clip = {
            let lanes = imp.lanes.borrow();
            lanes.get(lane).and_then(|lane| {
                let stack: Vec<_> = lane.clips.iter().map(|clip| clip.clip).collect();
                transient_core::edit::visible_parts(&stack)
                    .into_iter()
                    .rev()
                    .find(|part| part.start <= playhead && playhead < part.end())
                    .and_then(|part| lane.clips.iter().find(|clip| clip.clip.id == part.id))
                    .map(|clip| (clip.clip.id, clip.name.clone()))
            })
        };
        let track = self.lane_name(lane);
        let message = if let Some((id, name)) = clip {
            self.emit_by_name::<()>("clip-selected", &[&id.0, &false]);
            gettext("{track}, clip {clip}")
                .replace("{track}", &track)
                .replace("{clip}", &name)
        } else {
            // Nothing here: the clip on the track left behind is not selected either.
            self.emit_by_name::<()>("clip-selected", &[&0_u64, &false]);
            gettext("{track}, no clip at the playhead").replace("{track}", &track)
        };
        self.announce(&message, gtk::AccessibleAnnouncementPriority::Medium);
        self.update_description();
    }

    /// Reports a time selection (or none) to the window, as a drag would.
    fn report_selection(&self, selection: Option<LaneRange>) {
        let (range, first, last) = selection.map_or((TimeRange { start: 0, end: 0 }, 0, 0), |s| {
            (s.range, s.first_lane, s.last_lane)
        });
        let lane = |index: usize| u32::try_from(index).unwrap_or(0);
        self.emit_by_name::<()>(
            "range-selected",
            &[&range.start, &range.end, &lane(first), &lane(last)],
        );
    }

    /// The track at `lane`, by name ("Track 2" if it has none).
    fn lane_name(&self, lane: usize) -> String {
        self.imp()
            .lanes
            .borrow()
            .get(lane)
            .map(|lane| lane.name.clone())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| gettext("Track {}").replace("{}", &(lane + 1).to_string()))
    }

    fn rate_for_text(&self) -> transient_core::time::SampleRate {
        self.imp()
            .sample_rate
            .get()
            .unwrap_or(transient_core::time::SampleRate::HZ_48000)
    }

    /// Updates what a screen reader says about the timeline: the current track,
    /// the playhead and the time selection.
    pub(super) fn update_description(&self) {
        let imp = self.imp();
        let count = imp.lanes.borrow().len();
        if count == 0 {
            self.update_property(&[gtk::accessible::Property::Description(&gettext(
                "No tracks",
            ))]);
            return;
        }
        let lane = imp.current_lane.get().min(count - 1);
        let rate = self.rate_for_text();
        let mut text = gettext("Track {number} of {count}, {name}. Playhead at {time}.")
            .replace("{number}", &(lane + 1).to_string())
            .replace("{count}", &count.to_string())
            .replace("{name}", &self.lane_name(lane))
            .replace("{time}", &format_position(imp.playhead.get(), rate));
        if let Some(selection) = imp.time_selection.get() {
            text.push(' ');
            text.push_str(
                &gettext("Selected {start} to {end}.")
                    .replace("{start}", &format_position(selection.range.start, rate))
                    .replace("{end}", &format_position(selection.range.end, rate)),
            );
        }
        self.update_property(&[gtk::accessible::Property::Description(&text)]);
    }
}
