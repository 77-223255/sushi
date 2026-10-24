//! The global stereo width and pan stage of the voice sum.
//!
//! The stage runs after the tone tilt and before the master volume, so it
//! operates on the finished stereo mix. Width is a mid/side control: zero
//! collapses the mix to mono, one is the original stereo image and two doubles
//! the side signal. Pan uses an equal-power law with a unity gain in the
//! centre. The neutral setting (width one, pan zero) returns the input
//! unchanged, bit for bit, which keeps the old patches exactly as they were.

use std::f32::consts::{FRAC_PI_4, SQRT_2};

/// Applies the stereo width and the equal-power pan to one stereo sample.
///
/// `width` is clamped to `0..2` and `pan` to `-1..1`. At `width == 1.0` and
/// `pan == 0.0` the input is returned unchanged without any arithmetic.
#[inline]
pub fn apply_width_pan(left: f32, right: f32, width: f32, pan: f32) -> (f32, f32) {
    if width == 1.0 && pan == 0.0 {
        return (left, right);
    }

    let width = width.clamp(0.0, 2.0);
    let pan = pan.clamp(-1.0, 1.0);

    // Mid/side width. The factor of one half keeps the mono sum at the same
    // level as the input for every width.
    let mid = (left + right) * 0.5;
    let side = (left - right) * 0.5 * width;
    let mut left = mid + side;
    let mut right = mid - side;

    if pan != 0.0 {
        // Equal power with the centre at unity gain. The angle moves from
        // 0 (hard left) over pi/4 (centre) to pi/2 (hard right).
        let angle = (pan + 1.0) * FRAC_PI_4;
        left *= angle.cos() * SQRT_2;
        right *= angle.sin() * SQRT_2;
    }

    (left, right)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_neutral_setting_is_bit_exact() {
        for (left, right) in [
            (0.0, 0.0),
            (0.25, -0.5),
            (1.0, 1.0),
            (f32::MIN_POSITIVE, -f32::MIN_POSITIVE),
            (0.123_456_79, 0.987_654_3),
        ] {
            let (out_left, out_right) = apply_width_pan(left, right, 1.0, 0.0);
            assert_eq!(out_left.to_bits(), left.to_bits());
            assert_eq!(out_right.to_bits(), right.to_bits());
        }
    }

    #[test]
    fn a_width_of_zero_collapses_the_mix_to_mono() {
        let (left, right) = apply_width_pan(0.25, -0.75, 0.0, 0.0);
        assert_eq!(left.to_bits(), right.to_bits());
        assert!((left - -0.25).abs() < 1e-7);
    }

    #[test]
    fn the_width_scales_the_side_signal() {
        let (left, right) = apply_width_pan(0.5, -0.5, 2.0, 0.0);
        assert!((left - 1.0).abs() < 1e-7);
        assert!((right + 1.0).abs() < 1e-7);

        let (left, right) = apply_width_pan(0.5, -0.5, 1.0, 0.0);
        assert!((left - 0.5).abs() < 1e-7);
        assert!((right + 0.5).abs() < 1e-7);
    }

    #[test]
    fn the_pan_uses_equal_power_and_moves_the_image() {
        let (left, right) = apply_width_pan(1.0, 1.0, 1.0, 1.0);
        assert!(right > left * 10.0, "the pan did not move to the right");
        assert!(left.abs() < 1e-6);
        assert!((right - SQRT_2).abs() < 1e-6);

        let (left, right) = apply_width_pan(1.0, 1.0, 1.0, -1.0);
        assert!(left > right * 10.0, "the pan did not move to the left");
        assert!(right.abs() < 1e-6);
        assert!((left - SQRT_2).abs() < 1e-6);

        // The power of a centred mono input stays constant while it moves.
        for pan in [-1.0, -0.5, 0.25, 0.5, 1.0] {
            let (left, right) = apply_width_pan(1.0, 1.0, 1.0, pan);
            let power = left * left + right * right;
            assert!(
                (power - 2.0).abs() < 1e-5,
                "the power at pan {pan} was {power}"
            );
        }
    }

    #[test]
    fn the_controls_are_clamped_to_their_ranges() {
        let (left, right) = apply_width_pan(0.5, -0.5, 10.0, 4.0);
        let (clamped_left, clamped_right) = apply_width_pan(0.5, -0.5, 2.0, 1.0);
        assert_eq!(left.to_bits(), clamped_left.to_bits());
        assert_eq!(right.to_bits(), clamped_right.to_bits());
    }
}
