//! Where a recorded take goes on the timeline, so it lines up with what was
//! playing while it was recorded.
//!
//! The playhead shows what you hear. When recording starts, the input needs a
//! moment to start, and its audio arrives late by the input's latency, so the
//! take's first sample was captured a little after the playhead was read. The
//! app measures that delay; the user can add an offset of their own for what
//! the system does not report (Preferences, "Recording Offset").

use crate::time::{Frames, SampleRate};

/// Nanoseconds in a second.
const NANOS_PER_SECOND: i128 = 1_000_000_000;
/// Nanoseconds in a millisecond.
const NANOS_PER_MILLISECOND: i128 = 1_000_000;

/// Where a take starts on the timeline, and how many of its first frames are
/// left out (when it would otherwise start before 0:00).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TakePlacement {
    /// The timeline frame the take's clip starts at.
    pub start: Frames,
    /// How many frames at the take's beginning the clip skips.
    pub skip: Frames,
}

/// Places a take recorded when the playhead was at `playhead`.
///
/// `capture_delay_nanos` is how long after reading the playhead the take's
/// first sample was captured (negative if before), and `offset_ms` the user's
/// fine-tuning: negative moves takes earlier.
#[must_use]
pub fn place_take(
    playhead: Frames,
    capture_delay_nanos: i64,
    offset_ms: i32,
    rate: SampleRate,
) -> TakePlacement {
    let nanos = i128::from(capture_delay_nanos) + i128::from(offset_ms) * NANOS_PER_MILLISECOND;
    // Rounded to the nearest frame (`div_euclid` rounds down, also below zero).
    let shift = (nanos * i128::from(rate.hz()) + NANOS_PER_SECOND / 2).div_euclid(NANOS_PER_SECOND);
    let position = i128::from(playhead) + shift;
    if position >= 0 {
        TakePlacement {
            start: Frames::try_from(position).unwrap_or(Frames::MAX),
            skip: 0,
        }
    } else {
        TakePlacement {
            start: 0,
            skip: Frames::try_from(-position).unwrap_or(Frames::MAX),
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    const RATE: SampleRate = SampleRate::HZ_48000;

    #[test]
    fn without_a_delay_the_take_starts_at_the_playhead() {
        let placed = place_take(48_000, 0, 0, RATE);
        assert_eq!(
            placed,
            TakePlacement {
                start: 48_000,
                skip: 0
            }
        );
    }

    #[test]
    fn a_take_captured_later_starts_later() {
        // 10 ms at 48 kHz is 480 frames.
        let placed = place_take(48_000, 10_000_000, 0, RATE);
        assert_eq!(placed.start, 48_480);
    }

    #[test]
    fn the_offset_moves_takes_earlier_or_later() {
        assert_eq!(place_take(48_000, 0, -20, RATE).start, 47_040);
        assert_eq!(place_take(48_000, 10_000_000, -10, RATE).start, 48_000);
        assert_eq!(place_take(48_000, 0, 5, RATE).start, 48_240);
    }

    #[test]
    fn a_take_that_would_start_before_zero_loses_its_beginning() {
        let placed = place_take(240, 0, -10, RATE);
        assert_eq!(
            placed,
            TakePlacement {
                start: 0,
                skip: 240
            }
        );
    }

    proptest! {
        /// The take's first kept frame is always where the shift puts it, and
        /// nothing is skipped unless the take would start before 0:00.
        #[test]
        fn placement_keeps_the_audio_where_it_belongs(
            playhead in 0_u64..10_000_000,
            delay in -1_000_000_000_i64..1_000_000_000,
            offset in -1000_i32..1000,
        ) {
            let placed = place_take(playhead, delay, offset, RATE);
            let shift = place_take(1_000_000_000, delay, offset, RATE).start;
            let shift = i128::from(shift) - 1_000_000_000;
            prop_assert_eq!(
                i128::from(placed.start) - i128::from(placed.skip),
                i128::from(playhead) + shift
            );
            prop_assert!(placed.skip == 0 || placed.start == 0);
        }
    }
}
