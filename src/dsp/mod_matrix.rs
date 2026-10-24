//! The modulation matrix: eight slots that connect the sources of the
//! synthesizer to its destinations.
//!
//! Every slot holds a source, a destination and an amount. The amount is
//! bipolar, so that the modulation can be turned down as well as up. All the
//! slots that share a destination are added up, so that a single destination
//! can be modulated by several sources at the same time.

use crate::params::{MOD_SLOTS, ModDest, ModSlot, ModSource};

/// The range of the pitch modulation at full amount, in semitones.
const PITCH_RANGE: f32 = 12.0;

/// The range of the pulse width modulation at full amount.
const PULSE_WIDTH_RANGE: f32 = 0.45;

/// The range of the cutoff modulation at full amount, in octaves.
const CUTOFF_RANGE: f32 = 5.0;

/// The range of the resonance modulation at full amount.
const RESONANCE_RANGE: f32 = 1.0;

/// The range of the drive modulation at full amount.
const DRIVE_RANGE: f32 = 1.0;

/// The range of the pan modulation at full amount.
const PAN_RANGE: f32 = 1.0;

/// The range of the volume modulation at full amount. The gain of the voice
/// is `1 + modulation`, so that a modulation of zero leaves the voice alone.
const VOLUME_RANGE: f32 = 1.0;

/// The range of the coarse pitch modulation of the second oscillator at full
/// amount, in semitones.
const OSC2_PITCH_RANGE: f32 = 12.0;

/// The range of the fine pitch modulation of the second oscillator at full
/// amount, in cents.
const OSC2_FINE_RANGE: f32 = 100.0;

/// The range of the sub oscillator level modulation at full amount.
const SUB_LEVEL_RANGE: f32 = 1.0;

/// The range of the noise level modulation at full amount.
const NOISE_LEVEL_RANGE: f32 = 0.5;

/// The range of the unison detune modulation at full amount, in cents.
const DETUNE_RANGE: f32 = 50.0;

/// The range of the filter envelope amount modulation at full amount, in
/// octaves.
const FILTER_ENV_RANGE: f32 = 5.0;

/// The range of the coarse pitch modulation of the first oscillator at full
/// amount, in semitones.
const OSC1_PITCH_RANGE: f32 = 12.0;

/// The range of the fine pitch modulation of the first oscillator at full
/// amount, in cents.
const OSC1_FINE_RANGE: f32 = 100.0;

/// The range of the additional filter drive modulation at full amount.
const FILTER_DRIVE_RANGE: f32 = 1.0;

/// The range of the filter keytrack modulation at full amount.
const FILTER_KEYTRACK_RANGE: f32 = 1.0;

/// The range of the ring modulator level modulation at full amount.
const RING_MOD_RANGE: f32 = 1.0;

/// The number of semitones around the middle C that the key source covers.
/// Two octaves up and down fill the whole range of the source.
const KEY_RANGE: f32 = 24.0;

/// The values of the sources of the matrix, as they are at the moment.
#[derive(Debug, Clone, Copy, Default)]
pub struct ModSources {
    /// The value of the first LFO, `-1..1`.
    pub lfo1: f32,
    /// The value of the second LFO, `-1..1`.
    pub lfo2: f32,
    /// The level of the filter envelope, `0..1`.
    pub filter_env: f32,
    /// The level of the amplitude envelope, `0..1`.
    pub amp_env: f32,
    /// The velocity of the note, `0..1`.
    pub velocity: f32,
    /// The key of the note, `-1..1`, with the middle C in the middle.
    pub key: f32,
    /// The value of the first macro, `0..1`.
    pub macro1: f32,
    /// The value of the second macro, `0..1`.
    pub macro2: f32,
    /// The value of the third macro, `0..1`.
    pub macro3: f32,
    /// The smoothed position of the mod wheel, `0..1`.
    pub mod_wheel: f32,
    /// A deterministic per-voice value, `-1..1`. It stays the same for the
    /// life of a note.
    pub random: f32,
}

/// The modulation of the destinations of the matrix.
#[derive(Debug, Clone, Copy)]
pub struct Modulation {
    /// The offset of the pitch, in semitones.
    pub pitch_semitones: f32,
    /// The offset of the cutoff, in octaves.
    pub filter_octaves: f32,
    /// The offset of the pulse width of the first oscillator.
    pub pulse_width1: f32,
    /// The offset of the pulse width of the second oscillator.
    pub pulse_width2: f32,
    /// The offset of the resonance of the filter.
    pub resonance: f32,
    /// The offset of the drive of the filter.
    pub drive: f32,
    /// The pan of the voice, `-1..1`.
    pub pan: f32,
    /// The gain of the voice.
    pub volume: f32,
    /// The offset of the coarse pitch of the second oscillator, in semitones.
    pub osc2_pitch_semitones: f32,
    /// The offset of the fine pitch of the second oscillator, in semitones.
    pub osc2_fine_semitones: f32,
    /// The additive offset of the sub oscillator level.
    pub sub_level: f32,
    /// The additive offset of the noise level.
    pub noise_level: f32,
    /// The additive offset of the unison detune, in cents.
    pub detune_cents: f32,
    /// The additive offset of the filter envelope amount, in octaves.
    pub filter_env_octaves: f32,
    /// The offset of the coarse pitch of the first oscillator, in semitones.
    pub osc1_pitch_semitones: f32,
    /// The offset of the fine pitch of the first oscillator, in semitones.
    pub osc1_fine_semitones: f32,
    /// The additive offset of the filter drive.
    pub filter_drive: f32,
    /// The additive offset of the filter keytrack amount.
    pub filter_keytrack: f32,
    /// The additive offset of the ring modulator level.
    pub ring_mod: f32,
}

impl Default for Modulation {
    fn default() -> Self {
        Self {
            pitch_semitones: 0.0,
            filter_octaves: 0.0,
            pulse_width1: 0.0,
            pulse_width2: 0.0,
            resonance: 0.0,
            drive: 0.0,
            pan: 0.0,
            // Without a modulation the voice keeps its level.
            volume: 1.0,
            osc2_pitch_semitones: 0.0,
            osc2_fine_semitones: 0.0,
            sub_level: 0.0,
            noise_level: 0.0,
            detune_cents: 0.0,
            filter_env_octaves: 0.0,
            osc1_pitch_semitones: 0.0,
            osc1_fine_semitones: 0.0,
            filter_drive: 0.0,
            filter_keytrack: 0.0,
            ring_mod: 0.0,
        }
    }
}

/// The key of a note as a source of the matrix.
///
/// The middle C is in the middle, two octaves up are at the top and two
/// octaves down are at the bottom.
#[inline]
pub fn key_source(key: f32) -> f32 {
    ((key - 60.0) / KEY_RANGE).clamp(-1.0, 1.0)
}

/// The value of a single source.
#[inline]
fn source_value(source: ModSource, sources: &ModSources) -> f32 {
    match source {
        ModSource::Off => 0.0,
        ModSource::Lfo1 => sources.lfo1,
        ModSource::Lfo2 => sources.lfo2,
        ModSource::ModEnv => sources.filter_env,
        ModSource::AmpEnv => sources.amp_env,
        ModSource::Velocity => sources.velocity,
        ModSource::Key => sources.key,
        ModSource::Macro1 => sources.macro1,
        ModSource::Macro2 => sources.macro2,
        ModSource::Macro3 => sources.macro3,
        ModSource::ModWheel => sources.mod_wheel,
        ModSource::Random => sources.random,
    }
}

/// Adds up the modulation of all the slots of the matrix.
///
/// The result is neutral when every slot is switched off, so that a synth
/// without a matrix sounds exactly like it always did.
#[inline]
pub fn evaluate(slots: &[ModSlot; MOD_SLOTS], sources: &ModSources) -> Modulation {
    let mut modulation = Modulation::default();
    // The volume is a gain around 1.0, so the offsets of the slots are
    // collected on their own.
    let mut volume_offset = 0.0;

    for slot in slots {
        if slot.source == ModSource::Off || slot.dest == ModDest::Off || slot.amount == 0.0 {
            continue;
        }

        let value = source_value(slot.source, sources) * slot.amount;
        match slot.dest {
            ModDest::Off => {}
            ModDest::Pitch => modulation.pitch_semitones += value * PITCH_RANGE,
            ModDest::Pulse1 => modulation.pulse_width1 += value * PULSE_WIDTH_RANGE,
            ModDest::Pulse2 => modulation.pulse_width2 += value * PULSE_WIDTH_RANGE,
            ModDest::Cutoff => modulation.filter_octaves += value * CUTOFF_RANGE,
            ModDest::Resonance => modulation.resonance += value * RESONANCE_RANGE,
            ModDest::Drive => modulation.drive += value * DRIVE_RANGE,
            ModDest::Pan => modulation.pan += value * PAN_RANGE,
            ModDest::Volume => volume_offset += value * VOLUME_RANGE,
            ModDest::Osc2Pitch => modulation.osc2_pitch_semitones += value * OSC2_PITCH_RANGE,
            ModDest::Osc2Fine => modulation.osc2_fine_semitones += value * OSC2_FINE_RANGE / 100.0,
            ModDest::SubLevel => modulation.sub_level += value * SUB_LEVEL_RANGE,
            ModDest::NoiseLevel => modulation.noise_level += value * NOISE_LEVEL_RANGE,
            ModDest::Detune => modulation.detune_cents += value * DETUNE_RANGE,
            ModDest::FilterEnv => modulation.filter_env_octaves += value * FILTER_ENV_RANGE,
            ModDest::Osc1Pitch => modulation.osc1_pitch_semitones += value * OSC1_PITCH_RANGE,
            ModDest::Osc1Fine => modulation.osc1_fine_semitones += value * OSC1_FINE_RANGE / 100.0,
            ModDest::FilterDrive => modulation.filter_drive += value * FILTER_DRIVE_RANGE,
            ModDest::FilterKeytrack => modulation.filter_keytrack += value * FILTER_KEYTRACK_RANGE,
            ModDest::RingMod => modulation.ring_mod += value * RING_MOD_RANGE,
        }
    }

    // The pan of the voice cannot leave its range and the gain cannot become
    // negative.
    modulation.pan = modulation.pan.clamp(-1.0, 1.0);
    modulation.volume = (1.0 + volume_offset).max(0.0);

    modulation
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A matrix with a single slot.
    fn matrix(slot: ModSlot) -> [ModSlot; MOD_SLOTS] {
        let mut slots = [ModSlot::default(); MOD_SLOTS];
        slots[0] = slot;
        slots
    }

    #[test]
    fn an_empty_matrix_is_neutral() {
        let slots = [ModSlot::default(); MOD_SLOTS];
        let sources = ModSources {
            lfo1: 1.0,
            lfo2: -1.0,
            filter_env: 1.0,
            amp_env: 1.0,
            velocity: 1.0,
            key: 1.0,
            macro1: 1.0,
            macro2: 1.0,
            macro3: 1.0,
            mod_wheel: 1.0,
            random: 1.0,
        };

        let modulation = evaluate(&slots, &sources);
        assert_eq!(modulation.pitch_semitones, 0.0);
        assert_eq!(modulation.filter_octaves, 0.0);
        assert_eq!(modulation.pulse_width1, 0.0);
        assert_eq!(modulation.pulse_width2, 0.0);
        assert_eq!(modulation.resonance, 0.0);
        assert_eq!(modulation.drive, 0.0);
        assert_eq!(modulation.pan, 0.0);
        assert_eq!(modulation.volume, 1.0);
        assert_eq!(modulation.osc2_pitch_semitones, 0.0);
        assert_eq!(modulation.osc2_fine_semitones, 0.0);
        assert_eq!(modulation.sub_level, 0.0);
        assert_eq!(modulation.noise_level, 0.0);
        assert_eq!(modulation.detune_cents, 0.0);
        assert_eq!(modulation.filter_env_octaves, 0.0);
        assert_eq!(modulation.osc1_pitch_semitones, 0.0);
        assert_eq!(modulation.osc1_fine_semitones, 0.0);
        assert_eq!(modulation.filter_drive, 0.0);
        assert_eq!(modulation.filter_keytrack, 0.0);
        assert_eq!(modulation.ring_mod, 0.0);
    }

    #[test]
    fn the_slots_of_a_destination_are_added_up() {
        let mut slots = [ModSlot::default(); MOD_SLOTS];
        slots[0] = ModSlot {
            source: ModSource::Macro1,
            dest: ModDest::Pitch,
            amount: 0.5,
        };
        slots[1] = ModSlot {
            source: ModSource::Macro2,
            dest: ModDest::Pitch,
            amount: 0.25,
        };

        let sources = ModSources {
            macro1: 1.0,
            macro2: 1.0,
            ..ModSources::default()
        };

        let modulation = evaluate(&slots, &sources);
        assert!((modulation.pitch_semitones - 9.0).abs() < 1e-4);
    }

    #[test]
    fn the_pan_stays_in_its_range() {
        // Eight slots that all pan to the right must not leave the range.
        let mut slots = [ModSlot::default(); MOD_SLOTS];
        for slot in slots.iter_mut() {
            *slot = ModSlot {
                source: ModSource::Macro1,
                dest: ModDest::Pan,
                amount: 1.0,
            };
        }

        let sources = ModSources {
            macro1: 1.0,
            ..ModSources::default()
        };

        let modulation = evaluate(&slots, &sources);
        assert_eq!(modulation.pan, 1.0);
    }

    #[test]
    fn the_key_is_normalized_around_the_middle_c() {
        assert_eq!(key_source(60.0), 0.0);
        assert!((key_source(72.0) - 0.5).abs() < 1e-4);
        assert!((key_source(48.0) + 0.5).abs() < 1e-4);
        assert_eq!(key_source(0.0), -1.0);
        assert_eq!(key_source(127.0), 1.0);
    }

    #[test]
    fn the_new_sources_read_their_own_values() {
        let sources = ModSources {
            mod_wheel: 0.75,
            random: -0.5,
            ..ModSources::default()
        };

        assert_eq!(source_value(ModSource::ModWheel, &sources), 0.75);
        assert_eq!(source_value(ModSource::Random, &sources), -0.5);
    }

    #[test]
    fn the_new_destinations_reach_their_full_ranges() {
        let destinations = [
            (ModDest::Pitch, 12.0),
            (ModDest::Cutoff, 5.0),
            (ModDest::Volume, 2.0),
            (ModDest::Osc2Pitch, 12.0),
            (ModDest::Osc2Fine, 1.0),
            (ModDest::SubLevel, 1.0),
            (ModDest::NoiseLevel, 0.5),
            (ModDest::Detune, 50.0),
            (ModDest::FilterEnv, 5.0),
            (ModDest::Osc1Pitch, 12.0),
            (ModDest::Osc1Fine, 1.0),
            (ModDest::FilterDrive, 1.0),
            (ModDest::FilterKeytrack, 1.0),
            (ModDest::RingMod, 1.0),
        ];

        for (dest, expected) in destinations {
            let slots = matrix(ModSlot {
                source: ModSource::Macro1,
                dest,
                amount: 1.0,
            });
            let sources = ModSources {
                macro1: 1.0,
                ..ModSources::default()
            };
            let modulation = evaluate(&slots, &sources);
            let value = match dest {
                ModDest::Pitch => modulation.pitch_semitones,
                ModDest::Cutoff => modulation.filter_octaves,
                ModDest::Volume => modulation.volume,
                ModDest::Osc2Pitch => modulation.osc2_pitch_semitones,
                ModDest::Osc2Fine => modulation.osc2_fine_semitones,
                ModDest::SubLevel => modulation.sub_level,
                ModDest::NoiseLevel => modulation.noise_level,
                ModDest::Detune => modulation.detune_cents,
                ModDest::FilterEnv => modulation.filter_env_octaves,
                ModDest::Osc1Pitch => modulation.osc1_pitch_semitones,
                ModDest::Osc1Fine => modulation.osc1_fine_semitones,
                ModDest::FilterDrive => modulation.filter_drive,
                ModDest::FilterKeytrack => modulation.filter_keytrack,
                ModDest::RingMod => modulation.ring_mod,
                _ => unreachable!(),
            };
            assert!(
                (value - expected).abs() < 1e-4,
                "{dest:?} reached {value} instead of {expected}"
            );
        }
    }

    #[test]
    fn the_new_modulations_are_bipolar() {
        let slots = matrix(ModSlot {
            source: ModSource::Macro1,
            dest: ModDest::Osc2Pitch,
            amount: -0.5,
        });
        let sources = ModSources {
            macro1: 0.5,
            ..ModSources::default()
        };

        let modulation = evaluate(&slots, &sources);
        assert!((modulation.osc2_pitch_semitones + 3.0).abs() < 1e-4);
    }

    #[test]
    fn an_off_source_does_not_leak_the_new_sources() {
        // The raw source values are not neutral, but an off slot must not
        // read them.
        let sources = ModSources {
            mod_wheel: 1.0,
            random: 1.0,
            ..ModSources::default()
        };
        let slots = matrix(ModSlot {
            source: ModSource::Off,
            dest: ModDest::Osc2Pitch,
            amount: 1.0,
        });

        let modulation = evaluate(&slots, &sources);
        assert_eq!(modulation.osc2_pitch_semitones, 0.0);
        assert_eq!(modulation.sub_level, 0.0);
        assert_eq!(modulation.filter_env_octaves, 0.0);
        assert_eq!(modulation.osc1_pitch_semitones, 0.0);
        assert_eq!(modulation.osc1_fine_semitones, 0.0);
        assert_eq!(modulation.filter_drive, 0.0);
        assert_eq!(modulation.filter_keytrack, 0.0);
        assert_eq!(modulation.ring_mod, 0.0);
    }
}
