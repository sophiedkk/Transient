//! The time ruler above the timeline: where to put ticks and what to write.
//!
//! The interval between labelled ticks is picked from a list of "round" values
//! (1, 2, 5, 10, 15, 30 seconds, 1 minute, …) so that labels never crowd each
//! other at any zoom level.

/// Round tick intervals, in seconds, from very zoomed in to very zoomed out.
const INTERVALS: [f64; 22] = [
    0.001, 0.002, 0.005, 0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0,
    120.0, 300.0, 600.0, 900.0, 1800.0, 3600.0,
];

/// A labelled tick on the ruler.
#[derive(Debug, Clone, PartialEq)]
pub struct Tick {
    /// Horizontal position in pixels from the left edge of the view.
    pub x: f64,
    /// The time to show, like "1:30" or "0:01.250".
    pub label: String,
}

/// The ticks for a view showing `seconds_per_pixel` seconds per pixel, starting at
/// `view_start` seconds and `width` pixels wide. Ticks are at least
/// `min_spacing` pixels apart.
#[must_use]
pub fn ticks(view_start: f64, seconds_per_pixel: f64, width: f64, min_spacing: f64) -> Vec<Tick> {
    if seconds_per_pixel <= 0.0 || width <= 0.0 {
        return Vec::new();
    }
    let min_interval = seconds_per_pixel * min_spacing;
    let interval = INTERVALS
        .iter()
        .copied()
        .find(|&interval| interval >= min_interval)
        .unwrap_or(3600.0);

    let view_end = view_start + width * seconds_per_pixel;
    let mut time = (view_start / interval).ceil() * interval;
    let mut ticks = Vec::new();
    while time <= view_end {
        ticks.push(Tick {
            x: (time - view_start) / seconds_per_pixel,
            label: label(time, interval),
        });
        time += interval;
    }
    ticks
}

/// "m:ss" for whole-second intervals; with as many decimals as the interval needs
/// otherwise. Hours appear from one hour on.
fn label(seconds: f64, interval: f64) -> String {
    let decimals: usize = if interval >= 1.0 {
        0
    } else if interval >= 0.1 {
        1
    } else if interval >= 0.01 {
        2
    } else {
        3
    };
    // Round to the shown precision first, so 59.9999 does not become "0:60".
    let scale = 10_f64.powi(i32::try_from(decimals).unwrap_or(0));
    let seconds = (seconds.max(0.0) * scale).round() / scale;
    let whole = seconds.trunc();
    let fraction = seconds - whole;
    let (hours, minutes, secs) = split_hms(whole);
    let fraction = if decimals == 0 {
        String::new()
    } else {
        // "0.250" → ".250"
        let digits = format!("{fraction:.decimals$}");
        digits.trim_start_matches('0').to_owned()
    };
    if hours > 0.0 {
        format!("{hours}:{minutes:02}:{secs:02}{fraction}")
    } else {
        format!("{minutes}:{secs:02}{fraction}")
    }
}

fn split_hms(whole_seconds: f64) -> (f64, f64, f64) {
    let hours = (whole_seconds / 3600.0).trunc();
    let minutes = ((whole_seconds - hours * 3600.0) / 60.0).trunc();
    let seconds = whole_seconds - hours * 3600.0 - minutes * 60.0;
    (hours, minutes, seconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(ticks: &[Tick]) -> Vec<&str> {
        ticks.iter().map(|tick| tick.label.as_str()).collect()
    }

    #[test]
    fn a_three_minute_song_in_1200_pixels() {
        // 0.15 s per pixel; ticks at least 80 px (12 s) apart → every 15 s.
        let ticks = ticks(0.0, 0.15, 1200.0, 80.0);
        assert_eq!(
            labels(&ticks),
            [
                "0:00", "0:15", "0:30", "0:45", "1:00", "1:15", "1:30", "1:45", "2:00", "2:15",
                "2:30", "2:45", "3:00"
            ]
        );
        assert!((ticks[1].x - 100.0).abs() < 1e-9);
    }

    #[test]
    fn zoomed_in_ticks_show_milliseconds() {
        // 0.5 ms per pixel → ticks every 50 ms, starting at the first tick after 1.23 s.
        let ticks = ticks(1.23, 0.0005, 400.0, 80.0);
        assert_eq!(labels(&ticks), ["0:01.25", "0:01.30", "0:01.35", "0:01.40"]);
    }

    #[test]
    fn hours_appear_after_an_hour() {
        assert_eq!(label(3725.0, 5.0), "1:02:05");
        assert_eq!(label(59.999_99, 1.0), "1:00");
    }

    #[test]
    fn an_empty_view_has_no_ticks() {
        assert!(ticks(0.0, 0.0, 100.0, 80.0).is_empty());
        assert!(ticks(0.0, 1.0, 0.0, 80.0).is_empty());
    }
}
