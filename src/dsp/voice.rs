use nice_plug::midi::{Channel, Key};
use nice_plug::util;

use super::adsr::{Adsr, EnvCoeffs};
use super::filter::Svf;
use super::osc::Oscillator;
use crate::params::Waveform;

/// One synth voice: an oscillator, an amplitude envelope and a filter.
pub struct Voice {
    /// The voice ID provided by the host, or a fallback derived from the note.
    pub voice_id: i32,
    pub channel: Channel,
    pub key: Key,
    /// Monotonically increasing ID used to steal the oldest voice.
    pub internal_voice_id: u64,
    pub releasing: bool,

    /// Square root of the note velocity, used as an amplitude multiplier.
    velocity: f32,
    phase_delta: f32,
    osc: Oscillator,
    env: Adsr,
    filter: Svf,
}

impl Voice {
    pub fn new(
        voice_id: i32,
        channel: Channel,
        key: Key,
        internal_voice_id: u64,
        velocity: f32,
        sample_rate: f32,
    ) -> Self {
        let frequency = util::midi_key_to_freq(key.number_or_middle_c());

        let mut env = Adsr::default();
        env.gate_on();

        Self {
            voice_id,
            channel,
            key,
            internal_voice_id,
            releasing: false,
            velocity: velocity.clamp(0.0, 1.0).sqrt(),
            phase_delta: (frequency / sample_rate).clamp(0.0, 0.45),
            osc: Oscillator::new(),
            env,
            filter: Svf::new(),
        }
    }

    pub fn release(&mut self) {
        self.releasing = true;
        self.env.gate_off();
    }

    #[inline]
    pub fn is_finished(&self) -> bool {
        self.env.is_finished()
    }

    /// Renders a single sample.
    #[inline]
    pub fn next_sample(
        &mut self,
        waveform: Waveform,
        g: f32,
        k: f32,
        env_coeffs: &EnvCoeffs,
    ) -> f32 {
        let env = self.env.next(env_coeffs);
        let raw = self.osc.next(self.phase_delta, waveform);
        let filtered = self.filter.lowpass(raw, g, k);

        filtered * env * self.velocity
    }
}
