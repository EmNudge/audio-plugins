//! The editor: a preset bar, three live views of what the shaper does to a test sine, then the
//! controls.
//!
//! The transfer curve shows the shaper's input against its output, the waveform shows the sine
//! before (dim) and after (bright), and the bars show the first twelve harmonics, odd ones in
//! red and even ones in gold. All three are computed from the parameters with the same shaper
//! the audio uses, so they track every knob without any signal from the audio thread.

use nih_plug::prelude::*;
use nih_plug_egui::egui::{
    self, Align2, Color32, FontId, Painter, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2,
};
use std::f32::consts::TAU;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::dsp::{shape, Bands, Model, ModSource};
use crate::presets::{self, Preset};
use crate::{Live, SmolderParams, BAND_COUNT};

const BG: Color32 = Color32::from_rgb(0x0d, 0x09, 0x07);
const PANEL: Color32 = Color32::from_rgb(0x1a, 0x12, 0x10);
const GRID: Color32 = Color32::from_rgb(0x2e, 0x21, 0x1c);
const TEXT_DIM: Color32 = Color32::from_rgb(0x8a, 0x6f, 0x62);
const EMBER: Color32 = Color32::from_rgb(0xff, 0x7a, 0x1a);
const GOLD: Color32 = Color32::from_rgb(0xff, 0xd3, 0x6b);
const RED: Color32 = Color32::from_rgb(0xff, 0x3d, 0x2e);

/// Level of the sine the views are drawn with, before drive.
const TEST_LEVEL: f32 = 0.5;
const HARMONICS: usize = 12;
const DFT_POINTS: usize = 256;
/// Harmonic bars show this many dB below the fundamental.
const HARMONIC_RANGE_DB: f32 = 72.0;

pub fn background() -> Color32 {
    BG
}

pub struct UiState {
    /// The band the three views show in 3-band mode.
    focus: usize,
    factory: Vec<Preset>,
    user: Vec<Preset>,
    save_open: bool,
    save_name: String,
    message: String,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            focus: 1,
            factory: presets::factory(),
            user: presets::load_user(),
            save_open: false,
            save_name: String::new(),
            message: String::new(),
        }
    }
}

pub fn draw(ui: &mut Ui, params: &Arc<SmolderParams>, setter: &ParamSetter, live: &Live, state: &mut UiState) {
    // The modulation moves things while it runs, so the views follow it.
    ui.ctx().request_repaint();
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        preset_bar(ui, params, setter, state);
        ui.add_space(4.0);

        let multi = params.bands.value() == Bands::Multi;
        let modulated = params.mod_source.value() != ModSource::Off;
        let (mut drive_db, bias) = if modulated {
            (live.drive_db.load(Ordering::Relaxed), live.bias.load(Ordering::Relaxed))
        } else {
            (params.drive.unmodulated_plain_value(), params.bias.unmodulated_plain_value())
        };
        let model = if multi {
            drive_db += params.band[state.focus].drive.unmodulated_plain_value();
            params.band[state.focus].model.value()
        } else {
            params.model.value()
        };
        let gain = util::db_to_gain(drive_db.clamp(-24.0, 48.0));

        let label = if multi { "MODEL  (single-band only: each band has its own below)" } else { "MODEL" };
        section(ui, label);
        segments(ui, setter, &params.model);
        ui.add_space(4.0);

        let width = (ui.available_width() - 20.0) / 3.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            panel(ui, width, |p, r| transfer_curve(p, r, model, gain, bias));
            panel(ui, width, |p, r| waveform(p, r, model, gain, bias));
            panel(ui, width, |p, r| harmonics(p, r, model, gain, bias));
        });
        ui.add_space(6.0);

        ui.columns(2, |cols| {
            bar(&mut cols[0], setter, &params.drive, "DRIVE", false);
            bar(&mut cols[0], setter, &params.bias, "BIAS", true);
            bar(&mut cols[0], setter, &params.low_cut, "LOW CUT", false);
            bar(&mut cols[1], setter, &params.tone, "TONE", true);
            bar(&mut cols[1], setter, &params.mix, "MIX", false);
            bar(&mut cols[1], setter, &params.output, "OUTPUT", false);
        });

        section(ui, "BANDS");
        segments(ui, setter, &params.bands);
        if multi {
            ui.columns(2, |cols| {
                bar(&mut cols[0], setter, &params.crossover_low, "CROSSOVER 1", false);
                bar(&mut cols[1], setter, &params.crossover_high, "CROSSOVER 2", false);
            });
            ui.columns(BAND_COUNT, |cols| {
                for (i, name) in ["LOW", "MID", "HIGH"].into_iter().enumerate() {
                    let col = &mut cols[i];
                    let band = &params.band[i];
                    if tab(col, name, state.focus == i) {
                        state.focus = i;
                    }
                    cycle(col, setter, &band.model);
                    bar(col, setter, &band.drive, "DRIVE", true);
                    bar(col, setter, &band.level, "LEVEL", false);
                }
            });
        }

        section(ui, "MODULATION");
        segments(ui, setter, &params.mod_source);
        if modulated {
            ui.columns(2, |cols| {
                cols[0].label(egui::RichText::new("TARGET").monospace().size(10.0).color(TEXT_DIM));
                segments(&mut cols[0], setter, &params.mod_target);
                bar(&mut cols[1], setter, &params.mod_amount, "AMOUNT", true);
            });
            if params.mod_source.value() == ModSource::Lfo {
                ui.columns(2, |cols| {
                    segments(&mut cols[0], setter, &params.mod_shape);
                    bar(&mut cols[1], setter, &params.mod_rate, "RATE", false);
                });
            }
        }

        section(ui, "OVERSAMPLING");
        segments(ui, setter, &params.oversampling);
    });
}

fn section(ui: &mut Ui, text: &str) {
    ui.add_space(6.0);
    ui.label(egui::RichText::new(text).monospace().size(10.0).color(TEXT_DIM));
}

fn load(params: &SmolderParams, setter: &ParamSetter, preset: &Preset) {
    presets::apply(params, setter, preset);
    *params.preset_name.lock().unwrap() = preset.name.clone();
}

fn preset_bar(ui: &mut Ui, params: &Arc<SmolderParams>, setter: &ParamSetter, state: &mut UiState) {
    let current = params.preset_name.lock().unwrap().clone();
    let all: Vec<&Preset> = state.factory.iter().chain(state.user.iter()).collect();
    let index = all.iter().position(|p| p.name == current);
    let mut pick: Option<Preset> = None;

    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("SMOLDER").monospace().size(11.0).color(TEXT_DIM));
        ui.add_space(12.0);
        let step = |delta: isize| {
            let n = all.len() as isize;
            let from = index.map_or(if delta > 0 { -1 } else { 0 }, |i| i as isize);
            all[(from + delta).rem_euclid(n) as usize].clone()
        };
        if ui.button("<").clicked() {
            pick = Some(step(-1));
        }
        egui::ComboBox::from_id_salt("preset").width(220.0).selected_text(current.as_str()).show_ui(ui, |ui| {
            for p in &state.factory {
                if ui.selectable_label(p.name == current, &p.name).clicked() {
                    pick = Some(p.clone());
                }
            }
            if !state.user.is_empty() {
                ui.separator();
                ui.label(egui::RichText::new("USER").monospace().size(9.0).color(TEXT_DIM));
                for p in &state.user {
                    if ui.selectable_label(p.name == current, &p.name).clicked() {
                        pick = Some(p.clone());
                    }
                }
            }
        });
        if ui.button(">").clicked() {
            pick = Some(step(1));
        }
        if ui.button("Save").clicked() {
            state.save_open = !state.save_open;
            state.save_name = if state.user.iter().any(|p| p.name == current) { current.clone() } else { String::new() };
            state.message.clear();
        }
    });
    if let Some(p) = pick {
        load(params, setter, &p);
    }

    if state.save_open {
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut state.save_name).hint_text("Preset name").desired_width(220.0));
            if ui.button("Save preset").clicked() && !state.save_name.trim().is_empty() {
                let name = state.save_name.trim().to_string();
                match presets::save_user(&presets::capture(params, &name)) {
                    Ok(()) => {
                        state.user = presets::load_user();
                        *params.preset_name.lock().unwrap() = name;
                        state.save_open = false;
                        state.message.clear();
                    }
                    Err(e) => state.message = format!("Could not save: {e}"),
                }
            }
            if !state.message.is_empty() {
                ui.label(egui::RichText::new(&state.message).color(RED));
            }
        });
    }
}

/// A band header: highlighted when its band is the one the views show. True when clicked.
fn tab(ui: &mut Ui, text: &str, on: bool) -> bool {
    let (response, painter) = ui.allocate_painter(Vec2::new(ui.available_width(), 22.0), Sense::click());
    let rect = response.rect;
    painter.rect_filled(rect, 4.0, if on { EMBER.gamma_multiply(0.3) } else { PANEL });
    painter.rect_stroke(rect, 4.0, Stroke::new(1.0_f32, if on { EMBER } else { GRID }), egui::StrokeKind::Inside);
    painter.text(rect.center(), Align2::CENTER_CENTER, text, FontId::monospace(10.0), if on { GOLD } else { TEXT_DIM });
    response.clicked()
}

/// A button showing an enum parameter's value: click for the next one, right-click for the previous.
fn cycle<T: Enum + PartialEq + Copy + 'static>(ui: &mut Ui, setter: &ParamSetter, param: &EnumParam<T>) {
    let names = T::variants();
    let (response, painter) = ui.allocate_painter(Vec2::new(ui.available_width(), 26.0), Sense::click());
    let rect = response.rect;
    let current = param.unmodulated_plain_value().to_index();
    painter.rect_filled(rect, 4.0, PANEL);
    painter.rect_stroke(rect, 4.0, Stroke::new(1.0_f32, GRID), egui::StrokeKind::Inside);
    painter.text(rect.left_center() + Vec2::new(8.0, 0.0), Align2::LEFT_CENTER, "MODEL", FontId::monospace(10.0), TEXT_DIM);
    painter.text(rect.right_center() - Vec2::new(8.0, 0.0), Align2::RIGHT_CENTER, names[current].to_uppercase(), FontId::monospace(11.0), GOLD);

    let step = if response.clicked() {
        1
    } else if response.secondary_clicked() {
        names.len() - 1
    } else {
        return;
    };
    setter.begin_set_parameter(param);
    setter.set_parameter(param, T::from_index((current + step) % names.len()));
    setter.end_set_parameter(param);
}

fn panel(ui: &mut Ui, width: f32, content: impl FnOnce(&Painter, Rect)) {
    let (response, painter) = ui.allocate_painter(Vec2::new(width, 190.0), Sense::hover());
    let rect = response.rect;
    painter.rect_filled(rect, 4.0, PANEL);
    content(&painter, rect.shrink(12.0));
    painter.rect_stroke(rect, 4.0, Stroke::new(1.0_f32, GRID), egui::StrokeKind::Inside);
}

/// One cycle of the test sine through the shaper, with the DC the bias adds taken out.
fn test_cycle(model: Model, gain: f32, bias: f32) -> Vec<f32> {
    let y: Vec<f32> = (0..DFT_POINTS)
        .map(|i| shape(model, TEST_LEVEL * gain * (TAU * i as f32 / DFT_POINTS as f32).sin(), bias))
        .collect();
    let mean = y.iter().sum::<f32>() / DFT_POINTS as f32;
    y.into_iter().map(|v| v - mean).collect()
}

fn line(painter: &Painter, points: Vec<Pos2>, width: f32, color: Color32) {
    painter.add(Shape::line(points, Stroke::new(width, color)));
}

fn transfer_curve(painter: &Painter, rect: Rect, model: Model, gain: f32, bias: f32) {
    let limit = 1.2;
    let to_screen = |x: f32, y: f32| {
        Pos2::new(
            rect.center().x + x * rect.width() / 2.0,
            rect.center().y - y.clamp(-limit, limit) / limit * rect.height() / 2.0,
        )
    };
    line(painter, vec![to_screen(-1.0, 0.0), to_screen(1.0, 0.0)], 1.0, GRID);
    line(painter, vec![Pos2::new(rect.center().x, rect.top()), Pos2::new(rect.center().x, rect.bottom())], 1.0, GRID);
    line(painter, vec![to_screen(-1.0, -1.0), to_screen(1.0, 1.0)], 1.0, GRID);

    let points = (0..=160)
        .map(|i| {
            let x = i as f32 / 80.0 - 1.0;
            to_screen(x, shape(model, x * gain, bias))
        })
        .collect();
    line(painter, points, 2.0, EMBER);
    painter.text(rect.left_top(), Align2::LEFT_TOP, "TRANSFER", FontId::monospace(9.0), TEXT_DIM);
}

fn waveform(painter: &Painter, rect: Rect, model: Model, gain: f32, bias: f32) {
    let y = test_cycle(model, gain, bias);
    let limit = 1.2;
    let to_screen = |i: usize, v: f32| {
        Pos2::new(
            rect.left() + rect.width() * i as f32 / (DFT_POINTS - 1) as f32,
            rect.center().y - v.clamp(-limit, limit) / limit * rect.height() / 2.0,
        )
    };
    line(painter, vec![to_screen(0, 0.0), to_screen(DFT_POINTS - 1, 0.0)], 1.0, GRID);
    let input = (0..DFT_POINTS).map(|i| to_screen(i, TEST_LEVEL * (TAU * i as f32 / DFT_POINTS as f32).sin())).collect();
    line(painter, input, 1.0, TEXT_DIM);
    line(painter, y.iter().enumerate().map(|(i, v)| to_screen(i, *v)).collect(), 2.0, GOLD);
    painter.text(rect.left_top(), Align2::LEFT_TOP, "WAVEFORM", FontId::monospace(9.0), TEXT_DIM);
}

fn harmonics(painter: &Painter, rect: Rect, model: Model, gain: f32, bias: f32) {
    let y = test_cycle(model, gain, bias);
    let magnitude = |k: usize| {
        let (mut re, mut im) = (0.0, 0.0);
        for (i, v) in y.iter().enumerate() {
            let phase = TAU * (k * i) as f32 / DFT_POINTS as f32;
            re += v * phase.cos();
            im += v * phase.sin();
        }
        (re * re + im * im).sqrt()
    };
    let fundamental = magnitude(1).max(1e-9);
    let slot = rect.width() / HARMONICS as f32;
    let bottom = rect.bottom() - 12.0;
    let height = bottom - rect.top() - 12.0;
    for k in 1..=HARMONICS {
        let db = 20.0 * (magnitude(k) / fundamental).max(1e-6).log10();
        let level = ((db + HARMONIC_RANGE_DB) / HARMONIC_RANGE_DB).clamp(0.0, 1.0);
        let x = rect.left() + slot * (k - 1) as f32;
        let bar = Rect::from_min_max(Pos2::new(x + 2.0, bottom - level * height), Pos2::new(x + slot - 2.0, bottom));
        painter.rect_filled(bar, 1.5, if k % 2 == 0 { GOLD } else { RED });
        painter.text(Pos2::new(x + slot / 2.0, bottom + 2.0), Align2::CENTER_TOP, k.to_string(), FontId::monospace(8.0), TEXT_DIM);
    }
    painter.text(rect.left_top(), Align2::LEFT_TOP, "HARMONICS", FontId::monospace(9.0), TEXT_DIM);
}

/// A row of buttons for an enum parameter.
fn segments<T: Enum + PartialEq + Copy + 'static>(ui: &mut Ui, setter: &ParamSetter, param: &EnumParam<T>) {
    let names = T::variants();
    let size = Vec2::new(ui.available_width(), 28.0);
    let (response, painter) = ui.allocate_painter(size, Sense::click());
    let rect = response.rect;
    let width = rect.width() / names.len() as f32;
    let current = param.unmodulated_plain_value().to_index();

    for (i, name) in names.iter().enumerate() {
        let cell = Rect::from_min_size(Pos2::new(rect.left() + width * i as f32, rect.top()), Vec2::new(width, rect.height()));
        let on = i == current;
        painter.rect_filled(cell.shrink(1.0), 4.0, if on { EMBER.gamma_multiply(0.3) } else { PANEL });
        painter.rect_stroke(cell.shrink(1.0), 4.0, Stroke::new(1.0_f32, if on { EMBER } else { GRID }), egui::StrokeKind::Inside);
        painter.text(cell.center(), Align2::CENTER_CENTER, name.to_uppercase(), FontId::monospace(11.0), if on { GOLD } else { TEXT_DIM });
    }

    if response.clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            let i = (((pos.x - rect.left()) / width) as usize).min(names.len() - 1);
            setter.begin_set_parameter(param);
            setter.set_parameter(param, T::from_index(i));
            setter.end_set_parameter(param);
        }
    }
}

/// A horizontal bar fader for a float parameter: drag to set, double-click to reset. A bipolar bar
/// fills out from the middle.
fn bar(ui: &mut Ui, setter: &ParamSetter, param: &FloatParam, label: &str, bipolar: bool) {
    let size = Vec2::new(ui.available_width(), 26.0);
    let (response, painter) = ui.allocate_painter(size, Sense::click_and_drag());
    let rect = response.rect;
    let norm = param.unmodulated_normalized_value();

    painter.rect_filled(rect, 4.0, PANEL);
    let (from, to) = if bipolar { (0.5_f32.min(norm), 0.5_f32.max(norm)) } else { (0.0, norm) };
    let fill = Rect::from_min_max(
        Pos2::new(rect.left() + rect.width() * from, rect.top()),
        Pos2::new(rect.left() + rect.width() * to, rect.bottom()),
    );
    painter.rect_filled(fill, 4.0, EMBER.gamma_multiply(0.27));
    let edge = rect.left() + rect.width() * norm;
    painter.rect_filled(
        Rect::from_min_size(Pos2::new((edge - 3.0).max(rect.left()), rect.top()), Vec2::new(3.0, rect.height())),
        1.5,
        EMBER,
    );
    painter.text(Pos2::new(rect.left() + 8.0, rect.center().y), Align2::LEFT_CENTER, label, FontId::monospace(10.0), TEXT_DIM);
    painter.text(Pos2::new(rect.right() - 8.0, rect.center().y), Align2::RIGHT_CENTER, param.to_string(), FontId::monospace(11.0), GOLD);
    painter.rect_stroke(rect, 4.0, Stroke::new(1.0_f32, GRID), egui::StrokeKind::Inside);

    if response.drag_started() {
        setter.begin_set_parameter(param);
    }
    if response.dragged() || response.drag_started() {
        if let Some(pos) = response.interact_pointer_pos() {
            setter.set_parameter_normalized(param, ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0));
        }
    }
    if response.drag_stopped() {
        setter.end_set_parameter(param);
    }
    if response.double_clicked() {
        setter.begin_set_parameter(param);
        setter.set_parameter(param, param.default_plain_value());
        setter.end_set_parameter(param);
    }
}
