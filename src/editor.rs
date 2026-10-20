//! The Sushi editor: a retro Win95/Pascal-flavoured panel.
//!
//! Every continuous parameter is a vertical slider whose handle is a band that
//! is fully embedded inside the groove (it never sticks out), every parameter
//! can also be typed into a small numeric field, and every choice parameter
//! opens a popup menu. The parameters are grouped into labelled boxes below a
//! header that holds the wordmark, the factory presets and a small
//! oscilloscope.

mod presets;

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Layout, Margin, Pos2, Rect, Sense, Stroke,
};
use nice_plug::context::gui::GuiContext;
use nice_plug::prelude::*;
use nice_plug_egui::NiceEguiApp;

use crate::params::SushiParams;
use presets::{PRESETS, Rng};

// --- Win95 / Pascal grey palette -------------------------------------------

const BG: Color32 = Color32::from_rgb(0xC0, 0xC0, 0xC0);
const BUTTON: Color32 = Color32::from_rgb(0xD2, 0xD2, 0xD2);
const FIELD: Color32 = Color32::from_rgb(0xFB, 0xFB, 0xFB);
const LIGHT: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);
const SHADOW: Color32 = Color32::from_rgb(0x80, 0x80, 0x80);
const DARK: Color32 = Color32::from_rgb(0x40, 0x40, 0x40);
const TEXT: Color32 = Color32::from_rgb(0x10, 0x10, 0x10);
const LABEL: Color32 = Color32::from_rgb(0x28, 0x28, 0x28);
const MUTED: Color32 = Color32::from_rgb(0x60, 0x60, 0x60);
const TRACE: Color32 = Color32::from_rgb(0x33, 0x33, 0x33);
const SELECTION: Color32 = Color32::from_rgb(0x0A, 0x24, 0x6A);
/// The background of the title strip of a group box.
const TITLE_BG: Color32 = Color32::from_rgb(0xB8, 0xB8, 0xB8);
/// The background of the display of the presets.
const LCD_BG: Color32 = Color32::from_rgb(0x0B, 0x1A, 0x0B);
/// The color of the text in the display of the presets.
const LCD_FG: Color32 = Color32::from_rgb(0x40, 0xE0, 0x60);
/// The color of the light of the activity meter.
const LED_GREEN: Color32 = Color32::from_rgb(0x30, 0xD0, 0x40);
/// The color of the light of the clip meter.
const LED_RED: Color32 = Color32::from_rgb(0xE0, 0x30, 0x30);
/// The color of the lights of the sequencer and of the arpeggiator.
const LED_AMBER: Color32 = Color32::from_rgb(0xE0, 0xA0, 0x20);
/// The color of a light that is switched off.
const LED_OFF: Color32 = Color32::from_rgb(0x50, 0x50, 0x50);

// --- Layout -----------------------------------------------------------------

pub const WINDOW_WIDTH: f32 = 960.0;
pub const WINDOW_HEIGHT: f32 = 640.0;

/// The margin around the whole panel.
const MARGIN: f32 = 8.0;
/// The height of the header.
const HEADER_HEIGHT: f32 = 48.0;
/// The width of a group box.
const GROUP_WIDTH: f32 = 230.0;
/// The width of a group box that holds two columns. The two columns are as
/// wide as a single group box, so that the rows line up with the content.
const DOUBLE_GROUP_WIDTH: f32 = 2.0 * GROUP_WIDTH + GROUP_GAP;
/// The gap between two group boxes.
const GROUP_GAP: f32 = 8.0;
/// The padding inside a group box.
const GROUP_PADDING: f32 = 4.0;
/// The height of the title strip of a group box.
const TITLE_HEIGHT: f32 = 16.0;
/// The horizontal gap between two parameter columns.
const COLUMN_GAP: f32 = 4.0;

/// The height of the first row of group boxes.
const ROW_A_HEIGHT: f32 = 146.0;
/// The height of the second row of group boxes.
const ROW_B_HEIGHT: f32 = 134.0;
/// The height of the third row of group boxes.
const ROW_C_HEIGHT: f32 = 138.0;
/// The height of the fourth row of group boxes.
const ROW_D_HEIGHT: f32 = 126.0;

// --- Widget metrics ---------------------------------------------------------

/// The width of a vertical slider.
const SLIDER_WIDTH: f32 = 16.0;
/// The width of the sliders of the macros.
const MACRO_SLIDER_WIDTH: f32 = 24.0;
/// The width of the sliders of the steps of the sequencer.
const MINI_SLIDER_WIDTH: f32 = 12.0;
/// The height of the sliders of the steps of the sequencer.
const MINI_SLIDER_HEIGHT: f32 = 52.0;
/// The height of the slider handle.
const HANDLE_HEIGHT: f32 = 10.0;
/// How far the handle is kept away from the edges of the groove.
const GROOVE_INSET: f32 = 3.0;
/// The size of the numeric value fields.
const FIELD_WIDTH: f32 = 40.0;
const FIELD_HEIGHT: f32 = 14.0;
/// The size of the numeric value fields of the steps.
const MINI_FIELD_WIDTH: f32 = 24.0;
const MINI_FIELD_HEIGHT: f32 = 12.0;
/// The height of the captions above the controls.
const LABEL_HEIGHT: f32 = 11.0;
/// The height of the captions of the sub sections.
const SECTION_HEIGHT: f32 = 12.0;
/// The diameter of the small lights.
const LED_SIZE: f32 = 6.0;
/// The number of the lights of the voice meter.
const VOICE_LEDS: usize = 16;
/// The width of a light of the voice meter.
const VOICE_LED_WIDTH: f32 = 6.0;
/// The height of a light of the voice meter.
const VOICE_LED_HEIGHT: f32 = 8.0;
/// The gap between two lights of the voice meter.
const VOICE_LED_GAP: f32 = 2.0;
/// The gap between the caption of the voice meter and its lights.
const VOICE_METER_GAP: f32 = 8.0;
/// The size of a check box.
const CHECK_SIZE: f32 = 12.0;
/// The height of the buttons of the radio pair.
const RADIO_HEIGHT: f32 = 14.0;

/// The height of the sliders of the first row.
const SLIDER_A: f32 = 80.0;
/// The height of the sliders of the second row.
const SLIDER_B: f32 = 72.0;
/// The height of the sliders of the third row.
const SLIDER_C: f32 = 76.0;
/// The height of the sliders of the effects.
const SLIDER_FX: f32 = 58.0;
/// The height of the sliders of the macros.
const SLIDER_MACRO: f32 = 68.0;
/// The height of the sliders of the output.
const SLIDER_OUT: f32 = 70.0;
/// The height of the sliders of the low frequency oscillators.
const SLIDER_LFO: f32 = 64.0;
/// The height of the sliders of the arpeggiator.
const SLIDER_ARP: f32 = 68.0;

/// How many scope samples are kept around.
const HISTORY_LEN: usize = 4096;

/// How long the light of the arpeggiator stays on.
const ARP_LIGHT_TIME: Duration = Duration::from_millis(120);

/// The ids of the eight steps of the sequencer.
const STEP_IDS: [&str; 8] = [
    "step_1", "step_2", "step_3", "step_4", "step_5", "step_6", "step_7", "step_8",
];

// --- The editor -------------------------------------------------------------

/// The number of widgets of every kind that were drawn in the last frame.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default)]
struct Counts {
    sliders: usize,
    mini: usize,
    popups: usize,
    checks: usize,
    radios: usize,
}

/// The kind of a widget that is counted by the tests.
#[cfg(test)]
#[derive(Clone, Copy)]
enum Widget {
    Slider,
    Mini,
    Popup,
    Check,
    Radio,
}

/// The state that only the editor needs.
struct EditorState {
    /// The text buffers of the numeric value fields, keyed by parameter id.
    texts: HashMap<&'static str, String>,
    /// The parameters that currently have an open automation gesture.
    gestures: HashSet<&'static str>,
    /// The index of the factory preset that is loaded.
    preset_index: usize,
    /// Whether the sound was mutated after the preset was loaded.
    dirty: bool,
    /// The random number generator of the mutation.
    rng: Rng,
    /// The number of arpeggiator triggers that was seen last.
    arp_counter: usize,
    /// Whether the light of the arpeggiator is lit.
    arp_light: bool,
    /// When the light of the arpeggiator was switched on.
    arp_light_at: Instant,
    /// The peak level of the samples of the last frame.
    activity: f32,
    /// Whether the samples of the last frame were clipping.
    clipping: f32,
    /// The widgets that were drawn in the last frame.
    #[cfg(test)]
    counts: Counts,
    /// The places of the check boxes of the last frame.
    #[cfg(test)]
    check_rects: Vec<egui::Rect>,
    /// The places of the radio buttons of the last frame.
    #[cfg(test)]
    radio_rects: Vec<egui::Rect>,
    /// The places of the small sliders of the steps of the last frame.
    #[cfg(test)]
    mini_rects: Vec<egui::Rect>,
}

impl EditorState {
    /// Counts a widget of the last frame.
    #[cfg(test)]
    fn count(&mut self, widget: Widget) {
        let counts = &mut self.counts;
        match widget {
            Widget::Slider => counts.sliders += 1,
            Widget::Mini => counts.mini += 1,
            Widget::Popup => counts.popups += 1,
            Widget::Check => counts.checks += 1,
            Widget::Radio => counts.radios += 1,
        }
    }
}

pub struct SushiEditor {
    params: Arc<SushiParams>,
    scope_rx: rtrb::Consumer<f32>,
    history: VecDeque<f32>,
    gui_ctx: Option<GuiContext>,
    state: EditorState,
    /// The index of the step that the sequencer is playing.
    step_index: Arc<AtomicUsize>,
    /// The number of arpeggiator triggers.
    arp_led: Arc<AtomicUsize>,
    /// The number of voices that are currently playing.
    voice_count: Arc<AtomicUsize>,
}

impl SushiEditor {
    /// Creates a new editor for the given parameters.
    pub fn new(
        params: Arc<SushiParams>,
        scope_rx: rtrb::Consumer<f32>,
        step_index: Arc<AtomicUsize>,
        arp_led: Arc<AtomicUsize>,
        voice_count: Arc<AtomicUsize>,
    ) -> Self {
        Self {
            params,
            scope_rx,
            history: VecDeque::with_capacity(HISTORY_LEN),
            gui_ctx: None,
            state: EditorState {
                texts: HashMap::new(),
                gestures: HashSet::new(),
                preset_index: 0,
                dirty: false,
                rng: Rng::new(0x2545_F491),
                arp_counter: 0,
                arp_light: false,
                arp_light_at: Instant::now(),
                activity: 0.0,
                clipping: 0.0,
                #[cfg(test)]
                counts: Counts::default(),
                #[cfg(test)]
                check_rects: Vec::new(),
                #[cfg(test)]
                radio_rects: Vec::new(),
                #[cfg(test)]
                mini_rects: Vec::new(),
            },
            step_index,
            arp_led,
            voice_count,
        }
    }

    /// Takes the samples of the scope out of the ring buffer.
    ///
    /// The peak level and the clipping are kept for the two lights of the
    /// header. They fade away when the synthesizer goes quiet.
    fn collect_scope(&mut self) {
        let mut peak = 0.0f32;
        let mut clipped = false;
        let mut received = false;

        while let Ok(sample) = self.scope_rx.pop() {
            received = true;
            peak = peak.max(sample.abs());
            clipped |= sample.abs() > 0.98;

            if self.history.len() == HISTORY_LEN {
                self.history.pop_front();
            }
            self.history.push_back(sample);
        }

        if received {
            self.state.activity = peak;
            self.state.clipping = if clipped {
                1.0
            } else {
                self.state.clipping * 0.5
            };
        } else {
            self.state.activity *= 0.5;
            self.state.clipping *= 0.5;
        }
    }

    /// Switches the light of the arpeggiator on when the arpeggiator plays a
    /// note.
    fn update_arp_light(&mut self) {
        let counter = self.arp_led.load(Ordering::Relaxed);
        if counter != self.state.arp_counter {
            self.state.arp_counter = counter;
            self.state.arp_light = true;
            self.state.arp_light_at = Instant::now();
        } else if self.state.arp_light_at.elapsed() > ARP_LIGHT_TIME {
            self.state.arp_light = false;
        }
    }
}

impl NiceEguiApp for SushiEditor {
    fn build(
        &mut self,
        egui_ctx: egui::Context,
        nice_gui_ctx: GuiContext,
        _frame: &mut nice_plug_egui::Frame,
    ) -> Result<(), nice_plug_egui::baseview::HandlerError> {
        self.gui_ctx = Some(nice_gui_ctx);
        while self.scope_rx.pop().is_ok() {}
        self.history.clear();

        apply_theme(&egui_ctx);
        Ok(())
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut nice_plug_egui::Frame) {
        self.draw(ui);
    }
}

impl SushiEditor {
    /// Draws the whole panel.
    fn draw(&mut self, ui: &mut egui::Ui) {
        let Some(gui_ctx) = self.gui_ctx.clone() else {
            return;
        };
        let setter = gui_ctx.param_setter();

        self.collect_scope();
        self.update_arp_light();
        ui.ctx().request_repaint_after(Duration::from_millis(16));

        let full = ui.max_rect();
        ui.painter().rect_filled(full, CornerRadius::ZERO, BG);

        // The origin of the panel. The layout is fixed, so the panel stays in
        // the upper left corner when the window is bigger than the panel.
        let origin = full.min;
        let width = WINDOW_WIDTH - 2.0 * MARGIN;

        // The rows of group boxes.
        let mut y = MARGIN + HEADER_HEIGHT + GROUP_GAP;
        let row_a_rect = Rect::from_min_size(
            origin + egui::vec2(MARGIN, y),
            egui::vec2(width, ROW_A_HEIGHT),
        );
        y += ROW_A_HEIGHT + GROUP_GAP;
        let row_b_rect = Rect::from_min_size(
            origin + egui::vec2(MARGIN, y),
            egui::vec2(width, ROW_B_HEIGHT),
        );
        y += ROW_B_HEIGHT + GROUP_GAP;
        let row_c_rect = Rect::from_min_size(
            origin + egui::vec2(MARGIN, y),
            egui::vec2(width, ROW_C_HEIGHT),
        );
        y += ROW_C_HEIGHT + GROUP_GAP;
        let row_d_rect = Rect::from_min_size(
            origin + egui::vec2(MARGIN, y),
            egui::vec2(width, ROW_D_HEIGHT),
        );

        let header_rect = Rect::from_min_size(
            origin + egui::vec2(MARGIN, MARGIN),
            egui::vec2(width, HEADER_HEIGHT),
        );

        // Split the borrow of the editor, so that the parameters, the state of
        // the panel and the text buffers can be used at the same time.
        let params = self.params.as_ref();
        let history = &self.history;
        let step_index = &self.step_index;
        let voice_count = &self.voice_count;
        let mut ctx = Ctx {
            params,
            setter: &setter,
            state: &mut self.state,
            history,
            step_index,
            voice_count,
        };

        ui.scope_builder(
            egui::UiBuilder::new()
                .id_salt("sushi-header")
                .max_rect(header_rect)
                .layout(Layout::top_down(Align::Min)),
            |ui| header(ui, header_rect, &mut ctx),
        );

        row_a(ui, row_a_rect, &mut ctx);
        row_b(ui, row_b_rect, &mut ctx);
        row_c(ui, row_c_rect, &mut ctx);
        row_d(ui, row_d_rect, &mut ctx);
    }
}

// --- The header -------------------------------------------------------------

/// Draws the header: the wordmark, the presets and the oscilloscope.
fn header(ui: &mut egui::Ui, rect: Rect, ctx: &mut Ctx) {
    let painter = ui.painter_at(rect);

    // The wordmark. The second copy makes it look a little bolder.
    let wordmark = FontId::proportional(14.0);
    let wordmark_width = painter
        .layout_no_wrap("SUSHI".to_string(), wordmark.clone(), TEXT)
        .size()
        .x;
    painter.text(
        rect.left_top() + egui::vec2(6.0, 9.0),
        Align2::LEFT_TOP,
        "SUSHI",
        wordmark.clone(),
        TEXT,
    );
    painter.text(
        rect.left_top() + egui::vec2(6.6, 9.0),
        Align2::LEFT_TOP,
        "SUSHI",
        wordmark.clone(),
        TEXT,
    );
    painter.text(
        rect.left_top() + egui::vec2(7.0, 27.0),
        Align2::LEFT_TOP,
        "POLYPHONIC SYNTHESIZER",
        FontId::proportional(8.0),
        MUTED,
    );

    // The hint that tells the user that a double click resets a parameter. It
    // sits next to the wordmark, where there is still enough room.
    painter.text(
        egui::pos2(rect.left() + 6.6 + wordmark_width + 14.0, rect.top() + 17.0),
        Align2::LEFT_CENTER,
        "DBL-CLICK = RESET",
        FontId::proportional(8.0),
        MUTED,
    );

    // The presets.
    let preset_x = rect.left() + 150.0;
    let button_y = rect.top() + (HEADER_HEIGHT - 28.0) * 0.5;
    let previous = Rect::from_min_size(egui::pos2(preset_x, button_y), egui::vec2(22.0, 28.0));
    let display = Rect::from_min_size(
        egui::pos2(preset_x + 26.0, button_y),
        egui::vec2(160.0, 28.0),
    );
    let next = Rect::from_min_size(
        egui::pos2(preset_x + 190.0, button_y),
        egui::vec2(22.0, 28.0),
    );
    let mutate = Rect::from_min_size(
        egui::pos2(preset_x + 216.0, button_y),
        egui::vec2(62.0, 28.0),
    );

    let index = ctx.state.preset_index;
    let name = PRESETS[index].name;
    let text = format!(
        "{:02} {name}{}",
        index + 1,
        if ctx.state.dirty { "*" } else { "" }
    );
    lcd(&painter, display, &text);

    if button(ui, previous, "<").clicked() {
        ctx.load_preset(index + PRESETS.len() - 1);
    }
    if button(ui, next, ">").clicked() {
        ctx.load_preset(index + 1);
    }
    if button(ui, mutate, "MUTATE").clicked() {
        ctx.mutate();
    }

    // The meter that shows how many voices are playing. It sits in the free
    // space between the presets and the lights of the activity meter.
    let meter = Rect::from_min_max(
        egui::pos2(mutate.right() + 8.0, rect.top()),
        egui::pos2(rect.right() - 194.0, rect.bottom()),
    );
    voice_meter(
        &painter,
        meter,
        ctx.voice_count.load(Ordering::Relaxed).min(VOICE_LEDS),
    );

    // The oscilloscope and the two lights.
    let scope = Rect::from_min_size(
        egui::pos2(rect.right() - 154.0, rect.top() + 5.0),
        egui::vec2(150.0, 38.0),
    );
    small_scope(&painter, scope, ctx.history);

    let activity = egui::pos2(rect.right() - 190.0, rect.top() + 16.0);
    led(&painter, activity, LED_GREEN, ctx.state.activity > 0.002);
    painter.text(
        egui::pos2(activity.x, rect.top() + 28.0),
        Align2::CENTER_CENTER,
        "ACT",
        FontId::proportional(8.0),
        MUTED,
    );

    let clipping = egui::pos2(rect.right() - 162.0, rect.top() + 16.0);
    led(&painter, clipping, LED_RED, ctx.state.clipping > 0.5);
    painter.text(
        egui::pos2(clipping.x, rect.top() + 28.0),
        Align2::CENTER_CENTER,
        "CLIP",
        FontId::proportional(8.0),
        MUTED,
    );
}

// --- The rows of group boxes ------------------------------------------------

/// The first row: the two oscillators, the filter and the mixer.
fn row_a(ui: &mut egui::Ui, rect: Rect, ctx: &mut Ctx) {
    group_row(ui, "row-a", rect, &[GROUP_WIDTH; 4], |index, rect, ui| {
        let p = ctx.params;
        match index {
            0 => group(ui, rect, "OSC 1", |ui, part| match part {
                Part::Header => ctx.popup(ui, "osc1_wave", &p.osc1_wave, 66.0),
                Part::Body => column_row(ui, "osc1", 5, COLUMN_GAP, |index, ui| {
                    let p = ctx.params;
                    match index {
                        0 => ctx.int(
                            ui,
                            "osc1_octave",
                            "Oct",
                            &p.osc1_octave,
                            &Knob::plain(-2.0, 2.0, 0, SLIDER_A),
                        ),
                        1 => ctx.int(
                            ui,
                            "osc1_semi",
                            "Semi",
                            &p.osc1_semi,
                            &Knob::plain(-12.0, 12.0, 0, SLIDER_A),
                        ),
                        2 => ctx.float(
                            ui,
                            "osc1_fine",
                            "Fine",
                            &p.osc1_fine,
                            &Knob::plain(-100.0, 100.0, 0, SLIDER_A),
                        ),
                        3 => ctx.float(
                            ui,
                            "osc1_pw",
                            "PW",
                            &p.osc1_pw,
                            &Knob::scaled(0.05, 0.95, 0, SLIDER_A),
                        ),
                        4 => ctx.float(
                            ui,
                            "osc1_level",
                            "Lvl",
                            &p.osc1_level,
                            &Knob::percent(SLIDER_A),
                        ),
                        _ => {}
                    }
                }),
            }),
            1 => group(ui, rect, "OSC 2", |ui, part| match part {
                Part::Header => ctx.popup(ui, "osc2_wave", &p.osc2_wave, 66.0),
                Part::Body => column_row(ui, "osc2", 5, COLUMN_GAP, |index, ui| {
                    let p = ctx.params;
                    match index {
                        0 => ctx.int(
                            ui,
                            "osc2_octave",
                            "Oct",
                            &p.osc2_octave,
                            &Knob::plain(-2.0, 2.0, 0, SLIDER_A),
                        ),
                        1 => ctx.int(
                            ui,
                            "osc2_semi",
                            "Semi",
                            &p.osc2_semi,
                            &Knob::plain(-12.0, 12.0, 0, SLIDER_A),
                        ),
                        2 => ctx.float(
                            ui,
                            "osc2_fine",
                            "Fine",
                            &p.osc2_fine,
                            &Knob::plain(-100.0, 100.0, 0, SLIDER_A),
                        ),
                        3 => ctx.float(
                            ui,
                            "osc2_pw",
                            "PW",
                            &p.osc2_pw,
                            &Knob::scaled(0.05, 0.95, 0, SLIDER_A),
                        ),
                        4 => ctx.float(
                            ui,
                            "osc2_level",
                            "Lvl",
                            &p.osc2_level,
                            &Knob::percent(SLIDER_A),
                        ),
                        _ => {}
                    }
                }),
            }),
            2 => group(ui, rect, "FILTER", |ui, part| match part {
                Part::Header => ctx.popup(ui, "filter_mode", &p.filter_mode, 70.0),
                Part::Body => column_row(ui, "filter", 5, COLUMN_GAP, |index, ui| {
                    let p = ctx.params;
                    match index {
                        0 => ctx.float(
                            ui,
                            "cutoff",
                            "Cut",
                            &p.cutoff,
                            &Knob::logarithmic(20.0, 20_000.0, 0, SLIDER_A),
                        ),
                        1 => ctx.float(
                            ui,
                            "resonance",
                            "Res",
                            &p.resonance,
                            &Knob::percent(SLIDER_A),
                        ),
                        2 => ctx.float(
                            ui,
                            "filter_env_amount",
                            "Env",
                            &p.filter_env_amount,
                            &Knob::bipolar(SLIDER_A),
                        ),
                        3 => ctx.float(
                            ui,
                            "filter_keytrack",
                            "Key",
                            &p.filter_keytrack,
                            &Knob::percent(SLIDER_A),
                        ),
                        4 => ctx.float(
                            ui,
                            "filter_drive",
                            "Drv",
                            &p.filter_drive,
                            &Knob::percent(SLIDER_A),
                        ),
                        _ => {}
                    }
                }),
            }),
            _ => group(ui, rect, "MIX", |ui, part| match part {
                Part::Header => {}
                Part::Body => column_row(ui, "mix", 4, COLUMN_GAP, |index, ui| {
                    let p = ctx.params;
                    match index {
                        0 => ctx.float(
                            ui,
                            "noise_level",
                            "Noise",
                            &p.noise_level,
                            &Knob::percent(SLIDER_A),
                        ),
                        1 => ctx.int(
                            ui,
                            "unison",
                            "Uni",
                            &p.unison,
                            &Knob::plain(1.0, 4.0, 0, SLIDER_A),
                        ),
                        2 => ctx.float(
                            ui,
                            "detune",
                            "Detune",
                            &p.detune,
                            &Knob::plain(0.0, 50.0, 0, SLIDER_A),
                        ),
                        3 => ctx.float(ui, "spread", "Spread", &p.spread, &Knob::percent(SLIDER_A)),
                        _ => {}
                    }
                }),
            }),
        }
    });
}

/// The second row: the two envelopes and the low frequency oscillators.
fn row_b(ui: &mut egui::Ui, rect: Rect, ctx: &mut Ctx) {
    group_row(
        ui,
        "row-b",
        rect,
        &[GROUP_WIDTH, GROUP_WIDTH, DOUBLE_GROUP_WIDTH],
        |index, rect, ui| {
            match index {
                0 => group(ui, rect, "FILTER ENV", |ui, part| match part {
                    Part::Header => text_label(ui, "ADSR", 9.0, LABEL),
                    Part::Body => column_row(ui, "fe", 4, COLUMN_GAP, |index, ui| {
                        let p = ctx.params;
                        match index {
                            0 => ctx.float(
                                ui,
                                "fe_attack",
                                "A",
                                &p.fe_attack,
                                &Knob::logarithmic(0.0, 4_000.0, 1, SLIDER_B),
                            ),
                            1 => ctx.float(
                                ui,
                                "fe_decay",
                                "D",
                                &p.fe_decay,
                                &Knob::logarithmic(0.0, 4_000.0, 1, SLIDER_B),
                            ),
                            2 => ctx.float(
                                ui,
                                "fe_sustain",
                                "S",
                                &p.fe_sustain,
                                &Knob::percent(SLIDER_B),
                            ),
                            3 => ctx.float(
                                ui,
                                "fe_release",
                                "R",
                                &p.fe_release,
                                &Knob::logarithmic(0.0, 4_000.0, 1, SLIDER_B),
                            ),
                            _ => {}
                        }
                    }),
                }),
                1 => group(ui, rect, "AMP ENV", |ui, part| match part {
                    Part::Header => {}
                    Part::Body => column_row(ui, "ae", 4, COLUMN_GAP, |index, ui| {
                        let p = ctx.params;
                        match index {
                            0 => ctx.float(
                                ui,
                                "ae_attack",
                                "A",
                                &p.ae_attack,
                                &Knob::logarithmic(0.0, 4_000.0, 1, SLIDER_B),
                            ),
                            1 => ctx.float(
                                ui,
                                "ae_decay",
                                "D",
                                &p.ae_decay,
                                &Knob::logarithmic(0.0, 4_000.0, 1, SLIDER_B),
                            ),
                            2 => ctx.float(
                                ui,
                                "ae_sustain",
                                "S",
                                &p.ae_sustain,
                                &Knob::percent(SLIDER_B),
                            ),
                            3 => ctx.float(
                                ui,
                                "ae_release",
                                "R",
                                &p.ae_release,
                                &Knob::logarithmic(0.0, 8_000.0, 1, SLIDER_B),
                            ),
                            _ => {}
                        }
                    }),
                }),
                _ => group(ui, rect, "LFO 1 + LFO 2", |ui, part| match part {
                    Part::Header => {}
                    Part::Body => sections(ui, "lfo", 2, GROUP_GAP, |index, _rect, ui| {
                        let p = ctx.params;
                        let (title, wave, target, rate, amount, ids) = match index {
                            0 => (
                                "LFO 1",
                                &p.lfo1_wave,
                                &p.lfo1_target,
                                &p.lfo1_rate,
                                &p.lfo1_amount,
                                ("lfo1_wave", "lfo1_target", "lfo1_rate", "lfo1_amount"),
                            ),
                            _ => (
                                "LFO 2",
                                &p.lfo2_wave,
                                &p.lfo2_target,
                                &p.lfo2_rate,
                                &p.lfo2_amount,
                                ("lfo2_wave", "lfo2_target", "lfo2_rate", "lfo2_amount"),
                            ),
                        };

                        // The header of the sub section: its name and the two
                        // popups of the oscillator.
                        let header = Rect::from_min_size(
                            ui.next_widget_position(),
                            egui::vec2(ui.available_width(), FIELD_HEIGHT),
                        );
                        ui.scope_builder(
                            egui::UiBuilder::new()
                                .id_salt(("sushi-lfo-header", index))
                                .max_rect(header)
                                .layout(Layout::left_to_right(Align::Center)),
                            |ui| {
                                ui.spacing_mut().item_spacing = egui::vec2(4.0, 0.0);
                                text_label(ui, title, 9.0, LABEL);
                                ctx.popup(ui, ids.0, wave, 58.0);
                                ctx.popup(ui, ids.1, target, 74.0);
                            },
                        );

                        column_row(ui, ids.2, 2, COLUMN_GAP, |index, ui| match index {
                            0 => ctx.float(
                                ui,
                                ids.2,
                                "Rate",
                                rate,
                                &Knob::logarithmic(0.01, 40.0, 2, SLIDER_LFO),
                            ),
                            1 => ctx.float(ui, ids.3, "Amount", amount, &Knob::percent(SLIDER_LFO)),
                            _ => {}
                        });
                    }),
                }),
            }
        },
    );
}

/// The third row: the step sequencer, the arpeggiator and the voice.
fn row_c(ui: &mut egui::Ui, rect: Rect, ctx: &mut Ctx) {
    group_row(
        ui,
        "row-c",
        rect,
        &[DOUBLE_GROUP_WIDTH, GROUP_WIDTH, GROUP_WIDTH],
        |index, rect, ui| {
            let p = ctx.params;
            match index {
                0 => group(ui, rect, "STEP", |ui, part| match part {
                    Part::Header => {
                        ctx.check(ui, "step_on", &p.step_on, "ON");
                        ctx.popup(ui, "step_rate", &p.step_rate, 58.0);
                        ctx.popup(ui, "step_dest", &p.step_dest, 68.0);
                        ctx.check(ui, "step_smooth", &p.step_smooth, "SMOOTH");
                    }
                    Part::Body => steps(ui, ctx),
                }),
                1 => group(ui, rect, "ARP", |ui, part| match part {
                    Part::Header => {
                        ctx.check(ui, "arp_on", &p.arp_on, "ON");
                        ctx.check(ui, "arp_latch", &p.arp_latch, "LATCH");
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(LED_SIZE, LED_SIZE), Sense::hover());
                        led(
                            &ui.painter_at(rect),
                            rect.center(),
                            LED_AMBER,
                            ctx.state.arp_light,
                        );
                    }
                    Part::Body => {
                        ctx.popup(ui, "arp_pattern", &p.arp_pattern, 84.0);
                        ctx.popup(ui, "arp_rate", &p.arp_rate, 58.0);
                        column_row(ui, "arp", 2, COLUMN_GAP, |index, ui| {
                            let p = ctx.params;
                            match index {
                                0 => ctx.int(
                                    ui,
                                    "arp_octaves",
                                    "Oct",
                                    &p.arp_octaves,
                                    &Knob::plain(1.0, 3.0, 0, SLIDER_ARP),
                                ),
                                1 => ctx.float(
                                    ui,
                                    "arp_gate",
                                    "Gate",
                                    &p.arp_gate,
                                    &Knob::plain(5.0, 100.0, 0, SLIDER_ARP),
                                ),
                                _ => {}
                            }
                        });
                    }
                }),
                _ => group(ui, rect, "VOICE", |ui, part| match part {
                    Part::Header => ctx.popup(ui, "poly_mode", &p.poly_mode, 66.0),
                    Part::Body => column_row(ui, "voice", 3, COLUMN_GAP, |index, ui| {
                        let p = ctx.params;
                        match index {
                            0 => ctx.float(
                                ui,
                                "glide",
                                "Glide",
                                &p.glide,
                                &Knob::logarithmic(0.0, 2_000.0, 1, SLIDER_C),
                            ),
                            1 => {
                                ctx.float(ui, "drift", "Drift", &p.drift, &Knob::percent(SLIDER_C))
                            }
                            2 => ctx.float(
                                ui,
                                "velocity",
                                "Vel",
                                &p.velocity,
                                &Knob::percent(SLIDER_C),
                            ),
                            _ => {}
                        }
                    }),
                }),
            }
        },
    );
}

/// The fourth row: the effects, the macros and the output.
fn row_d(ui: &mut egui::Ui, rect: Rect, ctx: &mut Ctx) {
    group_row(
        ui,
        "row-d",
        rect,
        &[DOUBLE_GROUP_WIDTH, GROUP_WIDTH, GROUP_WIDTH],
        |index, rect, ui| match index {
            0 => group(ui, rect, "FX", |ui, part| match part {
                Part::Header => {}
                Part::Body => sections(ui, "fx", 3, GROUP_GAP, |index, _rect, ui| match index {
                    0 => {
                        caption(ui, "CHORUS", 9.0, LABEL);
                        column_row(ui, "chorus", 3, COLUMN_GAP, |index, ui| {
                            let p = ctx.params;
                            match index {
                                0 => ctx.float(
                                    ui,
                                    "chorus_mix",
                                    "Mix",
                                    &p.chorus_mix,
                                    &Knob::percent(SLIDER_FX),
                                ),
                                1 => ctx.float(
                                    ui,
                                    "chorus_rate",
                                    "Rate",
                                    &p.chorus_rate,
                                    &Knob::logarithmic(0.05, 8.0, 2, SLIDER_FX),
                                ),
                                2 => ctx.float(
                                    ui,
                                    "chorus_depth",
                                    "Depth",
                                    &p.chorus_depth,
                                    &Knob::percent(SLIDER_FX),
                                ),
                                _ => {}
                            }
                        });
                    }
                    1 => {
                        caption(ui, "DELAY", 9.0, LABEL);
                        column_row(ui, "delay", 3, COLUMN_GAP, |index, ui| {
                            let p = ctx.params;
                            match index {
                                0 => ctx.float(
                                    ui,
                                    "delay_time",
                                    "Time",
                                    &p.delay_time,
                                    &Knob::logarithmic(10.0, 1_500.0, 1, SLIDER_FX),
                                ),
                                1 => ctx.float(
                                    ui,
                                    "delay_feedback",
                                    "FB",
                                    &p.delay_feedback,
                                    &Knob::percent(SLIDER_FX),
                                ),
                                2 => ctx.float(
                                    ui,
                                    "delay_mix",
                                    "Mix",
                                    &p.delay_mix,
                                    &Knob::percent(SLIDER_FX),
                                ),
                                _ => {}
                            }
                        });
                    }
                    _ => {
                        caption(ui, "REVERB", 9.0, LABEL);
                        column_row(ui, "reverb", 2, COLUMN_GAP, |index, ui| {
                            let p = ctx.params;
                            match index {
                                0 => ctx.float(
                                    ui,
                                    "reverb_decay",
                                    "Decay",
                                    &p.reverb_decay,
                                    &Knob::percent(SLIDER_FX),
                                ),
                                1 => ctx.float(
                                    ui,
                                    "reverb_mix",
                                    "Mix",
                                    &p.reverb_mix,
                                    &Knob::percent(SLIDER_FX),
                                ),
                                _ => {}
                            }
                        });
                    }
                }),
            }),
            1 => group(ui, rect, "MACROS", |ui, part| match part {
                Part::Header => {}
                Part::Body => column_row(ui, "macros", 3, COLUMN_GAP, |index, ui| {
                    let p = ctx.params;
                    let knob = Knob::percent(SLIDER_MACRO).with_width(MACRO_SLIDER_WIDTH);
                    match index {
                        0 => ctx.float(ui, "macro_bright", "Brilliance", &p.macro_bright, &knob),
                        1 => ctx.float(ui, "macro_motion", "Motion", &p.macro_motion, &knob),
                        2 => ctx.float(ui, "macro_space", "Space", &p.macro_space, &knob),
                        _ => {}
                    }
                }),
            }),
            _ => group(ui, rect, "OUTPUT", |ui, part| match part {
                Part::Header => {
                    let p = ctx.params;
                    text_label(ui, "CHARACTER", 9.0, LABEL);
                    ctx.radio_pair(ui, "character", &p.character, 70.0);
                }
                Part::Body => column_row(ui, "output", 2, COLUMN_GAP, |index, ui| {
                    let p = ctx.params;
                    match index {
                        0 => ctx.float(
                            ui,
                            "volume",
                            "Volume",
                            &p.volume,
                            &Knob::decibels(-60.0, 6.0, SLIDER_OUT),
                        ),
                        1 => ctx.float(ui, "tone", "Tone", &p.tone, &Knob::bipolar(SLIDER_OUT)),
                        _ => {}
                    }
                }),
            }),
        },
    );
}

/// Draws the eight steps of the sequencer and the amount next to them.
fn steps(ui: &mut egui::Ui, ctx: &mut Ctx) {
    let p = ctx.params;
    let parameters = [
        &p.step_1, &p.step_2, &p.step_3, &p.step_4, &p.step_5, &p.step_6, &p.step_7, &p.step_8,
    ];

    let start = ui.next_widget_position();
    let width = ui.available_width();
    let height = ui.available_height();
    let amount_width = 64.0;
    let steps_width = width - amount_width - GROUP_GAP;
    let slot = steps_width / 8.0;

    let current = ctx.step_index.load(Ordering::Relaxed);
    let playing = p.step_on.value();

    for index in 0..8 {
        let rect = Rect::from_min_size(
            egui::pos2(start.x + index as f32 * slot, start.y),
            egui::vec2(slot, height),
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .id_salt(("sushi-step", index))
                .max_rect(rect)
                .layout(Layout::top_down(Align::Center)),
            |ui| {
                ctx.mini_step(
                    ui,
                    STEP_IDS[index],
                    index + 1,
                    parameters[index],
                    playing && current == index,
                );
            },
        );
    }

    // The amount of the sequencer.
    let rect = Rect::from_min_size(
        egui::pos2(start.x + steps_width + GROUP_GAP, start.y),
        egui::vec2(amount_width, height),
    );
    ui.scope_builder(
        egui::UiBuilder::new()
            .id_salt("sushi-step-amount")
            .max_rect(rect)
            .layout(Layout::top_down(Align::Center)),
        |ui| {
            let p = ctx.params;
            ctx.float(
                ui,
                "step_amount",
                "Amount",
                &p.step_amount,
                &Knob::percent(SLIDER_C),
            );
        },
    );

    ui.advance_cursor_after_rect(Rect::from_min_size(start, egui::vec2(width, height)));
}

// --- The widgets of the parameters ------------------------------------------

/// The range and the look of a continuous parameter.
#[derive(Clone, Copy)]
struct Knob {
    /// The lowest value of the parameter.
    min: f32,
    /// The highest value of the parameter.
    max: f32,
    /// Whether the range of the slider is logarithmic.
    log: bool,
    /// The number of decimals of the numeric field.
    decimals: usize,
    /// The conversion of the value of the parameter into the shown value.
    to_display: fn(f32) -> f32,
    /// The conversion of the shown value into the value of the parameter.
    from_display: fn(f32) -> f32,
    /// The height of the slider.
    height: f32,
    /// The width of the slider.
    width: f32,
}

impl Knob {
    /// A parameter that is shown as it is.
    fn plain(min: f32, max: f32, decimals: usize, height: f32) -> Self {
        Self {
            min,
            max,
            log: false,
            decimals,
            to_display: identity,
            from_display: identity,
            height,
            width: SLIDER_WIDTH,
        }
    }

    /// A parameter that is shown as a percentage.
    fn percent(height: f32) -> Self {
        Self {
            min: 0.0,
            max: 1.0,
            log: false,
            decimals: 0,
            to_display: to_percent,
            from_display: from_percent,
            height,
            width: SLIDER_WIDTH,
        }
    }

    /// A parameter that is shown as a signed percentage.
    fn bipolar(height: f32) -> Self {
        Self {
            min: -1.0,
            max: 1.0,
            log: false,
            decimals: 0,
            to_display: to_percent,
            from_display: from_percent,
            height,
            width: SLIDER_WIDTH,
        }
    }

    /// A parameter that is stored in another range than it is shown in.
    fn scaled(min: f32, max: f32, decimals: usize, height: f32) -> Self {
        Self {
            min,
            max,
            log: false,
            decimals,
            to_display: to_percent,
            from_display: from_percent,
            height,
            width: SLIDER_WIDTH,
        }
    }

    /// A parameter with a logarithmic range.
    fn logarithmic(min: f32, max: f32, decimals: usize, height: f32) -> Self {
        Self {
            min,
            max,
            log: true,
            decimals,
            to_display: identity,
            from_display: identity,
            height,
            width: SLIDER_WIDTH,
        }
    }

    /// A parameter that is shown in decibels but stored as a gain.
    fn decibels(min: f32, max: f32, height: f32) -> Self {
        Self {
            min: util::db_to_gain(min),
            max: util::db_to_gain(max),
            log: true,
            decimals: 1,
            to_display: util::gain_to_db,
            from_display: util::db_to_gain,
            height,
            width: SLIDER_WIDTH,
        }
    }

    /// The same knob, but with another width.
    fn with_width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// Converts the value of the parameter into the shown value.
    fn display(&self, value: f32) -> f32 {
        (self.to_display)(value)
    }

    /// Converts a shown value into the value of the parameter.
    fn parse(&self, value: f32) -> f32 {
        (self.from_display)(value)
    }
}

/// Everything a parameter widget needs to draw itself.
struct Ctx<'a, 'b> {
    params: &'a SushiParams,
    setter: &'b ParamSetter<'b>,
    state: &'a mut EditorState,
    /// The samples of the oscilloscope.
    history: &'a VecDeque<f32>,
    /// The index of the step that the sequencer is playing.
    step_index: &'a Arc<AtomicUsize>,
    /// The number of voices that are currently playing.
    voice_count: &'a Arc<AtomicUsize>,
}

impl<'a, 'b> Ctx<'a, 'b> {
    /// Draws a continuous parameter: a caption, a vertical slider and a
    /// numeric field that mirrors (or replaces) the value of the parameter.
    fn float(
        &mut self,
        ui: &mut egui::Ui,
        id: &'static str,
        label: &str,
        param: &FloatParam,
        knob: &Knob,
    ) {
        #[cfg(test)]
        self.state.count(Widget::Slider);

        caption(ui, label, 9.0, LABEL);

        let mut value = param.value();
        let response = v_slider(ui, &mut value, knob);
        // A double click resets the parameter to its factory default. The
        // reset is done after the value of the click, so that it wins.
        let reset = response.double_clicked();
        if self.begin_gesture(id, &response, ui) {
            self.setter.begin_set_parameter(param);
        }
        if response.changed() {
            self.setter.set_parameter(param, value);
        }
        if reset {
            self.reset_to_default(id, param);
        }
        if self.end_gesture(id, &response, ui) {
            self.setter.end_set_parameter(param);
        }

        self.float_field(ui, id, param, knob, FIELD_WIDTH, FIELD_HEIGHT);
    }

    /// Draws a stepped parameter. It works just like [`Ctx::float`], but the
    /// value is rounded to whole numbers.
    fn int(
        &mut self,
        ui: &mut egui::Ui,
        id: &'static str,
        label: &str,
        param: &IntParam,
        knob: &Knob,
    ) {
        #[cfg(test)]
        self.state.count(Widget::Slider);

        caption(ui, label, 9.0, LABEL);

        let mut value = param.value() as f32;
        let response = v_slider(ui, &mut value, knob);
        // A double click resets the parameter to its factory default. The
        // reset is done after the value of the click, so that it wins.
        let reset = response.double_clicked();
        if self.begin_gesture(id, &response, ui) {
            self.setter.begin_set_parameter(param);
        }
        if response.changed() {
            self.setter.set_parameter(param, value.round() as i32);
        }
        if reset {
            self.reset_to_default(id, param);
        }
        if self.end_gesture(id, &response, ui) {
            self.setter.end_set_parameter(param);
        }

        self.int_field(ui, id, param, knob, FIELD_WIDTH, FIELD_HEIGHT);
    }

    /// Draws one step of the sequencer: a light, a small slider, the number
    /// of the step and a small numeric field.
    fn mini_step(
        &mut self,
        ui: &mut egui::Ui,
        id: &'static str,
        number: usize,
        param: &FloatParam,
        lit: bool,
    ) {
        #[cfg(test)]
        self.state.count(Widget::Mini);

        // The light that shows the step that is playing.
        let (rect, _) = ui.allocate_exact_size(egui::vec2(LED_SIZE, LED_SIZE), Sense::hover());
        led(&ui.painter_at(rect), rect.center(), LED_AMBER, lit);

        // The slider.
        let knob = Knob::percent(MINI_SLIDER_HEIGHT).with_width(MINI_SLIDER_WIDTH);
        let mut value = param.value();
        let response = v_slider(ui, &mut value, &knob);
        #[cfg(test)]
        self.state.mini_rects.push(response.rect);
        // A double click resets the parameter to its factory default. The
        // reset is done after the value of the click, so that it wins.
        let reset = response.double_clicked();
        if self.begin_gesture(id, &response, ui) {
            self.setter.begin_set_parameter(param);
        }
        if response.changed() {
            self.setter.set_parameter(param, value);
        }
        if reset {
            self.reset_to_default(id, param);
        }
        if self.end_gesture(id, &response, ui) {
            self.setter.end_set_parameter(param);
        }

        // The number of the step.
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(MINI_FIELD_WIDTH, SECTION_HEIGHT), Sense::hover());
        ui.painter_at(rect).text(
            rect.center(),
            Align2::CENTER_CENTER,
            number.to_string(),
            FontId::proportional(8.0),
            LABEL,
        );

        // The numeric field.
        self.float_field(ui, id, param, &knob, MINI_FIELD_WIDTH, MINI_FIELD_HEIGHT);
    }

    /// Draws the numeric field of a continuous parameter.
    fn float_field(
        &mut self,
        ui: &mut egui::Ui,
        id: &'static str,
        param: &FloatParam,
        knob: &Knob,
        width: f32,
        height: f32,
    ) {
        let text = self.state.texts.entry(id).or_default();
        let field = field_id(id);

        // While the field has the focus it is never overwritten. Otherwise it
        // mirrors the parameter, but only when the parameter actually changed,
        // so that a value that was just typed isn't reverted before the host
        // has applied it.
        if !ui.memory(|memory| memory.has_focus(field)) {
            sync_text(ui, id, text, knob.decimals, knob.display(param.value()));
        }

        let response = number_field(ui, text, id, field, width, height);

        if response.lost_focus() {
            // The value was typed in: parse it and hand it to the host.
            match text.trim().parse::<f32>() {
                Ok(parsed) if parsed.is_finite() => {
                    let plain = knob.parse(parsed);
                    self.setter.begin_set_parameter(param);
                    self.setter.set_parameter(param, plain);
                    self.setter.end_set_parameter(param);
                    *text = format_number(knob.display(plain), knob.decimals);
                }
                _ => {
                    // Nonsense was typed: fall back to the parameter's value.
                    *text = format_number(knob.display(param.value()), knob.decimals);
                }
            }
        }
    }

    /// Draws the numeric field of a stepped parameter.
    fn int_field(
        &mut self,
        ui: &mut egui::Ui,
        id: &'static str,
        param: &IntParam,
        knob: &Knob,
        width: f32,
        height: f32,
    ) {
        let text = self.state.texts.entry(id).or_default();
        let field = field_id(id);

        if !ui.memory(|memory| memory.has_focus(field)) {
            sync_text(
                ui,
                id,
                text,
                knob.decimals,
                knob.display(param.value() as f32),
            );
        }

        let response = number_field(ui, text, id, field, width, height);

        if response.lost_focus() {
            match text.trim().parse::<f32>() {
                Ok(parsed) if parsed.is_finite() => {
                    let plain = knob.parse(parsed).round() as i32;
                    self.setter.begin_set_parameter(param);
                    self.setter.set_parameter(param, plain);
                    self.setter.end_set_parameter(param);
                    *text = format_number(plain as f32, knob.decimals);
                }
                _ => {
                    *text = format_number(param.value() as f32, knob.decimals);
                }
            }
        }
    }

    /// Draws a popup button that opens the list of choices.
    fn popup<T: Enum + PartialEq + Copy + 'static>(
        &mut self,
        ui: &mut egui::Ui,
        id: &'static str,
        param: &EnumParam<T>,
        width: f32,
    ) {
        #[cfg(test)]
        self.state.count(Widget::Popup);

        let current = param.value();
        let variants = T::variants();

        let response = egui::ComboBox::from_id_salt(id)
            .width(width)
            .selected_text(variants[T::to_index(current)])
            .truncate()
            .show_ui(ui, |ui| {
                for (index, name) in variants.iter().enumerate() {
                    let variant = T::from_index(index);
                    if ui.selectable_label(current == variant, *name).clicked() {
                        self.setter.set_parameter(param, variant);
                    }
                }
            })
            .response;

        // A double click on the button of the popup resets the parameter to
        // its factory default.
        if response.double_clicked() {
            self.setter
                .set_parameter(param, param.default_plain_value());
        }
    }

    /// Draws a check box with a label. Clicking it switches the parameter.
    fn check(&mut self, ui: &mut egui::Ui, id: &'static str, param: &BoolParam, label: &str) {
        #[cfg(test)]
        self.state.count(Widget::Check);

        let (rect, _) = ui.allocate_exact_size(egui::vec2(CHECK_SIZE, CHECK_SIZE), Sense::hover());
        #[cfg(test)]
        self.state.check_rects.push(rect);
        let response = ui.interact(
            rect,
            ui.scope_id().with(("sushi-check", id)),
            Sense::click(),
        );
        let painter = ui.painter_at(rect);
        sunken(&painter, rect, FIELD);
        if param.value() {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                "X",
                FontId::monospace(9.0),
                TEXT,
            );
        }

        if response.double_clicked() {
            // A double click resets the parameter to its factory default,
            // even though the two clicks would switch it twice.
            self.setter
                .set_parameter(param, param.default_plain_value());
        } else if response.clicked() {
            self.setter.set_parameter(param, !param.value());
        }

        text_label(ui, label, 9.0, LABEL);
    }

    /// Draws a pair of square buttons of which one is active.
    fn radio_pair<T: Enum + PartialEq + Copy + 'static>(
        &mut self,
        ui: &mut egui::Ui,
        id: &'static str,
        param: &EnumParam<T>,
        width: f32,
    ) {
        #[cfg(test)]
        self.state.count(Widget::Radio);

        let variants = T::variants();
        let current = T::to_index(param.value());
        let button_width = width / variants.len() as f32;

        for (index, variant) in variants.iter().enumerate() {
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(button_width, RADIO_HEIGHT), Sense::hover());
            #[cfg(test)]
            self.state.radio_rects.push(rect);
            let response = ui.interact(
                rect,
                ui.scope_id().with(("sushi-radio", id, index)),
                Sense::click(),
            );
            let painter = ui.painter_at(rect);

            // The active button is sunken.
            if index == current || response.is_pointer_button_down_on() {
                sunken(&painter, rect, BUTTON);
            } else {
                raised(&painter, rect, BUTTON);
            }
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                // The name of the variant is shown in uppercase, like the
                // other labels of the panel.
                variant.to_ascii_uppercase(),
                FontId::proportional(9.0),
                TEXT,
            );

            if response.double_clicked() {
                // A double click resets the parameter to its factory default.
                self.setter
                    .set_parameter(param, param.default_plain_value());
            } else if response.clicked() {
                self.setter.set_parameter(param, T::from_index(index));
            }
        }
    }

    /// Loads the factory preset with the given index.
    fn load_preset(&mut self, index: usize) {
        let index = index % PRESETS.len();
        self.state.preset_index = index;
        self.state.dirty = false;
        presets::apply(self.params, self.setter, index);
    }

    /// Mutates the current sound a little bit.
    fn mutate(&mut self) {
        presets::mutate(self.params, self.setter, &mut self.state.rng);
        self.state.dirty = true;
    }

    /// Puts a parameter back to its factory default. The reset is done
    /// inside of the automation gesture of the widget, so that the host
    /// records it like any other change of the parameter.
    fn reset_to_default<P: Param>(&mut self, id: &'static str, param: &P) {
        if !self.state.gestures.contains(id) {
            // The gesture was already closed (a very fast click), so the
            // reset gets a gesture of its own.
            self.setter.begin_set_parameter(param);
            self.setter
                .set_parameter(param, param.default_plain_value());
            self.setter.end_set_parameter(param);
            return;
        }

        self.setter
            .set_parameter(param, param.default_plain_value());
    }

    /// Opens the automation gesture of a parameter. The gesture starts when
    /// the user presses the widget, so that a plain click is recorded by the
    /// host as well.
    fn begin_gesture(
        &mut self,
        id: &'static str,
        response: &egui::Response,
        ui: &egui::Ui,
    ) -> bool {
        if self.state.gestures.contains(id) {
            return false;
        }

        let pressed = ui.input(|input| input.pointer.any_pressed());
        let started = response.drag_started() || (pressed && response.is_pointer_button_down_on());
        if !started {
            return false;
        }

        self.state.gestures.insert(id);
        true
    }

    /// Closes the automation gesture of a parameter. The gesture ends when
    /// the user releases the widget again.
    fn end_gesture(&mut self, id: &'static str, response: &egui::Response, ui: &egui::Ui) -> bool {
        if !self.state.gestures.contains(id) {
            return false;
        }

        let released = response.drag_stopped()
            || response.clicked()
            || ui.input(|input| input.pointer.any_released());
        if !released {
            return false;
        }

        self.state.gestures.remove(id);
        true
    }
}

// --- The layout -------------------------------------------------------------

/// The two parts of a group box.
#[derive(Clone, Copy)]
enum Part {
    Header,
    Body,
}

/// Lays out a row of group boxes of the given widths.
fn group_row(
    ui: &mut egui::Ui,
    name: &str,
    rect: Rect,
    widths: &[f32],
    mut add: impl FnMut(usize, Rect, &mut egui::Ui),
) {
    let mut x = rect.left();
    for (index, width) in widths.iter().enumerate() {
        let box_rect =
            Rect::from_min_size(egui::pos2(x, rect.top()), egui::vec2(*width, rect.height()));
        x += width + GROUP_GAP;

        ui.scope_builder(
            egui::UiBuilder::new()
                .id_salt(("sushi-group", name, index))
                .max_rect(box_rect)
                .layout(Layout::top_down(Align::Min)),
            |ui| add(index, box_rect, ui),
        );
    }
}

/// Draws a group box and hands its header and body to the given callback.
fn group(ui: &mut egui::Ui, rect: Rect, title: &str, mut add: impl FnMut(&mut egui::Ui, Part)) {
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::ZERO, BG);
    raised(&painter, rect, BG);

    // The title strip of the group box.
    let font = FontId::proportional(9.0);
    let galley = painter.layout_no_wrap(title.to_ascii_uppercase(), font, TEXT);
    let strip = Rect::from_min_size(
        rect.left_top() + egui::vec2(GROUP_PADDING, 0.0),
        egui::vec2(galley.size().x + 8.0, TITLE_HEIGHT),
    );
    painter.rect_filled(strip, CornerRadius::ZERO, TITLE_BG);
    painter.line_segment(
        [strip.left_top(), strip.right_top()],
        Stroke::new(1.0, LIGHT),
    );
    painter.galley(strip.left_top() + egui::vec2(4.0, 3.0), galley, TEXT);

    // The header of the group box, next to the title strip.
    let header = Rect::from_min_size(
        egui::pos2(strip.right() + 4.0, rect.top() + 1.0),
        egui::vec2(
            (rect.right() - GROUP_PADDING - strip.right() - 4.0).max(0.0),
            TITLE_HEIGHT,
        ),
    );
    ui.scope_builder(
        egui::UiBuilder::new()
            .id_salt(("sushi-header", title))
            .max_rect(header)
            .layout(Layout::left_to_right(Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(4.0, 0.0);
            add(ui, Part::Header);
        },
    );

    // The body of the group box.
    let body = Rect::from_min_size(
        rect.left_top() + egui::vec2(GROUP_PADDING, TITLE_HEIGHT + 3.0),
        egui::vec2(
            rect.width() - 2.0 * GROUP_PADDING,
            (rect.height() - TITLE_HEIGHT - 3.0 - GROUP_PADDING).max(0.0),
        ),
    );
    ui.scope_builder(
        egui::UiBuilder::new()
            .id_salt(("sushi-body", title))
            .max_rect(body)
            .layout(Layout::top_down(Align::Min)),
        |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(0.0, 2.0);
            add(ui, Part::Body);
        },
    );
}

/// Lays out a row of sub sections inside a group box.
fn sections(
    ui: &mut egui::Ui,
    name: &str,
    count: usize,
    gap: f32,
    mut add: impl FnMut(usize, Rect, &mut egui::Ui),
) {
    let start = ui.next_widget_position();
    let width = ui.available_width();
    let height = ui.available_height();
    let slot = (width - gap * (count as f32 - 1.0)) / count as f32;

    for index in 0..count {
        let rect = Rect::from_min_size(
            egui::pos2(start.x + index as f32 * (slot + gap), start.y),
            egui::vec2(slot, height),
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .id_salt(("sushi-section", name, index))
                .max_rect(rect)
                .layout(Layout::top_down(Align::Min)),
            |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 2.0);
                add(index, rect, ui);
            },
        );
    }

    // The dividers between the sub sections.
    let painter = ui.painter_at(Rect::from_min_size(start, egui::vec2(width, height)));
    for index in 1..count {
        let x = start.x + index as f32 * (slot + gap) - gap * 0.5;
        painter.line_segment(
            [
                egui::pos2(x, start.y + 2.0),
                egui::pos2(x, start.y + height - 2.0),
            ],
            Stroke::new(1.0, SHADOW),
        );
    }

    ui.advance_cursor_after_rect(Rect::from_min_size(start, egui::vec2(width, height)));
}

/// Lays out a row of parameter columns inside a group box. All the columns
/// have the same width, no matter how much space they actually use.
fn column_row(
    ui: &mut egui::Ui,
    name: &str,
    count: usize,
    gap: f32,
    mut add: impl FnMut(usize, &mut egui::Ui),
) {
    let start = ui.next_widget_position();
    let total_width = ui.available_width();
    let height = ui.available_height();
    let slot = ((total_width - gap * (count as f32 - 1.0)) / count as f32).max(1.0);

    for index in 0..count {
        let rect = Rect::from_min_size(
            egui::pos2(start.x + index as f32 * (slot + gap), start.y),
            egui::vec2(slot, height),
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .id_salt(("sushi-column", name, index))
                .max_rect(rect)
                .layout(Layout::top_down(Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 2.0);
                add(index, ui);
            },
        );
    }

    ui.advance_cursor_after_rect(Rect::from_min_size(start, egui::vec2(total_width, height)));
}

/// Draws a caption above a control. The caption is as wide as the space that
/// is left in its column.
fn caption(ui: &mut egui::Ui, text: &str, size: f32, color: Color32) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, LABEL_HEIGHT), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(size),
        color,
    );
}

/// Draws a small text label that takes just as much space as it needs.
fn text_label(ui: &mut egui::Ui, text: &str, size: f32, color: Color32) {
    let font = FontId::proportional(size);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, color);
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(galley.size().x, galley.size().y), Sense::hover());
    ui.painter_at(rect).galley(rect.left_top(), galley, color);
}

// --- The widgets -------------------------------------------------------------

/// A vertical slider. The handle is a band that is fully embedded inside the
/// groove: it never sticks out of it, no matter where the value is.
fn v_slider(ui: &mut egui::Ui, value: &mut f32, knob: &Knob) -> egui::Response {
    let (rect, mut response) =
        ui.allocate_exact_size(egui::vec2(knob.width, knob.height), Sense::click_and_drag());
    let painter = ui.painter_at(rect);

    // The groove.
    sunken(&painter, rect, FIELD);
    // The small sliders of the sequencer need a thinner frame.
    let inset = if knob.width <= MINI_SLIDER_WIDTH {
        2.0
    } else {
        GROOVE_INSET
    };
    let track = rect.shrink(inset);

    // The part of the groove below the handle is filled in.
    let handle_height = (knob.height * 0.13).clamp(4.0, HANDLE_HEIGHT);
    let position = value_to_position(*value, knob.min, knob.max, knob.log, track);
    let handle_center = position.clamp(
        track.top() + handle_height * 0.5,
        track.bottom() - handle_height * 0.5,
    );
    let filled = Rect::from_min_max(
        egui::pos2(track.left(), handle_center + handle_height * 0.5),
        egui::pos2(track.right(), track.bottom()),
    );
    if filled.height() > 0.0 {
        painter.rect_filled(filled, CornerRadius::ZERO, BUTTON);
    }

    // The handle itself: a full width band inside the groove.
    let handle = Rect::from_center_size(
        egui::pos2(rect.center().x, handle_center),
        egui::vec2(knob.width - 2.0 * inset, handle_height),
    );
    raised(&painter, handle, BUTTON);

    // Dragging anywhere on the groove sets the value.
    if (response.is_pointer_button_down_on() || response.dragged())
        && let Some(pointer) = response.interact_pointer_pos()
    {
        let position = ((track.bottom() - pointer.y) / track.height()).clamp(0.0, 1.0);
        *value = position_to_value(position, knob.min, knob.max, knob.log);
        response.mark_changed();
    }

    response
}

/// The numeric value field of a parameter.
fn number_field(
    ui: &mut egui::Ui,
    text: &mut String,
    id: &'static str,
    field: egui::Id,
    width: f32,
    height: f32,
) -> egui::Response {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), Sense::hover());
    let painter = ui.painter_at(rect);
    sunken(&painter, rect, FIELD);

    ui.scope_builder(
        egui::UiBuilder::new()
            .id_salt(("sushi-field", id))
            .max_rect(rect)
            .layout(Layout::top_down(Align::Center)),
        |ui| {
            ui.add(
                egui::TextEdit::singleline(text)
                    .id(field)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(width)
                    .align(egui::Align2::CENTER_CENTER)
                    .frame(
                        egui::Frame::new()
                            .fill(Color32::TRANSPARENT)
                            .stroke(Stroke::NONE)
                            .inner_margin(Margin::symmetric(2, 0)),
                    ),
            )
        },
    )
    .inner
}

/// The id of the numeric value field of a parameter.
fn field_id(id: &'static str) -> egui::Id {
    egui::Id::unique(("sushi-value-field", id))
}

/// Mirrors the parameter's value into the text buffer, but only when the
/// parameter actually changed. This way a value that was just typed in isn't
/// reverted before the host has applied it.
fn sync_text(ui: &egui::Ui, id: &'static str, text: &mut String, decimals: usize, display: f32) {
    let memory_id = egui::Id::unique(("sushi-value", id));
    let last: Option<f32> = ui.memory(|memory| memory.data.get_temp(memory_id));
    if last != Some(display) {
        *text = format_number(display, decimals);
        ui.memory_mut(|memory| memory.data.insert_temp(memory_id, display));
    }
}

/// Formats a number for the tiny value fields. Trailing zeros are dropped so
/// that the numbers stay short enough to fit.
fn format_number(value: f32, decimals: usize) -> String {
    let mut text = format!("{:.*}", decimals, value);
    if decimals > 0 {
        while text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
    }
    if text == "-0" {
        text = "0".to_string();
    }
    text
}

/// The meter of the header that shows how many voices are playing. The lights
/// are tiny squares, so that the whole meter fits between the presets and the
/// lights of the activity meter.
fn voice_meter(painter: &egui::Painter, rect: Rect, voices: usize) {
    let font = FontId::proportional(8.0);
    let label = painter.layout_no_wrap("VOICES".to_string(), font.clone(), MUTED);
    let label_width = label.size().x;

    let row_width = VOICE_LEDS as f32 * VOICE_LED_WIDTH + (VOICE_LEDS as f32 - 1.0) * VOICE_LED_GAP;
    let total = label_width + VOICE_METER_GAP + row_width;

    // The meter is centered in the free space of the header.
    let left = (rect.left() + (rect.width() - total) * 0.5).round();
    let top = rect.center().y - VOICE_LED_HEIGHT * 0.5;

    painter.text(
        egui::pos2(left, rect.center().y),
        Align2::LEFT_CENTER,
        "VOICES",
        font,
        MUTED,
    );

    // The row of the lights. Every light that stands for a playing voice is
    // green, the others are dark.
    let row_left = (left + label_width + VOICE_METER_GAP).round();
    for index in 0..VOICE_LEDS {
        let x = row_left + index as f32 * (VOICE_LED_WIDTH + VOICE_LED_GAP);
        let color = if index < voices { LED_GREEN } else { LED_OFF };
        painter.rect_filled(
            Rect::from_min_size(
                egui::pos2(x, top),
                egui::vec2(VOICE_LED_WIDTH, VOICE_LED_HEIGHT),
            ),
            CornerRadius::ZERO,
            color,
        );
    }
}

/// A small light with a dark ring around it.
fn led(painter: &egui::Painter, center: Pos2, color: Color32, on: bool) {
    let radius = LED_SIZE * 0.5;
    painter.circle_filled(center, radius, if on { color } else { LED_OFF });
    painter.circle_stroke(center, radius, Stroke::new(1.0, DARK));
}

/// The dark display that shows the name of the preset.
fn lcd(painter: &egui::Painter, rect: Rect, text: &str) {
    painter.rect_filled(rect, CornerRadius::ZERO, LCD_BG);
    painter.rect_stroke(rect, 0.0, Stroke::new(1.0, DARK), egui::StrokeKind::Inside);
    painter.text(
        rect.left_top() + egui::vec2(7.0, rect.height() * 0.5),
        Align2::LEFT_CENTER,
        text,
        FontId::monospace(12.0),
        LCD_FG,
    );
}

/// A square button that sinks while it is pressed.
fn button(ui: &mut egui::Ui, rect: Rect, text: &str) -> egui::Response {
    let response = ui.interact(
        rect,
        ui.scope_id().with(("sushi-button", text)),
        Sense::click(),
    );
    let painter = ui.painter_at(rect);

    if response.is_pointer_button_down_on() {
        sunken(&painter, rect, BUTTON);
    } else {
        raised(&painter, rect, BUTTON);
    }
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(9.0),
        TEXT,
    );

    response
}

/// The small oscilloscope of the header. It is just here to give the panel a
/// bit of life, not to be a measurement tool.
fn small_scope(painter: &egui::Painter, rect: Rect, history: &VecDeque<f32>) {
    painter.rect_filled(rect, CornerRadius::ZERO, LIGHT);
    painter.rect_stroke(
        rect,
        0.0,
        Stroke::new(1.0, SHADOW),
        egui::StrokeKind::Inside,
    );
    let inner = rect.shrink(3.0);

    let length = history.len();
    if length < 2 {
        return;
    }
    let count = length.min(1024);
    let start = length - count;
    let step = inner.width() / (count - 1) as f32;
    let scale = inner.height() * 0.9;

    let points = (0..count)
        .map(|index| {
            let x = inner.left() + index as f32 * step;
            let sample = history[start + index].clamp(-1.0, 1.0);
            egui::pos2(x, inner.center().y - sample * scale)
        })
        .collect();

    painter.add(egui::Shape::line(points, Stroke::new(1.0, TRACE)));
}

/// The offset that is added to the values of a logarithmic range so that a
/// range that starts at zero can be used as well.
fn log_offset(min: f32) -> f32 {
    if min > 0.0 { 0.0 } else { 1.0 }
}

/// The bounds of a logarithmic range, including the offset.
fn log_bounds(min: f32, max: f32) -> (f32, f32) {
    let offset = log_offset(min);
    (min + offset, max + offset)
}

/// Maps a value to its position in the groove, where 0 is the bottom and 1 is
/// the top.
fn value_to_position(value: f32, min: f32, max: f32, log: bool, track: Rect) -> f32 {
    let position = if log {
        let (low, high) = log_bounds(min, max);
        let value = value + log_offset(min);
        ((value.max(f32::MIN_POSITIVE).ln() - low.ln()) / (high.ln() - low.ln())).clamp(0.0, 1.0)
    } else {
        ((value - min) / (max - min)).clamp(0.0, 1.0)
    };
    track.bottom() - position * track.height()
}

/// Maps a position in the groove back to a value.
fn position_to_value(position: f32, min: f32, max: f32, log: bool) -> f32 {
    let value = if log {
        let (low, high) = log_bounds(min, max);
        let offset = log_offset(min);
        (low.ln() + position * (high.ln() - low.ln())).exp() - offset
    } else {
        min + position * (max - min)
    };
    value.clamp(min, max)
}

/// The identity conversion, used by the parameters that are shown as they are.
fn identity(value: f32) -> f32 {
    value
}

/// Converts a 0..1 value into a percentage.
fn to_percent(value: f32) -> f32 {
    value * 100.0
}

/// Converts a percentage into a 0..1 value.
fn from_percent(value: f32) -> f32 {
    value / 100.0
}

// --- The theme ---------------------------------------------------------------

fn apply_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::light();
    visuals.dark_mode = false;
    visuals.override_text_color = Some(TEXT);
    visuals.panel_fill = BG;
    visuals.window_fill = BG;
    visuals.window_stroke = Stroke::new(1.0, DARK);
    visuals.window_corner_radius = egui::CornerRadius::ZERO;
    visuals.menu_corner_radius = egui::CornerRadius::ZERO;
    visuals.extreme_bg_color = FIELD;
    visuals.text_edit_bg_color = Some(FIELD);
    visuals.faint_bg_color = FIELD;
    visuals.code_bg_color = FIELD;
    visuals.hyperlink_color = TEXT;
    visuals.slider_trailing_fill = false;
    visuals.selection.bg_fill = SELECTION;
    visuals.selection.stroke = Stroke::new(1.0, LIGHT);

    let square = egui::CornerRadius::ZERO;

    visuals.widgets.noninteractive.bg_fill = BG;
    visuals.widgets.noninteractive.weak_bg_fill = BG;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, SHADOW);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.noninteractive.corner_radius = square;

    // The slider rail and handle use the `inactive` widget style.
    visuals.widgets.inactive.bg_fill = LIGHT;
    visuals.widgets.inactive.weak_bg_fill = BUTTON;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, SHADOW);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, DARK);
    visuals.widgets.inactive.corner_radius = square;

    visuals.widgets.hovered.bg_fill = Color32::from_rgb(0xF0, 0xF0, 0xF0);
    visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(0xE8, 0xE8, 0xE8);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, DARK);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.5, TEXT);
    visuals.widgets.hovered.corner_radius = square;

    visuals.widgets.active.bg_fill = Color32::from_rgb(0xE4, 0xE4, 0xE4);
    visuals.widgets.active.weak_bg_fill = Color32::from_rgb(0xE0, 0xE0, 0xE0);
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, DARK);
    visuals.widgets.active.fg_stroke = Stroke::new(1.5, TEXT);
    visuals.widgets.active.corner_radius = square;

    visuals.widgets.open.bg_fill = FIELD;
    visuals.widgets.open.weak_bg_fill = BUTTON;
    visuals.widgets.open.bg_stroke = Stroke::new(1.0, DARK);
    visuals.widgets.open.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.open.corner_radius = square;

    ctx.all_styles_mut(|style| {
        style.visuals = visuals.clone();
        style.spacing.item_spacing = egui::vec2(6.0, 4.0);
        style.spacing.button_padding = egui::vec2(4.0, 1.0);
        style.spacing.interact_size.y = FIELD_HEIGHT;
        style.spacing.slider_rail_height = 8.0;
        style.spacing.slider_width = SLIDER_WIDTH;
        style.text_styles.insert(
            egui::TextStyle::Body,
            egui::FontId::new(11.0, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Button,
            egui::FontId::new(11.0, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Small,
            egui::FontId::new(10.0, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Monospace,
            egui::FontId::new(10.0, egui::FontFamily::Monospace),
        );
    });
}

/// Draws a Win95-style sunken field: dark top/left, light bottom/right.
fn sunken(painter: &egui::Painter, rect: egui::Rect, fill: Color32) {
    painter.rect_filled(rect, egui::CornerRadius::ZERO, fill);

    painter.line_segment(
        [rect.left_top(), rect.right_top()],
        Stroke::new(1.0, SHADOW),
    );
    painter.line_segment(
        [rect.left_top(), rect.left_bottom()],
        Stroke::new(1.0, SHADOW),
    );
    painter.line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0, LIGHT),
    );
    painter.line_segment(
        [rect.right_top(), rect.right_bottom()],
        Stroke::new(1.0, LIGHT),
    );

    let inner = rect.shrink(1.0);
    painter.line_segment(
        [inner.left_top(), inner.right_top()],
        Stroke::new(1.0, DARK),
    );
    painter.line_segment(
        [inner.left_top(), inner.left_bottom()],
        Stroke::new(1.0, DARK),
    );
    painter.line_segment(
        [inner.left_bottom(), inner.right_bottom()],
        Stroke::new(1.0, BG),
    );
    painter.line_segment(
        [inner.right_top(), inner.right_bottom()],
        Stroke::new(1.0, BG),
    );
}

/// Draws a Win95-style raised box: light top/left, dark bottom/right.
fn raised(painter: &egui::Painter, rect: egui::Rect, fill: Color32) {
    painter.rect_filled(rect, egui::CornerRadius::ZERO, fill);
    painter.line_segment([rect.left_top(), rect.right_top()], Stroke::new(1.0, LIGHT));
    painter.line_segment(
        [rect.left_top(), rect.left_bottom()],
        Stroke::new(1.0, LIGHT),
    );
    painter.line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0, SHADOW),
    );
    painter.line_segment(
        [rect.right_top(), rect.right_bottom()],
        Stroke::new(1.0, SHADOW),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use nice_plug::params::internals::ParamPtr;
    use std::cell::Cell;

    thread_local! {
        /// The number of gestures that have been opened and closed.
        static GESTURES: Cell<usize> = const { Cell::new(0) };
        /// The number of parameter values that have been set.
        static SETS: Cell<usize> = const { Cell::new(0) };
    }

    struct Stub;

    impl GuiContextInner for Stub {
        fn plugin_api(&self) -> nice_plug::context::PluginApi {
            unimplemented!()
        }
        unsafe fn raw_begin_set_parameter(&self, _param: ParamPtr) {
            GESTURES.with(|gestures| gestures.set(gestures.get() + 1));
        }
        unsafe fn raw_set_parameter_normalized(&self, param: ParamPtr, normalized: f32) {
            SETS.with(|sets| sets.set(sets.get() + 1));
            unsafe {
                param._internal_set_normalized_value(normalized);
            }
        }
        unsafe fn raw_end_set_parameter(&self, _param: ParamPtr) {
            GESTURES.with(|gestures| gestures.set(gestures.get() + 1));
        }
        fn get_state(&self) -> PluginState {
            unimplemented!()
        }
        fn set_state(&self, _state: PluginState) {}
        fn request_restart(&self) {}
    }

    fn make_editor() -> (egui::Context, SushiEditor, Arc<SushiParams>) {
        let params = Arc::new(SushiParams::default());
        let (_tx, rx) = rtrb::RingBuffer::new(1024);
        let step_index = Arc::new(AtomicUsize::new(0));
        let arp_led = Arc::new(AtomicUsize::new(0));
        let voice_count = Arc::new(AtomicUsize::new(0));
        let mut editor = SushiEditor::new(params.clone(), rx, step_index, arp_led, voice_count);
        let ctx = egui::Context::default();
        editor.gui_ctx = Some(GuiContext::new(Arc::new(Stub)));
        apply_theme(&ctx);
        (ctx, editor, params)
    }

    fn input(events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(WINDOW_WIDTH, WINDOW_HEIGHT),
            )),
            events,
            ..Default::default()
        }
    }

    fn run_frames(
        ctx: &egui::Context,
        editor: &mut SushiEditor,
        frames: Vec<egui::RawInput>,
    ) -> egui::FullOutput {
        let mut last = None;
        for input in frames {
            let mut output = ctx.run_ui(input, |ui| {
                editor.draw(ui);
            });
            output.textures_delta.clear();
            last = Some(output);
        }
        last.unwrap()
    }

    /// The rects of all the vertical sliders, in the order they are drawn.
    fn slider_rects(ctx: &egui::Context, output: &egui::FullOutput) -> Vec<egui::Rect> {
        let mut rects = Vec::new();
        for clipped in ctx.tessellate(output.shapes.clone(), ctx.pixels_per_point()) {
            if let egui::epaint::Primitive::Mesh(_mesh) = clipped.primitive {
                let rect = clipped.clip_rect;
                let width = rect.width();
                let is_slider =
                    (width - SLIDER_WIDTH).abs() < 0.5 || (width - MACRO_SLIDER_WIDTH).abs() < 0.5;
                if is_slider && rect.height() > 40.0 {
                    rects.push(rect);
                }
            }
        }
        rects
    }

    /// All the texts of a frame, with their positions.
    fn texts(output: &egui::FullOutput) -> Vec<(String, egui::Pos2)> {
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) => Some((text.galley.job.text.clone(), text.pos)),
                _ => None,
            })
            .collect()
    }

    /// All the texts of a frame, with their positions and their sizes.
    fn text_shapes(output: &egui::FullOutput) -> Vec<(String, egui::Pos2, egui::Vec2)> {
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) => {
                    Some((text.galley.job.text.clone(), text.pos, text.galley.size()))
                }
                _ => None,
            })
            .collect()
    }

    /// Clicks at the given position.
    fn click(pos: egui::Pos2) -> Vec<egui::RawInput> {
        vec![
            input(vec![egui::Event::PointerMoved(pos)]),
            input(vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            }]),
            input(vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::default(),
            }]),
        ]
    }

    /// Two clicks at the given position, close enough to be a double click.
    fn double_click(pos: egui::Pos2) -> Vec<egui::RawInput> {
        let mut frames = click(pos);
        frames.extend(click(pos));
        frames
    }

    /// Waits long enough for the window of the double click to pass.
    fn wait() -> Vec<egui::RawInput> {
        vec![input(vec![]); 25]
    }

    #[test]
    fn the_panel_has_the_expected_number_of_widgets() {
        let (ctx, mut editor, _params) = make_editor();
        run_frames(&ctx, &mut editor, vec![input(vec![])]);

        let counts = editor.state.counts;
        assert_eq!(counts.sliders, 50, "unexpected number of sliders");
        assert_eq!(counts.mini, 8, "unexpected number of step sliders");
        assert_eq!(counts.popups, 12, "unexpected number of popups");
        assert_eq!(counts.checks, 4, "unexpected number of check boxes");
        assert_eq!(counts.radios, 1, "unexpected number of radio pairs");
    }

    #[test]
    fn every_parameter_of_the_synthesizer_is_reachable() {
        let (ctx, mut editor, _params) = make_editor();
        run_frames(&ctx, &mut editor, vec![input(vec![])]);

        let counts = editor.state.counts;
        // The synthesizer has 75 parameters: 58 of them are continuous, 13 are
        // choices and 4 are switches.
        let continuous = counts.sliders + counts.mini;
        assert_eq!(continuous, 58);
        assert_eq!(
            continuous + counts.popups + counts.checks + counts.radios,
            75
        );
    }

    #[test]
    fn clicking_a_slider_sets_the_value() {
        let (ctx, mut editor, params) = make_editor();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let sliders = slider_rects(&ctx, &output);
        assert_eq!(sliders.len(), 50, "unexpected number of sliders");
        // The cutoff slider is the first one of the third group of the first
        // row.
        let rect = sliders[10];
        let pos = rect.center();

        run_frames(&ctx, &mut editor, click(pos));

        // The cutoff slider was clicked in the middle: half way through the
        // logarithmic range.
        let cutoff = params.cutoff.value();
        assert!(
            (cutoff - 632.0).abs() < 20.0,
            "unexpected cutoff value: {cutoff}"
        );
    }

    #[test]
    fn clicking_a_slider_opens_and_closes_a_gesture() {
        let (ctx, mut editor, _params) = make_editor();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let sliders = slider_rects(&ctx, &output);
        let rect = sliders[10];
        let pos = rect.center();

        GESTURES.with(|gestures| gestures.set(0));
        SETS.with(|sets| sets.set(0));

        run_frames(&ctx, &mut editor, click(pos));

        // The gesture is opened before the value is set and closed again
        // afterwards, so that the host records the click.
        let gestures = GESTURES.with(|gestures| gestures.get());
        let sets = SETS.with(|sets| sets.get());
        assert_eq!(gestures, 2, "the gesture was not opened and closed");
        assert_eq!(sets, 1, "the value was not set");
    }

    #[test]
    fn dragging_a_slider_opens_a_single_gesture() {
        let (ctx, mut editor, _params) = make_editor();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let sliders = slider_rects(&ctx, &output);
        let rect = sliders[10];
        let pos = rect.center();
        let moved = pos + egui::vec2(0.0, -30.0);

        GESTURES.with(|gestures| gestures.set(0));
        SETS.with(|sets| sets.set(0));

        run_frames(
            &ctx,
            &mut editor,
            vec![
                input(vec![egui::Event::PointerMoved(pos)]),
                input(vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::default(),
                }]),
                input(vec![egui::Event::PointerMoved(moved)]),
                input(vec![egui::Event::PointerMoved(moved)]),
                input(vec![egui::Event::PointerButton {
                    pos: moved,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::default(),
                }]),
            ],
        );

        // The gesture is opened once, when the drag starts, and closed again
        // when it ends.
        let gestures = GESTURES.with(|gestures| gestures.get());
        let sets = SETS.with(|sets| sets.get());
        assert_eq!(
            gestures, 2,
            "the gesture was not opened and closed exactly once"
        );
        assert!(sets >= 2, "the value was not set while dragging");
    }

    #[test]
    fn typing_a_value_sets_it_exactly() {
        let (ctx, mut editor, params) = make_editor();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let sliders = slider_rects(&ctx, &output);
        let rect = sliders[10];
        // The value field is right below the slider.
        let field = egui::pos2(rect.center().x, rect.bottom() + 2.0 + 7.0);

        let mut events = vec![egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        }];
        for character in "1000".chars() {
            events.push(egui::Event::Text(character.to_string()));
        }
        events.push(egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        });

        run_frames(
            &ctx,
            &mut editor,
            vec![
                input(vec![]),
                input(vec![
                    egui::Event::PointerMoved(field),
                    egui::Event::PointerButton {
                        pos: field,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::default(),
                    },
                    egui::Event::PointerButton {
                        pos: field,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: egui::Modifiers::default(),
                    },
                ]),
                input(events),
                input(vec![]),
            ],
        );

        assert_eq!(params.cutoff.value(), 1000.0);
    }

    #[test]
    fn typing_nonsense_keeps_the_old_value() {
        let (ctx, mut editor, params) = make_editor();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let sliders = slider_rects(&ctx, &output);
        let rect = sliders[10];
        // The value field is right below the slider.
        let field = egui::pos2(rect.center().x, rect.bottom() + 2.0 + 7.0);

        let mut events = vec![egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        }];
        for character in "NaN".chars() {
            events.push(egui::Event::Text(character.to_string()));
        }
        events.push(egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        });

        run_frames(
            &ctx,
            &mut editor,
            vec![
                input(vec![]),
                input(vec![
                    egui::Event::PointerMoved(field),
                    egui::Event::PointerButton {
                        pos: field,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::default(),
                    },
                    egui::Event::PointerButton {
                        pos: field,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: egui::Modifiers::default(),
                    },
                ]),
                input(events),
                input(vec![]),
            ],
        );

        // "NaN" is not a number, so the parameter keeps its old value.
        assert_eq!(params.cutoff.value(), 3_000.0);
    }

    #[test]
    fn the_popup_sets_the_value() {
        let (ctx, mut editor, params) = make_editor();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        // The waveform popup of the first oscillator.
        let (_, pos) = texts(&output)
            .into_iter()
            .find(|(text, _)| text == "Saw")
            .unwrap();
        let button = pos + egui::vec2(2.0, 2.0);

        // Open the popup.
        run_frames(
            &ctx,
            &mut editor,
            vec![
                input(vec![egui::Event::PointerMoved(button)]),
                input(vec![egui::Event::PointerButton {
                    pos: button,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::default(),
                }]),
                input(vec![egui::Event::PointerButton {
                    pos: button,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::default(),
                }]),
                // The popup is only painted from the second frame on.
                input(vec![]),
            ],
        );

        // The list of waveforms is open now.
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let names: Vec<String> = texts(&output).into_iter().map(|(t, _)| t).collect();
        assert!(names.contains(&"Triangle".to_string()), "{names:?}");

        // Pick the triangle.
        let (_, triangle) = texts(&output)
            .into_iter()
            .find(|(text, pos)| text == "Triangle" && pos.x < 200.0)
            .unwrap();
        let item = triangle + egui::vec2(2.0, 2.0);
        run_frames(&ctx, &mut editor, click(item));

        assert_eq!(params.osc1_wave.value(), crate::params::Waveform::Triangle);
    }

    #[test]
    fn the_next_button_loads_the_next_preset() {
        let (ctx, mut editor, params) = make_editor();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let (_, pos) = texts(&output)
            .into_iter()
            .find(|(text, _)| text == ">")
            .unwrap();
        let button = pos + egui::vec2(2.0, 2.0);

        run_frames(&ctx, &mut editor, click(button));

        assert_eq!(editor.state.preset_index, 1);
        // The second preset is the glass keys.
        assert_eq!(params.cutoff.value(), 4_200.0);
    }

    #[test]
    fn the_mutate_button_changes_the_sound() {
        let (ctx, mut editor, params) = make_editor();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let (_, pos) = texts(&output)
            .into_iter()
            .find(|(text, _)| text == "MUTATE")
            .unwrap();
        let button = pos + egui::vec2(2.0, 2.0);

        let cutoff = params.cutoff.value();
        run_frames(&ctx, &mut editor, click(button));

        assert!(editor.state.dirty);
        assert!(params.cutoff.value() != cutoff);
    }

    #[test]
    fn double_clicking_a_slider_resets_it_to_the_default() {
        let (ctx, mut editor, params) = make_editor();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let sliders = slider_rects(&ctx, &output);
        // The cutoff slider is the first one of the third group of the first
        // row.
        let rect = sliders[10];

        // A click at the top of the slider moves it away from its default.
        let top = egui::pos2(rect.center().x, rect.top() + 6.0);
        run_frames(&ctx, &mut editor, click(top));
        assert_ne!(params.cutoff.value(), 3_000.0);

        // The double click puts it back to its default.
        run_frames(&ctx, &mut editor, wait());
        run_frames(&ctx, &mut editor, double_click(rect.center()));
        let cutoff = params.cutoff.value();
        assert!(
            (cutoff - 3_000.0).abs() < 1.0,
            "the cutoff was not reset: {cutoff}"
        );
    }

    #[test]
    fn double_clicking_a_slider_opens_and_closes_a_gesture() {
        let (ctx, mut editor, _params) = make_editor();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let sliders = slider_rects(&ctx, &output);
        let rect = sliders[10];

        GESTURES.with(|gestures| gestures.set(0));
        SETS.with(|sets| sets.set(0));

        run_frames(&ctx, &mut editor, double_click(rect.center()));

        // The two clicks open and close a gesture each, and the reset is
        // done inside of the gesture of the second click.
        let gestures = GESTURES.with(|gestures| gestures.get());
        let sets = SETS.with(|sets| sets.get());
        assert_eq!(gestures, 4, "the gestures were not opened and closed");
        assert_eq!(sets, 3, "the value was not set three times");
    }

    #[test]
    fn double_clicking_a_step_slider_resets_it_to_the_default() {
        let (ctx, mut editor, params) = make_editor();
        run_frames(&ctx, &mut editor, vec![input(vec![])]);
        // The first step of the sequencer.
        let rect = editor.state.mini_rects[0];

        // A click at the top of the slider moves it away from its default.
        let top = egui::pos2(rect.center().x, rect.top() + 4.0);
        run_frames(&ctx, &mut editor, click(top));
        assert_ne!(params.step_1.value(), 0.5);

        // The double click puts it back to its default.
        run_frames(&ctx, &mut editor, wait());
        run_frames(&ctx, &mut editor, double_click(rect.center()));
        let step = params.step_1.value();
        assert!((step - 0.5).abs() < 0.001, "the step was not reset: {step}");
    }

    #[test]
    fn double_clicking_a_check_box_resets_it_to_the_default() {
        let (ctx, mut editor, params) = make_editor();
        run_frames(&ctx, &mut editor, vec![input(vec![])]);
        // The first check box is the switch of the step sequencer.
        let rect = editor.state.check_rects[0];
        let pos = rect.center();

        // A single click switches the check box on.
        run_frames(&ctx, &mut editor, click(pos));
        assert!(params.step_on.value());

        // The double click puts it back to its default, even though the two
        // clicks would switch it twice.
        run_frames(&ctx, &mut editor, wait());
        run_frames(&ctx, &mut editor, double_click(pos));
        assert!(!params.step_on.value());
    }

    #[test]
    fn double_clicking_the_character_resets_it_to_the_default() {
        let (ctx, mut editor, params) = make_editor();
        run_frames(&ctx, &mut editor, vec![input(vec![])]);
        // The second button of the radio pair is the harsh character.
        let rect = editor.state.radio_rects[1];
        let pos = rect.center();

        // A single click picks the harsh character.
        run_frames(&ctx, &mut editor, click(pos));
        assert_eq!(params.character.value(), crate::params::Character::Grit);

        // The double click puts it back to the default.
        run_frames(&ctx, &mut editor, wait());
        run_frames(&ctx, &mut editor, double_click(pos));
        assert_eq!(params.character.value(), crate::params::Character::Silk);
    }

    #[test]
    fn double_clicking_a_popup_resets_it_to_the_default() {
        let (ctx, mut editor, params) = make_editor();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        // The waveform popup of the first oscillator.
        let (_, pos) = texts(&output)
            .into_iter()
            .find(|(text, _)| text == "Saw")
            .unwrap();
        let button = pos + egui::vec2(2.0, 2.0);

        // Open the popup and pick the triangle.
        run_frames(
            &ctx,
            &mut editor,
            vec![
                input(vec![egui::Event::PointerMoved(button)]),
                input(vec![egui::Event::PointerButton {
                    pos: button,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::default(),
                }]),
                input(vec![egui::Event::PointerButton {
                    pos: button,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::default(),
                }]),
                input(vec![]),
            ],
        );
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let (_, triangle) = texts(&output)
            .into_iter()
            .find(|(text, pos)| text == "Triangle" && pos.x < 200.0)
            .unwrap();
        run_frames(&ctx, &mut editor, click(triangle + egui::vec2(2.0, 2.0)));
        assert_eq!(params.osc1_wave.value(), crate::params::Waveform::Triangle);

        // The double click on the button of the popup puts the parameter back
        // to its default.
        run_frames(&ctx, &mut editor, wait());
        run_frames(&ctx, &mut editor, double_click(button));
        assert_eq!(params.osc1_wave.value(), crate::params::Waveform::Saw);
    }

    #[test]
    fn the_header_hints_at_the_double_click_reset() {
        let (ctx, mut editor, _params) = make_editor();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);

        let (_, pos, size) = text_shapes(&output)
            .into_iter()
            .find(|(text, _, _)| text == "DBL-CLICK = RESET")
            .expect("the header has no hint");

        // The hint sits in the header, next to the wordmark, and it does not
        // touch the presets.
        assert!(pos.y > MARGIN && pos.y < MARGIN + HEADER_HEIGHT);
        assert!(
            pos.x + size.x < MARGIN + 150.0,
            "the hint overlaps the presets: {pos:?} {size:?}"
        );
    }
}
