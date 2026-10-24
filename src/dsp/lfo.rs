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

        let value = match waveform {
            LfoWaveform::Sine => (phase * TAU).sin(),
            LfoWaveform::Triangle => {
                // The two halves meet at `+1`, so the shape is continuous.
                if phase < 0.5 {
                    4.0 * phase - 1.0
                } else {
                    3.0 - 4.0 * phase
                }
            }
            LfoWaveform::Saw => 2.0 * phase - 1.0,
            LfoWaveform::Square => {
                if phase < 0.5 {
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

        value.clamp(-1.0, 1.0)
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
}
