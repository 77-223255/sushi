
use super::super::params::Waveform;
use std::f32::consts::TAU;

/// The maximum amplitude an oscillator can produce. The PolyBLEP residuals can
/// briefly overshoot the nominal ±1 range, so the output is clipped here.
const MAX_AMPLITUDE: f32 = 1.2;

/// The leak of the integrator of the triangle wave. It is very slow, so that
/// the shape of the wave is not affected by it, but it keeps the integrator
/// from drifting away over time.
const TRIANGLE_LEAK: f32 = 0.999_99;

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

    #[inline]
    pub fn set_phase(&mut self, phase: f32) {
        let phase = phase.rem_euclid(1.0);
        self.phase = phase;
        // The integrator of the triangle starts at the value of the wave, so
        // that the triangle does not have to settle first.
        self.triangle = naive_triangle(phase);
    }

    
    #[inline]
    pub fn next(&mut self, phase_delta: f32, waveform: Waveform, pulse_width: f32) -> f32 {
        self.next_modulated(phase_delta, waveform, pulse_width, 0.0)
            .0
    }

    #[inline]
    pub fn next_modulated(
        &mut self,
        phase_delta: f32,
        waveform: Waveform,
        pulse_width: f32,
        phase_offset: f32,
    ) -> (f32, bool) {
        let phase = (self.phase + phase_offset).rem_euclid(1.0);
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
        let wrapped = self.phase >= 1.0;
        if wrapped {
            self.phase -= 1.0;
        }

        (sample.clamp(-MAX_AMPLITUDE, MAX_AMPLITUDE), wrapped)
    }
}

#[inline]
fn naive_triangle(phase: f32) -> f32 {
    if phase < 0.5 {
        4.0 * phase - 1.0
    } else {
        3.0 - 4.0 * phase
    }
}

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

