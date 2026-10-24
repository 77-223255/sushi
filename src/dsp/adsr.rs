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

/// Shapes a finished envelope value with an exponent from
/// [`curve_exponent`]. The result stays inside `0..1`; an exponent of exactly
/// one returns the value untouched, so the default curve is bit-exact.
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

/// A classic ADSR envelope running at audio rate.
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
    fn the_legacy_release_uses_the_absolute_threshold() {
        let coeffs = EnvCoeffs::new(1.0, 10.0, 1.0, 100.0, 48_000.0);

        let mut modern = Adsr::default();
        modern.gate_on();
        for _ in 0..480 {
            modern.next(&coeffs);
        }
        modern.gate_off();

        let mut legacy = Adsr::default();
        legacy.set_legacy(true);
        legacy.gate_on();
        for _ in 0..480 {
            legacy.next(&coeffs);
        }
        legacy.gate_off();

        // 150 ms is past the -60 dB point used by the modern threshold but
        // before the old absolute -100 dB threshold.
        for _ in 0..(0.15 * 48_000.0) as usize {
            modern.next(&coeffs);
            legacy.next(&coeffs);
        }

        assert!(modern.is_finished());
        assert!(!legacy.is_finished());
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

    #[test]
    fn a_zero_curve_is_the_old_linear_envelope() {
        let exponent = curve_exponent(0.0);
        assert_eq!(exponent, 1.0);

        for value in [0.0, 1e-7, 0.1, 0.25, 0.5, 0.75, 0.999_999, 1.0] {
            assert_eq!(
                shape_envelope(value, exponent).to_bits(),
                value.to_bits(),
                "the neutral curve changed {value}"
            );
        }
    }

    #[test]
    fn the_curve_shapes_the_final_value_without_leaving_its_range() {
        let shallow = shape_envelope(0.25, curve_exponent(-1.0));
        let linear = shape_envelope(0.25, curve_exponent(0.0));
        let steep = shape_envelope(0.25, curve_exponent(1.0));

        assert!(shallow > linear, "the shallow curve did not lift 0.25");
        assert!(steep < linear, "the steep curve did not lower 0.25");
        assert!((shallow - 0.5).abs() < 1e-6);
        assert!((steep - 0.0625).abs() < 1e-6);

        // The endpoints stay fixed and the result is stable over the whole
        // range of the curve.
        for curve in [-1.0, -0.5, 0.0, 0.5, 1.0] {
            let exponent = curve_exponent(curve);
            assert_eq!(shape_envelope(0.0, exponent), 0.0);
            assert!((shape_envelope(1.0, exponent) - 1.0).abs() < 1e-6);
            for step in 0..=100 {
                let value = step as f32 / 100.0;
                let shaped = shape_envelope(value, exponent);
                assert!(
                    (0.0..=1.0).contains(&shaped),
                    "the curve {curve} produced {shaped} from {value}"
                );
            }
        }
    }
}
