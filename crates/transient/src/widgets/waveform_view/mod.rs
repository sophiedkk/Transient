//! `WaveformView`: draws a waveform with a playhead, and handles seeking,
//! scrolling and zooming.
//!
//! - Click or drag: emits `seek` with the frame under the pointer.
//! - Scroll: moves through the file. Ctrl+scroll: zooms around the pointer.
//! - The visible range lives in a `gtk::Adjustment` (seconds), which a
//!   `gtk::Scrollbar` can share, giving a scrollbar for free.
//!
//! Drawing happens in [`WaveformView::draw`]: the waveform is one filled shape
//! (top edge along the maxima, back along the minima), in the accent colour.

mod imp;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gdk, glib, graphene, gsk};
use transient_core::time::{Frames, frames_from_f64, frames_to_f64};
use transient_core::waveform::Waveform;

/// The furthest zoom in: 16 pixels per frame.
const MIN_FRAMES_PER_PIXEL: f64 = 1.0 / 16.0;
/// Each zoom step halves or doubles the visible length.
const ZOOM_STEP: f64 = 2.0;
/// While following the playhead, keep it this far (fraction of the view) from the edge.
const FOLLOW_MARGIN: f64 = 0.05;
/// The waveform uses this much of the half-height, leaving a little room.
const HEADROOM: f32 = 0.95;

glib::wrapper! {
    /// Draws a waveform with a playhead.
    pub(crate) struct WaveformView(ObjectSubclass<imp::WaveformView>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl WaveformView {
    /// Shows `waveform` (or nothing), zoomed to fit, with the playhead at the start.
    pub(crate) fn set_waveform(&self, waveform: Option<Waveform>) {
        let imp = self.imp();
        imp.waveform.replace(waveform);
        imp.playhead.set(0);
        imp.fit.set(true);
        if let Some(adjustment) = self.adjustment() {
            adjustment.set_value(0.0);
        }
        self.update_adjustment();
        self.queue_draw();
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

    /// Shows the whole file.
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

    /// The sample rate and length of the waveform, if one is set.
    fn rate_and_length(&self) -> Option<(f64, Frames)> {
        let waveform = self.imp().waveform.borrow();
        let buffer = waveform.as_ref()?.buffer();
        Some((f64::from(buffer.sample_rate().hz()), buffer.frames()))
    }

    /// The zoom level that shows the whole file in the current width.
    fn fit_frames_per_pixel(&self) -> Option<f64> {
        let (_, length) = self.rate_and_length()?;
        let width = f64::from(self.width());
        (width > 0.0).then(|| (frames_to_f64(length) / width).max(MIN_FRAMES_PER_PIXEL))
    }

    /// The frame at the left edge (fractional when zoomed in far).
    fn start_frame(&self) -> f64 {
        let rate = self.rate_and_length().map_or(0.0, |(rate, _)| rate);
        self.adjustment().map_or(0.0, |a| a.value()) * rate
    }

    /// Brings the adjustment in line with the waveform, width and zoom level.
    pub(super) fn update_adjustment(&self) {
        let imp = self.imp();
        let Some(adjustment) = self.adjustment() else {
            return;
        };
        let (Some((rate, length)), Some(fit)) =
            (self.rate_and_length(), self.fit_frames_per_pixel())
        else {
            adjustment.configure(0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
            return;
        };
        let frames_per_pixel = imp.frames_per_pixel.get();
        if imp.fit.get() || frames_per_pixel <= 0.0 || frames_per_pixel >= fit {
            imp.frames_per_pixel.set(fit);
            imp.fit.set(true);
        }
        let duration = frames_to_f64(length) / rate;
        let page = (f64::from(self.width()) * imp.frames_per_pixel.get() / rate).min(duration);
        let value = adjustment.value().clamp(0.0, (duration - page).max(0.0));
        adjustment.configure(value, 0.0, duration, page / 10.0, page * 0.9, page);
        self.queue_draw();
    }

    /// Changes the zoom by `factor`, keeping the frame under `anchor_x` in place.
    fn zoom_by(&self, factor: f64, anchor_x: f64) {
        let imp = self.imp();
        let (Some(fit), Some((rate, _)), Some(adjustment)) = (
            self.fit_frames_per_pixel(),
            self.rate_and_length(),
            self.adjustment(),
        ) else {
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
        let (Some(adjustment), Some((rate, _))) = (self.adjustment(), self.rate_and_length())
        else {
            return;
        };
        let start = adjustment.value() * rate;
        let page = adjustment.page_size() * rate;
        let frame = frames_to_f64(frame);
        if frame < start || frame > start + page * (1.0 - FOLLOW_MARGIN) {
            adjustment.set_value((frame - page * FOLLOW_MARGIN) / rate);
        }
    }

    /// The frame under pixel column `x`, limited to the waveform.
    fn frame_at(&self, x: f64) -> Frames {
        let length = self.rate_and_length().map_or(0, |(_, length)| length);
        let frame = self.start_frame() + x.max(0.0) * self.imp().frames_per_pixel.get();
        frames_from_f64(frame.floor()).unwrap_or(0).min(length)
    }

    fn seek_to_x(&self, x: f64) {
        self.emit_by_name::<()>("seek", &[&self.frame_at(x)]);
    }

    /// Mouse and touchpad handling. Called once, from `constructed`.
    fn setup_controllers(&self) {
        // Click and drag: seek. GestureDrag reports the start and the offset since.
        let drag = gtk::GestureDrag::new();
        let weak = self.downgrade();
        drag.connect_drag_begin(move |_, x, _| {
            if let Some(view) = weak.upgrade() {
                view.imp().drag_start_x.set(x);
                view.seek_to_x(x);
            }
        });
        let weak = self.downgrade();
        drag.connect_drag_update(move |_, offset_x, _| {
            if let Some(view) = weak.upgrade() {
                view.seek_to_x(view.imp().drag_start_x.get() + offset_x);
            }
        });
        self.add_controller(drag);

        // Remember the pointer position for Ctrl+scroll zooming.
        let motion = gtk::EventControllerMotion::new();
        let weak = self.downgrade();
        motion.connect_motion(move |_, x, _| {
            if let Some(view) = weak.upgrade() {
                view.imp().pointer_x.set(Some(x));
            }
        });
        let weak = self.downgrade();
        motion.connect_leave(move |_| {
            if let Some(view) = weak.upgrade() {
                view.imp().pointer_x.set(None);
            }
        });
        self.add_controller(motion);

        // Scroll: move through the file. Ctrl+scroll: zoom around the pointer.
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
        let weak = self.downgrade();
        scroll.connect_scroll(move |controller, dx, dy| {
            let Some(view) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if controller
                .current_event_state()
                .contains(gdk::ModifierType::CONTROL_MASK)
            {
                let anchor = view
                    .imp()
                    .pointer_x
                    .get()
                    .unwrap_or_else(|| f64::from(view.width()) / 2.0);
                view.zoom_by(ZOOM_STEP.powf(dy), anchor);
            } else if let Some(adjustment) = view.adjustment() {
                adjustment.set_value(adjustment.value() + (dx + dy) * adjustment.page_size() / 5.0);
            }
            glib::Propagation::Stop
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

    /// Draws the centre line, the waveform and the playhead.
    fn draw(&self, snapshot: &gtk::Snapshot) {
        let (width, height) = (pixels(self.width()), pixels(self.height()));
        if width <= 0.0 || height <= 0.0 {
            return;
        }
        let middle = height / 2.0;
        let foreground = self.color();
        let style = adw::StyleManager::default();
        let accent = style.accent_color().to_standalone_rgba(style.is_dark());

        snapshot.append_color(
            &foreground.with_alpha(0.15),
            &graphene::Rect::new(0.0, middle - 0.5, width, 1.0),
        );

        let frames_per_pixel = self.imp().frames_per_pixel.get();
        let start = self.start_frame();
        if let Some(waveform) = self.imp().waveform.borrow().as_ref() {
            let columns = usize::try_from(self.width()).unwrap_or(0);
            let peaks = waveform.peaks(start, frames_per_pixel, columns);
            if let Some(path) = waveform_path(&peaks, middle) {
                snapshot.append_fill(&path, gsk::FillRule::Winding, &accent);
            }
        }

        let playhead_x = (frames_to_f64(self.imp().playhead.get()) - start) / frames_per_pixel;
        if (0.0..=f64::from(self.width())).contains(&playhead_x) {
            // Keep the 2-pixel line fully visible at the very start and end.
            let x = pixels_f64(playhead_x).clamp(1.0, width - 1.0);
            snapshot.append_color(&foreground, &graphene::Rect::new(x - 1.0, 0.0, 2.0, height));
        }
    }
}

/// The outline of the waveform: along the maxima from left to right, then back
/// along the minima. `None` if no column has audio.
fn waveform_path(
    peaks: &[Option<transient_core::waveform::Peak>],
    middle: f32,
) -> Option<gsk::Path> {
    let to_y = |value: f32| middle - value.clamp(-1.0, 1.0) * middle * HEADROOM;
    // Every column is at least one pixel tall, so silence still shows as a line.
    let columns: Vec<(f32, f32, f32)> = peaks
        .iter()
        .enumerate()
        .filter_map(|(column, peak)| {
            let peak = (*peak)?;
            let x = pixels_f64(f64::from(u32::try_from(column).ok()?)) + 0.5;
            let (top, bottom) = (to_y(peak.max), to_y(peak.min));
            let grow = ((1.0 - (bottom - top)) / 2.0).max(0.0);
            Some((x, top - grow, bottom + grow))
        })
        .collect();
    let (&(first_x, first_top, _), rest) = columns.split_first()?;

    let builder = gsk::PathBuilder::new();
    builder.move_to(first_x, first_top);
    for &(x, top, _) in rest {
        builder.line_to(x, top);
    }
    for &(x, _, bottom) in columns.iter().rev() {
        builder.line_to(x, bottom);
    }
    builder.close();
    Some(builder.to_path())
}

/// A widget size in pixels as `f32`, the unit GTK draws in.
fn pixels(value: i32) -> f32 {
    pixels_f64(f64::from(value))
}

fn pixels_f64(value: f64) -> f32 {
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        reason = "pixel coordinates are small; f32 is what GTK draws with"
    )]
    let value = value as f32;
    value
}
