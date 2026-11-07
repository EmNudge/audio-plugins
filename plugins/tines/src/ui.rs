//! The editor: a comb of tuning-fork tines standing on a log-frequency spectrum.
//!
//! The faint white skyline is what the plugin is being fed. Every resonator in the bank is a
//! tine: a hairline from the floor to the top where it is tuned, and a bright bar that rises with
//! the amplitude it is really ringing at, so a struck bank lights up and then dies away partial by
//! partial, the high ones first when Damping is up. The root note's tines are coral, the other
//! notes of the chord violet, and the coral outline over the white skyline is the output.
//!
//! The diamond is the root: drag it sideways to retune (it snaps to semitones, hold shift to
//! slide) and up and down to set how long it rings. Its height is the decay time on a log scale,
//! so a tall diamond is a long tail. A dashed white needle marks the pitch found in the input when
//! the source is Track.
//!
//! Knobs: drag vertically (hold shift for fine), double-click to reset.

use nih_plug::prelude::*;
use nih_plug_egui::egui::{
    self, Align2, Color32, FontId, Mesh, Painter, Pos2, Rect, Sense, Shape as EguiShape, Stroke, Ui, Vec2,
};
use std::f32::consts::PI;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::dsp::{band_hz, ftom, Chord, Meter, Model, Source, BANDS, BAND_HI, BAND_LO, FLOOR_DB, MAX_MODES, MAX_PARTIALS};
use crate::{note_name, presets, TinesParams};

const BG: Color32 = Color32::from_rgb(0x0d, 0x0a, 0x12);
const PANEL: Color32 = Color32::from_rgb(0x18, 0x13, 0x1e);
const GRID: Color32 = Color32::from_rgb(0x2b, 0x26, 0x36);
const TEXT_DIM: Color32 = Color32::from_rgb(0x85, 0x7e, 0x96);
const TEXT: [f32; 3] = [240.0, 236.0, 244.0];
/// The root note, and the output.
const CORAL: [f32; 3] = [255.0, 90.0, 71.0];
/// The other notes of the chord.
const VIOLET: [f32; 3] = [160.0, 112.0, 255.0];

const CHIP_H: f32 = 22.0;
const SELECT_H: f32 = 30.0;
const KNOB_ROW: f32 = 100.0;
/// Decibels the spectrum display spans below full scale, and the same for a tine's ring.
const RANGE_DB: f32 = 70.0;
const TINE_RANGE_DB: f32 = 66.0;
/// Decibels a held bin falls per repaint.
const FALL_DB: f32 = 0.9;
const DECAY_MIN: f32 = 0.05;
const DECAY_MAX: f32 = 20.0;

pub struct UiState {
    last_frames: usize,
    in_db: [f32; BANDS],
    out_db: [f32; BANDS],
    mode_db: [f32; MAX_MODES],
}

impl Default for UiState {
    fn default() -> Self {
        Self { last_frames: 0, in_db: [FLOOR_DB; BANDS], out_db: [FLOOR_DB; BANDS], mode_db: [FLOOR_DB; MAX_MODES] }
    }
}

pub fn background() -> Color32 {
    BG
}

fn rgba(c: [f32; 3], a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c[0] as u8, c[1] as u8, c[2] as u8, a.clamp(0.0, 255.0) as u8)
}

/// Everything the audio thread published, with the levels held: rise fast, fall slowly.
struct Frame {
    root_hz: f32,
    pitch_hz: f32,
    mode_hz: [f32; MAX_MODES],
}

fn snapshot(meter: &Meter, state: &mut UiState) -> Frame {
    let frames = meter.frames.load(Ordering::Acquire);
    if frames != state.last_frames {
        state.last_frames = frames;
        for k in 0..BANDS {
            for (held, new) in [(&mut state.in_db[k], meter.spec_in[k].load(Ordering::Relaxed)), (&mut state.out_db[k], meter.spec_out[k].load(Ordering::Relaxed))] {
                *held = new.max(*held - FALL_DB);
            }
        }
    }
    let mut mode_hz = [0.0; MAX_MODES];
    for m in 0..MAX_MODES {
        mode_hz[m] = meter.mode_hz[m].load(Ordering::Relaxed);
        let level = meter.mode_level[m].load(Ordering::Relaxed);
        let db = (20.0 * (level + 1e-7).log10()).max(FLOOR_DB);
        state.mode_db[m] = db.max(state.mode_db[m] - 1.2);
    }
    Frame { root_hz: meter.root_hz.load(Ordering::Relaxed), pitch_hz: meter.pitch_hz.load(Ordering::Relaxed), mode_hz }
}

pub fn draw(ui: &mut Ui, params: &Arc<TinesParams>, setter: &ParamSetter, meter: &Arc<Meter>, state: &mut UiState) {
    ui.ctx().request_repaint();
    let f = snapshot(meter, state);

    header(ui, params, setter, &f);
    ui.add_space(8.0);
    source_row(ui, params, setter);
    ui.add_space(8.0);

    let display_h = ui.available_height() - SELECT_H - KNOB_ROW - 16.0;
    let (area, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), display_h), Sense::hover());
    tines(ui, area, params, setter, &f, state);

    ui.add_space(8.0);
    selectors(ui, params, setter);
    ui.add_space(8.0);
    knobs(ui, params, setter, &f);
}

fn header(ui: &mut Ui, params: &TinesParams, setter: &ParamSetter, f: &Frame) {
    let theme = preset_kit::ui::Theme { fill: PANEL, hover: GRID, ink: rgba(TEXT, 255.0), dim: TEXT_DIM };
    preset_kit::ui::picker_top_right(ui, 24.0, &presets::BANK, params, setter, &theme);
    let live = f.root_hz > 0.0;
    let note = if live { format!("{:>4}", note_name(ftom(f.root_hz as f64) as f32)) } else { "  --".to_string() };
    let hz = if live { format!("{:>6.1}", f.root_hz) } else { "    --".to_string() };
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("TINES").monospace().size(11.0).color(TEXT_DIM));
        ui.label(egui::RichText::new("tuned resonator").monospace().size(9.0).color(GRID));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(preset_kit::ui::WIDTH + 14.0);
            for (caption, text, color) in [
                ("DECAY", params.decay.to_string(), VIOLET),
                ("ROOT", format!("{hz}Hz"), CORAL),
                ("NOTE", note, CORAL),
            ] {
                ui.label(egui::RichText::new(format!("{text:>7}")).monospace().size(18.0).color(rgba(color, 255.0)));
                ui.label(egui::RichText::new(caption).monospace().size(9.0).color(TEXT_DIM));
                ui.add_space(12.0);
            }
        });
    });
}

/// A row of chips starting at `pos`. Returns the clicked index and the row's right edge. Locked
/// rows dim and ignore the pointer.
fn chips(ui: &mut Ui, pos: Pos2, id: &str, labels: &[String], selected: Option<usize>, enabled: bool, color: [f32; 3]) -> (Option<usize>, f32) {
    let painter = ui.painter().clone();
    let mut x = pos.x;
    let mut clicked = None;
    for (i, label) in labels.iter().enumerate() {
        let w = label.chars().count() as f32 * 6.4 + 18.0;
        let rect = Rect::from_min_size(Pos2::new(x, pos.y), Vec2::new(w, CHIP_H));
        let r = ui.interact(rect, ui.id().with((id, i)), if enabled { Sense::click() } else { Sense::hover() });
        let lit = enabled && r.hovered();
        let on = Some(i) == selected;
        let fade = if enabled { 1.0 } else { 0.4 };
        if on {
            painter.rect_filled(rect.expand(2.0), 6.0, rgba(color, 30.0 * fade));
            painter.rect_filled(rect, 4.0, rgba(color, 235.0 * fade));
        } else {
            painter.rect_filled(rect, 4.0, if lit { GRID } else { PANEL });
        }
        let ink = if on { BG } else if lit { rgba(TEXT, 255.0) } else { TEXT_DIM };
        painter.text(rect.center(), Align2::CENTER_CENTER, label, FontId::monospace(10.0), ink);
        if r.clicked() {
            clicked = Some(i);
        }
        x += w + 4.0;
    }
    (clicked, x)
}

fn pick<T: Enum + Copy + PartialEq>(setter: &ParamSetter, param: &EnumParam<T>, index: usize) {
    setter.begin_set_parameter(param);
    setter.set_parameter_normalized(param, index as f32 / (T::variants().len() - 1) as f32);
    setter.end_set_parameter(param);
}

fn upper<T: Enum>() -> Vec<String> {
    T::variants().iter().map(|n| n.to_uppercase()).collect()
}

fn source_row(ui: &mut Ui, params: &TinesParams, setter: &ParamSetter) {
    let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), CHIP_H), Sense::hover());
    let painter = ui.painter().clone();
    let label = |x: f32, text: &str| {
        painter.text(Pos2::new(x, row.center().y), Align2::LEFT_CENTER, text, FontId::monospace(9.0), TEXT_DIM);
    };
    label(row.left(), "SOURCE");
    let (clicked, end) = chips(ui, Pos2::new(row.left() + 58.0, row.top()), "source", &upper::<Source>(), Some(params.source.value().to_index()), true, CORAL);
    if let Some(i) = clicked {
        pick(setter, &params.source, i);
    }
    let x = end + 22.0;
    label(x, "MODEL");
    let (clicked, _) = chips(ui, Pos2::new(x + 50.0, row.top()), "model", &upper::<Model>(), Some(params.model.value().to_index()), true, VIOLET);
    if let Some(i) = clicked {
        pick(setter, &params.model, i);
    }
}

fn selectors(ui: &mut Ui, params: &TinesParams, setter: &ParamSetter) {
    let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), SELECT_H), Sense::hover());
    let y = row.top() + (SELECT_H - CHIP_H) * 0.5;
    let painter = ui.painter().clone();
    painter.text(Pos2::new(row.left(), row.center().y), Align2::LEFT_CENTER, "CHORD", FontId::monospace(9.0), TEXT_DIM);
    // Held keys are the chord in MIDI mode.
    let keys = params.source.value() == Source::Midi;
    let (clicked, end) = chips(ui, Pos2::new(row.left() + 50.0, y), "chord", &upper::<Chord>(), Some(params.chord.value().to_index()), !keys, VIOLET);
    if let Some(i) = clicked {
        pick(setter, &params.chord, i);
    }
    if keys {
        painter.text(Pos2::new(end + 8.0, row.center().y), Align2::LEFT_CENTER, "keys played are the chord", FontId::monospace(9.0), TEXT_DIM);
    }
    // Freeze sits at the right edge.
    let w = 6.0 * 6.4 + 18.0;
    let frozen = params.freeze.value();
    let (clicked, _) = chips(ui, Pos2::new(row.right() - w, y), "freeze", &["FREEZE".to_string()], frozen.then_some(0), true, CORAL);
    if clicked.is_some() {
        setter.begin_set_parameter(&params.freeze);
        setter.set_parameter(&params.freeze, !frozen);
        setter.end_set_parameter(&params.freeze);
    }
}

fn height(db: f32, range: f32) -> f32 {
    ((db + range) / range).clamp(0.0, 1.0)
}

fn decay_t(decay: f32) -> f32 {
    ((decay / DECAY_MIN).ln() / (DECAY_MAX / DECAY_MIN).ln()).clamp(0.0, 1.0)
}

fn tines(ui: &mut Ui, rect: Rect, params: &TinesParams, setter: &ParamSetter, f: &Frame, state: &UiState) {
    let response = ui.interact(rect, ui.id().with("tines"), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 6.0, PANEL);
    let s = params.settings();
    let lit = response.hovered() || response.dragged();

    let plot = Rect::from_min_max(Pos2::new(rect.left() + 16.0, rect.top() + 30.0), Pos2::new(rect.right() - 16.0, rect.bottom() - 26.0));
    let span = (BAND_HI / BAND_LO).ln();
    let x_of_hz = |hz: f32| plot.left() + plot.width() * ((hz.max(1.0) / BAND_LO).ln() / span).clamp(0.0, 1.0);
    let hz_of_x = |x: f32| BAND_LO * (((x - plot.left()) / plot.width()).clamp(0.0, 1.0) * span).exp();
    let base = plot.bottom();

    painter.text(Pos2::new(rect.left() + 10.0, rect.top() + 8.0), Align2::LEFT_TOP, "input in white, tines ring where they are tuned", FontId::monospace(9.0), TEXT_DIM);
    painter.text(Pos2::new(rect.right() - 10.0, rect.top() + 8.0), Align2::RIGHT_TOP, "drag the diamond: pitch / decay", FontId::monospace(9.0), TEXT_DIM);

    // Frequency ruler.
    painter.line_segment([Pos2::new(plot.left(), base), Pos2::new(plot.right(), base)], Stroke::new(1.0_f32, GRID));
    for hz in [50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10000.0] {
        let x = x_of_hz(hz);
        painter.line_segment([Pos2::new(x, base), Pos2::new(x, base + 4.0)], Stroke::new(1.0_f32, TEXT_DIM));
        painter.line_segment([Pos2::new(x, base), Pos2::new(x, plot.top())], Stroke::new(1.0_f32, rgba(TEXT, 5.0)));
        let label = if hz >= 1000.0 { format!("{:.0}k", hz / 1000.0) } else { format!("{hz:.0}") };
        painter.text(Pos2::new(x, base + 6.0), Align2::CENTER_TOP, label, FontId::monospace(8.0), TEXT_DIM);
    }

    // The input, and the output over it.
    let xs: Vec<f32> = (0..BANDS).map(|b| x_of_hz(band_hz(b))).collect();
    let tips = |db: &[f32; BANDS]| -> Vec<f32> { db.iter().map(|&d| height(d, RANGE_DB) * plot.height() * 0.6).collect() };
    let (in_tips, out_tips) = (tips(&state.in_db), tips(&state.out_db));
    skyline(&painter, &xs, &in_tips, base, TEXT, 4.0, 60.0);
    outline(&painter, &xs, &in_tips, base, TEXT, 0.5);
    skyline(&painter, &xs, &out_tips, base, CORAL, 4.0, 40.0);
    outline(&painter, &xs, &out_tips, base, CORAL, 0.9);

    // Tracked pitch of the input.
    if s.source == Source::Track && f.pitch_hz > 0.0 {
        let x = x_of_hz(f.pitch_hz);
        painter.add(EguiShape::Vec(EguiShape::dashed_line(&[Pos2::new(x, base), Pos2::new(x, plot.top())], Stroke::new(1.0_f32, rgba(TEXT, 150.0)), 4.0, 4.0)));
        painter.text(Pos2::new(x + 5.0, plot.top() + 12.0), Align2::LEFT_TOP, format!("in {:.0} Hz", f.pitch_hz), FontId::monospace(9.0), rgba(TEXT, 170.0));
    }

    // The tines. Hairlines first so the bright bars sit over them.
    let color_of = |m: usize| if m / MAX_PARTIALS == 0 { CORAL } else { VIOLET };
    for m in 0..MAX_MODES {
        if f.mode_hz[m] > 0.0 {
            let x = x_of_hz(f.mode_hz[m]);
            painter.line_segment([Pos2::new(x, base), Pos2::new(x, plot.top())], Stroke::new(1.0_f32, rgba(color_of(m), 26.0)));
        }
    }
    for m in 0..MAX_MODES {
        if f.mode_hz[m] <= 0.0 {
            continue;
        }
        let x = x_of_hz(f.mode_hz[m]);
        let c = color_of(m);
        let h = height(state.mode_db[m], TINE_RANGE_DB) * plot.height();
        // Every tine has a foot, so a silent bank still shows where it is tuned.
        let top = Pos2::new(x, base - h.max(3.0));
        let strength = if h > 1.0 { 1.0 } else { 0.35 };
        painter.line_segment([Pos2::new(x, base), top], Stroke::new(7.0_f32, rgba(c, 24.0 * strength)));
        painter.line_segment([Pos2::new(x, base), top], Stroke::new(2.0_f32, rgba(c, 235.0 * strength)));
        painter.circle_filled(top, 2.6, rgba(TEXT, 230.0 * strength));
        painter.circle_filled(top, 6.0, rgba(c, 40.0 * strength));
    }

    // The root: a diamond at (pitch, decay).
    let root_hz = if f.root_hz > 0.0 { f.root_hz } else { crate::dsp::mtof(s.note as f64 + s.octave as f64 * 12.0 + s.fine_cents as f64 / 100.0) as f32 };
    let hx = x_of_hz(root_hz);
    let hy = base - decay_t(s.decay_s) * plot.height();
    let strength = if lit { 1.0 } else { 0.85 };
    painter.line_segment([Pos2::new(plot.left(), hy), Pos2::new(plot.right(), hy)], Stroke::new(1.0_f32, rgba(CORAL, 22.0)));
    painter.line_segment([Pos2::new(hx, hy), Pos2::new(hx, base)], Stroke::new(1.0_f32, rgba(CORAL, 70.0)));
    let diamond = |r: f32| vec![Pos2::new(hx, hy - r), Pos2::new(hx + r, hy), Pos2::new(hx, hy + r), Pos2::new(hx - r, hy)];
    painter.add(EguiShape::convex_polygon(diamond(15.0), rgba(CORAL, 26.0 * strength), Stroke::NONE));
    painter.add(EguiShape::convex_polygon(diamond(9.0), rgba(CORAL, 255.0 * strength), Stroke::NONE));
    painter.add(EguiShape::convex_polygon(diamond(3.0), BG, Stroke::NONE));
    let flip = hx > plot.right() - 190.0;
    let label = format!("{}  {:.0} Hz  {}", note_name(ftom(root_hz as f64) as f32), root_hz, params.decay);
    let size = Vec2::new(label.chars().count() as f32 * 6.0 + 12.0, 18.0);
    let at = Pos2::new(if flip { hx - 16.0 - size.x } else { hx + 16.0 }, hy - 9.0 - size.y * 0.5);
    painter.rect_filled(Rect::from_min_size(at, size), 4.0, rgba([24.0, 19.0, 30.0], 225.0));
    painter.text(at + Vec2::new(6.0, size.y * 0.5), Align2::LEFT_CENTER, label, FontId::monospace(10.0), rgba(CORAL, 235.0));

    // Dragging: sideways is pitch (Note source only), up and down is decay.
    if response.drag_started() {
        setter.begin_set_parameter(&params.decay);
        if s.source == Source::Note {
            setter.begin_set_parameter(&params.note);
        }
    }
    if response.dragged() || response.drag_started() {
        if let Some(pos) = response.interact_pointer_pos() {
            let t = ((base - pos.y) / plot.height()).clamp(0.0, 1.0);
            setter.set_parameter(&params.decay, DECAY_MIN * (DECAY_MAX / DECAY_MIN).powf(t));
            if s.source == Source::Note {
                let hz = hz_of_x(pos.x);
                let mut note = ftom(hz as f64) as f32 - s.octave as f32 * 12.0 - s.fine_cents / 100.0;
                if !ui.input(|i| i.modifiers.shift) {
                    note = note.round();
                }
                setter.set_parameter(&params.note, note.clamp(12.0, 96.0));
            }
        }
    }
    if response.drag_stopped() {
        setter.end_set_parameter(&params.decay);
        if s.source == Source::Note {
            setter.end_set_parameter(&params.note);
        }
    }
    if response.double_clicked() {
        for param in [&params.decay, &params.note] {
            setter.begin_set_parameter(param);
            setter.set_parameter(param, param.default_plain_value());
            setter.end_set_parameter(param);
        }
    }
}

/// A filled skyline: columns at `xs` rising from `base` by `tips`, shaded from `base_alpha` at
/// the base to `tip_alpha` at the tip.
fn skyline(painter: &Painter, xs: &[f32], tips: &[f32], base: f32, color: [f32; 3], base_alpha: f32, tip_alpha: f32) {
    let mut mesh = Mesh::default();
    for i in 0..xs.len() {
        mesh.colored_vertex(Pos2::new(xs[i], base), rgba(color, base_alpha));
        mesh.colored_vertex(Pos2::new(xs[i], base - tips[i]), rgba(color, tip_alpha));
    }
    for i in 0..xs.len().saturating_sub(1) as u32 {
        let v = 2 * i;
        mesh.add_triangle(v, v + 1, v + 3);
        mesh.add_triangle(v, v + 3, v + 2);
    }
    painter.add(EguiShape::mesh(mesh));
}

/// A glowing polyline through the tips.
fn outline(painter: &Painter, xs: &[f32], tips: &[f32], base: f32, color: [f32; 3], strength: f32) {
    let points: Vec<Pos2> = xs.iter().zip(tips).map(|(&x, &t)| Pos2::new(x, base - t)).collect();
    painter.add(EguiShape::line(points.clone(), Stroke::new(5.0_f32, rgba(color, 26.0 * strength))));
    painter.add(EguiShape::line(points, Stroke::new(1.4_f32, rgba(color, 235.0 * strength))));
}

#[allow(unused_assignments)]
fn knobs(ui: &mut Ui, params: &TinesParams, setter: &ParamSetter, f: &Frame) {
    let (response, painter) = ui.allocate_painter(Vec2::new(ui.available_width(), KNOB_ROW), Sense::hover());
    let row = response.rect;
    let source = params.source.value();
    let _ = f;
    let fixed_note = match source {
        Source::Note => None,
        Source::Midi => Some("keys".to_string()),
        Source::Track => Some("input".to_string()),
    };
    let width = row.width() / 15.0;
    let mut index = 0;
    macro_rules! cell {
        ($param:expr, $label:expr, $color:expr, $enabled:expr, $bipolar:expr, $shown:expr) => {{
            let rect = Rect::from_min_size(Pos2::new(row.left() + index as f32 * width, row.top()), Vec2::new(width, KNOB_ROW));
            knob(ui, &painter, rect, setter, $param, $label, $color, $enabled, $bipolar, $shown);
            index += 1;
        }};
    }
    cell!(&params.note, "NOTE", CORAL, source == Source::Note, false, fixed_note.as_deref());
    cell!(&params.octave, "OCTAVE", CORAL, true, true, None);
    cell!(&params.fine, "FINE", CORAL, true, true, None);
    cell!(&params.decay, "DECAY", VIOLET, true, false, None);
    cell!(&params.damping, "DAMPING", VIOLET, true, false, None);
    cell!(&params.tilt, "TILT", VIOLET, true, true, None);
    cell!(&params.partials, "PARTIALS", VIOLET, true, false, None);
    cell!(&params.stiffness, "STIFF", VIOLET, params.model.value() == Model::String, false, None);
    cell!(&params.detune, "DETUNE", CORAL, true, false, None);
    cell!(&params.drift, "DRIFT", CORAL, true, false, None);
    cell!(&params.feedback, "FEEDBACK", CORAL, true, false, None);
    cell!(&params.pluck, "PLUCK", CORAL, true, false, None);
    cell!(&params.tone, "TONE", TEXT, true, false, None);
    cell!(&params.level, "LEVEL", TEXT, true, true, None);
    cell!(&params.mix, "MIX", TEXT, true, false, None);
}

fn arc(center: Pos2, radius: f32, from: f32, to: f32) -> Vec<Pos2> {
    let steps = (((to - from).abs() / 0.08) as usize).max(2);
    (0..=steps)
        .map(|s| {
            let a = from + (to - from) * s as f32 / steps as f32;
            center + Vec2::new(a.cos(), a.sin()) * radius
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn knob<P: Param + std::fmt::Display>(ui: &mut Ui, painter: &Painter, cell: Rect, setter: &ParamSetter, param: &P, label: &str, color: [f32; 3], enabled: bool, bipolar: bool, shown: Option<&str>) {
    let r = ui.interact(cell, ui.id().with(label), if enabled { Sense::click_and_drag() } else { Sense::hover() });
    if r.drag_started() {
        setter.begin_set_parameter(param);
    }
    if r.dragged() {
        let fine = ui.input(|i| i.modifiers.shift);
        let speed = if fine { 1.0 / 1200.0 } else { 1.0 / 160.0 };
        let v = param.unmodulated_normalized_value() - r.drag_delta().y * speed;
        setter.set_parameter_normalized(param, v.clamp(0.0, 1.0));
    }
    if r.drag_stopped() {
        setter.end_set_parameter(param);
    }
    if r.double_clicked() {
        setter.begin_set_parameter(param);
        setter.set_parameter(param, param.default_plain_value());
        setter.end_set_parameter(param);
    }

    let fade = if enabled { 1.0 } else { 0.3 };
    let norm = param.unmodulated_normalized_value();
    let center = Pos2::new(cell.center().x, cell.top() + 34.0);
    let radius = 22.0;
    let (start, end) = (PI * 0.75, PI * 2.25);
    let lit = enabled && (r.hovered() || r.dragged());
    let main = rgba(color, 255.0 * fade);

    painter.circle_filled(center, radius - 5.0, if lit { PANEL } else { Color32::from_rgb(0x11, 0x0e, 0x16) });
    painter.add(EguiShape::line(arc(center, radius, start, end), Stroke::new(3.0_f32, GRID)));
    let angle = start + (end - start) * norm;
    let from = if bipolar { start + (end - start) * 0.5 } else { start };
    if (angle - from).abs() > 0.01 {
        painter.add(EguiShape::line(arc(center, radius, from, angle), Stroke::new(8.0_f32, rgba(color, 30.0 * fade))));
        painter.add(EguiShape::line(arc(center, radius, from, angle), Stroke::new(3.0_f32, main)));
    }
    let dir = Vec2::new(angle.cos(), angle.sin());
    painter.line_segment([center + dir * (radius - 12.0), center + dir * (radius - 6.0)], Stroke::new(2.0_f32, rgba(TEXT, 255.0 * fade)));

    painter.text(Pos2::new(cell.center().x, cell.top() + 69.0), Align2::CENTER_CENTER, label, FontId::monospace(9.0), TEXT_DIM);
    let text = shown.map_or_else(|| param.to_string(), str::to_owned);
    painter.text(Pos2::new(cell.center().x, cell.top() + 85.0), Align2::CENTER_CENTER, text, FontId::monospace(10.0), if shown.is_some() { rgba(color, 150.0) } else { main });
}
