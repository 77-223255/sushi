//! A single synth voice: two oscillators, a unison bank, envelopes and a filter.

use nice_plug::midi::{Channel, Key};
use nice_plug::util;
use std::f32::consts::PI;

use super::adsr::{Adsr, EnvCoeffs};
use super::filter::Svf;
use super::osc::{Noise, Oscillator};
use crate::params::{FilterMode, LfoTarget, Waveform};

/// The maximum number of unison voices per oscillator.
pub const MAX_UNISON: usize = 4;

/// The number of samples between two updates of the pitch and of the filter.
/// The ear does not notice the difference, but the CPU does.
const CONTROL_RATE: u32 = 8;

/// The rate of the drift, in Hz. The drift is a slow random modulation of the
/// pitch and the cutoff.
const DRIFT_RATE: f32 = 0.5;

/// The smoothing factor of the drift value.
const DRIFT_SMOOTHING: f32 = 0.002;

/// The maximum pitch drift, in semitones.
const DRIFT_PITCH: f32 = 0.06;

/// The maximum cutoff drift, in octaves.
const DRIFT_CUTOFF: f32 = 0.2;

/// The frequency of the DC blocker, in Hz.
const DC_BLOCKER_FREQUENCY: f32 = 20.0;

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
            character_grit: false,
        }
    }
}

/// A one-pole highpass that takes the DC offset out of the signal.
#[derive(Debug, Clone, Copy, Default)]
struct DcBlocker {
    /// The previous input sample.
    input: f32,
    /// The previous output sample.
    output: f32,
    /// The feedback of the filter, which is the one of a highpass at 20 Hz.
    coefficient: f32,
    /// The sample rate that the coefficient was computed for.
    rate: f32,
}

impl DcBlocker {
    #[inline]
    fn process(&mut self, input: f32, sample_rate: f32) -> f32 {
        if self.rate != sample_rate {
            self.rate = sample_rate;
            self.coefficient = (-2.0 * PI * DC_BLOCKER_FREQUENCY / sample_rate.max(1.0)).exp();
        }

        let output = input - self.input + self.coefficient * self.output;
        self.input = input;

        // Flush the denormal state, so that the filter doesn't drag the CPU
        // down with it once the signal has died away.
        self.output = if output.abs() < 1e-20 { 0.0 } else { output };
        output
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
    dc_left: DcBlocker,
    dc_right: DcBlocker,

    /// The counter of the control rate. The pitch and the filter are only
    /// updated every eighth sample.
    control_counter: u32,
    /// The current level of the filter envelope.
    filter_env_level: f32,
    /// The phase increment of the unison voices of the first oscillator.
    phase_delta1: [f32; MAX_UNISON],
    /// The phase increment of the unison voices of the second oscillator.
    phase_delta2: [f32; MAX_UNISON],
    /// The coefficient of the filter.
    g: f32,
    /// The damping of the filter.
    k: f32,

    /// The phase of the drift, `0..1`.
    drift_phase: f32,
    /// The value the drift is moving towards, `-1..1`.
    drift_target: f32,
    /// The current drift value, `-1..1`.
    drift_value: f32,
    /// The random number generator of the drift.
    drift_rng: u32,
    /// The random number generator of the phases of the oscillators.
    phase_rng: u32,
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

        // The phases of the oscillators are random as well, so that a chord
        // does not cancel itself out.
        let mut phase_rng = 0x2545_F491 ^ (internal_voice_id as u32).wrapping_mul(0x9E37_79B9);
        if phase_rng == 0 {
            phase_rng = 0x2545_F491;
        }

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
            dc_left: DcBlocker::default(),
            dc_right: DcBlocker::default(),
            control_counter: CONTROL_RATE - 1,
            filter_env_level: 0.0,
            phase_delta1: [0.0; MAX_UNISON],
            phase_delta2: [0.0; MAX_UNISON],
            g: 0.0,
            k: 2.0,
            drift_phase,
            drift_target,
            drift_value: 0.0,
            drift_rng,
            phase_rng,
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
    /// LFO values in the `-1..1` range.
    #[inline]
    pub fn next_sample(
        &mut self,
        settings: &VoiceSettings,
        cutoff_hz: f32,
        lfo1: f32,
        lfo2: f32,
        sample_rate: f32,
    ) -> (f32, f32) {
        let sample_rate = sample_rate.max(1.0);

        // The envelopes.
        let amp = self.amp_env.next(&settings.amp_env);
        self.filter_env_level = self.filter_env.next(&settings.filter_env);

        // The LFO destinations.
        let (pitch_semitones, filter_octaves, amp_scale, pulse_width_offset) =
            lfo_modulation(settings, lfo1, lfo2);

        // The drift.
        self.update_drift(sample_rate);

        // The control rate. The pitch and the filter are updated every eighth
        // sample only, which saves a lot of CPU time.
        self.control_counter = (self.control_counter + 1) % CONTROL_RATE;
        if self.control_counter == 0 {
            self.update_control(
                settings,
                cutoff_hz,
                pitch_semitones,
                filter_octaves,
                sample_rate,
            );
        }

        let unison = self.unison.clamp(1, MAX_UNISON);
        let unison_gain = 1.0 / (unison as f32).sqrt();

        let mut left = 0.0;
        let mut right = 0.0;

        render_oscillator(
            &mut self.osc1_bank,
            &mut left,
            &mut right,
            &settings.osc1,
            &self.phase_delta1,
            unison,
            unison_gain,
            settings.spread,
            pulse_width_offset,
        );
        render_oscillator(
            &mut self.osc2_bank,
            &mut left,
            &mut right,
            &settings.osc2,
            &self.phase_delta2,
            unison,
            unison_gain,
            settings.spread,
            pulse_width_offset,
        );

        // The noise generator is shared by both channels. It is kept at half
        // scale, so that it doesn't dominate the mix.
        let noise = self.noise.next() * settings.noise_level * 0.5;
        left += noise;
        right += noise;

        // A high resonance would make the filter gain a lot of energy, so the
        // input of the filter is turned down a little.
        let attenuation = 1.0 / (1.0 + 0.35 * settings.resonance);

        // The harsh character adds a bit of drive to the filter.
        let drive = if settings.character_grit {
            (settings.filter_drive + 0.12).clamp(0.0, 1.0)
        } else {
            settings.filter_drive.clamp(0.0, 1.0)
        };

        let left = self.filter_left.process(
            left * attenuation,
            self.g,
            self.k,
            settings.filter_mode,
            drive,
        );
        let right = self.filter_right.process(
            right * attenuation,
            self.g,
            self.k,
            settings.filter_mode,
            drive,
        );

        // The DC blocker, which takes the offset of the wave out of the
        // signal.
        let left = self.dc_left.process(left, sample_rate);
        let right = self.dc_right.process(right, sample_rate);

        // The amplifier.
        let gain = amp * self.velocity * amp_scale;
        (left * gain, right * gain)
    }

    /// Updates the pitch and the filter of the voice. This only happens every
    /// eighth sample.
    fn update_control(
        &mut self,
        settings: &VoiceSettings,
        cutoff_hz: f32,
        pitch_semitones: f32,
        filter_octaves: f32,
        sample_rate: f32,
    ) {
        // The pitch, with optional glide.
        self.current_pitch += (self.target_pitch - self.current_pitch) * settings.glide_step;
        let drift_pitch = self.drift_value * settings.drift_amount * DRIFT_PITCH;
        let base_frequency =
            util::f32_midi_key_to_freq(self.current_pitch + pitch_semitones + drift_pitch);

        // The filter.
        let key = self.key.number_or_middle_c() as f32;
        let drift_filter = self.drift_value * settings.drift_amount * DRIFT_CUTOFF;
        let octaves = settings.filter_env_octaves * self.filter_env_level
            + settings.filter_keytrack * (key - 60.0) / 12.0
            + settings.velocity_amount * self.velocity_raw * 1.5
            + filter_octaves
            + drift_filter;
        let cutoff = (cutoff_hz * 2f32.powf(octaves)).clamp(20.0, sample_rate * 0.45);
        self.g = (PI * cutoff / sample_rate).tan();
        self.k = 2.0 - 1.98 * settings.resonance;

        // The phase increments of the unison voices.
        let unison = self.unison.clamp(1, MAX_UNISON);
        let frequency1 = base_frequency * 2f32.powf(settings.osc1.semitones / 12.0);
        let frequency2 = base_frequency * 2f32.powf(settings.osc2.semitones / 12.0);
        for index in 0..unison {
            let offset = unison_offset(index, unison);
            let ratio = unison_detune_ratio(settings.detune_cents, offset);
            self.phase_delta1[index] = (frequency1 * ratio / sample_rate).clamp(0.0, 0.5);
            self.phase_delta2[index] = (frequency2 * ratio / sample_rate).clamp(0.0, 0.5);
        }
    }

    /// Gives every unison oscillator a random starting phase, so that the
    /// unison voices do not start in lockstep and a chord does not cancel
    /// itself out. The phases are random, but they are the same for every run
    /// of the same voice.
    fn reset_osc_phases(&mut self) {
        for index in 0..MAX_UNISON {
            let phase1 = next_random(&mut self.phase_rng) * 0.5 + 0.5;
            let phase2 = next_random(&mut self.phase_rng) * 0.5 + 0.5;
            self.osc1_bank[index].set_phase(phase1);
            self.osc2_bank[index].set_phase(phase2);
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
    phase_deltas: &[f32; MAX_UNISON],
    unison: usize,
    unison_gain: f32,
    spread: f32,
    pulse_width_offset: f32,
) {
    let pulse_width = (osc.pulse_width + pulse_width_offset).clamp(0.05, 0.95);

    for (index, oscillator) in bank.iter_mut().take(unison).enumerate() {
        let offset = unison_offset(index, unison);
        let sample = oscillator.next(phase_deltas[index], osc.waveform, pulse_width);

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

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_RATE: f32 = 48_000.0;

    fn voice(settings: &VoiceSettings) -> Voice {
        Voice::new(1, Channel::Number(0), Key::Number(60), 0, 1.0, settings)
    }

    /// The frequency of the voice, measured by its zero crossings.
    fn measure_frequency(voice: &mut Voice, settings: &VoiceSettings) -> f32 {
        let mut crossings = 0;
        let mut previous = 0.0;
        for _ in 0..(SAMPLE_RATE as usize * 2) {
            let (left, _) = voice.next_sample(settings, 8_000.0, 0.0, 0.0, SAMPLE_RATE);
            if previous < 0.0 && left >= 0.0 {
                crossings += 1;
            }
            previous = left;
        }
        crossings as f32 / 2.0
    }

    #[test]
    fn the_oscillators_are_tuned_to_their_octave() {
        // The first oscillator plays the note itself.
        let settings = VoiceSettings {
            osc1: OscSettings {
                waveform: Waveform::Sine,
                level: 1.0,
                ..OscSettings::default()
            },
            osc2: OscSettings {
                waveform: Waveform::Sine,
                level: 0.0,
                ..OscSettings::default()
            },
            filter_env_octaves: 0.0,
            ..VoiceSettings::default()
        };
        let mut first = voice(&settings);
        let frequency = measure_frequency(&mut first, &settings);

        // The key 60 is a small C, which is 261.6 Hz.
        assert!(
            (frequency - 261.6).abs() < 5.0,
            "the oscillator played at {frequency} Hz"
        );

        // The second oscillator is an octave lower.
        let settings = VoiceSettings {
            osc1: OscSettings {
                waveform: Waveform::Sine,
                level: 0.0,
                ..OscSettings::default()
            },
            osc2: OscSettings {
                waveform: Waveform::Sine,
                level: 1.0,
                semitones: -12.0,
                pulse_width: 0.5,
            },
            filter_env_octaves: 0.0,
            ..VoiceSettings::default()
        };
        let mut second = voice(&settings);
        let frequency = measure_frequency(&mut second, &settings);

        assert!(
            (frequency - 130.8).abs() < 5.0,
            "the oscillator played at {frequency} Hz"
        );
    }

    #[test]
    fn a_voice_produces_audio() {
        let settings = VoiceSettings::default();
        let mut voice = voice(&settings);

        let mut peak: f32 = 0.0;
        for _ in 0..4_800 {
            let (left, right) = voice.next_sample(&settings, 3_000.0, 0.0, 0.0, SAMPLE_RATE);
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
            let (a, _) = voice_single.next_sample(&single, 2_000.0, 0.0, 0.0, SAMPLE_RATE);
            let (b, _) = voice_unison.next_sample(&unison, 2_000.0, 0.0, 0.0, SAMPLE_RATE);
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
            voice.next_sample(&settings, 3_000.0, 0.0, 0.0, SAMPLE_RATE);
        }
        assert!(!voice.is_finished());

        voice.release();
        assert!(voice.is_releasing());

        for _ in 0..48_000 {
            voice.next_sample(&settings, 3_000.0, 0.0, 0.0, SAMPLE_RATE);
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
            let (a, _) = gliding.next_sample(&settings, 3_000.0, 0.0, 0.0, SAMPLE_RATE);
            let (b, _) = direct.next_sample(&settings, 3_000.0, 0.0, 0.0, SAMPLE_RATE);
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
            let (left, right) = voice.next_sample(&settings, 3_000.0, 0.0, 0.0, SAMPLE_RATE);
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
    fn the_voices_start_with_different_phases() {
        let settings = VoiceSettings {
            unison: 4,
            noise_level: 0.0,
            drift_amount: 0.0,
            ..VoiceSettings::default()
        };

        let mut first = Voice::new(1, Channel::Number(0), Key::Number(60), 0, 1.0, &settings);
        let mut second = Voice::new(2, Channel::Number(0), Key::Number(60), 1, 1.0, &settings);

        // The phases of the two voices are not the same, so that a chord does
        // not cancel itself out.
        let mut difference = 0.0;
        for _ in 0..512 {
            let (a, _) = first.next_sample(&settings, 3_000.0, 0.0, 0.0, SAMPLE_RATE);
            let (b, _) = second.next_sample(&settings, 3_000.0, 0.0, 0.0, SAMPLE_RATE);
            difference += (a - b).abs();
        }

        assert!(
            difference > 0.01,
            "the voices started with the same phases (difference {difference})"
        );
    }

    #[test]
    fn the_voices_are_reproducible() {
        let settings = VoiceSettings {
            unison: 4,
            noise_level: 0.0,
            drift_amount: 0.0,
            ..VoiceSettings::default()
        };

        let mut first = Voice::new(1, Channel::Number(0), Key::Number(60), 7, 1.0, &settings);
        let mut second = Voice::new(2, Channel::Number(0), Key::Number(60), 7, 1.0, &settings);

        // The same voice always gets the same phases.
        for _ in 0..512 {
            let (a, _) = first.next_sample(&settings, 3_000.0, 0.0, 0.0, SAMPLE_RATE);
            let (b, _) = second.next_sample(&settings, 3_000.0, 0.0, 0.0, SAMPLE_RATE);
            assert_eq!(a, b);
        }
    }

    #[test]
    fn the_dc_blocker_removes_the_offset_of_the_wave() {
        // A pulse wave with a duty cycle of 25 % has a lot of DC offset.
        let settings = VoiceSettings {
            osc1: OscSettings {
                waveform: Waveform::Pulse,
                pulse_width: 0.25,
                level: 1.0,
                semitones: 0.0,
            },
            osc2: OscSettings {
                level: 0.0,
                ..OscSettings::default()
            },
            ..VoiceSettings::default()
        };
        let mut voice = voice(&settings);

        // The envelope has to settle first.
        for _ in 0..4_800 {
            voice.next_sample(&settings, 3_000.0, 0.0, 0.0, SAMPLE_RATE);
        }

        let mut sum = 0.0;
        let mut count = 0.0;
        for _ in 0..4_800 {
            let (left, _) = voice.next_sample(&settings, 3_000.0, 0.0, 0.0, SAMPLE_RATE);
            sum += left;
            count += 1.0;
        }
        let offset = sum / count;

        assert!(
            offset.abs() < 0.02,
            "the wave still had an offset of {offset}"
        );
    }

    #[test]
    fn the_dc_blocker_passes_audio() {
        let mut dc = DcBlocker::default();
        let mut input = 0.0f64;
        let mut output = 0.0f64;
        for i in 0..48_000 {
            let x = (2.0 * PI * 261.63 * i as f32 / SAMPLE_RATE).sin();
            let y = dc.process(x, SAMPLE_RATE);
            if i >= 4_800 {
                input += (x * x) as f64;
                output += (y * y) as f64;
            }
        }
        let ratio = (output / input).sqrt();
        assert!(
            (ratio - 1.0).abs() < 0.01,
            "DC blocker passband ratio {ratio}"
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
            let (a, _) = quiet.next_sample(&settings, 300.0, 0.0, 0.0, SAMPLE_RATE);
            let (b, _) = loud.next_sample(&settings, 300.0, 0.0, 0.0, SAMPLE_RATE);
            difference += (a - b).abs();
        }

        assert!(
            difference > 0.01,
            "the velocity did not change the sound (difference {difference})"
        );
    }

    #[test]
    fn the_resonance_does_not_blow_the_filter_up() {
        let settings = VoiceSettings {
            resonance: 1.0,
            filter_env_octaves: 0.0,
            ..VoiceSettings::default()
        };
        let mut voice = voice(&settings);

        let mut peak: f32 = 0.0;
        for _ in 0..4_800 {
            let (left, right) = voice.next_sample(&settings, 800.0, 0.0, 0.0, SAMPLE_RATE);
            assert!(left.is_finite() && right.is_finite());
            peak = peak.max(left.abs()).max(right.abs());
        }

        assert!(peak < 25.0, "the filter gained too much energy ({peak})");
    }
}
