//! An eight step modulation sequencer.
//!
//! The sequencer runs on the host's clock and produces a bipolar value in the
//! `-1..1` range, which can be routed to the pitch, the filter or the pulse
//! width of the oscillators.

/// The number of steps of the sequencer.
const NUM_STEPS: usize = 8;

/// The time constant of the smoothing filter, in seconds.
const SMOOTHING_TIME: f32 = 0.008;

/// A step sequencer with eight steps.
pub struct StepSequencer {
    /// The position of the clock, in beats.
    clock_beats: f64,
    /// The index of the current step.
    index: usize,
    /// The current value, `-1..1`.
    value: f32,
    /// The value that the sequencer moves towards.
    target: f32,
    /// The one-pole coefficient of the smoothing filter.
    smoothing_coefficient: f32,
    /// The sample rate the smoothing coefficient was computed for.
    smoothing_rate: f32,
    /// Whether the sequencer has been initialised with its step values.
    initialized: bool,
}

impl Default for StepSequencer {
    fn default() -> Self {
        Self::new()
    }
}

impl StepSequencer {
    pub fn new() -> Self {
        Self {
            clock_beats: 0.0,
            index: 0,
            value: 0.0,
            target: 0.0,
            smoothing_coefficient: 0.0,
            smoothing_rate: 0.0,
            initialized: false,
        }
    }

    /// Starts the sequencer from the beginning.
    pub fn reset(&mut self) {
        self.clock_beats = 0.0;
        self.index = 0;
        self.value = 0.0;
        self.target = 0.0;
        self.initialized = false;
    }

    /// Advances the sequencer by a single sample and returns the current value
    /// together with the index of the current step.
    ///
    /// `beats_per_sample` is the tempo of the host in beats per sample and
    /// `division_beats` the length of one step.
    pub fn tick(
        &mut self,
        beats_per_sample: f64,
        division_beats: f64,
        values: &[f32; NUM_STEPS],
        smooth: bool,
        sample_rate: f32,
    ) -> (f32, usize) {
        let beats_per_sample = beats_per_sample.max(0.0);
        let division_beats = division_beats.max(1e-9);

        if !self.initialized {
            // The sequencer starts at the first step.
            self.initialized = true;
            self.index = 0;
            self.target = bipolar(values[0]);
            self.value = self.target;
        }

        self.clock_beats += beats_per_sample;
        while self.clock_beats >= division_beats {
            self.clock_beats -= division_beats;
            self.index = (self.index + 1) % NUM_STEPS;
            self.target = bipolar(values[self.index]);
        }

        if smooth {
            let coefficient = self.smoothing_coefficient(sample_rate);
            self.value += (self.target - self.value) * (1.0 - coefficient);
        } else {
            self.value = self.target;
        }

        (self.value, self.index)
    }

    /// The coefficient of the one-pole filter. It is only recomputed when the
    /// sample rate changes.
    fn smoothing_coefficient(&mut self, sample_rate: f32) -> f32 {
        let sample_rate = sample_rate.max(1.0);
        if self.smoothing_rate != sample_rate {
            self.smoothing_rate = sample_rate;
            self.smoothing_coefficient = (-1.0 / (SMOOTHING_TIME * sample_rate)).exp();
        }
        self.smoothing_coefficient
    }
}

/// Turns a step value in the `0..1` range into a bipolar value in the `-1..1`
/// range.
fn bipolar(value: f32) -> f32 {
    value.clamp(0.0, 1.0) * 2.0 - 1.0
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_RATE: f32 = 48_000.0;

    /// The number of samples of one eighth note at 120 BPM.
    fn step_samples() -> usize {
        (SAMPLE_RATE as f64 * 0.5) as usize
    }

    fn beats_per_sample() -> f64 {
        120.0 / (60.0 * SAMPLE_RATE as f64)
    }

    /// Ticks the sequencer and collects the indices of the steps that were
    /// played.
    fn collect_indices(
        step: &mut StepSequencer,
        values: &[f32; 8],
        smooth: bool,
        divisions: usize,
    ) -> Vec<usize> {
        let mut indices = Vec::new();
        let mut last = usize::MAX;
        for _ in 0..(step_samples() * (divisions + 1)) {
            let (_, index) = step.tick(beats_per_sample(), 0.5, values, smooth, SAMPLE_RATE);
            if index != last {
                indices.push(index);
                last = index;
            }
        }
        indices
    }

    #[test]
    fn the_steps_are_played_in_order() {
        let values = [0.0, 0.25, 0.5, 0.75, 1.0, 0.75, 0.5, 0.25];
        let mut step = StepSequencer::new();

        let indices = collect_indices(&mut step, &values, false, 8);

        // The sequencer starts at the first step and wraps around.
        assert!(
            indices.len() >= 9,
            "only {} steps were played",
            indices.len()
        );
        assert_eq!(&indices[..9], &[0, 1, 2, 3, 4, 5, 6, 7, 0]);
    }

    #[test]
    fn the_values_are_bipolar() {
        let values = [0.0, 0.25, 0.5, 0.75, 1.0, 0.75, 0.5, 0.25];
        let mut step = StepSequencer::new();

        for _ in 0..(step_samples() * 8) {
            let (value, _) = step.tick(beats_per_sample(), 0.5, &values, false, SAMPLE_RATE);
            assert!((-1.0..=1.0).contains(&value), "{value} is out of range");
        }
    }

    #[test]
    fn the_stepped_mode_jumps_right_away() {
        let values = [0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0];
        let mut step = StepSequencer::new();

        // The first step is played right away.
        let (value, index) = step.tick(beats_per_sample(), 0.5, &values, false, SAMPLE_RATE);
        assert_eq!(index, 0);
        assert_eq!(value, -1.0);

        // The second step starts at the next division.
        let mut second = None;
        for _ in 0..(step_samples() * 2) {
            let (value, index) = step.tick(beats_per_sample(), 0.5, &values, false, SAMPLE_RATE);
            if index == 1 {
                second = Some(value);
                break;
            }
        }
        assert_eq!(second, Some(1.0));
    }

    #[test]
    fn the_smooth_mode_moves_gradually() {
        let values = [0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0];
        let mut step = StepSequencer::new();

        let (value, _) = step.tick(beats_per_sample(), 0.5, &values, true, SAMPLE_RATE);
        assert_eq!(value, -1.0);

        // Right after the step changes, the value has not moved all the way.
        let mut moved = None;
        for _ in 0..(step_samples() * 2) {
            let (value, index) = step.tick(beats_per_sample(), 0.5, &values, true, SAMPLE_RATE);
            if index == 1 {
                moved = Some(value);
                break;
            }
        }
        let moved = moved.expect("the second step was never reached");
        assert!(
            moved > -1.0 && moved < 1.0,
            "the smoothed value jumped to {moved}"
        );

        // After a while it has arrived at the target.
        for _ in 0..(step_samples() / 4) {
            step.tick(beats_per_sample(), 0.5, &values, true, SAMPLE_RATE);
        }
        let (value, _) = step.tick(beats_per_sample(), 0.5, &values, true, SAMPLE_RATE);
        assert!(value > 0.9, "the smoothed value only reached {value}");
    }

    #[test]
    fn the_reset_starts_over() {
        let values = [0.5, 1.0, 0.0, 0.5, 0.5, 0.5, 0.5, 0.5];
        let mut step = StepSequencer::new();

        for _ in 0..(step_samples() * 2) {
            step.tick(beats_per_sample(), 0.5, &values, false, SAMPLE_RATE);
        }

        step.reset();
        let (value, index) = step.tick(beats_per_sample(), 0.5, &values, false, SAMPLE_RATE);
        assert_eq!(index, 0);
        assert_eq!(value, 0.0);
    }
}
