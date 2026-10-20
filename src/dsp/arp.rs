//! A host-tempo-synced arpeggiator.
//!
//! The arpeggiator collects the notes that the host plays and turns them into
//! a repeating pattern. Its clock is driven by the host's tempo, so the
//! pattern always stays in time with the song.

use crate::params::ArpPattern;

/// The maximum number of notes that can be held at the same time. Notes beyond
/// this limit are ignored.
const MAX_HELD_NOTES: usize = 16;

/// The seed of the random number generator.
const RNG_SEED: u32 = 0x2545_F491;

/// A note that is held by the arpeggiator.
#[derive(Debug, Clone, Copy)]
struct HeldNote {
    /// The key of the note.
    key: u8,
    /// The velocity of the note.
    velocity: f32,
    /// Whether the note was added while the latch was on. Such a note stays
    /// held when the key is released.
    latched: bool,
}

impl Default for HeldNote {
    fn default() -> Self {
        Self {
            key: 60,
            velocity: 1.0,
            latched: false,
        }
    }
}

/// The result of a single arpeggiator tick.
#[derive(Debug, Clone, Copy, Default)]
pub struct ArpTick {
    /// The note that should be started, with its key and velocity.
    pub trigger: Option<(u8, f32)>,
    /// Whether the note that is currently sounding should be released.
    pub release: bool,
}

/// A simple arpeggiator with an up, down, up-down and random pattern.
pub struct Arp {
    /// The held notes, sorted by pitch. Only the first `held_len` entries are
    /// used.
    held: [Option<HeldNote>; MAX_HELD_NOTES],
    /// The number of held notes.
    held_len: usize,
    /// The number of keys that are physically pressed down.
    physical_held: usize,
    /// The position of the clock, in beats.
    clock_beats: f64,
    /// The number of samples left before the gate closes.
    gate_samples: u32,
    /// The index of the note that is played next.
    step: usize,
    /// The direction of the up-down pattern.
    direction: i32,
    /// The number of notes that have been played so far.
    played: usize,
    /// The octave that the next note is played in.
    octave: u8,
    /// The note that is currently sounding.
    current: Option<(u8, f32)>,
    /// The pattern that was used in the last tick. The state of the pattern is
    /// reset whenever the pattern changes.
    pattern: Option<ArpPattern>,
    /// The random number generator.
    rng: u32,
}

impl Default for Arp {
    fn default() -> Self {
        Self::new()
    }
}

impl Arp {
    pub fn new() -> Self {
        Self {
            held: [None; MAX_HELD_NOTES],
            held_len: 0,
            physical_held: 0,
            clock_beats: 0.0,
            gate_samples: 0,
            step: 0,
            direction: 1,
            played: 0,
            octave: 0,
            current: None,
            pattern: None,
            rng: RNG_SEED,
        }
    }

    /// Forgets everything: the held notes, the clock and the sounding note.
    pub fn reset(&mut self) {
        self.held = [None; MAX_HELD_NOTES];
        self.held_len = 0;
        self.physical_held = 0;
        self.clock_beats = 0.0;
        self.gate_samples = 0;
        self.step = 0;
        self.direction = 1;
        self.played = 0;
        self.octave = 0;
        self.current = None;
        self.pattern = None;
        self.rng = RNG_SEED;
    }

    /// Adds a note to the held set. The notes are kept sorted by pitch, so
    /// that the patterns can simply walk through them.
    ///
    /// When `latch` is set, the note stays held after the key is released. A
    /// new note that is played while no key is held down starts a fresh
    /// chord.
    pub fn note_on(&mut self, key: u8, velocity: f32, latch: bool) {
        if latch && self.physical_held == 0 {
            // The latch was released in the meantime, so this is a new chord.
            self.held = [None; MAX_HELD_NOTES];
            self.held_len = 0;
        }
        self.physical_held += 1;

        let velocity = velocity.clamp(0.0, 1.0);

        // Find the position of the note in the sorted list.
        let mut index = 0;
        while index < self.held_len {
            let existing = self.held[index].map(|note| note.key).unwrap_or(0);
            if existing == key {
                // The note is already held, so only its velocity is updated.
                // The latch of the note stays the one it was added with.
                if let Some(note) = self.held[index].as_mut() {
                    note.velocity = velocity;
                }
                return;
            }
            if existing > key {
                break;
            }
            index += 1;
        }

        if index >= MAX_HELD_NOTES {
            // The note pool is full, so the note is ignored.
            return;
        }

        // Make room for the new note.
        for shift in (index + 1..self.held_len).rev() {
            self.held[shift] = self.held[shift - 1];
        }
        self.held[index] = Some(HeldNote {
            key,
            velocity,
            latched: latch,
        });
        self.held_len = (self.held_len + 1).min(MAX_HELD_NOTES);
    }

    /// Removes a note from the held set.
    ///
    /// A note that was added while the latch was on stays held until a new
    /// chord is played. The current setting of the latch does not matter: the
    /// note remembers how it was added.
    pub fn note_off(&mut self, key: u8) {
        self.physical_held = self.physical_held.saturating_sub(1);

        for index in 0..self.held_len {
            let latched = self.held[index].map(|note| note.latched).unwrap_or(false);
            if latched {
                // The note was latched, so it stays in the held set.
                continue;
            }

            if self.held[index].map(|note| note.key) == Some(key) {
                for shift in index..self.held_len.saturating_sub(1) {
                    self.held[shift] = self.held[shift + 1];
                }
                self.held_len -= 1;
                self.held[self.held_len] = None;
                break;
            }
        }
    }

    /// Whether a note is currently sounding.
    pub fn is_sounding(&self) -> bool {
        self.current.is_some()
    }

    /// Advances the arpeggiator by a single sample.
    ///
    /// `beats_per_sample` is the tempo of the host in beats per sample,
    /// `division_beats` the length of one step of the pattern and
    /// `gate_percent` the length of the note as a percentage of the step.
    pub fn tick(
        &mut self,
        beats_per_sample: f64,
        division_beats: f64,
        gate_percent: f32,
        pattern: ArpPattern,
        octaves: u8,
        sample_rate: f32,
    ) -> ArpTick {
        let mut result = ArpTick::default();

        let beats_per_sample = beats_per_sample.max(0.0);
        let division_beats = division_beats.max(1e-9);
        let sample_rate = sample_rate.max(1.0) as f64;

        if self.pattern != Some(pattern) {
            // The pattern changed, so the pattern starts from the beginning
            // again.
            self.pattern = Some(pattern);
            self.step = 0;
            self.direction = 1;
            self.played = 0;
        }

        self.clock_beats += beats_per_sample;
        let mut boundary = false;
        while self.clock_beats >= division_beats {
            self.clock_beats -= division_beats;
            boundary = true;
        }

        if self.held_len == 0 {
            // Nothing is held, so the arpeggiator goes quiet.
            if self.current.is_some() {
                self.current = None;
                result.release = true;
            }
            return result;
        }

        if boundary {
            let index = self.next_index(self.held_len, pattern);
            let note = self.held[index].unwrap_or_default();
            let (key, velocity) = (note.key, note.velocity);

            // The octave cycles through the configured range.
            let key = (key as i32 + 12 * self.octave as i32).clamp(0, 127) as u8;
            self.octave = (self.octave + 1) % octaves.max(1);

            self.current = Some((key, velocity));

            // The gate closes at the end of the step.
            let beats_per_second = beats_per_sample * sample_rate;
            let division_seconds = division_beats / beats_per_second.max(1e-9);
            let gate =
                division_seconds * (gate_percent.clamp(0.0, 100.0) as f64 / 100.0) * sample_rate;
            self.gate_samples = gate.max(1.0) as u32;

            result.trigger = Some((key, velocity));
        }

        // The gate. The countdown does not start on the sample that triggers
        // the note, so that the note sounds for the whole length of the gate.
        if result.trigger.is_none() && self.gate_samples > 0 {
            self.gate_samples -= 1;
            if self.gate_samples == 0 && self.current.is_some() {
                self.current = None;
                result.release = true;
            }
        }

        result
    }

    /// Picks the index of the note that is played next and updates the state
    /// of the pattern.
    fn next_index(&mut self, count: usize, pattern: ArpPattern) -> usize {
        let first = self.played == 0;
        self.played += 1;

        match pattern {
            ArpPattern::Up => {
                let index = if first { 0 } else { self.step.min(count - 1) };
                self.step = (index + 1) % count;
                index
            }
            ArpPattern::Down => {
                let index = if first {
                    count - 1
                } else {
                    self.step.min(count - 1)
                };
                self.step = (index + count - 1) % count;
                index
            }
            ArpPattern::UpDown => {
                let index = if first { 0 } else { self.step.min(count - 1) };

                // The direction flips at the ends of the list. The end notes
                // are not repeated.
                let mut next = index as i32 + self.direction;
                if next >= count as i32 {
                    self.direction = -1;
                    next = count as i32 - 2;
                } else if next < 0 {
                    self.direction = 1;
                    next = 1;
                }
                self.step = next.clamp(0, count as i32 - 1) as usize;
                index
            }
            ArpPattern::Random => {
                let index = (self.next_random() as usize) % count;
                self.step = index;
                index
            }
        }
    }

    /// A random number, used by the random pattern.
    fn next_random(&mut self) -> u32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        x
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_RATE: f32 = 48_000.0;

    /// The number of samples of one sixteenth note at 120 BPM.
    fn step_samples() -> usize {
        (SAMPLE_RATE as f64 * 0.25 / 2.0) as usize
    }

    /// Runs the arpeggiator until the next note is triggered and returns its
    /// key.
    fn next_note(arp: &mut Arp, pattern: ArpPattern) -> u8 {
        let beats_per_sample = 120.0 / (60.0 * SAMPLE_RATE as f64);
        for _ in 0..(step_samples() * 4) {
            let tick = arp.tick(beats_per_sample, 0.25, 100.0, pattern, 1, SAMPLE_RATE);
            if let Some((key, _)) = tick.trigger {
                return key;
            }
        }
        panic!("the arpeggiator did not trigger a note");
    }

    #[test]
    fn the_up_pattern_walks_upwards() {
        let mut arp = Arp::new();
        arp.note_on(60, 1.0, false);
        arp.note_on(64, 1.0, false);
        arp.note_on(67, 1.0, false);

        assert_eq!(next_note(&mut arp, ArpPattern::Up), 60);
        assert_eq!(next_note(&mut arp, ArpPattern::Up), 64);
        assert_eq!(next_note(&mut arp, ArpPattern::Up), 67);
        // The pattern wraps around.
        assert_eq!(next_note(&mut arp, ArpPattern::Up), 60);
    }

    #[test]
    fn the_down_pattern_walks_downwards() {
        let mut arp = Arp::new();
        arp.note_on(60, 1.0, false);
        arp.note_on(64, 1.0, false);
        arp.note_on(67, 1.0, false);

        assert_eq!(next_note(&mut arp, ArpPattern::Down), 67);
        assert_eq!(next_note(&mut arp, ArpPattern::Down), 64);
        assert_eq!(next_note(&mut arp, ArpPattern::Down), 60);
        // The pattern wraps around.
        assert_eq!(next_note(&mut arp, ArpPattern::Down), 67);
    }

    #[test]
    fn the_up_down_pattern_does_not_repeat_the_end_points() {
        let mut arp = Arp::new();
        arp.note_on(60, 1.0, false);
        arp.note_on(64, 1.0, false);
        arp.note_on(67, 1.0, false);

        let notes: Vec<u8> = (0..6)
            .map(|_| next_note(&mut arp, ArpPattern::UpDown))
            .collect();
        assert_eq!(notes, [60, 64, 67, 64, 60, 64]);
    }

    #[test]
    fn the_gate_releases_the_note() {
        let mut arp = Arp::new();
        arp.note_on(60, 1.0, false);

        let beats_per_sample = 120.0 / (60.0 * SAMPLE_RATE as f64);
        let division = 0.25;
        let division_samples = (division / beats_per_sample).round() as usize;

        // The first step triggers the note.
        let mut trigger_sample = None;
        for sample in 0..(division_samples * 2) {
            let tick = arp.tick(
                beats_per_sample,
                division,
                50.0,
                ArpPattern::Up,
                1,
                SAMPLE_RATE,
            );
            if let Some((key, _)) = tick.trigger {
                assert_eq!(key, 60);
                trigger_sample = Some(sample);
                break;
            }
        }
        trigger_sample.expect("the note was never triggered");
        assert!(arp.is_sounding());

        // The gate closes halfway through the step.
        let mut release_sample = None;
        for sample in 0..(division_samples * 2) {
            let tick = arp.tick(
                beats_per_sample,
                division,
                50.0,
                ArpPattern::Up,
                1,
                SAMPLE_RATE,
            );
            if tick.release {
                release_sample = Some(sample);
                break;
            }
        }
        let release_sample = release_sample.expect("the gate never closed");

        let gate_samples = release_sample + 1;
        assert!(
            gate_samples > division_samples / 3 && gate_samples < division_samples * 2 / 3,
            "the gate closed after {gate_samples} samples"
        );
        assert!(!arp.is_sounding());
    }

    #[test]
    fn the_latch_keeps_the_notes() {
        let mut arp = Arp::new();
        arp.note_on(60, 1.0, true);
        arp.note_on(64, 1.0, true);
        arp.note_off(60);
        arp.note_off(64);

        // The notes are still there, so the arpegginator keeps playing.
        assert_eq!(next_note(&mut arp, ArpPattern::Up), 60);
        assert_eq!(next_note(&mut arp, ArpPattern::Up), 64);

        // A new note after the latch was released starts a new chord.
        arp.note_on(67, 1.0, true);
        assert_eq!(next_note(&mut arp, ArpPattern::Up), 67);
    }

    #[test]
    fn the_latch_is_remembered_per_note() {
        // A note that was played without the latch is released, even when the
        // latch is switched on in the meantime.
        let mut arp = Arp::new();
        arp.note_on(60, 1.0, false);
        arp.note_off(60);

        let beats_per_sample = 120.0 / (60.0 * SAMPLE_RATE as f64);
        for _ in 0..step_samples() {
            let tick = arp.tick(
                beats_per_sample,
                0.25,
                100.0,
                ArpPattern::Up,
                1,
                SAMPLE_RATE,
            );
            assert!(tick.trigger.is_none(), "a note was triggered");
        }
        assert!(!arp.is_sounding());

        // A note that was played with the latch stays held, even when the
        // latch is switched off in the meantime.
        let mut arp = Arp::new();
        arp.note_on(60, 1.0, true);
        arp.note_off(60);

        assert_eq!(next_note(&mut arp, ArpPattern::Up), 60);
    }

    #[test]
    fn the_gate_is_exactly_as_long_as_the_gate_setting() {
        let mut arp = Arp::new();
        arp.note_on(60, 1.0, false);

        let beats_per_sample = 120.0 / (60.0 * SAMPLE_RATE as f64);
        let division = 0.25;
        let division_samples = (division / beats_per_sample).round() as usize;

        let mut trigger_sample = None;
        let mut release_sample = None;
        for sample in 0..(division_samples * 2) {
            let tick = arp.tick(
                beats_per_sample,
                division,
                50.0,
                ArpPattern::Up,
                1,
                SAMPLE_RATE,
            );
            if tick.trigger.is_some() {
                trigger_sample = Some(sample);
            }
            if tick.release {
                release_sample = Some(sample);
                break;
            }
        }

        let trigger_sample = trigger_sample.expect("the note was never triggered");
        let release_sample = release_sample.expect("the gate never closed");

        // A gate of 50 % of a step of 6000 samples is 3000 samples long.
        assert_eq!(
            release_sample - trigger_sample,
            division_samples / 2,
            "the gate was {} samples long",
            release_sample - trigger_sample
        );
        assert!(!arp.is_sounding());
    }

    #[test]
    fn the_pattern_starts_over_when_it_changes() {
        let mut arp = Arp::new();
        arp.note_on(60, 1.0, false);
        arp.note_on(64, 1.0, false);
        arp.note_on(67, 1.0, false);

        assert_eq!(next_note(&mut arp, ArpPattern::Up), 60);

        // The downwards pattern starts at the top of the chord again.
        assert_eq!(next_note(&mut arp, ArpPattern::Down), 67);
        assert_eq!(next_note(&mut arp, ArpPattern::Down), 64);
    }

    #[test]
    fn without_the_latch_the_notes_are_released() {
        let mut arp = Arp::new();
        arp.note_on(60, 1.0, false);
        arp.note_off(60);

        let beats_per_sample = 120.0 / (60.0 * SAMPLE_RATE as f64);
        for _ in 0..step_samples() {
            let tick = arp.tick(
                beats_per_sample,
                0.25,
                100.0,
                ArpPattern::Up,
                1,
                SAMPLE_RATE,
            );
            assert!(tick.trigger.is_none(), "a note was triggered");
        }
        assert!(!arp.is_sounding());
    }

    #[test]
    fn the_octaves_are_played_in_turn() {
        let mut arp = Arp::new();
        arp.note_on(60, 1.0, false);

        let beats_per_sample = 120.0 / (60.0 * SAMPLE_RATE as f64);
        let mut keys = Vec::new();
        for _ in 0..(step_samples() * 5) {
            let tick = arp.tick(
                beats_per_sample,
                0.25,
                100.0,
                ArpPattern::Up,
                2,
                SAMPLE_RATE,
            );
            if let Some((key, _)) = tick.trigger {
                keys.push(key);
            }
        }

        assert!(keys.len() >= 4, "only {} notes were played", keys.len());
        assert_eq!(&keys[..4], &[60, 72, 60, 72]);
    }

    #[test]
    fn the_random_pattern_stays_in_range() {
        let mut arp = Arp::new();
        arp.note_on(60, 1.0, false);
        arp.note_on(64, 1.0, false);
        arp.note_on(67, 1.0, false);

        for _ in 0..16 {
            let key = next_note(&mut arp, ArpPattern::Random);
            assert!(key == 60 || key == 64 || key == 67, "{key} is not held");
        }
    }
}
