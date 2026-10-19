//! A topology-preserving state variable filter (Zavalishin).

use crate::params::FilterMode;

/// Topology-preserving state variable filter. All four classic filter modes
/// are available: the lowpass, highpass, bandpass and notch outputs are all
/// derived from the same pair of integrators.
#[derive(Debug, Clone, Copy, Default)]
pub struct Svf {
    ic1: f32,
    ic2: f32,
}

impl Svf {
    pub fn new() -> Self {
        Self::default()
    }

    /// Processes a single sample.
    ///
    /// `g` is `tan(pi * cutoff / sample_rate)` and `k` is `2 - 1.98 * resonance`,
    /// where a resonance approaching 1 means a high Q. `drive` adds saturation
    /// to the input stage.
    #[inline]
    pub fn process(&mut self, input: f32, g: f32, k: f32, mode: FilterMode, drive: f32) -> f32 {
        let input = if drive > 0.0 {
            (input * (1.0 + 8.0 * drive)).tanh() / (1.0 + 2.0 * drive)
        } else {
            input
        };

        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;

        let v3 = input - self.ic2;
        let v1 = a1 * self.ic1 + a2 * v3;
        let v2 = self.ic2 + a2 * self.ic1 + a3 * v3;

        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;

        // Flush the denormal state, so that the filter doesn't drag the CPU
        // down with it once the signal has died away.
        if self.ic1.abs() < 1e-20 && self.ic2.abs() < 1e-20 {
            self.ic1 = 0.0;
            self.ic2 = 0.0;
        }

        match mode {
            FilterMode::Lowpass => v2,
            FilterMode::Highpass => input - k * v1 - v2,
            FilterMode::Bandpass => v1,
            FilterMode::Notch => input - k * v1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_modes_stay_stable() {
        let sample_rate = 48_000.0;

        for mode in [
            FilterMode::Lowpass,
            FilterMode::Highpass,
            FilterMode::Bandpass,
            FilterMode::Notch,
        ] {
            for resonance in [0.0, 0.5, 0.99] {
                let mut filter = Svf::new();
                let g = (std::f32::consts::PI * 2_000.0 / sample_rate).tan();
                let k = 2.0 - 1.98 * resonance;

                // An impulse excites every mode.
                let mut last = 0.0;
                for sample in 0..4_800 {
                    let input = if sample == 0 { 1.0 } else { 0.0 };
                    let output = filter.process(input, g, k, mode, 0.0);
                    assert!(output.is_finite(), "{mode:?} produced {output}");
                    last = output;
                }

                // The filter must settle back to silence.
                assert!(last.abs() < 1e-3, "{mode:?} did not decay: {last}");
            }
        }
    }

    #[test]
    fn drive_does_not_produce_nonsense() {
        let mut filter = Svf::new();
        let g = (std::f32::consts::PI * 1_000.0 / 48_000.0).tan();
        let k = 2.0 - 1.98 * 0.5;

        for _ in 0..1_000 {
            let output = filter.process(4.0, g, k, FilterMode::Lowpass, 1.0);
            assert!(output.is_finite());
            assert!(output.abs() <= 4.0);
        }
    }
}
