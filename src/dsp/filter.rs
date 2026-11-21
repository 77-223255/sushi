
use crate::params::FilterMode;

use super::adaa::AdaaTanh;

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
    }

impl Svf {
    pub fn new() -> Self {
        Self::default()
    }

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

    #[inline]
    fn compute_coefficients(&mut self) {
        let g = self.cached_g;
        let k = self.cached_k;
        let a1 = 1.0 / (1.0 + g * (g + k));
        self.a1 = a1;
        self.a2 = g * a1;
        self.a3 = g * self.a2;
    }

    #[inline]
    pub fn process_prepared(
        &mut self,
        input: f32,
        mode: FilterMode,
        drive: f32,
        legacy: bool,
    ) -> f32 {
        
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

    }

