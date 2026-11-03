//! Mouse interaction: what is under the pointer, and dragging.
//!
//! One `GestureDrag` handles every kind of drag; what it does is decided when
//! the button goes down, by what is under the pointer:
//!
//! | Under the pointer          | Drag does                       |
//! |----------------------------|---------------------------------|
//! | the ruler or an empty lane | moves the playhead              |
//! | the middle of a clip       | moves the clip (also to another lane) |
//! | a clip's left/right edge   | trims the clip                  |
//!
//! While dragging, only a *preview* changes (drawn by `draw.rs`). The project
//! changes once, on release, through the `clip-changed` signal, so one drag is
//! one undo step. Edges snap to other clips' edges, the playhead and the start.

use adw::prelude::*;
use adw::subclass::prelude::*;
use transient_core::edit::snap;
use transient_core::project::{Clip, ClipId};
use transient_core::time::{Frames, frames_from_f64, frames_to_f64};

use super::{LANE_HEIGHT, LaneClip, RULER_HEIGHT, TimelineView};

/// How close (in pixels) to a clip's edge counts as grabbing the edge.
const EDGE_GRAB: f64 = 6.0;
/// How close (in pixels) an edge has to come to a snap point to snap to it.
const SNAP_DISTANCE: f64 = 8.0;
/// A press that moves less than this (in pixels) is a click, not a drag.
const CLICK_SLOP: f64 = 3.0;

/// The part of a clip under the pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Zone {
    LeftEdge,
    Body,
    RightEdge,
}

/// What is under the pointer.
#[derive(Debug, Clone)]
enum Hit {
    /// The time ruler, or below the last lane.
    Ruler,
    /// A lane, but no clip.
    Empty,
    /// A clip (its lane index, its data and which part of it).
    Clip(usize, Box<LaneClip>, Zone),
}

/// What a drag does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DragKind {
    Seek,
    Move,
    TrimStart,
    TrimEnd,
}

/// A drag in progress.
#[derive(Debug, Clone)]
pub(crate) struct Drag {
    kind: DragKind,
    start_x: f64,
    start_y: f64,
    /// The clip as it was when the drag began, and its lane.
    original: Option<(usize, LaneClip)>,
    /// Where the clip would end up if released now, and in which lane.
    pub(super) preview: Option<(usize, Clip)>,
    /// The other clips' edges, the playhead and the start.
    snap_points: Vec<Frames>,
    /// Whether the pointer moved far enough to count as a drag.
    moved: bool,
}

impl TimelineView {
    /// Sets up the drag gesture. Called once, from `setup_controllers`.
    pub(super) fn setup_drag(&self) {
        let drag = gtk::GestureDrag::new();
        let weak = self.downgrade();
        drag.connect_drag_begin(move |_, x, y| {
            if let Some(view) = weak.upgrade() {
                view.drag_begin(x, y);
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
        self.add_controller(drag);
    }

    /// Shows a resize cursor over clip edges.
    pub(super) fn update_cursor(&self, x: f64, y: f64) {
        let cursor = match self.hit(x, y) {
            Hit::Clip(_, _, Zone::LeftEdge | Zone::RightEdge) => Some("col-resize"),
            _ => None,
        };
        self.set_cursor_from_name(cursor);
    }

    /// The lane at height `y`, if it is inside a lane.
    fn lane_at(&self, y: f64) -> Option<usize> {
        let lanes = self.imp().lanes.borrow().len();
        let offset = y - f64::from(RULER_HEIGHT);
        if offset < 0.0 {
            return None;
        }
        let index = frames_from_f64((offset / f64::from(LANE_HEIGHT)).floor())?;
        usize::try_from(index).ok().filter(|&index| index < lanes)
    }

    /// What is at (`x`, `y`). Later clips are drawn on top, so they win.
    fn hit(&self, x: f64, y: f64) -> Hit {
        let Some(lane) = self.lane_at(y) else {
            return Hit::Ruler;
        };
        let lanes = self.imp().lanes.borrow();
        let Some(clips) = lanes.get(lane).map(|lane| &lane.clips) else {
            return Hit::Ruler;
        };
        let start = self.start_frame();
        let frames_per_pixel = self.imp().frames_per_pixel.get();
        for clip in clips.iter().rev() {
            let left = (frames_to_f64(clip.clip.start) - start) / frames_per_pixel;
            let right = (frames_to_f64(clip.clip.end()) - start) / frames_per_pixel;
            if x < left || x > right {
                continue;
            }
            // Narrow clips: keep a grabbable middle by shrinking the edge zones.
            let grab = EDGE_GRAB.min((right - left) / 3.0);
            let zone = if x - left < grab {
                Zone::LeftEdge
            } else if right - x < grab {
                Zone::RightEdge
            } else {
                Zone::Body
            };
            return Hit::Clip(lane, Box::new(clip.clone()), zone);
        }
        Hit::Empty
    }

    fn drag_begin(&self, x: f64, y: f64) {
        let hit = self.hit(x, y);
        let (kind, original) = match hit {
            Hit::Ruler => (DragKind::Seek, None),
            Hit::Empty => {
                self.emit_by_name::<()>("clip-selected", &[&0_u64]);
                (DragKind::Seek, None)
            }
            Hit::Clip(lane, clip, zone) => {
                self.emit_by_name::<()>("clip-selected", &[&clip.clip.id.0]);
                let kind = match zone {
                    Zone::LeftEdge => DragKind::TrimStart,
                    Zone::Body => DragKind::Move,
                    Zone::RightEdge => DragKind::TrimEnd,
                };
                (kind, Some((lane, *clip)))
            }
        };
        if kind == DragKind::Seek {
            self.seek_to_x(x);
        }
        let except = original.as_ref().map(|(_, clip)| clip.clip.id);
        let drag = Drag {
            kind,
            start_x: x,
            start_y: y,
            snap_points: self.snap_points(except),
            original,
            preview: None,
            moved: false,
        };
        self.imp().drag.replace(Some(drag));
    }

    fn drag_update(&self, dx: f64, dy: f64) {
        let mut drag_slot = self.imp().drag.borrow_mut();
        let Some(drag) = drag_slot.as_mut() else {
            return;
        };
        if !drag.moved && dx.abs() < CLICK_SLOP && dy.abs() < CLICK_SLOP {
            return;
        }
        drag.moved = true;
        let frames_per_pixel = self.imp().frames_per_pixel.get();
        let tolerance = frames_from_f64(SNAP_DISTANCE * frames_per_pixel).unwrap_or(0);
        let shift = |frame: Frames| {
            frames_from_f64((frames_to_f64(frame) + dx * frames_per_pixel).max(0.0)).unwrap_or(0)
        };

        let Some((lane, original)) = &drag.original else {
            // A seek drag: follow the pointer.
            let x = drag.start_x + dx;
            drop(drag_slot);
            self.seek_to_x(x);
            return;
        };
        let clip = original.clip;
        let source_frames = original.waveform.buffer().frames();
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
                let new_lane = self.lane_at(drag.start_y + dy).unwrap_or(*lane);
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
            DragKind::Seek => return,
        });
        drop(drag_slot);
        self.queue_draw();
    }

    fn drag_end(&self) {
        let Some(drag) = self.imp().drag.take() else {
            return;
        };
        self.queue_draw();
        if !drag.moved {
            // A click on a clip: select it (done in drag_begin) and move the playhead.
            if drag.original.is_some() {
                self.seek_to_x(drag.start_x);
            }
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
            &[&clip.id.0, &clip.start, &clip.offset, &clip.length, &lane],
        );
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
