
#[inline]
fn ln_cosh(x: f32) -> f32 {
    let absolute = x.abs();
    absolute + (1.0 + (-2.0 * absolute).exp()).ln() - std::f32::consts::LN_2
}

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
