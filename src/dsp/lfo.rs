
use crate::params::LfoWaveform;
use std::f32::consts::TAU;

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

    #[inline]
    pub fn next(&mut self, frequency: f32, sample_rate: f32, waveform: LfoWaveform) -> f32 {
        self.next_shaped(frequency, sample_rate, waveform, 0.0, 0.0)
    }

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

    #[inline]
    pub fn retrigger(&mut self) {
        self.phase = 0.0;
    }

    
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

#[inline]
fn curve_exponent(curve: f32) -> f32 {
    if curve == 0.0 {
        1.0
    } else {
        2f32.powf(curve.clamp(-1.0, 1.0))
    }
}

#[inline]
fn shape_lfo(value: f32, curve: f32) -> f32 {
    if curve == 0.0 {
        value
    } else {
        value.signum() * value.abs().powf(curve_exponent(curve))
    }
}

