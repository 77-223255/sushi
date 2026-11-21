//! A block-rate snapshot of the global modulation sources, and a short history
//! of what they actually did.
//!
//! The audio thread writes the values of the two LFOs, the three macros and
//! the mod wheel once per block. The editor reads them without a lock, so the
//! live markers of the modulation matrix cannot disturb the audio thread.
//! Every field is stored as the bits of an `f32` in an atomic integer, which
//! keeps the reads and the writes wait-free and allocation-free.
//!
//! The same block also appends the six values to a ring buffer, so the editor
//! can draw the shape that the sources really produced instead of a drawing of
//! the waveform they are set to.

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
    /// The value of the modulation envelope of the newest voice, `0..1`.
    pub mod_env: f32,
    /// The value of the amplifier envelope of the newest voice, `0..1`.
    pub amp_env: f32,
}

/// The sources that the history keeps, in the order of the matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedSource {
    Lfo1,
    Lfo2,
    ModEnv,
    AmpEnv,
    Macro1,
    Macro2,
    Macro3,
    ModWheel,
}

impl FeedSource {
    /// Every source that has a value the editor can read.
    pub const ALL: [FeedSource; 8] = [
        FeedSource::Lfo1,
        FeedSource::Lfo2,
        FeedSource::ModEnv,
        FeedSource::AmpEnv,
        FeedSource::Macro1,
        FeedSource::Macro2,
        FeedSource::Macro3,
        FeedSource::ModWheel,
    ];

    fn index(self) -> usize {
        match self {
            FeedSource::Lfo1 => 0,
            FeedSource::Lfo2 => 1,
            FeedSource::ModEnv => 2,
            FeedSource::AmpEnv => 3,
            FeedSource::Macro1 => 4,
            FeedSource::Macro2 => 5,
            FeedSource::Macro3 => 6,
            FeedSource::ModWheel => 7,
        }
    }

    fn value(self, values: &ModFeedbackValues) -> f32 {
        match self {
            FeedSource::Lfo1 => values.lfo1,
            FeedSource::Lfo2 => values.lfo2,
            FeedSource::ModEnv => values.mod_env,
            FeedSource::AmpEnv => values.amp_env,
            FeedSource::Macro1 => values.macro1,
            FeedSource::Macro2 => values.macro2,
            FeedSource::Macro3 => values.macro3,
            FeedSource::ModWheel => values.mod_wheel,
        }
    }
}

/// The number of blocks that the history keeps. At a 64 sample block and
/// 48 kHz this is about a third of a second, which shows more than one cycle
/// of a slow LFO and still settles fast enough to look live.
pub const HISTORY_LENGTH: usize = 256;

/// A wait-free ring buffer of the last values of one source.
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

/// The shared snapshot of the modulation sources.
///
/// The audio thread calls [`store`](Self::store) once per block and the GUI
/// calls the getters while it draws. A relaxed ordering is enough: the values
/// are readings of slowly moving signals, and a block-rate snapshot does not
/// need to be a single atomic transaction.
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
    history: [Ring; 8],
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
        for source in FeedSource::ALL {
            self.history[source.index()].push(source.value(&values));
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

    /// The history of one source, oldest first. The vector is reused by the
    /// caller, so drawing does not allocate.
    pub fn history(&self, source: FeedSource, out: &mut Vec<f32>) {
        self.history[source.index()].snapshot(out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            mod_env: 0.5,
            amp_env: 0.25,
        });

        assert_eq!(feedback.lfo1(), -0.75);
        assert_eq!(feedback.lfo2(), 0.5);
        assert_eq!(feedback.macro1(), 0.125);
        assert_eq!(feedback.macro2(), 0.25);
        assert_eq!(feedback.macro3(), 0.875);
        assert_eq!(feedback.mod_wheel(), 1.0);
        assert_eq!(feedback.mod_env(), 0.5);
        assert_eq!(feedback.amp_env(), 0.25);
    }

    #[test]
    fn the_history_keeps_the_last_values_in_order() {
        let feedback = ModFeedback::default();
        let mut history = Vec::new();
        for step in 0..HISTORY_LENGTH + 16 {
            feedback.store(ModFeedbackValues {
                lfo1: step as f32,
                ..Default::default()
            });
        }
        feedback.history(FeedSource::Lfo1, &mut history);
        assert_eq!(history.len(), HISTORY_LENGTH);
        // The last value written is the newest one.
        assert_eq!(
            history[HISTORY_LENGTH - 1],
            (HISTORY_LENGTH + 15) as f32
        );
        // The oldest of the ring is one past the newest.
        assert_eq!(history[0], 16.0);
    }
}
