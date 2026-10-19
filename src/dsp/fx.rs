//! The effect chain: a chorus, a ping-pong delay and a compact reverb.
//!
//! All the delay lines are allocated up front (in [`Fx::new`]) so that the
//! processing itself never allocates.

use std::f32::consts::PI;

/// The base delay of the chorus in seconds.
const CHORUS_BASE_DELAY: f32 = 0.012;
/// The maximum modulation depth of the chorus in seconds.
const CHORUS_MAX_DEPTH: f32 = 0.004;
/// The maximum delay time of the delay line in seconds.
const DELAY_MAX_TIME: f32 = 2.0;

/// The comb filter tunings of the reverb, in samples.
const COMB_TUNINGS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
/// The allpass filter tunings of the reverb, in samples.
const ALLPASS_TUNINGS: [usize; 4] = [556, 441, 341, 225];
/// The stereo spread of the reverb, in samples.
const STEREO_SPREAD: usize = 23;
/// The damping of the reverb's comb filters.
const REVERB_DAMPING: f32 = 0.25;
/// The input gain of the reverb.
const REVERB_INPUT_GAIN: f32 = 0.015;

/// The parameters of the effect chain.
#[derive(Debug, Clone, Copy)]
pub struct FxParams {
    pub chorus_mix: f32,
    pub chorus_rate: f32,
    pub chorus_depth: f32,
    pub delay_time_s: f32,
    pub delay_feedback: f32,
    pub delay_mix: f32,
    pub reverb_mix: f32,
    pub reverb_decay: f32,
}

impl Default for FxParams {
    fn default() -> Self {
        Self {
            chorus_mix: 0.0,
            chorus_rate: 0.8,
            chorus_depth: 0.5,
            delay_time_s: 0.35,
            delay_feedback: 0.3,
            delay_mix: 0.0,
            reverb_mix: 0.0,
            reverb_decay: 0.6,
        }
    }
}

/// A damped comb filter with a lowpass in its feedback path.
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
    fn process(&mut self, input: f32, feedback: f32, damping: f32) -> f32 {
        let output = self.buffer[self.index];
        self.filter_store = output * (1.0 - damping) + self.filter_store * damping;
        // Flush the denormal state, so that the comb doesn't drag the CPU down
        // with it once the signal has died away.
        if self.filter_store.abs() < 1e-20 {
            self.filter_store = 0.0;
        }
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

/// A Schroeder allpass filter.
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

/// The effect chain of the synthesizer.
pub struct Fx {
    sample_rate: f32,

    // The chorus.
    chorus_left: Vec<f32>,
    chorus_right: Vec<f32>,
    chorus_index: usize,
    chorus_phase: f32,

    // The ping-pong delay.
    delay_left: Vec<f32>,
    delay_right: Vec<f32>,
    delay_index: usize,

    // The reverb.
    combs: [Vec<Comb>; 2],
    allpasses: [Vec<Allpass>; 2],
}

impl Fx {
    /// Allocates all the delay lines. This is called from the plugin's
    /// `activate` function, never from the audio thread.
    pub fn new(sample_rate: f32) -> Self {
        let sample_rate = sample_rate.max(1.0);

        let chorus_len =
            (sample_rate * (CHORUS_BASE_DELAY + CHORUS_MAX_DEPTH + 0.002)).ceil() as usize + 2;
        let delay_len = (sample_rate * DELAY_MAX_TIME).ceil() as usize + 2;

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
            sample_rate,
            chorus_left: vec![0.0; chorus_len],
            chorus_right: vec![0.0; chorus_len],
            chorus_index: 0,
            chorus_phase: 0.0,
            delay_left: vec![0.0; delay_len],
            delay_right: vec![0.0; delay_len],
            delay_index: 0,
            combs,
            allpasses,
        }
    }

    /// Clears all the delay lines.
    pub fn reset(&mut self) {
        self.chorus_left.fill(0.0);
        self.chorus_right.fill(0.0);
        self.chorus_index = 0;
        self.chorus_phase = 0.0;
        self.delay_left.fill(0.0);
        self.delay_right.fill(0.0);
        self.delay_index = 0;
        for channel in 0..2 {
            for comb in &mut self.combs[channel] {
                comb.reset();
            }
            for allpass in &mut self.allpasses[channel] {
                allpass.reset();
            }
        }
    }

    /// Processes a single stereo sample.
    pub fn process(&mut self, left: f32, right: f32, params: &FxParams) -> (f32, f32) {
        // The effect chain is not initialized yet.
        if self.delay_left.is_empty() {
            return (left, right);
        }

        let (left, right) = self.process_chorus(left, right, params);
        let (left, right) = self.process_delay(left, right, params);
        self.process_reverb(left, right, params)
    }

    /// A two tap chorus with a quadrature LFO.
    fn process_chorus(&mut self, left: f32, right: f32, params: &FxParams) -> (f32, f32) {
        let sample_rate = self.sample_rate;

        // The LFO.
        self.chorus_phase += (params.chorus_rate / sample_rate).max(0.0);
        if self.chorus_phase >= 1.0 {
            self.chorus_phase -= self.chorus_phase.floor();
        }
        let phase = self.chorus_phase * PI * 2.0;
        let lfo_left = phase.sin();
        let lfo_right = phase.cos();

        let depth = params.chorus_depth.clamp(0.0, 1.0) * CHORUS_MAX_DEPTH;
        let delay_left = (CHORUS_BASE_DELAY + depth * lfo_left) * sample_rate;
        let delay_right = (CHORUS_BASE_DELAY + depth * lfo_right) * sample_rate;

        // Write the dry signal into the delay lines.
        let index = self.chorus_index;
        self.chorus_left[index] = left;
        self.chorus_right[index] = right;
        self.chorus_index = (index + 1) % self.chorus_left.len();

        let wet_left = read_interpolated(&self.chorus_left, index, delay_left);
        let wet_right = read_interpolated(&self.chorus_right, index, delay_right);

        let mix = params.chorus_mix.clamp(0.0, 1.0);
        (
            left * (1.0 - mix) + wet_left * mix,
            right * (1.0 - mix) + wet_right * mix,
        )
    }

    /// A ping-pong delay with a fractional read position.
    fn process_delay(&mut self, left: f32, right: f32, params: &FxParams) -> (f32, f32) {
        let len = self.delay_left.len();
        let max_delay = (len - 2) as f32;
        let delay = (params.delay_time_s * self.sample_rate).clamp(1.0, max_delay);
        let feedback = params.delay_feedback.clamp(0.0, 0.95);

        // Read the delayed signal before writing the new one.
        let wet_left = read_interpolated(&self.delay_left, self.delay_index, delay);
        let wet_right = read_interpolated(&self.delay_right, self.delay_index, delay);

        // Ping-pong: the left channel is fed by the right one and vice versa.
        self.delay_left[self.delay_index] = left + wet_right * feedback;
        self.delay_right[self.delay_index] = right + wet_left * feedback;
        self.delay_index = (self.delay_index + 1) % len;

        let mix = params.delay_mix.clamp(0.0, 1.0);
        (
            left * (1.0 - mix) + wet_left * mix,
            right * (1.0 - mix) + wet_right * mix,
        )
    }

    /// A compact freeverb style reverb: eight damped comb filters followed by
    /// four allpass filters per channel.
    fn process_reverb(&mut self, left: f32, right: f32, params: &FxParams) -> (f32, f32) {
        let feedback = 0.7 + 0.28 * params.reverb_decay.clamp(0.0, 1.0);
        let mix = params.reverb_mix.clamp(0.0, 1.0);

        let wet_left = self.reverb_channel(0, left * REVERB_INPUT_GAIN, feedback);
        let wet_right = self.reverb_channel(1, right * REVERB_INPUT_GAIN, feedback);

        (left + wet_left * mix, right + wet_right * mix)
    }

    /// Runs the reverb of a single channel.
    fn reverb_channel(&mut self, channel: usize, input: f32, feedback: f32) -> f32 {
        let mut output = 0.0;
        for comb in &mut self.combs[channel] {
            output += comb.process(input, feedback, REVERB_DAMPING);
        }
        for allpass in &mut self.allpasses[channel] {
            output = allpass.process(output);
        }
        // The allpass filters take a lot of energy out of the signal, so the
        // wet signal is boosted to make the reverb audible.
        output * 3.0
    }
}

/// Reads from a delay line with a fractional delay, using linear interpolation.
/// `write_index` is the position that was just written to.
#[inline]
fn read_interpolated(buffer: &[f32], write_index: usize, delay: f32) -> f32 {
    let len = buffer.len();
    let position = (write_index as f32 - delay).rem_euclid(len as f32);
    let index = position.floor() as usize % len;
    let fraction = position - position.floor();
    let next = (index + 1) % len;

    buffer[index] + (buffer[next] - buffer[index]) * fraction
}

impl Default for Fx {
    /// An effect chain without any allocated delay lines. Use [`Fx::new`] to
    /// create a usable one.
    fn default() -> Self {
        Self {
            sample_rate: 48_000.0,
            chorus_left: Vec::new(),
            chorus_right: Vec::new(),
            chorus_index: 0,
            chorus_phase: 0.0,
            delay_left: Vec::new(),
            delay_right: Vec::new(),
            delay_index: 0,
            combs: [Vec::new(), Vec::new()],
            allpasses: [Vec::new(), Vec::new()],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_impulse_produces_finite_output() {
        let mut fx = Fx::new(48_000.0);
        let params = FxParams {
            chorus_mix: 0.5,
            chorus_rate: 0.8,
            chorus_depth: 0.5,
            delay_time_s: 0.35,
            delay_feedback: 0.3,
            delay_mix: 0.5,
            reverb_mix: 0.5,
            reverb_decay: 0.6,
        };

        for sample in 0..48_000 {
            let input = if sample == 0 { 1.0 } else { 0.0 };
            let (left, right) = fx.process(input, input, &params);
            assert!(left.is_finite() && right.is_finite());
            assert!(left.abs() < 10.0 && right.abs() < 10.0);
        }
    }

    #[test]
    fn a_mix_of_zero_is_a_bypass() {
        let mut fx = Fx::new(48_000.0);
        let params = FxParams::default();

        for _ in 0..1_000 {
            let (left, right) = fx.process(0.5, -0.5, &params);
            assert!((left - 0.5).abs() < 1e-6);
            assert!((right + 0.5).abs() < 1e-6);
        }
    }

    #[test]
    fn the_delay_repeats_the_signal() {
        let mut fx = Fx::new(48_000.0);
        let params = FxParams {
            delay_mix: 1.0,
            delay_time_s: 0.1,
            delay_feedback: 0.0,
            ..FxParams::default()
        };

        // The impulse comes back after 100 ms.
        let mut first_repeat = None;
        for sample in 0..10_000 {
            let input = if sample == 0 { 1.0 } else { 0.0 };
            let (left, _) = fx.process(input, 0.0, &params);
            if sample > 0 && left.abs() > 0.5 {
                first_repeat = Some(sample);
                break;
            }
        }

        let first_repeat: i32 = first_repeat.expect("the delay did not repeat the signal");
        assert!(
            (first_repeat - 4_800).abs() < 8,
            "the repeat came at {first_repeat}"
        );
    }
}
