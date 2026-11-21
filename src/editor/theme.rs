
use egui::Color32;

pub struct Palette {
    pub bg: Color32,
    pub header_bg: Color32,
    pub rule: Color32,
    pub line: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub paper: Color32,
    pub title_blue: Color32,
    pub label: Color32,
    pub accent: Color32,
    pub track: Color32,
    pub handle: Color32,
    pub tick: Color32,
    pub field_bg: Color32,
    pub button_bg: Color32,
    pub button_active: Color32,
    pub primary_bg: Color32,
    pub lcd_bg: Color32,
    pub lcd_fg: Color32,
    pub led_on: Color32,
    pub led_off: Color32,
    pub trace: Color32,
    pub status_ok: Color32,
    pub status_error: Color32,
}

/// Black plate, one orange accent, green for anything that is data.
///
/// Every surface is a flat fill with a single 1 px line: no bevel, no shadow,
/// no gradient, and the only two hues that are allowed to speak are the orange
/// of the accent and the green of the readouts, the LEDs and the trace.
pub static LEGACY: Palette = Palette {
    bg: Color32::from_rgb(0x0A, 0x0A, 0x0B),
    header_bg: Color32::from_rgb(0x10, 0x10, 0x12),
    rule: Color32::from_rgb(0x1C, 0x1C, 0x1F),
    line: Color32::from_rgb(0x2C, 0x2C, 0x30),
    text: Color32::from_rgb(0xE9, 0xE9, 0xEA),
    muted: Color32::from_rgb(0x77, 0x77, 0x7D),
    paper: Color32::from_rgb(0x12, 0x12, 0x15),
    title_blue: Color32::from_rgb(0xE8, 0x7A, 0x28),
    label: Color32::from_rgb(0x8B, 0x8B, 0x91),
    accent: Color32::from_rgb(0xE8, 0x7A, 0x28),
    track: Color32::from_rgb(0x20, 0x20, 0x24),
    handle: Color32::from_rgb(0xE9, 0xE9, 0xEA),
    tick: Color32::from_rgb(0x3A, 0x3A, 0x3F),
    field_bg: Color32::from_rgb(0x16, 0x16, 0x1A),
    button_bg: Color32::from_rgb(0x1A, 0x1A, 0x1E),
    button_active: Color32::from_rgb(0x2A, 0x21, 0x18),
    primary_bg: Color32::from_rgb(0xB8, 0x62, 0x1C),
    lcd_bg: Color32::from_rgb(0x0D, 0x0F, 0x0C),
    lcd_fg: Color32::from_rgb(0x5C, 0xC9, 0x4A),
    led_on: Color32::from_rgb(0x5C, 0xC9, 0x4A),
    led_off: Color32::from_rgb(0x26, 0x26, 0x2A),
    trace: Color32::from_rgb(0x5C, 0xC9, 0x4A),
    status_ok: Color32::from_rgb(0x5C, 0xC9, 0x4A),
    status_error: Color32::from_rgb(0xE0, 0x52, 0x3C),
};

/// The palette of the panel. Sushi is Legacy-only, so this is always warm.
pub fn palette() -> &'static Palette {
    &LEGACY
}
