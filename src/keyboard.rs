
use std::sync::atomic::{AtomicU32, Ordering};

/// How many 32-bit words hold the 128 MIDI keys.
const WORDS: usize = 4;

/// The number of notes that the editor may queue between two audio blocks.
///
/// A block is a few milliseconds and a human hand cannot press more than a
/// handful of keys in that time, so the queue is small and fixed: the audio
/// thread never allocates and a full queue only drops the newest note.
pub const QUEUE_CAPACITY: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KeyboardNote {
    /// The MIDI key number, in `0..=127`.
    pub key: u8,
    /// The velocity of the note, in `0..=1`.
    pub velocity: f32,
    /// Whether the note starts (`true`) or ends (`false`).
    pub on: bool,
}

#[derive(Debug, Default)]
pub struct HeldKeys {
    /// Bit `key % 32` of word `key / 32` belongs to the key with that number.
    words: [AtomicU32; WORDS],
}

impl HeldKeys {
    /// Marks a key as down.
    pub fn press(&self, key: u8) {
        let (word, bit) = place(key);
        if let Some(word) = self.words.get(word) {
            word.fetch_or(bit, Ordering::Relaxed);
        }
    }

    /// Marks a key as up.
    pub fn release(&self, key: u8) {
        let (word, bit) = place(key);
        if let Some(word) = self.words.get(word) {
            word.fetch_and(!bit, Ordering::Relaxed);
        }
    }

    /// Forgets every key. It is used when the host asks for silence and when
    /// the plugin is reset, so a light can never stay on after a stop.
    pub fn clear(&self) {
        for word in &self.words {
            word.store(0, Ordering::Relaxed);
        }
    }

    /// Whether the key is down.
    pub fn is_down(&self, key: u8) -> bool {
        let (word, bit) = place(key);
        self.words
            .get(word)
            .map(|word| word.load(Ordering::Relaxed) & bit != 0)
            .unwrap_or(false)
    }
}

/// The word and the bit of a key. A key outside of the MIDI range has no
/// place and is ignored by the accessors.
fn place(key: u8) -> (usize, u32) {
    (key as usize / 32, 1 << (key % 32))
}

