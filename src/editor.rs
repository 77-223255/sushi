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
mod theme;
mod undo;

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

use crate::dsp::mod_feedback::ModFeedback;
use crate::params::{ModDest, ModSource, SushiParams};
use crate::patch;
use presets::{PRESETS, Rng};
use theme::palette;
use undo::UndoHistory;

// --- The fonts ---------------------------------------------------------------

/// The pixel font of the panel, in its regular weight.
const SILKSCREEN_REGULAR: &[u8] = include_bytes!("../assets/fonts/Silkscreen-Regular.ttf");
/// The pixel font of the panel, in its bold weight.
const SILKSCREEN_BOLD: &[u8] = include_bytes!("../assets/fonts/Silkscreen-Bold.ttf");
/// The name of the font family that holds the bold pixel font.
const BOLD_FAMILY: &str = "sushi-bold";

/// Common system CJK fonts and the face index to use inside a collection.
/// They are loaded as a fallback so Chinese patch names and folder paths do
/// not turn into boxes. The file is not bundled with the plugin.
const CJK_FONT_PATHS: &[(&str, u32)] = &[
    ("/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc", 2),
    ("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", 2),
    ("/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc", 2),
    ("/usr/share/fonts/wenquanyi/wqy-zenhei/wqy-zenhei.ttc", 0),
    ("/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc", 0),
    ("/System/Library/Fonts/PingFang.ttc", 0),
    ("/System/Library/Fonts/STHeiti Light.ttc", 0),
    ("/Library/Fonts/Arial Unicode.ttf", 0),
    (r"C:\Windows\Fonts\msyh.ttc", 0),
    (r"C:\Windows\Fonts\msyh.ttf", 0),
    (r"C:\Windows\Fonts\simhei.ttf", 0),
    (r"C:\Windows\Fonts\simsun.ttc", 0),
];
/// A normal monospace family for JSON and code previews. Silkscreen is kept
/// for the faceplate only, because it has no true lowercase.
const CODE_FAMILY: &str = "sushi-code";

/// Installs the pixel fonts of the panel. The regular weight becomes the
/// default of the proportional and of the monospaced families; the bold
/// weight gets its own family for the titles. The default egui fonts stay in
/// the list as a fallback for glyphs that Silkscreen does not have.
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let mut code_fonts = fonts
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

    // Add a system CJK fallback after the pixel font. The file stays on the
    // user's system; the plugin only reads it at runtime.
    let mut cjk_loaded = false;
    for (path, index) in CJK_FONT_PATHS {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let data = egui::FontData::from_blob(Arc::new(bytes), *index);
        fonts
            .font_data
            .insert("cjk-fallback".to_owned(), Arc::new(data));
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts
                .families
                .entry(family)
                .or_default()
                .push("cjk-fallback".to_owned());
        }
        cjk_loaded = true;
        break;
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
    if cjk_loaded {
        code_fonts.push("cjk-fallback".to_owned());
    }
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

// --- The modulation table ---------------------------------------------------

/// The height of the single header of the modulation table, in the design
/// size of the panel.
const MOD_HEADER_HEIGHT: f32 = 22.0;
/// The height of one routing row of the modulation table.
const MOD_ROW_HEIGHT: f32 = 28.0;
/// The height of the source and destination fields. It is the click target of
/// the popups as well.
const MOD_POPUP_HEIGHT: f32 = 24.0;
/// The empty margin around the columns of the modulation table.
const MOD_TABLE_PADDING: f32 = 6.0;
/// The width of the column that shows the number of a route.
const MOD_INDEX_WIDTH: f32 = 18.0;
/// The width of the source column.
const MOD_SOURCE_WIDTH: f32 = 128.0;
/// The width of the arrow between the source and the destination.
const MOD_ARROW_WIDTH: f32 = 16.0;
/// The width of the destination column.
const MOD_DEST_WIDTH: f32 = 128.0;
/// The width of the numeric amount readout.
const MOD_VALUE_WIDTH: f32 = 44.0;
/// The gap between two columns of the modulation table.
const MOD_COLUMN_GAP: f32 = 4.0;
/// The height of the visible track of an amount bar.
const MOD_BAR_HEIGHT: f32 = 16.0;
/// The width of the black handle of an amount bar.
const MOD_BAR_HANDLE_WIDTH: f32 = 7.0;
/// How far the handle of an amount bar stays away from the ends of its track.
const MOD_BAR_INSET: f32 = 2.0;
/// The width of the live marker that shows the current modulation of a route.
const MOD_LIVE_MARKER_WIDTH: f32 = 2.0;

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
/// The width of a page button (MOD / FX) in the header.
const PAGE_BUTTON_WIDTH: f32 = 44.0;
/// The longest name that the display of the header shows.
const LCD_NAME_LENGTH: usize = 20;
/// The largest font of the preset display, in the design size. Long names use
/// a smaller font instead of running into the border of the display.
const LCD_FONT_SIZE: f32 = 14.0;
/// The smallest font of the preset display before a name gets an ellipsis.
const LCD_MIN_FONT_SIZE: f32 = 10.0;
/// The empty margin at both ends of the preset display.
const LCD_PADDING: f32 = 7.0;
/// The mark that the preset display had to cut a name off.
const LCD_ELLIPSIS: &str = "...";
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

// --- The footer controls ----------------------------------------------------

/// The width of one UNDO or REDO button in the footer.
const FOOTER_BUTTON_WIDTH: f32 = 40.0;
/// The height of one UNDO or REDO button in the footer.
const FOOTER_BUTTON_HEIGHT: f32 = 14.0;

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

/// The part of the editor state that is stored with the plugin state by the
/// host. Comparing it with the last value lets the open editor notice a
/// project reload or an undo performed by the host.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct PersistedEditorState {
    preset_index: usize,
    patch_name: Option<String>,
    patch_path: Option<String>,
    dirty: bool,
}

/// The page that replaces the two upper bands of the panel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Page {
    /// The normal sound page.
    #[default]
    Main,
    /// The MOD 2.0 routing table.
    Mod,
    /// The FX overview page.
    Fx,
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
    /// The page that is shown in the panel.
    page: Page,
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
    /// The places of the amount bars of the modulation page of the last frame.
    #[cfg(test)]
    amount_rects: Vec<egui::Rect>,
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
    /// The actual file of the loaded patch. The JSON name and the file stem
    /// may differ after a manual edit or a rename, so the path is the
    /// authoritative identity.
    patch_active_path: Option<PathBuf>,
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
    /// The undo and redo steps of the editor. They are GUI-only: the audio
    /// thread never touches this history.
    undo_history: UndoHistory,
    gui_ctx: Option<GuiContext>,
    state: EditorState,
    /// The persisted identity that was last seen by this editor.
    last_persisted: PersistedEditorState,
    /// The zoom of the panel, as it was last applied to the styles.
    zoom: f32,
    /// The number of arpeggiator triggers.
    arp_led: Arc<AtomicUsize>,
    /// The position of the arpeggiator in its pattern.
    arp_position: Arc<AtomicUsize>,
    /// The number of voices that are currently playing.
    voice_count: Arc<AtomicUsize>,
    /// The last values of the global modulation sources.
    mod_feedback: Arc<ModFeedback>,
}

impl SushiEditor {
    /// Creates a new editor for the given parameters.
    pub fn new(
        params: Arc<SushiParams>,
        scope_rx: rtrb::Consumer<f32>,
        arp_led: Arc<AtomicUsize>,
        arp_position: Arc<AtomicUsize>,
        voice_count: Arc<AtomicUsize>,
        mod_feedback: Arc<ModFeedback>,
    ) -> Self {
        let mut editor = Self {
            params,
            scope_rx,
            history: VecDeque::with_capacity(HISTORY_LEN),
            undo_history: UndoHistory::default(),
            gui_ctx: None,
            last_persisted: PersistedEditorState::default(),
            zoom: 1.0,
            state: EditorState {
                texts: HashMap::new(),
                gestures: HashSet::new(),
                preset_index: 0,
                dirty: false,
                rng: Rng::new(0x2545_F491),
                arp_counter: 0,
                arp_step: 0,
                page: Page::Main,
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
                #[cfg(test)]
                amount_rects: Vec::new(),
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
                patch_active_path: None,
                patch_preview: String::new(),
                patch_preview_title: String::new(),
            },
            arp_led,
            arp_position,
            voice_count,
            mod_feedback,
        };
        editor.init_patch_state();
        editor.last_persisted = editor.read_persisted_state();
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

        // Restore the display state that the host saved with the project.
        // The sound itself lives in the parameters; this is only the patch
        // identity shown by the LCD and used by the browser.
        let index = *self
            .params
            .preset_index
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        self.state.preset_index = index.min(PRESETS.len() - 1);
        self.state.dirty = *self
            .params
            .patch_dirty
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        self.state.patch_active_path = self
            .params
            .patch_path
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
            .map(PathBuf::from);
        self.state.patch_active = self
            .params
            .patch_name
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
            .or_else(|| {
                self.state
                    .patch_active_path
                    .as_ref()
                    .and_then(|path| path.file_stem())
                    .map(|stem| stem.to_string_lossy().into_owned())
            });
        self.state.patch_name = self
            .state
            .patch_active_path
            .as_ref()
            .and_then(|path| path.file_stem())
            .map(|stem| stem.to_string_lossy().into_owned())
            .or_else(|| self.state.patch_active.clone())
            .unwrap_or_else(|| PRESETS[self.state.preset_index].name.to_string());
        self.state.patch_files = patch::list_patches(&self.state.patch_config.patch_dir);
        self.state.patch_status.clear();
        self.state.patch_status_error = false;
        self.state.patch_confirm_overwrite = false;
        self.state.patch_preview_title = match &self.state.patch_active {
            Some(name) => name.clone(),
            None => format!(
                "{:02} {}",
                self.state.preset_index + 1,
                PRESETS[self.state.preset_index].name
            ),
        };
        self.state.patch_preview = serde_json::to_string_pretty(&patch::to_json(
            &self.params,
            &self.state.patch_preview_title,
        ))
        .unwrap_or_default();
    }

    /// Reads the identity that the host has stored for this instance.
    fn read_persisted_state(&self) -> PersistedEditorState {
        PersistedEditorState {
            preset_index: (*self
                .params
                .preset_index
                .lock()
                .unwrap_or_else(|error| error.into_inner()))
            .min(PRESETS.len() - 1),
            patch_name: self
                .params
                .patch_name
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone(),
            patch_path: self
                .params
                .patch_path
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone(),
            dirty: *self
                .params
                .patch_dirty
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
        }
    }

    /// Applies a host state change (project reload, undo, another editor) to
    /// the open editor without touching the patch folder or the search.
    fn apply_persisted_state(&mut self, persisted: PersistedEditorState) {
        // The sound underneath the history changed (project reload, host
        // undo or another editor), so the old snapshots do not belong to it
        // anymore.
        self.undo_history.clear();
        self.state.preset_index = persisted.preset_index.min(PRESETS.len() - 1);
        self.state.dirty = persisted.dirty;
        self.state.patch_confirm_overwrite = false;
        self.state.patch_active_path = persisted.patch_path.map(PathBuf::from);
        self.state.patch_active = persisted.patch_name.or_else(|| {
            self.state
                .patch_active_path
                .as_ref()
                .and_then(|path| path.file_stem())
                .map(|stem| stem.to_string_lossy().into_owned())
        });
        self.state.patch_name = self
            .state
            .patch_active_path
            .as_ref()
            .and_then(|path| path.file_stem())
            .map(|stem| stem.to_string_lossy().into_owned())
            .or_else(|| self.state.patch_active.clone())
            .unwrap_or_else(|| PRESETS[self.state.preset_index].name.to_string());
        self.state.patch_preview_title = match &self.state.patch_active {
            Some(name) => name.clone(),
            None => format!(
                "{:02} {}",
                self.state.preset_index + 1,
                PRESETS[self.state.preset_index].name
            ),
        };
        self.state.patch_preview = serde_json::to_string_pretty(&patch::to_json(
            &self.params,
            &self.state.patch_preview_title,
        ))
        .unwrap_or_default();
    }

    /// Writes the patch identity and the dirty flag into the persistent
    /// fields so the host can restore the LCD after a project reload.
    fn sync_persisted_state(&self) {
        *self
            .params
            .preset_index
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = self.state.preset_index;
        *self
            .params
            .patch_name
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = self.state.patch_active.clone();
        *self
            .params
            .patch_path
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = self
            .state
            .patch_active_path
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned());
        *self
            .params
            .patch_dirty
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = self.state.dirty;
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

    /// Finishes an edit after the widgets are drawn. While a gesture is open
    /// its snapshot stays pending, so a whole drag becomes one undo step.
    fn finish_edit(&mut self) {
        if self.state.gestures.is_empty() {
            self.undo_history.commit(&self.params, &self.state);
        }
    }

    /// Restores the state before the last sound edit.
    fn undo(&mut self, setter: &ParamSetter) -> bool {
        self.finish_edit();
        let changed = self
            .undo_history
            .undo(&self.params, setter, &mut self.state);
        if changed {
            self.last_persisted = self.read_persisted_state();
        }
        changed
    }

    /// Restores the state that was undone last.
    fn redo(&mut self, setter: &ParamSetter) -> bool {
        self.finish_edit();
        let changed = self
            .undo_history
            .redo(&self.params, setter, &mut self.state);
        if changed {
            self.last_persisted = self.read_persisted_state();
        }
        changed
    }

    /// Handles Ctrl/Cmd+Z, Ctrl/Cmd+Shift+Z and Ctrl+Y. The keys are left to
    /// a focused text field, so that typing and the undo of a text edit keep
    /// working there.
    fn handle_history_shortcuts(&mut self, ui: &egui::Ui, setter: &ParamSetter) {
        if ui.ctx().text_edit_focused() {
            return;
        }

        let (undo, redo) = ui.input_mut(|input| {
            // The shift variant is checked first, because a pattern without
            // Shift also matches a press that has Shift held down. Both the
            // command and the plain Ctrl spelling are accepted, so the
            // shortcut works the same on every host and in the tests.
            let shift = egui::Modifiers::SHIFT;
            let redo = input.consume_key(egui::Modifiers::COMMAND.plus(shift), egui::Key::Z)
                || input.consume_key(egui::Modifiers::CTRL.plus(shift), egui::Key::Z)
                || input.consume_key(egui::Modifiers::MAC_CMD.plus(shift), egui::Key::Z)
                || input.consume_key(egui::Modifiers::CTRL, egui::Key::Y)
                || input.consume_key(egui::Modifiers::COMMAND, egui::Key::Y)
                || input.consume_key(egui::Modifiers::MAC_CMD, egui::Key::Y);
            let undo = input.consume_key(egui::Modifiers::COMMAND, egui::Key::Z)
                || input.consume_key(egui::Modifiers::CTRL, egui::Key::Z)
                || input.consume_key(egui::Modifiers::MAC_CMD, egui::Key::Z);
            (undo, redo)
        });

        if redo {
            self.redo(setter);
        } else if undo {
            self.undo(setter);
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
        self.undo_history.clear();
        self.init_patch_state();
        self.last_persisted = self.read_persisted_state();

        install_fonts(&egui_ctx);
        apply_theme(&egui_ctx, 1.0);
        Ok(())
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut nice_plug_egui::Frame) {
        let persisted = self.read_persisted_state();
        if persisted != self.last_persisted {
            self.apply_persisted_state(persisted.clone());
            self.last_persisted = persisted;
        }

        self.draw(ui);
        self.sync_persisted_state();
        self.last_persisted = self.read_persisted_state();
    }
}

impl SushiEditor {
    /// Draws the whole panel.
    fn draw(&mut self, ui: &mut egui::Ui) {
        let Some(gui_ctx) = self.gui_ctx.clone() else {
            return;
        };
        #[cfg(test)]
        {
            // The geometry snapshots belong to one frame. Clearing them here
            // also covers a page switch, where the old page would otherwise
            // leave its rectangles behind.
            self.state.popup_rects.clear();
            self.state.amount_rects.clear();
        }
        let setter = gui_ctx.param_setter();

        self.handle_history_shortcuts(ui, &setter);
        self.collect_scope();
        self.update_arp_position();
        ui.ctx().request_repaint_after(Duration::from_millis(16));

        let full = ui.max_rect();
        ui.painter()
            .rect_filled(full, CornerRadius::ZERO, palette().bg);

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

        let footer_action = {
            // Split the borrow of the editor, so that the parameters, the
            // state of the panel, the text buffers and the undo history can
            // be used at the same time.
            let params = self.params.as_ref();
            let history = &self.history;
            let voice_count = &self.voice_count;
            let mod_feedback = self.mod_feedback.as_ref();
            let mut ctx = Ctx {
                params,
                setter: &setter,
                state: &mut self.state,
                undo: &mut self.undo_history,
                history,
                voice_count,
                mod_feedback,
                slider_width: layout.slider_width(),
                zoom: layout.zoom,
            };

            if ctx.state.patch_open {
                ctx.patch_page(ui);
                None
            } else {
                let header_rect = layout.header();
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .id_salt("sushi-header")
                        .max_rect(header_rect)
                        .layout(EguiLayout::top_down(Align::Min)),
                    |ui| header(ui, header_rect, &mut ctx),
                );

                match ctx.state.page {
                    Page::Main => {
                        row_a(ui, &layout, &mut ctx);
                        row_b(ui, &layout, &mut ctx);
                        row_c(ui, &layout, &mut ctx);
                        row_d(ui, &layout, &mut ctx);
                    }
                    Page::Mod => {
                        row_mod(ui, &layout, &mut ctx);
                        row_c(ui, &layout, &mut ctx);
                        row_d(ui, &layout, &mut ctx);
                    }
                    Page::Fx => row_fx(ui, &layout, &mut ctx),
                }
                draw_footer(
                    ui,
                    &layout,
                    ctx.state,
                    ctx.undo.can_undo(),
                    ctx.undo.can_redo(),
                )
            }
        };

        // A drag may still be open, in which case its snapshot waits for the
        // next frame. Everything else is committed here, once per edit.
        self.finish_edit();

        match footer_action {
            Some(FooterAction::Undo) => {
                self.undo(&setter);
            }
            Some(FooterAction::Redo) => {
                self.redo(&setter);
            }
            None => {}
        }
    }
}

/// The command that a button of the footer asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FooterAction {
    Undo,
    Redo,
}

/// The places of the two history buttons of the footer.
fn footer_button_rects(panel: Rect, zoom: f32) -> (Rect, Rect) {
    let undo = Rect::from_min_size(
        egui::pos2(
            panel.left() + 118.0 * zoom,
            panel.bottom() - (FOOTER_HEIGHT - 3.0) * zoom,
        ),
        egui::vec2(FOOTER_BUTTON_WIDTH, FOOTER_BUTTON_HEIGHT) * zoom,
    );
    let redo = Rect::from_min_size(
        egui::pos2(undo.right() + 4.0 * zoom, undo.top()),
        undo.size(),
    );
    (undo, redo)
}

/// A small flat button of the footer. The disabled state is a pale fill and a
/// muted label instead of a click target that silently does nothing.
fn footer_button(
    ui: &mut egui::Ui,
    rect: Rect,
    text: &str,
    enabled: bool,
    zoom: f32,
) -> egui::Response {
    let response = ui.interact(
        rect,
        ui.scope_id().with(("sushi-footer", text)),
        Sense::click(),
    );
    let painter = ui.painter_at(rect);
    let pressed = enabled && response.is_pointer_button_down_on();
    let fill = if !enabled {
        palette().track
    } else if pressed {
        palette().button_active
    } else {
        palette().button_bg
    };
    flat_box(&painter, rect, fill);
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::new(font_size(8.0, zoom), bold()),
        if enabled {
            palette().text
        } else {
            palette().muted
        },
    );

    let hint = if enabled {
        if text == "UNDO" {
            "Undo (Ctrl/Cmd+Z)"
        } else {
            "Redo (Ctrl/Cmd+Shift+Z)"
        }
    } else if text == "UNDO" {
        "Nothing to undo"
    } else {
        "Nothing to redo"
    };
    response.on_hover_text(hint)
}

/// The footer silkscreen: the model name, the two history buttons and the two
/// shortcuts that are not visible on the faceplate.
fn draw_footer(
    ui: &mut egui::Ui,
    layout: &Layout,
    state: &EditorState,
    can_undo: bool,
    can_redo: bool,
) -> Option<FooterAction> {
    let painter = ui.painter_at(layout.panel);
    let zoom = layout.zoom;
    let panel = layout.panel;
    let y = panel.bottom() - (FOOTER_HEIGHT - 4.0) * zoom;
    painter.line_segment(
        [
            egui::pos2(panel.left(), panel.bottom() - FOOTER_HEIGHT * zoom),
            egui::pos2(panel.right(), panel.bottom() - FOOTER_HEIGHT * zoom),
        ],
        Stroke::new(1.0, palette().rule),
    );

    painter.text(
        egui::pos2(panel.left() + 12.0 * zoom, y),
        Align2::LEFT_TOP,
        "SUSHI / MODEL 101",
        FontId::new(font_size(8.0, zoom), bold()),
        palette().muted,
    );
    painter.text(
        egui::pos2(panel.right() - 12.0 * zoom, y),
        Align2::RIGHT_TOP,
        "DBL-CLICK = RESET   RIGHT-CLICK = VALUE",
        FontId::new(font_size(8.0, zoom), bold()),
        palette().muted,
    );

    // The two history buttons sit beside the model name, away from the
    // centered readout and the shortcut hints on the right.
    let (undo_rect, redo_rect) = footer_button_rects(panel, zoom);
    let undo = footer_button(ui, undo_rect, "UNDO", can_undo, zoom);
    let redo = footer_button(ui, redo_rect, "REDO", can_redo, zoom);

    // The live parameter readout: the reference's central value box.
    let (label, value) = state
        .readout
        .clone()
        .unwrap_or_else(|| ("PATCH".to_string(), display_text(state)));
    let box_rect = Rect::from_center_size(
        egui::pos2(panel.center().x, panel.bottom() - 10.0 * zoom),
        egui::vec2(300.0, 15.0) * zoom,
    );
    flat_box(&painter, box_rect, palette().paper);
    let label = truncate_name(&label, 22);
    painter.text(
        egui::pos2(box_rect.left() + 7.0 * zoom, box_rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::new(font_size(8.0, zoom), bold()),
        palette().title_blue,
    );
    painter.text(
        egui::pos2(box_rect.right() - 7.0 * zoom, box_rect.center().y),
        Align2::RIGHT_CENTER,
        value,
        FontId::new(font_size(10.0, zoom), bold()),
        palette().text,
    );

    if undo.clicked() && can_undo {
        Some(FooterAction::Undo)
    } else if redo.clicked() && can_redo {
        Some(FooterAction::Redo)
    } else {
        None
    }
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
    let face = painter.layout_no_wrap(":(".to_string(), FontId::proportional(size), palette().text);

    // The line below the face tells the user how big the window has to be.
    // It gets a bit smaller when the window is too narrow for it.
    let hint = format!("MINIMUM {} x {}", WINDOW_WIDTH as i32, WINDOW_HEIGHT as i32);
    let width = (rect.width() - 8.0).max(8.0);
    let galley = painter.layout_no_wrap(
        hint.to_string(),
        FontId::proportional(10.0),
        palette().muted,
    );
    let hint = if galley.size().x > width {
        let size = (10.0 * width / galley.size().x).max(4.0);
        painter.layout_no_wrap(
            hint.to_string(),
            FontId::proportional(size),
            palette().muted,
        )
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
        palette().text,
    );
    painter.galley(
        egui::pos2(rect.center().x - hint_size.x * 0.5, top + face_size.y + gap),
        hint,
        palette().muted,
    );
}

// --- The header -------------------------------------------------------------

/// The places of the widgets of the header.
struct HeaderRects {
    /// The area of the wordmark. The subtitle has to stay inside it.
    logo: Rect,
    /// The button of the previous factory preset.
    previous: Rect,
    /// The display that shows the name of the sound and opens the patches.
    display: Rect,
    /// The button of the next factory preset.
    next: Rect,
    /// The button that mutates the sound.
    mutate: Rect,
    /// The tab that switches the panel to the modulation matrix.
    mod_page: Rect,
    /// The tab that switches the panel to the FX overview.
    fx_page: Rect,
    /// The meter that shows the number of the voices that are playing.
    meter: Rect,
}

/// The places of the widgets of the header.
fn header_rects(rect: Rect, zoom: f32) -> HeaderRects {
    // The wordmark has a fixed place, and its subtitle has to fit into the
    // same area, so the logo stops before the presets.
    let logo = Rect::from_min_size(
        rect.left_top() + egui::vec2(6.0, 0.0) * zoom,
        egui::vec2(140.0, HEADER_HEIGHT) * zoom,
    );

    // The presets. The display is wider than before, because the separate
    // PATCHES button is gone and the LCD itself opens the browser now.
    let preset_x = rect.left() + 152.0 * zoom;
    let button_y = rect.top() + (HEADER_HEIGHT - 28.0) * 0.5 * zoom;
    let previous = Rect::from_min_size(
        egui::pos2(preset_x, button_y),
        egui::vec2(22.0, 28.0) * zoom,
    );
    let display = Rect::from_min_size(
        egui::pos2(preset_x + 26.0 * zoom, button_y),
        egui::vec2(198.0, 28.0) * zoom,
    );
    let next = Rect::from_min_size(
        egui::pos2(display.right() + 4.0 * zoom, button_y),
        egui::vec2(22.0, 28.0) * zoom,
    );
    let mutate = Rect::from_min_size(
        egui::pos2(next.right() + 4.0 * zoom, button_y),
        egui::vec2(62.0, 28.0) * zoom,
    );

    // The two page tabs sit between the patch commands and the meter. They
    // are mutually exclusive and the active one is filled like a pressed
    // tab, without moving any of the neighbouring controls.
    let mod_page = Rect::from_min_size(
        egui::pos2(mutate.right() + 4.0 * zoom, button_y),
        egui::vec2(PAGE_BUTTON_WIDTH, 28.0) * zoom,
    );
    let fx_page = Rect::from_min_size(
        egui::pos2(mod_page.right() + 4.0 * zoom, button_y),
        egui::vec2(PAGE_BUTTON_WIDTH, 28.0) * zoom,
    );

    // The meter that shows how many voices are playing. It sits in the free
    // space between the page tabs and the lights of the activity meter.
    let meter = Rect::from_min_max(
        egui::pos2(fx_page.right() + 8.0 * zoom, rect.top()),
        egui::pos2(rect.right() - 220.0 * zoom, rect.bottom()),
    );

    HeaderRects {
        logo,
        previous,
        display,
        next,
        mutate,
        mod_page,
        fx_page,
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
        Some(name) => format!(
            "{}{dirty}",
            truncate_name(&sanitize_lcd_name(name), LCD_NAME_LENGTH)
        ),
        None => format!(
            "{:02} {}{dirty}",
            state.preset_index + 1,
            sanitize_lcd_name(PRESETS[state.preset_index].name)
        ),
    }
}

/// Replaces the control characters that would break the single-line display
/// with spaces. A hand-edited patch name must not make the LCD multi-line.
fn sanitize_lcd_name(name: &str) -> String {
    name.chars()
        .map(|character| {
            if matches!(character, '\n' | '\r' | '\t') {
                ' '
            } else {
                character
            }
        })
        .collect()
}

/// Cuts a name off, so that it fits into the display of the header. A name
/// that is too long ends in an explicit ellipsis instead of being clipped in
/// the middle of a letter by the border of the display.
fn truncate_name(name: &str, length: usize) -> String {
    if name.chars().count() <= length {
        return name.to_owned();
    }
    if length <= LCD_ELLIPSIS.len() {
        return name.chars().take(length).collect();
    }
    let mut truncated: String = name.chars().take(length - LCD_ELLIPSIS.len()).collect();
    truncated.push_str(LCD_ELLIPSIS);
    truncated
}

/// Draws the header: the wordmark, the presets and the oscilloscope.
fn header(ui: &mut egui::Ui, rect: Rect, ctx: &mut Ctx) {
    let zoom = ctx.zoom;
    let painter = ui.painter_at(rect);

    // The header sits on its own deeper plate, separated by a full-width rule.
    painter.rect_filled(rect, CornerRadius::ZERO, palette().header_bg);
    let rule_y = rect.bottom() - 0.5;
    painter.line_segment(
        [
            egui::pos2(rect.left(), rule_y),
            egui::pos2(rect.right(), rule_y),
        ],
        Stroke::new(1.0, palette().rule),
    );

    // The wordmark is static: the engine is Legacy-only, so there is no
    // Modern/Legacy switch and no mode in the subtitle.
    let rects = header_rects(rect, zoom);
    let wordmark = FontId::new(font_size(20.0, zoom), bold());
    painter.text(
        rects.logo.left_top() + egui::vec2(2.0, 2.0) * zoom,
        Align2::LEFT_TOP,
        "SUSHI",
        wordmark,
        palette().text,
    );
    painter.text(
        rects.logo.left_top() + egui::vec2(2.0, 30.0) * zoom,
        Align2::LEFT_TOP,
        "POLYPHONIC SYNTHESIZER",
        FontId::proportional(font_size(8.0, zoom)),
        palette().muted,
    );

    // The preset display is the patch browser button now. It keeps showing
    // the current name and gets a pointer cursor and a tooltip instead of a
    // separate PATCHES button.
    let display_response = ui.interact(
        rects.display,
        ui.scope_id().with("sushi-lcd"),
        Sense::click(),
    );
    let index = ctx.state.preset_index;
    lcd(&painter, rects.display, &display_text(ctx.state), zoom);
    if display_response.clicked() {
        ctx.toggle_patch_browser();
        ui.ctx().request_repaint();
    }
    display_response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Open the patch browser");

    if button(ui, rects.previous, "<", zoom).clicked() {
        ctx.load_preset(index + PRESETS.len() - 1);
    }
    if button(ui, rects.next, ">", zoom).clicked() {
        ctx.load_preset(index + 1);
    }
    if button(ui, rects.mutate, "MUTATE", zoom).clicked() {
        ctx.mutate();
    }

    // The page tabs. MOD and FX are mutually exclusive, and clicking the
    // active tab returns to the normal sound page.
    if tab_button(ui, rects.mod_page, "MOD", ctx.state.page == Page::Mod, zoom).clicked() {
        ctx.state.page = if ctx.state.page == Page::Mod {
            Page::Main
        } else {
            Page::Mod
        };
    }
    if tab_button(ui, rects.fx_page, "FX", ctx.state.page == Page::Fx, zoom).clicked() {
        ctx.state.page = if ctx.state.page == Page::Fx {
            Page::Main
        } else {
            Page::Fx
        };
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
    led(
        &painter,
        activity,
        palette().led_on,
        ctx.state.activity > 0.002,
        zoom,
    );
    painter.text(
        egui::pos2(activity.x, rect.top() + 28.0 * zoom),
        Align2::CENTER_CENTER,
        "ACT",
        FontId::proportional(font_size(9.0, zoom)),
        palette().muted,
    );

    let clipping = egui::pos2(rect.right() - 172.0 * zoom, rect.top() + 16.0 * zoom);
    led(
        &painter,
        clipping,
        palette().led_red,
        ctx.state.clipping > 0.5,
        zoom,
    );
    painter.text(
        egui::pos2(clipping.x, rect.top() + 28.0 * zoom),
        Align2::CENTER_CENTER,
        "CLIP",
        FontId::proportional(font_size(9.0, zoom)),
        palette().muted,
    );
}

// --- The rows of sections ------------------------------------------------

/// The upper two bands of the modulation page: one routing table over both
/// bands and the two envelopes of the sound in a context column on the right.
///
/// The table has one header and one line per route: number, source,
/// destination, a horizontal bipolar amount bar and a numeric readout. There
/// are no per-slot frames and no repeated captions, so a full page of routes
/// reads as a list instead of eight small forms. The envelopes sit in the two
/// original bands, split on the band line, so the bands below them stay at
/// their exact places.
fn row_mod(ui: &mut egui::Ui, layout: &Layout, ctx: &mut Ctx) {
    let zoom = ctx.zoom;
    let row0 = layout.scale(layout.base_rows[0]);
    let row1 = layout.scale(layout.base_rows[1]);
    // The context column is the width of the original AMP ENV section, so its
    // faders and the envelopes of the normal page share one rhythm.
    let context = layout.group(1, 2);
    let table = Rect::from_min_max(
        row0.left_top(),
        egui::pos2(context.left() - GROUP_GAP * zoom, row1.bottom()),
    );

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

    let painter = ui.painter_at(table);
    let header_bottom = table.top() + MOD_HEADER_HEIGHT * zoom;
    let rows_top = header_bottom;

    // One header: the blue name of the table and a quiet column legend. The
    // legend is a caption, so 8 pt is its floor, not a setting of a value.
    painter.text(
        egui::pos2(
            table.left() + MOD_TABLE_PADDING * zoom,
            (table.top() + header_bottom) * 0.5,
        ),
        Align2::LEFT_CENTER,
        "MODULATION",
        FontId::new(font_size(13.0, zoom), bold()),
        palette().title_blue,
    );
    painter.text(
        egui::pos2(
            table.right() - MOD_TABLE_PADDING * zoom,
            (table.top() + header_bottom) * 0.5,
        ),
        Align2::RIGHT_CENTER,
        "SOURCE / DEST / AMOUNT",
        FontId::new(font_size(8.0, zoom), bold()),
        palette().title_blue,
    );
    painter.line_segment(
        [
            egui::pos2(table.left(), header_bottom),
            egui::pos2(table.right(), header_bottom),
        ],
        Stroke::new(1.0, palette().rule),
    );

    // The column positions. The bar fills the space before the right-aligned
    // value, so its travel grows with the panel instead of shrinking into a
    // caption.
    let left = table.left() + MOD_TABLE_PADDING * zoom;
    let index_x = left;
    let source_x = index_x + (MOD_INDEX_WIDTH + MOD_COLUMN_GAP) * zoom;
    let arrow_x = source_x + (MOD_SOURCE_WIDTH + MOD_COLUMN_GAP) * zoom;
    let dest_x = arrow_x + (MOD_ARROW_WIDTH + MOD_COLUMN_GAP) * zoom;
    let bar_x = dest_x + (MOD_DEST_WIDTH + MOD_COLUMN_GAP) * zoom;
    let value_right = table.right() - MOD_TABLE_PADDING * zoom;
    // The bar takes all the space between the destination and the value
    // column. The panel has a fixed minimum width, so this stays positive at
    // every zoom; the clamp only keeps a malformed layout from producing a
    // negative rectangle.
    let bar_width = (value_right - MOD_VALUE_WIDTH * zoom - bar_x).max(0.0);

    for slot in 0..crate::params::MOD_SLOTS {
        let top = rows_top + slot as f32 * MOD_ROW_HEIGHT * zoom;
        let row_height = MOD_ROW_HEIGHT * zoom;
        let source = sources[slot].value();
        let dest = dests[slot].value();
        let amount = amounts[slot].value();
        let active = source != ModSource::Off || dest != ModDest::Off;
        // An untouched route keeps its place, but it prints nothing: no
        // repeated "Off" and no row of zeros. The amount is silent when the
        // rounded readout is zero, so a tiny value cannot print a bare "0".
        let silent =
            source == ModSource::Off && dest == ModDest::Off && (amount * 100.0).round() == 0.0;

        painter.text(
            egui::pos2(
                index_x + MOD_INDEX_WIDTH * 0.5 * zoom,
                top + row_height * 0.5,
            ),
            Align2::CENTER_CENTER,
            format!("{:02}", slot + 1),
            FontId::new(font_size(8.0, zoom), bold()),
            palette().title_blue,
        );

        if active {
            painter.text(
                egui::pos2(
                    arrow_x + MOD_ARROW_WIDTH * 0.5 * zoom,
                    top + row_height * 0.5,
                ),
                Align2::CENTER_CENTER,
                ">",
                FontId::new(font_size(10.0, zoom), egui::FontFamily::Proportional),
                palette().title_blue,
            );
        }

        if !silent {
            painter.text(
                egui::pos2(value_right, top + row_height * 0.5),
                Align2::RIGHT_CENTER,
                format_amount(amount),
                FontId::new(font_size(10.0, zoom), egui::FontFamily::Proportional),
                palette().text,
            );
        }

        let source_selected = if source == ModSource::Off {
            Some("")
        } else {
            Some(ModSource::variants()[ModSource::to_index(source)])
        };
        let dest_selected = if dest == ModDest::Off {
            Some("")
        } else {
            Some(ModDest::variants()[ModDest::to_index(dest)])
        };

        let source_cell = Rect::from_min_size(
            egui::pos2(source_x, top),
            egui::vec2(MOD_SOURCE_WIDTH * zoom, row_height),
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .id_salt(("sushi-mod-source", slot))
                .max_rect(source_cell)
                .layout(EguiLayout::left_to_right(Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                ui.spacing_mut().interact_size.y = MOD_POPUP_HEIGHT * zoom;
                ui.style_mut().override_font_id = Some(FontId::new(
                    font_size(10.0, zoom),
                    egui::FontFamily::Proportional,
                ));
                ctx.popup_with_text(
                    ui,
                    source_ids[slot],
                    sources[slot],
                    MOD_SOURCE_WIDTH - MOD_COLUMN_GAP,
                    source_selected,
                );
            },
        );

        let dest_cell = Rect::from_min_size(
            egui::pos2(dest_x, top),
            egui::vec2(MOD_DEST_WIDTH * zoom, row_height),
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .id_salt(("sushi-mod-dest", slot))
                .max_rect(dest_cell)
                .layout(EguiLayout::left_to_right(Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                ui.spacing_mut().interact_size.y = MOD_POPUP_HEIGHT * zoom;
                ui.style_mut().override_font_id = Some(FontId::new(
                    font_size(10.0, zoom),
                    egui::FontFamily::Proportional,
                ));
                ctx.popup_with_text(
                    ui,
                    dest_ids[slot],
                    dests[slot],
                    MOD_DEST_WIDTH - MOD_COLUMN_GAP,
                    dest_selected,
                );
            },
        );

        let bar_cell =
            Rect::from_min_size(egui::pos2(bar_x, top), egui::vec2(bar_width, row_height));
        ui.scope_builder(
            egui::UiBuilder::new()
                .id_salt(("sushi-mod-amount", slot))
                .max_rect(bar_cell)
                .layout(EguiLayout::top_down(Align::Min)),
            |ui| {
                ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                ctx.amount_bar(ui, amount_ids[slot], slot, amounts[slot], source);
            },
        );

        painter.line_segment(
            [
                egui::pos2(table.left(), top + row_height),
                egui::pos2(table.right(), top + row_height),
            ],
            Stroke::new(1.0, palette().rule),
        );
    }

    // One quiet divider between the table and the context column. It runs
    // through the band gap, so the column still reads as two envelopes.
    ui.painter().line_segment(
        [
            egui::pos2(context.left() - GROUP_GAP * 0.5 * zoom, table.top()),
            egui::pos2(context.left() - GROUP_GAP * 0.5 * zoom, table.bottom()),
        ],
        Stroke::new(1.0, palette().rule),
    );

    let mod_env = Rect::from_min_max(
        egui::pos2(context.left(), row0.top()),
        egui::pos2(context.right(), row0.bottom()),
    );
    envelope_group(ui, mod_env, zoom, ctx, &EnvelopeSpec::mod_env(ctx.params));
    envelope_group(ui, context, zoom, ctx, &EnvelopeSpec::amp_env(ctx.params));
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
                    ctx.popup(ui, "filter_slope", &p.filter_slope, 66.0);
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
            1 => envelope_group(ui, rect, zoom, ctx, &EnvelopeSpec::mod_env(p)),
            _ => envelope_group(ui, rect, zoom, ctx, &EnvelopeSpec::amp_env(p)),
        }
    });
}

/// The parameters and the captions of one envelope. Keeping them together
/// lets the normal panel and the modulation page use one layout function
/// without a long argument list.
struct EnvelopeSpec<'a> {
    title: &'static str,
    column: &'static str,
    ids: [&'static str; 4],
    params: [&'a FloatParam; 4],
    release_max: f32,
}

impl<'a> EnvelopeSpec<'a> {
    /// The filter envelope of the normal panel and of the matrix page.
    fn mod_env(params: &'a SushiParams) -> Self {
        Self {
            title: "MOD ENV",
            column: "fe",
            ids: ["fe_attack", "fe_decay", "fe_sustain", "fe_release"],
            params: [
                &params.fe_attack,
                &params.fe_decay,
                &params.fe_sustain,
                &params.fe_release,
            ],
            release_max: 4_000.0,
        }
    }

    /// The amplifier envelope of the normal panel and of the matrix page.
    fn amp_env(params: &'a SushiParams) -> Self {
        Self {
            title: "AMP ENV",
            column: "ae",
            ids: ["ae_attack", "ae_decay", "ae_sustain", "ae_release"],
            params: [
                &params.ae_attack,
                &params.ae_decay,
                &params.ae_sustain,
                &params.ae_release,
            ],
            release_max: 8_000.0,
        }
    }
}

/// Draws one of the two envelopes: four A/D/S/R faders in the same geometry
/// as every other section. The normal panel and the modulation page both use
/// it, so the faders return to the page with the exact height and pitch that
/// they have on the panel.
fn envelope_group(ui: &mut egui::Ui, rect: Rect, zoom: f32, ctx: &mut Ctx, spec: &EnvelopeSpec) {
    group(ui, rect, spec.title, zoom, |ui, part| match part {
        Part::Header => {}
        Part::Body => {
            column_row_fixed(ui, spec.column, 4, COLUMN_GAP, 60.0, zoom, |index, ui| {
                let knob = match index {
                    0 | 1 => Knob::logarithmic(0.0, 4_000.0, 1),
                    2 => Knob::percent(),
                    _ => Knob::logarithmic(0.0, spec.release_max, 1),
                };
                let label = match index {
                    0 => "A",
                    1 => "D",
                    2 => "S",
                    _ => "R",
                };
                ctx.float(ui, spec.ids[index], label, spec.params[index], &knob);
            });
        }
    });
}

/// One of the two LFO sections.
///
/// The LFO is a source of the modulation matrix now, so its section has no
/// target or amount. It keeps the waveform, the clock and the retrigger, and
/// the `MOD ROUTE` caption points to the page that owns the routing.
fn lfo_group(ui: &mut egui::Ui, rect: Rect, zoom: f32, ctx: &mut Ctx, index: usize) {
    let p = ctx.params;
    let (title, wave, sync, division, retrigger, rate, ids) = if index == 0 {
        (
            "LFO 1",
            &p.lfo1_wave,
            &p.lfo1_sync,
            &p.lfo1_div,
            &p.lfo1_retrigger,
            &p.lfo1_rate,
            (
                "lfo1_wave",
                "lfo1_sync",
                "lfo1_div",
                "lfo1_retrigger",
                "lfo1_rate",
            ),
        )
    } else {
        (
            "LFO 2",
            &p.lfo2_wave,
            &p.lfo2_sync,
            &p.lfo2_div,
            &p.lfo2_retrigger,
            &p.lfo2_rate,
            (
                "lfo2_wave",
                "lfo2_sync",
                "lfo2_div",
                "lfo2_retrigger",
                "lfo2_rate",
            ),
        )
    };

    group(ui, rect, title, zoom, |ui, part| match part {
        Part::Header => {
            ctx.popup(ui, ids.0, wave, 54.0);
            ctx.check(ui, ids.1, sync, "SYNC");
        }
        Part::Body => {
            column_row_fixed(
                ui,
                ids.4,
                2,
                COLUMN_GAP,
                60.0,
                zoom,
                |index, ui| match index {
                    0 => ctx.float(ui, ids.4, "Rate", rate, &Knob::logarithmic(0.01, 40.0, 2)),
                    1 => {
                        ui.horizontal(|ui| {
                            ctx.check(ui, ids.3, retrigger, "RETRIG");
                        });
                        if sync.value() {
                            caption(ui, "Div", 9.0, zoom, palette().label);
                            ctx.popup(ui, ids.2, division, 50.0);
                        }
                        text_label(ui, "MOD ROUTE", 7.5, zoom, palette().muted);
                    }
                    _ => {}
                },
            );
        }
    });
}

/// The third row: the two LFOs, the arpeggiator and the voice.
///
/// Splitting the LFOs into two small sections gives each one its own waveform
/// and clock. The LFO routing lives in the modulation matrix.
fn row_c(ui: &mut egui::Ui, layout: &Layout, ctx: &mut Ctx) {
    let zoom = ctx.zoom;
    group_row(ui, "row-c", layout, 2, |index, rect, ui| {
        let p = ctx.params;
        match index {
            0 | 1 => lfo_group(ui, rect, zoom, ctx, index),
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
                Part::Body => sections(ui, "fx", 3, GROUP_GAP, zoom, |index, _rect, ui| {
                    let name = ["chorus", "delay", "reverb"][index];
                    fx_section_controls(ui, ctx, index, 52.0, name);
                }),
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
                    palette().muted,
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
                text_label(ui, "CHAR", 8.0, zoom, palette().muted);
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

/// The controls of one effect. The normal page uses them in the compact fourth
/// band, and the FX page uses the same controls with a wider slot.
fn fx_section_controls(ui: &mut egui::Ui, ctx: &mut Ctx, section: usize, slot: f32, name: &str) {
    let zoom = ctx.zoom;
    match section {
        0 => {
            column_row_fixed(ui, name, 3, COLUMN_GAP, slot, zoom, |index, ui| {
                let p = ctx.params;
                match index {
                    0 => ctx.float(ui, "chorus_mix", "Mix", &p.chorus_mix, &Knob::percent()),
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
            });
        }
        1 => {
            column_row_fixed(ui, name, 3, COLUMN_GAP, slot, zoom, |index, ui| {
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
                    2 => ctx.float(ui, "delay_mix", "Mix", &p.delay_mix, &Knob::percent()),
                    _ => {}
                }
            });
        }
        _ => {
            column_row_fixed(ui, name, 2, COLUMN_GAP, slot, zoom, |index, ui| {
                let p = ctx.params;
                match index {
                    0 => ctx.float(
                        ui,
                        "reverb_decay",
                        "Decay",
                        &p.reverb_decay,
                        &Knob::percent(),
                    ),
                    1 => ctx.float(ui, "reverb_mix", "Mix", &p.reverb_mix, &Knob::percent()),
                    _ => {}
                }
            });
        }
    }
}

/// The FX page: a simple, full-panel overview of the existing effects.
///
/// It draws the same chorus, delay and reverb controls as the fourth band of
/// the sound page, with wider fader slots. There are no new DSP effects here:
/// the page is the plumbing for the FX tab and the place where future effect
/// controls can be added.
fn row_fx(ui: &mut egui::Ui, layout: &Layout, ctx: &mut Ctx) {
    let zoom = ctx.zoom;
    let top = layout.scale(layout.base_rows[0]).left_top();
    let bottom = layout.scale(layout.base_rows[3]).right_bottom();
    let rect = Rect::from_min_max(top, bottom);

    group(ui, rect, "FX", zoom, |ui, part| match part {
        Part::Header => {
            text_label(ui, "CHORUS / DELAY / REVERB", 9.0, zoom, palette().muted);
        }
        Part::Body => {
            sections(ui, "fx-page", 3, GROUP_GAP, zoom, |index, _rect, ui| {
                let title = ["CHORUS", "DELAY", "REVERB"][index];
                let name = ["fx-page-chorus", "fx-page-delay", "fx-page-reverb"][index];
                caption(ui, title, 10.0, zoom, palette().title_blue);
                fx_section_controls(ui, ctx, index, 84.0, name);
            });
        }
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

/// Rebuilds the JSON preview of the patch browser. The preview belongs to the
/// patch identity, so undo and redo refresh it as well.
fn refresh_patch_preview(params: &SushiParams, state: &mut EditorState) {
    let title = if let Some(active) = &state.patch_active {
        active.clone()
    } else {
        format!(
            "{:02} {}",
            state.preset_index + 1,
            PRESETS[state.preset_index].name
        )
    };

    if let (Some(_active), Some(path)) = (&state.patch_active, &state.patch_active_path)
        && let Ok(text) = std::fs::read_to_string(path)
    {
        state.patch_preview = text;
        state.patch_preview_title = title;
        return;
    }

    state.patch_preview = serde_json::to_string_pretty(&patch::to_json(params, &title))
        .unwrap_or_else(|_| "{\n  \"error\": \"preview unavailable\"\n}".to_string());
    state.patch_preview_title = title;
}

/// Everything a parameter widget needs to draw itself.
struct Ctx<'a, 'b> {
    params: &'a SushiParams,
    setter: &'b ParamSetter<'b>,
    state: &'a mut EditorState,
    /// The undo and redo history of the panel.
    undo: &'a mut UndoHistory,
    /// The samples of the oscilloscope.
    history: &'a VecDeque<f32>,
    /// The number of voices that are currently playing.
    voice_count: &'a Arc<AtomicUsize>,
    /// The last values of the global modulation sources.
    mod_feedback: &'a ModFeedback,
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

    /// Remembers the sound before a change of this edit. A continuous drag
    /// calls this on every frame, but only the first call takes a snapshot.
    fn mark_edit(&mut self) {
        self.undo.mark(self.params, self.state);
    }

    /// Finishes the edit of this frame unless a gesture is still open. A drag
    /// is committed by its release frame, not by every frame in between.
    fn commit_edit(&mut self) {
        if self.state.gestures.is_empty() {
            self.undo.commit(self.params, self.state);
        }
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

        caption(ui, label, 9.0, zoom, palette().label);

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
            self.mark_edit();
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
        self.commit_edit();
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

        caption(ui, label, 9.0, zoom, palette().label);

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
            self.mark_edit();
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
        self.commit_edit();
        let value = format_number(knob.display(param.value() as f32), knob.decimals);
        if response.hovered() || response.dragged() {
            *self.readout() = Some((
                label.to_ascii_uppercase(),
                format!("{}{}", value, param.unit()),
            ));
        }
        response.on_hover_text(format!("{}{}\nRight-click to type", value, param.unit()));
    }

    /// Draws the horizontal bipolar amount of one route of the modulation
    /// table. It is the SH-101 bender bar turned on its side: a pale fill, a
    /// centre tick and a black handle. Drag changes the amount, Shift drags
    /// in fine steps, a right click opens the numeric editor and a double
    /// click puts the amount back to zero.
    fn amount_bar(
        &mut self,
        ui: &mut egui::Ui,
        id: &'static str,
        slot: usize,
        param: &FloatParam,
        source: ModSource,
    ) {
        #[cfg(test)]
        self.state.count(Widget::Slider);

        let zoom = self.zoom;
        let (rect, mut response) =
            ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        #[cfg(test)]
        self.state.amount_rects.push(rect);

        let painter = ui.painter_at(rect);
        let track = Rect::from_center_size(
            rect.center(),
            egui::vec2(rect.width(), MOD_BAR_HEIGHT * zoom),
        );
        painter.rect_filled(track, CornerRadius::ZERO, palette().track);
        painter.rect_stroke(
            track,
            CornerRadius::ZERO,
            Stroke::new(1.0, palette().rule),
            egui::StrokeKind::Inside,
        );

        let (start, end) = amount_bounds(rect, zoom);
        let centre = (start + end) * 0.5;
        let value = param.value();
        let handle_x = amount_to_x(value, rect, zoom);
        let fill = Rect::from_min_max(
            egui::pos2(centre.min(handle_x), track.top() + zoom),
            egui::pos2(centre.max(handle_x), track.bottom() - zoom),
        );
        painter.rect_filled(fill, CornerRadius::ZERO, palette().button_active);
        painter.line_segment(
            [
                egui::pos2(centre, track.top() + zoom),
                egui::pos2(centre, track.bottom() - zoom),
            ],
            Stroke::new(1.0, palette().rule),
        );
        let handle = Rect::from_center_size(
            egui::pos2(handle_x, track.center().y),
            egui::vec2(MOD_BAR_HANDLE_WIDTH * zoom, (MOD_BAR_HEIGHT - 4.0) * zoom),
        );
        let live = response.hovered() || response.dragged();
        painter.rect_filled(
            handle,
            CornerRadius::ZERO,
            if live {
                palette().title_blue
            } else {
                palette().handle
            },
        );

        // The live marker: the source value times the amount of the route.
        // Sources that belong to a single voice have no global value and are
        // deliberately left without a marker.
        if let Some(marker) = live_marker_rect(source, value, rect, zoom, self.mod_feedback) {
            painter.rect_filled(marker, CornerRadius::ZERO, palette().accent);
        }

        let drag_id = ui.scope_id().with("sushi-amount-drag");
        let mut drag: AmountDrag = ui
            .memory(|memory| memory.data.get_temp(drag_id))
            .unwrap_or_default();
        let (pressed, primary_down, shift) = ui.input(|input| {
            (
                input.pointer.primary_pressed(),
                input.pointer.primary_down(),
                input.modifiers.shift,
            )
        });
        let on_bar = response.hovered() || response.is_pointer_button_down_on();
        if (response.drag_started() || pressed) && on_bar && !drag.started {
            let pointer = response
                .interact_pointer_pos()
                .or_else(|| ui.input(|input| input.pointer.hover_pos()));
            drag = AmountDrag {
                started: true,
                base: value,
                start_x: pointer.map_or(rect.center().x, |pos| pos.x),
                fine: shift,
            };
            ui.memory_mut(|memory| memory.data.insert_temp(drag_id, drag));
        }
        if response.drag_stopped() || (!primary_down && drag.started) {
            drag = AmountDrag::default();
            ui.memory_mut(|memory| memory.data.insert_temp(drag_id, drag));
        }

        let pointer = response.interact_pointer_pos();
        let mut value = param.value();
        if drag.started
            && let Some(pointer) = pointer
            && (response.dragged_by(egui::PointerButton::Primary)
                || (response.is_pointer_button_down_on() && primary_down))
        {
            let next = if drag.fine {
                // A full width of the bar trims ten percent of the range,
                // like the fine rail of a fader.
                let travel = amount_travel(rect, zoom);
                let trim = (pointer.x - drag.start_x) / travel * 2.0 * FINE_TRIM_RANGE;
                (drag.base + trim).clamp(-1.0, 1.0)
            } else {
                amount_from_x(pointer.x, rect, zoom)
            };
            if next != value {
                value = next;
                response.mark_changed();
            }
        }

        // A double click resets the amount to zero. The reset is done after
        // the value of the click, so that it wins.
        let reset = response.double_clicked();
        if self.begin_gesture(id, &response, ui) {
            self.setter.begin_set_parameter(param);
        }
        if response.changed() {
            self.mark_edit();
            self.setter.set_parameter(param, value);
            self.state.dirty = true;
        }
        if reset {
            self.reset_to_default(id, param);
        }
        if self.end_gesture(id, &response, ui) {
            self.setter.end_set_parameter(param);
        }

        self.value_popup_float(ui, id, "Amount", param, &Knob::bipolar(), &response);
        self.commit_edit();
        let text = format_amount(param.value());
        if response.hovered() || response.dragged() {
            *self.readout() = Some((format!("MOD {} AMOUNT", slot + 1), format!("{text}%")));
        }
        response = response.on_hover_text(format!(
            "{text}%\nDrag; Shift for fine\nRight-click to type\nDouble-click to reset"
        ));
        response.on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
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
                    .fill(palette().button_bg)
                    .stroke(Stroke::new(1.0, palette().line))
                    .inner_margin(Margin::same((6.0 * zoom).round() as i8)),
            );

        popup.show(|ui| {
            let text = self.state.texts.entry(id).or_default();
            if response.secondary_clicked() {
                *text = format_number(knob.display(param.value()), knob.decimals);
            }
            let normalized = normalize_number_input(text);
            if normalized != *text {
                *text = normalized;
            }

            ui.label(format!("{}: {}", label, param.name()));
            let edit = ui.add(
                egui::TextEdit::singleline(text)
                    .font(FontId::new(
                        font_size(12.0, zoom),
                        egui::FontFamily::Name(Arc::from(CODE_FAMILY)),
                    ))
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
                        self.mark_edit();
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
                ui.colored_label(palette().status_error, "Invalid value");
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
                    .fill(palette().button_bg)
                    .stroke(Stroke::new(1.0, palette().line))
                    .inner_margin(Margin::same((6.0 * zoom).round() as i8)),
            );

        popup.show(|ui| {
            let text = self.state.texts.entry(id).or_default();
            if response.secondary_clicked() {
                *text = format_number(knob.display(param.value() as f32), knob.decimals);
            }
            let normalized = normalize_number_input(text);
            if normalized != *text {
                *text = normalized;
            }

            ui.label(format!("{}: {}", label, param.name()));
            let edit = ui.add(
                egui::TextEdit::singleline(text)
                    .font(FontId::new(
                        font_size(12.0, zoom),
                        egui::FontFamily::Name(Arc::from(CODE_FAMILY)),
                    ))
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
                        self.mark_edit();
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
                ui.colored_label(palette().status_error, "Invalid value");
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
        self.popup_with_text(ui, id, param, width, None);
    }

    /// The same popup, but with a text that can differ from the value. The
    /// modulation table uses an empty text for the `Off` state, so that eight
    /// empty routes do not print the word "Off" sixteen times.
    fn popup_with_text<T: Enum + PartialEq + Copy + 'static>(
        &mut self,
        ui: &mut egui::Ui,
        id: &'static str,
        param: &EnumParam<T>,
        width: f32,
        selected: Option<&str>,
    ) {
        #[cfg(test)]
        self.state.count(Widget::Popup);

        let current = param.value();
        let variants = T::variants();
        let selected_text = selected
            .map(str::to_owned)
            .unwrap_or_else(|| variants[T::to_index(current)].to_string());

        let response = egui::ComboBox::from_id_salt(id)
            .width(width * self.zoom)
            .selected_text(selected_text)
            .truncate()
            .show_ui(ui, |ui| {
                for (index, name) in variants.iter().enumerate() {
                    let variant = T::from_index(index);
                    if ui.selectable_label(current == variant, *name).clicked() {
                        self.mark_edit();
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
            self.mark_edit();
            self.setter
                .set_parameter(param, param.default_plain_value());
            self.state.dirty = true;
        }

        self.commit_edit();
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
        flat_box(&painter, rect, palette().track);
        if param.value() {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                "X",
                FontId::monospace(font_size(9.0, zoom)),
                palette().text,
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
            self.mark_edit();
            self.setter
                .set_parameter(param, param.default_plain_value());
            self.state.dirty = true;
        } else if response.clicked() {
            self.mark_edit();
            self.setter.set_parameter(param, !param.value());
            self.state.dirty = true;
        }
        self.commit_edit();

        text_label(ui, label, 9.0, zoom, palette().text);
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
                palette().button_active
            } else {
                palette().button_bg
            };
            flat_box(&painter, rect, fill);
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                // The name of the variant is shown in uppercase, like the
                // other labels of the panel.
                variant.to_ascii_uppercase(),
                FontId::proportional(font_size(9.0, zoom)),
                palette().text,
            );

            if response.double_clicked() {
                // A double click resets the parameter to its factory default.
                self.mark_edit();
                self.setter
                    .set_parameter(param, param.default_plain_value());
                self.state.dirty = true;
            } else if response.clicked() {
                self.mark_edit();
                self.setter.set_parameter(param, T::from_index(index));
                self.state.dirty = true;
            }
        }
        self.commit_edit();
    }

    /// Loads the factory preset with the given index.
    fn load_preset(&mut self, index: usize) {
        let index = index % PRESETS.len();
        self.mark_edit();
        self.state.preset_index = index;
        self.state.dirty = false;
        // A factory preset replaces the patch of the user.
        self.state.patch_active = None;
        self.state.patch_active_path = None;
        self.state.patch_name = PRESETS[index].name.to_string();
        presets::apply(self.params, self.setter, index);
        self.refresh_patch_preview();
        self.commit_edit();
    }

    /// Draws the patch browser as a full page. It replaces the synth panel
    /// instantly instead of opening a floating, resizable window.
    fn patch_page(&mut self, ui: &mut egui::Ui) {
        let zoom = self.zoom;
        let page = ui.max_rect();
        ui.painter()
            .rect_filled(page, CornerRadius::ZERO, palette().bg);
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
                            .color(palette().title_blue),
                    ));
                    ui.with_layout(EguiLayout::right_to_left(Align::Center), |ui| {
                        if ui.button("CLOSE").clicked() {
                            close = true;
                        }
                    });
                });

                // FOLDER: the button follows a field that absorbs the row.
                ui.horizontal(|ui| {
                    text_label(ui, "FOLDER", 9.0, zoom, palette().text);
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
                    ui.colored_label(palette().muted, FOLDER_HINT);
                }

                // SEARCH + REFRESH + count in one row.
                ui.horizontal(|ui| {
                    text_label(ui, "SEARCH", 9.0, zoom, palette().text);
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

                                    text_label(ui, "USER", 9.0, zoom, palette().muted);
                                    let files = self.state.patch_files.clone();
                                    if files.is_empty() {
                                        ui.colored_label(palette().muted, NO_PATCHES);
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
                                        let selected = self.state.patch_active.is_some()
                                            && self.state.patch_active_path.as_deref()
                                                == Some(path.as_path());
                                        let clicked = ui
                                            .push_id(("user", &name), |ui| {
                                                ui.selectable_label(selected, &name).clicked()
                                            })
                                            .inner;
                                        if clicked {
                                            self.load_patch_file(&path);
                                        }
                                    }

                                    text_label(ui, "FACTORY", 9.0, zoom, palette().muted);
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
                                        ui.colored_label(palette().muted, NO_MATCHES);
                                    }
                                });
                        },
                    );

                    // No separator here: in a horizontal_top it can extend
                    // to the parent page height and cut through the pinned
                    // bottom action bar. The gap alone separates the columns.
                    ui.add_space(10.0 * zoom);

                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), body_height),
                        EguiLayout::top_down(Align::Min),
                        |ui| {
                            ui.horizontal(|ui| {
                                text_label(ui, "JSON", 9.0, zoom, palette().title_blue);
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(&self.state.patch_preview_title)
                                            .size(font_size(9.0, zoom))
                                            .color(palette().muted),
                                    )
                                    .truncate(),
                                );
                            });
                            let preview_height = (body_height - 24.0 * zoom).max(40.0 * zoom);
                            egui::Frame::NONE
                                .fill(palette().field_bg)
                                .stroke(Stroke::new(1.0, palette().line))
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
                        ui.add_space(8.0 * zoom);
                        ui.horizontal(|ui| {
                            text_label(ui, "NAME", 9.0, zoom, palette().text);
                            // A short field, not a nearly full-width box whose
                            // top border reads as a separator across the bar.
                            let width =
                                (ui.available_width() * 0.32).clamp(160.0 * zoom, 300.0 * zoom);
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
                            palette().status_error
                        } else {
                            palette().status_ok
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
        refresh_patch_preview(self.params, self.state);
    }

    /// Loads a patch of the folder into the synthesizer.
    fn load_patch_file(&mut self, path: &Path) {
        self.mark_edit();
        match patch::load_patch(path, self.params, self.setter) {
            Ok(name) => {
                // The patch that is loaded is not the one that was waiting
                // for a confirmation anymore.
                self.state.patch_confirm_overwrite = false;
                self.state.patch_active = Some(name.clone());
                self.state.patch_active_path = Some(path.to_path_buf());
                // The save field follows the file name, not the display name
                // inside the JSON, so saving again overwrites the file the
                // user actually opened.
                self.state.patch_name = path
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
                    .unwrap_or_else(|| name.clone());
                self.state.dirty = false;
                self.refresh_patch_preview();
                self.set_status(format!("Loaded {name}"), false);
            }
            Err(error) => {
                self.set_status(error, true);
            }
        }
        self.commit_edit();
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
            Ok(path) => {
                self.state.patch_confirm_overwrite = false;
                self.state.patch_name = name.clone();
                self.state.patch_active = Some(name);
                self.state.patch_active_path = Some(path);
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
        self.mark_edit();
        presets::mutate(self.params, self.setter, &mut self.state.rng);
        self.state.dirty = true;
        self.commit_edit();
    }

    /// Puts a parameter back to its factory default. The reset is done
    /// inside of the automation gesture of the widget, so that the host
    /// records it like any other change of the parameter.
    fn reset_to_default<P: Param>(&mut self, id: &'static str, param: &P) {
        self.mark_edit();
        // A reset is a double click. Its first click is folded into the same
        // undo step, so undo returns to the value before the double click.
        self.undo.merge_double_click(self.params, self.state);
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
        self.mark_edit();
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
        // The release frame commits the whole drag as one undo step.
        self.commit_edit();
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
    palette().title_blue
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
        Stroke::new(1.0, palette().rule),
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
            Stroke::new(1.0, palette().tick),
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
            Stroke::new(1.0, palette().rule),
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DragMode {
    Coarse,
    Fine,
}

#[derive(Debug, Clone, Copy, Default)]
struct SliderDrag {
    mode: Option<DragMode>,
    base: f32,
    start_y: f32,
}

/// The drag state of an amount bar. The coarse mode follows the pointer
/// absolutely; the fine mode trims around the value at the start of the drag.
#[derive(Debug, Clone, Copy, Default)]
struct AmountDrag {
    started: bool,
    base: f32,
    start_x: f32,
    fine: bool,
}

/// The left and right centres of the handle of an amount bar. The travel
/// between them is the useful length of the bar.
fn amount_bounds(rect: Rect, zoom: f32) -> (f32, f32) {
    let inset = MOD_BAR_INSET * zoom;
    let handle = MOD_BAR_HANDLE_WIDTH * zoom;
    (
        rect.left() + inset + handle * 0.5,
        rect.right() - inset - handle * 0.5,
    )
}

/// The horizontal travel of the handle of an amount bar.
fn amount_travel(rect: Rect, zoom: f32) -> f32 {
    let (start, end) = amount_bounds(rect, zoom);
    (end - start).max(1.0)
}

/// The x position of the handle for an amount in `-1..1`.
fn amount_to_x(value: f32, rect: Rect, zoom: f32) -> f32 {
    let (start, end) = amount_bounds(rect, zoom);
    start + (value.clamp(-1.0, 1.0) + 1.0) * 0.5 * (end - start)
}

/// The amount in `-1..1` at an x position on an amount bar.
fn amount_from_x(x: f32, rect: Rect, zoom: f32) -> f32 {
    let (start, end) = amount_bounds(rect, zoom);
    ((x - start) / (end - start)).clamp(0.0, 1.0) * 2.0 - 1.0
}

/// The live value of a modulation source, in the units of the matrix.
///
/// Only the global sources have a single value that the editor can show. The
/// envelopes, the velocity, the key and the random source belong to one voice,
/// so they return `None` instead of a made-up value.
fn live_source_value(source: ModSource, feedback: &ModFeedback) -> Option<f32> {
    match source {
        ModSource::Lfo1 => Some(feedback.lfo1()),
        ModSource::Lfo2 => Some(feedback.lfo2()),
        ModSource::Macro1 => Some(feedback.macro1()),
        ModSource::Macro2 => Some(feedback.macro2()),
        ModSource::Macro3 => Some(feedback.macro3()),
        ModSource::ModWheel => Some(feedback.mod_wheel()),
        ModSource::Off
        | ModSource::ModEnv
        | ModSource::AmpEnv
        | ModSource::Velocity
        | ModSource::Key
        | ModSource::Random => None,
    }
}

/// The marker of the live modulation on an amount bar, or `None` when the
/// source has no global value. The marker sits at the product of the source
/// value and the amount of the route, which is exactly the modulation that the
/// matrix adds.
fn live_marker_rect(
    source: ModSource,
    amount: f32,
    rect: Rect,
    zoom: f32,
    feedback: &ModFeedback,
) -> Option<Rect> {
    let source_value = live_source_value(source, feedback)?;
    let live = (source_value * amount).clamp(-1.0, 1.0);
    let x = amount_to_x(live, rect, zoom);
    Some(Rect::from_center_size(
        egui::pos2(x, rect.center().y),
        egui::vec2(MOD_LIVE_MARKER_WIDTH * zoom, (MOD_BAR_HEIGHT - 6.0) * zoom),
    ))
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

    flat_box(&painter, geometry.track, palette().track);
    painter.line_segment(
        [
            egui::pos2(geometry.divider, geometry.track.top()),
            egui::pos2(geometry.divider, geometry.track.bottom()),
        ],
        Stroke::new(1.0, palette().line),
    );

    let id = ui.scope_id().with("sushi-fine-drag");
    let mut drag: SliderDrag = ui
        .memory(|memory| memory.data.get_temp(id))
        .unwrap_or_default();
    // `interact_pointer_pos()` is None while merely hovering. Use the real
    // hover position for lane highlighting and the interaction position only
    // for the drag math.
    let hover_pos = ui.input(|input| input.pointer.hover_pos());
    let pointer = response.interact_pointer_pos();
    let over_fine = hover_pos.is_some_and(|pos| pos.x > geometry.divider);
    let (pressed, primary_down) = ui.input(|input| {
        (
            input.pointer.primary_pressed(),
            input.pointer.primary_down(),
        )
    });
    // `pressed` is global: only this slider may claim the press. A click
    // without a drag must also clear the mode on release, otherwise the next
    // gesture on the other lane inherits the old mode.
    let on_slider = response.hovered() || response.is_pointer_button_down_on();
    if (response.drag_started() || pressed) && on_slider && drag.mode.is_none() {
        let start = pointer.or(hover_pos);
        drag = SliderDrag {
            mode: Some(if over_fine {
                DragMode::Fine
            } else {
                DragMode::Coarse
            }),
            base: value_to_normalized(*value, knob.min, knob.max, knob.log),
            start_y: start.map_or(0.0, |pos| pos.y),
        };
        ui.memory_mut(|memory| memory.data.insert_temp(id, drag));
    }
    if response.drag_stopped() || (!primary_down && drag.mode.is_some()) {
        drag = SliderDrag::default();
        ui.memory_mut(|memory| memory.data.insert_temp(id, drag));
    }

    let position = value_to_position(*value, knob.min, knob.max, knob.log, geometry.track);
    let thumb_y = geometry.thumb_y(position);
    let coarse_thumb = geometry.coarse_thumb(thumb_y);
    let fine_thumb = geometry.fine_thumb(thumb_y);

    let coarse_live = (response.hovered() && !over_fine)
        || (response.dragged() && drag.mode == Some(DragMode::Coarse));
    let fine_live = (response.hovered() && over_fine)
        || (response.dragged() && drag.mode == Some(DragMode::Fine));
    painter.rect_filled(
        coarse_thumb,
        CornerRadius::ZERO,
        if coarse_live {
            palette().title_blue
        } else {
            palette().handle
        },
    );
    painter.rect_filled(
        fine_thumb,
        CornerRadius::ZERO,
        if fine_live {
            palette().title_blue
        } else {
            palette().handle
        },
    );

    let primary = egui::PointerButton::Primary;
    match drag.mode {
        Some(DragMode::Fine) if response.dragged_by(primary) => {
            // Fine lane: a bounded trim around the value at the start of the
            // gesture; full-height travel is ±10% of the parameter range.
            let pointer_y = pointer.map_or(drag.start_y, |pos| pos.y);
            let travel = (drag.start_y - pointer_y) / geometry.track.height();
            let trim = (travel * FINE_TRIM_RANGE).clamp(-FINE_TRIM_RANGE, FINE_TRIM_RANGE);
            let moved = (drag.base + trim).clamp(0.0, 1.0);
            let new_value = position_to_value(moved, knob.min, knob.max, knob.log);
            if new_value != *value {
                *value = new_value;
                response.mark_changed();
            }
        }
        Some(DragMode::Coarse)
            if response.dragged_by(primary)
                || (response.is_pointer_button_down_on()
                    && ui.input(|input| input.pointer.primary_down())) =>
        {
            // Once the gesture starts in the coarse lane it stays coarse,
            // even if the pointer crosses the divider; no mid-drag jump.
            if let Some(pointer) = pointer.or(hover_pos) {
                let position = ((geometry.track.bottom() - pointer.y) / geometry.track.height())
                    .clamp(0.0, 1.0);
                *value = position_to_value(position, knob.min, knob.max, knob.log);
                response.mark_changed();
            }
        }
        _ => {}
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

/// Normalizes typed numeric input. Full-width digits, decimal commas and the
/// symbols produced by a stuck Shift key on the number row all become usable
/// ASCII digits. A numeric field has no use for any other character.
fn normalize_number_input(text: &str) -> String {
    let mut normalized = String::with_capacity(text.len());
    for character in text.chars() {
        let character = match character {
            '０'..='９' => char::from_u32(character as u32 - 0xFEE0).unwrap_or(character),
            '．' => '.',
            '，' => '.',
            '－' => '-',
            '！' => '1',
            '＠' => '2',
            '＃' => '3',
            '＄' => '4',
            '％' => '5',
            '＾' => '6',
            '＆' => '7',
            '＊' => '8',
            '（' => '9',
            '）' => '0',
            '!' => '1',
            '@' => '2',
            '#' => '3',
            '$' => '4',
            '%' => '5',
            '^' => '6',
            '&' => '7',
            '*' => '8',
            '(' => '9',
            ')' => '0',
            ',' => '.',
            ' ' => continue,
            other => other,
        };
        if character.is_ascii_digit() || matches!(character, '+' | '-' | '.') {
            normalized.push(character);
        }
    }
    normalized
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

/// Formats a bipolar amount as a signed whole percentage. Zero has no sign,
/// so an empty route stays quiet.
fn format_amount(value: f32) -> String {
    let percent = (value * 100.0).round();
    if percent == 0.0 {
        "0".to_string()
    } else {
        format!("{percent:+.0}")
    }
}

/// The meter of the header that shows how many voices are playing. The lights
/// are tiny squares, so that the whole meter fits between the presets and the
/// lights of the activity meter.
fn voice_meter(painter: &egui::Painter, rect: Rect, voices: usize, zoom: f32) {
    let font = FontId::proportional(font_size(8.0, zoom));
    let label = painter.layout_no_wrap("VOICES".to_string(), font.clone(), palette().muted);
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
        palette().muted,
    );

    // The row of the lights. Every light that stands for a playing voice is
    // green, the others are dark.
    let row_left = (left + label_width + VOICE_METER_GAP * zoom).round();
    for index in 0..VOICE_LEDS {
        let x = row_left + index as f32 * (VOICE_LED_WIDTH + VOICE_LED_GAP) * zoom;
        let color = if index < voices {
            palette().led_on
        } else {
            palette().led_off
        };
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
    let fill = if on { color } else { palette().led_off };
    painter.rect_filled(rect, CornerRadius::ZERO, fill);
    painter.rect_stroke(
        rect,
        CornerRadius::ZERO,
        Stroke::new(1.0, palette().line),
        egui::StrokeKind::Inside,
    );
}

/// The width of a text in the given font.
fn text_width(painter: &egui::Painter, text: &str, font: &FontId) -> f32 {
    painter
        .layout_no_wrap(text.to_owned(), font.clone(), palette().lcd_fg)
        .size()
        .x
}

/// Fits a name into the preset display: the largest font that works is used,
/// and a name that is still too wide is cut off with an explicit ellipsis.
/// The returned width is never bigger than `max_width`.
fn fit_lcd_text(
    painter: &egui::Painter,
    text: &str,
    max_width: f32,
    zoom: f32,
) -> (String, FontId) {
    let mut size = LCD_FONT_SIZE;
    let mut font = FontId::monospace(font_size(size, zoom));
    while size > LCD_MIN_FONT_SIZE && text_width(painter, text, &font) > max_width {
        size -= 1.0;
        font = FontId::monospace(font_size(size, zoom));
    }
    if text_width(painter, text, &font) <= max_width {
        return (text.to_owned(), font);
    }

    // The name does not even fit at the smallest font, so mark the cut with
    // an ellipsis instead of letting the border clip a glyph in half.
    let mut chars: Vec<char> = text.chars().collect();
    loop {
        chars.pop();
        let mut fitted: String = chars.iter().collect();
        fitted.push_str(LCD_ELLIPSIS);
        if text_width(painter, &fitted, &font) <= max_width {
            return (fitted, font);
        }
        if chars.is_empty() {
            let ellipsis = LCD_ELLIPSIS.to_owned();
            if text_width(painter, &ellipsis, &font) <= max_width {
                return (ellipsis, font);
            }
            return (String::new(), font);
        }
    }
}

/// The flat white display that shows the name of the preset.
fn lcd(painter: &egui::Painter, rect: Rect, text: &str, zoom: f32) {
    flat_box(painter, rect, palette().lcd_bg);

    let max_width = (rect.width() - 2.0 * LCD_PADDING * zoom).max(1.0);
    let (fitted, font) = fit_lcd_text(painter, text, max_width, zoom);

    let painter = painter.with_clip_rect(rect.shrink(2.0 * zoom));
    painter.text(
        rect.left_top() + egui::vec2(LCD_PADDING, rect.height() * 0.5),
        Align2::LEFT_CENTER,
        fitted,
        font,
        palette().lcd_fg,
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
            palette().button_active
        } else {
            palette().primary_bg
        }
    } else if pressed {
        palette().button_active
    } else {
        palette().button_bg
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
        palette().text,
    );

    response
}

/// A page tab. The active tab is filled as a whole and gets an accent frame,
/// so it reads as a pressed tab rather than as an underlined button. The
/// inactive tabs keep the normal button fill.
fn tab_button(
    ui: &mut egui::Ui,
    rect: Rect,
    text: &str,
    active: bool,
    zoom: f32,
) -> egui::Response {
    let response = ui.interact(
        rect,
        ui.scope_id().with(("sushi-tab", text)),
        Sense::click(),
    );
    let painter = ui.painter_at(rect);

    let pressed = response.is_pointer_button_down_on();
    let fill = if active || pressed {
        palette().button_active
    } else {
        palette().button_bg
    };
    flat_box(&painter, rect, fill);
    if active {
        painter.rect_stroke(
            rect.shrink(1.0 * zoom),
            CornerRadius::ZERO,
            Stroke::new(1.0, palette().accent),
            egui::StrokeKind::Inside,
        );
    }
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::new(
            font_size(10.0, zoom),
            if active {
                bold()
            } else {
                egui::FontFamily::Proportional
            },
        ),
        palette().text,
    );

    response
}

/// The small oscilloscope of the header. It is just here to give the panel a
/// bit of life, not to be a measurement tool.
fn small_scope(painter: &egui::Painter, rect: Rect, history: &VecDeque<f32>, zoom: f32) {
    // A dark instrument screen with a quiet grid, even while the sound is
    // silent. It is the one live object in the header.
    flat_box(painter, rect, palette().lcd_bg);
    let inner = rect.shrink(3.0 * zoom);
    for index in 1..4 {
        let x = inner.left() + inner.width() * index as f32 / 4.0;
        painter.line_segment(
            [egui::pos2(x, inner.top()), egui::pos2(x, inner.bottom())],
            Stroke::new(1.0, palette().rule),
        );
    }
    for index in 1..3 {
        let y = inner.top() + inner.height() * index as f32 / 3.0;
        painter.line_segment(
            [egui::pos2(inner.left(), y), egui::pos2(inner.right(), y)],
            Stroke::new(1.0, palette().rule),
        );
    }

    let length = history.len();
    if length < 2 {
        painter.line_segment(
            [
                egui::pos2(inner.left(), inner.center().y),
                egui::pos2(inner.right(), inner.center().y),
            ],
            Stroke::new(1.0, palette().trace),
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

    painter.add(egui::Shape::line(points, Stroke::new(1.0, palette().trace)));
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
    visuals.override_text_color = Some(palette().text);
    visuals.panel_fill = palette().bg;
    visuals.window_fill = palette().bg;
    visuals.window_stroke = Stroke::new(1.0, palette().line);
    visuals.window_corner_radius = CornerRadius::ZERO;
    visuals.window_shadow = egui::epaint::Shadow::NONE;
    visuals.menu_corner_radius = CornerRadius::ZERO;
    visuals.popup_shadow = egui::epaint::Shadow::NONE;
    visuals.extreme_bg_color = palette().field_bg;
    visuals.text_edit_bg_color = Some(palette().field_bg);
    visuals.faint_bg_color = palette().bg;
    visuals.code_bg_color = palette().field_bg;
    visuals.hyperlink_color = palette().text;
    visuals.slider_trailing_fill = false;
    visuals.button_frame = true;
    visuals.selection.bg_fill = palette().button_active;
    visuals.selection.stroke = Stroke::new(1.0, palette().line);

    let square = CornerRadius::ZERO;

    // Every widget is a flat fill with a single 1 px outline: no bevel, no
    // shadow and no expansion.
    visuals.widgets.noninteractive.bg_fill = palette().bg;
    visuals.widgets.noninteractive.weak_bg_fill = palette().bg;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, palette().rule);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, palette().text);
    visuals.widgets.noninteractive.corner_radius = square;
    visuals.widgets.noninteractive.expansion = 0.0;

    // The buttons, the popups and the text fields of the panel.
    visuals.widgets.inactive.bg_fill = palette().button_bg;
    visuals.widgets.inactive.weak_bg_fill = palette().button_bg;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, palette().line);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, palette().text);
    visuals.widgets.inactive.corner_radius = square;
    visuals.widgets.inactive.expansion = 0.0;

    visuals.widgets.hovered.bg_fill = palette().button_bg;
    visuals.widgets.hovered.weak_bg_fill = palette().button_active;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, palette().text);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, palette().text);
    visuals.widgets.hovered.corner_radius = square;
    visuals.widgets.hovered.expansion = 0.0;

    visuals.widgets.active.bg_fill = palette().button_active;
    visuals.widgets.active.weak_bg_fill = palette().button_active;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, palette().text);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, palette().text);
    visuals.widgets.active.corner_radius = square;
    visuals.widgets.active.expansion = 0.0;

    visuals.widgets.open.bg_fill = palette().field_bg;
    visuals.widgets.open.weak_bg_fill = palette().button_active;
    visuals.widgets.open.bg_stroke = Stroke::new(1.0, palette().line);
    visuals.widgets.open.fg_stroke = Stroke::new(1.0, palette().text);
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
        Stroke::new(1.0, palette().line),
        egui::StrokeKind::Inside,
    );
}

#[cfg(test)]
mod tests {
    use super::undo::UNDO_LIMIT;
    use super::*;
    use crate::dsp::mod_feedback::ModFeedbackValues;
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
        let mod_feedback = Arc::new(ModFeedback::default());
        let mut editor = SushiEditor::new(
            params.clone(),
            rx,
            arp_led,
            arp_position,
            voice_count,
            mod_feedback,
        );
        let ctx = egui::Context::default();
        // The tests draw with the fonts of the real panel, so that a text that
        // is too wide is caught here and not only in the host.
        install_fonts(&ctx);
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
            undo: &mut editor.undo_history,
            history: &editor.history,
            voice_count: &editor.voice_count,
            mod_feedback: &editor.mod_feedback,
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

    /// All the filled rectangles of a frame, with their fill colors.
    fn rect_fills(output: &egui::FullOutput) -> Vec<(egui::Rect, Color32)> {
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::epaint::Shape::Rect(rect) if rect.fill != Color32::TRANSPARENT => {
                    Some((rect.rect, rect.fill))
                }
                _ => None,
            })
            .collect()
    }

    /// The fill of the rectangle at the given place.
    fn fill_of(rect: egui::Rect, fills: &[(egui::Rect, Color32)]) -> Color32 {
        fills
            .iter()
            .find(|(candidate, _)| *candidate == rect)
            .map(|(_, fill)| *fill)
            .unwrap_or_else(|| panic!("there is no filled rectangle at {rect:?}"))
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

    /// The rects of the header of a window of the given size.
    fn header_rects_at(size: egui::Vec2) -> HeaderRects {
        let layout = Layout::for_rect(Rect::from_min_size(egui::pos2(0.0, 0.0), size))
            .expect("the window is big enough");
        header_rects(layout.header(), layout.zoom)
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

    /// Drags the primary pointer from one position to another. The modifiers
    /// are installed with an explicit event, because a bare pointer event does
    /// not change the modifier state of the context. The final frame puts the
    /// modifiers back, so that one test cannot leak Shift into the next.
    fn drag(start: egui::Pos2, end: egui::Pos2, modifiers: egui::Modifiers) -> Vec<egui::RawInput> {
        vec![
            input(vec![
                egui::Event::ModifiersChanged(modifiers),
                egui::Event::PointerMoved(start),
            ]),
            input(vec![egui::Event::PointerButton {
                pos: start,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers,
            }]),
            input(vec![egui::Event::PointerMoved(end)]),
            input(vec![egui::Event::PointerMoved(end)]),
            input(vec![egui::Event::PointerButton {
                pos: end,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers,
            }]),
            input(vec![egui::Event::ModifiersChanged(
                egui::Modifiers::default(),
            )]),
        ]
    }

    /// Presses one of the history shortcuts. On Linux the host reports the
    /// command key as Ctrl and as Command, which is what a real event does.
    fn shortcut(key: egui::Key, shift: bool) -> Vec<egui::RawInput> {
        let modifiers = egui::Modifiers {
            ctrl: true,
            command: true,
            shift,
            ..Default::default()
        };
        vec![input(vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }])]
    }

    /// The panel of the default test window.
    fn default_panel(ctx: &egui::Context) -> Rect {
        Layout::for_rect(ctx.viewport_rect())
            .expect("the window is big enough")
            .panel
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
    const VOICES_SLIDER: usize = 40;

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
        assert_eq!(counts.sliders, 54, "unexpected number of sliders");
        assert_eq!(counts.popups, 9, "unexpected number of popups");
        assert_eq!(counts.checks, 7, "unexpected number of check boxes");
        assert_eq!(counts.radios, 1, "unexpected number of radio pairs");
    }

    #[test]
    fn every_parameter_of_the_synthesizer_is_reachable() {
        let (ctx, mut editor, _params) = make_editor();
        run_frames(&ctx, &mut editor, vec![input(vec![])]);

        let counts = editor.state.counts;
        // The panel page has 71 visible patch parameters: 54 sliders, 9
        // choices, 7 switches and one pair of buttons. The LFO target and
        // amount are hidden legacy parameters now, and the two divisions
        // appear only while their sync switch is on (for 73 visible
        // parameters in all). The modulation matrix adds 24 more on its own
        // page, which has its own test. The Legacy DSP flag is a host
        // parameter kept for state compatibility; it is no longer exposed on
        // the panel.
        let continuous = counts.sliders;
        assert_eq!(continuous, 54);
        assert_eq!(
            continuous + counts.popups + counts.checks + counts.radios,
            71
        );
    }

    #[test]
    fn the_lfo_sections_route_through_mod_and_show_the_sync_controls() {
        let (ctx, mut editor, params) = make_editor();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let names: Vec<String> = texts(&output).into_iter().map(|(text, _)| text).collect();

        // The legacy Target/Amount pair is gone from the faceplate; the
        // matrix page is the only place that routes an LFO.
        for hidden in ["Target", "TARGET", "Amount", "AMOUNT"] {
            assert!(
                !names.iter().any(|text| text == hidden),
                "the panel still shows the old LFO {hidden}: {names:?}"
            );
        }
        assert!(
            names.iter().any(|text| text == "MOD ROUTE"),
            "the LFO section has no hint that routing lives in MOD"
        );
        // The division only appears once sync is on.
        assert!(!names.iter().any(|text| text == "1/4"));

        // The first LFO sync switch is the second check box of the panel.
        let sync = editor.state.check_rects[1];
        run_frames(&ctx, &mut editor, click(sync.center()));
        assert!(params.lfo1_sync.value());

        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        assert!(
            texts(&output).iter().any(|(text, _)| text == "1/4"),
            "the division popup did not appear with sync on"
        );
        assert_eq!(
            editor.state.popup_rects.len(),
            10,
            "the LFO 1 division popup is missing"
        );

        // The retrigger switch sits beside the rate fader.
        let retrigger = editor.state.check_rects[2];
        run_frames(&ctx, &mut editor, click(retrigger.center()));
        assert!(params.lfo1_retrigger.value());

        // Open the division popup and pick a different note value.
        let (_, division) = texts(&output)
            .into_iter()
            .find(|(text, _)| text == "1/4")
            .expect("the division popup has no selected value");
        let button = division + egui::vec2(2.0, 2.0);
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
        let (_, eighth) = texts(&output)
            .into_iter()
            .find(|(text, pos)| text == "1/8" && pos.x < 240.0)
            .expect("the division popup has no eighth note");
        run_frames(&ctx, &mut editor, click(eighth + egui::vec2(2.0, 2.0)));
        assert_eq!(params.lfo1_div.value(), crate::params::NoteDiv::Eighth);
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
    fn numeric_input_accepts_fullwidth_and_shifted_digits() {
        assert_eq!(normalize_number_input("１２３．４"), "123.4");
        assert_eq!(normalize_number_input("!@#"), "123");
        assert_eq!(normalize_number_input("1,5"), "1.5");
        assert_eq!(normalize_number_input("-2.50"), "-2.50");
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
    fn a_slider_edit_can_be_undone_and_redone() {
        let (ctx, mut editor, params) = make_editor();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let rect = slider_rects(&ctx, &output)[CUTOFF_SLIDER];
        let before = params.cutoff.value();
        assert!(!editor.undo_history.can_undo());

        run_frames(&ctx, &mut editor, click(rect.center()));
        let after = params.cutoff.value();
        assert_ne!(after, before, "the slider did not change the sound");
        assert!(editor.undo_history.can_undo());
        assert!(editor.state.dirty);

        run_frames(&ctx, &mut editor, shortcut(egui::Key::Z, false));
        let undone = params.cutoff.value();
        assert!(
            (undone - before).abs() < 0.01,
            "undo restored {undone} instead of {before}"
        );
        assert!(!editor.state.dirty, "undo did not restore the clean flag");
        assert!(editor.undo_history.can_redo());

        run_frames(&ctx, &mut editor, shortcut(egui::Key::Z, true));
        let redone = params.cutoff.value();
        assert!(
            (redone - after).abs() < 0.01,
            "redo restored {redone} instead of {after}"
        );
        assert!(editor.state.dirty, "redo did not restore the edit flag");
    }

    #[test]
    fn a_patch_load_can_be_undone_and_redone() {
        let (_ctx, mut editor, params) = make_editor();
        let dir = TempDir::new("undo-load");
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

        // A different sound with no patch loaded, then the patch is loaded.
        setter.set_parameter(&params.cutoff, 500.0);
        editor.state.dirty = true;
        editor.state.patch_active = None;
        editor.state.patch_active_path = None;
        let path = dir.path.join("MY PATCH.json");
        {
            let mut ctx = make_ctx(&mut editor, &params, &setter);
            ctx.load_patch_file(&path);
        }
        assert!((params.cutoff.value() - 1_234.0).abs() < 0.01);
        assert_eq!(editor.state.patch_active.as_deref(), Some("MY PATCH"));

        // Undo brings back the sound and the identity that was there before.
        assert!(editor.undo(&setter));
        assert!((params.cutoff.value() - 500.0).abs() < 0.01);
        assert!(editor.state.patch_active.is_none());
        assert!(editor.state.dirty);

        // Redo loads the patch again.
        assert!(editor.redo(&setter));
        assert!((params.cutoff.value() - 1_234.0).abs() < 0.01);
        assert_eq!(editor.state.patch_active.as_deref(), Some("MY PATCH"));
        assert!(!editor.state.dirty);
    }

    #[test]
    fn a_typed_value_can_be_undone() {
        let (ctx, mut editor, params) = make_editor();
        edit_cutoff(&mut editor, &ctx, "1000");
        assert!((params.cutoff.value() - 1_000.0).abs() < 0.01);

        run_frames(&ctx, &mut editor, shortcut(egui::Key::Z, false));
        let cutoff = params.cutoff.value();
        assert!(
            (cutoff - 3_000.0).abs() < 0.01,
            "the typed value was not undone: {cutoff}"
        );
    }

    #[test]
    fn a_reset_is_one_undo_step() {
        let (ctx, mut editor, params) = make_editor();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let rect = slider_rects(&ctx, &output)[CUTOFF_SLIDER];

        // Move the cutoff away from its default.
        run_frames(
            &ctx,
            &mut editor,
            click(egui::pos2(rect.center().x, rect.top() + 5.0)),
        );
        let before_reset = params.cutoff.value();
        assert_ne!(before_reset, 3_000.0);

        // The first click of the double click is part of the reset, so one
        // undo brings the sound back to the value before the double click.
        run_frames(&ctx, &mut editor, wait());
        run_frames(&ctx, &mut editor, double_click(rect.center()));
        assert!((params.cutoff.value() - 3_000.0).abs() < 0.01);

        run_frames(&ctx, &mut editor, shortcut(egui::Key::Z, false));
        let undone = params.cutoff.value();
        assert!(
            (undone - before_reset).abs() < 0.01,
            "the reset was not one step: {undone} instead of {before_reset}"
        );
    }

    #[test]
    fn a_check_box_reset_is_one_undo_step() {
        let (ctx, mut editor, params) = make_editor();
        run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let rect = editor.state.check_rects[0];

        // Switch the first check box on.
        run_frames(&ctx, &mut editor, click(rect.center()));
        assert!(params.osc_sync.value() || params.arp_on.value());
        let before_reset = if params.osc_sync.value() {
            params.osc_sync.value()
        } else {
            params.arp_on.value()
        };

        // The double click resets it, and one undo returns to the value that
        // the check box had before the double click.
        run_frames(&ctx, &mut editor, wait());
        run_frames(&ctx, &mut editor, double_click(rect.center()));
        run_frames(&ctx, &mut editor, shortcut(egui::Key::Z, false));
        let after_undo = if params.osc_sync.value() {
            params.osc_sync.value()
        } else {
            params.arp_on.value()
        };
        assert_eq!(after_undo, before_reset);
    }

    #[test]
    fn a_preset_load_can_be_undone_and_redone() {
        let (ctx, mut editor, params) = make_editor();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let (_, pos) = texts(&output)
            .into_iter()
            .find(|(text, _)| text == ">")
            .expect("the header has no next button");

        run_frames(&ctx, &mut editor, click(pos + egui::vec2(2.0, 2.0)));
        assert_eq!(editor.state.preset_index, 1);
        assert!((params.cutoff.value() - 4_200.0).abs() < 0.01);

        run_frames(&ctx, &mut editor, shortcut(egui::Key::Z, false));
        assert_eq!(editor.state.preset_index, 0);
        assert!(editor.state.patch_active.is_none());
        assert!((params.cutoff.value() - 3_000.0).abs() < 0.01);

        run_frames(&ctx, &mut editor, shortcut(egui::Key::Z, true));
        assert_eq!(editor.state.preset_index, 1);
        assert!((params.cutoff.value() - 4_200.0).abs() < 0.01);
    }

    #[test]
    fn a_mutation_can_be_undone_and_redone() {
        let (ctx, mut editor, params) = make_editor();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let (_, pos) = texts(&output)
            .into_iter()
            .find(|(text, _)| text == "MUTATE")
            .expect("the header has no mutate button");

        let before = params.cutoff.value();
        run_frames(&ctx, &mut editor, click(pos + egui::vec2(2.0, 2.0)));
        let mutated = params.cutoff.value();
        assert_ne!(mutated, before, "the mutation did not change the sound");

        run_frames(&ctx, &mut editor, shortcut(egui::Key::Z, false));
        let undone = params.cutoff.value();
        assert!(
            (undone - before).abs() < 0.01,
            "the mutation was not undone: {undone} instead of {before}"
        );

        run_frames(&ctx, &mut editor, shortcut(egui::Key::Z, true));
        let redone = params.cutoff.value();
        assert!(
            (redone - mutated).abs() < 0.01,
            "the mutation was not redone: {redone} instead of {mutated}"
        );
    }

    #[test]
    fn a_new_edit_clears_the_redo_branch() {
        let (ctx, mut editor, params) = make_editor();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let rect = slider_rects(&ctx, &output)[CUTOFF_SLIDER];

        // A click near the top changes the sound, and undo brings it back.
        run_frames(
            &ctx,
            &mut editor,
            click(egui::pos2(rect.center().x, rect.top() + 5.0)),
        );
        run_frames(&ctx, &mut editor, shortcut(egui::Key::Z, false));
        assert!(editor.undo_history.can_redo());

        // A new edit must forget the redo branch.
        run_frames(&ctx, &mut editor, wait());
        run_frames(
            &ctx,
            &mut editor,
            click(egui::pos2(rect.center().x, rect.bottom() - 5.0)),
        );
        let second = params.cutoff.value();
        assert!(
            !editor.undo_history.can_redo(),
            "the redo branch survived a new edit"
        );

        // Redo has nothing to restore anymore.
        run_frames(&ctx, &mut editor, shortcut(egui::Key::Z, true));
        assert!((params.cutoff.value() - second).abs() < 0.01);
    }

    #[test]
    fn a_drag_is_one_undo_step() {
        let (ctx, mut editor, params) = make_editor();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let rect = slider_rects(&ctx, &output)[CUTOFF_SLIDER];
        let before = params.cutoff.value();

        let start = egui::pos2(rect.center().x, rect.top() + 8.0);
        let end = egui::pos2(rect.center().x, rect.bottom() - 8.0);
        run_frames(
            &ctx,
            &mut editor,
            drag(start, end, egui::Modifiers::default()),
        );

        let after = params.cutoff.value();
        assert_ne!(after, before, "the drag did not change the sound");
        assert_eq!(
            editor.undo_history.len(),
            1,
            "the drag was recorded more than once"
        );

        run_frames(&ctx, &mut editor, shortcut(egui::Key::Z, false));
        let undone = params.cutoff.value();
        assert!(
            (undone - before).abs() < 0.01,
            "one undo did not restore the pre-drag sound: {undone} instead of {before}"
        );
    }

    #[test]
    fn the_undo_history_is_bounded() {
        let (_ctx, mut editor, params) = make_editor();
        let gui_ctx = editor.gui_ctx.clone().expect("the editor has no host");
        let setter = gui_ctx.param_setter();

        for step in 0..UNDO_LIMIT + 10 {
            editor.undo_history.mark(&params, &editor.state);
            setter.set_parameter(&params.cutoff, 1_000.0 + step as f32);
            editor.state.dirty = true;
            editor.undo_history.commit(&params, &editor.state);
        }

        assert_eq!(
            editor.undo_history.len(),
            UNDO_LIMIT,
            "the undo history grew past its limit"
        );
    }

    #[test]
    fn a_focused_text_field_keeps_the_undo_shortcut() {
        let (ctx, mut editor, params) = make_editor();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let rect = slider_rects(&ctx, &output)[CUTOFF_SLIDER];

        run_frames(&ctx, &mut editor, click(rect.center()));
        let edited = params.cutoff.value();
        assert!(editor.undo_history.can_undo());

        // Open the patch browser and focus its search field.
        editor.state.patch_open = true;
        settle(&ctx, &mut editor);
        ctx.memory_mut(|memory| {
            memory.request_focus(egui::Id::unique("sushi-patch-search"));
        });
        run_frames(&ctx, &mut editor, vec![input(vec![])]);
        assert!(
            ctx.text_edit_focused(),
            "the search field did not take the focus"
        );

        // The shortcut belongs to the text field, not to the sound.
        run_frames(&ctx, &mut editor, shortcut(egui::Key::Z, false));
        assert!(
            (params.cutoff.value() - edited).abs() < 0.01,
            "the sound was undone while a text field had the focus"
        );
        assert!(editor.undo_history.can_undo());
    }

    #[test]
    fn the_footer_undo_and_redo_buttons_work() {
        let (ctx, mut editor, params) = make_editor();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let panel = default_panel(&ctx);
        let (undo_rect, redo_rect) = footer_button_rects(panel, zoom_of(&ctx));

        // Both buttons are visible and disabled while the history is empty.
        let names: Vec<String> = texts(&output).into_iter().map(|(text, _)| text).collect();
        assert!(
            names.contains(&"UNDO".to_string()),
            "there is no undo button"
        );
        assert!(
            names.contains(&"REDO".to_string()),
            "there is no redo button"
        );
        let fills = rect_fills(&output);
        assert_eq!(fill_of(undo_rect, &fills), palette().track);
        assert_eq!(fill_of(redo_rect, &fills), palette().track);

        // An edit enables undo, and the button restores the sound.
        let rect = slider_rects(&ctx, &output)[CUTOFF_SLIDER];
        run_frames(&ctx, &mut editor, click(rect.center()));
        let edited = params.cutoff.value();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let fills = rect_fills(&output);
        assert_eq!(fill_of(undo_rect, &fills), palette().button_bg);
        assert_eq!(fill_of(redo_rect, &fills), palette().track);

        run_frames(&ctx, &mut editor, click(undo_rect.center()));
        assert!((params.cutoff.value() - 3_000.0).abs() < 0.01);

        // Undo enables redo, and the button restores the edit again.
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let fills = rect_fills(&output);
        assert_eq!(fill_of(redo_rect, &fills), palette().button_bg);
        run_frames(&ctx, &mut editor, click(redo_rect.center()));
        assert!((params.cutoff.value() - edited).abs() < 0.01);
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
        // The check boxes are VCO 2 SYNC, the two LFO sync/retrigger pairs,
        // then the ARP ON and LATCH switches. This test uses the first ARP
        // switch.
        let rect = editor.state.check_rects[5];
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
    fn the_header_controls_do_not_overlap_the_voice_meter() {
        for percent in (100..=400).step_by(5) {
            let zoom = percent as f32 / 100.0;
            let rect = Rect::from_min_size(
                egui::pos2(MARGIN, MARGIN) * zoom,
                egui::vec2(WINDOW_WIDTH - 2.0 * MARGIN, HEADER_HEIGHT) * zoom,
            );
            let rects = header_rects(rect, zoom);

            // Every header control has its own slot, in reading order. The
            // removed PATCHES button leaves room for the wider LCD and the two
            // page tabs, without an overlap.
            assert!(rects.previous.left() >= rects.logo.right());
            assert!(rects.display.left() >= rects.previous.right());
            assert!(rects.next.left() >= rects.display.right());
            assert!(rects.mutate.left() >= rects.next.right());
            assert!(rects.mod_page.left() >= rects.mutate.right());
            assert!(rects.fx_page.left() >= rects.mod_page.right());
            assert!(rects.meter.left() >= rects.fx_page.right());
            assert!(!rects.display.intersects(rects.meter));
            assert!(!rects.mod_page.intersects(rects.meter));
            assert!(!rects.fx_page.intersects(rects.meter));
            // The meter still fits between the tabs and the lights of the
            // activity meter.
            assert!(rects.meter.right() <= rect.right() - 194.0 * zoom);
            assert!(rects.meter.width() > VOICE_LEDS as f32 * VOICE_LED_WIDTH * zoom);
        }
    }

    #[test]
    fn the_header_texts_do_not_overlap_at_any_zoom() {
        let (ctx, mut editor, _params) = make_editor();
        for percent in (100..=400).step_by(5) {
            let zoom = percent as f32 / 100.0;
            let size = egui::vec2(WINDOW_WIDTH, WINDOW_HEIGHT) * zoom;
            let output = run_frames(&ctx, &mut editor, vec![input_at(size, vec![])]);
            let bottom = (MARGIN + HEADER_HEIGHT) * zoom;
            let header_texts: Vec<(String, Rect)> = text_shapes(&output)
                .into_iter()
                .filter(|(_, pos, _)| pos.y < bottom)
                .map(|(text, pos, text_size)| (text, Rect::from_min_size(pos, text_size)))
                .collect();

            // The static wordmark, the subtitle and the LCD have to share the
            // header with the buttons and the page tabs without touching.
            for (index, (first, first_rect)) in header_texts.iter().enumerate() {
                for (second, second_rect) in header_texts.iter().skip(index + 1) {
                    let overlap = first_rect.intersect(*second_rect);
                    assert!(
                        overlap.width() <= 1.0 || overlap.height() <= 1.0,
                        "the header texts {first:?} and {second:?} overlap at {percent}%: \
{first_rect:?} / {second_rect:?}"
                    );
                }
            }
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
        assert_eq!(editor.state.page, Page::Mod);
        editor.state.counts = Counts::default();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);

        let counts = editor.state.counts;
        assert_eq!(
            counts.sliders, 40,
            "the matrix needs eight bars and eight envelope faders"
        );
        assert_eq!(counts.popups, 21, "unexpected number of matrix popups");
        assert_eq!(
            editor.state.popup_rects.len(),
            counts.popups,
            "the matrix page left stale popup rectangles behind"
        );
        assert_eq!(counts.checks, 6);
        assert_eq!(counts.radios, 1);
        assert_eq!(
            editor.state.amount_rects.len(),
            8,
            "the matrix needs eight amount bars"
        );
        assert_eq!(
            slider_rects(&ctx, &output).len(),
            32,
            "the matrix lost a vertical fader"
        );

        // The two upper bands now show eight routes; the performance and
        // output bands have not moved.
        let names: Vec<String> = texts(&output).into_iter().map(|(text, _)| text).collect();
        for route in ["01", "02", "03", "04", "05", "06", "07", "08"] {
            assert!(names.contains(&route.to_string()), "missing route {route}");
        }

        // A second click brings the sound controls back.
        run_frames(&ctx, &mut editor, click(button));
        assert_eq!(editor.state.page, Page::Main);
        editor.state.counts = Counts::default();
        run_frames(&ctx, &mut editor, vec![input(vec![])]);
        assert_eq!(editor.state.counts.sliders, 54);
        assert_eq!(editor.state.counts.popups, 9);
        assert_eq!(
            editor.state.popup_rects.len(),
            editor.state.counts.popups,
            "the normal page left stale popup rectangles behind"
        );
    }

    #[test]
    fn the_matrix_page_is_one_compact_table_with_both_envelopes() {
        let (ctx, mut editor, _params) = make_editor();
        editor.state.page = Page::Mod;
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let all_texts = texts(&output);
        let names: Vec<String> = all_texts.iter().map(|(text, _)| text.clone()).collect();
        let table: Vec<&String> = all_texts
            .iter()
            .filter(|(_, pos)| pos.x < 656.0 && pos.y < 318.0)
            .map(|(text, _)| text)
            .collect();

        // One header, eight numbered routes, and both envelopes at once.
        assert_eq!(
            names.iter().filter(|text| *text == "MODULATION").count(),
            1,
            "the table has no single header: {names:?}"
        );
        for route in ["01", "02", "03", "04", "05", "06", "07", "08"] {
            assert!(names.contains(&route.to_string()), "missing route {route}");
        }
        assert!(
            names.contains(&"MOD ENV".to_string()),
            "the modulation page lost MOD ENV: {names:?}"
        );
        assert!(
            names.contains(&"AMP ENV".to_string()),
            "the modulation page lost AMP ENV: {names:?}"
        );

        // The empty state prints no value: no "Off" sixteen times, no "AMT"
        // captions and no row of zeros.
        assert_eq!(
            table.iter().filter(|text| ***text == "Off").count(),
            0,
            "the empty matrix repeats Off: {names:?}"
        );
        assert_eq!(
            table.iter().filter(|text| ***text == "AMT").count(),
            0,
            "the matrix repeats the AMT caption: {names:?}"
        );
        assert_eq!(
            table.iter().filter(|text| ***text == "0").count(),
            0,
            "the empty matrix prints a row of zeros: {names:?}"
        );

        // Every route has a bar whose travel is much longer than the old
        // 82 px fader.
        let zoom = zoom_of(&ctx);
        assert_eq!(editor.state.amount_rects.len(), 8);
        for rect in &editor.state.amount_rects {
            assert!(
                amount_travel(*rect, zoom) > 200.0 * zoom,
                "the amount bar is too short: {rect:?}"
            );
        }

        // Eight envelope faders are back: four in each of the two upper bands,
        // with the height of the envelope faders of the normal panel.
        let sliders = slider_rects(&ctx, &output);
        let envelopes: Vec<egui::Rect> = sliders
            .into_iter()
            .filter(|rect| rect.center().y < 318.0 * zoom && rect.left() >= 664.0 * zoom)
            .collect();
        assert_eq!(envelopes.len(), 8, "unexpected envelope faders");
        assert_eq!(
            envelopes
                .iter()
                .filter(|rect| rect.center().y < 187.0 * zoom)
                .count(),
            4,
            "MOD ENV does not have four faders in the upper band"
        );
        assert_eq!(
            envelopes
                .iter()
                .filter(|rect| rect.center().y > 195.0 * zoom)
                .count(),
            4,
            "AMP ENV does not have four faders in the lower band"
        );
        for rect in envelopes {
            assert!(
                rect.height() > 75.0 * zoom && rect.height() < 90.0 * zoom,
                "an envelope fader lost its height: {rect:?}"
            );
        }
    }

    #[test]
    fn an_amount_that_rounds_to_zero_stays_silent() {
        let (ctx, mut editor, params) = make_editor();
        editor.state.page = Page::Mod;
        let gui_ctx = editor.gui_ctx.clone().expect("the editor has no host");
        let setter = gui_ctx.param_setter();
        setter.set_parameter(&params.mod1_amount, 0.001);

        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let table: Vec<String> = texts(&output)
            .into_iter()
            .filter(|(_, pos)| pos.x < 656.0 && pos.y < 318.0)
            .map(|(text, _)| text)
            .collect();
        assert_eq!(format_amount(0.001), "0");
        assert!(
            !table.iter().any(|text| text == "0"),
            "an amount of 0.001 printed a misleading zero: {table:?}"
        );
    }

    #[test]
    fn the_matrix_page_keeps_the_lower_bands_in_place() {
        let titles = ["LFO 1", "LFO 2", "ARP", "VOICE", "FX", "MACROS", "VCA"];

        let (ctx, mut editor, _params) = make_editor();
        let normal = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let normal: Vec<(String, egui::Pos2)> = texts(&normal)
            .into_iter()
            .filter(|(text, pos)| pos.y > HEADER_HEIGHT && titles.contains(&text.as_str()))
            .collect();
        assert_eq!(normal.len(), titles.len());

        editor.state.page = Page::Mod;
        let matrix = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        for (title, position) in normal {
            let (_, other) = texts(&matrix)
                .into_iter()
                .find(|(text, pos)| *text == title && pos.y > HEADER_HEIGHT)
                .unwrap_or_else(|| panic!("the matrix page has no {title}"));
            assert_eq!(
                other, position,
                "the matrix page moved the lower band section {title}"
            );
        }
    }

    #[test]
    fn the_matrix_popups_keep_a_compact_click_target() {
        let (ctx, mut editor, _params) = make_editor();
        editor.state.page = Page::Mod;
        editor.state.popup_rects.clear();
        run_frames(&ctx, &mut editor, vec![input(vec![])]);

        let zoom = zoom_of(&ctx);
        assert_eq!(editor.state.popup_rects.len(), 21);
        for rect in editor.state.popup_rects.iter().take(16) {
            assert!(
                (rect.height() - 24.0 * zoom).abs() < 1.0,
                "a matrix popup is not 24 px tall: {rect:?}"
            );
            assert!(
                rect.width() >= 120.0 * zoom && rect.width() <= 128.0 * zoom,
                "a matrix popup has the wrong width: {rect:?}"
            );
        }
    }

    #[test]
    fn the_new_modulation_names_fit_the_matrix_popup() {
        let (ctx, _editor, _params) = make_editor();
        // Fonts are only laid out after the first frame of the context.
        let mut output = ctx.run_ui(egui::RawInput::default(), |_| {});
        output.textures_delta.clear();
        let painter = egui::Painter::new(
            ctx.clone(),
            egui::LayerId::background(),
            Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 200.0)),
        );
        let font = FontId::new(font_size(10.0, 1.0), egui::FontFamily::Proportional);
        // The source and destination popups are 124 px wide after the column
        // gap. The longer new names still fit without being cut off.
        let width = MOD_SOURCE_WIDTH - MOD_COLUMN_GAP;
        for name in ModSource::variants().iter().chain(ModDest::variants()) {
            let text_width = text_width(&painter, name, &font);
            assert!(
                text_width <= width,
                "the modulation name {name:?} is {text_width} px wide and does not fit in {width} px"
            );
        }
    }

    #[test]
    fn the_live_marker_follows_the_global_sources() {
        let feedback = ModFeedback::default();
        feedback.store(ModFeedbackValues {
            lfo1: -0.5,
            lfo2: 0.75,
            macro1: 0.25,
            macro2: 0.5,
            macro3: 0.75,
            mod_wheel: 1.0,
        });
        let rect = Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(220.0, 28.0));
        let zoom = 1.0;
        let amount = 0.5;

        // The marker is at the product of the source value and the amount, so
        // it shows the modulation that the matrix actually adds.
        let global = [
            (ModSource::Lfo1, -0.5),
            (ModSource::Lfo2, 0.75),
            (ModSource::Macro1, 0.25),
            (ModSource::Macro2, 0.5),
            (ModSource::Macro3, 0.75),
            (ModSource::ModWheel, 1.0),
        ];
        for (source, value) in global {
            assert_eq!(live_source_value(source, &feedback), Some(value));
            let marker = live_marker_rect(source, amount, rect, zoom, &feedback)
                .unwrap_or_else(|| panic!("{source:?} has no live marker"));
            let expected = amount_to_x(value * amount, rect, zoom);
            assert!(
                (marker.center().x - expected).abs() < 1.0e-4,
                "the marker of {source:?} is at {} instead of {expected}",
                marker.center().x
            );
            assert!((marker.width() - MOD_LIVE_MARKER_WIDTH).abs() < 1.0e-4);
        }

        // Per-voice sources have no single value outside a note, so they get
        // no marker at all instead of a fake one.
        for source in [
            ModSource::Off,
            ModSource::ModEnv,
            ModSource::AmpEnv,
            ModSource::Velocity,
            ModSource::Key,
            ModSource::Random,
        ] {
            assert_eq!(live_source_value(source, &feedback), None);
            assert!(
                live_marker_rect(source, amount, rect, zoom, &feedback).is_none(),
                "{source:?} must not have a live marker"
            );
        }
    }

    #[test]
    fn the_live_markers_are_drawn_only_on_global_routes() {
        let (ctx, mut editor, params) = make_editor();
        let gui_ctx = editor.gui_ctx.clone().expect("the editor has no host");
        let setter = gui_ctx.param_setter();
        editor.state.page = Page::Mod;
        // One route with a global source and one with a per-voice source.
        setter.set_parameter(&params.mod1_source, ModSource::Lfo1);
        setter.set_parameter(&params.mod1_dest, ModDest::Cutoff);
        setter.set_parameter(&params.mod1_amount, 0.5);
        setter.set_parameter(&params.mod2_source, ModSource::ModEnv);
        setter.set_parameter(&params.mod2_dest, ModDest::Volume);
        setter.set_parameter(&params.mod2_amount, 0.5);
        editor.mod_feedback.store(ModFeedbackValues {
            lfo1: 0.8,
            lfo2: -0.4,
            macro1: 0.2,
            macro2: 0.4,
            macro3: 0.6,
            mod_wheel: 0.3,
        });

        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let zoom = zoom_of(&ctx);
        let bars = editor.state.amount_rects.clone();
        assert_eq!(bars.len(), 8);

        // A marker is a narrow accent rectangle inside an amount bar. The
        // handle is not hovered in this frame, so it cannot be mistaken for
        // one.
        let markers: Vec<Rect> = rect_fills(&output)
            .into_iter()
            .filter(|(rect, fill)| {
                *fill == palette().accent
                    && (rect.width() - MOD_LIVE_MARKER_WIDTH * zoom).abs() < 0.1
                    && bars.iter().any(|bar| bar.contains_rect(*rect))
            })
            .map(|(rect, _)| rect)
            .collect();
        assert_eq!(markers.len(), 1, "unexpected live markers: {markers:?}");

        let expected = live_marker_rect(ModSource::Lfo1, 0.5, bars[0], zoom, &editor.mod_feedback)
            .expect("the LFO 1 route lost its marker");
        assert!(
            (markers[0].center().x - expected.center().x).abs() < 0.1,
            "the LFO 1 marker is at {} instead of {}",
            markers[0].center().x,
            expected.center().x
        );
        assert!(
            (markers[0].center().x - bars[0].center().x).abs() > 1.0,
            "the LFO 1 marker did not follow the source"
        );
        assert!(
            !markers.iter().any(|marker| bars[1].contains_rect(*marker)),
            "the per-voice route was given a fake live marker"
        );
    }

    #[test]
    fn an_empty_feedback_still_draws_the_matrix() {
        let (ctx, mut editor, params) = make_editor();
        let gui_ctx = editor.gui_ctx.clone().expect("the editor has no host");
        let setter = gui_ctx.param_setter();
        editor.state.page = Page::Mod;
        setter.set_parameter(&params.mod1_source, ModSource::Macro1);
        setter.set_parameter(&params.mod1_dest, ModDest::Pitch);
        setter.set_parameter(&params.mod1_amount, 0.5);

        // No audio block has run yet, so every reading is zero. The page must
        // still draw and the marker must sit in the middle of the bar.
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let zoom = zoom_of(&ctx);
        assert_eq!(editor.state.amount_rects.len(), 8);
        let bar = editor.state.amount_rects[0];
        let marker = rect_fills(&output)
            .into_iter()
            .find(|(rect, fill)| {
                *fill == palette().accent
                    && (rect.width() - MOD_LIVE_MARKER_WIDTH * zoom).abs() < 0.1
                    && bar.contains_rect(*rect)
            })
            .map(|(rect, _)| rect)
            .expect("the macro route is missing its live marker");
        assert!((marker.center().x - bar.center().x).abs() < 0.1);
    }

    #[test]
    fn dragging_the_amount_bar_sets_the_route_in_one_gesture() {
        let (ctx, mut editor, params) = make_editor();
        editor.state.page = Page::Mod;
        run_frames(&ctx, &mut editor, vec![input(vec![])]);

        let rect = editor.state.amount_rects[0];
        let zoom = zoom_of(&ctx);
        let travel = amount_travel(rect, zoom);
        let start = egui::pos2(rect.center().x - travel * 0.1, rect.center().y);
        let end = egui::pos2(rect.center().x + travel * 0.35, rect.center().y);

        GESTURES.with(|gestures| gestures.set(0));
        SETS.with(|sets| sets.set(0));
        run_frames(
            &ctx,
            &mut editor,
            drag(start, end, egui::Modifiers::default()),
        );

        let amount = params.mod1_amount.value();
        assert!(
            amount > 0.6,
            "the amount bar did not follow the drag: {amount}"
        );
        assert_eq!(
            GESTURES.with(|gestures| gestures.get()),
            2,
            "the amount bar did not open and close exactly one gesture"
        );
        assert!(
            SETS.with(|sets| sets.get()) >= 2,
            "the amount was not set while dragging"
        );
        assert!(editor.state.dirty, "the drag did not mark the sound dirty");
        let (label, value) = editor
            .state
            .readout
            .clone()
            .expect("the amount bar has no readout");
        assert_eq!(label, "MOD 1 AMOUNT");
        assert!(value.ends_with('%'), "the readout lost its unit: {value}");

        // A double click on the bar puts the amount back to zero.
        run_frames(&ctx, &mut editor, wait());
        run_frames(&ctx, &mut editor, double_click(rect.center()));
        assert!(
            params.mod1_amount.value().abs() < 1.0e-4,
            "the amount bar was not reset: {}",
            params.mod1_amount.value()
        );
    }

    #[test]
    fn shift_dragging_the_amount_bar_is_fine() {
        let (ctx, mut editor, params) = make_editor();
        editor.state.page = Page::Mod;
        run_frames(&ctx, &mut editor, vec![input(vec![])]);

        let rect = editor.state.amount_rects[0];
        let zoom = zoom_of(&ctx);
        let travel = amount_travel(rect, zoom);
        let start = rect.center();
        let end = start + egui::vec2(travel * 0.15, 0.0);
        let gui_ctx = editor.gui_ctx.clone().expect("the editor has no host");
        let setter = gui_ctx.param_setter();

        setter.set_parameter(&params.mod1_amount, 0.0);
        run_frames(
            &ctx,
            &mut editor,
            drag(start, end, egui::Modifiers::default()),
        );
        let coarse = params.mod1_amount.value();
        assert!(coarse > 0.2, "the coarse drag was too small: {coarse}");

        setter.set_parameter(&params.mod1_amount, 0.0);
        run_frames(
            &ctx,
            &mut editor,
            drag(
                start,
                end,
                egui::Modifiers {
                    shift: true,
                    ..Default::default()
                },
            ),
        );
        let fine = params.mod1_amount.value();
        assert!(fine > 0.0, "the fine drag did not move: {fine}");
        assert!(
            fine < coarse / 3.0,
            "the fine drag was not fine: {fine} instead of {coarse}"
        );
    }

    #[test]
    fn the_amount_bar_accepts_a_typed_value() {
        let (ctx, mut editor, params) = make_editor();
        editor.state.page = Page::Mod;
        run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let pos = editor.state.amount_rects[0].center();

        // Open the numeric editor with a secondary click.
        run_frames(
            &ctx,
            &mut editor,
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

        // Type a signed percentage, the same unit as the readout.
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
        for character in "50".chars() {
            events.push(egui::Event::Text(character.to_string()));
        }
        events.push(egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        });
        run_frames(&ctx, &mut editor, vec![input(events), input(vec![])]);

        assert!(
            (params.mod1_amount.value() - 0.5).abs() < 1.0e-3,
            "the typed amount was not applied: {}",
            params.mod1_amount.value()
        );
    }

    #[test]
    fn a_matrix_source_popup_sets_the_route() {
        let (ctx, mut editor, params) = make_editor();
        editor.state.page = Page::Mod;
        run_frames(&ctx, &mut editor, vec![input(vec![])]);

        // The empty source field of the first route. Its popup has no selected
        // text, but it is still a full click target.
        let button = editor.state.popup_rects[0].center();
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
                // The list of choices is only painted from the next frame on.
                input(vec![]),
            ],
        );

        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let (_, item) = texts(&output)
            .into_iter()
            .find(|(text, pos)| text == "LFO 1" && pos.y < 318.0)
            .expect("the source popup has no LFO 1");
        run_frames(&ctx, &mut editor, click(item + egui::vec2(2.0, 2.0)));

        assert_eq!(params.mod1_source.value(), ModSource::Lfo1);
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let names: Vec<String> = texts(&output).into_iter().map(|(text, _)| text).collect();
        assert!(
            names.contains(&"LFO 1".to_string()),
            "the chosen source is not shown in the table: {names:?}"
        );
    }

    #[test]
    fn no_two_texts_overlap_at_any_zoom() {
        for percent in (100..=400).step_by(5) {
            let zoom = percent as f32 / 100.0;
            for page in [Page::Main, Page::Mod, Page::Fx] {
                let (ctx, mut editor, params) = make_editor();
                let gui_ctx = editor.gui_ctx.clone().expect("the editor has no host");
                let setter = gui_ctx.param_setter();
                editor.state.page = page;
                if page == Page::Mod {
                    // Fill the table with the longest names, a full spread of
                    // amounts and a sign at both ends, so the overlap check is
                    // about the real page and not an empty one.
                    setter.set_parameter(&params.mod1_source, ModSource::Lfo1);
                    setter.set_parameter(&params.mod1_dest, ModDest::Resonance);
                    setter.set_parameter(&params.mod1_amount, 0.62);
                    setter.set_parameter(&params.mod2_source, ModSource::Lfo2);
                    setter.set_parameter(&params.mod2_dest, ModDest::Pulse2);
                    setter.set_parameter(&params.mod2_amount, -0.38);
                    setter.set_parameter(&params.mod3_source, ModSource::ModEnv);
                    setter.set_parameter(&params.mod3_dest, ModDest::Cutoff);
                    setter.set_parameter(&params.mod3_amount, 1.0);
                    setter.set_parameter(&params.mod4_source, ModSource::AmpEnv);
                    setter.set_parameter(&params.mod4_dest, ModDest::Volume);
                    setter.set_parameter(&params.mod4_amount, -0.5);
                    setter.set_parameter(&params.mod5_source, ModSource::Velocity);
                    setter.set_parameter(&params.mod5_dest, ModDest::Drive);
                    setter.set_parameter(&params.mod5_amount, 0.12);
                    setter.set_parameter(&params.mod6_source, ModSource::Key);
                    setter.set_parameter(&params.mod6_dest, ModDest::Pan);
                    setter.set_parameter(&params.mod6_amount, -0.75);
                    setter.set_parameter(&params.mod7_source, ModSource::Macro1);
                    setter.set_parameter(&params.mod7_dest, ModDest::Pulse1);
                    setter.set_parameter(&params.mod7_amount, 0.99);
                    setter.set_parameter(&params.mod8_source, ModSource::Macro3);
                    setter.set_parameter(&params.mod8_dest, ModDest::Pitch);
                    setter.set_parameter(&params.mod8_amount, -1.0);
                }
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
                let amount_bars = editor.state.amount_rects.clone();
                for (text, pos, size) in &texts {
                    let rect = Rect::from_min_size(*pos, *size);
                    for slider in &sliders {
                        let overlap = rect.intersect(*slider);
                        assert!(
                            overlap.width() <= 1.0 || overlap.height() <= 1.0,
                            "the text {text:?} is covered by a slider at {percent}% \
(page={page:?}): {rect:?} / {slider:?}"
                        );
                    }
                    for bar in &amount_bars {
                        let overlap = rect.intersect(*bar);
                        assert!(
                            overlap.width() <= 1.0 || overlap.height() <= 1.0,
                            "the text {text:?} is covered by an amount bar at {percent}% \
(page={page:?}): {rect:?} / {bar:?}"
                        );
                    }
                    for popup in &editor.state.popup_rects {
                        // The selected text lives inside its own popup button.
                        if popup.contains(rect.center()) {
                            assert!(
                                popup.expand(zoom).contains_rect(rect),
                                "the text {text:?} is clipped by its popup at {percent}% \
(page={page:?}): {rect:?} / {popup:?}"
                            );
                            continue;
                        }
                        let overlap = rect.expand(4.0 * zoom).intersect(*popup);
                        assert!(
                            overlap.width() <= 1.0 || overlap.height() <= 1.0,
                            "the text {text:?} is too close to a popup at {percent}% \
(page={page:?}): {rect:?} / {popup:?}"
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
                            "the texts {first:?} and {second:?} overlap at {percent}% \
(page={page:?}): {first_rect:?} / {second_rect:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_matrix_page_fits_into_the_panel() {
        let (ctx, mut editor, _params) = make_editor();
        editor.state.page = Page::Mod;
        editor.state.counts = Counts::default();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let panel = Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            egui::vec2(WINDOW_WIDTH, WINDOW_HEIGHT),
        );
        assert_the_widgets_fit(&ctx, &output, panel);

        // The horizontal bars are not vertical sliders, so they are checked
        // here: inside the panel and away from the texts, including the value
        // column on their right.
        let texts = text_shapes(&output);
        for bar in &editor.state.amount_rects {
            assert!(
                panel.contains_rect(*bar),
                "an amount bar is outside of the panel: {bar:?}"
            );
            for (text, pos, size) in &texts {
                let rect = Rect::from_min_size(*pos, *size);
                let overlap = rect.intersect(*bar);
                assert!(
                    overlap.width() <= 1.0 || overlap.height() <= 1.0,
                    "the text {text:?} is covered by an amount bar: {rect:?} / {bar:?}"
                );
            }
        }
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
    fn the_patches_button_is_gone_from_the_header() {
        let (ctx, mut editor, _params) = make_editor();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let names: Vec<String> = texts(&output).into_iter().map(|(text, _)| text).collect();
        assert!(
            !names.contains(&"PATCHES".to_string()),
            "the header still has a separate PATCHES button: {names:?}"
        );
    }

    #[test]
    fn the_lcd_opens_the_patch_browser() {
        let (ctx, mut editor, _params) = make_editor();
        run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let rects = header_rects_at(egui::vec2(WINDOW_WIDTH, WINDOW_HEIGHT));

        assert!(!editor.state.patch_open);
        run_frames(&ctx, &mut editor, click(rects.display.center()));
        assert!(
            editor.state.patch_open,
            "clicking the preset LCD did not open the patch browser"
        );
    }

    #[test]
    fn the_mod_and_fx_tabs_are_mutually_exclusive_and_fill_when_active() {
        let (ctx, mut editor, _params) = make_editor();
        let rects = header_rects_at(egui::vec2(WINDOW_WIDTH, WINDOW_HEIGHT));
        run_frames(&ctx, &mut editor, vec![input(vec![])]);

        // The normal page has two inactive tabs.
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let fills = rect_fills(&output);
        assert_eq!(fill_of(rects.mod_page, &fills), palette().button_bg);
        assert_eq!(fill_of(rects.fx_page, &fills), palette().button_bg);

        // FX fills as a whole while MOD stays normal.
        run_frames(&ctx, &mut editor, click(rects.fx_page.center()));
        assert_eq!(editor.state.page, Page::Fx);
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let fills = rect_fills(&output);
        assert_eq!(fill_of(rects.fx_page, &fills), palette().button_active);
        assert_eq!(fill_of(rects.mod_page, &fills), palette().button_bg);

        // MOD takes over the highlight; FX becomes inactive again.
        run_frames(&ctx, &mut editor, click(rects.mod_page.center()));
        assert_eq!(editor.state.page, Page::Mod);
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let fills = rect_fills(&output);
        assert_eq!(fill_of(rects.mod_page, &fills), palette().button_active);
        assert_eq!(fill_of(rects.fx_page, &fills), palette().button_bg);

        // Clicking the active tab returns to the normal page.
        run_frames(&ctx, &mut editor, click(rects.mod_page.center()));
        assert_eq!(editor.state.page, Page::Main);
    }

    #[test]
    fn the_fx_tab_opens_the_existing_effect_controls() {
        let (ctx, mut editor, _params) = make_editor();
        let rects = header_rects_at(egui::vec2(WINDOW_WIDTH, WINDOW_HEIGHT));

        run_frames(&ctx, &mut editor, click(rects.fx_page.center()));
        assert_eq!(editor.state.page, Page::Fx);
        editor.state.counts = Counts::default();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);

        // The same eight effect parameters as the compact band of the main
        // page, with one caption for each effect.
        assert_eq!(editor.state.counts.sliders, 8);
        assert_eq!(editor.state.counts.popups, 0);
        assert_eq!(editor.state.counts.checks, 0);
        assert_eq!(editor.state.counts.radios, 0);
        let names: Vec<String> = texts(&output).into_iter().map(|(text, _)| text).collect();
        for name in [
            "FX", "CHORUS", "DELAY", "REVERB", "MIX", "DEPTH", "TIME", "DECAY",
        ] {
            assert!(
                names.contains(&name.to_string()),
                "the FX page has no {name}"
            );
        }
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

        // Opening the browser reads the list of the folder again. The LCD
        // itself is the button now.
        run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let rects = header_rects_at(egui::vec2(WINDOW_WIDTH, WINDOW_HEIGHT));
        run_frames(&ctx, &mut editor, click(rects.display.center()));

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
    fn the_browser_previews_the_loaded_file_by_its_path() {
        let (_ctx, mut editor, params) = make_editor();
        let dir = TempDir::new("identity-preview");
        editor.state.patch_config.patch_dir = dir.path.clone();
        editor.state.patch_dir_text = dir.path.to_string_lossy().into_owned();

        // The internal name differs from the file stem on purpose. The path
        // must win, otherwise the preview and the selection point at the
        // wrong file.
        let path = dir.path.join("FILE.json");
        std::fs::write(
            &path,
            r#"{"format":"sushi-patch","version":1,"name":"DISPLAY","params":{"cutoff":1234.0}}"#,
        )
        .expect("the patch was not written");

        let gui_ctx = editor.gui_ctx.clone().expect("the editor has no host");
        let setter = gui_ctx.param_setter();
        {
            let mut ctx = make_ctx(&mut editor, &params, &setter);
            ctx.load_patch_file(&path);
        }

        assert_eq!(editor.state.patch_active.as_deref(), Some("DISPLAY"));
        assert_eq!(
            editor.state.patch_active_path.as_deref(),
            Some(path.as_path())
        );
        // Saving again targets FILE.json, not DISPLAY.json.
        assert_eq!(editor.state.patch_name, "FILE");
        assert!(
            editor.state.patch_preview.contains("\"name\":\"DISPLAY\"")
                || editor.state.patch_preview.contains("\"name\": \"DISPLAY\""),
            "the preview did not read the file: {}",
            editor.state.patch_preview
        );
    }

    #[test]
    fn a_system_cjk_font_is_used_when_available() {
        let available = CJK_FONT_PATHS
            .iter()
            .any(|(path, _)| Path::new(path).exists());
        if !available {
            return;
        }

        let ctx = egui::Context::default();
        install_fonts(&ctx);
        let _ = run_frames(&ctx, &mut make_editor().1, vec![input(vec![])]);
        let has_glyphs =
            ctx.fonts_mut(|fonts| fonts.has_glyphs(&egui::FontId::proportional(12.0), "中文"));

        assert!(has_glyphs, "the CJK fallback was not installed");
    }

    #[test]
    fn the_panel_always_uses_the_legacy_palette() {
        let (ctx, mut editor, _params) = make_editor();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let fills = rect_fills(&output);

        assert!(
            fills.iter().any(|(_, fill)| *fill == palette().bg),
            "the panel did not paint its legacy background"
        );
        assert!(
            !fills
                .iter()
                .any(|(_, fill)| *fill == Color32::from_rgb(0xF1, 0xF0, 0xEB)),
            "the modern paper background is still painted"
        );
    }

    #[test]
    fn the_wordmark_is_static_and_not_a_flavor_switch() {
        let (ctx, mut editor, params) = make_editor();
        assert!(!params.legacy.value());

        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let (_, wordmark) = texts(&output)
            .into_iter()
            .find(|(text, _)| text == "SUSHI")
            .expect("the header has no wordmark");
        run_frames(&ctx, &mut editor, click(wordmark + egui::vec2(2.0, 2.0)));

        // The logo is no switch anymore.
        assert!(
            !params.legacy.value(),
            "the logo still toggles the DSP flag"
        );

        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let names: Vec<String> = texts(&output).into_iter().map(|(text, _)| text).collect();
        assert!(
            names.contains(&"SUSHI".to_string()),
            "the wordmark disappeared: {names:?}"
        );
        assert!(
            names.contains(&"POLYPHONIC SYNTHESIZER".to_string()),
            "the header lost its static subtitle: {names:?}"
        );
        for mode in [
            "POLYPHONIC / MODERN",
            "POLYPHONIC / LEGACY",
            "/ MODERN",
            "/ LEGACY",
        ] {
            assert!(
                !names.contains(&mode.to_string()),
                "the header still shows the mode subtitle {mode:?}: {names:?}"
            );
        }
    }

    #[test]
    fn the_header_subtitle_stays_inside_the_logo() {
        let (ctx, mut editor, _params) = make_editor();
        let subtitle = "POLYPHONIC SYNTHESIZER";

        for percent in (100..=400).step_by(5) {
            let zoom = percent as f32 / 100.0;
            let size = egui::vec2(WINDOW_WIDTH, WINDOW_HEIGHT) * zoom;
            let output = run_frames(&ctx, &mut editor, vec![input_at(size, vec![])]);
            let rects = header_rects_at(size);
            assert!(
                !rects.logo.intersects(rects.previous),
                "the logo overlaps the previous preset at {percent}%: {:?} / {:?}",
                rects.logo,
                rects.previous
            );
            let (_, pos, text_size) = text_shapes(&output)
                .into_iter()
                .find(|(text, _, _)| text == subtitle)
                .unwrap_or_else(|| panic!("the header has no subtitle {subtitle:?} at {percent}%"));
            let rect = Rect::from_min_size(pos, text_size);

            assert!(
                rects.logo.contains_rect(rect),
                "the subtitle {subtitle:?} is outside the logo at {percent}%: {rect:?} / {:?}",
                rects.logo
            );
            for (name, control) in [
                ("preset display", rects.display),
                ("previous preset", rects.previous),
                ("next preset", rects.next),
                ("mutate", rects.mutate),
                ("mod tab", rects.mod_page),
                ("fx tab", rects.fx_page),
            ] {
                assert!(
                    !rect.intersects(control),
                    "the subtitle {subtitle:?} intersects the {name} at {percent}%: \
{rect:?} / {control:?}"
                );
            }
        }
    }

    #[test]
    fn the_patch_identity_survives_a_host_state_reload() {
        let (_ctx, mut editor, params) = make_editor();

        *params
            .preset_index
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = 3;
        *params
            .patch_name
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some("MY PATCH".to_string());
        *params
            .patch_path
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some("/tmp/MY PATCH.json".to_string());
        *params
            .patch_dirty
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = true;

        editor.init_patch_state();

        assert_eq!(editor.state.preset_index, 3);
        assert_eq!(editor.state.patch_active.as_deref(), Some("MY PATCH"));
        assert_eq!(
            editor.state.patch_active_path.as_deref(),
            Some(Path::new("/tmp/MY PATCH.json"))
        );
        assert!(editor.state.dirty);

        // The other direction: the editor writes the current identity back
        // into the host state.
        editor.state.preset_index = 4;
        editor.state.patch_active = Some("OTHER".to_string());
        editor.state.patch_active_path = Some(PathBuf::from("/tmp/OTHER.json"));
        editor.state.dirty = false;
        editor.sync_persisted_state();

        assert_eq!(
            *params
                .preset_index
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
            4
        );
        assert_eq!(
            params
                .patch_name
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .as_deref(),
            Some("OTHER")
        );
        assert_eq!(
            params
                .patch_path
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .as_deref(),
            Some("/tmp/OTHER.json")
        );
        assert!(
            !*params
                .patch_dirty
                .lock()
                .unwrap_or_else(|error| error.into_inner())
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
    fn the_display_shows_the_long_factory_names() {
        let names = [
            "PROPHET BRASS",
            "MELLOTRON FLUTE",
            "JUPITER STRINGS",
            "POLY STRINGS",
        ];
        let (ctx, mut editor, _params) = make_editor();
        for percent in (100..=400).step_by(5) {
            let zoom = percent as f32 / 100.0;
            let size = egui::vec2(WINDOW_WIDTH, WINDOW_HEIGHT) * zoom;
            let rects = header_rects_at(size);
            for name in names {
                let index = PRESETS
                    .iter()
                    .position(|preset| preset.name == name)
                    .unwrap_or_else(|| panic!("there is no preset {name:?}"));
                editor.state.preset_index = index;
                editor.state.patch_active = None;
                editor.state.patch_active_path = None;
                editor.state.dirty = false;

                let output = run_frames(&ctx, &mut editor, vec![input_at(size, vec![])]);
                let expected = format!("{:02} {name}", index + 1);
                let (_, pos, text_size) = text_shapes(&output)
                    .into_iter()
                    .find(|(text, _, _)| text == &expected)
                    .unwrap_or_else(|| {
                        panic!("the display does not show {expected:?} at {percent}%")
                    });
                let rect = Rect::from_min_size(pos, text_size);
                assert!(
                    rects.display.contains_rect(rect),
                    "the display text {expected:?} is outside the display at {percent}%: \
{rect:?} / {:?}",
                    rects.display
                );
            }
        }
    }

    #[test]
    fn a_name_that_is_too_long_ends_in_an_ellipsis() {
        let (ctx, mut editor, _params) = make_editor();
        editor.state.patch_active = Some("A VERY LONG PATCH NAME THAT DOES NOT FIT".to_string());

        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let rects = header_rects_at(egui::vec2(WINDOW_WIDTH, WINDOW_HEIGHT));
        let (text, pos, text_size) = text_shapes(&output)
            .into_iter()
            .find(|(_, pos, size)| {
                rects
                    .display
                    .contains_rect(Rect::from_min_size(*pos, *size))
            })
            .expect("the display has no text");
        let rect = Rect::from_min_size(pos, text_size);

        assert!(
            text.ends_with(LCD_ELLIPSIS),
            "the cut name {text:?} has no explicit ellipsis"
        );
        assert!(
            rects.display.contains_rect(rect),
            "the cut name is outside the display: {rect:?} / {:?}",
            rects.display
        );
    }

    #[test]
    fn fit_lcd_text_puts_an_ellipsis_on_a_narrow_display() {
        let ctx = egui::Context::default();
        install_fonts(&ctx);
        // Fonts are only laid out after the first frame of the context.
        let mut output = ctx.run_ui(egui::RawInput::default(), |_| {});
        output.textures_delta.clear();
        let painter = egui::Painter::new(
            ctx,
            egui::LayerId::background(),
            Rect::from_min_size(Pos2::ZERO, egui::vec2(100.0, 100.0)),
        );
        let text = "A VERY LONG PATCH NAME THAT DOES NOT FIT";
        let max_width = 40.0;

        let (fitted, font) = fit_lcd_text(&painter, text, max_width, 1.0);

        assert!(fitted.ends_with(LCD_ELLIPSIS), "{fitted:?} has no ellipsis");
        assert_ne!(fitted, text);
        assert!(text_width(&painter, &fitted, &font) <= max_width);
    }

    #[test]
    fn control_characters_in_display_names_become_spaces() {
        assert_eq!(sanitize_lcd_name("A\nB\rC\tD"), "A B C D");

        let (_ctx, mut editor, _params) = make_editor();
        editor.state.patch_active = Some("A\nB\rC\tD".to_string());
        assert_eq!(display_text(&editor.state), "A B C D");
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
    fn the_filter_slope_popup_shows_the_old_and_the_modern_four_pole() {
        let (ctx, mut editor, params) = make_editor();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let (_, pos) = texts(&output)
            .into_iter()
            .find(|(text, _)| text == "12 dB")
            .expect("the filter slope popup is missing");
        let button = pos + egui::vec2(2.0, 2.0);

        // Open the popup. The second frame paints the list of choices.
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
            names.contains(&"24 dB OLD".to_string()),
            "the popup has no old four-pole option: {names:?}"
        );
        assert!(
            names.contains(&"24 dB".to_string()),
            "the popup has no modern four-pole option: {names:?}"
        );

        // The old option is the legacy mapping, and it sits at the index that
        // old host states use.
        assert_eq!(crate::params::FilterSlope::variants()[1], "24 dB OLD");
        assert_eq!(
            crate::params::FilterSlope::from_index(1),
            crate::params::FilterSlope::FourPole
        );

        // Pick the old option and check that the parameter follows it.
        let (_, old) = texts(&output)
            .into_iter()
            .find(|(text, _)| text == "24 dB OLD")
            .expect("the old four-pole option is missing");
        let item = old + egui::vec2(2.0, 2.0);
        run_frames(&ctx, &mut editor, click(item));
        assert_eq!(
            params.filter_slope.value(),
            crate::params::FilterSlope::FourPole
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
            .filter(|(text, pos)| {
                pos.y > HEADER_HEIGHT
                    && text.chars().all(|c| {
                        c.is_ascii_uppercase() || c.is_ascii_digit() || c == ' ' || c == '/'
                    })
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
        assert_eq!(small.len(), 54, "unexpected number of sliders");

        // A bigger window: the sliders are taller and the sections are
        // wider.
        let output = run_frames(
            &ctx,
            &mut editor,
            vec![input_at(egui::vec2(1600.0, 900.0), vec![])],
        );
        let big = slider_rects(&ctx, &output);
        assert_eq!(big.len(), 54, "unexpected number of sliders");

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
