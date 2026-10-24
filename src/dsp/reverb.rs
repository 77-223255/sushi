//! A modulated feedback delay network reverb.
//!
//! The reverb uses four short allpass diffusers per channel, sixteen delay
//! lines, a normalized Hadamard feedback matrix, damping in every feedback
//! path, a global DC highpass on the reverb input and a short early reflection
//! section. The diffusers remove the metallic gaps of a sparse input, the early
//! taps reach the wet output directly so the first wet signal arrives after a
//! few milliseconds, and the damping follows the decay time: longer tails are a
//! little darker, shorter tails a little brighter. It is inspired by the
//! classic Schroeder/Feedback Delay Network topology and by the efficient FDN
//! designs used by fundsp and other open source synthesizers, but the
//! implementation is self contained.

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
/// The early reflection taps, in seconds. The first one is deliberately
/// shorter than the shortest FDN delay (29.7 ms), so the wet signal starts
/// about 5 ms after the input instead of waiting for the feedback matrix.
const EARLY_TAPS: [f32; 4] = [0.0053, 0.0089, 0.0137, 0.0211];
/// The gain of the early reflections that are injected into the network.
const EARLY_GAIN: f32 = 0.35;
/// The gain of the early reflections that are sent straight to the wet
/// output. The final wet gain is applied afterwards as well, so this stays a
/// subtle first reflection rather than a second dry signal.
const EARLY_OUTPUT_GAIN: f32 = 0.35;
/// The number of Schroeder allpasses in front of each FDN input channel.
const DIFFUSION_STAGES: usize = 4;
/// The diffusion delays, in seconds. The left and right sets are slightly
/// different, so the two input diffusers do not produce the same pattern.
/// The lengths are short and prime-ish, which keeps the diffused onset a few
/// milliseconds wide without smearing it.
const DIFFUSION_DELAYS: [[f32; DIFFUSION_STAGES]; 2] = [
    [0.0029, 0.0041, 0.0059, 0.0083],
    [0.0031, 0.0043, 0.0061, 0.0089],
];
/// The feedback coefficient of the diffusion allpasses. It sets how much the
/// short delays are mixed into the input; the allpasses themselves have unity
/// magnitude response, so they cannot add gain at any frequency.
const DIFFUSION_GAIN: f32 = 0.55;
/// The maximum delay modulation depth, in seconds.
const MODULATION_DEPTH: f32 = 0.00035;
/// The slowest and the steps between the modulation rates, in Hz.
const MODULATION_RATE: f32 = 0.07;
const MODULATION_RATE_STEP: f32 = 0.0073;
/// The damping of the lowpass in every feedback path at the reference sample
/// rate and the reference decay. The feedback lowpass is a one-pole whose
/// cutoff is fixed in Hz, so the coefficient has to follow the sample rate;
/// see [`damping_coefficient`].
const DAMPING: f32 = 0.22;
/// The sample rate the damping was tuned at. At this rate and the reference
/// decay the coefficient is exactly [`DAMPING`], which preserves the old
/// 48 kHz damping coefficient bit for bit. It is the only rate with a hard
/// coded coefficient.
const DAMPING_REFERENCE_RATE: f32 = 48_000.0;
/// The decay at which the damping is exactly the old 48 kHz value. This is
/// the default decay, so the untouched modern sound stays calibrated.
const DAMPING_REFERENCE_DECAY: f32 = 0.6;
/// How far the damping cutoff moves over the decay range, in octaves. The law
/// is `cutoff = 1898 Hz * 2^(0.5 * (0.6 - decay))`: decay 0.6 is the old
/// cutoff, decay 1.0 lowers it by 0.2 octaves (about 1650 Hz) for a slightly
/// darker, denser long tail, and decay 0.0 raises it by 0.3 octaves (about
/// 2330 Hz) so short tails keep some air.
const DAMPING_DECAY_OCTAVES: f32 = 0.5;
/// The cutoff of the DC blockers, in Hz. They sit on the reverb input, before
/// the early reflections and the FDN, never on the dry path.
const DC_BLOCKER_FREQUENCY: f32 = 20.0;
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

/// A Schroeder allpass with a fixed integer delay and a unity magnitude
/// response. It is the small, cheap diffuser in front of the FDN.
struct Allpass {
    buffer: Vec<f32>,
    index: usize,
    gain: f32,
}

impl Default for Allpass {
    fn default() -> Self {
        Self {
            buffer: Vec::new(),
            index: 0,
            gain: 0.0,
        }
    }
}

impl Allpass {
    fn new(delay_seconds: f32, sample_rate: f32, gain: f32) -> Self {
        let length = (delay_seconds * sample_rate).round() as usize;
        Self {
            buffer: vec![0.0; length.max(1)],
            index: 0,
            gain: gain.clamp(0.0, 0.95),
        }
    }

    /// `y[n] = -g x[n] + x[n-D] + g y[n-D]`, which is a true allpass.
    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        let delayed = self.buffer[self.index];
        let output = flush_denormal(delayed - self.gain * input);
        self.buffer[self.index] = flush_denormal(input + self.gain * output);
        self.index = if self.index + 1 == self.buffer.len() {
            0
        } else {
            self.index + 1
        };
        output
    }

    fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.index = 0;
    }
}

/// A one-pole DC blocker: `y = x - lowpass(x)`.
///
/// The global highpass on the reverb input uses one per channel. It is placed
/// before the early reflections and the FDN, so a constant input cannot reach
/// the delay lines and build up an offset in the network. Its pole is set by
/// the cutoff in [`DC_BLOCKER_FREQUENCY`]. The dry path never passes through
/// it.
#[derive(Clone, Copy)]
struct DcBlocker {
    state: f32,
    coefficient: f32,
}

impl Default for DcBlocker {
    fn default() -> Self {
        Self {
            state: 0.0,
            coefficient: 0.0,
        }
    }
}

impl DcBlocker {
    fn new(coefficient: f32) -> Self {
        Self {
            coefficient: coefficient.clamp(0.0, 1.0),
            ..Self::default()
        }
    }

    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        self.state = flush_denormal(self.state + (input - self.state) * self.coefficient);
        flush_denormal(input - self.state)
    }

    fn reset(&mut self) {
        self.state = 0.0;
    }
}

/// A stereo feedback delay network reverb.
pub struct Reverb {
    sample_rate: f32,

    /// The short buffer used for the early reflections.
    early_left: Vec<f32>,
    early_right: Vec<f32>,
    early_index: usize,

    /// The short allpass diffusers in front of the feedback matrix.
    diffusion_left: [Allpass; DIFFUSION_STAGES],
    diffusion_right: [Allpass; DIFFUSION_STAGES],

    /// The delay lines of the feedback matrix.
    lines: [Vec<f32>; LINE_COUNT],
    indices: [usize; LINE_COUNT],
    base_delays: [f32; LINE_COUNT],
    damping_state: [f32; LINE_COUNT],
    /// Global DC blockers on the two reverb input channels. They remove any
    /// offset before the early buffer and the feedback matrix, so a constant
    /// input cannot accumulate in the loop.
    input_dc_left: DcBlocker,
    input_dc_right: DcBlocker,

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
    /// derived from a cutoff in Hz that follows the sample rate and the decay,
    /// so the damping sounds the same at every rate and longer tails are
    /// darker. Computed in [`Reverb::update_decay`], never per sample.
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
            diffusion_left: std::array::from_fn(|_| Allpass::default()),
            diffusion_right: std::array::from_fn(|_| Allpass::default()),
            lines: std::array::from_fn(|_| Vec::new()),
            indices: [0; LINE_COUNT],
            base_delays: [0.0; LINE_COUNT],
            damping_state: [0.0; LINE_COUNT],
            input_dc_left: DcBlocker::default(),
            input_dc_right: DcBlocker::default(),
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

        let mut diffusion_left: [Allpass; DIFFUSION_STAGES] =
            std::array::from_fn(|_| Allpass::default());
        let mut diffusion_right: [Allpass; DIFFUSION_STAGES] =
            std::array::from_fn(|_| Allpass::default());
        for stage in 0..DIFFUSION_STAGES {
            diffusion_left[stage] =
                Allpass::new(DIFFUSION_DELAYS[0][stage], sample_rate, DIFFUSION_GAIN);
            diffusion_right[stage] =
                Allpass::new(DIFFUSION_DELAYS[1][stage], sample_rate, DIFFUSION_GAIN);
        }
        let dc_coefficient = dc_blocker_coefficient(sample_rate);

        let mut reverb = Self {
            sample_rate,
            early_left: vec![0.0; early_len],
            early_right: vec![0.0; early_len],
            early_index: 0,
            diffusion_left,
            diffusion_right,
            lines,
            indices,
            base_delays,
            damping_state: [0.0; LINE_COUNT],
            input_dc_left: DcBlocker::new(dc_coefficient),
            input_dc_right: DcBlocker::new(dc_coefficient),
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
        for diffuser in &mut self.diffusion_left {
            diffuser.reset();
        }
        for diffuser in &mut self.diffusion_right {
            diffuser.reset();
        }
        for index in 0..LINE_COUNT {
            self.lines[index].fill(0.0);
            self.indices[index] = 0;
            self.damping_state[index] = 0.0;
            self.modulation_phase[index] = index as f32 / LINE_COUNT as f32;
        }
        self.input_dc_left.reset();
        self.input_dc_right.reset();
        self.input_lp_left = 0.0;
        self.input_lp_right = 0.0;
        self.decay_counter = 0;
    }

    /// Updates the per-line feedback gains and the damping from the decay
    /// parameter. The parameter is smoothed by the caller, so this is only
    /// recomputed when it actually moves.
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

        // Longer decays are damped a little more (a darker, denser tail),
        // while short decays keep more high frequency.
        self.damping_coefficient = damping_coefficient_for_decay(decay, self.sample_rate);
    }

    /// Processes one stereo sample and returns the wet signal.
    ///
    /// `mix` is only used to decide whether the tail should keep ringing. When
    /// the reverb is bypassed the feedback is muted but the lines are still
    /// written with the highpassed input, so a later enable cannot replay old
    /// audio.
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

        // The global highpass sits before the early buffer and the FDN, so no
        // DC reaches either the wet early field or the feedback loop. Fx keeps
        // its dry signal separate, so the dry path is not filtered here.
        let left = self.input_dc_left.process(left);
        let right = self.input_dc_right.process(right);

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

        // Four short allpasses per channel turn the input into a dense burst
        // before it reaches the sparse FDN lines. They have unity magnitude
        // response, so the input level is unchanged.
        let mut diffused_left = self.input_lp_left;
        let mut diffused_right = self.input_lp_right;
        for stage in 0..DIFFUSION_STAGES {
            diffused_left = self.diffusion_left[stage].process(diffused_left);
            diffused_right = self.diffusion_right[stage].process(diffused_right);
        }
        let input_left = diffused_left * INPUT_GAIN;
        let input_right = diffused_right * INPUT_GAIN;

        let mut feedback = [0.0f32; LINE_COUNT];
        // The early reflections go to the wet output as well as into the FDN
        // input, so the first audible wet signal does not wait for the
        // shortest feedback line.
        let mut wet_left = early_left * EARLY_OUTPUT_GAIN;
        let mut wet_right = early_right * EARLY_OUTPUT_GAIN;

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

        // The even and odd lines feed opposite sides of the stereo image. The
        // global input highpass already keeps DC out of the loop and the early
        // field, so the wet gain is the last step.
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

/// The damping cutoff for a decay value.
///
/// The cutoff moves by [`DAMPING_DECAY_OCTAVES`] over the full decay range:
/// `cutoff = damping_cutoff() * 2^(0.5 * (0.6 - decay))`. The reference
/// decay 0.6 returns the old cutoff, longer decays are darker and shorter
/// decays are brighter. The law is intentionally gentle: the full range is
/// only half an octave.
fn damping_cutoff_for_decay(decay: f32) -> f32 {
    let octaves = DAMPING_DECAY_OCTAVES * (DAMPING_REFERENCE_DECAY - decay.clamp(0.0, 1.0));
    damping_cutoff() * 2.0f32.powf(octaves)
}

/// The damping coefficient of the feedback lowpass for a decay and sample
/// rate.
///
/// The lowpass is tuned by its cutoff in Hz, so the decay and the damping of
/// the reverb stay consistent at 44.1, 88.2 and 96 kHz instead of getting
/// darker at higher rates. At 48 kHz and the reference decay the coefficient
/// is exactly the old hard coded `0.22`, so that calibration is preserved bit
/// for bit; every other rate or decay uses the coefficient of the same
/// sample-rate-correct cutoff (for example 0.237 at 44.1 kHz, 0.117 at
/// 96 kHz, both at decay 0.6).
fn damping_coefficient_for_decay(decay: f32, sample_rate: f32) -> f32 {
    let decay = decay.clamp(0.0, 1.0);
    if sample_rate == DAMPING_REFERENCE_RATE && (decay - DAMPING_REFERENCE_DECAY).abs() < 1e-6 {
        DAMPING
    } else {
        lowpass_coefficient(damping_cutoff_for_decay(decay), sample_rate)
    }
}

/// The damping coefficient at the reference decay. Kept as a small helper so
/// the sample-rate tests can check the old calibration directly.
fn damping_coefficient(sample_rate: f32) -> f32 {
    damping_coefficient_for_decay(DAMPING_REFERENCE_DECAY, sample_rate)
}

/// The pole of the one-pole DC blocker for a sample rate.
fn dc_blocker_coefficient(sample_rate: f32) -> f32 {
    1.0 - (-2.0 * PI * DC_BLOCKER_FREQUENCY / sample_rate.max(1.0)).exp()
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
        // 48 kHz at the reference decay must keep the coefficient bit for bit,
        // so the damping of the untouched presets does not change.
        assert_eq!(damping_coefficient(48_000.0).to_bits(), 0.22f32.to_bits());
        let at_48 = Reverb::new(48_000.0);
        assert_eq!(at_48.damping_coefficient.to_bits(), 0.22f32.to_bits());
    }

    #[test]
    fn the_48_khz_fdn_geometry_is_unchanged() {
        let reverb = Reverb::new(48_000.0);
        let time = 0.35 + 14.65 * DAMPING_REFERENCE_DECAY * DAMPING_REFERENCE_DECAY;

        for (index, delay) in DELAYS.iter().enumerate() {
            // The old delay lengths and the old feedback gains are untouched.
            assert_eq!(
                reverb.base_delays[index].to_bits(),
                (delay * 48_000.0).to_bits()
            );
            let expected = 10f32.powf(-3.0 * delay / time).clamp(0.0, 0.9995);
            assert_eq!(reverb.feedback_gains[index].to_bits(), expected.to_bits());
            assert_eq!(
                reverb.modulation_rate[index].to_bits(),
                (MODULATION_RATE + index as f32 * MODULATION_RATE_STEP).to_bits()
            );
        }
    }

    #[test]
    fn longer_decays_are_darker_and_the_damping_law_matches_the_documentation() {
        let short = damping_coefficient_for_decay(0.0, 48_000.0);
        let reference = damping_coefficient_for_decay(DAMPING_REFERENCE_DECAY, 48_000.0);
        let long = damping_coefficient_for_decay(1.0, 48_000.0);

        assert_eq!(reference.to_bits(), 0.22f32.to_bits());
        assert!(
            long < reference,
            "long tails must be darker: {long} {reference}"
        );
        assert!(
            reference < short,
            "short tails must be brighter: {reference} {short}"
        );

        // The law is deliberately gentle: half an octave over the whole range.
        let long_cutoff = damping_cutoff_for_decay(1.0);
        let reference_cutoff = damping_cutoff_for_decay(DAMPING_REFERENCE_DECAY);
        let short_cutoff = damping_cutoff_for_decay(0.0);
        assert!((reference_cutoff - damping_cutoff()).abs() < 1e-3);
        assert!((long_cutoff / reference_cutoff - 2.0f32.powf(-0.2)).abs() < 1e-5);
        assert!((short_cutoff / reference_cutoff - 2.0f32.powf(0.3)).abs() < 1e-5);
        assert!(long_cutoff > 1_500.0 && short_cutoff < 2_500.0);
    }

    #[test]
    fn the_decay_update_is_throttled_but_applied_immediately() {
        let mut reverb = Reverb::new(48_000.0);
        let mut reference = Reverb::new(48_000.0);
        reference.update_decay(0.2);

        // The first sample applies the new decay right away.
        reverb.process(0.0, 0.0, 0.2, 1.0);
        assert_eq!(reverb.feedback_gains, reference.feedback_gains);
        assert_eq!(
            reverb.damping_coefficient.to_bits(),
            reference.damping_coefficient.to_bits()
        );

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

    #[test]
    fn the_early_reflections_arrive_before_ten_milliseconds() {
        let sample_rate = 48_000.0;
        let mut reverb = Reverb::new(sample_rate);
        let mut onset = None;
        let mut onset_level = 0.0f32;

        for sample in 0..(sample_rate * 0.05) as usize {
            let input = if sample == 0 { 1.0 } else { 0.0 };
            let (left, right) = reverb.process(input, input, 0.6, 1.0);
            let level = left.abs().max(right.abs());
            if onset.is_none() && level > 1e-4 {
                onset = Some(sample);
                onset_level = level;
            }
        }

        let onset = onset.expect("the reverb produced no wet onset at all");
        let onset_ms = onset as f32 / sample_rate * 1_000.0;
        assert!(
            (4.0..10.0).contains(&onset_ms),
            "the first wet signal arrived at {onset_ms:.2} ms instead of 4-10 ms"
        );
        assert!(
            onset_level < 0.2,
            "the early field is too loud ({onset_level})"
        );
    }

    #[test]
    fn the_early_field_is_stereo() {
        let sample_rate = 48_000.0;
        let mut reverb = Reverb::new(sample_rate);
        let mut right_onset = None;
        let mut difference = 0.0f32;

        // Only the left input is excited. The cross-fed, later early taps must
        // still reach the right output inside the first 10 ms.
        for sample in 0..(sample_rate * 0.01) as usize {
            let input = if sample == 0 { 1.0 } else { 0.0 };
            let (left, right) = reverb.process(input, 0.0, 0.6, 1.0);
            difference += (left - right).abs();
            if right_onset.is_none() && right.abs() > 1e-4 {
                right_onset = Some(sample);
            }
        }

        let right_onset = right_onset.expect("the right early field stayed silent");
        assert!((right_onset as f32) / sample_rate < 0.010);
        assert!(difference > 0.001, "the early field was mono");
    }

    #[test]
    fn a_dc_input_does_not_accumulate_in_the_feedback_loop() {
        let sample_rate = 48_000.0;
        let mut reverb = Reverb::new(sample_rate);
        let settle = (sample_rate * 5.0) as usize;
        let total = (sample_rate * 6.0) as usize;
        let mut sum = 0.0f64;
        let mut count = 0usize;

        for sample in 0..total {
            let (left, right) = reverb.process(0.1, 0.1, 0.6, 1.0);
            assert!(left.is_finite() && right.is_finite());
            if sample >= settle {
                sum += (left as f64 + right as f64) * 0.5;
                count += 1;
            }
        }

        let mean = sum / count as f64;
        assert!(
            mean.abs() < 0.02,
            "0.1 DC left a wet mean of {mean:.6} after settling"
        );

        // The loop itself must not be sitting on a large offset either. The
        // injected DC is only about 0.05 at the line inputs; without the loop
        // blockers this would climb into the units.
        for line in &reverb.lines {
            let peak = line
                .iter()
                .fold(0.0f32, |peak, &value| peak.max(value.abs()));
            assert!(peak < 0.2, "the feedback loop stored a DC offset ({peak})");
        }
    }

    #[test]
    fn a_long_tail_is_bounded_and_finite() {
        let sample_rate = 48_000.0;
        let mut reverb = Reverb::new(sample_rate);
        let mut peak = 0.0f32;
        let mut early_peak = 0.0f32;
        let mut late_peak = 0.0f32;

        // Twenty seconds exceeds the 15 s T60 of the longest decay. The tail
        // must neither run away nor produce a NaN, and it must really decay.
        for sample in 0..(sample_rate * 20.0) as usize {
            let input = if sample == 0 {
                1.0
            } else if sample == sample_rate as usize {
                -0.5
            } else {
                0.0
            };
            let (left, right) = reverb.process(input, input, 1.0, 1.0);
            assert!(
                left.is_finite() && right.is_finite(),
                "the long tail produced NaN or infinity at sample {sample}"
            );
            let level = left.abs().max(right.abs());
            peak = peak.max(level);
            if sample < sample_rate as usize {
                early_peak = early_peak.max(level);
            }
            if sample >= (sample_rate * 19.0) as usize {
                late_peak = late_peak.max(level);
            }
        }

        assert!(peak < 2.0, "the long tail ran away (peak {peak})");
        assert!(
            late_peak < early_peak * 0.01,
            "the long tail did not decay: {late_peak} vs {early_peak}"
        );
    }

    #[test]
    fn extreme_inputs_produce_no_nan() {
        let mut reverb = Reverb::new(48_000.0);

        for sample in 0..48_000 {
            let input = match sample % 4 {
                0 => 10.0,
                1 => -10.0,
                2 => 1.0,
                _ => -0.25,
            };
            let (left, right) = reverb.process(input, -input, 1.0, 1.0);
            assert!(
                left.is_finite() && right.is_finite(),
                "an extreme input produced NaN or infinity at sample {sample}"
            );
        }
    }

    #[test]
    fn the_input_diffusion_is_a_bounded_unity_gain_allpass() {
        for delays in DIFFUSION_DELAYS {
            let mut stages: [Allpass; DIFFUSION_STAGES] =
                std::array::from_fn(|_| Allpass::default());
            for stage in 0..DIFFUSION_STAGES {
                stages[stage] = Allpass::new(delays[stage], 48_000.0, DIFFUSION_GAIN);
            }

            let mut energy = 0.0f64;
            let mut peak = 0.0f32;
            for sample in 0..96_000 {
                let input = if sample == 0 { 1.0 } else { 0.0 };
                let mut output = input;
                for stage in &mut stages {
                    output = stage.process(output);
                }
                assert!(output.is_finite());
                energy += (output as f64) * (output as f64);
                peak = peak.max(output.abs());
            }

            assert!(
                (energy - 1.0).abs() < 0.01,
                "the diffusion chain did not preserve energy: {energy}"
            );
            assert!(peak < 2.0, "the diffusion chain exploded (peak {peak})");
        }
    }
}
