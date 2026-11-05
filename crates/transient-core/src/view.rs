//! Moving the timeline view: zooming around a point, gliding on after a swipe
//! (kinetic scrolling) and scrolling by itself while something is dragged near
//! the edge (auto-scroll).
//!
//! This is plain maths in pixels, frames and seconds, so it can be tested here;
//! the timeline widget only feeds in the numbers GTK gives it.

/// After a swipe, the speed halves roughly every `0.7 × GLIDE_TIME_CONSTANT`
/// seconds. 0.325 s feels like GNOME's own kinetic scrolling.
pub const GLIDE_TIME_CONSTANT: f64 = 0.325;
/// A glide slower than this (pixels per second) stops.
pub const GLIDE_STOP_SPEED: f64 = 20.0;
/// How close to the edge of the view (in pixels) a drag starts auto-scrolling.
pub const AUTOSCROLL_ZONE: f64 = 40.0;
/// The fastest auto-scroll, with the pointer at (or past) the edge, in pixels
/// per second.
pub const AUTOSCROLL_MAX_SPEED: f64 = 1200.0;

/// The zoom level during a pinch.
///
/// `scale` is how far the fingers have spread since the pinch began (2 = twice as
/// far apart), `start` the frames per pixel at that moment. Spreading the fingers
/// zooms in, so fewer frames per pixel. A scale that makes no sense (zero,
/// negative, not a number) keeps the zoom.
#[must_use]
pub fn pinch_frames_per_pixel(start: f64, scale: f64) -> f64 {
    if scale.is_finite() && scale > 0.0 {
        start / scale
    } else {
        start
    }
}

/// Where the view has to start (in frames) to keep `anchor_frame` in place.
///
/// That is, `anchor_frame` is drawn at pixel column `anchor_x` with
/// `frames_per_pixel`. Used to zoom around the pointer or the fingers: the audio
/// under them stays under them.
#[must_use]
pub fn start_keeping_anchor(anchor_frame: f64, anchor_x: f64, frames_per_pixel: f64) -> f64 {
    anchor_frame - anchor_x * frames_per_pixel
}

/// A glide after a swipe: the view keeps moving and slows down smoothly.
///
/// The speed decays exponentially (it loses the same fraction every second),
/// which is what makes it feel natural. Because of that, the total distance is
/// known in advance: at most `velocity × GLIDE_TIME_CONSTANT`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glide {
    /// Pixels per second; negative moves to the left.
    velocity: f64,
}

impl Glide {
    /// A glide starting at `velocity` pixels per second, or `None` when that is
    /// too slow to bother (or not a number).
    #[must_use]
    pub fn new(velocity: f64) -> Option<Self> {
        (velocity.is_finite() && velocity.abs() >= GLIDE_STOP_SPEED).then_some(Self { velocity })
    }

    /// Advances the glide by `seconds` and returns how many pixels to move, or
    /// `None` when it has come to a stop.
    pub fn step(&mut self, seconds: f64) -> Option<f64> {
        if !seconds.is_finite() || seconds <= 0.0 {
            return Some(0.0);
        }
        // Exact for exponential decay, so the result does not depend on the frame
        // rate: the distance is the area under the speed curve for this step.
        let decay = (-seconds / GLIDE_TIME_CONSTANT).exp();
        let distance = self.velocity * GLIDE_TIME_CONSTANT * (1.0 - decay);
        self.velocity *= decay;
        if self.velocity.abs() < GLIDE_STOP_SPEED {
            return None;
        }
        Some(distance)
    }
}

/// How fast to auto-scroll, in pixels per second (negative is to the left).
///
/// The pointer is at `x` in a view `width` pixels wide. Zero away from the edges;
/// it speeds up the closer the pointer gets to an edge, and is fastest past it.
#[must_use]
pub fn autoscroll_speed(x: f64, width: f64) -> f64 {
    if !x.is_finite() || !width.is_finite() || width <= 0.0 {
        return 0.0;
    }
    // In a narrow view, keep the middle third free of auto-scrolling.
    let zone = AUTOSCROLL_ZONE.min(width / 3.0);
    let left = ((zone - x) / zone).clamp(0.0, 1.0);
    let right = ((x - (width - zone)) / zone).clamp(0.0, 1.0);
    (right - left) * AUTOSCROLL_MAX_SPEED
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn spreading_the_fingers_zooms_in() {
        assert!(pinch_frames_per_pixel(100.0, 2.0) < 100.0);
        assert!(pinch_frames_per_pixel(100.0, 0.5) > 100.0);
    }

    #[test]
    fn a_nonsense_pinch_keeps_the_zoom() {
        for scale in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!((pinch_frames_per_pixel(100.0, scale) - 100.0).abs() < 1e-9);
        }
    }

    #[test]
    fn slow_swipes_do_not_glide() {
        assert_eq!(Glide::new(GLIDE_STOP_SPEED / 2.0), None);
        assert_eq!(Glide::new(f64::NAN), None);
        assert!(Glide::new(-500.0).is_some());
    }

    #[test]
    fn no_autoscroll_in_the_middle() {
        assert!(autoscroll_speed(500.0, 1000.0).abs() < 1e-9);
    }

    #[test]
    fn autoscroll_goes_towards_the_edge() {
        assert!(autoscroll_speed(5.0, 1000.0) < 0.0);
        assert!(autoscroll_speed(995.0, 1000.0) > 0.0);
    }

    #[test]
    fn autoscroll_is_fastest_past_the_edge() {
        let max = AUTOSCROLL_MAX_SPEED;
        assert!((autoscroll_speed(-50.0, 1000.0) + max).abs() < 1e-9);
        assert!((autoscroll_speed(1050.0, 1000.0) - max).abs() < 1e-9);
    }

    proptest! {
        /// After zooming with an anchor, the anchor frame is drawn at the anchor.
        #[test]
        fn zooming_keeps_the_anchor_in_place(
            anchor_frame in 0.0..1e9_f64,
            anchor_x in 0.0..4000.0_f64,
            frames_per_pixel in 0.0625..1e5_f64,
        ) {
            let start = start_keeping_anchor(anchor_frame, anchor_x, frames_per_pixel);
            let drawn_at = (anchor_frame - start) / frames_per_pixel;
            prop_assert!((drawn_at - anchor_x).abs() < 1e-3);
        }

        /// A glide always stops, never reverses, and never goes further than
        /// `velocity × GLIDE_TIME_CONSTANT`, whatever the frame rate.
        #[test]
        fn a_glide_stops_and_stays_within_its_distance(
            velocity in prop_oneof![-20_000.0..-GLIDE_STOP_SPEED, GLIDE_STOP_SPEED..20_000.0],
            frame_seconds in 0.001..0.1_f64,
        ) {
            let mut glide = Glide::new(velocity).unwrap();
            let mut total = 0.0;
            let mut steps = 0;
            while let Some(distance) = glide.step(frame_seconds) {
                prop_assert!(distance * velocity >= 0.0, "a glide must not reverse");
                total += distance;
                steps += 1;
                prop_assert!(steps < 100_000, "a glide must stop");
            }
            prop_assert!(total.abs() <= (velocity * GLIDE_TIME_CONSTANT).abs() + 1e-6);
        }

        /// Auto-scroll never exceeds its maximum speed, and gets faster (never
        /// slower) the further the pointer is towards the right.
        #[test]
        fn autoscroll_is_bounded_and_grows_towards_the_right(
            x in -200.0..1200.0_f64,
            step in 0.0..50.0_f64,
            width in 1.0..3000.0_f64,
        ) {
            let here = autoscroll_speed(x, width);
            prop_assert!(here.abs() <= AUTOSCROLL_MAX_SPEED + 1e-9);
            prop_assert!(autoscroll_speed(x + step, width) >= here - 1e-9);
        }
    }
}
