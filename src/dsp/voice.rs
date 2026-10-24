//! A single synth voice: two oscillators, a unison bank, envelopes and a filter.

use nice_plug::midi::{Channel, Key};
use nice_plug::util;
use std::f32::consts::PI;

use super::adsr::{Adsr, EnvCoeffs, shape_envelope};
use super::filter::{Ladder, LadderModern, Svf};
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

/// Whether these settings run a ladder instead of the cascaded SVF. Only the
/// modern four-pole lowpass uses one, in either the old or the new flavor.
#[inline]
fn uses_ladder(settings: &VoiceSettings) -> bool {
    !settings.legacy
        && matches!(
            settings.filter_slope,
            FilterSlope::FourPole | FilterSlope::FourPoleModern
        )
        && settings.filter_mode == FilterMode::Lowpass
}

/// Whether the ladder should use the zero-delay-feedback topology. This is
/// only true for the `24 dB` option in the modern flavor; `24 dB OLD` keeps
/// the original one-sample-delayed ladder.
#[inline]
fn uses_modern_ladder(settings: &VoiceSettings) -> bool {
    uses_ladder(settings) && settings.filter_slope == FilterSlope::FourPoleModern
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

/// A one-pole highpass that takes the DC offset out of the signal.
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

/// A one-pole high-shelf that takes a little of the top end away. It is unity
/// at DC and settles at [`LEGACY_HF_GAIN`] above the corner. The coefficient
/// is computed once per sample-rate change, not once per sample.
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
    ladder_left: Ladder,
    ladder_right: Ladder,
    modern_ladder_left: LadderModern,
    modern_ladder_right: LadderModern,
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
    /// Whether the previous sample ran the nonlinear ladder. A switch resets
    /// the filter path that becomes active, so that it never runs with stale
    /// or default coefficients while waiting for the next control tick.
    last_ladder_mode: bool,
    /// Whether the active ladder was the zero-delay-feedback one. The old and
    /// new ladders are separate filters, so a switch between them has to
    /// prepare the newly active one as well.
    last_modern_ladder: bool,
    /// The cutoff that the last control tick used by the filter, in Hz.
    current_cutoff: f32,
    /// The resonance that the last control tick used by the filter, `0..1`.
    current_resonance: f32,
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
            ladder_left: Ladder::new(),
            ladder_right: Ladder::new(),
            modern_ladder_left: LadderModern::new(),
            modern_ladder_right: LadderModern::new(),
            dc_left: DcBlocker::default(),
            dc_right: DcBlocker::default(),
            hf_left: LegacyHfShelf::default(),
            hf_right: LegacyHfShelf::default(),
            control_counter: CONTROL_RATE - 1,
            filter_env_level: 0.0,
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
            last_ladder_mode: uses_ladder(settings),
            last_modern_ladder: uses_modern_ladder(settings),
            current_cutoff: 1_000.0,
            current_resonance: settings.resonance.clamp(0.0, 1.0),
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

        if settings.legacy != self.last_legacy {
            // The ladder and the cascaded SVF have different state. Clear
            // both when the flavor changes so no old filter state leaks in.
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
            self.ladder_left.reset();
            self.ladder_right.reset();
            self.modern_ladder_left.reset();
            self.modern_ladder_right.reset();
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

        // The filter topology can change between two control ticks. The
        // ladder is only prepared by the control tick while it is active, so
        // a switch has to reset the newly active path and prepare its
        // coefficients here. Otherwise it would run with stale or default
        // coefficients for up to seven samples.
        let ladder_mode = uses_ladder(settings);
        let modern_ladder = ladder_mode && uses_modern_ladder(settings);
        if ladder_mode != self.last_ladder_mode {
            self.last_ladder_mode = ladder_mode;
            self.filter_left = Svf::new();
            self.filter_right = Svf::new();
            self.filter_left2 = Svf::new();
            self.filter_right2 = Svf::new();
            self.filter_left.set(self.g, self.k);
            self.filter_right.set(self.g, self.k);
            self.filter_left2.set(self.g, self.k);
            self.filter_right2.set(self.g, self.k);
            self.ladder_left.reset();
            self.ladder_right.reset();
            self.modern_ladder_left.reset();
            self.modern_ladder_right.reset();
            self.last_modern_ladder = modern_ladder;
            if ladder_mode {
                self.reset_and_prepare_ladder(modern_ladder, sample_rate);
            }
        } else if ladder_mode && modern_ladder != self.last_modern_ladder {
            self.last_modern_ladder = modern_ladder;
            self.reset_and_prepare_ladder(modern_ladder, sample_rate);
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

        // The 24 dB lowpass uses the nonlinear ladder; the other modes keep
        // the transparent cascaded SVF so they can still provide highpass,
        // bandpass and notch outputs.
        let (left, right) = if ladder_mode {
            if modern_ladder {
                (
                    self.modern_ladder_left.process(left, drive),
                    self.modern_ladder_right.process(right, drive),
                )
            } else {
                (
                    self.ladder_left.process(left, drive),
                    self.ladder_right.process(right, drive),
                )
            }
        } else {
            let left = self.filter_left.process_prepared(
                left,
                settings.filter_mode,
                drive,
                settings.legacy,
            );
            let right = self.filter_right.process_prepared(
                right,
                settings.filter_mode,
                drive,
                settings.legacy,
            );
            if settings.filter_slope != FilterSlope::TwoPole {
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
            }
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
            random: self.random_value,
        }
    }

    /// Clears the state of the newly active ladder and prepares its
    /// coefficients. Called when the slope changes between two control ticks,
    /// so the first sample already uses the right topology and cutoff.
    fn reset_and_prepare_ladder(&mut self, modern: bool, sample_rate: f32) {
        if modern {
            self.modern_ladder_left.reset();
            self.modern_ladder_right.reset();
            self.modern_ladder_left
                .set(self.current_cutoff, self.current_resonance, sample_rate);
            self.modern_ladder_right
                .set(self.current_cutoff, self.current_resonance, sample_rate);
        } else {
            self.ladder_left.reset();
            self.ladder_right.reset();
            self.ladder_left
                .set(self.current_cutoff, self.current_resonance, sample_rate);
            self.ladder_right
                .set(self.current_cutoff, self.current_resonance, sample_rate);
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
        self.current_cutoff = cutoff;
        self.g = (PI * cutoff / sample_rate).tan();

        // The resonance of the filter, with the modulation of the matrix.
        let resonance = (settings.resonance + self.modulation.resonance).clamp(0.0, 1.0);
        self.current_resonance = resonance;
        self.k = 2.0 - 1.98 * resonance;

        // Prepare the coefficients of the cascaded SVF stages once per
        // control tick instead of once per sample.
        self.filter_left.set(self.g, self.k);
        self.filter_right.set(self.g, self.k);
        self.filter_left2.set(self.g, self.k);
        self.filter_right2.set(self.g, self.k);

        if uses_ladder(settings) {
            if uses_modern_ladder(settings) {
                self.modern_ladder_left.set(cutoff, resonance, sample_rate);
                self.modern_ladder_right.set(cutoff, resonance, sample_rate);
            } else {
                self.ladder_left.set(cutoff, resonance, sample_rate);
                self.ladder_right.set(cutoff, resonance, sample_rate);
            }
        }

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

    /// Forces the SVF coefficients to be recomputed for every sample, as the
    /// original path did. Only used by the cached/uncached regression test.
    #[cfg(test)]
    fn force_uncached_svf(&mut self) {
        self.filter_left.force_uncached();
        self.filter_right.force_uncached();
        self.filter_left2.force_uncached();
        self.filter_right2.force_uncached();
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

    /// Moves the drift towards its new target. Every time the phase of the
    /// drift wraps around, a new target is picked. The legacy flavor uses a
    /// slower rate and a slower smoothing so that the drift wanders instead of
    /// stepping.
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

/// A deterministic random number in the `-1..1` range. The salt selects an
/// independent stream for the same voice.
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

/// The asymmetric quadratic warmth of the legacy flavor.
///
/// The quadratic term adds a little second harmonic. The normalizer keeps the
/// level of a full-scale sine unchanged, and the DC blockers after the filter
/// take care of the small offset that `x * x` introduces.
#[inline]
fn legacy_warmth(input: f32) -> f32 {
    (input + LEGACY_WARMTH * input * input) * LEGACY_WARMTH_NORMALIZER
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

/// The maximum phase offset of the FM path at full amount, in cycles.
const FM_PHASE_RANGE: f32 = 0.5;

/// Renders both oscillator banks into the stereo output. VCO 2 is rendered
/// first so that it can act as the master for hard sync and phase modulation
/// of VCO 1. The unison bank keeps its own equal-power pan.
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

/// Whether the modulation matrix contains a Mod Wheel route that actually
/// does something. A wheel route with an Off destination or a zero amount
/// leaves the built-in wheel behavior untouched.
#[inline]
fn has_mod_wheel_route(slots: &[ModSlot; MOD_SLOTS]) -> bool {
    slots.iter().any(|slot| {
        slot.source == ModSource::ModWheel && slot.dest != ModDest::Off && slot.amount != 0.0
    })
}

/// Calculates the modulation of the two global LFOs.
///
/// Returns the pitch offset in semitones, the filter offset in octaves, the
/// amplitude scale and the pulse width offset. `mod_wheel_routed` disables
/// the built-in wheel-to-LFO-1 coupling when the matrix already routes the
/// wheel.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::{ModDest, ModSource};

    const SAMPLE_RATE: f32 = 48_000.0;

    fn voice(settings: &VoiceSettings) -> Voice {
        Voice::new(1, Channel::Number(0), Key::Number(60), 0, 1.0, settings)
    }

    /// A matrix with a single slot.
    fn matrix(slot: ModSlot) -> [ModSlot; MOD_SLOTS] {
        let mut slots = [ModSlot::default(); MOD_SLOTS];
        slots[0] = slot;
        slots
    }

    /// The root mean square of a block of samples.
    fn rms(samples: &[f32]) -> f32 {
        let sum: f32 = samples.iter().map(|sample| sample * sample).sum();
        (sum / samples.len() as f32).sqrt()
    }

    /// The frequency of the voice, measured by its zero crossings.
    fn measure_frequency(voice: &mut Voice, settings: &VoiceSettings) -> f32 {
        measure_frequency_with_lfo(voice, settings, 0.0)
    }

    /// The frequency of the voice with a fixed value of the first LFO.
    fn measure_frequency_with_lfo(voice: &mut Voice, settings: &VoiceSettings, lfo1: f32) -> f32 {
        let mut crossings = 0;
        let mut previous = 0.0;
        for _ in 0..(SAMPLE_RATE as usize * 2) {
            let (left, _) = voice.next_sample(settings, 8_000.0, lfo1, 0.0, SAMPLE_RATE);
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
    fn the_tune_shifts_the_pitch_of_the_voice() {
        // A single sine, so that the measured frequency is the one of the
        // oscillator.
        let settings = VoiceSettings {
            osc1: OscSettings {
                waveform: Waveform::Sine,
                level: 1.0,
                ..OscSettings::default()
            },
            osc2: OscSettings {
                level: 0.0,
                ..OscSettings::default()
            },
            filter_env_octaves: 0.0,
            ..VoiceSettings::default()
        };

        let mut plain = voice(&settings);
        let plain_frequency = measure_frequency(&mut plain, &settings);

        // A tune of an octave makes the voice an octave higher.
        let tuned_settings = VoiceSettings {
            tune_semitones: 12.0,
            ..settings
        };
        let mut tuned = voice(&tuned_settings);
        let tuned_frequency = measure_frequency(&mut tuned, &tuned_settings);

        let expected = util::f32_midi_key_to_freq(60.0 + 12.0);
        assert!(
            (tuned_frequency - expected).abs() < 5.0,
            "the tuned voice played at {tuned_frequency} Hz instead of {expected} Hz"
        );
        assert!(
            (tuned_frequency - plain_frequency * 2.0).abs() < 5.0,
            "the tuned voice played at {tuned_frequency} Hz instead of {} Hz",
            plain_frequency * 2.0
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
    fn a_reused_voice_takes_the_new_velocity_and_channel() {
        let settings = VoiceSettings::default();
        let mut voice = voice(&settings);
        voice.set_velocity(0.2);
        voice.set_channel(Channel::Number(3));

        assert!((voice.velocity_raw - 0.2).abs() < 1e-6);
        assert_eq!(voice.channel(), Channel::Number(3));
    }

    #[test]
    fn the_mod_wheel_adds_vibrato_when_lfo1_is_off() {
        let settings = VoiceSettings {
            mod_wheel: 0.5,
            lfo1_target: LfoTarget::Off,
            lfo1_amount: 0.0,
            ..VoiceSettings::default()
        };

        let (pitch, _, _, _) = lfo_modulation(&settings, 1.0, 0.0, false);
        assert!(pitch > 0.0, "the mod wheel did not bend the pitch");
    }

    #[test]
    fn a_mod_wheel_route_takes_over_from_the_built_in_vibrato() {
        let coupled_settings = VoiceSettings {
            mod_wheel: 0.5,
            lfo1_target: LfoTarget::Off,
            lfo1_amount: 0.0,
            ..plain_settings()
        };

        // Without a matrix route the wheel still acts as the old vibrato
        // wheel. A high LFO 1 therefore bends the pitch up by a semitone.
        let mut coupled = voice(&coupled_settings);
        assert!(!coupled.mod_wheel_routed);
        let coupled_frequency = measure_frequency_with_lfo(&mut coupled, &coupled_settings, 1.0);
        let base_frequency = util::f32_midi_key_to_freq(60.0);
        let expected = base_frequency * 2f32.powf(1.0 / 12.0);
        assert!(
            (coupled_frequency - expected).abs() < 4.0,
            "the built-in wheel vibrato played at {coupled_frequency} Hz instead of {expected} Hz"
        );

        // The same wheel, now routed to the cutoff by the matrix, must not
        // touch the pitch at all: the route is the only wheel path.
        let routed_settings = VoiceSettings {
            mod_slots: matrix(ModSlot {
                source: ModSource::ModWheel,
                dest: ModDest::Cutoff,
                amount: 0.5,
            }),
            ..coupled_settings
        };
        let mut routed = voice(&routed_settings);
        assert!(routed.mod_wheel_routed);
        let routed_frequency = measure_frequency_with_lfo(&mut routed, &routed_settings, 1.0);
        assert!(
            (routed_frequency - base_frequency).abs() < 4.0,
            "the Mod Wheel to Cutoff route bent the pitch to {routed_frequency} Hz instead of {base_frequency} Hz"
        );
        assert!(
            routed_frequency < coupled_frequency - 5.0,
            "the routed wheel still added the built-in vibrato ({routed_frequency} vs {coupled_frequency} Hz)"
        );
    }

    #[test]
    fn a_matrix_lfo_route_disables_the_direct_path() {
        let direct = VoiceSettings {
            lfo1_amount: 0.5,
            lfo1_target: LfoTarget::Filter,
            lfo2_amount: 0.4,
            lfo2_target: LfoTarget::Pitch,
            ..VoiceSettings::default()
        };
        let (pitch, filter, _, _) = lfo_modulation(&direct, 1.0, 1.0, false);
        assert!(
            (filter - 2.0).abs() < 1e-6,
            "the direct filter LFO was {filter}"
        );
        assert!(
            (pitch - 0.8).abs() < 1e-6,
            "the direct pitch LFO was {pitch}"
        );

        // The matrix owns both LFOs now, so the hidden legacy route is quiet.
        let routed = VoiceSettings {
            lfo1_routed: true,
            lfo2_routed: true,
            ..direct
        };
        let (pitch, filter, amp, pulse_width) = lfo_modulation(&routed, 1.0, 1.0, false);
        assert_eq!(pitch, 0.0);
        assert_eq!(filter, 0.0);
        assert_eq!(amp, 1.0);
        assert_eq!(pulse_width, 0.0);
    }

    #[test]
    fn a_routed_lfo1_ignores_the_mod_wheel_vibrato() {
        let settings = VoiceSettings {
            mod_wheel: 0.5,
            lfo1_routed: true,
            ..VoiceSettings::default()
        };

        let (pitch, _, _, _) = lfo_modulation(&settings, 1.0, 0.0, false);
        assert_eq!(pitch, 0.0, "the wheel still added a direct LFO 1 vibrato");
    }

    #[test]
    fn the_sustain_pedal_defers_the_release() {
        let settings = VoiceSettings::default();
        let mut voice = voice(&settings);

        voice.sustain();
        assert!(voice.is_sustained());
        assert!(!voice.is_releasing());

        voice.release_sustained();
        assert!(voice.is_releasing());
        assert!(!voice.is_sustained());
    }

    #[test]
    fn the_pitch_bend_shifts_the_pitch() {
        let settings = VoiceSettings {
            osc1: OscSettings {
                waveform: Waveform::Sine,
                level: 1.0,
                ..OscSettings::default()
            },
            osc2: OscSettings {
                level: 0.0,
                ..OscSettings::default()
            },
            filter_env_octaves: 0.0,
            pitch_bend_semitones: 12.0,
            ..VoiceSettings::default()
        };
        let mut voice = voice(&settings);
        let frequency = measure_frequency(&mut voice, &settings);

        assert!(
            (frequency - 523.2).abs() < 8.0,
            "the bent voice played at {frequency} Hz"
        );
    }

    #[test]
    fn the_unison_count_can_change_while_a_note_is_held() {
        let mut settings = plain_settings();
        settings.unison = 1;
        let mut voice = voice(&settings);
        // The first sample triggers a control update for the initial count.
        voice.next_sample(&settings, 3_000.0, 0.0, 0.0, SAMPLE_RATE);

        settings.unison = 3;
        for _ in 0..CONTROL_RATE - 1 {
            voice.next_sample(&settings, 3_000.0, 0.0, 0.0, SAMPLE_RATE);
        }

        // The next sample updates the control values and renders once with
        // the new unison count. The first, already sounding slot must keep
        // its phase and merely advance by its new increment.
        let before = voice.osc1_bank[0].phase();
        voice.next_sample(&settings, 3_000.0, 0.0, 0.0, SAMPLE_RATE);
        let after = voice.osc1_bank[0].phase();
        let expected = (before + voice.phase_delta1[0]).rem_euclid(1.0);
        let distance = (after - expected).abs();
        let distance = distance.min(1.0 - distance);

        assert_eq!(voice.unison, 3);
        assert!(
            distance < 1e-6,
            "the first unison slot jumped from {before} to {after} instead of {expected}"
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
    fn the_sub_oscillator_adds_a_lower_octave() {
        fn level(sub_level: f32) -> f32 {
            let settings = VoiceSettings {
                sub_level,
                ..plain_settings()
            };
            let mut voice = Voice::new(1, Channel::Number(0), Key::Number(60), 0, 1.0, &settings);
            let mut samples = Vec::new();
            for _ in 0..2_048 {
                let (left, _) = voice.next_sample(&settings, 20_000.0, 0.0, 0.0, SAMPLE_RATE);
                samples.push(left);
            }
            rms(&samples)
        }

        let without = level(0.0);
        let with = level(0.5);
        assert!(
            with > without * 1.05,
            "the sub oscillator did not add energy: {with} vs {without}"
        );
    }

    #[test]
    fn the_fm_amount_changes_the_wave() {
        fn samples(fm_amount: f32) -> Vec<f32> {
            let settings = VoiceSettings {
                fm_amount,
                ..plain_settings()
            };
            let mut voice = Voice::new(1, Channel::Number(0), Key::Number(60), 0, 1.0, &settings);
            (0..2_048)
                .map(|_| {
                    let (left, _) = voice.next_sample(&settings, 20_000.0, 0.0, 0.0, SAMPLE_RATE);
                    left
                })
                .collect()
        }

        let plain = samples(0.0);
        let modulated = samples(0.8);
        let difference: f32 = plain
            .iter()
            .zip(&modulated)
            .map(|(a, b)| (a - b).abs())
            .sum();
        assert!(difference > 1.0, "the FM path did not change the wave");
    }

    #[test]
    fn hard_sync_changes_the_wave() {
        fn samples(sync: bool) -> Vec<f32> {
            let mut settings = plain_settings();
            settings.osc_sync = sync;
            settings.osc1.waveform = Waveform::Saw;
            settings.osc2.semitones = 7.0;
            let mut voice = Voice::new(1, Channel::Number(0), Key::Number(60), 0, 1.0, &settings);
            (0..2_048)
                .map(|_| {
                    let (left, _) = voice.next_sample(&settings, 20_000.0, 0.0, 0.0, SAMPLE_RATE);
                    left
                })
                .collect()
        }

        let free = samples(false);
        let synced = samples(true);
        let difference: f32 = free.iter().zip(&synced).map(|(a, b)| (a - b).abs()).sum();
        assert!(difference > 1.0, "hard sync did not change the wave");
    }

    #[test]
    fn a_static_voice_is_bit_identical_across_the_cached_and_uncached_svf() {
        for (mode, slope) in [
            (FilterMode::Lowpass, FilterSlope::TwoPole),
            (FilterMode::Lowpass, FilterSlope::FourPoleModern),
            (FilterMode::Highpass, FilterSlope::FourPole),
            (FilterMode::Highpass, FilterSlope::FourPoleModern),
            (FilterMode::Bandpass, FilterSlope::TwoPole),
            (FilterMode::Notch, FilterSlope::FourPole),
            (FilterMode::Notch, FilterSlope::FourPoleModern),
        ] {
            let settings = VoiceSettings {
                filter_mode: mode,
                filter_slope: slope,
                filter_drive: 0.2,
                character_grit: true,
                unison: 4,
                spread: 0.7,
                ..VoiceSettings::default()
            };

            let mut cached = Voice::new(1, Channel::Number(0), Key::Number(60), 42, 0.8, &settings);
            let mut uncached =
                Voice::new(1, Channel::Number(0), Key::Number(60), 42, 0.8, &settings);
            uncached.force_uncached_svf();

            for sample in 0..2_048 {
                let (cached_left, cached_right) =
                    cached.next_sample(&settings, 1_200.0, 0.0, 0.0, SAMPLE_RATE);
                let (uncached_left, uncached_right) =
                    uncached.next_sample(&settings, 1_200.0, 0.0, 0.0, SAMPLE_RATE);
                assert_eq!(
                    cached_left.to_bits(),
                    uncached_left.to_bits(),
                    "{mode:?} {slope:?} left at sample {sample}"
                );
                assert_eq!(
                    cached_right.to_bits(),
                    uncached_right.to_bits(),
                    "{mode:?} {slope:?} right at sample {sample}"
                );
            }
        }
    }

    #[test]
    fn the_four_pole_filter_is_steeper() {
        fn energy(slope: FilterSlope) -> f32 {
            let settings = VoiceSettings {
                filter_mode: FilterMode::Lowpass,
                filter_slope: slope,
                filter_env_octaves: 0.0,
                filter_keytrack: 0.0,
                velocity_amount: 0.0,
                ..plain_settings()
            };
            let mut voice = Voice::new(1, Channel::Number(0), Key::Number(84), 0, 1.0, &settings);
            let mut sum = 0.0;
            for index in 0..2_048 {
                let (left, _) = voice.next_sample(&settings, 120.0, 0.0, 0.0, SAMPLE_RATE);
                if index >= 512 {
                    sum += left * left;
                }
            }
            sum.sqrt()
        }

        let two_pole = energy(FilterSlope::TwoPole);
        let four_pole = energy(FilterSlope::FourPole);
        assert!(
            four_pole < two_pole * 0.8,
            "the four-pole filter is not steeper: {four_pole} vs {two_pole}"
        );
    }

    #[test]
    fn switching_the_ladder_slope_prepares_the_filter_immediately() {
        let mut settings = plain_settings();
        settings.resonance = 0.3;
        let mut voice = voice(&settings);

        // Let the SVF run through a couple of control ticks, then stop just
        // before the next one so that the switch happens in the middle of a
        // control period.
        for _ in 0..(2 * CONTROL_RATE as usize + 3) {
            voice.next_sample(&settings, 3_000.0, 0.0, 0.0, SAMPLE_RATE);
        }
        assert_ne!(voice.control_counter, CONTROL_RATE - 1);
        assert_eq!(voice.ladder_left.coefficients(), (0.0, 0.0, 0.0));

        // Switch to the 24 dB lowpass. The first sample already has to run
        // the ladder with real coefficients, not with the defaults.
        settings.filter_slope = FilterSlope::FourPole;
        let (left, right) = voice.next_sample(&settings, 3_000.0, 0.0, 0.0, SAMPLE_RATE);
        assert!(left.is_finite() && right.is_finite());
        let (p, k, rez) = voice.ladder_left.coefficients();
        assert!(
            p != 0.0 && k != 0.0 && rez != 0.0,
            "the ladder kept its default coefficients: {p}, {k}, {rez}"
        );
        assert_eq!(voice.ladder_right.coefficients(), (p, k, rez));

        let mut ladder_samples = vec![left];
        for _ in 0..255 {
            let (left, right) = voice.next_sample(&settings, 3_000.0, 0.0, 0.0, SAMPLE_RATE);
            assert!(left.is_finite() && right.is_finite());
            ladder_samples.push(left);
        }
        assert!(
            rms(&ladder_samples[64..]) > 0.01,
            "the voice went silent when the ladder took over: {}",
            rms(&ladder_samples[64..])
        );

        // Switch back to the 12 dB lowpass in the middle of a control period
        // as well. The SVF path has to come back without stale state.
        assert_ne!(voice.control_counter, CONTROL_RATE - 1);
        settings.filter_slope = FilterSlope::TwoPole;
        let mut svf_samples = Vec::new();
        for _ in 0..256 {
            let (left, right) = voice.next_sample(&settings, 3_000.0, 0.0, 0.0, SAMPLE_RATE);
            assert!(left.is_finite() && right.is_finite());
            svf_samples.push(left);
        }
        assert!(
            rms(&svf_samples[64..]) > 0.01,
            "the voice went silent when the SVF took over: {}",
            rms(&svf_samples[64..])
        );
    }

    #[test]
    fn switching_to_the_modern_ladder_prepares_the_zdf_filter() {
        let mut settings = plain_settings();
        settings.resonance = 1.0;
        let mut voice = voice(&settings);

        for _ in 0..(2 * CONTROL_RATE as usize + 3) {
            voice.next_sample(&settings, 3_000.0, 0.0, 0.0, SAMPLE_RATE);
        }
        assert_ne!(voice.control_counter, CONTROL_RATE - 1);

        // The switch happens in the middle of a control period, so the
        // transition preparation has to choose the modern ZDF ladder and
        // prepare its coefficients.
        settings.filter_slope = FilterSlope::FourPoleModern;
        let (left, right) = voice.next_sample(&settings, 3_000.0, 0.0, 0.0, SAMPLE_RATE);
        assert!(left.is_finite() && right.is_finite());
        let (_, k, gain) = voice.modern_ladder_left.coefficients();
        assert!(
            (3.8..4.0).contains(&k),
            "the modern ladder did not reach full resonance: {k}"
        );
        assert!(
            (4.8..5.0).contains(&gain),
            "the modern ladder lost its gain compensation: {gain}"
        );
        assert_eq!(
            voice.modern_ladder_right.coefficients(),
            voice.modern_ladder_left.coefficients()
        );

        for _ in 0..256 {
            let (left, right) = voice.next_sample(&settings, 3_000.0, 0.0, 0.0, SAMPLE_RATE);
            assert!(left.is_finite() && right.is_finite());
        }
    }

    #[test]
    fn switching_between_ladder_variants_prepares_the_new_one() {
        let mut settings = plain_settings();
        settings.resonance = 0.6;
        settings.filter_slope = FilterSlope::FourPole;
        let mut voice = voice(&settings);

        for _ in 0..(2 * CONTROL_RATE as usize + 3) {
            voice.next_sample(&settings, 1_200.0, 0.0, 0.0, SAMPLE_RATE);
        }
        let old_coefficients = voice.ladder_left.coefficients();
        assert_ne!(old_coefficients, (0.0, 0.0, 0.0));
        assert_ne!(voice.control_counter, CONTROL_RATE - 1);

        // The ladder mode stays active, so only the newly selected variant
        // has to be reset and prepared.
        settings.filter_slope = FilterSlope::FourPoleModern;
        let (left, right) = voice.next_sample(&settings, 1_200.0, 0.0, 0.0, SAMPLE_RATE);
        assert!(left.is_finite() && right.is_finite());
        assert_eq!(voice.ladder_left.coefficients(), old_coefficients);
        let (_, k, _) = voice.modern_ladder_left.coefficients();
        assert!(k > 0.0, "the modern ladder was not prepared: {k}");

        for _ in 0..128 {
            let (left, right) = voice.next_sample(&settings, 1_200.0, 0.0, 0.0, SAMPLE_RATE);
            assert!(left.is_finite() && right.is_finite());
        }

        assert_ne!(voice.control_counter, CONTROL_RATE - 1);
        settings.filter_slope = FilterSlope::FourPole;
        let (left, right) = voice.next_sample(&settings, 1_200.0, 0.0, 0.0, SAMPLE_RATE);
        assert!(left.is_finite() && right.is_finite());
        assert!(voice.ladder_left.coefficients().2 > 0.0);
    }

    #[test]
    fn the_fixed_paths_keep_their_pre_zdf_fingerprints() {
        // These fingerprints were captured before the modern ZDF ladder was
        // added. They pin the legacy path, the old 24 dB path and the default
        // 12 dB path (the one `render_demo` uses).
        fn fingerprint(hash: &mut u64, sample: f32) {
            *hash ^= sample.to_bits() as u64;
            *hash = hash.wrapping_mul(0x100_0000_01b3);
        }

        fn render(settings: &VoiceSettings, id: u64) -> u64 {
            let mut voice = Voice::new(
                id as i32,
                Channel::Number(0),
                Key::Number(60),
                id,
                0.8,
                settings,
            );
            let mut hash = 0xcbf2_9ce4_8422_2325u64;
            for _ in 0..8_192 {
                let (left, right) = voice.next_sample(settings, 1_400.0, 0.0, 0.0, SAMPLE_RATE);
                fingerprint(&mut hash, left);
                fingerprint(&mut hash, right);
            }
            hash
        }

        let base = VoiceSettings {
            osc1: OscSettings {
                waveform: Waveform::Saw,
                level: 0.8,
                ..OscSettings::default()
            },
            osc2: OscSettings {
                waveform: Waveform::Square,
                level: 0.3,
                semitones: -12.0,
                ..OscSettings::default()
            },
            filter_mode: FilterMode::Lowpass,
            filter_slope: FilterSlope::FourPole,
            resonance: 0.45,
            filter_env_octaves: 1.0,
            filter_drive: 0.3,
            unison: 2,
            detune_cents: 8.0,
            spread: 0.5,
            drift_amount: 0.2,
            ..VoiceSettings::default()
        };

        let old = VoiceSettings {
            filter_slope: FilterSlope::FourPole,
            ..base
        };
        let two_pole = VoiceSettings {
            filter_slope: FilterSlope::TwoPole,
            ..base
        };
        let legacy = VoiceSettings {
            legacy: true,
            ..base
        };

        assert_eq!(render(&legacy, 9), 0x0b35_beab_2d91_6476);
        assert_eq!(render(&old, 7), 0xd3c4_1b8b_a3ae_329c);
        assert_eq!(render(&two_pole, 11), 0x9534_eebd_dd8a_6d8e);
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

    #[test]
    fn the_ring_mod_mixes_the_oscillator_outputs() {
        fn render(settings: &VoiceSettings) -> Vec<f32> {
            let mut voice = Voice::new(1, Channel::Number(0), Key::Number(60), 7, 1.0, settings);
            (0..2_048)
                .map(|_| {
                    voice
                        .next_sample(settings, 8_000.0, 0.0, 0.0, SAMPLE_RATE)
                        .0
                })
                .collect()
        }

        let plain = plain_settings();
        let ring = VoiceSettings {
            ring_mod: 0.5,
            ..plain_settings()
        };
        let difference: f32 = render(&plain)
            .iter()
            .zip(render(&ring))
            .map(|(a, b)| (a - b).abs())
            .sum();
        assert!(
            difference > 1.0,
            "the ring modulator did not change the voice: {difference}"
        );

        // The matrix destination reaches the same ring level. The first
        // sample is already a control tick, so the two paths line up.
        let routed = VoiceSettings {
            ring_mod: 0.0,
            macro1: 1.0,
            mod_slots: matrix(ModSlot {
                source: ModSource::Macro1,
                dest: ModDest::RingMod,
                amount: 0.5,
            }),
            ..plain_settings()
        };
        let direct = VoiceSettings {
            ring_mod: 0.5,
            ..plain_settings()
        };
        assert_eq!(render(&direct), render(&routed));
    }

    #[test]
    fn the_phase_reset_pins_the_oscillators_and_is_off_by_default() {
        let reset_settings = VoiceSettings {
            osc_phase_reset: true,
            ..plain_settings()
        };
        let mut first = Voice::new(
            1,
            Channel::Number(0),
            Key::Number(60),
            0,
            1.0,
            &reset_settings,
        );
        let mut second = Voice::new(
            2,
            Channel::Number(0),
            Key::Number(60),
            1,
            1.0,
            &reset_settings,
        );

        for index in 0..MAX_UNISON {
            assert_eq!(first.osc1_bank[index].phase(), 0.0);
            assert_eq!(first.osc2_bank[index].phase(), 0.0);
            assert_eq!(second.osc1_bank[index].phase(), 0.0);
            assert_eq!(second.osc2_bank[index].phase(), 0.0);
        }

        // Two voices with different IDs are phase locked, so their output is
        // identical when nothing else depends on the voice ID.
        for _ in 0..512 {
            let (a, _) = first.next_sample(&reset_settings, 8_000.0, 0.0, 0.0, SAMPLE_RATE);
            let (b, _) = second.next_sample(&reset_settings, 8_000.0, 0.0, 0.0, SAMPLE_RATE);
            assert_eq!(a.to_bits(), b.to_bits());
        }

        // A retrigger puts the phases back to zero.
        for _ in 0..37 {
            first.next_sample(&reset_settings, 8_000.0, 0.0, 0.0, SAMPLE_RATE);
        }
        assert_ne!(first.osc1_bank[0].phase(), 0.0);
        first.set_key(Key::Number(64), &reset_settings, true);
        for index in 0..MAX_UNISON {
            assert_eq!(first.osc1_bank[index].phase(), 0.0);
            assert_eq!(first.osc2_bank[index].phase(), 0.0);
        }

        // With the switch off the old behavior is untouched: a retrigger
        // keeps the running random phases.
        let normal_settings = plain_settings();
        let mut normal = voice(&normal_settings);
        for _ in 0..37 {
            normal.next_sample(&normal_settings, 8_000.0, 0.0, 0.0, SAMPLE_RATE);
        }
        let before: Vec<f32> = (0..MAX_UNISON)
            .map(|index| normal.osc1_bank[index].phase())
            .collect();
        assert!(before.iter().any(|phase| *phase != 0.0));
        normal.set_key(Key::Number(64), &normal_settings, true);
        for (index, phase) in before.iter().enumerate() {
            assert_eq!(normal.osc1_bank[index].phase(), *phase);
        }
    }

    #[test]
    fn the_envelope_curves_are_neutral_at_one_and_shape_the_attack() {
        fn render(settings: &VoiceSettings, samples: usize) -> Vec<f32> {
            let mut voice = Voice::new(1, Channel::Number(0), Key::Number(60), 7, 1.0, settings);
            (0..samples)
                .map(|_| {
                    voice
                        .next_sample(settings, 8_000.0, 0.0, 0.0, SAMPLE_RATE)
                        .0
                })
                .collect()
        }

        let plain = plain_settings();
        let neutral = VoiceSettings {
            amp_env_curve: 1.0,
            filter_env_curve: 1.0,
            ..plain_settings()
        };
        assert_eq!(render(&plain, 2_048), render(&neutral, 2_048));

        let shallow = VoiceSettings {
            amp_env_curve: 0.5,
            ..plain_settings()
        };
        let steep = VoiceSettings {
            amp_env_curve: 2.0,
            ..plain_settings()
        };
        let shallow = rms(&render(&shallow, 512));
        let steep = rms(&render(&steep, 512));
        assert!(
            shallow > steep * 1.1,
            "the amplitude curve did not change the attack: {shallow} vs {steep}"
        );

        // The filter envelope curve is applied to its final value as well.
        let filter_base = VoiceSettings {
            filter_env_octaves: 4.0,
            filter_keytrack: 0.0,
            velocity_amount: 0.0,
            ..plain_settings()
        };
        let shallow = VoiceSettings {
            filter_env_curve: 0.5,
            ..filter_base
        };
        let steep = VoiceSettings {
            filter_env_curve: 2.0,
            ..filter_base
        };
        let difference: f32 = render(&shallow, 2_048)
            .iter()
            .zip(render(&steep, 2_048))
            .map(|(a, b)| (a - b).abs())
            .sum();
        assert!(
            difference > 1e-3,
            "the filter curve did not change the sound: {difference}"
        );
    }

    #[test]
    fn the_matrix_offsets_the_first_oscillator() {
        let settings = plain_settings();

        let mut plain = voice(&settings);
        let plain_frequency = measure_frequency(&mut plain, &settings);

        let octave = VoiceSettings {
            macro1: 1.0,
            mod_slots: matrix(ModSlot {
                source: ModSource::Macro1,
                dest: ModDest::Osc1Pitch,
                amount: 1.0,
            }),
            ..settings
        };
        let mut bent = voice(&octave);
        let bent_frequency = measure_frequency(&mut bent, &octave);
        assert!(
            (bent_frequency - plain_frequency * 2.0).abs() < 5.0,
            "Osc1 Pitch played at {bent_frequency} Hz instead of {plain_frequency} Hz"
        );

        let semitone = VoiceSettings {
            macro1: 1.0,
            mod_slots: matrix(ModSlot {
                source: ModSource::Macro1,
                dest: ModDest::Osc1Fine,
                amount: 1.0,
            }),
            ..settings
        };
        let mut fine = voice(&semitone);
        let fine_frequency = measure_frequency(&mut fine, &semitone);
        let expected = util::f32_midi_key_to_freq(61.0);
        assert!(
            (fine_frequency - expected).abs() < 5.0,
            "Osc1 Fine played at {fine_frequency} Hz instead of {expected} Hz"
        );
    }

    #[test]
    fn the_new_destinations_reach_the_filter() {
        fn render(settings: &VoiceSettings) -> Vec<(f32, f32)> {
            let mut voice = Voice::new(1, Channel::Number(0), Key::Number(72), 7, 1.0, settings);
            (0..1_024)
                .map(|_| voice.next_sample(settings, 1_000.0, 0.0, 0.0, SAMPLE_RATE))
                .collect()
        }

        let base = VoiceSettings {
            filter_env_octaves: 0.0,
            filter_keytrack: 0.0,
            velocity_amount: 0.0,
            drift_amount: 0.0,
            resonance: 0.35,
            ..plain_settings()
        };

        // Filter Drive and the old Drive destination both add to the same
        // filter drive value.
        let direct = VoiceSettings {
            filter_drive: 0.7,
            ..base
        };
        let routed = VoiceSettings {
            macro1: 1.0,
            mod_slots: matrix(ModSlot {
                source: ModSource::Macro1,
                dest: ModDest::FilterDrive,
                amount: 0.7,
            }),
            ..base
        };
        assert_eq!(render(&direct), render(&routed));

        // Filter Keytrack is an offset of the keytrack amount. At key 72 it
        // opens the filter by one octave.
        let keytrack = VoiceSettings {
            macro1: 1.0,
            mod_slots: matrix(ModSlot {
                source: ModSource::Macro1,
                dest: ModDest::FilterKeytrack,
                amount: 1.0,
            }),
            ..base
        };
        let mut plain_voice = voice(&base);
        let mut keytrack_voice =
            Voice::new(1, Channel::Number(0), Key::Number(72), 7, 1.0, &keytrack);
        plain_voice.next_sample(&base, 1_000.0, 0.0, 0.0, SAMPLE_RATE);
        keytrack_voice.next_sample(&keytrack, 1_000.0, 0.0, 0.0, SAMPLE_RATE);
        assert!(
            (plain_voice.current_cutoff - 1_000.0).abs() < 1.0,
            "the base cutoff was {}",
            plain_voice.current_cutoff
        );
        assert!(
            (keytrack_voice.current_cutoff - 2_000.0).abs() < 2.0,
            "the keytrack route produced {} Hz instead of 2000 Hz",
            keytrack_voice.current_cutoff
        );
    }

    /// A voice with a single sine wave, so that the tests can measure what
    /// the modulation matrix does to it.
    fn plain_settings() -> VoiceSettings {
        VoiceSettings {
            osc1: OscSettings {
                waveform: Waveform::Sine,
                level: 1.0,
                ..OscSettings::default()
            },
            osc2: OscSettings {
                level: 0.0,
                ..OscSettings::default()
            },
            filter_env_octaves: 0.0,
            filter_keytrack: 0.0,
            velocity_amount: 0.0,
            drift_amount: 0.0,
            ..VoiceSettings::default()
        }
    }

    #[test]
    fn the_matrix_bends_the_pitch() {
        let settings = plain_settings();

        let mut plain = voice(&settings);
        let plain_frequency = measure_frequency(&mut plain, &settings);

        // A macro at its top bends the pitch by an octave.
        let modulated = VoiceSettings {
            macro1: 1.0,
            mod_slots: matrix(ModSlot {
                source: ModSource::Macro1,
                dest: ModDest::Pitch,
                amount: 1.0,
            }),
            ..settings
        };

        let mut bent = voice(&modulated);
        let bent_frequency = measure_frequency(&mut bent, &modulated);

        let expected = plain_frequency * 2.0;
        assert!(
            (bent_frequency - expected).abs() < 5.0,
            "the bent voice played at {bent_frequency} Hz instead of {expected} Hz"
        );
    }

    #[test]
    fn the_matrix_opens_the_filter() {
        // A sine that is far above the cutoff of the filter.
        let settings = VoiceSettings {
            tune_semitones: 36.0,
            ..plain_settings()
        };

        let mut closed = voice(&settings);
        let mut closed_samples = Vec::new();
        for _ in 0..4_800 {
            let (left, _) = closed.next_sample(&settings, 200.0, 0.0, 0.0, SAMPLE_RATE);
            closed_samples.push(left);
        }

        // The matrix opens the filter by five octaves.
        let modulated = VoiceSettings {
            macro1: 1.0,
            mod_slots: matrix(ModSlot {
                source: ModSource::Macro1,
                dest: ModDest::Cutoff,
                amount: 1.0,
            }),
            ..settings
        };

        let mut open = voice(&modulated);
        let mut open_samples = Vec::new();
        for _ in 0..4_800 {
            let (left, _) = open.next_sample(&modulated, 200.0, 0.0, 0.0, SAMPLE_RATE);
            open_samples.push(left);
        }

        let closed = rms(&closed_samples);
        let open = rms(&open_samples);
        assert!(
            open > closed * 5.0,
            "the filter did not open ({open} instead of {closed})"
        );
    }

    #[test]
    fn the_matrix_moves_the_voice_in_the_stereo_field() {
        let settings = plain_settings();

        // A macro at its top pans the voice to the right.
        let modulated = VoiceSettings {
            macro1: 1.0,
            mod_slots: matrix(ModSlot {
                source: ModSource::Macro1,
                dest: ModDest::Pan,
                amount: 1.0,
            }),
            ..settings
        };

        let mut voice = voice(&modulated);
        let mut left = 0.0;
        let mut right = 0.0;
        for _ in 0..4_800 {
            let (a, b) = voice.next_sample(&modulated, 8_000.0, 0.0, 0.0, SAMPLE_RATE);
            left += a.abs();
            right += b.abs();
        }

        assert!(
            right > left * 10.0,
            "the voice was not panned to the right ({left} vs {right})"
        );
    }

    #[test]
    fn the_matrix_scales_the_level_of_the_voice() {
        let settings = plain_settings();

        let mut plain = voice(&settings);
        let mut plain_samples = Vec::new();
        for _ in 0..4_800 {
            let (left, _) = plain.next_sample(&settings, 8_000.0, 0.0, 0.0, SAMPLE_RATE);
            plain_samples.push(left);
        }

        // A macro at its top doubles the level of the voice.
        let modulated = VoiceSettings {
            macro1: 1.0,
            mod_slots: matrix(ModSlot {
                source: ModSource::Macro1,
                dest: ModDest::Volume,
                amount: 1.0,
            }),
            ..settings
        };

        let mut loud = voice(&modulated);
        let mut loud_samples = Vec::new();
        for _ in 0..4_800 {
            let (left, _) = loud.next_sample(&modulated, 8_000.0, 0.0, 0.0, SAMPLE_RATE);
            loud_samples.push(left);
        }

        let plain = rms(&plain_samples);
        let loud = rms(&loud_samples);
        assert!(
            (loud - plain * 2.0).abs() < plain * 0.1,
            "the level of the voice was {loud} instead of {}",
            plain * 2.0
        );
    }

    #[test]
    fn the_random_source_is_deterministic_and_stable_for_a_note() {
        let settings = plain_settings();
        let mut first = Voice::new(1, Channel::Number(0), Key::Number(60), 123, 1.0, &settings);
        let value = first.random_value;
        assert!(
            (-1.0..=1.0).contains(&value),
            "the Random source left its range: {value}"
        );

        for _ in 0..4_096 {
            first.next_sample(&settings, 3_000.0, 0.0, 0.0, SAMPLE_RATE);
            assert_eq!(
                first.random_value, value,
                "the Random source changed while the note was held"
            );
        }

        // The same voice ID gives the same value on every run.
        let second = Voice::new(1, Channel::Number(0), Key::Number(60), 123, 1.0, &settings);
        assert_eq!(second.random_value, value);

        // Another voice gets its own value.
        let third = Voice::new(1, Channel::Number(0), Key::Number(60), 124, 1.0, &settings);
        assert_ne!(third.random_value, value);
    }

    #[test]
    fn a_retrigger_refreshes_the_random_source_but_a_legato_note_keeps_it() {
        let settings = plain_settings();
        let mut voice = Voice::new(1, Channel::Number(0), Key::Number(60), 17, 1.0, &settings);
        let first = voice.random_value;

        // A gliding note keeps the value of the note that is already sounding.
        voice.set_key(Key::Number(62), &settings, false);
        assert_eq!(voice.random_value, first);

        // A retriggered mono note gets a fresh value.
        voice.set_key(Key::Number(64), &settings, true);
        let refreshed = voice.random_value;
        assert_ne!(refreshed, first);
        for _ in 0..256 {
            voice.next_sample(&settings, 3_000.0, 0.0, 0.0, SAMPLE_RATE);
            assert_eq!(voice.random_value, refreshed);
        }

        voice.set_key(Key::Number(65), &settings, false);
        assert_eq!(voice.random_value, refreshed);
    }

    #[test]
    fn the_matrix_offsets_the_pitch_of_the_second_oscillator() {
        let settings = VoiceSettings {
            osc1: OscSettings {
                waveform: Waveform::Sine,
                level: 0.0,
                ..OscSettings::default()
            },
            osc2: OscSettings {
                waveform: Waveform::Sine,
                level: 1.0,
                semitones: 0.0,
                ..OscSettings::default()
            },
            filter_env_octaves: 0.0,
            filter_keytrack: 0.0,
            velocity_amount: 0.0,
            drift_amount: 0.0,
            ..VoiceSettings::default()
        };

        let mut plain = voice(&settings);
        let plain_frequency = measure_frequency(&mut plain, &settings);

        let octave = VoiceSettings {
            macro1: 1.0,
            mod_slots: matrix(ModSlot {
                source: ModSource::Macro1,
                dest: ModDest::Osc2Pitch,
                amount: 1.0,
            }),
            ..settings
        };
        let mut bent = voice(&octave);
        let bent_frequency = measure_frequency(&mut bent, &octave);
        let expected = util::f32_midi_key_to_freq(72.0);
        assert!(
            (bent_frequency - expected).abs() < 5.0,
            "Osc2 Pitch played at {bent_frequency} Hz instead of {expected} Hz"
        );
        assert!(
            (bent_frequency - plain_frequency * 2.0).abs() < 5.0,
            "Osc2 Pitch did not move an octave"
        );

        let semitone = VoiceSettings {
            macro1: 1.0,
            mod_slots: matrix(ModSlot {
                source: ModSource::Macro1,
                dest: ModDest::Osc2Fine,
                amount: 1.0,
            }),
            ..settings
        };
        let mut fine = voice(&semitone);
        let fine_frequency = measure_frequency(&mut fine, &semitone);
        let expected = util::f32_midi_key_to_freq(61.0);
        assert!(
            (fine_frequency - expected).abs() < 5.0,
            "Osc2 Fine played at {fine_frequency} Hz instead of {expected} Hz"
        );
    }

    #[test]
    fn the_matrix_offsets_the_unison_detune() {
        let settings = VoiceSettings {
            detune_cents: 0.0,
            unison: 2,
            filter_env_octaves: 0.0,
            ..plain_settings()
        };
        let modulated = VoiceSettings {
            macro1: 1.0,
            mod_slots: matrix(ModSlot {
                source: ModSource::Macro1,
                dest: ModDest::Detune,
                amount: 1.0,
            }),
            ..settings
        };

        let mut voice = voice(&modulated);
        voice.next_sample(&modulated, 3_000.0, 0.0, 0.0, SAMPLE_RATE);

        let base = util::f32_midi_key_to_freq(60.0);
        for index in 0..2 {
            let offset = unison_offset(index, 2);
            let expected = base * unison_detune_ratio(50.0, offset) / SAMPLE_RATE;
            assert!(
                (voice.phase_delta1[index] - expected).abs() < 1e-6,
                "the Detune matrix wrote {} instead of {expected}",
                voice.phase_delta1[index]
            );
        }
    }

    #[test]
    fn the_matrix_moves_the_filter_envelope_amount() {
        let settings = VoiceSettings {
            filter_env_octaves: 0.0,
            filter_env: EnvCoeffs::new(0.1, 1.0, 1.0, 100.0, SAMPLE_RATE),
            ..plain_settings()
        };
        let modulated = VoiceSettings {
            macro1: 1.0,
            mod_slots: matrix(ModSlot {
                source: ModSource::Macro1,
                dest: ModDest::FilterEnv,
                amount: 1.0,
            }),
            ..settings
        };

        let mut plain_voice = voice(&settings);
        let mut modulated_voice = voice(&modulated);
        for _ in 0..4_096 {
            plain_voice.next_sample(&settings, 200.0, 0.0, 0.0, SAMPLE_RATE);
            modulated_voice.next_sample(&modulated, 200.0, 0.0, 0.0, SAMPLE_RATE);
        }

        assert!(
            modulated_voice.current_cutoff > plain_voice.current_cutoff * 4.0,
            "Filter Env opened the filter only from {} to {} Hz",
            plain_voice.current_cutoff,
            modulated_voice.current_cutoff
        );
    }

    #[test]
    fn the_sub_and_noise_levels_are_clamped_to_their_range() {
        fn render(settings: &VoiceSettings) -> Vec<(f32, f32)> {
            let mut voice = Voice::new(1, Channel::Number(0), Key::Number(60), 5, 1.0, settings);
            (0..4_096)
                .map(|_| voice.next_sample(settings, 8_000.0, 0.0, 0.0, SAMPLE_RATE))
                .collect()
        }

        let full_sub = VoiceSettings {
            osc1: OscSettings {
                level: 0.0,
                ..OscSettings::default()
            },
            osc2: OscSettings {
                level: 0.0,
                ..OscSettings::default()
            },
            sub_level: 1.0,
            noise_level: 0.0,
            ..plain_settings()
        };
        let clipped_sub = VoiceSettings {
            sub_level: 0.8,
            macro1: 1.0,
            mod_slots: matrix(ModSlot {
                source: ModSource::Macro1,
                dest: ModDest::SubLevel,
                amount: 1.0,
            }),
            ..full_sub
        };
        assert_eq!(render(&full_sub), render(&clipped_sub));

        let full_noise = VoiceSettings {
            osc1: OscSettings {
                level: 0.0,
                ..OscSettings::default()
            },
            osc2: OscSettings {
                level: 0.0,
                ..OscSettings::default()
            },
            sub_level: 0.0,
            noise_level: 1.0,
            ..plain_settings()
        };
        let clipped_noise = VoiceSettings {
            noise_level: 0.8,
            macro1: 1.0,
            mod_slots: matrix(ModSlot {
                source: ModSource::Macro1,
                dest: ModDest::NoiseLevel,
                amount: 1.0,
            }),
            ..full_noise
        };
        assert_eq!(render(&full_noise), render(&clipped_noise));
    }

    #[test]
    fn the_empty_matrix_keeps_the_legacy_output_unchanged() {
        fn render(settings: &VoiceSettings) -> u64 {
            let mut voice = Voice::new(1, Channel::Number(0), Key::Number(48), 9, 0.8, settings);
            let mut hash = 0xcbf2_9ce4_8422_2325u64;
            for _ in 0..8_192 {
                let (left, right) = voice.next_sample(settings, 1_400.0, 0.0, 0.0, SAMPLE_RATE);
                for sample in [left, right] {
                    hash ^= sample.to_bits() as u64;
                    hash = hash.wrapping_mul(0x100_0000_01b3);
                }
            }
            hash
        }

        let settings = VoiceSettings {
            legacy: true,
            osc1: OscSettings {
                waveform: Waveform::Saw,
                level: 0.8,
                ..OscSettings::default()
            },
            osc2: OscSettings {
                waveform: Waveform::Square,
                level: 0.3,
                semitones: -12.0,
                ..OscSettings::default()
            },
            filter_env_octaves: 1.0,
            unison: 2,
            detune_cents: 8.0,
            drift_amount: 0.2,
            ..VoiceSettings::default()
        };

        // The empty matrix has only Off slots, so the new sources and
        // destinations are never selected.
        assert!(
            settings
                .mod_slots
                .iter()
                .all(|slot| slot.source == ModSource::Off && slot.dest == ModDest::Off)
        );
        let empty = render(&settings);

        // Explicitly zeroed old routes are neutral as well.
        let zeroed = VoiceSettings {
            mod_slots: [ModSlot {
                source: ModSource::Macro1,
                dest: ModDest::Volume,
                amount: 0.0,
            }; MOD_SLOTS],
            ..settings
        };
        assert_eq!(render(&zeroed), empty);
    }

    #[test]
    fn the_default_matrix_is_neutral() {
        let settings = plain_settings();

        // Every slot of the default matrix is switched off.
        for slot in settings.mod_slots {
            assert_eq!(slot.source, ModSource::Off);
            assert_eq!(slot.dest, ModDest::Off);
            assert_eq!(slot.amount, 0.0);
        }

        // The voice is not moved at all: both channels are the same and the
        // pitch is the one of the key.
        let mut first = voice(&settings);
        let mut difference = 0.0;
        for _ in 0..4_800 {
            let (left, right) = first.next_sample(&settings, 3_000.0, 0.0, 0.0, SAMPLE_RATE);
            difference += (left - right).abs();
        }
        assert!(
            difference < 1e-6,
            "the default matrix moved the voice ({difference})"
        );

        let mut second = voice(&settings);
        let frequency = measure_frequency(&mut second, &settings);
        assert!(
            (frequency - 261.6).abs() < 5.0,
            "the default matrix changed the pitch ({frequency} Hz)"
        );
    }

    /// The magnitude of a single frequency in a block of samples.
    fn tone_magnitude(samples: &[f32], frequency: f32, sample_rate: f32) -> f32 {
        let mut real = 0.0;
        let mut imaginary = 0.0;
        for (index, sample) in samples.iter().enumerate() {
            let phase = 2.0 * PI * frequency * index as f32 / sample_rate;
            real += sample * phase.cos();
            imaginary += sample * phase.sin();
        }
        2.0 * (real * real + imaginary * imaginary).sqrt() / samples.len() as f32
    }

    #[test]
    fn the_legacy_component_tolerances_are_deterministic_and_bounded() {
        for id in 0..64 {
            let tolerances = legacy_tolerances(id);
            assert_eq!(legacy_tolerances(id), tolerances);
            assert!(tolerances.0.abs() <= TOLERANCE_PITCH);
            assert!(tolerances.1.abs() <= TOLERANCE_GAIN);
            assert!(tolerances.2.abs() <= TOLERANCE_CUTOFF);
        }

        // Two voices must not get the same component set.
        assert_ne!(legacy_tolerances(1), legacy_tolerances(2));
    }

    #[test]
    fn the_legacy_component_tolerances_average_out() {
        let count = 4_096;
        let mut pitch = 0.0;
        let mut gain = 0.0;
        let mut cutoff = 0.0;
        for id in 0..count {
            let (voice_pitch, voice_gain, voice_cutoff) = legacy_tolerances(id);
            pitch += voice_pitch;
            gain += voice_gain;
            cutoff += voice_cutoff;
        }
        let count = count as f32;
        assert!((pitch / count).abs() < TOLERANCE_PITCH * 0.1);
        assert!((gain / count).abs() < TOLERANCE_GAIN * 0.1);
        assert!((cutoff / count).abs() < TOLERANCE_CUTOFF * 0.1);
    }

    #[test]
    fn modern_voices_have_no_component_tolerances() {
        let modern = VoiceSettings::default();
        let legacy = VoiceSettings {
            legacy: true,
            ..modern
        };

        let mut voice = Voice::new(1, Channel::Number(0), Key::Number(60), 7, 1.0, &modern);
        assert_eq!(voice.pitch_tolerance, 0.0);
        assert_eq!(voice.gain_tolerance, 0.0);
        assert_eq!(voice.cutoff_tolerance, 0.0);

        // A modern voice that becomes legacy gets its tolerances then, and a
        // voice that goes back to modern loses them again.
        voice.next_sample(&legacy, 3_000.0, 0.0, 0.0, SAMPLE_RATE);
        assert_eq!(
            (
                voice.pitch_tolerance,
                voice.gain_tolerance,
                voice.cutoff_tolerance,
            ),
            legacy_tolerances(7)
        );
        voice.next_sample(&modern, 3_000.0, 0.0, 0.0, SAMPLE_RATE);
        assert_eq!(voice.pitch_tolerance, 0.0);
        assert_eq!(voice.gain_tolerance, 0.0);
        assert_eq!(voice.cutoff_tolerance, 0.0);

        // A voice that starts legacy gets the same fixed set right away.
        let voice = Voice::new(1, Channel::Number(0), Key::Number(60), 7, 1.0, &legacy);
        assert_eq!(
            (
                voice.pitch_tolerance,
                voice.gain_tolerance,
                voice.cutoff_tolerance,
            ),
            legacy_tolerances(7)
        );
    }

    #[test]
    fn the_legacy_drift_is_slower() {
        let modern_settings = VoiceSettings::default();
        let legacy_settings = VoiceSettings {
            legacy: true,
            ..VoiceSettings::default()
        };
        let mut modern = Voice::new(
            1,
            Channel::Number(0),
            Key::Number(60),
            11,
            1.0,
            &modern_settings,
        );
        let mut legacy = Voice::new(
            1,
            Channel::Number(0),
            Key::Number(60),
            11,
            1.0,
            &legacy_settings,
        );

        for voice in [&mut modern, &mut legacy] {
            voice.drift_phase = 0.0;
            voice.drift_value = 0.0;
            voice.drift_target = 1.0;
        }
        modern.update_drift(SAMPLE_RATE, false);
        legacy.update_drift(SAMPLE_RATE, true);

        assert_eq!(modern.drift_value, DRIFT_SMOOTHING);
        assert_eq!(legacy.drift_value, LEGACY_DRIFT_SMOOTHING);
        assert!(legacy.drift_phase < modern.drift_phase);
        assert!((modern.drift_phase - DRIFT_RATE / SAMPLE_RATE).abs() < 1e-9);
        assert!((legacy.drift_phase - LEGACY_DRIFT_RATE / SAMPLE_RATE).abs() < 1e-9);
    }

    #[test]
    fn the_legacy_warmth_adds_even_harmonics_and_keeps_the_rms() {
        let frequency = 220.0;
        let amplitude = 0.5;
        let mut input = Vec::new();
        let mut output = Vec::new();
        for index in 0..(SAMPLE_RATE as usize) {
            let sample = amplitude * (2.0 * PI * frequency * index as f32 / SAMPLE_RATE).sin();
            input.push(sample);
            output.push(legacy_warmth(sample));
        }

        let input_rms = rms(&input);
        let output_rms = rms(&output);
        assert!(
            (output_rms - input_rms).abs() < input_rms * 0.005,
            "the warmth changed the RMS: {output_rms} instead of {input_rms}"
        );

        let fundamental = tone_magnitude(&output, frequency, SAMPLE_RATE);
        let second = tone_magnitude(&output, frequency * 2.0, SAMPLE_RATE);
        assert!(fundamental > 0.4);
        assert!(
            second > fundamental * 0.002,
            "the second harmonic was too weak: {second}"
        );
        assert!(
            second < fundamental * 0.02,
            "the second harmonic was too strong: {second}"
        );

        // The quadratic term has a positive mean. The voice path removes it
        // again with the DC blockers after the filter.
        let mean = output.iter().sum::<f32>() / output.len() as f32;
        assert!(
            mean > 0.001,
            "there was no DC offset from the warmth: {mean}"
        );
    }

    #[test]
    fn the_legacy_warmth_does_not_add_dc_to_the_output() {
        let settings = VoiceSettings {
            legacy: true,
            osc1: OscSettings {
                waveform: Waveform::Sine,
                level: 1.0,
                ..OscSettings::default()
            },
            osc2: OscSettings {
                level: 0.0,
                ..OscSettings::default()
            },
            filter_env_octaves: 0.0,
            filter_keytrack: 0.0,
            velocity_amount: 0.0,
            drift_amount: 0.0,
            ..VoiceSettings::default()
        };
        let mut voice = Voice::new(1, Channel::Number(0), Key::Number(48), 3, 1.0, &settings);

        // Let the envelope settle and the DC blocker remove the offset that
        // the quadratic term puts into the filter.
        for _ in 0..(SAMPLE_RATE as usize) {
            voice.next_sample(&settings, 12_000.0, 0.0, 0.0, SAMPLE_RATE);
        }
        let mut sum = 0.0;
        for _ in 0..(SAMPLE_RATE as usize) {
            let (left, _) = voice.next_sample(&settings, 12_000.0, 0.0, 0.0, SAMPLE_RATE);
            sum += left;
        }
        let offset = sum / SAMPLE_RATE;
        assert!(
            offset.abs() < 0.001,
            "the legacy voice had a DC offset of {offset}"
        );
    }

    #[test]
    fn the_legacy_character_stays_bounded() {
        let settings = VoiceSettings {
            legacy: true,
            resonance: 1.0,
            filter_drive: 1.0,
            filter_slope: FilterSlope::FourPole,
            character_grit: true,
            drift_amount: 1.0,
            unison: 4,
            ..VoiceSettings::default()
        };
        let mut voice = voice(&settings);

        let mut peak: f32 = 0.0;
        for _ in 0..(SAMPLE_RATE as usize) {
            let (left, right) = voice.next_sample(&settings, 800.0, 0.0, 0.0, SAMPLE_RATE);
            assert!(left.is_finite() && right.is_finite());
            peak = peak.max(left.abs()).max(right.abs());
        }

        assert!(peak < 25.0, "the legacy voice ran away ({peak})");
    }

    #[test]
    fn the_legacy_hf_shelf_rolls_off_the_top_end() {
        fn gain_at(frequency: f32) -> f32 {
            let mut shelf = LegacyHfShelf::default();
            let mut input_energy = 0.0;
            let mut output_energy = 0.0;
            let samples = SAMPLE_RATE as usize;
            for index in 0..samples {
                let input = (2.0 * PI * frequency * index as f32 / SAMPLE_RATE).sin();
                let output = shelf.process(input, SAMPLE_RATE);
                if index >= samples / 2 {
                    input_energy += input * input;
                    output_energy += output * output;
                }
            }
            (output_energy / input_energy).sqrt()
        }

        let low = gain_at(100.0);
        let high = gain_at(12_000.0);
        assert!(
            (low - 1.0).abs() < 0.01,
            "the shelf changed the low end: {low}"
        );
        assert!(
            (0.85..=0.9).contains(&high),
            "the shelf gain at 12 kHz was {high}"
        );
        assert!(high < low * 0.92);
    }
}
