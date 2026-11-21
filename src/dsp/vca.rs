
use std::f32::consts::{FRAC_PI_4, SQRT_2};

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

