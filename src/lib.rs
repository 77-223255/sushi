//! Sushi — a polyphonic subtractive synthesizer.
//!
//! The plugin is built on [nice-plug](https://codeberg.org/RustAudio/nice-plug)
//! and exported as CLAP, VST3 and a standalone binary.

use nice_plug::editor::dpi::LogicalSize;
use nice_plug::midi::{Channel, Key, VoiceID};
use nice_plug::prelude::*;
use nice_plug_egui::{
    EguiEditor, EguiEditorState, EguiNiceSettings, RepaintNotifier, create_egui_editor,
};
use std::sync::Arc;

pub mod dsp;
mod editor;
pub mod params;

use dsp::adsr::EnvCoeffs;
use dsp::fx::{Fx, FxParams};
use dsp::lfo::Lfo;
use dsp::voice::{OscSettings, Voice, VoiceSettings};
use editor::{SushiEditor, WINDOW_HEIGHT, WINDOW_WIDTH};
pub use params::{SushiParams, Waveform};

/// Number of simultaneous voices. Steals the oldest voice when exceeded.
const NUM_VOICES: usize = 16;

/// How many samples the editor's oscilloscope can buffer.
const SCOPE_CAPACITY: usize = 16_384;

/// The seeds of the two global LFOs.
const LFO1_SEED: u32 = 0x2545_F491;
const LFO2_SEED: u32 = 0x9E37_79B9;

pub struct Sushi {
    params: Arc<SushiParams>,
    voices: [Option<Voice>; NUM_VOICES],
    next_internal_voice_id: u64,
    sample_rate: f32,

    /// The two global LFOs. Their values are shared by every voice.
    lfo1: Lfo,
    lfo2: Lfo,

    /// The effect chain. All of its buffers are allocated in `activate`.
    fx: Fx,

    editor_state: Arc<EguiEditorState>,
    repaint_notifier: RepaintNotifier,
    scope_tx: rtrb::Producer<f32>,
    scope_rx: Option<rtrb::Consumer<f32>>,
}

impl Default for Sushi {
    fn default() -> Self {
        let (scope_tx, scope_rx) = rtrb::RingBuffer::new(SCOPE_CAPACITY);

        Self {
            params: Arc::new(SushiParams::default()),
            voices: std::array::from_fn(|_| None),
            next_internal_voice_id: 0,
            sample_rate: 48_000.0,

            lfo1: Lfo::new(LFO1_SEED),
            lfo2: Lfo::new(LFO2_SEED),
            fx: Fx::default(),

            editor_state: EguiEditorState::from_size(
                LogicalSize::new(WINDOW_WIDTH, WINDOW_HEIGHT),
                1.0,
            ),
            repaint_notifier: RepaintNotifier::new(),
            scope_tx,
            scope_rx: Some(scope_rx),
        }
    }
}

impl Sushi {
    /// Collects all the settings the voices need. These are rebuilt once per
    /// audio block.
    fn voice_settings(&self) -> VoiceSettings {
        let params = &self.params;
        let sample_rate = self.sample_rate;

        let osc1 = OscSettings {
            waveform: params.osc1_wave.value(),
            pulse_width: params.osc1_pw.value(),
            level: params.osc1_level.value(),
            semitones: (params.osc1_octave.value() * 12 + params.osc1_semi.value()) as f32
                + params.osc1_fine.value() / 100.0,
        };
        let osc2 = OscSettings {
            waveform: params.osc2_wave.value(),
            pulse_width: params.osc2_pw.value(),
            level: params.osc2_level.value(),
            semitones: (params.osc2_octave.value() * 12 + params.osc2_semi.value()) as f32
                + params.osc2_fine.value() / 100.0,
        };

        let amp_env = EnvCoeffs::new(
            params.ae_attack.value(),
            params.ae_decay.value(),
            params.ae_sustain.value(),
            params.ae_release.value(),
            sample_rate,
        );
        let filter_env = EnvCoeffs::new(
            params.fe_attack.value(),
            params.fe_decay.value(),
            params.fe_sustain.value(),
            params.fe_release.value(),
            sample_rate,
        );

        // The glide is a one-pole filter: the bigger the time, the slower the
        // pitch moves towards the target.
        let glide_ms = params.glide.value();
        let glide_step = if glide_ms <= 0.0 {
            1.0
        } else {
            1.0 - (-1.0 / (glide_ms * 0.001 * sample_rate)).exp()
        };

        VoiceSettings {
            osc1,
            osc2,
            noise_level: params.noise_level.value(),
            filter_mode: params.filter_mode.value(),
            resonance: params.resonance.value(),
            filter_env_octaves: params.filter_env_amount.value() * 5.0,
            filter_keytrack: params.filter_keytrack.value(),
            filter_drive: params.filter_drive.value(),
            amp_env,
            filter_env,
            unison: params
                .unison
                .value()
                .clamp(1, dsp::voice::MAX_UNISON as i32) as usize,
            detune_cents: params.detune.value(),
            spread: params.spread.value(),
            glide_step,
            lfo1_amount: params.lfo1_amount.value(),
            lfo1_target: params.lfo1_target.value(),
            lfo2_amount: params.lfo2_amount.value(),
            lfo2_target: params.lfo2_target.value(),
        }
    }

    /// Collects the settings of the effect chain.
    fn fx_params(&self) -> FxParams {
        let params = &self.params;
        FxParams {
            chorus_mix: params.chorus_mix.value(),
            chorus_rate: params.chorus_rate.value(),
            chorus_depth: params.chorus_depth.value(),
            delay_time_s: params.delay_time.value() * 0.001,
            delay_feedback: params.delay_feedback.value(),
            delay_mix: params.delay_mix.value(),
            reverb_mix: params.reverb_mix.value(),
            reverb_decay: params.reverb_decay.value(),
        }
    }

    /// The index of the most recently started voice.
    fn newest_voice_index(&self) -> Option<usize> {
        self.voices
            .iter()
            .enumerate()
            .filter_map(|(index, voice)| {
                voice
                    .as_ref()
                    .map(|voice| (index, voice.internal_voice_id()))
            })
            .max_by_key(|(_, internal_voice_id)| *internal_voice_id)
            .map(|(index, _)| index)
    }

    /// Starts a new voice, or reuses an existing one in mono and legato mode.
    #[allow(clippy::too_many_arguments)]
    fn start_voice(
        &mut self,
        context: &mut impl ProcessContext<Self>,
        sample_offset: u32,
        voice_id: VoiceID,
        channel: Channel,
        key: Key,
        velocity: f32,
        settings: &VoiceSettings,
    ) {
        // The voices that have already died don't need a slot anymore.
        self.free_finished_voices(context, sample_offset);

        match self.params.poly_mode.value() {
            params::PolyMode::Poly => {
                // Use a free voice, or steal the oldest one.
                let voice_idx = self.voice_slot_to_use();

                // Tell the host about the stolen voice so it can recycle the
                // voice's modulation resources.
                if let Some(stolen) = self.voices[voice_idx].take() {
                    terminate_voice(context, sample_offset, &stolen);
                }

                self.fill_voice_slot(voice_idx, voice_id, channel, key, velocity, settings);
            }
            params::PolyMode::Mono => {
                // A mono synth only ever has a single voice.
                if let Some(index) = self.newest_voice_index()
                    && let Some(voice) = self.voices[index].as_mut()
                {
                    // The voice is reused for the new note, so the host is told
                    // that the old note is over.
                    terminate_voice(context, sample_offset, voice);
                    voice.set_voice_id(voice_id.id_or_fallback(key, channel));
                    voice.set_key(key, settings, true);
                    return;
                }

                self.new_voice(
                    context,
                    sample_offset,
                    voice_id,
                    channel,
                    key,
                    velocity,
                    settings,
                );
            }
            params::PolyMode::Legato => {
                if let Some(index) = self.newest_voice_index() {
                    let retrigger = self.voices[index]
                        .as_ref()
                        .map(|voice| voice.is_releasing())
                        .unwrap_or(false);
                    if let Some(voice) = self.voices[index].as_mut() {
                        // The voice is reused for the new note, so the host is
                        // told that the old note is over.
                        terminate_voice(context, sample_offset, voice);
                        voice.set_voice_id(voice_id.id_or_fallback(key, channel));
                        voice.set_key(key, settings, retrigger);
                    }
                    return;
                }

                self.new_voice(
                    context,
                    sample_offset,
                    voice_id,
                    channel,
                    key,
                    velocity,
                    settings,
                );
            }
        }
    }

    /// The slot a new voice should use: a free one, or the oldest voice when
    /// all of them are in use.
    fn voice_slot_to_use(&self) -> usize {
        match self.voices.iter().position(|voice| voice.is_none()) {
            Some(free_idx) => free_idx,
            None => self
                .voices
                .iter()
                .enumerate()
                .min_by_key(|(_, voice)| {
                    voice
                        .as_ref()
                        .map(|voice| voice.internal_voice_id())
                        .unwrap_or(u64::MAX)
                })
                .map(|(index, _)| index)
                .unwrap_or(0),
        }
    }

    /// Frees the voices that have already died, so that they don't take up a
    /// slot of the voice pool.
    fn free_finished_voices(
        &mut self,
        context: &mut impl ProcessContext<Self>,
        sample_offset: u32,
    ) {
        for slot in self.voices.iter_mut() {
            let finished = slot.as_ref().is_some_and(|voice| voice.is_finished());
            if !finished {
                continue;
            }

            if let Some(voice) = slot.take() {
                terminate_voice(context, sample_offset, &voice);
            }
        }
    }

    /// Starts a brand new voice. When every slot is in use, the oldest voice
    /// is stolen.
    #[allow(clippy::too_many_arguments)]
    fn new_voice(
        &mut self,
        context: &mut impl ProcessContext<Self>,
        sample_offset: u32,
        voice_id: VoiceID,
        channel: Channel,
        key: Key,
        velocity: f32,
        settings: &VoiceSettings,
    ) {
        let voice_idx = self.voice_slot_to_use();

        // Tell the host about the stolen voice so it can recycle the voice's
        // modulation resources.
        if let Some(stolen) = self.voices[voice_idx].take() {
            terminate_voice(context, sample_offset, &stolen);
        }

        self.fill_voice_slot(voice_idx, voice_id, channel, key, velocity, settings);
    }

    /// Puts a brand new voice into the given slot.
    #[allow(clippy::too_many_arguments)]
    fn fill_voice_slot(
        &mut self,
        voice_idx: usize,
        voice_id: VoiceID,
        channel: Channel,
        key: Key,
        velocity: f32,
        settings: &VoiceSettings,
    ) {
        let internal_voice_id = self.next_internal_voice_id;
        self.next_internal_voice_id = self.next_internal_voice_id.wrapping_add(1);

        self.voices[voice_idx] = Some(Voice::new(
            voice_id.id_or_fallback(key, channel),
            channel,
            key,
            internal_voice_id,
            velocity,
            settings,
        ));
    }

    /// Puts every matching voice into its release stage.
    fn release_voices(&mut self, voice_id: VoiceID, channel: Channel, key: Key) {
        for voice in self.voices.iter_mut().flatten() {
            let matches = voice_id == VoiceID::ID(voice.voice_id())
                || (voice.channel() == channel && voice.key() == key);
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
                voice_id == VoiceID::ID(voice.voice_id())
                    || (voice.channel() == channel && voice.key() == key)
            });
            if !matches {
                continue;
            }

            if let Some(voice) = slot.take() {
                let _ = context.try_send_event(NoteEvent::VoiceTerminated {
                    timing: sample_offset,
                    voice_id: VoiceID::ID(voice.voice_id()),
                    channel: voice.channel(),
                    key: voice.key(),
                });
            }

            if !voice_id.is_wildcard() {
                return;
            }
        }
    }
}

/// Tells the host that a voice has ended, so that it can recycle the voice's
/// modulation resources.
fn terminate_voice(context: &mut impl ProcessContext<Sushi>, sample_offset: u32, voice: &Voice) {
    let _ = context.try_send_event(NoteEvent::VoiceTerminated {
        timing: sample_offset,
        voice_id: VoiceID::ID(voice.voice_id()),
        channel: voice.channel(),
        key: voice.key(),
    });
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

    type Editor = EguiEditor<SushiEditor>;
    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Self::Editor> {
        let scope_rx = self.scope_rx.take()?;

        create_egui_editor(
            self.editor_state.clone(),
            self.repaint_notifier.clone(),
            EguiNiceSettings::new(),
            SushiEditor::new(self.params.clone(), scope_rx),
        )
    }

    fn activate(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl ActivateContext<Self>,
    ) -> bool {
        self.sample_rate = buffer_config.sample_rate;
        // The effect chain allocates all of its delay lines here, so that the
        // audio thread never has to allocate.
        self.fx = Fx::new(self.sample_rate);
        true
    }

    fn reset(&mut self) {
        for voice in self.voices.iter_mut() {
            *voice = None;
        }
        self.next_internal_voice_id = 0;
        self.lfo1 = Lfo::new(LFO1_SEED);
        self.lfo2 = Lfo::new(LFO2_SEED);
        self.fx.reset();
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

        let sample_rate = self.sample_rate;
        let mut settings = self.voice_settings();
        let mut fx_params = self.fx_params();
        // The output channels are looked up once, so that the sample loop
        // doesn't have to iterate over all of them for every single sample.
        let output = buffer.as_slice();
        let mut channels = output.iter_mut();
        let mut left_channel = channels.next().map(|channel| &mut **channel);
        let mut right_channel = channels.next().map(|channel| &mut **channel);

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
                        self.start_voice(
                            context, timing, voice_id, channel, key, velocity, &settings,
                        );
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

            // The smoothed parameters.
            let volume = self.params.volume.smoothed.next();
            let cutoff = self.params.cutoff.smoothed.next();
            settings.resonance = self.params.resonance.smoothed.next();
            settings.filter_env_octaves = self.params.filter_env_amount.smoothed.next() * 5.0;
            settings.filter_keytrack = self.params.filter_keytrack.smoothed.next();
            fx_params.chorus_mix = self.params.chorus_mix.smoothed.next();
            fx_params.chorus_rate = self.params.chorus_rate.smoothed.next();
            fx_params.chorus_depth = self.params.chorus_depth.smoothed.next();
            fx_params.delay_time_s = self.params.delay_time.smoothed.next() * 0.001;
            fx_params.delay_feedback = self.params.delay_feedback.smoothed.next();
            fx_params.delay_mix = self.params.delay_mix.smoothed.next();
            fx_params.reverb_mix = self.params.reverb_mix.smoothed.next();
            fx_params.reverb_decay = self.params.reverb_decay.smoothed.next();

            // The global LFOs.
            let lfo1 = self.lfo1.next(
                self.params.lfo1_rate.smoothed.next(),
                sample_rate,
                self.params.lfo1_wave.value(),
            );
            let lfo2 = self.lfo2.next(
                self.params.lfo2_rate.smoothed.next(),
                sample_rate,
                self.params.lfo2_wave.value(),
            );

            // All the voices.
            let mut left = 0.0;
            let mut right = 0.0;
            for slot in self.voices.iter_mut() {
                let Some(voice) = slot else { continue };

                if voice.is_finished() {
                    let _ = context.try_send_event(NoteEvent::VoiceTerminated {
                        timing: sample_idx as u32,
                        voice_id: VoiceID::ID(voice.voice_id()),
                        channel: voice.channel(),
                        key: voice.key(),
                    });
                    *slot = None;
                    continue;
                }

                let (voice_left, voice_right) =
                    voice.next_sample(&settings, cutoff, lfo1, lfo2, sample_rate);
                left += voice_left;
                right += voice_right;
            }

            // The master volume and the effect chain.
            left *= volume;
            right *= volume;
            let (left, right) = self.fx.process(left, right, &fx_params);

            // Gentle saturation keeps stacked voices from clipping harshly.
            let left = left.tanh();
            let right = right.tanh();

            // Feed the editor's oscilloscope, but only while the editor is open.
            if self.editor_state.is_open() {
                let _ = self.scope_tx.push((left + right) * 0.5);
            }

            // The output.
            if let Some(channel) = left_channel.as_deref_mut()
                && let Some(sample) = channel.get_mut(sample_idx)
            {
                *sample = left;
            }
            if let Some(channel) = right_channel.as_deref_mut()
                && let Some(sample) = channel.get_mut(sample_idx)
            {
                *sample = right;
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
