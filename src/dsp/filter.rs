//! A topology-preserving state variable filter (Zavalishin), the original
//! one-sample-delayed Moog-style ladder and a zero-delay-feedback ladder.

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

    /// The coefficients that were last set, as `(p, k, rez)`. Only the tests
    /// that pin the old topology use this.
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

/// The zero-delay-feedback (ZDF) four-pole ladder of the `24 dB` (new) slope.
///
/// Unlike [`Ladder`], which is the original one-sample-delayed feedback
/// structure and is kept for `24 dB OLD`, this filter solves the feedback loop
/// for the current sample. Each pole is a topology-preserving transform (TPT)
/// one-pole with `G = g / (1 + g)` and `g = tan(pi * cutoff / sample_rate)`,
/// which makes the cutoff accurate up to Nyquist without any per-sample
/// trigonometry. The loop is linear and the feedback gain stays below four, so
/// the filter is unconditionally stable; the only nonlinearity is the
/// saturating input stage.
///
/// The feedback gain is chosen so that the gain at the cutoff matches the
/// 12 dB SVF at the same resonance. The output is then scaled by `1 + k`,
/// which restores the unity passband gain that the ladder feedback would
/// otherwise remove. Together these keep the small-signal level close to the
/// 12 dB filter over the whole resonance range. As in every feedback ladder,
/// the actual resonant maximum sits a fraction of a percent below the cutoff
/// and is a few dB above the gain at the cutoff; that is inherent to the
/// four-pole feedback loop.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct LadderModern {
    /// `g = tan(pi * cutoff / sample_rate)`.
    g: f32,
    /// The gain of one TPT one-pole, `g / (1 + g)`.
    stage_gain: f32,
    /// The feedback gain. It stays below four, so the loop is always stable.
    k: f32,
    /// The output compensation, `1 + k`.
    output_gain: f32,
    /// `(1 - G) * G^3`, the contribution of the first integrator state to the
    /// fourth pole output.
    c0: f32,
    /// `(1 - G) * G^2`, the contribution of the second integrator state.
    c1: f32,
    /// `(1 - G) * G`, the contribution of the third integrator state.
    c2: f32,
    /// `1 - G`, the contribution of the fourth integrator state.
    c3: f32,
    /// `G^4`, the direct gain from the loop input to the fourth pole output.
    g4: f32,
    /// `1 / (1 + k * G^4)`, the zero-delay feedback solve.
    inv_denominator: f32,
    /// The state of the four TPT integrators.
    s0: f32,
    s1: f32,
    s2: f32,
    s3: f32,
    /// The anti-aliased saturation of the input stage.
    drive_adaa: AdaaTanh,
    /// The drive of the previous sample, so that the ADAA state is cleared
    /// when the drive is switched on after a fully clean sample.
    last_drive: f32,
}

impl LadderModern {
    pub fn new() -> Self {
        Self::default()
    }

    /// Updates the cutoff and resonance coefficients. Called at the control
    /// rate, not once per sample.
    ///
    /// `resonance` is the same `0..1` control value that the 12 dB filter
    /// uses. The feedback gain is derived from the damping of the SVF, so the
    /// two filters have the same gain at the cutoff.
    pub fn set(&mut self, cutoff_hz: f32, resonance: f32, sample_rate: f32) {
        let sample_rate = sample_rate.max(1.0);
        let cutoff = cutoff_hz.clamp(20.0, sample_rate * 0.45);
        let g = (std::f32::consts::PI * cutoff / sample_rate).tan();
        let stage_gain = g / (1.0 + g);

        // Match the resonant peak of the 12 dB SVF. Its damping is
        // `ksvf = 2 - 1.98 * q`, and its gain at the cutoff is `1 / ksvf`.
        // The ladder gain at the cutoff is `(1 + k) / (4 - k)` after the
        // output compensation, hence
        //
        //   (1 + k) / (4 - k) = 1 / ksvf
        //   k = (2 + 1.98 q) / (3 - 1.98 q),
        //
        // which runs from 2/3 at q = 0 to about 3.902 at q = 1. The whole
        // curve is monotonic and never reaches the self-oscillation point at
        // four, so the linear feedback loop cannot run away.
        let q = resonance.clamp(0.0, 1.0);
        let k = (2.0 + 1.98 * q) / (3.0 - 1.98 * q);
        let output_gain = 1.0 + k;

        let om = 1.0 - stage_gain;
        let g2 = stage_gain * stage_gain;
        let g3 = g2 * stage_gain;
        let g4 = g3 * stage_gain;

        self.g = g;
        self.stage_gain = stage_gain;
        self.k = k;
        self.output_gain = output_gain;
        self.c0 = om * g3;
        self.c1 = om * g2;
        self.c2 = om * stage_gain;
        self.c3 = om;
        self.g4 = g4;
        self.inv_denominator = 1.0 / (1.0 + k * g4);
    }

    #[inline]
    pub fn reset(&mut self) {
        self.s0 = 0.0;
        self.s1 = 0.0;
        self.s2 = 0.0;
        self.s3 = 0.0;
        self.drive_adaa.reset();
        self.last_drive = 0.0;
    }

    /// The coefficients that were last set, as `(g, k, output_gain)`. Only the
    /// tests use this.
    #[cfg(test)]
    pub(crate) fn coefficients(&self) -> (f32, f32, f32) {
        (self.g, self.k, self.output_gain)
    }

    /// Processes a single sample.
    ///
    /// The input stage uses exactly the drive law of the old [`Ladder`]:
    /// `tanh(input * (1 + 8 * drive)) / (1 + 2 * drive)` when the drive is
    /// on, and the untouched input when it is off. The saturator is
    /// anti-aliased with [`AdaaTanh`], and its state is reset when the drive
    /// comes back after a fully clean sample.
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

        // The fourth pole output is `G^4 * x + S`, where `x` is the input of
        // the feedback loop and `S` is the state contribution. With
        // `x = input - k * y4`, solving for `y4` gives the zero-delay
        // feedback expression below.
        let state = self.c0 * self.s0 + self.c1 * self.s1 + self.c2 * self.s2 + self.c3 * self.s3;
        let y4 = (self.g4 * input + state) * self.inv_denominator;
        let x = input - self.k * y4;

        // Run the four TPT one-poles forward with the solved loop input.
        let g = self.stage_gain;

        let v0 = (x - self.s0) * g;
        let y0 = v0 + self.s0;
        self.s0 = y0 + v0;

        let v1 = (y0 - self.s1) * g;
        let y1 = v1 + self.s1;
        self.s1 = y1 + v1;

        let v2 = (y1 - self.s2) * g;
        let y2 = v2 + self.s2;
        self.s2 = y2 + v2;

        let v3 = (y2 - self.s3) * g;
        let y3 = v3 + self.s3;
        self.s3 = y3 + v3;

        // Flush the denormal state, so that a silent voice does not drag the
        // CPU down.
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

        y3 * self.output_gain
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_SAMPLE_RATE: f32 = 48_000.0;

    /// The level tolerance of the modern ladder against the 12 dB SVF. The
    /// acceptance target is 1-2 dB; the test uses the tighter end of it.
    ///
    /// The comparison is made well inside the passband (0.22 of the cutoff),
    /// where both filters are nominally flat, and at the cutoff, where the
    /// resonance compensation anchors the peak. The two slopes necessarily
    /// differ further up the rolloff, so a four-pole filter cannot match the
    /// twelve dB filter at every frequency.
    const MODERN_LEVEL_TOLERANCE_DB: f32 = 1.0;

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32).sqrt()
    }

    fn decibels(ratio: f32) -> f32 {
        20.0 * ratio.log10()
    }

    /// Renders a sine through the modern ladder and returns its tail.
    fn modern_sine(
        frequency: f32,
        cutoff: f32,
        resonance: f32,
        drive: f32,
        amplitude: f32,
        settle: usize,
        measure: usize,
    ) -> Vec<f32> {
        let mut filter = LadderModern::new();
        filter.set(cutoff, resonance, TEST_SAMPLE_RATE);
        let mut output = Vec::with_capacity(measure);
        for index in 0..(settle + measure) {
            let input = amplitude
                * (std::f32::consts::TAU * frequency * index as f32 / TEST_SAMPLE_RATE).sin();
            let sample = filter.process(input, drive);
            if index >= settle {
                output.push(sample);
            }
        }
        output
    }

    fn modern_sine_rms(
        frequency: f32,
        cutoff: f32,
        resonance: f32,
        drive: f32,
        amplitude: f32,
        settle: usize,
        measure: usize,
    ) -> f32 {
        rms(&modern_sine(
            frequency, cutoff, resonance, drive, amplitude, settle, measure,
        ))
    }

    /// Renders a sine through the old ladder and returns its tail RMS.
    fn old_sine_rms(
        frequency: f32,
        cutoff: f32,
        resonance: f32,
        drive: f32,
        amplitude: f32,
        settle: usize,
        measure: usize,
    ) -> f32 {
        let mut filter = Ladder::new();
        filter.set(cutoff, resonance, TEST_SAMPLE_RATE);
        let mut output = Vec::with_capacity(measure);
        for index in 0..(settle + measure) {
            let input = amplitude
                * (std::f32::consts::TAU * frequency * index as f32 / TEST_SAMPLE_RATE).sin();
            let sample = filter.process(input, drive);
            if index >= settle {
                output.push(sample);
            }
        }
        rms(&output)
    }

    fn svf_sine_rms(
        frequency: f32,
        cutoff: f32,
        resonance: f32,
        amplitude: f32,
        settle: usize,
        measure: usize,
    ) -> f32 {
        let mut filter = Svf::new();
        let g = (std::f32::consts::PI * cutoff / TEST_SAMPLE_RATE).tan();
        let k = 2.0 - 1.98 * resonance;
        let mut output = Vec::with_capacity(measure);
        for index in 0..(settle + measure) {
            let input = amplitude
                * (std::f32::consts::TAU * frequency * index as f32 / TEST_SAMPLE_RATE).sin();
            let sample = filter.process(input, g, k, FilterMode::Lowpass, 0.0, false);
            if index >= settle {
                output.push(sample);
            }
        }
        rms(&output)
    }

    /// Renders a sine through the modern ladder and returns its largest
    /// absolute sample. The whole run is checked for non-finite values.
    fn modern_sine_peak(
        frequency: f32,
        cutoff: f32,
        resonance: f32,
        drive: f32,
        amplitude: f32,
        samples: usize,
    ) -> f32 {
        let mut filter = LadderModern::new();
        filter.set(cutoff, resonance, TEST_SAMPLE_RATE);
        let mut peak: f32 = 0.0;
        for index in 0..samples {
            let input = amplitude
                * (std::f32::consts::TAU * frequency * index as f32 / TEST_SAMPLE_RATE).sin();
            let sample = filter.process(input, drive);
            assert!(sample.is_finite(), "the modern ladder produced {sample}");
            peak = peak.max(sample.abs());
        }
        peak
    }

    fn fingerprint(hash: &mut u64, sample: f32) {
        *hash ^= sample.to_bits() as u64;
        *hash = hash.wrapping_mul(0x100_0000_01b3);
    }

    #[test]
    fn the_modern_ladder_cutoff_is_accurate() {
        let settle = TEST_SAMPLE_RATE as usize;
        let measure = (TEST_SAMPLE_RATE * 0.5) as usize;

        for cutoff in [100.0, 200.0, 500.0, 1_000.0, 2_000.0, 4_000.0, 8_000.0] {
            // At full resonance the peak sits exactly at the prewarped
            // cutoff. A 3 % detune has to lose a clear amount of gain, which
            // locates the peak to within a few percent.
            let center = modern_sine_rms(cutoff, cutoff, 1.0, 0.0, 0.001, settle, measure);
            for edge in [cutoff * 0.97, cutoff * 1.03] {
                let side = modern_sine_rms(edge, cutoff, 1.0, 0.0, 0.001, settle, measure);
                assert!(
                    center > side * 1.2,
                    "the modern ladder peak at {cutoff} Hz was not at the cutoff: {center} vs {side} at {edge} Hz"
                );
            }

            // The gain at the cutoff is the one of the 12 dB SVF at q = 1,
            // which is 1 / (2 - 1.98) = 50. The true maximum is slightly
            // below the cutoff and a few dB higher, as in any feedback ladder.
            let gain = center / (0.001 / 2f32.sqrt());
            assert!(
                (decibels(gain) - decibels(50.0)).abs() < 0.6,
                "the modern ladder peak at {cutoff} Hz was {:.2} dB instead of {:.2} dB",
                decibels(gain),
                decibels(50.0)
            );
        }
    }

    #[test]
    fn the_modern_ladder_resonance_is_monotonic_and_stable() {
        for cutoff in [20.0, 200.0, 800.0, 3_000.0, 8_000.0] {
            let mut previous_k = 0.0;
            for step in 0..=100 {
                let q = step as f32 / 100.0;
                let mut ladder = LadderModern::new();
                ladder.set(cutoff, q, TEST_SAMPLE_RATE);
                let (_, k, _) = ladder.coefficients();
                assert!(
                    k > previous_k,
                    "the modern resonance dipped at {cutoff} Hz and q={q}: {k} <= {previous_k}"
                );
                previous_k = k;
            }
            assert!(
                previous_k < 4.0,
                "the modern feedback reached the unstable point at {cutoff} Hz: {previous_k}"
            );

            // The height of the peak at the cutoff has to grow with the
            // resonance as well, not just the feedback coefficient.
            let settle = if cutoff < 100.0 {
                (TEST_SAMPLE_RATE * 4.0) as usize
            } else {
                TEST_SAMPLE_RATE as usize
            };
            let measure = (TEST_SAMPLE_RATE * 0.5) as usize;
            let mut previous_level = 0.0;
            for q in [0.0, 0.25, 0.5, 0.75, 1.0] {
                let level = modern_sine_rms(cutoff, cutoff, q, 0.0, 0.001, settle, measure);
                assert!(
                    level > previous_level * 1.01,
                    "the modern peak did not grow at {cutoff} Hz and q={q}: {level} <= {previous_level}"
                );
                previous_level = level;
            }

            // The loop is linear and its gain stays below four, so it cannot
            // run away. Check both an impulse and a loud sine at full
            // resonance, with and without drive.
            for drive in [0.0, 1.0] {
                let mut ladder = LadderModern::new();
                ladder.set(cutoff, 1.0, TEST_SAMPLE_RATE);
                let mut peak: f32 = 0.0;
                for sample in 0..(TEST_SAMPLE_RATE as usize * 2) {
                    let input = if sample == 0 { 1.0 } else { 0.0 };
                    let output = ladder.process(input, drive);
                    assert!(
                        output.is_finite(),
                        "the modern ladder produced {output} at {cutoff} Hz with drive {drive}"
                    );
                    peak = peak.max(output.abs());
                }
                assert!(
                    peak < 2.0,
                    "the modern ladder impulse ran away at {cutoff} Hz with drive {drive} ({peak})"
                );

                let peak = modern_sine_peak(
                    cutoff,
                    cutoff,
                    1.0,
                    drive,
                    0.5,
                    (TEST_SAMPLE_RATE * 2.0) as usize,
                );
                assert!(
                    peak < 40.0,
                    "the modern ladder sine ran away at {cutoff} Hz with drive {drive} ({peak})"
                );
            }
        }
    }

    #[test]
    fn the_modern_ladder_level_matches_the_svf() {
        let cutoff = 1_000.0;
        let passband = 220.0;
        let amplitude = 0.01;
        let settle = (TEST_SAMPLE_RATE * 0.5) as usize;
        let measure = (TEST_SAMPLE_RATE * 0.5) as usize;
        let mut passband_levels = Vec::new();

        for q in [0.0, 0.1, 0.3, 0.5, 0.7, 0.9, 1.0] {
            let modern_passband =
                modern_sine_rms(passband, cutoff, q, 0.0, amplitude, settle, measure);
            let svf_passband = svf_sine_rms(passband, cutoff, q, amplitude, settle, measure);
            let difference = decibels(modern_passband / svf_passband);
            assert!(
                difference.abs() <= MODERN_LEVEL_TOLERANCE_DB,
                "the modern passband level at q={q} was {difference:+.2} dB from the SVF"
            );

            let modern_cutoff = modern_sine_rms(cutoff, cutoff, q, 0.0, amplitude, settle, measure);
            let svf_cutoff = svf_sine_rms(cutoff, cutoff, q, amplitude, settle, measure);
            let difference = decibels(modern_cutoff / svf_cutoff);
            assert!(
                difference.abs() <= MODERN_LEVEL_TOLERANCE_DB,
                "the modern cutoff level at q={q} was {difference:+.2} dB from the SVF"
            );

            passband_levels.push(modern_passband);
        }

        // The resonance sweep must not move the passband level around.
        let reference = passband_levels[0];
        for level in passband_levels {
            let difference = decibels(level / reference);
            assert!(
                difference.abs() <= MODERN_LEVEL_TOLERANCE_DB,
                "the modern passband swung {difference:+.2} dB over the resonance sweep"
            );
        }
    }

    #[test]
    fn the_modern_ladder_drive_follows_the_old_gain_law() {
        let cutoff = 1_000.0;
        let frequency = 220.0;
        // A small signal keeps the tanh linear enough that the level follows
        // the drive law itself, not the compression of a hot input.
        let amplitude = 0.01;
        let q = 0.3;
        let settle = (TEST_SAMPLE_RATE * 0.25) as usize;
        let measure = (TEST_SAMPLE_RATE * 0.25) as usize;

        let reference = modern_sine_rms(frequency, cutoff, q, 0.0, amplitude, settle, measure);
        for drive in [0.01, 0.05, 0.1, 0.15, 0.5, 1.0] {
            let level = modern_sine_rms(frequency, cutoff, q, drive, amplitude, settle, measure);
            let difference = decibels(level / reference);
            let expected = decibels((1.0 + 8.0 * drive) / (1.0 + 2.0 * drive));
            assert!(
                (difference - expected).abs() <= MODERN_LEVEL_TOLERANCE_DB,
                "a drive of {drive} moved the small-signal level by {difference:+.2} dB instead of {expected:+.2} dB"
            );
        }

        // A hot signal saturates, but it stays finite. The drive law boosts
        // small signals; only once the tanh is saturated does the level fall
        // back below the clean signal.
        let hot = 0.5;
        let reference = modern_sine_rms(frequency, cutoff, q, 0.0, hot, settle, measure);
        let full = modern_sine_rms(frequency, cutoff, q, 1.0, hot, settle, measure);
        assert!(full.is_finite() && full > 0.0 && full < reference);
    }

    #[test]
    fn the_modern_ladder_drive_matches_the_old_small_signal_gain() {
        let cutoff = 1_000.0;
        let frequency = 220.0;
        let amplitude = 1.0e-3;
        let q = 0.3;
        let settle = (TEST_SAMPLE_RATE * 0.25) as usize;
        let measure = (TEST_SAMPLE_RATE * 0.25) as usize;

        let old_reference = old_sine_rms(frequency, cutoff, q, 0.0, amplitude, settle, measure);
        let new_reference = modern_sine_rms(frequency, cutoff, q, 0.0, amplitude, settle, measure);
        for drive in [0.5, 1.0] {
            let old_level = old_sine_rms(frequency, cutoff, q, drive, amplitude, settle, measure);
            let new_level =
                modern_sine_rms(frequency, cutoff, q, drive, amplitude, settle, measure);
            let old_gain = old_level / old_reference;
            let new_gain = new_level / new_reference;
            let difference = decibels(old_gain / new_gain);
            assert!(
                difference.abs() < 0.1,
                "at drive {drive} the old and new small-signal gains differ by {difference:+.3} dB ({old_gain} vs {new_gain})"
            );
        }
    }

    #[test]
    fn the_modern_ladder_drive_zero_is_transparent() {
        let cutoff = 1_200.0;
        let q = 0.4;

        fn first_sample(input: f32, cutoff: f32, q: f32) -> f32 {
            let mut filter = LadderModern::new();
            filter.set(cutoff, q, TEST_SAMPLE_RATE);
            filter.process(input, 0.0)
        }

        // With drive off, the input stage is exactly the identity. A fresh
        // filter is linear, so every first sample has to scale with the input
        // instead of being bent by a tanh.
        let reference = first_sample(1.0, cutoff, q);
        for input in [1.0e-6, 0.001, 0.5, -0.75, 8.0] {
            let actual = first_sample(input, cutoff, q);
            let expected = reference * input;
            let tolerance = expected.abs() * 1.0e-5 + 1.0e-12;
            assert!(
                (actual - expected).abs() <= tolerance,
                "drive 0 was not transparent for an input of {input}: {actual} instead of {expected}"
            );
        }
    }

    #[test]
    fn the_old_ladder_is_bit_identical_to_the_previous_implementation() {
        // These fingerprints were captured from the implementation before the
        // modern ZDF ladder was added. They pin the old 24 dB path exactly.
        for (cutoff, q, drive, expected) in [
            (200.0, 0.0, 0.0, 0x93d5_a683_4fcc_1a88u64),
            (800.0, 0.35, 0.2, 0x38e7_18fc_bf8c_d84fu64),
            (4_000.0, 0.8, 0.6, 0x5601_2655_ec23_305au64),
            (12_000.0, 1.0, 1.0, 0xe715_956f_b8a2_54bau64),
        ] {
            let mut ladder = Ladder::new();
            ladder.set(cutoff, q, TEST_SAMPLE_RATE);
            let mut hash = 0xcbf2_9ce4_8422_2325u64;
            for sample in 0..8_192 {
                let input = if sample == 0 { 1.0 } else { 0.0 };
                fingerprint(&mut hash, ladder.process(input, drive));
            }
            assert_eq!(
                hash, expected,
                "the old ladder changed at {cutoff} Hz, q={q}, drive={drive}"
            );
        }
    }

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
