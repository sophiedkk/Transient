//! Conversions between number types.
//!
//! Rust's `as` turns any number into any other number type, silently rounding,
//! truncating or saturating, so the workspace forbids it (`clippy::as_conversions`).
//! These functions convert with `num_traits::ToPrimitive`, which reports a value
//! that does not fit as `None`, and each says in its documentation what happens
//! then. Frame counts have their own conversions in [`crate::time`].

use num_traits::ToPrimitive;

/// The nearest `f32` to `value` (GTK draws with `f32`; samples and gains are `f32`).
///
/// Values beyond `f32`'s range become infinity (rounded the way IEEE 754
/// floating point rounds); infinities and NaN stay what they are.
#[must_use]
pub fn f32_from_f64(value: f64) -> f32 {
    value
        .to_f32()
        // `to_f32` only fails for values it cannot represent at all; there are
        // none for `f64`, but the type still asks what to do then.
        .unwrap_or(f32::NAN)
}

/// `value` as `f64`, for maths with counts (samples, columns, blocks).
///
/// Exact below 2^53, which no count in Transient comes near; larger values are
/// rounded to the nearest `f64`.
#[must_use]
pub fn f64_from_usize(value: usize) -> f64 {
    value.to_f64().unwrap_or(f64::MAX)
}

/// `value` rounded to the nearest whole number, as `i32` (pixels, Pango units).
///
/// Values beyond `i32`'s range become `i32::MAX` or `i32::MIN`; NaN becomes 0.
#[must_use]
pub fn i32_from_f64(value: f64) -> i32 {
    value.round().to_i32().unwrap_or(if value > 0.0 {
        i32::MAX
    } else if value < 0.0 {
        i32::MIN
    } else {
        0
    })
}

/// `value` rounded to the nearest whole number, as `u32` (column counts).
///
/// Negative values and NaN become 0, values beyond `u32`'s range `u32::MAX`.
#[must_use]
pub fn u32_from_f64(value: f64) -> u32 {
    value
        .round()
        .to_u32()
        .unwrap_or(if value > 0.0 { u32::MAX } else { 0 })
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn values_that_do_not_fit_saturate_or_become_infinite() {
        assert_eq!(f32_from_f64(1e300).to_bits(), f32::INFINITY.to_bits());
        assert_eq!(f32_from_f64(-1e300).to_bits(), f32::NEG_INFINITY.to_bits());
        assert_eq!(i32_from_f64(1e12), i32::MAX);
        assert_eq!(i32_from_f64(-1e12), i32::MIN);
        assert_eq!(u32_from_f64(1e12), u32::MAX);
        assert_eq!(u32_from_f64(-5.0), 0);
    }

    #[test]
    fn nan_and_infinity_are_handled() {
        assert!(f32_from_f64(f64::NAN).is_nan());
        assert_eq!(
            f32_from_f64(f64::INFINITY).to_bits(),
            f32::INFINITY.to_bits()
        );
        assert_eq!(i32_from_f64(f64::NAN), 0);
        assert_eq!(i32_from_f64(f64::INFINITY), i32::MAX);
        assert_eq!(u32_from_f64(f64::NAN), 0);
    }

    #[test]
    fn whole_numbers_round_to_the_nearest() {
        assert_eq!(i32_from_f64(2.5), 3);
        assert_eq!(i32_from_f64(-2.5), -3);
        assert_eq!(i32_from_f64(2.4), 2);
        assert_eq!(u32_from_f64(7.6), 8);
    }

    proptest! {
        /// Every `f32` survives a round trip through `f64`.
        #[test]
        fn f32_values_come_back_unchanged(value in proptest::num::f32::NORMAL) {
            prop_assert_eq!(f32_from_f64(f64::from(value)).to_bits(), value.to_bits());
        }

        /// Within `f32`'s range the result is as close as `f32` gets.
        #[test]
        fn f64_values_become_the_nearest_f32(value in -1e30_f64..1e30) {
            let converted = f64::from(f32_from_f64(value));
            prop_assert!((converted - value).abs() <= value.abs() * f64::from(f32::EPSILON));
        }

        /// Counts below 2^53 convert exactly (checked by converting back).
        #[test]
        fn counts_convert_exactly(value in 0_usize..(1 << 53)) {
            prop_assert_eq!(f64_from_usize(value).to_usize(), Some(value));
        }

        /// Every `i32` comes back unchanged, and a fraction moves it by at most ½.
        #[test]
        fn whole_pixels_round_trip(value in any::<i32>(), fraction in -0.49_f64..0.49) {
            prop_assert_eq!(i32_from_f64(f64::from(value)), value);
            if value > i32::MIN && value < i32::MAX {
                prop_assert_eq!(i32_from_f64(f64::from(value) + fraction), value);
            }
        }

        /// Every `u32` comes back unchanged.
        #[test]
        fn column_counts_round_trip(value in any::<u32>()) {
            prop_assert_eq!(u32_from_f64(f64::from(value)), value);
        }
    }
}
