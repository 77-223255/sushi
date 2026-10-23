//! A topology-preserving state variable filter (Zavalishin) and a nonlinear
//! Moog-style ladder lowpass.

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

/// A four-pole Moog-style ladder lowpass. The first three stages are linear
/// TPT one-poles and the last stage is saturated, which gives the filter a
/// resonant, slightly compressed character that the plain cascaded SVF does
/// not have. The topology follows the classic Stilson-Smith ladder model.
#[derive(Debug, Clone, Copy, Default)]
pub struct Ladder {
    p: f32,
    k: f32,
    rez: f32,
    s0: f32,
    s1: f32,
    s2: f32,
    s3: f32,
    px: f32,
    ps0: f32,
    ps1: f32,
    ps2: f32,
    drive_adaa: AdaaTanh,
    last_drive: f32,
}

impl Ladder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Updates the cutoff and resonance coefficients. Called at the control
    /// rate, not once per sample.
    pub fn set(&mut self, cutoff_hz: f32, resonance: f32, sample_rate: f32) {
        let sample_rate = sample_rate.max(1.0);
        let cutoff = cutoff_hz.clamp(20.0, sample_rate * 0.45);
        let c = (2.0 * cutoff / sample_rate).clamp(0.0, 0.99);
        self.p = c * (1.8 - 0.8 * c);
        self.k = 2.0 * (c * std::f32::consts::PI * 0.5).sin() - 1.0;

        // The original ladder feedback curve, normalized so 1.0 is close to
        // self oscillation but still bounded by the saturated stage.
        let t1 = (1.0 - self.p) * 1.386_249;
        let t2 = 12.0 + t1 * t1;
        let q = resonance.clamp(0.0, 1.0);
        self.rez = (q * (t2 + 6.0 * t1) / (t2 - 6.0 * t1)).clamp(0.0, 0.999);
    }

    /// Updates the cutoff and resonance coefficients for the corrected ladder.
    /// The topology is the same as [`Ladder::set`], but the resonance curve is
    /// monotonic instead of being clamped at 0.999. The saturated last stage
    /// keeps the feedback bounded.
    pub fn set_modern(&mut self, cutoff_hz: f32, resonance: f32, sample_rate: f32) {
        let sample_rate = sample_rate.max(1.0);
        let cutoff = cutoff_hz.clamp(20.0, sample_rate * 0.45);
        let c = (2.0 * cutoff / sample_rate).clamp(0.0, 0.99);
        self.p = c * (1.8 - 0.8 * c);
        self.k = 2.0 * (c * std::f32::consts::PI * 0.5).sin() - 1.0;

        // The same normalized curve as the old ladder, but without the
        // 0.999 ceiling. 3.95 is just below the point where the ladder would
        // start to self-oscillate; the tanh in the last stage bounds it.
        let t1 = (1.0 - self.p) * 1.386_249;
        let t2 = 12.0 + t1 * t1;
        let q = resonance.clamp(0.0, 1.0);
        self.rez = (q * (t2 + 6.0 * t1) / (t2 - 6.0 * t1)).clamp(0.0, 3.95);
    }

    #[inline]
    pub fn reset(&mut self) {
        self.s0 = 0.0;
        self.s1 = 0.0;
        self.s2 = 0.0;
        self.s3 = 0.0;
        self.px = 0.0;
        self.ps0 = 0.0;
        self.ps1 = 0.0;
        self.ps2 = 0.0;
        self.drive_adaa.reset();
        self.last_drive = 0.0;
    }

    /// The coefficients that were last set, as `(p, k, rez)`. Only the voice
    /// test that checks the topology switch uses this.
    #[cfg(test)]
    pub(crate) fn coefficients(&self) -> (f32, f32, f32) {
        (self.p, self.k, self.rez)
    }

    #[inline]
    pub fn process(&mut self, input: f32, drive: f32) -> f32 {
        let input = if drive > 0.0 {
            if self.last_drive <= 0.0 {
                self.drive_adaa.reset();
            }
            self.drive_adaa.process(input * (1.0 + 8.0 * drive)) / (1.0 + 2.0 * drive)
        } else {
            input
        };
        self.last_drive = drive;

        let x = input - self.rez * self.s3;
        self.s0 = (x + self.px) * self.p - self.k * self.s0;
        self.s1 = (self.s0 + self.ps0) * self.p - self.k * self.s1;
        self.s2 = (self.s1 + self.ps1) * self.p - self.k * self.s2;
        self.s3 = ((self.s2 + self.ps2) * self.p - self.k * self.s3).tanh();

        self.px = x;
        self.ps0 = self.s0;
        self.ps1 = self.s1;
        self.ps2 = self.s2;

        // Flush the denormal state so a silent voice does not drag the CPU
        // down. The saturated stage is part of the feedback path, so it is
        // flushed as well once every state has become denormal.
        if self.s0.abs() < 1e-20
            && self.s1.abs() < 1e-20
            && self.s2.abs() < 1e-20
            && self.s3.abs() < 1e-20
        {
            self.s0 = 0.0;
            self.s1 = 0.0;
            self.s2 = 0.0;
            self.s3 = 0.0;
        }

        self.s3
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
    fn the_ladder_is_bounded_at_high_resonance() {
        let sample_rate = 48_000.0;
        for cutoff in [20.0, 800.0, 5_000.0] {
            let mut ladder = Ladder::new();
            ladder.set(cutoff, 0.98, sample_rate);

            let mut peak: f32 = 0.0;
            for sample in 0..48_000 {
                let input = if sample == 0 { 1.0 } else { 0.0 };
                let output = ladder.process(input, 0.0);
                assert!(output.is_finite());
                peak = peak.max(output.abs());
            }

            assert!(peak < 10.0, "the ladder ran away at {cutoff} Hz ({peak})");
        }
    }

    #[test]
    fn the_modern_ladder_resonance_is_monotonic() {
        let sample_rate = 48_000.0;
        for cutoff in [20.0, 200.0, 800.0, 3_000.0, 8_000.0] {
            let mut ladder = Ladder::new();
            let mut previous = 0.0;
            for step in 0..=100 {
                let q = step as f32 / 100.0;
                ladder.set_modern(cutoff, q, sample_rate);
                let (_, _, rez) = ladder.coefficients();
                assert!(
                    rez >= previous,
                    "the modern resonance dipped at {cutoff} Hz and q={q}: {rez} < {previous}"
                );
                previous = rez;
            }

            assert!(
                previous > 1.0,
                "the modern resonance did not open at {cutoff} Hz: {previous}"
            );
        }
    }

    #[test]
    fn the_modern_ladder_is_bounded_at_full_resonance() {
        let sample_rate = 48_000.0;
        for cutoff in [20.0, 100.0, 800.0, 5_000.0, 12_000.0] {
            for drive in [0.0, 1.0] {
                let mut ladder = Ladder::new();
                ladder.set_modern(cutoff, 1.0, sample_rate);

                let mut peak: f32 = 0.0;
                for sample in 0..48_000 {
                    let input = if sample == 0 { 1.0 } else { 0.0 };
                    let output = ladder.process(input, drive);
                    assert!(
                        output.is_finite(),
                        "the modern ladder produced {output} at {cutoff} Hz with drive {drive}"
                    );
                    peak = peak.max(output.abs());
                }

                assert!(
                    peak < 10.0,
                    "the modern ladder ran away at {cutoff} Hz with drive {drive} ({peak})"
                );
            }
        }
    }

    #[test]
    fn the_old_ladder_still_clamps_its_resonance() {
        let sample_rate = 48_000.0;
        for cutoff in [20.0, 200.0, 800.0, 3_000.0, 8_000.0] {
            let mut ladder = Ladder::new();

            ladder.set(cutoff, 0.0, sample_rate);
            assert_eq!(ladder.coefficients().2, 0.0);

            ladder.set(cutoff, 0.25, sample_rate);
            let partial = ladder.coefficients().2;
            assert!(partial > 0.0 && partial < 0.999);

            // The old curve saturates at 0.999 long before the top of the
            // resonance range, exactly as it always did.
            ladder.set(cutoff, 1.0, sample_rate);
            assert_eq!(ladder.coefficients().2, 0.999);
        }
    }

    #[test]
    fn the_ladder_is_steeper_than_the_svf() {
        let sample_rate = 48_000.0;
        let mut ladder = Ladder::new();
        ladder.set(200.0, 0.1, sample_rate);
        let mut svf = Svf::new();
        let g = (std::f32::consts::PI * 200.0 / sample_rate).tan();
        let k = 2.0 - 1.98 * 0.1;

        let mut ladder_energy = 0.0;
        let mut svf_energy = 0.0;
        for sample in 0..4_800 {
            let input = (sample as f32 * 2_000.0 / sample_rate * 2.0 * std::f32::consts::PI).sin();
            let ladder_out = ladder.process(input, 0.0);
            let svf_out = svf.process(input, g, k, FilterMode::Lowpass, 0.0, false);
            if sample > 1_000 {
                ladder_energy += ladder_out * ladder_out;
                svf_energy += svf_out * svf_out;
            }
        }

        assert!(
            ladder_energy < svf_energy,
            "the ladder was not steeper: {ladder_energy} vs {svf_energy}"
        );
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
