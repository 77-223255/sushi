//! The keyboard strip under the four bands.
//!
//! The strip is a small piano that behaves like the one of Serum: clicking a
//! key plays it, dragging across the keys glides from note to note, and every
//! key that sounds is lit — no matter whether the note came from the mouse,
//! from the computer keyboard or from a MIDI keyboard that plays into the
//! plugin.
//!
//! The computer keyboard follows the layout of Ableton Live: the middle row
//! (`A S D F G H J K L ; '`) plays the white keys, the row above it
//! (`W E T Y U O P`) plays the black keys, and `Z` and `X` move the whole
//! layout one octave down or up.

use std::collections::BTreeSet;

use egui::{Align2, FontId, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2};

use super::{flat_box, font_size, label_family, palette, tracked_text, value_family};
use crate::keyboard::HeldKeys;

// --- The design of the strip ------------------------------------------------

/// The height of the whole strip, in the design size of the panel.
pub const STRIP_HEIGHT: f32 = 72.0;
/// The padding between the strip and its keys.
const STRIP_PADDING: f32 = 4.0;
/// The width of the column that holds the octave controls.
const COLUMN_WIDTH: f32 = 52.0;
/// How many octaves the strip shows.
const OCTAVES: usize = 5;
/// The height of a black key, as a share of the height of a white key.
const BLACK_SHARE: f32 = 0.62;
/// The width of a black key, as a share of the width of a white key.
const BLACK_WIDTH_SHARE: f32 = 0.62;
/// The height of the octave buttons.
const OCTAVE_BUTTON_HEIGHT: f32 = 16.0;
/// The width of the octave buttons.
const OCTAVE_BUTTON_WIDTH: f32 = 20.0;

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

/// The places of the keys of the strip.
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

/// The fixed places of the strip: the box, the octave controls and the keys.
#[derive(Clone, Copy, Debug)]
struct Chrome {
    strip: Rect,
    caption: Rect,
    down: Rect,
    up: Rect,
    readout: Rect,
    hint: Rect,
    keys: Rect,
}

impl Chrome {
    fn new(rect: Rect, zoom: f32) -> Self {
        let padding = STRIP_PADDING * zoom;
        let inner = rect.shrink(padding);
        let column = Rect::from_min_size(
            inner.min,
            vec2(COLUMN_WIDTH * zoom - padding, inner.height()),
        );

        // The column is a small stack: the caption, the two buttons, the
        // octave readout and the shortcut hint. The rows never overlap, so a
        // text of the strip cannot run into its neighbour.
        let caption = Rect::from_min_size(column.min, vec2(column.width(), 10.0 * zoom));
        let button = vec2(OCTAVE_BUTTON_WIDTH * zoom, OCTAVE_BUTTON_HEIGHT * zoom);
        let down = Rect::from_min_size(pos2(column.left(), caption.bottom() + 3.0 * zoom), button);
        let up = Rect::from_min_size(pos2(down.right() + 4.0 * zoom, down.top()), button);
        let readout = Rect::from_min_size(
            pos2(column.left(), down.bottom() + 3.0 * zoom),
            vec2(column.width(), 14.0 * zoom),
        );
        let hint = Rect::from_min_size(
            pos2(column.left(), column.bottom() - 11.0 * zoom),
            vec2(column.width(), 11.0 * zoom),
        );
        let keys = Rect::from_min_size(
            pos2(column.right() + 6.0 * zoom, inner.top()),
            vec2(inner.right() - column.right() - 6.0 * zoom, inner.height()),
        );

        Self {
            strip: rect,
            caption,
            down,
            up,
            readout,
            hint,
            keys,
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
    /// The octave of the computer keyboard.
    #[cfg(test)]
    pub fn octave(&self) -> i32 {
        self.octave
    }

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

        // The octave controls and their readout.
        self.octave_buttons(ui, &chrome, zoom);
        tracked_text(
            &painter,
            chrome.caption.center(),
            Align2::CENTER_CENTER,
            "OCT",
            FontId::new(font_size(7.5, zoom), label_family()),
            palette().muted,
            0.8 * zoom,
        );
        painter.text(
            chrome.readout.center(),
            Align2::CENTER_CENTER,
            note_name(self.base_note()),
            FontId::new(font_size(11.0, zoom), value_family()),
            palette().title_blue,
        );
        tracked_text(
            &painter,
            chrome.hint.center(),
            Align2::CENTER_CENTER,
            "Z/X",
            FontId::monospace(font_size(7.2, zoom)),
            palette().muted,
            0.4 * zoom,
        );

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

    /// The two octave buttons of the strip.
    fn octave_buttons(&mut self, ui: &Ui, chrome: &Chrome, zoom: f32) {
        for (rect, label, delta) in [(chrome.down, "<", -1), (chrome.up, ">", 1)] {
            let response = ui.interact(
                rect,
                ui.scope_id().with(("sushi-keyboard-octave", label)),
                Sense::click(),
            );
            let enabled = (self.octave + delta).clamp(MIN_OCTAVE, MAX_OCTAVE) != self.octave;
            let fill = if !enabled {
                palette().track
            } else if response.is_pointer_button_down_on() {
                palette().button_active
            } else {
                palette().button_bg
            };
            flat_box(&ui.painter_at(rect), rect, fill);
            ui.painter_at(rect).text(
                rect.center(),
                Align2::CENTER_CENTER,
                label,
                FontId::new(font_size(9.0, zoom), value_family()),
                if enabled {
                    palette().text
                } else {
                    palette().muted
                },
            );

            if response.clicked() && enabled {
                self.shift_octave(delta);
            }
        }
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

/// The rect and the note of a white key of the strip. The tests of the panel
/// click a key of the real layout with it, so the note that a click plays is
/// never guessed.
#[cfg(test)]
pub fn white_key(strip: Rect, zoom: f32, octave: i32, index: usize) -> (Rect, u8) {
    let bed = KeyBed::new(Chrome::new(strip, zoom).keys, first_octave_of(octave));
    (bed.white_rect(index), bed.white_note(index))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keyboard::HeldKeys;

    /// The strip of the design size of the panel.
    fn strip() -> Rect {
        Rect::from_min_size(pos2(8.0, 588.0), vec2(944.0, STRIP_HEIGHT))
    }

    /// The keys of the strip at the default octave.
    fn bed() -> KeyBed {
        let chrome = Chrome::new(strip(), 1.0);
        KeyBed::new(chrome.keys, first_octave_of(DEFAULT_OCTAVE))
    }

    /// One editor frame of the strip. The context is kept alive between the
    /// calls, because egui only knows which widget a press belongs to after it
    /// has seen the widget in the frame before.
    struct Harness {
        ctx: egui::Context,
        time: f64,
        keys: VirtualKeyboard,
        held: HeldKeys,
        notes: Vec<(u8, f32, bool)>,
        shapes: Vec<egui::Shape>,
    }

    impl Harness {
        fn new() -> Self {
            Self {
                ctx: egui::Context::default(),
                time: 0.0,
                keys: VirtualKeyboard::default(),
                held: HeldKeys::default(),
                notes: Vec::new(),
                shapes: Vec::new(),
            }
        }

        fn frame(&mut self, events: Vec<egui::Event>) {
            self.time += 1.0 / 60.0;
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    Pos2::ZERO,
                    vec2(crate::editor::WINDOW_WIDTH, crate::editor::WINDOW_HEIGHT),
                )),
                time: Some(self.time),
                events,
                ..Default::default()
            };

            let Self {
                ctx,
                keys,
                held,
                notes,
                shapes,
                ..
            } = self;
            let mut output = ctx.run_ui(input, |ui| {
                keys.show(ui, strip(), 1.0, held, |note, velocity, on| {
                    notes.push((note, velocity, on));
                });
            });
            output.textures_delta.clear();
            shapes.clear();
            shapes.extend(output.shapes.into_iter().map(|clipped| clipped.shape));
        }

        /// The note requests of the frames that were run since the last call.
        fn take(&mut self) -> Vec<(u8, f32, bool)> {
            std::mem::take(&mut self.notes)
        }

        /// The rects that are filled with the colour of a lit white key.
        fn lit_white_keys(&self) -> Vec<Rect> {
            self.shapes
                .iter()
                .filter_map(|shape| match shape {
                    egui::Shape::Rect(rect) if rect.fill == palette().primary_bg => Some(rect.rect),
                    _ => None,
                })
                .collect()
        }
    }

    fn moved(pos: Pos2) -> egui::Event {
        egui::Event::PointerMoved(pos)
    }

    fn button(pos: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        }
    }

    fn typed(key: egui::Key, pressed: bool) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        }
    }

    #[test]
    fn a_click_plays_the_key_under_the_pointer_and_releases_it() {
        let mut harness = Harness::new();
        let bed = bed();

        // The fourth white key, so the index is not the first one.
        let rect = bed.white_rect(3);
        let note = bed.white_note(3);
        let pos = rect.center();

        harness.frame(vec![moved(pos)]);
        harness.frame(vec![button(pos, true)]);
        let notes = harness.take();
        assert_eq!(notes.len(), 1, "the press sent {notes:?}");
        assert_eq!(notes[0].0, note);
        assert!(notes[0].2, "the press is not a note on");
        assert!(
            notes[0].1 > 0.3 && notes[0].1 <= 1.0,
            "the velocity {} is out of range",
            notes[0].1
        );

        // The key is lit while it is held.
        assert_eq!(harness.lit_white_keys(), vec![rect.shrink(0.5)]);

        harness.frame(vec![button(pos, false)]);
        assert_eq!(harness.take(), vec![(note, 0.0, false)]);
        assert!(harness.lit_white_keys().is_empty());
    }

    #[test]
    fn a_click_on_a_black_key_plays_the_black_key() {
        let mut harness = Harness::new();
        let bed = bed();
        let rect = bed.black_rect(0).expect("the first key has a black key");
        let pos = rect.center();

        harness.frame(vec![moved(pos)]);
        harness.frame(vec![button(pos, true)]);
        let notes = harness.take();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].0, bed.black_note(0));
        assert!(notes[0].2);

        harness.frame(vec![button(pos, false)]);
        assert_eq!(harness.take().len(), 1);
    }

    #[test]
    fn dragging_glides_from_key_to_key() {
        let mut harness = Harness::new();
        let bed = bed();
        let first = bed.white_rect(3).center();
        let second = bed.white_rect(4).center();

        harness.frame(vec![moved(first)]);
        harness.frame(vec![button(first, true)]);
        assert_eq!(harness.take().len(), 1);

        harness.frame(vec![moved(second)]);
        let notes = harness.take();
        assert_eq!(
            notes,
            vec![
                (bed.white_note(3), 0.0, false),
                (bed.white_note(4), click_velocity(&bed, second), true),
            ],
            "the drag did not move the note"
        );

        harness.frame(vec![button(second, false)]);
        assert_eq!(harness.take(), vec![(bed.white_note(4), 0.0, false)]);
    }

    #[test]
    fn the_typed_keys_follow_the_layout_of_ableton_live() {
        let mut harness = Harness::new();
        let base = ((DEFAULT_OCTAVE + 1) * 12) as u8;

        // The white keys of the middle row.
        for (key, semitone) in TYPED_WHITE {
            harness.frame(vec![typed(key, true)]);
            let notes = harness.take();
            assert_eq!(
                notes,
                vec![(base + semitone, TYPED_VELOCITY, true)],
                "the key {key:?} does not play the semitone {semitone}"
            );
            harness.frame(vec![typed(key, false)]);
            assert_eq!(harness.take(), vec![(base + semitone, 0.0, false)]);
        }

        // The black keys of the row above.
        for (key, semitone) in TYPED_BLACK {
            harness.frame(vec![typed(key, true)]);
            assert_eq!(
                harness.take(),
                vec![(base + semitone, TYPED_VELOCITY, true)],
                "the key {key:?} does not play the semitone {semitone}"
            );
            harness.frame(vec![typed(key, false)]);
            assert_eq!(harness.take(), vec![(base + semitone, 0.0, false)]);
        }
    }

    #[test]
    fn z_and_x_move_the_typed_keys_by_whole_octaves() {
        let mut harness = Harness::new();
        let base = ((DEFAULT_OCTAVE + 1) * 12) as u8;

        harness.frame(vec![typed(egui::Key::A, true)]);
        assert_eq!(harness.take(), vec![(base, TYPED_VELOCITY, true)]);
        harness.frame(vec![typed(egui::Key::A, false)]);
        harness.take();

        // X goes one octave up, and the held key moves with it.
        harness.frame(vec![typed(egui::Key::X, true)]);
        harness.frame(vec![typed(egui::Key::A, true)]);
        assert_eq!(
            harness.take(),
            vec![(base + 12, TYPED_VELOCITY, true)],
            "X did not move the octave up"
        );
        harness.frame(vec![typed(egui::Key::X, false)]);
        harness.frame(vec![typed(egui::Key::A, false)]);
        harness.take();

        // Z goes back down, and further down to the lowest octave.
        harness.frame(vec![typed(egui::Key::Z, true)]);
        harness.frame(vec![typed(egui::Key::A, true)]);
        assert_eq!(
            harness.take(),
            vec![(base, TYPED_VELOCITY, true)],
            "Z did not move the octave down"
        );
        harness.frame(vec![typed(egui::Key::A, false)]);
        harness.frame(vec![typed(egui::Key::Z, false)]);
        harness.take();

        for _ in 0..(DEFAULT_OCTAVE - MIN_OCTAVE + 3) {
            harness.frame(vec![typed(egui::Key::Z, true)]);
            harness.frame(vec![typed(egui::Key::Z, false)]);
        }
        assert_eq!(harness.keys.octave(), MIN_OCTAVE);

        for _ in 0..(MAX_OCTAVE - MIN_OCTAVE + 3) {
            harness.frame(vec![typed(egui::Key::X, true)]);
            harness.frame(vec![typed(egui::Key::X, false)]);
        }
        assert_eq!(harness.keys.octave(), MAX_OCTAVE);

        // The typed notes stay inside the strip at every octave.
        harness.frame(vec![typed(egui::Key::A, true)]);
        let notes = harness.take();
        assert_eq!(
            notes,
            vec![((MAX_OCTAVE + 1) as u8 * 12, TYPED_VELOCITY, true)]
        );
    }

    #[test]
    fn the_octave_buttons_move_the_typed_keys() {
        let mut harness = Harness::new();
        let chrome = Chrome::new(strip(), 1.0);
        let base = ((DEFAULT_OCTAVE + 1) * 12) as u8;

        let up = chrome.up.center();
        harness.frame(vec![moved(up)]);
        harness.frame(vec![button(up, true)]);
        harness.frame(vec![button(up, false)]);
        assert_eq!(harness.keys.octave(), DEFAULT_OCTAVE + 1);

        // The strip shows the new octave, and the readout names its C.
        harness.frame(vec![]);
        let names: Vec<String> = harness
            .shapes
            .iter()
            .filter_map(|shape| match shape {
                egui::Shape::Text(text) => Some(text.galley.text().to_string()),
                _ => None,
            })
            .collect();
        assert!(
            names.contains(&note_name(base + 12)),
            "the readout does not show the new octave: {names:?}"
        );

        let down = chrome.down.center();
        harness.frame(vec![moved(down)]);
        harness.frame(vec![button(down, true)]);
        harness.frame(vec![button(down, false)]);
        assert_eq!(harness.keys.octave(), DEFAULT_OCTAVE);
    }

    #[test]
    fn the_c_of_the_typed_keys_is_red_without_a_bar_over_the_key() {
        let mut harness = Harness::new();
        harness.frame(vec![]);
        let base = harness.keys.base_note();
        let bed = bed();

        // The name of the C that the `A` key plays is the only red text of the
        // keys.
        let mut red = Vec::new();
        for shape in &harness.shapes {
            if let egui::Shape::Text(text) = shape {
                let color = text
                    .galley
                    .job
                    .sections
                    .first()
                    .map(|section| section.format.color);
                if color == Some(palette().accent) && bed.rect.contains(text.pos) {
                    red.push(text.galley.text().to_string());
                }
            }
        }
        assert_eq!(red, vec![note_name(base)]);

        // Nothing covers the top of the key, so the mark of the octave is the
        // red lettering alone.
        let index = (base as i32 / 12 - bed.first_octave) * 7;
        let key = bed.white_rect(index as usize);
        let top = Rect::from_min_size(
            pos2(key.left() + 1.0, key.top() + 1.0),
            vec2(key.width() - 2.0, 3.0),
        );
        assert!(
            harness.shapes.iter().all(|shape| {
                !matches!(shape, egui::Shape::Rect(rect)
                    if rect.fill == palette().accent && rect.rect.intersects(top))
            }),
            "a bar still covers the top of the key"
        );
    }

    #[test]
    fn the_lights_show_the_keys_of_a_midi_keyboard() {
        let mut harness = Harness::new();
        let bed = bed();

        // A note that the panel did not start, as a MIDI keyboard would.
        let note = bed.white_note(5);
        harness.held.press(note);
        harness.frame(vec![]);

        let rect = bed.white_rect(5).shrink(0.5);
        assert!(
            harness.lit_white_keys().contains(&rect),
            "the key of the MIDI note is not lit: {:?}",
            harness.lit_white_keys()
        );

        harness.held.release(note);
        harness.frame(vec![]);
        assert!(harness.lit_white_keys().is_empty());
    }

    #[test]
    fn a_closing_editor_releases_the_keys_it_started() {
        let mut harness = Harness::new();
        let pos = bed().white_rect(2).center();
        harness.frame(vec![moved(pos)]);
        harness.frame(vec![button(pos, true)]);
        harness.take();

        let mut released = Vec::new();
        harness
            .keys
            .release_all(|note, velocity, on| released.push((note, velocity, on)));
        assert_eq!(released, vec![(bed().white_note(2), 0.0, false)]);

        // A second release sends nothing, so the closing editor cannot send a
        // note off twice.
        harness
            .keys
            .release_all(|note, velocity, on| released.push((note, velocity, on)));
        assert_eq!(released.len(), 1);
    }

    #[test]
    fn the_strip_shows_five_octaves_and_a_closing_c() {
        let bed = bed();
        assert_eq!(KeyBed::white_count(), OCTAVES * 7 + 1);
        assert_eq!(bed.white_note(0) % 12, 0);
        assert_eq!(bed.white_note(KeyBed::white_count() - 1) % 12, 0);
        assert_eq!(
            bed.white_note(KeyBed::white_count() - 1),
            bed.white_note(0) + OCTAVES as u8 * 12
        );

        // The white keys fill the strip without gaps.
        let first = bed.white_rect(0);
        let last = bed.white_rect(KeyBed::white_count() - 1);
        assert!((first.left() - bed.rect.left()).abs() < 0.01);
        assert!((last.right() - bed.rect.right()).abs() < 0.01);
        assert!(
            (last.right() - first.left() - bed.white_width * (KeyBed::white_count() as f32)).abs()
                < 0.01
        );
    }

    #[test]
    fn the_black_keys_sit_between_the_right_white_keys() {
        let bed = bed();
        for index in 0..KeyBed::white_count() - 1 {
            let has_black = has_black_above(index) && index / 7 < OCTAVES;
            assert_eq!(
                bed.black_rect(index).is_some(),
                has_black,
                "the white key {index} has the wrong black key above it"
            );
            if let Some(rect) = bed.black_rect(index) {
                // The black key covers the boundary between the two white
                // keys and is shorter than them.
                let left = bed.white_rect(index);
                let right = bed.white_rect(index + 1);
                assert!(rect.left() > left.center().x);
                assert!(rect.right() < right.center().x);
                assert!((rect.center().x - right.left()).abs() < 0.01);
                assert!(rect.height() < bed.rect.height());
            }
        }
    }

    #[test]
    fn the_names_of_the_notes_use_middle_c_as_c4() {
        assert_eq!(note_name(0), "C-1");
        assert_eq!(note_name(60), "C4");
        assert_eq!(note_name(61), "C#4");
        assert_eq!(note_name(72), "C5");
        assert_eq!(note_name(127), "G9");
    }
}
