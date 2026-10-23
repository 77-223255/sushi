//! A modulated feedback delay network reverb.
//!
//! The reverb uses sixteen delay lines, a normalized Hadamard feedback matrix,
//! damping in every feedback path, slow delay modulation and a short early
//! reflection section. It is inspired by the classic Schroeder/Feedback Delay
//! Network topology and by the efficient FDN designs used by fundsp and other
//! open source synthesizers, but the implementation is self contained.

use std::f32::consts::PI;

/// The number of delay lines in the feedback matrix. A power of two so the
/// Hadamard transform is a single fast butterfly.
const LINE_COUNT: usize = 16;

/// The delay lengths at 44.1 kHz, in seconds. They are deliberately spread
/// over a wide range so the echo density builds up quickly.
const DELAYS: [f32; LINE_COUNT] = [
    0.0297, 0.0371, 0.0411, 0.0437, 0.0463, 0.0491, 0.0533, 0.0569, 0.0607, 0.0649, 0.0691, 0.0737,
    0.0779, 0.0821, 0.0863, 0.0907,
];

/// The length of the early reflection buffer, in seconds.
const EARLY_BUFFER: f32 = 0.1;
/// The early reflection taps, in seconds.
const EARLY_TAPS: [f32; 4] = [0.011, 0.017, 0.023, 0.031];
/// The gain of the early reflections.
const EARLY_GAIN: f32 = 0.35;
/// The maximum delay modulation depth, in seconds.
const MODULATION_DEPTH: f32 = 0.00035;
/// The slowest and the steps between the modulation rates, in Hz.
const MODULATION_RATE: f32 = 0.07;
const MODULATION_RATE_STEP: f32 = 0.0073;
/// The damping of the lowpass in every feedback path at the reference sample
/// rate. The feedback lowpass is a one-pole whose cutoff is fixed in Hz, so the
/// coefficient has to follow the sample rate; see [`damping_coefficient`].
const DAMPING: f32 = 0.22;
/// The sample rate the damping was tuned at. At this rate the coefficient is
/// exactly [`DAMPING`], which preserves the old 48 kHz damping coefficient bit
/// for bit. It is the only rate with a hard coded coefficient.
const DAMPING_REFERENCE_RATE: f32 = 48_000.0;
/// The gain of the input injection.
const INPUT_GAIN: f32 = 0.25;
/// The gain of the wet output.
const WET_GAIN: f32 = 0.45;
/// The frequency of the input lowpass, in Hz.
const INPUT_LOWPASS: f32 = 8_000.0;
/// The mix below which the reverb is treated as bypassed.
const BYPASS_MIX: f32 = 1e-4;
/// The number of samples between decay updates. A moving decay knob would
/// otherwise run sixteen `powf` calls per sample.
const DECAY_UPDATE_INTERVAL: u32 = 32;

/// A stereo feedback delay network reverb.
pub struct Reverb {
    sample_rate: f32,

    /// The short buffer used for the early reflections.
    early_left: Vec<f32>,
    early_right: Vec<f32>,
    early_index: usize,

    /// The delay lines of the feedback matrix.
    lines: [Vec<f32>; LINE_COUNT],
    indices: [usize; LINE_COUNT],
    base_delays: [f32; LINE_COUNT],
    damping_state: [f32; LINE_COUNT],

    /// Slow modulation of the delay lengths, in phase and Hz.
    modulation_phase: [f32; LINE_COUNT],
    modulation_rate: [f32; LINE_COUNT],

    /// The feedback gain of every line. It is derived from the requested
    /// decay time and the length of the line.
    feedback_gains: [f32; LINE_COUNT],
    decay: f32,
    /// The number of samples since the last decay update. Zero means the next
    /// sample updates the gains immediately.
    decay_counter: u32,

    input_lp_left: f32,
    input_lp_right: f32,
    input_lp_coefficient: f32,

    /// The coefficient of the one-pole lowpass in every feedback path. It is
    /// derived from a fixed cutoff in Hz, so the damping sounds the same at
    /// every sample rate. Computed in [`Reverb::new`], never per sample.
    damping_coefficient: f32,
}

impl Default for Reverb {
    /// An empty reverb. `Fx` never processes it before `new` has allocated
    /// the lines.
    fn default() -> Self {
        Self {
            sample_rate: 48_000.0,
            early_left: Vec::new(),
            early_right: Vec::new(),
            early_index: 0,
            lines: std::array::from_fn(|_| Vec::new()),
            indices: [0; LINE_COUNT],
            base_delays: [0.0; LINE_COUNT],
            damping_state: [0.0; LINE_COUNT],
            modulation_phase: [0.0; LINE_COUNT],
            modulation_rate: [0.0; LINE_COUNT],
            feedback_gains: [0.0; LINE_COUNT],
            decay: 0.6,
            decay_counter: 0,
            input_lp_left: 0.0,
            input_lp_right: 0.0,
            input_lp_coefficient: 0.0,
            damping_coefficient: DAMPING,
        }
    }
}

impl Reverb {
    /// Allocates the delay lines. Called from `Fx::new`, never from the audio
    /// thread.
    pub fn new(sample_rate: f32) -> Self {
        let sample_rate = sample_rate.max(1.0);
        let early_len = (sample_rate * EARLY_BUFFER).ceil() as usize + 4;
        let mut lines: [Vec<f32>; LINE_COUNT] = std::array::from_fn(|_| Vec::new());
        let mut base_delays = [0.0; LINE_COUNT];
        let indices = [0; LINE_COUNT];
        let mut modulation_phase = [0.0; LINE_COUNT];
        let mut modulation_rate = [0.0; LINE_COUNT];

        for index in 0..LINE_COUNT {
            base_delays[index] = DELAYS[index] * sample_rate;
            let length =
                (base_delays[index] + MODULATION_DEPTH * sample_rate + 4.0).ceil() as usize;
            lines[index] = vec![0.0; length.max(4)];
            modulation_phase[index] = index as f32 / LINE_COUNT as f32;
            modulation_rate[index] = MODULATION_RATE + index as f32 * MODULATION_RATE_STEP;
        }

        let mut reverb = Self {
            sample_rate,
            early_left: vec![0.0; early_len],
            early_right: vec![0.0; early_len],
            early_index: 0,
            lines,
            indices,
            base_delays,
            damping_state: [0.0; LINE_COUNT],
            modulation_phase,
            modulation_rate,
            feedback_gains: [0.0; LINE_COUNT],
            decay: 0.6,
            decay_counter: 0,
            input_lp_left: 0.0,
            input_lp_right: 0.0,
            input_lp_coefficient: lowpass_coefficient(INPUT_LOWPASS, sample_rate),
            damping_coefficient: damping_coefficient(sample_rate),
        };
        reverb.update_decay(0.6);
        reverb
    }

    pub fn reset(&mut self) {
        self.early_left.fill(0.0);
        self.early_right.fill(0.0);
        self.early_index = 0;
        for index in 0..LINE_COUNT {
            self.lines[index].fill(0.0);
            self.indices[index] = 0;
            self.damping_state[index] = 0.0;
            self.modulation_phase[index] = index as f32 / LINE_COUNT as f32;
        }
        self.input_lp_left = 0.0;
        self.input_lp_right = 0.0;
        self.decay_counter = 0;
    }

    /// Updates the per-line feedback gains from the decay parameter. The
    /// parameter is smoothed by the caller, so this is only recomputed when it
    /// actually moves.
    fn update_decay(&mut self, decay: f32) {
        let decay = decay.clamp(0.0, 1.0);
        if (decay - self.decay).abs() < 1e-5 && self.feedback_gains[0] != 0.0 {
            return;
        }

        self.decay = decay;
        // 0.35 s to 15 s to -60 dB.
        let time = 0.35 + 14.65 * decay * decay;
        for (index, delay) in DELAYS.iter().enumerate() {
            // A line with delay d is fed back 1 / d times per second. The
            // gain that reaches -60 dB after `time` is 10^(-3 d / time).
            let gain = 10f32.powf(-3.0 * delay / time);
            self.feedback_gains[index] = gain.clamp(0.0, 0.9995);
        }
    }

    /// Processes one stereo sample and returns the wet signal.
    ///
    /// `mix` is only used to decide whether the tail should keep ringing. When
    /// the reverb is bypassed the feedback is muted but the lines are still
    /// written with the dry signal, so a later enable cannot replay old audio.
    pub fn process(&mut self, left: f32, right: f32, decay: f32, mix: f32) -> (f32, f32) {
        if self.lines[0].is_empty() {
            return (0.0, 0.0);
        }

        // Update the feedback gains immediately on the first sample, then
        // throttle them so a moving decay knob stays cheap.
        if self.decay_counter == 0 {
            self.update_decay(decay);
        }
        self.decay_counter = (self.decay_counter + 1) % DECAY_UPDATE_INTERVAL;
        let active = mix > BYPASS_MIX;
        let sample_rate = self.sample_rate;

        // Read the early reflections before writing this sample.
        let early_index = self.early_index;
        let mut early_left = 0.0;
        let mut early_right = 0.0;
        for (tap_index, tap) in EARLY_TAPS.iter().enumerate() {
            let delay = tap * sample_rate;
            let left_tap = read_interpolated(&self.early_left, early_index, delay);
            let right_tap = read_interpolated(&self.early_right, early_index, delay);
            let gain = EARLY_GAIN * (1.0 - tap_index as f32 * 0.15);
            if tap_index % 2 == 0 {
                early_left += left_tap * gain;
                early_right += right_tap * gain;
            } else {
                // Cross-feed the later taps for a wider early stereo image.
                early_left += right_tap * gain;
                early_right += left_tap * gain;
            }
        }
        // The early taps are the same kind of low level state as the feedback
        // lines, so they are flushed before they reach the input lowpass.
        early_left = flush_denormal(early_left);
        early_right = flush_denormal(early_right);
        self.early_left[early_index] = flush_denormal(left);
        self.early_right[early_index] = flush_denormal(right);
        self.early_index = (early_index + 1) % self.early_left.len();

        // A gentle lowpass on the reverb input avoids exciting the network
        // with harsh high frequencies.
        self.input_lp_left += (left + early_left - self.input_lp_left) * self.input_lp_coefficient;
        self.input_lp_right +=
            (right + early_right - self.input_lp_right) * self.input_lp_coefficient;
        self.input_lp_left = flush_denormal(self.input_lp_left);
        self.input_lp_right = flush_denormal(self.input_lp_right);
        let input_left = self.input_lp_left * INPUT_GAIN;
        let input_right = self.input_lp_right * INPUT_GAIN;

        let mut feedback = [0.0f32; LINE_COUNT];
        let mut wet_left = 0.0;
        let mut wet_right = 0.0;

        for (index, feedback_slot) in feedback.iter_mut().enumerate() {
            let phase = self.modulation_phase[index];
            let triangle = if phase < 0.5 {
                4.0 * phase - 1.0
            } else {
                3.0 - 4.0 * phase
            };
            let line = &self.lines[index];
            let max_delay = (line.len() - 2) as f32;
            let delay = (self.base_delays[index] + MODULATION_DEPTH * sample_rate * triangle)
                .clamp(1.0, max_delay);
            let delayed = read_interpolated(line, self.indices[index], delay);

            self.damping_state[index] +=
                (delayed - self.damping_state[index]) * self.damping_coefficient;
            self.damping_state[index] = flush_denormal(self.damping_state[index]);
            let damped = self.damping_state[index];

            if index % 2 == 0 {
                wet_left += damped;
            } else {
                wet_right += damped;
            }

            *feedback_slot = if active {
                damped * self.feedback_gains[index]
            } else {
                0.0
            };

            let next_phase = phase + self.modulation_rate[index] / sample_rate;
            self.modulation_phase[index] = if next_phase >= 1.0 {
                next_phase - next_phase.floor()
            } else {
                next_phase
            };
        }

        // Normalized Hadamard matrix: the fast butterfly transform replaces a
        // 16 x 16 matrix multiplication.
        hadamard16(&mut feedback);

        for (index, feedback_value) in feedback.iter().enumerate() {
            let left_sign = if index % 2 == 0 { 1.0 } else { -1.0 };
            let right_sign = if (index / 2) % 2 == 0 { 1.0 } else { -1.0 };
            let injected = input_left * left_sign + input_right * right_sign;
            let line = &mut self.lines[index];
            let write_index = self.indices[index];
            // A long tail decays into the denormal range, which is slow on
            // some CPUs. Values this small are inaudible, so flush them.
            line[write_index] = flush_denormal(*feedback_value + injected);
            self.indices[index] = (write_index + 1) % line.len();
        }

        // The even and odd lines feed opposite sides of the stereo image.
        (wet_left * WET_GAIN, wet_right * WET_GAIN)
    }
}

/// The fast normalized Hadamard transform for 16 values.
#[inline]
fn hadamard16(values: &mut [f32; LINE_COUNT]) {
    let mut half = 1;
    while half < LINE_COUNT {
        let mut start = 0;
        while start < LINE_COUNT {
            for index in start..start + half {
                let a = values[index];
                let b = values[index + half];
                values[index] = a + b;
                values[index + half] = a - b;
            }
            start += half * 2;
        }
        half *= 2;
    }

    for value in values {
        *value *= 0.25;
    }
}

#[inline]
fn lowpass_coefficient(frequency: f32, sample_rate: f32) -> f32 {
    1.0 - (-2.0 * PI * frequency / sample_rate.max(1.0)).exp()
}

/// The cutoff, in Hz, that [`DAMPING`] stands for at
/// [`DAMPING_REFERENCE_RATE`]: `0.22 = 1 - exp(-2 pi f / 48000)`. It is
/// resolved once when the reverb is allocated, not per sample.
fn damping_cutoff() -> f32 {
    -((1.0 - DAMPING).ln()) * DAMPING_REFERENCE_RATE / (2.0 * PI)
}

/// The damping coefficient of the feedback lowpass for a sample rate.
///
/// The lowpass is tuned by its cutoff in Hz, so the decay and the damping of
/// the reverb stay consistent at 44.1, 88.2 and 96 kHz instead of getting
/// darker at higher rates. 48 kHz is special cased to return exactly the old
/// hard coded `0.22`, so the old damping coefficient is preserved bit for bit;
/// every other rate uses the coefficient of the same 1898 Hz cutoff (0.237 at
/// 44.1 kHz, 0.117 at 96 kHz).
fn damping_coefficient(sample_rate: f32) -> f32 {
    if sample_rate == DAMPING_REFERENCE_RATE {
        DAMPING
    } else {
        lowpass_coefficient(damping_cutoff(), sample_rate)
    }
}

#[inline]
fn flush_denormal(value: f32) -> f32 {
    if value.abs() < 1e-20 { 0.0 } else { value }
}

/// Reads from a delay line with a fractional delay, using linear interpolation.
#[inline]
fn read_interpolated(buffer: &[f32], write_index: usize, delay: f32) -> f32 {
    let len = buffer.len();
    let position = (write_index as f32 - delay).rem_euclid(len as f32);
    let index = position.floor() as usize % len;
    let fraction = position - position.floor();
    let next = (index + 1) % len;

    buffer[index] + (buffer[next] - buffer[index]) * fraction
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_impulse_stays_bounded() {
        let mut reverb = Reverb::new(48_000.0);
        let mut peak = 0.0f32;

        for sample in 0..48_000 {
            let input = if sample == 0 { 1.0 } else { 0.0 };
            let (left, right) = reverb.process(input, input, 0.8, 1.0);
            assert!(left.is_finite() && right.is_finite());
            peak = peak.max(left.abs()).max(right.abs());
        }

        assert!(peak < 10.0, "the reverb ran away ({peak})");
        assert!(peak > 0.001, "the reverb produced silence");
    }

    #[test]
    fn the_delays_scale_with_the_sample_rate() {
        let at_48 = Reverb::new(48_000.0);
        let at_96 = Reverb::new(96_000.0);

        let ratio = at_96.lines[0].len() as f32 / at_48.lines[0].len() as f32;
        assert!(
            (ratio - 2.0).abs() < 0.01,
            "the reverb did not scale with the sample rate: {ratio}"
        );
    }

    #[test]
    fn the_tail_decays() {
        let mut reverb = Reverb::new(48_000.0);
        let mut early_energy = 0.0;
        let mut late_energy = 0.0;

        for sample in 0..96_000 {
            let input = if sample == 0 { 1.0 } else { 0.0 };
            let (left, _) = reverb.process(input, input, 0.6, 1.0);
            if (1_000..20_000).contains(&sample) {
                early_energy += left * left;
            }
            if (70_000..90_000).contains(&sample) {
                late_energy += left * left;
            }
        }

        assert!(
            late_energy < early_energy * 0.1,
            "the tail did not decay: {late_energy} vs {early_energy}"
        );
    }

    #[test]
    fn the_damping_coefficient_matches_the_old_value_at_48_khz() {
        // The old implementation used this constant at every sample rate.
        // 48 kHz must keep the coefficient bit for bit, so the damping of the
        // presets does not change.
        assert_eq!(damping_coefficient(48_000.0).to_bits(), 0.22f32.to_bits());
        let at_48 = Reverb::new(48_000.0);
        assert_eq!(at_48.damping_coefficient.to_bits(), 0.22f32.to_bits());
    }

    #[test]
    fn the_decay_update_is_throttled_but_applied_immediately() {
        let mut reverb = Reverb::new(48_000.0);
        let mut reference = Reverb::new(48_000.0);
        reference.update_decay(0.2);

        // The first sample applies the new decay right away.
        reverb.process(0.0, 0.0, 0.2, 1.0);
        assert_eq!(reverb.feedback_gains, reference.feedback_gains);

        // The next samples in the window keep the same gains.
        for _ in 0..DECAY_UPDATE_INTERVAL - 1 {
            reverb.process(0.0, 0.0, 0.9, 1.0);
        }
        assert_eq!(reverb.feedback_gains, reference.feedback_gains);

        // The first sample of the next window picks the new decay up.
        reverb.process(0.0, 0.0, 0.9, 1.0);
        assert_ne!(reverb.feedback_gains, reference.feedback_gains);
    }

    #[test]
    fn the_damping_follows_the_sample_rate() {
        let at_44 = damping_coefficient(44_100.0);
        let at_88 = damping_coefficient(88_200.0);
        let at_96 = damping_coefficient(96_000.0);

        for coefficient in [at_44, at_88, at_96] {
            assert!(coefficient.is_finite(), "the damping was not finite");
            assert!(coefficient > 0.0 && coefficient < 1.0, "{coefficient}");
            assert_ne!(coefficient.to_bits(), 0.22f32.to_bits());
        }

        // A one-pole with a fixed cutoff in Hz damps less per sample at a
        // higher rate, because a sample covers less time.
        assert!(at_44 > at_88 && at_88 > at_96, "{at_44} {at_88} {at_96}");
        assert!(at_44 < 0.3, "the 44.1 kHz damping is too strong: {at_44}");

        // All three still damp at the same cutoff in Hz as 48 kHz does.
        let target = damping_cutoff();
        assert!((target - 1_900.0).abs() < 20.0, "the cutoff is {target}");
        for (coefficient, sample_rate) in [
            (0.22, 48_000.0),
            (at_44, 44_100.0),
            (at_88, 88_200.0),
            (at_96, 96_000.0),
        ] {
            let cutoff = -((1.0 - coefficient).ln()) * sample_rate / (2.0 * PI);
            assert!(
                (cutoff - target).abs() < 0.01,
                "{sample_rate} Hz damps at {cutoff} Hz instead of {target} Hz"
            );
        }
    }

    #[test]
    fn a_bypassed_reverb_is_flushed() {
        let mut reverb = Reverb::new(48_000.0);

        // Excite the network while it is active.
        for sample in 0..2_000 {
            let input = if sample == 0 { 1.0 } else { 0.0 };
            reverb.process(input, input, 1.0, 1.0);
        }

        // Bypass it for longer than the longest delay line.
        for _ in 0..20_000 {
            reverb.process(0.0, 0.0, 1.0, 0.0);
        }

        let mut peak = 0.0f32;
        for _ in 0..10_000 {
            let (left, right) = reverb.process(0.0, 0.0, 1.0, 1.0);
            peak = peak.max(left.abs()).max(right.abs());
        }
        assert!(peak < 1e-4, "the reverb replayed stale audio ({peak})");
    }
}
