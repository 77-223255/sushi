//! First order antiderivative anti-aliasing (ADAA) for the tanh stages.
//!
//! A memoryless `tanh(x)` generates a lot of aliasing when it is driven hard.
//! The first order ADAA form replaces the pointwise value with the average of
//! the nonlinearity over the interval between the previous and the current
//! input sample:
//!
//! `y[n] = (F(x[n]) - F(x[n - 1])) / (x[n] - x[n - 1])`
//!
//! where `F` is an antiderivative of the nonlinearity. The extra `ln`/`exp`
//! pair per sample is much cheaper than running the whole voice at 2x or 4x
//! the sample rate, and it removes most of the aliasing from the saturators.

/// The natural logarithm of `cosh(x)`, written in a form that stays finite for
/// large `x` without overflowing `exp`.
#[inline]
fn ln_cosh(x: f32) -> f32 {
    let absolute = x.abs();
    absolute + (1.0 + (-2.0 * absolute).exp()).ln() - std::f32::consts::LN_2
}

/// A first order ADAA tanh.
#[derive(Debug, Clone, Copy, Default)]
pub struct AdaaTanh {
    previous_input: f32,
    previous_antiderivative: f32,
}

impl AdaaTanh {
    pub fn reset(&mut self) {
        self.previous_input = 0.0;
        self.previous_antiderivative = 0.0;
    }

    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        let antiderivative = ln_cosh(input);
        let difference = input - self.previous_input;
        let output = if difference.abs() < 1e-5 {
            // The two points are too close for the quotient to be accurate.
            // The midpoint value is the correct limit.
            ((input + self.previous_input) * 0.5).tanh()
        } else {
            (antiderivative - self.previous_antiderivative) / difference
        };

        self.previous_input = input;
        self.previous_antiderivative = antiderivative;
        output
    }
}

/// A first order ADAA tanh with a different gain for the negative half of the
/// wave. It is used by the grit character, which is intentionally asymmetric.
#[derive(Debug, Clone, Copy)]
pub struct AdaaTanhAsym {
    previous_input: f32,
    previous_antiderivative: f32,
    negative_gain: f32,
}

impl Default for AdaaTanhAsym {
    fn default() -> Self {
        Self {
            previous_input: 0.0,
            previous_antiderivative: 0.0,
            negative_gain: 1.0,
        }
    }
}

impl AdaaTanhAsym {
    pub fn set_negative_gain(&mut self, negative_gain: f32) {
        let negative_gain = negative_gain.max(1e-4);
        if (negative_gain - self.negative_gain).abs() > 1e-6 {
            self.negative_gain = negative_gain;
            self.previous_input = 0.0;
            self.previous_antiderivative = 0.0;
        }
    }

    #[inline]
    fn antiderivative(&self, input: f32) -> f32 {
        if input >= 0.0 {
            ln_cosh(input)
        } else {
            ln_cosh(input * self.negative_gain) / self.negative_gain
        }
    }

    #[inline]
    fn nonlinearity(&self, input: f32) -> f32 {
        if input >= 0.0 {
            input.tanh()
        } else {
            (input * self.negative_gain).tanh()
        }
    }

    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        let antiderivative = self.antiderivative(input);
        let difference = input - self.previous_input;
        let output = if difference.abs() < 1e-5 {
            (self.nonlinearity(input) + self.nonlinearity(self.previous_input)) * 0.5
        } else {
            (antiderivative - self.previous_antiderivative) / difference
        };

        self.previous_input = input;
        self.previous_antiderivative = antiderivative;
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_adaa_tanh_stays_bounded() {
        let mut adaa = AdaaTanh::default();
        for sample in 0..48_000 {
            let input = (sample as f32 * 440.0 / 48_000.0 * std::f32::consts::TAU).sin() * 8.0;
            let output = adaa.process(input);
            assert!(output.is_finite());
            assert!(output.abs() <= 1.01, "the ADAA output was {output}");
        }
    }

    #[test]
    fn the_asymmetric_adaa_tanh_stays_bounded() {
        let mut adaa = AdaaTanhAsym::default();
        adaa.set_negative_gain(0.82);
        for sample in 0..48_000 {
            let input = (sample as f32 * 330.0 / 48_000.0 * std::f32::consts::TAU).sin() * 8.0;
            let output = adaa.process(input);
            assert!(output.is_finite());
            assert!(output.abs() <= 1.01, "the ADAA output was {output}");
        }
    }

    #[test]
    fn silence_stays_silent() {
        let mut adaa = AdaaTanh::default();
        for _ in 0..1_000 {
            assert_eq!(adaa.process(0.0), 0.0);
        }
    }
}
