//! The effect chain: a drive, a phaser, a chorus, a ping-pong delay, a
//! compact reverb and a three band EQ.
//!
//! All the delay lines are allocated up front (in [`Fx::new`]) so that the
//! processing itself never allocates. The new effects default to completely
//! neutral settings, so old patches and presets keep sounding the same.

use std::f32::consts::PI;

use super::adaa::AdaaTanh;
use super::reverb::Reverb;
use super::reverb_legacy::LegacyReverb;

/// The base delay of the chorus in seconds.
const CHORUS_BASE_DELAY: f32 = 0.012;
/// The maximum modulation depth of the chorus in seconds.
const CHORUS_MAX_DEPTH: f32 = 0.004;
/// The frequency of the lowpass of the wet signal of the chorus, in Hz.
const CHORUS_LOWPASS: f32 = 8_000.0;
/// The maximum delay time of the delay line in seconds.
const DELAY_MAX_TIME: f32 = 2.0;
/// The frequency of the lowpass in the feedback path of the delay, in Hz.
const DELAY_LOWPASS: f32 = 5_000.0;
/// The time constant of the one-pole that follows a change of the delay time,
/// in seconds. Without it a moved control jumps the read position, which is
/// heard as a click or a zipper.
const DELAY_TIME_SMOOTHING: f32 = 0.02;
/// The mix below which an effect is switched off completely.
const BYPASS_MIX: f32 = 1e-4;
/// The maximum pre-gain of the drive, in decibels.
const DRIVE_MAX_GAIN_DB: f32 = 24.0;
/// The output compensation of the drive at full amount. The tanh already
/// limits the wet signal to ±1; this keeps a hot input from sounding louder
/// than the dry signal.
const DRIVE_COMPENSATION: f32 = 1.5;
/// The number of first-order allpass stages in the phaser.
const PHASER_STAGES: usize = 4;
/// The centre frequency of the phaser, in Hz.
const PHASER_CENTER: f32 = 700.0;
/// The sweep range of the phaser at full depth, in octaves.
const PHASER_OCTAVES: f32 = 2.0;
/// The lowest frequency of an allpass stage. The coefficient stays well
/// inside the stable range even at very low sample rates.
const PHASER_MIN_FREQUENCY: f32 = 10.0;
/// The frequency of the low/mid crossover of the EQ, in Hz.
const EQ_LOW_CROSSOVER: f32 = 250.0;
/// The frequency of the mid/high crossover of the EQ, in Hz.
const EQ_HIGH_CROSSOVER: f32 = 4_000.0;
/// The largest boost or cut of an EQ band, in decibels.
const EQ_MAX_GAIN_DB: f32 = 12.0;
/// A band gain closer to 0 dB than this is treated as neutral.
const EQ_BYPASS_DB: f32 = 1e-4;

/// The parameters of the effect chain.
#[derive(Debug, Clone, Copy)]
pub struct FxParams {
    pub drive_amount: f32,
    pub drive_mix: f32,
    pub phaser_rate: f32,
    pub phaser_depth: f32,
    pub phaser_mix: f32,
    pub eq_low: f32,
    pub eq_mid: f32,
    pub eq_high: f32,
    pub chorus_mix: f32,
    pub chorus_rate: f32,
    pub chorus_depth: f32,
    pub delay_time_s: f32,
    pub delay_feedback: f32,
    pub delay_mix: f32,
    pub reverb_mix: f32,
    pub reverb_decay: f32,
    /// Use the original Freeverb-style reverb and the unprocessed feedback
    /// path. Set by the legacy flavor.
    pub legacy: bool,
}

impl Default for FxParams {
    fn default() -> Self {
        Self {
            drive_amount: 0.0,
            drive_mix: 0.0,
            phaser_rate: 0.3,
            phaser_depth: 0.5,
            phaser_mix: 0.0,
            eq_low: 0.0,
            eq_mid: 0.0,
            eq_high: 0.0,
            chorus_mix: 0.0,
            chorus_rate: 0.8,
            chorus_depth: 0.5,
            delay_time_s: 0.35,
            delay_feedback: 0.3,
            delay_mix: 0.0,
            reverb_mix: 0.0,
            reverb_decay: 0.6,
            legacy: false,
        }
    }
}

/// The coefficient of a one-pole lowpass at the given frequency.
#[inline]
fn lowpass_coefficient(frequency: f32, sample_rate: f32) -> f32 {
    1.0 - (-2.0 * PI * frequency / sample_rate.max(1.0)).exp()
}

/// The coefficient of a one-pole that follows a control with the given time
/// constant. The time constant is in seconds, so the smoothing takes the same
/// time at every sample rate.
#[inline]
fn smoothing_coefficient(time_seconds: f32, sample_rate: f32) -> f32 {
    let samples = (time_seconds.max(1e-6) * sample_rate.max(1.0)).max(1.0);
    1.0 - (-1.0 / samples).exp()
}

/// Flushes a value that is too small to be heard to exactly zero.
#[inline]
fn flush_denormal(value: f32) -> f32 {
    if value.abs() < 1e-20 { 0.0 } else { value }
}

/// The coefficient of a first-order allpass at the given frequency. The
/// frequency is clamped so the coefficient stays inside the stable range even
/// at the lowest sample rates.
#[inline]
fn allpass_coefficient(frequency: f32, sample_rate: f32) -> f32 {
    let sample_rate = sample_rate.max(1.0);
    let max_frequency = (sample_rate * 0.45).max(PHASER_MIN_FREQUENCY);
    let frequency = frequency.clamp(PHASER_MIN_FREQUENCY, max_frequency);
    let tangent = (PI * frequency / sample_rate).tan();
    (1.0 - tangent) / (1.0 + tangent)
}

/// One first-order allpass stage in direct form II. The state is the single
/// delay of the stage.
#[inline]
fn allpass_stage(input: f32, coefficient: f32, state: &mut f32) -> f32 {
    let output = coefficient * input + *state;
    *state = flush_denormal(input - coefficient * output);
    output
}

/// The effect chain of the synthesizer.
pub struct Fx {
    sample_rate: f32,

    // The drive.
    drive_adaa_left: AdaaTanh,
    drive_adaa_right: AdaaTanh,

    // The phaser.
    phaser_phase: f32,
    phaser_left: [f32; PHASER_STAGES],
    phaser_right: [f32; PHASER_STAGES],

    // The three band EQ. The low band is the output of a lowpass at the low
    // crossover, the high band is the input minus a lowpass at the high
    // crossover, and the mid band is the rest.
    eq_low_left: f32,
    eq_low_right: f32,
    eq_high_left: f32,
    eq_high_right: f32,
    eq_low_coefficient: f32,
    eq_high_coefficient: f32,

    // The chorus.
    chorus_left: Vec<f32>,
    chorus_right: Vec<f32>,
    chorus_index: usize,
    chorus_phase: f32,
    /// The state of the lowpass of the wet signal.
    chorus_lp_left: f32,
    chorus_lp_right: f32,
    /// The coefficient of the lowpass of the wet signal.
    chorus_coefficient: f32,

    // The ping-pong delay.
    delay_left: Vec<f32>,
    delay_right: Vec<f32>,
    delay_index: usize,
    /// The smoothed delay time of the ping-pong delay, in samples. `None`
    /// until the first sample is processed, so the first read uses the
    /// requested time directly instead of ramping up from zero.
    delay_time_samples: Option<f32>,
    /// The coefficient of the one-pole that smoothes the delay time.
    delay_smoothing: f32,
    /// The state of the lowpass in the feedback path.
    delay_lp_left: f32,
    delay_lp_right: f32,
    /// The coefficient of the lowpass in the feedback path.
    delay_coefficient: f32,
    /// Anti-aliased saturation in the delay feedback.
    delay_adaa_left: AdaaTanh,
    delay_adaa_right: AdaaTanh,

    // The reverb.
    reverb: Reverb,
    legacy_reverb: LegacyReverb,
    /// The flavor used by the previous sample. Switching flavors clears the
    /// reverb that was not running, so it cannot replay stale audio.
    last_legacy: Option<bool>,
    /// Whether the whole chain was bypassed on the previous sample. The
    /// first bypassed sample clears all the state, so re-enabling an effect
    /// cannot replay an old tail.
    bypassed: bool,
}

impl Fx {
    /// Allocates all the delay lines. This is called from the plugin's
    /// `activate` function, never from the audio thread.
    pub fn new(sample_rate: f32) -> Self {
        let sample_rate = sample_rate.max(1.0);

        let chorus_len =
            (sample_rate * (CHORUS_BASE_DELAY + CHORUS_MAX_DEPTH + 0.002)).ceil() as usize + 2;
        let delay_len = (sample_rate * DELAY_MAX_TIME).ceil() as usize + 2;

        Self {
            sample_rate,
            drive_adaa_left: AdaaTanh::default(),
            drive_adaa_right: AdaaTanh::default(),
            phaser_phase: 0.0,
            phaser_left: [0.0; PHASER_STAGES],
            phaser_right: [0.0; PHASER_STAGES],
            eq_low_left: 0.0,
            eq_low_right: 0.0,
            eq_high_left: 0.0,
            eq_high_right: 0.0,
            eq_low_coefficient: lowpass_coefficient(EQ_LOW_CROSSOVER, sample_rate),
            eq_high_coefficient: lowpass_coefficient(EQ_HIGH_CROSSOVER, sample_rate),
            chorus_left: vec![0.0; chorus_len],
            chorus_right: vec![0.0; chorus_len],
            chorus_index: 0,
            chorus_phase: 0.0,
            chorus_lp_left: 0.0,
            chorus_lp_right: 0.0,
            chorus_coefficient: lowpass_coefficient(CHORUS_LOWPASS, sample_rate),
            delay_left: vec![0.0; delay_len],
            delay_right: vec![0.0; delay_len],
            delay_index: 0,
            delay_time_samples: None,
            delay_smoothing: smoothing_coefficient(DELAY_TIME_SMOOTHING, sample_rate),
            delay_lp_left: 0.0,
            delay_lp_right: 0.0,
            delay_coefficient: lowpass_coefficient(DELAY_LOWPASS, sample_rate),
            delay_adaa_left: AdaaTanh::default(),
            delay_adaa_right: AdaaTanh::default(),
            reverb: Reverb::new(sample_rate),
            legacy_reverb: LegacyReverb::new(sample_rate),
            last_legacy: None,
            bypassed: true,
        }
    }

    /// Clears all the delay lines and the state of the other effects.
    pub fn reset(&mut self) {
        self.drive_adaa_left.reset();
        self.drive_adaa_right.reset();
        self.phaser_phase = 0.0;
        self.phaser_left = [0.0; PHASER_STAGES];
        self.phaser_right = [0.0; PHASER_STAGES];
        self.eq_low_left = 0.0;
        self.eq_low_right = 0.0;
        self.eq_high_left = 0.0;
        self.eq_high_right = 0.0;
        self.chorus_left.fill(0.0);
        self.chorus_right.fill(0.0);
        self.chorus_index = 0;
        self.chorus_phase = 0.0;
        self.chorus_lp_left = 0.0;
        self.chorus_lp_right = 0.0;
        self.delay_left.fill(0.0);
        self.delay_right.fill(0.0);
        self.delay_index = 0;
        self.delay_time_samples = None;
        self.delay_lp_left = 0.0;
        self.delay_lp_right = 0.0;
        self.delay_adaa_left = AdaaTanh::default();
        self.delay_adaa_right = AdaaTanh::default();
        self.reverb.reset();
        self.legacy_reverb.reset();
        self.last_legacy = None;
        self.bypassed = true;
    }

    /// Processes a single stereo sample.
    pub fn process(&mut self, left: f32, right: f32, params: &FxParams) -> (f32, f32) {
        // The effect chain is not initialized yet.
        if self.delay_left.is_empty() {
            return (left, right);
        }

        // Clamp first, so a fully bypassed chain is detected before any
        // effect updates its state.
        let drive_mix = params.drive_mix.clamp(0.0, 1.0);
        let phaser_mix = params.phaser_mix.clamp(0.0, 1.0);
        let chorus_mix = params.chorus_mix.clamp(0.0, 1.0);
        let delay_mix = params.delay_mix.clamp(0.0, 1.0);
        let reverb_mix = params.reverb_mix.clamp(0.0, 1.0);
        let eq_low = params.eq_low.clamp(-EQ_MAX_GAIN_DB, EQ_MAX_GAIN_DB);
        let eq_mid = params.eq_mid.clamp(-EQ_MAX_GAIN_DB, EQ_MAX_GAIN_DB);
        let eq_high = params.eq_high.clamp(-EQ_MAX_GAIN_DB, EQ_MAX_GAIN_DB);
        let eq_neutral = eq_low.abs() <= EQ_BYPASS_DB
            && eq_mid.abs() <= EQ_BYPASS_DB
            && eq_high.abs() <= EQ_BYPASS_DB;
        let all_bypassed = drive_mix <= BYPASS_MIX
            && phaser_mix <= BYPASS_MIX
            && chorus_mix <= BYPASS_MIX
            && delay_mix <= BYPASS_MIX
            && reverb_mix <= BYPASS_MIX
            && eq_neutral;

        if all_bypassed {
            if !self.bypassed {
                // Entering bypass clears every delay line, the reverb and
                // the new effects, so a later enable starts from silence
                // instead of replaying an old tail.
                self.reset();
            }
            return (left, right);
        }

        // The reset on entering bypass left the state clean, including
        // `last_legacy`. The first active sample can therefore start here.
        self.bypassed = false;

        if self.last_legacy != Some(params.legacy) {
            // Switching flavor resets both reverb engines and the delay
            // saturator so no state from the other algorithm leaks in.
            self.reverb.reset();
            self.legacy_reverb.reset();
            self.delay_adaa_left = AdaaTanh::default();
            self.delay_adaa_right = AdaaTanh::default();
            self.last_legacy = Some(params.legacy);
        }

        let (left, right) = self.process_drive(left, right, params);
        let (left, right) = self.process_phaser(left, right, params);
        let (left, right) = self.process_chorus(left, right, params);
        let (left, right) = self.process_delay(left, right, params);
        let (left, right) = self.process_reverb(left, right, params);
        self.process_eq(left, right, params)
    }

    /// An anti-aliased tanh drive with a dry/wet crossfade.
    fn process_drive(&mut self, left: f32, right: f32, params: &FxParams) -> (f32, f32) {
        let mix = params.drive_mix.clamp(0.0, 1.0);

        if mix <= BYPASS_MIX {
            // A bypassed drive must not colour the signal, and its ADAA
            // history must not leak into a later enable.
            self.drive_adaa_left.reset();
            self.drive_adaa_right.reset();
            return (left, right);
        }

        let amount = params.drive_amount.clamp(0.0, 1.0);
        // The amount maps to 0..24 dB of pre-gain. The tanh limits the wet
        // signal, while the compensation keeps a hot input close to the level
        // of the dry signal.
        let gain = 10f32.powf(amount * DRIVE_MAX_GAIN_DB / 20.0);
        let compensation = 1.0 / (1.0 + amount * DRIVE_COMPENSATION);
        let wet_left = self.drive_adaa_left.process(left * gain) * compensation;
        let wet_right = self.drive_adaa_right.process(right * gain) * compensation;

        (
            left * (1.0 - mix) + wet_left * mix,
            right * (1.0 - mix) + wet_right * mix,
        )
    }

    /// A four stage first-order allpass phaser with a stereo quadrature LFO.
    fn process_phaser(&mut self, left: f32, right: f32, params: &FxParams) -> (f32, f32) {
        let mix = params.phaser_mix.clamp(0.0, 1.0);

        if mix <= BYPASS_MIX {
            // No state may influence the sound while the phaser is off.
            self.phaser_phase = 0.0;
            self.phaser_left = [0.0; PHASER_STAGES];
            self.phaser_right = [0.0; PHASER_STAGES];
            return (left, right);
        }

        let rate = params.phaser_rate.clamp(0.0, self.sample_rate * 0.5);
        self.phaser_phase += rate / self.sample_rate;
        if self.phaser_phase >= 1.0 {
            self.phaser_phase -= self.phaser_phase.floor();
        }

        let phase = self.phaser_phase * PI * 2.0;
        let depth = params.phaser_depth.clamp(0.0, 1.0) * PHASER_OCTAVES;
        let (lfo_left, lfo_right) = phase.sin_cos();
        let frequency_left = PHASER_CENTER * (depth * lfo_left).exp2();
        let frequency_right = PHASER_CENTER * (depth * lfo_right).exp2();
        let coefficient_left = allpass_coefficient(frequency_left, self.sample_rate);
        let coefficient_right = allpass_coefficient(frequency_right, self.sample_rate);

        let wet_left = self.phaser_left.iter_mut().fold(left, |sample, state| {
            allpass_stage(sample, coefficient_left, state)
        });
        let wet_right = self.phaser_right.iter_mut().fold(right, |sample, state| {
            allpass_stage(sample, coefficient_right, state)
        });

        (
            left * (1.0 - mix) + wet_left * mix,
            right * (1.0 - mix) + wet_right * mix,
        )
    }

    /// A three band EQ built from two one-pole crossovers. The bands always
    /// sum back to the input, so three gains of 0 dB would be transparent (and
    /// the neutral case is bypassed outright).
    fn process_eq(&mut self, left: f32, right: f32, params: &FxParams) -> (f32, f32) {
        let low_db = params.eq_low.clamp(-EQ_MAX_GAIN_DB, EQ_MAX_GAIN_DB);
        let mid_db = params.eq_mid.clamp(-EQ_MAX_GAIN_DB, EQ_MAX_GAIN_DB);
        let high_db = params.eq_high.clamp(-EQ_MAX_GAIN_DB, EQ_MAX_GAIN_DB);

        if low_db.abs() <= EQ_BYPASS_DB
            && mid_db.abs() <= EQ_BYPASS_DB
            && high_db.abs() <= EQ_BYPASS_DB
        {
            self.eq_low_left = 0.0;
            self.eq_low_right = 0.0;
            self.eq_high_left = 0.0;
            self.eq_high_right = 0.0;
            return (left, right);
        }

        self.eq_low_left += (left - self.eq_low_left) * self.eq_low_coefficient;
        self.eq_low_right += (right - self.eq_low_right) * self.eq_low_coefficient;
        self.eq_high_left += (left - self.eq_high_left) * self.eq_high_coefficient;
        self.eq_high_right += (right - self.eq_high_right) * self.eq_high_coefficient;
        self.eq_low_left = flush_denormal(self.eq_low_left);
        self.eq_low_right = flush_denormal(self.eq_low_right);
        self.eq_high_left = flush_denormal(self.eq_high_left);
        self.eq_high_right = flush_denormal(self.eq_high_right);

        let low_gain = 10f32.powf(low_db / 20.0);
        let mid_gain = 10f32.powf(mid_db / 20.0);
        let high_gain = 10f32.powf(high_db / 20.0);

        let (low_left, high_left) = (self.eq_low_left, left - self.eq_high_left);
        let (low_right, high_right) = (self.eq_low_right, right - self.eq_high_right);
        let mid_left = left - low_left - high_left;
        let mid_right = right - low_right - high_right;

        (
            low_left * low_gain + mid_left * mid_gain + high_left * high_gain,
            low_right * low_gain + mid_right * mid_gain + high_right * high_gain,
        )
    }

    /// A two tap chorus with a quadrature LFO.
    fn process_chorus(&mut self, left: f32, right: f32, params: &FxParams) -> (f32, f32) {
        let mix = params.chorus_mix.clamp(0.0, 1.0);

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
        let next_index = index + 1;
        self.chorus_index = if next_index == self.chorus_left.len() {
            0
        } else {
            next_index
        };

        let mut wet_left = read_interpolated(&self.chorus_left, index, delay_left);
        let mut wet_right = read_interpolated(&self.chorus_right, index, delay_right);

        // The wet signal is lowpassed, so that the chorus does not sound
        // harsh.
        self.chorus_lp_left += (wet_left - self.chorus_lp_left) * self.chorus_coefficient;
        self.chorus_lp_right += (wet_right - self.chorus_lp_right) * self.chorus_coefficient;
        self.chorus_lp_left = flush_denormal(self.chorus_lp_left);
        self.chorus_lp_right = flush_denormal(self.chorus_lp_right);
        wet_left = self.chorus_lp_left;
        wet_right = self.chorus_lp_right;

        if mix <= BYPASS_MIX {
            // Keep the delay line and the LFO moving so re-enabling the
            // chorus cannot play audio from minutes ago.
            return (left, right);
        }

        (
            left * (1.0 - mix) + wet_left * mix,
            right * (1.0 - mix) + wet_right * mix,
        )
    }

    /// A ping-pong delay with a fractional read position.
    fn process_delay(&mut self, left: f32, right: f32, params: &FxParams) -> (f32, f32) {
        let mix = params.delay_mix.clamp(0.0, 1.0);

        let len = self.delay_left.len();
        let max_delay = (len - 2) as f32;
        let target = (params.delay_time_s * self.sample_rate).clamp(1.0, max_delay);
        // Follow the requested time with a one-pole. A static delay adds
        // exactly zero on every step, so its output does not change at all;
        // a moved control glides instead of stepping the read position.
        let delay = match self.delay_time_samples {
            Some(current) => {
                (current + (target - current) * self.delay_smoothing).clamp(1.0, max_delay)
            }
            // The first sample (and the first after a reset) uses the
            // requested delay directly.
            None => target,
        };
        self.delay_time_samples = Some(delay);
        // A bypassed delay writes the dry signal without feedback, so old
        // repeats cannot survive in the line and reappear later.
        let feedback = if mix <= BYPASS_MIX {
            0.0
        } else {
            params.delay_feedback.clamp(0.0, 0.95)
        };

        // Read the delayed signal before writing the new one.
        let wet_left = read_interpolated(&self.delay_left, self.delay_index, delay);
        let wet_right = read_interpolated(&self.delay_right, self.delay_index, delay);

        // The feedback is damped and softly clipped, so that a long feedback
        // does not build up into a howl.
        let (mut feedback_left, mut feedback_right) = if feedback > 0.0 {
            if params.legacy {
                ((wet_right * feedback).tanh(), (wet_left * feedback).tanh())
            } else {
                (
                    self.delay_adaa_left.process(wet_right * feedback),
                    self.delay_adaa_right.process(wet_left * feedback),
                )
            }
        } else {
            self.delay_adaa_left.reset();
            self.delay_adaa_right.reset();
            (0.0, 0.0)
        };
        self.delay_lp_left += (feedback_left - self.delay_lp_left) * self.delay_coefficient;
        self.delay_lp_right += (feedback_right - self.delay_lp_right) * self.delay_coefficient;
        self.delay_lp_left = flush_denormal(self.delay_lp_left);
        self.delay_lp_right = flush_denormal(self.delay_lp_right);
        feedback_left = self.delay_lp_left;
        feedback_right = self.delay_lp_right;

        // Ping-pong: the left channel is fed by the right one and vice versa.
        self.delay_left[self.delay_index] = left + feedback_left;
        self.delay_right[self.delay_index] = right + feedback_right;
        let next_index = self.delay_index + 1;
        self.delay_index = if next_index == len { 0 } else { next_index };

        if mix <= BYPASS_MIX {
            // The line is kept up to date while the delay is bypassed.
            return (left, right);
        }

        (
            left * (1.0 - mix) + wet_left * mix,
            right * (1.0 - mix) + wet_right * mix,
        )
    }

    /// A modulated feedback delay network reverb. The dry and wet signals are
    /// crossfaded instead of simply added, so the effect does not raise the
    /// perceived level when the mix is turned up.
    fn process_reverb(&mut self, left: f32, right: f32, params: &FxParams) -> (f32, f32) {
        let mix = params.reverb_mix.clamp(0.0, 1.0);

        let (wet_left, wet_right) = if params.legacy {
            self.legacy_reverb
                .process(left, right, params.reverb_decay, mix)
        } else {
            self.reverb.process(left, right, params.reverb_decay, mix)
        };

        if mix <= BYPASS_MIX {
            // The reverb keeps its lines current while bypassed, so enabling
            // it again cannot replay an old tail.
            return (left, right);
        }

        // The old Freeverb path added its wet signal, while the FDN uses a
        // dry/wet crossfade.
        if params.legacy {
            (left + wet_left * mix, right + wet_right * mix)
        } else {
            (
                left * (1.0 - mix) + wet_left * mix,
                right * (1.0 - mix) + wet_right * mix,
            )
        }
    }
}

/// Reads from a delay line with a fractional delay, using linear interpolation.
/// `write_index` is the position that was just written to.
#[inline]
fn read_interpolated(buffer: &[f32], write_index: usize, delay: f32) -> f32 {
    let len = buffer.len();

    // A single conditional wrap replaces `rem_euclid`: all the callers keep
    // the delay below the length of the line, so the read position is never
    // more than one line away from the valid range.
    let mut position = write_index as f32 - delay;
    if position < 0.0 {
        position += len as f32;
    }

    let floored = position.floor();
    let mut index = floored as usize;
    if index >= len {
        index -= len;
    }
    let fraction = position - floored;
    let next = if index + 1 == len { 0 } else { index + 1 };

    buffer[index] + (buffer[next] - buffer[index]) * fraction
}

impl Default for Fx {
    /// An effect chain without any allocated delay lines. Use [`Fx::new`] to
    /// create a usable one.
    fn default() -> Self {
        Self {
            sample_rate: 48_000.0,
            drive_adaa_left: AdaaTanh::default(),
            drive_adaa_right: AdaaTanh::default(),
            phaser_phase: 0.0,
            phaser_left: [0.0; PHASER_STAGES],
            phaser_right: [0.0; PHASER_STAGES],
            eq_low_left: 0.0,
            eq_low_right: 0.0,
            eq_high_left: 0.0,
            eq_high_right: 0.0,
            eq_low_coefficient: 0.0,
            eq_high_coefficient: 0.0,
            chorus_left: Vec::new(),
            chorus_right: Vec::new(),
            chorus_index: 0,
            chorus_phase: 0.0,
            chorus_lp_left: 0.0,
            chorus_lp_right: 0.0,
            chorus_coefficient: 0.0,
            delay_left: Vec::new(),
            delay_right: Vec::new(),
            delay_index: 0,
            delay_time_samples: None,
            delay_smoothing: 0.0,
            delay_lp_left: 0.0,
            delay_lp_right: 0.0,
            delay_coefficient: 0.0,
            delay_adaa_left: AdaaTanh::default(),
            delay_adaa_right: AdaaTanh::default(),
            reverb: Reverb::default(),
            legacy_reverb: LegacyReverb::default(),
            last_legacy: None,
            bypassed: true,
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
            drive_amount: 0.0,
            drive_mix: 0.0,
            phaser_rate: 0.3,
            phaser_depth: 0.5,
            phaser_mix: 0.0,
            eq_low: 0.0,
            eq_mid: 0.0,
            eq_high: 0.0,
            chorus_mix: 0.5,
            chorus_rate: 0.8,
            chorus_depth: 0.5,
            delay_time_s: 0.35,
            delay_feedback: 0.3,
            delay_mix: 0.5,
            reverb_mix: 0.5,
            reverb_decay: 0.6,
            legacy: false,
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
            assert_eq!(left, 0.5);
            assert_eq!(right, -0.5);
        }
    }

    #[test]
    fn a_tiny_mix_is_a_bypass() {
        let mut fx = Fx::new(48_000.0);
        let params = FxParams {
            chorus_mix: 1e-5,
            delay_mix: 1e-5,
            reverb_mix: 1e-5,
            ..FxParams::default()
        };

        for _ in 0..1_000 {
            let (left, right) = fx.process(0.5, -0.5, &params);
            assert_eq!(left, 0.5);
            assert_eq!(right, -0.5);
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

    #[test]
    fn a_static_delay_is_unchanged() {
        let sample_rate = 48_000.0f32;
        let mut fx = Fx::new(sample_rate);
        let params = FxParams {
            delay_mix: 1.0,
            delay_time_s: 0.0501,
            delay_feedback: 0.0,
            ..FxParams::default()
        };
        let delay = params.delay_time_s * sample_rate;
        assert!(delay.fract() != 0.0, "the delay should be fractional");

        // The same fractional read, on the dry input history. With no
        // feedback the delay line only ever holds the dry signal.
        fn read(history: &[f32], delay: f32) -> f32 {
            let position = (history.len() - 1) as f32 - delay;
            let index = position.floor() as usize;
            let fraction = position - position.floor();
            history[index] + (history[index + 1] - history[index]) * fraction
        }

        let mut history = Vec::new();
        for sample in 0..12_000 {
            let input = (sample as f32 * 0.017).sin() * 0.7;
            history.push(input);
            let (left, _) = fx.process(input, 0.0, &params);

            // Smoothing a static control adds exactly zero, so the read
            // position is the requested delay, sample for sample.
            assert_eq!(fx.delay_time_samples.unwrap().to_bits(), delay.to_bits());

            // The first samples would read before the start of the history.
            if sample > delay.ceil() as usize {
                assert_eq!(
                    left.to_bits(),
                    read(&history, delay).to_bits(),
                    "sample {sample} changed"
                );
            }
        }

        // A reset clears the smoother, so the next sample is not a ramp
        // either.
        fx.reset();
        fx.process(0.0, 0.0, &params);
        assert_eq!(fx.delay_time_samples.unwrap().to_bits(), delay.to_bits());
    }

    #[test]
    fn a_jump_in_the_delay_time_moves_gradually() {
        let sample_rate = 48_000.0f32;
        let mut fx = Fx::new(sample_rate);
        let mut params = FxParams {
            delay_mix: 1.0,
            delay_time_s: 0.1,
            delay_feedback: 0.0,
            ..FxParams::default()
        };

        // The first processed sample uses the requested delay directly, with
        // no ramp up from zero.
        fx.process(0.0, 0.0, &params);
        let start = 0.1 * sample_rate;
        assert_eq!(fx.delay_time_samples.unwrap().to_bits(), start.to_bits());

        params.delay_time_s = 0.2;
        let target = 0.2 * sample_rate;
        let total = target - start;
        let mut previous = start;
        let mut last_step = total;

        for sample in 0..(sample_rate * 0.25) as usize {
            fx.process(0.0, 0.0, &params);
            let current = fx.delay_time_samples.unwrap();
            let step = current - previous;

            // The one-pole never runs backwards and it may land exactly on
            // the requested time while the control is held, which is fine.
            assert!(
                step >= 0.0,
                "the delay moved backwards at sample {sample}: {step}"
            );
            assert!(
                step <= total * 0.01,
                "the delay jumped {step} samples at sample {sample}"
            );
            // A one-pole moves the most on the first sample and then slows
            // down, so the read position never overshoots the control.
            assert!(
                step <= last_step,
                "the glide sped up at sample {sample}: {step} after {last_step}"
            );
            assert!(current <= target, "the delay overshot at sample {sample}");

            previous = current;
            last_step = step;
        }

        // Quarter of a second is twelve time constants, so the glide is done.
        assert!(
            (previous - target).abs() < 1.0,
            "the delay stopped at {previous} instead of {target}"
        );
    }

    #[test]
    fn re_enabling_after_full_bypass_does_not_replay_the_tail() {
        let mut fx = Fx::new(48_000.0);
        let mut params = FxParams {
            delay_mix: 1.0,
            delay_time_s: 0.1,
            delay_feedback: 0.8,
            ..FxParams::default()
        };

        // Fill the delay line with an impulse and several feedback repeats.
        fx.process(1.0, 0.0, &params);
        for _ in 0..20_000 {
            fx.process(0.0, 0.0, &params);
        }

        // Bypass the whole chain while the input is silent. The first
        // bypassed sample clears the delay line, so re-enabling the delay
        // cannot replay the old repeats.
        params.delay_mix = 0.0;
        for _ in 0..20_000 {
            fx.process(0.0, 0.0, &params);
        }

        params.delay_mix = 1.0;
        let mut peak: f32 = 0.0;
        for _ in 0..10_000 {
            let (left, right) = fx.process(0.0, 0.0, &params);
            peak = peak.max(left.abs()).max(right.abs());
        }

        assert!(
            peak < 1e-4,
            "the re-enabled delay replayed stale audio (peak {peak})"
        );
    }

    #[test]
    fn a_full_bypass_returns_dry_and_clears_the_state() {
        let mut fx = Fx::new(48_000.0);
        let mut params = FxParams {
            chorus_mix: 1.0,
            delay_mix: 1.0,
            delay_time_s: 0.1,
            delay_feedback: 0.85,
            reverb_mix: 1.0,
            reverb_decay: 1.0,
            ..FxParams::default()
        };

        // Excite every effect, then let the tails fill the delay lines and
        // the reverb.
        fx.process(1.0, -1.0, &params);
        for _ in 0..20_000 {
            fx.process(0.0, 0.0, &params);
        }

        // Bypassing the whole chain returns the dry signal and clears the
        // state on the first bypassed sample.
        params.chorus_mix = 0.0;
        params.delay_mix = 0.0;
        params.reverb_mix = 0.0;
        for _ in 0..1_000 {
            let (left, right) = fx.process(0.25, -0.75, &params);
            assert_eq!(left, 0.25);
            assert_eq!(right, -0.75);
        }
        assert!(fx.delay_left.iter().all(|&sample| sample == 0.0));
        assert!(fx.delay_right.iter().all(|&sample| sample == 0.0));
        assert!(fx.chorus_left.iter().all(|&sample| sample == 0.0));
        assert!(fx.chorus_right.iter().all(|&sample| sample == 0.0));

        // Enabling one effect after the bypass starts from silence, with no
        // tail left over from before the bypass.
        params.delay_mix = 1.0;
        let mut peak: f32 = 0.0;
        for _ in 0..10_000 {
            let (left, right) = fx.process(0.0, 0.0, &params);
            peak = peak.max(left.abs()).max(right.abs());
        }
        assert!(
            peak < 1e-4,
            "the re-enabled delay replayed an old tail (peak {peak})"
        );
    }

    #[test]
    fn the_two_reverb_engines_are_different() {
        let mut modern = Fx::new(48_000.0);
        let mut legacy = Fx::new(48_000.0);
        let modern_params = FxParams {
            reverb_mix: 0.5,
            reverb_decay: 0.6,
            ..FxParams::default()
        };
        let legacy_params = FxParams {
            legacy: true,
            ..modern_params
        };

        let mut difference = 0.0f32;
        for sample in 0..8_000 {
            let input = if sample == 0 { 1.0 } else { 0.0 };
            let (modern_left, _) = modern.process(input, input, &modern_params);
            let (legacy_left, _) = legacy.process(input, input, &legacy_params);
            difference += (modern_left - legacy_left).abs();
        }

        assert!(
            difference > 0.01,
            "the legacy and modern reverbs were identical"
        );
    }

    #[test]
    fn the_one_wrap_reader_matches_the_reference() {
        let len = 97;
        let buffer: Vec<f32> = (0..len).map(|i| (i as f32 * 0.37).sin()).collect();

        // The original reader, which wrapped the position with `rem_euclid`.
        fn reference(buffer: &[f32], write_index: usize, delay: f32) -> f32 {
            let len = buffer.len();
            let position = (write_index as f32 - delay).rem_euclid(len as f32);
            let index = position.floor() as usize % len;
            let fraction = position - position.floor();
            let next = (index + 1) % len;
            buffer[index] + (buffer[next] - buffer[index]) * fraction
        }

        for write_index in 0..len {
            for delay in [0.0, 0.5, 1.0, 1.25, 2.0, 3.75, (len - 1) as f32, len as f32] {
                assert_eq!(
                    read_interpolated(&buffer, write_index, delay).to_bits(),
                    reference(&buffer, write_index, delay).to_bits(),
                    "write {write_index}, delay {delay}"
                );
            }
        }

        // Fractional delays across the whole line, including the wrap.
        for step in 0..(len * 4) {
            let delay = step as f32 * 0.25;
            let write_index = (step * 7) % len;
            assert_eq!(
                read_interpolated(&buffer, write_index, delay).to_bits(),
                reference(&buffer, write_index, delay).to_bits(),
                "write {write_index}, delay {delay}"
            );
        }
    }

    #[test]
    fn the_wet_path_stays_bounded() {
        let mut fx = Fx::new(48_000.0);
        let params = FxParams {
            drive_amount: 1.0,
            drive_mix: 1.0,
            phaser_rate: 5.0,
            phaser_depth: 1.0,
            phaser_mix: 1.0,
            eq_low: 12.0,
            eq_mid: -12.0,
            eq_high: 12.0,
            chorus_mix: 1.0,
            chorus_rate: 0.8,
            chorus_depth: 0.5,
            delay_time_s: 0.35,
            delay_feedback: 0.95,
            delay_mix: 1.0,
            reverb_mix: 1.0,
            reverb_decay: 1.0,
            legacy: false,
        };

        // A loud signal that is fed back again and again must not run away.
        let mut peak: f32 = 0.0;
        for sample in 0..48_000 {
            let input = if sample % 480 == 0 { 0.9 } else { 0.0 };
            let (left, right) = fx.process(input, input, &params);
            assert!(left.is_finite() && right.is_finite());
            peak = peak.max(left.abs()).max(right.abs());
        }

        assert!(peak < 10.0, "the wet signal ran away ({peak})");
    }

    /// A deterministic noise source for the tests.
    fn noise(state: &mut u32) -> f32 {
        *state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        ((*state >> 8) as f32 / (1u32 << 23) as f32) * 2.0 - 1.0
    }

    #[test]
    fn neutral_defaults_are_bit_exact_dry() {
        let mut fx = Fx::new(48_000.0);
        let params = FxParams::default();
        let mut state = 0x5EED_1234;

        for sample in 0..48_000 {
            let left = noise(&mut state) * 0.7;
            let right = noise(&mut state) * 0.7;
            let (out_left, out_right) = fx.process(left, right, &params);
            assert_eq!(out_left.to_bits(), left.to_bits(), "sample {sample}");
            assert_eq!(out_right.to_bits(), right.to_bits(), "sample {sample}");
        }
    }

    #[test]
    fn the_drive_changes_the_signal_and_stays_bounded() {
        let mut fx = Fx::new(48_000.0);
        let mut params = FxParams {
            drive_amount: 1.0,
            drive_mix: 1.0,
            ..FxParams::default()
        };

        let mut difference = 0.0f32;
        let mut peak: f32 = 0.0;
        for sample in 0..48_000 {
            let time = sample as f32 / 48_000.0;
            let input = (time * 440.0 * std::f32::consts::TAU).sin() * 0.8;
            let (left, right) = fx.process(input, input, &params);
            assert!(left.is_finite() && right.is_finite());
            difference += (left - input).abs();
            peak = peak.max(left.abs()).max(right.abs());
        }

        assert!(difference > 1.0, "the drive did not change the signal");
        assert!(peak <= 1.01, "the drive exceeded the tanh range ({peak})");

        // With the mix at zero the drive is a bit-exact bypass, even though
        // the amount is still at its maximum.
        params.drive_mix = 0.0;
        for _ in 0..1_000 {
            let (left, right) = fx.process(0.25, -0.75, &params);
            assert_eq!(left, 0.25);
            assert_eq!(right, -0.75);
        }
    }

    #[test]
    fn the_phaser_changes_the_signal_and_stays_bounded() {
        let mut fx = Fx::new(48_000.0);
        let params = FxParams {
            phaser_rate: 1.5,
            phaser_depth: 0.8,
            phaser_mix: 1.0,
            ..FxParams::default()
        };

        let mut difference = 0.0f32;
        let mut peak: f32 = 0.0;
        for sample in 0..48_000 {
            let time = sample as f32 / 48_000.0;
            let input = (time * 440.0 * std::f32::consts::TAU).sin() * 0.5;
            let (left, right) = fx.process(input, input, &params);
            assert!(left.is_finite() && right.is_finite());
            difference += (left - input).abs();
            peak = peak.max(left.abs()).max(right.abs());
        }

        assert!(difference > 1.0, "the phaser did not change the signal");
        assert!(peak < 1.0, "the phaser ran away ({peak})");

        // Turning the mix off returns exactly the dry input, and no allpass
        // state is left behind.
        let off = FxParams {
            phaser_mix: 0.0,
            ..params
        };
        for _ in 0..1_000 {
            let (left, right) = fx.process(0.25, -0.75, &off);
            assert_eq!(left, 0.25);
            assert_eq!(right, -0.75);
        }
        assert!(fx.phaser_left.iter().all(|&state| state == 0.0));
        assert!(fx.phaser_right.iter().all(|&state| state == 0.0));
        assert_eq!(fx.phaser_phase, 0.0);
    }

    #[test]
    fn the_eq_changes_the_bands_and_stays_bounded() {
        fn rms_of(frequency: f32, params: &FxParams) -> f32 {
            let mut fx = Fx::new(48_000.0);
            let mut sum = 0.0f32;
            let mut peak: f32 = 0.0;
            for sample in 0..48_000 {
                let time = sample as f32 / 48_000.0;
                let input = (time * frequency * std::f32::consts::TAU).sin() * 0.25;
                let (left, right) = fx.process(input, input, params);
                assert!(left.is_finite() && right.is_finite());
                // Ignore the first quarter second while the filters settle.
                if sample >= 12_000 {
                    sum += left * left;
                    peak = peak.max(left.abs()).max(right.abs());
                }
            }
            assert!(peak < 4.0, "the EQ ran away at {frequency} Hz ({peak})");
            (sum / 36_000.0).sqrt()
        }

        let input_rms = 0.25 / 2f32.sqrt();
        let boosted = rms_of(
            100.0,
            &FxParams {
                eq_low: 12.0,
                eq_mid: -12.0,
                eq_high: -12.0,
                ..FxParams::default()
            },
        );
        let cut = rms_of(
            10_000.0,
            &FxParams {
                eq_low: -12.0,
                eq_mid: -12.0,
                eq_high: 12.0,
                ..FxParams::default()
            },
        );

        assert!(
            boosted > input_rms * 2.0,
            "the low boost did not raise the level ({boosted})"
        );
        assert!(
            cut > input_rms * 2.0,
            "the high boost did not raise the level ({cut})"
        );
    }

    #[test]
    fn a_full_bypass_considers_the_new_effects_and_resets_them() {
        let mut fx = Fx::new(48_000.0);
        let mut params = FxParams {
            drive_amount: 1.0,
            drive_mix: 1.0,
            phaser_rate: 5.0,
            phaser_depth: 1.0,
            phaser_mix: 1.0,
            eq_low: 12.0,
            eq_mid: -6.0,
            eq_high: 9.0,
            ..FxParams::default()
        };

        // Excite the new effects with a loud signal.
        for sample in 0..12_000 {
            let input = if sample == 0 { 1.0 } else { 0.0 };
            fx.process(input, -input, &params);
        }

        // With every old and new effect neutral the whole chain is bypassed.
        params.drive_mix = 0.0;
        params.phaser_mix = 0.0;
        params.eq_low = 0.0;
        params.eq_mid = 0.0;
        params.eq_high = 0.0;
        for _ in 0..1_000 {
            let (left, right) = fx.process(0.25, -0.75, &params);
            assert_eq!(left, 0.25);
            assert_eq!(right, -0.75);
        }

        assert!(fx.phaser_left.iter().all(|&state| state == 0.0));
        assert!(fx.phaser_right.iter().all(|&state| state == 0.0));
        assert_eq!(fx.phaser_phase, 0.0);
        assert_eq!(fx.eq_low_left, 0.0);
        assert_eq!(fx.eq_low_right, 0.0);
        assert_eq!(fx.eq_high_left, 0.0);
        assert_eq!(fx.eq_high_right, 0.0);

        // Re-enabling the new effects on silence starts from a clean state.
        params.drive_mix = 1.0;
        let (left, right) = fx.process(0.0, 0.0, &params);
        assert_eq!(left, 0.0);
        assert_eq!(right, 0.0);

        params.drive_mix = 0.0;
        params.phaser_mix = 1.0;
        let (left, right) = fx.process(0.0, 0.0, &params);
        assert_eq!(left, 0.0);
        assert_eq!(right, 0.0);

        params.phaser_mix = 0.0;
        params.eq_low = 12.0;
        let (left, right) = fx.process(0.0, 0.0, &params);
        assert_eq!(left, 0.0);
        assert_eq!(right, 0.0);
    }

    #[test]
    fn reset_clears_the_new_effects() {
        let mut fx = Fx::new(48_000.0);
        let params = FxParams {
            drive_amount: 1.0,
            drive_mix: 1.0,
            phaser_mix: 1.0,
            eq_low: 12.0,
            eq_mid: -12.0,
            eq_high: 12.0,
            ..FxParams::default()
        };

        for sample in 0..4_800 {
            let input = (sample as f32 * 0.01).sin();
            fx.process(input, input, &params);
        }
        assert!(
            fx.phaser_left.iter().any(|&state| state != 0.0)
                || fx.eq_low_left != 0.0
                || fx.eq_high_left != 0.0
        );

        fx.reset();
        assert_eq!(fx.phaser_phase, 0.0);
        assert!(fx.phaser_left.iter().all(|&state| state == 0.0));
        assert!(fx.phaser_right.iter().all(|&state| state == 0.0));
        assert_eq!(fx.eq_low_left, 0.0);
        assert_eq!(fx.eq_low_right, 0.0);
        assert_eq!(fx.eq_high_left, 0.0);
        assert_eq!(fx.eq_high_right, 0.0);
    }
}
