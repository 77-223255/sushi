//! Sushi — a polyphonic subtractive synthesizer.
//!
//! The plugin is built on [nice-plug](https://codeberg.org/RustAudio/nice-plug)
//! and exported as CLAP, VST3 and a standalone binary.

use nice_plug::midi::{Channel, Key, VoiceID};
use nice_plug::prelude::*;
use std::f32::consts::PI;
use std::sync::Arc;

pub mod dsp;
pub mod params;

use dsp::adsr::EnvCoeffs;
use dsp::voice::Voice;
pub use params::{SushiParams, Waveform};

/// Number of simultaneous voices. Steals the oldest voice when exceeded.
const NUM_VOICES: usize = 16;

pub struct Sushi {
    params: Arc<SushiParams>,
    voices: [Option<Voice>; NUM_VOICES],
    next_internal_voice_id: u64,
    sample_rate: f32,
}

impl Default for Sushi {
    fn default() -> Self {
        Self {
            params: Arc::new(SushiParams::default()),
            voices: std::array::from_fn(|_| None),
            next_internal_voice_id: 0,
            sample_rate: 48_000.0,
        }
    }
}

impl Sushi {
    /// Starts a new voice, stealing the oldest one if all voices are in use.
    fn start_voice(
        &mut self,
        context: &mut impl ProcessContext<Self>,
        sample_offset: u32,
        voice_id: VoiceID,
        channel: Channel,
        key: Key,
        velocity: f32,
    ) {
        let internal_voice_id = self.next_internal_voice_id;
        self.next_internal_voice_id = self.next_internal_voice_id.wrapping_add(1);

        let new_voice = Voice::new(
            voice_id.id_or_fallback(key, channel),
            channel,
            key,
            internal_voice_id,
            velocity,
            self.sample_rate,
        );

        let voice_idx = match self.voices.iter().position(|voice| voice.is_none()) {
            Some(free_idx) => free_idx,
            None => self
                .voices
                .iter()
                .enumerate()
                .min_by_key(|(_, voice)| {
                    voice
                        .as_ref()
                        .map(|voice| voice.internal_voice_id)
                        .unwrap_or(u64::MAX)
                })
                .map(|(idx, _)| idx)
                .unwrap_or(0),
        };

        // Tell the host about the stolen voice so it can recycle the voice's
        // modulation resources.
        if let Some(stolen) = self.voices[voice_idx].take() {
            let _ = context.try_send_event(NoteEvent::VoiceTerminated {
                timing: sample_offset,
                voice_id: VoiceID::ID(stolen.voice_id),
                channel: stolen.channel,
                key: stolen.key,
            });
        }

        self.voices[voice_idx] = Some(new_voice);
    }

    /// Puts every matching voice into its release stage.
    fn release_voices(&mut self, voice_id: VoiceID, channel: Channel, key: Key) {
        for voice in self.voices.iter_mut().flatten() {
            let matches = voice_id == VoiceID::ID(voice.voice_id)
                || (voice.channel == channel && voice.key == key);
            if matches {
                voice.release();

                // A wildcard voice ID targets every matching note (e.g. all
                // layered voices for the same key).
                if !voice_id.is_wildcard() {
                    return;
                }
            }
        }
    }

    /// Immediately kills the matching voices.
    fn choke_voices(
        &mut self,
        context: &mut impl ProcessContext<Self>,
        sample_offset: u32,
        voice_id: VoiceID,
        channel: Channel,
        key: Key,
    ) {
        for slot in self.voices.iter_mut() {
            let matches = slot.as_ref().is_some_and(|voice| {
                voice_id == VoiceID::ID(voice.voice_id)
                    || (voice.channel == channel && voice.key == key)
            });
            if !matches {
                continue;
            }

            if let Some(voice) = slot.take() {
                let _ = context.try_send_event(NoteEvent::VoiceTerminated {
                    timing: sample_offset,
                    voice_id: VoiceID::ID(voice.voice_id),
                    channel: voice.channel,
                    key: voice.key,
                });
            }

            if !voice_id.is_wildcard() {
                return;
            }
        }
    }
}

impl Plugin for Sushi {
    const NAME: &'static str = "Sushi";
    const VENDOR: &'static str = "shienze";
    const URL: &'static str = "https://github.com/77-223255/sushi";
    const EMAIL: &'static str = "shienze2005@gmail.com";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: None,
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];

    const MIDI_INPUT: MidiConfig = MidiConfig::Basic;
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    type Editor = ();
    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn activate(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl ActivateContext<Self>,
    ) -> bool {
        self.sample_rate = buffer_config.sample_rate;
        true
    }

    fn reset(&mut self) {
        for voice in self.voices.iter_mut() {
            *voice = None;
        }
        self.next_internal_voice_id = 0;
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let num_samples = buffer.samples();
        if num_samples == 0 {
            return ProcessStatus::Normal;
        }

        // Envelope coefficients are computed once per block: cheap to update and
        // far too expensive to calculate for every sample.
        let env_coeffs = EnvCoeffs::new(
            self.params.attack.value(),
            self.params.decay.value(),
            self.params.sustain.value(),
            self.params.release.value(),
            self.sample_rate,
        );
        let waveform = self.params.waveform.value();
        let output = buffer.as_slice();

        let mut next_event = context.next_event();
        for sample_idx in 0..num_samples {
            // An event is applied at the sample it belongs to.
            while let Some(event) = next_event {
                if event.timing() as usize > sample_idx {
                    break;
                }

                match event {
                    NoteEvent::NoteOn {
                        timing,
                        voice_id,
                        channel,
                        key,
                        velocity,
                    } => {
                        self.start_voice(context, timing, voice_id, channel, key, velocity);
                    }
                    NoteEvent::NoteOff {
                        voice_id,
                        channel,
                        key,
                        ..
                    } => self.release_voices(voice_id, channel, key),
                    NoteEvent::Choke {
                        timing,
                        voice_id,
                        channel,
                        key,
                    } => self.choke_voices(context, timing, voice_id, channel, key),
                    _ => (),
                }

                next_event = context.next_event();
            }

            let gain = self.params.gain.smoothed.next();
            let cutoff = self
                .params
                .cutoff
                .smoothed
                .next()
                .clamp(20.0, self.sample_rate * 0.45);
            let resonance = self.params.resonance.smoothed.next();

            let g = (PI * cutoff / self.sample_rate).tan();
            let k = 2.0 - 1.98 * resonance;

            let mut mix = 0.0;
            for slot in self.voices.iter_mut() {
                let Some(voice) = slot else { continue };

                if voice.is_finished() {
                    let _ = context.try_send_event(NoteEvent::VoiceTerminated {
                        timing: sample_idx as u32,
                        voice_id: VoiceID::ID(voice.voice_id),
                        channel: voice.channel,
                        key: voice.key,
                    });
                    *slot = None;
                    continue;
                }

                mix += voice.next_sample(waveform, g, k, &env_coeffs);
            }

            // Gentle saturation keeps stacked voices from clipping harshly.
            let out = (mix * gain).tanh();
            for channel in output.iter_mut() {
                if let Some(sample) = channel.get_mut(sample_idx) {
                    *sample = out;
                }
            }
        }

        ProcessStatus::Normal
    }
}

impl ClapPlugin for Sushi {
    const CLAP_ID: &'static str = "com.shienze.sushi";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("A polyphonic subtractive synthesizer");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Synthesizer,
        ClapFeature::Stereo,
    ];
}

impl Vst3Plugin for Sushi {
    const VST3_CLASS_ID: [u8; 16] = *b"SushiSynth000001";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[
        Vst3SubCategory::Instrument,
        Vst3SubCategory::Synth,
        Vst3SubCategory::Stereo,
    ];
}

nice_export_clap!(Sushi);
nice_export_vst3!(Sushi);
