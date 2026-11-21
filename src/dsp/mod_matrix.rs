
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

#[inline]
pub fn key_source(key: f32) -> f32 {
    ((key - 60.0) / KEY_RANGE).clamp(-1.0, 1.0)
}

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
        ModSource::ModWheel => sources.mod_wheel,
        ModSource::Random => sources.random,
    }
}

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

