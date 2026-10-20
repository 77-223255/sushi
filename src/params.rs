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
}

/// The note division of the arpeggiator and of the step sequencer.
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

/// The destination the step sequencer modulates.
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy, Default)]
pub enum StepDest {
    Pitch,
    #[default]
    Filter,
    Shape,
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

/// A step of the modulation sequencer. The steps are smoothed a bit more
/// gently than the macros, so that they still feel like discrete steps.
fn step_param(name: &str, default: f32) -> FloatParam {
    percentage_param(name, default).with_smoother(SmoothingStyle::Linear(5.0))
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

    // --- Output -----------------------------------------------------------
    #[id = "volume"]
    pub volume: FloatParam,

    // --- Filter -----------------------------------------------------------
    #[id = "filter_mode"]
    pub filter_mode: EnumParam<FilterMode>,

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

    // --- Step sequencer ----------------------------------------------------
    #[id = "step_on"]
    pub step_on: BoolParam,

    #[id = "step_rate"]
    pub step_rate: EnumParam<NoteDiv>,

    #[id = "step_dest"]
    pub step_dest: EnumParam<StepDest>,

    #[id = "step_amount"]
    pub step_amount: FloatParam,

    #[id = "step_smooth"]
    pub step_smooth: BoolParam,

    #[id = "step_1"]
    pub step_1: FloatParam,

    #[id = "step_2"]
    pub step_2: FloatParam,

    #[id = "step_3"]
    pub step_3: FloatParam,

    #[id = "step_4"]
    pub step_4: FloatParam,

    #[id = "step_5"]
    pub step_5: FloatParam,

    #[id = "step_6"]
    pub step_6: FloatParam,

    #[id = "step_7"]
    pub step_7: FloatParam,

    #[id = "step_8"]
    pub step_8: FloatParam,
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

            // --- Step sequencer ----------------------------------------------
            step_on: BoolParam::new("Step Sequencer", false),
            step_rate: EnumParam::new("Step Rate", NoteDiv::Eighth),
            step_dest: EnumParam::new("Step Dest", StepDest::Filter),
            step_amount: percentage_param("Step Amount", 0.0)
                .with_smoother(SmoothingStyle::Linear(20.0)),
            step_smooth: BoolParam::new("Step Smooth", true),
            step_1: step_param("Step 1", 0.5),
            step_2: step_param("Step 2", 0.65),
            step_3: step_param("Step 3", 0.35),
            step_4: step_param("Step 4", 0.8),
            step_5: step_param("Step 5", 0.5),
            step_6: step_param("Step 6", 0.7),
            step_7: step_param("Step 7", 0.3),
            step_8: step_param("Step 8", 0.6),
        }
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

        assert!(!params.step_on.value());
        assert_eq!(params.step_rate.value(), NoteDiv::Eighth);
        assert_eq!(params.step_dest.value(), StepDest::Filter);
        assert_eq!(params.step_amount.value(), 0.0);
        assert!(params.step_smooth.value());

        let steps = [
            params.step_1.value(),
            params.step_2.value(),
            params.step_3.value(),
            params.step_4.value(),
            params.step_5.value(),
            params.step_6.value(),
            params.step_7.value(),
            params.step_8.value(),
        ];
        assert_eq!(steps, [0.5, 0.65, 0.35, 0.8, 0.5, 0.7, 0.3, 0.6]);
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
        assert_eq!(ArpPattern::variants(), ["Up", "Down", "UpDown", "Random"]);
        assert_eq!(StepDest::variants(), ["Pitch", "Filter", "Shape"]);
        assert_eq!(Character::variants(), ["Silk", "Grit"]);
    }
}
