//! Moving around the timeline with a touchpad, touchscreen or mouse wheel:
//!
//! - Scrolling sideways (touchpad, Shift+wheel) moves through time. On a
//!   touchpad, the view keeps gliding for a moment after a quick swipe
//!   ("kinetic scrolling").
//! - Ctrl+scroll zooms around the pointer.
//! - Pinching (touchpad or touchscreen) zooms around the fingers. The audio
//!   under the fingers stays under them, so moving both fingers also scrolls.
//!
//! The maths (glide, zoom anchor) lives in `transient_core::view`.

use std::cell::Cell;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gdk, glib};
use transient_core::view::{Glide, pinch_frames_per_pixel};

use super::{TimelineView, ZOOM_STEP};

/// A pinch in progress: the zoom level when it began, and the frame that was
/// between the fingers then (it stays between them).
#[derive(Debug, Clone, Copy)]
pub(super) struct Pinch {
    start_frames_per_pixel: f64,
    anchor_frame: f64,
}

impl TimelineView {
    /// Sets up scrolling: sideways, zooming with Ctrl, and gliding after a swipe.
    pub(super) fn setup_scroll(&self) {
        // KINETIC makes GTK report the swipe speed (`decelerate`) when the fingers
        // leave the touchpad; we then glide on ourselves.
        let scroll = gtk::EventControllerScroll::new(
            gtk::EventControllerScrollFlags::BOTH_AXES | gtk::EventControllerScrollFlags::KINETIC,
        );
        let weak = self.downgrade();
        scroll.connect_scroll_begin(move |_| {
            // Fingers on the touchpad again: stop gliding.
            if let Some(view) = weak.upgrade() {
                view.stop_glide();
            }
        });
        let weak = self.downgrade();
        scroll.connect_scroll(move |controller, dx, dy| {
            let Some(view) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            view.scrolled(controller, dx, dy)
        });
        let weak = self.downgrade();
        scroll.connect_decelerate(move |controller, velocity_x, _| {
            // Only touchpads report their speed in pixels; a mouse wheel has no glide.
            if let Some(view) = weak.upgrade()
                && controller.unit() == gdk::ScrollUnit::Surface
            {
                view.start_glide(velocity_x);
            }
        });
        self.add_controller(scroll);
    }

    /// Handles one scroll event. `Stop` means we used it; `Proceed` passes it on
    /// to the scrolled window around the view, which scrolls through the tracks.
    fn scrolled(
        &self,
        controller: &gtk::EventControllerScroll,
        dx: f64,
        dy: f64,
    ) -> glib::Propagation {
        self.stop_glide();
        let state = controller.current_event_state();
        if state.contains(gdk::ModifierType::CONTROL_MASK) {
            let anchor = self
                .imp()
                .pointer_x
                .get()
                .unwrap_or_else(|| f64::from(self.width()) / 2.0);
            self.zoom_by(ZOOM_STEP.powf(dy), anchor);
            return glib::Propagation::Stop;
        }
        // Shift+wheel is the usual way to scroll sideways with a mouse.
        let sideways = if state.contains(gdk::ModifierType::SHIFT_MASK) {
            dx + dy
        } else {
            dx
        };
        if sideways.abs() < f64::EPSILON {
            return glib::Propagation::Proceed;
        }
        match controller.unit() {
            // A touchpad reports pixels: the timeline follows the fingers exactly.
            gdk::ScrollUnit::Surface => {
                self.scroll_by_pixels(sideways);
            }
            // A mouse wheel reports clicks: each one moves a fifth of the view.
            _ => {
                if let Some(adjustment) = self.adjustment() {
                    adjustment
                        .set_value(adjustment.value() + sideways * adjustment.page_size() / 5.0);
                }
            }
        }
        glib::Propagation::Stop
    }

    /// Glides on after a swipe of `velocity` pixels per second, slowing down
    /// smoothly. A tick callback runs once per frame the screen draws.
    fn start_glide(&self, velocity: f64) {
        self.stop_glide();
        let Some(glide) = Glide::new(velocity) else {
            return;
        };
        // The callback is called again every frame; `Cell`s let it keep state.
        let glide = Cell::new(glide);
        let last_frame_time = Cell::new(None);
        let id = self.add_tick_callback(move |view, clock| {
            let now = clock.frame_time();
            let Some(before) = last_frame_time.replace(Some(now)) else {
                return glib::ControlFlow::Continue;
            };
            let mut current = glide.get();
            let step = current.step(microseconds_to_seconds(now - before));
            glide.set(current);
            match step {
                // Keep going until the glide stops or reaches the end of the timeline.
                Some(distance) if view.scroll_by_pixels(distance) => glib::ControlFlow::Continue,
                _ => {
                    // Forget the id: the callback is finished, there is nothing to remove.
                    view.imp().glide_tick.take();
                    glib::ControlFlow::Break
                }
            }
        });
        self.imp().glide_tick.replace(Some(id));
    }

    /// Stops a glide, if one is running.
    pub(super) fn stop_glide(&self) {
        if let Some(id) = self.imp().glide_tick.take() {
            id.remove();
        }
    }

    /// Sets up pinch to zoom. `drag` is the clip/playhead drag gesture: the two
    /// are put in one group, so a second finger can turn a one-finger drag into
    /// a pinch (see `pinch_begin`).
    pub(super) fn setup_pinch(&self, drag: &gtk::GestureDrag) {
        let zoom = gtk::GestureZoom::new();
        zoom.group_with(drag);
        let weak = self.downgrade();
        zoom.connect_begin(move |gesture, _| {
            if let Some(view) = weak.upgrade() {
                view.pinch_begin(gesture);
            }
        });
        let weak = self.downgrade();
        zoom.connect_scale_changed(move |gesture, scale| {
            if let Some(view) = weak.upgrade() {
                view.pinch_update(gesture, scale);
            }
        });
        let weak = self.downgrade();
        zoom.connect_end(move |_, _| {
            if let Some(view) = weak.upgrade() {
                view.imp().pinch.set(None);
            }
        });
        self.add_controller(zoom);
    }

    fn pinch_begin(&self, gesture: &gtk::GestureZoom) {
        self.stop_glide();
        // A pinch is not a drag: drop any drag the first finger started, without
        // changing the clip.
        self.cancel_drag();
        let frames_per_pixel = self.imp().frames_per_pixel.get();
        let centre_x = self.pinch_centre_x(gesture);
        self.imp().pinch.set(Some(Pinch {
            start_frames_per_pixel: frames_per_pixel,
            anchor_frame: self.start_frame() + centre_x * frames_per_pixel,
        }));
        // Ours: the scrolled window around the view must not scroll as well.
        gesture.set_state(gtk::EventSequenceState::Claimed);
    }

    fn pinch_update(&self, gesture: &gtk::GestureZoom, scale: f64) {
        let Some(pinch) = self.imp().pinch.get() else {
            return;
        };
        self.zoom_to(
            pinch_frames_per_pixel(pinch.start_frames_per_pixel, scale),
            pinch.anchor_frame,
            self.pinch_centre_x(gesture),
        );
    }

    /// The point between the fingers (for a touchpad: the pointer).
    fn pinch_centre_x(&self, gesture: &gtk::GestureZoom) -> f64 {
        gesture
            .bounding_box_center()
            .map_or_else(|| f64::from(self.width()) / 2.0, |(x, _)| x)
    }
}

/// Converts a difference between two frame-clock times (microseconds) to seconds.
pub(super) fn microseconds_to_seconds(microseconds: i64) -> f64 {
    // Frames are milliseconds apart, so this always fits; a huge gap (the window
    // was hidden) just becomes a long step, after which a glide has stopped.
    f64::from(i32::try_from(microseconds).unwrap_or(i32::MAX)) / 1_000_000.0
}
