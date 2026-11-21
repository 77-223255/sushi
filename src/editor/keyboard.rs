
use std::collections::BTreeSet;

use egui::{Align2, FontId, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2};

use super::{flat_box, font_size, palette, value_family};
use crate::keyboard::HeldKeys;

// --- The design of the strip ------------------------------------------------

/// How many octaves the strip shows.
const OCTAVES: usize = 5;
/// The height of a black key, as a share of the height of a white key.
const BLACK_SHARE: f32 = 0.62;
/// The width of a black key, as a share of the width of a white key.
const BLACK_WIDTH_SHARE: f32 = 0.62;

/// The lowest octave that the computer keyboard can play.
///
/// A note is `(octave + 1) * 12 + semitone`, so middle C is the C of octave
/// four, as it is written in the rest of the world.
const MIN_OCTAVE: i32 = 1;
/// The highest octave that the computer keyboard can play.
const MAX_OCTAVE: i32 = 5;
/// The octave that the computer keyboard starts at. It puts middle C in the
/// middle of the strip.
const DEFAULT_OCTAVE: i32 = 3;

/// The velocity of a note that the computer keyboard plays.
const TYPED_VELOCITY: f32 = 0.8;
/// The velocity of a click at the very top of a key.
const CLICK_VELOCITY_TOP: f32 = 1.0;
/// The velocity of a click at the very bottom of a key.
const CLICK_VELOCITY_BOTTOM: f32 = 0.35;

/// The semitones of the white keys of an octave.
const WHITE_SEMITONES: [u8; 7] = [0, 2, 4, 5, 7, 9, 11];
/// The semitones of the black keys of an octave.
const BLACK_SEMITONES: [u8; 5] = [1, 3, 6, 8, 10];

/// Whether a black key sits above the white key of the given index in its
/// octave. E and B have none, so the black keys come in the usual groups of
/// two and three.
fn has_black_above(index: usize) -> bool {
    let semitone = WHITE_SEMITONES[index % 7];
    BLACK_SEMITONES.contains(&(semitone + 1))
}

/// The middle row of the computer keyboard: the white keys, starting at the
/// C of the current octave.
const TYPED_WHITE: [(egui::Key, u8); 11] = [
    (egui::Key::A, 0),
    (egui::Key::S, 2),
    (egui::Key::D, 4),
    (egui::Key::F, 5),
    (egui::Key::G, 7),
    (egui::Key::H, 9),
    (egui::Key::J, 11),
    (egui::Key::K, 12),
    (egui::Key::L, 14),
    (egui::Key::Semicolon, 16),
    (egui::Key::Quote, 17),
];

/// The row above the white keys: the black keys.
const TYPED_BLACK: [(egui::Key, u8); 7] = [
    (egui::Key::W, 1),
    (egui::Key::E, 3),
    (egui::Key::T, 6),
    (egui::Key::Y, 8),
    (egui::Key::U, 10),
    (egui::Key::O, 13),
    (egui::Key::P, 15),
];

// --- The keys of the strip --------------------------------------------------

#[derive(Clone, Copy, Debug)]
struct KeyBed {
    /// The rect that holds the keys.
    rect: Rect,
    /// The width of one white key.
    white_width: f32,
    /// The height of one black key.
    black_height: f32,
    /// The octave of the first key of the strip.
    first_octave: i32,
}

impl KeyBed {
    fn new(rect: Rect, first_octave: i32) -> Self {
        let white_width = rect.width() / (OCTAVES as f32 * 7.0 + 1.0);

        Self {
            rect,
            white_width,
            black_height: rect.height() * BLACK_SHARE,
            first_octave,
        }
    }

    /// The number of white keys, including the closing C of the last octave.
    fn white_count() -> usize {
        OCTAVES * 7 + 1
    }

    /// The rect of a white key.
    fn white_rect(&self, index: usize) -> Rect {
        Rect::from_min_size(
            pos2(
                self.rect.left() + index as f32 * self.white_width,
                self.rect.top(),
            ),
            vec2(self.white_width, self.rect.height()),
        )
    }

    /// The rect of the black key that follows the white key with the given
    /// index, if that white key has one above its right edge.
    fn black_rect(&self, index: usize) -> Option<Rect> {
        if !has_black_above(index % 7) || index + 1 >= Self::white_count() {
            return None;
        }
        if index / 7 >= OCTAVES {
            return None;
        }

        let width = self.white_width * BLACK_WIDTH_SHARE;
        let center = self.rect.left() + (index + 1) as f32 * self.white_width;
        Some(Rect::from_min_size(
            pos2(center - width * 0.5, self.rect.top()),
            vec2(width, self.black_height),
        ))
    }

    /// The note of a white key of the strip.
    fn white_note(&self, index: usize) -> u8 {
        let octave = self.first_octave + index as i32 / 7;
        let semitone = WHITE_SEMITONES[index % 7];
        (octave * 12 + semitone as i32).clamp(0, 127) as u8
    }

    /// The note of a black key that follows a white key.
    fn black_note(&self, index: usize) -> u8 {
        let octave = self.first_octave + index as i32 / 7;
        let semitone = WHITE_SEMITONES[index % 7] + 1;
        (octave * 12 + semitone as i32).clamp(0, 127) as u8
    }

    /// The note under the pointer. The black keys are on top, so they are
    /// tested first.
    fn note_at(&self, pointer: Pos2) -> Option<u8> {
        if !self.rect.contains(pointer) {
            return None;
        }

        if pointer.y < self.rect.top() + self.black_height {
            let index = ((pointer.x - self.rect.left()) / self.white_width).floor() as isize;
            // The black key of a boundary can be reached from the white key
            // on either side of it.
            for candidate in [index - 1, index] {
                if candidate < 0 {
                    continue;
                }
                let candidate = candidate as usize;
                if let Some(rect) = self.black_rect(candidate)
                    && rect.contains(pointer)
                {
                    return Some(self.black_note(candidate));
                }
            }
        }

        let index = ((pointer.x - self.rect.left()) / self.white_width).floor() as isize;
        if index < 0 || index as usize >= Self::white_count() {
            return None;
        }
        Some(self.white_note(index as usize))
    }
}

#[derive(Clone, Copy, Debug)]
struct Chrome {
    strip: Rect,
    keys: Rect,
}

impl Chrome {
    /// The strip is nothing but keys now: the octave controls and their
    /// readout moved into the info panel next to the keys.
    fn new(rect: Rect, _zoom: f32) -> Self {
        Self {
            strip: rect,
            keys: rect,
        }
    }
}

// --- The widget -------------------------------------------------------------

/// The state of the keyboard strip of one editor.
pub struct VirtualKeyboard {
    /// The octave of the first note that the computer keyboard plays.
    octave: i32,
    /// The notes that this editor started and has not released yet.
    held: BTreeSet<u8>,
}

impl Default for VirtualKeyboard {
    fn default() -> Self {
        Self {
            octave: DEFAULT_OCTAVE,
            held: BTreeSet::new(),
        }
    }
}

impl VirtualKeyboard {
    
    /// Moves the computer keyboard by whole octaves, as `Z` and `X` do.
    pub fn shift_octave(&mut self, delta: i32) {
        self.octave = (self.octave + delta).clamp(MIN_OCTAVE, MAX_OCTAVE);
    }

    /// The note of the `A` key.
    fn base_note(&self) -> u8 {
        ((self.octave + 1) * 12).clamp(0, 127) as u8
    }

    /// The octave of the first key of the strip.
    fn first_octave(&self) -> i32 {
        first_octave_of(self.octave)
    }

    /// Releases every note that this editor started. It is used when the
    /// panel closes, so a held key cannot leave a stuck voice behind.
    pub fn release_all<T>(&mut self, send: T)
    where
        T: FnMut(u8, f32, bool),
    {
        let mut send = send;
        for note in std::mem::take(&mut self.held) {
            send(note, 0.0, false);
        }
    }

    /// Draws the strip and applies the keys that the mouse and the computer
    /// keyboard play.
    pub fn show<T>(&mut self, ui: &Ui, rect: Rect, zoom: f32, down: &HeldKeys, mut send: T)
    where
        T: FnMut(u8, f32, bool),
    {
        let chrome = Chrome::new(rect, zoom);
        let painter = ui.painter_at(rect);
        flat_box(&painter, chrome.strip, palette().field_bg);

        // The typed octave is shifted before the notes are read, so a frame
        // that presses `X` already plays the new octave. A text field keeps
        // its `z` and `x` for typing.
        let typing = ui.ctx().text_edit_focused();
        let shifted = if typing {
            0
        } else {
            ui.input_mut(|input| {
                let mut shifted = 0;
                if input.consume_key(egui::Modifiers::NONE, egui::Key::X) {
                    shifted += 1;
                }
                if input.consume_key(egui::Modifiers::NONE, egui::Key::Z) {
                    shifted -= 1;
                }
                shifted
            })
        };
        if shifted != 0 {
            self.shift_octave(shifted);
        }

        let bed = KeyBed::new(chrome.keys, self.first_octave());

        // Everything that should sound in this frame, with the velocity that
        // started it.
        let mut wanted: Vec<(u8, f32)> = Vec::new();

        // The computer keyboard. Text fields keep their keys, and a shortcut
        // never plays a note.
        let modifiers = ui.input(|input| input.modifiers);
        if !typing && !modifiers.command && !modifiers.ctrl && !modifiers.alt {
            let base = self.base_note() as i32;
            ui.input(|input| {
                for (key, semitone) in TYPED_WHITE.iter().chain(TYPED_BLACK.iter()) {
                    if input.key_down(*key) {
                        let note = base + *semitone as i32;
                        if (0..=127).contains(&note) {
                            wanted.push((note as u8, TYPED_VELOCITY));
                        }
                    }
                }
            });
        }

        // The mouse. The strip only takes the pointer when the press started
        // on a key, so a drag that began on a fader is never stolen.
        let response = ui.interact(
            chrome.keys,
            ui.scope_id().with("sushi-keyboard-keys"),
            Sense::click_and_drag(),
        );
        let (pointer, primary_down) =
            ui.input(|input| (input.pointer.interact_pos(), input.pointer.primary_down()));
        let pointer_note = if primary_down && response.is_pointer_button_down_on() {
            pointer.and_then(|pointer| {
                bed.note_at(pointer)
                    .map(|note| (note, click_velocity(&bed, pointer)))
            })
        } else {
            None
        };
        if let Some(note) = pointer_note {
            wanted.push(note);
        }

        self.apply(wanted, &mut send);

        // The keys, with the notes that actually sound lit up.
        self.paint_keys(&painter, &bed, zoom, down);
    }

    /// Sends the difference between the keys that should sound and the keys
    /// that still sound, so a frame never retriggers a held note.
    fn apply<T>(&mut self, wanted: Vec<(u8, f32)>, send: &mut T)
    where
        T: FnMut(u8, f32, bool),
    {
        let wanted_notes: BTreeSet<u8> = wanted.iter().map(|(note, _)| *note).collect();

        for note in self.held.difference(&wanted_notes) {
            send(*note, 0.0, false);
        }
        for (note, velocity) in &wanted {
            if !self.held.contains(note) {
                send(*note, *velocity, true);
            }
        }

        self.held = wanted_notes;
    }


    /// Draws the keys and the lights of the notes that sound.
    fn paint_keys(&self, painter: &egui::Painter, bed: &KeyBed, zoom: f32, down: &HeldKeys) {
        let lit = |note: u8| self.held.contains(&note) || down.is_down(note);
        // The note that the `A` key plays. Its name is the red mark of the
        // strip: it shows the home of the computer keyboard without a bar
        // that would cover the top of the key.
        let base = self.base_note();

        for index in 0..KeyBed::white_count() {
            let note = bed.white_note(index);
            let rect = bed.white_rect(index);
            let on = lit(note);
            painter.rect_filled(
                rect.shrink(0.5),
                egui::CornerRadius::ZERO,
                if on {
                    palette().primary_bg
                } else {
                    palette().paper
                },
            );
            painter.rect_stroke(
                rect.shrink(0.5),
                egui::CornerRadius::ZERO,
                Stroke::new(1.0, palette().line),
                egui::StrokeKind::Inside,
            );

            // The name of every C, so the player can find the octave. The C
            // of the computer keyboard is red, which is what marks the
            // octave that the layout plays.
            if note.is_multiple_of(12) {
                let color = if note == base {
                    palette().accent
                } else if on {
                    palette().line
                } else {
                    palette().muted
                };
                painter.text(
                    pos2(rect.center().x, rect.bottom() - 3.0 * zoom),
                    Align2::CENTER_BOTTOM,
                    note_name(note),
                    FontId::new(font_size(6.6, zoom), value_family()),
                    color,
                );
            }
        }

        for index in 0..KeyBed::white_count() {
            let Some(rect) = bed.black_rect(index) else {
                continue;
            };
            let note = bed.black_note(index);
            let on = lit(note);
            painter.rect_filled(
                rect.shrink(0.5),
                egui::CornerRadius::ZERO,
                if on { palette().accent } else { palette().line },
            );
        }
    }
}


/// The octave of the first key of the strip for the octave of the computer
/// keyboard. The strip follows the typed octave, so the typed notes always
/// have a key to light up.
fn first_octave_of(octave: i32) -> i32 {
    (octave - 1).clamp(0, 3)
}

/// The velocity of a click at the given place: the higher on the key, the
/// louder the note, like the keyboard of Serum.
fn click_velocity(bed: &KeyBed, pointer: Pos2) -> f32 {
    let share = ((pointer.y - bed.rect.top()) / bed.rect.height()).clamp(0.0, 1.0);
    CLICK_VELOCITY_TOP + (CLICK_VELOCITY_BOTTOM - CLICK_VELOCITY_TOP) * share
}

/// The name of a note, with middle C (60) as C4.
pub fn note_name(note: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    format!("{}{}", NAMES[note as usize % 12], note as i32 / 12 - 1)
}

