
use crate::params::{ArpChord, ArpPattern};

/// The maximum number of notes that can be held at the same time. Notes beyond
/// this limit are ignored.
const MAX_HELD_NOTES: usize = 16;

/// The maximum number of steps of the pattern.
const MAX_LENGTH: usize = 16;

/// The maximum number of octaves of the pattern.
const MAX_OCTAVES: usize = 3;

/// The maximum number of retriggers of a step.
const MAX_RATCHET: usize = 4;

/// The maximum number of notes that one arpeggiator step can start. The
/// chord modes currently need at most two, but the scheduler and the voice
/// bookkeeping in the host keep room for a four-note chord without allocating.
pub const MAX_CHORD_NOTES: usize = 4;

/// The seed of the random number generator.
const RNG_SEED: u32 = 0x2545_F491;

#[derive(Debug, Clone, Copy)]
struct PendingNote {
    /// The key of the note.
    key: u8,
    /// The velocity of the note.
    velocity: f32,
    /// The number of samples until the note starts.
    samples: u32,
}

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

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ArpTick {
    /// The base note of a new pattern step, with its key and velocity. This is
    /// kept separate from `chord` so that the host can flash the step LED once
    /// and update the pattern position even when the chord is strummed.
    pub trigger: Option<(u8, f32)>,
    /// Every note that should start on this sample, including the base note.
    /// Strummed notes appear here on the sample they are due.
    pub chord: [Option<(u8, f32)>; MAX_CHORD_NOTES],
    /// The number of valid entries at the start of `chord`.
    pub chord_len: usize,
    /// Whether every note of the current step should be released. Pending
    /// strummed notes are cancelled at the same time.
    pub release: bool,
}

impl ArpTick {
    /// The notes that should start on this sample.
    pub fn notes(&self) -> impl Iterator<Item = (u8, f32)> + '_ {
        self.chord[..self.chord_len].iter().flatten().copied()
    }

    /// Adds a note to the chord that starts on this sample.
    fn push_note(&mut self, key: u8, velocity: f32) {
        if self.chord_len < MAX_CHORD_NOTES {
            self.chord[self.chord_len] = Some((key, velocity));
            self.chord_len += 1;
        }
    }

    /// Forgets the notes of a previous step in the same tick.
    fn clear_chord(&mut self) {
        self.chord = [None; MAX_CHORD_NOTES];
        self.chord_len = 0;
    }
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
    /// The strummed notes that have not started yet.
    pending: [Option<PendingNote>; MAX_CHORD_NOTES],
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
            pending: [None; MAX_CHORD_NOTES],
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
        self.clear_pending();
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
        self.clear_pending();
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
        self.clear_pending();
    }

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

    /// Whether a note is currently sounding, or a strummed note is waiting to
    /// start on a later sample.
    pub fn is_sounding(&self) -> bool {
        self.current.is_some() || self.has_pending_notes()
    }

    /// Whether the strum scheduler still has notes waiting to start.
    pub fn has_pending_notes(&self) -> bool {
        self.pending.iter().any(Option::is_some)
    }

    /// Whether the arpeggiator still holds any notes. This is used to keep
    /// the host from suspending the plugin between two arpeggiator steps.
    pub fn has_notes(&self) -> bool {
        self.held_len > 0
    }

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
        self.tick_extended(
            beats_per_sample,
            division_beats,
            gate_percent,
            pattern,
            octaves,
            swing_percent,
            length,
            ratchet,
            ArpChord::Off,
            0.0,
            1.0,
            48_000.0,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn tick_extended(
        &mut self,
        beats_per_sample: f64,
        division_beats: f64,
        gate_percent: f32,
        pattern: ArpPattern,
        octaves: u8,
        swing_percent: f32,
        length: usize,
        ratchet: usize,
        chord: ArpChord,
        strum_ms: f32,
        probability: f32,
        sample_rate: f32,
    ) -> ArpTick {
        let mut result = ArpTick::default();

        let division_beats = division_beats.max(1e-9);
        let octaves = octaves.clamp(1, MAX_OCTAVES as u8) as usize;
        let ratchet = ratchet.clamp(1, MAX_RATCHET);
        let length = length.clamp(1, MAX_LENGTH);
        let swing = (swing_percent.clamp(0.0, 75.0) as f64) / 100.0;
        let probability = if probability.is_finite() {
            probability.clamp(0.0, 1.0)
        } else {
            1.0
        };
        let strum_ms = if strum_ms.is_finite() {
            strum_ms.clamp(0.0, 80.0)
        } else {
            0.0
        };
        let sample_rate = if sample_rate.is_finite() {
            sample_rate.max(0.0)
        } else {
            0.0
        };
        let strum_samples = (strum_ms as f64 * 0.001 * sample_rate as f64).round() as u32;

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
            let was_sounding = self.is_sounding();
            self.clear_pending();
            self.current = None;
            if was_sounding {
                result.release = true;
            }
            return result;
        }

        let beats_per_sample = beats_per_sample.max(0.0);
        if beats_per_sample <= 0.0 {
            // The host does not play, so the clock does not move. A strummed
            // note must not be left waiting forever, though.
            if self.has_pending_notes() {
                self.clear_pending();
                self.current = None;
                self.gate_samples = 0;
                result.release = true;
            }
            return result;
        }

        // The strummed notes count down in samples, independent of the host
        // tempo. The due notes are held back until the clock and the gate have
        // been processed: a new step or a gate close cancels them instead.
        let (due, due_len) = self.take_due_pending();

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

            if self.accept_trigger(probability) {
                self.current = Some((key, velocity));
                self.current_step = self.position;
                result.trigger = Some((key, velocity));
                result.release = false;
                result.clear_chord();

                // A new step replaces the previous chord. Any pending strum
                // note from the old step is cancelled, including one that
                // happened to be due on this very sample.
                self.clear_pending();
                self.push_chord_notes(&mut result, key, velocity, chord, strum_samples);

                // The gate closes at the end of the sub step.
                let gate =
                    interval * (gate_percent.clamp(0.0, 100.0) as f64 / 100.0) / beats_per_sample;
                self.gate_samples = gate.round().max(1.0) as u32;
            } else {
                // The step was skipped by the probability. The previous chord
                // and every pending strummed note are released at once.
                self.clear_pending();
                self.current = None;
                self.gate_samples = 0;
                result.trigger = None;
                result.clear_chord();
                result.release = true;
            }

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
        if result.trigger.is_none() && !result.release && self.gate_samples > 0 {
            self.gate_samples -= 1;
            if self.gate_samples == 0 && self.current.is_some() {
                self.current = None;
                self.clear_pending();
                result.release = true;
            }
        }

        // Notes that were due on this sample only start when the step did not
        // trigger a new chord and the gate did not close. This is the
        // sample-accurate part of the strum.
        if result.trigger.is_none() && !result.release {
            for note in due[..due_len].iter().flatten() {
                result.push_note(note.0, note.1);
            }
        }

        result
    }

    /// Decides whether a step starts. The default of one is an exact bypass,
    /// so it never consumes a random number and old patches keep their random
    /// pattern sequence.
    fn accept_trigger(&mut self, probability: f32) -> bool {
        if probability >= 1.0 {
            true
        } else if probability <= 0.0 {
            false
        } else {
            let value = f64::from(self.next_random()) / 4_294_967_296.0;
            value < f64::from(probability)
        }
    }

    /// Builds the chord of a step and puts its notes into the result or into
    /// the pending scheduler.
    ///
    /// The base note is always immediate. With a strum, the added notes are
    /// spread evenly over the strum time in the order of the chord intervals.
    fn push_chord_notes(
        &mut self,
        result: &mut ArpTick,
        key: u8,
        velocity: f32,
        chord: ArpChord,
        strum_samples: u32,
    ) {
        let mut notes = [None; MAX_CHORD_NOTES];
        let note_count = build_chord(key, velocity, chord, &mut notes);

        if note_count <= 1 || strum_samples == 0 {
            for note in notes[..note_count].iter().flatten() {
                result.push_note(note.0, note.1);
            }
            return;
        }

        // The base note starts immediately.
        result.push_note(key, velocity);

        let extra_count = note_count - 1;
        for (index, note) in notes[1..note_count].iter().flatten().enumerate() {
            let delay = (u64::from(strum_samples) * (index as u64 + 1) / extra_count as u64) as u32;
            if delay == 0 {
                result.push_note(note.0, note.1);
            } else {
                self.schedule_pending(note.0, note.1, delay);
            }
        }
    }

    /// Puts a strummed note into the first free slot of the pending array.
    fn schedule_pending(&mut self, key: u8, velocity: f32, samples: u32) {
        if let Some(slot) = self.pending.iter_mut().find(|slot| slot.is_none()) {
            *slot = Some(PendingNote {
                key,
                velocity,
                samples,
            });
        }
    }

    /// Counts every pending note down by one sample and returns the notes that
    /// are due now.
    fn take_due_pending(&mut self) -> ([Option<(u8, f32)>; MAX_CHORD_NOTES], usize) {
        let mut due = [None; MAX_CHORD_NOTES];
        let mut due_len = 0;

        for slot in self.pending.iter_mut() {
            let Some(note) = slot else {
                continue;
            };
            note.samples = note.samples.saturating_sub(1);
            if note.samples == 0 {
                due[due_len] = Some((note.key, note.velocity));
                due_len += 1;
                *slot = None;
            }
        }

        (due, due_len)
    }

    /// Cancels every strummed note that has not started yet.
    fn clear_pending(&mut self) {
        self.pending = [None; MAX_CHORD_NOTES];
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

/// The intervals that a chord mode adds to the base note.
///
/// The modes are deliberately simple: each one adds its named interval to
/// every arpeggiator step. The fixed array in the audio thread leaves room for
/// richer chords later without changing the scheduler.
fn chord_intervals(chord: ArpChord) -> &'static [i32] {
    match chord {
        ArpChord::Off => &[],
        ArpChord::Octave => &[12],
        ArpChord::Fifth => &[7],
        ArpChord::Minor => &[3, 7],
        ArpChord::Major => &[4, 7],
    }
}

/// Builds the notes of a chord step. Notes above the MIDI range are skipped
/// instead of wrapped, and a key is only added once.
fn build_chord(
    key: u8,
    velocity: f32,
    chord: ArpChord,
    notes: &mut [Option<(u8, f32)>; MAX_CHORD_NOTES],
) -> usize {
    notes[0] = Some((key, velocity));
    let mut len = 1;

    for interval in chord_intervals(chord) {
        let note = key as i32 + interval;
        if !(0..=127).contains(&note) {
            continue;
        }
        let note = note as u8;
        if notes[..len]
            .iter()
            .flatten()
            .any(|(existing, _)| *existing == note)
        {
            continue;
        }

        if len < MAX_CHORD_NOTES {
            notes[len] = Some((note, velocity));
            len += 1;
        }
    }

    len
}

