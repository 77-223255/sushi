//! The two color palettes of the panel.
//!
//! The modern flavor keeps the light SH-101-inspired paper palette. The legacy
//! flavor uses a warmer, aged-paper and burnt-orange palette. The selected
//! palette is thread local because all painting happens on the GUI thread and
//! several plugin instances can be open in the same process.

use egui::Color32;
use std::cell::Cell;

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
    pub led_red: Color32,
    pub led_off: Color32,
    pub trace: Color32,
    pub status_ok: Color32,
    pub status_error: Color32,
}

pub static MODERN: Palette = Palette {
    bg: Color32::from_rgb(0xF1, 0xF0, 0xEB),
    header_bg: Color32::from_rgb(0xE6, 0xE5, 0xDF),
    rule: Color32::from_rgb(0xBC, 0xBB, 0xB4),
    line: Color32::from_rgb(0x55, 0x55, 0x51),
    text: Color32::from_rgb(0x1D, 0x1D, 0x1B),
    muted: Color32::from_rgb(0x75, 0x73, 0x6B),
    paper: Color32::from_rgb(0xFB, 0xFA, 0xF6),
    title_blue: Color32::from_rgb(0x2E, 0x5C, 0x9C),
    label: Color32::from_rgb(0x35, 0x5D, 0x88),
    accent: Color32::from_rgb(0x2E, 0x5C, 0x9C),
    track: Color32::from_rgb(0xFB, 0xFA, 0xF6),
    handle: Color32::from_rgb(0x1C, 0x1C, 0x1A),
    tick: Color32::from_rgb(0x8A, 0x89, 0x81),
    field_bg: Color32::from_rgb(0xFC, 0xFB, 0xF7),
    button_bg: Color32::from_rgb(0xE3, 0xE2, 0xDC),
    button_active: Color32::from_rgb(0xD4, 0xDC, 0xE8),
    primary_bg: Color32::from_rgb(0xDD, 0xE4, 0xEF),
    lcd_bg: Color32::from_rgb(0xF6, 0xF6, 0xF1),
    lcd_fg: Color32::from_rgb(0x23, 0x4F, 0x82),
    led_on: Color32::from_rgb(0x2E, 0x5C, 0x9C),
    led_red: Color32::from_rgb(0xB2, 0x3A, 0x32),
    led_off: Color32::from_rgb(0xC9, 0xC8, 0xC1),
    trace: Color32::from_rgb(0x2E, 0x5C, 0x9C),
    status_ok: Color32::from_rgb(0x2E, 0x5C, 0x9C),
    status_error: Color32::from_rgb(0xA3, 0x3A, 0x30),
};

pub static LEGACY: Palette = Palette {
    bg: Color32::from_rgb(0xE9, 0xDE, 0xC7),
    header_bg: Color32::from_rgb(0xDC, 0xCB, 0xAA),
    rule: Color32::from_rgb(0xB0, 0x9A, 0x76),
    line: Color32::from_rgb(0x3A, 0x2E, 0x24),
    text: Color32::from_rgb(0x2B, 0x21, 0x18),
    muted: Color32::from_rgb(0x7A, 0x68, 0x52),
    paper: Color32::from_rgb(0xF7, 0xEE, 0xDB),
    title_blue: Color32::from_rgb(0xB3, 0x4A, 0x2B),
    label: Color32::from_rgb(0x7D, 0x3E, 0x25),
    accent: Color32::from_rgb(0xB3, 0x4A, 0x2B),
    track: Color32::from_rgb(0xF8, 0xF0, 0xDD),
    handle: Color32::from_rgb(0x2B, 0x21, 0x18),
    tick: Color32::from_rgb(0x9B, 0x84, 0x66),
    field_bg: Color32::from_rgb(0xFB, 0xF4, 0xE3),
    button_bg: Color32::from_rgb(0xD8, 0xC5, 0xA5),
    button_active: Color32::from_rgb(0xE7, 0xC0, 0x8E),
    primary_bg: Color32::from_rgb(0xE8, 0xC1, 0x8E),
    lcd_bg: Color32::from_rgb(0xEA, 0xD9, 0xB8),
    lcd_fg: Color32::from_rgb(0x6F, 0x2A, 0x18),
    led_on: Color32::from_rgb(0xC0, 0x5A, 0x2A),
    led_red: Color32::from_rgb(0x9A, 0x2E, 0x22),
    led_off: Color32::from_rgb(0xBB, 0xA9, 0x8C),
    trace: Color32::from_rgb(0xB5, 0x53, 0x2E),
    status_ok: Color32::from_rgb(0x6B, 0x7A, 0x3A),
    status_error: Color32::from_rgb(0x9A, 0x2E, 0x22),
};

thread_local! {
    static LEGACY_SELECTED: Cell<bool> = const { Cell::new(false) };
}

pub fn set_legacy(legacy: bool) {
    LEGACY_SELECTED.with(|selected| selected.set(legacy));
}

pub fn palette() -> &'static Palette {
    if LEGACY_SELECTED.with(Cell::get) {
        &LEGACY
    } else {
        &MODERN
    }
}
