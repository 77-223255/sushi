//! The Sushi editor: a retro Win95/Pascal-flavoured panel.
//!
//! Every continuous parameter is a vertical slider whose handle is a band that
//! is fully embedded inside the groove (it never sticks out), every parameter
//! can also be typed into a small numeric field, and every choice parameter
//! opens a popup menu. The parameters are grouped into labelled boxes below a
//! simple oscilloscope.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use egui::{Align, Color32, CornerRadius, Layout, Margin, Rect, Sense, Stroke};
use nice_plug::context::gui::GuiContext;
use nice_plug::prelude::*;
use nice_plug_egui::NiceEguiApp;

use crate::params::SushiParams;

// --- Win95 / Reaper grey palette -------------------------------------------

const BG: Color32 = Color32::from_rgb(0xC0, 0xC0, 0xC0);
const BUTTON: Color32 = Color32::from_rgb(0xD2, 0xD2, 0xD2);
const FIELD: Color32 = Color32::from_rgb(0xFB, 0xFB, 0xFB);
const LIGHT: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);
const SHADOW: Color32 = Color32::from_rgb(0x80, 0x80, 0x80);
const DARK: Color32 = Color32::from_rgb(0x40, 0x40, 0x40);
const TEXT: Color32 = Color32::from_rgb(0x10, 0x10, 0x10);
const LABEL: Color32 = Color32::from_rgb(0x28, 0x28, 0x28);
const TRACE: Color32 = Color32::from_rgb(0x33, 0x33, 0x33);
const SELECTION: Color32 = Color32::from_rgb(0x0A, 0x24, 0x6A);

// --- Layout -----------------------------------------------------------------

pub const WINDOW_WIDTH: f32 = 1008.0;
pub const WINDOW_HEIGHT: f32 = 600.0;

/// The margin around the whole panel.
const OUTER_MARGIN: f32 = 8.0;
/// The height of the oscilloscope strip.
const SCOPE_HEIGHT: f32 = 100.0;
/// The height of a group box.
const GROUP_HEIGHT: f32 = 150.0;
/// The horizontal gap between two group boxes.
const GROUP_GAP: f32 = 8.0;
/// The padding inside a group box.
const GROUP_PADDING: f32 = 6.0;
/// The height of a group box header (title and popups).
const HEADER_HEIGHT: f32 = 20.0;
/// The horizontal gap between two parameter columns.
const COLUMN_GAP: f32 = 4.0;

/// The width of the four group boxes in every row.
const GROUP_WIDTH: f32 = 242.0;

// --- Widget metrics ---------------------------------------------------------

/// The width of a vertical slider.
const SLIDER_WIDTH: f32 = 16.0;
/// The height of a vertical slider.
const SLIDER_HEIGHT: f32 = 78.0;
/// The height of the slider handle.
const HANDLE_HEIGHT: f32 = 10.0;
/// How far the handle is kept away from the edges of the groove.
const GROOVE_INSET: f32 = 3.0;
/// The size of the numeric value fields.
const FIELD_WIDTH: f32 = 40.0;
const FIELD_HEIGHT: f32 = 15.0;

/// How many scope samples are kept around.
const HISTORY_LEN: usize = 4096;

pub struct SushiEditor {
    params: Arc<SushiParams>,
    scope_rx: rtrb::Consumer<f32>,
    history: VecDeque<f32>,

    gui_ctx: Option<GuiContext>,

    /// The text buffers of the numeric value fields, keyed by parameter id.
    texts: HashMap<&'static str, String>,

    /// The parameters that currently have an open automation gesture.
    gestures: HashSet<&'static str>,
}

impl SushiEditor {
    pub fn new(params: Arc<SushiParams>, scope_rx: rtrb::Consumer<f32>) -> Self {
        Self {
            params,
            scope_rx,
            history: VecDeque::with_capacity(HISTORY_LEN),
            gui_ctx: None,
            texts: HashMap::new(),
            gestures: HashSet::new(),
        }
    }

    fn collect_scope(&mut self) {
        while let Ok(sample) = self.scope_rx.pop() {
            if self.history.len() == HISTORY_LEN {
                self.history.pop_front();
            }
            self.history.push_back(sample);
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
        ui.ctx().request_repaint_after(Duration::from_millis(16));

        // Split the borrow of the editor so that the parameters and the text
        // buffers can be used at the same time.
        let SushiEditor {
            params,
            history,
            texts,
            gestures,
            ..
        } = self;
        let params = params.as_ref();

        let full = ui.max_rect();
        ui.painter().rect_filled(full, CornerRadius::ZERO, BG);

        egui::Frame::new()
            .inner_margin(Margin::symmetric(OUTER_MARGIN as i8, OUTER_MARGIN as i8))
            .show(ui, |ui| {
                draw_scope(ui, history);

                ui.add_space(GROUP_GAP);
                let mut ctx = Ctx {
                    params,
                    setter: &setter,
                    texts,
                    gestures,
                };
                row_a(ui, &mut ctx);

                ui.add_space(GROUP_GAP);
                row_b(ui, &mut ctx);

                ui.add_space(GROUP_GAP);
                row_c(ui, &mut ctx);
            });
    }
}

/// Everything a parameter widget needs to draw itself.
struct Ctx<'a, 'b> {
    params: &'a SushiParams,
    setter: &'b ParamSetter<'b>,
    texts: &'a mut HashMap<&'static str, String>,
    /// The parameters that currently have an open automation gesture.
    gestures: &'a mut HashSet<&'static str>,
}

impl<'a, 'b> Ctx<'a, 'b> {
    /// Draws a continuous parameter: a tiny label, a vertical slider and a
    /// numeric field that mirrors (or replaces) the parameter's value.
    #[allow(clippy::too_many_arguments)]
    fn float(
        &mut self,
        ui: &mut egui::Ui,
        id: &'static str,
        label: &str,
        param: &FloatParam,
        min: f32,
        max: f32,
        log: bool,
        decimals: usize,
        to_display: fn(f32) -> f32,
        from_display: fn(f32) -> f32,
    ) {
        ui.label(egui::RichText::new(label).size(10.0).color(LABEL));

        let mut value = param.value();
        let response = v_slider(ui, &mut value, min, max, log, SLIDER_HEIGHT);
        if self.begin_gesture(id, &response, ui) {
            self.setter.begin_set_parameter(param);
        }
        if response.changed() {
            self.setter.set_parameter(param, value);
        }
        if self.end_gesture(id, &response, ui) {
            self.setter.end_set_parameter(param);
        }

        let text = self.texts.entry(id).or_default();
        let field_id = field_id(id);

        // While the field has the focus it is never overwritten. Otherwise it
        // mirrors the parameter, but only when the parameter actually changed,
        // so that a value that was just typed isn't reverted before the host
        // has applied it.
        if !ui.memory(|memory| memory.has_focus(field_id)) {
            sync_text(ui, id, text, decimals, to_display(param.value()));
        }

        let field = number_field(ui, text, id, field_id);

        if field.lost_focus() {
            // The value was typed in: parse it and hand it to the host.
            match text.trim().parse::<f32>() {
                Ok(parsed) if parsed.is_finite() => {
                    let plain = from_display(parsed);
                    self.setter.begin_set_parameter(param);
                    self.setter.set_parameter(param, plain);
                    self.setter.end_set_parameter(param);
                    *text = format_number(to_display(plain), decimals);
                }
                _ => {
                    // Nonsense was typed: fall back to the parameter's value.
                    *text = format_number(to_display(param.value()), decimals);
                }
            }
        }
    }

    /// Draws a stepped parameter. It works just like [`Ctx::float`], but the
    /// value is rounded to whole numbers.
    fn int(
        &mut self,
        ui: &mut egui::Ui,
        id: &'static str,
        label: &str,
        param: &IntParam,
        min: i32,
        max: i32,
    ) {
        ui.label(egui::RichText::new(label).size(10.0).color(LABEL));

        let mut value = param.value() as f32;
        let response = v_slider(ui, &mut value, min as f32, max as f32, false, SLIDER_HEIGHT);
        if self.begin_gesture(id, &response, ui) {
            self.setter.begin_set_parameter(param);
        }
        if response.changed() {
            self.setter.set_parameter(param, value.round() as i32);
        }
        if self.end_gesture(id, &response, ui) {
            self.setter.end_set_parameter(param);
        }

        let text = self.texts.entry(id).or_default();
        let field_id = field_id(id);

        if !ui.memory(|memory| memory.has_focus(field_id)) {
            sync_text(ui, id, text, 0, param.value() as f32);
        }

        let field = number_field(ui, text, id, field_id);

        if field.lost_focus() {
            match text.trim().parse::<f32>() {
                Ok(parsed) if parsed.is_finite() => {
                    let plain = parsed.round() as i32;
                    self.setter.begin_set_parameter(param);
                    self.setter.set_parameter(param, plain);
                    self.setter.end_set_parameter(param);
                    *text = format_number(plain as f32, 0);
                }
                _ => {
                    *text = format_number(param.value() as f32, 0);
                }
            }
        }
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
        if self.gestures.contains(id) {
            return false;
        }

        let pressed = ui.input(|input| input.pointer.any_pressed());
        let started = response.drag_started() || (pressed && response.is_pointer_button_down_on());
        if !started {
            return false;
        }

        self.gestures.insert(id);
        true
    }

    /// Closes the automation gesture of a parameter. The gesture ends when the
    /// user releases the widget again.
    fn end_gesture(&mut self, id: &'static str, response: &egui::Response, ui: &egui::Ui) -> bool {
        if !self.gestures.contains(id) {
            return false;
        }

        let released = response.drag_stopped()
            || response.clicked()
            || ui.input(|input| input.pointer.any_released());
        if !released {
            return false;
        }

        self.gestures.remove(id);
        true
    }

    /// Draws a Win95-style popup button that opens the list of choices. The
    /// `prefix` is put in front of the selected value and the choices.
    fn popup<T: Enum + PartialEq + Copy + 'static>(
        &mut self,
        ui: &mut egui::Ui,
        param: &EnumParam<T>,
        width: f32,
        prefix: &str,
    ) {
        let current = param.value();
        let variants = T::variants();
        let selected = format!("{prefix}{}", variants[T::to_index(current)]);
        egui::ComboBox::from_id_salt(param.name())
            .width(width)
            .selected_text(selected)
            .truncate()
            .show_ui(ui, |ui| {
                for (index, name) in variants.iter().enumerate() {
                    let variant = T::from_index(index);
                    let label = format!("{prefix}{name}");
                    if ui.selectable_label(current == variant, label).clicked() {
                        self.setter.set_parameter(param, variant);
                    }
                }
            });
    }
}

// --- The three rows of group boxes -----------------------------------------

fn row_a(ui: &mut egui::Ui, ctx: &mut Ctx) {
    group_row(
        ui,
        "row-a",
        &[GROUP_WIDTH; 4],
        GROUP_HEIGHT,
        GROUP_GAP,
        |index, rect, ui| {
            let p = ctx.params;
            match index {
                0 => group(ui, rect, "OSC 1", |ui, part| match part {
                    Part::Header => ctx.popup(ui, &p.osc1_wave, 78.0, ""),
                    Part::Body => column_row(ui, "osc1", 5, COLUMN_GAP, |index, ui| {
                        let p = ctx.params;
                        match index {
                            0 => ctx.int(ui, "osc1_octave", "Oct", &p.osc1_octave, -2, 2),
                            1 => ctx.int(ui, "osc1_semi", "Semi", &p.osc1_semi, -12, 12),
                            2 => ctx.float(
                                ui,
                                "osc1_fine",
                                "Fine",
                                &p.osc1_fine,
                                -100.0,
                                100.0,
                                false,
                                0,
                                identity,
                                identity,
                            ),
                            3 => ctx.float(
                                ui,
                                "osc1_pw",
                                "PW",
                                &p.osc1_pw,
                                0.05,
                                0.95,
                                false,
                                0,
                                to_percent,
                                from_percent,
                            ),
                            4 => ctx.float(
                                ui,
                                "osc1_level",
                                "Lvl",
                                &p.osc1_level,
                                0.0,
                                1.0,
                                false,
                                0,
                                to_percent,
                                from_percent,
                            ),
                            _ => {}
                        }
                    }),
                }),
                1 => group(ui, rect, "OSC 2", |ui, part| match part {
                    Part::Header => ctx.popup(ui, &p.osc2_wave, 78.0, ""),
                    Part::Body => column_row(ui, "osc2", 5, COLUMN_GAP, |index, ui| {
                        let p = ctx.params;
                        match index {
                            0 => ctx.int(ui, "osc2_octave", "Oct", &p.osc2_octave, -2, 2),
                            1 => ctx.int(ui, "osc2_semi", "Semi", &p.osc2_semi, -12, 12),
                            2 => ctx.float(
                                ui,
                                "osc2_fine",
                                "Fine",
                                &p.osc2_fine,
                                -100.0,
                                100.0,
                                false,
                                0,
                                identity,
                                identity,
                            ),
                            3 => ctx.float(
                                ui,
                                "osc2_pw",
                                "PW",
                                &p.osc2_pw,
                                0.05,
                                0.95,
                                false,
                                0,
                                to_percent,
                                from_percent,
                            ),
                            4 => ctx.float(
                                ui,
                                "osc2_level",
                                "Lvl",
                                &p.osc2_level,
                                0.0,
                                1.0,
                                false,
                                0,
                                to_percent,
                                from_percent,
                            ),
                            _ => {}
                        }
                    }),
                }),
                2 => group(ui, rect, "MIX", |ui, part| match part {
                    Part::Header => {}
                    Part::Body => column_row(ui, "mix", 4, COLUMN_GAP, |index, ui| {
                        let p = ctx.params;
                        match index {
                            0 => ctx.float(
                                ui,
                                "noise_level",
                                "Noise",
                                &p.noise_level,
                                0.0,
                                1.0,
                                false,
                                0,
                                to_percent,
                                from_percent,
                            ),
                            1 => ctx.int(ui, "unison", "Uni", &p.unison, 1, 4),
                            2 => ctx.float(
                                ui, "detune", "Detune", &p.detune, 0.0, 50.0, false, 0, identity,
                                identity,
                            ),
                            3 => ctx.float(
                                ui,
                                "spread",
                                "Spread",
                                &p.spread,
                                0.0,
                                1.0,
                                false,
                                0,
                                to_percent,
                                from_percent,
                            ),
                            _ => {}
                        }
                    }),
                }),
                _ => group(ui, rect, "LFO 1", |ui, part| match part {
                    Part::Header => {
                        ctx.popup(ui, &p.lfo1_wave, 82.0, "");
                        ctx.popup(ui, &p.lfo1_target, 90.0, "> ");
                    }
                    Part::Body => column_row(ui, "lfo1", 2, COLUMN_GAP, |index, ui| {
                        let p = ctx.params;
                        match index {
                            0 => ctx.float(
                                ui,
                                "lfo1_rate",
                                "Rate",
                                &p.lfo1_rate,
                                0.01,
                                40.0,
                                true,
                                2,
                                identity,
                                identity,
                            ),
                            1 => ctx.float(
                                ui,
                                "lfo1_amount",
                                "Amount",
                                &p.lfo1_amount,
                                0.0,
                                1.0,
                                false,
                                0,
                                to_percent,
                                from_percent,
                            ),
                            _ => {}
                        }
                    }),
                }),
            }
        },
    );
}

fn row_b(ui: &mut egui::Ui, ctx: &mut Ctx) {
    group_row(
        ui,
        "row-b",
        &[GROUP_WIDTH; 4],
        GROUP_HEIGHT,
        GROUP_GAP,
        |index, rect, ui| {
            let p = ctx.params;
            match index {
                0 => group(ui, rect, "FILTER", |ui, part| match part {
                    Part::Header => ctx.popup(ui, &p.filter_mode, 78.0, ""),
                    Part::Body => column_row(ui, "filter", 5, COLUMN_GAP, |index, ui| {
                        let p = ctx.params;
                        match index {
                            0 => ctx.float(
                                ui, "cutoff", "Cut", &p.cutoff, 20.0, 20_000.0, true, 0, identity,
                                identity,
                            ),
                            1 => ctx.float(
                                ui,
                                "resonance",
                                "Res",
                                &p.resonance,
                                0.0,
                                1.0,
                                false,
                                0,
                                to_percent,
                                from_percent,
                            ),
                            2 => ctx.float(
                                ui,
                                "filter_env_amount",
                                "Env",
                                &p.filter_env_amount,
                                -1.0,
                                1.0,
                                false,
                                0,
                                to_percent,
                                from_percent,
                            ),
                            3 => ctx.float(
                                ui,
                                "filter_keytrack",
                                "Key",
                                &p.filter_keytrack,
                                0.0,
                                1.0,
                                false,
                                0,
                                to_percent,
                                from_percent,
                            ),
                            4 => ctx.float(
                                ui,
                                "filter_drive",
                                "Drv",
                                &p.filter_drive,
                                0.0,
                                1.0,
                                false,
                                0,
                                to_percent,
                                from_percent,
                            ),
                            _ => {}
                        }
                    }),
                }),
                1 => group(ui, rect, "FILTER ENV", |ui, part| match part {
                    Part::Header => {}
                    Part::Body => column_row(ui, "fe", 4, COLUMN_GAP, |index, ui| {
                        let p = ctx.params;
                        match index {
                            0 => ctx.float(
                                ui,
                                "fe_attack",
                                "A",
                                &p.fe_attack,
                                0.0,
                                4_000.0,
                                true,
                                1,
                                identity,
                                identity,
                            ),
                            1 => ctx.float(
                                ui,
                                "fe_decay",
                                "D",
                                &p.fe_decay,
                                0.0,
                                4_000.0,
                                true,
                                1,
                                identity,
                                identity,
                            ),
                            2 => ctx.float(
                                ui,
                                "fe_sustain",
                                "S",
                                &p.fe_sustain,
                                0.0,
                                1.0,
                                false,
                                0,
                                to_percent,
                                from_percent,
                            ),
                            3 => ctx.float(
                                ui,
                                "fe_release",
                                "R",
                                &p.fe_release,
                                0.0,
                                4_000.0,
                                true,
                                1,
                                identity,
                                identity,
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
                                0.0,
                                4_000.0,
                                true,
                                1,
                                identity,
                                identity,
                            ),
                            1 => ctx.float(
                                ui,
                                "ae_decay",
                                "D",
                                &p.ae_decay,
                                0.0,
                                4_000.0,
                                true,
                                1,
                                identity,
                                identity,
                            ),
                            2 => ctx.float(
                                ui,
                                "ae_sustain",
                                "S",
                                &p.ae_sustain,
                                0.0,
                                1.0,
                                false,
                                0,
                                to_percent,
                                from_percent,
                            ),
                            3 => ctx.float(
                                ui,
                                "ae_release",
                                "R",
                                &p.ae_release,
                                0.0,
                                8_000.0,
                                true,
                                1,
                                identity,
                                identity,
                            ),
                            _ => {}
                        }
                    }),
                }),
                _ => group(ui, rect, "LFO 2", |ui, part| match part {
                    Part::Header => {
                        ctx.popup(ui, &p.lfo2_wave, 82.0, "");
                        ctx.popup(ui, &p.lfo2_target, 90.0, "> ");
                    }
                    Part::Body => column_row(ui, "lfo2", 2, COLUMN_GAP, |index, ui| {
                        let p = ctx.params;
                        match index {
                            0 => ctx.float(
                                ui,
                                "lfo2_rate",
                                "Rate",
                                &p.lfo2_rate,
                                0.01,
                                40.0,
                                true,
                                2,
                                identity,
                                identity,
                            ),
                            1 => ctx.float(
                                ui,
                                "lfo2_amount",
                                "Amount",
                                &p.lfo2_amount,
                                0.0,
                                1.0,
                                false,
                                0,
                                to_percent,
                                from_percent,
                            ),
                            _ => {}
                        }
                    }),
                }),
            }
        },
    );
}

fn row_c(ui: &mut egui::Ui, ctx: &mut Ctx) {
    group_row(
        ui,
        "row-c",
        &[GROUP_WIDTH; 4],
        GROUP_HEIGHT,
        GROUP_GAP,
        |index, rect, ui| {
            let p = ctx.params;
            match index {
                0 => group(ui, rect, "OUTPUT", |ui, part| match part {
                    Part::Header => ctx.popup(ui, &p.poly_mode, 70.0, ""),
                    Part::Body => column_row(ui, "output", 2, COLUMN_GAP, |index, ui| {
                        let p = ctx.params;
                        match index {
                            0 => ctx.float(
                                ui, "glide", "Glide", &p.glide, 0.0, 2_000.0, true, 1, identity,
                                identity,
                            ),
                            1 => ctx.float(
                                ui,
                                "volume",
                                "Volume",
                                &p.volume,
                                util::db_to_gain(-60.0),
                                util::db_to_gain(6.0),
                                true,
                                1,
                                util::gain_to_db,
                                util::db_to_gain,
                            ),
                            _ => {}
                        }
                    }),
                }),
                1 => group(ui, rect, "CHORUS", |ui, part| match part {
                    Part::Header => {}
                    Part::Body => column_row(ui, "chorus", 3, COLUMN_GAP, |index, ui| {
                        let p = ctx.params;
                        match index {
                            0 => ctx.float(
                                ui,
                                "chorus_mix",
                                "Mix",
                                &p.chorus_mix,
                                0.0,
                                1.0,
                                false,
                                0,
                                to_percent,
                                from_percent,
                            ),
                            1 => ctx.float(
                                ui,
                                "chorus_rate",
                                "Rate",
                                &p.chorus_rate,
                                0.05,
                                8.0,
                                true,
                                2,
                                identity,
                                identity,
                            ),
                            2 => ctx.float(
                                ui,
                                "chorus_depth",
                                "Depth",
                                &p.chorus_depth,
                                0.0,
                                1.0,
                                false,
                                0,
                                to_percent,
                                from_percent,
                            ),
                            _ => {}
                        }
                    }),
                }),
                2 => group(ui, rect, "DELAY", |ui, part| match part {
                    Part::Header => {}
                    Part::Body => column_row(ui, "delay", 3, COLUMN_GAP, |index, ui| {
                        let p = ctx.params;
                        match index {
                            0 => ctx.float(
                                ui,
                                "delay_time",
                                "Time",
                                &p.delay_time,
                                10.0,
                                1_500.0,
                                true,
                                1,
                                identity,
                                identity,
                            ),
                            1 => ctx.float(
                                ui,
                                "delay_feedback",
                                "FB",
                                &p.delay_feedback,
                                0.0,
                                0.95,
                                false,
                                0,
                                to_percent,
                                from_percent,
                            ),
                            2 => ctx.float(
                                ui,
                                "delay_mix",
                                "Mix",
                                &p.delay_mix,
                                0.0,
                                1.0,
                                false,
                                0,
                                to_percent,
                                from_percent,
                            ),
                            _ => {}
                        }
                    }),
                }),
                _ => group(ui, rect, "REVERB", |ui, part| match part {
                    Part::Header => {}
                    Part::Body => column_row(ui, "reverb", 2, COLUMN_GAP, |index, ui| {
                        let p = ctx.params;
                        match index {
                            0 => ctx.float(
                                ui,
                                "reverb_decay",
                                "Decay",
                                &p.reverb_decay,
                                0.0,
                                1.0,
                                false,
                                0,
                                to_percent,
                                from_percent,
                            ),
                            1 => ctx.float(
                                ui,
                                "reverb_mix",
                                "Mix",
                                &p.reverb_mix,
                                0.0,
                                1.0,
                                false,
                                0,
                                to_percent,
                                from_percent,
                            ),
                            _ => {}
                        }
                    }),
                }),
            }
        },
    );
}

// --- The oscilloscope -------------------------------------------------------

/// Draws the oscilloscope: a plain white field with a single trace. It is
/// just here to give the panel a bit of life, not to be a measurement tool.
fn draw_scope(ui: &mut egui::Ui, history: &VecDeque<f32>) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), SCOPE_HEIGHT),
        egui::Sense::hover(),
    );
    let painter = ui.painter_at(rect);

    sunken(&painter, rect, LIGHT);
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
        .map(|i| {
            let x = inner.left() + i as f32 * step;
            let sample = history[start + i].clamp(-1.0, 1.0);
            egui::pos2(x, inner.center().y - sample * scale)
        })
        .collect();

    painter.add(egui::Shape::line(points, Stroke::new(1.0, TRACE)));
}

// --- The widgets -------------------------------------------------------------

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
    widths: &[f32],
    height: f32,
    gap: f32,
    mut add: impl FnMut(usize, Rect, &mut egui::Ui),
) {
    let start = ui.next_widget_position();
    let mut x = start.x;
    for (index, width) in widths.iter().enumerate() {
        let rect = Rect::from_min_size(egui::pos2(x, start.y), egui::vec2(*width, height));
        x += width + gap;
        ui.scope_builder(
            egui::UiBuilder::new()
                .id_salt(("sushi-group", name, index))
                .max_rect(rect)
                .layout(Layout::top_down(Align::Min)),
            |ui| add(index, rect, ui),
        );
    }
    let total = Rect::from_min_size(start, egui::vec2(x - gap - start.x, height));
    ui.advance_cursor_after_rect(total);
}

/// Draws a group box and hands its header and body to the given callback.
fn group(ui: &mut egui::Ui, rect: Rect, title: &str, mut add: impl FnMut(&mut egui::Ui, Part)) {
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::ZERO, BG);
    raised(&painter, rect, BG);

    let inner = rect.shrink(GROUP_PADDING);
    let header = Rect::from_min_size(inner.min, egui::vec2(inner.width(), HEADER_HEIGHT));
    let body = Rect::from_min_size(
        inner.min + egui::vec2(0.0, HEADER_HEIGHT),
        egui::vec2(inner.width(), (inner.height() - HEADER_HEIGHT).max(0.0)),
    );

    ui.scope_builder(
        egui::UiBuilder::new()
            .id_salt(("sushi-header", title))
            .max_rect(header)
            .layout(Layout::left_to_right(Align::Center)),
        |ui| {
            ui.label(egui::RichText::new(title).strong().size(11.0).color(TEXT));
            add(ui, Part::Header);
        },
    );

    ui.scope_builder(
        egui::UiBuilder::new()
            .id_salt(("sushi-body", title))
            .max_rect(body)
            .layout(Layout::top_down(Align::Min)),
        |ui| {
            add(ui, Part::Body);
        },
    );
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
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 3.0);
                add(index, ui);
            },
        );
    }

    ui.advance_cursor_after_rect(Rect::from_min_size(start, egui::vec2(total_width, height)));
}

/// A vertical slider. The handle is a band that is fully embedded inside the
/// groove: it never sticks out of it, no matter where the value is.
fn v_slider(
    ui: &mut egui::Ui,
    value: &mut f32,
    min: f32,
    max: f32,
    log: bool,
    height: f32,
) -> egui::Response {
    let (rect, mut response) =
        ui.allocate_exact_size(egui::vec2(SLIDER_WIDTH, height), Sense::click_and_drag());
    let painter = ui.painter_at(rect);

    // The groove.
    sunken(&painter, rect, FIELD);
    let track = rect.shrink(GROOVE_INSET);

    // The part of the groove below the handle is filled in.
    let position = value_to_position(*value, min, max, log, track);
    let handle_center = position.clamp(
        track.top() + HANDLE_HEIGHT * 0.5,
        track.bottom() - HANDLE_HEIGHT * 0.5,
    );
    let filled = Rect::from_min_max(
        egui::pos2(track.left(), handle_center + HANDLE_HEIGHT * 0.5),
        egui::pos2(track.right(), track.bottom()),
    );
    if filled.height() > 0.0 {
        painter.rect_filled(filled, CornerRadius::ZERO, BUTTON);
    }

    // The handle itself: a full width band inside the groove.
    let handle = Rect::from_center_size(
        egui::pos2(rect.center().x, handle_center),
        egui::vec2(SLIDER_WIDTH - 2.0 * GROOVE_INSET, HANDLE_HEIGHT),
    );
    raised(&painter, handle, BUTTON);

    // Dragging anywhere on the groove sets the value.
    if (response.is_pointer_button_down_on() || response.dragged())
        && let Some(pointer) = response.interact_pointer_pos()
    {
        let position = ((track.bottom() - pointer.y) / track.height()).clamp(0.0, 1.0);
        *value = position_to_value(position, min, max, log);
        response.mark_changed();
    }

    response
}

/// The number of the numeric value field.
fn number_field(
    ui: &mut egui::Ui,
    text: &mut String,
    id: &'static str,
    field_id: egui::Id,
) -> egui::Response {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(FIELD_WIDTH, FIELD_HEIGHT), Sense::hover());
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
                    .id(field_id)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(FIELD_WIDTH)
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
            egui::FontId::new(11.0, egui::FontFamily::Monospace),
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
mod preview {
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
        let mut editor = SushiEditor::new(params.clone(), rx);
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

    /// The rects of all the sliders, in the order they are drawn.
    fn slider_rects(ctx: &egui::Context, output: &egui::FullOutput) -> Vec<egui::Rect> {
        let mut rects = Vec::new();
        for clipped in ctx.tessellate(output.shapes.clone(), ctx.pixels_per_point()) {
            if let egui::epaint::Primitive::Mesh(_mesh) = clipped.primitive {
                let rect = clipped.clip_rect;
                if (rect.width() - SLIDER_WIDTH).abs() < 0.5
                    && (rect.height() - SLIDER_HEIGHT).abs() < 0.5
                {
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

    #[test]
    fn clicking_a_slider_sets_the_value() {
        let (ctx, mut editor, params) = make_editor();
        let output = run_frames(&ctx, &mut editor, vec![input(vec![])]);
        let sliders = slider_rects(&ctx, &output);
        assert_eq!(sliders.len(), 49 - 8, "unexpected number of sliders");
        // The cutoff slider is the first one of the second row.
        let rect = sliders[16];
        let pos = rect.center();

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
                input(vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::default(),
                }]),
            ],
        );

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
        let rect = sliders[16];
        let pos = rect.center();

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
                input(vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::default(),
                }]),
            ],
        );

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
        let rect = sliders[16];
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
        let rect = sliders[16];
        // The value field is right below the slider.
        let field = egui::pos2(rect.center().x, rect.bottom() + 3.0 + 7.5);

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
        let rect = sliders[16];
        // The value field is right below the slider.
        let field = egui::pos2(rect.center().x, rect.bottom() + 3.0 + 7.5);

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
        run_frames(
            &ctx,
            &mut editor,
            vec![
                input(vec![egui::Event::PointerMoved(item)]),
                input(vec![egui::Event::PointerButton {
                    pos: item,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::default(),
                }]),
                input(vec![egui::Event::PointerButton {
                    pos: item,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::default(),
                }]),
            ],
        );

        assert_eq!(params.osc1_wave.value(), crate::params::Waveform::Triangle);
    }
}
