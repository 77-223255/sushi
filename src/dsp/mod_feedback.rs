
use std::sync::atomic::{AtomicU32, Ordering};

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ModFeedbackValues {
    /// The value of the first LFO, `-1..1`.
    pub lfo1: f32,
    /// The value of the second LFO, `-1..1`.
    pub lfo2: f32,
    /// The value of the first macro, `0..1`.
    pub macro1: f32,
    /// The value of the second macro, `0..1`.
    pub macro2: f32,
    /// The value of the third macro, `0..1`.
    pub macro3: f32,
    /// The smoothed position of the mod wheel, `0..1`.
    pub mod_wheel: f32,
    /// The value of the modulation envelope of the newest voice, `0..1`.
    pub mod_env: f32,
    /// The value of the amplifier envelope of the newest voice, `0..1`.
    pub amp_env: f32,
    /// The velocity of the newest note, `0..1`.
    pub velocity: f32,
    /// The key of the newest note as a MIDI number.
    pub key: f32,
    /// The per-note random value of the newest voice, `-1..1`.
    pub random: f32,
    /// Whether a voice is sounding, so the panel knows that the three values
    /// above belong to a real note.
    pub voice_active: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedSource {
    Lfo1,
    Lfo2,
    ModEnv,
    AmpEnv,
    ModWheel,
}

impl FeedSource {
    /// Every source that has a value the editor can read.
    pub const ALL: [FeedSource; 5] = [
        FeedSource::Lfo1,
        FeedSource::Lfo2,
        FeedSource::ModEnv,
        FeedSource::AmpEnv,
        FeedSource::ModWheel,
    ];

    fn index(self) -> usize {
        match self {
            FeedSource::Lfo1 => 0,
            FeedSource::Lfo2 => 1,
            FeedSource::ModEnv => 2,
            FeedSource::AmpEnv => 3,
            FeedSource::ModWheel => 4,
        }
    }

    fn value(self, values: &ModFeedbackValues) -> f32 {
        match self {
            FeedSource::Lfo1 => values.lfo1,
            FeedSource::Lfo2 => values.lfo2,
            FeedSource::ModEnv => values.mod_env,
            FeedSource::AmpEnv => values.amp_env,
            FeedSource::ModWheel => values.mod_wheel,
        }
    }
}

/// The number of blocks that the history keeps. At a 64 sample block and
/// 48 kHz this is about a third of a second, which shows more than one cycle
/// of a slow LFO and still settles fast enough to look live.
pub const HISTORY_LENGTH: usize = 256;

#[derive(Debug)]
struct Ring {
    values: [AtomicU32; HISTORY_LENGTH],
    next: AtomicU32,
}

impl Default for Ring {
    fn default() -> Self {
        Self {
            values: [const { AtomicU32::new(0) }; HISTORY_LENGTH],
            next: AtomicU32::new(0),
        }
    }
}

impl Ring {
    #[inline]
    fn push(&self, value: f32) {
        let index = self.next.load(Ordering::Relaxed) as usize % HISTORY_LENGTH;
        self.values[index].store(value.to_bits(), Ordering::Relaxed);
        self.next
            .store((index + 1) as u32 % HISTORY_LENGTH as u32, Ordering::Relaxed);
    }

    /// Copies the history in the order it was written, oldest first.
    fn snapshot(&self, out: &mut Vec<f32>) {
        out.clear();
        let next = self.next.load(Ordering::Relaxed) as usize % HISTORY_LENGTH;
        for step in 0..HISTORY_LENGTH {
            let index = (next + step) % HISTORY_LENGTH;
            out.push(f32::from_bits(self.values[index].load(Ordering::Relaxed)));
        }
    }
}

#[derive(Debug, Default)]
pub struct ModFeedback {
    lfo1: AtomicU32,
    lfo2: AtomicU32,
    macro1: AtomicU32,
    macro2: AtomicU32,
    macro3: AtomicU32,
    mod_wheel: AtomicU32,
    mod_env: AtomicU32,
    amp_env: AtomicU32,
    velocity: AtomicU32,
    key: AtomicU32,
    random: AtomicU32,
    voice_active: AtomicU32,
    history: [Ring; 5],
    /// The last random values, one per note.
    note_random: Ring,
    /// The last value that was pushed into `note_random`.
    last_note_random: AtomicU32,
}

impl ModFeedback {
    /// Publishes the last values of one block. This method does not allocate
    /// and never blocks.
    pub fn store(&self, values: ModFeedbackValues) {
        self.lfo1.store(values.lfo1.to_bits(), Ordering::Relaxed);
        self.lfo2.store(values.lfo2.to_bits(), Ordering::Relaxed);
        self.macro1
            .store(values.macro1.to_bits(), Ordering::Relaxed);
        self.macro2
            .store(values.macro2.to_bits(), Ordering::Relaxed);
        self.macro3
            .store(values.macro3.to_bits(), Ordering::Relaxed);
        self.mod_wheel
            .store(values.mod_wheel.to_bits(), Ordering::Relaxed);
        self.mod_env
            .store(values.mod_env.to_bits(), Ordering::Relaxed);
        self.amp_env
            .store(values.amp_env.to_bits(), Ordering::Relaxed);
        self.velocity
            .store(values.velocity.to_bits(), Ordering::Relaxed);
        self.key.store(values.key.to_bits(), Ordering::Relaxed);
        self.random
            .store(values.random.to_bits(), Ordering::Relaxed);
        self.voice_active
            .store(u32::from(values.voice_active), Ordering::Relaxed);
        for source in FeedSource::ALL {
            self.history[source.index()].push(source.value(&values));
        }
        // The per-note random only changes when a new note is drawn, so the
        // ring records one step per note instead of one per block.
        let bits = values.random.to_bits();
        if bits != self.last_note_random.load(Ordering::Relaxed) {
            self.last_note_random.store(bits, Ordering::Relaxed);
            self.note_random.push(values.random);
        }
    }

    /// The last value of the first LFO, `-1..1`.
    pub fn lfo1(&self) -> f32 {
        f32::from_bits(self.lfo1.load(Ordering::Relaxed))
    }

    /// The last value of the second LFO, `-1..1`.
    pub fn lfo2(&self) -> f32 {
        f32::from_bits(self.lfo2.load(Ordering::Relaxed))
    }

    /// The last value of the first macro, `0..1`.
    pub fn macro1(&self) -> f32 {
        f32::from_bits(self.macro1.load(Ordering::Relaxed))
    }

    /// The last value of the second macro, `0..1`.
    pub fn macro2(&self) -> f32 {
        f32::from_bits(self.macro2.load(Ordering::Relaxed))
    }

    /// The last value of the third macro, `0..1`.
    pub fn macro3(&self) -> f32 {
        f32::from_bits(self.macro3.load(Ordering::Relaxed))
    }

    /// The last smoothed position of the mod wheel, `0..1`.
    pub fn mod_wheel(&self) -> f32 {
        f32::from_bits(self.mod_wheel.load(Ordering::Relaxed))
    }

    /// The last value of the modulation envelope, `0..1`.
    pub fn mod_env(&self) -> f32 {
        f32::from_bits(self.mod_env.load(Ordering::Relaxed))
    }

    /// The last value of the amplifier envelope, `0..1`.
    pub fn amp_env(&self) -> f32 {
        f32::from_bits(self.amp_env.load(Ordering::Relaxed))
    }

    /// The velocity of the newest note, `0..1`.
    pub fn velocity(&self) -> f32 {
        f32::from_bits(self.velocity.load(Ordering::Relaxed))
    }

    /// The key of the newest note as a MIDI number.
    pub fn key(&self) -> f32 {
        f32::from_bits(self.key.load(Ordering::Relaxed))
    }

    /// The per-note random value of the newest voice, `-1..1`.
    pub fn random(&self) -> f32 {
        f32::from_bits(self.random.load(Ordering::Relaxed))
    }

    /// Whether a voice is sounding right now.
    pub fn voice_active(&self) -> bool {
        self.voice_active.load(Ordering::Relaxed) != 0
    }

    /// The history of the per-note random values, oldest first: one step per
    /// note. The panel draws it like a sample and hold LFO.
    pub fn note_random_history(&self, out: &mut Vec<f32>) {
        self.note_random.snapshot(out);
    }

    /// The history of one source, oldest first. The vector is reused by the
    /// caller, so drawing does not allocate.
    pub fn history(&self, source: FeedSource, out: &mut Vec<f32>) {
        self.history[source.index()].snapshot(out);
    }
}

