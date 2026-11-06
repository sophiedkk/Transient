//! `TimelineView`: the timeline. A time ruler on top, one lane per track below
//! it with the clips and their waveforms, and a playhead across everything.
//!
//! - Click on the ruler or an empty lane: emits `seek` with the frame there.
//! - Click a clip: selects it (`clip-selected`). Drag its name strip to move it,
//!   also to another lane or below the last one (a new track); drag its left or
//!   right edge to trim it. Drag over its waveform (or empty lane space) to
//!   select time over the lanes dragged across (`range-selected`). While dragging, the
//!   view shows a preview; on release it emits `clip-changed` (see `interaction.rs`).
//! - Shift+scroll (or a touchpad): moves through the timeline, gliding on after
//!   a swipe. Ctrl+scroll: zooms around the pointer. Pinch: zooms around the
//!   fingers (see `gestures.rs`).
//! - Dragging near the left or right edge scrolls the view (auto-scroll).
//! - The visible range lives in a `gtk::Adjustment` (seconds), which a
//!   `gtk::Scrollbar` can share, giving a scrollbar for free.
//!
//! The lanes line up with the track headers next to the view because both use
//! [`RULER_HEIGHT`] and [`LANE_HEIGHT`]. Drawing is in `draw.rs`.

pub(crate) mod clip_waveform;
mod context_menu;
mod draw;
mod gestures;
mod imp;
mod interaction;
mod keyboard;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use transient_core::project::{Clip, ClipId, TrackColor};
use transient_core::range::TimeRange;
use transient_core::time::{Frames, SampleRate, frames_from_f64, frames_to_f64};
use transient_core::view::start_keeping_anchor;
use transient_core::waveform::Waveform;

use clip_waveform::ClipHeard;

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
/// Empty space before 0:00, in pixels, so that the playhead and the left edge of a
/// clip at the start are not squeezed against the track headers. It scrolls away
/// with the rest of the timeline.
const LEADING_SPACE: f64 = 8.0;

/// A clip as the timeline draws it.
#[derive(Debug, Clone)]
pub(crate) struct LaneClip {
    /// Position, in-point and length (the project's own clip).
    pub(crate) clip: Clip,
    /// The source's waveform overview.
    /// `None` when the audio file could not be found: the clip is drawn grey.
    pub(crate) waveform: Option<Waveform>,
    /// The label drawn on the clip.
    pub(crate) name: String,
    /// Where its audio is in an overview as heard, to draw it with gain,
    /// fades and effects (`None`: not rendered yet; see `window/heard.rs`).
    pub(crate) heard: Option<ClipHeard>,
}

/// A time selection as the timeline shows it: a stretch of time over a run of
/// lanes (from `first_lane` down to `last_lane`, both included).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LaneRange {
    pub(crate) range: TimeRange,
    pub(crate) first_lane: usize,
    pub(crate) last_lane: usize,
}

/// A take being recorded, drawn growing in the empty lane below the last track
/// (see `draw_live_take`).
#[derive(Debug, Clone, Default)]
pub(crate) struct LiveTake {
    /// Where the take starts on the timeline.
    pub(crate) start: Frames,
    /// How long it is so far.
    pub(crate) frames: Frames,
    /// Its outline: the loudest sample in each block of
    /// `transient_engine::recorder::PEAK_BLOCK` frames.
    pub(crate) peaks: Vec<f32>,
}

/// One track's row in the timeline.
#[derive(Debug, Clone, Default)]
pub(crate) struct Lane {
    /// The clips on this track.
    pub(crate) clips: Vec<LaneClip>,
    /// False when the track is muted or silenced by another track's solo;
    /// its clips are then drawn faded.
    pub(crate) audible: bool,
    /// The track's name, for screen readers.
    pub(crate) name: String,
    /// The track's colour (`None`: the accent colour).
    pub(crate) color: Option<TrackColor>,
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
        self.update_description();
    }

    /// Shows the time selection (or none).
    pub(crate) fn set_time_selection(&self, selection: Option<LaneRange>) {
        if self.imp().time_selection.replace(selection) != selection {
            self.queue_draw();
            self.update_description();
        }
    }

    /// Calls `callback` with the time selection the user dragged.
    pub(crate) fn connect_range_selected<F: Fn(&Self, LaneRange) + 'static>(&self, callback: F) {
        self.connect_closure(
            "range-selected",
            false,
            glib::closure_local!(
                move |view: Self, start: u64, end: u64, first: u32, last: u32| {
                    let lane = |index: u32| usize::try_from(index).unwrap_or(0);
                    callback(
                        &view,
                        LaneRange {
                            range: TimeRange { start, end },
                            first_lane: lane(first),
                            last_lane: lane(last),
                        },
                    );
                }
            ),
        );
    }

    /// Starts drawing a take being recorded from `start` (empty at first).
    pub(crate) fn start_live_take(&self, start: Frames, sample_rate: SampleRate) {
        let imp = self.imp();
        if imp.sample_rate.get().is_none() {
            imp.sample_rate.set(Some(sample_rate));
        }
        imp.live_take.replace(Some(LiveTake {
            start,
            ..LiveTake::default()
        }));
        self.queue_resize();
    }

    /// The take being recorded grew to `frames`, with these new outline values.
    pub(crate) fn extend_live_take(&self, peaks: &[f32], frames: Frames) {
        let imp = self.imp();
        let end = {
            let mut take = imp.live_take.borrow_mut();
            let Some(take) = take.as_mut() else {
                return;
            };
            take.peaks.extend_from_slice(peaks);
            take.frames = frames;
            take.start + frames
        };
        // The timeline grows with the take (recording past the end).
        if end > imp.length.get() {
            imp.length.set(end);
            self.update_adjustment();
        }
        self.queue_draw();
    }

    /// Stops drawing the take (recording stopped; it becomes a clip).
    pub(crate) fn end_live_take(&self) {
        if self.imp().live_take.take().is_some() {
            self.queue_resize();
            self.queue_draw();
        }
    }

    /// Highlights the selected clips: `primary` (the last one clicked, which
    /// gets the fade handles) and `all` (including the primary one).
    pub(crate) fn set_selection(&self, primary: Option<ClipId>, all: &[ClipId]) {
        let imp = self.imp();
        let changed =
            imp.selected.replace(primary) != primary || imp.selection.borrow().as_slice() != all;
        if changed {
            imp.selection.replace(all.to_vec());
            self.queue_draw();
        }
    }

    /// Whether `id` is one of the selected clips.
    fn is_selected(&self, id: ClipId) -> bool {
        self.imp().selection.borrow().contains(&id)
    }

    /// Calls `callback` with how far the selected clips were dragged together:
    /// frames to the right and lanes down (negative: left or up).
    pub(crate) fn connect_clips_moved<F: Fn(&Self, i64, i32) + 'static>(&self, callback: F) {
        self.connect_closure(
            "clips-moved",
            false,
            glib::closure_local!(move |view: Self, frames: i64, lanes: i32| {
                callback(&view, frames, lanes);
            }),
        );
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
        // Only while focused: during playback this runs every frame.
        if self.has_focus() {
            self.update_description();
        }
    }

    /// Whether the view has a size yet (it has none until it is first shown).
    pub(crate) fn is_laid_out(&self) -> bool {
        self.width() > 0
    }

    /// Zooms in by `steps` steps (out for a negative number), around the playhead
    /// if it is visible. One step in shows `ZOOM_STEP` times fewer frames per pixel.
    pub(crate) fn zoom(&self, steps: f64) {
        self.zoom_by(ZOOM_STEP.powf(-steps), self.zoom_anchor());
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

    /// The zoom level that shows the whole timeline (and the space around it).
    fn fit_frames_per_pixel(&self) -> Option<f64> {
        self.rate()?;
        let width = f64::from(self.width()) - LEADING_SPACE;
        (width > 0.0).then(|| (self.scroll_length() / width).max(MIN_FRAMES_PER_PIXEL))
    }

    /// The frame at the left edge (fractional when zoomed in far). Scrolled all
    /// the way to the start this is slightly negative: [`LEADING_SPACE`] pixels
    /// before 0:00. Every pixel ↔ frame conversion goes through this, so the
    /// space is the same for drawing, clicking and dragging.
    fn start_frame(&self) -> f64 {
        let rate = self.rate().unwrap_or(0.0);
        self.adjustment().map_or(0.0, |a| a.value()) * rate
            - LEADING_SPACE * self.imp().frames_per_pixel.get()
    }

    /// Scrolls so that the left edge shows `frame` (the opposite of `start_frame`).
    fn set_start_frame(&self, frame: f64) {
        if let (Some(adjustment), Some(rate)) = (self.adjustment(), self.rate()) {
            let value = frame + LEADING_SPACE * self.imp().frames_per_pixel.get();
            adjustment.set_value(value / rate);
        }
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
        // The leading space is not part of the scrollable range.
        let visible_width = (f64::from(self.width()) - LEADING_SPACE).max(0.0);
        let page = (visible_width * imp.frames_per_pixel.get() / rate).min(duration);
        let value = adjustment.value().clamp(0.0, (duration - page).max(0.0));
        adjustment.configure(value, 0.0, duration, page / 10.0, page * 0.9, page);
        self.queue_draw();
    }

    /// Changes the zoom by `factor`, keeping the frame under `anchor_x` in place.
    fn zoom_by(&self, factor: f64, anchor_x: f64) {
        let old = self.imp().frames_per_pixel.get();
        let anchor_frame = self.start_frame() + anchor_x * old;
        self.zoom_to(old * factor, anchor_frame, anchor_x);
    }

    /// Zooms to `frames_per_pixel` (kept between the furthest zoom in and "show
    /// everything"), with `anchor_frame` drawn at pixel column `anchor_x`.
    fn zoom_to(&self, frames_per_pixel: f64, anchor_frame: f64, anchor_x: f64) {
        let imp = self.imp();
        let Some(fit) = self.fit_frames_per_pixel() else {
            return;
        };
        let new = frames_per_pixel.clamp(MIN_FRAMES_PER_PIXEL, fit);
        imp.frames_per_pixel.set(new);
        imp.fit.set(new >= fit);
        self.update_adjustment();
        self.set_start_frame(start_keeping_anchor(anchor_frame, anchor_x, new));
    }

    /// Scrolls sideways by `pixels` (negative: towards the start). Returns false
    /// when the view could not move because it is already at the start or end.
    fn scroll_by_pixels(&self, pixels: f64) -> bool {
        let (Some(adjustment), Some(rate)) = (self.adjustment(), self.rate()) else {
            return false;
        };
        if pixels.abs() < f64::EPSILON {
            return true;
        }
        let before = adjustment.value();
        // The adjustment counts in seconds; `set_value` keeps it inside the timeline.
        adjustment.set_value(before + pixels * self.imp().frames_per_pixel.get() / rate);
        (adjustment.value() - before).abs() > f64::EPSILON
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
        let start = self.start_frame();
        let page = f64::from(self.width()) * self.imp().frames_per_pixel.get();
        let frame = frames_to_f64(frame);
        if frame < start || frame > start + page * (1.0 - FOLLOW_MARGIN) {
            self.set_start_frame(frame - page * FOLLOW_MARGIN);
        }
    }

    /// The frame under pixel column `x`, limited to the timeline.
    fn frame_at(&self, x: f64) -> Frames {
        let frame = self.start_frame() + x.max(0.0) * self.imp().frames_per_pixel.get();
        frames_from_f64(frame.floor())
            .unwrap_or(0)
            .min(self.imp().length.get())
    }

    /// Calls `callback` with the clip the user clicked (or `None` for empty space)
    /// and whether Ctrl or Shift was held (add it to the selection, or remove it).
    pub(crate) fn connect_clip_selected<F: Fn(&Self, Option<ClipId>, bool) + 'static>(
        &self,
        callback: F,
    ) {
        self.connect_closure(
            "clip-selected",
            false,
            glib::closure_local!(move |view: Self, id: u64, extend: bool| {
                callback(&view, (id != 0).then_some(ClipId(id)), extend);
            }),
        );
    }

    /// Calls `callback` when the user clicks empty space (not a clip or the ruler,
    /// and without dragging).
    pub(crate) fn connect_empty_clicked<F: Fn(&Self) + 'static>(&self, callback: F) {
        self.connect_closure(
            "empty-clicked",
            false,
            glib::closure_local!(move |view: Self| callback(&view)),
        );
    }

    /// Calls `callback` with the clip the user double-clicked (to show its
    /// properties).
    pub(crate) fn connect_clip_activated<F: Fn(&Self, ClipId) + 'static>(&self, callback: F) {
        self.connect_closure(
            "clip-activated",
            false,
            glib::closure_local!(move |view: Self, id: u64| callback(&view, ClipId(id))),
        );
    }

    /// Calls `callback` with the changed clip and its (possibly new) lane, after
    /// the user moved or trimmed it. A lane one past the last means "a new track".
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

    /// Mouse, touchpad and touchscreen handling. Called once, from `constructed`.
    fn setup_controllers(&self) {
        let drag = self.drag_gesture();
        self.setup_double_click(&drag);
        // The pinch is added before the drag, so that when a second finger lands
        // the pinch can take over from a drag the first finger started.
        self.setup_pinch(&drag);
        self.add_controller(drag);
        self.setup_scroll();
        self.setup_context_menu();
        self.setup_keyboard();

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
