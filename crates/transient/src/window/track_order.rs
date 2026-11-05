//! Reordering tracks by dragging their header up or down.
//!
//! Each header is a drag source carrying its track's ID (see
//! `TrackHeader::setup_drag`); the column of headers is the drop target. While
//! a header is dragged over the column, a line shows the gap it would land in;
//! dropping moves the track there (`Project::move_track_to`), as one undo step.
//! The menu's Move Up / Move Down (Alt+Up / Alt+Down) do the same by keyboard.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::gdk;
use transient_core::convert::u32_from_f64;
use transient_core::project::TrackId;

use super::Window;
use crate::widgets::timeline_view::LANE_HEIGHT;
use crate::widgets::track_header::TrackHeader;

impl Window {
    /// Makes the header column accept dragged headers. Called once, from
    /// `constructed`.
    pub(super) fn setup_track_reordering(&self) {
        let target = gtk::DropTarget::new(u64::static_type(), gdk::DragAction::MOVE);
        let weak = self.downgrade();
        target.connect_motion(move |_, _, y| {
            if let Some(window) = weak.upgrade() {
                window.show_drop_line(Some(window.drop_slot(y)));
            }
            gdk::DragAction::MOVE
        });
        let weak = self.downgrade();
        target.connect_leave(move |_| {
            if let Some(window) = weak.upgrade() {
                window.show_drop_line(None);
            }
        });
        let weak = self.downgrade();
        target.connect_drop(move |_, value, _, y| {
            let (Some(window), Ok(id)) = (weak.upgrade(), value.get::<u64>()) else {
                return false;
            };
            window.show_drop_line(None);
            let slot = window.drop_slot(y);
            window.apply_edit(|project| project.move_track_to(TrackId(id), slot))
        });
        self.imp().track_headers.add_controller(target);
    }

    /// The gap between tracks nearest to height `y` in the header column: 0 is
    /// above the first track, the number of tracks below the last. Every header
    /// is exactly `LANE_HEIGHT` tall, like the timeline's lanes.
    fn drop_slot(&self, y: f64) -> usize {
        let tracks = self.imp().project.borrow().tracks.len();
        let gap = u32_from_f64(y / f64::from(LANE_HEIGHT));
        usize::try_from(gap).unwrap_or(usize::MAX).min(tracks)
    }

    /// Draws the drop line at `slot` (see `drop_slot`), or removes it (`None`).
    fn show_drop_line(&self, slot: Option<usize>) {
        let mut headers = Vec::new();
        let mut child = self.imp().track_headers.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            if let Ok(header) = widget.downcast::<TrackHeader>() {
                headers.push(header);
            }
        }
        let last = headers.len().saturating_sub(1);
        for (index, header) in headers.iter().enumerate() {
            // The line goes above the header at the slot, or below the last one.
            let before = slot == Some(index);
            let after = index == last && slot == Some(headers.len());
            header.show_drop_line(before, after);
        }
    }
}
