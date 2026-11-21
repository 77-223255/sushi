
use std::f32::consts::{PI, SQRT_2};

use super::filter::Svf;
use crate::params::FilterMode;

/// The lowest cutoff of the highpass, in Hz.
pub const HPF_MIN_CUTOFF: f32 = 20.0;

/// The highest cutoff of the highpass, in Hz.
pub const HPF_MAX_CUTOFF: f32 = 2_000.0;

/// At or below this cutoff the highpass is bypassed exactly. The parameter
/// minimum is 20 Hz, so a default patch never enters the filter path.
pub const HPF_BYPASS_CUTOFF: f32 = 20.1;

/// The damping of a second-order Butterworth highpass.
const BUTTERWORTH_DAMPING: f32 = SQRT_2;

#[derive(Debug, Clone, Copy)]
pub struct GlobalHighpass {
    left: Svf,
    right: Svf,
    /// The cutoff that the current coefficients were derived from, in Hz.
    cutoff: f32,
    /// The sample rate of the cached coefficients.
    sample_rate: f32,
    /// Whether the filter is currently doing any work.
    active: bool,
}

impl Default for GlobalHighpass {
    fn default() -> Self {
        Self {
            left: Svf::new(),
            right: Svf::new(),
            cutoff: HPF_MIN_CUTOFF,
            sample_rate: 0.0,
            active: false,
        }
    }
}

impl GlobalHighpass {
    pub fn new() -> Self {
        Self::default()
    }

    /// Clears the state of both channels. Called on reset and whenever the
    /// filter is switched in or out, so it can never replay stale audio.
    pub fn reset(&mut self) {
        self.left = Svf::new();
        self.right = Svf::new();
        // Keep the coefficients of an active filter so the first sample after
        // a reset already runs the correct cutoff.
        if self.active && self.sample_rate > 0.0 {
            let g = (PI * self.cutoff / self.sample_rate).tan();
            self.left.set(g, BUTTERWORTH_DAMPING);
            self.right.set(g, BUTTERWORTH_DAMPING);
        }
    }

    /// Updates the cutoff. This is meant to be called once per block (or at
    /// another control rate), not once per sample. A cutoff at or below
    /// [`HPF_BYPASS_CUTOFF`] switches the filter off completely.
    pub fn set_cutoff(&mut self, cutoff_hz: f32, sample_rate: f32) {
        let sample_rate = sample_rate.max(1.0);
        let active = cutoff_hz.is_finite() && cutoff_hz > HPF_BYPASS_CUTOFF;

        if !active {
            if self.active {
                self.reset();
            }
            self.active = false;
            return;
        }

        let cutoff = cutoff_hz.clamp(HPF_MIN_CUTOFF, HPF_MAX_CUTOFF);
        if !self.active || self.sample_rate != sample_rate || self.cutoff != cutoff {
            if !self.active {
                self.reset();
            }
            let g = (PI * cutoff / sample_rate).tan();
            self.left.set(g, BUTTERWORTH_DAMPING);
            self.right.set(g, BUTTERWORTH_DAMPING);
            self.cutoff = cutoff;
            self.sample_rate = sample_rate;
        }
        self.active = true;
    }

    #[inline]
    pub fn process(&mut self, left: f32, right: f32) -> (f32, f32) {
        if !self.active {
            return (left, right);
        }

        (
            self.left
                .process_prepared(left, FilterMode::Highpass, 0.0, false),
            self.right
                .process_prepared(right, FilterMode::Highpass, 0.0, false),
        )
    }

    }

