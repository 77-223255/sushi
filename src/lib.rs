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
pub mod patch;

use dsp::adsr::{EnvCoeffs, curve_exponent};
use dsp::arp::{Arp, MAX_CHORD_NOTES};
use dsp::fx::{Fx, FxParams};
use dsp::highpass::GlobalHighpass;
use dsp::lfo::Lfo;
use dsp::mod_feedback::{ModFeedback, ModFeedbackValues};
use dsp::vca::apply_width_pan;
use dsp::voice::{CONTROL_RATE, DcBlocker, OscSettings, Voice, VoiceSettings};
use editor::{SushiEditor, WINDOW_HEIGHT, WINDOW_WIDTH};
use params::{Character, ModDest, ModSource, NoteDiv, PolyMode};
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

/// How quickly the frequency of a synced LFO follows a tempo or division
/// change, in seconds. The phase itself keeps running, only the rate glides.
const LFO_SYNC_SMOOTHING_SECONDS: f32 = 0.01;

/// The maximum number of keys that a mono or legato voice remembers. This is
/// deliberately a fixed array: note handling runs on the audio thread.
const MAX_MONO_NOTES: usize = 16;

/// The length of the short fade applied to a stolen voice, in samples.
const STEAL_FADE_SAMPLES: f32 = 64.0;

/// The length of the reported reverb/delay tail, in seconds.
const TAIL_SECONDS: f32 = 12.0;

/// Sushi always runs the original Legacy engine. The `legacy` host parameter
/// is kept for state compatibility, but it cannot switch the engine to the
/// Modern path anymore.
const LEGACY_ENGINE: bool = true;

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

/// The saturation of a single sample used by the legacy flavor.
#[inline]
fn saturate_sample(sample: f32, drive: f32, negative_gain: f32) -> f32 {
    if sample >= 0.0 {
        (sample * drive).tanh()
    } else {
        (sample * drive * negative_gain).tanh()
    }
}

/// The extra wet mix that the space macro adds. The first release scaled the
/// macro straight into the wet mix, and that curve is kept so old patches keep
/// their sound.
fn space_amount(space: f32) -> f32 {
    (space * 0.35).clamp(0.0, 0.35)
}

/// The frequency of a synced LFO, from the host tempo and a note division.
///
/// A quarter note at 120 BPM is one beat every half second, so it runs at
/// 2 Hz. Invalid tempo readings fall back to 120 BPM instead of stopping the
/// LFO or producing a NaN phase increment.
fn lfo_sync_frequency(bpm: f64, division: NoteDiv) -> f32 {
    let bpm = if bpm.is_finite() { bpm.max(0.0) } else { 120.0 };
    (bpm / 60.0 / division.beats()) as f32
}

/// The glide coefficient for one control-rate update. A control update happens
/// every `CONTROL_RATE` samples, so the requested time constant is divided by
/// that rate.
fn glide_coefficient(glide_ms: f32, sample_rate: f32) -> f32 {
    if glide_ms <= 0.0 {
        1.0
    } else {
        let control_samples = (glide_ms * 0.001 * sample_rate / CONTROL_RATE as f32).max(1.0);
        1.0 - (-1.0 / control_samples).exp()
    }
}

/// The values of the three macros of the current block. They are the sources
/// of the modulation matrix.
#[derive(Debug, Clone, Copy)]
struct Macros {
    /// The brightness macro.
    bright: f32,
    /// The motion macro.
    motion: f32,
    /// The space macro.
    space: f32,
}

impl Default for Macros {
    fn default() -> Self {
        Self {
            bright: 0.5,
            motion: 0.5,
            space: 0.5,
        }
    }
}

/// A note that a mono or legato voice remembers while it is held.
#[derive(Debug, Clone, Copy)]
struct MonoNote {
    voice_id: VoiceID,
    channel: Channel,
    key: Key,
    velocity: f32,
}

/// A stolen voice that is fading out instead of being cut off immediately.
struct FadingVoice {
    voice: Voice,
    gain: f32,
    step: f32,
}

pub struct Sushi {
    params: Arc<SushiParams>,
    voices: [Option<Voice>; NUM_VOICES],
    /// The keys that are still held in mono and legato mode. The newest note
    /// is at the end of the array.
    mono_notes: [Option<MonoNote>; MAX_MONO_NOTES],
    mono_len: usize,
    /// The poly mode that was active in the last block. A mode change clears
    /// the mono note stack.
    last_poly_mode: PolyMode,
    /// Stolen voices that are faded out over a few samples.
    fading_voices: [Option<FadingVoice>; NUM_VOICES],
    next_internal_voice_id: u64,
    sample_rate: f32,
    /// The number of voices that may play at the same time. It is read once
    /// per block.
    voice_limit: usize,

    /// The two global LFOs. Their values are shared by every voice.
    lfo1: Lfo,
    lfo2: Lfo,
    /// The smoothed frequencies of the two LFOs while they are synced, in Hz.
    lfo1_sync_rate: f32,
    lfo2_sync_rate: f32,

    /// The effect chain. All of its buffers are allocated in `activate`.
    fx: Fx,

    /// The global highpass on the summed voices. Its default cutoff is an
    /// exact bypass, so old patches keep their sound.
    hpf: GlobalHighpass,

    /// The arpeggiator.
    arp: Arp,
    /// The number of arpeggiator triggers. The GUI uses it to flash a light.
    arp_led: Arc<AtomicUsize>,
    /// The position of the arpeggiator in its pattern. The GUI shows it in
    /// the strip of lights of the arpeggiator.
    arp_position: Arc<AtomicUsize>,
    /// The number of voices that are currently playing. The GUI shows it in a
    /// small meter of the header.
    voice_count: Arc<AtomicUsize>,
    /// The last values of the global modulation sources. The GUI uses them to
    /// draw the live markers of the modulation matrix.
    mod_feedback: Arc<ModFeedback>,
    /// The voices of the arpeggiator chord that is currently playing. A fixed
    /// array keeps the audio thread free of allocations, and every entry is
    /// released together so a chord can never leave a stuck voice behind.
    arp_voice_ids: [Option<i32>; MAX_CHORD_NOTES],
    /// The ID of the next voice of the arpeggiator.
    next_arp_voice_id: i32,
    /// Whether the arpeggiator was enabled in the last block.
    arp_was_on: bool,
    /// Whether the arpeggiator clock was running in the last block.
    arp_was_playing: bool,
    /// The host transport position at the start of the last block. It is used
    /// to detect loops and jumps so the arpeggiator can rephase.
    arp_last_position: Option<f64>,
    /// The smoothed pitch bend and mod wheel values.
    pitch_bend: f32,
    pitch_bend_target: f32,
    mod_wheel: f32,
    mod_wheel_target: f32,
    /// Whether the sustain pedal is down.
    sustain: bool,
    /// The flavor used by the previous block. A flavor switch resets the
    /// nonlinear stages that differ between the two engines.
    last_legacy: Option<bool>,
    /// The state of the two tone tilt filters.
    tone_left: f32,
    tone_right: f32,
    /// DC blockers after the character saturator. The grit character is
    /// intentionally asymmetric and otherwise leaves a DC offset.
    character_dc_left: DcBlocker,
    character_dc_right: DcBlocker,
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
            mono_notes: [None; MAX_MONO_NOTES],
            mono_len: 0,
            last_poly_mode: PolyMode::Poly,
            fading_voices: std::array::from_fn(|_| None),
            next_internal_voice_id: 0,
            sample_rate: 48_000.0,
            voice_limit: NUM_VOICES,

            lfo1: Lfo::new(LFO1_SEED),
            lfo2: Lfo::new(LFO2_SEED),
            // The default synced division is a quarter note at 120 BPM.
            lfo1_sync_rate: 2.0,
            lfo2_sync_rate: 2.0,
            fx: Fx::default(),
            hpf: GlobalHighpass::new(),

            arp: Arp::new(),
            arp_led: Arc::new(AtomicUsize::new(0)),
            arp_position: Arc::new(AtomicUsize::new(0)),
            voice_count: Arc::new(AtomicUsize::new(0)),
            mod_feedback: Arc::new(ModFeedback::default()),
            arp_voice_ids: [None; MAX_CHORD_NOTES],
            next_arp_voice_id: FIRST_ARP_VOICE_ID,
            arp_was_on: false,
            arp_was_playing: false,
            arp_last_position: None,
            pitch_bend: 0.0,
            pitch_bend_target: 0.0,
            mod_wheel: 0.0,
            mod_wheel_target: 0.0,
            sustain: false,
            last_legacy: None,
            tone_left: 0.0,
            tone_right: 0.0,
            character_dc_left: DcBlocker::default(),
            character_dc_right: DcBlocker::default(),

            // Open the editor a little larger than the 960x600 design so
            // the panel is comfortable on a normal DAW screen. The design
            // still scales continuously from its 960x600 minimum.
            editor_state: EguiEditorState::from_size(
                LogicalSize::new(WINDOW_WIDTH * 4.0 / 3.0, WINDOW_HEIGHT * 4.0 / 3.0),
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
    fn voice_settings(&self, steps: u32, macros: Macros) -> VoiceSettings {
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

        // The envelope curves are converted to an exponent here, once per
        // block. Zero is exactly one, so the default envelope stays fully
        // linear. The curve is applied to the final envelope value in the
        // voice, never to its time coefficients.
        let amp_env_curve = curve_exponent(params.ae_curve.smoothed.next_step(steps));
        let filter_env_curve = curve_exponent(params.fe_curve.smoothed.next_step(steps));

        // The glide is a one-pole filter: the bigger the time, the slower the
        // pitch moves towards the target. The coefficient is applied at the
        // control rate.
        let glide_step = glide_coefficient(params.glide.value(), sample_rate);

        // The matrix slots are read once per block. An active route from an
        // LFO owns that LFO, so the hidden legacy target/amount does not run
        // a second time.
        let mod_slots = params.mod_slots(steps);
        let lfo1_routed = mod_slots.iter().any(|slot| {
            slot.source == ModSource::Lfo1 && slot.dest != ModDest::Off && slot.amount != 0.0
        });
        let lfo2_routed = mod_slots.iter().any(|slot| {
            slot.source == ModSource::Lfo2 && slot.dest != ModDest::Off && slot.amount != 0.0
        });

        VoiceSettings {
            osc1,
            osc2,
            sub_level: params.sub_level.value(),
            osc_sync: params.osc_sync.value(),
            fm_amount: params.fm_amount.value(),
            noise_level: params.noise_level.value(),
            filter_mode: params.filter_mode.value(),
            filter_slope: params.filter_slope.value(),
            resonance: params.resonance.value(),
            filter_env_octaves: params.filter_env_amount.value() * 5.0,
            filter_keytrack: params.filter_keytrack.value(),
            filter_drive: params.filter_drive.value(),
            ring_mod: params.ring_mod.value(),
            osc_phase_reset: params.osc_phase_reset.value(),
            amp_env,
            filter_env,
            amp_env_curve,
            filter_env_curve,
            unison: params
                .unison
                .value()
                .clamp(1, dsp::voice::MAX_UNISON as i32) as usize,
            detune_cents: params.detune.value(),
            spread: params.spread.value(),
            tune_semitones: params.tune.smoothed.next_step(steps),
            pitch_bend_semitones: self.pitch_bend,
            mod_wheel: self.mod_wheel,
            glide_step,
            lfo1_amount: params.lfo1_amount.value(),
            lfo1_target: params.lfo1_target.value(),
            lfo1_routed,
            lfo2_amount: params.lfo2_amount.value(),
            lfo2_target: params.lfo2_target.value(),
            lfo2_routed,
            velocity_amount: params.velocity.smoothed.next_step(steps),
            drift_amount: 0.0,
            character_grit: false,
            legacy: false,
            // The slots of the modulation matrix are read once per block.
            mod_slots,
            // The macros are the sources of the modulation matrix.
            macro1: macros.bright,
            macro2: macros.motion,
            macro3: macros.space,
        }
    }

    /// Collects the settings of the effect chain.
    fn fx_params(&self) -> FxParams {
        let params = &self.params;
        FxParams {
            order: params.fx_order.value(),
            bypass_mask: params.fx_bypass_mask.value(),
            global_mix: params.fx_global_mix.value(),
            drive_amount: params.drive_amount.value(),
            drive_mix: params.drive_mix.value(),
            drive_tone: params.drive_tone.value(),
            phaser_rate: params.phaser_rate.value(),
            phaser_depth: params.phaser_depth.value(),
            phaser_mix: params.phaser_mix.value(),
            phaser_feedback: params.phaser_feedback.value(),
            eq_low: params.eq_low.value(),
            eq_mid: params.eq_mid.value(),
            eq_high: params.eq_high.value(),
            chorus_mix: params.chorus_mix.value(),
            chorus_rate: params.chorus_rate.value(),
            chorus_depth: params.chorus_depth.value(),
            chorus_width: params.chorus_width.value(),
            delay_time_s: params.delay_time.value() * 0.001,
            delay_feedback: params.delay_feedback.value(),
            delay_mix: params.delay_mix.value(),
            delay_damping: params.delay_damping.value(),
            reverb_mix: params.reverb_mix.value(),
            reverb_decay: params.reverb_decay.value(),
            reverb_tone: params.reverb_tone.value(),
            comp_threshold: params.comp_threshold.value(),
            comp_ratio: params.comp_ratio.value(),
            comp_attack: params.comp_attack.value(),
            comp_release: params.comp_release.value(),
            comp_makeup: params.comp_makeup.value(),
            comp_mix: params.comp_mix.value(),
            trem_rate: params.trem_rate.value(),
            trem_depth: params.trem_depth.value(),
            trem_stereo: params.trem_stereo.value(),
        }
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

    /// The position of the arpeggiator in its pattern. The editor uses it to
    /// light up the strip of the arpeggiator.
    pub fn arp_position(&self) -> Arc<AtomicUsize> {
        self.arp_position.clone()
    }

    /// The last values of the global modulation sources. The editor uses them
    /// to show the live modulation of the matrix.
    pub fn mod_feedback(&self) -> Arc<ModFeedback> {
        self.mod_feedback.clone()
    }

    /// Puts every voice of the arpeggiator chord into its release stage.
    fn release_arp_voices(&mut self) {
        let voice_ids = self.arp_voice_ids;
        if voice_ids.iter().all(Option::is_none) {
            return;
        }
        self.arp_voice_ids = [None; MAX_CHORD_NOTES];

        for voice in self.voices.iter_mut().flatten() {
            if voice_ids
                .iter()
                .flatten()
                .any(|voice_id| *voice_id == voice.voice_id())
            {
                voice.release();
            }
        }
    }

    /// Whether any voice is still producing sound. A voice that is only
    /// waiting in a slot for its finished envelope has already gone silent.
    fn any_voice_sounding(&self) -> bool {
        self.voices
            .iter()
            .flatten()
            .any(|voice| !voice.is_finished())
            || self.fading_voices.iter().any(Option::is_some)
    }

    /// Restarts the enabled LFOs on the first note after silence.
    ///
    /// The two LFOs are global, so restarting them on every NoteOn would make
    /// the phase of a chord or an arpeggio jump between the notes. The policy
    /// is therefore one retrigger per silent gap: the LFOs line up with the
    /// first note that starts a phrase, while the later notes of the same
    /// phrase share the running phase.
    fn retrigger_lfos_if_silent(&mut self) {
        if self.any_voice_sounding() {
            return;
        }
        if self.params.lfo1_retrigger.value() {
            self.lfo1.retrigger();
        }
        if self.params.lfo2_retrigger.value() {
            self.lfo2.retrigger();
        }
    }

    /// Starts every note of an arpeggiator step.
    ///
    /// A new pattern step releases the previous chord first. A strummed note
    /// that becomes due later only adds its own voice, because it belongs to
    /// the same step and must not release the base note.
    fn start_arp_notes(
        &mut self,
        context: &mut impl ProcessContext<Self>,
        sample_offset: u32,
        notes: &[Option<(u8, f32)>],
        new_step: bool,
        settings: &VoiceSettings,
    ) {
        if new_step {
            // An arpeggiated phrase retriggers on its first note just like a
            // played phrase does.
            self.retrigger_lfos_if_silent();
            self.release_arp_voices();
        }

        for (key, velocity) in notes.iter().flatten() {
            self.start_arp_voice(context, sample_offset, *key, *velocity, settings);
        }
    }

    /// Starts one arpeggiator voice and remembers its ID for the chord release.
    fn start_arp_voice(
        &mut self,
        context: &mut impl ProcessContext<Self>,
        sample_offset: u32,
        key: u8,
        velocity: f32,
        settings: &VoiceSettings,
    ) {
        self.free_finished_voices(context, sample_offset);

        // Do not start a note that cannot be released because all four chord
        // slots are already taken. This cannot happen with the four-note
        // limit, but keeps the audio path safe if a future chord is larger.
        let Some(chord_slot) = self.arp_voice_ids.iter().position(Option::is_none) else {
            return;
        };

        // The voice of the arpeggiator is not part of the limit of the voices,
        // so it always uses a free slot.
        let voice_idx = self.voice_slot_to_use(NUM_VOICES);
        if let Some(stolen) = self.voices[voice_idx].take() {
            terminate_voice(context, sample_offset, &stolen);
            self.start_fade(stolen);
        }

        let voice_id = self.next_arp_voice_id;
        self.next_arp_voice_id = self.next_arp_voice_id.wrapping_add(1);
        self.arp_voice_ids[chord_slot] = Some(voice_id);

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
    /// The asymmetry only changes the gain of the two halves of the wave. That
    /// does introduce a DC offset on non-silent signals, which the character
    /// DC blocker after this stage removes; silence stays exactly silent.
    fn saturate(&mut self, left: f32, right: f32, grit: bool) -> (f32, f32) {
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

    /// Reads the limit of the voices from the parameters.
    ///
    /// This is done once per block, so that the audio thread never has to
    /// allocate.
    fn update_voice_limit(&mut self) {
        self.voice_limit = self.params.voices.value().clamp(1, NUM_VOICES as i32) as usize;
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

    /// Adds a note to the mono/legato held-key stack. A repeated note is
    /// moved to the top instead of being stored twice.
    fn push_mono_note(&mut self, note: MonoNote) {
        self.remove_mono_note(note.voice_id, note.channel, note.key);

        if self.mono_len == MAX_MONO_NOTES {
            // Drop the oldest note. This can only happen with a broken host
            // that never sends note offs.
            self.mono_notes.copy_within(1..MAX_MONO_NOTES, 0);
            self.mono_len -= 1;
        }

        self.mono_notes[self.mono_len] = Some(note);
        self.mono_len += 1;
    }

    /// Removes a held note from the mono/legato stack.
    fn remove_mono_note(
        &mut self,
        voice_id: VoiceID,
        channel: Channel,
        key: Key,
    ) -> Option<MonoNote> {
        for index in (0..self.mono_len).rev() {
            let Some(note) = self.mono_notes[index] else {
                continue;
            };
            let same_id = !voice_id.is_wildcard() && note.voice_id == voice_id;
            if same_id || (note.channel == channel && note.key == key) {
                for shift in index..self.mono_len - 1 {
                    self.mono_notes[shift] = self.mono_notes[shift + 1];
                }
                self.mono_len -= 1;
                self.mono_notes[self.mono_len] = None;
                return Some(note);
            }
        }

        None
    }

    /// The most recently pressed key that is still held.
    fn mono_top(&self) -> Option<MonoNote> {
        if self.mono_len == 0 {
            None
        } else {
            self.mono_notes[self.mono_len - 1]
        }
    }

    fn clear_mono_notes(&mut self) {
        self.mono_notes = [None; MAX_MONO_NOTES];
        self.mono_len = 0;
    }

    /// Puts a stolen voice into the short fade-out pool instead of cutting it
    /// off at a random sample value.
    fn start_fade(&mut self, voice: Voice) {
        let slot = match self.fading_voices.iter().position(Option::is_none) {
            Some(slot) => slot,
            None => {
                let mut quietest = 0;
                let mut gain = f32::MAX;
                for (index, fading) in self.fading_voices.iter().enumerate() {
                    if let Some(fading) = fading
                        && fading.gain < gain
                    {
                        gain = fading.gain;
                        quietest = index;
                    }
                }
                quietest
            }
        };

        self.fading_voices[slot] = Some(FadingVoice {
            voice,
            gain: 1.0,
            step: 1.0 / STEAL_FADE_SAMPLES,
        });
    }

    /// Handles a note off in mono and legato mode. Releasing the sounding note
    /// returns to the most recent key that is still held.
    fn release_mono_note(
        &mut self,
        context: &mut impl ProcessContext<Self>,
        sample_offset: u32,
        voice_id: VoiceID,
        channel: Channel,
        key: Key,
        settings: &VoiceSettings,
    ) {
        let Some(released) = self.remove_mono_note(voice_id, channel, key) else {
            return;
        };

        let Some(index) = self.newest_voice_index() else {
            return;
        };

        let is_current = self.voices[index].as_ref().is_some_and(|voice| {
            (voice.channel() == released.channel && voice.key() == released.key)
                || voice.voice_id()
                    == released
                        .voice_id
                        .id_or_fallback(released.key, released.channel)
        });
        if !is_current {
            // An older held key was released while a newer one is sounding.
            return;
        }

        if self.sustain {
            // The pedal keeps the released note alive. On pedal up the
            // release_sustained_voices method returns to the held key.
            if let Some(voice) = self.voices[index].as_mut() {
                voice.sustain();
            }
            return;
        }

        if let Some(top) = self.mono_top() {
            if let Some(voice) = self.voices[index].as_mut() {
                // The current note is over and the previous held note takes
                // over. Its envelopes keep running, so returning to it feels
                // legato instead of restarting the sound.
                terminate_voice(context, sample_offset, voice);
                voice.set_voice_id(top.voice_id.id_or_fallback(top.key, top.channel));
                voice.set_channel(top.channel);
                voice.set_velocity(top.velocity);
                let retrigger = voice.is_releasing();
                voice.set_key(top.key, settings, retrigger);
            }
        } else if let Some(voice) = self.voices[index].as_mut() {
            voice.release();
        }
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

        let poly_mode = self.params.poly_mode.value();
        if matches!(poly_mode, PolyMode::Mono | PolyMode::Legato) {
            self.push_mono_note(MonoNote {
                voice_id,
                channel,
                key,
                velocity,
            });
        }

        match poly_mode {
            PolyMode::Poly => {
                // Use a free voice, or steal the oldest one when the limit of
                // the voices is reached.
                let (voice_idx, stolen) = self.reserve_voice_slot();

                // Tell the host about the stolen voice so it can recycle the
                // voice's modulation resources.
                if let Some(stolen) = stolen {
                    terminate_voice(context, sample_offset, &stolen);
                    self.start_fade(stolen);
                }

                self.fill_voice_slot(voice_idx, voice_id, channel, key, velocity, settings);
            }
            PolyMode::Mono => {
                // A mono synth only ever has a single voice.
                if let Some(index) = self.newest_voice_index()
                    && let Some(voice) = self.voices[index].as_mut()
                {
                    // The voice is reused for the new note, so the host is told
                    // that the old note is over.
                    terminate_voice(context, sample_offset, voice);
                    voice.set_voice_id(voice_id.id_or_fallback(key, channel));
                    voice.set_channel(channel);
                    voice.set_velocity(velocity);
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
            PolyMode::Legato => {
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
                        voice.set_channel(channel);
                        voice.set_velocity(velocity);
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

    /// Reserves the slot of a new voice.
    ///
    /// As long as the limit of the voices is not reached, a free slot is used.
    /// When the limit is reached, the oldest voice is stolen, so that the
    /// number of voices never grows beyond the limit. The stolen voice is
    /// returned, so that the host can be told about it.
    fn reserve_voice_slot(&mut self) -> (usize, Option<Voice>) {
        let voice_idx = self.voice_slot_to_use(self.voice_limit);
        let stolen = self.voices[voice_idx].take();
        (voice_idx, stolen)
    }

    /// The slot a new voice should use: a free one, or the oldest voice when
    /// all of them are in use.
    ///
    /// The limit is the number of voices that may play at the same time. When
    /// it is reached, the oldest voice is stolen, so that the limit is never
    /// exceeded.
    fn voice_slot_to_use(&self, limit: usize) -> usize {
        let active = self.voices.iter().flatten().count();
        if active >= limit {
            return self.oldest_voice_index();
        }

        match self.voices.iter().position(|voice| voice.is_none()) {
            Some(free_idx) => free_idx,
            None => self.oldest_voice_index(),
        }
    }

    /// The index of the oldest voice, which is the one that gets stolen.
    fn oldest_voice_index(&self) -> usize {
        self.voices
            .iter()
            .enumerate()
            .min_by_key(|(_, voice)| {
                voice
                    .as_ref()
                    .map(|voice| voice.internal_voice_id())
                    .unwrap_or(u64::MAX)
            })
            .map(|(index, _)| index)
            .unwrap_or(0)
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
        let (voice_idx, stolen) = self.reserve_voice_slot();

        // Tell the host about the stolen voice so it can recycle the voice's
        // modulation resources.
        if let Some(stolen) = stolen {
            terminate_voice(context, sample_offset, &stolen);
            self.start_fade(stolen);
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
        let arp_voice_ids = self.arp_voice_ids;
        for voice in self.voices.iter_mut().flatten() {
            let is_arp = arp_voice_ids
                .iter()
                .flatten()
                .any(|voice_id| *voice_id == voice.voice_id());
            if is_arp {
                continue;
            }
            voice.release();
        }
    }

    /// Handles MIDI All Notes Off and All Sound Off. The voices are released
    /// instead of killed, so their envelopes close without a click.
    fn all_notes_off(&mut self) {
        self.release_normal_voices();
        self.release_arp_voices();
        self.clear_mono_notes();
        self.arp.reset();
        self.sustain = false;
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

    /// Defers the release of the matching voices until the sustain pedal is
    /// lifted.
    fn sustain_voices(&mut self, voice_id: VoiceID, channel: Channel, key: Key) {
        for voice in self.voices.iter_mut().flatten() {
            let matches = voice_id == VoiceID::ID(voice.voice_id())
                || (voice.channel() == channel && voice.key() == key);
            if matches {
                voice.sustain();
                if !voice_id.is_wildcard() {
                    return;
                }
            }
        }
    }

    /// Releases every voice that was held by the sustain pedal. In mono and
    /// legato mode the voice returns to the most recent key that is still held.
    fn release_sustained_voices(
        &mut self,
        context: &mut impl ProcessContext<Self>,
        sample_offset: u32,
        settings: &VoiceSettings,
    ) {
        let top = if self.params.poly_mode.value() == PolyMode::Poly {
            None
        } else {
            self.mono_top()
        };

        for voice in self.voices.iter_mut().flatten() {
            if !voice.is_sustained() {
                continue;
            }

            if let Some(top) = top {
                terminate_voice(context, sample_offset, voice);
                voice.set_voice_id(top.voice_id.id_or_fallback(top.key, top.channel));
                voice.set_channel(top.channel);
                voice.set_velocity(top.velocity);
                let retrigger = voice.is_releasing();
                voice.set_key(top.key, settings, retrigger);
            } else {
                voice.release_sustained();
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

    const MIDI_INPUT: MidiConfig = MidiConfig::MidiCCs;
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
            // The editor may be resized by the host. The panel reflows
            // itself and shows a sad face when the window gets too small.
            EguiNiceSettings::new().with_resize_hint(
                ResizeHint::resizable().with_min_logical_size(LogicalSize::new(560.0, 360.0)),
            ),
            SushiEditor::new(
                self.params.clone(),
                scope_rx,
                self.arp_led.clone(),
                self.arp_position.clone(),
                self.voice_count.clone(),
                self.mod_feedback.clone(),
            ),
        )
    }

    /// Old projects were saved before the Modern/Legacy switch existed. The
    /// migration is kept for state compatibility: it marks them as legacy,
    /// which is now the only engine anyway. A state without any of the old
    /// parameters (for example an empty host state) does not get the flag.
    fn filter_state(state: &mut PluginState) {
        if state.params.contains_key("osc1_wave") && !state.params.contains_key("legacy") {
            state.params.insert(
                "legacy".to_string(),
                nice_plug::plugin::ParamValue::Bool(true),
            );
        }
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
        self.hpf.reset();
        self.arp.reset();
        self.update_voice_limit();
        true
    }

    fn reset(&mut self) {
        for voice in self.voices.iter_mut() {
            *voice = None;
        }
        self.next_internal_voice_id = 0;
        self.lfo1 = Lfo::new(LFO1_SEED);
        self.lfo2 = Lfo::new(LFO2_SEED);
        self.lfo1_sync_rate = 2.0;
        self.lfo2_sync_rate = 2.0;
        self.fx.reset();
        self.hpf.reset();
        self.arp.reset();
        self.arp_voice_ids = [None; MAX_CHORD_NOTES];
        self.arp_was_on = false;
        self.arp_was_playing = false;
        self.arp_last_position = None;
        self.pitch_bend = 0.0;
        self.pitch_bend_target = 0.0;
        self.mod_wheel = 0.0;
        self.mod_wheel_target = 0.0;
        self.sustain = false;
        self.last_legacy = None;
        self.clear_mono_notes();
        self.last_poly_mode = PolyMode::Poly;
        self.fading_voices = std::array::from_fn(|_| None);
        self.tone_left = 0.0;
        self.tone_right = 0.0;
        self.character_dc_left = DcBlocker::default();
        self.character_dc_right = DcBlocker::default();
        self.arp_led.store(0, Ordering::Relaxed);
        self.arp_position.store(0, Ordering::Relaxed);
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

        // The limit of the voices is read once per block.
        self.update_voice_limit();
        // The engine is Legacy-only. The host parameter is intentionally not
        // consulted, so old projects cannot switch back to the Modern path.
        let legacy = LEGACY_ENGINE;
        if self.last_legacy != Some(legacy) {
            self.character_dc_left = DcBlocker::default();
            self.character_dc_right = DcBlocker::default();
            self.last_legacy = Some(legacy);
        }

        // The transport of the host, which runs the arpeggiator. The
        // fields are copied so the context is free for the event loop.
        let (bpm, transport_playing, transport_position) = {
            let transport = context.transport();
            (
                transport.tempo.unwrap_or(120.0),
                transport.playing,
                transport.pos_beats,
            )
        };
        let beats_per_sample = bpm / (60.0 * sample_rate as f64);
        let block_beats = beats_per_sample * num_samples as f64;

        // The synced LFO frequencies. They are computed once per block and
        // the phase increment follows them through a short one-pole glide, so
        // a tempo change does not step the rate.
        let lfo1_sync_target = lfo_sync_frequency(bpm, self.params.lfo1_div.value());
        let lfo2_sync_target = lfo_sync_frequency(bpm, self.params.lfo2_div.value());
        let lfo_sync_step = 1.0 - (-1.0 / (LFO_SYNC_SMOOTHING_SECONDS * sample_rate)).exp();

        // The macros, read once per block.
        let bright = self.params.macro_bright.smoothed.next_step(steps);
        let motion = self.params.macro_motion.smoothed.next_step(steps);
        let space = self.params.macro_space.smoothed.next_step(steps);
        let space_wet = space_amount(space);
        let grit = self.params.character.value() == Character::Grit;

        let mut settings = self.voice_settings(
            steps,
            Macros {
                bright,
                motion,
                space,
            },
        );
        settings.legacy = legacy;
        if legacy {
            // The first release applied a per-sample coefficient at the
            // control rate, which made the glide eight times slower.
            let glide_ms = self.params.glide.value();
            settings.glide_step = if glide_ms <= 0.0 {
                1.0
            } else {
                1.0 - (-1.0 / (glide_ms * 0.001 * sample_rate)).exp()
            };
        }
        let mut fx_params = self.fx_params();

        // The global highpass sits on the summed voices, before the character
        // saturator. Its default 20 Hz cutoff is an exact bypass.
        let hpf_cutoff = self.params.hpf_cutoff.smoothed.next_step(steps);
        self.hpf.set_cutoff(hpf_cutoff, sample_rate);

        // The LFO phase offsets and curves are block-rate controls. The
        // offsets move only the read position; the phase advance itself is
        // unchanged.
        let lfo1_phase = self.params.lfo1_phase.value();
        let lfo2_phase = self.params.lfo2_phase.value();
        let lfo1_curve = self.params.lfo1_curve.value();
        let lfo2_curve = self.params.lfo2_curve.value();

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

        // The tone tilt. Its filter coefficient is fixed; the gains follow
        // the smoothed tone parameter per sample.
        let tilt_coefficient =
            1.0 - (-2.0 * std::f32::consts::PI * TONE_TILT_FREQUENCY / sample_rate).exp();

        // The arpeggiator.
        let arp_on = self.params.arp_on.value();
        let arp_latch = self.params.arp_latch.value();
        let arp_pattern = self.params.arp_pattern.value();
        let arp_division = self.params.arp_rate.value().beats();
        let arp_octaves = self.params.arp_octaves.value().clamp(1, 3) as u8;
        let arp_gate = self.params.arp_gate.value();
        let arp_swing = self.params.arp_swing.value();
        let arp_length = self.params.arp_length.value().clamp(1, 16) as usize;
        let arp_ratchet = self.params.arp_ratchet.value().clamp(1, 4) as usize;
        let arp_chord = self.params.arp_chord.value();
        let arp_strum = self.params.arp_strum.value();
        let arp_probability = self.params.arp_probability.value();

        // A change of the poly mode invalidates the held keys of the old
        // mode. The mode is also read here so the note off handling and the
        // voice allocation agree within one block.
        let poly_mode = self.params.poly_mode.value();
        if poly_mode != self.last_poly_mode {
            // Do not leave voices from the old allocation mode hanging.
            self.release_normal_voices();
            self.clear_mono_notes();
            self.last_poly_mode = poly_mode;
        }

        if arp_on != self.arp_was_on {
            if arp_on {
                // The arpeggiator was switched on, so the voices of the host
                // are released. From now on the note offs of the host only go
                // to the arpeggiator, so the voices would stay stuck forever.
                self.release_normal_voices();
                self.clear_mono_notes();
            } else {
                // The arpeggiator was switched off, so its voice goes away.
                self.release_arp_voices();
                self.arp.reset();
                self.clear_mono_notes();
            }
            self.arp_was_on = arp_on;
        }

        // If the host reports a transport position, the arpeggiator follows
        // the host clock and pauses when the transport stops. Hosts without
        // position information keep the old free-running behaviour.
        let arp_running = arp_on && (legacy || transport_position.is_none() || transport_playing);
        if legacy {
            // The first release did not follow the host transport; the arp
            // was free running as soon as it was enabled.
            self.arp_was_playing = arp_on;
            self.arp_last_position = None;
        } else if arp_running {
            let jumped = match (transport_position, self.arp_last_position) {
                (Some(position), Some(last)) => (position - (last + block_beats)).abs() > 0.01,
                (Some(_), None) => true,
                _ => false,
            };

            if !self.arp_was_playing || jumped {
                self.release_arp_voices();
                if let Some(position) = transport_position {
                    self.arp.resync(position, arp_division);
                } else {
                    self.arp.pause();
                }
            }
            self.arp_was_playing = true;
            self.arp_last_position = transport_position.map(|position| position + block_beats);
        } else {
            if self.arp_was_playing {
                self.release_arp_voices();
                self.arp.pause();
            }
            self.arp_was_playing = false;
            self.arp_last_position = None;
        }

        // The output channels are looked up once, so that the sample loop
        // doesn't have to iterate over all of them for every single sample.
        let output = buffer.as_slice();
        let mut channels = output.iter_mut();
        let mut left_channel = channels.next().map(|channel| &mut **channel);
        let mut right_channel = channels.next().map(|channel| &mut **channel);

        let mut next_event = context.next_event();
        // The last values of the two global LFOs of this block. They are kept
        // here so the editor can show them without another read of the LFOs.
        let mut last_lfo1 = 0.0;
        let mut last_lfo2 = 0.0;
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
                            self.retrigger_lfos_if_silent();
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
                        } else if poly_mode == PolyMode::Poly {
                            if self.sustain {
                                self.sustain_voices(voice_id, channel, key);
                            } else {
                                self.release_voices(voice_id, channel, key);
                            }
                        } else {
                            self.release_mono_note(
                                context,
                                sample_idx as u32,
                                voice_id,
                                channel,
                                key,
                                &settings,
                            );
                        }
                    }
                    NoteEvent::MidiPitchBend { value, .. } => {
                        // A standard +/- 2 semitone bend range.
                        self.pitch_bend_target = ((value - 0.5) * 4.0).clamp(-2.0, 2.0);
                    }
                    NoteEvent::MidiCC { cc, value, .. } => match cc {
                        1 => {
                            // Mod wheel: adds to LFO 1. If LFO 1 has no
                            // destination it becomes a vibrato wheel.
                            self.mod_wheel_target = value.clamp(0.0, 1.0);
                        }
                        64 => {
                            let was_down = self.sustain;
                            self.sustain = value >= 0.5;
                            if was_down && !self.sustain {
                                self.release_sustained_voices(
                                    context,
                                    sample_idx as u32,
                                    &settings,
                                );
                            }
                        }
                        // All Sound Off and All Notes Off. Both release the
                        // voices and clear the held state without allocating.
                        120 | 123 => self.all_notes_off(),
                        _ => (),
                    },
                    NoteEvent::Choke {
                        timing,
                        voice_id,
                        channel,
                        key,
                    } => {
                        if arp_on {
                            self.release_arp_voices();
                            self.arp.reset();
                        } else {
                            self.choke_voices(context, timing, voice_id, channel, key);
                            if poly_mode != PolyMode::Poly {
                                self.clear_mono_notes();
                            }
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
            // Pitch bend and the mod wheel are smoothed here so MIDI events
            // do not step the pitch or the vibrato depth.
            self.pitch_bend += (self.pitch_bend_target - self.pitch_bend) * 0.01;
            self.mod_wheel += (self.mod_wheel_target - self.mod_wheel) * 0.01;
            settings.pitch_bend_semitones = self.pitch_bend;
            settings.mod_wheel = self.mod_wheel;
            // Continuous mixer and oscillator settings are smoothed per
            // sample as well; a block-level step is audible as a zipper.
            settings.osc1.level = self.params.osc1_level.smoothed.next();
            settings.osc1.pulse_width = self.params.osc1_pw.smoothed.next();
            settings.osc2.level = self.params.osc2_level.smoothed.next();
            settings.osc2.pulse_width = self.params.osc2_pw.smoothed.next();
            settings.sub_level = self.params.sub_level.smoothed.next();
            settings.noise_level = self.params.noise_level.smoothed.next();
            settings.fm_amount = self.params.fm_amount.smoothed.next();
            settings.filter_drive = self.params.filter_drive.smoothed.next();
            settings.ring_mod = self.params.ring_mod.smoothed.next();
            settings.detune_cents = self.params.detune.smoothed.next();
            settings.spread = self.params.spread.smoothed.next();
            let vca_width = self.params.vca_width.smoothed.next();
            let vca_pan = self.params.vca_pan.smoothed.next();
            fx_params.drive_amount = self.params.drive_amount.smoothed.next();
            fx_params.drive_mix = self.params.drive_mix.smoothed.next();
            fx_params.drive_tone = self.params.drive_tone.smoothed.next();
            fx_params.phaser_rate = self.params.phaser_rate.smoothed.next();
            fx_params.phaser_depth = self.params.phaser_depth.smoothed.next();
            fx_params.phaser_mix = self.params.phaser_mix.smoothed.next();
            fx_params.phaser_feedback = self.params.phaser_feedback.smoothed.next();
            fx_params.eq_low = self.params.eq_low.smoothed.next();
            fx_params.eq_mid = self.params.eq_mid.smoothed.next();
            fx_params.eq_high = self.params.eq_high.smoothed.next();
            fx_params.chorus_mix = self.params.chorus_mix.smoothed.next() + space_wet;
            fx_params.chorus_rate = self.params.chorus_rate.smoothed.next();
            fx_params.chorus_depth = self.params.chorus_depth.smoothed.next();
            fx_params.chorus_width = self.params.chorus_width.smoothed.next();
            fx_params.delay_time_s = self.params.delay_time.smoothed.next() * 0.001;
            fx_params.delay_feedback = self.params.delay_feedback.smoothed.next();
            fx_params.delay_mix = self.params.delay_mix.smoothed.next() + space_wet;
            fx_params.delay_damping = self.params.delay_damping.smoothed.next();
            fx_params.reverb_mix = self.params.reverb_mix.smoothed.next() + space_wet;
            fx_params.reverb_decay = self.params.reverb_decay.smoothed.next();
            fx_params.reverb_tone = self.params.reverb_tone.smoothed.next();
            fx_params.comp_threshold = self.params.comp_threshold.smoothed.next();
            fx_params.comp_ratio = self.params.comp_ratio.smoothed.next();
            fx_params.comp_attack = self.params.comp_attack.smoothed.next();
            fx_params.comp_release = self.params.comp_release.smoothed.next();
            fx_params.comp_makeup = self.params.comp_makeup.smoothed.next();
            fx_params.comp_mix = self.params.comp_mix.smoothed.next();
            fx_params.trem_rate = self.params.trem_rate.smoothed.next();
            fx_params.trem_depth = self.params.trem_depth.smoothed.next();
            fx_params.chorus_mix = fx_params.chorus_mix.clamp(0.0, 1.0);
            fx_params.delay_mix = fx_params.delay_mix.clamp(0.0, 1.0);
            fx_params.reverb_mix = fx_params.reverb_mix.clamp(0.0, 1.0);

            // The arpeggiator plays the next note, chord or strummed note of
            // its pattern.
            if arp_running {
                let tick = self.arp.tick_extended(
                    beats_per_sample,
                    arp_division,
                    arp_gate,
                    arp_pattern,
                    arp_octaves,
                    arp_swing,
                    arp_length,
                    arp_ratchet,
                    arp_chord,
                    arp_strum,
                    arp_probability,
                    sample_rate,
                );
                if tick.release {
                    self.release_arp_voices();
                }

                // The LED and the pattern position follow the base step, not
                // every note of a strummed chord.
                let new_step = tick.trigger.is_some();
                if new_step {
                    self.arp_position
                        .store(self.arp.position() % 16, Ordering::Relaxed);
                    self.arp_led.fetch_add(1, Ordering::Relaxed);
                }

                if tick.chord_len > 0 {
                    self.start_arp_notes(
                        context,
                        sample_idx as u32,
                        &tick.chord[..tick.chord_len],
                        new_step,
                        &settings,
                    );
                }
            }

            // The global LFOs. The free rate keeps smoothing even while a
            // synced LFO runs, so switching sync off does not jump to a stale
            // value.
            let lfo1_free_rate = self.params.lfo1_rate.smoothed.next();
            let lfo2_free_rate = self.params.lfo2_rate.smoothed.next();
            let lfo1_frequency = if self.params.lfo1_sync.value() {
                self.lfo1_sync_rate += (lfo1_sync_target - self.lfo1_sync_rate) * lfo_sync_step;
                self.lfo1_sync_rate
            } else {
                lfo1_free_rate
            };
            let lfo2_frequency = if self.params.lfo2_sync.value() {
                self.lfo2_sync_rate += (lfo2_sync_target - self.lfo2_sync_rate) * lfo_sync_step;
                self.lfo2_sync_rate
            } else {
                lfo2_free_rate
            };
            let lfo1 = self.lfo1.next_shaped(
                lfo1_frequency,
                sample_rate,
                self.params.lfo1_wave.value(),
                lfo1_phase,
                lfo1_curve,
            );
            let lfo2 = self.lfo2.next_shaped(
                lfo2_frequency,
                sample_rate,
                self.params.lfo2_wave.value(),
                lfo2_phase,
                lfo2_curve,
            );
            last_lfo1 = lfo1;
            last_lfo2 = lfo2;

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

            // Stolen voices fade out over a few samples instead of being cut
            // off at a non-zero sample value.
            for slot in self.fading_voices.iter_mut() {
                let Some(fading) = slot else { continue };

                let (voice_left, voice_right) =
                    fading
                        .voice
                        .next_sample(&settings, cutoff, lfo1, lfo2, sample_rate);
                left += voice_left * fading.gain;
                right += voice_right * fading.gain;

                fading.gain -= fading.step;
                if fading.gain <= 0.0 || fading.voice.is_finished() {
                    *slot = None;
                }
            }

            // The global highpass on the summed voices, before the character
            // saturator. It is an exact bypass at its default cutoff.
            let (left, right) = self.hpf.process(left, right);

            // The character saturator, its DC blocker, the effect chain and
            // the tone tilt. Only the asymmetric grit character needs the DC
            // blocker; the symmetric silk character keeps its old low end.
            let (left, right) = self.saturate(left, right, grit);
            let (left, right) = if grit {
                (
                    self.character_dc_left.process(left, sample_rate),
                    self.character_dc_right.process(right, sample_rate),
                )
            } else {
                // Start a later Grit switch from a clean blocker state.
                self.character_dc_left = DcBlocker::default();
                self.character_dc_right = DcBlocker::default();
                (left, right)
            };
            let (left, right) = self.fx.process(left, right, &fx_params);
            let tone = self.params.tone.smoothed.next();
            let low_gain = 10f32.powf(tone * 6.0 / 20.0);
            let high_gain = 10f32.powf(-tone * 6.0 / 20.0);
            let (left, right) = self.tone_tilt(left, right, low_gain, high_gain, tilt_coefficient);

            // The global VCA width and pan, after the tone tilt and before
            // the master volume. Its neutral setting is bit-exact.
            let (left, right) = apply_width_pan(left, right, vca_width, vca_pan);

            // The master volume.
            let left = left * volume;
            let right = right * volume;

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

        // Publish the global modulation sources once per block. The editor
        // draws a live marker from this snapshot; the audio path never reads
        // it back. The macros are already block-rate values, while the LFOs
        // and the mod wheel are the last samples of the block.
        self.mod_feedback.store(ModFeedbackValues {
            lfo1: last_lfo1,
            lfo2: last_lfo2,
            macro1: bright,
            macro2: motion,
            macro3: space,
            mod_wheel: self.mod_wheel,
        });

        // The number of voices that are playing, for the meter of the editor.
        self.voice_count
            .store(self.voices.iter().flatten().count(), Ordering::Relaxed);

        // Do not let a host suspend the plugin while a note, an arpeggiator or
        // an effect tail is still producing sound. Without this, some hosts
        // cut reverb and delay tails after the last note off.
        let voices_active = self.voices.iter().any(Option::is_some)
            || self.fading_voices.iter().any(Option::is_some);
        let arp_active = arp_running && (self.arp.has_notes() || self.arp.is_sounding());
        // A masked stage, a silent effect and a zero global mix neither
        // produce nor keep a tail alive.
        let fx_active = fx_params.is_active();

        if voices_active || arp_active {
            ProcessStatus::KeepAlive
        } else if fx_active {
            ProcessStatus::Tail((sample_rate * TAIL_SECONDS) as u32)
        } else {
            ProcessStatus::Normal
        }
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
    use crate::params::{ArpChord, LfoTarget, LfoWaveform, NoteDiv};
    use crate::patch::test_support::setter;

    const SAMPLE_RATE: f32 = 48_000.0;

    /// A minimal process context for the engine tests. It has no events and
    /// accepts every outgoing event.
    struct TestContext {
        transport: Transport,
        events: std::collections::VecDeque<PluginNoteEvent<Sushi>>,
    }

    impl TestContext {
        fn new() -> Self {
            Self {
                transport: Transport::new(SAMPLE_RATE),
                events: std::collections::VecDeque::new(),
            }
        }
    }

    impl ProcessContext<Sushi> for TestContext {
        fn plugin_api(&self) -> PluginApi {
            PluginApi::Standalone
        }

        fn execute_background(&self, _task: ()) {}

        fn execute_gui(&self, _task: ()) {}

        fn transport(&self) -> &Transport {
            &self.transport
        }

        fn next_event(&mut self) -> Option<PluginNoteEvent<Sushi>> {
            self.events.pop_front()
        }

        fn try_send_event(
            &mut self,
            _event: PluginNoteEvent<Sushi>,
        ) -> Result<
            (),
            (
                PluginNoteEvent<Sushi>,
                nice_plug::context::process::SendEventError,
            ),
        > {
            Ok(())
        }

        fn set_latency_samples(&self, _samples: u32) {}

        fn request_restart(&self) {}

        fn set_current_voice_capacity(&self, _capacity: u32) {}
    }

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
    fn an_old_host_state_is_migrated_to_the_legacy_flavor() {
        let mut state = PluginState {
            version: "0.0.1".to_string(),
            params: std::collections::BTreeMap::new(),
            fields: std::collections::BTreeMap::new(),
        };
        state.params.insert(
            "osc1_wave".to_string(),
            nice_plug::plugin::ParamValue::String("Saw".to_string()),
        );
        Sushi::filter_state(&mut state);

        assert!(matches!(
            state.params.get("legacy"),
            Some(nice_plug::plugin::ParamValue::Bool(true))
        ));

        // A state without parameters has no flavor to migrate.
        let mut empty = PluginState {
            version: String::new(),
            params: std::collections::BTreeMap::new(),
            fields: std::collections::BTreeMap::new(),
        };
        Sushi::filter_state(&mut empty);
        assert!(!empty.params.contains_key("legacy"));
    }

    #[test]
    fn the_legacy_engine_is_forced_even_when_the_host_parameter_is_off() {
        let params = Arc::new(SushiParams::default());
        let setter = setter();
        setter.set_parameter(&params.legacy, false);

        let mut sushi = Sushi {
            params,
            ..Default::default()
        };
        sushi.sample_rate = SAMPLE_RATE;
        sushi.fx = Fx::new(SAMPLE_RATE);

        let mut left = vec![0.0; 64];
        let mut right = vec![0.0; 64];
        let mut buffer = Buffer::default();
        // The slices stay alive until the buffer is dropped at the end of the
        // test, so the unsafe contract of `set_slices` is upheld.
        unsafe {
            buffer.set_slices(64, |slices| {
                slices.push(&mut left);
                slices.push(&mut right);
            });
        }
        let mut aux = AuxiliaryBuffers {
            inputs: &mut [],
            outputs: &mut [],
        };
        let mut context = TestContext::new();

        let _ = Plugin::process(&mut sushi, &mut buffer, &mut aux, &mut context);

        assert_eq!(
            sushi.last_legacy,
            Some(true),
            "the process function did not force the Legacy engine"
        );
        assert!(
            !sushi.params.legacy.value(),
            "the host parameter was changed instead of being ignored"
        );
    }

    #[test]
    fn the_process_publishes_the_modulation_feedback() {
        let params = Arc::new(SushiParams::default());
        let setter = setter();
        setter.set_parameter(&params.lfo1_rate, 5.0);
        setter.set_parameter(&params.lfo2_rate, 2.0);
        setter.set_parameter(&params.macro_bright, 0.2);
        setter.set_parameter(&params.macro_motion, 0.4);
        setter.set_parameter(&params.macro_space, 0.6);
        let mut sushi = Sushi {
            params,
            ..Default::default()
        };
        sushi.sample_rate = SAMPLE_RATE;
        sushi.fx = Fx::new(SAMPLE_RATE);
        let feedback = sushi.mod_feedback();

        let mut left = vec![0.0; 1024];
        let mut right = vec![0.0; 1024];
        let mut buffer = Buffer::default();
        // The slices stay alive until the buffer is dropped at the end of the
        // test, so the unsafe contract of `set_slices` is upheld.
        unsafe {
            buffer.set_slices(1024, |slices| {
                slices.push(&mut left);
                slices.push(&mut right);
            });
        }
        let mut aux = AuxiliaryBuffers {
            inputs: &mut [],
            outputs: &mut [],
        };
        let mut context = TestContext::new();

        let _ = Plugin::process(&mut sushi, &mut buffer, &mut aux, &mut context);

        // The macros are block-rate values, so the last block contributes
        // exactly the values of the parameters.
        assert!((feedback.macro1() - 0.2).abs() < 1.0e-4);
        assert!((feedback.macro2() - 0.4).abs() < 1.0e-4);
        assert!((feedback.macro3() - 0.6).abs() < 1.0e-4);
        assert_eq!(feedback.mod_wheel(), 0.0);
        // The LFOs are sampled at the end of the block, so they are already
        // moving and stay in their bipolar range.
        assert!(feedback.lfo1().abs() <= 1.0);
        assert!(feedback.lfo2().abs() <= 1.0);
        assert!(
            feedback.lfo1() != 0.0,
            "the process function did not publish the LFO 1 value"
        );
    }

    #[test]
    fn the_synced_lfo_follows_the_host_tempo() {
        // 120 BPM is two beats per second. A quarter note is one beat, an
        // eighth note half a beat.
        assert!((lfo_sync_frequency(120.0, NoteDiv::Quarter) - 2.0).abs() < 1e-6);
        assert!((lfo_sync_frequency(120.0, NoteDiv::Eighth) - 4.0).abs() < 1e-6);
        assert!((lfo_sync_frequency(120.0, NoteDiv::Sixteenth) - 8.0).abs() < 1e-6);
        assert!((lfo_sync_frequency(90.0, NoteDiv::Eighth) - 3.0).abs() < 1e-6);

        // A broken tempo reading falls back to 120 BPM instead of freezing
        // or poisoning the phase.
        assert!((lfo_sync_frequency(f64::NAN, NoteDiv::Quarter) - 2.0).abs() < 1e-6);
    }

    #[test]
    fn a_matrix_lfo_route_suppresses_the_hidden_direct_path() {
        let params = SushiParams::default();
        let setter = setter();
        // The old direct path is still stored in the patch ...
        setter.set_parameter(&params.lfo1_amount, 0.5);
        setter.set_parameter(&params.lfo1_target, LfoTarget::Filter);
        setter.set_parameter(&params.lfo2_amount, 0.4);
        setter.set_parameter(&params.lfo2_target, LfoTarget::Pitch);
        // ... but the matrix now owns LFO 1, and LFO 2 has no route.
        setter.set_parameter(&params.mod1_source, ModSource::Lfo1);
        setter.set_parameter(&params.mod1_dest, ModDest::Cutoff);
        setter.set_parameter(&params.mod1_amount, 0.3);

        let sushi = Sushi {
            params: Arc::new(params),
            ..Default::default()
        };
        let settings = sushi.voice_settings(64, Macros::default());

        assert!(settings.lfo1_routed, "the LFO 1 route was not detected");
        assert!(!settings.lfo2_routed, "LFO 2 has no route");
    }

    #[test]
    fn an_old_patch_without_a_route_keeps_the_direct_lfo_path() {
        let params = Arc::new(SushiParams::default());
        let setter = setter();
        let patch = serde_json::json!({
            "params": {
                "lfo1_amount": 0.25,
                "lfo1_target": "Filter"
            }
        });
        crate::patch::apply_json(&params, &setter, &patch)
            .expect("the old patch could not be loaded");

        let sushi = Sushi {
            params,
            ..Default::default()
        };
        let settings = sushi.voice_settings(64, Macros::default());

        assert!(!settings.lfo1_routed, "an old patch has no matrix route");
        assert_eq!(settings.lfo1_target, LfoTarget::Filter);
        assert!((settings.lfo1_amount - 0.25).abs() < 1e-6);
    }

    #[test]
    fn the_lfos_retrigger_only_on_the_first_note_after_silence() {
        let params = Arc::new(SushiParams::default());
        let setter = setter();
        setter.set_parameter(&params.lfo1_retrigger, true);
        let mut sushi = Sushi {
            params,
            ..Default::default()
        };

        for _ in 0..100 {
            sushi.lfo1.next(5.0, SAMPLE_RATE, LfoWaveform::Sine);
        }
        assert!(sushi.lfo1.phase() > 0.0);

        // The first note of a phrase restarts the phase.
        sushi.retrigger_lfos_if_silent();
        assert_eq!(sushi.lfo1.phase(), 0.0);

        // A later note of the same chord or phrase must not jitter the shared
        // LFO, because a voice is still sounding.
        let settings = VoiceSettings::default();
        sushi.voices[0] = Some(Voice::new(
            1,
            Channel::Number(0),
            Key::Number(60),
            0,
            1.0,
            &settings,
        ));
        for _ in 0..100 {
            sushi.lfo1.next(5.0, SAMPLE_RATE, LfoWaveform::Sine);
        }
        let running_phase = sushi.lfo1.phase();
        assert!(running_phase > 0.0);
        sushi.retrigger_lfos_if_silent();
        assert_eq!(sushi.lfo1.phase(), running_phase);

        // Once the phrase is over, the next phrase retriggers again.
        sushi.voices[0] = None;
        sushi.retrigger_lfos_if_silent();
        assert_eq!(sushi.lfo1.phase(), 0.0);
    }

    #[test]
    fn the_space_macro_scales_the_wet_mix() {
        assert_eq!(space_amount(0.0), 0.0);
        assert!((space_amount(0.5) - 0.175).abs() < 1e-6);
        assert!((space_amount(0.7) - 0.245).abs() < 1e-6);
        assert!((space_amount(1.0) - 0.35).abs() < 1e-6);
    }

    #[test]
    fn the_glide_coefficient_matches_the_requested_time() {
        let coefficient = glide_coefficient(100.0, SAMPLE_RATE);
        let updates = (0.1 * SAMPLE_RATE / CONTROL_RATE as f32).round() as usize;
        let mut level = 0.0;

        // One time constant reaches about 63.2 % of the target.
        for _ in 0..updates {
            level += (1.0 - level) * coefficient;
        }

        assert!(
            (level - 0.632).abs() < 0.02,
            "the glide reached {level} after one time constant"
        );
    }

    #[test]
    fn the_mono_note_stack_remembers_the_previous_key() {
        let mut sushi = Sushi::default();
        let first = MonoNote {
            voice_id: VoiceID::Wildcard,
            channel: Channel::Number(0),
            key: Key::Number(60),
            velocity: 0.5,
        };
        let second = MonoNote {
            key: Key::Number(64),
            velocity: 0.8,
            ..first
        };

        sushi.push_mono_note(first);
        sushi.push_mono_note(second);
        assert_eq!(sushi.mono_top().map(|note| note.key), Some(Key::Number(64)));

        let released =
            sushi.remove_mono_note(VoiceID::Wildcard, Channel::Number(0), Key::Number(64));
        assert_eq!(released.map(|note| note.key), Some(Key::Number(64)));
        assert_eq!(sushi.mono_top().map(|note| note.key), Some(Key::Number(60)));

        sushi.remove_mono_note(VoiceID::Wildcard, Channel::Number(0), Key::Number(60));
        assert!(sushi.mono_top().is_none());
    }

    #[test]
    fn the_saturator_keeps_the_signal_bounded() {
        let mut sushi = Sushi::default();

        for grit in [false, true] {
            let (left, right) = sushi.saturate(10.0, -10.0, grit);
            assert!(left.is_finite() && right.is_finite());
            assert!(left.abs() <= 1.0 && right.abs() <= 1.0);

            // Silence stays exactly silent, without an offset, also when the
            // saturator starts from a clean state.
            let mut silent = Sushi::default();
            let (left, right) = silent.saturate(0.0, 0.0, grit);
            assert_eq!(left, 0.0);
            assert_eq!(right, 0.0);
        }
    }

    #[test]
    fn the_grit_character_is_harsher_than_the_silk_character() {
        let mut silk_sushi = Sushi::default();
        let (silk, _) = silk_sushi.saturate(0.1, 0.1, false);
        let mut grit_sushi = Sushi::default();
        let (grit, _) = grit_sushi.saturate(0.1, 0.1, true);
        assert!(
            grit > silk,
            "the grit character was not louder ({grit} vs {silk})"
        );

        // The grit character is a little asymmetric, so the negative half of
        // the wave is driven less hard.
        let mut silk_sushi = Sushi::default();
        let (silk, _) = silk_sushi.saturate(-0.1, -0.1, false);
        let mut grit_sushi = Sushi::default();
        let (grit, _) = grit_sushi.saturate(-0.1, -0.1, true);
        assert!(
            grit.abs() < silk.abs(),
            "the grit character was not asymmetric ({grit} vs {silk})"
        );
    }

    #[test]
    fn the_character_chain_removes_the_dc_of_grit() {
        let mut sushi = Sushi::default();
        let mut left = DcBlocker::default();
        let mut right = DcBlocker::default();
        let mut sum = 0.0;
        let mut count = 0.0;

        // A sine through the asymmetric saturator, then through the same DC
        // blocker that the output uses.
        for sample in 0..48_000 {
            let x = (sample as f32 * 440.0 / SAMPLE_RATE * std::f32::consts::TAU).sin() * 0.5;
            let (saturated_left, saturated_right) = sushi.saturate(x, x, true);
            let output = left.process(saturated_left, SAMPLE_RATE);
            let _ = right.process(saturated_right, SAMPLE_RATE);

            // Let the highpass settle before measuring the mean.
            if sample >= 4_800 {
                sum += output;
                count += 1.0;
            }
        }

        let offset = sum / count;
        assert!(
            offset.abs() < 1e-3,
            "the character chain still had an offset of {offset}"
        );
    }

    #[test]
    fn the_sustain_pedal_marks_the_matching_voice() {
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

        sushi.sustain_voices(VoiceID::Wildcard, Channel::Number(0), Key::Number(60));

        assert!(sushi.voices[0].as_ref().is_some_and(Voice::is_sustained));
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
        sushi.voices[1] = Some(Voice::new(
            2,
            Channel::Number(0),
            Key::Number(64),
            1,
            1.0,
            &settings,
        ));
        sushi.arp_voice_ids[0] = Some(2);
        assert!(!sushi.voices[0].as_ref().is_some_and(Voice::is_releasing));

        sushi.release_normal_voices();
        assert!(sushi.voices[0].as_ref().is_some_and(Voice::is_releasing));
        assert!(
            !sushi.voices[1].as_ref().is_some_and(Voice::is_releasing),
            "the arpeggiator voice was released with the normal voices"
        );
    }

    #[test]
    fn an_arp_chord_starts_and_releases_all_of_its_voices() {
        let mut sushi = Sushi::default();
        let settings = VoiceSettings::default();
        let mut context = TestContext::new();
        let chord = [
            Some((60, 1.0)),
            Some((64, 0.8)),
            Some((67, 0.6)),
            Some((72, 0.4)),
        ];

        sushi.start_arp_notes(&mut context, 0, &chord, true, &settings);

        assert_eq!(sushi.arp_voice_ids.iter().flatten().count(), 4);
        assert_eq!(sushi.voices.iter().flatten().count(), 4);
        assert!(
            sushi
                .voices
                .iter()
                .flatten()
                .all(|voice| !voice.is_releasing())
        );

        sushi.release_arp_voices();

        assert!(sushi.arp_voice_ids.iter().all(Option::is_none));
        assert!(sushi.voices.iter().flatten().all(Voice::is_releasing));
    }

    #[test]
    fn a_strummed_note_does_not_release_the_base_note() {
        let mut sushi = Sushi::default();
        let settings = VoiceSettings::default();
        let mut context = TestContext::new();

        sushi.start_arp_notes(&mut context, 0, &[Some((60, 1.0))], true, &settings);
        let base_id = sushi.arp_voice_ids[0];
        assert!(base_id.is_some());

        // The delayed octave belongs to the same step, so it is added without
        // releasing the base note.
        sushi.start_arp_notes(&mut context, 480, &[Some((72, 1.0))], false, &settings);

        assert_eq!(sushi.arp_voice_ids.iter().flatten().count(), 2);
        assert!(
            sushi
                .voices
                .iter()
                .flatten()
                .all(|voice| !voice.is_releasing()),
            "the base note was released by its strummed octave"
        );
        assert!(
            sushi
                .arp_voice_ids
                .iter()
                .flatten()
                .any(|voice_id| *voice_id == base_id.unwrap())
        );
    }

    #[test]
    fn all_notes_off_releases_the_normal_and_arp_voices() {
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
        sushi.voices[1] = Some(Voice::new(
            2,
            Channel::Number(0),
            Key::Number(64),
            1,
            1.0,
            &settings,
        ));
        sushi.arp_voice_ids[0] = Some(2);
        sushi.sustain = true;
        sushi.arp.note_on(60, 0.5, false);
        sushi.arp.note_on(64, 0.5, false);
        // Leave a strummed chord note pending as well.
        sushi.arp.tick_extended(
            120.0 / (60.0 * SAMPLE_RATE as f64),
            0.25,
            100.0,
            crate::params::ArpPattern::Up,
            1,
            0.0,
            16,
            1,
            ArpChord::Octave,
            80.0,
            1.0,
            SAMPLE_RATE,
        );
        assert!(sushi.arp.has_notes());
        assert!(sushi.arp.has_pending_notes());
        sushi.push_mono_note(MonoNote {
            voice_id: VoiceID::Wildcard,
            channel: Channel::Number(0),
            key: Key::Number(60),
            velocity: 0.5,
        });

        sushi.all_notes_off();

        assert!(sushi.voices.iter().flatten().all(Voice::is_releasing));
        assert!(sushi.arp_voice_ids.iter().all(Option::is_none));
        assert!(!sushi.arp.has_notes());
        assert!(!sushi.arp.has_pending_notes());
        assert!(!sushi.arp.is_sounding());
        assert_eq!(sushi.mono_len, 0);
        assert!(!sushi.sustain);
    }

    #[test]
    fn resetting_the_plugin_clears_the_arp_chord_and_the_strum() {
        let mut sushi = Sushi::default();
        sushi.arp.note_on(60, 1.0, false);
        sushi.arp.tick_extended(
            120.0 / (60.0 * SAMPLE_RATE as f64),
            0.25,
            100.0,
            crate::params::ArpPattern::Up,
            1,
            0.0,
            16,
            1,
            ArpChord::Major,
            80.0,
            1.0,
            SAMPLE_RATE,
        );
        sushi.arp_voice_ids[0] = Some(FIRST_ARP_VOICE_ID);
        assert!(sushi.arp.has_pending_notes());

        sushi.reset();

        assert!(sushi.arp_voice_ids.iter().all(Option::is_none));
        assert!(!sushi.arp.has_notes());
        assert!(!sushi.arp.has_pending_notes());
        assert!(!sushi.arp.is_sounding());
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

        // In the middle the tilt leaves the balance alone.
        let low = tilt(100.0, 0.0);
        let high = tilt(5_000.0, 0.0);
        assert!(
            (low - high).abs() < low * 0.1,
            "the tone tilt changed the balance ({low} vs {high})"
        );
    }

    #[test]
    fn the_modulation_matrix_reaches_the_voices() {
        let params = SushiParams::default();
        let setter = setter();
        setter.set_parameter(&params.mod1_source, ModSource::Macro1);
        setter.set_parameter(&params.mod1_dest, ModDest::Pitch);
        setter.set_parameter(&params.mod1_amount, 0.5);
        setter.set_parameter(&params.mod3_source, ModSource::Lfo1);
        setter.set_parameter(&params.mod3_dest, ModDest::Cutoff);
        setter.set_parameter(&params.mod3_amount, -0.25);

        let sushi = Sushi {
            params: Arc::new(params),
            ..Default::default()
        };

        let settings = sushi.voice_settings(64, Macros::default());

        assert_eq!(settings.mod_slots[0].source, ModSource::Macro1);
        assert_eq!(settings.mod_slots[0].dest, ModDest::Pitch);
        assert!(
            (settings.mod_slots[0].amount - 0.5).abs() < 0.01,
            "the amount of the first slot was {}",
            settings.mod_slots[0].amount
        );

        assert_eq!(settings.mod_slots[2].source, ModSource::Lfo1);
        assert_eq!(settings.mod_slots[2].dest, ModDest::Cutoff);
        assert!(
            (settings.mod_slots[2].amount + 0.25).abs() < 0.01,
            "the amount of the third slot was {}",
            settings.mod_slots[2].amount
        );

        // The slots that were not touched are still switched off.
        assert_eq!(settings.mod_slots[1].source, ModSource::Off);
        assert_eq!(settings.mod_slots[1].dest, ModDest::Off);
        assert_eq!(settings.mod_slots[1].amount, 0.0);
    }

    #[test]
    fn the_macros_reach_the_voices() {
        let params = SushiParams::default();

        let sushi = Sushi {
            params: Arc::new(params),
            ..Default::default()
        };

        let settings = sushi.voice_settings(
            64,
            Macros {
                bright: 0.2,
                motion: 0.4,
                space: 0.6,
            },
        );

        assert!((settings.macro1 - 0.2).abs() < 0.001);
        assert!((settings.macro2 - 0.4).abs() < 0.001);
        assert!((settings.macro3 - 0.6).abs() < 0.001);
    }

    #[test]
    fn the_dsp_batch_parameters_reach_the_voice_settings() {
        let params = SushiParams::default();
        let setter = setter();
        setter.set_parameter(&params.ring_mod, 0.6);
        setter.set_parameter(&params.osc_phase_reset, true);
        setter.set_parameter(&params.ae_curve, -0.5);
        setter.set_parameter(&params.fe_curve, 0.5);
        setter.set_parameter(&params.tune, 12.0);

        let sushi = Sushi {
            params: Arc::new(params),
            ..Default::default()
        };
        let settings = sushi.voice_settings(64, Macros::default());

        assert!((settings.ring_mod - 0.6).abs() < 1e-6);
        assert!(settings.osc_phase_reset);
        assert!((settings.amp_env_curve - curve_exponent(-0.5)).abs() < 1e-6);
        assert!((settings.filter_env_curve - curve_exponent(0.5)).abs() < 1e-6);
        assert!((settings.tune_semitones - 12.0).abs() < 0.01);
    }

    #[test]
    fn the_fx_chain_parameters_reach_the_effect_chain() {
        let params = Arc::new(SushiParams::default());
        let setter = setter();
        setter.set_parameter(&params.fx_order, 4);
        setter.set_parameter(&params.fx_bypass_mask, 9);
        setter.set_parameter(&params.fx_global_mix, 0.25);
        setter.set_parameter(&params.drive_tone, 0.5);
        setter.set_parameter(&params.phaser_feedback, 0.4);
        setter.set_parameter(&params.chorus_width, 0.75);
        setter.set_parameter(&params.delay_damping, 0.6);
        setter.set_parameter(&params.reverb_tone, -0.5);
        setter.set_parameter(&params.comp_threshold, -30.0);
        setter.set_parameter(&params.comp_ratio, 4.0);
        setter.set_parameter(&params.comp_attack, 12.0);
        setter.set_parameter(&params.comp_release, 180.0);
        setter.set_parameter(&params.comp_makeup, 3.0);
        setter.set_parameter(&params.comp_mix, 0.8);
        setter.set_parameter(&params.trem_rate, 6.0);
        setter.set_parameter(&params.trem_depth, 0.7);
        setter.set_parameter(&params.trem_stereo, true);

        let sushi = Sushi {
            params,
            ..Default::default()
        };
        let fx_params = sushi.fx_params();

        assert_eq!(fx_params.order, 4);
        assert_eq!(fx_params.bypass_mask, 9);
        assert!((fx_params.global_mix - 0.25).abs() < 1e-6);
        assert!((fx_params.drive_tone - 0.5).abs() < 1e-6);
        assert!((fx_params.phaser_feedback - 0.4).abs() < 1e-6);
        assert!((fx_params.chorus_width - 0.75).abs() < 1e-6);
        assert!((fx_params.delay_damping - 0.6).abs() < 1e-6);
        assert!((fx_params.reverb_tone + 0.5).abs() < 1e-6);
        assert!((fx_params.comp_threshold + 30.0).abs() < 1e-6);
        assert!((fx_params.comp_ratio - 4.0).abs() < 1e-6);
        assert!((fx_params.comp_attack - 12.0).abs() < 1e-6);
        assert!((fx_params.comp_release - 180.0).abs() < 1e-6);
        assert!((fx_params.comp_makeup - 3.0).abs() < 1e-6);
        assert!((fx_params.comp_mix - 0.8).abs() < 1e-6);
        assert!((fx_params.trem_rate - 6.0).abs() < 1e-6);
        assert!((fx_params.trem_depth - 0.7).abs() < 1e-6);
        assert!(fx_params.trem_stereo);

        // The defaults keep the old chain and the old dry/wet behavior, and
        // both new modules are neutral.
        let defaults = Sushi::default().fx_params();
        assert_eq!(defaults.order, 0);
        assert_eq!(defaults.bypass_mask, 0);
        assert_eq!(defaults.global_mix, 1.0);
        assert_eq!(defaults.drive_tone, 0.0);
        assert_eq!(defaults.phaser_feedback, 0.0);
        assert_eq!(defaults.chorus_width, 0.0);
        assert_eq!(defaults.delay_damping, 0.0);
        assert_eq!(defaults.reverb_tone, 0.0);
        assert_eq!(defaults.comp_mix, 0.0);
        assert_eq!(defaults.trem_depth, 0.0);
    }

    #[test]
    fn the_limit_of_the_voices_is_enforced() {
        let params = SushiParams::default();
        let setter = setter();
        setter.set_parameter(&params.voices, 2);

        let mut sushi = Sushi {
            params: Arc::new(params),
            ..Default::default()
        };
        let settings = VoiceSettings::default();

        // The limit is read once per block.
        sushi.update_voice_limit();

        // Four notes, but only two voices may play at the same time.
        for key in 0..4u8 {
            let (slot, stolen) = sushi.reserve_voice_slot();

            // The oldest voice is the one that makes room for the new one.
            if let Some(stolen) = stolen {
                assert!(
                    stolen.internal_voice_id() < key as u64,
                    "the voice {} was stolen",
                    stolen.internal_voice_id()
                );
            }

            sushi.fill_voice_slot(
                slot,
                VoiceID::ID(key as i32),
                Channel::Number(0),
                Key::Number(60 + key),
                1.0,
                &settings,
            );
        }

        // Only the two newest voices are still playing.
        assert_eq!(sushi.voices.iter().flatten().count(), 2);
        let ids: Vec<u64> = sushi
            .voices
            .iter()
            .flatten()
            .map(|voice| voice.internal_voice_id())
            .collect();
        assert_eq!(ids, [2, 3]);
    }
}
