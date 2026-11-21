
use nice_plug::midi::{Channel, Key};
use nice_plug::util;
use std::f32::consts::PI;

use super::adsr::{Adsr, EnvCoeffs, shape_envelope};
use super::filter::Svf;
use super::mod_matrix::{ModSources, Modulation, evaluate, key_source};
use super::osc::{Noise, Oscillator};
use crate::params::{
    FilterMode, FilterSlope, LfoTarget, MOD_SLOTS, ModDest, ModSlot, ModSource, Waveform,
};

/// The maximum number of unison voices per oscillator.
pub const MAX_UNISON: usize = 4;

/// The number of samples between two updates of the pitch and of the filter.
/// The ear does not notice the difference, but the CPU does.
pub const CONTROL_RATE: u32 = 8;

/// The rate of the drift, in Hz. The drift is a slow random modulation of the
/// pitch and the cutoff.
const DRIFT_RATE: f32 = 0.5;

/// The smoothing factor of the drift value.
const DRIFT_SMOOTHING: f32 = 0.002;

/// The maximum pitch drift, in semitones.
const DRIFT_PITCH: f32 = 0.06;

/// The maximum cutoff drift, in octaves.
const DRIFT_CUTOFF: f32 = 0.2;

/// The drift rate of the legacy flavor, in Hz. The old engine wandered more
/// slowly than the modern one.
const LEGACY_DRIFT_RATE: f32 = 0.15;

/// The smoothing factor of the drift in the legacy flavor. A smaller value
/// makes the drift wander instead of stepping towards its target.
const LEGACY_DRIFT_SMOOTHING: f32 = 0.0006;

/// The maximum pitch error of the component tolerances of a legacy voice, in
/// semitones. It is 1.5 cents.
const TOLERANCE_PITCH: f32 = 0.015;

/// The maximum gain error of the component tolerances of a legacy voice, as a
/// linear fraction. It is 1.5 %.
const TOLERANCE_GAIN: f32 = 0.015;

/// The maximum cutoff error of the component tolerances of a legacy voice, in
/// octaves. It is about 2 % in frequency.
const TOLERANCE_CUTOFF: f32 = 0.0286;

/// The amount of the asymmetric quadratic term of the legacy flavor. It adds a
/// touch of even harmonics without a `tanh` in the per-sample path.
const LEGACY_WARMTH: f32 = 0.03;

/// Compensates the tiny RMS increase of the quadratic term for a full-scale
/// sine. The term is second order in `LEGACY_WARMTH`, so this is close to one.
const LEGACY_WARMTH_NORMALIZER: f32 = 1.0 / (1.0 + 0.375 * LEGACY_WARMTH * LEGACY_WARMTH);

/// The frequency of the DC blocker, in Hz.
const DC_BLOCKER_FREQUENCY: f32 = 20.0;

/// The corner frequency of the fixed high-frequency loss of the legacy
/// flavor, in Hz.
const LEGACY_HF_CORNER: f32 = 6_000.0;

/// The high-frequency gain of the legacy shelf. 0.785 puts the top octave
/// around -1 to -1.3 dB down, which is subtle but audible on bright sounds.
const LEGACY_HF_GAIN: f32 = 0.785;

#[inline]
pub fn unison_detune_ratio(detune_cents: f32, offset: f32) -> f32 {
    2f32.powf(detune_cents * offset / 1200.0)
}

#[inline]
fn unison_offset(index: usize, count: usize) -> f32 {
    if count <= 1 {
        0.0
    } else {
        (index as f32 * 2.0 / (count - 1) as f32) - 1.0
    }
}

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

#[derive(Debug, Clone, Copy)]
pub struct VoiceSettings {
    pub osc1: OscSettings,
    pub osc2: OscSettings,
    /// The level of the square sub oscillator, one octave below VCO 1.
    pub sub_level: f32,
    /// Whether VCO 1 is hard synced to VCO 2.
    pub osc_sync: bool,
    /// The phase modulation depth from VCO 2 into VCO 1.
    pub fm_amount: f32,
    pub noise_level: f32,
    pub filter_mode: FilterMode,
    /// Whether the filter runs one or two cascaded stages.
    pub filter_slope: FilterSlope,
    /// `0..1`.
    pub resonance: f32,
    /// The filter envelope amount in octaves, `-5..5`.
    pub filter_env_octaves: f32,
    /// `0..1`.
    pub filter_keytrack: f32,
    /// `0..1`.
    pub filter_drive: f32,
    /// The level of the ring modulator, `0..1`. It is the product of the two
    /// oscillator outputs, mixed into the voice signal.
    pub ring_mod: f32,
    /// Whether a retrigger puts the oscillator phases at zero instead of a
    /// random starting phase.
    pub osc_phase_reset: bool,
    pub amp_env: EnvCoeffs,
    pub filter_env: EnvCoeffs,
    /// The exponent of the final amplitude envelope shaper. One is the old
    /// linear envelope.
    pub amp_env_curve: f32,
    /// The exponent of the final filter envelope shaper.
    pub filter_env_curve: f32,
    /// `1..=4`.
    pub unison: usize,
    pub detune_cents: f32,
    /// `0..1`.
    pub spread: f32,
    /// The global tuning offset, in semitones.
    pub tune_semitones: f32,
    /// The pitch bend offset, in semitones.
    pub pitch_bend_semitones: f32,
    /// The mod wheel position, `0..1`.
    pub mod_wheel: f32,
    /// The glide coefficient for one control-rate update, `0..=1`.
    pub glide_step: f32,
    pub lfo1_amount: f32,
    pub lfo1_target: LfoTarget,
    /// Whether the modulation matrix has an active route from the first LFO.
    /// The hidden legacy target/amount is then ignored, so the same LFO is
    /// never added twice.
    pub lfo1_routed: bool,
    pub lfo2_amount: f32,
    pub lfo2_target: LfoTarget,
    /// Whether the modulation matrix has an active route from the second LFO.
    pub lfo2_routed: bool,
    /// How much the velocity of the note opens the filter, `0..1`.
    pub velocity_amount: f32,
    /// How much the voice drifts, `0..1`. The value is already scaled by the
    /// character of the sound.
    pub drift_amount: f32,
    /// Whether the harsh character is active. It adds a bit of filter drive.
    pub character_grit: bool,
    /// Whether the voice should use the original DSP paths.
    pub legacy: bool,
    /// The eight slots of the modulation matrix.
    pub mod_slots: [ModSlot; MOD_SLOTS],
    /// The value of the random source at the start of the block, `-1..1`.
    pub random: f32,
    /// The value of the first macro, `0..1`. It is a source of the modulation
    /// matrix.
    pub macro1: f32,
    /// The value of the second macro, `0..1`.
    pub macro2: f32,
    /// The value of the third macro, `0..1`.
    pub macro3: f32,
}

impl Default for VoiceSettings {
    fn default() -> Self {
        Self {
            osc1: OscSettings::default(),
            osc2: OscSettings {
                semitones: -12.0,
                ..OscSettings::default()
            },
            sub_level: 0.0,
            random: 0.0,
            osc_sync: false,
            fm_amount: 0.0,
            noise_level: 0.0,
            filter_mode: FilterMode::Lowpass,
            filter_slope: FilterSlope::TwoPole,
            resonance: 0.2,
            filter_env_octaves: 2.0,
            filter_keytrack: 0.3,
            filter_drive: 0.0,
            ring_mod: 0.0,
            osc_phase_reset: false,
            amp_env: EnvCoeffs::default(),
            filter_env: EnvCoeffs::default(),
            amp_env_curve: 1.0,
            filter_env_curve: 1.0,
            unison: 1,
            detune_cents: 12.0,
            spread: 0.5,
            tune_semitones: 0.0,
            pitch_bend_semitones: 0.0,
            mod_wheel: 0.0,
            glide_step: 1.0,
            lfo1_amount: 0.0,
            lfo1_target: LfoTarget::Off,
            lfo1_routed: false,
            lfo2_amount: 0.0,
            lfo2_target: LfoTarget::Off,
            lfo2_routed: false,
            velocity_amount: 0.0,
            drift_amount: 0.0,
            character_grit: false,
            legacy: false,
            mod_slots: [ModSlot::default(); MOD_SLOTS],
            macro1: 0.5,
            macro2: 0.5,
            macro3: 0.5,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct DcBlocker {
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
    pub(crate) fn process(&mut self, input: f32, sample_rate: f32) -> f32 {
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

#[derive(Debug, Clone, Copy, Default)]
struct LegacyHfShelf {
    /// The state of the one-pole lowpass.
    lowpass: f32,
    /// The one-pole coefficient that was computed for `rate`.
    coefficient: f32,
    /// The sample rate of the cached coefficient.
    rate: f32,
}

impl LegacyHfShelf {
    #[inline]
    fn process(&mut self, input: f32, sample_rate: f32) -> f32 {
        if self.rate != sample_rate {
            self.rate = sample_rate;
            let omega = 2.0 * PI * LEGACY_HF_CORNER / sample_rate.max(1.0);
            self.coefficient = (1.0 - (-omega).exp()).clamp(0.0, 1.0);
        }

        self.lowpass += self.coefficient * (input - self.lowpass);
        if self.lowpass.abs() < 1e-20 {
            self.lowpass = 0.0;
        }

        // Unity at DC, `LEGACY_HF_GAIN` well above the corner.
        self.lowpass + LEGACY_HF_GAIN * (input - self.lowpass)
    }

    fn reset(&mut self) {
        self.lowpass = 0.0;
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
    /// Whether a note off has been deferred by the sustain pedal.
    sustained: bool,

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
    sub_osc: Oscillator,
    sub_delta: f32,
    noise: Noise,

    amp_env: Adsr,
    filter_env: Adsr,
    filter_left: Svf,
    filter_right: Svf,
    filter_left2: Svf,
    filter_right2: Svf,
    dc_left: DcBlocker,
    dc_right: DcBlocker,
    /// The fixed top-end loss of the legacy flavor, after the DC blockers.
    hf_left: LegacyHfShelf,
    hf_right: LegacyHfShelf,

    /// The counter of the control rate. The pitch and the filter are only
    /// updated every eighth sample.
    control_counter: u32,
    /// The current level of the filter envelope.
    filter_env_level: f32,
    /// The current level of the amplifier envelope.
    amp_env_level: f32,
    /// The phase increment of the unison voices of the first oscillator.
    phase_delta1: [f32; MAX_UNISON],
    /// The phase increment of the unison voices of the second oscillator.
    phase_delta2: [f32; MAX_UNISON],
    /// The coefficient of the filter.
    g: f32,
    /// The damping of the filter.
    k: f32,
    /// The gain compensation of the unison bank, `1/sqrt(unison)`. It is
    /// cached at the control rate.
    unison_gain: f32,
    /// The equal-power left pan gain of each unison voice, cached at the
    /// control rate.
    pan_left: [f32; MAX_UNISON],
    /// The equal-power right pan gain of each unison voice, cached at the
    /// control rate.
    pan_right: [f32; MAX_UNISON],
    /// The stereo spread that the cached pan gains were computed for.
    pan_spread: f32,

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
    /// The random number generator of the Random modulation source. It is
    /// separate from the drift and phase streams, so the new source cannot
    /// change an existing sound.
    random_rng: u32,
    /// The deterministic value of the Random modulation source, `-1..1`. It
    /// stays the same for the life of a note.
    random_value: f32,
    /// The deterministic pitch tolerance of a legacy voice, in semitones.
    pitch_tolerance: f32,
    /// The deterministic gain tolerance of a legacy voice, as a linear
    /// fraction.
    gain_tolerance: f32,
    /// The deterministic cutoff tolerance of a legacy voice, in octaves.
    cutoff_tolerance: f32,
    /// The modulation of the destinations of the modulation matrix. It is
    /// refreshed at the control rate.
    modulation: Modulation,
    /// Whether the matrix contains a live Mod Wheel route. When it does, the
    /// wheel belongs to the matrix and the built-in wheel-to-LFO-1 coupling is
    /// switched off. Cached at the control rate, so the sample path only reads
    /// a bool.
    mod_wheel_routed: bool,
    /// The flavor used by the previous sample. A switch clears the filter
    /// state that belongs to the other topology.
    last_legacy: bool,
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
        amp_env.set_legacy(settings.legacy);
        amp_env.gate_on();
        let mut filter_env = Adsr::default();
        filter_env.set_legacy(settings.legacy);
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

        // The Random modulation source uses its own deterministic stream. The
        // drift and phase streams above stay untouched, and the value is
        // stable until the note is retriggered.
        let mut random_rng = 0xD1B5_4A32
            ^ (internal_voice_id as u32).wrapping_mul(0x85EB_CA6B)
            ^ ((internal_voice_id >> 32) as u32).wrapping_mul(0x9E37_79B9);
        if random_rng == 0 {
            random_rng = 0x6C8E_9CF5;
        }
        let random_value = next_random(&mut random_rng);

        // The component tolerances are fixed for the life of the voice. They
        // use their own hash, so the existing drift and phase streams stay
        // untouched. The modern flavor has no tolerances at all.
        let (pitch_tolerance, gain_tolerance, cutoff_tolerance) = if settings.legacy {
            legacy_tolerances(internal_voice_id)
        } else {
            (0.0, 0.0, 0.0)
        };

        let mut voice = Self {
            voice_id,
            channel,
            key,
            internal_voice_id,
            releasing: false,
            sustained: false,
            velocity: velocity.sqrt(),
            velocity_raw: velocity,
            target_pitch: pitch,
            current_pitch: pitch,
            unison: settings.unison.clamp(1, MAX_UNISON),
            osc1_bank: [Oscillator::new(); MAX_UNISON],
            osc2_bank: [Oscillator::new(); MAX_UNISON],
            sub_osc: Oscillator::new(),
            sub_delta: 0.0,
            noise: Noise::new(0x9E37_79B9 ^ internal_voice_id as u32),
            amp_env,
            filter_env,
            filter_left: Svf::new(),
            filter_right: Svf::new(),
            filter_left2: Svf::new(),
            filter_right2: Svf::new(),
            dc_left: DcBlocker::default(),
            dc_right: DcBlocker::default(),
            hf_left: LegacyHfShelf::default(),
            hf_right: LegacyHfShelf::default(),
            control_counter: CONTROL_RATE - 1,
            filter_env_level: 0.0,
            amp_env_level: 0.0,
            phase_delta1: [0.0; MAX_UNISON],
            phase_delta2: [0.0; MAX_UNISON],
            g: 0.0,
            k: 2.0,
            unison_gain: 1.0,
            pan_left: [0.0; MAX_UNISON],
            pan_right: [0.0; MAX_UNISON],
            pan_spread: settings.spread,
            drift_phase,
            drift_target,
            drift_value: 0.0,
            drift_rng,
            phase_rng,
            random_rng,
            random_value,
            pitch_tolerance,
            gain_tolerance,
            cutoff_tolerance,
            modulation: Modulation::default(),
            mod_wheel_routed: has_mod_wheel_route(&settings.mod_slots),
            last_legacy: settings.legacy,
                    };

        if settings.osc_phase_reset {
            voice.reset_osc_phases_fixed();
        } else {
            voice.reset_osc_phases();
        }
        voice.update_pan_gains(settings.spread);
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

    /// Changes the MIDI channel of a reused voice (mono and legato mode).
    pub fn set_channel(&mut self, channel: Channel) {
        self.channel = channel;
    }

    /// Changes the velocity of a reused voice (mono and legato mode). The
    /// amplitude and the velocity-to-filter modulation follow the new note.
    pub fn set_velocity(&mut self, velocity: f32) {
        let velocity = velocity.clamp(0.0, 1.0);
        self.velocity = velocity.sqrt();
        self.velocity_raw = velocity;
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

    /// Whether the sustain pedal is holding this voice after its key was
    /// released.
    pub fn is_sustained(&self) -> bool {
        self.sustained
    }

    /// Keeps the voice sounding after a note off until the pedal is released.
    pub fn sustain(&mut self) {
        if !self.releasing {
            self.sustained = true;
        }
    }

    /// Releases a voice that was held by the sustain pedal.
    pub fn release_sustained(&mut self) {
        if self.sustained {
            self.sustained = false;
            self.release();
        }
    }

    /// Changes the played key. When `retrigger` is set, both envelopes are
    /// restarted, otherwise the pitch glides to the new key.
    pub fn set_key(&mut self, key: Key, settings: &VoiceSettings, retrigger: bool) {
        self.sustained = false;
        self.key = key;
        self.target_pitch = key.number_or_middle_c() as f32;

        if retrigger {
            self.amp_env.gate_on();
            self.filter_env.gate_on();
            self.releasing = false;
            // A retriggered note gets a fresh Random value. A legato or gliding
            // note keeps the value of the note that is already sounding.
            self.random_value = next_random(&mut self.random_rng);
            // The phase reset is an explicit choice. With the switch off the
            // phases keep running exactly as they always did.
            if settings.osc_phase_reset {
                self.reset_osc_phases_fixed();
            }
        }

        let unison = settings.unison.clamp(1, MAX_UNISON);
        if unison != self.unison {
            self.unison = unison;
            self.update_pan_gains(settings.spread);
        }
    }

    /// Puts the voice into its release stage.
    pub fn release(&mut self) {
        self.releasing = true;
        self.sustained = false;
        self.amp_env.gate_off();
        self.filter_env.gate_off();
    }

    #[inline]
    pub fn mod_env_level(&self) -> f32 {
        self.filter_env_level
    }

    #[inline]
    pub fn amp_env_level(&self) -> f32 {
        self.amp_env_level
    }

    /// The velocity of the note, `0..1`.
    #[inline]
    pub fn velocity_value(&self) -> f32 {
        self.velocity_raw
    }

    /// The key of the note as a MIDI number, so the panel can place it on the
    /// keyboard.
    #[inline]
    pub fn key_number(&self) -> f32 {
        self.key.number_or_middle_c() as f32
    }


    #[inline]
    pub fn is_finished(&self) -> bool {
        self.amp_env.is_finished()
    }

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

        if settings.legacy != self.last_legacy {
            // The two flavors have different filter state. Clear it when the
            // flavor changes so no old state leaks in.
            self.filter_left = Svf::new();
            self.filter_right = Svf::new();
            self.filter_left2 = Svf::new();
            self.filter_right2 = Svf::new();
            // Fresh filters carry no coefficients, so prepare them with the
            // current control values right away.
            self.filter_left.set(self.g, self.k);
            self.filter_right.set(self.g, self.k);
            self.filter_left2.set(self.g, self.k);
            self.filter_right2.set(self.g, self.k);
            self.hf_left.reset();
            self.hf_right.reset();
            if settings.legacy {
                let (pitch, gain, cutoff) = legacy_tolerances(self.internal_voice_id);
                self.pitch_tolerance = pitch;
                self.gain_tolerance = gain;
                self.cutoff_tolerance = cutoff;
            } else {
                // The modern flavor has no component tolerances at all.
                self.pitch_tolerance = 0.0;
                self.gain_tolerance = 0.0;
                self.cutoff_tolerance = 0.0;
            }
            self.last_legacy = settings.legacy;
        }

        // The envelopes. The threshold flavor follows the mode even for a
        // note that is already sounding.
        self.amp_env.set_legacy(settings.legacy);
        self.filter_env.set_legacy(settings.legacy);
        let amp = shape_envelope(self.amp_env.next(&settings.amp_env), settings.amp_env_curve);
        self.amp_env_level = amp;
        self.filter_env_level = shape_envelope(
            self.filter_env.next(&settings.filter_env),
            settings.filter_env_curve,
        );

        // The control rate. The pitch, the filter and the modulation matrix
        // are updated every eighth sample only, which saves a lot of CPU
        // time.
        self.control_counter = (self.control_counter + 1) % CONTROL_RATE;
        let control_tick = self.control_counter == 0;
        if control_tick {
            // The Mod Wheel has two possible paths. As soon as the matrix has
            // a live wheel route, that route owns the wheel and the built-in
            // wheel-to-LFO-1 coupling must stay out of its way.
            self.mod_wheel_routed = has_mod_wheel_route(&settings.mod_slots);
        }

        // The LFO destinations.
        let (pitch_semitones, filter_octaves, amp_scale, pulse_width_offset) =
            lfo_modulation(settings, lfo1, lfo2, self.mod_wheel_routed);

        // The drift.
        self.update_drift(sample_rate, settings.legacy);

        if control_tick {
            // The modulation matrix. Its sources are read at the control
            // rate as well, which is fast enough for the ear.
            self.modulation = evaluate(
                &settings.mod_slots,
                &self.mod_sources(settings, lfo1, lfo2, amp),
            );

            self.update_control(
                settings,
                cutoff_hz,
                pitch_semitones + self.modulation.pitch_semitones,
                filter_octaves + self.modulation.filter_octaves,
                sample_rate,
            );
        }

        let unison = self.unison;
        let unison_gain = self.unison_gain;

        // The pan gains are refreshed at the control rate as well. The spread
        // can change between two control ticks, so pick it up here.
        if settings.spread != self.pan_spread {
            self.update_pan_gains(settings.spread);
        }

        let mut left = 0.0;
        let mut right = 0.0;

        render_oscillators(
            &mut self.osc1_bank,
            &mut self.osc2_bank,
            &mut left,
            &mut right,
            settings,
            &self.phase_delta1,
            &self.phase_delta2,
            unison,
            unison_gain,
            &self.pan_left,
            &self.pan_right,
            pulse_width_offset + self.modulation.pulse_width1,
            pulse_width_offset + self.modulation.pulse_width2,
            (settings.ring_mod + self.modulation.ring_mod).clamp(0.0, 1.0),
        );

        // The sub oscillator is one octave below VCO 1 and stays in the
        // centre of the image. The matrix adds to its level, so the final
        // level is kept inside the parameter range. The base level is read
        // every sample, as it was before the matrix had this destination, so
        // its smoothing is unchanged.
        let sub_level = (settings.sub_level + self.modulation.sub_level).clamp(0.0, 1.0);
        let sub = self.sub_osc.next(self.sub_delta, Waveform::Square, 0.5) * sub_level;
        left += sub;
        right += sub;

        // The noise generator is shared by both channels. It is kept at half
        // scale, so that it doesn't dominate the mix. Its modulated level is
        // clamped as well.
        let noise_level = (settings.noise_level + self.modulation.noise_level).clamp(0.0, 1.0);
        let noise = self.noise.next() * noise_level * 0.5;
        left += noise;
        right += noise;

        // The resonance of the filter, with the modulation of the matrix.
        let resonance = (settings.resonance + self.modulation.resonance).clamp(0.0, 1.0);

        // A high resonance would make the filter gain a lot of energy, so the
        // input of the filter is turned down a little.
        let attenuation = 1.0 / (1.0 + 0.35 * resonance);

        // The harsh character adds a bit of drive to the filter. `Drive` is
        // the original destination; `Filter Drive` is the appended one and
        // adds on top of it, so old patches keep the exact old expression.
        let drive = if settings.character_grit {
            (settings.filter_drive + 0.12 + self.modulation.drive + self.modulation.filter_drive)
                .clamp(0.0, 1.0)
        } else {
            (settings.filter_drive + self.modulation.drive + self.modulation.filter_drive)
                .clamp(0.0, 1.0)
        };

        // The legacy flavor adds a tiny asymmetric quadratic term just before
        // the filter. The normalizer keeps the RMS where it was, and the DC
        // blockers after the filter take the small offset back out.
        let (left, right) = if settings.legacy {
            (
                legacy_warmth(left * attenuation),
                legacy_warmth(right * attenuation),
            )
        } else {
            (left * attenuation, right * attenuation)
        };

        // The 24 dB slopes run a second cascaded SVF stage. The original
        // ladder filter is gone; the legacy engine has always used this
        // transparent cascade, so old patches keep their sound.
        let left =
            self.filter_left
                .process_prepared(left, settings.filter_mode, drive, settings.legacy);
        let right =
            self.filter_right
                .process_prepared(right, settings.filter_mode, drive, settings.legacy);
        let (left, right) = if settings.filter_slope != FilterSlope::TwoPole {
            (
                self.filter_left2.process_prepared(
                    left,
                    settings.filter_mode,
                    drive,
                    settings.legacy,
                ),
                self.filter_right2.process_prepared(
                    right,
                    settings.filter_mode,
                    drive,
                    settings.legacy,
                ),
            )
        } else {
            (left, right)
        };

        // The DC blocker, which takes the offset of the wave out of the
        // signal.
        let left = self.dc_left.process(left, sample_rate);
        let right = self.dc_right.process(right, sample_rate);

        // The fixed high-frequency loss of the legacy flavor. It sits after
        // the DC blocker, so the quadratic warmth cannot leave any offset in
        // the output. The modern path skips it entirely.
        let (left, right) = if settings.legacy {
            (
                self.hf_left.process(left, sample_rate),
                self.hf_right.process(right, sample_rate),
            )
        } else {
            (left, right)
        };

        // The amplifier. The legacy gain tolerance is a fixed trim, while the
        // modern flavor keeps exactly the gain it had before.
        let gain = amp * self.velocity * amp_scale * self.modulation.volume;
        let gain = if settings.legacy {
            gain * (1.0 + self.gain_tolerance)
        } else {
            gain
        };
        (left * gain, right * gain)
    }

    /// The values of the sources of the modulation matrix.
    fn mod_sources(
        &self,
        settings: &VoiceSettings,
        lfo1: f32,
        lfo2: f32,
        amp_env: f32,
    ) -> ModSources {
        ModSources {
            lfo1,
            lfo2,
            filter_env: self.filter_env_level,
            amp_env,
            velocity: self.velocity_raw,
            key: key_source(self.key.number_or_middle_c() as f32),
            macro1: settings.macro1,
            macro2: settings.macro2,
            macro3: settings.macro3,
            mod_wheel: settings.mod_wheel,
            random: settings.random,
        }
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
        let mut pitch = self.current_pitch
            + settings.tune_semitones
            + settings.pitch_bend_semitones
            + pitch_semitones
            + drift_pitch;
        if settings.legacy {
            pitch += self.pitch_tolerance;
        }
        let base_frequency = util::f32_midi_key_to_freq(pitch);

        // The filter.
        let key = self.key.number_or_middle_c() as f32;
        let drift_filter = self.drift_value * settings.drift_amount * DRIFT_CUTOFF;
        // The matrix offsets the amount of the filter envelope, before the
        // envelope level scales it.
        let filter_env_octaves = settings.filter_env_octaves + self.modulation.filter_env_octaves;
        let keytrack = settings.filter_keytrack + self.modulation.filter_keytrack;
        let mut octaves = filter_env_octaves * self.filter_env_level
            + keytrack * (key - 60.0) / 12.0
            + settings.velocity_amount * self.velocity_raw * 1.5
            + filter_octaves
            + drift_filter;
        if settings.legacy {
            octaves += self.cutoff_tolerance;
        }
        let cutoff = (cutoff_hz * 2f32.powf(octaves)).clamp(20.0, sample_rate * 0.45);
                self.g = (PI * cutoff / sample_rate).tan();

        // The resonance of the filter, with the modulation of the matrix.
        let resonance = (settings.resonance + self.modulation.resonance).clamp(0.0, 1.0);
        self.k = 2.0 - 1.98 * resonance;

        // Prepare the coefficients of the cascaded SVF stages once per
        // control tick instead of once per sample.
        self.filter_left.set(self.g, self.k);
        self.filter_right.set(self.g, self.k);
        self.filter_left2.set(self.g, self.k);
        self.filter_right2.set(self.g, self.k);

        // The phase increments of the unison voices. Unison can be changed
        // while a note is held, so the cached count is refreshed here. The
        // sounding slots keep their phases; the new slots already have their
        // deterministic random phases from the construction of the voice.
        let unison = settings.unison.clamp(1, MAX_UNISON);
        if unison != self.unison {
            self.unison = unison;
        }
        let osc1_semitones = settings.osc1.semitones
            + self.modulation.osc1_pitch_semitones
            + self.modulation.osc1_fine_semitones;
        let frequency1 = base_frequency * 2f32.powf(osc1_semitones / 12.0);
        let osc2_semitones = settings.osc2.semitones
            + self.modulation.osc2_pitch_semitones
            + self.modulation.osc2_fine_semitones;
        let frequency2 = base_frequency * 2f32.powf(osc2_semitones / 12.0);
        let detune_cents = settings.detune_cents + self.modulation.detune_cents;
        for index in 0..unison {
            let offset = unison_offset(index, unison);
            let ratio = unison_detune_ratio(detune_cents, offset);
            self.phase_delta1[index] = (frequency1 * ratio / sample_rate).clamp(0.0, 0.5);
            self.phase_delta2[index] = (frequency2 * ratio / sample_rate).clamp(0.0, 0.5);
        }
        self.sub_delta = (frequency1 * 0.5 / sample_rate).clamp(0.0, 0.5);

        // The pan gains depend on the unison count, the spread and the pan
        // modulation, all of which are control rate values as well.
        self.update_pan_gains(settings.spread);
    }

    /// Caches the equal-power pan gains of the unison bank and its gain
    /// compensation. This uses the same formula as the per-sample path, just
    /// once per control tick.
    fn update_pan_gains(&mut self, spread: f32) {
        let unison = self.unison.clamp(1, MAX_UNISON);
        self.unison = unison;
        self.unison_gain = 1.0 / (unison as f32).sqrt();
        self.pan_spread = spread;

        let pan_offset = self.modulation.pan;
        for index in 0..unison {
            let offset = unison_offset(index, unison);
            let pan = (offset * spread + pan_offset).clamp(-1.0, 1.0);
            let angle = (pan + 1.0) * PI * 0.25;
            self.pan_left[index] = angle.cos();
            self.pan_right[index] = angle.sin();
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

    /// Puts every oscillator at a fixed phase of zero. This is only used when
    /// the phase reset switch is on; the random stream is then left untouched.
    fn reset_osc_phases_fixed(&mut self) {
        for index in 0..MAX_UNISON {
            self.osc1_bank[index].set_phase(0.0);
            self.osc2_bank[index].set_phase(0.0);
        }
        self.sub_osc.set_phase(0.0);
    }

    #[inline]
    fn update_drift(&mut self, sample_rate: f32, legacy: bool) {
        if legacy {
            self.drift_phase += LEGACY_DRIFT_RATE / sample_rate;
            if self.drift_phase >= 1.0 {
                self.drift_phase -= 1.0;
                self.drift_target = next_random(&mut self.drift_rng);
            }
            self.drift_value += (self.drift_target - self.drift_value) * LEGACY_DRIFT_SMOOTHING;
        } else {
            // The modern drift stays exactly as it was before the legacy
            // additions.
            self.drift_phase += DRIFT_RATE / sample_rate;
            if self.drift_phase >= 1.0 {
                self.drift_phase -= 1.0;
                self.drift_target = next_random(&mut self.drift_rng);
            }
            self.drift_value += (self.drift_target - self.drift_value) * DRIFT_SMOOTHING;
        }
    }
}

/// The fixed component tolerances of a legacy voice, as a pitch offset in
/// semitones, a linear gain offset and a cutoff offset in octaves.
///
/// This is a private hash of `internal_voice_id`, not the drift or phase RNG,
/// so adding it leaves the existing random sequences untouched.
fn legacy_tolerances(internal_voice_id: u64) -> (f32, f32, f32) {
    (
        signed_hash(internal_voice_id, 0xD1B5_4A32_D192_ED03) * TOLERANCE_PITCH,
        signed_hash(internal_voice_id, 0x2545_F491_4F6C_DD1D) * TOLERANCE_GAIN,
        signed_hash(internal_voice_id, 0x9E37_79B9_7F4A_7C15) * TOLERANCE_CUTOFF,
    )
}

#[inline]
fn signed_hash(internal_voice_id: u64, salt: u64) -> f32 {
    let mut x = internal_voice_id ^ salt;
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;

    // Use the upper 24 bits so that the full float precision is used.
    ((x >> 40) as f32 / 16_777_215.0) * 2.0 - 1.0
}

#[inline]
fn legacy_warmth(input: f32) -> f32 {
    (input + LEGACY_WARMTH * input * input) * LEGACY_WARMTH_NORMALIZER
}

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

/// The maximum phase offset of the FM path at full amount, in cycles.
const FM_PHASE_RANGE: f32 = 0.5;

#[allow(clippy::too_many_arguments)]
#[inline]
fn render_oscillators(
    osc1_bank: &mut [Oscillator; MAX_UNISON],
    osc2_bank: &mut [Oscillator; MAX_UNISON],
    left: &mut f32,
    right: &mut f32,
    settings: &VoiceSettings,
    phase_deltas1: &[f32; MAX_UNISON],
    phase_deltas2: &[f32; MAX_UNISON],
    unison: usize,
    unison_gain: f32,
    pan_left: &[f32; MAX_UNISON],
    pan_right: &[f32; MAX_UNISON],
    pulse_width_offset1: f32,
    pulse_width_offset2: f32,
    ring_mod: f32,
) {
    let pulse_width1 = (settings.osc1.pulse_width + pulse_width_offset1).clamp(0.05, 0.95);
    let pulse_width2 = (settings.osc2.pulse_width + pulse_width_offset2).clamp(0.05, 0.95);

    // FM and hard sync create discontinuities that PolyBLEP cannot remove.
    // Rendering those two cases at twice the control rate and averaging the
    // two sub-samples is a cheap first step towards anti-aliasing. Normal
    // waveforms keep the single-rate path.
    let oversample = !settings.legacy && (settings.osc_sync || settings.fm_amount > 1e-4);
    let steps = if oversample { 2 } else { 1 };
    let step_scale = 1.0 / steps as f32;
    let step_gain = 1.0 / steps as f32;

    for index in 0..unison {
        let mut sample = 0.0;
        for _ in 0..steps {
            let (master, wrapped) = osc2_bank[index].next_modulated(
                phase_deltas2[index] * step_scale,
                settings.osc2.waveform,
                pulse_width2,
                0.0,
            );
            if settings.osc_sync && wrapped {
                osc1_bank[index].set_phase(0.0);
            }

            let phase_offset = master * settings.fm_amount * FM_PHASE_RANGE;
            let (slave, _) = osc1_bank[index].next_modulated(
                phase_deltas1[index] * step_scale,
                settings.osc1.waveform,
                pulse_width1,
                phase_offset,
            );

            let mixed = slave * settings.osc1.level + master * settings.osc2.level;
            // The ring modulator is the simple product of the two oscillator
            // outputs. At zero the old expression is kept untouched.
            let mixed = if ring_mod > 0.0 {
                mixed + ring_mod * slave * master
            } else {
                mixed
            };
            sample += mixed * step_gain;
        }

        let sample = sample * unison_gain;
        *left += sample * pan_left[index];
        *right += sample * pan_right[index];
    }
}

#[inline]
fn has_mod_wheel_route(slots: &[ModSlot; MOD_SLOTS]) -> bool {
    slots.iter().any(|slot| {
        slot.source == ModSource::ModWheel && slot.dest != ModDest::Off && slot.amount != 0.0
    })
}

#[inline]
fn lfo_modulation(
    settings: &VoiceSettings,
    lfo1: f32,
    lfo2: f32,
    mod_wheel_routed: bool,
) -> (f32, f32, f32, f32) {
    let mut pitch_semitones = 0.0;
    let mut filter_octaves = 0.0;
    let mut amp_scale = 1.0;
    let mut pulse_width_offset = 0.0;

    // The mod wheel adds to LFO 1. When no destination was chosen, it acts
    // as a conventional vibrato wheel. As soon as the matrix routes LFO 1,
    // the legacy direct path is completely out of the way, including this
    // coupling, so the wheel can not add a second LFO 1 vibrato.
    let mut lfo1_target = settings.lfo1_target;
    let mut lfo1_amount = settings.lfo1_amount;
    if settings.lfo1_routed {
        lfo1_target = LfoTarget::Off;
        lfo1_amount = 0.0;
    } else if !mod_wheel_routed && settings.mod_wheel > 1e-4 {
        lfo1_amount = (lfo1_amount + settings.mod_wheel).clamp(0.0, 1.0);
        if lfo1_target == LfoTarget::Off {
            lfo1_target = LfoTarget::Pitch;
        }
    }

    let (lfo2_target, lfo2_amount) = if settings.lfo2_routed {
        (LfoTarget::Off, 0.0)
    } else {
        (settings.lfo2_target, settings.lfo2_amount)
    };

    for (value, target, amount) in [
        (lfo1, lfo1_target, lfo1_amount),
        (lfo2, lfo2_target, lfo2_amount),
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

