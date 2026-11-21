
mod keyboard;
mod presets;
mod theme;

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

use crate::dsp::fx::{FX_STAGE_COUNT, FxStage, stage_order, stage_order_index};
use crate::dsp::mod_feedback::{FeedSource, ModFeedback};
use crate::keyboard::{HeldKeys, KeyboardNote};
use crate::params::{ModDest, ModSource, SushiParams};
use std::cell::RefCell;

thread_local! {
    /// The module that is being drawn. A group stamps it here, so the info
    /// block can name the module of a reading without every widget knowing
    /// where it lives.
    static CURRENT_MODULE: RefCell<String> = RefCell::new(String::from("SUSHI"));
}
use crate::patch;
use keyboard::VirtualKeyboard;
use presets::{PRESETS, Rng};
use theme::palette;

// --- The fonts ---------------------------------------------------------------

/// Quicksand 500: the running text of the panel.
const FACE_BODY: &[u8] = include_bytes!("../assets/fonts/Quicksand-Medium.ttf");
/// Quicksand 600: the small labels above the controls.
const FACE_LABEL: &[u8] = include_bytes!("../assets/fonts/Quicksand-SemiBold.ttf");
/// Quicksand 700: the titles and the wordmark.
const FACE_DISPLAY: &[u8] = include_bytes!("../assets/fonts/Quicksand-Bold.ttf");
/// JetBrains Mono 500: every value of the instrument.
const MONO_VALUE: &[u8] = include_bytes!("../assets/fonts/JetBrainsMono-Medium.ttf");
/// JetBrains Mono 700: the preset display and the chain numbers.
const MONO_DISPLAY: &[u8] = include_bytes!("../assets/fonts/JetBrainsMono-Bold.ttf");
/// The family of the small labels, in the semi-bold weight.
const LABEL_FAMILY: &str = "sushi-label";
/// The family of the titles, the wordmark and the button faces.
const DISPLAY_FAMILY: &str = "sushi-display";
/// The family of the display numbers of the monospace font.
const VALUE_FAMILY: &str = "sushi-value";

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
/// Installs the type system of the panel.
///
/// The proportional family is the body weight, and the label, display and
/// value families sit on top of it with a heavier weight in front of the
/// same fallback chain. The monospace family is JetBrains Mono, so the JSON
/// preview, the numeric fields and every painted value share one set of
/// tabular figures. The default egui fonts and a system CJK font stay at the
/// end of every chain, so no glyph of a patch name can turn into a box.
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    for (name, bytes) in [
        ("face-body", FACE_BODY),
        ("face-label", FACE_LABEL),
        ("face-display", FACE_DISPLAY),
        ("mono-value", MONO_VALUE),
        ("mono-display", MONO_DISPLAY),
    ] {
        fonts.font_data.insert(
            name.to_owned(),
            Arc::new(egui::FontData::from_static(bytes)),
        );
    }

    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, "face-body".to_owned());
    fonts
        .families
        .entry(egui::FontFamily::Monospace)
        .or_default()
        .insert(0, "mono-value".to_owned());

    // Add a system CJK fallback after the bundled fonts. The file stays on
    // the user's system; the plugin only reads it at runtime.
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
        break;
    }

    let proportional = fonts
        .families
        .get(&egui::FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    // The mono fallback needs the extra glyphs of a normal text font (the
    // patch browser prints names and messages next to the code).
    let mut value_chain = fonts
        .families
        .get(&egui::FontFamily::Monospace)
        .cloned()
        .unwrap_or_default();
    let mut label_chain = proportional.clone();
    label_chain.insert(0, "face-label".to_owned());
    let mut display_chain = proportional.clone();
    display_chain.insert(0, "face-display".to_owned());
    value_chain.insert(0, "mono-display".to_owned());
    for (family, chain) in [
        (LABEL_FAMILY, label_chain),
        (DISPLAY_FAMILY, display_chain),
        (VALUE_FAMILY, value_chain),
    ] {
        fonts
            .families
            .insert(egui::FontFamily::Name(Arc::from(family)), chain);
    }

    ctx.set_fonts(fonts);
}

/// The font family of the titles and the wordmark.
fn bold() -> egui::FontFamily {
    egui::FontFamily::Name(Arc::from(DISPLAY_FAMILY))
}

/// The font family of the small labels above the controls.
fn label_family() -> egui::FontFamily {
    egui::FontFamily::Name(Arc::from(LABEL_FAMILY))
}

/// The font family of the display numbers of the preset LCD.
fn value_family() -> egui::FontFamily {
    egui::FontFamily::Name(Arc::from(VALUE_FAMILY))
}

/// Draws a text with explicit tracking. The panel tracks its small uppercase
/// labels and its titles, which is what makes them read as a type scale
/// instead of as squeezed text. Returns the rect that the text covers.
fn tracked_text(
    painter: &egui::Painter,
    pos: Pos2,
    align: Align2,
    text: &str,
    font: FontId,
    color: Color32,
    tracking: f32,
) -> Rect {
    let galley = tracked_galley(painter, text, font, color, tracking);
    let rect = align.anchor_size(pos, galley.size());
    painter.galley(rect.min, galley, color);
    rect
}

/// Lays a tracked text out without drawing it, so a header can measure its
/// title before the rect of the header is known.
fn tracked_galley(
    painter: &egui::Painter,
    text: &str,
    font: FontId,
    color: Color32,
    tracking: f32,
) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::default();
    job.append(
        text,
        0.0,
        egui::TextFormat {
            font_id: font,
            color,
            extra_letter_spacing: tracking,
            ..Default::default()
        },
    );
    painter.layout_job(job)
}

// --- Layout -----------------------------------------------------------------

pub const WINDOW_WIDTH: f32 = 960.0;
pub const WINDOW_HEIGHT: f32 = 680.0;

/// The margin around the whole panel.
const MARGIN: f32 = 8.0;
/// The height of the header.
const HEADER_HEIGHT: f32 = 48.0;
/// The gap between two sections.
const GROUP_GAP: f32 = 12.0;
/// The gap between two bands. It holds the legend of the band.
const BAND_GAP: f32 = 8.0;
/// The width of the info panel at the left of the bottom row, in the design
/// size of the panel.
const INFO_WIDTH: f32 = 300.0;
/// The gap between the info panel and the keys.
const INFO_GAP: f32 = 8.0;
/// The padding inside a section.
const GROUP_PADDING: f32 = 4.0;
/// The height of the line that holds the title of a section and the controls
/// of its header. The rule below the line sits on its bottom edge.
const TITLE_HEIGHT: f32 = 20.0;
/// The horizontal gap between two parameter columns.
const COLUMN_GAP: f32 = 4.0;
/// The gap between the wide fader slots of the lower bands. A wide section
/// gets a bank that uses its whole width, so the spacing between the faders
/// grows with the section instead of leaving an empty right edge.
const WIDE_COLUMN_GAP: f32 = 8.0;

// --- The modulation table ---------------------------------------------------

/// The height of the single header of the modulation table, in the design
/// size of the panel.
const MOD_HEADER_HEIGHT: f32 = 22.0;
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
/// The width of the black handle of an amount bar.
const MOD_BAR_HANDLE_WIDTH: f32 = 7.0;
/// How far the handle of an amount bar stays away from the ends of its track.
const MOD_BAR_INSET: f32 = 2.0;
/// The width of the live marker that shows the current modulation of a route.
const MOD_LIVE_MARKER_WIDTH: f32 = 2.0;
/// The width of the accent line that marks the selected route of the table.
const MOD_SELECTED_LINE_WIDTH: f32 = 2.0;

// --- The target inspector ---------------------------------------------------


/// The ranges that the matrix destinations reach at full amount. They mirror
/// the constants of `dsp::mod_matrix`; the inspector is read-only and must not
/// make the DSP module public just to print them.
const TARGET_PITCH_RANGE: f32 = 12.0;
const TARGET_PULSE_WIDTH_RANGE: f32 = 0.45;
const TARGET_CUTOFF_RANGE: f32 = 5.0;
const TARGET_NOISE_RANGE: f32 = 0.5;
const TARGET_FINE_RANGE: f32 = 100.0;
const TARGET_DETUNE_RANGE: f32 = 50.0;

/// The gap between the routing/target workspace and the source column of the
/// modulation page.
const MOD_WORKSPACE_GAP: f32 = 12.0;
/// The share of the modulation workspace that the routing table and the target
/// inspector use. The five source modules get the remaining third.
const MOD_LEFT_SHARE: f32 = 2.0 / 3.0;
/// The gap between two modules of the right-hand source column.
const MOD_MODULE_GAP: f32 = 6.0;
/// The relative heights of the five modules of the right-hand column. They sum
/// to the height of the four bands minus the four module gaps, so the column
/// fills the whole workspace.
const MOD_MODULE_WEIGHTS: [f32; 5] = [108.0, 108.0, 92.0, 92.0, 88.0];
/// The heights of the four rows of sections. They are the proportions of the
/// rows and are scaled with the zoom of the panel.
const ROW_HEIGHTS: [f32; 4] = [123.0, 123.0, 123.0, 123.0];

/// The widths of the sections of the first row: `VCO 1 | VCO 2 | PULSE WIDTH |
/// SOURCE MIXER`. They are the proportions of the sections and are scaled
/// with the zoom of the panel.
const ROW_A_WIDTHS: [f32; 4] = [228.0, 224.0, 124.0, 332.0];
/// The widths of the sections of the second row: `VCF | MOD ENV | AMP ENV`.
const ROW_B_WIDTHS: [f32; 3] = [348.0, 284.0, 288.0];
/// The widths of the two wide sections of the third row. The normal page
/// shows `ARP | VOICE`.
const ROW_C_WIDTHS: [f32; 2] = [472.0, 472.0];
/// The widths of the two wide sections of the fourth row. The normal page
/// shows `MACROS | VCA`.
const ROW_D_WIDTHS: [f32; 2] = [472.0, 472.0];

/// The widths of the sections of every row.
const BASE_ROW_WIDTHS: [&[f32]; 4] = [&ROW_A_WIDTHS, &ROW_B_WIDTHS, &ROW_C_WIDTHS, &ROW_D_WIDTHS];

// --- The FX rack ------------------------------------------------------------

/// The width of the chain column on the left of the FX page.
const FX_CHAIN_COLUMN_WIDTH: f32 = 196.0;
/// The gap between the chain column and the module grid.
const FX_CHAIN_COLUMN_GAP: f32 = 8.0;
/// The padding inside the chain column.
const FX_CHAIN_PADDING: f32 = 6.0;
/// The height of the caption at the top of the chain column.
const FX_CHAIN_TITLE_HEIGHT: f32 = 22.0;
/// The height of one movable chain tile.
const FX_CHAIN_TILE_HEIGHT: f32 = 48.0;
/// The gap between two chain tiles.
const FX_CHAIN_TILE_GAP: f32 = 2.0;
/// The gap between the last chain tile and the global wet fader.
const FX_CHAIN_WET_GAP: f32 = 8.0;
/// The height of the compact global wet fader.
const FX_CHAIN_WET_HEIGHT: f32 = 24.0;
/// The height of the caption above the chain preset popup.
const FX_CHAIN_PRESET_LABEL_HEIGHT: f32 = 12.0;
/// The gap between the preset caption and its popup.
const FX_CHAIN_PRESET_LABEL_GAP: f32 = 4.0;
/// The height of the chain preset popup.
const FX_CHAIN_PRESET_HEIGHT: f32 = 20.0;
/// The size of the power button of a chain tile.
const FX_CHAIN_POWER_SIZE: f32 = 16.0;
/// The width of an up/down arrow of a chain tile.
const FX_CHAIN_ARROW_WIDTH: f32 = 14.0;
/// The height of an up/down arrow of a chain tile.
const FX_CHAIN_ARROW_HEIGHT: f32 = 18.0;
/// The width of the caption of the global wet fader.
const FX_GLOBAL_LABEL_WIDTH: f32 = 66.0;
/// The width of the value readout of the global wet fader.
const FX_GLOBAL_VALUE_WIDTH: f32 = 30.0;
/// The gap between two modules of the module grid.
const FX_MODULE_GAP: f32 = 8.0;
/// The eight effect stages, in the original order of the chain.
const FX_STAGES: [&str; FX_STAGE_COUNT] = [
    "DRIVE",
    "PHASER",
    "CHORUS",
    "DELAY",
    "REVERB",
    "EQ",
    "COMPRESSOR",
    "TREMOLO",
];
/// The titles of the eight modules, row by row. The module grid is fixed:
/// reordering the chain changes the signal path, not the order of the cards.
const FX_MODULE_TITLES: [[&str; 2]; 4] = [
    ["DRIVE", "PHASER"],
    ["CHORUS", "DELAY"],
    ["REVERB", "EQ"],
    ["COMPRESSOR", "TREMOLO"],
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FxChainPreset {
    Standard,
    EqFirst,
    DriveLast,
    Ambient,
    WideEcho,
    Glue,
}

impl FxChainPreset {
    /// Every named preset, in the order of the popup.
    const ALL: [Self; 6] = [
        Self::Standard,
        Self::EqFirst,
        Self::DriveLast,
        Self::Ambient,
        Self::WideEcho,
        Self::Glue,
    ];

    /// The caption of the preset.
    fn name(self) -> &'static str {
        match self {
            Self::Standard => "STANDARD",
            Self::EqFirst => "EQ FIRST",
            Self::DriveLast => "DRIVE LAST",
            Self::Ambient => "AMBIENT",
            Self::WideEcho => "WIDE ECHO",
            Self::Glue => "GLUE",
        }
    }

    /// The full chain order of the preset. The order index in the parameter is
    /// derived from this array with `stage_order_index`, so the names can
    /// never point at a different permutation than the one they describe.
    fn stages(self) -> [FxStage; FX_STAGE_COUNT] {
        match self {
            Self::Standard => [
                FxStage::Drive,
                FxStage::Phaser,
                FxStage::Chorus,
                FxStage::Delay,
                FxStage::Reverb,
                FxStage::Eq,
                FxStage::Compressor,
                FxStage::Tremolo,
            ],
            Self::EqFirst => [
                FxStage::Eq,
                FxStage::Drive,
                FxStage::Phaser,
                FxStage::Chorus,
                FxStage::Delay,
                FxStage::Reverb,
                FxStage::Compressor,
                FxStage::Tremolo,
            ],
            Self::DriveLast => [
                FxStage::Phaser,
                FxStage::Chorus,
                FxStage::Delay,
                FxStage::Reverb,
                FxStage::Eq,
                FxStage::Drive,
                FxStage::Compressor,
                FxStage::Tremolo,
            ],
            // Reverb before delay, so the echoes feed the room.
            Self::Ambient => [
                FxStage::Drive,
                FxStage::Phaser,
                FxStage::Chorus,
                FxStage::Reverb,
                FxStage::Delay,
                FxStage::Eq,
                FxStage::Compressor,
                FxStage::Tremolo,
            ],
            // Delay before chorus, so the chorus smears the repeats.
            Self::WideEcho => [
                FxStage::Drive,
                FxStage::Phaser,
                FxStage::Delay,
                FxStage::Chorus,
                FxStage::Reverb,
                FxStage::Eq,
                FxStage::Compressor,
                FxStage::Tremolo,
            ],
            // The compressor glues the wet chain together before the final
            // tone shaping; the tremolo stays at the very end.
            Self::Glue => [
                FxStage::Drive,
                FxStage::Phaser,
                FxStage::Chorus,
                FxStage::Delay,
                FxStage::Reverb,
                FxStage::Compressor,
                FxStage::Eq,
                FxStage::Tremolo,
            ],
        }
    }

    /// The permutation index that selects this preset.
    fn order(self) -> i32 {
        stage_order_index(self.stages())
    }
}

/// The named preset that a permutation index matches, if any.
fn fx_chain_preset_at(order: i32) -> Option<FxChainPreset> {
    let stages = stage_order(order);
    FxChainPreset::ALL
        .into_iter()
        .find(|preset| preset.stages() == stages)
}

/// The caption of the preset popup. Any order that is not one of the named
/// presets is a custom chain.
fn fx_chain_preset_name(order: i32) -> &'static str {
    fx_chain_preset_at(order).map_or("CUSTOM", FxChainPreset::name)
}

/// The name of one stage on the chain tiles.
fn fx_stage_name(stage: FxStage) -> &'static str {
    FX_STAGES[stage.index()]
}

/// The whole FX rack: the space between the header and the footer.
fn fx_rack_rect(layout: &Layout) -> Rect {
    let top = layout.scale(layout.base_rows[0]).left_top();
    let bottom = layout.scale(layout.base_rows[3]).right_bottom();
    Rect::from_min_max(top, bottom)
}

/// The column that holds the chain tiles, the global wet fader and the preset
/// popup.
fn fx_chain_column_rect(rack: Rect, zoom: f32) -> Rect {
    Rect::from_min_size(
        rack.min,
        egui::vec2(FX_CHAIN_COLUMN_WIDTH * zoom, rack.height()),
    )
}

/// The inner rectangle of the chain column.
fn fx_chain_inner_rect(rack: Rect, zoom: f32) -> Rect {
    fx_chain_column_rect(rack, zoom).shrink2(egui::vec2(FX_CHAIN_PADDING, FX_CHAIN_PADDING) * zoom)
}

/// The rect of the title of the chain column.
fn fx_chain_title_rect(rack: Rect, zoom: f32) -> Rect {
    let inner = fx_chain_inner_rect(rack, zoom);
    Rect::from_min_size(
        inner.min,
        egui::vec2(inner.width(), FX_CHAIN_TITLE_HEIGHT * zoom),
    )
}

/// The tile at a position of the chain, `0` being the first stage in the
/// signal path.
fn fx_chain_tile_rect(rack: Rect, position: usize, zoom: f32) -> Rect {
    let inner = fx_chain_inner_rect(rack, zoom);
    let top = inner.top()
        + (FX_CHAIN_TITLE_HEIGHT + FX_CHAIN_TILE_GAP) * zoom
        + position as f32 * (FX_CHAIN_TILE_HEIGHT + FX_CHAIN_TILE_GAP) * zoom;
    Rect::from_min_size(
        egui::pos2(inner.left(), top),
        egui::vec2(inner.width(), FX_CHAIN_TILE_HEIGHT * zoom),
    )
}

/// The rect of the compact global wet fader.
fn fx_chain_wet_rect(rack: Rect, zoom: f32) -> Rect {
    let inner = fx_chain_inner_rect(rack, zoom);
    let top = inner.top()
        + (FX_CHAIN_TITLE_HEIGHT + FX_CHAIN_TILE_GAP) * zoom
        + FX_STAGES.len() as f32 * FX_CHAIN_TILE_HEIGHT * zoom
        + (FX_STAGES.len() as f32 - 1.0) * FX_CHAIN_TILE_GAP * zoom
        + FX_CHAIN_WET_GAP * zoom;
    Rect::from_min_size(
        egui::pos2(inner.left(), top),
        egui::vec2(inner.width(), FX_CHAIN_WET_HEIGHT * zoom),
    )
}

/// The caption and the popup of the chain presets, anchored to the bottom of
/// the chain column.
fn fx_chain_preset_rects(rack: Rect, zoom: f32) -> (Rect, Rect) {
    let inner = fx_chain_inner_rect(rack, zoom);
    let popup = Rect::from_min_size(
        egui::pos2(inner.left(), inner.bottom() - FX_CHAIN_PRESET_HEIGHT * zoom),
        egui::vec2(inner.width(), FX_CHAIN_PRESET_HEIGHT * zoom),
    );
    let label = Rect::from_min_size(
        egui::pos2(
            inner.left(),
            popup.top() - (FX_CHAIN_PRESET_LABEL_GAP + FX_CHAIN_PRESET_LABEL_HEIGHT) * zoom,
        ),
        egui::vec2(inner.width(), FX_CHAIN_PRESET_LABEL_HEIGHT * zoom),
    );
    (label, popup)
}

/// The eight module cards of the fixed 2 x 4 grid.
fn fx_module_rects(rack: Rect, zoom: f32) -> [Rect; FX_STAGE_COUNT] {
    let left = rack.left() + (FX_CHAIN_COLUMN_WIDTH + FX_CHAIN_COLUMN_GAP) * zoom;
    let gap = FX_MODULE_GAP * zoom;
    let width = (rack.right() - left - gap) * 0.5;
    let height = (rack.height() - 3.0 * gap) / 4.0;
    let mut rects = [Rect::ZERO; FX_STAGE_COUNT];
    for (index, rect) in rects.iter_mut().enumerate() {
        let column = index % 2;
        let row = index / 2;
        *rect = Rect::from_min_size(
            egui::pos2(
                left + column as f32 * (width + gap),
                rack.top() + row as f32 * (height + gap),
            ),
            egui::vec2(width, height),
        );
    }
    rects
}

#[derive(Clone, Copy)]
struct FxChainTileParts {
    drag: Rect,
    number: Rect,
    name: Rect,
    up: Rect,
    down: Rect,
    power: Rect,
}

/// The internal geometry of one chain tile.
fn fx_chain_tile_parts(tile: Rect, zoom: f32) -> FxChainTileParts {
    let pad = 4.0 * zoom;
    let power_size = FX_CHAIN_POWER_SIZE * zoom;
    let power = Rect::from_min_size(
        egui::pos2(
            tile.right() - pad - power_size,
            tile.center().y - power_size * 0.5,
        ),
        egui::vec2(power_size, power_size),
    );
    let arrow = egui::vec2(FX_CHAIN_ARROW_WIDTH, FX_CHAIN_ARROW_HEIGHT) * zoom;
    let down = Rect::from_center_size(
        egui::pos2(power.left() - 4.0 * zoom - arrow.x * 0.5, tile.center().y),
        arrow,
    );
    let up = Rect::from_center_size(
        egui::pos2(down.left() - 2.0 * zoom - arrow.x * 0.5, tile.center().y),
        arrow,
    );

    let drag = Rect::from_min_max(tile.min, egui::pos2(up.left() - 4.0 * zoom, tile.bottom()));
    let number = Rect::from_min_size(
        egui::pos2(tile.left() + 12.0 * zoom, tile.top()),
        egui::vec2(16.0 * zoom, tile.height()),
    );
    let name = Rect::from_min_max(
        egui::pos2(number.right() + 3.0 * zoom, tile.top()),
        egui::pos2(drag.right() - 4.0 * zoom, tile.bottom()),
    );
    FxChainTileParts {
        drag,
        number,
        name,
        up,
        down,
        power,
    }
}

/// The first position at or above the pointer in a list of tiles. `len` means
/// that the pointer is below every tile.
fn fx_chain_drop_slot(pointer_y: f32, tiles: &[Rect]) -> usize {
    tiles
        .iter()
        .position(|tile| pointer_y < tile.center().y)
        .unwrap_or(tiles.len())
}

/// The position that a dragged tile takes when it is dropped at a slot. The
/// tile is removed before it is inserted, so a source above the slot moves
/// every later position up by one.
fn fx_chain_drop_position(source: usize, slot: usize, count: usize) -> usize {
    if count == 0 {
        return 0;
    }
    let target = if source < slot { slot - 1 } else { slot };
    target.min(count - 1)
}

#[derive(Clone, Copy, Debug, Default)]
struct FxChainDrag {
    /// The position of the tile in the chain.
    source: usize,
}

// --- Widget metrics ---------------------------------------------------------

/// The width of a vertical slider.
const SLIDER_WIDTH: f32 = 32.0;
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
/// The width of a page button (MOD / FX) in the header.
const PAGE_BUTTON_WIDTH: f32 = 44.0;
/// The width of the `NOW` caption of a modulation row.
const MOD_NOW_LABEL_WIDTH: f32 = 30.0;
/// The column of the centre value of the NOW reading.
const MOD_NOW_BASE_WIDTH: f32 = 84.0;
/// The width of the oscilloscope at the right end of the header.
const SCOPE_WIDTH: f32 = 190.0;
/// The number of cells of the level meter of the header.
const LEVEL_CELLS: usize = 28;
/// The gap between two cells of the level meter.
const LEVEL_CELL_GAP: f32 = 2.0;
/// The height of a cell of the level meter.
const LEVEL_CELL_HEIGHT: f32 = 8.0;
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

// --- The layout of the panel ------------------------------------------------

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
    fn for_rect(rect: Rect, desired: f32) -> Option<Self> {
        if rect.width() < WINDOW_WIDTH * MIN_ZOOM || rect.height() < WINDOW_HEIGHT * MIN_ZOOM {
            return None;
        }

        // The panel is drawn at the level the user picked in the right-click
        // menu. It never stretches past that level, even when the window of
        // the host is bigger; it only falls back to the window when the window
        // is too small to hold the level.
        let fits = (rect.width() / WINDOW_WIDTH).min(rect.height() / WINDOW_HEIGHT);
        let zoom = desired.min(fits);

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

    /// The number of modules of the right-hand column of the modulation page.
    fn mod_module_count() -> usize {
        MOD_MODULE_WEIGHTS.len()
    }

    /// The rect of the header.
    fn header(&self) -> Rect {
        self.scale(self.base_header)
    }

    /// The rect of a section of a row. The sections of a row share its width
    /// and keep their proportions.
    fn group(&self, row: usize, index: usize) -> Rect {
        self.group_with(BASE_ROW_WIDTHS[row], row, index)
    }

    /// The part of the panel that the modulation page uses: everything between
    /// the header and the footer, with the same side margins as the main rows.
    fn mod_workspace(&self) -> Rect {
        self.scale(Rect::from_min_max(
            egui::pos2(MARGIN, self.base_rows[0].top()),
            egui::pos2(WINDOW_WIDTH - MARGIN, self.base_rows[3].bottom()),
        ))
    }

    /// The left two thirds of the modulation page: the routing table over the
    /// target inspector.
    fn mod_left(&self) -> Rect {
        let workspace = self.mod_workspace();
        let gap = MOD_WORKSPACE_GAP * self.zoom;
        let width = (workspace.width() - gap) * MOD_LEFT_SHARE;
        Rect::from_min_size(workspace.min, egui::vec2(width, workspace.height()))
    }

    /// The right third of the modulation page: the five compact source
    /// modules, stacked from the top to the bottom of the workspace.
    fn mod_right(&self) -> Rect {
        let workspace = self.mod_workspace();
        let left = self.mod_left();
        let gap = MOD_WORKSPACE_GAP * self.zoom;
        Rect::from_min_max(
            egui::pos2(left.right() + gap, workspace.top()),
            workspace.max,
        )
    }

    /// The routing table of the modulation page.
    fn mod_table(&self) -> Rect {
        // The table fills the whole left column: the readings of every route
        // live in its own row now, so there is no inspector under it.
        self.mod_left()
    }

    /// The rect of one of the five modules of the right-hand column.
    fn mod_module(&self, index: usize) -> Rect {
        debug_assert!(index < Self::mod_module_count());
        let column = self.mod_right();
        let gap = MOD_MODULE_GAP * self.zoom;
        let available = (column.height() - gap * (Self::mod_module_count() as f32 - 1.0)).max(0.0);
        let total: f32 = MOD_MODULE_WEIGHTS.iter().sum();
        let before: f32 = MOD_MODULE_WEIGHTS[..index].iter().sum();
        let top = column.top() + gap * index as f32 + available * before / total;
        let height = available * MOD_MODULE_WEIGHTS[index] / total;
        Rect::from_min_size(
            egui::pos2(column.left(), top),
            egui::vec2(column.width(), height),
        )
    }

    /// The rect of the keyboard strip, between the last band and the footer.
    /// The bottom row: the info panel on the left and the keys on the right.
    fn bottom(&self) -> Rect {
        let top = self.base_rows[3].bottom() + BAND_GAP;
        self.scale(Rect::from_min_size(
            egui::pos2(MARGIN, top),
            egui::vec2(WINDOW_WIDTH - 2.0 * MARGIN, WINDOW_HEIGHT - MARGIN - top),
        ))
    }

    /// The info panel at the left of the bottom row.
    fn info(&self) -> Rect {
        let row = self.bottom();
        Rect::from_min_size(row.min, egui::vec2(INFO_WIDTH * self.zoom, row.height()))
    }

    /// The keys at the right of the bottom row.
    fn keyboard(&self) -> Rect {
        let row = self.bottom();
        let left = row.left() + (INFO_WIDTH + INFO_GAP) * self.zoom;
        Rect::from_min_max(
            egui::pos2(left, row.top()),
            egui::pos2(row.right(), row.bottom()),
        )
    }

    /// The rect of one section of a row with explicit proportions.
    fn group_with(&self, widths: &[f32], row: usize, index: usize) -> Rect {
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
}

/// A linear peak mapped onto the 60 dB scale of the meters.
fn db_norm(value: f32) -> f32 {
    if value <= 0.000_001 {
        return 0.0;
    }
    ((20.0 * value.log10() + 60.0) / 60.0).clamp(0.0, 1.0)
}

/// The level meter of the header: a row of cells that is filled up to the
/// level of the sound. A cell keeps the color of its own position, so a loud
/// signal also pushes the lit cells into the orange and into the red.
fn level_meter(painter: &egui::Painter, rect: Rect, peak: f32, zoom: f32) {
    let cells = LEVEL_CELLS;
    let gap = LEVEL_CELL_GAP * zoom;
    let cell_width = ((rect.width() - gap * (cells as f32 - 1.0)) / cells as f32).max(1.0);
    let height = LEVEL_CELL_HEIGHT * zoom;
    let top = rect.center().y - height * 0.5;
    let lit = (db_norm(peak) * cells as f32).round() as usize;

    for index in 0..cells {
        let cell = Rect::from_min_size(
            egui::pos2(rect.left() + index as f32 * (cell_width + gap), top),
            egui::vec2(cell_width, height),
        );
        let share = (index as f32 + 1.0) / cells as f32;
        let color = if index < lit {
            if share > 0.94 {
                palette().status_error
            } else if share > 0.80 {
                palette().accent
            } else {
                palette().led_on
            }
        } else {
            palette().track
        };
        painter.rect_filled(cell, CornerRadius::ZERO, color);
    }
}

/// The size of a font of the panel at the given zoom.
fn font_size(size: f32, zoom: f32) -> f32 {
    size * zoom
}

// --- The editor -------------------------------------------------------------



#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct PersistedEditorState {
    preset_index: usize,
    patch_name: Option<String>,
    patch_path: Option<String>,
    dirty: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Page {
    #[default]
    Main,
    /// The MOD 2.0 routing table.
    Mod,
    /// The two-row FX rack.
    Fx,
}

/// The state that only the editor needs.
struct EditorState {
    /// The ballistic level of the velocity strips, per route.
    velocity_meter: [f32; crate::params::MOD_SLOTS],
    /// The animated hue of the key strips, per route.
    key_hue: [f32; crate::params::MOD_SLOTS],
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
    /// The route of the modulation table that the target inspector shows.
    /// Selecting a route is a view-only action, so it never marks the sound
    /// dirty and it is kept while the route is edited.
    mod_selected: usize,
    /// The parameter and value under the pointer, or the last one seen.
    readout: Option<(String, String, String)>,
    /// The peak level of the samples of the last frame.
    activity: f32,
    /// Whether the samples of the last frame were clipping.
    clipping: f32,
    /// The widgets that were drawn in the last frame.
        /// The places of the check boxes of the last frame.
        /// The places of the radio buttons of the last frame.
        /// The places of the popup buttons of the last frame.
        /// The places of the amount bars of the modulation page of the last frame.
        /// The places of the eight route rows of the modulation page of the last
    /// frame, so that a test can click anywhere on a row.
        /// The range bar, its current marker and its live dot in the target
    /// inspector.
                    /// The chain tile that is currently being dragged, if any.
    fx_chain_drag: Option<FxChainDrag>,
    /// The keyboard strip under the bands: its octave and the notes that the
    /// panel itself started.
    keyboard: VirtualKeyboard,
    /// The tile, drag handle, arrows and power button of every stage, keyed by
    /// the original stage index. The tests click these directly, even after
    /// the chain was reordered.
                        /// The track of the compact global wet fader.
        /// The caption and the popup of the chain presets.
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
    /// Whether the patch browser has to ask before it deletes a file.
    patch_confirm_delete: bool,
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
    }

pub struct SushiEditor {
    params: Arc<SushiParams>,
    scope_rx: rtrb::Consumer<f32>,
    /// The notes that the panel plays travel to the audio thread here.
    keyboard_tx: rtrb::Producer<KeyboardNote>,
    /// The keys that the synthesizer plays, for the lights of the strip.
    held_keys: Arc<HeldKeys>,
    history: VecDeque<f32>,
    gui_ctx: Option<GuiContext>,
    state: EditorState,
    /// The persisted identity that was last seen by this editor.
    last_persisted: PersistedEditorState,
    /// The zoom of the panel, as it was last applied to the styles.
    zoom: f32,
    /// The index in [`ZOOM_LEVELS`] of the level picked in the right-click menu.
    zoom_level: usize,
    /// The level picked in the menu, as a cap for [`Layout::for_rect`].
    zoom_cap: f32,
    /// How the panel asks the host for a bigger or a smaller window.
    resize_window: Option<Arc<dyn Fn(f32, f32) + Send + Sync>>,
    /// Whether the panel snaps to a level instead of following the window
    /// continuously. The geometry tests turn this off to measure the
    /// continuous layout.
    snap_zoom: bool,
    /// Where the right-click zoom menu is open, if it is open.
    zoom_menu: Option<(Pos2, MenuPanel)>,
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
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        params: Arc<SushiParams>,
        scope_rx: rtrb::Consumer<f32>,
        keyboard_tx: rtrb::Producer<KeyboardNote>,
        held_keys: Arc<HeldKeys>,
        arp_led: Arc<AtomicUsize>,
        arp_position: Arc<AtomicUsize>,
        voice_count: Arc<AtomicUsize>,
        mod_feedback: Arc<ModFeedback>,
        resize_window: Option<Arc<dyn Fn(f32, f32) + Send + Sync>>,
    ) -> Self {
        let mut editor = Self {
            params,
            scope_rx,
            keyboard_tx,
            held_keys,
            arp_led,
            arp_position,
            voice_count,
            mod_feedback,
            history: VecDeque::with_capacity(HISTORY_LEN),
            gui_ctx: None,
            last_persisted: PersistedEditorState::default(),
            zoom: 1.0,
            zoom_level: DEFAULT_ZOOM_LEVEL,
            zoom_cap: ZOOM_LEVELS[DEFAULT_ZOOM_LEVEL],
            resize_window,
            snap_zoom: true,
            zoom_menu: None,
            state: EditorState {
                velocity_meter: [0.0; crate::params::MOD_SLOTS],
                key_hue: [210.0; crate::params::MOD_SLOTS],
                texts: HashMap::new(),
                gestures: HashSet::new(),
                preset_index: 0,
                dirty: false,
                rng: Rng::new(0x2545_F491),
                arp_counter: 0,
                arp_step: 0,
                page: Page::Main,
                mod_selected: 0,
                readout: None,
                activity: 0.0,
                clipping: 0.0,
                                                                                                                                                                keyboard: VirtualKeyboard::default(),
                fx_chain_drag: None,
                                                                                                                                                patch_config: patch::PatchConfig::default(),
                patch_open: false,
                patch_dir_text: String::new(),
                patch_name: String::new(),
                patch_search: String::new(),
                patch_files: Vec::new(),
                patch_status: String::new(),
                patch_status_error: false,
                patch_confirm_overwrite: false,
                patch_confirm_delete: false,
                patch_active: None,
                patch_active_path: None,
                patch_preview: String::new(),
                patch_preview_title: String::new(),
            },
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
        self.state.patch_confirm_delete = false;
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

    /// Applies a host state change (project reload, another editor) to
    /// the open editor without touching the patch folder or the search.
    fn apply_persisted_state(&mut self, persisted: PersistedEditorState) {
        self.state.preset_index = persisted.preset_index.min(PRESETS.len() - 1);
        self.state.dirty = persisted.dirty;
        self.state.patch_confirm_overwrite = false;
        self.state.patch_confirm_delete = false;
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

impl Drop for SushiEditor {
    /// Releases the keys of the panel when the window closes, so a key that
    /// was held while the editor disappeared cannot leave a stuck voice.
    fn drop(&mut self) {
        let keyboard_tx = &mut self.keyboard_tx;
        self.state.keyboard.release_all(|note, velocity, on| {
            let _ = keyboard_tx.push(KeyboardNote {
                key: note,
                velocity,
                on,
            });
        });
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
        // 120 fps: the meters and the trace are the only animation, and the
        // panel should follow the sound without a visible step.
        ui.ctx().request_repaint_after(Duration::from_millis(8));

        let full = ui.max_rect();
        ui.painter()
            .rect_filled(full, CornerRadius::ZERO, palette().bg);

        // The panel needs at least the size it was made for. A smaller window
        // only shows a sad face, so that nothing is cut in half.
        // The background of the panel is the one that answers a right click
        // with the zoom menu. It is registered before every widget, so any
        // widget that is drawn later sits on top of it and keeps its own
        // right click (the value editor of a slider, for instance).
        let background = ui.interact(
            full,
            ui.scope_id().with("sushi-background"),
            Sense::click(),
        );
        if background.secondary_clicked() {
            self.zoom_menu = ui
                .input(|input| input.pointer.interact_pos())
                .map(|pos| (pos, MenuPanel::Main));
        } else if background.clicked() {
            self.zoom_menu = None;
        }

        // The panel is always drawn at one of the levels of the menu: the
        // level the user picked, or the largest level that still fits the
        // window. Dragging the window border does not scale the panel, it only
        // makes higher levels reachable.
        let desired = if self.snap_zoom {
            let fits = (full.width() / WINDOW_WIDTH).min(full.height() / WINDOW_HEIGHT);
            ZOOM_LEVELS
                .iter()
                .rev()
                .copied()
                .find(|level| *level <= fits + 1e-3)
                .unwrap_or(MIN_ZOOM)
                .min(self.zoom_cap)
        } else {
            self.zoom_cap
        };
        let Some(layout) = Layout::for_rect(full, desired) else {
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

        {
            // Split the borrow of the editor, so that the parameters and the
            // state of the panel can be used at the same time.
            let params = self.params.as_ref();
            let history = &self.history;
            let mod_feedback = self.mod_feedback.as_ref();
            let mut ctx = Ctx {
                params,
                setter: &setter,
                state: &mut self.state,
                history,
                mod_feedback,
                slider_width: layout.slider_width(),
                zoom: layout.zoom,
                wave_history: Vec::with_capacity(
                    crate::dsp::mod_feedback::HISTORY_LENGTH,
                ),
                span_history: Vec::with_capacity(
                    crate::dsp::mod_feedback::HISTORY_LENGTH,
                ),
            };

            if ctx.state.patch_open {
                // The browser stops above the keyboard strip, so a patch can
                // be played and auditioned while it is browsed. The strip is
                // drawn after the page, so it is never covered.
                let page =
                    Rect::from_min_max(full.min, egui::pos2(full.right(), layout.keyboard().top()));
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .id_salt("sushi-patch-page-host")
                        .max_rect(page)
                        .layout(EguiLayout::top_down(Align::Min)),
                    |ui| ctx.patch_page(ui),
                );
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
                    Page::Mod => mod_page(ui, &layout, &mut ctx),
                    Page::Fx => row_fx(ui, &layout, &mut ctx),
                }
            }

            draw_info_bar(ui, &layout, ctx.state, &self.voice_count);
        }

        // The keyboard strip sits under the bands and above the footer. It is
        // drawn after the pages, so its keys are never covered by a band, and
        // it stays alive while the patch browser is open, so every patch can
        // be auditioned before it is loaded.
        let keyboard_rect = layout.keyboard();
        let held_keys = self.held_keys.clone();
        let keyboard_tx = &mut self.keyboard_tx;
        let keyboard = &mut self.state.keyboard;
        let send = |note: u8, velocity: f32, on: bool| {
            let _ = keyboard_tx.push(KeyboardNote {
                key: note,
                velocity,
                on,
            });
        };
        keyboard.show(ui, keyboard_rect, layout.zoom, &held_keys, send);

        // The menu is drawn last, so it covers everything.
        if let Some((pos, panel)) = self.zoom_menu {
            match panel_menu(ui, pos, layout.zoom, panel, self.zoom_level) {
                Some(MenuAction::Zoom(level)) => {
                    self.zoom_level = level;
                    self.zoom_cap = ZOOM_LEVELS[level];
                    self.zoom_menu = None;
                    // The window of the host has to grow with the level,
                    // otherwise the panel could never be drawn bigger.
                    if let Some(resize) = &self.resize_window {
                        resize(
                            WINDOW_WIDTH * ZOOM_LEVELS[level],
                            WINDOW_HEIGHT * ZOOM_LEVELS[level],
                        );
                    }
                    ui.ctx().request_repaint();
                }
                Some(MenuAction::Open(panel)) => {
                    self.zoom_menu = Some((pos, panel));
                    ui.ctx().request_repaint();
                }
                Some(MenuAction::Close) => self.zoom_menu = None,
                None => {}
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum MenuPanel {
    /// `Zoom` and `Shortcuts`.
    Main,
    /// The zoom levels, opened from `Zoom`.
    Zoom,
    /// The list of mouse and keyboard shortcuts.
    Help,
}

#[derive(Clone)]
pub enum MenuAction {
    Zoom(usize),
    Open(MenuPanel),
    Close,
}

/// The lines of the shortcut panel: a key and what it does.
const SHORTCUTS: [(&str, &str); 8] = [
    ("DRAG", "change a value"),
    ("WHEEL", "change a value"),
    ("SHIFT+WHEEL", "fine steps"),
    ("RIGHT-CLICK", "type a value"),
    ("DBL-CLICK", "reset a value"),
    ("A S D F ...", "play the white keys"),
    ("W E T Y U O P", "play the black keys"),
    ("Z / X", "octave down / up"),
];

/// Draws the menu of the panel and reports what was clicked.
///
/// The parent panel always stays on screen: `Zoom` and `Shortcuts` open a
/// second panel next to their row, so the item that was clicked never
/// disappears from under the pointer.
fn panel_menu(ui: &mut egui::Ui, pos: Pos2, zoom: f32, panel: MenuPanel, current: usize) -> Option<MenuAction> {
    let row = 22.0 * zoom;
    let pad = 4.0 * zoom;
    let screen = ui.ctx().content_rect();
    let painter = ui.painter();

    let main: [(&str, MenuPanel); 2] = [
        ("Zoom", MenuPanel::Zoom),
        ("Shortcuts", MenuPanel::Help),
    ];
    let main_w = 128.0 * zoom;
    let main_h = row * main.len() as f32 + 2.0 * pad;
    let main_rect = Rect::from_min_size(
        egui::pos2(
            pos.x.clamp(screen.left() + 4.0, (screen.right() - main_w - 4.0).max(4.0)),
            pos.y.clamp(screen.top() + 4.0, (screen.bottom() - main_h - 4.0).max(4.0)),
        ),
        egui::vec2(main_w, main_h),
    );
    flat_menu(painter, main_rect);

    let mut action = None;
    for (index, (label, opens)) in main.iter().enumerate() {
        let cell = Rect::from_min_size(
            egui::pos2(main_rect.left() + pad, main_rect.top() + pad + row * index as f32),
            egui::vec2(main_w - 2.0 * pad, row),
        );
        let response = ui.interact(cell, ui.scope_id().with(("sushi-menu", index)), Sense::click());
        let open = *opens == panel;
        if open {
            painter.rect_filled(cell, CornerRadius::ZERO, palette().button_active);
        } else if response.hovered() {
            painter.rect_filled(cell, CornerRadius::ZERO, palette().track);
        }
        painter.text(
            egui::pos2(cell.left() + 6.0 * zoom, cell.center().y),
            Align2::LEFT_CENTER,
            *label,
            FontId::new(font_size(10.5, zoom), label_family()),
            if open { palette().accent } else { palette().muted },
        );
        // The chevron of a submenu, drawn with two lines.
        let (cx, cy) = (cell.right() - 10.0 * zoom, cell.center().y);
        for (from, to) in [
            ((cx - 2.0 * zoom, cy - 3.5 * zoom), (cx + 1.0 * zoom, cy)),
            ((cx + 1.0 * zoom, cy), (cx - 2.0 * zoom, cy + 3.5 * zoom)),
        ] {
            painter.line_segment(
                [egui::pos2(from.0, from.1), egui::pos2(to.0, to.1)],
                Stroke::new(1.0 * zoom, palette().muted),
            );
        }
        if response.clicked() {
            action = Some(MenuAction::Open(*opens));
        }
    }

    // The submenu, anchored on the row of the panel that is open.
    if panel != MenuPanel::Main {
        let entries: Vec<(String, Option<MenuAction>, bool)> = match panel {
            MenuPanel::Zoom => ZOOM_LEVELS
                .iter()
                .enumerate()
                .map(|(index, level)| {
                    (
                        format!("{:.0}%", level * 100.0),
                        Some(MenuAction::Zoom(index)),
                        index == current,
                    )
                })
                .collect(),
            _ => SHORTCUTS
                .iter()
                .map(|(key, what)| (format!("{key}  {what}"), None, false))
                .collect(),
        };
        let sub_w = if panel == MenuPanel::Zoom { 78.0 * zoom } else { 250.0 * zoom };
        let sub_h = row * entries.len() as f32 + 2.0 * pad;
        let anchor_row = if panel == MenuPanel::Zoom { 0.0 } else { 1.0 };
        let mut sub_x = main_rect.right();
        if sub_x + sub_w > screen.right() - 4.0 {
            sub_x = (main_rect.left() - sub_w).max(screen.left() + 4.0);
        }
        let sub_rect = Rect::from_min_size(
            egui::pos2(
                sub_x,
                (main_rect.top() + pad + row * anchor_row)
                    .clamp(screen.top() + 4.0, (screen.bottom() - sub_h - 4.0).max(4.0)),
            ),
            egui::vec2(sub_w, sub_h),
        );
        flat_menu(painter, sub_rect);

        for (index, (label, item_action, checked)) in entries.iter().enumerate() {
            let cell = Rect::from_min_size(
                egui::pos2(sub_rect.left() + pad, sub_rect.top() + pad + row * index as f32),
                egui::vec2(sub_w - 2.0 * pad, row),
            );
            let response = ui.interact(
                cell,
                ui.scope_id().with(("sushi-submenu", panel as u8, index)),
                if item_action.is_some() { Sense::click() } else { Sense::hover() },
            );
            if *checked {
                painter.rect_filled(cell, CornerRadius::ZERO, palette().button_active);
            } else if response.hovered() && item_action.is_some() {
                painter.rect_filled(cell, CornerRadius::ZERO, palette().track);
            }
            painter.text(
                egui::pos2(cell.left() + 6.0 * zoom, cell.center().y),
                Align2::LEFT_CENTER,
                label,
                FontId::new(
                    font_size(if panel == MenuPanel::Zoom { 10.5 } else { 9.0 }, zoom),
                    if panel == MenuPanel::Zoom {
                        label_family()
                    } else {
                        egui::FontFamily::Monospace
                    },
                ),
                if *checked {
                    palette().accent
                } else if response.hovered() {
                    palette().text
                } else {
                    palette().muted
                },
            );
            if response.clicked() {
                action = item_action.clone();
            }
        }
    }

    // A click outside of both panels closes the menu.
    if action.is_none() && ui.input(|input| input.pointer.any_click()) {
        if let Some(pointer) = ui.input(|input| input.pointer.interact_pos()) {
            if !main_rect.contains(pointer) {
                action = Some(MenuAction::Close);
            }
        }
    }
    action
}

/// The flat plate of one menu panel.
fn flat_menu(painter: &egui::Painter, rect: Rect) {
    painter.rect_filled(rect, CornerRadius::ZERO, palette().field_bg);
    painter.rect_stroke(
        rect,
        CornerRadius::ZERO,
        Stroke::new(1.0, palette().line),
        egui::StrokeKind::Inside,
    );
}



/// The info panel at the left of the bottom row.
///
/// One line: the name of the last parameter on the left and its value on the
/// right, both as large as the block allows, and the number of voices that are
/// sounding along the bottom edge.
fn draw_info_bar(ui: &mut egui::Ui, layout: &Layout, state: &EditorState, voice_count: &AtomicUsize) {
    let zoom = layout.zoom;
    let rect = layout.info();
    let painter = ui.painter_at(rect);
    flat_box(&painter, rect, palette().paper);
    let inner = rect.shrink(10.0 * zoom);

    let (module, label, value) = state
        .readout
        .clone()
        .unwrap_or_else(|| ("PATCH".to_string(), "PRESET".to_string(), display_text(state)));

    // The module that the reading comes from, in the green of the readouts, so
    // a bare control name never has to be guessed.
    tracked_text(
        &painter,
        egui::pos2(inner.left(), inner.top() + 11.0 * zoom),
        Align2::LEFT_BOTTOM,
        &truncate_name(&module, 18),
        FontId::new(font_size(10.0, zoom), label_family()),
        palette().led_on,
        0.9 * zoom,
    );

    // The label and the value share one baseline: the label on the left, the
    // value flush right, so the block reads like one line of an instrument.
    // The three rows of the block — module, reading, voices — sit on equal
    // thirds of it instead of leaving a hole in the middle.
    let baseline = inner.top() + (inner.height() * 0.66).max(26.0 * zoom);
    tracked_text(
        &painter,
        egui::pos2(inner.left(), baseline),
        Align2::LEFT_BOTTOM,
        &truncate_name(&label, 16),
        FontId::new(font_size(14.0, zoom), label_family()),
        palette().title_blue,
        0.8 * zoom,
    );
    let value_text = truncate_name(&value, 14);
    let value_font = FontId::new(font_size(21.0, zoom), value_family());
    let value_width = text_width(&painter, &value_text, &value_font);
    painter.text(
        egui::pos2(inner.right() - value_width, baseline),
        Align2::LEFT_BOTTOM,
        value_text,
        value_font,
        palette().text,
    );

    // The voices along the bottom edge of the block: the light bar of the old
    // faceplate, one square per voice.
    let footer = inner.bottom();
    let label_font = FontId::new(font_size(9.5, zoom), label_family());
    tracked_text(
        &painter,
        egui::pos2(inner.left(), footer),
        Align2::LEFT_BOTTOM,
        "VOICES",
        label_font.clone(),
        palette().muted,
        0.9 * zoom,
    );
    let label_width = text_width(&painter, "VOICES", &label_font) + 0.9 * zoom * 5.0;
    voice_leds(
        &painter,
        Rect::from_min_max(
            egui::pos2(inner.left() + label_width + 8.0 * zoom, footer - 10.0 * zoom),
            egui::pos2(inner.right(), footer),
        ),
        voice_count.load(Ordering::Relaxed),
        zoom,
    );
}

/// The light bar of the voice count: one square per voice that the panel can
/// play, lit for every voice that is sounding right now.
fn voice_leds(painter: &egui::Painter, rect: Rect, voices: usize, zoom: f32) {
    // The cells share the whole width of the row, so the bar reads as one
    // line of light instead of a short strip in the corner.
    let gap = 3.0 * zoom;
    let cell = ((rect.width() - gap * (VOICE_LEDS as f32 - 1.0)) / VOICE_LEDS as f32).max(1.0);
    let height = rect.height();
    let lit = voices.min(VOICE_LEDS);
    for index in 0..VOICE_LEDS {
        let x = rect.left() + index as f32 * (cell + gap);
        let square = Rect::from_min_size(
            egui::pos2(x, rect.bottom() - height),
            egui::vec2(cell, height),
        );
        painter.rect_filled(
            square,
            CornerRadius::ZERO,
            if index < lit {
                palette().led_on
            } else {
                palette().track
            },
        );
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
    let smallest = ZOOM_LEVELS[0];
    let hint = format!(
        "MINIMUM {} x {}",
        (WINDOW_WIDTH * smallest) as i32,
        (WINDOW_HEIGHT * smallest) as i32
    );
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

/// The zoom levels of the panel, in the order of the right-click menu. A level
/// is a *cap*: the panel never grows past the chosen level, and it only shrinks
/// below it when the window of the host is smaller than the level.
pub const ZOOM_LEVELS: [f32; 8] = [0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 2.5, 3.0];
/// The level the panel starts on. The editor opens at exactly this level, so
/// the panel fills the window on the first frame.
const DEFAULT_ZOOM_LEVEL: usize = 4;
/// The same level as a plain scale, for the initial size of the window.
pub const DEFAULT_ZOOM_SCALE: f32 = ZOOM_LEVELS[DEFAULT_ZOOM_LEVEL];
/// The smallest level the panel can be drawn at.
const MIN_ZOOM: f32 = ZOOM_LEVELS[0];

/// The places of the widgets of the header.
struct HeaderRects {
    /// The button of the previous sound: a patch of the user or a preset.
    previous: Rect,
    /// The display that shows the name of the sound and opens the patches.
    display: Rect,
    /// The button of the next sound: a patch of the user or a preset.
    next: Rect,
    /// The button that mutates the sound.
    mutate: Rect,
    /// The tab that switches the panel to the modulation matrix.
    mod_page: Rect,
    /// The tab that switches the panel to the FX rack.
    fx_page: Rect,
    /// The meter that shows the number of the voices that are playing.
    meter: Rect,
}

/// The places of the widgets of the header.
fn header_rects(rect: Rect, zoom: f32) -> HeaderRects {
    // There is no wordmark any more: the header starts with the presets, so
    // the row reads from left to right as "sound, mutate, page, meter".
    let preset_x = rect.left() + 6.0 * zoom;
    let button_y = rect.top() + (HEADER_HEIGHT - 28.0) * 0.5 * zoom;
    let previous = Rect::from_min_size(
        egui::pos2(preset_x, button_y),
        egui::vec2(22.0, 28.0) * zoom,
    );
    let display = Rect::from_min_size(
        egui::pos2(preset_x + 26.0 * zoom, button_y),
        egui::vec2(252.0, 28.0) * zoom,
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
    // The tabs sit a little further right than the patch commands, so the two
    // groups read as two groups.
    let mod_page = Rect::from_min_size(
        egui::pos2(mutate.right() + 14.0 * zoom, button_y),
        egui::vec2(PAGE_BUTTON_WIDTH, 28.0) * zoom,
    );
    let fx_page = Rect::from_min_size(
        egui::pos2(mod_page.right() + 4.0 * zoom, button_y),
        egui::vec2(PAGE_BUTTON_WIDTH, 28.0) * zoom,
    );

    // The voice meter takes the free space between the page tabs and the
    // oscilloscope, so the header has no hole in it.
    let meter = Rect::from_min_max(
        egui::pos2(fx_page.right() + 14.0 * zoom, rect.top()),
        egui::pos2(rect.right() - (SCOPE_WIDTH + 16.0) * zoom, rect.bottom()),
    );

    HeaderRects {
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

    // There is no wordmark: the header is the preset display, the patch
    // commands, the page tabs and the meter, nothing else.
    let rects = header_rects(rect, zoom);

    // The preset display is the patch browser button. It keeps showing
    // the current name and gets a pointer cursor and a tooltip instead of a
    // separate PATCHES button.
    let display_response = ui.interact(
        rects.display,
        ui.scope_id().with("sushi-lcd"),
        Sense::click(),
    );
    lcd(&painter, rects.display, &display_text(ctx.state), zoom);
    if display_response.clicked() {
        ctx.toggle_patch_browser();
        ui.ctx().request_repaint();
    }
    display_response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Open the patch browser");

    if button(ui, rects.previous, "<", zoom).clicked() {
        ctx.step_patch(-1);
    }
    if button(ui, rects.next, ">", zoom).clicked() {
        ctx.step_patch(1);
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

    // The meter that shows how loud the panel is playing: a row of cells that
    // change color with the level.
    level_meter(&painter, rects.meter, ctx.state.activity, zoom);

    // The oscilloscope fills the right-hand end of the header. The activity
    // and clipping lights are gone: the trace and the voice meter are the only
    // live things in the header.
    small_scope(&painter, header_scope_rect(rect, zoom), ctx.history, zoom);
}

/// The oscilloscope at the right end of the header.
fn header_scope_rect(rect: Rect, zoom: f32) -> Rect {
    Rect::from_min_size(
        egui::pos2(
            rect.right() - SCOPE_WIDTH * zoom - 2.0 * zoom,
            rect.top() + 5.0 * zoom,
        ),
        egui::vec2(SCOPE_WIDTH, HEADER_HEIGHT - 10.0) * zoom,
    )
}

// --- The rows of sections ------------------------------------------------

/// The modulation page: the routing table and the target inspector on the
/// left, and one column of five compact source modules on the right.
///
/// The four modules of the right column are the sources that have controls of
/// their own: both LFOs and the two envelopes. The macros are a performance
/// control of the main page and are not repeated here. The modules fill the
/// whole height of the workspace, so the page never scrolls.
fn mod_page(ui: &mut egui::Ui, layout: &Layout, ctx: &mut Ctx) {
    let zoom = ctx.zoom;

    mod_routing_table(ui, layout.mod_table(), ctx);

    for index in 0..Layout::mod_module_count() {
        let rect = layout.mod_module(index);
        match index {
            0 | 1 => lfo_group(ui, rect, zoom, ctx, index),
            2 => envelope_group(ui, rect, zoom, ctx, &EnvelopeSpec::mod_env(ctx.params)),
            3 => envelope_group(ui, rect, zoom, ctx, &EnvelopeSpec::amp_env(ctx.params)),
            _ => random_group(ui, rect, ctx),
        }
    }
}

/// The routing table on the upper left of the modulation page.
///
/// The table has one header and one line per route: number, source,
/// destination, a horizontal bipolar amount bar and a numeric readout. There
/// are no per-slot frames and no repeated captions, so a full page of routes
/// reads as a list instead of eight small forms.
fn mod_routing_table(ui: &mut egui::Ui, table: Rect, ctx: &mut Ctx) {
    CURRENT_MODULE.with(|module| *module.borrow_mut() = String::from("MODULATION"));
    let zoom = ctx.zoom;

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
    tracked_text(
        &painter,
        egui::pos2(
            table.left() + MOD_TABLE_PADDING * zoom,
            (table.top() + header_bottom) * 0.5,
        ),
        Align2::LEFT_CENTER,
        "MODULATION",
        FontId::new(font_size(13.0, zoom), bold()),
        palette().title_blue,
        0.5 * zoom,
    );
    tracked_text(
        &painter,
        egui::pos2(
            table.right() - MOD_TABLE_PADDING * zoom,
            (table.top() + header_bottom) * 0.5,
        ),
        Align2::RIGHT_CENTER,
        "SOURCE / DEST / AMOUNT",
        FontId::new(font_size(7.2, zoom), label_family()),
        palette().title_blue,
        0.8 * zoom,
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

    let row_height =
        ((table.bottom() - rows_top) / crate::params::MOD_SLOTS as f32).max(1.0);
    for slot in 0..crate::params::MOD_SLOTS {
        let top = rows_top + slot as f32 * row_height;
        // The controls sit on the upper line; the readings of the route get
        // the lower one.
        let control_h = (row_height * 0.62).min(MOD_POPUP_HEIGHT * zoom * 1.7);
        let control_y = top + control_h * 0.5;
        let source = sources[slot].value();
        let dest = dests[slot].value();
        let amount = amounts[slot].value();
        let active = source != ModSource::Off || dest != ModDest::Off;
        // An untouched route keeps its place, but it prints nothing: no
        // repeated "Off" and no row of zeros. The amount is silent when the
        // rounded readout is zero, so a tiny value cannot print a bare "0".
        let silent =
            source == ModSource::Off && dest == ModDest::Off && (amount * 100.0).round() == 0.0;

        // The whole row is a selection target. The popups and the amount bar
        // are drawn after this response, so they stay on top and keep their
        // own click and drag behavior. Only the empty parts of the row (the
        // number, the gaps and the space beside the fields) select it.
        let row_rect = Rect::from_min_size(
            egui::pos2(table.left(), top),
            egui::vec2(table.width(), row_height),
        );
                let row_response = ui.interact(
            row_rect,
            ui.scope_id().with(("sushi-mod-row", slot)),
            Sense::click(),
        );
        if row_response.clicked() {
            ctx.state.mod_selected = slot;
        }

        // A thin accent line on the left edge marks the selected route. It
        // uses the existing title colour, so the row stays as legible as the
        // unselected ones.
        if ctx.state.mod_selected == slot {
            painter.rect_filled(
                Rect::from_min_size(
                    egui::pos2(table.left(), top),
                    egui::vec2(MOD_SELECTED_LINE_WIDTH * zoom, row_height),
                ),
                CornerRadius::ZERO,
                palette().title_blue,
            );
        }

        painter.text(
            egui::pos2(index_x + MOD_INDEX_WIDTH * 0.5 * zoom, control_y),
            Align2::CENTER_CENTER,
            format!("{:02}", slot + 1),
            FontId::monospace(font_size(8.0, zoom)),
            palette().title_blue,
        );

        if active {
            painter.text(
                egui::pos2(arrow_x + MOD_ARROW_WIDTH * 0.5 * zoom, control_y),
                Align2::CENTER_CENTER,
                ">",
                FontId::monospace(font_size(9.0, zoom)),
                palette().title_blue,
            );
        }

        if !silent {
            painter.text(
                egui::pos2(value_right, control_y),
                Align2::RIGHT_CENTER,
                format_amount(amount),
                FontId::monospace(font_size(9.5, zoom)),
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
            egui::vec2(MOD_SOURCE_WIDTH * zoom, control_h),
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .id_salt(("sushi-mod-source", slot))
                .max_rect(source_cell)
                .layout(EguiLayout::left_to_right(Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                ui.spacing_mut().interact_size.y = MOD_POPUP_HEIGHT * zoom;
                ui.style_mut().override_font_id =
                    Some(FontId::new(font_size(9.5, zoom), label_family()));
                let response = ctx.popup_with_text(
                    ui,
                    source_ids[slot],
                    sources[slot],
                    MOD_SOURCE_WIDTH - MOD_COLUMN_GAP,
                    source_selected,
                );
                if response.clicked() {
                    ctx.state.mod_selected = slot;
                }
            },
        );

        let dest_cell = Rect::from_min_size(
            egui::pos2(dest_x, top),
            egui::vec2(MOD_DEST_WIDTH * zoom, control_h),
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .id_salt(("sushi-mod-dest", slot))
                .max_rect(dest_cell)
                .layout(EguiLayout::left_to_right(Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                ui.spacing_mut().interact_size.y = MOD_POPUP_HEIGHT * zoom;
                ui.style_mut().override_font_id =
                    Some(FontId::new(font_size(9.5, zoom), label_family()));
                let response = ctx.popup_with_text(
                    ui,
                    dest_ids[slot],
                    dests[slot],
                    MOD_DEST_WIDTH - MOD_COLUMN_GAP,
                    dest_selected,
                );
                if response.clicked() {
                    ctx.state.mod_selected = slot;
                }
            },
        );

        let bar_cell =
            Rect::from_min_size(egui::pos2(bar_x, top), egui::vec2(bar_width, control_h));
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

        // The readings of the route, on the line under its controls: the real
        // history of the source on the left, and what it does to the
        // destination on the right.
        if active {
            let live = live_source_value(source, ctx.mod_feedback);
            let row_middle = top + control_h + (row_height - control_h) * 0.5;
            let reading = FontId::monospace(font_size(12.0, zoom));
            let caption = FontId::new(font_size(9.0, zoom), label_family());

            // The curve of the source is its own history, from the audio
            // thread: it shows what the source really did, not its setting.
            let wave_left = source_x;
            let wave_right = dest_x + MOD_DEST_WIDTH * zoom;
            let wave_h = ((row_height - control_h) * 0.72).max(9.0 * zoom);
            let wave = Rect::from_min_size(
                egui::pos2(wave_left, row_middle - wave_h * 0.5),
                egui::vec2(wave_right - wave_left, wave_h),
            );
            let unipolar = match source {
                ModSource::Lfo1 | ModSource::Lfo2 => false,
                ModSource::Random => ctx.params.random_unipolar.value(),
                _ => true,
            };
            let floor_y = if unipolar {
                wave.bottom()
            } else {
                wave.center().y
            };
            let span = if unipolar {
                wave.height() - 2.0 * zoom
            } else {
                wave.height() * 0.5 - 2.0 * zoom
            };
            // A source that belongs to one note has no history of its own, so
            // each of them draws what it is: the velocity as a level bar, the
            // key as a strip of the keyboard, the random as the steps of the
            // notes that were played.
            if matches!(source, ModSource::Velocity | ModSource::Key) {
                let seconds = ui.input(|input| input.stable_dt).clamp(0.0, 0.1);
                let voice = (
                    ctx.mod_feedback.voice_active(),
                    ctx.mod_feedback.velocity(),
                    ctx.mod_feedback.key(),
                );
                let bunch = (
                    &mut ctx.state.velocity_meter[slot],
                    &mut ctx.state.key_hue[slot],
                );
                draw_note_source(
                    &painter,
                    wave,
                    source,
                    voice,
                    bunch.0,
                    bunch.1,
                    zoom,
                    seconds,
                );
            }
            let history = source_history(source, ctx.mod_feedback, &mut ctx.wave_history);
            if let Some(history) = history {
                let last = history.len().saturating_sub(1).max(1) as f32;
                let points: Vec<egui::Pos2> = history
                    .iter()
                    .enumerate()
                    .map(|(index, value)| {
                        let x = wave.left() + wave.width() * index as f32 / last;
                        let y = floor_y - value.clamp(-1.0, 1.0) * span;
                        egui::pos2(
                            x.clamp(wave.left(), wave.right()),
                            y.clamp(wave.top() + zoom, wave.bottom() - zoom),
                        )
                    })
                    .collect();
                painter.line_segment(
                    [
                        egui::pos2(wave.left(), floor_y),
                        egui::pos2(wave.right(), floor_y),
                    ],
                    Stroke::new(1.0, palette().rule),
                );
                painter.add(egui::Shape::line(
                    points.clone(),
                    Stroke::new(1.0, palette().accent),
                ));
                // The newest sample is the only moving part of the curve: a dot
                // at the live end says "here, now" without a stretching block.
                if let Some(tip) = points.last() {
                    painter.circle_filled(*tip, 2.5 * zoom, palette().accent);
                }
            }

            // The reading sits on the column of the amount bar, under it, so
            // the two values start at a stable x instead of hanging off the
            // right edge.
            //
            // A number that follows every sample is unreadable at 5 Hz, so the
            // orange side shows the span the route is sweeping right now, taken
            // from the same history as the curve. It only changes when the
            // extremes of the modulation change, and it says more than an
            // instant that flickers: "this route moves the cutoff between 2 and
            // 12 kHz". A source that stands still collapses to a single value.
            let now_x = bar_x.max(wave_right + 12.0 * zoom);
            let base_x = now_x + MOD_NOW_LABEL_WIDTH * zoom;
            let (live_text, live_color) = match live {
                Some(_) => match source_span(source, ctx.mod_feedback, amount, &mut ctx.span_history) {
                    Some((low, high)) => {
                        let low_text = target_live_text(dest, ctx.params, low);
                        let high_text = target_live_text(dest, ctx.params, high);
                        let span_text = format!("{low_text} – {high_text}");
                        let room = value_right
                            - (base_x + MOD_NOW_BASE_WIDTH * zoom)
                            - 4.0 * zoom;
                        if low_text == high_text
                            || text_width(&painter, &span_text, &reading) > room
                        {
                            (
                                target_live_text(
                                    dest,
                                    ctx.params,
                                    live.unwrap_or(0.0) * amount,
                                ),
                                palette().accent,
                            )
                        } else {
                            (span_text, palette().accent)
                        }
                    }
                    None => (
                        target_live_text(dest, ctx.params, live.unwrap_or(0.0) * amount),
                        palette().accent,
                    ),
                },
                None => ("per-voice".to_string(), palette().muted),
            };
            tracked_text(
                &painter,
                egui::pos2(now_x, row_middle),
                Align2::LEFT_CENTER,
                "NOW",
                caption,
                palette().muted,
                0.8 * zoom,
            );
            painter.text(
                egui::pos2(base_x, row_middle),
                Align2::LEFT_CENTER,
                target_current_text(dest, ctx.params),
                reading.clone(),
                palette().led_on,
            );
            painter.text(
                egui::pos2(base_x + MOD_NOW_BASE_WIDTH * zoom, row_middle),
                Align2::LEFT_CENTER,
                live_text,
                reading,
                live_color,
            );
        }

        painter.line_segment(
            [
                egui::pos2(table.left(), top + row_height),
                egui::pos2(table.right(), top + row_height),
            ],
            Stroke::new(1.0, palette().rule),
        );
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
                Part::Body => column_row_fixed(ui, "osc1", 5, 3.0, 41.0, zoom, |index, ui| {
                    let p = ctx.params;
                    match index {
                        0 => ctx.float(ui, "tune", "Tune", &p.tune, &Knob::plain(-12.0, 12.0, 2)),
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
                }),
            }),
            1 => group(ui, rect, "VCO 2", zoom, |ui, part| match part {
                Part::Header => {
                    ctx.popup(ui, "osc2_wave", &p.osc2_wave, 66.0);
                    ctx.check(ui, "osc_sync", &p.osc_sync, "SYNC");
                    ctx.check(ui, "osc_phase_reset", &p.osc_phase_reset, "RESET");
                }
                Part::Body => column_row_fixed(ui, "osc2", 5, 3.0, 39.0, zoom, |index, ui| {
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
                        4 => ctx.float(ui, "ring_mod", "Ring", &p.ring_mod, &Knob::percent()),
                        _ => {}
                    }
                }),
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
                    column_row_fixed(ui, "mix", 6, COLUMN_GAP, 50.0, zoom, |index, ui| {
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
                    column_row_fixed(ui, "filter", 6, COLUMN_GAP, 50.0, zoom, |index, ui| {
                        let p = ctx.params;
                        match index {
                            0 => ctx.float(
                                ui,
                                "cutoff",
                                "Cut",
                                &p.cutoff,
                                &Knob::logarithmic(20.0, 20_000.0, 0),
                            ),
                            1 => ctx.float(
                                ui,
                                "hpf_cutoff",
                                "HPF",
                                &p.hpf_cutoff,
                                &Knob::logarithmic(20.0, 2_000.0, 0),
                            ),
                            2 => ctx.float(ui, "resonance", "Res", &p.resonance, &Knob::percent()),
                            3 => ctx.float(
                                ui,
                                "filter_env_amount",
                                "Env",
                                &p.filter_env_amount,
                                &Knob::bipolar(),
                            ),
                            4 => ctx.float(
                                ui,
                                "filter_keytrack",
                                "Key",
                                &p.filter_keytrack,
                                &Knob::percent(),
                            ),
                            5 => ctx.float(
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
    ids: [&'static str; 5],
    params: [&'a FloatParam; 5],
    release_max: f32,
}

impl<'a> EnvelopeSpec<'a> {
    /// The filter envelope of the normal panel and of the modulation page.
    fn mod_env(params: &'a SushiParams) -> Self {
        Self {
            title: "MOD ENV",
            column: "fe",
            ids: [
                "fe_attack",
                "fe_decay",
                "fe_sustain",
                "fe_release",
                "fe_curve",
            ],
            params: [
                &params.fe_attack,
                &params.fe_decay,
                &params.fe_sustain,
                &params.fe_release,
                &params.fe_curve,
            ],
            release_max: 4_000.0,
        }
    }

    /// The amplifier envelope of the normal panel and of the modulation page.
    fn amp_env(params: &'a SushiParams) -> Self {
        Self {
            title: "AMP ENV",
            column: "ae",
            ids: [
                "ae_attack",
                "ae_decay",
                "ae_sustain",
                "ae_release",
                "ae_curve",
            ],
            params: [
                &params.ae_attack,
                &params.ae_decay,
                &params.ae_sustain,
                &params.ae_release,
                &params.ae_curve,
            ],
            release_max: 8_000.0,
        }
    }
}

/// Draws one of the two envelopes: five A/D/S/R/Curve faders in the same
/// geometry as every other section. The normal panel and the modulation page
/// both use it, so the faders return to the page with the exact height and
/// pitch that they have on the panel.
fn envelope_group(ui: &mut egui::Ui, rect: Rect, zoom: f32, ctx: &mut Ctx, spec: &EnvelopeSpec) {
    group(ui, rect, spec.title, zoom, |ui, part| match part {
        Part::Header => {}
        Part::Body => {
            column_row_fixed(ui, spec.column, 5, COLUMN_GAP, 50.0, zoom, |index, ui| {
                let knob = match index {
                    0 | 1 => Knob::logarithmic(0.0, 4_000.0, 1),
                    2 => Knob::percent(),
                    4 => Knob::bipolar(),
                    _ => Knob::logarithmic(0.0, spec.release_max, 1),
                };
                let label = match index {
                    0 => "A",
                    1 => "D",
                    2 => "S",
                    3 => "R",
                    _ => "Curve",
                };
                ctx.float(ui, spec.ids[index], label, spec.params[index], &knob);
            });
        }
    });
}

/// The faders of a source module and the strip under them, on the modulation
/// page. On any other page the strip has no height and the faders keep theirs.

/// The live strip of a source module: the recorded history of that source, in
/// the green of the readouts. The dot at the right end is the current sample.

/// The live strip of the macro module: one bar per macro, filled to the level
/// the macro is sending right now.

/// One of the two LFO editors of the modulation page.
///
/// The LFO is a source of the modulation matrix now, so its section has no
/// target or amount. It keeps the waveform, the clock and the retrigger. The
/// compact module packs the three faders and the clock controls into the
/// right-hand source column, next to the table that owns the routing.
fn lfo_group(ui: &mut egui::Ui, rect: Rect, zoom: f32, ctx: &mut Ctx, index: usize) {
    let p = ctx.params;
    let (title, wave, sync, division, retrigger, rate, phase, curve, ids) = if index == 0 {
        (
            "LFO 1",
            &p.lfo1_wave,
            &p.lfo1_sync,
            &p.lfo1_div,
            &p.lfo1_retrigger,
            &p.lfo1_rate,
            &p.lfo1_phase,
            &p.lfo1_curve,
            (
                "lfo1_wave",
                "lfo1_sync",
                "lfo1_div",
                "lfo1_retrigger",
                "lfo1_rate",
                "lfo1_phase",
                "lfo1_curve",
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
            &p.lfo2_phase,
            &p.lfo2_curve,
            (
                "lfo2_wave",
                "lfo2_sync",
                "lfo2_div",
                "lfo2_retrigger",
                "lfo2_rate",
                "lfo2_phase",
                "lfo2_curve",
            ),
        )
    };

    group(ui, rect, title, zoom, |ui, part| match part {
        Part::Header => {
            ctx.popup(ui, ids.0, wave, 54.0);
            ctx.check(ui, ids.1, sync, "SYNC");
        }
        Part::Body => {
            column_row_fill(
                ui,
                ids.4,
                4,
                WIDE_COLUMN_GAP,
                zoom,
                |index, ui| match index {
                    0 => ctx.float(ui, ids.4, "Rate", rate, &Knob::logarithmic(0.01, 40.0, 2)),
                    1 => ctx.float(ui, ids.5, "Phase", phase, &Knob::percent()),
                    2 => ctx.float(ui, ids.6, "Curve", curve, &Knob::bipolar()),
                    _ => {
                        ui.horizontal(|ui| {
                            ctx.check(ui, ids.3, retrigger, "RETRIG");
                        });
                        caption(ui, "Div", 9.0, zoom, palette().label);
                        ui.add_space(4.0 * zoom);
                        ctx.popup(ui, ids.2, division, 66.0);
                    }
                },
            );
        }
    });
}

/// The third row of the normal page: the arpeggiator and the voice.
///
/// The two wide sections use their whole width for the fader banks, so the
/// lower half of the panel reads as two balanced columns instead of one
/// dense block on the left and empty paper on the right.
fn row_c(ui: &mut egui::Ui, layout: &Layout, ctx: &mut Ctx) {
    let zoom = ctx.zoom;
    group_row(ui, "row-c", layout, 2, |index, rect, ui| {
        let p = ctx.params;
        match index {
            0 => group(ui, rect, "ARP", zoom, |ui, part| match part {
                Part::Header => {
                    ctx.check(ui, "arp_on", &p.arp_on, "ON");
                    ctx.check(ui, "arp_latch", &p.arp_latch, "LATCH");
                    ctx.popup(ui, "arp_pattern", &p.arp_pattern, 74.0);
                    ctx.popup(ui, "arp_rate", &p.arp_rate, 52.0);
                    ctx.popup(ui, "arp_chord", &p.arp_chord, 66.0);
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
                    column_row_fill(ui, "arp", 7, WIDE_COLUMN_GAP, zoom, |index, ui| {
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
                            5 => ctx.float(
                                ui,
                                "arp_strum",
                                "Strum",
                                &p.arp_strum,
                                &Knob::plain(0.0, 80.0, 1),
                            ),
                            6 => ctx.float(
                                ui,
                                "arp_probability",
                                "Prob",
                                &p.arp_probability,
                                &Knob::percent(),
                            ),
                            _ => {}
                        }
                    });
                }
            }),
            _ => group(ui, rect, "VOICE", zoom, |ui, part| match part {
                Part::Header => ctx.popup(ui, "poly_mode", &p.poly_mode, 66.0),
                Part::Body => {
                    column_row_fill(ui, "voice", 4, WIDE_COLUMN_GAP, zoom, |index, ui| {
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

/// The fourth row of the normal page: the macros and the output.
///
/// Both are clean fader groups. The modulation page owns the source editors;
/// the VCA stays on the main page only.
fn row_d(ui: &mut egui::Ui, layout: &Layout, ctx: &mut Ctx) {
    let zoom = ctx.zoom;
    group_row(ui, "row-d", layout, 3, |index, rect, ui| match index {
        0 => macros_group(ui, rect, ctx),
        _ => group(ui, rect, "VCA", zoom, |ui, part| match part {
            Part::Header => {
                let p = ctx.params;
                text_label(ui, "CHAR", 8.0, zoom, palette().muted);
                ctx.radio_pair(ui, "character", &p.character, 58.0);
            }
            Part::Body => {
                column_row_fill(ui, "output", 4, WIDE_COLUMN_GAP, zoom, |index, ui| {
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
                        2 => ctx.float(
                            ui,
                            "vca_width",
                            "Width",
                            &p.vca_width,
                            &Knob::plain(0.0, 2.0, 2),
                        ),
                        3 => ctx.float(ui, "vca_pan", "Pan", &p.vca_pan, &Knob::bipolar()),
                        _ => {}
                    }
                });
            }
        }),
    });
}

/// The random source: how often it draws a new value and how far it glides to
/// it. The value itself is the same one the matrix reads, so the row that uses
/// it and this module can never disagree.
fn random_group(ui: &mut egui::Ui, rect: Rect, ctx: &mut Ctx) {
    let zoom = ctx.zoom;
    group(ui, rect, "RANDOM", zoom, |ui, part| match part {
        Part::Header => {
            let p = ctx.params;
            ctx.check(ui, "random_unipolar", &p.random_unipolar, "UNI");
        }
        Part::Body => {
            column_row_fixed(ui, "random", 3, COLUMN_GAP, 48.0, zoom, |index, ui| {
                let p = ctx.params;
                match index {
                    0 => ctx.float(
                        ui,
                        "random_rate",
                        "Rate",
                        &p.random_rate,
                        &Knob::logarithmic(0.02, 20.0, 2),
                    ),
                    1 => ctx.float(
                        ui,
                        "random_smooth",
                        "Smooth",
                        &p.random_smooth,
                        &Knob::percent(),
                    ),
                    _ => ctx.int(
                        ui,
                        "random_seed",
                        "Seed",
                        &p.random_seed,
                        &Knob::plain(1.0, 999.0, 0),
                    ),
                }
            });
        }
    });
}

/// The three macro faders. The same clean group is used by the main page and
/// by the right-hand source column of the modulation page.
fn macros_group(ui: &mut egui::Ui, rect: Rect, ctx: &mut Ctx) {
    let zoom = ctx.zoom;
    group(ui, rect, "MACROS", zoom, |ui, part| match part {
        Part::Header => {}
        Part::Body => {
            column_row_fill(ui, "macros", 3, WIDE_COLUMN_GAP, zoom, |index, ui| {
                let p = ctx.params;
                let (id, label, param) = match index {
                    0 => ("macro_bright", "Brilliance", &p.macro_bright),
                    1 => ("macro_motion", "Motion", &p.macro_motion),
                    _ => ("macro_space", "Space", &p.macro_space),
                };
                ctx.float(ui, id, label, param, &Knob::percent());
            });
        }
    });
}






/// The controls of one module of the FX rack. The module index follows the
/// fixed card grid: drive, phaser, chorus, delay, reverb, EQ, compressor and
/// tremolo.
fn fx_module_controls(ui: &mut egui::Ui, ctx: &mut Ctx, module: usize) {
    let zoom = ctx.zoom;
    let p = ctx.params;
    match module {
        0 => column_row_fill(
            ui,
            "fx-drive",
            3,
            WIDE_COLUMN_GAP,
            zoom,
            |index, ui| match index {
                0 => ctx.float(
                    ui,
                    "drive_amount",
                    "Amount",
                    &p.drive_amount,
                    &Knob::percent(),
                ),
                1 => ctx.float(ui, "drive_tone", "Tone", &p.drive_tone, &Knob::bipolar()),
                2 => ctx.float(ui, "drive_mix", "Mix", &p.drive_mix, &Knob::percent()),
                _ => {}
            },
        ),
        1 => column_row_fill(
            ui,
            "fx-phaser",
            4,
            WIDE_COLUMN_GAP,
            zoom,
            |index, ui| match index {
                0 => ctx.float(
                    ui,
                    "phaser_rate",
                    "Rate",
                    &p.phaser_rate,
                    &Knob::logarithmic(0.02, 5.0, 2),
                ),
                1 => ctx.float(
                    ui,
                    "phaser_depth",
                    "Depth",
                    &p.phaser_depth,
                    &Knob::percent(),
                ),
                2 => ctx.float(
                    ui,
                    "phaser_feedback",
                    "Feedback",
                    &p.phaser_feedback,
                    &Knob::scaled(0.0, 0.9, 0),
                ),
                3 => ctx.float(ui, "phaser_mix", "Mix", &p.phaser_mix, &Knob::percent()),
                _ => {}
            },
        ),
        2 => column_row_fill(
            ui,
            "fx-chorus",
            4,
            WIDE_COLUMN_GAP,
            zoom,
            |index, ui| match index {
                0 => ctx.float(
                    ui,
                    "chorus_rate",
                    "Rate",
                    &p.chorus_rate,
                    &Knob::logarithmic(0.05, 8.0, 2),
                ),
                1 => ctx.float(
                    ui,
                    "chorus_depth",
                    "Depth",
                    &p.chorus_depth,
                    &Knob::percent(),
                ),
                2 => ctx.float(
                    ui,
                    "chorus_width",
                    "Width",
                    &p.chorus_width,
                    &Knob::percent(),
                ),
                3 => ctx.float(ui, "chorus_mix", "Mix", &p.chorus_mix, &Knob::percent()),
                _ => {}
            },
        ),
        3 => column_row_fill(
            ui,
            "fx-delay",
            4,
            WIDE_COLUMN_GAP,
            zoom,
            |index, ui| match index {
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
                    "Feedback",
                    &p.delay_feedback,
                    &Knob::percent(),
                ),
                2 => ctx.float(
                    ui,
                    "delay_damping",
                    "Damping",
                    &p.delay_damping,
                    &Knob::percent(),
                ),
                3 => ctx.float(ui, "delay_mix", "Mix", &p.delay_mix, &Knob::percent()),
                _ => {}
            },
        ),
        4 => column_row_fill(
            ui,
            "fx-reverb",
            3,
            WIDE_COLUMN_GAP,
            zoom,
            |index, ui| match index {
                0 => ctx.float(
                    ui,
                    "reverb_decay",
                    "Decay",
                    &p.reverb_decay,
                    &Knob::percent(),
                ),
                1 => ctx.float(ui, "reverb_tone", "Tone", &p.reverb_tone, &Knob::bipolar()),
                2 => ctx.float(ui, "reverb_mix", "Mix", &p.reverb_mix, &Knob::percent()),
                _ => {}
            },
        ),
        5 => {
            let eq = Knob::plain(-12.0, 12.0, 1);
            column_row_fill(
                ui,
                "fx-eq",
                3,
                WIDE_COLUMN_GAP,
                zoom,
                |index, ui| match index {
                    0 => ctx.float(ui, "eq_low", "Low", &p.eq_low, &eq),
                    1 => ctx.float(ui, "eq_mid", "Mid", &p.eq_mid, &eq),
                    2 => ctx.float(ui, "eq_high", "High", &p.eq_high, &eq),
                    _ => {}
                },
            );
        }
        6 => column_row_fill(
            ui,
            "fx-compressor",
            6,
            WIDE_COLUMN_GAP,
            zoom,
            |index, ui| match index {
                0 => ctx.float(
                    ui,
                    "comp_threshold",
                    "Threshold",
                    &p.comp_threshold,
                    &Knob::plain(-60.0, 0.0, 1),
                ),
                1 => ctx.float(
                    ui,
                    "comp_ratio",
                    "Ratio",
                    &p.comp_ratio,
                    &Knob::plain(1.0, 20.0, 1),
                ),
                2 => ctx.float(
                    ui,
                    "comp_attack",
                    "Attack",
                    &p.comp_attack,
                    &Knob::logarithmic(0.1, 100.0, 1),
                ),
                3 => ctx.float(
                    ui,
                    "comp_release",
                    "Release",
                    &p.comp_release,
                    &Knob::logarithmic(10.0, 1_000.0, 0),
                ),
                4 => ctx.float(
                    ui,
                    "comp_makeup",
                    "Makeup",
                    &p.comp_makeup,
                    &Knob::plain(0.0, 24.0, 1),
                ),
                5 => ctx.float(ui, "comp_mix", "Mix", &p.comp_mix, &Knob::percent()),
                _ => {}
            },
        ),
        _ => column_row_fill(
            ui,
            "fx-tremolo",
            3,
            WIDE_COLUMN_GAP,
            zoom,
            |index, ui| match index {
                0 => ctx.float(
                    ui,
                    "trem_rate",
                    "Rate",
                    &p.trem_rate,
                    &Knob::logarithmic(0.05, 20.0, 2),
                ),
                1 => ctx.float(ui, "trem_depth", "Depth", &p.trem_depth, &Knob::percent()),
                2 => ctx.check(ui, "trem_stereo", &p.trem_stereo, "STEREO"),
                _ => {}
            },
        ),
    }
}

/// Draws the small grip of the drag handle.
fn fx_chain_grip(painter: &egui::Painter, tile: Rect, zoom: f32) {
    let color = palette().muted;
    let radius = 1.1 * zoom;
    let x = tile.left() + 7.0 * zoom;
    for row in -1..=1 {
        for column in 0..2 {
            painter.circle_filled(
                egui::pos2(
                    x + column as f32 * 3.0 * zoom,
                    tile.center().y + row as f32 * 5.0 * zoom,
                ),
                radius,
                color,
            );
        }
    }
}

/// Draws one of the compact reorder arrows of a chain tile.
fn fx_chain_arrow(
    painter: &egui::Painter,
    rect: Rect,
    up: bool,
    enabled: bool,
    hovered: bool,
    zoom: f32,
) {
    let fill = if !enabled {
        palette().track
    } else if hovered {
        palette().button_active
    } else {
        palette().button_bg
    };
    flat_box(painter, rect, fill);

    let center = rect.center();
    let half_width = (rect.width() * 0.28).max(2.0 * zoom);
    let half_height = (rect.height() * 0.22).max(2.0 * zoom);
    let tip = if up {
        center.y - half_height
    } else {
        center.y + half_height
    };
    let base = if up {
        center.y + half_height
    } else {
        center.y - half_height
    };
    let color = if enabled {
        palette().text
    } else {
        palette().muted
    };
    painter.add(egui::Shape::convex_polygon(
        vec![
            egui::pos2(center.x, tip),
            egui::pos2(center.x - half_width, base),
            egui::pos2(center.x + half_width, base),
        ],
        color,
        Stroke::NONE,
    ));
}

/// Draws the small power/bypass icon of a chain tile.
fn fx_chain_power_icon(
    painter: &egui::Painter,
    rect: Rect,
    bypassed: bool,
    hovered: bool,
    zoom: f32,
) {
    if hovered {
        painter.rect_filled(rect, CornerRadius::ZERO, palette().button_active);
    }
    let center = rect.center();
    let radius = rect.width() * 0.27;
    let color = if bypassed {
        palette().muted
    } else if hovered {
        palette().accent
    } else {
        palette().title_blue
    };
    let stroke = Stroke::new((1.4 * zoom).round().max(1.0), color);
    painter.circle_stroke(center, radius, stroke);
    painter.line_segment(
        [
            egui::pos2(center.x, center.y - radius - 1.5 * zoom),
            egui::pos2(center.x, center.y + 1.0 * zoom),
        ],
        stroke,
    );
    if bypassed {
        // The diagonal slash makes the bypass state readable even when the
        // warm palette is flattened by a screenshot.
        painter.line_segment(
            [
                egui::pos2(center.x - radius * 0.75, center.y + radius * 0.75),
                egui::pos2(center.x + radius * 0.75, center.y - radius * 0.75),
            ],
            Stroke::new((1.0 * zoom).round().max(1.0), palette().accent),
        );
    }
}

/// Draws one movable chain tile and handles its power button, its two reorder
/// arrows and the start of a drag.
fn fx_chain_tile(ui: &mut egui::Ui, ctx: &mut Ctx, position: usize, stage: FxStage, tile: Rect) {
    let zoom = ctx.zoom;
    let parts = fx_chain_tile_parts(tile, zoom);
    let p = ctx.params;
    let bypassed = p.fx_bypass_mask.value() & (stage.bit() as i32) != 0;
    let dragging = ctx
        .state
        .fx_chain_drag
        .is_some_and(|drag| drag.source == position);

    let drag_id = ui.scope_id().with(("sushi-fx-chain-drag", stage.index()));
    let power_id = ui.scope_id().with(("sushi-fx-chain-power", stage.index()));
    let up_id = ui.scope_id().with(("sushi-fx-chain-up", stage.index()));
    let down_id = ui.scope_id().with(("sushi-fx-chain-down", stage.index()));
    let drag_response = ui.interact(parts.drag, drag_id, Sense::click_and_drag());
    let power_response = ui.interact(parts.power, power_id, Sense::click());
    let up_response = ui.interact(parts.up, up_id, Sense::click());
    let down_response = ui.interact(parts.down, down_id, Sense::click());

    
    let painter = ui.painter_at(tile);
    let hovered = drag_response.hovered()
        || power_response.hovered()
        || up_response.hovered()
        || down_response.hovered();
    let fill = if dragging || (hovered && !bypassed) {
        palette().button_active
    } else if bypassed {
        palette().button_bg
    } else {
        palette().field_bg
    };
    painter.rect_filled(tile, CornerRadius::ZERO, fill);
    painter.rect_stroke(
        tile,
        CornerRadius::ZERO,
        Stroke::new(
            if dragging { 2.0 } else { 1.0 },
            if dragging {
                palette().accent
            } else if bypassed {
                palette().rule
            } else {
                palette().line
            },
        ),
        egui::StrokeKind::Inside,
    );

    fx_chain_grip(&painter, tile, zoom);

    let number_color = if bypassed {
        palette().muted
    } else {
        palette().title_blue
    };
    let text_color = if bypassed {
        palette().muted
    } else {
        palette().text
    };
    painter.text(
        parts.number.center(),
        Align2::CENTER_CENTER,
        format!("{}", position + 1),
        FontId::new(font_size(13.0, zoom), value_family()),
        number_color,
    );
    let name_font = FontId::new(font_size(9.5, zoom), bold());
    let name = fit_text(
        &painter,
        fx_stage_name(stage),
        parts.name.width(),
        &name_font,
    );
    painter.text(
        egui::pos2(parts.name.left(), parts.name.center().y),
        Align2::LEFT_CENTER,
        name,
        name_font,
        text_color,
    );

    fx_chain_arrow(
        &painter,
        parts.up,
        true,
        position > 0,
        up_response.hovered(),
        zoom,
    );
    fx_chain_arrow(
        &painter,
        parts.down,
        false,
        position + 1 < FX_STAGES.len(),
        down_response.hovered(),
        zoom,
    );
    fx_chain_power_icon(
        &painter,
        parts.power,
        bypassed,
        power_response.hovered(),
        zoom,
    );

    if drag_response.hovered() {
        ctx.set_readout(
            "FX CHAIN".to_string(),
            format!("{} / {}", position + 1, fx_stage_name(stage)),
        );
    }
    if power_response.hovered() {
        ctx.set_readout(
            fx_stage_name(stage).to_string(),
            if bypassed { "BYPASSED" } else { "ACTIVE" }.to_string(),
        );
    }
    let power_double_clicked = power_response.double_clicked();
    let power_clicked = power_response.clicked();
    let up_clicked = up_response.clicked();
    let down_clicked = down_response.clicked();
    let drag_started = drag_response.drag_started();

    drag_response.on_hover_text("Drag to reorder\nUse the arrows for one step");
    power_response.on_hover_text(if bypassed {
        "Bypassed - click to enable"
    } else {
        "Active - click to bypass"
    });
    up_response.on_hover_text("Move this effect up the chain");
    down_response.on_hover_text("Move this effect down the chain");

    if power_double_clicked {
        // A double click resets the stage to the active state, even though
        // the first click of the double click already toggled the bit.
        ctx.fx_set_bypass(stage, false);
    } else if power_clicked {
        ctx.fx_toggle_bypass(stage);
    }
    if up_clicked && position > 0 {
        ctx.fx_move_stage(position, position - 1);
    }
    if down_clicked && position + 1 < FX_STAGES.len() {
        ctx.fx_move_stage(position, position + 1);
    }
    if drag_started {
        ctx.state.fx_chain_drag = Some(FxChainDrag { source: position });
    }
}

/// Draws the live drop indicator of a chain drag and applies the reorder when
/// the pointer is released. The source tile is only moved once, so a whole
/// drag is one reorder.
fn fx_chain_drag_feedback(ui: &mut egui::Ui, ctx: &mut Ctx, tiles: &[Rect]) {
    let Some(drag) = ctx.state.fx_chain_drag else {
        return;
    };
    let zoom = ctx.zoom;
    let (pointer, released, down) = ui.input(|input| {
        (
            input
                .pointer
                .interact_pos()
                .or_else(|| input.pointer.latest_pos()),
            input.pointer.primary_released(),
            input.pointer.primary_down(),
        )
    });

    if let Some(pointer) = pointer {
        let slot = fx_chain_drop_slot(pointer.y, tiles);
        let gap = FX_CHAIN_TILE_GAP * zoom * 0.5;
        let y = if slot == 0 {
            tiles[0].top() - gap
        } else if slot >= tiles.len() {
            tiles[tiles.len() - 1].bottom() + gap
        } else {
            tiles[slot].top() - gap
        };
        ui.painter().line_segment(
            [
                egui::pos2(tiles[0].left(), y),
                egui::pos2(tiles[0].right(), y),
            ],
            Stroke::new(2.0, palette().accent),
        );
    }

    if released {
        ctx.state.fx_chain_drag = None;
        if let Some(pointer) = pointer {
            let slot = fx_chain_drop_slot(pointer.y, tiles);
            let target = fx_chain_drop_position(drag.source, slot, tiles.len());
            if target != drag.source {
                ctx.fx_move_stage(drag.source, target);
            }
        }
    } else if !down {
        // Cancelling outside the window must not leave a dangling drag.
        ctx.state.fx_chain_drag = None;
    }
}

/// Draws the left chain column: the title, the eight movable tiles, the
/// compact global wet fader and the chain preset popup.
fn fx_chain_column(ui: &mut egui::Ui, rack: Rect, ctx: &mut Ctx) {
    CURRENT_MODULE.with(|module| *module.borrow_mut() = String::from("FX CHAIN"));
    let zoom = ctx.zoom;
    let p = ctx.params;
    let stages = stage_order(p.fx_order.value());
    let title = fx_chain_title_rect(rack, zoom);
    let painter = ui.painter_at(rack);

    tracked_text(
        &painter,
        title.left_center(),
        Align2::LEFT_CENTER,
        "CHAIN",
        FontId::new(font_size(13.0, zoom), bold()),
        palette().title_blue,
        0.5 * zoom,
    );

    // A quiet vertical rule separates the chain column from the module grid.
    let rule_x = rack.left() + (FX_CHAIN_COLUMN_WIDTH + FX_CHAIN_COLUMN_GAP * 0.5) * zoom;
    painter.line_segment(
        [
            egui::pos2(rule_x, rack.top()),
            egui::pos2(rule_x, rack.bottom()),
        ],
        Stroke::new(1.0, palette().rule),
    );

    let mut tiles = [Rect::ZERO; FX_STAGE_COUNT];
    for (position, stage) in stages.iter().enumerate() {
        let tile = fx_chain_tile_rect(rack, position, zoom);
        tiles[position] = tile;
        fx_chain_tile(ui, ctx, position, *stage, tile);
    }

    let wet_rect = fx_chain_wet_rect(rack, zoom);
    ctx.h_fader(
        ui,
        wet_rect,
        "fx_global_mix",
        "GLOBAL WET",
        &p.fx_global_mix,
        &Knob::percent(),
    );

    let (label_rect, popup_rect) = fx_chain_preset_rects(rack, zoom);
        ui.painter_at(label_rect).text(
        label_rect.left_center(),
        Align2::LEFT_CENTER,
        "CHAIN PRESET",
        FontId::new(font_size(7.8, zoom), label_family()),
        palette().muted,
    );
    ui.scope_builder(
        egui::UiBuilder::new()
            .id_salt("sushi-fx-chain-preset")
            .max_rect(popup_rect)
            .layout(EguiLayout::left_to_right(Align::Center)),
        |ui| ctx.fx_chain_preset_popup(ui, popup_rect.width()),
    );

    fx_chain_drag_feedback(ui, ctx, &tiles);
}

/// The FX page: a movable chain column on the left and a compact 2 x 4 grid of
/// the eight effect modules on the right.
///
/// The order of the signal path lives entirely in the left column: the tiles
/// are dragged or stepped with their arrows, and the preset popup selects a
/// complete permutation at once. The module cards on the right keep the
/// original parameter controls and never move, so a reordered chain does not
/// make the knobs jump around.
fn row_fx(ui: &mut egui::Ui, layout: &Layout, ctx: &mut Ctx) {
    let zoom = ctx.zoom;
    let rack = fx_rack_rect(layout);
    fx_chain_column(ui, rack, ctx);

    for (module, rect) in fx_module_rects(rack, zoom).iter().enumerate() {
        let title = FX_MODULE_TITLES[module / 2][module % 2];
        group(ui, *rect, title, zoom, |ui, part| {
            if let Part::Body = part {
                fx_module_controls(ui, ctx, module);
            }
        });
    }
}

// --- The widgets of the parameters ------------------------------------------

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
/// patch identity, so another sound refreshes it as well.
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
    /// The samples of the oscilloscope.
    history: &'a VecDeque<f32>,
    /// The number of voices that are currently playing.
    /// The last values of the global modulation sources.
    mod_feedback: &'a ModFeedback,
    /// The width of the sliders of the panel.
    slider_width: f32,
    /// The zoom of the panel: how much bigger it is than its design size.
    zoom: f32,
    /// The scratch buffer of the history of a modulation source.
    wave_history: Vec<f32>,
    /// The scratch buffer of the span of a modulation source.
    span_history: Vec<f32>,
}

impl<'a, 'b> Ctx<'a, 'b> {

    /// Fills the info block with one reading: the module it belongs to, the
    /// control, and the value. The module is the section that is being drawn,
    /// so a widget never has to know where it lives.
    fn set_readout(&mut self, label: String, value: String) {
        let module = CURRENT_MODULE.with(|module| module.borrow().clone());
        self.state.readout = Some((module, label, value));
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
                
        let zoom = self.zoom;

        caption(ui, label, 9.0, zoom, palette().label);

        // The slider takes the height that is left in its column above the
        // numeric field, so that a section never overflows its rectangle and
        // the field always sits at the bottom of the column.
        let height = slider_height(ui, zoom).max(MIN_SLIDER_HEIGHT * zoom);
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
            self.set_readout(
                label.to_ascii_uppercase(),
                format!("{}{}", value, param.unit()),
            );
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
            self.set_readout(
                label.to_ascii_uppercase(),
                format!("{}{}", value, param.unit()),
            );
        }
        response.on_hover_text(format!("{}{}\nRight-click to type", value, param.unit()));
    }

    /// Sets one bypass bit of the chain without touching the others.
    fn fx_set_bypass(&mut self, stage: FxStage, bypassed: bool) {
        let param = &self.params.fx_bypass_mask;
        let mask = param.value();
        let bit = stage.bit() as i32;
        let next = if bypassed { mask | bit } else { mask & !bit };
        if next == mask {
            return;
        }
        self.setter.set_parameter(param, next);
        self.state.dirty = true;
    }

    /// Toggles the bypass bit of one chain stage.
    fn fx_toggle_bypass(&mut self, stage: FxStage) {
        let bypassed = self.params.fx_bypass_mask.value() & (stage.bit() as i32) != 0;
        self.fx_set_bypass(stage, !bypassed);
    }

    /// Applies one of the 40,320 permutation indices of the chain.
    fn fx_apply_chain_index(&mut self, index: i32) {
        if index == self.params.fx_order.value() {
            return;
        }
        self.setter.set_parameter(&self.params.fx_order, index);
        self.state.dirty = true;
    }

    /// Applies a complete chain order. The permutation index is derived from
    /// the stage array with `stage_order_index`, so a drag or a preset can
    /// never write a value that does not describe the shown order.
    fn fx_apply_chain_order(&mut self, stages: [FxStage; FX_STAGE_COUNT]) {
        self.fx_apply_chain_index(stage_order_index(stages));
    }

    /// Moves one stage of the chain to another position and writes the full
    /// permutation back.
    fn fx_move_stage(&mut self, from: usize, to: usize) {
        if from == to || from >= FX_STAGES.len() || to >= FX_STAGES.len() {
            return;
        }
        let mut stages = stage_order(self.params.fx_order.value());
        if from < to {
            stages[from..=to].rotate_left(1);
        } else {
            stages[to..=from].rotate_right(1);
        }
        self.fx_apply_chain_order(stages);
    }

    /// Draws the chain preset popup. Every named preset sets the complete
    /// permutation; an order that does not match one of them shows `CUSTOM`
    /// and can still be restored with the tiles.
    fn fx_chain_preset_popup(&mut self, ui: &mut egui::Ui, width: f32) {
        
        let current = fx_chain_preset_name(self.params.fx_order.value());
        let named = fx_chain_preset_at(self.params.fx_order.value());
        let response = egui::ComboBox::from_id_salt("sushi-fx-chain-preset")
            .width((width / self.zoom).max(20.0))
            .selected_text(current)
            .truncate()
            .show_ui(ui, |ui| {
                for preset in FxChainPreset::ALL {
                    if ui
                        .selectable_label(named == Some(preset), preset.name())
                        .clicked()
                    {
                        self.fx_apply_chain_index(preset.order());
                    }
                }
            })
            .response;

        
        if response.hovered() {
            self.set_readout("CHAIN PRESET".to_string(), current.to_string());
        }
    }

    /// Draws a compact horizontal fader in a given rectangle. The caption sits
    /// on the left, the value on the right and the track in between.
    fn h_fader(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        id: &'static str,
        label: &str,
        param: &FloatParam,
        knob: &Knob,
    ) {
                
        let zoom = self.zoom;
        let label_rect = Rect::from_min_max(
            rect.min,
            egui::pos2(rect.left() + FX_GLOBAL_LABEL_WIDTH * zoom, rect.bottom()),
        );
        let value_rect = Rect::from_min_max(
            egui::pos2(rect.right() - FX_GLOBAL_VALUE_WIDTH * zoom, rect.top()),
            rect.max,
        );
        let track = Rect::from_min_max(
            egui::pos2(label_rect.right() + 4.0 * zoom, rect.top() + 4.0 * zoom),
            egui::pos2(value_rect.left() - 4.0 * zoom, rect.bottom() - 4.0 * zoom),
        );
        
        let painter = ui.painter_at(rect);
        tracked_text(
            &painter,
            label_rect.left_center(),
            Align2::LEFT_CENTER,
            &label.to_ascii_uppercase(),
            FontId::new(font_size(8.0, zoom), label_family()),
            palette().label,
            0.4 * zoom,
        );
        let value = format_number(knob.display(param.value()), knob.decimals);
        painter.text(
            value_rect.center(),
            Align2::CENTER_CENTER,
            format!("{}{}", value, param.unit()),
            FontId::monospace(font_size(9.0, zoom)),
            palette().text,
        );

        let mut value = param.value();
        let response = h_slider(ui, track, &mut value, knob, zoom);
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
            self.set_readout(
                label.to_ascii_uppercase(),
                format!("{}{}", value, param.unit()),
            );
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
        
        let zoom = self.zoom;
        let (rect, mut response) =
            ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        
        // Reaching the amount bar means reaching its route. The selection is
        // only a view state; it does not open an edit by itself.
        if response.clicked() || response.is_pointer_button_down_on() {
            self.state.mod_selected = slot;
        }

        let painter = ui.painter_at(rect);
        // The bar is as tall as the popups above it: one line of controls, one
        // height.
        let track_height = MOD_POPUP_HEIGHT * zoom;
        let track = Rect::from_center_size(
            rect.center(),
            egui::vec2(rect.width(), track_height),
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
        painter.line_segment(
            [
                egui::pos2(centre, track.top() + zoom),
                egui::pos2(centre, track.bottom() - zoom),
            ],
            Stroke::new(1.0, palette().rule),
        );
        let handle = Rect::from_center_size(
            egui::pos2(handle_x, track.center().y),
            egui::vec2(MOD_BAR_HANDLE_WIDTH * zoom, (track_height - 4.0 * zoom).max(4.0)),
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
        let text = format_amount(param.value());
        if response.hovered() || response.dragged() {
            self.set_readout(format!("MOD {} AMOUNT", slot + 1), format!("{text}%"));
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
                        egui::FontFamily::Monospace,
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
                        egui::FontFamily::Monospace,
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
        let _ = self.popup_with_text(ui, id, param, width, None);
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
    ) -> egui::Response {
                
        let current = param.value();
        let variants = T::variants();
        let selected_text = selected
            .map(str::to_owned)
            .unwrap_or_else(|| variants[T::to_index(current)].to_string());

        // The face and the entries of the popup use the label weight, so a
        // choice reads like the caption above its fader instead of like the
        // running text of a dialog.
        let face = FontId::new(font_size(9.5, self.zoom), label_family());
        let response = ui
            .scope(|ui| {
                ui.style_mut().override_font_id = Some(face);
                egui::ComboBox::from_id_salt(id)
                    .width(width * self.zoom)
                    .selected_text(selected_text)
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
                    .response
            })
            .inner;

        
        if response.hovered() {
            let label = id.replace('_', " ").to_ascii_uppercase();
            let value = variants[T::to_index(param.value())].to_ascii_uppercase();
            self.set_readout(label, value);
        }

        // A double click on the button of the popup resets the parameter to
        // its factory default.
        if response.double_clicked() {
            self.setter
                .set_parameter(param, param.default_plain_value());
            self.state.dirty = true;
        }

        response
    }

    /// Draws a check box with a label. Clicking it switches the parameter.
    fn check(&mut self, ui: &mut egui::Ui, id: &'static str, param: &BoolParam, label: &str) {
                
        let zoom = self.zoom;

        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(CHECK_SIZE, CHECK_SIZE) * zoom, Sense::hover());
                let response = ui.interact(
            rect,
            ui.scope_id().with(("sushi-check", id)),
            Sense::click(),
        );
        let painter = ui.painter_at(rect);
        // A flat white square with a thin outline and a black check mark.
        draw_check_box(&painter, rect, param.value(), zoom);

        if response.hovered() {
            self.set_readout(
                label.to_ascii_uppercase(),
                if param.value() { "ON" } else { "OFF" }.to_string(),
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
        
        let zoom = self.zoom;

        let variants = T::variants();
        let current = T::to_index(param.value());
        let button_width = width * zoom / variants.len() as f32;

        for (index, variant) in variants.iter().enumerate() {
            let (rect, _) = ui.allocate_exact_size(
                egui::vec2(button_width, RADIO_HEIGHT * zoom),
                Sense::hover(),
            );
                        let response = ui.interact(
                rect,
                ui.scope_id().with(("sushi-radio", id, index)),
                Sense::click(),
            );
            if response.hovered() {
                self.set_readout(id.to_ascii_uppercase(), variant.to_ascii_uppercase());
            }
            let painter = ui.painter_at(rect);

            // The active button of the pair is filled darker.
            let fill = if index == current || response.is_pointer_button_down_on() {
                palette().button_active
            } else {
                palette().button_bg
            };
            flat_box(&painter, rect, fill);
            tracked_text(
                &painter,
                rect.center(),
                Align2::CENTER_CENTER,
                // The name of the variant is shown in uppercase, like the
                // other labels of the panel.
                &variant.to_ascii_uppercase(),
                FontId::new(font_size(8.5, zoom), label_family()),
                palette().text,
                0.3 * zoom,
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
        self.state.patch_active_path = None;
        self.state.patch_name = PRESETS[index].name.to_string();
        presets::apply(self.params, self.setter, index);
        self.refresh_patch_preview();
    }

    /// Steps through the sounds that can be browsed: the patches of the user
    /// folder first, then the factory presets, in the order of the lists of
    /// the browser. A step from the last entry wraps around to the first one,
    /// so the buttons never run out of sounds.
    ///
    /// The list of the folder is read again on every step, so a patch that
    /// was saved in the meantime is reachable right away.
    fn step_patch(&mut self, delta: isize) {
        let files = patch::list_patches(&self.patch_dir());
        let total = files.len() + PRESETS.len();
        if total == 0 {
            return;
        }

        // The position of the loaded patch in the user list. A patch whose
        // file is gone falls back to the factory preset that was loaded
        // before it, so the step still starts from somewhere sensible.
        let user_position = self
            .state
            .patch_active_path
            .as_ref()
            .and_then(|path| files.iter().position(|file| file == path))
            .or_else(|| {
                let name = self.state.patch_active.as_ref()?;
                files
                    .iter()
                    .position(|file| file.file_stem().is_some_and(|stem| stem == name.as_str()))
            });
        let current = match user_position {
            Some(position) => position,
            None => files.len() + self.state.preset_index.min(PRESETS.len() - 1),
        };

        let next = (current as isize + delta).rem_euclid(total as isize) as usize;
        self.state.patch_files = files;
        if let Some(path) = self.state.patch_files.get(next).cloned() {
            self.load_patch_file(&path);
        } else {
            self.load_preset(next - self.state.patch_files.len());
        }
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
                            .extra_letter_spacing(1.0 * zoom)
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
                                                    egui::FontFamily::Monospace,
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
                                self.clear_patch_confirmation();
                            }
                            if ui.button("LOAD").clicked() {
                                self.load_patch_by_name();
                            }
                            if ui.button("SAVE").clicked() {
                                self.save_patch();
                            }
                            // DELETE asks for a second click. The label turns
                            // red while the confirmation is pending, so the
                            // second click is never blind.
                            let delete = if self.state.patch_confirm_delete {
                                egui::RichText::new("DELETE").color(palette().status_error)
                            } else {
                                egui::RichText::new("DELETE")
                            };
                            if ui.button(delete).clicked() {
                                self.delete_patch();
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

    /// Deletes the patch whose name is in the field of the name.
    ///
    /// The first click only asks for the second one, so a patch is never lost
    /// by a stray click. A loaded patch that is deleted is replaced by its
    /// neighbour in the list (or by the factory preset when the folder is
    /// empty), so the editor never keeps an identity that points at a file
    /// that does not exist anymore.
    fn delete_patch(&mut self) {
        if !self.folder_is_applied() {
            self.set_status(FOLDER_HINT.to_string(), true);
            return;
        }

        let name = patch::sanitize_file_name(&self.state.patch_name);
        if name.is_empty() {
            self.set_status("The name must not be empty".to_string(), true);
            return;
        }
        let dir = self.patch_dir();
        let path = dir.join(format!("{name}.json"));

        if !path.exists() {
            self.clear_patch_confirmation();
            self.set_status("Not found".to_string(), true);
            return;
        }

        if !self.state.patch_confirm_delete {
            // A delete always asks for a second click, no matter what the
            // save confirmation said.
            self.state.patch_confirm_overwrite = false;
            self.state.patch_confirm_delete = true;
            self.set_status(format!("Click DELETE again to remove {name}"), true);
            return;
        }

        // The place of the file in the list, so its neighbour can take over
        // when the deleted patch is the loaded one.
        let position = patch::list_patches(&dir)
            .iter()
            .position(|file| file == &path);
        let was_active = self.state.patch_active_path.as_deref() == Some(path.as_path());

        if let Err(error) = std::fs::remove_file(&path) {
            self.set_status(format!("{name} could not be deleted: {error}"), true);
            return;
        }

        self.clear_patch_confirmation();
        self.refresh_patch_list();
        if was_active {
            self.state.patch_active = None;
            self.state.patch_active_path = None;
            let neighbour = position
                .and_then(|position| {
                    let files = &self.state.patch_files;
                    files
                        .get(position)
                        .or_else(|| files.get(position.saturating_sub(1)))
                        .cloned()
                })
                .filter(|file| file != &path);
            match neighbour {
                Some(neighbour) => self.load_patch_file(&neighbour),
                // The folder has no patch left, so the factory preset that
                // the editor shows in the LCD becomes the loaded sound again.
                None => self.load_preset(self.state.preset_index),
            }
        }
        self.set_status(format!("Deleted {name}"), false);
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
        self.clear_patch_confirmation();
    }

    /// Rebuilds the JSON preview from the selected user patch, or from the
    /// current sound when a factory preset is selected. This is a read-only
    /// view of the actual document, not a second copy of the sound.
    fn refresh_patch_preview(&mut self) {
        refresh_patch_preview(self.params, self.state);
    }

    /// Loads a patch of the folder into the synthesizer.
    fn load_patch_file(&mut self, path: &Path) {
        match patch::load_patch(path, self.params, self.setter) {
            Ok(name) => {
                // The patch that is loaded is not the one that was waiting
                // for a confirmation anymore.
                self.clear_patch_confirmation();
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
            self.state.patch_confirm_delete = false;
            self.set_status("File exists - click SAVE again".to_string(), true);
            return;
        }

        match patch::save_patch(&dir, &name, self.params) {
            Ok(path) => {
                self.clear_patch_confirmation();
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
        self.clear_patch_confirmation();
        self.refresh_patch_list();
        self.refresh_patch_preview();
        self.set_status("Folder set".to_string(), false);
    }

    /// Writes a message into the status line of the patch browser.
    fn set_status(&mut self, message: String, error: bool) {
        self.state.patch_status = message;
        self.state.patch_status_error = error;
    }

    /// Forgets a pending SAVE or DELETE confirmation. Both ask for a second
    /// click, so every action that changes the target patch clears them.
    fn clear_patch_confirmation(&mut self) {
        self.state.patch_confirm_overwrite = false;
        self.state.patch_confirm_delete = false;
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
        // The release frame applies the whole drag at once.
        true
    }
}

// --- The layout -------------------------------------------------------------

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
    CURRENT_MODULE.with(|module| *module.borrow_mut() = title.to_ascii_uppercase());
    let painter = ui.painter_at(rect);
    let color = title_color(title);

    // No vertical accent bars and no coloured rule heads: the reference uses
    // blue section names on paper and lets the type do the grouping. The
    // titles are tracked out, which is what makes a 13 pt line read as a
    // section head instead of as a label.
    let font = FontId::new(font_size(13.0, zoom), bold());
    let galley = tracked_galley(
        &painter,
        &title.to_ascii_uppercase(),
        font,
        color,
        0.5 * zoom,
    );
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

/// Lays out a fader bank that uses the whole width of its section.
///
/// The slots share the available width equally, so a wide section gets a
/// wide, centered bank instead of a small one pinned to the left. The faders
/// themselves keep their design width; the extra room becomes breathing
/// space between them.
fn column_row_fill(
    ui: &mut egui::Ui,
    name: &str,
    count: usize,
    gap: f32,
    zoom: f32,
    mut add: impl FnMut(usize, &mut egui::Ui),
) {
    let gap = gap * zoom;
    let start = ui.next_widget_position();
    let total_width = ui.available_width();
    let height = ui.available_height();
    let slot = ((total_width - gap * (count as f32 - 1.0)) / count as f32).max(0.0);

    for index in 0..count {
        let rect = Rect::from_min_size(
            egui::pos2(start.x + index as f32 * (slot + gap), start.y),
            egui::vec2(slot, height),
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .id_salt(("sushi-column-fill", name, index))
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
    tracked_text(
        &ui.painter_at(rect),
        rect.center(),
        Align2::CENTER_CENTER,
        &text.to_ascii_uppercase(),
        FontId::new(font_size(size, zoom), label_family()),
        color,
        0.35 * zoom,
    );
}

/// Draws a small text label that takes just as much space as it needs.
fn text_label(ui: &mut egui::Ui, text: &str, size: f32, zoom: f32, color: Color32) {
    let font = FontId::new(font_size(size, zoom), label_family());
    let galley = tracked_galley(ui.painter(), text, font, color, 0.4 * zoom);
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
        ModSource::ModWheel => Some(feedback.mod_wheel()),
        // The envelopes belong to the voices. The audio thread publishes the
        // newest sounding voice, so a route can show a real value for them.
        ModSource::ModEnv => Some(feedback.mod_env()),
        ModSource::AmpEnv => Some(feedback.amp_env()),
        // These belong to one note. The audio thread publishes the newest
        // voice, so they have a value while a note is sounding.
        ModSource::Velocity => feedback.voice_active().then(|| feedback.velocity()),
        ModSource::Key => feedback
            .voice_active()
            .then(|| key_source_value(feedback.key())),
        ModSource::Random => Some(feedback.random()),
        ModSource::Off => None,
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
        egui::vec2(MOD_LIVE_MARKER_WIDTH * zoom, (MOD_POPUP_HEIGHT - 6.0) * zoom),
    ))
}

/// A number with a sign for positive and negative values. Zero stays bare, so
/// a route that is at rest does not print a plus sign.
fn format_signed_number(value: f32, decimals: usize) -> String {
    let text = format_number(value, decimals);
    if text == "0" {
        text
    } else if value > 0.0 {
        format!("+{text}")
    } else {
        text
    }
}



/// A signed whole percentage for a bipolar target or a modulation offset.
fn format_signed_percent(value: f32) -> String {
    let percent = (value * 100.0).round();
    if percent == 0.0 {
        "0%".to_string()
    } else {
        format!("{percent:+.0}%")
    }
}




/// A semitone value, used by the pitch destinations.
fn format_semitones(semitones: f32) -> String {
    format!("{} st", format_signed_number(semitones, 1))
}

/// A cents value, used by the fine pitch and detune destinations.
fn format_cents(cents: f32) -> String {
    format!("{} cents", format_signed_number(cents, 1))
}



/// A whole percentage, used by the unit destinations of the matrix.
fn format_whole_percent(value: f32) -> String {
    let percent = (value * 100.0).round();
    if percent == 0.0 {
        "0%".to_string()
    } else {
        format!("{percent:.0}%")
    }
}

/// A frequency in the compact units of the panel: hertz below one kilohertz
/// and kilohertz above it.
fn format_frequency(hz: f32) -> String {
    if hz >= 1_000.0 {
        format!("{} kHz", format_number(hz / 1_000.0, 1))
    } else {
        format!("{} Hz", format_number(hz, 0))
    }
}

/// A gain in decibels, with the scale of the volume fader.
fn format_gain_db(gain: f32) -> String {
    format!("{} dB", format_signed_number(util::gain_to_db(gain), 1))
}

/// The value that a destination reads right now, in its own units. This is the
/// centre that the modulation of a route moves around, and it is drawn in green
/// next to the live orange value.
///
/// `Pitch` has no single scalar parameter: it offsets every oscillator at once,
/// so it gets a description instead of a tune value.
fn target_current_text(dest: ModDest, params: &SushiParams) -> String {
    match dest {
        ModDest::Off => "off".to_string(),
        ModDest::Pitch => "all osc".to_string(),
        ModDest::Pulse1 => format_whole_percent(params.osc1_pw.value()),
        ModDest::Pulse2 => format_whole_percent(params.osc2_pw.value()),
        ModDest::Cutoff => format_frequency(params.cutoff.value()),
        ModDest::Resonance => format_whole_percent(params.resonance.value()),
        ModDest::Drive => format_whole_percent(params.filter_drive.value()),
        ModDest::Pan => format_signed_percent(params.vca_pan.value()),
        ModDest::Volume => format_gain_db(params.volume.value()),
        ModDest::Osc2Pitch => format_semitones(
            params.osc2_octave.value() as f32 * 12.0 + params.osc2_semi.value() as f32,
        ),
        ModDest::Osc2Fine => format_cents(params.osc2_fine.value()),
        ModDest::SubLevel => format_whole_percent(params.sub_level.value()),
        ModDest::NoiseLevel => format_whole_percent(params.noise_level.value()),
        ModDest::Detune => format_cents(params.detune.value()),
        ModDest::FilterEnv => format_signed_percent(params.filter_env_amount.value()),
        ModDest::Osc1Pitch => format_semitones(
            params.osc1_octave.value() as f32 * 12.0 + params.osc1_semi.value() as f32,
        ),
        ModDest::Osc1Fine => format_cents(params.osc1_fine.value()),
        ModDest::FilterDrive => format_whole_percent(params.filter_drive.value()),
        ModDest::FilterKeytrack => format_whole_percent(params.filter_keytrack.value()),
        ModDest::RingMod => format_whole_percent(params.ring_mod.value()),
    }
}

/// The value that a destination reads right now, in the same units as the
/// centre value of [`target_current_text`]. A route moves the centre by a
/// normalized amount, so the live line converts the contribution back into the
/// unit of the destination instead of printing a delta of another kind.
fn target_live_text(dest: ModDest, params: &SushiParams, contribution: f32) -> String {
    let percent = |base: f32, offset: f32| format_whole_percent((base + offset).clamp(0.0, 1.0));
    match dest {
        ModDest::Off => "off".to_string(),
        // Pitch has no scalar centre, so its live line stays in semitones.
        ModDest::Pitch => format_semitones(contribution * TARGET_PITCH_RANGE),
        ModDest::Osc1Pitch => format_semitones(
            params.osc1_octave.value() as f32 * 12.0
                + params.osc1_semi.value() as f32
                + contribution * TARGET_PITCH_RANGE,
        ),
        ModDest::Osc2Pitch => format_semitones(
            params.osc2_octave.value() as f32 * 12.0
                + params.osc2_semi.value() as f32
                + contribution * TARGET_PITCH_RANGE,
        ),
        ModDest::Pulse1 => percent(
            params.osc1_pw.value(),
            contribution * TARGET_PULSE_WIDTH_RANGE,
        ),
        ModDest::Pulse2 => percent(
            params.osc2_pw.value(),
            contribution * TARGET_PULSE_WIDTH_RANGE,
        ),
        ModDest::Cutoff => format_frequency(
            params.cutoff.value() * (contribution * TARGET_CUTOFF_RANGE * std::f32::consts::LN_2).exp(),
        ),
        ModDest::FilterEnv => {
            format_signed_percent((params.filter_env_amount.value() + contribution).clamp(-1.0, 1.0))
        }
        ModDest::Resonance => percent(params.resonance.value(), contribution),
        ModDest::Drive => percent(params.filter_drive.value(), contribution),
        ModDest::FilterDrive => percent(params.filter_drive.value(), contribution),
        ModDest::FilterKeytrack => percent(params.filter_keytrack.value(), contribution),
        ModDest::RingMod => percent(params.ring_mod.value(), contribution),
        ModDest::SubLevel => percent(params.sub_level.value(), contribution),
        ModDest::NoiseLevel => percent(
            params.noise_level.value(),
            contribution * TARGET_NOISE_RANGE,
        ),
        ModDest::Pan => {
            format_signed_percent((params.vca_pan.value() + contribution).clamp(-1.0, 1.0))
        }
        // The volume of a voice is a gain multiplier, so the live line is a
        // level in decibels like its centre.
        ModDest::Volume => format!(
            "{} dB",
            format_signed_number(
                util::gain_to_db(params.volume.value())
                    + 20.0 * (1.0 + contribution).max(0.001).log10(),
                1
            )
        ),
        ModDest::Osc1Fine => format_cents(params.osc1_fine.value() + contribution * TARGET_FINE_RANGE),
        ModDest::Osc2Fine => format_cents(params.osc2_fine.value() + contribution * TARGET_FINE_RANGE),
        ModDest::Detune => format_cents(params.detune.value() + contribution * TARGET_DETUNE_RANGE),
    }
}

/// The lowest and the highest contribution that a route made over the window
/// of the history, in the units of the matrix. `None` when the source has no
/// history, so the caller can fall back to the live value.
fn source_span(
    source: ModSource,
    feedback: &ModFeedback,
    amount: f32,
    history: &mut Vec<f32>,
) -> Option<(f32, f32)> {
    let mut low = f32::INFINITY;
    let mut high = f32::NEG_INFINITY;
    match source {
        ModSource::Lfo1 => feedback.history(FeedSource::Lfo1, history),
        ModSource::Lfo2 => feedback.history(FeedSource::Lfo2, history),
        ModSource::ModEnv => feedback.history(FeedSource::ModEnv, history),
        ModSource::AmpEnv => feedback.history(FeedSource::AmpEnv, history),
        ModSource::ModWheel => feedback.history(FeedSource::ModWheel, history),
        ModSource::Random => feedback.history(FeedSource::Random, history),
        _ => return None,
    }
    for value in history.iter() {
        low = low.min(*value);
        high = high.max(*value);
    }
    if !low.is_finite() || !high.is_finite() {
        return None;
    }
    let a = low * amount;
    let b = high * amount;
    Some((a.min(b), a.max(b)))
}

/// The strip of a source that belongs to one note. It has no history to draw,
/// so it shows what the source is: a level bar for the velocity, the strip of
/// the keyboard for the key and the steps of the played notes for the random.
#[allow(clippy::too_many_arguments)]
fn draw_note_source(
    painter: &egui::Painter,
    cell: Rect,
    source: ModSource,
    voice: (bool, f32, f32),
    shown_velocity: &mut f32,
    shown_hue: &mut f32,
    zoom: f32,
    seconds: f32,
) {
    let (voice_active, velocity_now, key_now) = voice;
    // The strips share one band in the middle of the cell, so a row of any
    // source keeps the same reading height.
    let height = cell.height() * 0.62;
    let track = Rect::from_center_size(cell.center(), egui::vec2(cell.width(), height));
    match source {
        ModSource::Velocity => {
            // A level meter, not a sample: the bar jumps to the velocity of the
            // new note at once and then falls back quickly, like the meter of a
            // mixer.
            let target = if voice_active {
                velocity_now.clamp(0.0, 1.0)
            } else {
                0.0
            };
            let shown = shown_velocity;
            if target > *shown {
                *shown = target;
            } else {
                // About a quarter of a second to fall from full scale.
                let fall = 1.0 - (-seconds / 0.22).exp();
                *shown += (target - *shown) * fall;
            }
            let velocity = shown.clamp(0.0, 1.0);
            painter.rect_filled(track, CornerRadius::ZERO, palette().field_bg);
            painter.rect_stroke(
                track,
                CornerRadius::ZERO,
                Stroke::new(1.0, palette().line),
                egui::StrokeKind::Inside,
            );
            let fill = Rect::from_min_max(
                egui::pos2(track.left() + zoom, track.top() + zoom),
                egui::pos2(
                    track.left() + zoom + (track.width() - 2.0 * zoom) * velocity,
                    track.bottom() - zoom,
                ),
            );
            painter.rect_filled(fill, CornerRadius::ZERO, palette().accent);
        }
        _ => {
            // One vivid bar for the key. Its hue follows the note along the
            // rainbow, and the hue travels to the new colour instead of
            // jumping, so a played melody is a smooth sweep of colour.
            let target = ((key_now - 36.0) / 48.0).clamp(0.0, 1.0) * 300.0;
            let shown = shown_hue;
            // The shortest way around the circle, at about a twentieth of a
            // second, which reads as a quick fade and never as a step.
            let mut difference = (target - *shown).rem_euclid(360.0);
            if difference > 180.0 {
                difference -= 360.0;
            }
            let travel = 1.0 - (-seconds / 0.05).exp();
            *shown = (*shown + difference * travel).rem_euclid(360.0);
            let color = spectrum_color(*shown, 0.92);
            painter.rect_filled(track, CornerRadius::ZERO, color);
        }
    }
}

/// A colour of the keyboard strip: the hue in degrees, with a low saturation
/// and value, so the spectrum stays a quiet structure line of the panel.
fn spectrum_color(hue: f32, value: f32) -> egui::Color32 {
    let h = (hue.rem_euclid(360.0)) / 60.0;
    let saturation = 0.55;
    let chroma = value * saturation;
    let x = chroma * (1.0 - ((h % 2.0) - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (chroma, x, 0.0),
        1 => (x, chroma, 0.0),
        2 => (0.0, chroma, x),
        3 => (0.0, x, chroma),
        4 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    let m = value - chroma;
    egui::Color32::from_rgb(
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
    )
}

/// The key of a note as a modulation value, `-1..1`, with the middle C in the
/// middle. The DSP uses the same mapping, so the panel shows the value that
/// actually reaches the matrix.
fn key_source_value(key: f32) -> f32 {
    // Two octaves above and below the middle C reach the ends of the range,
    // exactly like `dsp::mod_matrix::key_source`.
    ((key - 60.0) / 24.0).clamp(-1.0, 1.0)
}

/// The characteristic of a source that belongs to one note: the curve that the
/// source reads from, drawn from the bottom left to the top right. The dot of
/// the live value sits on it, so a velocity or a key route is visible even
/// though it has no history of its own.

/// The recorded history of one source of the matrix, oldest first. The hand
/// controls and the envelopes have a history too, so a route always draws what
/// its source really did. `None` means the source has no value the panel can
/// read, which is true of the sources that belong to one note.
fn source_history<'a>(
    source: ModSource,
    feedback: &ModFeedback,
    scratch: &'a mut Vec<f32>,
) -> Option<&'a [f32]> {
    let feed = match source {
        ModSource::Lfo1 => FeedSource::Lfo1,
        ModSource::Lfo2 => FeedSource::Lfo2,
        ModSource::ModEnv => FeedSource::ModEnv,
        ModSource::AmpEnv => FeedSource::AmpEnv,
        ModSource::ModWheel => FeedSource::ModWheel,
        ModSource::Random => FeedSource::Random,
        _ => return None,
    };
    feedback.history(feed, scratch);
    Some(scratch)
}








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

/// A compact horizontal fader for the FX chain strip. The whole track is the
/// click and drag target; the handle never leaves the track.
fn h_slider(
    ui: &mut egui::Ui,
    rect: Rect,
    value: &mut f32,
    knob: &Knob,
    zoom: f32,
) -> egui::Response {
    let mut response = ui.interact(
        rect,
        ui.scope_id().with("sushi-h-slider"),
        Sense::click_and_drag(),
    );
    let painter = ui.painter_at(rect);
    flat_box(&painter, rect, palette().track);

    let normalized = value_to_normalized(*value, knob.min, knob.max, knob.log);
    let handle_x = rect.left() + normalized * rect.width();
    let fill = Rect::from_min_max(
        egui::pos2(rect.left() + zoom, rect.top() + zoom),
        egui::pos2(handle_x, rect.bottom() - zoom),
    );
    painter.rect_filled(fill, CornerRadius::ZERO, palette().button_active);
    let handle = Rect::from_center_size(
        egui::pos2(handle_x, rect.center().y),
        egui::vec2(5.0 * zoom, (rect.height() - 2.0 * zoom).max(2.0)),
    );
    painter.rect_filled(handle, CornerRadius::ZERO, palette().handle);

    let pointer = response
        .interact_pointer_pos()
        .or_else(|| ui.input(|input| input.pointer.hover_pos()));
    let primary_down = ui.input(|input| input.pointer.primary_down());
    if (response.clicked_by(egui::PointerButton::Primary)
        || response.dragged_by(egui::PointerButton::Primary)
        || (response.is_pointer_button_down_on() && primary_down))
        && let Some(pointer) = pointer
    {
        let position = ((pointer.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
        let next = position_to_value(position, knob.min, knob.max, knob.log);
        if next != *value {
            *value = next;
            response.mark_changed();
        }
    }

    response.on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
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

/// Fits a text into a width by cutting it off with an explicit ellipsis. The
/// target inspector uses it for the route summary, so a long combination of a
/// source and a destination can never run into the amount on the right.
fn fit_text(painter: &egui::Painter, text: &str, max_width: f32, font: &FontId) -> String {
    if text_width(painter, text, font) <= max_width {
        return text.to_owned();
    }

    let mut chars: Vec<char> = text.chars().collect();
    loop {
        chars.pop();
        let mut fitted: String = chars.iter().collect();
        fitted.push_str(LCD_ELLIPSIS);
        if text_width(painter, &fitted, font) <= max_width {
            return fitted;
        }
        if chars.is_empty() {
            return String::new();
        }
    }
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
    let mut font = FontId::new(font_size(size, zoom), value_family());
    while size > LCD_MIN_FONT_SIZE && text_width(painter, text, &font) > max_width {
        size -= 1.0;
        font = FontId::new(font_size(size, zoom), value_family());
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
    let arrow = matches!(text, "<" | ">");
    tracked_text(
        &painter,
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        if arrow {
            FontId::new(font_size(11.0, zoom), value_family())
        } else {
            FontId::new(
                font_size(10.0, zoom),
                if primary { bold() } else { label_family() },
            )
        },
        palette().text,
        if arrow { 0.0 } else { 0.4 * zoom },
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
    tracked_text(
        &painter,
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::new(
            font_size(10.0, zoom),
            if active { bold() } else { label_family() },
        ),
        palette().text,
        0.5 * zoom,
    );

    response
}

/// The small oscilloscope of the header. It is just here to give the panel a
/// bit of life, not to be a measurement tool.
fn small_scope(painter: &egui::Painter, rect: Rect, history: &VecDeque<f32>, zoom: f32) {
    // A dark instrument screen with a quiet grid, even while the sound is
    // silent. It is the one live object in the header.
    // Nothing but the trace: the grid of the old screen is gone.
    flat_box(painter, rect, palette().lcd_bg);
    let inner = rect.shrink(3.0 * zoom);

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
    let mut visuals = egui::Visuals::dark();
    visuals.dark_mode = true;
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
            egui::FontId::new(font_size(11.0, zoom), label_family()),
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

/// Draws the square of a check box and its check mark when it is set.
fn draw_check_box(painter: &egui::Painter, rect: Rect, checked: bool, zoom: f32) {
    flat_box(painter, rect, palette().track);
    if checked {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            "X",
            FontId::monospace(font_size(9.0, zoom)),
            palette().text,
        );
    }
}





