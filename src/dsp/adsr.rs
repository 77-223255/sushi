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

/// Converts the bipolar `-1..1` envelope curve parameter into the exponent of
/// the final envelope shaper.
///
/// Zero is exactly one, which makes the shaper a plain pass-through. A
/// negative curve gives a shallower exponent (more of the envelope is spent
/// near the top), a positive curve gives a steeper one (the envelope sits
/// lower for longer). The exponent is computed once per block, not once per
/// sample.
pub fn curve_exponent(curve: f32) -> f32 {
    if curve == 0.0 {
        1.0
    } else {
        2f32.powf(curve.clamp(-1.0, 1.0))
    }
}

#[inline]
pub fn shape_envelope(value: f32, exponent: f32) -> f32 {
    if exponent == 1.0 {
        value
    } else {
        value.clamp(0.0, 1.0).powf(exponent)
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

#[derive(Debug, Clone)]
pub struct Adsr {
    stage: Stage,
    level: f32,
    /// The level at the start of the decay stage. It makes the decay time
    /// refer to the actual distance to the sustain level.
    decay_start: f32,
    /// The level at the start of the release stage. It makes the release time
    /// refer to the actual signal level instead of an absolute threshold.
    release_start: f32,
    /// The original engine used absolute thresholds. The legacy flavor keeps
    /// those thresholds so its envelopes match the first release.
    legacy: bool,
}

impl Default for Adsr {
    fn default() -> Self {
        Self {
            stage: Stage::Off,
            level: 0.0,
            decay_start: 0.0,
            release_start: 0.0,
            legacy: false,
        }
    }
}

impl Adsr {
    /// Selects the threshold behavior of the first release.
    pub fn set_legacy(&mut self, legacy: bool) {
        self.legacy = legacy;
    }

    /// Starts (or retriggers) the envelope. Retriggering keeps the current level so
    /// legato lines don't click.
    pub fn gate_on(&mut self) {
        if self.stage == Stage::Off {
            self.level = 0.0;
        }
        self.decay_start = self.level;
        self.release_start = 0.0;
        self.stage = Stage::Attack;
    }

    /// Enters the release stage. Safe to call multiple times.
    pub fn gate_off(&mut self) {
        if self.stage != Stage::Off && self.stage != Stage::Release {
            self.release_start = self.level.max(1e-6);
            self.stage = Stage::Release;
        }
    }

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
                    self.decay_start = 1.0;
                    self.stage = Stage::Decay;
                }
            }
            Stage::Decay => {
                self.level = coeffs.sustain + (self.level - coeffs.sustain) * coeffs.decay_coef;
                let threshold = if self.legacy {
                    1e-4
                } else {
                    (self.decay_start - coeffs.sustain).abs().max(1e-6) * 1e-3
                };
                if (self.level - coeffs.sustain).abs() < threshold {
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
                let threshold = if self.legacy {
                    1e-5
                } else {
                    self.release_start * 1e-3
                };
                if self.level < threshold {
                    self.level = 0.0;
                    self.stage = Stage::Off;
                }
            }
            Stage::Off => self.level = 0.0,
        }

        self.level
    }
}

