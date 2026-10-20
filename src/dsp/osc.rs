//! Band limited oscillators and a noise generator.

use super::super::params::Waveform;
use std::f32::consts::TAU;

/// The maximum amplitude an oscillator can produce. The PolyBLEP residuals can
/// briefly overshoot the nominal ±1 range, so the output is clipped here.
const MAX_AMPLITUDE: f32 = 1.2;

/// The leak of the integrator of the triangle wave. It is very slow, so that
/// the shape of the wave is not affected by it, but it keeps the integrator
/// from drifting away over time.
const TRIANGLE_LEAK: f32 = 0.999_99;

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
#[derive(Debug, Clone, Copy)]
pub struct Oscillator {
    phase: f32,
    /// The state of the integrator of the triangle wave.
    triangle: f32,
}

impl Default for Oscillator {
    fn default() -> Self {
        Self {
            phase: 0.0,
            // The triangle starts at the bottom of its range.
            triangle: -1.0,
        }
    }
}

impl Oscillator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the phase of the oscillator, `0..1`.
    #[inline]
    pub fn set_phase(&mut self, phase: f32) {
        let phase = phase.rem_euclid(1.0);
        self.phase = phase;
        // The integrator of the triangle starts at the value of the wave, so
        // that the triangle does not have to settle first.
        self.triangle = naive_triangle(phase);
    }

    /// Renders the next sample. `phase_delta` is the phase increment per sample
    /// (i.e. the frequency divided by the sample rate) and `pulse_width` is the
    /// duty cycle of the pulse waveform.
    #[inline]
    pub fn next(&mut self, phase_delta: f32, waveform: Waveform, pulse_width: f32) -> f32 {
        let phase = self.phase;
        let dt = phase_delta.clamp(0.0, 0.5);

        let sample = match waveform {
            Waveform::Sine => (phase * TAU).sin(),
            Waveform::Triangle => {
                // The triangle is the integral of a square wave. The square
                // wave is band limited with a PolyBLEP residual, so that the
                // corners of the triangle are smooth as well.
                let square = if phase < 0.5 { 1.0 } else { -1.0 };
                let rising = poly_blep(phase, dt);
                let falling = poly_blep((phase - 0.5).rem_euclid(1.0), dt);

                self.triangle += (square + rising - falling) * 4.0 * dt;
                // The leak keeps the integrator from drifting away.
                self.triangle *= TRIANGLE_LEAK;
                self.triangle.clamp(-MAX_AMPLITUDE, MAX_AMPLITUDE)
            }
            Waveform::Saw => 2.0 * phase - 1.0 - poly_blep(phase, dt),
            Waveform::Square => pulse(phase, dt, 0.5),
            Waveform::Pulse => pulse(phase, dt, pulse_width.clamp(0.05, 0.95)),
        };

        self.phase += dt;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }

        sample.clamp(-MAX_AMPLITUDE, MAX_AMPLITUDE)
    }
}

/// The value of a naive triangle wave at the given phase.
#[inline]
fn naive_triangle(phase: f32) -> f32 {
    if phase < 0.5 {
        4.0 * phase - 1.0
    } else {
        3.0 - 4.0 * phase
    }
}

/// A pulse wave with the given duty cycle. Both edges are band limited with a
/// PolyBLEP residual.
#[inline]
fn pulse(phase: f32, dt: f32, width: f32) -> f32 {
    let mut sample = if phase < width { 1.0 } else { -1.0 };

    // Rising edge at phase 0.
    sample += poly_blep(phase, dt);

    // Falling edge at phase `width`.
    let falling_phase = if phase < width {
        phase + (1.0 - width)
    } else {
        phase - width
    };
    sample -= poly_blep(falling_phase, dt);

    sample
}

/// A simple xorshift32 based noise generator.
#[derive(Debug, Clone, Copy)]
pub struct Noise {
    state: u32,
}

impl Noise {
    pub fn new(seed: u32) -> Self {
        Self {
            state: if seed == 0 { 0x9E37_79B9 } else { seed },
        }
    }

    /// Returns the next noise sample in the `-1..1` range.
    // The name is mandated by the plugin's internal API, so it stays even
    // though it looks like an iterator method.
    #[allow(clippy::should_implement_trait)]
    #[inline]
    pub fn next(&mut self) -> f32 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.state = x;

        // Use the upper 24 bits so that the full float precision is used.
        ((x >> 8) as f32 / 16_777_215.0) * 2.0 - 1.0
    }
}

impl Default for Noise {
    fn default() -> Self {
        Self::new(0x9E37_79B9)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_RATE: f32 = 48_000.0;

    #[test]
    fn sine_has_expected_frequency() {
        let frequency = 440.0;
        let dt = frequency / SAMPLE_RATE;

        let mut osc = Oscillator::new();
        let mut previous = osc.next(dt, Waveform::Sine, 0.5);
        let mut rising_crossings = 0;
        for _ in 0..SAMPLE_RATE as usize {
            let sample = osc.next(dt, Waveform::Sine, 0.5);
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
        for waveform in [
            Waveform::Sine,
            Waveform::Triangle,
            Waveform::Saw,
            Waveform::Square,
            Waveform::Pulse,
        ] {
            let mut osc = Oscillator::new();
            for _ in 0..SAMPLE_RATE as usize {
                let sample = osc.next(440.0 / SAMPLE_RATE, waveform, 0.5);
                assert!(sample.is_finite());
                assert!(
                    sample.abs() <= MAX_AMPLITUDE,
                    "{waveform:?} produced {sample}"
                );
            }
        }
    }

    #[test]
    fn triangle_is_continuous_and_bounded() {
        let dt = 440.0 / SAMPLE_RATE;

        let mut osc = Oscillator::new();
        let mut previous: Option<f32> = None;
        for _ in 0..SAMPLE_RATE as usize {
            let sample = osc.next(dt, Waveform::Triangle, 0.5);
            assert!(
                (-1.05..=1.05).contains(&sample),
                "the triangle left the expected range: {sample}"
            );
            if let Some(previous) = previous {
                assert!(
                    (sample - previous).abs() < 0.12,
                    "the triangle jumped from {previous} to {sample}"
                );
            }
            previous = Some(sample);
        }
    }

    #[test]
    fn the_triangle_keeps_its_shape_over_time() {
        let dt = 220.0 / SAMPLE_RATE;

        // The integrator of the triangle must not drift away.
        let mut osc = Oscillator::new();
        let mut minimum: f32 = 1.0;
        let mut maximum: f32 = -1.0;
        for _ in 0..(SAMPLE_RATE as usize * 4) {
            let sample = osc.next(dt, Waveform::Triangle, 0.5);
            minimum = minimum.min(sample);
            maximum = maximum.max(sample);
        }

        assert!(minimum < -0.9, "the triangle did not reach its bottom");
        assert!(maximum > 0.9, "the triangle did not reach its top");
    }

    #[test]
    fn pulse_width_controls_the_duty_cycle() {
        let frequency = 100.0;
        let dt = frequency / SAMPLE_RATE;

        for width in [0.25_f32, 0.5, 0.75] {
            let mut osc = Oscillator::new();
            // Render exactly one period.
            let samples = (SAMPLE_RATE / frequency) as usize;
            let mut high = 0;
            for _ in 0..samples {
                let sample = osc.next(dt, Waveform::Pulse, width);
                if sample > 0.0 {
                    high += 1;
                }
            }

            let duty = high as f32 / samples as f32;
            assert!(
                (duty - width).abs() < 0.05,
                "expected a duty cycle of {width}, got {duty}"
            );
        }
    }

    #[test]
    fn noise_stays_in_range() {
        let mut noise = Noise::new(1234);
        for _ in 0..10_000 {
            let sample = noise.next();
            assert!(sample.is_finite());
            assert!((-1.0..=1.0).contains(&sample), "{sample} is out of range");
        }
    }
}
