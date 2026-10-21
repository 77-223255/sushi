//! The Sushi editor: a flat blueprint panel.
//!
//! Every continuous parameter is a vertical slider whose handle is a band that
//! is fully embedded inside the track (it never sticks out), every parameter
//! can also be typed into a small numeric popup that opens with a right click,
//! and every choice parameter opens a popup menu. The parameters are grouped
//! into sections that are named by a coloured title and a thin rule below a
//! header that holds the wordmark, the factory presets and a small
//! oscilloscope. Nothing on the panel has a bevel, a shadow or a rounded
//! corner: every widget is a flat fill with a single 1 px border.
//!
//! The panel is drawn with the Silkscreen pixel font, which is embedded in the
//! program. Every size of the design is multiplied by the zoom of the panel,
//! and so is every font.

mod presets;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Layout as EguiLayout, Margin, Pos2, Rect, Sense,
    Stroke,
};
use nice_plug::context::gui::GuiContext;
use nice_plug::prelude::*;
use nice_plug_egui::NiceEguiApp;

use crate::params::SushiParams;
use crate::patch;
use presets::{PRESETS, Rng};

// --- Light SH-101-inspired palette ------------------------------------------

/// The warm paper background of the panel, close to the reference.
const BG: Color32 = Color32::from_rgb(0xF1, 0xF0, 0xEB);
/// The slightly darker strip behind the header.
const HEADER_BG: Color32 = Color32::from_rgb(0xE6, 0xE5, 0xDF);
/// Fine structural rules between sections.
const RULE: Color32 = Color32::from_rgb(0xBC, 0xBB, 0xB4);
/// The dark outline of fields, tracks and the panel.
const LINE: Color32 = Color32::from_rgb(0x55, 0x55, 0x51);
const TEXT: Color32 = Color32::from_rgb(0x1D, 0x1D, 0x1B);
const MUTED: Color32 = Color32::from_rgb(0x75, 0x73, 0x6B);
/// The near-white paper of buttons and fader tracks.
const PAPER: Color32 = Color32::from_rgb(0xFB, 0xFA, 0xF6);
/// The blue of the section titles, taken from the SH-101 reference.
const TITLE_BLUE: Color32 = Color32::from_rgb(0x2E, 0x5C, 0x9C);
/// Parameter captions use a slightly quieter blue-grey.
const LABEL: Color32 = Color32::from_rgb(0x35, 0x5D, 0x88);
/// The one interactive accent: the MOD page underline and focus states.
const ACCENT: Color32 = TITLE_BLUE;
/// The track of a slider: white, outlined in dark ink.
const TRACK: Color32 = Color32::from_rgb(0xFB, 0xFA, 0xF6);
/// The handle band of a slider: black, like the reference.
const HANDLE: Color32 = Color32::from_rgb(0x1C, 0x1C, 0x1A);
/// The centre/scale marks of a slider.
const TICK: Color32 = Color32::from_rgb(0x8A, 0x89, 0x81);
const FIELD_BG: Color32 = Color32::from_rgb(0xFC, 0xFB, 0xF7);
const BUTTON_BG: Color32 = Color32::from_rgb(0xE3, 0xE2, 0xDC);
const BUTTON_ACTIVE: Color32 = Color32::from_rgb(0xD4, 0xDC, 0xE8);
/// The light preset display and scope screen.
const LCD_BG: Color32 = Color32::from_rgb(0xF6, 0xF6, 0xF1);
const LCD_FG: Color32 = Color32::from_rgb(0x23, 0x4F, 0x82);
const LED_ON: Color32 = Color32::from_rgb(0x2E, 0x5C, 0x9C);
const LED_RED: Color32 = Color32::from_rgb(0xB2, 0x3A, 0x32);
const LED_OFF: Color32 = Color32::from_rgb(0xC9, 0xC8, 0xC1);
const TRACE: Color32 = Color32::from_rgb(0x2E, 0x5C, 0x9C);
const STATUS_OK: Color32 = Color32::from_rgb(0x2E, 0x5C, 0x9C);
const STATUS_ERROR: Color32 = Color32::from_rgb(0xA3, 0x3A, 0x30);

// --- The fonts ---------------------------------------------------------------

/// The pixel font of the panel, in its regular weight.
const SILKSCREEN_REGULAR: &[u8] = include_bytes!("../assets/fonts/Silkscreen-Regular.ttf");
/// The pixel font of the panel, in its bold weight.
const SILKSCREEN_BOLD: &[u8] = include_bytes!("../assets/fonts/Silkscreen-Bold.ttf");
/// The name of the font family that holds the bold pixel font.
const BOLD_FAMILY: &str = "sushi-bold";
/// A normal monospace family for JSON and code previews. Silkscreen is kept
/// for the faceplate only, because it has no true lowercase.
const CODE_FAMILY: &str = "sushi-code";

/// Installs the pixel fonts of the panel. The regular weight becomes the
/// default of the proportional and of the monospaced families; the bold
/// weight gets its own family for the titles. The default egui fonts stay in
/// the list as a fallback for glyphs that Silkscreen does not have.
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let code_fonts = fonts
        .families
        .get(&egui::FontFamily::Monospace)
        .cloned()
        .unwrap_or_default();
    fonts.font_data.insert(
        "silkscreen".to_owned(),
        Arc::new(egui::FontData::from_static(SILKSCREEN_REGULAR)),
    );
    fonts.font_data.insert(
        "silkscreen-bold".to_owned(),
        Arc::new(egui::FontData::from_static(SILKSCREEN_BOLD)),
    );

    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, "silkscreen".to_owned());
    }

    let mut bold_list = fonts
        .families
        .get(&egui::FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    bold_list.insert(0, "silkscreen-bold".to_owned());
    fonts
        .families
        .insert(egui::FontFamily::Name(Arc::from(BOLD_FAMILY)), bold_list);
    fonts
        .families
        .insert(egui::FontFamily::Name(Arc::from(CODE_FAMILY)), code_fonts);

    ctx.set_fonts(fonts);
}

/// The font family of the bold titles.
fn bold() -> egui::FontFamily {
    egui::FontFamily::Name(Arc::from(BOLD_FAMILY))
}

// --- Layout -----------------------------------------------------------------

pub const WINDOW_WIDTH: f32 = 960.0;
pub const WINDOW_HEIGHT: f32 = 600.0;

/// The margin around the whole panel.
const MARGIN: f32 = 8.0;
/// The height of the header.
const HEADER_HEIGHT: f32 = 48.0;
/// The gap between two sections.
const GROUP_GAP: f32 = 12.0;
/// The gap between two bands. It holds the legend of the band.
const BAND_GAP: f32 = 8.0;
/// The footer strip under the last band.
const FOOTER_HEIGHT: f32 = 20.0;
/// The padding inside a section.
const GROUP_PADDING: f32 = 4.0;
/// The height of the line that holds the title of a section and the controls
/// of its header. The rule below the line sits on its bottom edge.
const TITLE_HEIGHT: f32 = 20.0;
/// The horizontal gap between two parameter columns.
const COLUMN_GAP: f32 = 4.0;

/// The heights of the four rows of sections. They are the proportions of the
/// rows and are scaled with the zoom of the panel.
const ROW_HEIGHTS: [f32; 4] = [123.0, 123.0, 123.0, 123.0];

/// The widths of the sections of the first row: `VCO 1 | VCO 2 | SOURCE MIXER |
/// VCF`. They are the proportions of the sections and are scaled with the
/// zoom of the panel.
const ROW_A_WIDTHS: [f32; 4] = [240.0, 192.0, 124.0, 352.0];
/// The widths of the sections of the second row: `PULSE WIDTH | MOD ENV | AMP
/// ENV | LFO / CLK`.
const ROW_B_WIDTHS: [f32; 3] = [348.0, 284.0, 288.0];
/// The widths of the sections of the third row: `ARP | VOICE`.
const ROW_C_WIDTHS: [f32; 4] = [210.0, 210.0, 300.0, 188.0];
/// The widths of the sections of the fourth row: `FX | MACROS | VCA`.
const ROW_D_WIDTHS: [f32; 3] = [552.0, 216.0, 152.0];

/// The widths of the sections of every row.
const BASE_ROW_WIDTHS: [&[f32]; 4] = [&ROW_A_WIDTHS, &ROW_B_WIDTHS, &ROW_C_WIDTHS, &ROW_D_WIDTHS];

// --- Widget metrics ---------------------------------------------------------

/// The width of a vertical slider.
const SLIDER_WIDTH: f32 = 32.0;
/// The width of the sliders of the macros.
const MACRO_SLIDER_WIDTH: f32 = SLIDER_WIDTH;
/// The height of the slider handle.
const HANDLE_HEIGHT: f32 = 6.0;
/// The empty margin around a handle inside its lane.
const HANDLE_INSET: f32 = 2.0;
/// How far the track and the handle are kept away from the edge of the slider.
const GROOVE_INSET: f32 = 3.0;
/// The smallest height of the buttons and of the text fields of the panel.
const INTERACT_HEIGHT: f32 = 16.0;
/// The height of the captions above the controls.
const LABEL_HEIGHT: f32 = 12.0;
/// The width of the popup that edits the value of a parameter.
const VALUE_POPUP_WIDTH: f32 = 180.0;
/// The diameter of the small lights.
const LED_SIZE: f32 = 6.0;
/// The arpeggiator has a single, slightly bigger light, because its color is
/// the whole readout.
const ARP_LED_SIZE: f32 = 10.0;
/// The number of the lights of the voice meter.
const VOICE_LEDS: usize = 16;
/// The width of a light of the voice meter.
const VOICE_LED_WIDTH: f32 = 5.0;
/// The height of a light of the voice meter.
const VOICE_LED_HEIGHT: f32 = 8.0;
/// The gap between two lights of the voice meter.
const VOICE_LED_GAP: f32 = 2.0;
/// The gap between the caption of the voice meter and its lights.
const VOICE_METER_GAP: f32 = 8.0;
/// The width of the button of the patch browser in the header.
const PATCHES_BUTTON_WIDTH: f32 = 62.0;
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
/// The caption that is shown when the search does not find anything.
const NO_MATCHES: &str = "No matches";

/// How many scope samples are kept around.
const HISTORY_LEN: usize = 4096;

// --- The layout of the panel ------------------------------------------------

/// The layout of the panel for the size of the window.
///
/// The panel has a fixed design size of 960 x 640, like the skin of Surge
/// XT. It is scaled by a single zoom factor, so that it always keeps its
/// proportions, and it is centered in the window. Every measure of the
/// design is multiplied by the zoom, and so is every font.
#[derive(Clone, Copy, Debug)]
struct Layout {
    /// The rect of the header, in the design size of the panel.
    base_header: Rect,
    /// The rects of the four rows of sections, in the design size.
    base_rows: [Rect; 4],
    /// The rect of the whole panel in the window.
    panel: Rect,
    /// How much bigger the panel is than its design size.
    zoom: f32,
}

impl Layout {
    /// Computes the layout of the panel for the given rect.
    ///
    /// It returns `None` when the rect is too small for the panel: a smaller
    /// window only shows a sad face.
    fn for_rect(rect: Rect) -> Option<Self> {
        if rect.width() < WINDOW_WIDTH || rect.height() < WINDOW_HEIGHT {
            return None;
        }

        // The panel follows the window border continuously: it is as big as
        // the smaller of the two ratios allows, so dragging the border scales
        // the whole panel (and its fonts) without any zoom levels.
        let zoom = (rect.width() / WINDOW_WIDTH)
            .min(rect.height() / WINDOW_HEIGHT)
            .max(1.0);

        // The panel keeps its proportions and sits in the middle of the
        // window.
        let panel = Rect::from_center_size(
            rect.center(),
            egui::vec2(WINDOW_WIDTH * zoom, WINDOW_HEIGHT * zoom),
        );

        // The header sits above the rows and only grows with the zoom.
        let base_header = Rect::from_min_size(
            egui::pos2(MARGIN, MARGIN),
            egui::vec2(WINDOW_WIDTH - 2.0 * MARGIN, HEADER_HEIGHT),
        );

        // The rows share the rest of the height. There is a gap between the
        // header and the first row and between every two rows.
        let mut base_rows = [Rect::ZERO; 4];
        let mut y = MARGIN + HEADER_HEIGHT + BAND_GAP;
        for (index, height) in ROW_HEIGHTS.iter().enumerate() {
            base_rows[index] = Rect::from_min_size(
                egui::pos2(MARGIN, y),
                egui::vec2(WINDOW_WIDTH - 2.0 * MARGIN, *height),
            );
            y += height + BAND_GAP;
        }

        Some(Self {
            base_header,
            base_rows,
            panel,
            zoom,
        })
    }

    /// The number of sections of a row.
    fn group_count(row: usize) -> usize {
        BASE_ROW_WIDTHS[row].len()
    }

    /// The rect of the header.
    fn header(&self) -> Rect {
        self.scale(self.base_header)
    }

    /// The rect of a section of a row. The sections of a row share its width
    /// and keep their proportions.
    fn group(&self, row: usize, index: usize) -> Rect {
        let widths = BASE_ROW_WIDTHS[row];
        let base: f32 = widths.iter().sum();
        let row = self.base_rows[row];
        let available = row.width() - GROUP_GAP * (widths.len() as f32 - 1.0);

        let mut x = row.left();
        for (current, width) in widths.iter().enumerate() {
            let width = available * width / base;
            if current == index {
                return self.scale(Rect::from_min_size(
                    egui::pos2(x, row.top()),
                    egui::vec2(width, row.height()),
                ));
            }
            x += width + GROUP_GAP;
        }

        Rect::ZERO
    }

    /// Scales a rect of the design size to the size of the panel.
    fn scale(&self, rect: Rect) -> Rect {
        Rect::from_min_size(
            self.panel.min + rect.min.to_vec2() * self.zoom,
            rect.size() * self.zoom,
        )
    }

    /// The width of a slider. It grows with the zoom of the panel.
    fn slider_width(&self) -> f32 {
        SLIDER_WIDTH * self.zoom
    }

    /// The width of the sliders of the macros.
    fn macro_width(&self) -> f32 {
        MACRO_SLIDER_WIDTH * self.zoom
    }
}

/// The size of a font of the panel at the given zoom.
fn font_size(size: f32, zoom: f32) -> f32 {
    size * zoom
}

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
    /// The position of the arpeggiator in its pattern.
    arp_step: usize,
    /// Whether the modulation matrix page is shown in the two upper bands.
    mod_page: bool,
    /// The parameter and value under the pointer, or the last one seen.
    readout: Option<(String, String)>,
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
    /// The places of the popup buttons of the last frame.
    #[cfg(test)]
    popup_rects: Vec<egui::Rect>,
    /// The configuration of the patch browser.
    patch_config: patch::PatchConfig,
    /// Whether the window of the patch browser is open.
    patch_open: bool,
    /// The folder in which the patches are stored, as it is shown in the
    /// patch browser.
    patch_dir_text: String,
    /// The name of the patch that is saved by the patch browser.
    patch_name: String,
    /// The text that the patch browser searches for.
    patch_search: String,
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
    /// The raw JSON shown in the right pane of the browser.
    patch_preview: String,
    /// The name that belongs to the preview.
    patch_preview_title: String,
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
    /// The zoom of the panel, as it was last applied to the styles.
    zoom: f32,
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
            zoom: 1.0,
            state: EditorState {
                texts: HashMap::new(),
                gestures: HashSet::new(),
                preset_index: 0,
                dirty: false,
                rng: Rng::new(0x2545_F491),
                arp_counter: 0,
                arp_step: 0,
                mod_page: false,
                readout: None,
                activity: 0.0,
                clipping: 0.0,
                #[cfg(test)]
                counts: Counts::default(),
                #[cfg(test)]
                check_rects: Vec::new(),
                #[cfg(test)]
                radio_rects: Vec::new(),
                #[cfg(test)]
                popup_rects: Vec::new(),
                patch_config: patch::PatchConfig::default(),
                patch_open: false,
                patch_dir_text: String::new(),
                patch_name: String::new(),
                patch_search: String::new(),
                patch_files: Vec::new(),
                patch_status: String::new(),
                patch_status_error: false,
                patch_confirm_overwrite: false,
                patch_active: None,
                patch_preview: String::new(),
                patch_preview_title: String::new(),
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
        self.state.patch_preview_title = format!(
            "{:02} {}",
            self.state.preset_index + 1,
            PRESETS[self.state.preset_index].name
        );
        self.state.patch_preview = serde_json::to_string_pretty(&patch::to_json(
            &self.params,
            &self.state.patch_preview_title,
        ))
        .unwrap_or_default();
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

    /// Reads the step of the arpeggiator when it plays a note. The light uses
    /// the position as its color, so it does not need a timer: every step
    /// replaces the color, which stays visible at any tempo.
    fn update_arp_position(&mut self) {
        let counter = self.arp_led.load(Ordering::Relaxed);
        if counter != self.state.arp_counter {
            self.state.arp_counter = counter;
            self.state.arp_step = self.arp_position.load(Ordering::Relaxed);
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
        self.init_patch_state();

        install_fonts(&egui_ctx);
        apply_theme(&egui_ctx, 1.0);
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
        self.update_arp_position();
        ui.ctx().request_repaint_after(Duration::from_millis(16));

        let full = ui.max_rect();
        ui.painter().rect_filled(full, CornerRadius::ZERO, BG);

        // The panel needs at least the size it was made for. A smaller window
        // only shows a sad face, so that nothing is cut in half.
        let Some(layout) = Layout::for_rect(full) else {
            draw_too_small(ui, full);
            return;
        };

        // The styles of the widgets follow the zoom of the panel, so that the
        // texts of the fields and of the windows grow with it.
        if layout.zoom != self.zoom {
            apply_theme(ui.ctx(), layout.zoom);
            // The root ui was made before the theme was changed, so it still
            // has the old one.
            ui.reset_style();
            self.zoom = layout.zoom;
        }

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
            slider_width: layout.slider_width(),
            zoom: layout.zoom,
        };

        if ctx.state.patch_open {
            ctx.patch_page(ui);
            return;
        }

        let header_rect = layout.header();
        ui.scope_builder(
            egui::UiBuilder::new()
                .id_salt("sushi-header")
                .max_rect(header_rect)
                .layout(EguiLayout::top_down(Align::Min)),
            |ui| header(ui, header_rect, &mut ctx),
        );

        let mod_page = ctx.state.mod_page;
        if mod_page {
            row_mod(ui, &layout, 0, 0, &mut ctx);
            row_mod(ui, &layout, 1, 4, &mut ctx);
        } else {
            row_a(ui, &layout, &mut ctx);
            row_b(ui, &layout, &mut ctx);
        }
        row_c(ui, &layout, &mut ctx);
        row_d(ui, &layout, &mut ctx);
        draw_footer(ui, &layout, ctx.state);
    }
}

/// The footer silkscreen: the model name and the two shortcuts that are not
/// visible on the faceplate.
fn draw_footer(ui: &egui::Ui, layout: &Layout, state: &EditorState) {
    let painter = ui.painter_at(layout.panel);
    let zoom = layout.zoom;
    let panel = layout.panel;
    let y = panel.bottom() - (FOOTER_HEIGHT - 4.0) * zoom;
    painter.line_segment(
        [
            egui::pos2(panel.left(), panel.bottom() - FOOTER_HEIGHT * zoom),
            egui::pos2(panel.right(), panel.bottom() - FOOTER_HEIGHT * zoom),
        ],
        Stroke::new(1.0, RULE),
    );

    painter.text(
        egui::pos2(panel.left() + 12.0 * zoom, y),
        Align2::LEFT_TOP,
        "SUSHI / MODEL 101",
        FontId::new(font_size(8.0, zoom), bold()),
        MUTED,
    );
    painter.text(
        egui::pos2(panel.right() - 12.0 * zoom, y),
        Align2::RIGHT_TOP,
        "DBL-CLICK = RESET   RIGHT-CLICK = VALUE",
        FontId::new(font_size(8.0, zoom), bold()),
        MUTED,
    );

    // The live parameter readout: the reference's central value box.
    let (label, value) = state
        .readout
        .clone()
        .unwrap_or_else(|| ("PATCH".to_string(), display_text(state)));
    let box_rect = Rect::from_center_size(
        egui::pos2(panel.center().x, panel.bottom() - 10.0 * zoom),
        egui::vec2(300.0, 15.0) * zoom,
    );
    flat_box(&painter, box_rect, PAPER);
    let label = truncate_name(&label, 22);
    painter.text(
        egui::pos2(box_rect.left() + 7.0 * zoom, box_rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::new(font_size(8.0, zoom), bold()),
        TITLE_BLUE,
    );
    painter.text(
        egui::pos2(box_rect.right() - 7.0 * zoom, box_rect.center().y),
        Align2::RIGHT_CENTER,
        value,
        FontId::new(font_size(10.0, zoom), bold()),
        TEXT,
    );
}

/// Draws the sad face that tells the user that the window is too small for
/// the panel. It works at every size, even at a tiny one.
///
/// There is no layout and therefore no zoom here: the face grows with the
/// window and the hint keeps the size of the design.
fn draw_too_small(ui: &mut egui::Ui, rect: Rect) {
    let painter = ui.painter_at(rect);

    // The face grows with the window, but never gets bigger than 64 px.
    let size = (rect.height().min(rect.width()) * 0.4).clamp(12.0, 64.0);
    let face = painter.layout_no_wrap(":(".to_string(), FontId::proportional(size), TEXT);

    // The line below the face tells the user how big the window has to be.
    // It gets a bit smaller when the window is too narrow for it.
    let hint = format!("MINIMUM {} x {}", WINDOW_WIDTH as i32, WINDOW_HEIGHT as i32);
    let width = (rect.width() - 8.0).max(8.0);
    let galley = painter.layout_no_wrap(hint.to_string(), FontId::proportional(10.0), MUTED);
    let hint = if galley.size().x > width {
        let size = (10.0 * width / galley.size().x).max(4.0);
        painter.layout_no_wrap(hint.to_string(), FontId::proportional(size), MUTED)
    } else {
        galley
    };

    // The face and the line are centered as a block.
    let gap = 12.0;
    let face_size = face.size();
    let hint_size = hint.size();
    let block = face_size.y + gap + hint_size.y;
    let top = rect.center().y - block * 0.5;

    painter.galley(
        egui::pos2(rect.center().x - face_size.x * 0.5, top),
        face,
        TEXT,
    );
    painter.galley(
        egui::pos2(rect.center().x - hint_size.x * 0.5, top + face_size.y + gap),
        hint,
        MUTED,
    );
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
    /// The button that switches the two upper bands to the modulation matrix.
    mod_page: Rect,
    /// The meter that shows the number of the voices that are playing.
    meter: Rect,
}

/// The places of the widgets of the header.
fn header_rects(rect: Rect, zoom: f32) -> HeaderRects {
    // The presets.
    let preset_x = rect.left() + 172.0 * zoom;
    let button_y = rect.top() + (HEADER_HEIGHT - 28.0) * 0.5 * zoom;
    let previous = Rect::from_min_size(
        egui::pos2(preset_x, button_y),
        egui::vec2(22.0, 28.0) * zoom,
    );
    let display = Rect::from_min_size(
        egui::pos2(preset_x + 26.0 * zoom, button_y),
        egui::vec2(160.0, 28.0) * zoom,
    );
    let next = Rect::from_min_size(
        egui::pos2(preset_x + 190.0 * zoom, button_y),
        egui::vec2(22.0, 28.0) * zoom,
    );
    let mutate = Rect::from_min_size(
        egui::pos2(preset_x + 216.0 * zoom, button_y),
        egui::vec2(62.0, 28.0) * zoom,
    );
    let patches = Rect::from_min_size(
        egui::pos2(mutate.right() + 4.0 * zoom, button_y),
        egui::vec2(PATCHES_BUTTON_WIDTH, 28.0) * zoom,
    );

    // The page switch sits between the patch commands and the meter.
    let mod_page = Rect::from_min_size(
        egui::pos2(patches.right() + 4.0 * zoom, button_y),
        egui::vec2(44.0, 28.0) * zoom,
    );

    // The meter that shows how many voices are playing. It sits in the free
    // space between the page switch and the lights of the activity meter.
    let meter = Rect::from_min_max(
        egui::pos2(mod_page.right() + 8.0 * zoom, rect.top()),
        egui::pos2(rect.right() - 220.0 * zoom, rect.bottom()),
    );

    HeaderRects {
        previous,
        display,
        next,
        mutate,
        patches,
        mod_page,
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
    let zoom = ctx.zoom;
    let painter = ui.painter_at(rect);

    // The header sits on its own deeper plate, separated by a full-width rule.
    painter.rect_filled(rect, CornerRadius::ZERO, HEADER_BG);
    let rule_y = rect.bottom() - 0.5;
    painter.line_segment(
        [
            egui::pos2(rect.left(), rule_y),
            egui::pos2(rect.right(), rule_y),
        ],
        Stroke::new(1.0, RULE),
    );

    // The wordmark. It is drawn with the bold pixel font, so it does not
    // have to be drawn twice.
    let wordmark = FontId::new(font_size(20.0, zoom), bold());
    painter.text(
        rect.left_top() + egui::vec2(8.0, 2.0) * zoom,
        Align2::LEFT_TOP,
        "SUSHI",
        wordmark,
        TEXT,
    );
    painter.text(
        rect.left_top() + egui::vec2(8.0, 27.0) * zoom,
        Align2::LEFT_TOP,
        "POLYPHONIC SYNTHESIZER",
        FontId::proportional(font_size(8.0, zoom)),
        MUTED,
    );
    // The presets.
    let rects = header_rects(rect, zoom);
    let index = ctx.state.preset_index;
    lcd(&painter, rects.display, &display_text(ctx.state), zoom);

    if button(ui, rects.previous, "<", zoom).clicked() {
        ctx.load_preset(index + PRESETS.len() - 1);
    }
    if button(ui, rects.next, ">", zoom).clicked() {
        ctx.load_preset(index + 1);
    }
    if button(ui, rects.mutate, "MUTATE", zoom).clicked() {
        ctx.mutate();
    }
    if button(ui, rects.patches, "PATCHES", zoom).clicked() {
        ctx.toggle_patch_browser();
    }

    // The page switch: MOD replaces the two upper bands with the routing
    // matrix, while the performance and output bands stay exactly where they
    // are.
    if button(ui, rects.mod_page, "MOD", zoom).clicked() {
        ctx.state.mod_page = !ctx.state.mod_page;
    }
    if ctx.state.mod_page {
        let underline = Rect::from_min_max(
            egui::pos2(rects.mod_page.left(), rects.mod_page.bottom() - 2.0 * zoom),
            rects.mod_page.right_bottom(),
        );
        painter.rect_filled(underline, CornerRadius::ZERO, ACCENT);
    }

    // The meter that shows how many voices are playing.
    voice_meter(
        &painter,
        rects.meter,
        ctx.voice_count.load(Ordering::Relaxed).min(VOICE_LEDS),
        zoom,
    );

    // The oscilloscope and the two lights.
    let scope = Rect::from_min_size(
        egui::pos2(rect.right() - 154.0 * zoom, rect.top() + 5.0 * zoom),
        egui::vec2(150.0, 38.0) * zoom,
    );
    small_scope(&painter, scope, ctx.history, zoom);

    let activity = egui::pos2(rect.right() - 202.0 * zoom, rect.top() + 16.0 * zoom);
    led(&painter, activity, LED_ON, ctx.state.activity > 0.002, zoom);
    painter.text(
        egui::pos2(activity.x, rect.top() + 28.0 * zoom),
        Align2::CENTER_CENTER,
        "ACT",
        FontId::proportional(font_size(9.0, zoom)),
        MUTED,
    );

    let clipping = egui::pos2(rect.right() - 172.0 * zoom, rect.top() + 16.0 * zoom);
    led(&painter, clipping, LED_RED, ctx.state.clipping > 0.5, zoom);
    painter.text(
        egui::pos2(clipping.x, rect.top() + 28.0 * zoom),
        Align2::CENTER_CENTER,
        "CLIP",
        FontId::proportional(font_size(9.0, zoom)),
        MUTED,
    );
}

// --- The rows of sections ------------------------------------------------

/// The upper two bands of the modulation page: eight routing slots, four per
/// band. Each slot keeps the same grammar as a section: a number, a source
/// field, a destination field and one bipolar amount fader.
fn row_mod(ui: &mut egui::Ui, layout: &Layout, row: usize, first: usize, ctx: &mut Ctx) {
    let zoom = ctx.zoom;
    let row_rect = layout.scale(layout.base_rows[row]);
    let gap = GROUP_GAP * zoom;
    let width = (row_rect.width() - 3.0 * gap) / 4.0;

    let sources = [
        &ctx.params.mod1_source,
        &ctx.params.mod2_source,
        &ctx.params.mod3_source,
        &ctx.params.mod4_source,
        &ctx.params.mod5_source,
        &ctx.params.mod6_source,
        &ctx.params.mod7_source,
        &ctx.params.mod8_source,
    ];
    let dests = [
        &ctx.params.mod1_dest,
        &ctx.params.mod2_dest,
        &ctx.params.mod3_dest,
        &ctx.params.mod4_dest,
        &ctx.params.mod5_dest,
        &ctx.params.mod6_dest,
        &ctx.params.mod7_dest,
        &ctx.params.mod8_dest,
    ];
    let amounts = [
        &ctx.params.mod1_amount,
        &ctx.params.mod2_amount,
        &ctx.params.mod3_amount,
        &ctx.params.mod4_amount,
        &ctx.params.mod5_amount,
        &ctx.params.mod6_amount,
        &ctx.params.mod7_amount,
        &ctx.params.mod8_amount,
    ];
    let source_ids = [
        "mod1_source",
        "mod2_source",
        "mod3_source",
        "mod4_source",
        "mod5_source",
        "mod6_source",
        "mod7_source",
        "mod8_source",
    ];
    let dest_ids = [
        "mod1_dest",
        "mod2_dest",
        "mod3_dest",
        "mod4_dest",
        "mod5_dest",
        "mod6_dest",
        "mod7_dest",
        "mod8_dest",
    ];
    let amount_ids = [
        "mod1_amount",
        "mod2_amount",
        "mod3_amount",
        "mod4_amount",
        "mod5_amount",
        "mod6_amount",
        "mod7_amount",
        "mod8_amount",
    ];

    for index in 0..4 {
        let slot = first + index;
        let rect = Rect::from_min_size(
            egui::pos2(
                row_rect.left() + index as f32 * (width + gap),
                row_rect.top(),
            ),
            egui::vec2(width, row_rect.height()),
        );
        let title = format!("{:02}", slot + 1);

        group(ui, rect, &title, zoom, |ui, part| match part {
            Part::Header => {
                ctx.popup(ui, source_ids[slot], sources[slot], 70.0);
                ctx.popup(ui, dest_ids[slot], dests[slot], 70.0);
            }
            Part::Body => {
                column_row_fixed(ui, amount_ids[slot], 1, 0.0, 60.0, zoom, |_, ui| {
                    ctx.float(ui, amount_ids[slot], "Amt", amounts[slot], &Knob::bipolar());
                });
            }
        });
    }
}

/// The first row: the two oscillators, the pulse widths and the source mixer.
///
/// This is the dense generator band. The pulse widths sit next to the
/// oscillators they shape, and the filter opens the second band.
fn row_a(ui: &mut egui::Ui, layout: &Layout, ctx: &mut Ctx) {
    let zoom = ctx.zoom;
    group_row(ui, "row-a", layout, 0, |index, rect, ui| {
        let p = ctx.params;
        match index {
            0 => group(ui, rect, "VCO 1", zoom, |ui, part| match part {
                Part::Header => ctx.popup(ui, "osc1_wave", &p.osc1_wave, 66.0),
                Part::Body => {
                    column_row_fixed(ui, "osc1", 5, COLUMN_GAP, 43.0, zoom, |index, ui| {
                        let p = ctx.params;
                        match index {
                            0 => {
                                ctx.float(ui, "tune", "Tune", &p.tune, &Knob::plain(-12.0, 12.0, 2))
                            }
                            1 => ctx.int(
                                ui,
                                "osc1_octave",
                                "Oct",
                                &p.osc1_octave,
                                &Knob::plain(-2.0, 2.0, 0),
                            ),
                            2 => ctx.int(
                                ui,
                                "osc1_semi",
                                "Semi",
                                &p.osc1_semi,
                                &Knob::plain(-12.0, 12.0, 0),
                            ),
                            3 => ctx.float(
                                ui,
                                "osc1_fine",
                                "Fine",
                                &p.osc1_fine,
                                &Knob::plain(-100.0, 100.0, 0),
                            ),
                            4 => ctx.float(ui, "sub_level", "Sub", &p.sub_level, &Knob::percent()),
                            _ => {}
                        }
                    })
                }
            }),
            1 => group(ui, rect, "VCO 2", zoom, |ui, part| match part {
                Part::Header => {
                    ctx.popup(ui, "osc2_wave", &p.osc2_wave, 66.0);
                    ctx.check(ui, "osc_sync", &p.osc_sync, "SYNC");
                }
                Part::Body => {
                    column_row_fixed(ui, "osc2", 4, COLUMN_GAP, 43.0, zoom, |index, ui| {
                        let p = ctx.params;
                        match index {
                            0 => ctx.int(
                                ui,
                                "osc2_octave",
                                "Oct",
                                &p.osc2_octave,
                                &Knob::plain(-2.0, 2.0, 0),
                            ),
                            1 => ctx.int(
                                ui,
                                "osc2_semi",
                                "Semi",
                                &p.osc2_semi,
                                &Knob::plain(-12.0, 12.0, 0),
                            ),
                            2 => ctx.float(
                                ui,
                                "osc2_fine",
                                "Fine",
                                &p.osc2_fine,
                                &Knob::plain(-100.0, 100.0, 0),
                            ),
                            3 => ctx.float(ui, "fm_amount", "FM", &p.fm_amount, &Knob::percent()),
                            _ => {}
                        }
                    })
                }
            }),
            2 => group(ui, rect, "PULSE WIDTH", zoom, |ui, part| match part {
                Part::Header => {}
                Part::Body => column_row_fixed(ui, "pw", 2, COLUMN_GAP, 52.0, zoom, |index, ui| {
                    let p = ctx.params;
                    match index {
                        0 => ctx.float(
                            ui,
                            "osc1_pw",
                            "PW 1",
                            &p.osc1_pw,
                            &Knob::scaled(0.05, 0.95, 0),
                        ),
                        1 => ctx.float(
                            ui,
                            "osc2_pw",
                            "PW 2",
                            &p.osc2_pw,
                            &Knob::scaled(0.05, 0.95, 0),
                        ),
                        _ => {}
                    }
                }),
            }),
            _ => group(ui, rect, "SOURCE MIXER", zoom, |ui, part| match part {
                Part::Header => {}
                Part::Body => {
                    column_row_fixed(ui, "mix", 6, COLUMN_GAP, 52.0, zoom, |index, ui| {
                        let p = ctx.params;
                        match index {
                            0 => ctx.float(
                                ui,
                                "osc1_level",
                                "Lvl 1",
                                &p.osc1_level,
                                &Knob::percent(),
                            ),
                            1 => ctx.float(
                                ui,
                                "osc2_level",
                                "Lvl 2",
                                &p.osc2_level,
                                &Knob::percent(),
                            ),
                            2 => ctx.float(
                                ui,
                                "noise_level",
                                "Noise",
                                &p.noise_level,
                                &Knob::percent(),
                            ),
                            3 => ctx.int(ui, "unison", "Uni", &p.unison, &Knob::plain(1.0, 4.0, 0)),
                            4 => ctx.float(
                                ui,
                                "detune",
                                "Detune",
                                &p.detune,
                                &Knob::plain(0.0, 50.0, 0),
                            ),
                            5 => ctx.float(ui, "spread", "Spread", &p.spread, &Knob::percent()),
                            _ => {}
                        }
                    })
                }
            }),
        }
    });
}

/// The second row: the filter and the two envelopes.
///
/// The filter is the first section of the shaping band, so that the signal
/// still reads left to right: oscillators above, filter and envelopes here.
fn row_b(ui: &mut egui::Ui, layout: &Layout, ctx: &mut Ctx) {
    let zoom = ctx.zoom;
    group_row(ui, "row-b", layout, 1, |index, rect, ui| {
        let p = ctx.params;
        match index {
            0 => group(ui, rect, "VCF", zoom, |ui, part| match part {
                Part::Header => {
                    ctx.popup(ui, "filter_mode", &p.filter_mode, 70.0);
                    ctx.popup(ui, "filter_slope", &p.filter_slope, 52.0);
                }
                Part::Body => {
                    column_row_fixed(ui, "filter", 5, COLUMN_GAP, 60.0, zoom, |index, ui| {
                        let p = ctx.params;
                        match index {
                            0 => ctx.float(
                                ui,
                                "cutoff",
                                "Cut",
                                &p.cutoff,
                                &Knob::logarithmic(20.0, 20_000.0, 0),
                            ),
                            1 => ctx.float(ui, "resonance", "Res", &p.resonance, &Knob::percent()),
                            2 => ctx.float(
                                ui,
                                "filter_env_amount",
                                "Env",
                                &p.filter_env_amount,
                                &Knob::bipolar(),
                            ),
                            3 => ctx.float(
                                ui,
                                "filter_keytrack",
                                "Key",
                                &p.filter_keytrack,
                                &Knob::percent(),
                            ),
                            4 => ctx.float(
                                ui,
                                "filter_drive",
                                "Drv",
                                &p.filter_drive,
                                &Knob::percent(),
                            ),
                            _ => {}
                        }
                    })
                }
            }),
            1 => group(ui, rect, "MOD ENV", zoom, |ui, part| match part {
                Part::Header => {}
                Part::Body => column_row_fixed(ui, "fe", 4, COLUMN_GAP, 60.0, zoom, |index, ui| {
                    let p = ctx.params;
                    match index {
                        0 => ctx.float(
                            ui,
                            "fe_attack",
                            "A",
                            &p.fe_attack,
                            &Knob::logarithmic(0.0, 4_000.0, 1),
                        ),
                        1 => ctx.float(
                            ui,
                            "fe_decay",
                            "D",
                            &p.fe_decay,
                            &Knob::logarithmic(0.0, 4_000.0, 1),
                        ),
                        2 => ctx.float(ui, "fe_sustain", "S", &p.fe_sustain, &Knob::percent()),
                        3 => ctx.float(
                            ui,
                            "fe_release",
                            "R",
                            &p.fe_release,
                            &Knob::logarithmic(0.0, 4_000.0, 1),
                        ),
                        _ => {}
                    }
                }),
            }),
            _ => group(ui, rect, "AMP ENV", zoom, |ui, part| match part {
                Part::Header => {}
                Part::Body => column_row_fixed(ui, "ae", 4, COLUMN_GAP, 60.0, zoom, |index, ui| {
                    let p = ctx.params;
                    match index {
                        0 => ctx.float(
                            ui,
                            "ae_attack",
                            "A",
                            &p.ae_attack,
                            &Knob::logarithmic(0.0, 4_000.0, 1),
                        ),
                        1 => ctx.float(
                            ui,
                            "ae_decay",
                            "D",
                            &p.ae_decay,
                            &Knob::logarithmic(0.0, 4_000.0, 1),
                        ),
                        2 => ctx.float(ui, "ae_sustain", "S", &p.ae_sustain, &Knob::percent()),
                        3 => ctx.float(
                            ui,
                            "ae_release",
                            "R",
                            &p.ae_release,
                            &Knob::logarithmic(0.0, 8_000.0, 1),
                        ),
                        _ => {}
                    }
                }),
            }),
        }
    });
}

/// The third row: the two LFOs, the arpeggiator and the voice.
///
/// Splitting the LFOs into two small sections gives each one its own waveform
/// and destination field, so neither is hidden behind an "OFF" box.
fn row_c(ui: &mut egui::Ui, layout: &Layout, ctx: &mut Ctx) {
    let zoom = ctx.zoom;
    group_row(ui, "row-c", layout, 2, |index, rect, ui| {
        let p = ctx.params;
        match index {
            0 | 1 => {
                let (title, wave, target, rate, amount, ids) = if index == 0 {
                    (
                        "LFO 1",
                        &p.lfo1_wave,
                        &p.lfo1_target,
                        &p.lfo1_rate,
                        &p.lfo1_amount,
                        ("lfo1_wave", "lfo1_target", "lfo1_rate", "lfo1_amount"),
                    )
                } else {
                    (
                        "LFO 2",
                        &p.lfo2_wave,
                        &p.lfo2_target,
                        &p.lfo2_rate,
                        &p.lfo2_amount,
                        ("lfo2_wave", "lfo2_target", "lfo2_rate", "lfo2_amount"),
                    )
                };
                group(ui, rect, title, zoom, |ui, part| match part {
                    Part::Header => {
                        ctx.popup(ui, ids.0, wave, 54.0);
                        ctx.popup(ui, ids.1, target, 54.0);
                    }
                    Part::Body => {
                        column_row_fixed(ui, ids.2, 2, COLUMN_GAP, 60.0, zoom, |index, ui| {
                            match index {
                                0 => ctx.float(
                                    ui,
                                    ids.2,
                                    "Rate",
                                    rate,
                                    &Knob::logarithmic(0.01, 40.0, 2),
                                ),
                                1 => ctx.float(ui, ids.3, "Amount", amount, &Knob::percent()),
                                _ => {}
                            }
                        });
                    }
                });
            }
            2 => group(ui, rect, "ARP", zoom, |ui, part| match part {
                Part::Header => {
                    ctx.check(ui, "arp_on", &p.arp_on, "ON");
                    ctx.check(ui, "arp_latch", &p.arp_latch, "LATCH");
                    ctx.popup(ui, "arp_pattern", &p.arp_pattern, 74.0);
                    ctx.popup(ui, "arp_rate", &p.arp_rate, 52.0);
                    let (rect, _) = ui.allocate_exact_size(
                        egui::vec2(ARP_LED_SIZE, ARP_LED_SIZE) * zoom,
                        Sense::hover(),
                    );
                    led(
                        &ui.painter_at(rect),
                        rect.center(),
                        arp_light_color(ctx.state.arp_step),
                        p.arp_on.value(),
                        zoom,
                    );
                }
                Part::Body => {
                    column_row_fixed(ui, "arp", 5, COLUMN_GAP, 56.0, zoom, |index, ui| {
                        let p = ctx.params;
                        match index {
                            0 => ctx.float(
                                ui,
                                "arp_swing",
                                "Swing",
                                &p.arp_swing,
                                &Knob::plain(0.0, 75.0, 0),
                            ),
                            1 => ctx.int(
                                ui,
                                "arp_octaves",
                                "Oct",
                                &p.arp_octaves,
                                &Knob::plain(1.0, 3.0, 0),
                            ),
                            2 => ctx.float(
                                ui,
                                "arp_gate",
                                "Gate",
                                &p.arp_gate,
                                &Knob::plain(5.0, 100.0, 0),
                            ),
                            3 => ctx.int(
                                ui,
                                "arp_ratchet",
                                "Ratchet",
                                &p.arp_ratchet,
                                &Knob::plain(1.0, 4.0, 0),
                            ),
                            4 => ctx.int(
                                ui,
                                "arp_length",
                                "Length",
                                &p.arp_length,
                                &Knob::plain(1.0, 16.0, 0),
                            ),
                            _ => {}
                        }
                    });
                }
            }),
            _ => group(ui, rect, "VOICE", zoom, |ui, part| match part {
                Part::Header => ctx.popup(ui, "poly_mode", &p.poly_mode, 66.0),
                Part::Body => {
                    column_row_fixed(ui, "voice", 4, COLUMN_GAP, 44.0, zoom, |index, ui| {
                        let p = ctx.params;
                        match index {
                            0 => ctx.float(
                                ui,
                                "glide",
                                "Glide",
                                &p.glide,
                                &Knob::logarithmic(0.0, 2_000.0, 1),
                            ),
                            1 => ctx.float(ui, "drift", "Drift", &p.drift, &Knob::percent()),
                            2 => ctx.float(ui, "velocity", "Vel", &p.velocity, &Knob::percent()),
                            3 => ctx.int(
                                ui,
                                "voices",
                                "Voices",
                                &p.voices,
                                &Knob::plain(1.0, 16.0, 0),
                            ),
                            _ => {}
                        }
                    })
                }
            }),
        }
    });
}

/// The fourth row: the effects, the macros and the output.
fn row_d(ui: &mut egui::Ui, layout: &Layout, ctx: &mut Ctx) {
    let zoom = ctx.zoom;
    group_row(ui, "row-d", layout, 3, |index, rect, ui| match index {
        0 => {
            group(ui, rect, "FX", zoom, |ui, part| match part {
                Part::Header => {}
                Part::Body => sections(
                    ui,
                    "fx",
                    3,
                    GROUP_GAP,
                    zoom,
                    |index, _rect, ui| match index {
                        0 => {
                            column_row_fixed(
                                ui,
                                "chorus",
                                3,
                                COLUMN_GAP,
                                52.0,
                                zoom,
                                |index, ui| {
                                    let p = ctx.params;
                                    match index {
                                        0 => ctx.float(
                                            ui,
                                            "chorus_mix",
                                            "Mix",
                                            &p.chorus_mix,
                                            &Knob::percent(),
                                        ),
                                        1 => ctx.float(
                                            ui,
                                            "chorus_rate",
                                            "Rate",
                                            &p.chorus_rate,
                                            &Knob::logarithmic(0.05, 8.0, 2),
                                        ),
                                        2 => ctx.float(
                                            ui,
                                            "chorus_depth",
                                            "Depth",
                                            &p.chorus_depth,
                                            &Knob::percent(),
                                        ),
                                        _ => {}
                                    }
                                },
                            );
                        }
                        1 => {
                            column_row_fixed(
                                ui,
                                "delay",
                                3,
                                COLUMN_GAP,
                                52.0,
                                zoom,
                                |index, ui| {
                                    let p = ctx.params;
                                    match index {
                                        0 => ctx.float(
                                            ui,
                                            "delay_time",
                                            "Time",
                                            &p.delay_time,
                                            &Knob::logarithmic(10.0, 1_500.0, 1),
                                        ),
                                        1 => ctx.float(
                                            ui,
                                            "delay_feedback",
                                            "FB",
                                            &p.delay_feedback,
                                            &Knob::percent(),
                                        ),
                                        2 => ctx.float(
                                            ui,
                                            "delay_mix",
                                            "Mix",
                                            &p.delay_mix,
                                            &Knob::percent(),
                                        ),
                                        _ => {}
                                    }
                                },
                            );
                        }
                        _ => {
                            column_row_fixed(
                                ui,
                                "reverb",
                                2,
                                COLUMN_GAP,
                                52.0,
                                zoom,
                                |index, ui| {
                                    let p = ctx.params;
                                    match index {
                                        0 => ctx.float(
                                            ui,
                                            "reverb_decay",
                                            "Decay",
                                            &p.reverb_decay,
                                            &Knob::percent(),
                                        ),
                                        1 => ctx.float(
                                            ui,
                                            "reverb_mix",
                                            "Mix",
                                            &p.reverb_mix,
                                            &Knob::percent(),
                                        ),
                                        _ => {}
                                    }
                                },
                            );
                        }
                    },
                ),
            });

            // The labels sit over the centre of their real subgroups, not at
            // arbitrary fractions of the title bar.
            let painter = ui.painter_at(rect);
            let body_left = rect.left() + GROUP_PADDING * zoom;
            let body_width = rect.width() - 2.0 * GROUP_PADDING * zoom;
            let gap = GROUP_GAP * zoom;
            let slot = (body_width - 2.0 * gap) / 3.0;
            for (index, label) in ["CHORUS", "DELAY", "REVERB"].iter().enumerate() {
                let x = body_left + slot * 0.5 + index as f32 * (slot + gap);
                painter.text(
                    egui::pos2(x, rect.top() + TITLE_HEIGHT * 0.5 * zoom),
                    Align2::CENTER_CENTER,
                    *label,
                    FontId::new(font_size(8.0, zoom), bold()),
                    MUTED,
                );
            }
        }
        1 => group(ui, rect, "MACROS", zoom, |ui, part| match part {
            Part::Header => {}
            Part::Body => column_row_fixed(ui, "macros", 3, COLUMN_GAP, 60.0, zoom, |index, ui| {
                let p = ctx.params;
                let knob = Knob::percent().with_width(layout.macro_width());
                match index {
                    0 => ctx.float(ui, "macro_bright", "Brilliance", &p.macro_bright, &knob),
                    1 => ctx.float(ui, "macro_motion", "Motion", &p.macro_motion, &knob),
                    2 => ctx.float(ui, "macro_space", "Space", &p.macro_space, &knob),
                    _ => {}
                }
            }),
        }),
        _ => group(ui, rect, "VCA", zoom, |ui, part| match part {
            Part::Header => {
                let p = ctx.params;
                text_label(ui, "CHAR", 8.0, zoom, MUTED);
                ctx.radio_pair(ui, "character", &p.character, 58.0);
            }
            Part::Body => column_row_fixed(ui, "output", 2, COLUMN_GAP, 60.0, zoom, |index, ui| {
                let p = ctx.params;
                match index {
                    0 => ctx.float(
                        ui,
                        "volume",
                        "Volume",
                        &p.volume,
                        &Knob::decibels(-60.0, 6.0),
                    ),
                    1 => ctx.float(ui, "tone", "Tone", &p.tone, &Knob::bipolar()),
                    _ => {}
                }
            }),
        }),
    });
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
    /// The width of the slider.
    width: f32,
}

impl Knob {
    /// A parameter that is shown as it is.
    fn plain(min: f32, max: f32, decimals: usize) -> Self {
        Self {
            min,
            max,
            log: false,
            decimals,
            to_display: identity,
            from_display: identity,
            width: SLIDER_WIDTH,
        }
    }

    /// A parameter that is shown as a percentage.
    fn percent() -> Self {
        Self {
            min: 0.0,
            max: 1.0,
            log: false,
            decimals: 0,
            to_display: to_percent,
            from_display: from_percent,
            width: SLIDER_WIDTH,
        }
    }

    /// A parameter that is shown as a signed percentage.
    fn bipolar() -> Self {
        Self {
            min: -1.0,
            max: 1.0,
            log: false,
            decimals: 0,
            to_display: to_percent,
            from_display: from_percent,
            width: SLIDER_WIDTH,
        }
    }

    /// A parameter that is stored in another range than it is shown in.
    fn scaled(min: f32, max: f32, decimals: usize) -> Self {
        Self {
            min,
            max,
            log: false,
            decimals,
            to_display: to_percent,
            from_display: from_percent,
            width: SLIDER_WIDTH,
        }
    }

    /// A parameter with a logarithmic range.
    fn logarithmic(min: f32, max: f32, decimals: usize) -> Self {
        Self {
            min,
            max,
            log: true,
            decimals,
            to_display: identity,
            from_display: identity,
            width: SLIDER_WIDTH,
        }
    }

    /// A parameter that is shown in decibels but stored as a gain.
    fn decibels(min: f32, max: f32) -> Self {
        Self {
            min: util::db_to_gain(min),
            max: util::db_to_gain(max),
            log: true,
            decimals: 1,
            to_display: util::gain_to_db,
            from_display: util::db_to_gain,
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
    /// The width of the sliders of the panel.
    slider_width: f32,
    /// The zoom of the panel: how much bigger it is than its design size.
    zoom: f32,
}

impl<'a, 'b> Ctx<'a, 'b> {
    /// The readout slot of the panel.
    fn readout(&mut self) -> &mut Option<(String, String)> {
        &mut self.state.readout
    }

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

        let zoom = self.zoom;

        caption(ui, label, 9.0, zoom, LABEL);

        // The slider takes the height that is left in its column above the
        // numeric field, so that a section never overflows its rectangle and
        // the field always sits at the bottom of the column.
        let height = slider_height(ui, zoom);
        let knob = &Knob {
            width: knob.width.max(self.slider_width),
            ..*knob
        };

        let mut value = param.value();
        let response = v_slider(ui, &mut value, knob, height, zoom);
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

        self.value_popup_float(ui, id, label, param, knob, &response);
        let value = format_number(knob.display(param.value()), knob.decimals);
        if response.hovered() || response.dragged() {
            *self.readout() = Some((
                label.to_ascii_uppercase(),
                format!("{}{}", value, param.unit()),
            ));
        }
        response.on_hover_text(format!("{}{}\nRight-click to type", value, param.unit()));
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

        let zoom = self.zoom;

        caption(ui, label, 9.0, zoom, LABEL);

        // The slider takes the height that is left in its column above the
        // numeric field, so that a section never overflows its rectangle and
        // the field always sits at the bottom of the column.
        let height = slider_height(ui, zoom);
        let knob = &Knob {
            width: knob.width.max(self.slider_width),
            ..*knob
        };

        let mut value = param.value() as f32;
        let response = v_slider(ui, &mut value, knob, height, zoom);
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

        self.value_popup_int(ui, id, label, param, knob, &response);
        let value = format_number(knob.display(param.value() as f32), knob.decimals);
        if response.hovered() || response.dragged() {
            *self.readout() = Some((
                label.to_ascii_uppercase(),
                format!("{}{}", value, param.unit()),
            ));
        }
        response.on_hover_text(format!("{}{}\nRight-click to type", value, param.unit()));
    }

    /// A small popup that edits the value of a continuous parameter. It opens
    /// on a secondary click on the slider and replaces the old numeric field.
    fn value_popup_float(
        &mut self,
        _ui: &mut egui::Ui,
        id: &'static str,
        label: &str,
        param: &FloatParam,
        knob: &Knob,
        response: &egui::Response,
    ) {
        let zoom = self.zoom;
        let popup = egui::Popup::context_menu(response)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .frame(
                egui::Frame::new()
                    .fill(BUTTON_BG)
                    .stroke(Stroke::new(1.0, LINE))
                    .inner_margin(Margin::same((6.0 * zoom).round() as i8)),
            );

        popup.show(|ui| {
            let text = self.state.texts.entry(id).or_default();
            if response.secondary_clicked() {
                *text = format_number(knob.display(param.value()), knob.decimals);
            }

            ui.label(format!("{}: {}", label, param.name()));
            let edit = ui.add(
                egui::TextEdit::singleline(text)
                    .font(egui::TextStyle::Monospace)
                    .desired_width((VALUE_POPUP_WIDTH - 12.0) * zoom),
            );
            if !edit.has_focus() {
                edit.request_focus();
            }

            let mut commit = ui.input(|input| input.key_pressed(egui::Key::Enter));
            let mut cancel = ui.input(|input| input.key_pressed(egui::Key::Escape));
            let mut invalid = false;
            ui.horizontal(|ui| {
                if ui.button("OK").clicked() {
                    commit = true;
                }
                if ui.button("CANCEL").clicked() {
                    cancel = true;
                }
            });

            if commit {
                match text.trim().parse::<f32>() {
                    Ok(parsed) if parsed.is_finite() => {
                        let plain = knob.parse(parsed);
                        self.setter.begin_set_parameter(param);
                        self.setter.set_parameter(param, plain);
                        self.setter.end_set_parameter(param);
                        self.state.dirty = true;
                        egui::Popup::close_all(ui.ctx());
                    }
                    _ => invalid = true,
                }
            } else if cancel {
                egui::Popup::close_all(ui.ctx());
            }

            if invalid {
                ui.colored_label(STATUS_ERROR, "Invalid value");
            }
        });
    }

    /// The same editor for a stepped parameter.
    fn value_popup_int(
        &mut self,
        _ui: &mut egui::Ui,
        id: &'static str,
        label: &str,
        param: &IntParam,
        knob: &Knob,
        response: &egui::Response,
    ) {
        let zoom = self.zoom;
        let popup = egui::Popup::context_menu(response)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .frame(
                egui::Frame::new()
                    .fill(BUTTON_BG)
                    .stroke(Stroke::new(1.0, LINE))
                    .inner_margin(Margin::same((6.0 * zoom).round() as i8)),
            );

        popup.show(|ui| {
            let text = self.state.texts.entry(id).or_default();
            if response.secondary_clicked() {
                *text = format_number(knob.display(param.value() as f32), knob.decimals);
            }

            ui.label(format!("{}: {}", label, param.name()));
            let edit = ui.add(
                egui::TextEdit::singleline(text)
                    .font(egui::TextStyle::Monospace)
                    .desired_width((VALUE_POPUP_WIDTH - 12.0) * zoom),
            );
            if !edit.has_focus() {
                edit.request_focus();
            }

            let mut commit = ui.input(|input| input.key_pressed(egui::Key::Enter));
            let mut cancel = ui.input(|input| input.key_pressed(egui::Key::Escape));
            let mut invalid = false;
            ui.horizontal(|ui| {
                if ui.button("OK").clicked() {
                    commit = true;
                }
                if ui.button("CANCEL").clicked() {
                    cancel = true;
                }
            });

            if commit {
                match text.trim().parse::<f32>() {
                    Ok(parsed) if parsed.is_finite() => {
                        let plain = knob.parse(parsed).round() as i32;
                        self.setter.begin_set_parameter(param);
                        self.setter.set_parameter(param, plain);
                        self.setter.end_set_parameter(param);
                        self.state.dirty = true;
                        egui::Popup::close_all(ui.ctx());
                    }
                    _ => invalid = true,
                }
            } else if cancel {
                egui::Popup::close_all(ui.ctx());
            }

            if invalid {
                ui.colored_label(STATUS_ERROR, "Invalid value");
            }
        });
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
            .width(width * self.zoom)
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

        #[cfg(test)]
        self.state.popup_rects.push(response.rect);

        if response.hovered() {
            let label = id.replace('_', " ").to_ascii_uppercase();
            let value = variants[T::to_index(param.value())].to_ascii_uppercase();
            self.state.readout = Some((label, value));
        }

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

        let zoom = self.zoom;

        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(CHECK_SIZE, CHECK_SIZE) * zoom, Sense::hover());
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
                FontId::monospace(font_size(9.0, zoom)),
                TEXT,
            );
        }

        if response.hovered() {
            self.state.readout = Some((
                label.to_ascii_uppercase(),
                if param.value() { "ON" } else { "OFF" }.to_string(),
            ));
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

        text_label(ui, label, 9.0, zoom, TEXT);
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

        let zoom = self.zoom;

        let variants = T::variants();
        let current = T::to_index(param.value());
        let button_width = width * zoom / variants.len() as f32;

        for (index, variant) in variants.iter().enumerate() {
            let (rect, _) = ui.allocate_exact_size(
                egui::vec2(button_width, RADIO_HEIGHT * zoom),
                Sense::hover(),
            );
            #[cfg(test)]
            self.state.radio_rects.push(rect);
            let response = ui.interact(
                rect,
                ui.scope_id().with(("sushi-radio", id, index)),
                Sense::click(),
            );
            if response.hovered() {
                self.state.readout = Some((id.to_ascii_uppercase(), variant.to_ascii_uppercase()));
            }
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
                FontId::proportional(font_size(9.0, zoom)),
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
        self.refresh_patch_preview();
    }

    /// Draws the patch browser as a full page. It replaces the synth panel
    /// instantly instead of opening a floating, resizable window.
    fn patch_page(&mut self, ui: &mut egui::Ui) {
        let zoom = self.zoom;
        let page = ui.max_rect();
        ui.painter().rect_filled(page, CornerRadius::ZERO, BG);
        let mut close = false;
        let inner = page.shrink(12.0 * zoom);

        ui.scope_builder(
            egui::UiBuilder::new()
                .id_salt("sushi-patch-page")
                .max_rect(inner)
                .layout(EguiLayout::top_down(Align::Min)),
            |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::Label::new(
                        egui::RichText::new("PATCHES")
                            .font(FontId::new(font_size(16.0, zoom), bold()))
                            .color(TITLE_BLUE),
                    ));
                    ui.with_layout(EguiLayout::right_to_left(Align::Center), |ui| {
                        if ui.button("CLOSE").clicked() {
                            close = true;
                        }
                    });
                });

                // FOLDER: the button follows a field that absorbs the row.
                ui.horizontal(|ui| {
                    text_label(ui, "FOLDER", 9.0, zoom, TEXT);
                    let width = (ui.available_width() - 52.0 * zoom).max(120.0 * zoom);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.state.patch_dir_text)
                            .id(egui::Id::unique("sushi-patch-folder"))
                            .desired_width(width),
                    );
                    if ui.button("SET").clicked() {
                        self.set_patch_folder();
                    }
                });
                if !self.folder_is_applied() {
                    ui.colored_label(MUTED, FOLDER_HINT);
                }

                // SEARCH + REFRESH + count in one row.
                ui.horizontal(|ui| {
                    text_label(ui, "SEARCH", 9.0, zoom, TEXT);
                    let width = (ui.available_width() - 150.0 * zoom).max(120.0 * zoom);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.state.patch_search)
                            .id(egui::Id::unique("sushi-patch-search"))
                            .desired_width(width),
                    );
                    if ui.button("REFRESH").clicked() {
                        self.refresh_patch_list();
                    }
                    ui.label(format!("{} patches", self.state.patch_files.len()));
                });

                // Pin the action area to the page bottom, then give the
                // remaining height to the lists and JSON.
                let bottom_height = 64.0 * zoom;
                let bottom_rect = Rect::from_min_max(
                    egui::pos2(inner.left(), inner.bottom() - bottom_height),
                    inner.max,
                );
                let body_height = (bottom_rect.top() - ui.next_widget_position().y - 8.0 * zoom)
                    .max(120.0 * zoom);
                let body_width = ui.available_width();
                let left_width = (body_width * 0.32)
                    .clamp(240.0 * zoom, 340.0 * zoom)
                    .min((body_width - 240.0 * zoom).max(200.0 * zoom));

                ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(left_width, body_height),
                        EguiLayout::top_down(Align::Min),
                        |ui| {
                            egui::ScrollArea::vertical()
                                .id_salt("sushi-patch-list")
                                .max_height(body_height)
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    let search = self.state.patch_search.trim().to_lowercase();
                                    let matches = |name: &str| {
                                        search.is_empty() || name.to_lowercase().contains(&search)
                                    };

                                    text_label(ui, "USER", 9.0, zoom, MUTED);
                                    let files = self.state.patch_files.clone();
                                    if files.is_empty() {
                                        ui.colored_label(MUTED, NO_PATCHES);
                                    }
                                    let mut found = 0;
                                    for path in files {
                                        let Some(stem) = path.file_stem() else {
                                            continue;
                                        };
                                        let name = stem.to_string_lossy().into_owned();
                                        if !matches(&name) {
                                            continue;
                                        }
                                        found += 1;
                                        let selected = self.state.patch_active.as_deref()
                                            == Some(name.as_str());
                                        let clicked = ui
                                            .push_id(("user", &name), |ui| {
                                                ui.selectable_label(selected, &name).clicked()
                                            })
                                            .inner;
                                        if clicked {
                                            self.load_patch_file(&path);
                                        }
                                    }

                                    text_label(ui, "FACTORY", 9.0, zoom, MUTED);
                                    for (index, preset) in PRESETS.iter().enumerate() {
                                        if !matches(preset.name) {
                                            continue;
                                        }
                                        found += 1;
                                        let selected = self.state.patch_active.is_none()
                                            && self.state.preset_index == index;
                                        let clicked = ui
                                            .push_id(("factory", index), |ui| {
                                                ui.selectable_label(selected, preset.name).clicked()
                                            })
                                            .inner;
                                        if clicked {
                                            self.load_preset(index);
                                        }
                                    }

                                    if found == 0 {
                                        ui.colored_label(MUTED, NO_MATCHES);
                                    }
                                });
                        },
                    );

                    ui.separator();

                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), body_height),
                        EguiLayout::top_down(Align::Min),
                        |ui| {
                            ui.horizontal(|ui| {
                                text_label(ui, "JSON", 9.0, zoom, TITLE_BLUE);
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(&self.state.patch_preview_title)
                                            .size(font_size(9.0, zoom))
                                            .color(MUTED),
                                    )
                                    .truncate(),
                                );
                            });
                            let preview_height = (body_height - 24.0 * zoom).max(40.0 * zoom);
                            egui::Frame::NONE
                                .fill(FIELD_BG)
                                .stroke(Stroke::new(1.0, LINE))
                                .inner_margin(Margin::same(3))
                                .show(ui, |ui| {
                                    ui.set_min_height(preview_height);
                                    egui::ScrollArea::vertical()
                                        .id_salt("sushi-patch-json")
                                        .max_height(preview_height)
                                        .auto_shrink([false, false])
                                        .show(ui, |ui| {
                                            ui.add(
                                                egui::TextEdit::multiline(
                                                    &mut self.state.patch_preview,
                                                )
                                                .font(FontId::new(
                                                    font_size(10.0, zoom),
                                                    egui::FontFamily::Name(Arc::from(CODE_FAMILY)),
                                                ))
                                                .desired_width(ui.available_width())
                                                .interactive(false),
                                            );
                                        });
                                });
                        },
                    );
                });

                ui.scope_builder(
                    egui::UiBuilder::new()
                        .id_salt("sushi-patch-actions")
                        .max_rect(bottom_rect)
                        .layout(EguiLayout::top_down(Align::Min)),
                    |ui| {
                        ui.horizontal(|ui| {
                            text_label(ui, "NAME", 9.0, zoom, TEXT);
                            let width = (ui.available_width() - 110.0 * zoom).max(100.0 * zoom);
                            let response = ui.add(
                                egui::TextEdit::singleline(&mut self.state.patch_name)
                                    .id(egui::Id::unique("sushi-patch-name"))
                                    .desired_width(width),
                            );
                            if response.changed() {
                                self.state.patch_confirm_overwrite = false;
                            }
                            if ui.button("LOAD").clicked() {
                                self.load_patch_by_name();
                            }
                            if ui.button("SAVE").clicked() {
                                self.save_patch();
                            }
                        });

                        let color = if self.state.patch_status_error {
                            STATUS_ERROR
                        } else {
                            STATUS_OK
                        };
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(&self.state.patch_status).color(color),
                            )
                            .truncate(),
                        );
                    },
                );
            },
        );

        if close {
            self.state.patch_open = false;
        }
    }

    /// Opens or closes the window of the patch browser.
    ///
    /// Opening it reads the list of the patches of the folder again, so that
    /// the files that were saved in the meantime show up.
    fn toggle_patch_browser(&mut self) {
        if self.state.patch_open {
            self.state.patch_open = false;
            return;
        }

        self.refresh_patch_list();
        self.refresh_patch_preview();
        self.state.patch_open = true;
    }

    /// Loads the patch whose name is in the field of the name.
    fn load_patch_by_name(&mut self) {
        let name = patch::sanitize_file_name(&self.state.patch_name);
        let path = self.patch_dir().join(format!("{name}.json"));

        if !path.exists() {
            self.set_status("Not found".to_string(), true);
            return;
        }

        self.load_patch_file(&path);
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

    /// Rebuilds the JSON preview from the selected user patch, or from the
    /// current sound when a factory preset is selected. This is a read-only
    /// view of the actual document, not a second copy of the sound.
    fn refresh_patch_preview(&mut self) {
        let title = if let Some(active) = &self.state.patch_active {
            active.clone()
        } else {
            format!(
                "{:02} {}",
                self.state.preset_index + 1,
                PRESETS[self.state.preset_index].name
            )
        };

        if let Some(active) = &self.state.patch_active {
            let path = self
                .patch_dir()
                .join(format!("{}.json", patch::sanitize_file_name(active)));
            if let Ok(text) = std::fs::read_to_string(&path) {
                self.state.patch_preview = text;
                self.state.patch_preview_title = title;
                return;
            }
        }

        self.state.patch_preview =
            serde_json::to_string_pretty(&patch::to_json(self.params, &title))
                .unwrap_or_else(|_| "{\n  \"error\": \"preview unavailable\"\n}".to_string());
        self.state.patch_preview_title = title;
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
                self.refresh_patch_preview();
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
                self.refresh_patch_preview();
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
        self.refresh_patch_preview();
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

/// Lays out a row of sections. The places of the sections come from the
/// layout, so that they grow with the zoom of the panel.
fn group_row(
    ui: &mut egui::Ui,
    name: &str,
    layout: &Layout,
    row: usize,
    mut add: impl FnMut(usize, Rect, &mut egui::Ui),
) {
    for index in 0..Layout::group_count(row) {
        let box_rect = layout.group(row, index);

        ui.scope_builder(
            egui::UiBuilder::new()
                .id_salt(("sushi-group", name, index))
                .max_rect(box_rect)
                .layout(EguiLayout::top_down(Align::Min)),
            |ui| add(index, box_rect, ui),
        );
    }
}

/// The colour of the title of a section: blue for the sound sources, the
/// pulse widths and the arpeggiator, olive for the envelopes and the
/// amplifier, gray for the rest.
fn title_color(_title: &str) -> Color32 {
    TITLE_BLUE
}

/// Draws a section and hands its header and body to the given callback.
fn group(
    ui: &mut egui::Ui,
    rect: Rect,
    title: &str,
    zoom: f32,
    mut add: impl FnMut(&mut egui::Ui, Part),
) {
    let painter = ui.painter_at(rect);
    let color = title_color(title);

    // No vertical accent bars and no coloured rule heads: the reference uses
    // blue section names on paper and lets the type do the grouping.
    let font = FontId::new(font_size(13.0, zoom), bold());
    let galley = painter.layout_no_wrap(title.to_ascii_uppercase(), font, color);
    let strip = Rect::from_min_size(
        rect.left_top() + egui::vec2(GROUP_PADDING, 0.0) * zoom,
        egui::vec2(galley.size().x + 6.0 * zoom, TITLE_HEIGHT * zoom),
    );
    let title_pos =
        strip.left_top() + egui::vec2(0.0, (TITLE_HEIGHT * zoom - galley.size().y) * 0.5);
    painter.galley(title_pos, galley, color);

    // A quiet neutral rule remains for structure, but no yellow segment.
    let rule_y = rect.top() + TITLE_HEIGHT * zoom;
    painter.line_segment(
        [
            egui::pos2(rect.left(), rule_y),
            egui::pos2(rect.right(), rule_y),
        ],
        Stroke::new(1.0, RULE),
    );
    // A faint line through the fader bank, as in the reference.
    let body_top = rect.top() + (TITLE_HEIGHT + 3.0) * zoom;
    let body_bottom = rect.bottom() - GROUP_PADDING * zoom;
    let bank_center =
        body_top + LABEL_HEIGHT * zoom + (body_bottom - body_top - LABEL_HEIGHT * zoom) * 0.5;
    for offset in [-2.0, 2.0] {
        let y = bank_center + offset * zoom;
        painter.line_segment(
            [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
            Stroke::new(1.0, TICK),
        );
    }

    // The header of the section, next to the title.
    let header = Rect::from_min_size(
        egui::pos2(strip.right() + 4.0 * zoom, rect.top()),
        egui::vec2(
            (rect.right() - GROUP_PADDING * zoom - strip.right() - 4.0 * zoom).max(0.0),
            TITLE_HEIGHT * zoom,
        ),
    );
    ui.scope_builder(
        egui::UiBuilder::new()
            .id_salt(("sushi-header", title))
            .max_rect(header)
            .layout(EguiLayout::left_to_right(Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(4.0, 0.0) * zoom;
            add(ui, Part::Header);
        },
    );

    // The body of the section, below the rule. It has no frame, so its
    // content can use the full height of the row.
    let body = Rect::from_min_size(
        rect.left_top() + egui::vec2(GROUP_PADDING, (TITLE_HEIGHT + 3.0) * zoom),
        egui::vec2(
            rect.width() - 2.0 * GROUP_PADDING * zoom,
            (rect.height() - (TITLE_HEIGHT + 3.0) * zoom - GROUP_PADDING * zoom).max(0.0),
        ),
    );
    ui.scope_builder(
        egui::UiBuilder::new()
            .id_salt(("sushi-body", title))
            .max_rect(body)
            .layout(EguiLayout::top_down(Align::Min)),
        |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(0.0, 2.0) * zoom;
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
    zoom: f32,
    mut add: impl FnMut(usize, Rect, &mut egui::Ui),
) {
    let gap = gap * zoom;
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
                .layout(EguiLayout::top_down(Align::Min)),
            |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 2.0) * zoom;
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
                egui::pos2(x, start.y + 2.0 * zoom),
                egui::pos2(x, start.y + height - 2.0 * zoom),
            ],
            Stroke::new(1.0, RULE),
        );
    }

    ui.advance_cursor_after_rect(Rect::from_min_size(start, egui::vec2(width, height)));
}

/// Lays out a row of parameter columns with a fixed slot width. The bank is
/// centered in its section, so a four-fader oscillator and a six-fader mixer
/// share the same pitch: one of the strongest cues that this is a machined
/// instrument rather than a form.
fn column_row_fixed(
    ui: &mut egui::Ui,
    name: &str,
    count: usize,
    gap: f32,
    slot: f32,
    zoom: f32,
    mut add: impl FnMut(usize, &mut egui::Ui),
) {
    let gap = gap * zoom;
    let slot = slot * zoom;
    let start = ui.next_widget_position();
    let total_width = ui.available_width();
    let height = ui.available_height();
    let bank = count as f32 * slot + (count as f32 - 1.0) * gap;
    let left = start.x + (total_width - bank).max(0.0) * 0.5;

    for index in 0..count {
        let rect = Rect::from_min_size(
            egui::pos2(left + index as f32 * (slot + gap), start.y),
            egui::vec2(slot, height),
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .id_salt(("sushi-column-fixed", name, index))
                .max_rect(rect)
                .layout(EguiLayout::top_down(Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 2.0) * zoom;
                add(index, ui);
            },
        );
    }

    ui.advance_cursor_after_rect(Rect::from_min_size(start, egui::vec2(total_width, height)));
}

/// Draws a caption above a control. The caption is as wide as the space that
/// is left in its column.
fn caption(ui: &mut egui::Ui, text: &str, size: f32, zoom: f32, color: Color32) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, LABEL_HEIGHT * zoom), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        text.to_ascii_uppercase(),
        FontId::proportional(font_size(size, zoom)),
        color,
    );
}

/// Draws a small text label that takes just as much space as it needs.
fn text_label(ui: &mut egui::Ui, text: &str, size: f32, zoom: f32, color: Color32) {
    let font = FontId::proportional(font_size(size, zoom));
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, color);
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(galley.size().x, galley.size().y), Sense::hover());
    ui.painter_at(rect).galley(rect.left_top(), galley, color);
}

// --- The widgets -------------------------------------------------------------

/// The smallest height that a slider is ever given. Rows are laid out so that
/// this is never reached; it only keeps a slider usable in a squeezed layout.
const MIN_SLIDER_HEIGHT: f32 = 24.0;

/// The height of a slider: all the room that is left in its column below its
/// caption. Without a numeric field the slider uses the full height, which
/// keeps the panel compact.
fn slider_height(ui: &egui::Ui, zoom: f32) -> f32 {
    ui.available_height().max(MIN_SLIDER_HEIGHT * zoom)
}

/// The step of the mouse wheel of a slider: a hundredth of the range.
const WHEEL_STEP: f32 = 0.01;
/// The step of the mouse wheel of a slider while the shift key is held
/// down: a five hundredth of the range.
const WHEEL_STEP_FINE: f32 = 0.002;

/// The scroll of the mouse wheel of this frame and whether the fine step was
/// asked for.
///
/// The wheel events of the frame carry the raw scroll of the wheel and the
/// modifiers of the wheel itself, which is more reliable than the ones of the
/// input state. The smooth scroll delta of egui is the fallback for the
/// hosts that do not send the wheel events. It is only used when it grows,
/// so that the tail of a scroll that egui smoothed out does not move the
/// value again and again.
fn wheel_scroll(ui: &egui::Ui) -> (f32, bool) {
    let (raw, fine) = ui.input(|input| {
        let mut scroll = 0.0;
        let mut fine = input.modifiers.shift;

        for event in &input.events {
            if let egui::Event::MouseWheel {
                delta, modifiers, ..
            } = event
            {
                scroll += delta.y;
                fine = modifiers.shift;
            }
        }

        (scroll, fine)
    });

    let smooth = ui.input(|input| input.smooth_scroll_delta.y);

    // The last smooth scroll of this slider, so that the tail of a scroll
    // can be told apart from a new one.
    let id = ui.scope_id().with("sushi-wheel");
    let last: Option<f32> = ui.memory(|memory| memory.data.get_temp(id));
    ui.memory_mut(|memory| memory.data.insert_temp(id, smooth));

    if raw != 0.0 {
        return (raw, fine);
    }

    let fresh = match last {
        Some(last) => smooth.abs() > last.abs(),
        None => smooth != 0.0,
    };

    if fresh { (smooth, fine) } else { (0.0, fine) }
}

/// The maximum trim of the fine rail, as a fraction of the full range. The
/// narrow lane is a relative trim around the coarse value, not a second
/// absolute control.
const FINE_TRIM_RANGE: f32 = 0.1;

/// The drag state of the fine lane: the value it started from and where the
/// pointer started. A full-height drag gives exactly ±10% of the range.
#[derive(Debug, Clone, Copy, Default)]
struct FineDrag {
    active: bool,
    base: f32,
    start_y: f32,
}

/// The geometry of one two-lane fader. The thumb width is a ratio of the
/// inner lane width and the thumb is always centered in that lane, so the
/// blank margins are identical by construction no matter how wide the panel
/// or the fader becomes.
#[derive(Clone, Copy)]
struct FaderGeometry {
    track: Rect,
    coarse_lane: Rect,
    fine_lane: Rect,
    divider: f32,
    handle_height: f32,
    vertical_inset: f32,
}

impl FaderGeometry {
    const COARSE_THUMB_RATIO: f32 = 0.75;
    const FINE_THUMB_RATIO: f32 = 0.62;

    fn new(rect: Rect, zoom: f32) -> Self {
        let track = rect.shrink(GROOVE_INSET * zoom);
        let fine_width = 8.0 * zoom;
        let divider = track.right() - fine_width;
        let border = zoom;
        // Inner lanes exclude the outer frame and the divider line. A thumb
        // is centered in one of these lanes, never in the full track.
        let coarse_lane = Rect::from_min_max(
            egui::pos2(track.left() + border, track.top()),
            egui::pos2(divider - border, track.bottom()),
        );
        let fine_lane = Rect::from_min_max(
            egui::pos2(divider + border, track.top()),
            egui::pos2(track.right() - border, track.bottom()),
        );
        Self {
            track,
            coarse_lane,
            fine_lane,
            divider,
            handle_height: HANDLE_HEIGHT * zoom,
            vertical_inset: HANDLE_INSET * zoom,
        }
    }

    /// The vertical centre of both thumbs for a parameter position.
    fn thumb_y(&self, position: f32) -> f32 {
        position.clamp(
            self.track.top() + self.handle_height * 0.5 + self.vertical_inset,
            self.track.bottom() - self.handle_height * 0.5 - self.vertical_inset,
        )
    }

    /// A thumb centered in its lane. The two side margins are the same by
    /// definition, which is the whole point of this helper.
    fn thumb(&self, lane: Rect, ratio: f32, y: f32) -> Rect {
        Rect::from_center_size(
            egui::pos2(lane.center().x, y),
            egui::vec2((lane.width() * ratio).max(2.0), self.handle_height),
        )
    }

    fn coarse_thumb(&self, y: f32) -> Rect {
        self.thumb(self.coarse_lane, Self::COARSE_THUMB_RATIO, y)
    }

    fn fine_thumb(&self, y: f32) -> Rect {
        self.thumb(self.fine_lane, Self::FINE_THUMB_RATIO, y)
    }
}

/// A vertical slider with a coarse lane and a fine lane, following the
/// reference: one white framed track, one divider, a black bar and a smaller
/// black square. The two lanes are separate drag targets.
fn v_slider(
    ui: &mut egui::Ui,
    value: &mut f32,
    knob: &Knob,
    height: f32,
    zoom: f32,
) -> egui::Response {
    let (rect, mut response) =
        ui.allocate_exact_size(egui::vec2(knob.width, height), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    let geometry = FaderGeometry::new(rect, zoom);

    flat_box(&painter, geometry.track, TRACK);
    painter.line_segment(
        [
            egui::pos2(geometry.divider, geometry.track.top()),
            egui::pos2(geometry.divider, geometry.track.bottom()),
        ],
        Stroke::new(1.0, LINE),
    );

    let id = ui.scope_id().with("sushi-fine-drag");
    let mut fine_drag: FineDrag = ui
        .memory(|memory| memory.data.get_temp(id))
        .unwrap_or_default();
    // `interact_pointer_pos()` is None while merely hovering. Use the real
    // hover position for lane highlighting and the interaction position only
    // for the drag math.
    let hover_pos = ui.input(|input| input.pointer.hover_pos());
    let pointer = response.interact_pointer_pos();
    let over_fine = hover_pos.is_some_and(|pos| pos.x > geometry.divider);
    let pressed = ui.input(|input| input.pointer.primary_pressed());

    if (response.drag_started() || pressed) && over_fine && !fine_drag.active {
        let start = pointer.or(hover_pos);
        fine_drag = FineDrag {
            active: true,
            base: value_to_normalized(*value, knob.min, knob.max, knob.log),
            start_y: start.map_or(0.0, |pos| pos.y),
        };
        ui.memory_mut(|memory| memory.data.insert_temp(id, fine_drag));
    } else if (response.drag_started() || pressed) && !over_fine && !fine_drag.active {
        ui.memory_mut(|memory| memory.data.insert_temp(id, FineDrag::default()));
    }
    if response.drag_stopped() {
        fine_drag = FineDrag::default();
        ui.memory_mut(|memory| memory.data.insert_temp(id, fine_drag));
    }

    let position = value_to_position(*value, knob.min, knob.max, knob.log, geometry.track);
    let thumb_y = geometry.thumb_y(position);
    let coarse_thumb = geometry.coarse_thumb(thumb_y);
    let fine_thumb = geometry.fine_thumb(thumb_y);

    let coarse_live =
        (response.hovered() && !over_fine) || (response.dragged() && !fine_drag.active);
    let fine_live = (response.hovered() && over_fine) || (response.dragged() && fine_drag.active);
    painter.rect_filled(
        coarse_thumb,
        CornerRadius::ZERO,
        if coarse_live { TITLE_BLUE } else { HANDLE },
    );
    painter.rect_filled(
        fine_thumb,
        CornerRadius::ZERO,
        if fine_live { TITLE_BLUE } else { HANDLE },
    );

    let primary = egui::PointerButton::Primary;
    if response.dragged_by(primary) && fine_drag.active {
        // Fine lane: a bounded trim around the value at the start of the
        // gesture; full-height travel is ±10% of the parameter range.
        let pointer_y = pointer.map_or(fine_drag.start_y, |pos| pos.y);
        let travel = (fine_drag.start_y - pointer_y) / geometry.track.height();
        let trim = (travel * FINE_TRIM_RANGE).clamp(-FINE_TRIM_RANGE, FINE_TRIM_RANGE);
        let moved = (fine_drag.base + trim).clamp(0.0, 1.0);
        let new_value = position_to_value(moved, knob.min, knob.max, knob.log);
        if new_value != *value {
            *value = new_value;
            response.mark_changed();
        }
    } else if (response.dragged_by(primary)
        || (response.is_pointer_button_down_on() && ui.input(|input| input.pointer.primary_down())))
        && let Some(pointer) = pointer
        && pointer.x <= geometry.divider
    {
        // Coarse lane: absolute, as in the old slider.
        let position =
            ((geometry.track.bottom() - pointer.y) / geometry.track.height()).clamp(0.0, 1.0);
        *value = position_to_value(position, knob.min, knob.max, knob.log);
        response.mark_changed();
    }

    if response.hovered() && !response.dragged() && !response.is_pointer_button_down_on() {
        let (scroll, fine) = wheel_scroll(ui);
        if scroll != 0.0 {
            let step = if fine || over_fine {
                WHEEL_STEP_FINE
            } else {
                WHEEL_STEP
            };
            let direction = if scroll > 0.0 { 1.0 } else { -1.0 };
            let normalized = value_to_normalized(*value, knob.min, knob.max, knob.log);
            let moved = (normalized + direction * step).clamp(0.0, 1.0);
            let new_value = position_to_value(moved, knob.min, knob.max, knob.log);
            if new_value != *value {
                *value = new_value;
                response.mark_changed();
            }
        }
    }

    if response.hovered() && over_fine {
        response = response.on_hover_text("Fine adjustment: drag slowly");
    }
    response.on_hover_cursor(egui::CursorIcon::ResizeVertical)
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
fn voice_meter(painter: &egui::Painter, rect: Rect, voices: usize, zoom: f32) {
    let font = FontId::proportional(font_size(8.0, zoom));
    let label = painter.layout_no_wrap("VOICES".to_string(), font.clone(), MUTED);
    let label_width = label.size().x;

    let row_width =
        (VOICE_LEDS as f32 * VOICE_LED_WIDTH + (VOICE_LEDS as f32 - 1.0) * VOICE_LED_GAP) * zoom;
    let total = label_width + VOICE_METER_GAP * zoom + row_width;

    // The meter is centered in the free space of the header.
    let left = (rect.left() + (rect.width() - total) * 0.5).round();
    let top = rect.center().y - VOICE_LED_HEIGHT * zoom * 0.5;

    painter.text(
        egui::pos2(left, rect.center().y),
        Align2::LEFT_CENTER,
        "VOICES",
        font,
        MUTED,
    );

    // The row of the lights. Every light that stands for a playing voice is
    // green, the others are dark.
    let row_left = (left + label_width + VOICE_METER_GAP * zoom).round();
    for index in 0..VOICE_LEDS {
        let x = row_left + index as f32 * (VOICE_LED_WIDTH + VOICE_LED_GAP) * zoom;
        let color = if index < voices { LED_ON } else { LED_OFF };
        painter.rect_filled(
            Rect::from_min_size(
                egui::pos2(x, top),
                egui::vec2(VOICE_LED_WIDTH, VOICE_LED_HEIGHT) * zoom,
            ),
            CornerRadius::ZERO,
            color,
        );
    }
}

/// The color of the arpeggiator light for a step. The color is replaced on
/// every step instead of switching the light off and on, so that a pattern at
/// 1/32 still looks alive: it becomes a small, moving color cycle.
fn arp_light_color(step: usize) -> Color32 {
    const COLORS: [Color32; 8] = [
        Color32::from_rgb(0x2A, 0x9D, 0x6A), // green
        Color32::from_rgb(0x1E, 0x9E, 0x9E), // teal
        Color32::from_rgb(0x3A, 0xA6, 0xC9), // cyan
        Color32::from_rgb(0x3F, 0x6F, 0xB5), // blue
        Color32::from_rgb(0x59, 0x61, 0xB3), // indigo
        Color32::from_rgb(0x8A, 0x5B, 0xB5), // violet
        Color32::from_rgb(0xB3, 0x55, 0xA3), // magenta
        Color32::from_rgb(0xC1, 0x4B, 0x56), // red
    ];
    COLORS[step % COLORS.len()]
}

/// A small light: a flat square that is either lit or switched off.
fn led(painter: &egui::Painter, center: Pos2, color: Color32, on: bool, zoom: f32) {
    let rect = Rect::from_center_size(center, egui::vec2(LED_SIZE, LED_SIZE) * zoom);
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
fn lcd(painter: &egui::Painter, rect: Rect, text: &str, zoom: f32) {
    flat_box(painter, rect, LCD_BG);

    let font = FontId::monospace(font_size(14.0, zoom));
    let max_width = (rect.width() - 14.0 * zoom).max(1.0);
    let mut fitted = text.to_string();
    while !fitted.is_empty()
        && painter
            .layout_no_wrap(fitted.clone(), font.clone(), LCD_FG)
            .size()
            .x
            > max_width
    {
        fitted.pop();
    }

    let painter = painter.with_clip_rect(rect.shrink(2.0 * zoom));
    painter.text(
        rect.left_top() + egui::vec2(7.0, rect.height() * 0.5),
        Align2::LEFT_CENTER,
        fitted,
        font,
        LCD_FG,
    );
}

/// A flat square button: it is filled darker while it is pressed.
fn button(ui: &mut egui::Ui, rect: Rect, text: &str, zoom: f32) -> egui::Response {
    let response = ui.interact(
        rect,
        ui.scope_id().with(("sushi-button", text)),
        Sense::click(),
    );
    let painter = ui.painter_at(rect);

    let primary = text == "MUTATE";
    let pressed = response.is_pointer_button_down_on();
    let fill = if primary {
        if pressed {
            BUTTON_ACTIVE
        } else {
            Color32::from_rgb(0xDD, 0xE4, 0xEF)
        }
    } else if pressed {
        BUTTON_ACTIVE
    } else {
        BUTTON_BG
    };
    flat_box(&painter, rect, fill);
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::new(
            font_size(10.0, zoom),
            if primary {
                bold()
            } else {
                egui::FontFamily::Proportional
            },
        ),
        if primary { PAPER } else { TEXT },
    );

    response
}

/// The small oscilloscope of the header. It is just here to give the panel a
/// bit of life, not to be a measurement tool.
fn small_scope(painter: &egui::Painter, rect: Rect, history: &VecDeque<f32>, zoom: f32) {
    // A dark instrument screen with a quiet grid, even while the sound is
    // silent. It is the one live object in the header.
    flat_box(painter, rect, LCD_BG);
    let inner = rect.shrink(3.0 * zoom);
    for index in 1..4 {
        let x = inner.left() + inner.width() * index as f32 / 4.0;
        painter.line_segment(
            [egui::pos2(x, inner.top()), egui::pos2(x, inner.bottom())],
            Stroke::new(1.0, RULE),
        );
    }
    for index in 1..3 {
        let y = inner.top() + inner.height() * index as f32 / 3.0;
        painter.line_segment(
            [egui::pos2(inner.left(), y), egui::pos2(inner.right(), y)],
            Stroke::new(1.0, RULE),
        );
    }

    let length = history.len();
    if length < 2 {
        painter.line_segment(
            [
                egui::pos2(inner.left(), inner.center().y),
                egui::pos2(inner.right(), inner.center().y),
            ],
            Stroke::new(1.0, TRACE),
        );
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

/// Maps a value to its place in the range, where 0 is the bottom and 1 is the
/// top.
fn value_to_normalized(value: f32, min: f32, max: f32, log: bool) -> f32 {
    if log {
        let (low, high) = log_bounds(min, max);
        let value = value + log_offset(min);
        ((value.max(f32::MIN_POSITIVE).ln() - low.ln()) / (high.ln() - low.ln())).clamp(0.0, 1.0)
    } else {
        ((value - min) / (max - min)).clamp(0.0, 1.0)
    }
}

/// Maps a value to its position in the track, where 0 is the bottom and 1 is
/// the top.
fn value_to_position(value: f32, min: f32, max: f32, log: bool, track: Rect) -> f32 {
    let position = value_to_normalized(value, min, max, log);
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

fn apply_theme(ctx: &egui::Context, zoom: f32) {
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
        style.spacing.item_spacing = egui::vec2(6.0, 4.0) * zoom;
        style.spacing.button_padding = egui::vec2(4.0, 1.0) * zoom;
        style.spacing.interact_size.y = INTERACT_HEIGHT * zoom;
        style.spacing.slider_rail_height = 8.0 * zoom;
        style.spacing.slider_width = SLIDER_WIDTH * zoom;
        style.text_styles.insert(
            egui::TextStyle::Body,
            egui::FontId::new(font_size(12.0, zoom), egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Button,
            egui::FontId::new(font_size(11.0, zoom), egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Small,
            egui::FontId::new(font_size(11.0, zoom), egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Monospace,
            egui::FontId::new(font_size(12.0, zoom), egui::FontFamily::Monospace),
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
        /// The time of the last frame of the tests. It only grows, so that the
        /// time of the frames never moves backwards.
        static TIME: Cell<f64> = const { Cell::new(0.0) };
    }

    /// The time of the next frame of the tests. The frames of a test always
    /// move forward, so that the animations of the panel can settle.
    fn next_time(step: f64) -> f64 {
        TIME.with(|time| {
            let next = time.get() + step;
            time.set(next);
            next
        })
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
        apply_theme(&ctx, 1.0);
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
            slider_width: SLIDER_WIDTH,
            zoom: 1.0,
        }
    }

    fn input(events: Vec<egui::Event>) -> egui::RawInput {
        input_at(egui::vec2(WINDOW_WIDTH, WINDOW_HEIGHT), events)
    }

    /// The input of a frame in a window of the given size.
    fn input_at(size: egui::Vec2, events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), size)),
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
        for mut input in frames {
            // Every frame of a test moves the time forward, so that the
            // animations of the panel can settle.
            input.time = Some(next_time(1.0 / 60.0));
            let mut output = ctx.run_ui(input, |ui| {
                editor.draw(ui);
            });
            output.textures_delta.clear();
            last = Some(output);
        }
        last.unwrap()
    }

    /// The zoom of the panel in the given context. It is the zoom of the
    /// layout that the panel uses in the window of the context.
    fn zoom_of(ctx: &egui::Context) -> f32 {
        Layout::for_rect(ctx.viewport_rect())
            .map(|layout| layout.zoom)
            .unwrap_or(1.0)
    }

    /// The rects of all the vertical sliders, in the order they are drawn.
    /// The sliders grow with the zoom of the panel, so they are recognized
    /// by their width and their height.
    fn slider_rects(ctx: &egui::Context, output: &egui::FullOutput) -> Vec<egui::Rect> {
        let zoom = zoom_of(ctx);
        let mut rects = Vec::new();
        for clipped in ctx.tessellate(output.shapes.clone(), ctx.pixels_per_point()) {
            if let egui::epaint::Primitive::Mesh(_mesh) = clipped.primitive {
                let rect = clipped.clip_rect;
                let width = rect.width();
                let is_slider = (width - SLIDER_WIDTH * zoom).abs() < 0.5
                    || (width - MACRO_SLIDER_WIDTH * zoom).abs() < 0.5;
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

    /// The centre of the ring of the arpeggiator and the number of its lit
    /// sectors.
    /// Runs a few frames, so that the animations of the panel can settle.
    fn settle(ctx: &egui::Context, editor: &mut SushiEditor) -> egui::FullOutput {
        let mut last = None;
        for _ in 0..10 {
            let mut raw = input(vec![]);
            raw.time = Some(next_time(0.05));
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
    const CUTOFF_SLIDER: usize = 17;
    /// The index of the slider of the master tune. It is the first slider of
    /// the panel.
    const TUNE_SLIDER: usize = 0;
    /// The index of the slider of the number of voices. It is the last slider
    /// of the voice section.
    const VOICES_SLIDER: usize = 42;

    #[test]
    fn fader_thumbs_keep_equal_side_margins() {
        let rect = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(SLIDER_WIDTH, 200.0));
        let geometry = FaderGeometry::new(rect, 1.0);
        for (lane, thumb) in [
            (geometry.coarse_lane, geometry.coarse_thumb(100.0)),
            (geometry.fine_lane, geometry.fine_thumb(100.0)),
        ] {
            let left = thumb.left() - lane.left();
            let right = lane.right() - thumb.right();
            assert!(
                (left - right).abs() < 0.01,
                "the thumb is not centered in its lane: left {left}, right {right}"
            );
        }
    }

    #[test]
    fn the_panel_has_the_expected_number_of_widgets() {
        let (ctx, mut editor, _params) = make_editor();
        run_frames(&ctx, &mut editor, vec![input(vec![])]);

        let counts = editor.state.counts;
        assert_eq!(counts.sliders, 56, "unexpected number of sliders");
        assert_eq!(counts.popups, 11, "unexpected number of popups");
        assert_eq!(counts.checks, 3, "unexpected number of check boxes");
        assert_eq!(counts.radios, 1, "unexpected number of radio pairs");
    }

    #[test]
    fn every_parameter_of_the_synthesizer_is_reachable() {
        let (ctx, mut editor, _params) = make_editor();
        run_frames(&ctx, &mut editor, vec![input(vec![])]);

        let counts = editor.state.counts;
        // The panel page has 71 parameters: 56 sliders, 11 choices, 3 switches
        // and one pair of buttons. The modulation matrix adds 24 more on its
        // own page, which has its own test.
        let continuous = counts.sliders;
        assert_eq!(continuous, 56);
        assert_eq!(
            continuous + counts.popups + counts.checks + counts.radios,
            71
        );
    }

    #[test]
    fn dragging_the_fine_rail_moves_the_value_slowly() {
        let (ctx, mut editor, params) = make_editor();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let rect = slider_rects(&ctx, &output)[TUNE_SLIDER];
        let start = egui::pos2(rect.right() - 4.0, rect.center().y);
        let end = start - egui::vec2(0.0, 20.0);

        run_frames(
            &ctx,
            &mut editor,
            vec![
                input(vec![egui::Event::PointerMoved(start)]),
                input(vec![egui::Event::PointerButton {
                    pos: start,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::default(),
                }]),
                input(vec![egui::Event::PointerMoved(end)]),
                input(vec![egui::Event::PointerButton {
                    pos: end,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::default(),
                }]),
            ],
        );

        let value = params.tune.value();
        assert!(
            value > 0.0 && value < 2.0,
            "the fine rail was not fine: {value}"
        );
    }

    #[test]
    fn hovering_a_slider_updates_the_bottom_readout() {
        let (ctx, mut editor, _params) = make_editor();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let sliders = slider_rects(&ctx, &output);
        let pos = sliders[TUNE_SLIDER].center();

        run_frames(
            &ctx,
            &mut editor,
            vec![input(vec![egui::Event::PointerMoved(pos)])],
        );

        let (label, value) = editor
            .state
            .readout
            .clone()
            .expect("the bottom bar has no readout");
        assert_eq!(label, "TUNE");
        assert!(value.ends_with("st"), "the readout lost its unit: {value}");
    }

    #[test]
    fn clicking_a_slider_sets_the_value() {
        let (ctx, mut editor, params) = make_editor();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let sliders = slider_rects(&ctx, &output);
        assert_eq!(sliders.len(), 56, "unexpected number of sliders");
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

    /// Right-clicks the cutoff slider, clears the editor and types `text`.
    fn edit_cutoff(editor: &mut SushiEditor, ctx: &egui::Context, text: &str) {
        let output = run_frames(ctx, editor, vec![input(vec![])]);
        let sliders = slider_rects(ctx, &output);
        let pos = sliders[CUTOFF_SLIDER].center();

        // Open the value editor with a secondary click.
        run_frames(
            ctx,
            editor,
            vec![
                input(vec![egui::Event::PointerMoved(pos)]),
                input(vec![
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Secondary,
                        pressed: true,
                        modifiers: egui::Modifiers::default(),
                    },
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Secondary,
                        pressed: false,
                        modifiers: egui::Modifiers::default(),
                    },
                ]),
                input(vec![]),
                input(vec![]),
            ],
        );

        // Clear the current value and type the new one.
        let mut events = Vec::new();
        for _ in 0..8 {
            events.push(egui::Event::Key {
                key: egui::Key::Backspace,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::default(),
            });
        }
        for character in text.chars() {
            events.push(egui::Event::Text(character.to_string()));
        }
        events.push(egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        });
        run_frames(ctx, editor, vec![input(events), input(vec![])]);
    }

    #[test]
    fn right_click_does_not_move_the_slider() {
        let (ctx, mut editor, params) = make_editor();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let pos = slider_rects(&ctx, &output)[CUTOFF_SLIDER].center();
        run_frames(
            &ctx,
            &mut editor,
            vec![input(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Secondary,
                    pressed: true,
                    modifiers: egui::Modifiers::default(),
                },
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Secondary,
                    pressed: false,
                    modifiers: egui::Modifiers::default(),
                },
            ])],
        );
        assert_eq!(params.cutoff.value(), 3_000.0);
    }

    #[test]
    fn the_right_click_editor_sets_the_value() {
        let (ctx, mut editor, params) = make_editor();
        edit_cutoff(&mut editor, &ctx, "1000");
        assert_eq!(params.cutoff.value(), 1_000.0);
    }

    #[test]
    fn the_right_click_editor_rejects_nonsense() {
        let (ctx, mut editor, params) = make_editor();
        edit_cutoff(&mut editor, &ctx, "NaN");
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

    /// A scroll of the mouse wheel at the given position.
    fn wheel(pos: egui::Pos2, delta: f32, shift: bool) -> Vec<egui::RawInput> {
        let modifiers = egui::Modifiers {
            shift,
            ..Default::default()
        };

        vec![
            input(vec![egui::Event::PointerMoved(pos)]),
            input(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, delta),
                    phase: egui::TouchPhase::Move,
                    source: egui::MouseWheelSource::Wheel,
                    modifiers,
                },
            ]),
        ]
    }

    #[test]
    fn the_mouse_wheel_moves_a_slider() {
        let (ctx, mut editor, params) = make_editor();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let sliders = slider_rects(&ctx, &output);
        let rect = sliders[CUTOFF_SLIDER];
        let pos = rect.center();

        let before = params.cutoff.value();
        run_frames(&ctx, &mut editor, wheel(pos, 1.0, false));

        // The wheel moved the slider one step up.
        let after = params.cutoff.value();
        assert!(
            after > before,
            "the wheel did not move the slider: {before}"
        );
        assert!(
            editor.state.dirty,
            "the wheel did not mark the sound as dirty"
        );

        // And down again.
        run_frames(&ctx, &mut editor, wheel(pos, -1.0, false));
        let back = params.cutoff.value();
        assert!(
            (back - before).abs() < 0.001,
            "the wheel did not move the slider back: {back} instead of {before}"
        );
    }

    #[test]
    fn the_mouse_wheel_moves_a_slider_in_small_steps_with_shift() {
        let (ctx, mut editor, params) = make_editor();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let sliders = slider_rects(&ctx, &output);
        let rect = sliders[CUTOFF_SLIDER];
        let pos = rect.center();

        let before = params.cutoff.value();

        // A whole step.
        run_frames(&ctx, &mut editor, wheel(pos, 1.0, false));
        let coarse = params.cutoff.value();

        // And a small one.
        run_frames(&ctx, &mut editor, wheel(pos, 1.0, true));
        let fine = params.cutoff.value();

        assert!(fine > coarse, "the shift did not move the slider: {fine}");
        assert!(
            fine - coarse < coarse - before,
            "the shift did not move the slider in small steps: {fine} {coarse} {before}"
        );
    }

    #[test]
    fn the_mouse_wheel_does_not_move_a_slider_that_is_not_hovered() {
        let (ctx, mut editor, params) = make_editor();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let sliders = slider_rects(&ctx, &output);
        let rect = sliders[CUTOFF_SLIDER];

        // The wheel is somewhere else on the panel.
        let elsewhere = egui::pos2(rect.center().x + 40.0, rect.center().y);
        let before = params.cutoff.value();
        run_frames(&ctx, &mut editor, wheel(elsewhere, 1.0, false));

        assert_eq!(params.cutoff.value(), before);
    }

    #[test]
    fn the_mouse_wheel_does_not_steal_the_scroll_of_the_browser() {
        let (ctx, mut editor, params) = make_editor();
        editor.state.patch_open = true;

        let output = settle(&ctx, &mut editor);
        let (_, pos) = texts(&output)
            .into_iter()
            .find(|(text, _)| text == "FACTORY")
            .expect("the browser has no list");
        let list = pos + egui::vec2(2.0, 2.0);

        // The wheel is over the list of the browser, so it belongs to the
        // list and not to a slider of the panel.
        let before = params.cutoff.value();
        run_frames(&ctx, &mut editor, wheel(list, 1.0, false));

        assert_eq!(params.cutoff.value(), before);
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
        // The check boxes are VCO 2 SYNC, then the ARP ON and LATCH switches.
        // This test uses the first ARP switch.
        let rect = editor.state.check_rects[1];
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
    fn the_footer_hints_at_the_double_click_reset() {
        let (ctx, mut editor, _params) = make_editor();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);

        let (_, pos, _) = text_shapes(&output)
            .into_iter()
            .find(|(text, _, _)| text.contains("DBL-CLICK = RESET"))
            .expect("the footer has no hint");

        // The hint sits in the footer strip, below the last band.
        assert!(pos.y > WINDOW_HEIGHT - FOOTER_HEIGHT - 4.0);
    }

    #[test]
    fn the_patches_button_does_not_overlap_the_voice_meter() {
        for percent in (100..=400).step_by(5) {
            let zoom = percent as f32 / 100.0;
            let rect = Rect::from_min_size(
                egui::pos2(MARGIN, MARGIN) * zoom,
                egui::vec2(WINDOW_WIDTH - 2.0 * MARGIN, HEADER_HEIGHT) * zoom,
            );
            let rects = header_rects(rect, zoom);

            // The button of the patch browser sits between the mutation and the
            // meter of the voices.
            assert!(rects.patches.left() >= rects.mutate.right());
            assert!(rects.mod_page.left() >= rects.patches.right());
            assert!(rects.meter.left() >= rects.mod_page.right());
            assert!(!rects.patches.intersects(rects.meter));
            // The meter still fits between the button and the lights of the
            // activity meter.
            assert!(rects.meter.right() <= rect.right() - 194.0 * zoom);
            assert!(rects.meter.width() > VOICE_LEDS as f32 * VOICE_LED_WIDTH * zoom);
        }
    }

    #[test]
    fn the_mod_button_opens_the_matrix_page() {
        let (ctx, mut editor, _params) = make_editor();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let (_, pos) = texts(&output)
            .into_iter()
            .find(|(text, _)| text == "MOD")
            .expect("the header has no mod button");
        let button = pos + egui::vec2(2.0, 2.0);

        run_frames(&ctx, &mut editor, click(button));
        assert!(editor.state.mod_page);
        editor.state.counts = Counts::default();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);

        let counts = editor.state.counts;
        assert_eq!(counts.sliders, 34, "unexpected number of matrix sliders");
        assert_eq!(counts.popups, 23, "unexpected number of matrix popups");
        assert_eq!(counts.checks, 2);
        assert_eq!(counts.radios, 1);

        // The two upper bands now show eight routes; the performance and
        // output bands have not moved.
        let names: Vec<String> = texts(&output).into_iter().map(|(text, _)| text).collect();
        for route in ["01", "02", "03", "04", "05", "06", "07", "08"] {
            assert!(names.contains(&route.to_string()), "missing route {route}");
        }

        // A second click brings the sound controls back.
        run_frames(&ctx, &mut editor, click(button));
        assert!(!editor.state.mod_page);
        editor.state.counts = Counts::default();
        run_frames(&ctx, &mut editor, vec![input(vec![])]);
        assert_eq!(editor.state.counts.sliders, 56);
        assert_eq!(editor.state.counts.popups, 11);
    }

    #[test]
    fn no_two_texts_overlap_at_any_zoom() {
        for percent in (100..=400).step_by(5) {
            let zoom = percent as f32 / 100.0;
            for mod_page in [false, true] {
                let (ctx, mut editor, _params) = make_editor();
                editor.state.mod_page = mod_page;
                let output = run_frames(
                    &ctx,
                    &mut editor,
                    vec![input_at(
                        egui::vec2(WINDOW_WIDTH, WINDOW_HEIGHT) * zoom,
                        vec![],
                    )],
                );

                let texts = text_shapes(&output);
                let sliders = slider_rects(&ctx, &output);
                for (text, pos, size) in &texts {
                    let rect = Rect::from_min_size(*pos, *size);
                    for slider in &sliders {
                        let overlap = rect.intersect(*slider);
                        assert!(
                            overlap.width() <= 1.0 || overlap.height() <= 1.0,
                            "the text {text:?} is covered by a slider at {percent}% (mod={mod_page}): \
{rect:?} / {slider:?}"
                        );
                    }
                    for popup in &editor.state.popup_rects {
                        // The selected text lives inside its own popup button.
                        if popup.contains(rect.center()) {
                            continue;
                        }
                        let overlap = rect.expand(4.0 * zoom).intersect(*popup);
                        assert!(
                            overlap.width() <= 1.0 || overlap.height() <= 1.0,
                            "the text {text:?} is too close to a popup at {percent}% (mod={mod_page}): \
{rect:?} / {popup:?}"
                        );
                    }
                }
                for (index, (first, first_pos, first_size)) in texts.iter().enumerate() {
                    let first_rect = Rect::from_min_size(*first_pos, *first_size);
                    for (second, second_pos, second_size) in texts.iter().skip(index + 1) {
                        let second_rect = Rect::from_min_size(*second_pos, *second_size);
                        let overlap = first_rect.intersect(second_rect);
                        assert!(
                            overlap.width() <= 1.0 || overlap.height() <= 1.0,
                            "the texts {first:?} and {second:?} overlap at {percent}% (mod={mod_page}): \
{first_rect:?} / {second_rect:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_matrix_page_fits_into_the_panel() {
        let (ctx, mut editor, _params) = make_editor();
        editor.state.mod_page = true;
        editor.state.counts = Counts::default();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let panel = Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            egui::vec2(WINDOW_WIDTH, WINDOW_HEIGHT),
        );
        assert_the_widgets_fit(&ctx, &output, panel);
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
    fn the_list_of_the_browser_is_refreshed_when_it_opens() {
        let (ctx, mut editor, _params) = make_editor();
        let dir = TempDir::new("refresh-on-open");
        editor.state.patch_config.patch_dir = dir.path.clone();
        editor.state.patch_dir_text = dir.path.to_string_lossy().into_owned();

        // A patch that was saved after the editor was started.
        std::fs::write(
            dir.path.join("LATE.json"),
            r#"{"format":"sushi-patch","version":1,"name":"LATE","params":{"cutoff":800.0}}"#,
        )
        .expect("the patch was not written");
        editor.state.patch_files.clear();
        assert!(
            editor.state.patch_files.is_empty(),
            "the list is not empty yet"
        );

        // Opening the browser reads the list of the folder again.
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let (_, pos) = texts(&output)
            .into_iter()
            .find(|(text, _)| text == "PATCHES")
            .expect("the header has no patch button");
        run_frames(&ctx, &mut editor, click(pos + egui::vec2(2.0, 2.0)));

        assert!(editor.state.patch_open);
        assert_eq!(editor.state.patch_files.len(), 1, "the list was not read");

        let output = settle(&ctx, &mut editor);
        let names: Vec<String> = texts(&output).into_iter().map(|(text, _)| text).collect();
        assert!(
            names.contains(&"LATE".to_string()),
            "the list was not refreshed: {names:?}"
        );
    }

    #[test]
    fn the_search_filters_the_presets_and_the_patches() {
        let (ctx, mut editor, params) = make_editor();
        let dir = TempDir::new("search");
        editor.state.patch_config.patch_dir = dir.path.clone();
        editor.state.patch_dir_text = dir.path.to_string_lossy().into_owned();

        // A patch of the user.
        let gui_ctx = editor.gui_ctx.clone().expect("the editor has no host");
        let setter = gui_ctx.param_setter();
        setter.set_parameter(&params.cutoff, 1_234.0);
        editor.state.patch_name = "MY PATCH".to_string();
        {
            let mut ctx = make_ctx(&mut editor, &params, &setter);
            ctx.save_patch();
        }

        // The name of the patch is not in the list, so that the entries of the
        // list can be told apart from it.
        editor.state.patch_name = "SOMETHING ELSE".to_string();
        editor.state.patch_active = None;
        editor.state.patch_open = true;

        // The search for a preset of the factory. It does not care about the
        // case of the letters.
        editor.state.patch_search = "GlAsS".to_string();
        let output = settle(&ctx, &mut editor);
        let names: Vec<String> = texts(&output)
            .into_iter()
            .filter(|(_, pos)| pos.x < 200.0 && pos.y < 400.0)
            .map(|(text, _)| text)
            .collect();
        assert!(
            names.contains(&"GLASS KEYS".to_string()),
            "the search hid the preset: {names:?}"
        );
        assert!(
            !names.contains(&"INIT PAD".to_string()),
            "the search did not hide the preset: {names:?}"
        );
        assert!(
            !names.contains(&"MY PATCH".to_string()),
            "the search did not hide the patch: {names:?}"
        );

        // The search for a patch of the user.
        editor.state.patch_search = "my".to_string();
        let output = settle(&ctx, &mut editor);
        let names: Vec<String> = texts(&output)
            .into_iter()
            .filter(|(_, pos)| pos.x < 200.0 && pos.y < 400.0)
            .map(|(text, _)| text)
            .collect();
        assert!(
            names.contains(&"MY PATCH".to_string()),
            "the search hid the patch: {names:?}"
        );
        assert!(
            !names.contains(&"GLASS KEYS".to_string()),
            "the search did not hide the preset: {names:?}"
        );

        // A search that finds nothing at all.
        editor.state.patch_search = "nothing at all".to_string();
        let output = settle(&ctx, &mut editor);
        let names: Vec<String> = texts(&output)
            .into_iter()
            .filter(|(_, pos)| pos.x < 200.0 && pos.y < 400.0)
            .map(|(text, _)| text)
            .collect();
        assert!(
            names.contains(&NO_MATCHES.to_string()),
            "the browser has no caption: {names:?}"
        );
        assert!(
            !names.contains(&"GLASS KEYS".to_string()),
            "the search did not hide the preset: {names:?}"
        );
        assert!(
            !names.contains(&"MY PATCH".to_string()),
            "the search did not hide the patch: {names:?}"
        );
    }

    #[test]
    fn a_preset_of_the_factory_can_be_loaded_from_the_browser() {
        let (ctx, mut editor, params) = make_editor();
        editor.state.patch_active = Some("MY PATCH".to_string());
        editor.state.patch_open = true;

        let output = settle(&ctx, &mut editor);
        let (_, pos) = texts(&output)
            .into_iter()
            .find(|(text, _)| text == "GLASS KEYS")
            .expect("the browser has no preset");
        let preset = pos + egui::vec2(2.0, 2.0);

        run_frames(&ctx, &mut editor, click(preset));

        // The preset was applied and it replaced the patch of the user.
        assert_eq!(editor.state.preset_index, 1);
        assert!(editor.state.patch_active.is_none());
        assert!(!editor.state.dirty);
        assert_eq!(params.cutoff.value(), 4_200.0);

        // The browser stays open.
        assert!(editor.state.patch_open);
    }

    #[test]
    fn a_patch_can_be_loaded_by_its_name() {
        let (_ctx, mut editor, params) = make_editor();
        let dir = TempDir::new("load");
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

        // The sound is changed and the patch is loaded by its name.
        setter.set_parameter(&params.cutoff, 500.0);
        editor.state.patch_name = "MY PATCH".to_string();
        {
            let mut ctx = make_ctx(&mut editor, &params, &setter);
            ctx.load_patch_by_name();
        }

        assert!((params.cutoff.value() - 1_234.0).abs() < 1.0);
        assert_eq!(editor.state.patch_status, "Loaded MY PATCH");
        assert_eq!(editor.state.patch_active.as_deref(), Some("MY PATCH"));
        assert!(!editor.state.dirty);

        // A name that is not in the folder is reported.
        editor.state.patch_name = "NOT THERE".to_string();
        {
            let mut ctx = make_ctx(&mut editor, &params, &setter);
            ctx.load_patch_by_name();
        }
        assert_eq!(editor.state.patch_status, "Not found");
        assert!(editor.state.patch_status_error);
    }

    #[test]
    fn the_load_button_of_the_browser_loads_the_patch_of_the_name() {
        let (ctx, mut editor, params) = make_editor();
        let dir = TempDir::new("load-button");
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

        // The sound is changed and the patch is loaded with the button.
        setter.set_parameter(&params.cutoff, 500.0);
        editor.state.patch_name = "MY PATCH".to_string();
        editor.state.patch_open = true;

        let output = settle(&ctx, &mut editor);
        let (_, pos) = texts(&output)
            .into_iter()
            .find(|(text, _)| text == "LOAD")
            .expect("the browser has no load button");
        run_frames(&ctx, &mut editor, click(pos + egui::vec2(2.0, 2.0)));

        assert!((params.cutoff.value() - 1_234.0).abs() < 1.0);
        assert_eq!(editor.state.patch_active.as_deref(), Some("MY PATCH"));
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
    fn the_browser_previews_the_current_sound_as_json() {
        let (_ctx, mut editor, params) = make_editor();
        let gui_ctx = editor.gui_ctx.clone().expect("the editor has no host");
        let setter = gui_ctx.param_setter();
        {
            let mut ctx = make_ctx(&mut editor, &params, &setter);
            ctx.toggle_patch_browser();
        }

        assert!(editor.state.patch_open);
        assert!(
            editor
                .state
                .patch_preview
                .contains("\"format\": \"sushi-patch\""),
            "the preview is not a patch document: {}",
            editor.state.patch_preview
        );
        assert!(
            editor.state.patch_preview.contains("\"cutoff\""),
            "the preview has no parameters: {}",
            editor.state.patch_preview
        );
        assert_eq!(editor.state.patch_preview_title, "01 INIT PAD");
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
            .filter(|(text, pos)| text == "MY PATCH" && pos.x < 200.0 && pos.y < 400.0)
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
    fn the_sections_of_the_panel_have_the_blueprint_layout() {
        let (ctx, mut editor, _params) = make_editor();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);

        // The title of every section and the place where it has to be: the
        // left edge of its section and the top of its row.
        let expected = [
            ("VCO 1", 12.0, 64.0),
            ("VCO 2", 264.0, 64.0),
            ("PULSE WIDTH", 468.0, 64.0),
            ("SOURCE MIXER", 604.0, 64.0),
            ("VCF", 12.0, 195.0),
            ("MOD ENV", 372.0, 195.0),
            ("AMP ENV", 668.0, 195.0),
            ("LFO 1", 12.0, 326.0),
            ("LFO 2", 234.0, 326.0),
            ("ARP", 456.0, 326.0),
            ("VOICE", 768.0, 326.0),
            ("FX", 12.0, 457.0),
            ("MACROS", 576.0, 457.0),
            ("VCA", 804.0, 457.0),
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
        assert_the_widgets_fit(&ctx, &output, panel);
    }

    /// Checks that the widgets of the panel stay inside of the panel and that
    /// they never touch each other.
    fn assert_the_widgets_fit(ctx: &egui::Context, output: &egui::FullOutput, panel: Rect) {
        // Every slider is inside of the panel and touches no other slider.
        let sliders = slider_rects(ctx, output);
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

    /// The sizes of the windows in which the panel is drawn.
    const SIZES: [(f32, f32); 5] = [
        (960.0, 640.0),
        (1280.0, 800.0),
        (1600.0, 900.0),
        (2000.0, 1000.0),
        (1024.0, 1122.0),
    ];

    #[test]
    fn the_panel_needs_the_minimum_size() {
        // The panel is drawn in the size it was made for.
        let default = Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            egui::vec2(WINDOW_WIDTH, WINDOW_HEIGHT),
        );
        assert!(Layout::for_rect(default).is_some());

        // A window that is too small has no layout.
        for (width, height) in [
            (WINDOW_WIDTH - 1.0, WINDOW_HEIGHT),
            (WINDOW_WIDTH, WINDOW_HEIGHT - 1.0),
            (WINDOW_WIDTH - 1.0, WINDOW_HEIGHT - 1.0),
            (800.0, 600.0),
            (300.0, 200.0),
            (0.0, 0.0),
        ] {
            let rect = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(width, height));
            assert!(
                Layout::for_rect(rect).is_none(),
                "the window of {width} x {height} has a layout"
            );
        }
    }

    #[test]
    fn the_panel_follows_the_window_continuously() {
        // The size of the window, the zoom of the panel and the size of the
        // panel that goes with it.
        let cases = [
            (960.0, 640.0, 1.0, 960.0, 600.0),
            (1280.0, 800.0, 1280.0 / 960.0, 1280.0, 800.0),
            (1600.0, 900.0, 900.0 / 600.0, 1440.0, 900.0),
            (2000.0, 1000.0, 1000.0 / 600.0, 1600.0, 1000.0),
            (1024.0, 1122.0, 1024.0 / 960.0, 1024.0, 640.0),
            (1280.0, 701.0, 701.0 / 600.0, 1121.6, 701.0),
        ];

        for (width, height, zoom, panel_width, panel_height) in cases {
            let rect = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(width, height));
            let layout = Layout::for_rect(rect).expect("the window is big enough");

            assert!(
                (layout.zoom - zoom).abs() < 0.001,
                "the window of {width} x {height} has the zoom {} instead of {zoom}",
                layout.zoom
            );
            assert!(
                (layout.panel.width() - panel_width).abs() < 0.01,
                "the panel of the window of {width} x {height} has the wrong width: {:?}",
                layout.panel
            );
            assert!(
                (layout.panel.height() - panel_height).abs() < 0.01,
                "the panel of the window of {width} x {height} has the wrong height: {:?}",
                layout.panel
            );
            // The panel sits in the middle of the window.
            assert!((layout.panel.center().x - rect.center().x).abs() < 0.01);
            assert!((layout.panel.center().y - rect.center().y).abs() < 0.01);
            // The panel keeps the proportions of the design.
            let ratio = layout.panel.width() / layout.panel.height();
            assert!((ratio - WINDOW_WIDTH / WINDOW_HEIGHT).abs() < 0.001);
        }

        // A taller window of the same width leaves room above and below.
        let rect = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1280.0, 1000.0));
        let layout = Layout::for_rect(rect).expect("the window is big enough");
        assert!((layout.panel.left() - 0.0).abs() < 0.01);
        assert!((layout.panel.top() - 100.0).abs() < 0.01);
    }

    #[test]
    fn the_layout_keeps_the_design_of_the_panel() {
        for (width, height) in SIZES {
            let rect = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(width, height));
            let layout = Layout::for_rect(rect).expect("the window is big enough");
            let zoom = layout.zoom;

            // The header sits at the top of the panel and grows with the zoom.
            let header = layout.header();
            assert!(
                (header.left() - (layout.panel.left() + MARGIN * zoom)).abs() < 0.01,
                "the header is at the wrong place: {header:?}"
            );
            assert!(
                (header.top() - (layout.panel.top() + MARGIN * zoom)).abs() < 0.01,
                "the header is at the wrong place: {header:?}"
            );
            assert!((header.height() - HEADER_HEIGHT * zoom).abs() < 0.01);

            // The rows fill the rest of the panel.
            let first = layout.group(0, 0);
            assert!(
                (first.top() - (layout.panel.top() + (MARGIN + HEADER_HEIGHT + BAND_GAP) * zoom))
                    .abs()
                    < 0.01,
                "the first row is at the wrong place: {first:?}"
            );
            let last = layout.group(3, 0);
            assert!(
                (last.bottom() - (layout.panel.bottom() - FOOTER_HEIGHT * zoom)).abs() < 0.01,
                "the last row does not fill the panel: {last:?}"
            );

            // The panel is never bigger than the window.
            assert!(layout.panel.width() <= width + 0.01);
            assert!(layout.panel.height() <= height + 0.01);
        }
    }

    #[test]
    fn the_sections_of_the_layout_do_not_overlap() {
        for (width, height) in SIZES {
            let rect = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(width, height));
            let layout = Layout::for_rect(rect).expect("the window is big enough");

            for row in 0..4 {
                let count = Layout::group_count(row);
                for index in 0..count {
                    let group = layout.group(row, index);
                    assert!(
                        layout.panel.contains_rect(group),
                        "a section is outside of the panel: {group:?}"
                    );
                    assert!(
                        group.width() > 0.0 && group.height() > 0.0,
                        "a section has no size: {group:?}"
                    );
                    // The sections of a row touch no other section.
                    for other in index + 1..count {
                        let other = layout.group(row, other);
                        assert!(
                            !group.intersects(other),
                            "two sections touch each other: {group:?} {other:?}"
                        );
                    }
                }

                // The sections of a row fill the whole row.
                let last = layout.group(row, count - 1);
                assert!(
                    (last.right() - (layout.panel.right() - MARGIN * layout.zoom)).abs() < 0.01,
                    "the sections of the row {row} do not fill the panel: {last:?}"
                );
            }
        }
    }

    #[test]
    fn the_fonts_scale_with_the_zoom() {
        // The size of a font is the size of the design times the zoom.
        for percent in (100..=400).step_by(5) {
            let zoom = percent as f32 / 100.0;
            for size in [8.0, 9.0, 10.0, 12.0, 14.0] {
                assert!(
                    (font_size(size, zoom) - size * zoom).abs() < 0.0001,
                    "the font of {size} does not scale with the zoom {zoom}"
                );
            }
        }
        assert!((font_size(10.0, 1.25) - 12.5).abs() < 0.0001);
        assert!((font_size(10.0, 1.0) - 10.0).abs() < 0.0001);
    }

    #[test]
    fn the_texts_of_the_panel_grow_with_the_zoom() {
        let (ctx, mut editor, _params) = make_editor();

        // The wordmark of the header at the zoom of 100 %.
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let (_, _, small) = text_shapes(&output)
            .into_iter()
            .find(|(text, _, _)| text == "SUSHI")
            .expect("the header has no wordmark");

        // The same wordmark at the zoom of 125 %.
        let output = run_frames(
            &ctx,
            &mut editor,
            vec![input_at(egui::vec2(1280.0, 800.0), vec![])],
        );
        let (_, _, big) = text_shapes(&output)
            .into_iter()
            .find(|(text, _, _)| text == "SUSHI")
            .expect("the header has no wordmark");

        // The pixel font is snapped to whole pixels, so the growth is close
        // to the zoom instead of exactly proportional.
        assert!(
            big.x > small.x && big.y > small.y,
            "the wordmark did not grow with the zoom: {small:?} {big:?}"
        );
        let ratio = big.y / small.y;
        assert!(
            (1.05..1.6).contains(&ratio),
            "the wordmark grew by {ratio} instead of about 1.25: {small:?} {big:?}"
        );
    }

    #[test]
    fn a_small_window_shows_a_sad_face() {
        let (ctx, mut editor, _params) = make_editor();
        let size = egui::vec2(300.0, 200.0);
        let output = run_frames(&ctx, &mut editor, vec![input_at(size, vec![])]);

        let names: Vec<String> = texts(&output).into_iter().map(|(t, _)| t).collect();
        assert!(
            names.contains(&":(".to_string()),
            "the panel has no sad face: {names:?}"
        );
        assert!(
            names.contains(&"MINIMUM 960 x 600".to_string()),
            "the panel has no hint: {names:?}"
        );

        // The panel itself is not drawn at all.
        let counts = editor.state.counts;
        assert_eq!(counts.sliders, 0, "the panel has sliders");
        assert_eq!(counts.popups, 0, "the panel has popups");
        assert_eq!(counts.checks, 0, "the panel has check boxes");
        assert_eq!(counts.radios, 0, "the panel has radio buttons");

        // The face and the hint stay inside of the window.
        let panel = Rect::from_min_size(egui::pos2(0.0, 0.0), size);
        for shape in &output.shapes {
            if let egui::epaint::Shape::Text(text) = &shape.shape {
                let rect = Rect::from_min_size(text.pos, text.galley.size());
                assert!(
                    panel.contains_rect(rect),
                    "the text {:?} is outside of the window: {rect:?}",
                    text.galley.job.text
                );
            }
        }
    }

    #[test]
    fn a_tiny_window_shows_a_sad_face() {
        let (ctx, mut editor, _params) = make_editor();
        let size = egui::vec2(120.0, 80.0);
        let output = run_frames(&ctx, &mut editor, vec![input_at(size, vec![])]);

        let names: Vec<String> = texts(&output).into_iter().map(|(t, _)| t).collect();
        assert!(
            names.contains(&":(".to_string()),
            "the panel has no sad face: {names:?}"
        );
    }

    #[test]
    fn the_panel_grows_with_the_window() {
        let (ctx, mut editor, _params) = make_editor();

        // The default size.
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let small = slider_rects(&ctx, &output);
        assert_eq!(small.len(), 56, "unexpected number of sliders");

        // A bigger window: the sliders are taller and the sections are
        // wider.
        let output = run_frames(
            &ctx,
            &mut editor,
            vec![input_at(egui::vec2(1600.0, 900.0), vec![])],
        );
        let big = slider_rects(&ctx, &output);
        assert_eq!(big.len(), 56, "unexpected number of sliders");

        for (small, big) in small.iter().zip(big.iter()) {
            assert!(
                big.height() > small.height(),
                "a slider did not grow: {small:?} {big:?}"
            );
        }

        // The last slider of the panel moved to the right, so that the panel
        // uses the whole width of the window.
        assert!(
            big[big.len() - 1].right() > small[small.len() - 1].right(),
            "the panel did not grow to the right"
        );
    }

    #[test]
    fn the_panel_fits_into_a_huge_window() {
        // The biggest zoom level needs a window of 3840 x 2560.
        let (ctx, mut editor, _params) = make_editor();
        let output = run_frames(
            &ctx,
            &mut editor,
            vec![input_at(egui::vec2(3840.0, 2560.0), vec![])],
        );
        let panel = Layout::for_rect(Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            egui::vec2(3840.0, 2560.0),
        ))
        .expect("the window is big enough")
        .panel;
        assert!((panel.width() - 3840.0).abs() < 0.01);
        assert!((panel.height() - 2400.0).abs() < 0.01);
        assert_the_widgets_fit(&ctx, &output, panel);
    }

    #[test]
    fn the_widgets_fit_into_every_window() {
        for (width, height) in SIZES {
            let (ctx, mut editor, _params) = make_editor();
            let output = run_frames(
                &ctx,
                &mut editor,
                vec![input_at(egui::vec2(width, height), vec![])],
            );
            // The panel sits in the middle of the window, so that is where
            // the widgets have to stay.
            let panel = Layout::for_rect(Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(width, height),
            ))
            .expect("the window is big enough")
            .panel;
            assert_the_widgets_fit(&ctx, &output, panel);
        }
    }
}
