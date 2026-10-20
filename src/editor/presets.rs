//! The factory presets of Sushi and the mutation of the current sound.
//!
//! A preset is a list of parameter values that is written on top of the
//! defaults of the synthesizer. The mutation takes the current sound and
//! moves a handful of musical parameters a little bit, so that new sounds
//! can be found without a piano keyboard.

use nice_plug::prelude::*;

use crate::params::{
    ArpPattern, Character, FilterMode, LfoTarget, LfoWaveform, NoteDiv, PolyMode, StepDest,
    SushiParams, Waveform,
};

/// A value that a preset can write into a parameter.
#[derive(Clone, Copy, Debug)]
pub enum Value {
    /// A floating point parameter.
    Float(f32),
    /// A parameter that is shown in decibels but stored as a gain.
    Decibels(f32),
    /// An integer parameter.
    Int(i32),
    /// A boolean parameter.
    Bool(bool),
    /// The waveform of an oscillator.
    Wave(Waveform),
    /// The mode of the filter.
    Filter(FilterMode),
    /// The waveform of an LFO.
    LfoWave(LfoWaveform),
    /// The destination of an LFO.
    LfoTarget(LfoTarget),
    /// The way the notes are turned into voices.
    Poly(PolyMode),
    /// The character of the sound.
    Character(Character),
    /// The pattern of the arpeggiator.
    ArpPattern(ArpPattern),
    /// The note division of the arpeggiator or of the sequencer.
    NoteDiv(NoteDiv),
    /// The destination of the step sequencer.
    StepDest(StepDest),
}

/// Defines the table of all the parameters of the synthesizer.
///
/// Every parameter is listed exactly once, with the kind of value it takes
/// and the conversion of that value into the plain value of the parameter.
macro_rules! parameters {
    ($( $id:literal => $field:ident : $kind:ident $convert:expr ),* $(,)?) => {
        /// Writes the default value of every parameter into the host.
        pub fn apply_defaults(params: &SushiParams, setter: &ParamSetter) {
            $(
                setter.set_parameter(&params.$field, params.$field.default_plain_value());
            )*
        }

        /// Writes a single value into the parameter with the given id.
        ///
        /// Values that don't fit the parameter are ignored.
        fn set_value(params: &SushiParams, setter: &ParamSetter, id: &str, value: Value) {
            match (id, value) {
                $(
                    ($id, Value::$kind(raw)) => {
                        setter.set_parameter(&params.$field, ($convert)(raw));
                    }
                )*
                _ => {}
            }
        }
    };
}

parameters! {
    "osc1_wave" => osc1_wave: Wave(|value: Waveform| value),
    "osc1_octave" => osc1_octave: Int(|value: i32| value),
    "osc1_semi" => osc1_semi: Int(|value: i32| value),
    "osc1_fine" => osc1_fine: Float(|value: f32| value),
    "osc1_pw" => osc1_pw: Float(|value: f32| value),
    "osc1_level" => osc1_level: Float(|value: f32| value),
    "osc2_wave" => osc2_wave: Wave(|value: Waveform| value),
    "osc2_octave" => osc2_octave: Int(|value: i32| value),
    "osc2_semi" => osc2_semi: Int(|value: i32| value),
    "osc2_fine" => osc2_fine: Float(|value: f32| value),
    "osc2_pw" => osc2_pw: Float(|value: f32| value),
    "osc2_level" => osc2_level: Float(|value: f32| value),
    "noise_level" => noise_level: Float(|value: f32| value),
    "unison" => unison: Int(|value: i32| value),
    "detune" => detune: Float(|value: f32| value),
    "spread" => spread: Float(|value: f32| value),
    "poly_mode" => poly_mode: Poly(|value: PolyMode| value),
    "glide" => glide: Float(|value: f32| value),
    "volume" => volume: Decibels(|value: f32| util::db_to_gain(value)),
    "filter_mode" => filter_mode: Filter(|value: FilterMode| value),
    "cutoff" => cutoff: Float(|value: f32| value),
    "resonance" => resonance: Float(|value: f32| value),
    "filter_env_amount" => filter_env_amount: Float(|value: f32| value),
    "filter_keytrack" => filter_keytrack: Float(|value: f32| value),
    "filter_drive" => filter_drive: Float(|value: f32| value),
    "fe_attack" => fe_attack: Float(|value: f32| value),
    "fe_decay" => fe_decay: Float(|value: f32| value),
    "fe_sustain" => fe_sustain: Float(|value: f32| value),
    "fe_release" => fe_release: Float(|value: f32| value),
    "ae_attack" => ae_attack: Float(|value: f32| value),
    "ae_decay" => ae_decay: Float(|value: f32| value),
    "ae_sustain" => ae_sustain: Float(|value: f32| value),
    "ae_release" => ae_release: Float(|value: f32| value),
    "lfo1_wave" => lfo1_wave: LfoWave(|value: LfoWaveform| value),
    "lfo1_rate" => lfo1_rate: Float(|value: f32| value),
    "lfo1_amount" => lfo1_amount: Float(|value: f32| value),
    "lfo1_target" => lfo1_target: LfoTarget(|value: LfoTarget| value),
    "lfo2_wave" => lfo2_wave: LfoWave(|value: LfoWaveform| value),
    "lfo2_rate" => lfo2_rate: Float(|value: f32| value),
    "lfo2_amount" => lfo2_amount: Float(|value: f32| value),
    "lfo2_target" => lfo2_target: LfoTarget(|value: LfoTarget| value),
    "chorus_mix" => chorus_mix: Float(|value: f32| value),
    "chorus_rate" => chorus_rate: Float(|value: f32| value),
    "chorus_depth" => chorus_depth: Float(|value: f32| value),
    "delay_time" => delay_time: Float(|value: f32| value),
    "delay_feedback" => delay_feedback: Float(|value: f32| value),
    "delay_mix" => delay_mix: Float(|value: f32| value),
    "reverb_mix" => reverb_mix: Float(|value: f32| value),
    "reverb_decay" => reverb_decay: Float(|value: f32| value),
    "macro_bright" => macro_bright: Float(|value: f32| value),
    "macro_motion" => macro_motion: Float(|value: f32| value),
    "macro_space" => macro_space: Float(|value: f32| value),
    "character" => character: Character(|value: Character| value),
    "drift" => drift: Float(|value: f32| value),
    "velocity" => velocity: Float(|value: f32| value),
    "tone" => tone: Float(|value: f32| value),
    "arp_on" => arp_on: Bool(|value: bool| value),
    "arp_pattern" => arp_pattern: ArpPattern(|value: ArpPattern| value),
    "arp_rate" => arp_rate: NoteDiv(|value: NoteDiv| value),
    "arp_octaves" => arp_octaves: Int(|value: i32| value),
    "arp_gate" => arp_gate: Float(|value: f32| value),
    "arp_latch" => arp_latch: Bool(|value: bool| value),
    "step_on" => step_on: Bool(|value: bool| value),
    "step_rate" => step_rate: NoteDiv(|value: NoteDiv| value),
    "step_dest" => step_dest: StepDest(|value: StepDest| value),
    "step_amount" => step_amount: Float(|value: f32| value),
    "step_smooth" => step_smooth: Bool(|value: bool| value),
    "step_1" => step_1: Float(|value: f32| value),
    "step_2" => step_2: Float(|value: f32| value),
    "step_3" => step_3: Float(|value: f32| value),
    "step_4" => step_4: Float(|value: f32| value),
    "step_5" => step_5: Float(|value: f32| value),
    "step_6" => step_6: Float(|value: f32| value),
    "step_7" => step_7: Float(|value: f32| value),
    "step_8" => step_8: Float(|value: f32| value),
}

/// A factory preset of the synthesizer.
pub struct Preset {
    /// The name of the preset, as it is shown in the display of the editor.
    pub name: &'static str,
    /// The values that differ from the defaults of the synthesizer.
    pub values: &'static [(&'static str, Value)],
}

/// The factory presets of the synthesizer.
pub const PRESETS: &[Preset] = &[
    Preset {
        name: "INIT PAD",
        values: &[
            ("osc2_wave", Value::Wave(Waveform::Triangle)),
            ("osc2_octave", Value::Int(-1)),
            ("osc2_semi", Value::Int(7)),
            ("cutoff", Value::Float(900.0)),
            ("resonance", Value::Float(0.15)),
            ("filter_env_amount", Value::Float(0.25)),
            ("fe_attack", Value::Float(300.0)),
            ("fe_decay", Value::Float(900.0)),
            ("fe_sustain", Value::Float(0.4)),
            ("fe_release", Value::Float(900.0)),
            ("ae_attack", Value::Float(200.0)),
            ("ae_decay", Value::Float(600.0)),
            ("ae_sustain", Value::Float(0.8)),
            ("ae_release", Value::Float(700.0)),
            ("lfo1_rate", Value::Float(0.3)),
            ("lfo1_target", Value::LfoTarget(LfoTarget::Filter)),
            ("unison", Value::Int(2)),
            ("spread", Value::Float(0.6)),
            ("chorus_mix", Value::Float(0.3)),
            ("reverb_mix", Value::Float(0.25)),
            ("volume", Value::Decibels(-4.0)),
        ],
    },
    Preset {
        name: "GLASS KEYS",
        values: &[
            ("osc1_wave", Value::Wave(Waveform::Sine)),
            ("osc2_wave", Value::Wave(Waveform::Triangle)),
            ("osc2_octave", Value::Int(1)),
            ("cutoff", Value::Float(4_200.0)),
            ("resonance", Value::Float(0.1)),
            ("filter_env_amount", Value::Float(0.15)),
            ("fe_attack", Value::Float(2.0)),
            ("fe_decay", Value::Float(1_200.0)),
            ("fe_sustain", Value::Float(0.1)),
            ("fe_release", Value::Float(1_200.0)),
            ("ae_attack", Value::Float(3.0)),
            ("ae_decay", Value::Float(1_500.0)),
            ("ae_sustain", Value::Float(0.05)),
            ("ae_release", Value::Float(1_500.0)),
            ("lfo2_rate", Value::Float(4.5)),
            ("lfo2_target", Value::LfoTarget(LfoTarget::Pitch)),
            ("glide", Value::Float(60.0)),
            ("delay_mix", Value::Float(0.25)),
            ("delay_feedback", Value::Float(0.4)),
            ("reverb_mix", Value::Float(0.3)),
            ("velocity", Value::Float(0.7)),
        ],
    },
    Preset {
        name: "GRIT BASS",
        values: &[
            ("osc1_wave", Value::Wave(Waveform::Saw)),
            ("osc2_wave", Value::Wave(Waveform::Square)),
            ("osc2_octave", Value::Int(-1)),
            ("cutoff", Value::Float(420.0)),
            ("resonance", Value::Float(0.55)),
            ("filter_drive", Value::Float(0.4)),
            ("filter_env_amount", Value::Float(0.6)),
            ("fe_attack", Value::Float(2.0)),
            ("fe_decay", Value::Float(180.0)),
            ("fe_sustain", Value::Float(0.15)),
            ("fe_release", Value::Float(200.0)),
            ("ae_attack", Value::Float(2.0)),
            ("ae_decay", Value::Float(300.0)),
            ("ae_sustain", Value::Float(0.6)),
            ("ae_release", Value::Float(200.0)),
            ("poly_mode", Value::Poly(PolyMode::Mono)),
            ("unison", Value::Int(3)),
            ("detune", Value::Float(25.0)),
            ("character", Value::Character(Character::Grit)),
            ("tone", Value::Float(0.3)),
        ],
    },
    Preset {
        name: "PLUCK",
        values: &[
            ("osc1_wave", Value::Wave(Waveform::Square)),
            ("osc2_wave", Value::Wave(Waveform::Saw)),
            ("osc2_octave", Value::Int(1)),
            ("cutoff", Value::Float(1_800.0)),
            ("resonance", Value::Float(0.35)),
            ("filter_env_amount", Value::Float(0.7)),
            ("fe_attack", Value::Float(1.0)),
            ("fe_decay", Value::Float(220.0)),
            ("fe_sustain", Value::Float(0.0)),
            ("fe_release", Value::Float(250.0)),
            ("ae_attack", Value::Float(1.0)),
            ("ae_decay", Value::Float(260.0)),
            ("ae_sustain", Value::Float(0.0)),
            ("ae_release", Value::Float(300.0)),
            ("lfo1_rate", Value::Float(6.0)),
            ("lfo1_target", Value::LfoTarget(LfoTarget::Filter)),
            ("delay_mix", Value::Float(0.3)),
            ("delay_feedback", Value::Float(0.35)),
            ("reverb_mix", Value::Float(0.15)),
            ("macro_bright", Value::Float(0.55)),
        ],
    },
    Preset {
        name: "WIDE BRASS",
        values: &[
            ("osc2_wave", Value::Wave(Waveform::Saw)),
            ("osc2_octave", Value::Int(1)),
            ("osc2_semi", Value::Int(7)),
            ("osc2_level", Value::Float(0.5)),
            ("cutoff", Value::Float(1_200.0)),
            ("resonance", Value::Float(0.25)),
            ("filter_env_amount", Value::Float(0.3)),
            ("ae_attack", Value::Float(60.0)),
            ("ae_decay", Value::Float(400.0)),
            ("ae_sustain", Value::Float(0.85)),
            ("ae_release", Value::Float(400.0)),
            ("lfo1_rate", Value::Float(5.2)),
            ("lfo1_amount", Value::Float(0.12)),
            ("lfo1_target", Value::LfoTarget(LfoTarget::Pitch)),
            ("poly_mode", Value::Poly(PolyMode::Legato)),
            ("glide", Value::Float(80.0)),
            ("unison", Value::Int(4)),
            ("detune", Value::Float(35.0)),
            ("spread", Value::Float(0.9)),
            ("chorus_mix", Value::Float(0.5)),
            ("chorus_depth", Value::Float(0.7)),
        ],
    },
    Preset {
        name: "STEP MOTION",
        values: &[
            ("osc1_wave", Value::Wave(Waveform::Saw)),
            ("osc2_wave", Value::Wave(Waveform::Square)),
            ("cutoff", Value::Float(1_500.0)),
            ("resonance", Value::Float(0.3)),
            ("filter_env_amount", Value::Float(0.35)),
            ("fe_decay", Value::Float(250.0)),
            ("fe_sustain", Value::Float(0.2)),
            ("step_on", Value::Bool(true)),
            ("step_rate", Value::NoteDiv(NoteDiv::Sixteenth)),
            ("step_dest", Value::StepDest(StepDest::Filter)),
            ("step_amount", Value::Float(0.6)),
            ("step_smooth", Value::Bool(false)),
            ("step_1", Value::Float(0.9)),
            ("step_2", Value::Float(0.2)),
            ("step_3", Value::Float(0.7)),
            ("step_4", Value::Float(0.1)),
            ("step_5", Value::Float(0.8)),
            ("step_6", Value::Float(0.3)),
            ("step_7", Value::Float(0.6)),
            ("step_8", Value::Float(0.4)),
            ("filter_mode", Value::Filter(FilterMode::Bandpass)),
            ("lfo1_wave", Value::LfoWave(LfoWaveform::SampleHold)),
            ("lfo1_rate", Value::Float(0.8)),
            ("lfo1_amount", Value::Float(0.15)),
            ("lfo1_target", Value::LfoTarget(LfoTarget::Pitch)),
            ("macro_motion", Value::Float(0.7)),
        ],
    },
    Preset {
        name: "ARP TOY",
        values: &[
            ("osc1_wave", Value::Wave(Waveform::Pulse)),
            ("osc2_wave", Value::Wave(Waveform::Square)),
            ("osc2_octave", Value::Int(1)),
            ("osc1_pw", Value::Float(0.35)),
            ("cutoff", Value::Float(2_200.0)),
            ("resonance", Value::Float(0.4)),
            ("filter_env_amount", Value::Float(0.5)),
            ("fe_decay", Value::Float(300.0)),
            ("fe_sustain", Value::Float(0.25)),
            ("ae_attack", Value::Float(2.0)),
            ("ae_decay", Value::Float(400.0)),
            ("ae_sustain", Value::Float(0.5)),
            ("arp_on", Value::Bool(true)),
            ("arp_pattern", Value::ArpPattern(ArpPattern::Up)),
            ("arp_rate", Value::NoteDiv(NoteDiv::Sixteenth)),
            ("arp_octaves", Value::Int(2)),
            ("arp_gate", Value::Float(45.0)),
            ("lfo1_wave", Value::LfoWave(LfoWaveform::Triangle)),
            ("lfo1_rate", Value::Float(3.0)),
            ("lfo1_amount", Value::Float(0.1)),
            ("lfo1_target", Value::LfoTarget(LfoTarget::Pitch)),
            ("delay_mix", Value::Float(0.35)),
            ("reverb_mix", Value::Float(0.3)),
            ("drift", Value::Float(0.25)),
            ("macro_motion", Value::Float(0.6)),
        ],
    },
    Preset {
        name: "SILK STRINGS",
        values: &[
            ("osc1_wave", Value::Wave(Waveform::Saw)),
            ("osc2_wave", Value::Wave(Waveform::Triangle)),
            ("osc2_semi", Value::Int(5)),
            ("osc2_level", Value::Float(0.45)),
            ("cutoff", Value::Float(2_400.0)),
            ("resonance", Value::Float(0.1)),
            ("filter_env_amount", Value::Float(0.15)),
            ("fe_attack", Value::Float(400.0)),
            ("fe_sustain", Value::Float(0.7)),
            ("ae_attack", Value::Float(400.0)),
            ("ae_decay", Value::Float(900.0)),
            ("ae_sustain", Value::Float(0.9)),
            ("ae_release", Value::Float(1_200.0)),
            ("lfo1_rate", Value::Float(0.2)),
            ("lfo1_amount", Value::Float(0.06)),
            ("lfo1_target", Value::LfoTarget(LfoTarget::Pitch)),
            ("unison", Value::Int(3)),
            ("detune", Value::Float(18.0)),
            ("spread", Value::Float(0.8)),
            ("chorus_mix", Value::Float(0.6)),
            ("reverb_mix", Value::Float(0.45)),
            ("tone", Value::Float(-0.15)),
        ],
    },
];

/// Writes the preset with the given index into the host.
///
/// The defaults of the synthesizer are written first, so that a preset always
/// sounds the same, no matter what was played before it.
pub fn apply(params: &SushiParams, setter: &ParamSetter, index: usize) {
    apply_defaults(params, setter);

    let Some(preset) = PRESETS.get(index) else {
        return;
    };

    for (id, value) in preset.values {
        set_value(params, setter, id, *value);
    }
}

/// A small xorshift random number generator.
pub struct Rng {
    state: u32,
}

impl Rng {
    /// Creates a generator with the given seed.
    pub fn new(seed: u32) -> Self {
        Self { state: seed.max(1) }
    }

    /// Returns a random number between zero and one.
    pub fn next_f32(&mut self) -> f32 {
        let mut state = self.state;
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        self.state = state;

        (state >> 8) as f32 / 16_777_216.0
    }

    /// Returns a random number between minus one and one.
    pub fn next_signed(&mut self) -> f32 {
        self.next_f32() * 2.0 - 1.0
    }
}

/// Moves a value by an absolute amount and clamps it to its range.
fn jitter(rng: &mut Rng, value: f32, min: f32, max: f32, amount: f32) -> f32 {
    (value + rng.next_signed() * amount).clamp(min, max)
}

/// Moves a value by a relative amount and clamps it to its range.
fn jitter_relative(rng: &mut Rng, value: f32, min: f32, max: f32, amount: f32) -> f32 {
    (value * (1.0 + rng.next_signed() * amount)).clamp(min, max)
}

/// Randomizes a musical subset of the parameters.
///
/// The values are read before they are written, so that the sound does not
/// depend on the order in which the host applies the changes.
pub fn mutate(params: &SushiParams, setter: &ParamSetter, rng: &mut Rng) {
    let cutoff = jitter_relative(rng, params.cutoff.value(), 20.0, 20_000.0, 0.2);
    let resonance = jitter(rng, params.resonance.value(), 0.0, 1.0, 0.12);
    let pw1 = jitter(rng, params.osc1_pw.value(), 0.05, 0.95, 0.08);
    let pw2 = jitter(rng, params.osc2_pw.value(), 0.05, 0.95, 0.08);
    let lfo1_rate = jitter_relative(rng, params.lfo1_rate.value(), 0.01, 40.0, 0.25);
    let lfo2_rate = jitter_relative(rng, params.lfo2_rate.value(), 0.01, 40.0, 0.25);
    let lfo1_amount = jitter(rng, params.lfo1_amount.value(), 0.0, 1.0, 0.08);
    let lfo2_amount = jitter(rng, params.lfo2_amount.value(), 0.0, 1.0, 0.08);
    let step_amount = jitter(rng, params.step_amount.value(), 0.0, 1.0, 0.1);
    let bright = jitter(rng, params.macro_bright.value(), 0.0, 1.0, 0.08);
    let motion = jitter(rng, params.macro_motion.value(), 0.0, 1.0, 0.08);
    let space = jitter(rng, params.macro_space.value(), 0.0, 1.0, 0.08);
    let drift = jitter(rng, params.drift.value(), 0.0, 1.0, 0.08);
    let chorus = jitter(rng, params.chorus_mix.value(), 0.0, 1.0, 0.1);
    let delay = jitter(rng, params.delay_mix.value(), 0.0, 1.0, 0.1);
    let reverb = jitter(rng, params.reverb_mix.value(), 0.0, 1.0, 0.1);

    setter.set_parameter(&params.cutoff, cutoff);
    setter.set_parameter(&params.resonance, resonance);
    setter.set_parameter(&params.osc1_pw, pw1);
    setter.set_parameter(&params.osc2_pw, pw2);
    setter.set_parameter(&params.lfo1_rate, lfo1_rate);
    setter.set_parameter(&params.lfo2_rate, lfo2_rate);
    setter.set_parameter(&params.lfo1_amount, lfo1_amount);
    setter.set_parameter(&params.lfo2_amount, lfo2_amount);
    setter.set_parameter(&params.step_amount, step_amount);
    setter.set_parameter(&params.macro_bright, bright);
    setter.set_parameter(&params.macro_motion, motion);
    setter.set_parameter(&params.macro_space, space);
    setter.set_parameter(&params.drift, drift);
    setter.set_parameter(&params.chorus_mix, chorus);
    setter.set_parameter(&params.delay_mix, delay);
    setter.set_parameter(&params.reverb_mix, reverb);
}

#[cfg(test)]
mod tests {
    use super::*;
    use nice_plug::params::internals::ParamPtr;

    use std::cell::Cell;

    thread_local! {
        /// The number of parameter values that were set.
        static SETS: Cell<usize> = const { Cell::new(0) };
    }

    struct Stub;

    impl GuiContextInner for Stub {
        fn plugin_api(&self) -> nice_plug::context::PluginApi {
            unimplemented!()
        }
        unsafe fn raw_begin_set_parameter(&self, _param: ParamPtr) {}
        unsafe fn raw_set_parameter_normalized(&self, param: ParamPtr, normalized: f32) {
            SETS.with(|sets| sets.set(sets.get() + 1));
            unsafe {
                param._internal_set_normalized_value(normalized);
            }
        }
        unsafe fn raw_end_set_parameter(&self, _param: ParamPtr) {}
        fn get_state(&self) -> PluginState {
            unimplemented!()
        }
        fn set_state(&self, _state: PluginState) {}
        fn request_restart(&self) {}
    }

    /// A setter that writes into the parameters of a synthesizer.
    fn setter() -> ParamSetter<'static> {
        // The stub is leaked on purpose: the setter lives as long as the test.
        let inner: &'static dyn GuiContextInner = Box::leak(Box::new(Stub));
        ParamSetter::new(inner)
    }

    #[test]
    fn there_are_eight_presets_with_short_names() {
        assert_eq!(PRESETS.len(), 8);
        for preset in PRESETS {
            assert!(!preset.name.is_empty());
            assert!(preset.name.len() <= 18, "{}", preset.name);
            assert!(preset.name.is_ascii(), "{}", preset.name);
        }
    }

    #[test]
    fn the_defaults_are_written_into_the_host() {
        let params = SushiParams::default();
        let setter = setter();

        SETS.with(|sets| sets.set(0));
        apply_defaults(&params, &setter);

        // Every one of the 75 parameters is written.
        assert_eq!(SETS.with(|sets| sets.get()), 75);
    }

    #[test]
    fn a_preset_only_touches_known_parameters() {
        for preset in PRESETS {
            assert!(!preset.values.is_empty());
            for (id, _) in preset.values {
                assert!(!id.is_empty());
                assert!(id.is_ascii());
            }
        }
    }

    #[test]
    fn the_presets_change_the_sound() {
        let params = SushiParams::default();
        let setter = setter();

        apply(&params, &setter, 1);
        assert!((params.cutoff.value() - 4_200.0).abs() < 1.0);
        assert_eq!(params.osc1_wave.value(), Waveform::Sine);

        apply(&params, &setter, 0);
        assert!((params.cutoff.value() - 900.0).abs() < 1.0);
        assert_eq!(params.osc1_wave.value(), Waveform::Saw);
    }

    #[test]
    fn the_mutation_moves_the_parameters() {
        let params = SushiParams::default();
        let setter = setter();
        let mut rng = Rng::new(0x1234_5678);

        let cutoff = params.cutoff.value();
        let resonance = params.resonance.value();

        mutate(&params, &setter, &mut rng);

        assert!(params.cutoff.value() != cutoff);
        assert!(params.resonance.value() != resonance);
        assert!(params.cutoff.value() >= 20.0 && params.cutoff.value() <= 20_000.0);
        assert!(params.resonance.value() >= 0.0 && params.resonance.value() <= 1.0);
    }

    #[test]
    fn the_random_number_generator_stays_in_its_range() {
        let mut rng = Rng::new(1);
        for _ in 0..1000 {
            let value = rng.next_f32();
            assert!((0.0..1.0).contains(&value), "{value}");
        }
    }
}
