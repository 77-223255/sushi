//! A single synth voice: two oscillators, a unison bank, envelopes and a filter.

use nice_plug::midi::{Channel, Key};
use nice_plug::util;
use std::f32::consts::PI;

use super::adsr::{Adsr, EnvCoeffs};
use super::filter::Svf;
use super::osc::{Noise, Oscillator};
use crate::params::{FilterMode, LfoTarget, StepDest, Waveform};

/// The maximum number of unison voices per oscillator.
pub const MAX_UNISON: usize = 4;

/// The rate of the drift, in Hz. The drift is a slow random modulation of the
/// pitch and the cutoff.
const DRIFT_RATE: f32 = 0.5;

/// The smoothing factor of the drift value.
const DRIFT_SMOOTHING: f32 = 0.002;

/// The maximum pitch drift, in semitones.
const DRIFT_PITCH: f32 = 0.06;

/// The maximum cutoff drift, in octaves.
const DRIFT_CUTOFF: f32 = 0.2;

/// The frequency ratio of a single unison voice, relative to the base
/// frequency. `offset` is the position of the voice in the unison bank, in the
/// `-1..1` range.
#[inline]
pub fn unison_detune_ratio(detune_cents: f32, offset: f32) -> f32 {
    2f32.powf(detune_cents * offset / 1200.0)
}

/// The position of a unison voice inside the unison bank, in the `-1..1` range.
#[inline]
fn unison_offset(index: usize, count: usize) -> f32 {
    if count <= 1 {
        0.0
    } else {
        (index as f32 * 2.0 / (count - 1) as f32) - 1.0
    }
}

/// The settings of a single oscillator.
#[derive(Debug, Clone, Copy)]
pub struct OscSettings {
    pub waveform: Waveform,
    pub pulse_width: f32,
    pub level: f32,
    /// Octave * 12 + semitone + fine/100.
    pub semitones: f32,
}

impl Default for OscSettings {
    fn default() -> Self {
        Self {
            waveform: Waveform::Saw,
            pulse_width: 0.5,
            level: 0.5,
            semitones: 0.0,
        }
    }
}

/// All the settings a voice needs. These are rebuilt once per audio block.
#[derive(Debug, Clone, Copy)]
pub struct VoiceSettings {
    pub osc1: OscSettings,
    pub osc2: OscSettings,
    pub noise_level: f32,
    pub filter_mode: FilterMode,
    /// `0..1`.
    pub resonance: f32,
    /// The filter envelope amount in octaves, `-5..5`.
    pub filter_env_octaves: f32,
    /// `0..1`.
    pub filter_keytrack: f32,
    /// `0..1`.
    pub filter_drive: f32,
    pub amp_env: EnvCoeffs,
    pub filter_env: EnvCoeffs,
    /// `1..=4`.
    pub unison: usize,
    pub detune_cents: f32,
    /// `0..1`.
    pub spread: f32,
    /// The per-sample glide coefficient, `0..=1`.
    pub glide_step: f32,
    pub lfo1_amount: f32,
    pub lfo1_target: LfoTarget,
    pub lfo2_amount: f32,
    pub lfo2_target: LfoTarget,
    /// How much the velocity of the note opens the filter, `0..1`.
    pub velocity_amount: f32,
    /// How much the voice drifts, `0..1`. The value is already scaled by the
    /// character of the sound.
    pub drift_amount: f32,
    /// The destination of the step sequencer.
    pub step_dest: StepDest,
    /// The amount of the step sequencer, `0..1`. The value that is handed to
    /// the voice is already scaled by it.
    pub step_amount: f32,
    /// Whether the harsh character is active. It adds a bit of filter drive.
    pub character_grit: bool,
}

impl Default for VoiceSettings {
    fn default() -> Self {
        Self {
            osc1: OscSettings::default(),
            osc2: OscSettings {
                semitones: -12.0,
                ..OscSettings::default()
            },
            noise_level: 0.0,
            filter_mode: FilterMode::Lowpass,
            resonance: 0.2,
            filter_env_octaves: 2.0,
            filter_keytrack: 0.3,
            filter_drive: 0.0,
            amp_env: EnvCoeffs::default(),
            filter_env: EnvCoeffs::default(),
            unison: 1,
            detune_cents: 12.0,
            spread: 0.5,
            glide_step: 1.0,
            lfo1_amount: 0.0,
            lfo1_target: LfoTarget::Off,
            lfo2_amount: 0.0,
            lfo2_target: LfoTarget::Off,
            velocity_amount: 0.0,
            drift_amount: 0.0,
            step_dest: StepDest::Filter,
            step_amount: 0.0,
            character_grit: false,
        }
    }
}

/// One synth voice: two oscillators with a unison bank each, an amplitude and
/// a filter envelope and a stereo filter.
pub struct Voice {
    /// The voice ID provided by the host, or a fallback derived from the note.
    voice_id: i32,
    channel: Channel,
    key: Key,
    /// Monotonically increasing ID used to steal the oldest voice.
    internal_voice_id: u64,
    releasing: bool,

    /// Square root of the note velocity, used as an amplitude multiplier.
    velocity: f32,
    /// The velocity of the note, `0..1`.
    velocity_raw: f32,
    /// The pitch the voice is gliding towards, as a MIDI key number.
    target_pitch: f32,
    /// The current pitch, as a MIDI key number.
    current_pitch: f32,
    /// The number of active unison voices per oscillator.
    unison: usize,

    osc1_bank: [Oscillator; MAX_UNISON],
    osc2_bank: [Oscillator; MAX_UNISON],
    noise: Noise,

    amp_env: Adsr,
    filter_env: Adsr,
    filter_left: Svf,
    filter_right: Svf,

    /// The phase of the drift, `0..1`.
    drift_phase: f32,
    /// The value the drift is moving towards, `-1..1`.
    drift_target: f32,
    /// The current drift value, `-1..1`.
    drift_value: f32,
    /// The random number generator of the drift.
    drift_rng: u32,
}

impl Voice {
    pub fn new(
        voice_id: i32,
        channel: Channel,
        key: Key,
        internal_voice_id: u64,
        velocity: f32,
        settings: &VoiceSettings,
    ) -> Self {
        let mut amp_env = Adsr::default();
        amp_env.gate_on();
        let mut filter_env = Adsr::default();
        filter_env.gate_on();

        let pitch = key.number_or_middle_c() as f32;
        let velocity = velocity.clamp(0.0, 1.0);

        // Every voice drifts on its own, so that a chord does not move as one
        // block.
        let mut drift_rng = 0x9E37_79B9 ^ (internal_voice_id as u32).wrapping_mul(0x2545_F491);
        if drift_rng == 0 {
            drift_rng = 0x2545_F491;
        }
        let drift_phase = (next_random(&mut drift_rng) * 0.5 + 0.5).clamp(0.0, 1.0);
        let drift_target = next_random(&mut drift_rng);

        let mut voice = Self {
            voice_id,
            channel,
            key,
            internal_voice_id,
            releasing: false,
            velocity: velocity.sqrt(),
            velocity_raw: velocity,
            target_pitch: pitch,
            current_pitch: pitch,
            unison: settings.unison.clamp(1, MAX_UNISON),
            osc1_bank: [Oscillator::new(); MAX_UNISON],
            osc2_bank: [Oscillator::new(); MAX_UNISON],
            noise: Noise::new(0x9E37_79B9 ^ internal_voice_id as u32),
            amp_env,
            filter_env,
            filter_left: Svf::new(),
            filter_right: Svf::new(),
            drift_phase,
            drift_target,
            drift_value: 0.0,
            drift_rng,
        };

        voice.reset_osc_phases();
        voice
    }

    pub fn voice_id(&self) -> i32 {
        self.voice_id
    }

    /// Changes the voice ID. This is used when a voice is reused for a new
    /// note in mono and legato mode.
    pub fn set_voice_id(&mut self, voice_id: i32) {
        self.voice_id = voice_id;
    }

    pub fn channel(&self) -> Channel {
        self.channel
    }

    pub fn key(&self) -> Key {
        self.key
    }

    pub fn internal_voice_id(&self) -> u64 {
        self.internal_voice_id
    }

    pub fn is_releasing(&self) -> bool {
        self.releasing
    }

    /// Changes the played key. When `retrigger` is set, both envelopes are
    /// restarted, otherwise the pitch glides to the new key.
    pub fn set_key(&mut self, key: Key, settings: &VoiceSettings, retrigger: bool) {
        self.key = key;
        self.target_pitch = key.number_or_middle_c() as f32;

        if retrigger {
            self.amp_env.gate_on();
            self.filter_env.gate_on();
            self.releasing = false;
        }

        let unison = settings.unison.clamp(1, MAX_UNISON);
        if unison != self.unison {
            self.unison = unison;
            self.reset_osc_phases();
        }
    }

    /// Puts the voice into its release stage.
    pub fn release(&mut self) {
        self.releasing = true;
        self.amp_env.gate_off();
        self.filter_env.gate_off();
    }

    /// Whether the voice has completely died.
    #[inline]
    pub fn is_finished(&self) -> bool {
        self.amp_env.is_finished()
    }

    /// Renders a single stereo sample. `lfo1` and `lfo2` are the current global
    /// LFO values in the `-1..1` range. `step_value` is the value of the step
    /// sequencer, already scaled by its amount.
    #[inline]
    #[allow(clippy::too_many_arguments)]
    pub fn next_sample(
        &mut self,
        settings: &VoiceSettings,
        cutoff_hz: f32,
        lfo1: f32,
        lfo2: f32,
        step_value: f32,
        sample_rate: f32,
    ) -> (f32, f32) {
        let sample_rate = sample_rate.max(1.0);

        // The envelopes.
        let amp = self.amp_env.next(&settings.amp_env);
        let filter_env = self.filter_env.next(&settings.filter_env);

        // The LFO destinations.
        let (pitch_semitones, filter_octaves, amp_scale, pulse_width_offset) =
            lfo_modulation(settings, lfo1, lfo2);

        // The step sequencer.
        let (step_pitch, step_filter, step_shape) = step_modulation(settings, step_value);
        let pulse_width_offset = pulse_width_offset + step_shape;

        // The drift.
        self.update_drift(sample_rate);
        let drift_pitch = self.drift_value * settings.drift_amount * DRIFT_PITCH;
        let drift_filter = self.drift_value * settings.drift_amount * DRIFT_CUTOFF;

        // The pitch, with optional glide.
        self.current_pitch += (self.target_pitch - self.current_pitch) * settings.glide_step;
        let base_frequency = util::f32_midi_key_to_freq(
            self.current_pitch + pitch_semitones + step_pitch + drift_pitch,
        );

        let unison = self.unison.clamp(1, MAX_UNISON);
        let unison_gain = 1.0 / (unison as f32).sqrt();

        let mut left = 0.0;
        let mut right = 0.0;

        render_oscillator(
            &mut self.osc1_bank,
            &mut left,
            &mut right,
            &settings.osc1,
            base_frequency,
            unison,
            unison_gain,
            settings.detune_cents,
            settings.spread,
            pulse_width_offset,
            sample_rate,
        );
        render_oscillator(
            &mut self.osc2_bank,
            &mut left,
            &mut right,
            &settings.osc2,
            base_frequency,
            unison,
            unison_gain,
            settings.detune_cents,
            settings.spread,
            pulse_width_offset,
            sample_rate,
        );

        // The noise generator is shared by both channels. It is kept at half
        // scale, so that it doesn't dominate the mix.
        let noise = self.noise.next() * settings.noise_level * 0.5;
        left += noise;
        right += noise;

        // The filter.
        let key = self.key.number_or_middle_c() as f32;
        let octaves = settings.filter_env_octaves * filter_env
            + settings.filter_keytrack * (key - 60.0) / 12.0
            + settings.velocity_amount * self.velocity_raw * 1.5
            + filter_octaves
            + step_filter
            + drift_filter;
        let cutoff = (cutoff_hz * 2f32.powf(octaves)).clamp(20.0, sample_rate * 0.45);
        let g = (PI * cutoff / sample_rate).tan();
        let k = 2.0 - 1.98 * settings.resonance;

        // The harsh character adds a bit of drive to the filter.
        let drive = if settings.character_grit {
            (settings.filter_drive + 0.12).clamp(0.0, 1.0)
        } else {
            settings.filter_drive.clamp(0.0, 1.0)
        };

        let left = self
            .filter_left
            .process(left, g, k, settings.filter_mode, drive);
        let right = self
            .filter_right
            .process(right, g, k, settings.filter_mode, drive);

        // The amplifier.
        let gain = amp * self.velocity * amp_scale;
        (left * gain, right * gain)
    }

    /// Gives every unison oscillator a different starting phase so that the
    /// unison voices don't start in lockstep.
    fn reset_osc_phases(&mut self) {
        for index in 0..MAX_UNISON {
            let phase = (index as f32 * 0.37).fract();
            self.osc1_bank[index].set_phase(phase);
            self.osc2_bank[index].set_phase(phase);
        }
    }

    /// Moves the drift towards its new target. Every time the phase of the
    /// drift wraps around, a new target is picked.
    #[inline]
    fn update_drift(&mut self, sample_rate: f32) {
        self.drift_phase += DRIFT_RATE / sample_rate;
        if self.drift_phase >= 1.0 {
            self.drift_phase -= 1.0;
            self.drift_target = next_random(&mut self.drift_rng);
        }
        self.drift_value += (self.drift_target - self.drift_value) * DRIFT_SMOOTHING;
    }
}

/// A random number in the `-1..1` range.
#[inline]
fn next_random(rng: &mut u32) -> f32 {
    let mut x = *rng;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *rng = x;

    // Use the upper 24 bits so that the full float precision is used.
    ((x >> 8) as f32 / 16_777_215.0) * 2.0 - 1.0
}

/// Renders a single oscillator with its unison bank into the stereo output.
#[allow(clippy::too_many_arguments)]
#[inline]
fn render_oscillator(
    bank: &mut [Oscillator; MAX_UNISON],
    left: &mut f32,
    right: &mut f32,
    osc: &OscSettings,
    base_frequency: f32,
    unison: usize,
    unison_gain: f32,
    detune_cents: f32,
    spread: f32,
    pulse_width_offset: f32,
    sample_rate: f32,
) {
    let pulse_width = (osc.pulse_width + pulse_width_offset).clamp(0.05, 0.95);
    let frequency = base_frequency * 2f32.powf(osc.semitones);

    for (index, oscillator) in bank.iter_mut().take(unison).enumerate() {
        let offset = unison_offset(index, unison);
        let ratio = unison_detune_ratio(detune_cents, offset);
        let phase_delta = (frequency * ratio / sample_rate).clamp(0.0, 0.5);

        let sample = oscillator.next(phase_delta, osc.waveform, pulse_width);

        // Equal power panning.
        let pan = (offset * spread).clamp(-1.0, 1.0);
        let angle = (pan + 1.0) * PI * 0.25;
        let gain_left = angle.cos();
        let gain_right = angle.sin();

        let level = osc.level * unison_gain;
        *left += sample * gain_left * level;
        *right += sample * gain_right * level;
    }
}

/// Calculates the modulation of the two global LFOs.
///
/// Returns the pitch offset in semitones, the filter offset in octaves, the
/// amplitude scale and the pulse width offset.
#[inline]
fn lfo_modulation(settings: &VoiceSettings, lfo1: f32, lfo2: f32) -> (f32, f32, f32, f32) {
    let mut pitch_semitones = 0.0;
    let mut filter_octaves = 0.0;
    let mut amp_scale = 1.0;
    let mut pulse_width_offset = 0.0;

    for (value, target, amount) in [
        (lfo1, settings.lfo1_target, settings.lfo1_amount),
        (lfo2, settings.lfo2_target, settings.lfo2_amount),
    ] {
        match target {
            LfoTarget::Off => {}
            LfoTarget::Pitch => pitch_semitones += value * amount * 2.0,
            LfoTarget::Filter => filter_octaves += value * amount * 4.0,
            LfoTarget::Amp => amp_scale *= 1.0 - amount * (0.5 - 0.5 * value),
            LfoTarget::PulseWidth => pulse_width_offset += value * amount * 0.4,
        }
    }

    (
        pitch_semitones,
        filter_octaves,
        amp_scale,
        pulse_width_offset,
    )
}

/// Calculates the modulation of the step sequencer.
///
/// Returns the pitch offset in semitones, the filter offset in octaves and the
/// pulse width offset.
#[inline]
fn step_modulation(settings: &VoiceSettings, step_value: f32) -> (f32, f32, f32) {
    match settings.step_dest {
        StepDest::Pitch => (step_value * 2.0, 0.0, 0.0),
        StepDest::Filter => (0.0, step_value * 4.0, 0.0),
        StepDest::Shape => (0.0, 0.0, step_value * 0.4),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_RATE: f32 = 48_000.0;

    fn voice(settings: &VoiceSettings) -> Voice {
        Voice::new(1, Channel::Number(0), Key::Number(60), 0, 1.0, settings)
    }

    #[test]
    fn a_voice_produces_audio() {
        let settings = VoiceSettings::default();
        let mut voice = voice(&settings);

        let mut peak: f32 = 0.0;
        for _ in 0..4_800 {
            let (left, right) = voice.next_sample(&settings, 3_000.0, 0.0, 0.0, 0.0, SAMPLE_RATE);
            assert!(left.is_finite() && right.is_finite());
            peak = peak.max(left.abs()).max(right.abs());
        }

        assert!(peak > 0.1, "the voice was silent (peak {peak})");
    }

    #[test]
    fn unison_voices_are_detuned() {
        // The two unison voices of a bank are detuned in opposite directions.
        let low = unison_detune_ratio(50.0, -1.0);
        let high = unison_detune_ratio(50.0, 1.0);
        assert!((low - high).abs() > 1e-3);
        assert!(low < 1.0 && high > 1.0);

        // A unison voice therefore sounds different from a single oscillator.
        let single = VoiceSettings {
            unison: 1,
            detune_cents: 50.0,
            ..VoiceSettings::default()
        };
        let unison = VoiceSettings {
            unison: 2,
            detune_cents: 50.0,
            ..VoiceSettings::default()
        };

        let mut voice_single = voice(&single);
        let mut voice_unison = voice(&unison);

        let mut difference = 0.0;
        for _ in 0..512 {
            let (a, _) = voice_single.next_sample(&single, 2_000.0, 0.0, 0.0, 0.0, SAMPLE_RATE);
            let (b, _) = voice_unison.next_sample(&unison, 2_000.0, 0.0, 0.0, 0.0, SAMPLE_RATE);
            difference += (a - b).abs();
        }

        assert!(
            difference > 0.1,
            "the unison voices did not differ (difference {difference})"
        );
    }

    #[test]
    fn releasing_a_voice_finishes_it() {
        let settings = VoiceSettings::default();
        let mut voice = voice(&settings);

        for _ in 0..480 {
            voice.next_sample(&settings, 3_000.0, 0.0, 0.0, 0.0, SAMPLE_RATE);
        }
        assert!(!voice.is_finished());

        voice.release();
        assert!(voice.is_releasing());

        for _ in 0..48_000 {
            voice.next_sample(&settings, 3_000.0, 0.0, 0.0, 0.0, SAMPLE_RATE);
            if voice.is_finished() {
                break;
            }
        }
        assert!(voice.is_finished());
    }

    #[test]
    fn gliding_voices_move_towards_the_new_key() {
        let settings = VoiceSettings {
            glide_step: 0.01,
            ..VoiceSettings::default()
        };
        let mut gliding = voice(&settings);
        let mut direct = Voice::new(2, Channel::Number(0), Key::Number(72), 1, 1.0, &settings);

        gliding.set_key(Key::Number(72), &settings, false);
        assert_eq!(gliding.key(), Key::Number(72));

        // The gliding voice has not reached the new key yet, so it sounds
        // different from a voice that starts on that key.
        let mut difference = 0.0;
        for _ in 0..480 {
            let (a, _) = gliding.next_sample(&settings, 3_000.0, 0.0, 0.0, 0.0, SAMPLE_RATE);
            let (b, _) = direct.next_sample(&settings, 3_000.0, 0.0, 0.0, 0.0, SAMPLE_RATE);
            difference += (a - b).abs();
        }

        assert!(
            difference > 0.01,
            "the voice did not glide (difference {difference})"
        );
    }

    #[test]
    fn the_drift_stays_bounded() {
        let settings = VoiceSettings {
            drift_amount: 1.0,
            ..VoiceSettings::default()
        };
        let mut voice = voice(&settings);

        // The drift is slow, so a few seconds are enough to see it move.
        for _ in 0..(SAMPLE_RATE as usize * 4) {
            let (left, right) = voice.next_sample(&settings, 3_000.0, 0.0, 0.0, 0.0, SAMPLE_RATE);
            assert!(left.is_finite() && right.is_finite());
            assert!(
                (-1.0..=1.0).contains(&voice.drift_value),
                "the drift left its range: {}",
                voice.drift_value
            );
            assert!(
                (-1.0..=1.0).contains(&voice.drift_target),
                "the drift target left its range: {}",
                voice.drift_target
            );
        }

        // The drift has to be audible, so it must have moved a little.
        assert!(voice.drift_value.abs() > 0.001);
    }

    #[test]
    fn the_step_modulates_the_pitch() {
        let settings = VoiceSettings {
            step_dest: StepDest::Pitch,
            ..VoiceSettings::default()
        };
        let mut plain = voice(&settings);
        let mut stepped = voice(&settings);

        let mut difference = 0.0;
        for _ in 0..512 {
            let (a, _) = plain.next_sample(&settings, 3_000.0, 0.0, 0.0, 0.0, SAMPLE_RATE);
            let (b, _) = stepped.next_sample(&settings, 3_000.0, 0.0, 0.0, 1.0, SAMPLE_RATE);
            difference += (a - b).abs();
        }

        assert!(
            difference > 0.1,
            "the step did not change the pitch (difference {difference})"
        );
    }

    #[test]
    fn the_step_modulates_the_filter() {
        let settings = VoiceSettings {
            step_dest: StepDest::Filter,
            ..VoiceSettings::default()
        };
        let mut plain = voice(&settings);
        let mut stepped = voice(&settings);

        let mut difference = 0.0;
        for _ in 0..512 {
            let (a, _) = plain.next_sample(&settings, 500.0, 0.0, 0.0, 0.0, SAMPLE_RATE);
            let (b, _) = stepped.next_sample(&settings, 500.0, 0.0, 0.0, 1.0, SAMPLE_RATE);
            difference += (a - b).abs();
        }

        assert!(
            difference > 0.01,
            "the step did not change the filter (difference {difference})"
        );
    }

    #[test]
    fn the_velocity_opens_the_filter() {
        let settings = VoiceSettings {
            velocity_amount: 1.0,
            filter_env_octaves: 0.0,
            ..VoiceSettings::default()
        };

        let mut quiet = Voice::new(1, Channel::Number(0), Key::Number(60), 0, 0.1, &settings);
        let mut loud = Voice::new(2, Channel::Number(0), Key::Number(60), 1, 1.0, &settings);

        let mut difference = 0.0;
        for _ in 0..512 {
            let (a, _) = quiet.next_sample(&settings, 300.0, 0.0, 0.0, 0.0, SAMPLE_RATE);
            let (b, _) = loud.next_sample(&settings, 300.0, 0.0, 0.0, 0.0, SAMPLE_RATE);
            difference += (a - b).abs();
        }

        assert!(
            difference > 0.01,
            "the velocity did not change the sound (difference {difference})"
        );
    }

    #[test]
    fn the_step_modulation_math_is_sane() {
        let settings = VoiceSettings {
            step_dest: StepDest::Pitch,
            ..VoiceSettings::default()
        };
        assert_eq!(step_modulation(&settings, 1.0), (2.0, 0.0, 0.0));
        assert_eq!(step_modulation(&settings, -1.0), (-2.0, 0.0, 0.0));

        let settings = VoiceSettings {
            step_dest: StepDest::Filter,
            ..VoiceSettings::default()
        };
        assert_eq!(step_modulation(&settings, 1.0), (0.0, 4.0, 0.0));
        assert_eq!(step_modulation(&settings, -1.0), (0.0, -4.0, 0.0));

        let settings = VoiceSettings {
            step_dest: StepDest::Shape,
            ..VoiceSettings::default()
        };
        assert_eq!(step_modulation(&settings, 1.0), (0.0, 0.0, 0.4));
        assert_eq!(step_modulation(&settings, -1.0), (0.0, 0.0, -0.4));
    }
}
