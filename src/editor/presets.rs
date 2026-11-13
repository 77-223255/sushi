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
            "mod1_source": "LFO 1",
            "mod1_dest": "Cutoff",
            "mod1_amount": 0.12,
            "unison": 2,
            "spread": 0.6,
            "chorus_mix": 0.3,
            "reverb_mix": 0.25,
            "volume": 0.63,
            "poly_mode": "Poly"
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
            "poly_mode": "Poly",
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
            "poly_mode": "Poly",
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
            "poly_mode": "Poly",
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
        name: "MODEL D BASS",
        json: r#"{
            "macro_space": 0.0,
            "osc1_wave": "Saw",
            "osc1_level": 0.7,
            "osc2_wave": "Square",
            "osc2_octave": -1,
            "osc2_level": 0.5,
            "sub_level": 0.2,
            "poly_mode": "Poly",
            "glide": 25.0,
            "filter_mode": "Lowpass",
            "filter_slope": "24 dB",
            "cutoff": 420.0,
            "resonance": 0.45,
            "filter_env_amount": 0.55,
            "filter_drive": 0.35,
            "fe_attack": 2.0,
            "fe_decay": 220.0,
            "fe_sustain": 0.15,
            "fe_release": 180.0,
            "ae_attack": 2.0,
            "ae_decay": 350.0,
            "ae_sustain": 0.65,
            "ae_release": 180.0,
            "character": "Grit",
            "tone": 0.18,
            "volume": 0.62
        }"#,
    },
    Preset {
        name: "CS BRASS",
        json: r#"{
            "macro_space": 0.0,
            "osc1_wave": "Saw",
            "osc1_level": 0.65,
            "osc2_wave": "Square",
            "osc2_octave": -1,
            "osc2_level": 0.45,
            "unison": 2,
            "detune": 10.0,
            "spread": 0.5,
            "filter_mode": "Lowpass",
            "filter_slope": "24 dB",
            "cutoff": 1200.0,
            "resonance": 0.2,
            "filter_env_amount": 0.45,
            "filter_drive": 0.15,
            "fe_attack": 30.0,
            "fe_decay": 600.0,
            "fe_sustain": 0.5,
            "fe_release": 400.0,
            "ae_attack": 25.0,
            "ae_decay": 700.0,
            "ae_sustain": 0.85,
            "ae_release": 500.0,
            "chorus_mix": 0.25,
            "reverb_mix": 0.25,
            "character": "Silk",
            "volume": 0.6,
            "poly_mode": "Poly"
        }"#,
    },
    Preset {
        name: "MONO 101",
        json: r#"{
            "macro_space": 0.0,
            "osc1_wave": "Pulse",
            "osc1_pw": 0.35,
            "osc1_level": 0.75,
            "osc2_wave": "Square",
            "osc2_octave": -1,
            "osc2_level": 0.25,
            "sub_level": 0.35,
            "poly_mode": "Poly",
            "glide": 15.0,
            "filter_mode": "Lowpass",
            "filter_slope": "24 dB",
            "cutoff": 700.0,
            "resonance": 0.35,
            "filter_env_amount": 0.55,
            "filter_drive": 0.2,
            "fe_attack": 2.0,
            "fe_decay": 250.0,
            "fe_sustain": 0.2,
            "fe_release": 200.0,
            "ae_attack": 2.0,
            "ae_decay": 300.0,
            "ae_sustain": 0.7,
            "ae_release": 180.0,
            "delay_mix": 0.22,
            "reverb_mix": 0.12,
            "character": "Grit",
            "tone": 0.15,
            "volume": 0.62
        }"#,
    },
    Preset {
        name: "JUNO BASS",
        json: r#"{
            "macro_space": 0.0,
            "osc1_wave": "Saw",
            "osc1_level": 0.7,
            "osc2_wave": "Square",
            "osc2_octave": -1,
            "osc2_level": 0.4,
            "sub_level": 0.4,
            "poly_mode": "Poly",
            "glide": 10.0,
            "filter_mode": "Lowpass",
            "filter_slope": "24 dB",
            "cutoff": 550.0,
            "resonance": 0.3,
            "filter_env_amount": 0.5,
            "filter_drive": 0.2,
            "fe_attack": 2.0,
            "fe_decay": 250.0,
            "fe_sustain": 0.15,
            "fe_release": 180.0,
            "ae_attack": 2.0,
            "ae_decay": 400.0,
            "ae_sustain": 0.7,
            "ae_release": 200.0,
            "chorus_mix": 0.35,
            "reverb_mix": 0.08,
            "tone": 0.1,
            "volume": 0.62
        }"#,
    },
    Preset {
        name: "String",
        json: r#"{
            "ae_attack": 3.0,
            "ae_curve": 0.0,
            "ae_decay": 1200.0,
            "ae_release": 1580.5,
            "ae_sustain": 0.15,
            "arp_chord": "Off",
            "arp_gate": 60.0,
            "arp_latch": false,
            "arp_length": 16,
            "arp_octaves": 1,
            "arp_on": false,
            "arp_pattern": "As Played",
            "arp_probability": 1.0,
            "arp_ratchet": 1,
            "arp_rate": "1/16",
            "arp_strum": 0.0,
            "arp_swing": 0.0,
            "character": "Silk",
            "chorus_depth": 0.5,
            "chorus_mix": 0.0,
            "chorus_rate": 0.8,
            "chorus_width": 0.0,
            "comp_attack": 20.0,
            "comp_makeup": 0.0,
            "comp_mix": 0.0,
            "comp_ratio": 1.0,
            "comp_release": 200.0,
            "comp_threshold": 0.0,
            "cutoff": 494.52072,
            "delay_damping": 0.0,
            "delay_feedback": 0.3,
            "delay_mix": 0.0,
            "delay_time": 350.0,
            "detune": 0.0,
            "drift": 0.0,
            "drive_amount": 0.0,
            "drive_mix": 0.0,
            "drive_tone": 0.0,
            "eq_high": 0.0,
            "eq_low": 0.0,
            "eq_mid": 0.0,
            "fe_attack": 20.5,
            "fe_curve": 0.0,
            "fe_decay": 900.0,
            "fe_release": 900.0,
            "fe_sustain": 0.1,
            "filter_drive": 0.0,
            "filter_env_amount": 0.52870786,
            "filter_keytrack": 0.3,
            "filter_mode": "Lowpass",
            "filter_slope": "24 dB OLD",
            "fm_amount": 0.0,
            "fx_bypass_mask": 0,
            "fx_global_mix": 1.0,
            "fx_order": 0,
            "glide": 0.0,
            "hpf_cutoff": 20.0,
            "lfo1_amount": 0.0,
            "lfo1_curve": 0.0,
            "lfo1_div": "1/4",
            "lfo1_phase": 0.0,
            "lfo1_rate": 5.0000005,
            "lfo1_retrigger": false,
            "lfo1_sync": false,
            "lfo1_wave": "Sine",
            "lfo2_amount": 0.0,
            "lfo2_curve": 0.0,
            "lfo2_div": "1/4",
            "lfo2_phase": 0.0,
            "lfo2_rate": 0.50000006,
            "lfo2_retrigger": false,
            "lfo2_sync": false,
            "lfo2_wave": "Triangle",
            "macro_bright": 0.5407958,
            "macro_motion": 0.5,
            "macro_space": 0.5,
            "mod1_amount": 0.0,
            "mod1_dest": "Off",
            "mod1_source": "Off",
            "mod2_amount": 0.0,
            "mod2_dest": "Off",
            "mod2_source": "Off",
            "mod3_amount": 0.0,
            "mod3_dest": "Off",
            "mod3_source": "Off",
            "mod4_amount": 0.0,
            "mod4_dest": "Off",
            "mod4_source": "Off",
            "mod5_amount": 0.0,
            "mod5_dest": "Off",
            "mod5_source": "Off",
            "mod6_amount": 0.0,
            "mod6_dest": "Off",
            "mod6_source": "Off",
            "mod7_amount": 0.0,
            "mod7_dest": "Off",
            "mod7_source": "Off",
            "mod8_amount": 0.0,
            "mod8_dest": "Off",
            "mod8_source": "Off",
            "noise_level": 0.0,
            "osc1_fine": 0.0,
            "osc1_level": 0.6,
            "osc1_octave": 0,
            "osc1_pw": 0.4,
            "osc1_semi": 0,
            "osc1_wave": "Pulse",
            "osc2_fine": 0.0,
            "osc2_level": 0.35,
            "osc2_octave": 1,
            "osc2_pw": 0.5,
            "osc2_semi": 0,
            "osc2_wave": "Triangle",
            "osc_phase_reset": false,
            "osc_sync": false,
            "phaser_depth": 0.5,
            "phaser_feedback": 0.0,
            "phaser_mix": 0.0,
            "phaser_rate": 0.3,
            "poly_mode": "Poly",
            "resonance": 0.5827937,
            "reverb_decay": 0.6,
            "reverb_mix": 0.0,
            "reverb_tone": 0.0,
            "ring_mod": 0.0,
            "spread": 0.5565834,
            "sub_level": 0.0,
            "tone": 0.0,
            "trem_depth": 0.0,
            "trem_rate": 5.0,
            "trem_stereo": false,
            "tune": 0.0,
            "unison": 1,
            "vca_pan": 0.0,
            "vca_width": 1.0,
            "velocity": 0.7,
            "voices": 16,
            "volume": 0.63
        }"#,
    },
    Preset {
        name: "Piano",
        json: r#"{
            "ae_attack": 1.5,
            "ae_curve": 0.0,
            "ae_decay": 2116.7,
            "ae_release": 154.3,
            "ae_sustain": 0.14457083,
            "arp_chord": "Off",
            "arp_gate": 60.0,
            "arp_latch": false,
            "arp_length": 16,
            "arp_octaves": 1,
            "arp_on": false,
            "arp_pattern": "As Played",
            "arp_probability": 1.0,
            "arp_ratchet": 1,
            "arp_rate": "1/16",
            "arp_strum": 0.0,
            "arp_swing": 1.3963003,
            "character": "Silk",
            "chorus_depth": 0.5,
            "chorus_mix": 0.0,
            "chorus_rate": 1.5066168,
            "chorus_width": 0.0,
            "comp_attack": 20.0,
            "comp_makeup": 0.0,
            "comp_mix": 0.0,
            "comp_ratio": 1.0,
            "comp_release": 200.0,
            "comp_threshold": 0.0,
            "cutoff": 2223.4465,
            "delay_damping": 0.0,
            "delay_feedback": 0.3,
            "delay_mix": 0.0,
            "delay_time": 350.0,
            "detune": 0.0,
            "drift": 0.0,
            "drive_amount": 0.0,
            "drive_mix": 0.0,
            "drive_tone": 0.0,
            "eq_high": 1.8559418,
            "eq_low": -0.19199371,
            "eq_mid": 3.1094084,
            "fe_attack": 2.0,
            "fe_curve": 0.0,
            "fe_decay": 900.0,
            "fe_release": 900.0,
            "fe_sustain": 0.1,
            "filter_drive": 0.0,
            "filter_env_amount": 0.14463949,
            "filter_keytrack": 0.3,
            "filter_mode": "Lowpass",
            "filter_slope": "24 dB OLD",
            "fm_amount": 0.592199,
            "fx_bypass_mask": 0,
            "fx_global_mix": 1.0,
            "fx_order": 0,
            "glide": 0.0,
            "hpf_cutoff": 20.0,
            "lfo1_amount": 0.006612091,
            "lfo1_curve": 0.0,
            "lfo1_div": "1/4",
            "lfo1_phase": 0.0,
            "lfo1_rate": 6.1199083,
            "lfo1_retrigger": false,
            "lfo1_sync": false,
            "lfo1_wave": "Sine",
            "lfo2_amount": 0.0,
            "lfo2_curve": 0.0,
            "lfo2_div": "1/4",
            "lfo2_phase": 0.0,
            "lfo2_rate": 0.62386817,
            "lfo2_retrigger": false,
            "lfo2_sync": false,
            "lfo2_wave": "Triangle",
            "macro_bright": 0.5,
            "macro_motion": 0.43238991,
            "macro_space": 0.29160964,
            "mod1_amount": 0.0,
            "mod1_dest": "Off",
            "mod1_source": "Off",
            "mod2_amount": 0.0,
            "mod2_dest": "Off",
            "mod2_source": "Off",
            "mod3_amount": 0.0,
            "mod3_dest": "Off",
            "mod3_source": "Off",
            "mod4_amount": 0.0,
            "mod4_dest": "Off",
            "mod4_source": "Off",
            "mod5_amount": 0.0,
            "mod5_dest": "Off",
            "mod5_source": "Off",
            "mod6_amount": 0.0,
            "mod6_dest": "Off",
            "mod6_source": "Off",
            "mod7_amount": 0.0,
            "mod7_dest": "Off",
            "mod7_source": "Off",
            "mod8_amount": 0.0,
            "mod8_dest": "Off",
            "mod8_source": "Off",
            "noise_level": 0.0,
            "osc1_fine": 0.0,
            "osc1_level": 0.6,
            "osc1_octave": 0,
            "osc1_pw": 0.38305104,
            "osc1_semi": 0,
            "osc1_wave": "Pulse",
            "osc2_fine": 0.0,
            "osc2_level": 0.35,
            "osc2_octave": 0,
            "osc2_pw": 0.42005682,
            "osc2_semi": 0,
            "osc2_wave": "Triangle",
            "osc_phase_reset": false,
            "osc_sync": false,
            "phaser_depth": 0.5,
            "phaser_feedback": 0.0,
            "phaser_mix": 0.0,
            "phaser_rate": 0.3,
            "poly_mode": "Poly",
            "resonance": 0.0,
            "reverb_decay": 0.6,
            "reverb_mix": 0.22944435,
            "reverb_tone": 0.0,
            "ring_mod": 0.0,
            "spread": 0.38380855,
            "sub_level": 0.07670682,
            "tone": 0.0,
            "trem_depth": 0.0,
            "trem_rate": 5.0,
            "trem_stereo": false,
            "tune": 0.0,
            "unison": 2,
            "vca_pan": 0.0,
            "vca_width": 1.0,
            "velocity": 0.7,
            "voices": 16,
            "volume": 0.63
        }"#,
    },
    Preset {
        name: "Bad",
        json: r#"{
            "ae_attack": 2.3,
            "ae_curve": 0.0,
            "ae_decay": 800.0,
            "ae_release": 856.8,
            "ae_sustain": 0.7,
            "arp_chord": "Off",
            "arp_gate": 60.0,
            "arp_latch": false,
            "arp_length": 16,
            "arp_octaves": 1,
            "arp_on": false,
            "arp_pattern": "Up",
            "arp_probability": 1.0,
            "arp_ratchet": 1,
            "arp_rate": "1/16",
            "arp_strum": 0.0,
            "arp_swing": 0.0,
            "character": "Silk",
            "chorus_depth": 0.5,
            "chorus_mix": 0.43275148,
            "chorus_rate": 0.8,
            "chorus_width": 0.0,
            "comp_attack": 12.8,
            "comp_makeup": 0.0,
            "comp_mix": 0.0,
            "comp_ratio": 12.055644,
            "comp_release": 393.2,
            "comp_threshold": -20.525352,
            "cutoff": 475.7448,
            "delay_damping": 0.0,
            "delay_feedback": 0.3,
            "delay_mix": 0.0,
            "delay_time": 350.0,
            "detune": 12.0,
            "drift": 0.70206654,
            "drive_amount": 0.0,
            "drive_mix": 0.0,
            "drive_tone": 0.074252844,
            "eq_high": 6.5223637,
            "eq_low": 0.0,
            "eq_mid": 4.849598,
            "fe_attack": 13.400001,
            "fe_curve": 0.0,
            "fe_decay": 400.0,
            "fe_release": 300.0,
            "fe_sustain": 0.3,
            "filter_drive": 0.0,
            "filter_env_amount": 0.37839186,
            "filter_keytrack": 0.60048866,
            "filter_mode": "Lowpass",
            "filter_slope": "24 dB OLD",
            "fm_amount": 0.52445394,
            "fx_bypass_mask": 0,
            "fx_global_mix": 1.0,
            "fx_order": 0,
            "glide": 0.0,
            "hpf_cutoff": 20.0,
            "lfo1_amount": 0.0,
            "lfo1_curve": 0.0,
            "lfo1_div": "1/4",
            "lfo1_phase": 0.0,
            "lfo1_rate": 5.0,
            "lfo1_retrigger": false,
            "lfo1_sync": false,
            "lfo1_wave": "Sine",
            "lfo2_amount": 0.0,
            "lfo2_curve": 0.0,
            "lfo2_div": "1/4",
            "lfo2_phase": 0.0,
            "lfo2_rate": 0.5,
            "lfo2_retrigger": false,
            "lfo2_sync": false,
            "lfo2_wave": "Triangle",
            "macro_bright": 0.5375224,
            "macro_motion": 0.5,
            "macro_space": 0.0,
            "mod1_amount": 0.0,
            "mod1_dest": "Off",
            "mod1_source": "Off",
            "mod2_amount": 0.0,
            "mod2_dest": "Off",
            "mod2_source": "Off",
            "mod3_amount": 0.0,
            "mod3_dest": "Off",
            "mod3_source": "Off",
            "mod4_amount": 0.0,
            "mod4_dest": "Off",
            "mod4_source": "Off",
            "mod5_amount": 0.0,
            "mod5_dest": "Off",
            "mod5_source": "Off",
            "mod6_amount": 0.0,
            "mod6_dest": "Off",
            "mod6_source": "Off",
            "mod7_amount": 0.0,
            "mod7_dest": "Off",
            "mod7_source": "Off",
            "mod8_amount": 0.0,
            "mod8_dest": "Off",
            "mod8_source": "Off",
            "noise_level": 0.0,
            "osc1_fine": 0.0,
            "osc1_level": 0.6,
            "osc1_octave": 0,
            "osc1_pw": 0.5,
            "osc1_semi": 0,
            "osc1_wave": "Saw",
            "osc2_fine": 0.0,
            "osc2_level": 0.4,
            "osc2_octave": 0,
            "osc2_pw": 0.5,
            "osc2_semi": 0,
            "osc2_wave": "Sine",
            "osc_phase_reset": false,
            "osc_sync": true,
            "phaser_depth": 0.49910903,
            "phaser_feedback": 0.6544921,
            "phaser_mix": 0.49910903,
            "phaser_rate": 0.3,
            "poly_mode": "Poly",
            "resonance": 0.771567,
            "reverb_decay": 0.6,
            "reverb_mix": 0.0,
            "reverb_tone": 0.0,
            "ring_mod": 0.0,
            "spread": 0.5,
            "sub_level": 0.2583321,
            "tone": 0.0,
            "trem_depth": 0.0,
            "trem_rate": 1.6332253,
            "trem_stereo": false,
            "tune": 0.0,
            "unison": 1,
            "vca_pan": 0.0,
            "vca_width": 1.0,
            "velocity": 0.5,
            "voices": 16,
            "volume": 0.5011872
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
    use crate::params::{FilterSlope, LfoTarget, ModDest, ModSource, PolyMode, Waveform};
    use crate::patch::test_support::setter;

    #[test]
    fn there_are_eleven_presets_with_short_names() {
        assert_eq!(PRESETS.len(), 11);
        for preset in PRESETS {
            assert!(!preset.name.is_empty());
            assert!(preset.name.len() <= 18, "{}", preset.name);
            assert!(preset.name.is_ascii(), "{}", preset.name);
        }

        // The three sounds that came out of the patch folder are appended,
        // so the indices of the older presets stay in the order of the list
        // that shipped before them.
        let names: Vec<&str> = PRESETS.iter().map(|preset| preset.name).collect();
        assert_eq!(
            names,
            [
                "INIT PAD",
                "WIDE BRASS",
                "MOOG BASS",
                "ACID LINE",
                "MODEL D BASS",
                "CS BRASS",
                "MONO 101",
                "JUNO BASS",
                "String",
                "Piano",
                "Bad",
            ]
        );
    }

    #[test]
    fn every_preset_is_polyphonic() {
        let params = SushiParams::default();
        let setter = setter();

        for (index, preset) in PRESETS.iter().enumerate() {
            apply(&params, &setter, index);
            assert_eq!(
                params.poly_mode.value(),
                PolyMode::Poly,
                "the preset {} is not polyphonic",
                preset.name
            );
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

        // A preset starts from the factory defaults, so these two controls
        // come back to their defaults even when the user moved them first.
        setter.set_parameter(&params.tune, 5.0);
        setter.set_parameter(&params.voices, 2);

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
            assert!(
                params.tune.value().abs() < 0.001,
                "the tune of {} is {}",
                preset.name,
                params.tune.value()
            );
            assert_eq!(
                params.voices.value(),
                16,
                "the number of voices of {} is wrong",
                preset.name
            );
        }
    }

    #[test]
    fn the_presets_keep_the_old_slope_value() {
        let params = SushiParams::default();
        let setter = setter();
        let mut checked = 0;

        // Both the "24 dB" and the "24 dB OLD" spelling of a patch are the
        // first 24 dB alias. No preset may silently move to the second one.
        for (index, preset) in PRESETS.iter().enumerate() {
            let document: Value = serde_json::from_str(preset.json)
                .unwrap_or_else(|error| panic!("the preset {} is broken: {error}", preset.name));
            let uses_old_four_pole = matches!(
                document.get("filter_slope").and_then(Value::as_str),
                Some("24 dB") | Some("24 dB OLD")
            );

            apply(&params, &setter, index);
            assert_ne!(
                params.filter_slope.value(),
                FilterSlope::FourPoleModern,
                "the preset {} uses the second 24 dB alias",
                preset.name
            );
            if uses_old_four_pole {
                checked += 1;
                assert_eq!(
                    params.filter_slope.value(),
                    FilterSlope::FourPole,
                    "the preset {} lost its original 24 dB alias",
                    preset.name
                );
            }
        }

        assert_eq!(checked, 7, "the number of four-pole presets changed");
    }

    #[test]
    fn the_direct_lfo_presets_use_matrix_routes() {
        let params = SushiParams::default();
        let setter = setter();

        // The old direct target fields are no longer used by any factory
        // preset. The amounts are the effective depths that the presets had
        // with their macro motion at load time.
        for preset in PRESETS {
            assert!(
                !preset.json.contains("\"lfo1_target\"")
                    && !preset.json.contains("\"lfo2_target\""),
                "the preset {} still uses a direct LFO target",
                preset.name
            );
        }

        let expected = [("INIT PAD", ModDest::Cutoff, None)];
        for (name, first_dest, second_dest) in expected {
            let index = PRESETS
                .iter()
                .position(|preset| preset.name == name)
                .unwrap_or_else(|| panic!("the preset {name} is missing"));
            apply(&params, &setter, index);

            assert_eq!(params.lfo1_target.value(), LfoTarget::Off);
            assert_eq!(params.lfo1_amount.value(), 0.0);
            assert_eq!(params.lfo2_target.value(), LfoTarget::Off);
            assert_eq!(params.lfo2_amount.value(), 0.0);

            assert_eq!(params.mod1_source.value(), ModSource::Lfo1);
            assert_eq!(params.mod1_dest.value(), first_dest);
            assert!(params.mod1_amount.value() != 0.0);

            match second_dest {
                Some(dest) => {
                    assert_eq!(params.mod2_source.value(), ModSource::Lfo2);
                    assert_eq!(params.mod2_dest.value(), dest);
                    assert!(params.mod2_amount.value() != 0.0);
                }
                None => {
                    assert_eq!(params.mod2_source.value(), ModSource::Off);
                    assert_eq!(params.mod2_dest.value(), ModDest::Off);
                }
            }
        }
    }

    #[test]
    fn the_presets_keep_their_space_mix() {
        let params = SushiParams::default();
        let setter = setter();
        let expected = [
            ("MODEL D BASS", 0.0),
            ("CS BRASS", 0.0),
            ("MONO 101", 0.0),
            ("JUNO BASS", 0.0),
            ("String", 0.5),
            ("Piano", 0.291_609_64),
            ("Bad", 0.0),
        ];

        for (name, space) in expected {
            let index = PRESETS
                .iter()
                .position(|preset| preset.name == name)
                .unwrap_or_else(|| panic!("there is no preset {name:?}"));
            apply(&params, &setter, index);
            assert!(
                (params.macro_space.value() - space).abs() < 1e-6,
                "the space macro of {name} is {}",
                params.macro_space.value()
            );
        }
    }

    #[test]
    fn the_presets_change_the_sound() {
        let params = SushiParams::default();
        let setter = setter();

        let bass = PRESETS
            .iter()
            .position(|preset| preset.name == "MOOG BASS")
            .expect("the preset is missing");
        apply(&params, &setter, bass);
        assert!((params.cutoff.value() - 500.0).abs() < 1.0);
        assert!(params.resonance.value() > 0.3);
        assert_eq!(params.poly_mode.value(), PolyMode::Poly);

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
}
