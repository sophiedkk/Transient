//! Mouse interaction: what is under the pointer, and dragging.
//!
//! One `GestureDrag` handles every kind of drag; what it does is decided when
//! the button goes down, by what is under the pointer:
//!
//! | Under the pointer          | Drag does                       |
//! |----------------------------|---------------------------------|
//! | the ruler                  | moves the playhead              |
//! | a clip's name strip (top)  | moves the clip (also to another lane, or below the last one) |
//! | a clip's waveform, or an empty lane | selects time, over the lanes dragged across |
//! | a clip's left/right edge   | trims the clip                  |
//! | a fade handle (selected clip, top corners) | changes the fade |
//!
//! A click (no drag) selects the clip under the pointer (or nothing) and moves
//! the playhead there.
//!
//! While dragging, only a *preview* changes (drawn by `draw.rs`). The project
//! changes once, on release, through the `clip-changed` signal, so one drag is
//! one undo step; a time selection is reported with `range-selected`. Edges
//! snap to other clips' edges, the playhead and the start.
//!
//! A drag is measured in frames, not pixels: the clip follows the audio under the
//! pointer. That keeps the preview right while the view scrolls by itself
//! because the pointer is near an edge (auto-scroll), or zooms during a drag.
//! On a touchscreen, edges and fade handles are easier to grab.

use std::cell::Cell;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gdk, glib};
use transient_core::edit::{snap, visible_parts};
use transient_core::project::{Clip, ClipId};
use transient_core::range::TimeRange;
use transient_core::time::{Frames, frames_from_f64, frames_to_f64};
use transient_core::view::autoscroll_speed;

use super::gestures::microseconds_to_seconds;
use super::{LANE_HEIGHT, LaneClip, LaneRange, RULER_HEIGHT, TimelineView};

/// How close (in pixels) an edge has to come to a snap point to snap to it.
const SNAP_DISTANCE: f64 = 8.0;

/// How easy things are to grab. A fingertip is much less precise than a mouse
/// pointer, so touchscreens get larger zones.
#[derive(Debug, Clone, Copy)]
struct Grab {
    /// How close (in pixels) to a clip's edge counts as grabbing the edge.
    edge: f64,
    /// How close (in pixels) to a fade handle counts as grabbing it.
    handle: f64,
    /// A press that moves less than this (in pixels) is a click, not a drag.
    click_slop: f64,
}

const POINTER_GRAB: Grab = Grab {
    edge: 6.0,
    handle: 8.0,
    click_slop: 3.0,
};
const TOUCH_GRAB: Grab = Grab {
    edge: 16.0,
    handle: 20.0,
    click_slop: 8.0,
};

/// The part of a clip under the pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Zone {
    LeftEdge,
    /// The name strip along the top: grab it to move the clip.
    Strip,
    /// The waveform: drag over it to select time.
    Waveform,
    RightEdge,
    FadeInHandle,
    FadeOutHandle,
}

/// What is under the pointer.
#[derive(Debug, Clone)]
enum Hit {
    /// The time ruler, or below the last lane.
    Ruler,
    /// A lane (this one), but no clip.
    Empty(usize),
    /// A clip (its lane index, its data and which part of it).
    Clip(usize, Box<LaneClip>, Zone),
}

/// What a drag does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DragKind {
    Seek,
    Select,
    Move,
    TrimStart,
    TrimEnd,
    FadeIn,
    FadeOut,
}

/// A drag in progress.
#[derive(Debug, Clone)]
pub(crate) struct Drag {
    kind: DragKind,
    start_x: f64,
    start_y: f64,
    /// The (fractional) frame under the pointer when the drag began.
    grab_frame: f64,
    /// How far the pointer has moved since then, in pixels (as GTK reports it).
    last_dx: f64,
    last_dy: f64,
    /// A press that moves less than this is a click (larger for touch).
    click_slop: f64,
    /// The clip as it was when the drag began, and its lane.
    original: Option<(usize, LaneClip)>,
    /// Where the clip would end up if released now, and in which lane.
    pub(super) preview: Option<(usize, Clip)>,
    /// For a time selection: the lane it started in, the (snapped) frame it
    /// started at, and the selection if released now.
    start_lane: usize,
    select_from: Frames,
    pub(super) range_preview: Option<LaneRange>,
    /// The other clips' edges, the playhead and the start.
    snap_points: Vec<Frames>,
    /// Whether the pointer moved far enough to count as a drag.
    moved: bool,
}

impl TimelineView {
    /// Creates the drag gesture (the caller adds it to the view). Called once,
    /// from `setup_controllers`.
    pub(super) fn drag_gesture(&self) -> gtk::GestureDrag {
        let drag = gtk::GestureDrag::new();
        let weak = self.downgrade();
        drag.connect_drag_begin(move |gesture, x, y| {
            let Some(view) = weak.upgrade() else {
                return;
            };
            let touch = gesture
                .current_event_device()
                .is_some_and(|device| device.source() == gdk::InputSource::Touchscreen);
            if view.drag_begin(x, y, touch) {
                // A clip or a selection is being dragged: claim the touch, so the
                // scrolled window around the view does not scroll the tracks instead.
                gesture.set_state(gtk::EventSequenceState::Claimed);
            }
        });
        let weak = self.downgrade();
        drag.connect_drag_update(move |_, dx, dy| {
            if let Some(view) = weak.upgrade() {
                view.drag_update(dx, dy);
            }
        });
        let weak = self.downgrade();
        drag.connect_drag_end(move |_, _, _| {
            if let Some(view) = weak.upgrade() {
                view.drag_end();
            }
        });
        // Something else took over the touch (a pinch, or the scrolled window):
        // forget the drag without changing anything.
        let weak = self.downgrade();
        drag.connect_cancel(move |_, _| {
            if let Some(view) = weak.upgrade() {
                view.cancel_drag();
            }
        });
        drag
    }

    /// Drops the drag in progress (and its preview) without changing the clip.
    pub(super) fn cancel_drag(&self) {
        self.stop_autoscroll();
        if self.imp().drag.take().is_some() {
            self.queue_draw();
        }
    }

    /// Shows what a drag would do: resize over edges, a hand over a clip's name
    /// strip (move), a text cursor where dragging selects time.
    pub(super) fn update_cursor(&self, x: f64, y: f64) {
        let cursor = match self.hit(x, y, POINTER_GRAB) {
            Hit::Clip(
                _,
                _,
                Zone::LeftEdge | Zone::RightEdge | Zone::FadeInHandle | Zone::FadeOutHandle,
            ) => Some("col-resize"),
            Hit::Clip(_, _, Zone::Strip) => Some("grab"),
            Hit::Clip(_, _, Zone::Waveform) | Hit::Empty(_) => Some("text"),
            Hit::Ruler => None,
        };
        self.set_cursor_from_name(cursor);
    }

    /// The lane at height `y`, if it is inside a lane.
    pub(super) fn lane_at(&self, y: f64) -> Option<usize> {
        let lanes = self.imp().lanes.borrow().len();
        let offset = y - f64::from(RULER_HEIGHT);
        if offset < 0.0 {
            return None;
        }
        let index = frames_from_f64((offset / f64::from(LANE_HEIGHT)).floor())?;
        usize::try_from(index).ok().filter(|&index| index < lanes)
    }

    /// The lane a clip dragged to height `y` lands in. Below the last lane, that
    /// is one past it: a new track (see `clip-changed`). Above the lanes (on the
    /// ruler), `None`.
    fn drop_lane(&self, y: f64) -> Option<usize> {
        let lanes = self.imp().lanes.borrow().len();
        if y < f64::from(RULER_HEIGHT) {
            return None;
        }
        Some(self.lane_at(y).unwrap_or(lanes))
    }

    /// The clip you see at (`x`, `y`), if any.
    pub(super) fn clip_at(&self, x: f64, y: f64) -> Option<ClipId> {
        match self.hit(x, y, POINTER_GRAB) {
            Hit::Clip(_, clip, _) => Some(clip.clip.id),
            Hit::Ruler | Hit::Empty(_) => None,
        }
    }

    /// What is at (`x`, `y`): what you see there. Only the visible parts of clips
    /// count (see `visible_parts`), and later parts are drawn on top, so they win.
    fn hit(&self, x: f64, y: f64, grab: Grab) -> Hit {
        let Some(lane) = self.lane_at(y) else {
            return Hit::Ruler;
        };
        let lanes = self.imp().lanes.borrow();
        let Some(clips) = lanes.get(lane).map(|lane| &lane.clips) else {
            return Hit::Ruler;
        };
        let start = self.start_frame();
        let frames_per_pixel = self.imp().frames_per_pixel.get();
        let to_x = |frame: Frames| (frames_to_f64(frame) - start) / frames_per_pixel;

        // The selected clip's fade handles are drawn above everything, in its name
        // strip, and win over the edges, so a fade can be dragged even when it is zero.
        let strip_top = super::lane_top_f64(lane) + f64::from(super::CLIP_INSET);
        let strip_bottom = strip_top + f64::from(super::CLIP_LABEL_HEIGHT);
        let stack: Vec<Clip> = clips.iter().map(|clip| clip.clip).collect();
        let parts = visible_parts(&stack);
        let selected = self.imp().selected.get();
        if (strip_top..=strip_bottom).contains(&y)
            && let Some(clip) = clips.iter().find(|clip| Some(clip.clip.id) == selected)
        {
            let (fade_in_x, fade_out_x) = self.fade_handle_positions(&clip.clip);
            let visible = |x| self.is_visible_at(&parts, clip.clip.id, x);
            if (x - fade_in_x).abs() <= grab.handle && visible(fade_in_x) {
                return Hit::Clip(lane, Box::new(clip.clone()), Zone::FadeInHandle);
            }
            if (x - fade_out_x).abs() <= grab.handle && visible(fade_out_x) {
                return Hit::Clip(lane, Box::new(clip.clone()), Zone::FadeOutHandle);
            }
        }

        for part in parts.iter().rev() {
            let (left, right) = (to_x(part.start), to_x(part.end()));
            if x < left || x > right {
                continue;
            }
            let Some(clip) = clips.iter().find(|clip| clip.clip.id == part.id) else {
                continue;
            };
            // Narrow parts: keep a grabbable middle by shrinking the edge zones.
            // Only the clip's own edges trim it; where another clip covers it, the
            // edge of the visible part is not an edge of the clip.
            let edge = grab.edge.min((right - left) / 3.0);
            let zone = if part.start == clip.clip.start && x - left < edge {
                Zone::LeftEdge
            } else if part.end() == clip.clip.end() && right - x < edge {
                Zone::RightEdge
            } else if y <= strip_bottom {
                Zone::Strip
            } else {
                Zone::Waveform
            };
            return Hit::Clip(lane, Box::new(clip.clone()), zone);
        }
        Hit::Empty(lane)
    }

    /// Starts a drag at (`x`, `y`). Returns true when it grabbed a clip.
    fn drag_begin(&self, x: f64, y: f64, touch: bool) -> bool {
        self.stop_glide();
        let grab = if touch { TOUCH_GRAB } else { POINTER_GRAB };
        let hit = self.hit(x, y, grab);
        let mut start_lane = 0;
        let (kind, original) = match hit {
            Hit::Ruler => (DragKind::Seek, None),
            Hit::Empty(lane) => {
                // Clicking empty space selects nothing; dragging selects time.
                self.emit_by_name::<()>("clip-selected", &[&0_u64]);
                start_lane = lane;
                (DragKind::Select, None)
            }
            Hit::Clip(lane, clip, Zone::Waveform) => {
                // A click selects the clip; a drag turns into a time selection.
                self.emit_by_name::<()>("clip-selected", &[&clip.clip.id.0]);
                start_lane = lane;
                (DragKind::Select, None)
            }
            Hit::Clip(lane, clip, zone) => {
                self.emit_by_name::<()>("clip-selected", &[&clip.clip.id.0]);
                let kind = match zone {
                    Zone::LeftEdge => DragKind::TrimStart,
                    Zone::RightEdge => DragKind::TrimEnd,
                    Zone::FadeInHandle => DragKind::FadeIn,
                    Zone::FadeOutHandle => DragKind::FadeOut,
                    Zone::Strip | Zone::Waveform => DragKind::Move,
                };
                (kind, Some((lane, *clip)))
            }
        };
        if kind == DragKind::Seek {
            self.seek_to_x(x);
        }
        let except = original.as_ref().map(|(_, clip)| clip.clip.id);
        let claims = kind != DragKind::Seek;
        let snap_points = self.snap_points(except);
        let frames_per_pixel = self.imp().frames_per_pixel.get();
        let tolerance = frames_from_f64(SNAP_DISTANCE * frames_per_pixel).unwrap_or(0);
        let select_from = snap(self.frame_at(x), &snap_points, tolerance);
        let drag = Drag {
            kind,
            start_x: x,
            start_y: y,
            grab_frame: self.start_frame() + x * self.imp().frames_per_pixel.get(),
            last_dx: 0.0,
            last_dy: 0.0,
            click_slop: grab.click_slop,
            snap_points,
            original,
            preview: None,
            start_lane,
            select_from,
            range_preview: None,
            moved: false,
        };
        self.imp().drag.replace(Some(drag));
        claims
    }

    fn drag_update(&self, dx: f64, dy: f64) {
        self.update_preview(dx, dy);
        self.update_autoscroll();
    }

    /// Works out where the dragged clip (or the playhead) goes with the pointer
    /// `dx`, `dy` pixels from where the drag began.
    fn update_preview(&self, dx: f64, dy: f64) {
        let mut drag_slot = self.imp().drag.borrow_mut();
        let Some(drag) = drag_slot.as_mut() else {
            return;
        };
        if !drag.moved && dx.abs() < drag.click_slop && dy.abs() < drag.click_slop {
            return;
        }
        drag.moved = true;
        drag.last_dx = dx;
        drag.last_dy = dy;
        let frames_per_pixel = self.imp().frames_per_pixel.get();
        let tolerance = frames_from_f64(SNAP_DISTANCE * frames_per_pixel).unwrap_or(0);
        // How far the audio under the pointer is from where it was grabbed. This
        // also counts any scrolling or zooming since the drag began.
        let pointer_frame = self.start_frame() + (drag.start_x + dx) * frames_per_pixel;
        let delta = pointer_frame - drag.grab_frame;
        let shift =
            |frame: Frames| frames_from_f64((frames_to_f64(frame) + delta).max(0.0)).unwrap_or(0);

        if drag.kind == DragKind::Select {
            // The selection runs from where the drag began to the pointer (both
            // snapped), over the lanes from the first to the one under the pointer.
            let lanes = self.imp().lanes.borrow().len();
            let to = snap(
                self.frame_at(drag.start_x + dx),
                &drag.snap_points,
                tolerance,
            );
            let y = drag.start_y + dy;
            let lane = self.lane_at(y).unwrap_or(if y < f64::from(RULER_HEIGHT) {
                0
            } else {
                lanes.saturating_sub(1)
            });
            drag.range_preview = Some(LaneRange {
                range: TimeRange::between(drag.select_from, to),
                first_lane: lane.min(drag.start_lane),
                last_lane: lane.max(drag.start_lane),
            });
            drop(drag_slot);
            self.queue_draw();
            return;
        }
        let Some((lane, original)) = &drag.original else {
            // A seek drag: follow the pointer.
            let x = drag.start_x + dx;
            drop(drag_slot);
            self.seek_to_x(x);
            return;
        };
        let clip = original.clip;
        // Without its audio, a clip cannot be extended beyond what it already plays.
        let source_frames = original
            .waveform
            .as_ref()
            .map_or(clip.offset + clip.length, |waveform| {
                waveform.buffer().frames()
            });
        let points = &drag.snap_points;
        drag.preview = Some(match drag.kind {
            DragKind::Move => {
                // Snap whichever edge is closer to a snap point.
                let start = shift(clip.start);
                let snapped_start = snap(start, points, tolerance);
                let snapped_end = snap(start + clip.length, points, tolerance);
                let new_start = if snapped_start == start {
                    snapped_end.saturating_sub(clip.length)
                } else {
                    snapped_start
                };
                let new_lane = self.drop_lane(drag.start_y + dy).unwrap_or(*lane);
                (new_lane, clip.moved_to(new_start))
            }
            DragKind::TrimStart => {
                let new_start = snap(shift(clip.start), points, tolerance);
                (*lane, clip.trimmed_start(new_start))
            }
            DragKind::TrimEnd => {
                let new_end = snap(shift(clip.end()), points, tolerance);
                (*lane, clip.trimmed_end(new_end, source_frames))
            }
            DragKind::FadeIn => {
                // The fade-in ends where the handle is.
                let end = shift(clip.start + clip.fade_in);
                (
                    *lane,
                    clip.with_fades(end.saturating_sub(clip.start), clip.fade_out),
                )
            }
            DragKind::FadeOut => {
                // The fade-out starts where the handle is.
                let start = shift(clip.end() - clip.fade_out);
                (
                    *lane,
                    clip.with_fades(clip.fade_in, clip.end().saturating_sub(start)),
                )
            }
            DragKind::Seek | DragKind::Select => return,
        });
        drop(drag_slot);
        self.queue_draw();
    }

    fn drag_end(&self) {
        self.stop_autoscroll();
        let Some(drag) = self.imp().drag.take() else {
            return;
        };
        self.queue_draw();
        if !drag.moved {
            // A click: the clip (or nothing) was selected in drag_begin; move the
            // playhead here too. (On the ruler it already moved on press.)
            if drag.kind != DragKind::Seek {
                self.seek_to_x(drag.start_x);
            }
            return;
        }
        if let Some(selection) = drag.range_preview.filter(|s| !s.range.is_empty()) {
            let lanes = [selection.first_lane, selection.last_lane]
                .map(|lane| u32::try_from(lane).unwrap_or(0));
            self.emit_by_name::<()>(
                "range-selected",
                &[
                    &selection.range.start,
                    &selection.range.end,
                    &lanes[0],
                    &lanes[1],
                ],
            );
            return;
        }
        let (Some((lane, clip)), Some((old_lane, original))) = (drag.preview, drag.original) else {
            return;
        };
        if lane == old_lane && clip == original.clip {
            return;
        }
        let lane = u32::try_from(lane).unwrap_or(0);
        self.emit_by_name::<()>(
            "clip-changed",
            &[
                &clip.id.0,
                &clip.start,
                &clip.offset,
                &clip.length,
                &clip.fade_in,
                &clip.fade_out,
                &lane,
            ],
        );
    }

    /// How fast the view should scroll for the drag in progress (pixels per
    /// second), or `None` when there is no drag that has started moving.
    fn autoscroll_speed_now(&self) -> Option<f64> {
        let drag = self.imp().drag.borrow();
        let drag = drag.as_ref().filter(|drag| drag.moved)?;
        Some(autoscroll_speed(
            drag.start_x + drag.last_dx,
            f64::from(self.width()),
        ))
    }

    /// Starts auto-scrolling when the dragging pointer is near an edge. The
    /// per-frame callback scrolls, updates the preview, and stops by itself when
    /// the pointer leaves the edge zone or the drag ends.
    fn update_autoscroll(&self) {
        let imp = self.imp();
        if imp.autoscroll_tick.borrow().is_some()
            || self
                .autoscroll_speed_now()
                .is_none_or(|speed| speed.abs() < f64::EPSILON)
        {
            return;
        }
        let last_frame_time = Cell::new(None);
        let id = self.add_tick_callback(move |view, clock| {
            let Some(speed) = view
                .autoscroll_speed_now()
                .filter(|speed| speed.abs() >= f64::EPSILON)
            else {
                // The callback is finished: forget its id, there is nothing to remove.
                view.imp().autoscroll_tick.take();
                return glib::ControlFlow::Break;
            };
            let now = clock.frame_time();
            if let Some(before) = last_frame_time.replace(Some(now)) {
                view.scroll_by_pixels(speed * microseconds_to_seconds(now - before));
                // The pointer did not move, but the audio under it did.
                let (dx, dy) = view
                    .imp()
                    .drag
                    .borrow()
                    .as_ref()
                    .map_or((0.0, 0.0), |drag| (drag.last_dx, drag.last_dy));
                view.update_preview(dx, dy);
            }
            glib::ControlFlow::Continue
        });
        imp.autoscroll_tick.replace(Some(id));
    }

    fn stop_autoscroll(&self) {
        if let Some(id) = self.imp().autoscroll_tick.take() {
            id.remove();
        }
    }

    /// Whether clip `id` is visible at pixel column `x`, given a lane's visible
    /// `parts`. A fade handle where another clip covers the clip is not shown
    /// (that part is not heard anyway).
    pub(super) fn is_visible_at(&self, parts: &[Clip], id: ClipId, x: f64) -> bool {
        let start = self.start_frame();
        let frames_per_pixel = self.imp().frames_per_pixel.get();
        let to_x = |frame: Frames| (frames_to_f64(frame) - start) / frames_per_pixel;
        parts
            .iter()
            .filter(|part| part.id == id)
            .any(|part| (to_x(part.start)..=to_x(part.end())).contains(&x))
    }

    /// Where a clip's fade-in and fade-out handles are drawn, in pixels: at the end
    /// of the fade-in and the start of the fade-out, kept just inside the clip.
    pub(super) fn fade_handle_positions(&self, clip: &Clip) -> (f64, f64) {
        let start = self.start_frame();
        let frames_per_pixel = self.imp().frames_per_pixel.get();
        let x = |frame: Frames| (frames_to_f64(frame) - start) / frames_per_pixel;
        let (left, right) = (x(clip.start), x(clip.end()));
        let inset = super::FADE_HANDLE_INSET.min((right - left) / 4.0);
        (
            x(clip.start + clip.fade_in).max(left + inset),
            x(clip.end() - clip.fade_out).min(right - inset),
        )
    }

    /// All clip edges except those of `except`, plus the playhead and the start.
    fn snap_points(&self, except: Option<ClipId>) -> Vec<Frames> {
        let mut points: Vec<Frames> = self
            .imp()
            .lanes
            .borrow()
            .iter()
            .flat_map(|lane| &lane.clips)
            .filter(|clip| Some(clip.clip.id) != except)
            .flat_map(|clip| [clip.clip.start, clip.clip.end()])
            .collect();
        points.push(0);
        points.push(self.imp().playhead.get());
        points
    }
}
