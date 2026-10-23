//! The original Freeverb-style reverb from the first Sushi release.
//!
//! It is kept as a separate engine for the legacy flavor. The topology and the
//! fixed 44.1 kHz tunings are intentionally unchanged; only the bypass path
//! now flushes the lines instead of freezing them.

use std::f32::consts::PI;

const PREDELAY: f32 = 0.015;
const LOWPASS: f32 = 8_000.0;
const COMB_TUNINGS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASS_TUNINGS: [usize; 4] = [556, 441, 341, 225];
const STEREO_SPREAD: usize = 23;
const DAMPING: f32 = 0.25;
const INPUT_GAIN: f32 = 0.015;
const BYPASS_MIX: f32 = 1e-4;

struct Comb {
    buffer: Vec<f32>,
    index: usize,
    filter_store: f32,
}

impl Comb {
    fn new(size: usize) -> Self {
        Self {
            buffer: vec![0.0; size],
            index: 0,
            filter_store: 0.0,
        }
    }

    #[inline]
    fn process(&mut self, input: f32, feedback: f32) -> f32 {
        let output = self.buffer[self.index];
        self.filter_store = output * (1.0 - DAMPING) + self.filter_store * DAMPING;
        self.filter_store = flush_denormal(self.filter_store);
        self.buffer[self.index] = input + self.filter_store * feedback;
        self.index = (self.index + 1) % self.buffer.len();
        output
    }

    fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.index = 0;
        self.filter_store = 0.0;
    }
}

struct Allpass {
    buffer: Vec<f32>,
    index: usize,
}

impl Allpass {
    fn new(size: usize) -> Self {
        Self {
            buffer: vec![0.0; size],
            index: 0,
        }
    }

    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        let buffered = self.buffer[self.index];
        let output = -input + buffered;
        self.buffer[self.index] = input + buffered * 0.5;
        self.index = (self.index + 1) % self.buffer.len();
        output
    }

    fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.index = 0;
    }
}

/// The legacy Freeverb-style reverb.
pub struct LegacyReverb {
    combs: [Vec<Comb>; 2],
    allpasses: [Vec<Allpass>; 2],
    predelay_left: Vec<f32>,
    predelay_right: Vec<f32>,
    predelay_index: usize,
    lowpass_left: f32,
    lowpass_right: f32,
    coefficient: f32,
}

impl Default for LegacyReverb {
    fn default() -> Self {
        Self {
            combs: [Vec::new(), Vec::new()],
            allpasses: [Vec::new(), Vec::new()],
            predelay_left: Vec::new(),
            predelay_right: Vec::new(),
            predelay_index: 0,
            lowpass_left: 0.0,
            lowpass_right: 0.0,
            coefficient: 0.0,
        }
    }
}

impl LegacyReverb {
    pub fn new(sample_rate: f32) -> Self {
        let sample_rate = sample_rate.max(1.0);
        let predelay_len = (sample_rate * PREDELAY).ceil() as usize + 2;
        let mut combs: [Vec<Comb>; 2] = [Vec::new(), Vec::new()];
        let mut allpasses: [Vec<Allpass>; 2] = [Vec::new(), Vec::new()];
        for channel in 0..2 {
            let spread = if channel == 0 { 0 } else { STEREO_SPREAD };
            combs[channel] = COMB_TUNINGS
                .iter()
                .map(|tuning| Comb::new(tuning + spread + 1))
                .collect();
            allpasses[channel] = ALLPASS_TUNINGS
                .iter()
                .map(|tuning| Allpass::new(tuning + spread + 1))
                .collect();
        }

        Self {
            combs,
            allpasses,
            predelay_left: vec![0.0; predelay_len],
            predelay_right: vec![0.0; predelay_len],
            predelay_index: 0,
            lowpass_left: 0.0,
            lowpass_right: 0.0,
            coefficient: lowpass_coefficient(LOWPASS, sample_rate),
        }
    }

    pub fn reset(&mut self) {
        for channel in 0..2 {
            for comb in &mut self.combs[channel] {
                comb.reset();
            }
            for allpass in &mut self.allpasses[channel] {
                allpass.reset();
            }
        }
        self.predelay_left.fill(0.0);
        self.predelay_right.fill(0.0);
        self.predelay_index = 0;
        self.lowpass_left = 0.0;
        self.lowpass_right = 0.0;
    }

    /// Returns the wet signal. `mix` only controls whether the feedback is
    /// muted while the effect is bypassed.
    pub fn process(&mut self, left: f32, right: f32, decay: f32, mix: f32) -> (f32, f32) {
        if self.predelay_left.is_empty() {
            return (0.0, 0.0);
        }

        let index = self.predelay_index;
        let delayed_left = self.predelay_left[index];
        let delayed_right = self.predelay_right[index];
        self.predelay_left[index] = left;
        self.predelay_right[index] = right;
        self.predelay_index = (index + 1) % self.predelay_left.len();

        self.lowpass_left += (delayed_left - self.lowpass_left) * self.coefficient;
        self.lowpass_right += (delayed_right - self.lowpass_right) * self.coefficient;
        self.lowpass_left = flush_denormal(self.lowpass_left);
        self.lowpass_right = flush_denormal(self.lowpass_right);

        let feedback = if mix <= BYPASS_MIX {
            0.0
        } else {
            0.7 + 0.28 * decay.clamp(0.0, 1.0)
        };

        let wet_left = self.channel(0, self.lowpass_left * INPUT_GAIN, feedback);
        let wet_right = self.channel(1, self.lowpass_right * INPUT_GAIN, feedback);
        (wet_left, wet_right)
    }

    fn channel(&mut self, channel: usize, input: f32, feedback: f32) -> f32 {
        let mut output = 0.0;
        for comb in &mut self.combs[channel] {
            output += comb.process(input, feedback);
        }
        for allpass in &mut self.allpasses[channel] {
            output = allpass.process(output);
        }
        output * 3.0
    }
}

#[inline]
fn lowpass_coefficient(frequency: f32, sample_rate: f32) -> f32 {
    1.0 - (-2.0 * PI * frequency / sample_rate.max(1.0)).exp()
}

#[inline]
fn flush_denormal(value: f32) -> f32 {
    if value.abs() < 1e-20 { 0.0 } else { value }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_legacy_reverb_stays_bounded() {
        let mut reverb = LegacyReverb::new(48_000.0);
        for sample in 0..48_000 {
            let input = if sample == 0 { 1.0 } else { 0.0 };
            let (left, right) = reverb.process(input, input, 0.8, 1.0);
            assert!(left.is_finite() && right.is_finite());
            assert!(left.abs() < 10.0 && right.abs() < 10.0);
        }
    }
}
