/// Per-sample coefficients for the amplitude envelope. These are computed once per
/// audio block so the inner loop stays free of `exp`/`powf` calls.
#[derive(Debug, Clone, Copy)]
pub struct EnvCoeffs {
    /// Linear increment per sample for the attack stage.
    attack_inc: f32,
    /// One-pole coefficient for the decay stage.
    decay_coef: f32,
    /// One-pole coefficient for the release stage.
    release_coef: f32,
    /// Sustain level, `0..1`.
    sustain: f32,
}

impl Default for EnvCoeffs {
    /// 48 kHz, 5 ms attack, 300 ms decay, 0.7 sustain and 400 ms release.
    fn default() -> Self {
        Self::new(5.0, 300.0, 0.7, 400.0, 48_000.0)
    }
}

impl EnvCoeffs {
    pub fn new(
        attack_ms: f32,
        decay_ms: f32,
        sustain: f32,
        release_ms: f32,
        sample_rate: f32,
    ) -> Self {
        let sample_rate = sample_rate.max(1.0);
        let attack_samples = (attack_ms * 0.001 * sample_rate).max(1.0);
        let decay_samples = (decay_ms * 0.001 * sample_rate).max(1.0);
        let release_samples = (release_ms * 0.001 * sample_rate).max(1.0);

        Self {
            attack_inc: 1.0 / attack_samples,
            // `ln(1000)` puts the -60 dB point at the configured time.
            decay_coef: (-6.907_755 / decay_samples).exp(),
            release_coef: (-6.907_755 / release_samples).exp(),
            sustain: sustain.clamp(0.0, 1.0),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Attack,
    Decay,
    Sustain,
    Release,
    Off,
}

/// A classic ADSR envelope running at audio rate.
#[derive(Debug, Clone)]
pub struct Adsr {
    stage: Stage,
    level: f32,
}

impl Default for Adsr {
    fn default() -> Self {
        Self {
            stage: Stage::Off,
            level: 0.0,
        }
    }
}

impl Adsr {
    /// Starts (or retriggers) the envelope. Retriggering keeps the current level so
    /// legato lines don't click.
    pub fn gate_on(&mut self) {
        if self.stage == Stage::Off {
            self.level = 0.0;
        }
        self.stage = Stage::Attack;
    }

    /// Enters the release stage. Safe to call multiple times.
    pub fn gate_off(&mut self) {
        if self.stage != Stage::Off {
            self.stage = Stage::Release;
        }
    }

    /// Whether the envelope has fully decayed to silence after a release.
    #[inline]
    pub fn is_finished(&self) -> bool {
        self.stage == Stage::Off
    }

    #[inline]
    pub fn next(&mut self, coeffs: &EnvCoeffs) -> f32 {
        match self.stage {
            Stage::Attack => {
                self.level += coeffs.attack_inc;
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = Stage::Decay;
                }
            }
            Stage::Decay => {
                self.level = coeffs.sustain + (self.level - coeffs.sustain) * coeffs.decay_coef;
                if (self.level - coeffs.sustain).abs() < 1e-4 {
                    self.level = coeffs.sustain;
                    self.stage = Stage::Sustain;
                }
            }
            Stage::Sustain => {
                // Glide to the sustain level so parameter changes don't click.
                self.level += (coeffs.sustain - self.level) * 0.005;
                // A sustain level of zero would otherwise decay into the
                // denormal range.
                if self.level.abs() < 1e-6 {
                    self.level = 0.0;
                }
            }
            Stage::Release => {
                self.level *= coeffs.release_coef;
                if self.level < 1e-5 {
                    self.level = 0.0;
                    self.stage = Stage::Off;
                }
            }
            Stage::Off => self.level = 0.0,
        }

        self.level
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attack_reaches_full_level() {
        let coeffs = EnvCoeffs::new(10.0, 300.0, 0.7, 400.0, 48_000.0);
        let mut env = Adsr::default();
        env.gate_on();

        // 10 ms at 48 kHz.
        for _ in 0..480 {
            env.next(&coeffs);
        }
        assert!((env.next(&coeffs) - 1.0).abs() < 1e-3);
    }

    #[test]
    fn release_eventually_finishes() {
        let coeffs = EnvCoeffs::new(1.0, 10.0, 1.0, 100.0, 48_000.0);
        let mut env = Adsr::default();
        env.gate_on();
        for _ in 0..480 {
            env.next(&coeffs);
        }

        env.gate_off();
        for _ in 0..48_000 {
            env.next(&coeffs);
            if env.is_finished() {
                break;
            }
        }
        assert!(env.is_finished());
    }

    #[test]
    fn attack_takes_the_configured_time() {
        let coeffs = EnvCoeffs::new(10.0, 300.0, 0.7, 400.0, 48_000.0);
        let mut env = Adsr::default();
        env.gate_on();

        // Halfway through a 10 ms attack the level should be around 0.5.
        for _ in 0..240 {
            env.next(&coeffs);
        }
        let level = env.next(&coeffs);
        assert!(
            (level - 0.5).abs() < 0.05,
            "the attack was at {level} after 5 ms"
        );
    }

    #[test]
    fn release_falls_off_quickly() {
        let coeffs = EnvCoeffs::new(1.0, 10.0, 1.0, 100.0, 48_000.0);
        let mut env = Adsr::default();
        env.gate_on();
        for _ in 0..480 {
            env.next(&coeffs);
        }
        assert!(env.next(&coeffs) > 0.9);

        env.gate_off();
        // The release coefficient puts the -60 dB point at the configured time.
        for _ in 0..(0.3 * 48_000.0) as usize {
            env.next(&coeffs);
        }
        let level = env.next(&coeffs);
        assert!(level < 0.01, "the release was at {level} after 300 ms");
    }
}
