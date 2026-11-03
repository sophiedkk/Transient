//! `TimelineView`: the timeline. A time ruler on top, one lane per track below
//! it with the clips and their waveforms, and a playhead across everything.
//!
//! - Click on the ruler or an empty lane: emits `seek` with the frame there.
//! - Click a clip: selects it (`clip-selected`). Drag it to move it, also to
//!   another lane; drag its left or right edge to trim it. While dragging, the
//!   view shows a preview; on release it emits `clip-changed` (see `interaction.rs`).
//! - Shift+scroll (or a touchpad): moves through the timeline.
//!   Ctrl+scroll: zooms around the pointer.
//! - The visible range lives in a `gtk::Adjustment` (seconds), which a
//!   `gtk::Scrollbar` can share, giving a scrollbar for free.
//!
//! The lanes line up with the track headers next to the view because both use
//! [`RULER_HEIGHT`] and [`LANE_HEIGHT`]. Drawing is in `draw.rs`.

mod draw;
mod imp;
mod interaction;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gdk, glib};
use transient_core::project::{Clip, ClipId};
use transient_core::time::{Frames, SampleRate, frames_from_f64, frames_to_f64};
use transient_core::waveform::Waveform;

/// Height of the time ruler, in pixels.
pub(crate) const RULER_HEIGHT: i32 = 28;
/// Height of one track lane, in pixels. Track headers use the same height.
pub(crate) const LANE_HEIGHT: i32 = 96;

/// Space between a clip and the top and bottom of its lane, in pixels.
const CLIP_INSET: f32 = 3.0;
/// Height of the clip name strip (where the fade handles are), in pixels.
const CLIP_LABEL_HEIGHT: f32 = 18.0;
/// Fade handles stay at least this far inside the clip edges, in pixels.
const FADE_HANDLE_INSET: f64 = 9.0;

/// The top of lane `index`, in pixels.
fn lane_top_f64(index: usize) -> f64 {
    f64::from(RULER_HEIGHT)
        + f64::from(LANE_HEIGHT) * f64::from(u32::try_from(index).unwrap_or(u32::MAX))
}

/// The furthest zoom in: 16 pixels per frame.
const MIN_FRAMES_PER_PIXEL: f64 = 1.0 / 16.0;
/// Each zoom step halves or doubles the visible length.
const ZOOM_STEP: f64 = 2.0;
/// Empty space after the last clip: this fraction of the timeline's length…
const END_SPACE_FRACTION: f64 = 0.1;
/// …but at least this many seconds.
const END_SPACE_MIN_SECONDS: f64 = 2.0;
/// While following the playhead, keep it this far (fraction of the view) from the edge.
const FOLLOW_MARGIN: f64 = 0.05;

/// A clip as the timeline draws it.
#[derive(Debug, Clone)]
pub(crate) struct LaneClip {
    /// Position, in-point and length (the project's own clip).
    pub(crate) clip: Clip,
    /// The source's waveform overview.
    pub(crate) waveform: Waveform,
    /// The label drawn on the clip.
    pub(crate) name: String,
}

/// One track's row in the timeline.
#[derive(Debug, Clone, Default)]
pub(crate) struct Lane {
    /// The clips on this track.
    pub(crate) clips: Vec<LaneClip>,
    /// False when the track is muted or silenced by another track's solo;
    /// its clips are then drawn faded.
    pub(crate) audible: bool,
}

glib::wrapper! {
    /// The timeline: ruler, track lanes with clips, and the playhead.
    pub(crate) struct TimelineView(ObjectSubclass<imp::TimelineView>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl TimelineView {
    /// Shows `lanes`. `length` is the end of the last clip. The zoom level and
    /// scroll position are kept, unless the view was zoomed to fit.
    pub(crate) fn set_lanes(&self, lanes: Vec<Lane>, sample_rate: SampleRate, length: Frames) {
        let imp = self.imp();
        let lane_count_changed = imp.lanes.borrow().len() != lanes.len();
        imp.lanes.replace(lanes);
        imp.sample_rate.set(Some(sample_rate));
        imp.length.set(length);
        if lane_count_changed {
            // The height depends on the number of lanes.
            self.queue_resize();
        }
        self.update_adjustment();
    }

    /// Highlights the selected clip (or none).
    pub(crate) fn set_selection(&self, clip: Option<ClipId>) {
        if self.imp().selected.replace(clip) != clip {
            self.queue_draw();
        }
    }

    /// The zoom level, as frames per pixel (used for nudging by a few pixels).
    pub(crate) fn frames_per_pixel(&self) -> f64 {
        self.imp().frames_per_pixel.get()
    }

    /// Moves the playhead. With `follow`, scrolls so that it stays visible.
    pub(crate) fn set_playhead(&self, frame: Frames, follow: bool) {
        if self.imp().playhead.replace(frame) == frame {
            return;
        }
        if follow {
            self.scroll_to_show(frame);
        }
        self.queue_draw();
    }

    /// Whether the view has a size yet (it has none until it is first shown).
    pub(crate) fn is_laid_out(&self) -> bool {
        self.width() > 0
    }

    /// Zooms in one step, around the playhead if it is visible.
    pub(crate) fn zoom_in(&self) {
        self.zoom_by(1.0 / ZOOM_STEP, self.zoom_anchor());
    }

    /// Zooms out one step.
    pub(crate) fn zoom_out(&self) {
        self.zoom_by(ZOOM_STEP, self.zoom_anchor());
    }

    /// Shows the whole timeline.
    pub(crate) fn zoom_to_fit(&self) {
        self.imp().fit.set(true);
        self.update_adjustment();
    }

    /// Calls `callback` with the frame when the user clicks or drags to a position.
    pub(crate) fn connect_seek<F: Fn(&Self, Frames) + 'static>(&self, callback: F) {
        self.connect_closure(
            "seek",
            false,
            glib::closure_local!(move |view: Self, frame: u64| callback(&view, frame)),
        );
    }

    /// Frames per second, or `None` while the timeline is empty.
    fn rate(&self) -> Option<f64> {
        let imp = self.imp();
        let rate = imp.sample_rate.get()?;
        (imp.length.get() > 0).then(|| f64::from(rate.hz()))
    }

    /// The scrollable length in frames: the timeline plus some empty space after
    /// it, so the end of the last clip is never against the window edge and can
    /// still be dragged further out after trimming it.
    fn scroll_length(&self) -> f64 {
        let length = frames_to_f64(self.imp().length.get());
        let minimum = self.rate().map_or(0.0, |rate| rate * END_SPACE_MIN_SECONDS);
        length + (length * END_SPACE_FRACTION).max(minimum)
    }

    /// The zoom level that shows the whole timeline (and the space after it).
    fn fit_frames_per_pixel(&self) -> Option<f64> {
        self.rate()?;
        let width = f64::from(self.width());
        (width > 0.0).then(|| (self.scroll_length() / width).max(MIN_FRAMES_PER_PIXEL))
    }

    /// The frame at the left edge (fractional when zoomed in far).
    fn start_frame(&self) -> f64 {
        let rate = self.rate().unwrap_or(0.0);
        self.adjustment().map_or(0.0, |a| a.value()) * rate
    }

    /// Brings the adjustment in line with the timeline, width and zoom level.
    pub(super) fn update_adjustment(&self) {
        let imp = self.imp();
        let Some(adjustment) = self.adjustment() else {
            return;
        };
        let (Some(rate), Some(fit)) = (self.rate(), self.fit_frames_per_pixel()) else {
            adjustment.configure(0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
            self.queue_draw();
            return;
        };
        let frames_per_pixel = imp.frames_per_pixel.get();
        if imp.fit.get() || frames_per_pixel <= 0.0 || frames_per_pixel >= fit {
            imp.frames_per_pixel.set(fit);
            imp.fit.set(true);
        }
        let duration = self.scroll_length() / rate;
        let page = (f64::from(self.width()) * imp.frames_per_pixel.get() / rate).min(duration);
        let value = adjustment.value().clamp(0.0, (duration - page).max(0.0));
        adjustment.configure(value, 0.0, duration, page / 10.0, page * 0.9, page);
        self.queue_draw();
    }

    /// Changes the zoom by `factor`, keeping the frame under `anchor_x` in place.
    fn zoom_by(&self, factor: f64, anchor_x: f64) {
        let imp = self.imp();
        let (Some(fit), Some(rate), Some(adjustment)) =
            (self.fit_frames_per_pixel(), self.rate(), self.adjustment())
        else {
            return;
        };
        let old = imp.frames_per_pixel.get();
        let new = (old * factor).clamp(MIN_FRAMES_PER_PIXEL, fit);
        let anchor_frame = self.start_frame() + anchor_x * old;
        imp.frames_per_pixel.set(new);
        imp.fit.set(new >= fit);
        self.update_adjustment();
        adjustment.set_value((anchor_frame - anchor_x * new) / rate);
    }

    /// Zoom around the playhead when it is visible, otherwise around the middle.
    fn zoom_anchor(&self) -> f64 {
        let width = f64::from(self.width());
        let playhead_x = (frames_to_f64(self.imp().playhead.get()) - self.start_frame())
            / self.imp().frames_per_pixel.get();
        if (0.0..=width).contains(&playhead_x) {
            playhead_x
        } else {
            width / 2.0
        }
    }

    /// Scrolls so that `frame` is visible, a little away from the edge.
    fn scroll_to_show(&self, frame: Frames) {
        let (Some(adjustment), Some(rate)) = (self.adjustment(), self.rate()) else {
            return;
        };
        let start = adjustment.value() * rate;
        let page = adjustment.page_size() * rate;
        let frame = frames_to_f64(frame);
        if frame < start || frame > start + page * (1.0 - FOLLOW_MARGIN) {
            adjustment.set_value((frame - page * FOLLOW_MARGIN) / rate);
        }
    }

    /// The frame under pixel column `x`, limited to the timeline.
    fn frame_at(&self, x: f64) -> Frames {
        let frame = self.start_frame() + x.max(0.0) * self.imp().frames_per_pixel.get();
        frames_from_f64(frame.floor())
            .unwrap_or(0)
            .min(self.imp().length.get())
    }

    /// Calls `callback` with the clip the user clicked (or `None` for empty space).
    pub(crate) fn connect_clip_selected<F: Fn(&Self, Option<ClipId>) + 'static>(
        &self,
        callback: F,
    ) {
        self.connect_closure(
            "clip-selected",
            false,
            glib::closure_local!(move |view: Self, id: u64| {
                callback(&view, (id != 0).then_some(ClipId(id)));
            }),
        );
    }

    /// Calls `callback` with the changed clip and its (possibly new) lane, after
    /// the user moved or trimmed it.
    pub(crate) fn connect_clip_changed<F: Fn(&Self, Clip, usize) + 'static>(&self, callback: F) {
        self.connect_closure(
            "clip-changed",
            false,
            glib::closure_local!(move |view: Self,
                                       id: u64,
                                       start: u64,
                                       offset: u64,
                                       length: u64,
                                       fade_in: u64,
                                       fade_out: u64,
                                       lane: u32| {
                let Some(original) = view.find_clip(ClipId(id)).map(|(_, c)| c.clip) else {
                    return;
                };
                let clip = Clip {
                    start,
                    offset,
                    length,
                    ..original
                }
                .with_fades(fade_in, fade_out);
                callback(&view, clip, usize::try_from(lane).unwrap_or(0));
            }),
        );
    }

    /// The lane index and drawing data of a clip.
    fn find_clip(&self, id: ClipId) -> Option<(usize, LaneClip)> {
        self.imp()
            .lanes
            .borrow()
            .iter()
            .enumerate()
            .find_map(|(index, lane)| {
                let clip = lane.clips.iter().find(|c| c.clip.id == id)?;
                Some((index, clip.clone()))
            })
    }

    fn seek_to_x(&self, x: f64) {
        if self.rate().is_some() {
            self.emit_by_name::<()>("seek", &[&self.frame_at(x)]);
        }
    }

    /// Mouse and touchpad handling. Called once, from `constructed`.
    fn setup_controllers(&self) {
        self.setup_drag();

        // Remember the pointer position for Ctrl+scroll zooming.
        let motion = gtk::EventControllerMotion::new();
        let weak = self.downgrade();
        motion.connect_motion(move |_, x, y| {
            if let Some(view) = weak.upgrade() {
                view.imp().pointer_x.set(Some(x));
                view.update_cursor(x, y);
            }
        });
        let weak = self.downgrade();
        motion.connect_leave(move |_| {
            if let Some(view) = weak.upgrade() {
                view.imp().pointer_x.set(None);
            }
        });
        self.add_controller(motion);

        // Horizontal scroll moves through the timeline; Ctrl+scroll zooms around the
        // pointer. Plain vertical scrolling is left to the scrolled window around the
        // view, which scrolls through the tracks.
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
        let weak = self.downgrade();
        scroll.connect_scroll(move |controller, dx, dy| {
            let Some(view) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            let state = controller.current_event_state();
            if state.contains(gdk::ModifierType::CONTROL_MASK) {
                let anchor = view
                    .imp()
                    .pointer_x
                    .get()
                    .unwrap_or_else(|| f64::from(view.width()) / 2.0);
                view.zoom_by(ZOOM_STEP.powf(dy), anchor);
                return glib::Propagation::Stop;
            }
            // Shift+wheel is the usual way to scroll sideways with a mouse.
            let sideways = if state.contains(gdk::ModifierType::SHIFT_MASK) {
                dx + dy
            } else {
                dx
            };
            match view.adjustment() {
                Some(adjustment) if sideways != 0.0 => {
                    adjustment
                        .set_value(adjustment.value() + sideways * adjustment.page_size() / 5.0);
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        self.add_controller(scroll);

        // Redraw when the accent colour or light/dark style changes.
        let style = adw::StyleManager::default();
        let weak = self.downgrade();
        style.connect_accent_color_notify(move |_| {
            if let Some(view) = weak.upgrade() {
                view.queue_draw();
            }
        });
        let weak = self.downgrade();
        style.connect_dark_notify(move |_| {
            if let Some(view) = weak.upgrade() {
                view.queue_draw();
            }
        });
    }
}
