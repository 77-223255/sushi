//! The bridge between the two keyboards of the panel and the audio thread.
//!
//! The on-screen piano and the computer keyboard produce the same note
//! requests as a MIDI keyboard. The requests travel through a lock-free ring
//! buffer to the audio thread, which handles them on exactly the same path as
//! the note events of the host, so a key of the panel can never behave
//! differently from the note of a MIDI keyboard.
//!
//! The audio thread publishes the keys that are down in a few atomics. The
//! editor lights the keys up from that, which includes the notes that arrive
//! over MIDI: the panel shows what the synthesizer actually plays, not what
//! the GUI itself pressed.

use std::sync::atomic::{AtomicU32, Ordering};

/// How many 32-bit words hold the 128 MIDI keys.
const WORDS: usize = 4;

/// The number of notes that the editor may queue between two audio blocks.
///
/// A block is a few milliseconds and a human hand cannot press more than a
/// handful of keys in that time, so the queue is small and fixed: the audio
/// thread never allocates and a full queue only drops the newest note.
pub const QUEUE_CAPACITY: usize = 128;

/// A note that the panel asked for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KeyboardNote {
    /// The MIDI key number, in `0..=127`.
    pub key: u8,
    /// The velocity of the note, in `0..=1`.
    pub velocity: f32,
    /// Whether the note starts (`true`) or ends (`false`).
    pub on: bool,
}

/// The keys that the synthesizer currently plays.
///
/// The bits are set by the audio thread when it starts a voice, so every
/// source of notes is included: the host, a MIDI keyboard and the panel
/// itself. Reading and writing are relaxed on purpose: the lights only have
/// to be close to the sound, and a frame that misses a key is redrawn a few
/// milliseconds later.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_down_only_between_its_press_and_its_release() {
        let keys = HeldKeys::default();
        assert!(!keys.is_down(60));

        keys.press(60);
        assert!(keys.is_down(60));
        assert!(!keys.is_down(61));

        keys.release(60);
        assert!(!keys.is_down(60));
    }

    #[test]
    fn every_one_of_the_128_keys_has_its_own_bit() {
        let keys = HeldKeys::default();
        for key in 0..128u8 {
            keys.press(key);
        }

        for key in 0..128u8 {
            assert!(keys.is_down(key), "the key {key} is not lit");
        }

        // Releasing one key leaves the others alone, including the keys of
        // the neighbouring words.
        keys.release(31);
        keys.release(32);
        keys.release(127);
        assert!(!keys.is_down(31));
        assert!(!keys.is_down(32));
        assert!(!keys.is_down(127));
        assert!(keys.is_down(30));
        assert!(keys.is_down(33));
        assert!(keys.is_down(126));

        keys.clear();
        for key in 0..128u8 {
            assert!(!keys.is_down(key), "the key {key} is still lit");
        }
    }

    #[test]
    fn a_key_outside_of_the_midi_range_is_ignored() {
        let keys = HeldKeys::default();
        keys.press(200);
        keys.release(200);
        assert!(!keys.is_down(200));
        assert!(!keys.is_down(72));
    }
}
