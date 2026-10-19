use super::super::params::Waveform;
use std::f32::consts::TAU;

/// PolyBLEP residual for a discontinuity at phase 0.
#[inline]
fn poly_blep(phase: f32, dt: f32) -> f32 {
    if dt <= 0.0 {
        return 0.0;
    }

    if phase < dt {
        let x = phase / dt;
        x + x - x * x - 1.0
    } else if phase > 1.0 - dt {
        let x = (phase - 1.0) / dt;
        x * x + x + x + 1.0
    } else {
        0.0
    }
}

/// A single free-running oscillator with PolyBLEP band limiting for the
/// discontinuous waveforms.
#[derive(Debug, Clone, Default)]
pub struct Oscillator {
    phase: f32,
}

impl Oscillator {
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn next(&mut self, phase_delta: f32, waveform: Waveform) -> f32 {
        let phase = self.phase;

        let sample = match waveform {
            Waveform::Sine => (phase * TAU).sin(),
            Waveform::Saw => 2.0 * phase - 1.0 - poly_blep(phase, phase_delta),
            Waveform::Square => {
                let mut square = if phase < 0.5 { 1.0 } else { -1.0 };
                // Rising edge at phase 0, falling edge at phase 0.5.
                square += poly_blep(phase, phase_delta);
                let phase_half = if phase < 0.5 {
                    phase + 0.5
                } else {
                    phase - 0.5
                };
                square - poly_blep(phase_half, phase_delta)
            }
        };

        self.phase += phase_delta;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }

        sample
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_has_expected_frequency() {
        let sample_rate = 48_000.0;
        let frequency = 440.0;
        let dt = frequency / sample_rate;

        let mut osc = Oscillator::new();
        let mut previous = osc.next(dt, Waveform::Sine);
        let mut rising_crossings = 0;
        for _ in 0..sample_rate as usize {
            let sample = osc.next(dt, Waveform::Sine);
            if previous < 0.0 && sample >= 0.0 {
                rising_crossings += 1;
            }
            previous = sample;
        }

        // One rising zero crossing per cycle, ±1 for the partial first/last cycle.
        assert!((rising_crossings - frequency as i32).abs() <= 1);
    }

    #[test]
    fn polyblep_waveforms_stay_bounded() {
        for waveform in [Waveform::Saw, Waveform::Square] {
            let mut osc = Oscillator::new();
            for _ in 0..48_000 {
                let sample = osc.next(440.0 / 48_000.0, waveform);
                assert!(sample.is_finite());
                assert!(sample.abs() < 1.5, "{waveform:?} produced {sample}");
            }
        }
    }
}
