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
use std::sync::atomic::{AtomicUsize, Ordering};

pub mod dsp;
mod editor;
pub mod params;

use dsp::adsr::EnvCoeffs;
use dsp::arp::Arp;
use dsp::fx::{Fx, FxParams};
use dsp::lfo::Lfo;
use dsp::step::StepSequencer;
use dsp::voice::{OscSettings, Voice, VoiceSettings};
use editor::{SushiEditor, WINDOW_HEIGHT, WINDOW_WIDTH};
use params::Character;
pub use params::{SushiParams, Waveform};

/// Number of simultaneous voices. Steals the oldest voice when exceeded.
const NUM_VOICES: usize = 16;

/// How many samples the editor's oscilloscope can buffer.
const SCOPE_CAPACITY: usize = 16_384;

/// The seeds of the two global LFOs.
const LFO1_SEED: u32 = 0x2545_F491;
const LFO2_SEED: u32 = 0x9E37_79B9;

/// The ID of the first voice that the arpeggiator plays. The IDs of the host
/// are usually small numbers, so a high one keeps them apart.
const FIRST_ARP_VOICE_ID: i32 = 0x0100_0000;

/// The frequency of the tone tilt filter, in Hz.
const TONE_TILT_FREQUENCY: f32 = 700.0;

/// Values that are smaller than this are flushed to zero, so that they do not
/// linger in the denormal range.
const DENORMAL_LIMIT: f32 = 1e-20;

/// Flushes a value that is too small to be heard to exactly zero.
fn flush_denormal(value: f32) -> f32 {
    if value.abs() < DENORMAL_LIMIT {
        0.0
    } else {
        value
    }
}

/// The saturation of a single sample. The two halves of the wave are driven
/// with different gains, so that the harsh character is a little asymmetric
/// without adding a DC offset to the signal.
fn saturate_sample(sample: f32, drive: f32, negative_gain: f32) -> f32 {
    if sample >= 0.0 {
        (sample * drive).tanh()
    } else {
        (sample * drive * negative_gain).tanh()
    }
}

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

    /// The arpeggiator.
    arp: Arp,
    /// The step sequencer.
    step: StepSequencer,
    /// The index of the step that is currently playing.
    step_index: Arc<AtomicUsize>,
    /// The number of arpeggiator triggers. The GUI uses it to flash a light.
    arp_led: Arc<AtomicUsize>,
    /// The number of voices that are currently playing. The GUI shows it in a
    /// small meter of the header.
    voice_count: Arc<AtomicUsize>,
    /// The voice that the arpeggiator is currently playing.
    arp_voice_id: Option<i32>,
    /// The ID of the next voice of the arpeggiator.
    next_arp_voice_id: i32,
    /// Whether the arpeggiator was enabled in the last block.
    arp_was_on: bool,
    /// The state of the two tone tilt filters.
    tone_left: f32,
    tone_right: f32,

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

            arp: Arp::new(),
            step: StepSequencer::new(),
            step_index: Arc::new(AtomicUsize::new(0)),
            arp_led: Arc::new(AtomicUsize::new(0)),
            voice_count: Arc::new(AtomicUsize::new(0)),
            arp_voice_id: None,
            next_arp_voice_id: FIRST_ARP_VOICE_ID,
            arp_was_on: false,
            tone_left: 0.0,
            tone_right: 0.0,

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
    fn voice_settings(&self, steps: u32) -> VoiceSettings {
        let params = &self.params;
        let sample_rate = self.sample_rate.max(1.0);

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
            velocity_amount: params.velocity.smoothed.next_step(steps),
            drift_amount: 0.0,
            step_dest: params.step_dest.value(),
            step_amount: params.step_amount.smoothed.next_step(steps),
            character_grit: false,
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

    /// The index of the step that is currently playing. The editor uses it to
    /// highlight the step of the sequencer.
    pub fn step_index(&self) -> Arc<AtomicUsize> {
        self.step_index.clone()
    }

    /// The number of arpeggiator triggers. The editor uses it to flash a
    /// light whenever the arpeggiator plays a note.
    pub fn arp_led(&self) -> Arc<AtomicUsize> {
        self.arp_led.clone()
    }

    /// The number of voices that are currently playing. The editor uses it to
    /// show the meter of the header.
    pub fn voice_count(&self) -> Arc<AtomicUsize> {
        self.voice_count.clone()
    }

    /// Puts the voice of the arpeggiator into its release stage.
    fn release_arp_voice(&mut self) {
        let Some(voice_id) = self.arp_voice_id.take() else {
            return;
        };

        for slot in self.voices.iter_mut() {
            let matches = slot
                .as_ref()
                .is_some_and(|voice| voice.voice_id() == voice_id);
            if !matches {
                continue;
            }

            if let Some(voice) = slot.as_mut() {
                voice.release();
            }
            return;
        }
    }

    /// Starts a new voice for the arpeggiator. The voice that the arpeggiator
    /// was playing before is released.
    fn start_arp_voice(
        &mut self,
        context: &mut impl ProcessContext<Self>,
        sample_offset: u32,
        key: u8,
        velocity: f32,
        settings: &VoiceSettings,
    ) {
        self.release_arp_voice();
        self.free_finished_voices(context, sample_offset);

        let voice_idx = self.voice_slot_to_use();
        if let Some(stolen) = self.voices[voice_idx].take() {
            terminate_voice(context, sample_offset, &stolen);
        }

        let voice_id = self.next_arp_voice_id;
        self.next_arp_voice_id = self.next_arp_voice_id.wrapping_add(1);
        self.arp_voice_id = Some(voice_id);
        self.arp_led.fetch_add(1, Ordering::Relaxed);

        let internal_voice_id = self.next_internal_voice_id;
        self.next_internal_voice_id = self.next_internal_voice_id.wrapping_add(1);

        self.voices[voice_idx] = Some(Voice::new(
            voice_id,
            Channel::Number(0),
            Key::Number(key),
            internal_voice_id,
            velocity,
            settings,
        ));
    }

    /// The saturation of the character. The silk character is gentle, the grit
    /// character is harsh and a little asymmetric.
    ///
    /// The asymmetry only changes the gain of the two halves of the wave, so
    /// the signal does not get a DC offset and silence stays exactly silent.
    fn saturate(&self, left: f32, right: f32, grit: bool) -> (f32, f32) {
        let amount = if grit { 0.30 } else { 0.10 };
        let drive = 1.0 + 2.0 * amount;
        let normalizer = 1.0 / (1.0 + amount);
        // The harsh character drives the positive half of the wave harder than
        // the negative one.
        let negative_gain = if grit { 1.0 - 0.6 * amount } else { 1.0 };

        (
            flush_denormal(saturate_sample(left, drive, negative_gain) * normalizer),
            flush_denormal(saturate_sample(right, drive, negative_gain) * normalizer),
        )
    }

    /// A one-pole lowpass that splits the signal into a low and a high band.
    /// The low band is boosted and the high band attenuated, or the other way
    /// around.
    fn tone_tilt(
        &mut self,
        left: f32,
        right: f32,
        low_gain: f32,
        high_gain: f32,
        coefficient: f32,
    ) -> (f32, f32) {
        self.tone_left += (left - self.tone_left) * coefficient;
        self.tone_right += (right - self.tone_right) * coefficient;

        // The state of the filter is flushed to zero, so that it does not
        // linger in the denormal range.
        self.tone_left = flush_denormal(self.tone_left);
        self.tone_right = flush_denormal(self.tone_right);

        (
            self.tone_left * low_gain + (left - self.tone_left) * high_gain,
            self.tone_right * low_gain + (right - self.tone_right) * high_gain,
        )
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

    /// Puts every voice that does not belong to the arpeggiator into its
    /// release stage.
    ///
    /// The arpeggiator takes over the note offs of the host, so the voices of
    /// the host have to be released when it is switched on. Otherwise they
    /// would stay stuck forever.
    fn release_normal_voices(&mut self) {
        let arp_voice_id = self.arp_voice_id;
        for voice in self.voices.iter_mut().flatten() {
            if arp_voice_id == Some(voice.voice_id()) {
                continue;
            }
            voice.release();
        }
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
            SushiEditor::new(
                self.params.clone(),
                scope_rx,
                self.step_index.clone(),
                self.arp_led.clone(),
                self.voice_count.clone(),
            ),
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
        self.arp.reset();
        self.step.reset();
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
        self.arp.reset();
        self.step.reset();
        self.arp_voice_id = None;
        self.arp_was_on = false;
        self.tone_left = 0.0;
        self.tone_right = 0.0;
        self.step_index.store(0, Ordering::Relaxed);
        self.arp_led.store(0, Ordering::Relaxed);
        self.voice_count.store(0, Ordering::Relaxed);
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

        // A sample rate of zero would make the clock of the arpeggiator spin
        // forever, so it is clamped to a sane value.
        let sample_rate = self.sample_rate.max(1.0);
        let steps = num_samples as u32;

        // The transport of the host, which runs the arpeggiator and the step
        // sequencer.
        let bpm = context.transport().tempo.unwrap_or(120.0);
        let beats_per_sample = bpm / (60.0 * sample_rate as f64);

        // The macros, read once per block.
        let bright = self.params.macro_bright.smoothed.next_step(steps);
        let motion = self.params.macro_motion.smoothed.next_step(steps);
        let space = self.params.macro_space.smoothed.next_step(steps);
        let grit = self.params.character.value() == Character::Grit;

        let mut settings = self.voice_settings(steps);
        let mut fx_params = self.fx_params();

        // The brightness macro scales the cutoff and the resonance.
        let cutoff_multiplier = 2f32.powf((bright - 0.5) * 4.0);
        let resonance_offset = (bright - 0.5) * 0.2;

        // The motion macro adds to the LFO amounts and to the drift.
        settings.lfo1_amount = (settings.lfo1_amount + motion * 0.3).clamp(0.0, 1.0);
        settings.lfo2_amount = (settings.lfo2_amount + motion * 0.3).clamp(0.0, 1.0);
        // The grit character drifts twice as much as the silk character, which
        // drifts half as much.
        let character_drift = if grit { 2.0 } else { 0.5 };
        settings.drift_amount =
            (self.params.drift.smoothed.next_step(steps) * (0.5 + motion) * character_drift)
                .clamp(0.0, 1.0);
        // The harsh character adds a bit of drive to the filter.
        settings.character_grit = grit;

        // The tone tilt, whose coefficients are computed once per block.
        let tone = self.params.tone.smoothed.next_step(steps);
        let low_gain = 10f32.powf(tone * 6.0 / 20.0);
        let high_gain = 10f32.powf(-tone * 6.0 / 20.0);
        let tilt_coefficient =
            1.0 - (-2.0 * std::f32::consts::PI * TONE_TILT_FREQUENCY / sample_rate).exp();

        // The arpeggiator.
        let arp_on = self.params.arp_on.value();
        let arp_latch = self.params.arp_latch.value();
        let arp_pattern = self.params.arp_pattern.value();
        let arp_division = self.params.arp_rate.value().beats();
        let arp_octaves = self.params.arp_octaves.value().clamp(1, 3) as u8;
        let arp_gate = self.params.arp_gate.value();

        if arp_on != self.arp_was_on {
            if arp_on {
                // The arpeggiator was switched on, so the voices of the host
                // are released. From now on the note offs of the host only go
                // to the arpeggiator, so the voices would stay stuck forever.
                self.release_normal_voices();
            } else {
                // The arpeggiator was switched off, so its voice goes away.
                self.release_arp_voice();
                self.arp.reset();
            }
            self.arp_was_on = arp_on;
        }

        // The step sequencer.
        let step_on = self.params.step_on.value();
        let step_division = self.params.step_rate.value().beats();
        let step_smooth = self.params.step_smooth.value();
        let step_amount = settings.step_amount;
        let step_values = [
            self.params.step_1.smoothed.next_step(steps),
            self.params.step_2.smoothed.next_step(steps),
            self.params.step_3.smoothed.next_step(steps),
            self.params.step_4.smoothed.next_step(steps),
            self.params.step_5.smoothed.next_step(steps),
            self.params.step_6.smoothed.next_step(steps),
            self.params.step_7.smoothed.next_step(steps),
            self.params.step_8.smoothed.next_step(steps),
        ];

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
                        if arp_on {
                            // The arpeggiator collects the notes, the voices
                            // are started by the arpeggiator itself.
                            self.arp
                                .note_on(key.number_or_middle_c(), velocity, arp_latch);
                        } else {
                            self.start_voice(
                                context, timing, voice_id, channel, key, velocity, &settings,
                            );
                        }
                    }
                    NoteEvent::NoteOff {
                        voice_id,
                        channel,
                        key,
                        ..
                    } => {
                        if arp_on {
                            self.arp.note_off(key.number_or_middle_c());
                        } else {
                            self.release_voices(voice_id, channel, key);
                        }
                    }
                    NoteEvent::Choke {
                        timing,
                        voice_id,
                        channel,
                        key,
                    } => {
                        if arp_on {
                            self.release_arp_voice();
                            self.arp.reset();
                        } else {
                            self.choke_voices(context, timing, voice_id, channel, key);
                        }
                    }
                    _ => (),
                }

                next_event = context.next_event();
            }

            // The smoothed parameters.
            let volume = self.params.volume.smoothed.next();
            let cutoff = self.params.cutoff.smoothed.next() * cutoff_multiplier;
            settings.resonance =
                (self.params.resonance.smoothed.next() + resonance_offset).clamp(0.0, 1.0);
            settings.filter_env_octaves = self.params.filter_env_amount.smoothed.next() * 5.0;
            settings.filter_keytrack = self.params.filter_keytrack.smoothed.next();
            fx_params.chorus_mix = self.params.chorus_mix.smoothed.next() + space * 0.35;
            fx_params.chorus_rate = self.params.chorus_rate.smoothed.next();
            fx_params.chorus_depth = self.params.chorus_depth.smoothed.next();
            fx_params.delay_time_s = self.params.delay_time.smoothed.next() * 0.001;
            fx_params.delay_feedback = self.params.delay_feedback.smoothed.next();
            fx_params.delay_mix = self.params.delay_mix.smoothed.next() + space * 0.35;
            fx_params.reverb_mix = self.params.reverb_mix.smoothed.next() + space * 0.35;
            fx_params.reverb_decay = self.params.reverb_decay.smoothed.next();
            fx_params.chorus_mix = fx_params.chorus_mix.clamp(0.0, 1.0);
            fx_params.delay_mix = fx_params.delay_mix.clamp(0.0, 1.0);
            fx_params.reverb_mix = fx_params.reverb_mix.clamp(0.0, 1.0);

            // The arpeggiator plays the next note of its pattern.
            if arp_on {
                let tick = self.arp.tick(
                    beats_per_sample,
                    arp_division,
                    arp_gate,
                    arp_pattern,
                    arp_octaves,
                    sample_rate,
                );
                if tick.release {
                    self.release_arp_voice();
                }
                if let Some((key, velocity)) = tick.trigger {
                    self.start_arp_voice(context, sample_idx as u32, key, velocity, &settings);
                }
            }

            // The step sequencer.
            let mut step_value = 0.0;
            if step_on {
                let (value, index) = self.step.tick(
                    beats_per_sample,
                    step_division,
                    &step_values,
                    step_smooth,
                    sample_rate,
                );
                step_value = value * step_amount;
                if self.step_index.load(Ordering::Relaxed) != index {
                    self.step_index.store(index, Ordering::Relaxed);
                }
            }

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
                    voice.next_sample(&settings, cutoff, lfo1, lfo2, step_value, sample_rate);
                left += voice_left;
                right += voice_right;
            }

            // The character saturator, the effect chain and the tone tilt.
            let (left, right) = self.saturate(left, right, grit);
            let (left, right) = self.fx.process(left, right, &fx_params);
            let (mut left, mut right) =
                self.tone_tilt(left, right, low_gain, high_gain, tilt_coefficient);

            // The master volume.
            left *= volume;
            right *= volume;

            // Gentle saturation keeps stacked voices from clipping harshly.
            let left = flush_denormal(left.tanh());
            let right = flush_denormal(right.tanh());

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

        // The number of voices that are playing, for the meter of the editor.
        self.voice_count
            .store(self.voices.iter().flatten().count(), Ordering::Relaxed);

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

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_RATE: f32 = 48_000.0;

    /// The root mean square of a block of samples.
    fn rms(samples: &[f32]) -> f32 {
        let sum: f32 = samples.iter().map(|sample| sample * sample).sum();
        (sum / samples.len() as f32).sqrt()
    }

    /// Runs a sine wave through the tone tilt of the plugin.
    fn tilt(frequency: f32, tone: f32) -> f32 {
        let mut sushi = Sushi::default();
        let low_gain = 10f32.powf(tone * 6.0 / 20.0);
        let high_gain = 10f32.powf(-tone * 6.0 / 20.0);
        let coefficient =
            1.0 - (-2.0 * std::f32::consts::PI * TONE_TILT_FREQUENCY / SAMPLE_RATE).exp();

        let mut output = Vec::new();
        for sample in 0..4_800 {
            let input =
                (sample as f32 * frequency / SAMPLE_RATE * 2.0 * std::f32::consts::PI).sin();
            let (left, _) = sushi.tone_tilt(input, input, low_gain, high_gain, coefficient);
            output.push(left);
        }

        rms(&output)
    }

    #[test]
    fn the_saturator_keeps_the_signal_bounded() {
        let sushi = Sushi::default();

        for grit in [false, true] {
            let (left, right) = sushi.saturate(10.0, -10.0, grit);
            assert!(left.is_finite() && right.is_finite());
            assert!(left.abs() <= 1.0 && right.abs() <= 1.0);
        }
    }

    #[test]
    fn the_grit_character_is_harsher_than_the_silk_character() {
        let sushi = Sushi::default();

        let (silk, _) = sushi.saturate(0.1, 0.1, false);
        let (grit, _) = sushi.saturate(0.1, 0.1, true);
        assert!(
            grit > silk,
            "the grit character was not louder ({grit} vs {silk})"
        );

        // The grit character is a little asymmetric, so the negative half of
        // the wave is driven less hard.
        let (silk, _) = sushi.saturate(-0.1, -0.1, false);
        let (grit, _) = sushi.saturate(-0.1, -0.1, true);
        assert!(
            grit.abs() < silk.abs(),
            "the grit character was not asymmetric ({grit} vs {silk})"
        );
    }

    #[test]
    fn the_saturator_does_not_add_a_dc_offset() {
        let sushi = Sushi::default();

        for grit in [false, true] {
            let (left, right) = sushi.saturate(0.0, 0.0, grit);
            assert_eq!(left, 0.0, "the left channel had an offset");
            assert_eq!(right, 0.0, "the right channel had an offset");
        }
    }

    #[test]
    fn the_normal_voices_are_released_when_the_arpeggiator_starts() {
        let mut sushi = Sushi::default();
        let settings = VoiceSettings::default();

        sushi.voices[0] = Some(Voice::new(
            1,
            Channel::Number(0),
            Key::Number(60),
            0,
            1.0,
            &settings,
        ));
        assert!(!sushi.voices[0].as_ref().is_some_and(Voice::is_releasing));

        sushi.release_normal_voices();
        assert!(sushi.voices[0].as_ref().is_some_and(Voice::is_releasing));
    }

    #[test]
    fn the_tone_tilt_boosts_the_lows() {
        let low = tilt(100.0, 1.0);
        let high = tilt(5_000.0, 1.0);
        assert!(
            low > high * 2.0,
            "the lows were not boosted ({low} vs {high})"
        );

        // With the tone turned the other way around, the highs win.
        let low = tilt(100.0, -1.0);
        let high = tilt(5_000.0, -1.0);
        assert!(
            high > low * 2.0,
            "the highs were not boosted ({low} vs {high})"
        );
    }

    #[test]
    fn the_tone_tilt_is_neutral_in_the_middle() {
        let low = tilt(100.0, 0.0);
        let high = tilt(5_000.0, 0.0);
        assert!(
            (low - high).abs() < low * 0.1,
            "the tone tilt changed the balance ({low} vs {high})"
        );
    }
}
