//! A global, non-resonant stereo highpass for the voice sum.
//!
//! The filter runs on the summed voice signal, before the character
//! saturator. It is a single stereo object: both channels share the
//! coefficients and each channel keeps its own state. The topology is the
//! cascaded TPT state variable filter with a Butterworth damping of
//! `sqrt(2)`, so the response is 12 dB/octave and has no resonant peak.
//!
//! The cutoff at the bottom of the parameter range is an exact bypass. This is
//! deliberate: old patches have no `hpf_cutoff` value and therefore load the
//! 20 Hz default, and they must keep their original sound bit for bit. The
//! coefficients are only recomputed when the cutoff or the sample rate
//! changes, never once per sample.

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

/// A global stereo highpass with a shared cutoff and per-channel state.
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

    /// Processes one stereo sample. When the filter is bypassed, the input is
    /// returned bit for bit unchanged.
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

    /// Whether the filter is currently active. Only the tests need this.
    #[cfg(test)]
    pub(crate) fn is_active(&self) -> bool {
        self.active
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_RATE: f32 = 48_000.0;

    fn fingerprint(hash: &mut u64, sample: f32) {
        *hash ^= sample.to_bits() as u64;
        *hash = hash.wrapping_mul(0x100_0000_01b3);
    }

    #[test]
    fn the_bottom_of_the_range_is_an_exact_bypass() {
        for cutoff in [HPF_MIN_CUTOFF, 20.05, HPF_BYPASS_CUTOFF] {
            let mut hpf = GlobalHighpass::new();
            hpf.set_cutoff(cutoff, SAMPLE_RATE);
            assert!(!hpf.is_active(), "{cutoff} Hz was not bypassed");

            let mut hash = 0xcbf2_9ce4_8422_2325u64;
            for index in 0..4_096 {
                let left = (index as f32 * 0.013).sin() * 0.7;
                let right = (index as f32 * 0.017).cos() * 0.5;
                let (out_left, out_right) = hpf.process(left, right);
                assert_eq!(out_left.to_bits(), left.to_bits());
                assert_eq!(out_right.to_bits(), right.to_bits());
                fingerprint(&mut hash, out_left);
                fingerprint(&mut hash, out_right);
            }
        }
    }

    #[test]
    fn a_cutoff_above_the_bypass_removes_the_low_end() {
        fn level(frequency: f32) -> f32 {
            let mut hpf = GlobalHighpass::new();
            hpf.set_cutoff(500.0, SAMPLE_RATE);
            assert!(hpf.is_active());

            let mut sum = 0.0;
            let samples = SAMPLE_RATE as usize;
            for index in 0..samples {
                let input = (std::f32::consts::TAU * frequency * index as f32 / SAMPLE_RATE).sin();
                let (output, _) = hpf.process(input, input);
                // Skip the settling time of the filter.
                if index >= samples / 2 {
                    sum += output * output;
                }
            }
            (sum / (samples / 2) as f32).sqrt()
        }

        let low = level(100.0);
        let high = level(5_000.0);
        assert!(
            low < high * 0.1,
            "the highpass did not remove the low end: {low} vs {high}"
        );
        assert!(
            (high - 0.707).abs() < 0.05,
            "the highpass changed the passband: {high}"
        );
    }

    #[test]
    fn resetting_the_filter_clears_its_state() {
        let mut used = GlobalHighpass::new();
        used.set_cutoff(800.0, SAMPLE_RATE);
        for index in 0..512 {
            used.process((index as f32 * 0.1).sin(), 0.25);
        }

        let mut fresh = GlobalHighpass::new();
        fresh.set_cutoff(800.0, SAMPLE_RATE);
        used.reset();
        for index in 0..256 {
            let input = (index as f32 * 0.03).cos();
            let a = used.process(input, input * 0.5);
            let b = fresh.process(input, input * 0.5);
            assert_eq!(a.0.to_bits(), b.0.to_bits());
            assert_eq!(a.1.to_bits(), b.1.to_bits());
        }
    }

    #[test]
    fn switching_the_filter_off_clears_the_state() {
        let mut hpf = GlobalHighpass::new();
        hpf.set_cutoff(1_000.0, SAMPLE_RATE);
        for index in 0..512 {
            hpf.process((index as f32 * 0.2).sin(), 0.0);
        }

        hpf.set_cutoff(HPF_MIN_CUTOFF, SAMPLE_RATE);
        assert!(!hpf.is_active());
        assert_eq!(hpf.process(0.5, -0.25), (0.5, -0.25));

        // Switching the filter back on starts from a clean state.
        let mut fresh = GlobalHighpass::new();
        fresh.set_cutoff(1_000.0, SAMPLE_RATE);
        hpf.set_cutoff(1_000.0, SAMPLE_RATE);
        for index in 0..128 {
            let input = (index as f32 * 0.05).sin();
            let a = hpf.process(input, input);
            let b = fresh.process(input, input);
            assert_eq!(a.0.to_bits(), b.0.to_bits());
            assert_eq!(a.1.to_bits(), b.1.to_bits());
        }
    }
}
