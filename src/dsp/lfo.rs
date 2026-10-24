//! A global low frequency oscillator with a sample & hold waveform.

use crate::params::LfoWaveform;
use std::f32::consts::TAU;

/// A low frequency oscillator. The output is in the `-1..1` range.
#[derive(Debug, Clone, Copy)]
pub struct Lfo {
    phase: f32,
    sample_hold: f32,
    rng: u32,
}

impl Lfo {
    pub fn new(seed: u32) -> Self {
        Self {
            phase: 0.0,
            sample_hold: 0.0,
            rng: if seed == 0 { 0x2545_F491 } else { seed },
        }
    }

    /// Renders the next sample. `frequency` is in Hz and `sample_rate` in Hz as
    /// well.
    #[inline]
    pub fn next(&mut self, frequency: f32, sample_rate: f32, waveform: LfoWaveform) -> f32 {
        self.next_shaped(frequency, sample_rate, waveform, 0.0, 0.0)
    }

    /// Renders the next sample with an output phase offset and a curve.
    ///
    /// The phase offset only changes the phase at which the waveform is read;
    /// the internal phase still advances by the same amount. This keeps every
    /// voice and every block on the same clock. The curve is a safe signed
    /// power shaper: zero is exactly the old waveform, so old patches keep
    /// their sound bit for bit.
    #[inline]
    pub fn next_shaped(
        &mut self,
        frequency: f32,
        sample_rate: f32,
        waveform: LfoWaveform,
        phase_offset: f32,
        curve: f32,
    ) -> f32 {
        let sample_rate = sample_rate.max(1.0);
        let phase = self.phase;

        // Advance the phase and remember whether we wrapped around.
        self.phase += (frequency / sample_rate).max(0.0);
        let mut wrapped = false;
        if self.phase >= 1.0 {
            let wraps = self.phase.floor();
            self.phase -= wraps;
            wrapped = true;
        }

        // The offset is applied to the read position only. A sign of the
        // sample & hold cannot be shifted, because its value is tied to the
        // random update at each wrap, so it deliberately ignores the offset.
        let output_phase = if phase_offset == 0.0 {
            phase
        } else {
            (phase + phase_offset).rem_euclid(1.0)
        };

        let value = match waveform {
            LfoWaveform::Sine => (output_phase * TAU).sin(),
            LfoWaveform::Triangle => {
                // The two halves meet at `+1`, so the shape is continuous.
                if output_phase < 0.5 {
                    4.0 * output_phase - 1.0
                } else {
                    3.0 - 4.0 * output_phase
                }
            }
            LfoWaveform::Saw => 2.0 * output_phase - 1.0,
            LfoWaveform::Square => {
                if output_phase < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            LfoWaveform::SampleHold => {
                // A new random value is picked every time the phase wraps.
                if wrapped {
                    self.sample_hold = self.next_random();
                }
                self.sample_hold
            }
        };

        shape_lfo(value, curve).clamp(-1.0, 1.0)
    }

    /// Restarts the waveform at the beginning of its cycle.
    ///
    /// The random state of the sample & hold is deliberately kept, so a
    /// retriggered note does not repeat the same random sequence.
    #[inline]
    pub fn retrigger(&mut self) {
        self.phase = 0.0;
    }

    /// The current phase of the oscillator, `0..1`. Only the tests need it.
    #[cfg(test)]
    pub(crate) fn phase(&self) -> f32 {
        self.phase
    }

    /// A random number in the `-1..1` range.
    #[inline]
    fn next_random(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;

        ((x >> 8) as f32 / 16_777_215.0) * 2.0 - 1.0
    }
}

impl Default for Lfo {
    fn default() -> Self {
        Self::new(0x2545_F491)
    }
}

/// The exponent of the LFO curve for a bipolar `-1..1` parameter. Zero is
/// exactly one, so the default curve is a pass-through.
#[inline]
fn curve_exponent(curve: f32) -> f32 {
    if curve == 0.0 {
        1.0
    } else {
        2f32.powf(curve.clamp(-1.0, 1.0))
    }
}

/// Shapes an LFO value with a signed power. The sign is kept, the endpoints
/// `-1`, `0` and `1` stay fixed, and a curve of exactly zero returns the
/// input untouched.
#[inline]
fn shape_lfo(value: f32, curve: f32) -> f32 {
    if curve == 0.0 {
        value
    } else {
        value.signum() * value.abs().powf(curve_exponent(curve))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_RATE: f32 = 48_000.0;

    #[test]
    fn all_waveforms_stay_in_range() {
        for waveform in [
            LfoWaveform::Sine,
            LfoWaveform::Triangle,
            LfoWaveform::Saw,
            LfoWaveform::Square,
            LfoWaveform::SampleHold,
        ] {
            let mut lfo = Lfo::new(42);
            for _ in 0..10_000 {
                let value = lfo.next(5.0, SAMPLE_RATE, waveform);
                assert!(value.is_finite());
                assert!(
                    (-1.0..=1.0).contains(&value),
                    "{waveform:?} produced {value}"
                );
            }
        }
    }

    #[test]
    fn sample_and_hold_holds_its_value() {
        let mut lfo = Lfo::new(7);
        let frequency = 10.0;

        let first = lfo.next(frequency, SAMPLE_RATE, LfoWaveform::SampleHold);
        // The value only changes once the phase wraps, so it stays the same for
        // most of a cycle.
        for _ in 0..100 {
            let value = lfo.next(frequency, SAMPLE_RATE, LfoWaveform::SampleHold);
            assert_eq!(value, first);
        }

        // After a full period the value must have changed.
        let mut changed = false;
        for _ in 0..(SAMPLE_RATE / frequency) as usize {
            let value = lfo.next(frequency, SAMPLE_RATE, LfoWaveform::SampleHold);
            changed |= value != first;
        }
        assert!(changed, "the sample & hold value never changed");
    }

    #[test]
    fn retrigger_restarts_the_phase() {
        let mut lfo = Lfo::new(7);
        for _ in 0..1_000 {
            lfo.next(5.0, SAMPLE_RATE, LfoWaveform::Sine);
        }
        assert!(lfo.phase() > 0.0);

        lfo.retrigger();
        assert_eq!(lfo.phase(), 0.0);
        assert_eq!(lfo.next(5.0, SAMPLE_RATE, LfoWaveform::Sine), 0.0);
    }

    #[test]
    fn a_zero_curve_keeps_the_old_output_exactly() {
        for waveform in [
            LfoWaveform::Sine,
            LfoWaveform::Triangle,
            LfoWaveform::Saw,
            LfoWaveform::Square,
            LfoWaveform::SampleHold,
        ] {
            let mut plain = Lfo::new(123);
            let mut shaped = Lfo::new(123);
            for _ in 0..2_000 {
                let a = plain.next(3.7, SAMPLE_RATE, waveform);
                let b = shaped.next_shaped(3.7, SAMPLE_RATE, waveform, 0.0, 0.0);
                assert_eq!(a.to_bits(), b.to_bits(), "{waveform:?} changed");
            }
        }
    }

    #[test]
    fn the_phase_offset_shifts_the_output_but_not_the_advance() {
        let mut plain = Lfo::new(99);
        let mut shifted = Lfo::new(99);

        // A quarter-cycle offset turns the first sine sample into the top of
        // the cycle, while both phases advance by exactly the same amount.
        let plain_first = plain.next(5.0, SAMPLE_RATE, LfoWaveform::Sine);
        let shifted_first = shifted.next_shaped(5.0, SAMPLE_RATE, LfoWaveform::Sine, 0.25, 0.0);
        assert!(plain_first.abs() < 1e-6);
        assert!((shifted_first - 1.0).abs() < 1e-6);
        assert_eq!(plain.phase(), shifted.phase());

        for _ in 0..2_000 {
            plain.next(5.0, SAMPLE_RATE, LfoWaveform::Sine);
            shifted.next_shaped(5.0, SAMPLE_RATE, LfoWaveform::Sine, 0.25, 0.0);
            assert_eq!(plain.phase(), shifted.phase());
        }
    }

    #[test]
    fn the_curve_is_a_safe_signed_power() {
        assert_eq!(shape_lfo(0.25, 0.0), 0.25);
        assert!((shape_lfo(0.25, 1.0) - 0.0625).abs() < 1e-6);
        assert!((shape_lfo(-0.25, 1.0) + 0.0625).abs() < 1e-6);
        assert_eq!(shape_lfo(1.0, 1.0), 1.0);
        assert_eq!(shape_lfo(-1.0, 1.0), -1.0);
        assert_eq!(shape_lfo(0.0, -1.0), 0.0);

        let mut lfo = Lfo::new(5);
        for curve in [-1.0, -0.5, 0.5, 1.0] {
            for _ in 0..2_000 {
                let value = lfo.next_shaped(7.0, SAMPLE_RATE, LfoWaveform::Sine, 0.0, curve);
                assert!(value.is_finite());
                assert!((-1.0..=1.0).contains(&value), "curve {curve}: {value}");
            }
        }
    }
}
