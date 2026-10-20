//! All parameter and enum definitions for the Sushi synthesizer.
//!
//! The parameter IDs are part of the plugin's public interface (they are used
//! by hosts for automation and for saving presets), so they must never change.

use nice_plug::prelude::*;
use std::sync::Arc;

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

/// The slope of the filter. The four-pole mode runs the signal through two
/// cascaded state variable filters.
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy, Default)]
pub enum FilterSlope {
    #[default]
    #[name = "12 dB"]
    TwoPole,
    #[name = "24 dB"]
    FourPole,
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

/// The destination an LFO modulates.
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

/// The note division of the arpeggiator.
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
            .with_unit("%"),
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
            .with_unit("%"),
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
            .with_unit(" cents"),
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
        }
    }
}

impl SushiParams {
    /// The eight slots of the modulation matrix, in the order of their
    /// parameters.
    ///
    /// The slots are built once per audio block, so that the audio thread
    /// never has to allocate.
    pub fn mod_slots(&self) -> [ModSlot; MOD_SLOTS] {
        [
            ModSlot {
                source: self.mod1_source.value(),
                dest: self.mod1_dest.value(),
                amount: self.mod1_amount.value(),
            },
            ModSlot {
                source: self.mod2_source.value(),
                dest: self.mod2_dest.value(),
                amount: self.mod2_amount.value(),
            },
            ModSlot {
                source: self.mod3_source.value(),
                dest: self.mod3_dest.value(),
                amount: self.mod3_amount.value(),
            },
            ModSlot {
                source: self.mod4_source.value(),
                dest: self.mod4_dest.value(),
                amount: self.mod4_amount.value(),
            },
            ModSlot {
                source: self.mod5_source.value(),
                dest: self.mod5_dest.value(),
                amount: self.mod5_amount.value(),
            },
            ModSlot {
                source: self.mod6_source.value(),
                dest: self.mod6_dest.value(),
                amount: self.mod6_amount.value(),
            },
            ModSlot {
                source: self.mod7_source.value(),
                dest: self.mod7_dest.value(),
                amount: self.mod7_amount.value(),
            },
            ModSlot {
                source: self.mod8_source.value(),
                dest: self.mod8_dest.value(),
                amount: self.mod8_amount.value(),
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
        assert_eq!(params.chorus_rate.value(), 0.8);
        assert_eq!(params.chorus_depth.value(), 0.5);
        assert_eq!(params.delay_time.value(), 350.0);
        assert_eq!(params.delay_feedback.value(), 0.3);
        assert_eq!(params.reverb_decay.value(), 0.6);
    }

    #[test]
    fn the_new_defaults_match_the_spec() {
        let params = SushiParams::default();

        assert_eq!(params.macro_bright.value(), 0.5);
        assert_eq!(params.macro_motion.value(), 0.5);
        assert_eq!(params.macro_space.value(), 0.5);
        assert_eq!(params.character.value(), Character::Silk);
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
    }

    #[test]
    fn the_modulation_matrix_starts_empty() {
        let params = SushiParams::default();

        // Every slot of the matrix is switched off.
        let slots = params.mod_slots();
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
        assert_eq!(ModDest::variants()[0], "Off");
        assert_eq!(ModDest::variants()[1], "Pitch");
        assert_eq!(ModDest::variants()[2], "Pulse 1");
        assert_eq!(ModDest::variants()[4], "Cutoff");
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
        assert_eq!(Character::variants(), ["Silk", "Grit"]);
    }
}
