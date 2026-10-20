//! The factory presets of Sushi and the mutation of the current sound.
//!
//! A preset is a patch in the same JSON format that the patch files use, so
//! that a preset is read with exactly the same code as a patch that the user
//! saved. The factory defaults are written first, so that a preset always
//! sounds the same, no matter what was played before it. The tune and the
//! number of voices are part of those defaults, so that every preset starts
//! at the same pitch and with the same polyphony.

use nice_plug::prelude::*;
use nice_plug::tracing::warn;
use serde_json::Value;

use crate::params::SushiParams;
use crate::patch;

/// A factory preset of the synthesizer.
pub struct Preset {
    /// The name of the preset, as it is shown in the display of the editor.
    pub name: &'static str,
    /// The patch of the preset, in the JSON format of the patch files.
    pub json: &'static str,
}

/// The factory presets of the synthesizer.
pub const PRESETS: &[Preset] = &[
    Preset {
        name: "INIT PAD",
        json: r#"{
            "osc2_wave": "Triangle",
            "osc2_octave": -1,
            "osc2_semi": 7,
            "cutoff": 900.0,
            "resonance": 0.15,
            "filter_env_amount": 0.25,
            "fe_attack": 300.0,
            "fe_decay": 900.0,
            "fe_sustain": 0.4,
            "fe_release": 900.0,
            "ae_attack": 200.0,
            "ae_decay": 600.0,
            "ae_sustain": 0.8,
            "ae_release": 700.0,
            "lfo1_rate": 0.3,
            "lfo1_target": "Filter",
            "unison": 2,
            "spread": 0.6,
            "chorus_mix": 0.3,
            "reverb_mix": 0.25,
            "volume": 0.63
        }"#,
    },
    Preset {
        name: "GLASS KEYS",
        json: r#"{
            "osc1_wave": "Pulse",
            "osc1_pw": 0.4,
            "osc2_wave": "Triangle",
            "osc2_octave": 1,
            "osc2_level": 0.35,
            "cutoff": 4200.0,
            "resonance": 0.1,
            "filter_env_amount": 0.15,
            "fe_attack": 2.0,
            "fe_decay": 900.0,
            "fe_sustain": 0.1,
            "fe_release": 900.0,
            "ae_attack": 3.0,
            "ae_decay": 1200.0,
            "ae_sustain": 0.15,
            "ae_release": 1000.0,
            "chorus_mix": 0.3,
            "reverb_mix": 0.3,
            "velocity": 0.7,
            "macro_bright": 0.65,
            "volume": 0.63
        }"#,
    },
    Preset {
        name: "JUNO PAD",
        json: r#"{
            "osc1_wave": "Saw",
            "osc2_wave": "Saw",
            "osc2_octave": -1,
            "osc2_level": 0.5,
            "unison": 2,
            "detune": 10.0,
            "spread": 0.4,
            "cutoff": 1800.0,
            "resonance": 0.15,
            "filter_env_amount": 0.2,
            "fe_attack": 200.0,
            "fe_decay": 800.0,
            "fe_sustain": 0.35,
            "fe_release": 600.0,
            "ae_attack": 250.0,
            "ae_decay": 900.0,
            "ae_sustain": 0.85,
            "ae_release": 900.0,
            "chorus_mix": 0.55,
            "chorus_depth": 0.6,
            "reverb_mix": 0.35,
            "reverb_decay": 0.7,
            "character": "Silk",
            "drift": 0.2,
            "volume": 0.63
        }"#,
    },
    Preset {
        name: "PROPHET BRASS",
        json: r#"{
            "osc1_wave": "Saw",
            "osc2_wave": "Saw",
            "osc2_fine": 8.0,
            "osc2_level": 0.5,
            "unison": 2,
            "detune": 14.0,
            "spread": 0.35,
            "cutoff": 1200.0,
            "resonance": 0.25,
            "filter_env_amount": 0.55,
            "filter_drive": 0.35,
            "fe_attack": 5.0,
            "fe_decay": 400.0,
            "fe_sustain": 0.4,
            "fe_release": 200.0,
            "ae_attack": 8.0,
            "ae_decay": 400.0,
            "ae_sustain": 0.8,
            "ae_release": 250.0,
            "poly_mode": "Legato",
            "glide": 20.0,
            "character": "Grit",
            "tone": 0.1,
            "reverb_mix": 0.15,
            "volume": 0.63
        }"#,
    },
    Preset {
        name: "JUPITER STRINGS",
        json: r#"{
            "osc1_wave": "Saw",
            "osc2_wave": "Saw",
            "osc2_fine": -6.0,
            "osc2_level": 0.5,
            "unison": 3,
            "detune": 18.0,
            "spread": 0.85,
            "cutoff": 2600.0,
            "resonance": 0.1,
            "filter_env_amount": 0.15,
            "fe_attack": 300.0,
            "fe_sustain": 0.6,
            "fe_release": 800.0,
            "ae_attack": 400.0,
            "ae_decay": 1200.0,
            "ae_sustain": 0.9,
            "ae_release": 1500.0,
            "chorus_mix": 0.6,
            "chorus_depth": 0.7,
            "reverb_mix": 0.4,
            "reverb_decay": 0.75,
            "macro_space": 0.7,
            "drift": 0.2,
            "tone": -0.1,
            "volume": 0.63
        }"#,
    },
    Preset {
        name: "VANGELIS PAD",
        json: r#"{
            "osc1_wave": "Saw",
            "osc2_wave": "Square",
            "osc2_octave": -1,
            "osc2_level": 0.35,
            "unison": 4,
            "detune": 25.0,
            "spread": 0.9,
            "cutoff": 900.0,
            "resonance": 0.12,
            "filter_env_amount": 0.2,
            "fe_attack": 1200.0,
            "fe_decay": 2000.0,
            "fe_sustain": 0.5,
            "fe_release": 2500.0,
            "ae_attack": 1500.0,
            "ae_decay": 2000.0,
            "ae_sustain": 0.9,
            "ae_release": 3000.0,
            "chorus_mix": 0.5,
            "reverb_mix": 0.7,
            "reverb_decay": 0.85,
            "macro_space": 0.85,
            "drift": 0.25,
            "tone": -0.2,
            "volume": 0.63
        }"#,
    },
    Preset {
        name: "SILK STRINGS",
        json: r#"{
            "osc1_wave": "Triangle",
            "osc2_wave": "Pulse",
            "osc2_pw": 0.45,
            "osc2_semi": 5,
            "osc2_level": 0.4,
            "unison": 3,
            "detune": 14.0,
            "spread": 0.8,
            "cutoff": 2400.0,
            "resonance": 0.08,
            "filter_env_amount": 0.12,
            "fe_attack": 400.0,
            "fe_sustain": 0.7,
            "fe_release": 900.0,
            "ae_attack": 400.0,
            "ae_decay": 900.0,
            "ae_sustain": 0.9,
            "ae_release": 1200.0,
            "chorus_mix": 0.6,
            "reverb_mix": 0.45,
            "tone": -0.15,
            "character": "Silk",
            "volume": 0.63
        }"#,
    },
    Preset {
        name: "WIDE BRASS",
        json: r#"{
            "osc1_wave": "Saw",
            "osc2_wave": "Saw",
            "osc2_semi": 7,
            "osc2_level": 0.5,
            "unison": 4,
            "detune": 30.0,
            "spread": 0.85,
            "cutoff": 1200.0,
            "resonance": 0.25,
            "filter_env_amount": 0.35,
            "filter_drive": 0.25,
            "fe_attack": 30.0,
            "fe_decay": 600.0,
            "fe_sustain": 0.5,
            "fe_release": 300.0,
            "ae_attack": 60.0,
            "ae_decay": 400.0,
            "ae_sustain": 0.85,
            "ae_release": 400.0,
            "poly_mode": "Legato",
            "glide": 80.0,
            "chorus_mix": 0.4,
            "chorus_depth": 0.7,
            "tone": 0.15,
            "volume": 0.63
        }"#,
    },
    Preset {
        name: "MOOG BASS",
        json: r#"{
            "osc1_wave": "Saw",
            "osc2_wave": "Square",
            "osc2_octave": -1,
            "osc2_level": 0.45,
            "poly_mode": "Mono",
            "glide": 40.0,
            "cutoff": 500.0,
            "resonance": 0.45,
            "filter_env_amount": 0.5,
            "filter_drive": 0.4,
            "fe_attack": 3.0,
            "fe_decay": 250.0,
            "fe_sustain": 0.2,
            "fe_release": 200.0,
            "ae_attack": 3.0,
            "ae_decay": 400.0,
            "ae_sustain": 0.7,
            "ae_release": 200.0,
            "character": "Grit",
            "tone": 0.25,
            "detune": 8.0,
            "volume": 0.63
        }"#,
    },
    Preset {
        name: "ACID LINE",
        json: r#"{
            "osc1_wave": "Saw",
            "osc2_wave": "Saw",
            "osc2_level": 0.0,
            "poly_mode": "Legato",
            "glide": 120.0,
            "cutoff": 300.0,
            "resonance": 0.75,
            "filter_env_amount": 0.75,
            "filter_drive": 0.3,
            "fe_attack": 2.0,
            "fe_decay": 180.0,
            "fe_sustain": 0.1,
            "fe_release": 150.0,
            "ae_attack": 2.0,
            "ae_decay": 500.0,
            "ae_sustain": 0.7,
            "ae_release": 200.0,
            "character": "Grit",
            "tone": 0.2,
            "volume": 0.63
        }"#,
    },
    Preset {
        name: "GRIT BASS",
        json: r#"{
            "osc1_wave": "Saw",
            "osc2_wave": "Square",
            "osc2_octave": -1,
            "cutoff": 420.0,
            "resonance": 0.55,
            "filter_drive": 0.4,
            "filter_env_amount": 0.6,
            "fe_attack": 2.0,
            "fe_decay": 180.0,
            "fe_sustain": 0.15,
            "fe_release": 200.0,
            "ae_attack": 2.0,
            "ae_decay": 300.0,
            "ae_sustain": 0.6,
            "ae_release": 200.0,
            "poly_mode": "Mono",
            "unison": 3,
            "detune": 25.0,
            "character": "Grit",
            "tone": 0.3,
            "volume": 0.63
        }"#,
    },
    Preset {
        name: "PLUCK",
        json: r#"{
            "osc1_wave": "Square",
            "osc2_wave": "Saw",
            "osc2_octave": 1,
            "osc2_level": 0.35,
            "cutoff": 1800.0,
            "resonance": 0.35,
            "filter_env_amount": 0.7,
            "fe_attack": 1.0,
            "fe_decay": 220.0,
            "fe_sustain": 0.0,
            "fe_release": 250.0,
            "ae_attack": 1.0,
            "ae_decay": 260.0,
            "ae_sustain": 0.0,
            "ae_release": 300.0,
            "delay_mix": 0.3,
            "delay_feedback": 0.35,
            "reverb_mix": 0.15,
            "macro_bright": 0.55,
            "volume": 0.63
        }"#,
    },
    Preset {
        name: "MOTION PAD",
        json: r#"{
            "osc1_wave": "Saw",
            "osc2_wave": "Square",
            "osc2_level": 0.35,
            "cutoff": 1500.0,
            "resonance": 0.3,
            "filter_env_amount": 0.35,
            "filter_mode": "Bandpass",
            "fe_decay": 250.0,
            "fe_sustain": 0.2,
            "ae_attack": 300.0,
            "ae_release": 900.0,
            "lfo1_wave": "S&H",
            "lfo1_rate": 0.8,
            "lfo1_amount": 0.15,
            "lfo1_target": "Pitch",
            "lfo2_wave": "Triangle",
            "lfo2_rate": 0.25,
            "lfo2_amount": 0.2,
            "lfo2_target": "Filter",
            "chorus_mix": 0.45,
            "reverb_mix": 0.4,
            "drift": 0.3,
            "macro_motion": 0.7,
            "volume": 0.63
        }"#,
    },
    Preset {
        name: "ARP TOY",
        json: r#"{
            "osc1_wave": "Pulse",
            "osc1_pw": 0.35,
            "osc2_wave": "Square",
            "osc2_octave": 1,
            "osc2_level": 0.35,
            "cutoff": 2200.0,
            "resonance": 0.4,
            "filter_env_amount": 0.5,
            "fe_decay": 300.0,
            "fe_sustain": 0.25,
            "ae_attack": 2.0,
            "ae_decay": 400.0,
            "ae_sustain": 0.5,
            "arp_on": true,
            "arp_pattern": "UpDown",
            "arp_rate": "1/16",
            "arp_octaves": 2,
            "arp_gate": 45.0,
            "arp_length": 6,
            "arp_ratchet": 2,
            "arp_swing": 15.0,
            "lfo1_wave": "Triangle",
            "lfo1_rate": 3.0,
            "lfo1_amount": 0.1,
            "lfo1_target": "Pitch",
            "delay_mix": 0.35,
            "reverb_mix": 0.3,
            "drift": 0.25,
            "macro_motion": 0.6,
            "volume": 0.63
        }"#,
    },
];

/// Writes the preset with the given index into the host.
///
/// The defaults of the synthesizer are written first, so that a preset always
/// sounds the same, no matter what was played before it. An index that does
/// not exist is ignored.
pub fn apply(params: &SushiParams, setter: &ParamSetter, index: usize) {
    let Some(preset) = PRESETS.get(index) else {
        return;
    };

    let document: Value = match serde_json::from_str(preset.json) {
        Ok(document) => document,
        Err(error) => {
            warn!("the preset {} is broken: {error}", preset.name);
            return;
        }
    };

    if let Err(error) = patch::apply_json(params, setter, &document) {
        warn!("the preset {} could not be applied: {error}", preset.name);
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
    let arp_swing = jitter(rng, params.arp_swing.value(), 0.0, 75.0, 8.0);
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
    setter.set_parameter(&params.arp_swing, arp_swing);
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
    use crate::params::{ArpPattern, NoteDiv, Waveform};
    use crate::patch::test_support::setter;

    #[test]
    fn there_are_fourteen_presets_with_short_names() {
        assert_eq!(PRESETS.len(), 14);
        for preset in PRESETS {
            assert!(!preset.name.is_empty());
            assert!(preset.name.len() <= 18, "{}", preset.name);
            assert!(preset.name.is_ascii(), "{}", preset.name);
        }
    }

    #[test]
    fn every_preset_is_a_valid_patch() {
        for preset in PRESETS {
            let document: Value = serde_json::from_str(preset.json)
                .unwrap_or_else(|error| panic!("the preset {} is broken: {error}", preset.name));
            assert!(
                document.is_object(),
                "the preset {} is not an object",
                preset.name
            );
        }
    }

    #[test]
    fn every_preset_can_be_applied() {
        let params = SushiParams::default();
        let setter = setter();

        for (index, preset) in PRESETS.iter().enumerate() {
            apply(&params, &setter, index);
            assert!(
                params.cutoff.value() >= 20.0 && params.cutoff.value() <= 20_000.0,
                "the cutoff of {} is out of range",
                preset.name
            );
            assert!(
                params.unison.value() >= 1 && params.unison.value() <= 4,
                "the unison of {} is out of range",
                preset.name
            );
        }
    }

    #[test]
    fn the_presets_use_the_default_tune_and_voices() {
        let params = SushiParams::default();
        let setter = setter();

        // Move the two parameters away from their default.
        setter.set_parameter(&params.tune, 5.0);
        setter.set_parameter(&params.voices, 2);

        for (index, preset) in PRESETS.iter().enumerate() {
            apply(&params, &setter, index);

            // The factory defaults are written first, so every preset starts
            // at the same pitch and with the same number of voices.
            let tune = params.tune.value();
            assert!(tune.abs() < 0.001, "the tune of {} is {tune}", preset.name);
            assert_eq!(
                params.voices.value(),
                16,
                "the number of voices of {} is wrong",
                preset.name
            );
        }
    }

    #[test]
    fn an_index_that_does_not_exist_is_ignored() {
        let params = SushiParams::default();
        let setter = setter();

        apply(&params, &setter, PRESETS.len());
        assert_eq!(params.cutoff.value(), 3_000.0);
    }

    #[test]
    fn the_presets_change_the_sound() {
        let params = SushiParams::default();
        let setter = setter();

        let arp_toy = PRESETS
            .iter()
            .position(|preset| preset.name == "ARP TOY")
            .expect("the preset is missing");
        apply(&params, &setter, arp_toy);
        assert!(params.arp_on.value());
        assert_eq!(params.arp_pattern.value(), ArpPattern::UpDown);
        assert_eq!(params.arp_rate.value(), NoteDiv::Sixteenth);
        assert_eq!(params.arp_octaves.value(), 2);

        let init = PRESETS
            .iter()
            .position(|preset| preset.name == "INIT PAD")
            .expect("the preset is missing");
        apply(&params, &setter, init);
        assert!(!params.arp_on.value());
        assert_eq!(params.osc1_wave.value(), Waveform::Saw);
        assert!((params.cutoff.value() - 900.0).abs() < 1.0);
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
