
use nice_plug::editor::dpi::{LogicalSize, NativeSize, Size};
use nice_plug::context::gui::GuiContext;
use nice_plug::editor::{HostCallbacks, HostMethods, SpawnedEditor};
use std::error::Error;
use nice_plug::midi::{Channel, Key, VoiceID};
use nice_plug::prelude::*;
use nice_plug_egui::{
    EguiEditor, EguiEditorHandle, EguiEditorState, EguiNiceSettings, RepaintNotifier,
    create_egui_editor,
};
use std::sync::Mutex;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A pair of callbacks the panel can use to ask the host for a new window
/// size. The handle is shared with the window, which forwards the request to
/// the CLAP gui extension of the host.
type Callbacks = Option<Box<dyn HostCallbacks>>;

#[derive(Clone)]
struct CallbackSlot(Arc<Mutex<Callbacks>>);

unsafe impl Send for CallbackSlot {}
unsafe impl Sync for CallbackSlot {}

impl CallbackSlot {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(None)))
    }
    fn set(&self, callbacks: Box<dyn HostCallbacks>) {
        if let Ok(mut slot) = self.0.lock() {
            *slot = Some(callbacks);
        }
    }
    fn take(&self) -> Option<Box<dyn HostCallbacks>> {
        self.0.lock().ok().and_then(|mut slot| slot.take())
    }
    fn put(&self, callbacks: Box<dyn HostCallbacks>) {
        if let Ok(mut slot) = self.0.lock() {
            *slot = Some(callbacks);
        }
    }
}

#[derive(Clone)]
struct WindowResize(CallbackSlot);

// The callbacks are only ever used from the window thread, and the mutex
// serializes access, so sharing the handle with the app is sound.
unsafe impl Send for WindowResize {}
unsafe impl Sync for WindowResize {}

impl WindowResize {
    fn request(&self, width: f32, height: f32) {
        if let Some(mut callbacks) = self.0.take() {
            let _ = callbacks.request_resize(
                Size::Logical(LogicalSize::new(width as f64, height as f64)),
                1.0,
            );
            self.0.put(callbacks);
        }
    }
}

/// The callbacks that the window actually holds: it forwards to the host.
struct SharedCallbacks(CallbackSlot);

impl HostCallbacks for SharedCallbacks {
    fn request_resize(&mut self, new_size: Size, scale_factor: f64) -> Result<(), Box<dyn Error>> {
        match self.0.take() {
            Some(mut callbacks) => {
                let result = callbacks.request_resize(new_size, scale_factor);
                self.0.put(callbacks);
                result
            }
            None => Err("the host has no window callbacks".into()),
        }
    }

    fn destroyed(&mut self) {
        if let Some(mut callbacks) = self.0.take() {
            callbacks.destroyed();
            self.0.put(callbacks);
        }
    }
}

/// The editor of the plugin: the egui editor with a hook that lets the panel
/// resize its own window.
pub struct SushiEditors {
    inner: EguiEditor<SushiEditor>,
    callbacks: CallbackSlot,
}

impl Editor for SushiEditors {
    type Handle = EguiEditorHandle;

    fn spawn(
        &self,
        parent: Option<ParentWindowHandle>,
        wait_for_parent: bool,
        fallback_scale_factor: Option<f64>,
        gui_context: GuiContext,
        host: Option<HostMethods>,
    ) -> Result<SpawnedEditor<Self::Handle>, Box<dyn Error>> {
        let host = host.map(|host| {
            self.callbacks.set(host.callbacks);
            HostMethods {
                callbacks: Box::new(SharedCallbacks(self.callbacks.clone())),
                main_thread_caller: host.main_thread_caller,
            }
        });
        self.inner
            .spawn(parent, wait_for_parent, fallback_scale_factor, gui_context, host)
    }

    fn size(&self) -> NativeSize<u32> {
        self.inner.size()
    }

    fn resize_hint(&self) -> ResizeHint {
        self.inner.resize_hint()
    }

    fn track_info_updated(&self, info: TrackInfo) {
        self.inner.track_info_updated(info);
    }
}

pub mod dsp;
mod editor;
mod keyboard;
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
use keyboard::{HeldKeys, KeyboardNote};
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
    /// The random source of the modulation matrix.
    random_source: dsp::random_source::SmoothRandom,
    /// The last value of the random source, `-1..1`.
    random_value: f32,
    /// The seed of the last block, so a new seed can be noticed.
    random_seed: u32,
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
    /// The notes of the keyboards of the panel: the editor produces them and
    /// the audio thread consumes them here. The consumer never leaves the
    /// plugin, so a note that is queued while the editor closes is still
    /// played and released instead of staying in the ring buffer.
    keyboard_tx: Option<rtrb::Producer<KeyboardNote>>,
    keyboard_rx: rtrb::Consumer<KeyboardNote>,
    /// The keys that are playing, for the lights of the panel. Every source
    /// of notes writes into it.
    held_keys: Arc<HeldKeys>,
}

impl Default for Sushi {
    fn default() -> Self {
        let (scope_tx, scope_rx) = rtrb::RingBuffer::new(SCOPE_CAPACITY);
        let (keyboard_tx, keyboard_rx) = rtrb::RingBuffer::new(keyboard::QUEUE_CAPACITY);

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
            random_source: dsp::random_source::SmoothRandom::new(0x51ED_270B),
            random_value: 0.0,
            random_seed: 0,
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

            // Open the editor a little larger than the 960x680 design so
            // the panel is comfortable on a normal DAW screen and still fits
            // under a 1080p window. The design scales continuously from its
            // 960x680 minimum.
            // The window opens at exactly the default zoom level, so the
            // panel fills it without stretching.
            editor_state: EguiEditorState::from_size(
                LogicalSize::new(
                    WINDOW_WIDTH * crate::editor::DEFAULT_ZOOM_SCALE,
                    WINDOW_HEIGHT * crate::editor::DEFAULT_ZOOM_SCALE,
                ),
                1.0,
            ),
            repaint_notifier: RepaintNotifier::new(),
            scope_tx,
            scope_rx: Some(scope_rx),
            keyboard_tx: Some(keyboard_tx),
            keyboard_rx,
            held_keys: Arc::new(HeldKeys::default()),
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
            // The random source of the matrix, running at the block rate.
            random: self.random_value,
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
        self.held_keys.clear();
    }

    #[allow(clippy::too_many_arguments)]
    fn note_on(
        &mut self,
        context: &mut impl ProcessContext<Self>,
        timing: u32,
        voice_id: VoiceID,
        channel: Channel,
        key: Key,
        velocity: f32,
        arp_on: bool,
        arp_latch: bool,
        settings: &VoiceSettings,
    ) {
        let note = key.number_or_middle_c();
        self.held_keys.press(note);

        if arp_on {
            // The arpeggiator collects the notes, the voices are started by
            // the arpeggiator itself.
            self.arp.note_on(note, velocity, arp_latch);
        } else {
            self.retrigger_lfos_if_silent();
            self.start_voice(context, timing, voice_id, channel, key, velocity, settings);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn note_off(
        &mut self,
        context: &mut impl ProcessContext<Self>,
        timing: u32,
        voice_id: VoiceID,
        channel: Channel,
        key: Key,
        arp_on: bool,
        poly_mode: PolyMode,
        settings: &VoiceSettings,
    ) {
        let note = key.number_or_middle_c();
        self.held_keys.release(note);

        if arp_on {
            self.arp.note_off(note);
        } else if poly_mode == PolyMode::Poly {
            if self.sustain {
                self.sustain_voices(voice_id, channel, key);
            } else {
                self.release_voices(voice_id, channel, key);
            }
        } else {
            self.release_mono_note(context, timing, voice_id, channel, key, settings);
        }
    }

    /// Applies the notes that the keyboards of the panel produced since the
    /// last block. They enter the same path as the note events of the host
    /// and are placed at the start of the block.
    fn drain_keyboard(
        &mut self,
        context: &mut impl ProcessContext<Self>,
        arp_on: bool,
        arp_latch: bool,
        poly_mode: PolyMode,
        settings: &VoiceSettings,
    ) {
        while let Ok(note) = self.keyboard_rx.pop() {
            let key = Key::Number(note.key.min(127));
            if note.on {
                self.note_on(
                    context,
                    0,
                    VoiceID::Wildcard,
                    Channel::Number(0),
                    key,
                    note.velocity.clamp(0.0, 1.0),
                    arp_on,
                    arp_latch,
                    settings,
                );
            } else {
                self.note_off(
                    context,
                    0,
                    VoiceID::Wildcard,
                    Channel::Number(0),
                    key,
                    arp_on,
                    poly_mode,
                    settings,
                );
            }
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

    type Editor = SushiEditors;
    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Self::Editor> {
        let scope_rx = self.scope_rx.take()?;
        let keyboard_tx = self.keyboard_tx.take()?;

        let callbacks = CallbackSlot::new();
        let resize = Arc::new(WindowResize(callbacks.clone()));

        let inner = create_egui_editor(
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
                keyboard_tx,
                self.held_keys.clone(),
                self.arp_led.clone(),
                self.arp_position.clone(),
                self.voice_count.clone(),
                self.mod_feedback.clone(),
                Some(Arc::new(move |width: f32, height: f32| resize.request(width, height))),
            ),
        )?;
        Some(SushiEditors { inner, callbacks })
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
        self.held_keys.clear();
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
        // The random source walks once per block, which is plenty for a slow
        // control signal.
        let block_seconds = num_samples as f32 / self.sample_rate.max(1.0);
        // A new seed restarts the sequence; the same seed always draws the same
        // values, so a sound can be recalled.
        let seed = self.params.random_seed.value().max(1) as u32;
        if seed != self.random_seed {
            self.random_seed = seed;
            self.random_source.reseed(seed);
        }
        let random = self.random_source.next(
            self.params.random_rate.smoothed.next(),
            self.params.random_smooth.smoothed.next(),
            block_seconds,
        );
        // The unipolar mode lifts the swing out of the middle, which is what
        // most sources of a synth offer as a switch.
        self.random_value = if self.params.random_unipolar.value() {
            random * 0.5 + 0.5
        } else {
            random
        };
        for sample_idx in 0..num_samples {
            // The notes of the panel are applied at the start of the block,
            // like an event that the host sends at sample zero.
            if sample_idx == 0 {
                self.drain_keyboard(context, arp_on, arp_latch, poly_mode, &settings);
            }

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
                    } => self.note_on(
                        context, timing, voice_id, channel, key, velocity, arp_on, arp_latch,
                        &settings,
                    ),
                    NoteEvent::NoteOff {
                        timing,
                        voice_id,
                        channel,
                        key,
                        ..
                    } => self.note_off(
                        context, timing, voice_id, channel, key, arp_on, poly_mode, &settings,
                    ),
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
        // The envelopes belong to the voices, so the newest sounding voice
        // speaks for all of them.
        let newest = self.voices.iter().flatten().next_back();
        let (mod_env, amp_env) = match newest {
            Some(voice) => (voice.mod_env_level(), voice.amp_env_level()),
            None => (0.0, 0.0),
        };
        // The note sources speak with the voice of the newest note, so the
        // panel can show what velocity, key and random are doing right now.
        let (velocity, key, random) = match newest {
            Some(voice) => (voice.velocity_value(), voice.key_number(), self.random_value),
            None => (0.0, 60.0, 0.0),
        };
        self.mod_feedback.store(ModFeedbackValues {
            lfo1: last_lfo1,
            lfo2: last_lfo2,
            macro1: bright,
            macro2: motion,
            macro3: space,
            mod_wheel: self.mod_wheel,
            mod_env,
            amp_env,
            velocity,
            key,
            random,
            voice_active: newest.is_some(),
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

nice_export_clap!(Sushi);

