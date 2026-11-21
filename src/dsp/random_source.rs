//! A smooth random modulation source.
//!
//! The random source of the matrix is a sample and hold clock with a glide on
//! it: a new value is drawn every cycle of the rate, and the output travels to
//! it instead of jumping. With the glide at zero the source steps, like the
//! sample and hold waveform of an LFO; with the glide open it wanders, which is
//! what makes it useful as a musical modulation.

/// The state of the random source.
#[derive(Debug, Clone, Copy)]
pub struct SmoothRandom {
    /// The position in the current cycle, `0..1`.
    phase: f32,
    /// The value that is being travelled to.
    target: f32,
    /// The value that is being sent out.
    current: f32,
    /// The generator of the values.
    rng: u32,
}

impl SmoothRandom {
    /// A source with a fixed seed, so a session is repeatable.
    pub fn new(seed: u32) -> Self {
        let mut source = Self {
            phase: 0.0,
            target: 0.0,
            current: 0.0,
            rng: if seed == 0 { 0x9E37_79B9 } else { seed },
        };
        source.target = source.next_random();
        source.current = source.target;
        source
    }

    /// The value of one block. `rate` is the cycle rate in Hz, `smooth` is the
    /// glide in `0..1`, and `seconds` is the length of the block.
    pub fn next(&mut self, rate: f32, smooth: f32, seconds: f32) -> f32 {
        let seconds = seconds.max(0.0);
        self.phase += rate.max(0.0) * seconds;
        if self.phase >= 1.0 {
            self.phase -= self.phase.floor();
            self.target = self.next_random();
        }

        // A glide of zero is no glide at all: the source steps.
        let smooth = smooth.clamp(0.0, 1.0);
        if smooth == 0.0 {
            self.current = self.target;
        } else {
            // The glide time grows with the square of the control, so the low
            // end stays snappy and the top end is a slow wander.
            let glide = 0.02 + smooth * smooth * 1.2;
            let alpha = 1.0 - (-seconds / glide).exp();
            self.current += (self.target - self.current) * alpha;
        }
        self.current.clamp(-1.0, 1.0)
    }

    /// Restarts the cycle, used when the source is switched or reset.
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

impl Default for SmoothRandom {
    fn default() -> Self {
        Self::new(0x9E37_79B9)
    }
}
