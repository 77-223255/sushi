//! The Sushi editor: a flat blueprint panel.
//!
//! Every continuous parameter is a vertical slider whose handle is a band that
//! is fully embedded inside the track (it never sticks out), every parameter
//! can also be typed into a small numeric field, and every choice parameter
//! opens a popup menu. The parameters are grouped into sections that are named
//! by a coloured title and a thin rule below a header that holds the wordmark,
//! the factory presets and a small oscilloscope. Nothing on the panel has a
//! bevel, a shadow or a rounded corner: every widget is a flat fill with a
//! single 1 px border.

mod presets;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
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
use crate::patch;
use presets::{PRESETS, Rng};

// --- Flat blueprint palette -------------------------------------------------

/// The background of the whole panel. The sections have no fill of their own.
const BG: Color32 = Color32::from_rgb(0xF1, 0xF1, 0xF1);
/// The 1 px rules: below the section titles, below the header, around fields.
const RULE: Color32 = Color32::from_rgb(0x8A, 0x8A, 0x8A);
/// The outlines of the widgets.
const LINE: Color32 = Color32::from_rgb(0x7A, 0x7A, 0x7A);
const TEXT: Color32 = Color32::from_rgb(0x11, 0x11, 0x11);
const MUTED: Color32 = Color32::from_rgb(0x5A, 0x5A, 0x5A);
/// The titles of the oscillators, the mixer, the filter and the arpeggiator.
const TITLE_BLUE: Color32 = Color32::from_rgb(0x25, 0x4C, 0x9A);
/// The titles of the envelopes, the low frequency oscillators and the VCA.
const TITLE_OLIVE: Color32 = Color32::from_rgb(0x6E, 0x6A, 0x24);
/// The titles of the voice, the effects and the macros.
const TITLE_GRAY: Color32 = Color32::from_rgb(0x3A, 0x3A, 0x3A);
/// The track of a slider.
const TRACK: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);
/// The handle band of a slider.
const HANDLE: Color32 = Color32::from_rgb(0x1A, 0x1A, 0x1A);
/// The tick in the middle of the track of a slider.
const TICK: Color32 = Color32::from_rgb(0xD8, 0xD8, 0xD8);
/// The background of the numeric fields.
const FIELD_BG: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);
const BUTTON_BG: Color32 = Color32::from_rgb(0xE7, 0xE7, 0xE7);
const BUTTON_ACTIVE: Color32 = Color32::from_rgb(0xCF, 0xCF, 0xCF);
/// The background of the display of the presets.
const LCD_BG: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);
/// The color of the text in the display of the presets.
const LCD_FG: Color32 = Color32::from_rgb(0x11, 0x11, 0x11);
/// The color of a light of the activity meter.
const LED_ON: Color32 = Color32::from_rgb(0x22, 0xA8, 0x4A);
/// The color of the light of the clip meter.
const LED_RED: Color32 = Color32::from_rgb(0xC8, 0x32, 0x32);
/// The color of the lights of the arpeggiator.
const LED_AMBER: Color32 = Color32::from_rgb(0xD9, 0x9A, 0x20);
/// The color of a light that is switched off.
const LED_OFF: Color32 = Color32::from_rgb(0xB8, 0xB8, 0xB8);
/// The color of the trace of the oscilloscope.
const TRACE: Color32 = Color32::from_rgb(0x22, 0x22, 0x22);
/// The color of the text of a status line that reports a success.
const STATUS_OK: Color32 = Color32::from_rgb(0x00, 0x60, 0x00);
/// The color of the text of a status line that reports a problem.
const STATUS_ERROR: Color32 = Color32::from_rgb(0x90, 0x10, 0x10);

// --- Layout -----------------------------------------------------------------

pub const WINDOW_WIDTH: f32 = 960.0;
pub const WINDOW_HEIGHT: f32 = 640.0;

/// The margin around the whole panel.
const MARGIN: f32 = 8.0;
/// The height of the header.
const HEADER_HEIGHT: f32 = 48.0;
/// The width of the sections of the third and the fourth row.
const GROUP_WIDTH: f32 = 230.0;
/// The width of the section of the effects in the fourth row.
const DOUBLE_GROUP_WIDTH: f32 = 2.0 * GROUP_WIDTH + GROUP_GAP;
/// The width of the section of the arpeggiator in the third row.
const TRIPLE_GROUP_WIDTH: f32 = 3.0 * GROUP_WIDTH + 2.0 * GROUP_GAP;
/// The gap between two sections.
const GROUP_GAP: f32 = 8.0;
/// The padding inside a section.
const GROUP_PADDING: f32 = 4.0;
/// The height of the line that holds the title of a section and the controls
/// of its header. The rule below the line sits on its bottom edge.
const TITLE_HEIGHT: f32 = 18.0;
/// The horizontal gap between two parameter columns.
const COLUMN_GAP: f32 = 4.0;

/// The height of the first row of sections.
const ROW_A_HEIGHT: f32 = 144.0;
/// The height of the second row of sections.
const ROW_B_HEIGHT: f32 = 136.0;
/// The height of the third row of sections.
const ROW_C_HEIGHT: f32 = 138.0;
/// The height of the fourth row of sections.
const ROW_D_HEIGHT: f32 = 126.0;

/// The widths of the sections of the first row: `VCO 1 | VCO 2 | SOURCE MIXER |
/// VCF`.
const ROW_A_WIDTHS: [f32; 4] = [200.0, 160.0, 280.0, 280.0];
/// The widths of the sections of the second row: `PULSE WIDTH | MOD ENV | AMP
/// ENV | LFO / CLK`.
const ROW_B_WIDTHS: [f32; 4] = [160.0, 180.0, 180.0, 400.0];
/// The widths of the sections of the third row: `ARP | VOICE`.
const ROW_C_WIDTHS: [f32; 2] = [TRIPLE_GROUP_WIDTH, GROUP_WIDTH];
/// The widths of the sections of the fourth row: `FX | MACROS | VCA`.
const ROW_D_WIDTHS: [f32; 3] = [DOUBLE_GROUP_WIDTH, GROUP_WIDTH, GROUP_WIDTH];

// --- Widget metrics ---------------------------------------------------------

/// The width of a vertical slider.
const SLIDER_WIDTH: f32 = 16.0;
/// The width of the sliders of the macros.
const MACRO_SLIDER_WIDTH: f32 = 24.0;
/// The height of the slider handle.
const HANDLE_HEIGHT: f32 = 6.0;
/// How far the track and the handle are kept away from the edge of the slider.
const GROOVE_INSET: f32 = 2.0;
/// The size of the numeric value fields.
const FIELD_WIDTH: f32 = 38.0;
const FIELD_HEIGHT: f32 = 14.0;
/// The height of the captions above the controls.
const LABEL_HEIGHT: f32 = 11.0;
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
/// The number of the lights of the position strip of the arpeggiator.
const POS_LEDS: usize = 16;
/// The width of a light of the position strip of the arpeggiator.
const POS_LED_WIDTH: f32 = 6.0;
/// The height of a light of the position strip of the arpeggiator.
const POS_LED_HEIGHT: f32 = 8.0;
/// The gap between two lights of the position strip of the arpeggiator.
const POS_LED_GAP: f32 = 2.0;
/// The width of the position strip of the arpeggiator.
const POS_STRIP_WIDTH: f32 =
    POS_LEDS as f32 * POS_LED_WIDTH + (POS_LEDS as f32 - 1.0) * POS_LED_GAP;
/// The gap between the sliders and the position strip of the arpeggiator.
const POS_STRIP_GAP: f32 = 12.0;
/// The width of the button of the patch browser in the header.
const PATCHES_BUTTON_WIDTH: f32 = 62.0;
/// The width of the window of the patch browser.
const PATCH_WINDOW_WIDTH: f32 = 430.0;
/// The longest name that the display of the header shows.
const LCD_NAME_LENGTH: usize = 20;
/// The size of a check box.
const CHECK_SIZE: f32 = 11.0;
/// The height of the buttons of the radio pair.
const RADIO_HEIGHT: f32 = 14.0;
/// The hint that tells the user that the folder has to be applied with SET.
const FOLDER_HINT: &str = "Press SET to apply the folder";
/// The caption that is shown when the folder of the patches is still empty.
const NO_PATCHES: &str = "No patches yet - use SAVE";

/// The wanted height of the sliders of the first row. The sliders only ever
/// use the height that is actually left in their column, so a row can never
/// overflow, but these sizes fill their rows completely.
const SLIDER_A: f32 = 90.0;
/// The wanted height of the sliders of the envelopes of the second row.
const SLIDER_B: f32 = 82.0;
/// The wanted height of the sliders of the third row.
const SLIDER_C: f32 = 84.0;
/// The height of the sliders of the effects.
const SLIDER_FX: f32 = 59.0;
/// The height of the sliders of the macros.
const SLIDER_MACRO: f32 = 72.0;
/// The height of the sliders of the VCA.
const SLIDER_OUT: f32 = 72.0;
/// The height of the sliders of the low frequency oscillators.
const SLIDER_LFO: f32 = 62.0;
/// The height of the sliders of the arpeggiator.
const SLIDER_ARP: f32 = 84.0;

/// How many scope samples are kept around.
const HISTORY_LEN: usize = 4096;

/// How long the light of the arpeggiator stays on.
const ARP_LIGHT_TIME: Duration = Duration::from_millis(120);

// --- The editor -------------------------------------------------------------

/// The number of widgets of every kind that were drawn in the last frame.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default)]
struct Counts {
    sliders: usize,
    popups: usize,
    checks: usize,
    radios: usize,
}

/// The kind of a widget that is counted by the tests.
#[cfg(test)]
#[derive(Clone, Copy)]
enum Widget {
    Slider,
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
    /// The position of the arpeggiator in its pattern.
    arp_step: usize,
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
    /// The configuration of the patch browser.
    patch_config: patch::PatchConfig,
    /// Whether the window of the patch browser is open.
    patch_open: bool,
    /// The folder in which the patches are stored, as it is shown in the
    /// patch browser.
    patch_dir_text: String,
    /// The name of the patch that is saved by the patch browser.
    patch_name: String,
    /// The patches of the folder.
    patch_files: Vec<PathBuf>,
    /// The last message of the patch browser.
    patch_status: String,
    /// Whether the last message of the patch browser reports a problem.
    patch_status_error: bool,
    /// Whether the patch browser has to ask before it overwrites a file.
    patch_confirm_overwrite: bool,
    /// The name of the patch of the user that is loaded.
    patch_active: Option<String>,
}

impl EditorState {
    /// Counts a widget of the last frame.
    #[cfg(test)]
    fn count(&mut self, widget: Widget) {
        let counts = &mut self.counts;
        match widget {
            Widget::Slider => counts.sliders += 1,
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
    /// The number of arpeggiator triggers.
    arp_led: Arc<AtomicUsize>,
    /// The position of the arpeggiator in its pattern.
    arp_position: Arc<AtomicUsize>,
    /// The number of voices that are currently playing.
    voice_count: Arc<AtomicUsize>,
}

impl SushiEditor {
    /// Creates a new editor for the given parameters.
    pub fn new(
        params: Arc<SushiParams>,
        scope_rx: rtrb::Consumer<f32>,
        arp_led: Arc<AtomicUsize>,
        arp_position: Arc<AtomicUsize>,
        voice_count: Arc<AtomicUsize>,
    ) -> Self {
        let mut editor = Self {
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
                arp_step: 0,
                activity: 0.0,
                clipping: 0.0,
                #[cfg(test)]
                counts: Counts::default(),
                #[cfg(test)]
                check_rects: Vec::new(),
                #[cfg(test)]
                radio_rects: Vec::new(),
                patch_config: patch::PatchConfig::default(),
                patch_open: false,
                patch_dir_text: String::new(),
                patch_name: String::new(),
                patch_files: Vec::new(),
                patch_status: String::new(),
                patch_status_error: false,
                patch_confirm_overwrite: false,
                patch_active: None,
            },
            arp_led,
            arp_position,
            voice_count,
        };
        editor.init_patch_state();
        editor
    }

    /// Reads the configuration of the patch browser and fills the list of the
    /// patches of the folder. No folder is created here: the folder only
    /// appears when a patch is saved.
    fn init_patch_state(&mut self) {
        self.state.patch_config = patch::load_config();
        self.state.patch_dir_text = self
            .state
            .patch_config
            .patch_dir
            .to_string_lossy()
            .into_owned();
        self.state.patch_name = PRESETS[self.state.preset_index].name.to_string();
        self.state.patch_files = patch::list_patches(&self.state.patch_config.patch_dir);
        self.state.patch_status.clear();
        self.state.patch_status_error = false;
        self.state.patch_confirm_overwrite = false;
        self.state.patch_active = None;
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

        // The position of the arpeggiator, for the strip of lights.
        self.state.arp_step = self
            .arp_position
            .load(Ordering::Relaxed)
            .min(POS_LEDS.saturating_sub(1));
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
        self.init_patch_state();

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

        // The rows of sections.
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
        let voice_count = &self.voice_count;
        let mut ctx = Ctx {
            params,
            setter: &setter,
            state: &mut self.state,
            history,
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

        // The window of the patch browser floats above the panel.
        ctx.patch_window(ui);
    }
}

// --- The header -------------------------------------------------------------

/// The places of the widgets of the header.
struct HeaderRects {
    /// The button of the previous factory preset.
    previous: Rect,
    /// The display that shows the name of the sound.
    display: Rect,
    /// The button of the next factory preset.
    next: Rect,
    /// The button that mutates the sound.
    mutate: Rect,
    /// The button that opens the patch browser.
    patches: Rect,
    /// The meter that shows the number of the voices that are playing.
    meter: Rect,
}

/// The places of the widgets of the header.
fn header_rects(rect: Rect) -> HeaderRects {
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
    let patches = Rect::from_min_size(
        egui::pos2(mutate.right() + 4.0, button_y),
        egui::vec2(PATCHES_BUTTON_WIDTH, 28.0),
    );

    // The meter that shows how many voices are playing. It sits in the free
    // space between the patches and the lights of the activity meter.
    let meter = Rect::from_min_max(
        egui::pos2(patches.right() + 8.0, rect.top()),
        egui::pos2(rect.right() - 194.0, rect.bottom()),
    );

    HeaderRects {
        previous,
        display,
        next,
        mutate,
        patches,
        meter,
    }
}

/// The text that the display of the header shows.
///
/// The display shows the name of the patch of the user, if one is loaded, and
/// the name of the factory preset otherwise. A star marks a sound that was
/// changed after it was loaded.
fn display_text(state: &EditorState) -> String {
    let dirty = if state.dirty { "*" } else { "" };
    match &state.patch_active {
        Some(name) => format!("{}{dirty}", truncate_name(name, LCD_NAME_LENGTH)),
        None => format!(
            "{:02} {}{dirty}",
            state.preset_index + 1,
            PRESETS[state.preset_index].name
        ),
    }
}

/// Cuts a name off, so that it fits into the display of the header.
fn truncate_name(name: &str, length: usize) -> String {
    name.chars().take(length).collect()
}

/// Draws the header: the wordmark, the presets and the oscilloscope.
fn header(ui: &mut egui::Ui, rect: Rect, ctx: &mut Ctx) {
    let painter = ui.painter_at(rect);

    // The header is flat and only separated from the sections by a rule.
    let rule_y = rect.bottom() - 0.5;
    painter.line_segment(
        [
            egui::pos2(rect.left(), rule_y),
            egui::pos2(rect.right(), rule_y),
        ],
        Stroke::new(1.0, RULE),
    );

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
    let rects = header_rects(rect);

    let index = ctx.state.preset_index;
    lcd(&painter, rects.display, &display_text(ctx.state));

    if button(ui, rects.previous, "<").clicked() {
        ctx.load_preset(index + PRESETS.len() - 1);
    }
    if button(ui, rects.next, ">").clicked() {
        ctx.load_preset(index + 1);
    }
    if button(ui, rects.mutate, "MUTATE").clicked() {
        ctx.mutate();
    }
    if button(ui, rects.patches, "PATCHES").clicked() {
        ctx.state.patch_open = !ctx.state.patch_open;
    }

    // The meter that shows how many voices are playing.
    voice_meter(
        &painter,
        rects.meter,
        ctx.voice_count.load(Ordering::Relaxed).min(VOICE_LEDS),
    );

    // The oscilloscope and the two lights.
    let scope = Rect::from_min_size(
        egui::pos2(rect.right() - 154.0, rect.top() + 5.0),
        egui::vec2(150.0, 38.0),
    );
    small_scope(&painter, scope, ctx.history);

    let activity = egui::pos2(rect.right() - 190.0, rect.top() + 16.0);
    led(&painter, activity, LED_ON, ctx.state.activity > 0.002);
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

// --- The rows of sections ------------------------------------------------

/// The first row: `VCO 1 | VCO 2 | SOURCE MIXER | VCF`.
///
/// The master tune is shown in the first oscillator, the pulse widths live in
/// their own section of the second row and the levels of the oscillators live
/// in the mixer.
fn row_a(ui: &mut egui::Ui, rect: Rect, ctx: &mut Ctx) {
    group_row(ui, "row-a", rect, &ROW_A_WIDTHS, |index, rect, ui| {
        let p = ctx.params;
        match index {
            0 => group(ui, rect, "VCO 1", |ui, part| match part {
                Part::Header => ctx.popup(ui, "osc1_wave", &p.osc1_wave, 66.0),
                Part::Body => column_row(ui, "osc1", 4, COLUMN_GAP, |index, ui| {
                    let p = ctx.params;
                    match index {
                        0 => ctx.float(
                            ui,
                            "tune",
                            "Tune",
                            &p.tune,
                            &Knob::plain(-12.0, 12.0, 2, SLIDER_A),
                        ),
                        1 => ctx.int(
                            ui,
                            "osc1_octave",
                            "Oct",
                            &p.osc1_octave,
                            &Knob::plain(-2.0, 2.0, 0, SLIDER_A),
                        ),
                        2 => ctx.int(
                            ui,
                            "osc1_semi",
                            "Semi",
                            &p.osc1_semi,
                            &Knob::plain(-12.0, 12.0, 0, SLIDER_A),
                        ),
                        3 => ctx.float(
                            ui,
                            "osc1_fine",
                            "Fine",
                            &p.osc1_fine,
                            &Knob::plain(-100.0, 100.0, 0, SLIDER_A),
                        ),
                        _ => {}
                    }
                }),
            }),
            1 => group(ui, rect, "VCO 2", |ui, part| match part {
                Part::Header => ctx.popup(ui, "osc2_wave", &p.osc2_wave, 66.0),
                Part::Body => column_row(ui, "osc2", 3, COLUMN_GAP, |index, ui| {
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
                        _ => {}
                    }
                }),
            }),
            2 => group(ui, rect, "SOURCE MIXER", |ui, part| match part {
                Part::Header => {}
                Part::Body => column_row(ui, "mix", 6, COLUMN_GAP, |index, ui| {
                    let p = ctx.params;
                    match index {
                        0 => ctx.float(
                            ui,
                            "osc1_level",
                            "Lvl 1",
                            &p.osc1_level,
                            &Knob::percent(SLIDER_A),
                        ),
                        1 => ctx.float(
                            ui,
                            "osc2_level",
                            "Lvl 2",
                            &p.osc2_level,
                            &Knob::percent(SLIDER_A),
                        ),
                        2 => ctx.float(
                            ui,
                            "noise_level",
                            "Noise",
                            &p.noise_level,
                            &Knob::percent(SLIDER_A),
                        ),
                        3 => ctx.int(
                            ui,
                            "unison",
                            "Uni",
                            &p.unison,
                            &Knob::plain(1.0, 4.0, 0, SLIDER_A),
                        ),
                        4 => ctx.float(
                            ui,
                            "detune",
                            "Detune",
                            &p.detune,
                            &Knob::plain(0.0, 50.0, 0, SLIDER_A),
                        ),
                        5 => ctx.float(ui, "spread", "Spread", &p.spread, &Knob::percent(SLIDER_A)),
                        _ => {}
                    }
                }),
            }),
            _ => group(ui, rect, "VCF", |ui, part| match part {
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
        }
    });
}

/// The second row: the pulse widths, the two envelopes and the low frequency
/// oscillators.
fn row_b(ui: &mut egui::Ui, rect: Rect, ctx: &mut Ctx) {
    group_row(ui, "row-b", rect, &ROW_B_WIDTHS, |index, rect, ui| {
        match index {
            0 => group(ui, rect, "PULSE WIDTH", |ui, part| match part {
                Part::Header => {}
                Part::Body => column_row(ui, "pw", 2, COLUMN_GAP, |index, ui| {
                    let p = ctx.params;
                    match index {
                        0 => ctx.float(
                            ui,
                            "osc1_pw",
                            "PW 1",
                            &p.osc1_pw,
                            &Knob::scaled(0.05, 0.95, 0, SLIDER_B),
                        ),
                        1 => ctx.float(
                            ui,
                            "osc2_pw",
                            "PW 2",
                            &p.osc2_pw,
                            &Knob::scaled(0.05, 0.95, 0, SLIDER_B),
                        ),
                        _ => {}
                    }
                }),
            }),
            1 => group(ui, rect, "MOD ENV", |ui, part| match part {
                Part::Header => text_label(ui, "ADSR", 9.0, TEXT),
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
            2 => group(ui, rect, "AMP ENV", |ui, part| match part {
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
            _ => group(ui, rect, "LFO / CLK", |ui, part| match part {
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
                            text_label(ui, title, 9.0, TEXT);
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
    });
}

/// The third row: the arpeggiator and the voice.
///
/// The arpeggiator gets a wide section, so that all of its parameters and the
/// strip of lights fit on one line.
fn row_c(ui: &mut egui::Ui, rect: Rect, ctx: &mut Ctx) {
    group_row(ui, "row-c", rect, &ROW_C_WIDTHS, |index, rect, ui| {
        let p = ctx.params;
        match index {
            0 => group(ui, rect, "ARP", |ui, part| match part {
                Part::Header => {
                    ctx.check(ui, "arp_on", &p.arp_on, "ON");
                    ctx.check(ui, "arp_latch", &p.arp_latch, "LATCH");
                    text_label(ui, "PATTERN", 9.0, TEXT);
                    ctx.popup(ui, "arp_pattern", &p.arp_pattern, 120.0);
                    text_label(ui, "RATE", 9.0, TEXT);
                    ctx.popup(ui, "arp_rate", &p.arp_rate, 70.0);
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
                    // The sliders are on the left, the strip of lights that
                    // shows the position of the arpeggiator on the right.
                    let body = ui.max_rect();
                    let slider_rect = Rect::from_min_max(
                        body.left_top(),
                        egui::pos2(
                            body.right() - POS_STRIP_WIDTH - POS_STRIP_GAP,
                            body.bottom(),
                        ),
                    );

                    ui.scope_builder(
                        egui::UiBuilder::new()
                            .id_salt("sushi-arp-sliders")
                            .max_rect(slider_rect)
                            .layout(Layout::top_down(Align::Min)),
                        |ui| {
                            let ctx = &mut *ctx;
                            column_row(ui, "arp", 5, COLUMN_GAP, |index, ui| {
                                let p = ctx.params;
                                match index {
                                    0 => ctx.float(
                                        ui,
                                        "arp_swing",
                                        "Swing",
                                        &p.arp_swing,
                                        &Knob::plain(0.0, 75.0, 0, SLIDER_ARP),
                                    ),
                                    1 => ctx.int(
                                        ui,
                                        "arp_octaves",
                                        "Oct",
                                        &p.arp_octaves,
                                        &Knob::plain(1.0, 3.0, 0, SLIDER_ARP),
                                    ),
                                    2 => ctx.float(
                                        ui,
                                        "arp_gate",
                                        "Gate",
                                        &p.arp_gate,
                                        &Knob::plain(5.0, 100.0, 0, SLIDER_ARP),
                                    ),
                                    3 => ctx.int(
                                        ui,
                                        "arp_ratchet",
                                        "Ratchet",
                                        &p.arp_ratchet,
                                        &Knob::plain(1.0, 4.0, 0, SLIDER_ARP),
                                    ),
                                    4 => ctx.int(
                                        ui,
                                        "arp_length",
                                        "Length",
                                        &p.arp_length,
                                        &Knob::plain(1.0, 16.0, 0, SLIDER_ARP),
                                    ),
                                    _ => {}
                                }
                            });
                        },
                    );

                    // The strip of lights.
                    let strip = Rect::from_min_size(
                        egui::pos2(body.right() - POS_STRIP_WIDTH, body.top()),
                        egui::vec2(POS_STRIP_WIDTH, body.height()),
                    );
                    position_strip(&ui.painter_at(strip), strip, ctx.state.arp_step);
                }
            }),
            _ => group(ui, rect, "VOICE", |ui, part| match part {
                Part::Header => ctx.popup(ui, "poly_mode", &p.poly_mode, 66.0),
                Part::Body => column_row(ui, "voice", 4, COLUMN_GAP, |index, ui| {
                    let p = ctx.params;
                    match index {
                        0 => ctx.float(
                            ui,
                            "glide",
                            "Glide",
                            &p.glide,
                            &Knob::logarithmic(0.0, 2_000.0, 1, SLIDER_C),
                        ),
                        1 => ctx.float(ui, "drift", "Drift", &p.drift, &Knob::percent(SLIDER_C)),
                        2 => {
                            ctx.float(ui, "velocity", "Vel", &p.velocity, &Knob::percent(SLIDER_C))
                        }
                        3 => ctx.int(
                            ui,
                            "voices",
                            "Voices",
                            &p.voices,
                            &Knob::plain(1.0, 16.0, 0, SLIDER_C),
                        ),
                        _ => {}
                    }
                }),
            }),
        }
    });
}

/// The fourth row: the effects, the macros and the output.
fn row_d(ui: &mut egui::Ui, rect: Rect, ctx: &mut Ctx) {
    group_row(
        ui,
        "row-d",
        rect,
        &ROW_D_WIDTHS,
        |index, rect, ui| match index {
            0 => group(ui, rect, "FX", |ui, part| match part {
                Part::Header => {}
                Part::Body => sections(ui, "fx", 3, GROUP_GAP, |index, _rect, ui| match index {
                    0 => {
                        caption(ui, "CHORUS", 9.0, TEXT);
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
                        caption(ui, "DELAY", 9.0, TEXT);
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
                        caption(ui, "REVERB", 9.0, TEXT);
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
            _ => group(ui, rect, "VCA", |ui, part| match part {
                Part::Header => {
                    let p = ctx.params;
                    text_label(ui, "CHARACTER", 9.0, TEXT);
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

        caption(ui, label, 9.0, TEXT);

        // The slider only takes the height that is left in its column above
        // the numeric field, so that a section never overflows its rectangle.
        let knob = &Knob {
            height: slider_height(knob.height, ui),
            ..*knob
        };

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
            self.state.dirty = true;
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

        caption(ui, label, 9.0, TEXT);

        // The slider only takes the height that is left in its column above
        // the numeric field, so that a section never overflows its rectangle.
        let knob = &Knob {
            height: slider_height(knob.height, ui),
            ..*knob
        };

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
            self.state.dirty = true;
        }
        if reset {
            self.reset_to_default(id, param);
        }
        if self.end_gesture(id, &response, ui) {
            self.setter.end_set_parameter(param);
        }

        self.int_field(ui, id, param, knob, FIELD_WIDTH, FIELD_HEIGHT);
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
                    self.state.dirty = true;
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
                    self.state.dirty = true;
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
                        self.state.dirty = true;
                    }
                }
            })
            .response;

        // A double click on the button of the popup resets the parameter to
        // its factory default.
        if response.double_clicked() {
            self.setter
                .set_parameter(param, param.default_plain_value());
            self.state.dirty = true;
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
        // A flat white square with a thin outline and a black check mark.
        flat_box(&painter, rect, TRACK);
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
            self.state.dirty = true;
        } else if response.clicked() {
            self.setter.set_parameter(param, !param.value());
            self.state.dirty = true;
        }

        text_label(ui, label, 9.0, TEXT);
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

            // The active button of the pair is filled darker.
            let fill = if index == current || response.is_pointer_button_down_on() {
                BUTTON_ACTIVE
            } else {
                BUTTON_BG
            };
            flat_box(&painter, rect, fill);
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
                self.state.dirty = true;
            } else if response.clicked() {
                self.setter.set_parameter(param, T::from_index(index));
                self.state.dirty = true;
            }
        }
    }

    /// Loads the factory preset with the given index.
    fn load_preset(&mut self, index: usize) {
        let index = index % PRESETS.len();
        self.state.preset_index = index;
        self.state.dirty = false;
        // A factory preset replaces the patch of the user.
        self.state.patch_active = None;
        self.state.patch_name = PRESETS[index].name.to_string();
        presets::apply(self.params, self.setter, index);
    }

    /// Draws the window of the patch browser.
    fn patch_window(&mut self, ui: &egui::Ui) {
        let mut open = self.state.patch_open;
        let mut close = false;

        // The window opens in the middle of the panel.
        let center = ui.ctx().content_rect().center();
        let position = center - egui::vec2(PATCH_WINDOW_WIDTH * 0.5, 160.0);

        egui::Window::new("PATCHES")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(PATCH_WINDOW_WIDTH)
            .default_pos(position)
            .show(ui.ctx(), |ui| {
                // The folder in which the patches are stored.
                ui.horizontal(|ui| {
                    text_label(ui, "FOLDER", 9.0, TEXT);
                    let width = ui.available_width() - 44.0;
                    ui.add(
                        egui::TextEdit::singleline(&mut self.state.patch_dir_text)
                            .desired_width(width.max(80.0)),
                    );
                    if ui.button("SET").clicked() {
                        self.set_patch_folder();
                    }
                });

                // The hint that the folder of the text field is not the
                // folder that is in use yet.
                if !self.folder_is_applied() {
                    ui.colored_label(MUTED, FOLDER_HINT);
                }

                // The list of the patches of the folder.
                ui.horizontal(|ui| {
                    if ui.button("REFRESH").clicked() {
                        self.refresh_patch_list();
                    }
                    let count = self.state.patch_files.len();
                    ui.label(format!("{count} patches"));
                });

                egui::ScrollArea::vertical()
                    .max_height(150.0)
                    .show(ui, |ui| {
                        let files = self.state.patch_files.clone();
                        if files.is_empty() {
                            // An empty folder would leave a blank panel
                            // behind, so it gets a caption.
                            ui.colored_label(MUTED, NO_PATCHES);
                            return;
                        }
                        for path in files {
                            let Some(stem) = path.file_stem() else {
                                continue;
                            };
                            let name = stem.to_string_lossy().into_owned();
                            if ui.selectable_label(false, name).clicked() {
                                self.load_patch_file(&path);
                            }
                        }
                    });

                ui.separator();

                // The name of the patch and the button that saves it.
                ui.horizontal(|ui| {
                    text_label(ui, "NAME", 9.0, TEXT);
                    let width = ui.available_width() - 48.0;
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut self.state.patch_name)
                            .desired_width(width.max(80.0)),
                    );
                    if response.changed() {
                        // A new name is a new file, so the file does not have
                        // to be confirmed again.
                        self.state.patch_confirm_overwrite = false;
                    }
                    if ui.button("SAVE").clicked() {
                        self.save_patch();
                    }
                });

                // The message of the last action.
                let color = if self.state.patch_status_error {
                    STATUS_ERROR
                } else {
                    STATUS_OK
                };
                ui.colored_label(color, &self.state.patch_status);

                if ui.button("CLOSE").clicked() {
                    close = true;
                }
            });

        if close {
            self.state.patch_open = false;
        } else {
            self.state.patch_open = open;
        }
    }

    /// The folder in which the patches are stored.
    ///
    /// It is the folder that was applied with SET, not the text of the field,
    /// so that the browser always works in the folder that was chosen.
    fn patch_dir(&self) -> PathBuf {
        self.state.patch_config.patch_dir.clone()
    }

    /// Whether the folder of the text field is the folder that is in use.
    fn folder_is_applied(&self) -> bool {
        Path::new(self.state.patch_dir_text.trim()) == self.state.patch_config.patch_dir
    }

    /// Reads the list of the patches of the folder again.
    fn refresh_patch_list(&mut self) {
        if !self.folder_is_applied() {
            // The folder of the text field is not the folder that is in use,
            // so the list is not read from the wrong folder.
            self.set_status(FOLDER_HINT.to_string(), true);
            return;
        }

        self.state.patch_files = patch::list_patches(&self.patch_dir());
        // A new list means that the files may have changed, so a pending
        // confirmation does not belong to any of them anymore.
        self.state.patch_confirm_overwrite = false;
    }

    /// Loads a patch of the folder into the synthesizer.
    fn load_patch_file(&mut self, path: &Path) {
        match patch::load_patch(path, self.params, self.setter) {
            Ok(name) => {
                // The patch that is loaded is not the one that was waiting
                // for a confirmation anymore.
                self.state.patch_confirm_overwrite = false;
                self.state.patch_active = Some(name.clone());
                self.state.patch_name = name.clone();
                self.state.dirty = false;
                self.set_status(format!("Loaded {name}"), false);
            }
            Err(error) => {
                self.set_status(error, true);
            }
        }
    }

    /// Saves the current sound as a patch of the folder.
    fn save_patch(&mut self) {
        if !self.folder_is_applied() {
            // The patch would end up in the wrong folder.
            self.set_status(FOLDER_HINT.to_string(), true);
            return;
        }

        let dir = self.patch_dir();
        let name = patch::sanitize_file_name(&self.state.patch_name);
        let path = dir.join(format!("{name}.json"));

        if path.exists() && !self.state.patch_confirm_overwrite {
            // The file is not overwritten without a second click.
            self.state.patch_confirm_overwrite = true;
            self.set_status("File exists - click SAVE again".to_string(), true);
            return;
        }

        match patch::save_patch(&dir, &name, self.params) {
            Ok(_) => {
                self.state.patch_confirm_overwrite = false;
                self.state.patch_name = name.clone();
                self.state.patch_active = Some(name);
                self.state.dirty = false;
                self.refresh_patch_list();
                self.set_status("Saved".to_string(), false);
            }
            Err(error) => {
                self.set_status(error, true);
            }
        }
    }

    /// Sets the folder in which the patches are stored.
    fn set_patch_folder(&mut self) {
        let text = self.state.patch_dir_text.trim().to_string();
        if text.is_empty() {
            // An empty folder would be the working directory of the process.
            self.set_status("The folder must not be empty".to_string(), true);
            return;
        }

        let path = absolute_path(&expand_home(&text));
        let config = patch::PatchConfig {
            patch_dir: path.clone(),
        };

        if let Err(error) = patch::save_config(&config) {
            self.set_status(format!("The folder could not be saved: {error}"), true);
            return;
        }

        self.state.patch_config = config;
        self.state.patch_dir_text = path.to_string_lossy().into_owned();
        self.state.patch_confirm_overwrite = false;
        self.refresh_patch_list();
        self.set_status("Folder set".to_string(), false);
    }

    /// Writes a message into the status line of the patch browser.
    fn set_status(&mut self, message: String, error: bool) {
        self.state.patch_status = message;
        self.state.patch_status_error = error;
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
        // The sound was changed by the user, so it is marked as dirty.
        self.state.dirty = true;

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

/// The two parts of a section.
#[derive(Clone, Copy)]
enum Part {
    Header,
    Body,
}

/// Lays out a row of sections of the given widths.
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

/// The colour of the title of a section: blue for the sound sources, the
/// pulse widths and the arpeggiator, olive for the envelopes and the
/// amplifier, gray for the rest.
fn title_color(title: &str) -> Color32 {
    match title {
        "VCO 1" | "VCO 2" | "SOURCE MIXER" | "VCF" | "PULSE WIDTH" | "ARP" => TITLE_BLUE,
        "MOD ENV" | "AMP ENV" | "LFO / CLK" | "VCA" => TITLE_OLIVE,
        _ => TITLE_GRAY,
    }
}

/// Draws a section and hands its header and body to the given callback.
fn group(ui: &mut egui::Ui, rect: Rect, title: &str, mut add: impl FnMut(&mut egui::Ui, Part)) {
    let painter = ui.painter_at(rect);
    let color = title_color(title);

    // The title of the section. It is drawn twice, half a pixel apart, so
    // that it reads as bold without a second font.
    let font = FontId::proportional(10.0);
    let galley = painter.layout_no_wrap(title.to_ascii_uppercase(), font, color);
    let strip = Rect::from_min_size(
        rect.left_top() + egui::vec2(GROUP_PADDING, 0.0),
        egui::vec2(galley.size().x + 8.0, TITLE_HEIGHT),
    );
    let title_pos = strip.left_top() + egui::vec2(4.0, (TITLE_HEIGHT - galley.size().y) * 0.5);
    painter.galley(title_pos, galley.clone(), color);
    painter.galley(title_pos + egui::vec2(0.5, 0.0), galley, color);

    // The rule below the title spans the whole section.
    let rule_y = rect.top() + TITLE_HEIGHT;
    painter.line_segment(
        [
            egui::pos2(rect.left(), rule_y),
            egui::pos2(rect.right(), rule_y),
        ],
        Stroke::new(1.0, RULE),
    );

    // The header of the section, next to the title.
    let header = Rect::from_min_size(
        egui::pos2(strip.right() + 4.0, rect.top()),
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

    // The body of the section, below the rule. It has no frame, so its
    // content can use the full height of the row.
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

/// Lays out a row of sub sections inside a section.
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
            Stroke::new(1.0, RULE),
        );
    }

    ui.advance_cursor_after_rect(Rect::from_min_size(start, egui::vec2(width, height)));
}

/// Lays out a row of parameter columns inside a section. All the columns
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

/// The smallest height that a slider is ever given. Rows are laid out so that
/// this is never reached; it only keeps a slider usable in a squeezed layout.
const MIN_SLIDER_HEIGHT: f32 = 24.0;

/// The height of a slider: the size that its section asks for, but never more
/// than the height that is actually left in the column above the numeric
/// field. This fills the rows completely and makes an overflow impossible.
fn slider_height(wanted: f32, ui: &egui::Ui) -> f32 {
    // What is left after the caption: the slider, the gap below it and the
    // numeric field.
    let room = ui.available_height() - FIELD_HEIGHT - 2.0;
    wanted.min(room).max(MIN_SLIDER_HEIGHT)
}

/// A vertical slider: a flat white track with a thin outline, a light tick in
/// the middle and a black handle band that is fully embedded inside the
/// track: it never sticks out of it, no matter where the value is.
fn v_slider(ui: &mut egui::Ui, value: &mut f32, knob: &Knob) -> egui::Response {
    let (rect, mut response) =
        ui.allocate_exact_size(egui::vec2(knob.width, knob.height), Sense::click_and_drag());
    let painter = ui.painter_at(rect);

    // The track. It has no fill below the handle and no bevel at all.
    flat_box(&painter, rect, TRACK);
    let track = rect.shrink(GROOVE_INSET);

    // The tick in the middle of the track marks the centre of the range. The
    // handle covers it while the value sits in the middle.
    painter.line_segment(
        [
            egui::pos2(track.left(), rect.center().y),
            egui::pos2(track.right(), rect.center().y),
        ],
        Stroke::new(1.0, TICK),
    );

    // The handle itself: a black band that stays fully inside the track.
    let position = value_to_position(*value, knob.min, knob.max, knob.log, track);
    let handle_center = position.clamp(
        track.top() + HANDLE_HEIGHT * 0.5,
        track.bottom() - HANDLE_HEIGHT * 0.5,
    );
    let handle = Rect::from_center_size(
        egui::pos2(rect.center().x, handle_center),
        egui::vec2(track.width(), HANDLE_HEIGHT),
    );
    painter.rect_filled(handle, CornerRadius::ZERO, HANDLE);

    // Dragging anywhere on the track sets the value.
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
    // A flat white field with a thin outline, no bevel.
    flat_box(&painter, rect, FIELD_BG);

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
        let color = if index < voices { LED_ON } else { LED_OFF };
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

/// The strip of lights that shows the position of the arpeggiator.
///
/// The light of the step that is currently playing shines amber, the other
/// ones are dark.
fn position_strip(painter: &egui::Painter, rect: Rect, position: usize) {
    painter.text(
        egui::pos2(rect.center().x, rect.top() + LABEL_HEIGHT * 0.5),
        Align2::CENTER_CENTER,
        "POS",
        FontId::proportional(9.0),
        TEXT,
    );

    let row_width = POS_LEDS as f32 * POS_LED_WIDTH + (POS_LEDS as f32 - 1.0) * POS_LED_GAP;
    let left = (rect.center().x - row_width * 0.5).round();
    let top = rect.top() + LABEL_HEIGHT + 4.0;

    // The lights sit in a flat frame with a single outline.
    let frame = Rect::from_min_size(
        egui::pos2(left - 3.0, top - 3.0),
        egui::vec2(row_width + 6.0, POS_LED_HEIGHT + 6.0),
    );
    painter.rect_filled(frame, CornerRadius::ZERO, TRACK);
    painter.rect_stroke(
        frame,
        CornerRadius::ZERO,
        Stroke::new(1.0, LINE),
        egui::StrokeKind::Inside,
    );

    for index in 0..POS_LEDS {
        let x = left + index as f32 * (POS_LED_WIDTH + POS_LED_GAP);
        let color = if index == position {
            LED_AMBER
        } else {
            LED_OFF
        };
        painter.rect_filled(
            Rect::from_min_size(
                egui::pos2(x, top),
                egui::vec2(POS_LED_WIDTH, POS_LED_HEIGHT),
            ),
            CornerRadius::ZERO,
            color,
        );
    }
}

/// A small light: a flat square that is either lit or switched off.
fn led(painter: &egui::Painter, center: Pos2, color: Color32, on: bool) {
    let rect = Rect::from_center_size(center, egui::vec2(LED_SIZE, LED_SIZE));
    let fill = if on { color } else { LED_OFF };
    painter.rect_filled(rect, CornerRadius::ZERO, fill);
    painter.rect_stroke(
        rect,
        CornerRadius::ZERO,
        Stroke::new(1.0, LINE),
        egui::StrokeKind::Inside,
    );
}

/// The flat white display that shows the name of the preset.
fn lcd(painter: &egui::Painter, rect: Rect, text: &str) {
    flat_box(painter, rect, LCD_BG);
    painter.text(
        rect.left_top() + egui::vec2(7.0, rect.height() * 0.5),
        Align2::LEFT_CENTER,
        text,
        FontId::monospace(12.0),
        LCD_FG,
    );
}

/// A flat square button: it is filled darker while it is pressed.
fn button(ui: &mut egui::Ui, rect: Rect, text: &str) -> egui::Response {
    let response = ui.interact(
        rect,
        ui.scope_id().with(("sushi-button", text)),
        Sense::click(),
    );
    let painter = ui.painter_at(rect);

    let fill = if response.is_pointer_button_down_on() {
        BUTTON_ACTIVE
    } else {
        BUTTON_BG
    };
    flat_box(&painter, rect, fill);
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
    // A flat white field with a dark trace, no bevel.
    flat_box(painter, rect, LCD_BG);
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

/// Maps a value to its position in the track, where 0 is the bottom and 1 is
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

/// Maps a position in the track back to a value.
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

// --- The patch browser -------------------------------------------------------

/// Expands a leading `~/` into the home directory of the user.
fn expand_home(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME").filter(|home| !home.is_empty())
    {
        return PathBuf::from(home).join(rest);
    }

    PathBuf::from(path)
}

/// The absolute path of a folder.
fn absolute_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }

    match std::env::current_dir() {
        Ok(current) => current.join(path),
        Err(_) => path.to_path_buf(),
    }
}

// --- The theme ---------------------------------------------------------------

fn apply_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::light();
    visuals.dark_mode = false;
    visuals.override_text_color = Some(TEXT);
    visuals.panel_fill = BG;
    visuals.window_fill = BG;
    visuals.window_stroke = Stroke::new(1.0, LINE);
    visuals.window_corner_radius = CornerRadius::ZERO;
    visuals.window_shadow = egui::epaint::Shadow::NONE;
    visuals.menu_corner_radius = CornerRadius::ZERO;
    visuals.popup_shadow = egui::epaint::Shadow::NONE;
    visuals.extreme_bg_color = FIELD_BG;
    visuals.text_edit_bg_color = Some(FIELD_BG);
    visuals.faint_bg_color = BG;
    visuals.code_bg_color = FIELD_BG;
    visuals.hyperlink_color = TEXT;
    visuals.slider_trailing_fill = false;
    visuals.button_frame = true;
    visuals.selection.bg_fill = BUTTON_ACTIVE;
    visuals.selection.stroke = Stroke::new(1.0, LINE);

    let square = CornerRadius::ZERO;

    // Every widget is a flat fill with a single 1 px outline: no bevel, no
    // shadow and no expansion.
    visuals.widgets.noninteractive.bg_fill = BG;
    visuals.widgets.noninteractive.weak_bg_fill = BG;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, RULE);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.noninteractive.corner_radius = square;
    visuals.widgets.noninteractive.expansion = 0.0;

    // The buttons, the popups and the text fields of the panel.
    visuals.widgets.inactive.bg_fill = BUTTON_BG;
    visuals.widgets.inactive.weak_bg_fill = BUTTON_BG;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, LINE);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.inactive.corner_radius = square;
    visuals.widgets.inactive.expansion = 0.0;

    visuals.widgets.hovered.bg_fill = BUTTON_BG;
    visuals.widgets.hovered.weak_bg_fill = BUTTON_ACTIVE;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.hovered.corner_radius = square;
    visuals.widgets.hovered.expansion = 0.0;

    visuals.widgets.active.bg_fill = BUTTON_ACTIVE;
    visuals.widgets.active.weak_bg_fill = BUTTON_ACTIVE;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.active.corner_radius = square;
    visuals.widgets.active.expansion = 0.0;

    visuals.widgets.open.bg_fill = FIELD_BG;
    visuals.widgets.open.weak_bg_fill = BUTTON_ACTIVE;
    visuals.widgets.open.bg_stroke = Stroke::new(1.0, LINE);
    visuals.widgets.open.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.open.corner_radius = square;
    visuals.widgets.open.expansion = 0.0;

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
            egui::FontId::new(10.0, egui::FontFamily::Proportional),
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

/// Draws a flat box: one fill and a single 1 px border inside of it. There is
/// no bevel, no shadow and no second edge anywhere on the panel.
fn flat_box(painter: &egui::Painter, rect: Rect, fill: Color32) {
    painter.rect_filled(rect, CornerRadius::ZERO, fill);
    painter.rect_stroke(
        rect,
        CornerRadius::ZERO,
        Stroke::new(1.0, LINE),
        egui::StrokeKind::Inside,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::test_support::ENV_LOCK;
    use nice_plug::params::internals::ParamPtr;
    use std::cell::Cell;

    thread_local! {
        /// The number of gestures that have been opened and closed.
        static GESTURES: Cell<usize> = const { Cell::new(0) };
        /// The number of parameter values that have been set.
        static SETS: Cell<usize> = const { Cell::new(0) };
    }

    /// A counter that makes the names of the temporary folders unique.
    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    /// A folder in the temporary directory that is removed again when it is
    /// dropped.
    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        /// Creates a folder with the given name.
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "sushi-editor-{name}-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("the temporary folder could not be created");
            Self { path }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
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
        let arp_led = Arc::new(AtomicUsize::new(0));
        let arp_position = Arc::new(AtomicUsize::new(0));
        let voice_count = Arc::new(AtomicUsize::new(0));
        let mut editor = SushiEditor::new(params.clone(), rx, arp_led, arp_position, voice_count);
        let ctx = egui::Context::default();
        editor.gui_ctx = Some(GuiContext::new(Arc::new(Stub)));
        apply_theme(&ctx);
        (ctx, editor, params)
    }

    /// The context that the widgets of the panel use.
    fn make_ctx<'a, 'b>(
        editor: &'a mut SushiEditor,
        params: &'a SushiParams,
        setter: &'b ParamSetter<'b>,
    ) -> Ctx<'a, 'b> {
        Ctx {
            params,
            setter,
            state: &mut editor.state,
            history: &editor.history,
            voice_count: &editor.voice_count,
        }
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

    /// The rects of all the numeric value fields, in the order they are
    /// drawn.
    fn field_rects(ctx: &egui::Context, output: &egui::FullOutput) -> Vec<egui::Rect> {
        let mut rects: Vec<egui::Rect> = Vec::new();
        for clipped in ctx.tessellate(output.shapes.clone(), ctx.pixels_per_point()) {
            if let egui::epaint::Primitive::Mesh(_mesh) = clipped.primitive {
                let rect = clipped.clip_rect;
                let is_field = (rect.width() - FIELD_WIDTH).abs() < 0.5
                    && (rect.height() - FIELD_HEIGHT).abs() < 0.5;
                // Every field is drawn twice: once filled and once stroked.
                if is_field && !rects.contains(&rect) {
                    rects.push(rect);
                }
            }
        }
        rects
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

    /// Runs a few frames, so that the animations of the panel can settle.
    fn settle(ctx: &egui::Context, editor: &mut SushiEditor) -> egui::FullOutput {
        let mut last = None;
        for step in 0..10 {
            let mut raw = input(vec![]);
            raw.time = Some(step as f64 * 0.05);
            let mut output = ctx.run_ui(raw, |ui| {
                editor.draw(ui);
            });
            output.textures_delta.clear();
            last = Some(output);
        }
        last.unwrap()
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

    /// The index of the slider of the cutoff in the list of the sliders of a
    /// frame. It is the first slider of the filter, the fourth group of the
    /// first row.
    const CUTOFF_SLIDER: usize = 13;
    /// The index of the slider of the master tune. It is the first slider of
    /// the panel.
    const TUNE_SLIDER: usize = 0;
    /// The index of the slider of the number of voices. It is the last slider
    /// of the voice section.
    const VOICES_SLIDER: usize = 40;

    #[test]
    fn the_panel_has_the_expected_number_of_widgets() {
        let (ctx, mut editor, _params) = make_editor();
        run_frames(&ctx, &mut editor, vec![input(vec![])]);

        let counts = editor.state.counts;
        assert_eq!(counts.sliders, 54, "unexpected number of sliders");
        assert_eq!(counts.popups, 10, "unexpected number of popups");
        assert_eq!(counts.checks, 2, "unexpected number of check boxes");
        assert_eq!(counts.radios, 1, "unexpected number of radio pairs");
    }

    #[test]
    fn every_parameter_of_the_synthesizer_is_reachable() {
        let (ctx, mut editor, _params) = make_editor();
        run_frames(&ctx, &mut editor, vec![input(vec![])]);

        let counts = editor.state.counts;
        // The synthesizer has 67 parameters: 54 of them are on the panel as
        // sliders, 10 are choices, 2 are switches and one is a pair of
        // buttons. Every one of them can be reached.
        let continuous = counts.sliders;
        assert_eq!(continuous, 54);
        assert_eq!(
            continuous + counts.popups + counts.checks + counts.radios,
            67
        );
    }

    #[test]
    fn clicking_a_slider_sets_the_value() {
        let (ctx, mut editor, params) = make_editor();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let sliders = slider_rects(&ctx, &output);
        assert_eq!(sliders.len(), 54, "unexpected number of sliders");
        // The cutoff slider is the first one of the fourth group of the
        // first row.
        let rect = sliders[CUTOFF_SLIDER];
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
        let rect = sliders[CUTOFF_SLIDER];
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
        let rect = sliders[CUTOFF_SLIDER];
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
        let rect = sliders[CUTOFF_SLIDER];
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
        let rect = sliders[CUTOFF_SLIDER];
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
        // The cutoff slider is the first one of the fourth group of the first
        // row.
        let rect = sliders[CUTOFF_SLIDER];

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
        let rect = sliders[CUTOFF_SLIDER];

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
    fn double_clicking_a_check_box_resets_it_to_the_default() {
        let (ctx, mut editor, params) = make_editor();
        run_frames(&ctx, &mut editor, vec![input(vec![])]);
        // The first check box is the switch of the arpeggiator.
        let rect = editor.state.check_rects[0];
        let pos = rect.center();

        // A single click switches the check box on.
        run_frames(&ctx, &mut editor, click(pos));
        assert!(params.arp_on.value());

        // The double click puts it back to its default, even though the two
        // clicks would switch it twice.
        run_frames(&ctx, &mut editor, wait());
        run_frames(&ctx, &mut editor, double_click(pos));
        assert!(!params.arp_on.value());
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

    #[test]
    fn the_patches_button_does_not_overlap_the_voice_meter() {
        let rect = Rect::from_min_size(
            egui::pos2(MARGIN, MARGIN),
            egui::vec2(WINDOW_WIDTH - 2.0 * MARGIN, HEADER_HEIGHT),
        );
        let rects = header_rects(rect);

        // The button of the patch browser sits between the mutation and the
        // meter of the voices.
        assert!(rects.patches.left() >= rects.mutate.right());
        assert!(!rects.patches.intersects(rects.meter));
        // The meter still fits between the button and the lights of the
        // activity meter.
        assert!(rects.meter.right() <= rect.right() - 194.0);
        assert!(rects.meter.width() > VOICE_LEDS as f32 * VOICE_LED_WIDTH);
    }

    #[test]
    fn editing_a_parameter_marks_the_sound_as_dirty() {
        let (ctx, mut editor, _params) = make_editor();
        run_frames(&ctx, &mut editor, vec![input(vec![])]);
        assert!(
            !editor.state.dirty,
            "the sound was dirty before it was edited"
        );

        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let sliders = slider_rects(&ctx, &output);
        // The cutoff slider is the first one of the fourth group of the first
        // row.
        let rect = sliders[CUTOFF_SLIDER];

        // A click on the slider changes the sound.
        run_frames(&ctx, &mut editor, click(rect.center()));
        assert!(editor.state.dirty, "the sound was not marked as dirty");

        // A factory preset is a clean sound again.
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let (_, pos) = texts(&output)
            .into_iter()
            .find(|(text, _)| text == ">")
            .expect("the header has no next button");
        run_frames(&ctx, &mut editor, click(pos + egui::vec2(2.0, 2.0)));
        assert!(!editor.state.dirty, "the preset did not clean the sound up");
    }

    #[test]
    fn a_reset_marks_the_sound_as_dirty() {
        let (ctx, mut editor, _params) = make_editor();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let sliders = slider_rects(&ctx, &output);
        let rect = sliders[CUTOFF_SLIDER];

        run_frames(&ctx, &mut editor, wait());
        run_frames(&ctx, &mut editor, double_click(rect.center()));

        assert!(
            editor.state.dirty,
            "the reset did not mark the sound as dirty"
        );
    }

    #[test]
    fn switching_a_check_box_marks_the_sound_as_dirty() {
        let (ctx, mut editor, _params) = make_editor();
        run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let rect = editor.state.check_rects[0];

        run_frames(&ctx, &mut editor, click(rect.center()));

        assert!(
            editor.state.dirty,
            "the check box did not mark the sound as dirty"
        );
    }

    #[test]
    fn the_patches_button_opens_the_patch_browser() {
        let (ctx, mut editor, _params) = make_editor();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let (_, pos) = texts(&output)
            .into_iter()
            .find(|(text, _)| text == "PATCHES")
            .expect("the header has no patch button");
        let button = pos + egui::vec2(2.0, 2.0);

        assert!(!editor.state.patch_open);
        run_frames(&ctx, &mut editor, click(button));
        assert!(editor.state.patch_open);
    }

    #[test]
    fn the_patch_browser_shows_its_contents() {
        let (ctx, mut editor, _params) = make_editor();
        editor.state.patch_open = true;

        // The window fades in, so it needs a few frames.
        let output = settle(&ctx, &mut editor);
        let names: Vec<String> = texts(&output).into_iter().map(|(text, _)| text).collect();

        for expected in ["FOLDER", "SET", "REFRESH", "NAME", "SAVE", "CLOSE"] {
            assert!(
                names.contains(&expected.to_string()),
                "the browser has no {expected}: {names:?}"
            );
        }

        // The button in the corner of the window closes it.
        let (_, pos) = texts(&output)
            .into_iter()
            .find(|(text, _)| text == "CLOSE")
            .expect("the browser has no close button");
        run_frames(&ctx, &mut editor, click(pos + egui::vec2(2.0, 2.0)));

        assert!(!editor.state.patch_open);
    }

    #[test]
    fn a_patch_can_be_saved_and_loaded() {
        let (_ctx, mut editor, params) = make_editor();
        let dir = TempDir::new("browser");
        editor.state.patch_config.patch_dir = dir.path.clone();
        editor.state.patch_dir_text = dir.path.to_string_lossy().into_owned();
        editor.state.patch_name = "MY PATCH".to_string();
        editor.state.dirty = true;

        let gui_ctx = editor.gui_ctx.clone().expect("the editor has no host");
        let setter = gui_ctx.param_setter();
        setter.set_parameter(&params.cutoff, 1_234.0);

        // Save the patch.
        {
            let mut ctx = make_ctx(&mut editor, &params, &setter);
            ctx.save_patch();
        }

        let path = dir.path.join("MY PATCH.json");
        assert!(path.exists(), "the patch was not written");
        assert_eq!(editor.state.patch_status, "Saved");
        assert_eq!(editor.state.patch_active.as_deref(), Some("MY PATCH"));
        assert!(!editor.state.dirty);
        assert_eq!(editor.state.patch_files.len(), 1);

        // Change the sound and load the patch again.
        setter.set_parameter(&params.cutoff, 500.0);
        {
            let mut ctx = make_ctx(&mut editor, &params, &setter);
            ctx.load_patch_file(&path);
        }

        assert!((params.cutoff.value() - 1_234.0).abs() < 1.0);
        assert_eq!(editor.state.patch_status, "Loaded MY PATCH");
        assert_eq!(editor.state.patch_active.as_deref(), Some("MY PATCH"));
    }

    #[test]
    fn a_patch_can_be_loaded_from_the_list_of_the_browser() {
        let (ctx, mut editor, params) = make_editor();
        let dir = TempDir::new("list");
        editor.state.patch_config.patch_dir = dir.path.clone();
        editor.state.patch_dir_text = dir.path.to_string_lossy().into_owned();
        editor.state.patch_name = "MY PATCH".to_string();

        let gui_ctx = editor.gui_ctx.clone().expect("the editor has no host");
        let setter = gui_ctx.param_setter();
        setter.set_parameter(&params.cutoff, 1_234.0);
        {
            let mut ctx = make_ctx(&mut editor, &params, &setter);
            ctx.save_patch();
        }

        // The name of the patch is changed, so that the entry of the list can
        // be told apart from the field of the name.
        editor.state.patch_name = "SOMETHING ELSE".to_string();
        editor.state.patch_active = None;
        editor.state.patch_open = true;
        let output = settle(&ctx, &mut editor);

        let positions: Vec<egui::Pos2> = texts(&output)
            .into_iter()
            .filter(|(text, _)| text == "MY PATCH")
            .map(|(_, pos)| pos)
            .collect();
        assert_eq!(positions.len(), 1, "the patch is not in the list");

        // Change the sound and load the patch from the list.
        setter.set_parameter(&params.cutoff, 500.0);
        run_frames(
            &ctx,
            &mut editor,
            click(positions[0] + egui::vec2(2.0, 2.0)),
        );

        assert!((params.cutoff.value() - 1_234.0).abs() < 1.0);
        assert_eq!(editor.state.patch_active.as_deref(), Some("MY PATCH"));
    }

    #[test]
    fn a_patch_is_not_overwritten_without_a_confirmation() {
        let (_ctx, mut editor, params) = make_editor();
        let dir = TempDir::new("overwrite");
        editor.state.patch_config.patch_dir = dir.path.clone();
        editor.state.patch_dir_text = dir.path.to_string_lossy().into_owned();
        editor.state.patch_name = "MY PATCH".to_string();

        let gui_ctx = editor.gui_ctx.clone().expect("the editor has no host");
        let setter = gui_ctx.param_setter();

        {
            let mut ctx = make_ctx(&mut editor, &params, &setter);
            ctx.save_patch();
        }
        assert_eq!(editor.state.patch_status, "Saved");

        // The second save asks first.
        {
            let mut ctx = make_ctx(&mut editor, &params, &setter);
            ctx.save_patch();
        }
        assert_eq!(editor.state.patch_status, "File exists - click SAVE again");
        assert!(editor.state.patch_confirm_overwrite);

        // The third save overwrites the file.
        {
            let mut ctx = make_ctx(&mut editor, &params, &setter);
            ctx.save_patch();
        }
        assert_eq!(editor.state.patch_status, "Saved");
        assert!(!editor.state.patch_confirm_overwrite);
    }

    #[test]
    fn a_pending_confirmation_is_forgotten_when_a_patch_is_loaded() {
        let (_ctx, mut editor, params) = make_editor();
        let dir = TempDir::new("confirm");
        editor.state.patch_config.patch_dir = dir.path.clone();
        editor.state.patch_dir_text = dir.path.to_string_lossy().into_owned();
        editor.state.patch_name = "MY PATCH".to_string();

        let gui_ctx = editor.gui_ctx.clone().expect("the editor has no host");
        let setter = gui_ctx.param_setter();

        {
            let mut ctx = make_ctx(&mut editor, &params, &setter);
            ctx.save_patch();
        }
        assert_eq!(editor.state.patch_status, "Saved");

        // The second save asks before it overwrites the file.
        {
            let mut ctx = make_ctx(&mut editor, &params, &setter);
            ctx.save_patch();
        }
        assert!(editor.state.patch_confirm_overwrite);

        // A patch that is loaded is a different file, so the confirmation of
        // the old one is forgotten.
        let other = dir.path.join("OTHER.json");
        std::fs::write(
            &other,
            r#"{"format":"sushi-patch","version":1,"name":"OTHER","params":{"cutoff":800.0}}"#,
        )
        .expect("the patch was not written");

        {
            let mut ctx = make_ctx(&mut editor, &params, &setter);
            ctx.load_patch_file(&other);
        }

        assert_eq!(editor.state.patch_active.as_deref(), Some("OTHER"));
        assert!(
            !editor.state.patch_confirm_overwrite,
            "the confirmation was not forgotten"
        );
    }

    #[test]
    fn a_refresh_forgets_a_pending_confirmation() {
        let (_ctx, mut editor, params) = make_editor();
        let dir = TempDir::new("refresh");
        editor.state.patch_config.patch_dir = dir.path.clone();
        editor.state.patch_dir_text = dir.path.to_string_lossy().into_owned();

        let gui_ctx = editor.gui_ctx.clone().expect("the editor has no host");
        let setter = gui_ctx.param_setter();

        editor.state.patch_confirm_overwrite = true;
        {
            let mut ctx = make_ctx(&mut editor, &params, &setter);
            ctx.refresh_patch_list();
        }

        assert!(
            !editor.state.patch_confirm_overwrite,
            "the confirmation was not forgotten"
        );
    }

    #[test]
    fn the_browser_uses_the_folder_that_was_applied() {
        let (_ctx, mut editor, params) = make_editor();
        let dir = TempDir::new("applied");
        let other = TempDir::new("typed");
        editor.state.patch_config.patch_dir = dir.path.clone();
        editor.state.patch_dir_text = dir.path.to_string_lossy().into_owned();
        editor.state.patch_name = "MY PATCH".to_string();

        let gui_ctx = editor.gui_ctx.clone().expect("the editor has no host");
        let setter = gui_ctx.param_setter();

        // A folder that was typed in but not applied is not used.
        editor.state.patch_dir_text = other.path.to_string_lossy().into_owned();
        {
            let mut ctx = make_ctx(&mut editor, &params, &setter);
            ctx.save_patch();
        }

        assert_eq!(editor.state.patch_status, FOLDER_HINT);
        assert!(!dir.path.join("MY PATCH.json").exists());
        assert!(!other.path.join("MY PATCH.json").exists());

        // The same is true for the list of the patches.
        {
            let mut ctx = make_ctx(&mut editor, &params, &setter);
            ctx.refresh_patch_list();
        }
        assert_eq!(editor.state.patch_status, FOLDER_HINT);
    }

    #[test]
    fn an_empty_folder_is_not_applied() {
        let (_ctx, mut editor, params) = make_editor();
        let before = editor.state.patch_config.patch_dir.clone();
        editor.state.patch_dir_text = "   ".to_string();

        let gui_ctx = editor.gui_ctx.clone().expect("the editor has no host");
        let setter = gui_ctx.param_setter();
        {
            let mut ctx = make_ctx(&mut editor, &params, &setter);
            ctx.set_patch_folder();
        }

        assert_eq!(editor.state.patch_status, "The folder must not be empty");
        assert!(editor.state.patch_status_error);
        // The folder was not changed.
        assert_eq!(editor.state.patch_config.patch_dir, before);
    }

    #[test]
    fn the_browser_hints_at_the_folder_that_has_to_be_applied() {
        let (ctx, mut editor, _params) = make_editor();
        let dir = TempDir::new("hint");
        editor.state.patch_config.patch_dir = dir.path.clone();
        editor.state.patch_dir_text = dir.path.to_string_lossy().into_owned();
        editor.state.patch_open = true;

        // The folder of the text field is the folder that is in use, so there
        // is no hint.
        let output = settle(&ctx, &mut editor);
        let names: Vec<String> = texts(&output).into_iter().map(|(text, _)| text).collect();
        assert!(
            !names.contains(&FOLDER_HINT.to_string()),
            "the browser has a hint: {names:?}"
        );

        // A folder that was typed in is not in use yet.
        editor.state.patch_dir_text = "/somewhere/else".to_string();
        let output = settle(&ctx, &mut editor);
        let names: Vec<String> = texts(&output).into_iter().map(|(text, _)| text).collect();
        assert!(
            names.contains(&FOLDER_HINT.to_string()),
            "the browser has no hint: {names:?}"
        );
    }

    #[test]
    fn the_browser_says_when_there_are_no_patches() {
        let (ctx, mut editor, _params) = make_editor();
        let dir = TempDir::new("empty");
        editor.state.patch_config.patch_dir = dir.path.clone();
        editor.state.patch_dir_text = dir.path.to_string_lossy().into_owned();
        editor.state.patch_files.clear();
        editor.state.patch_open = true;

        let output = settle(&ctx, &mut editor);
        let names: Vec<String> = texts(&output).into_iter().map(|(text, _)| text).collect();
        assert!(
            names.contains(&NO_PATCHES.to_string()),
            "the browser has no caption: {names:?}"
        );
    }

    #[test]
    fn the_display_shows_the_name_of_the_patch() {
        let (ctx, mut editor, _params) = make_editor();
        editor.state.patch_active = Some("MY PATCH".to_string());

        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let names: Vec<String> = texts(&output).into_iter().map(|(text, _)| text).collect();
        assert!(
            names.contains(&"MY PATCH".to_string()),
            "the display does not show the patch: {names:?}"
        );

        // A sound that was changed is marked with a star.
        editor.state.dirty = true;
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let names: Vec<String> = texts(&output).into_iter().map(|(text, _)| text).collect();
        assert!(
            names.contains(&"MY PATCH*".to_string()),
            "the display does not mark the patch: {names:?}"
        );
    }

    #[test]
    fn a_factory_preset_replaces_the_patch_of_the_user() {
        let (ctx, mut editor, _params) = make_editor();
        editor.state.patch_active = Some("MY PATCH".to_string());

        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let (_, pos) = texts(&output)
            .into_iter()
            .find(|(text, _)| text == ">")
            .expect("the header has no next button");
        let button = pos + egui::vec2(2.0, 2.0);

        run_frames(&ctx, &mut editor, click(button));

        assert!(editor.state.patch_active.is_none());
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let names: Vec<String> = texts(&output).into_iter().map(|(text, _)| text).collect();
        assert!(
            names.contains(&"02 GLASS KEYS".to_string()),
            "the display does not show the preset: {names:?}"
        );
    }

    #[test]
    fn the_folder_of_the_patches_can_be_changed() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let old = std::env::var_os("XDG_CONFIG_HOME");
        let home = TempDir::new("config");
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", &home.path);
        }

        let (_ctx, mut editor, params) = make_editor();
        let dir = TempDir::new("patches");
        editor.state.patch_dir_text = dir.path.to_string_lossy().into_owned();

        let gui_ctx = editor.gui_ctx.clone().expect("the editor has no host");
        let setter = gui_ctx.param_setter();
        {
            let mut ctx = make_ctx(&mut editor, &params, &setter);
            ctx.set_patch_folder();
        }

        assert_eq!(editor.state.patch_status, "Folder set");
        assert_eq!(editor.state.patch_dir_text, dir.path.to_string_lossy());
        // The folder was written into the configuration file.
        assert_eq!(patch::load_config().patch_dir, dir.path);

        match old {
            Some(value) => unsafe {
                std::env::set_var("XDG_CONFIG_HOME", value);
            },
            None => unsafe {
                std::env::remove_var("XDG_CONFIG_HOME");
            },
        }
    }

    #[test]
    fn a_tilde_in_the_folder_is_expanded() {
        let path = expand_home("~/Music");
        let home = std::env::var_os("HOME").expect("the user has no home");
        assert_eq!(path, PathBuf::from(home).join("Music"));
        // A path that does not start with a tilde stays as it is.
        assert_eq!(expand_home("/tmp/patches"), PathBuf::from("/tmp/patches"));
    }

    #[test]
    fn the_pattern_popup_knows_the_as_played_pattern() {
        let (ctx, mut editor, _params) = make_editor();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let names: Vec<String> = texts(&output).into_iter().map(|(text, _)| text).collect();
        assert!(
            names.contains(&"Up".to_string()),
            "the pattern popup is missing: {names:?}"
        );

        // The popup holds every pattern, the as played one among them.
        let (_, pos) = texts(&output)
            .into_iter()
            .find(|(text, _)| text == "Up")
            .unwrap();
        let button = pos + egui::vec2(2.0, 2.0);
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
        let names: Vec<String> = texts(&output).into_iter().map(|(text, _)| text).collect();
        assert!(
            names.contains(&"As Played".to_string()),
            "the popup has no as played pattern: {names:?}"
        );
    }

    #[test]
    fn the_position_strip_lights_the_current_step() {
        let (ctx, mut editor, _params) = make_editor();
        editor.arp_position.store(5, Ordering::Relaxed);

        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);

        // Only the light of the fifth step shines.
        let lights: Vec<egui::Rect> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::epaint::Shape::Rect(rect)
                    if rect.fill == LED_AMBER
                        && rect.rect.width() == POS_LED_WIDTH
                        && rect.rect.height() == POS_LED_HEIGHT =>
                {
                    Some(rect.rect)
                }
                _ => None,
            })
            .collect();

        assert_eq!(lights.len(), 1, "the strip does not have one lit light");
    }

    #[test]
    fn the_sections_of_the_panel_have_the_blueprint_layout() {
        let (ctx, mut editor, _params) = make_editor();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);

        // The title of every section and the place where it has to be: the
        // left edge of its section and the top of its row.
        let expected = [
            ("VCO 1", 16.0, 64.0),
            ("VCO 2", 224.0, 64.0),
            ("SOURCE MIXER", 392.0, 64.0),
            ("VCF", 680.0, 64.0),
            ("PULSE WIDTH", 16.0, 216.0),
            ("MOD ENV", 184.0, 216.0),
            ("AMP ENV", 372.0, 216.0),
            ("LFO / CLK", 560.0, 216.0),
            ("ARP", 16.0, 360.0),
            ("VOICE", 730.0, 360.0),
            ("FX", 16.0, 506.0),
            ("MACROS", 492.0, 506.0),
            ("VCA", 730.0, 506.0),
        ];

        let titles: Vec<(String, egui::Pos2)> = texts(&output)
            .into_iter()
            .filter(|(text, _)| {
                text.chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == ' ' || c == '/')
            })
            .collect();

        for (title, x, row) in expected {
            let found = titles
                .iter()
                .find(|(text, _)| text == title)
                .unwrap_or_else(|| panic!("the panel has no {title}"));
            let (_, pos) = found;
            assert!(
                (pos.x - x).abs() < 1.0,
                "the title {title} is at {pos:?} instead of {x}"
            );
            assert!(
                pos.y >= row && pos.y <= row + TITLE_HEIGHT,
                "the title {title} is at {pos:?} instead of in the row of {row}"
            );
        }
    }

    #[test]
    fn the_widgets_of_the_panel_fit_into_the_window() {
        let (ctx, mut editor, _params) = make_editor();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let panel = Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            egui::vec2(WINDOW_WIDTH, WINDOW_HEIGHT),
        );

        // Every slider is inside of the panel and touches no other slider.
        let sliders = slider_rects(&ctx, &output);
        assert_eq!(sliders.len(), 54, "unexpected number of sliders");
        for (index, rect) in sliders.iter().enumerate() {
            assert!(
                panel.contains_rect(*rect),
                "a slider is outside of the panel: {rect:?}"
            );
            for other in &sliders[index + 1..] {
                assert!(
                    !rect.intersects(*other),
                    "two sliders touch each other: {rect:?} {other:?}"
                );
            }
        }

        // The same goes for the fields of the values.
        let fields = field_rects(&ctx, &output);
        assert_eq!(fields.len(), 54, "unexpected number of value fields");
        for (index, rect) in fields.iter().enumerate() {
            assert!(
                panel.contains_rect(*rect),
                "a value field is outside of the panel: {rect:?}"
            );
            for other in &fields[index + 1..] {
                assert!(
                    !rect.intersects(*other),
                    "two value fields touch each other: {rect:?} {other:?}"
                );
            }
        }

        // And no text sticks out of the panel.
        for shape in &output.shapes {
            if let egui::epaint::Shape::Text(text) = &shape.shape {
                let rect = Rect::from_min_size(text.pos, text.galley.size());
                assert!(
                    panel.contains_rect(rect),
                    "the text {:?} is outside of the panel: {rect:?}",
                    text.galley.job.text
                );
            }
        }
    }

    #[test]
    fn the_new_parameters_can_be_edited_and_reset() {
        let (ctx, mut editor, params) = make_editor();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let sliders = slider_rects(&ctx, &output);

        // The tune is the first slider of the panel, the number of voices is
        // the last one of the voice section.
        let tune = sliders[TUNE_SLIDER];
        let voices = sliders[VOICES_SLIDER];

        // A click at the top of the tune slider raises it by an octave.
        let top = egui::pos2(tune.center().x, tune.top() + 2.0);
        run_frames(&ctx, &mut editor, click(top));
        assert!(
            params.tune.value() > 11.0,
            "the tune was not changed: {}",
            params.tune.value()
        );

        // The double click puts it back to its default.
        run_frames(&ctx, &mut editor, wait());
        run_frames(&ctx, &mut editor, double_click(tune.center()));
        assert!(
            params.tune.value().abs() < 0.001,
            "the tune was not reset: {}",
            params.tune.value()
        );

        // A click at the bottom of the voice slider leaves a single voice.
        let bottom = egui::pos2(voices.center().x, voices.bottom() - 3.0);
        run_frames(&ctx, &mut editor, click(bottom));
        assert_eq!(params.voices.value(), 1);

        // The double click puts the number of voices back to its default.
        run_frames(&ctx, &mut editor, wait());
        run_frames(&ctx, &mut editor, double_click(voices.center()));
        assert_eq!(params.voices.value(), 16);
    }
}
