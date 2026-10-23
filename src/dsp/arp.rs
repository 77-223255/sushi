//! A host-tempo-synced arpeggiator.
//!
//! The arpeggiator collects the notes that the host plays and turns them into
//! a repeating pattern. Its clock is driven by the host's tempo, so the
//! pattern always stays in time with the song.

use crate::params::ArpPattern;

/// The maximum number of notes that can be held at the same time. Notes beyond
/// this limit are ignored.
const MAX_HELD_NOTES: usize = 16;

/// The maximum number of steps of the pattern.
const MAX_LENGTH: usize = 16;

/// The maximum number of octaves of the pattern.
const MAX_OCTAVES: usize = 3;

/// The maximum number of retriggers of a step.
const MAX_RATCHET: usize = 4;

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
    /// The number of the note-on that added the note. It defines the order in
    /// which the notes were played.
    order: u32,
}

impl Default for HeldNote {
    fn default() -> Self {
        Self {
            key: 60,
            velocity: 1.0,
            latched: false,
            order: 0,
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

/// A simple arpeggiator with an up, down, up-down, random and as-played
/// pattern.
pub struct Arp {
    /// The held notes, sorted by pitch. Only the first `held_len` entries are
    /// used.
    held: [Option<HeldNote>; MAX_HELD_NOTES],
    /// The number of held notes.
    held_len: usize,
    /// The number of keys that are physically pressed down.
    physical_held: usize,
    /// The number of notes that have been played. It is the order in which the
    /// notes were played.
    order: u32,
    /// The number of beats until the next step of the pattern.
    beats_to_boundary: f64,
    /// The length of the current sub step, in beats.
    sub_beats: f64,
    /// The number of samples left before the gate closes.
    gate_samples: u32,
    /// The position in the pattern.
    position: usize,
    /// The position of the step that is currently sounding. It only changes
    /// at the boundaries of the steps, never on a retrigger of the ratchet.
    current_step: usize,
    /// The number of the sub step inside the current step.
    sub_index: usize,
    /// The parity of the swing. It flips after every step.
    parity: bool,
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
            order: 0,
            beats_to_boundary: 0.0,
            sub_beats: 0.0,
            gate_samples: 0,
            position: 0,
            current_step: 0,
            sub_index: 0,
            parity: false,
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
        self.order = 0;
        self.beats_to_boundary = 0.0;
        self.sub_beats = 0.0;
        self.gate_samples = 0;
        self.position = 0;
        self.current_step = 0;
        self.sub_index = 0;
        self.parity = false;
        self.current = None;
        self.pattern = None;
        self.rng = RNG_SEED;
    }

    /// Stops the clock without forgetting the held notes. Used when the host
    /// transport stops; playback can resume at the next transport start.
    pub fn pause(&mut self) {
        self.current = None;
        self.gate_samples = 0;
        self.beats_to_boundary = 0.0;
        self.sub_beats = 0.0;
        self.sub_index = 0;
        self.parity = false;
    }

    /// Aligns the next step with the host's beat grid. Called after a
    /// transport start, loop or jump.
    pub fn resync(&mut self, position_beats: f64, division_beats: f64) {
        let division = division_beats.max(1e-9);
        let position = if position_beats.is_finite() {
            position_beats
        } else {
            0.0
        };
        let phase = position.rem_euclid(division);
        self.beats_to_boundary = if phase <= 1e-9 { 0.0 } else { division - phase };
        self.sub_index = 0;
        self.sub_beats = 0.0;
        self.position = 0;
        self.current_step = 0;
        self.current = None;
        self.gate_samples = 0;
        self.parity = false;
    }

    /// The position of the pattern that is currently sounding.
    ///
    /// The position is the index of the step of the pattern, counted from
    /// zero. It only changes at the boundaries of the steps: the retriggers
    /// of the ratchet play the same step again.
    #[allow(clippy::misnamed_getters)]
    pub fn position(&self) -> usize {
        self.current_step
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

        if self.held_len >= MAX_HELD_NOTES {
            // The note pool is full, so the note is ignored.
            return;
        }

        // Make room for the new note. The notes that come after it are moved
        // one slot up, so that the list stays sorted by pitch.
        for shift in (index..self.held_len).rev() {
            self.held[shift + 1] = self.held[shift];
        }
        let order = self.order;
        self.order = self.order.wrapping_add(1);
        self.held[index] = Some(HeldNote {
            key,
            velocity,
            latched: latch,
            order,
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

    /// Whether the arpeggiator still holds any notes. This is used to keep
    /// the host from suspending the plugin between two arpeggiator steps.
    pub fn has_notes(&self) -> bool {
        self.held_len > 0
    }

    /// Advances the arpeggiator by a single sample.
    ///
    /// `beats_per_sample` is the tempo of the host in beats per sample and
    /// `division_beats` the length of one step of the pattern.
    /// `gate_percent` is the length of the note as a percentage of the step,
    /// `pattern` the way the notes are walked through, `octaves` the number of
    /// octaves that the pattern covers, `swing_percent` the swing of the
    /// steps, `length` the number of steps of the pattern and `ratchet` the
    /// number of times every step is repeated.
    #[allow(clippy::too_many_arguments)]
    pub fn tick(
        &mut self,
        beats_per_sample: f64,
        division_beats: f64,
        gate_percent: f32,
        pattern: ArpPattern,
        octaves: u8,
        swing_percent: f32,
        length: usize,
        ratchet: usize,
    ) -> ArpTick {
        let mut result = ArpTick::default();

        let division_beats = division_beats.max(1e-9);
        let octaves = octaves.clamp(1, MAX_OCTAVES as u8) as usize;
        let ratchet = ratchet.clamp(1, MAX_RATCHET);
        let length = length.clamp(1, MAX_LENGTH);
        let swing = (swing_percent.clamp(0.0, 75.0) as f64) / 100.0;

        if self.pattern != Some(pattern) {
            // The pattern changed, so the pattern starts from the beginning
            // again.
            self.pattern = Some(pattern);
            self.position = 0;
            self.current_step = 0;
            self.sub_index = 0;
            self.parity = false;
        }

        if self.held_len == 0 {
            // Nothing is held, so the arpeggiator goes quiet. The pattern
            // starts from the beginning again when a new note comes in.
            self.position = 0;
            self.current_step = 0;
            self.sub_index = 0;
            self.parity = false;
            self.beats_to_boundary = 0.0;
            if self.current.is_some() {
                self.current = None;
                result.release = true;
            }
            return result;
        }

        let beats_per_sample = beats_per_sample.max(0.0);
        if beats_per_sample <= 0.0 {
            // The host does not play, so the clock does not move.
            return result;
        }

        self.beats_to_boundary -= beats_per_sample;
        while self.beats_to_boundary <= 0.0 {
            if self.sub_index == 0 {
                // A new step of the pattern begins. Its length alternates
                // between a long and a short one, which is the swing.
                let factor = if self.parity {
                    1.0 - swing * 0.5
                } else {
                    1.0 + swing * 0.5
                };
                self.parity = !self.parity;
                self.sub_beats = division_beats * factor / ratchet as f64;
            }
            let interval = self.sub_beats;

            let Some((key, velocity)) = self.note_at(self.position, pattern, octaves) else {
                break;
            };
            self.current = Some((key, velocity));
            self.current_step = self.position;
            result.trigger = Some((key, velocity));

            // The gate closes at the end of the sub step.
            let gate =
                interval * (gate_percent.clamp(0.0, 100.0) as f64 / 100.0) / beats_per_sample;
            self.gate_samples = gate.round().max(1.0) as u32;

            // The next sub step.
            self.sub_index += 1;
            if self.sub_index >= ratchet {
                self.sub_index = 0;
                self.position = (self.position + 1) % length;
            }

            self.beats_to_boundary += interval;
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

    /// The note that is played at the given position of the pattern.
    ///
    /// The pattern walks through the held notes, which are repeated over the
    /// octaves. The As Played pattern walks through them in the order in which
    /// they were played instead of their pitch.
    fn note_at(
        &mut self,
        position: usize,
        pattern: ArpPattern,
        octaves: usize,
    ) -> Option<(u8, f32)> {
        let held_len = self.held_len;
        if held_len == 0 {
            return None;
        }
        let list_len = held_len * octaves;

        // The entry of the list that is played at this position.
        let entry = match pattern {
            ArpPattern::Up | ArpPattern::AsPlayed => position % list_len,
            ArpPattern::Down => list_len - 1 - (position % list_len),
            ArpPattern::UpDown => {
                // The pattern goes up and down again, without repeating the
                // notes at the ends.
                let length = if list_len > 1 { 2 * list_len - 2 } else { 1 };
                let index = position % length;
                if index < list_len {
                    index
                } else {
                    2 * list_len - 2 - index
                }
            }
            ArpPattern::Random => (self.next_random() as usize) % list_len,
        };

        // The note of the entry and the octave it is played in.
        let base = entry % held_len;
        let octave = entry / held_len;
        let held_index = match pattern {
            ArpPattern::AsPlayed => self.as_played_index(base),
            _ => base,
        };

        let note = self.held[held_index]?;
        let key = (note.key as i32 + 12 * octave as i32).clamp(0, 127) as u8;
        Some((key, note.velocity))
    }

    /// The index of the held note that was played `index`-th, counting from
    /// the first note that was pressed down.
    fn as_played_index(&self, index: usize) -> usize {
        let mut order: [usize; MAX_HELD_NOTES] = [0; MAX_HELD_NOTES];
        for (position, slot) in order.iter_mut().enumerate().take(self.held_len) {
            *slot = position;
        }

        // The held notes are sorted by pitch, so the notes have to be sorted
        // by the order in which they were played.
        for position in 1..self.held_len {
            let mut current = position;
            while current > 0
                && self.note_order(order[current - 1]) > self.note_order(order[current])
            {
                order.swap(current - 1, current);
                current -= 1;
            }
        }

        order[index.min(self.held_len.saturating_sub(1))]
    }

    /// The number of the note-on that added the note at the given index.
    fn note_order(&self, index: usize) -> u32 {
        self.held[index].map(|note| note.order).unwrap_or(0)
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

    /// The tempo of the host in beats per sample.
    fn beats_per_sample() -> f64 {
        120.0 / (60.0 * SAMPLE_RATE as f64)
    }

    /// The number of samples of one sixteenth note at 120 BPM.
    fn step_samples() -> usize {
        (SAMPLE_RATE as f64 * 0.25 / 2.0) as usize
    }

    /// Ticks the arpeggiator with the default settings.
    fn tick(arp: &mut Arp, pattern: ArpPattern) -> ArpTick {
        arp.tick(beats_per_sample(), 0.25, 100.0, pattern, 1, 0.0, 16, 1)
    }

    /// Runs the arpeggiator until the next note is triggered and returns its
    /// key.
    fn next_note(arp: &mut Arp, pattern: ArpPattern) -> u8 {
        for _ in 0..(step_samples() * 4) {
            let tick = tick(arp, pattern);
            if let Some((key, _)) = tick.trigger {
                return key;
            }
        }
        panic!("the arpeggiator did not trigger a note");
    }

    /// The samples at which the arpeggiator triggers a note.
    fn trigger_samples(arp: &mut Arp, samples: usize, swing: f32, ratchet: usize) -> Vec<usize> {
        let mut triggers = Vec::new();
        for sample in 0..samples {
            let tick = arp.tick(
                beats_per_sample(),
                0.25,
                50.0,
                ArpPattern::Up,
                1,
                swing,
                16,
                ratchet,
            );
            if tick.trigger.is_some() {
                triggers.push(sample);
            }
        }
        triggers
    }

    #[test]
    fn the_arp_can_align_to_the_host_grid() {
        let mut arp = Arp::new();
        arp.note_on(60, 1.0, false);
        arp.resync(1.1, 0.25);

        assert!(arp.has_notes());
        assert!((arp.beats_to_boundary - 0.15).abs() < 1e-9);
    }

    #[test]
    fn pausing_the_arp_keeps_the_held_notes() {
        let mut arp = Arp::new();
        arp.note_on(60, 1.0, false);
        arp.pause();

        assert!(arp.has_notes());
        assert!(!arp.is_sounding());
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
    fn the_notes_are_kept_when_they_are_played_out_of_order() {
        let mut arp = Arp::new();
        arp.note_on(67, 1.0, false);
        arp.note_on(60, 1.0, false);
        arp.note_on(64, 1.0, false);

        // All three notes are still there.
        assert_eq!(next_note(&mut arp, ArpPattern::Up), 60);
        assert_eq!(next_note(&mut arp, ArpPattern::Up), 64);
        assert_eq!(next_note(&mut arp, ArpPattern::Up), 67);
    }

    #[test]
    fn the_down_pattern_walks_downwards_over_the_octaves() {
        let mut arp = Arp::new();
        arp.note_on(60, 1.0, false);
        arp.note_on(64, 1.0, false);
        arp.note_on(67, 1.0, false);

        let mut keys = Vec::new();
        for _ in 0..(step_samples() * 8) {
            let tick = arp.tick(
                beats_per_sample(),
                0.25,
                100.0,
                ArpPattern::Down,
                2,
                0.0,
                16,
                1,
            );
            if let Some((key, _)) = tick.trigger {
                keys.push(key);
            }
        }

        assert!(keys.len() >= 6, "only {} notes were played", keys.len());
        assert_eq!(&keys[..6], &[79, 76, 72, 67, 64, 60]);
    }

    #[test]
    fn the_up_down_pattern_walks_over_the_octaves() {
        let mut arp = Arp::new();
        arp.note_on(60, 1.0, false);
        arp.note_on(64, 1.0, false);
        arp.note_on(67, 1.0, false);

        let mut keys = Vec::new();
        for _ in 0..(step_samples() * 12) {
            let tick = arp.tick(
                beats_per_sample(),
                0.25,
                100.0,
                ArpPattern::UpDown,
                2,
                0.0,
                16,
                1,
            );
            if let Some((key, _)) = tick.trigger {
                keys.push(key);
            }
        }

        assert!(keys.len() >= 10, "only {} notes were played", keys.len());
        assert_eq!(&keys[..10], &[60, 64, 67, 72, 76, 79, 76, 72, 67, 64]);
    }

    #[test]
    fn the_as_played_pattern_follows_the_order_of_the_notes() {
        let mut arp = Arp::new();
        // The notes are played from the top down.
        arp.note_on(67, 1.0, false);
        arp.note_on(60, 1.0, false);
        arp.note_on(64, 1.0, false);

        assert_eq!(next_note(&mut arp, ArpPattern::AsPlayed), 67);
        assert_eq!(next_note(&mut arp, ArpPattern::AsPlayed), 60);
        assert_eq!(next_note(&mut arp, ArpPattern::AsPlayed), 64);
        // The pattern wraps around.
        assert_eq!(next_note(&mut arp, ArpPattern::AsPlayed), 67);
    }

    #[test]
    fn the_octaves_are_played_in_turn() {
        let mut arp = Arp::new();
        arp.note_on(60, 1.0, false);

        let mut keys = Vec::new();
        for _ in 0..(step_samples() * 5) {
            let tick = arp.tick(
                beats_per_sample(),
                0.25,
                100.0,
                ArpPattern::Up,
                2,
                0.0,
                16,
                1,
            );
            if let Some((key, _)) = tick.trigger {
                keys.push(key);
            }
        }

        assert!(keys.len() >= 4, "only {} notes were played", keys.len());
        assert_eq!(&keys[..4], &[60, 72, 60, 72]);
    }

    #[test]
    fn the_octaves_are_played_after_the_notes() {
        let mut arp = Arp::new();
        arp.note_on(60, 1.0, false);
        arp.note_on(64, 1.0, false);

        let mut keys = Vec::new();
        for _ in 0..(step_samples() * 5) {
            let tick = arp.tick(
                beats_per_sample(),
                0.25,
                100.0,
                ArpPattern::Up,
                2,
                0.0,
                16,
                1,
            );
            if let Some((key, _)) = tick.trigger {
                keys.push(key);
            }
        }

        assert!(keys.len() >= 4, "only {} notes were played", keys.len());
        assert_eq!(&keys[..4], &[60, 64, 72, 76]);
    }

    #[test]
    fn the_length_limits_the_pattern() {
        let mut arp = Arp::new();
        arp.note_on(60, 1.0, false);
        arp.note_on(64, 1.0, false);
        arp.note_on(67, 1.0, false);

        let mut keys = Vec::new();
        for _ in 0..(step_samples() * 5) {
            let tick = arp.tick(
                beats_per_sample(),
                0.25,
                100.0,
                ArpPattern::Up,
                1,
                0.0,
                2,
                1,
            );
            if let Some((key, _)) = tick.trigger {
                keys.push(key);
            }
        }

        assert!(keys.len() >= 4, "only {} notes were played", keys.len());
        assert_eq!(&keys[..4], &[60, 64, 60, 64]);
    }

    #[test]
    fn a_long_pattern_repeats_the_notes() {
        let mut arp = Arp::new();
        arp.note_on(60, 1.0, false);
        arp.note_on(64, 1.0, false);

        let mut keys = Vec::new();
        for _ in 0..(step_samples() * 6) {
            let tick = arp.tick(
                beats_per_sample(),
                0.25,
                100.0,
                ArpPattern::Up,
                1,
                0.0,
                16,
                1,
            );
            if let Some((key, _)) = tick.trigger {
                keys.push(key);
            }
        }

        assert!(keys.len() >= 4, "only {} notes were played", keys.len());
        assert_eq!(&keys[..4], &[60, 64, 60, 64]);
    }

    #[test]
    fn the_ratchet_repeats_every_step() {
        let mut arp = Arp::new();
        arp.note_on(60, 1.0, false);

        let triggers = trigger_samples(&mut arp, step_samples() * 3, 0.0, 4);

        // Every step of the pattern is repeated four times, so the notes come
        // every quarter of a step.
        assert!(
            triggers.len() >= 8,
            "only {} notes were played",
            triggers.len()
        );
        for pair in triggers.windows(2) {
            let interval = pair[1] - pair[0];
            assert!(
                (interval as i32 - (step_samples() / 4) as i32).abs() <= 2,
                "the retrigger came after {interval} samples"
            );
        }
    }

    #[test]
    fn the_swing_alternates_the_steps() {
        let mut arp = Arp::new();
        arp.note_on(60, 1.0, false);

        let triggers = trigger_samples(&mut arp, step_samples() * 4, 50.0, 1);
        assert!(
            triggers.len() >= 4,
            "only {} notes were played",
            triggers.len()
        );

        let intervals: Vec<usize> = triggers.windows(2).map(|pair| pair[1] - pair[0]).collect();

        // The first step is long, the second one is short.
        assert!(
            intervals[0] > intervals[1],
            "the steps do not alternate ({intervals:?})"
        );
        // The average stays the same, so the long step is 5/3 as long as the
        // short one.
        let ratio = intervals[0] as f32 / intervals[1] as f32;
        assert!(
            (ratio - 5.0 / 3.0).abs() < 0.05,
            "unexpected swing ratio: {ratio}"
        );
    }

    #[test]
    fn the_swing_is_off_by_default() {
        let mut arp = Arp::new();
        arp.note_on(60, 1.0, false);

        let triggers = trigger_samples(&mut arp, step_samples() * 4, 0.0, 1);
        let intervals: Vec<usize> = triggers.windows(2).map(|pair| pair[1] - pair[0]).collect();

        for interval in intervals {
            assert!(
                (interval as i32 - step_samples() as i32).abs() <= 1,
                "the steps are not even: {interval}"
            );
        }
    }

    #[test]
    fn the_gate_releases_the_note() {
        let mut arp = Arp::new();
        arp.note_on(60, 1.0, false);

        let division = 0.25;
        let division_samples = (division / beats_per_sample()).round() as usize;

        // The first step triggers the note.
        let mut trigger_sample = None;
        for sample in 0..(division_samples * 2) {
            let tick = arp.tick(
                beats_per_sample(),
                division,
                50.0,
                ArpPattern::Up,
                1,
                0.0,
                16,
                1,
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
                beats_per_sample(),
                division,
                50.0,
                ArpPattern::Up,
                1,
                0.0,
                16,
                1,
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
    fn the_position_advances_at_the_step_boundaries() {
        let mut arp = Arp::new();
        arp.note_on(60, 1.0, false);

        let mut positions = Vec::new();
        for _ in 0..(step_samples() * 5) {
            let tick = arp.tick(
                beats_per_sample(),
                0.25,
                100.0,
                ArpPattern::Up,
                1,
                0.0,
                4,
                1,
            );
            if tick.trigger.is_some() {
                positions.push(arp.position());
            }
        }

        assert!(
            positions.len() >= 4,
            "only {} notes were played",
            positions.len()
        );
        // The pattern has four steps, so it walks from zero to three and then
        // starts over.
        assert_eq!(&positions[..5], &[0, 1, 2, 3, 0]);
    }

    #[test]
    fn the_ratchet_does_not_move_the_position() {
        let mut arp = Arp::new();
        arp.note_on(60, 1.0, false);

        let mut positions = Vec::new();
        for _ in 0..(step_samples() * 3) {
            let tick = arp.tick(
                beats_per_sample(),
                0.25,
                100.0,
                ArpPattern::Up,
                1,
                0.0,
                16,
                4,
            );
            if tick.trigger.is_some() {
                positions.push(arp.position());
            }
        }

        assert!(
            positions.len() >= 5,
            "only {} notes were played",
            positions.len()
        );
        // Every step is played four times, so the position only changes once
        // every four notes.
        assert_eq!(&positions[..6], &[0, 0, 0, 0, 1, 1]);
    }

    #[test]
    fn the_position_starts_over_with_a_new_chord() {
        let mut arp = Arp::new();
        arp.note_on(60, 1.0, false);
        assert_eq!(next_note(&mut arp, ArpPattern::Up), 60);
        assert_eq!(arp.position(), 0);

        arp.note_off(60);
        // The arpeggiator goes quiet, so the pattern starts over.
        tick(&mut arp, ArpPattern::Up);
        arp.note_on(67, 1.0, false);
        assert_eq!(next_note(&mut arp, ArpPattern::Up), 67);
        assert_eq!(arp.position(), 0);
    }

    #[test]
    fn the_latch_keeps_the_notes() {
        let mut arp = Arp::new();
        arp.note_on(60, 1.0, true);
        arp.note_on(64, 1.0, true);
        arp.note_off(60);
        arp.note_off(64);

        // The notes are still there, so the arpeggiator keeps playing.
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

        for _ in 0..step_samples() {
            let tick = tick(&mut arp, ArpPattern::Up);
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

        let division = 0.25;
        let division_samples = (division / beats_per_sample()).round() as usize;

        let mut trigger_sample = None;
        let mut release_sample = None;
        for sample in 0..(division_samples * 2) {
            let tick = arp.tick(
                beats_per_sample(),
                division,
                50.0,
                ArpPattern::Up,
                1,
                0.0,
                16,
                1,
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
    fn the_gate_is_exact_with_the_ratchet() {
        let mut arp = Arp::new();
        arp.note_on(60, 1.0, false);

        let division = 0.25;
        let division_samples = (division / beats_per_sample()).round() as usize;

        let mut trigger_sample = None;
        let mut release_sample = None;
        for sample in 0..(division_samples * 2) {
            let tick = arp.tick(
                beats_per_sample(),
                division,
                50.0,
                ArpPattern::Up,
                1,
                0.0,
                16,
                4,
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

        // The step is divided into four sub steps, so the gate is a quarter
        // of the step long.
        assert_eq!(
            release_sample - trigger_sample,
            division_samples / 8,
            "the gate was {} samples long",
            release_sample - trigger_sample
        );
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

        for _ in 0..step_samples() {
            let tick = tick(&mut arp, ArpPattern::Up);
            assert!(tick.trigger.is_none(), "a note was triggered");
        }
        assert!(!arp.is_sounding());
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

    #[test]
    fn the_pattern_starts_over_when_the_notes_are_released() {
        let mut arp = Arp::new();
        arp.note_on(60, 1.0, false);
        arp.note_on(64, 1.0, false);
        assert_eq!(next_note(&mut arp, ArpPattern::Up), 60);

        arp.note_off(60);
        arp.note_off(64);

        // Nothing is held anymore, so the arpeggiator goes quiet.
        let mut released = false;
        for _ in 0..step_samples() {
            let tick = tick(&mut arp, ArpPattern::Up);
            assert!(tick.trigger.is_none(), "a note was triggered");
            released |= tick.release;
        }
        assert!(released, "the note was never released");

        // A new chord starts at the beginning of the pattern again.
        arp.note_on(67, 1.0, false);
        arp.note_on(60, 1.0, false);
        assert_eq!(next_note(&mut arp, ArpPattern::Up), 60);
    }
}
