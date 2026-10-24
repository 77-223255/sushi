//! All parameter and enum definitions for the Sushi synthesizer.
//!
//! The parameter IDs are part of the plugin's public interface (they are used
//! by hosts for automation and for saving presets), so they must never change.

use nice_plug::prelude::*;
use std::sync::{Arc, Mutex};

/// The different oscillator shapes Sushi can generate.
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy, Default)]
pub enum Waveform {
    Sine,
    Triangle,
    #[default]
    Saw,
    Square,
    Pulse,
}

/// The available filter topologies.
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy, Default)]
pub enum FilterMode {
    #[default]
    Lowpass,
    Highpass,
    Bandpass,
    Notch,
}

/// The slope of the filter.
///
/// `TwoPole` uses a single state variable filter. `FourPole` is the original
/// 24 dB ladder, which is kept bit-identical for old patches and host states.
/// `FourPoleModern` uses a zero-delay-feedback four-pole ladder with a
/// monotonic resonance curve and gain compensation. The first two variants
/// stay at indices 0 and 1, so old host states keep their original behavior.
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy, Default)]
pub enum FilterSlope {
    #[default]
    #[name = "12 dB"]
    TwoPole,
    #[name = "24 dB OLD"]
    FourPole,
    #[name = "24 dB"]
    FourPoleModern,
}

/// The shapes an LFO can produce.
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy, Default)]
pub enum LfoWaveform {
    #[default]
    Sine,
    Triangle,
    Saw,
    Square,
    #[name = "S&H"]
    SampleHold,
}

/// The destination of the hidden legacy LFO routing.
///
/// New patches route the LFOs through the modulation matrix. These values are
/// kept so that patches and host states from before that change still sound
/// the same.
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy, Default)]
pub enum LfoTarget {
    #[default]
    Off,
    Pitch,
    Filter,
    Amp,
    #[name = "Pulse Width"]
    PulseWidth,
}

/// The sources of the modulation matrix.
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy, Default)]
pub enum ModSource {
    #[default]
    Off,
    #[name = "LFO 1"]
    Lfo1,
    #[name = "LFO 2"]
    Lfo2,
    #[name = "Mod Env"]
    ModEnv,
    #[name = "Amp Env"]
    AmpEnv,
    Velocity,
    Key,
    #[name = "Macro 1"]
    Macro1,
    #[name = "Macro 2"]
    Macro2,
    #[name = "Macro 3"]
    Macro3,
    #[name = "Mod Wheel"]
    ModWheel,
    Random,
}

/// The destinations of the modulation matrix.
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy, Default)]
pub enum ModDest {
    #[default]
    Off,
    Pitch,
    #[name = "Pulse 1"]
    Pulse1,
    #[name = "Pulse 2"]
    Pulse2,
    Cutoff,
    Resonance,
    Drive,
    Pan,
    Volume,
    #[name = "Osc2 Pitch"]
    Osc2Pitch,
    #[name = "Osc2 Fine"]
    Osc2Fine,
    #[name = "Sub Level"]
    SubLevel,
    #[name = "Noise Level"]
    NoiseLevel,
    Detune,
    #[name = "Filter Env"]
    FilterEnv,
    #[name = "Osc1 Pitch"]
    Osc1Pitch,
    #[name = "Osc1 Fine"]
    Osc1Fine,
    #[name = "Filter Drive"]
    FilterDrive,
    #[name = "Filter Keytrack"]
    FilterKeytrack,
    #[name = "Ring Mod"]
    RingMod,
}

/// The number of slots of the modulation matrix.
pub const MOD_SLOTS: usize = 8;

/// A single slot of the modulation matrix: a source, a destination and the
/// amount of the modulation.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ModSlot {
    /// The source of the modulation.
    pub source: ModSource,
    /// The destination of the modulation.
    pub dest: ModDest,
    /// The amount of the modulation, `-1..1`.
    pub amount: f32,
}

/// How incoming notes are turned into voices.
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy, Default)]
pub enum PolyMode {
    #[default]
    Poly,
    Mono,
    Legato,
}

/// The global character of the sound. `Silk` is clean and gentle, `Grit` is
/// harsh and saturated.
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy, Default)]
pub enum Character {
    #[default]
    Silk,
    Grit,
}

/// The pattern the arpeggiator plays.
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy, Default)]
pub enum ArpPattern {
    #[default]
    Up,
    Down,
    UpDown,
    Random,
    /// The notes are played in the order in which they were pressed down.
    #[name = "As Played"]
    AsPlayed,
}

/// The chord that every arpeggiator trigger adds to its base note.
///
/// The variants are appended as a new enum, so the indices of every existing
/// parameter and enum stay the same. The interval of each chord is applied to
/// the note that the pattern selected, and notes outside the MIDI range are
/// skipped instead of wrapped.
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy, Default)]
pub enum ArpChord {
    #[default]
    Off,
    Octave,
    Fifth,
    Minor,
    Major,
}

/// The note division of the arpeggiator and of the synced LFOs.
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy, Default)]
pub enum NoteDiv {
    #[name = "1/4"]
    Quarter,
    #[default]
    #[name = "1/8"]
    Eighth,
    #[name = "1/16"]
    Sixteenth,
    #[name = "1/32"]
    ThirtySecond,
}

impl NoteDiv {
    /// The length of one division, in beats.
    pub fn beats(self) -> f64 {
        match self {
            NoteDiv::Quarter => 1.0,
            NoteDiv::Eighth => 0.5,
            NoteDiv::Sixteenth => 0.25,
            NoteDiv::ThirtySecond => 0.125,
        }
    }
}

/// A `0..1` parameter that is displayed as a percentage.
fn percentage_param(name: &str, default: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
        .with_value_to_string(formatters::v2s_f32_percentage(0))
        .with_string_to_value(formatters::s2v_f32_percentage())
        .with_unit("%")
        .with_smoother(SmoothingStyle::Linear(10.0))
}

/// A time parameter in milliseconds. The range is skewed towards the short end
/// so that the interesting (fast) part of the range gets most of the travel.
fn time_param(name: &str, default: f32, max: f32) -> FloatParam {
    FloatParam::new(
        name,
        default,
        FloatRange::Skewed {
            min: 0.0,
            max,
            factor: FloatRange::skew_factor(-2.0),
        },
    )
    .with_step_size(0.1)
    .with_unit(" ms")
}

/// A bipolar parameter in the `-1..1` range, displayed as a signed percentage.
fn bipolar_param(name: &str, default: f32) -> FloatParam {
    FloatParam::new(
        name,
        default,
        FloatRange::Linear {
            min: -1.0,
            max: 1.0,
        },
    )
    .with_value_to_string(Arc::new(|value: f32| format!("{:+.0}", value * 100.0)))
    .with_string_to_value(Arc::new(|text: &str| {
        text.trim()
            .trim_end_matches('%')
            .trim()
            .parse::<f32>()
            .ok()
            .map(|value| value / 100.0)
    }))
    .with_unit("%")
    .with_smoother(SmoothingStyle::Linear(10.0))
}

/// A frequency parameter in Hertz, skewed towards the low end.
fn frequency_param(name: &str, default: f32, min: f32, max: f32, skew: f32) -> FloatParam {
    FloatParam::new(
        name,
        default,
        FloatRange::Skewed {
            min,
            max,
            factor: FloatRange::skew_factor(skew),
        },
    )
    .with_unit(" Hz")
}

/// A gain parameter in decibels, in the `-12..12` range. 0 dB is neutral.
fn db_param(name: &str, default: f32) -> FloatParam {
    FloatParam::new(
        name,
        default,
        FloatRange::Linear {
            min: -12.0,
            max: 12.0,
        },
    )
    .with_value_to_string(formatters::v2s_f32_rounded(1))
    .with_string_to_value(Arc::new(|text: &str| {
        text.trim()
            .trim_end_matches("dB")
            .trim()
            .parse::<f32>()
            .ok()
    }))
    .with_unit(" dB")
    .with_smoother(SmoothingStyle::Linear(10.0))
}

#[derive(Params)]
pub struct SushiParams {
    // --- Oscillator 1 -----------------------------------------------------
    #[id = "osc1_wave"]
    pub osc1_wave: EnumParam<Waveform>,

    #[id = "osc1_octave"]
    pub osc1_octave: IntParam,

    #[id = "osc1_semi"]
    pub osc1_semi: IntParam,

    #[id = "osc1_fine"]
    pub osc1_fine: FloatParam,

    #[id = "osc1_pw"]
    pub osc1_pw: FloatParam,

    #[id = "osc1_level"]
    pub osc1_level: FloatParam,

    #[id = "sub_level"]
    pub sub_level: FloatParam,

    // --- Oscillator 2 -----------------------------------------------------
    #[id = "osc2_wave"]
    pub osc2_wave: EnumParam<Waveform>,

    #[id = "osc2_octave"]
    pub osc2_octave: IntParam,

    #[id = "osc2_semi"]
    pub osc2_semi: IntParam,

    #[id = "osc2_fine"]
    pub osc2_fine: FloatParam,

    #[id = "osc2_pw"]
    pub osc2_pw: FloatParam,

    #[id = "osc2_level"]
    pub osc2_level: FloatParam,

    #[id = "osc_sync"]
    pub osc_sync: BoolParam,

    #[id = "fm_amount"]
    pub fm_amount: FloatParam,

    // --- Mixer ------------------------------------------------------------
    #[id = "noise_level"]
    pub noise_level: FloatParam,

    #[id = "unison"]
    pub unison: IntParam,

    #[id = "detune"]
    pub detune: FloatParam,

    #[id = "spread"]
    pub spread: FloatParam,

    // --- Voice allocation -------------------------------------------------
    #[id = "poly_mode"]
    pub poly_mode: EnumParam<PolyMode>,

    #[id = "glide"]
    pub glide: FloatParam,

    #[id = "tune"]
    pub tune: FloatParam,

    #[id = "voices"]
    pub voices: IntParam,

    // --- Output -----------------------------------------------------------
    #[id = "volume"]
    pub volume: FloatParam,

    // --- Filter -----------------------------------------------------------
    #[id = "filter_mode"]
    pub filter_mode: EnumParam<FilterMode>,

    #[id = "filter_slope"]
    pub filter_slope: EnumParam<FilterSlope>,

    #[id = "cutoff"]
    pub cutoff: FloatParam,

    #[id = "resonance"]
    pub resonance: FloatParam,

    #[id = "filter_env_amount"]
    pub filter_env_amount: FloatParam,

    #[id = "filter_keytrack"]
    pub filter_keytrack: FloatParam,

    #[id = "filter_drive"]
    pub filter_drive: FloatParam,

    // --- Filter envelope --------------------------------------------------
    #[id = "fe_attack"]
    pub fe_attack: FloatParam,

    #[id = "fe_decay"]
    pub fe_decay: FloatParam,

    #[id = "fe_sustain"]
    pub fe_sustain: FloatParam,

    #[id = "fe_release"]
    pub fe_release: FloatParam,

    // --- Amplitude envelope -----------------------------------------------
    #[id = "ae_attack"]
    pub ae_attack: FloatParam,

    #[id = "ae_decay"]
    pub ae_decay: FloatParam,

    #[id = "ae_sustain"]
    pub ae_sustain: FloatParam,

    #[id = "ae_release"]
    pub ae_release: FloatParam,

    // --- LFO 1 ------------------------------------------------------------
    #[id = "lfo1_wave"]
    pub lfo1_wave: EnumParam<LfoWaveform>,

    #[id = "lfo1_rate"]
    pub lfo1_rate: FloatParam,

    #[id = "lfo1_amount"]
    pub lfo1_amount: FloatParam,

    #[id = "lfo1_target"]
    pub lfo1_target: EnumParam<LfoTarget>,

    // --- LFO 2 ------------------------------------------------------------
    #[id = "lfo2_wave"]
    pub lfo2_wave: EnumParam<LfoWaveform>,

    #[id = "lfo2_rate"]
    pub lfo2_rate: FloatParam,

    #[id = "lfo2_amount"]
    pub lfo2_amount: FloatParam,

    #[id = "lfo2_target"]
    pub lfo2_target: EnumParam<LfoTarget>,

    // --- Chorus -----------------------------------------------------------
    #[id = "chorus_mix"]
    pub chorus_mix: FloatParam,

    #[id = "chorus_rate"]
    pub chorus_rate: FloatParam,

    #[id = "chorus_depth"]
    pub chorus_depth: FloatParam,

    // --- Delay ------------------------------------------------------------
    #[id = "delay_time"]
    pub delay_time: FloatParam,

    #[id = "delay_feedback"]
    pub delay_feedback: FloatParam,

    #[id = "delay_mix"]
    pub delay_mix: FloatParam,

    // --- Reverb -----------------------------------------------------------
    #[id = "reverb_mix"]
    pub reverb_mix: FloatParam,

    #[id = "reverb_decay"]
    pub reverb_decay: FloatParam,

    // --- Modulation matrix ------------------------------------------------
    #[id = "mod1_source"]
    pub mod1_source: EnumParam<ModSource>,

    #[id = "mod1_dest"]
    pub mod1_dest: EnumParam<ModDest>,

    #[id = "mod1_amount"]
    pub mod1_amount: FloatParam,

    #[id = "mod2_source"]
    pub mod2_source: EnumParam<ModSource>,

    #[id = "mod2_dest"]
    pub mod2_dest: EnumParam<ModDest>,

    #[id = "mod2_amount"]
    pub mod2_amount: FloatParam,

    #[id = "mod3_source"]
    pub mod3_source: EnumParam<ModSource>,

    #[id = "mod3_dest"]
    pub mod3_dest: EnumParam<ModDest>,

    #[id = "mod3_amount"]
    pub mod3_amount: FloatParam,

    #[id = "mod4_source"]
    pub mod4_source: EnumParam<ModSource>,

    #[id = "mod4_dest"]
    pub mod4_dest: EnumParam<ModDest>,

    #[id = "mod4_amount"]
    pub mod4_amount: FloatParam,

    #[id = "mod5_source"]
    pub mod5_source: EnumParam<ModSource>,

    #[id = "mod5_dest"]
    pub mod5_dest: EnumParam<ModDest>,

    #[id = "mod5_amount"]
    pub mod5_amount: FloatParam,

    #[id = "mod6_source"]
    pub mod6_source: EnumParam<ModSource>,

    #[id = "mod6_dest"]
    pub mod6_dest: EnumParam<ModDest>,

    #[id = "mod6_amount"]
    pub mod6_amount: FloatParam,

    #[id = "mod7_source"]
    pub mod7_source: EnumParam<ModSource>,

    #[id = "mod7_dest"]
    pub mod7_dest: EnumParam<ModDest>,

    #[id = "mod7_amount"]
    pub mod7_amount: FloatParam,

    #[id = "mod8_source"]
    pub mod8_source: EnumParam<ModSource>,

    #[id = "mod8_dest"]
    pub mod8_dest: EnumParam<ModDest>,

    #[id = "mod8_amount"]
    pub mod8_amount: FloatParam,

    // --- Macros ------------------------------------------------------------
    #[id = "macro_bright"]
    pub macro_bright: FloatParam,

    #[id = "macro_motion"]
    pub macro_motion: FloatParam,

    #[id = "macro_space"]
    pub macro_space: FloatParam,

    // --- Character ---------------------------------------------------------
    #[id = "character"]
    pub character: EnumParam<Character>,

    /// The DSP flavor flag, kept for host-state compatibility. Sushi always
    /// runs the Legacy engine now, so this value no longer selects a flavor.
    /// It is deliberately not part of the patch JSON.
    #[id = "legacy"]
    pub legacy: BoolParam,

    #[id = "drift"]
    pub drift: FloatParam,

    #[id = "velocity"]
    pub velocity: FloatParam,

    #[id = "tone"]
    pub tone: FloatParam,

    // --- Arpeggiator -------------------------------------------------------
    #[id = "arp_on"]
    pub arp_on: BoolParam,

    #[id = "arp_pattern"]
    pub arp_pattern: EnumParam<ArpPattern>,

    #[id = "arp_rate"]
    pub arp_rate: EnumParam<NoteDiv>,

    #[id = "arp_octaves"]
    pub arp_octaves: IntParam,

    #[id = "arp_gate"]
    pub arp_gate: FloatParam,

    #[id = "arp_latch"]
    pub arp_latch: BoolParam,

    #[id = "arp_swing"]
    pub arp_swing: FloatParam,

    #[id = "arp_length"]
    pub arp_length: IntParam,

    #[id = "arp_ratchet"]
    pub arp_ratchet: IntParam,

    // --- LFO clock ---------------------------------------------------------
    // These are appended to the parameter list so that the order (and thus
    // the host automation index) of every existing parameter stays the same.
    #[id = "lfo1_sync"]
    pub lfo1_sync: BoolParam,

    #[id = "lfo1_div"]
    pub lfo1_div: EnumParam<NoteDiv>,

    #[id = "lfo1_retrigger"]
    pub lfo1_retrigger: BoolParam,

    #[id = "lfo2_sync"]
    pub lfo2_sync: BoolParam,

    #[id = "lfo2_div"]
    pub lfo2_div: EnumParam<NoteDiv>,

    #[id = "lfo2_retrigger"]
    pub lfo2_retrigger: BoolParam,

    // --- Drive, phaser and EQ ---------------------------------------------
    // These are appended to the parameter list, so every existing parameter
    // keeps its host automation index. Their defaults are completely neutral.
    #[id = "drive_amount"]
    pub drive_amount: FloatParam,

    #[id = "drive_mix"]
    pub drive_mix: FloatParam,

    #[id = "phaser_rate"]
    pub phaser_rate: FloatParam,

    #[id = "phaser_depth"]
    pub phaser_depth: FloatParam,

    #[id = "phaser_mix"]
    pub phaser_mix: FloatParam,

    #[id = "eq_low"]
    pub eq_low: FloatParam,

    #[id = "eq_mid"]
    pub eq_mid: FloatParam,

    #[id = "eq_high"]
    pub eq_high: FloatParam,

    // --- Global highpass, VCA and envelope curves -------------------------
    // These are appended to the parameter list, so every existing parameter
    // keeps its host automation index. Their defaults are completely neutral.
    #[id = "hpf_cutoff"]
    pub hpf_cutoff: FloatParam,

    #[id = "vca_width"]
    pub vca_width: FloatParam,

    #[id = "vca_pan"]
    pub vca_pan: FloatParam,

    #[id = "ae_curve"]
    pub ae_curve: FloatParam,

    #[id = "fe_curve"]
    pub fe_curve: FloatParam,

    #[id = "ring_mod"]
    pub ring_mod: FloatParam,

    #[id = "osc_phase_reset"]
    pub osc_phase_reset: BoolParam,

    #[id = "lfo1_phase"]
    pub lfo1_phase: FloatParam,

    #[id = "lfo2_phase"]
    pub lfo2_phase: FloatParam,

    #[id = "lfo1_curve"]
    pub lfo1_curve: FloatParam,

    #[id = "lfo2_curve"]
    pub lfo2_curve: FloatParam,

    // --- Arpeggiator chord, strum and probability -------------------------
    // These are appended after every existing parameter, so the host
    // automation indices of old parameters do not move. Their defaults are
    // the old single-note, unstrummed, always-playing behavior.
    #[id = "arp_chord"]
    pub arp_chord: EnumParam<ArpChord>,

    #[id = "arp_strum"]
    pub arp_strum: FloatParam,

    #[id = "arp_probability"]
    pub arp_probability: FloatParam,

    // --- Effect chain order, bypass and global mix -------------------------
    // These are appended after every existing parameter, so the host
    // automation indices of old parameters do not move. Their defaults keep
    // the original chain order, no bypass and a fully wet signal.
    /// A rotation offset of the six-stage effect chain.
    ///
    /// `0` is the original order `Drive > Phaser > Chorus > Delay > Reverb >
    /// EQ`; every step rotates the first stage to the end:
    ///
    /// - `0`: Drive > Phaser > Chorus > Delay > Reverb > EQ
    /// - `1`: Phaser > Chorus > Delay > Reverb > EQ > Drive
    /// - `2`: Chorus > Delay > Reverb > EQ > Drive > Phaser
    /// - `3`: Delay > Reverb > EQ > Drive > Phaser > Chorus
    /// - `4`: Reverb > EQ > Drive > Phaser > Chorus > Delay
    /// - `5`: EQ > Drive > Phaser > Chorus > Delay > Reverb
    #[id = "fx_order"]
    pub fx_order: IntParam,

    /// The stages that are bypassed completely.
    ///
    /// Bit 0 is Drive, 1 Phaser, 2 Chorus, 3 Delay, 4 Reverb and 5 EQ. A set
    /// bit skips its stage and clears the state of that stage.
    #[id = "fx_bypass_mask"]
    pub fx_bypass_mask: IntParam,

    /// The dry/wet crossfade around the entire effect chain. `1` is the fully
    /// wet output of the chain and is bit-exact with the old behavior; `0` is
    /// exactly the dry input and clears the effects.
    #[id = "fx_global_mix"]
    pub fx_global_mix: FloatParam,

    // --- Persistent editor state ------------------------------------------
    // These fields are not parameters; nice-plug stores them with the plugin
    // state so a reopened project keeps showing the loaded patch even though
    // the sound itself is made of parameters.
    #[persist = "patch_name"]
    pub patch_name: Mutex<Option<String>>,

    #[persist = "patch_path"]
    pub patch_path: Mutex<Option<String>>,

    #[persist = "preset_index"]
    pub preset_index: Mutex<usize>,

    #[persist = "patch_dirty"]
    pub patch_dirty: Mutex<bool>,
}

impl Default for SushiParams {
    fn default() -> Self {
        Self {
            // --- Oscillator 1 -------------------------------------------------
            osc1_wave: EnumParam::new("OSC 1 Wave", Waveform::Saw),
            osc1_octave: IntParam::new("OSC 1 Octave", 0, IntRange::Linear { min: -2, max: 2 }),
            osc1_semi: IntParam::new("OSC 1 Semi", 0, IntRange::Linear { min: -12, max: 12 }),
            osc1_fine: FloatParam::new(
                "OSC 1 Fine",
                0.0,
                FloatRange::Linear {
                    min: -100.0,
                    max: 100.0,
                },
            )
            .with_step_size(1.0)
            .with_unit(" cents"),
            osc1_pw: FloatParam::new(
                "OSC 1 PW",
                0.5,
                FloatRange::Linear {
                    min: 0.05,
                    max: 0.95,
                },
            )
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage())
            .with_unit("%")
            .with_smoother(SmoothingStyle::Linear(10.0)),
            osc1_level: percentage_param("OSC 1 Level", 0.6),
            sub_level: percentage_param("Sub Level", 0.0),

            // --- Oscillator 2 -------------------------------------------------
            osc2_wave: EnumParam::new("OSC 2 Wave", Waveform::Saw),
            osc2_octave: IntParam::new("OSC 2 Octave", -1, IntRange::Linear { min: -2, max: 2 }),
            osc2_semi: IntParam::new("OSC 2 Semi", 0, IntRange::Linear { min: -12, max: 12 }),
            osc2_fine: FloatParam::new(
                "OSC 2 Fine",
                0.0,
                FloatRange::Linear {
                    min: -100.0,
                    max: 100.0,
                },
            )
            .with_step_size(1.0)
            .with_unit(" cents"),
            osc2_pw: FloatParam::new(
                "OSC 2 PW",
                0.5,
                FloatRange::Linear {
                    min: 0.05,
                    max: 0.95,
                },
            )
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage())
            .with_unit("%")
            .with_smoother(SmoothingStyle::Linear(10.0)),
            osc2_level: percentage_param("OSC 2 Level", 0.4),
            osc_sync: BoolParam::new("Osc Sync", false),
            fm_amount: percentage_param("FM Amount", 0.0),

            // --- Mixer --------------------------------------------------------
            noise_level: percentage_param("Noise", 0.0),
            unison: IntParam::new("Unison", 1, IntRange::Linear { min: 1, max: 4 }),
            detune: FloatParam::new(
                "Detune",
                12.0,
                FloatRange::Linear {
                    min: 0.0,
                    max: 50.0,
                },
            )
            .with_unit(" cents")
            .with_smoother(SmoothingStyle::Linear(10.0)),
            spread: percentage_param("Spread", 0.5),

            // --- Voice allocation ---------------------------------------------
            poly_mode: EnumParam::new("Poly Mode", PolyMode::Poly),
            glide: time_param("Glide", 0.0, 2_000.0),
            tune: FloatParam::new(
                "Tune",
                0.0,
                FloatRange::Linear {
                    min: -12.0,
                    max: 12.0,
                },
            )
            .with_step_size(0.01)
            .with_smoother(SmoothingStyle::Linear(20.0))
            .with_value_to_string(formatters::v2s_f32_rounded(2))
            .with_string_to_value(Arc::new(|text: &str| {
                text.trim()
                    .trim_end_matches("st")
                    .trim()
                    .parse::<f32>()
                    .ok()
            }))
            .with_unit(" st"),
            voices: IntParam::new("Voices", 16, IntRange::Linear { min: 1, max: 16 }),

            // --- Output -------------------------------------------------------
            volume: FloatParam::new(
                "Volume",
                util::db_to_gain(-6.0),
                FloatRange::Linear {
                    min: util::db_to_gain(-60.0),
                    max: util::db_to_gain(6.0),
                },
            )
            .with_smoother(SmoothingStyle::Logarithmic(10.0))
            .with_value_to_string(formatters::v2s_f32_gain_to_db(1))
            .with_string_to_value(formatters::s2v_f32_gain_to_db())
            .with_unit(" dB"),

            // --- Filter -------------------------------------------------------
            filter_mode: EnumParam::new("Filter Mode", FilterMode::Lowpass),
            filter_slope: EnumParam::new("Filter Slope", FilterSlope::TwoPole),
            cutoff: FloatParam::new(
                "Cutoff",
                3_000.0,
                FloatRange::Skewed {
                    min: 20.0,
                    max: 20_000.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_smoother(SmoothingStyle::Logarithmic(10.0))
            .with_value_to_string(formatters::v2s_f32_hz_then_khz(0))
            .with_string_to_value(formatters::s2v_f32_hz_then_khz()),
            resonance: percentage_param("Resonance", 0.2)
                .with_smoother(SmoothingStyle::Linear(10.0)),
            filter_env_amount: FloatParam::new(
                "Filter Env",
                0.4,
                FloatRange::Linear {
                    min: -1.0,
                    max: 1.0,
                },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage())
            .with_unit("%"),
            filter_keytrack: percentage_param("Keytrack", 0.3)
                .with_smoother(SmoothingStyle::Linear(10.0)),
            filter_drive: percentage_param("Drive", 0.0),

            // --- Filter envelope ----------------------------------------------
            fe_attack: time_param("Filter Env Attack", 5.0, 4_000.0),
            fe_decay: time_param("Filter Env Decay", 400.0, 4_000.0),
            fe_sustain: percentage_param("Filter Env Sustain", 0.3),
            fe_release: time_param("Filter Env Release", 300.0, 4_000.0),

            // --- Amplitude envelope -------------------------------------------
            ae_attack: time_param("Amp Env Attack", 5.0, 4_000.0),
            ae_decay: time_param("Amp Env Decay", 800.0, 4_000.0),
            ae_sustain: percentage_param("Amp Env Sustain", 0.7),
            ae_release: time_param("Amp Env Release", 400.0, 8_000.0),

            // --- LFO 1 --------------------------------------------------------
            lfo1_wave: EnumParam::new("LFO 1 Wave", LfoWaveform::Sine),
            lfo1_rate: frequency_param("LFO 1 Rate", 5.0, 0.01, 40.0, -2.0)
                .with_smoother(SmoothingStyle::Logarithmic(10.0)),
            lfo1_amount: percentage_param("LFO 1 Amount", 0.0),
            lfo1_target: EnumParam::new("LFO 1 Target", LfoTarget::Off),

            // --- LFO 2 --------------------------------------------------------
            lfo2_wave: EnumParam::new("LFO 2 Wave", LfoWaveform::Triangle),
            lfo2_rate: frequency_param("LFO 2 Rate", 0.5, 0.01, 40.0, -2.0)
                .with_smoother(SmoothingStyle::Logarithmic(10.0)),
            lfo2_amount: percentage_param("LFO 2 Amount", 0.0),
            lfo2_target: EnumParam::new("LFO 2 Target", LfoTarget::Off),

            // --- Chorus -------------------------------------------------------
            chorus_mix: percentage_param("Chorus Mix", 0.0)
                .with_smoother(SmoothingStyle::Linear(10.0)),
            chorus_rate: frequency_param("Chorus Rate", 0.8, 0.05, 8.0, -1.0)
                .with_smoother(SmoothingStyle::Logarithmic(10.0)),
            chorus_depth: percentage_param("Chorus Depth", 0.5)
                .with_smoother(SmoothingStyle::Linear(10.0)),

            // --- Delay --------------------------------------------------------
            delay_time: FloatParam::new(
                "Delay Time",
                350.0,
                FloatRange::Skewed {
                    min: 10.0,
                    max: 1_500.0,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_step_size(0.1)
            .with_unit(" ms")
            .with_smoother(SmoothingStyle::Linear(10.0)),
            delay_feedback: FloatParam::new(
                "Delay Feedback",
                0.3,
                FloatRange::Linear {
                    min: 0.0,
                    max: 0.95,
                },
            )
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage())
            .with_unit("%")
            .with_smoother(SmoothingStyle::Linear(10.0)),
            delay_mix: percentage_param("Delay Mix", 0.0)
                .with_smoother(SmoothingStyle::Linear(10.0)),

            // --- Reverb -------------------------------------------------------
            reverb_mix: percentage_param("Reverb Mix", 0.0)
                .with_smoother(SmoothingStyle::Linear(10.0)),
            reverb_decay: percentage_param("Reverb Decay", 0.6)
                .with_smoother(SmoothingStyle::Linear(10.0)),

            // --- Modulation matrix ---------------------------------------------
            mod1_source: EnumParam::new("MOD 1 Source", ModSource::Off),
            mod1_dest: EnumParam::new("MOD 1 Dest", ModDest::Off),
            mod1_amount: bipolar_param("MOD 1 Amount", 0.0),

            mod2_source: EnumParam::new("MOD 2 Source", ModSource::Off),
            mod2_dest: EnumParam::new("MOD 2 Dest", ModDest::Off),
            mod2_amount: bipolar_param("MOD 2 Amount", 0.0),

            mod3_source: EnumParam::new("MOD 3 Source", ModSource::Off),
            mod3_dest: EnumParam::new("MOD 3 Dest", ModDest::Off),
            mod3_amount: bipolar_param("MOD 3 Amount", 0.0),

            mod4_source: EnumParam::new("MOD 4 Source", ModSource::Off),
            mod4_dest: EnumParam::new("MOD 4 Dest", ModDest::Off),
            mod4_amount: bipolar_param("MOD 4 Amount", 0.0),

            mod5_source: EnumParam::new("MOD 5 Source", ModSource::Off),
            mod5_dest: EnumParam::new("MOD 5 Dest", ModDest::Off),
            mod5_amount: bipolar_param("MOD 5 Amount", 0.0),

            mod6_source: EnumParam::new("MOD 6 Source", ModSource::Off),
            mod6_dest: EnumParam::new("MOD 6 Dest", ModDest::Off),
            mod6_amount: bipolar_param("MOD 6 Amount", 0.0),

            mod7_source: EnumParam::new("MOD 7 Source", ModSource::Off),
            mod7_dest: EnumParam::new("MOD 7 Dest", ModDest::Off),
            mod7_amount: bipolar_param("MOD 7 Amount", 0.0),

            mod8_source: EnumParam::new("MOD 8 Source", ModSource::Off),
            mod8_dest: EnumParam::new("MOD 8 Dest", ModDest::Off),
            mod8_amount: bipolar_param("MOD 8 Amount", 0.0),

            // --- Macros -------------------------------------------------------
            macro_bright: percentage_param("Macro Bright", 0.5)
                .with_smoother(SmoothingStyle::Linear(20.0)),
            macro_motion: percentage_param("Macro Motion", 0.5)
                .with_smoother(SmoothingStyle::Linear(20.0)),
            macro_space: percentage_param("Macro Space", 0.5)
                .with_smoother(SmoothingStyle::Linear(20.0)),

            // --- Character ---------------------------------------------------
            character: EnumParam::new("Character", Character::Silk),
            legacy: BoolParam::new("Legacy DSP", false),
            drift: percentage_param("Drift", 0.15).with_smoother(SmoothingStyle::Linear(20.0)),
            velocity: percentage_param("Velocity", 0.5).with_smoother(SmoothingStyle::Linear(20.0)),
            tone: bipolar_param("Tone", 0.0).with_smoother(SmoothingStyle::Linear(20.0)),

            // --- Arpeggiator -------------------------------------------------
            arp_on: BoolParam::new("Arpeggiator", false),
            arp_pattern: EnumParam::new("Arp Pattern", ArpPattern::Up),
            arp_rate: EnumParam::new("Arp Rate", NoteDiv::Sixteenth),
            arp_octaves: IntParam::new("Arp Octaves", 1, IntRange::Linear { min: 1, max: 3 }),
            arp_gate: FloatParam::new(
                "Arp Gate",
                60.0,
                FloatRange::Linear {
                    min: 5.0,
                    max: 100.0,
                },
            )
            .with_value_to_string(formatters::v2s_f32_rounded(0))
            .with_string_to_value(Arc::new(|text: &str| {
                text.trim().trim_end_matches('%').trim().parse::<f32>().ok()
            }))
            .with_unit("%"),
            arp_latch: BoolParam::new("Arp Latch", false),
            arp_swing: FloatParam::new(
                "Arp Swing",
                0.0,
                FloatRange::Linear {
                    min: 0.0,
                    max: 75.0,
                },
            )
            .with_value_to_string(formatters::v2s_f32_rounded(0))
            .with_string_to_value(Arc::new(|text: &str| {
                text.trim().trim_end_matches('%').trim().parse::<f32>().ok()
            }))
            .with_unit("%"),
            arp_length: IntParam::new("Arp Length", 16, IntRange::Linear { min: 1, max: 16 }),
            arp_ratchet: IntParam::new("Arp Ratchet", 1, IntRange::Linear { min: 1, max: 4 }),

            // --- LFO clock ----------------------------------------------------
            lfo1_sync: BoolParam::new("LFO 1 Sync", false),
            lfo1_div: EnumParam::new("LFO 1 Division", NoteDiv::Quarter),
            lfo1_retrigger: BoolParam::new("LFO 1 Retrigger", false),
            lfo2_sync: BoolParam::new("LFO 2 Sync", false),
            lfo2_div: EnumParam::new("LFO 2 Division", NoteDiv::Quarter),
            lfo2_retrigger: BoolParam::new("LFO 2 Retrigger", false),

            // --- Drive, phaser and EQ -----------------------------------------
            drive_amount: percentage_param("Drive Amount", 0.0),
            drive_mix: percentage_param("Drive Mix", 0.0),
            phaser_rate: frequency_param("Phaser Rate", 0.3, 0.02, 5.0, -1.0)
                .with_smoother(SmoothingStyle::Logarithmic(10.0)),
            phaser_depth: percentage_param("Phaser Depth", 0.5),
            phaser_mix: percentage_param("Phaser Mix", 0.0),
            eq_low: db_param("EQ Low", 0.0),
            eq_mid: db_param("EQ Mid", 0.0),
            eq_high: db_param("EQ High", 0.0),

            // --- Global highpass, VCA and envelope curves --------------------
            hpf_cutoff: FloatParam::new(
                "HPF Cutoff",
                20.0,
                FloatRange::Skewed {
                    min: 20.0,
                    max: 2_000.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_hz_then_khz(0))
            .with_string_to_value(formatters::s2v_f32_hz_then_khz())
            .with_smoother(SmoothingStyle::Logarithmic(10.0)),
            vca_width: FloatParam::new("VCA Width", 1.0, FloatRange::Linear { min: 0.0, max: 2.0 })
                .with_value_to_string(formatters::v2s_f32_rounded(2))
                .with_unit(" x")
                .with_smoother(SmoothingStyle::Linear(10.0)),
            vca_pan: bipolar_param("VCA Pan", 0.0),
            ae_curve: bipolar_param("Amp Env Curve", 0.0),
            fe_curve: bipolar_param("Filter Env Curve", 0.0),
            ring_mod: percentage_param("Ring Mod", 0.0),
            osc_phase_reset: BoolParam::new("Osc Phase Reset", false),
            lfo1_phase: percentage_param("LFO 1 Phase", 0.0),
            lfo2_phase: percentage_param("LFO 2 Phase", 0.0),
            lfo1_curve: bipolar_param("LFO 1 Curve", 0.0),
            lfo2_curve: bipolar_param("LFO 2 Curve", 0.0),

            // --- Arpeggiator chord, strum and probability --------------------
            arp_chord: EnumParam::new("Arp Chord", ArpChord::Off),
            arp_strum: FloatParam::new(
                "Arp Strum",
                0.0,
                FloatRange::Linear {
                    min: 0.0,
                    max: 80.0,
                },
            )
            .with_value_to_string(formatters::v2s_f32_rounded(1))
            .with_string_to_value(Arc::new(|text: &str| {
                text.trim()
                    .trim_end_matches("ms")
                    .trim()
                    .parse::<f32>()
                    .ok()
            }))
            .with_unit(" ms"),
            arp_probability: percentage_param("Arp Probability", 1.0),

            // --- Effect chain order, bypass and global mix --------------------
            fx_order: IntParam::new("FX Order", 0, IntRange::Linear { min: 0, max: 5 }),
            fx_bypass_mask: IntParam::new(
                "FX Bypass Mask",
                0,
                IntRange::Linear { min: 0, max: 63 },
            ),
            fx_global_mix: percentage_param("FX Global Mix", 1.0),

            patch_name: Mutex::new(None),
            patch_path: Mutex::new(None),
            preset_index: Mutex::new(0),
            patch_dirty: Mutex::new(false),
        }
    }
}

impl SushiParams {
    /// The eight slots of the modulation matrix, in the order of their
    /// parameters.
    ///
    /// The slots are built once per audio block, so that the audio thread
    /// never has to allocate.
    pub fn mod_slots(&self, steps: u32) -> [ModSlot; MOD_SLOTS] {
        [
            ModSlot {
                source: self.mod1_source.value(),
                dest: self.mod1_dest.value(),
                amount: self.mod1_amount.smoothed.next_step(steps),
            },
            ModSlot {
                source: self.mod2_source.value(),
                dest: self.mod2_dest.value(),
                amount: self.mod2_amount.smoothed.next_step(steps),
            },
            ModSlot {
                source: self.mod3_source.value(),
                dest: self.mod3_dest.value(),
                amount: self.mod3_amount.smoothed.next_step(steps),
            },
            ModSlot {
                source: self.mod4_source.value(),
                dest: self.mod4_dest.value(),
                amount: self.mod4_amount.smoothed.next_step(steps),
            },
            ModSlot {
                source: self.mod5_source.value(),
                dest: self.mod5_dest.value(),
                amount: self.mod5_amount.smoothed.next_step(steps),
            },
            ModSlot {
                source: self.mod6_source.value(),
                dest: self.mod6_dest.value(),
                amount: self.mod6_amount.smoothed.next_step(steps),
            },
            ModSlot {
                source: self.mod7_source.value(),
                dest: self.mod7_dest.value(),
                amount: self.mod7_amount.smoothed.next_step(steps),
            },
            ModSlot {
                source: self.mod8_source.value(),
                dest: self.mod8_dest.value(),
                amount: self.mod8_amount.smoothed.next_step(steps),
            },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_spec() {
        let params = SushiParams::default();

        assert_eq!(params.osc1_wave.value(), Waveform::Saw);
        assert_eq!(params.osc1_octave.value(), 0);
        assert_eq!(params.osc1_semi.value(), 0);
        assert_eq!(params.osc1_fine.value(), 0.0);
        assert_eq!(params.osc1_pw.value(), 0.5);
        assert_eq!(params.osc1_level.value(), 0.6);

        assert_eq!(params.osc2_wave.value(), Waveform::Saw);
        assert_eq!(params.osc2_octave.value(), -1);
        assert_eq!(params.osc2_level.value(), 0.4);

        assert_eq!(params.unison.value(), 1);
        assert_eq!(params.detune.value(), 12.0);
        assert_eq!(params.spread.value(), 0.5);
        assert_eq!(params.poly_mode.value(), PolyMode::Poly);
        assert_eq!(params.glide.value(), 0.0);
        assert_eq!(params.tune.value(), 0.0);
        assert_eq!(params.voices.value(), 16);
        assert_eq!(params.filter_mode.value(), FilterMode::Lowpass);
        assert_eq!(params.cutoff.value(), 3_000.0);
        assert_eq!(params.resonance.value(), 0.2);
        assert_eq!(params.filter_env_amount.value(), 0.4);
        assert_eq!(params.filter_keytrack.value(), 0.3);
        assert_eq!(params.filter_drive.value(), 0.0);
        assert_eq!(params.fe_attack.value(), 5.0);
        assert_eq!(params.fe_decay.value(), 400.0);
        assert_eq!(params.fe_sustain.value(), 0.3);
        assert_eq!(params.fe_release.value(), 300.0);
        assert_eq!(params.ae_attack.value(), 5.0);
        assert_eq!(params.ae_decay.value(), 800.0);
        assert_eq!(params.ae_sustain.value(), 0.7);
        assert_eq!(params.ae_release.value(), 400.0);
        assert_eq!(params.lfo1_wave.value(), LfoWaveform::Sine);
        assert_eq!(params.lfo1_rate.value(), 5.0);
        assert_eq!(params.lfo2_wave.value(), LfoWaveform::Triangle);
        assert_eq!(params.lfo2_rate.value(), 0.5);
        assert!(!params.lfo1_sync.value());
        assert_eq!(params.lfo1_div.value(), NoteDiv::Quarter);
        assert!(!params.lfo1_retrigger.value());
        assert!(!params.lfo2_sync.value());
        assert_eq!(params.lfo2_div.value(), NoteDiv::Quarter);
        assert!(!params.lfo2_retrigger.value());
        assert_eq!(params.chorus_rate.value(), 0.8);
        assert_eq!(params.chorus_depth.value(), 0.5);
        assert_eq!(params.delay_time.value(), 350.0);
        assert_eq!(params.delay_feedback.value(), 0.3);
        assert_eq!(params.reverb_decay.value(), 0.6);
    }

    #[test]
    fn the_new_fx_defaults_are_neutral() {
        let params = SushiParams::default();

        assert_eq!(params.drive_amount.value(), 0.0);
        assert_eq!(params.drive_mix.value(), 0.0);
        assert_eq!(params.phaser_rate.value(), 0.3);
        assert_eq!(params.phaser_depth.value(), 0.5);
        assert_eq!(params.phaser_mix.value(), 0.0);
        assert_eq!(params.eq_low.value(), 0.0);
        assert_eq!(params.eq_mid.value(), 0.0);
        assert_eq!(params.eq_high.value(), 0.0);

        // The global highpass is parked at its minimum, which is an exact
        // bypass, and the VCA is centered with a width of one.
        assert_eq!(params.hpf_cutoff.value(), 20.0);
        assert_eq!(params.vca_width.value(), 1.0);
        assert_eq!(params.vca_pan.value(), 0.0);
        assert_eq!(params.ae_curve.value(), 0.0);
        assert_eq!(params.fe_curve.value(), 0.0);
        assert_eq!(params.ring_mod.value(), 0.0);
        assert!(!params.osc_phase_reset.value());
        assert_eq!(params.lfo1_phase.value(), 0.0);
        assert_eq!(params.lfo2_phase.value(), 0.0);
        assert_eq!(params.lfo1_curve.value(), 0.0);
        assert_eq!(params.lfo2_curve.value(), 0.0);

        // The EQ gains are shown and read in decibels.
        assert_eq!(params.eq_low.to_string(), "0.0 dB");
        let normalized = params
            .eq_high
            .string_to_normalized_value("+12 dB")
            .expect("the EQ gain could not be read");
        let value = params.eq_high.range().unnormalize(normalized);
        assert!((value - 12.0).abs() < 0.01, "the EQ gain was {value}");
    }

    #[test]
    fn the_new_defaults_match_the_spec() {
        let params = SushiParams::default();

        assert_eq!(params.macro_bright.value(), 0.5);
        assert_eq!(params.macro_motion.value(), 0.5);
        assert_eq!(params.macro_space.value(), 0.5);
        assert_eq!(params.character.value(), Character::Silk);
        assert!(!params.legacy.value());
        assert_eq!(params.drift.value(), 0.15);
        assert_eq!(params.velocity.value(), 0.5);
        assert_eq!(params.tone.value(), 0.0);

        assert!(!params.arp_on.value());
        assert_eq!(params.arp_pattern.value(), ArpPattern::Up);
        assert_eq!(params.arp_rate.value(), NoteDiv::Sixteenth);
        assert_eq!(params.arp_octaves.value(), 1);
        assert_eq!(params.arp_gate.value(), 60.0);
        assert!(!params.arp_latch.value());
        assert_eq!(params.arp_swing.value(), 0.0);
        assert_eq!(params.arp_length.value(), 16);
        assert_eq!(params.arp_ratchet.value(), 1);

        // The appended arp controls keep the old sound: no chord, no strum
        // and every step is played.
        assert_eq!(params.arp_chord.value(), ArpChord::Off);
        assert_eq!(params.arp_strum.value(), 0.0);
        assert_eq!(params.arp_probability.value(), 1.0);

        // The effect chain starts in its original order, without a bypass
        // and fully wet, so old patches keep their sound.
        assert_eq!(params.fx_order.value(), 0);
        assert_eq!(params.fx_bypass_mask.value(), 0);
        assert_eq!(params.fx_global_mix.value(), 1.0);
    }

    #[test]
    fn the_patch_identity_is_persisted_with_the_plugin_state() {
        let params = SushiParams::default();
        *params
            .patch_name
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some("MY PATCH".to_string());
        *params
            .patch_path
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some("/tmp/MY PATCH.json".to_string());
        *params
            .preset_index
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = 3;
        *params
            .patch_dirty
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = true;

        let fields = params.serialize_fields();
        let restored = SushiParams::default();
        restored.deserialize_fields(&fields);

        assert_eq!(
            restored
                .patch_name
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .as_deref(),
            Some("MY PATCH")
        );
        assert_eq!(
            restored
                .patch_path
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .as_deref(),
            Some("/tmp/MY PATCH.json")
        );
        assert_eq!(
            *restored
                .preset_index
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
            3
        );
        assert!(
            *restored
                .patch_dirty
                .lock()
                .unwrap_or_else(|error| error.into_inner())
        );
    }

    #[test]
    fn the_modulation_matrix_starts_empty() {
        let params = SushiParams::default();

        // Every slot of the matrix is switched off.
        let slots = params.mod_slots(64);
        assert_eq!(slots.len(), 8);
        for slot in slots {
            assert_eq!(slot.source, ModSource::Off);
            assert_eq!(slot.dest, ModDest::Off);
            assert_eq!(slot.amount, 0.0);
        }

        assert_eq!(params.mod1_source.value(), ModSource::Off);
        assert_eq!(params.mod1_dest.value(), ModDest::Off);
        assert_eq!(params.mod1_amount.value(), 0.0);
        assert_eq!(params.mod8_source.value(), ModSource::Off);
        assert_eq!(params.mod8_dest.value(), ModDest::Off);
        assert_eq!(params.mod8_amount.value(), 0.0);
    }

    #[test]
    fn the_modulation_sources_and_destinations_have_readable_names() {
        assert_eq!(ModSource::variants()[0], "Off");
        assert_eq!(ModSource::variants()[1], "LFO 1");
        assert_eq!(ModSource::variants()[3], "Mod Env");
        assert_eq!(ModSource::variants()[7], "Macro 1");
        assert_eq!(ModSource::variants()[10], "Mod Wheel");
        assert_eq!(ModSource::variants()[11], "Random");
        assert_eq!(ModDest::variants()[0], "Off");
        assert_eq!(ModDest::variants()[1], "Pitch");
        assert_eq!(ModDest::variants()[2], "Pulse 1");
        assert_eq!(ModDest::variants()[4], "Cutoff");
        assert_eq!(ModDest::variants()[8], "Volume");
        assert_eq!(ModDest::variants()[9], "Osc2 Pitch");
        assert_eq!(ModDest::variants()[10], "Osc2 Fine");
        assert_eq!(ModDest::variants()[11], "Sub Level");
        assert_eq!(ModDest::variants()[12], "Noise Level");
        assert_eq!(ModDest::variants()[13], "Detune");
        assert_eq!(ModDest::variants()[14], "Filter Env");
        assert_eq!(ModDest::variants()[15], "Osc1 Pitch");
        assert_eq!(ModDest::variants()[16], "Osc1 Fine");
        assert_eq!(ModDest::variants()[17], "Filter Drive");
        assert_eq!(ModDest::variants()[18], "Filter Keytrack");
        assert_eq!(ModDest::variants()[19], "Ring Mod");
    }

    #[test]
    fn new_modulation_variants_are_appended() {
        // The indices are part of the patch and host-state format. Existing
        // variants must keep their exact positions so old patches keep
        // sounding the same.
        assert_eq!(ModSource::to_index(ModSource::Off), 0);
        assert_eq!(ModSource::to_index(ModSource::Lfo1), 1);
        assert_eq!(ModSource::to_index(ModSource::Lfo2), 2);
        assert_eq!(ModSource::to_index(ModSource::ModEnv), 3);
        assert_eq!(ModSource::to_index(ModSource::AmpEnv), 4);
        assert_eq!(ModSource::to_index(ModSource::Velocity), 5);
        assert_eq!(ModSource::to_index(ModSource::Key), 6);
        assert_eq!(ModSource::to_index(ModSource::Macro1), 7);
        assert_eq!(ModSource::to_index(ModSource::Macro2), 8);
        assert_eq!(ModSource::to_index(ModSource::Macro3), 9);
        assert_eq!(ModSource::to_index(ModSource::ModWheel), 10);
        assert_eq!(ModSource::to_index(ModSource::Random), 11);
        assert_eq!(ModSource::variants().len(), 12);

        assert_eq!(ModDest::to_index(ModDest::Off), 0);
        assert_eq!(ModDest::to_index(ModDest::Pitch), 1);
        assert_eq!(ModDest::to_index(ModDest::Pulse1), 2);
        assert_eq!(ModDest::to_index(ModDest::Pulse2), 3);
        assert_eq!(ModDest::to_index(ModDest::Cutoff), 4);
        assert_eq!(ModDest::to_index(ModDest::Resonance), 5);
        assert_eq!(ModDest::to_index(ModDest::Drive), 6);
        assert_eq!(ModDest::to_index(ModDest::Pan), 7);
        assert_eq!(ModDest::to_index(ModDest::Volume), 8);
        assert_eq!(ModDest::to_index(ModDest::Osc2Pitch), 9);
        assert_eq!(ModDest::to_index(ModDest::Osc2Fine), 10);
        assert_eq!(ModDest::to_index(ModDest::SubLevel), 11);
        assert_eq!(ModDest::to_index(ModDest::NoiseLevel), 12);
        assert_eq!(ModDest::to_index(ModDest::Detune), 13);
        assert_eq!(ModDest::to_index(ModDest::FilterEnv), 14);
        assert_eq!(ModDest::to_index(ModDest::Osc1Pitch), 15);
        assert_eq!(ModDest::to_index(ModDest::Osc1Fine), 16);
        assert_eq!(ModDest::to_index(ModDest::FilterDrive), 17);
        assert_eq!(ModDest::to_index(ModDest::FilterKeytrack), 18);
        assert_eq!(ModDest::to_index(ModDest::RingMod), 19);
        assert_eq!(ModDest::variants().len(), 20);
    }

    #[test]
    fn the_tune_is_shown_in_semitones() {
        let params = SushiParams::default();

        // The tune is shown with two decimals and the unit of the semitone.
        assert_eq!(params.tune.to_string(), "0.00 st");

        // A value that was typed is read back.
        let normalized = params
            .tune
            .string_to_normalized_value("-1.5 st")
            .expect("the tune could not be read");
        let value = params.tune.range().unnormalize(normalized);
        assert!((value + 1.5).abs() < 0.01, "the tune was {value}");
    }

    #[test]
    fn the_note_divisions_have_the_right_length() {
        assert_eq!(NoteDiv::Quarter.beats(), 1.0);
        assert_eq!(NoteDiv::Eighth.beats(), 0.5);
        assert_eq!(NoteDiv::Sixteenth.beats(), 0.25);
        assert_eq!(NoteDiv::ThirtySecond.beats(), 0.125);
    }

    #[test]
    fn the_variants_have_readable_names() {
        assert_eq!(NoteDiv::variants(), ["1/4", "1/8", "1/16", "1/32"]);
        assert_eq!(
            ArpPattern::variants(),
            ["Up", "Down", "UpDown", "Random", "As Played"]
        );
        assert_eq!(
            ArpChord::variants(),
            ["Off", "Octave", "Fifth", "Minor", "Major"]
        );
        assert_eq!(ArpChord::to_index(ArpChord::Off), 0);
        assert_eq!(ArpChord::to_index(ArpChord::Major), 4);
        assert_eq!(Character::variants(), ["Silk", "Grit"]);
        assert_eq!(FilterSlope::variants(), ["12 dB", "24 dB OLD", "24 dB"]);
        // Old host states address the old ladder by its index.
        assert_eq!(FilterSlope::from_index(1), FilterSlope::FourPole);
    }
}
