//! A block-rate snapshot of the global modulation sources.
//!
//! The audio thread writes the values of the two LFOs, the three macros and
//! the mod wheel once per block. The editor reads them without a lock, so the
//! live markers of the modulation matrix cannot disturb the audio thread.
//! Every field is stored as the bits of an `f32` in an atomic integer, which
//! keeps the reads and the writes wait-free and allocation-free.

use std::sync::atomic::{AtomicU32, Ordering};

/// The values of the global modulation sources at the end of one block.
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
}

/// The shared snapshot of the modulation sources.
///
/// The audio thread calls [`store`](Self::store) once per block and the GUI
/// calls the getters while it draws. A relaxed ordering is enough: the values
/// are six independent readings of a slowly moving signal, and a block-rate
/// snapshot does not need to be a single atomic transaction.
#[derive(Debug, Default)]
pub struct ModFeedback {
    lfo1: AtomicU32,
    lfo2: AtomicU32,
    macro1: AtomicU32,
    macro2: AtomicU32,
    macro3: AtomicU32,
    mod_wheel: AtomicU32,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_feedback_starts_at_zero() {
        let feedback = ModFeedback::default();

        assert_eq!(feedback.lfo1(), 0.0);
        assert_eq!(feedback.lfo2(), 0.0);
        assert_eq!(feedback.macro1(), 0.0);
        assert_eq!(feedback.macro2(), 0.0);
        assert_eq!(feedback.macro3(), 0.0);
        assert_eq!(feedback.mod_wheel(), 0.0);
    }

    #[test]
    fn the_feedback_stores_and_returns_every_source() {
        let feedback = ModFeedback::default();
        feedback.store(ModFeedbackValues {
            lfo1: -0.75,
            lfo2: 0.5,
            macro1: 0.125,
            macro2: 0.25,
            macro3: 0.875,
            mod_wheel: 1.0,
        });

        assert_eq!(feedback.lfo1(), -0.75);
        assert_eq!(feedback.lfo2(), 0.5);
        assert_eq!(feedback.macro1(), 0.125);
        assert_eq!(feedback.macro2(), 0.25);
        assert_eq!(feedback.macro3(), 0.875);
        assert_eq!(feedback.mod_wheel(), 1.0);
    }

    #[test]
    fn a_later_store_replaces_the_snapshot() {
        let feedback = ModFeedback::default();
        feedback.store(ModFeedbackValues {
            lfo1: 1.0,
            lfo2: -1.0,
            macro1: 1.0,
            macro2: 0.0,
            macro3: 0.5,
            mod_wheel: 0.75,
        });
        feedback.store(ModFeedbackValues {
            lfo1: -0.25,
            lfo2: 0.25,
            macro1: 0.0,
            macro2: 0.5,
            macro3: 1.0,
            mod_wheel: 0.0,
        });

        assert_eq!(feedback.lfo1(), -0.25);
        assert_eq!(feedback.lfo2(), 0.25);
        assert_eq!(feedback.macro1(), 0.0);
        assert_eq!(feedback.macro2(), 0.5);
        assert_eq!(feedback.macro3(), 1.0);
        assert_eq!(feedback.mod_wheel(), 0.0);
    }
}
