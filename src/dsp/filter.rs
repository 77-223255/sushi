//! A topology-preserving state variable filter (Zavalishin).

use crate::params::FilterMode;

use super::adaa::AdaaTanh;

/// Topology-preserving state variable filter. All four classic filter modes
/// are available: the lowpass, highpass, bandpass and notch outputs are all
/// derived from the same pair of integrators.
#[derive(Debug, Clone, Copy, Default)]
pub struct Svf {
    ic1: f32,
    ic2: f32,
    drive_adaa: AdaaTanh,
    last_drive: f32,
    /// The `g` that the cached coefficients were derived from.
    cached_g: f32,
    /// The `k` that the cached coefficients were derived from.
    cached_k: f32,
    /// Whether [`Svf::set`] has run at least once, so that the cached
    /// coefficients are valid.
    prepared: bool,
    /// Cached coefficient of the first integrator.
    a1: f32,
    /// Cached coefficient of the input into the first integrator.
    a2: f32,
    /// Cached coefficient of the input into the second integrator.
    a3: f32,
    /// Recomputes the coefficients from the cached `g` and `k` for every
    /// sample. Only the regression test that compares the cached and the
    /// uncached path turns this on.
    #[cfg(test)]
    recompute: bool,
}

impl Svf {
    pub fn new() -> Self {
        Self::default()
    }

    /// Derives the coefficients of the filter from `g` and `k`. Call this at
    /// the control rate and process the samples with
    /// [`Svf::process_prepared`], which saves a division per sample.
    ///
    /// `g` is `tan(pi * cutoff / sample_rate)` and `k` is
    /// `2 - 1.98 * resonance`, where a resonance approaching 1 means a high Q.
    #[inline]
    pub fn set(&mut self, g: f32, k: f32) {
        if self.prepared && self.cached_g == g && self.cached_k == k {
            return;
        }

        self.cached_g = g;
        self.cached_k = k;
        self.compute_coefficients();
        self.prepared = true;
    }

    /// Recomputes `a1`, `a2` and `a3` from the cached `g` and `k`. The
    /// arithmetic is the one of the original per-sample path, so the cached
    /// values are bit-identical.
    #[inline]
    fn compute_coefficients(&mut self) {
        let g = self.cached_g;
        let k = self.cached_k;
        let a1 = 1.0 / (1.0 + g * (g + k));
        self.a1 = a1;
        self.a2 = g * a1;
        self.a3 = g * self.a2;
    }

    /// Processes a single sample with coefficients that were prepared by
    /// [`Svf::set`]. `drive` adds saturation to the input stage.
    #[inline]
    pub fn process_prepared(
        &mut self,
        input: f32,
        mode: FilterMode,
        drive: f32,
        legacy: bool,
    ) -> f32 {
        #[cfg(test)]
        if self.recompute {
            self.compute_coefficients();
        }

        let input = if drive > 0.0 {
            if legacy {
                (input * (1.0 + 8.0 * drive)).tanh() / (1.0 + 2.0 * drive)
            } else {
                if self.last_drive <= 0.0 {
                    self.drive_adaa.reset();
                }
                self.drive_adaa.process(input * (1.0 + 8.0 * drive)) / (1.0 + 2.0 * drive)
            }
        } else {
            input
        };
        self.last_drive = drive;

        let a1 = self.a1;
        let a2 = self.a2;
        let a3 = self.a3;
        let k = self.cached_k;

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

    /// Processes a single sample.
    ///
    /// `g` is `tan(pi * cutoff / sample_rate)` and `k` is `2 - 1.98 * resonance`,
    /// where a resonance approaching 1 means a high Q. `drive` adds saturation
    /// to the input stage. The coefficients are recomputed (and cached) when
    /// `g` or `k` change, so this API stays a drop-in for the uncached path.
    #[inline]
    pub fn process(
        &mut self,
        input: f32,
        g: f32,
        k: f32,
        mode: FilterMode,
        drive: f32,
        legacy: bool,
    ) -> f32 {
        self.set(g, k);
        self.process_prepared(input, mode, drive, legacy)
    }

    /// Forces the coefficients to be recomputed for every sample, as the
    /// original path did. Only used to prove that the cached path is
    /// bit-identical.
    #[cfg(test)]
    pub(crate) fn force_uncached(&mut self) {
        self.recompute = true;
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
                    let output = filter.process(input, g, k, mode, 0.0, false);
                    assert!(output.is_finite(), "{mode:?} produced {output}");
                    last = output;
                }

                // The filter must settle back to silence.
                assert!(last.abs() < 1e-3, "{mode:?} did not decay: {last}");
            }
        }
    }

    #[test]
    fn the_cached_svf_matches_the_per_sample_path() {
        for legacy in [false, true] {
            for drive in [0.0, 0.6] {
                for mode in [
                    FilterMode::Lowpass,
                    FilterMode::Highpass,
                    FilterMode::Bandpass,
                    FilterMode::Notch,
                ] {
                    let mut cached = Svf::new();
                    let mut uncached = Svf::new();
                    uncached.force_uncached();

                    // A new cutoff and resonance every 16 samples, as the
                    // voice updates them at its control rate.
                    for block in 0..64 {
                        let g = 0.002 + block as f32 * 0.012;
                        let k = 0.1 + block as f32 * 0.025;
                        cached.set(g, k);

                        for sample in 0..16 {
                            let input = (sample as f32 * 0.7).sin() * 0.9;
                            let expected = uncached.process(input, g, k, mode, drive, legacy);
                            let actual = cached.process_prepared(input, mode, drive, legacy);
                            assert_eq!(
                                actual.to_bits(),
                                expected.to_bits(),
                                "{mode:?}, drive {drive}, legacy {legacy}, block {block}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn drive_does_not_produce_nonsense() {
        let mut filter = Svf::new();
        let g = (std::f32::consts::PI * 1_000.0 / 48_000.0).tan();
        let k = 2.0 - 1.98 * 0.5;

        for _ in 0..1_000 {
            let output = filter.process(4.0, g, k, FilterMode::Lowpass, 1.0, false);
            assert!(output.is_finite());
            assert!(output.abs() <= 4.0);
        }
    }
}
