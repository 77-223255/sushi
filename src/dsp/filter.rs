/// Topology-preserving state variable filter (Zavalishin). The lowpass output is
/// used, but bandpass/highpass are one line away if we want them later.
#[derive(Debug, Clone, Default)]
pub struct Svf {
    ic1: f32,
    ic2: f32,
}

impl Svf {
    pub fn new() -> Self {
        Self::default()
    }

    /// `g` is `tan(pi * cutoff / sample_rate)`, `k` is `2 - 2 * resonance` where
    /// resonance approaching 1 means a high Q.
    #[inline]
    pub fn lowpass(&mut self, input: f32, g: f32, k: f32) -> f32 {
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;

        let v3 = input - self.ic2;
        let v1 = a1 * self.ic1 + a2 * v3;
        let v2 = self.ic2 + a2 * self.ic1 + a3 * v3;

        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;

        v2
    }
}
