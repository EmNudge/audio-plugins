//! The editor: four drumheads seen from above. Each dot is one cell of the head's simulation,
//! sized by how fast the skin is moving there and coloured by what it is doing: coral for the
//! fundamental, cyan for overtones. Eight ticks around the rim are the shell's resonances.
//! Click a head to strike it; the mallet point stays where you leave it, and its circle shows
//! how much skin the mallet touches.
//!
//! Under the drums, a card for each hat and cymbal: a plate seen from above, with its modes laid
//! out in a spiral from the bell (lowest) to the edge (highest), each dot sized by how loud that
//! mode is ringing, so you see the top end leave first.

use crate::engine::{Meters, HEADS, METALS, METAL_NAMES, METAL_NOTES, MODES, NAMES, NOTES};
use crate::metal::MMODES;
use crate::membrane::{cell_pos, in_head, NN, RADIUS};
use crate::{presets, TympanParams};
use nih_plug::prelude::*;
use nih_plug_egui::egui::{self, Align2, Color32, CursorIcon, FontId, Painter, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, Vec2};
use std::f32::consts::PI;
use std::sync::atomic::Ordering;
use std::sync::Arc;

pub const BG: Color32 = Color32::from_rgb(0x0a, 0x11, 0x10);
pub const PANEL: Color32 = Color32::from_rgb(0x12, 0x1c, 0x1a);
pub const PANEL_HI: Color32 = Color32::from_rgb(0x1c, 0x2a, 0x27);
pub const GRID: Color32 = Color32::from_rgb(0x2b, 0x3d, 0x38);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x7d, 0x93, 0x8c);
/// Skin at the fundamental, skin at an overtone, shell, and numerals.
pub const CORAL: [f32; 3] = [255.0, 106.0, 61.0];
pub const CYAN: [f32; 3] = [61.0, 214.0, 255.0];
pub const YELLOW: [f32; 3] = [255.0, 214.0, 64.0];
pub const CREAM: [f32; 3] = [238.0, 232.0, 214.0];
/// Hats and cymbals.
pub const MINT: [f32; 3] = [96.0, 236.0, 160.0];

const HEADER_H: f32 = 44.0;
const GAP: f32 = 12.0;
const LABEL_H: f32 = 30.0;
const KNOB_H: f32 = 76.0;
const STRIP_H: f32 = 86.0;
const PLATE_H: f32 = 152.0;
/// The row of striker and shell selectors under each drum's knobs.
const CYCLE_H: f32 = 28.0;

pub fn background() -> Color32 {
    BG
}

pub fn rgba(c: [f32; 3], a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c[0] as u8, c[1] as u8, c[2] as u8, a.clamp(0.0, 255.0) as u8)
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

pub struct UiState {
    /// Displacement and velocity envelopes per head, eased so they fall slowly.
    env: Vec<f32>,
    modes: [[f32; MODES]; HEADS],
    hit: [f32; HEADS],
    plate_modes: [[f32; MMODES]; METALS],
    plate_hit: [f32; METALS],
    peak: f32,
    wires: f32,
    /// Whether a mallet drag is in progress on a head.
    dragging: [bool; HEADS],
}

impl Default for UiState {
    fn default() -> Self {
        Self { env: vec![0.0; HEADS * 2 * NN], modes: [[0.0; MODES]; HEADS], hit: [0.0; HEADS], plate_modes: [[0.0; MMODES]; METALS], plate_hit: [0.0; METALS], peak: 0.0, wires: 0.0, dragging: [false; HEADS] }
    }
}

/// A hardware knob: vertical drag, shift for fine, double-click to reset.
#[allow(clippy::too_many_arguments)]
fn knob(ui: &mut Ui, painter: &Painter, cell: Rect, setter: &ParamSetter, param: &FloatParam, label: &str, color: [f32; 3], bipolar: bool) {
    let r = ui.interact(cell, ui.id().with(("knob", label, param.name(), cell.min.x as i32, cell.min.y as i32)), Sense::click_and_drag());
    if r.drag_started() {
        setter.begin_set_parameter(param);
    }
    if r.dragged() {
        let speed = if ui.input(|i| i.modifiers.shift) { 1.0 / 1200.0 } else { 1.0 / 160.0 };
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
    if r.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeVertical);
    }
    let norm = param.unmodulated_normalized_value();
    let center = Pos2::new(cell.center().x, cell.top() + 24.0);
    let radius = 17.0;
    let (start, end) = (PI * 0.75, PI * 2.25);
    let arc = |from: f32, to: f32| -> Vec<Pos2> {
        let steps = (((to - from).abs() / 0.08) as usize).max(2);
        (0..=steps)
            .map(|s| {
                let a = from + (to - from) * s as f32 / steps as f32;
                center + Vec2::new(a.cos(), a.sin()) * radius
            })
            .collect()
    };
    let lit = r.hovered() || r.dragged();
    painter.circle_filled(center, radius - 4.0, if lit { PANEL_HI } else { PANEL });
    painter.add(Shape::line(arc(start, end), Stroke::new(3.0_f32, GRID)));
    let to = start + (end - start) * norm;
    let from = if bipolar { start + (end - start) * 0.5 } else { start };
    let (a, b) = if to >= from { (from, to) } else { (to, from) };
    if b - a > 0.01 {
        painter.add(Shape::line(arc(a, b), Stroke::new(8.0_f32, rgba(color, 28.0))));
        painter.add(Shape::line(arc(a, b), Stroke::new(3.0_f32, rgba(color, 255.0))));
    }
    let dir = Vec2::new(to.cos(), to.sin());
    painter.line_segment([center + dir * (radius - 10.0), center + dir * (radius - 4.0)], Stroke::new(2.0_f32, rgba(CREAM, 255.0)));
    painter.text(Pos2::new(cell.center().x, cell.top() + 50.0), Align2::CENTER_CENTER, label, FontId::monospace(8.5), TEXT_DIM);
    painter.text(Pos2::new(cell.center().x, cell.top() + 63.0), Align2::CENTER_CENTER, param.to_string(), FontId::monospace(if cell.width() < 50.0 { 8.5 } else { 9.5 }), rgba(color, 255.0));
}

pub fn draw(ui: &mut Ui, params: &Arc<TympanParams>, meters: &Meters, setter: &ParamSetter, state: &mut UiState) {
    let dt = ui.input(|i| i.unstable_dt).clamp(0.001, 0.1);
    ui.ctx().request_repaint();

    // Read what the audio thread wrote: rise fast, fall slow.
    let fall = (-dt * 5.0).exp();
    for h in 0..HEADS {
        for kind in 0..2 {
            let slots = meters.head_grid(h, kind);
            let base = (h * 2 + kind) * NN;
            for (i, slot) in slots.iter().enumerate() {
                let v = slot.load(Ordering::Relaxed);
                let e = &mut state.env[base + i];
                *e = v.max(*e * fall);
            }
        }
        for (k, slot) in meters.head_modes(h).iter().enumerate() {
            state.modes[h][k] = slot.load(Ordering::Relaxed).max(state.modes[h][k] * (-dt * 6.0).exp());
        }
        state.hit[h] = (state.hit[h] * (-dt * 4.0).exp()).max(meters.hit[h].swap(0.0, Ordering::Relaxed));
    }
    for m in 0..METALS {
        for (k, slot) in meters.plate_modes(m).iter().enumerate() {
            state.plate_modes[m][k] = slot.load(Ordering::Relaxed).max(state.plate_modes[m][k] * (-dt * 5.0).exp());
        }
        state.plate_hit[m] = (state.plate_hit[m] * (-dt * 4.0).exp()).max(meters.metal_hit[m].swap(0.0, Ordering::Relaxed));
    }
    state.wires = meters.wires.load(Ordering::Relaxed).max(state.wires * (-dt * 6.0).exp());
    state.peak = (state.peak - dt * 1.4).max(meters.out_peak.swap(0.0, Ordering::Relaxed));

    let painter = ui.painter().clone();
    let full = ui.max_rect();
    draw_header(ui, &painter, Rect::from_min_size(full.min, Vec2::new(full.width(), HEADER_H)), params, setter);

    let col_w = (full.width() - 3.0 * GAP) / 4.0;
    let top = full.top() + HEADER_H + 8.0;
    for h in 0..HEADS {
        let x = full.left() + h as f32 * (col_w + GAP);
        let head_rect = Rect::from_min_size(Pos2::new(x, top), Vec2::splat(col_w));
        draw_head(ui, &painter, head_rect, h, params, meters, setter, state);

        let label = Rect::from_min_size(Pos2::new(x, head_rect.bottom()), Vec2::new(col_w, LABEL_H));
        let p = &params.heads[h];
        painter.text(label.left_top() + Vec2::new(2.0, 4.0), Align2::LEFT_TOP, NAMES[h], FontId::monospace(12.0), rgba(CREAM, 255.0));
        painter.text(label.left_top() + Vec2::new(2.0, 19.0), Align2::LEFT_TOP, format!("MIDI {}", NOTES[h]), FontId::monospace(8.5), TEXT_DIM);
        painter.text(label.right_top() + Vec2::new(-2.0, 2.0), Align2::RIGHT_TOP, p.tune.to_string(), FontId::monospace(18.0), rgba(CORAL, 255.0));

        let cw = col_w / 5.0;
        let cell = |col: usize, row: usize| Rect::from_min_size(Pos2::new(x + col as f32 * cw, label.bottom() + row as f32 * KNOB_H), Vec2::new(cw, KNOB_H));
        knob(ui, &painter, cell(0, 0), setter, &p.tune, "TUNE", CORAL, false);
        knob(ui, &painter, cell(1, 0), setter, &p.decay, "DECAY", CORAL, false);
        knob(ui, &painter, cell(2, 0), setter, &p.hard, "MALLET", CORAL, false);
        knob(ui, &painter, cell(3, 0), setter, &p.bend, "BEND", CORAL, false);
        knob(ui, &painter, cell(4, 0), setter, &p.click, "STICK", CORAL, false);
        knob(ui, &painter, cell(0, 1), setter, &p.diameter, "DIAM", YELLOW, false);
        knob(ui, &painter, cell(1, 1), setter, &p.length, "LENGTH", YELLOW, false);
        knob(ui, &painter, cell(2, 1), setter, &p.thickness, "WALL", YELLOW, false);
        knob(ui, &painter, cell(3, 1), setter, &p.level, "LEVEL", CREAM, false);
        knob(ui, &painter, cell(4, 1), setter, &p.pan, "PAN", CREAM, true);
        let row = Rect::from_min_size(Pos2::new(x, label.bottom() + 2.0 * KNOB_H + 2.0), Vec2::new(col_w, CYCLE_H - 6.0));
        let half = (col_w - 6.0) / 2.0;
        cycler(ui, &painter, Rect::from_min_size(row.min, Vec2::new(half, row.height())), setter, &p.striker, "STRIKER", CORAL);
        cycler(ui, &painter, Rect::from_min_size(row.min + Vec2::new(half + 6.0, 0.0), Vec2::new(half, row.height())), setter, &p.shell_kind, "SHELL", YELLOW);
    }

    let plates_top = top + col_w + LABEL_H + 2.0 * KNOB_H + CYCLE_H + 10.0;
    for m in 0..METALS {
        let x = full.left() + m as f32 * (col_w + GAP);
        draw_plate(ui, &painter, Rect::from_min_size(Pos2::new(x, plates_top), Vec2::new(col_w, PLATE_H)), m, params, meters, setter, state);
    }

    let strip_top = plates_top + PLATE_H + 8.0;
    draw_strip(ui, &painter, Rect::from_min_size(Pos2::new(full.left(), strip_top), Vec2::new(full.width(), STRIP_H)), params, setter, state);

    painter.text(
        Pos2::new(full.left(), full.bottom() - 6.0),
        Align2::LEFT_CENTER,
        "CLICK A HEAD OR PLATE TO STRIKE IT   DRAG TO MOVE THE MALLET   SHIFT: SOFT   CLOSED HAT CHOKES OPEN HAT   GENERAL MIDI IN",
        FontId::monospace(8.5),
        TEXT_DIM,
    );
}

fn draw_header(ui: &mut Ui, painter: &Painter, rect: Rect, params: &Arc<TympanParams>, setter: &ParamSetter) {
    painter.text(rect.left_center() + Vec2::new(0.0, -5.0), Align2::LEFT_CENTER, "TYMPAN", FontId::monospace(22.0), rgba(CREAM, 255.0));
    painter.text(rect.left_center() + Vec2::new(2.0, 14.0), Align2::LEFT_CENTER, &format!("PHYSICALLY MODELLED DRUMS   BUILD {}", env!("TYMPAN_BUILD")), FontId::monospace(8.5), TEXT_DIM);
    let theme = preset_kit::ui::Theme { fill: PANEL, hover: GRID, ink: rgba(CREAM, 255.0), dim: TEXT_DIM };
    preset_kit::ui::picker(ui, Pos2::new(rect.left() + 230.0, rect.top() + 8.0), 26.0, &presets::BANK, &**params, setter, &theme);
}

#[allow(clippy::too_many_arguments)]
fn draw_head(ui: &mut Ui, painter: &Painter, rect: Rect, h: usize, params: &Arc<TympanParams>, meters: &Meters, setter: &ParamSetter, state: &mut UiState) {
    let c = rect.center();
    let r = rect.width() * 0.5 - 26.0;
    let p = &params.heads[h];
    let k = r / RADIUS;

    // The shell, and a flash around the rim when it is hit.
    painter.circle_filled(c, r + 8.0, PANEL);
    painter.circle_stroke(c, r + 8.0, Stroke::new(1.5_f32, GRID));
    let flash = state.hit[h];
    if flash > 0.01 {
        painter.circle_stroke(c, r + 4.0, Stroke::new(7.0_f32, rgba(CREAM, flash * 30.0)));
        painter.circle_stroke(c, r + 4.0, Stroke::new(2.0_f32, rgba(CREAM, flash * 200.0)));
    }
    painter.circle_stroke(c, r + 4.0, Stroke::new(1.0_f32, rgba(CREAM, 60.0)));

    // The shell's resonances, one tick each around the rim.
    for m in 0..MODES {
        let a = -std::f32::consts::FRAC_PI_2 + (m as f32 + 0.5) * std::f32::consts::TAU / MODES as f32;
        let dir = Vec2::new(a.cos(), a.sin());
        let level = (state.modes[h][m] * 10.0).sqrt().min(1.0);
        let from = c + dir * (r + 12.0);
        let to = c + dir * (r + 14.0 + level * 9.0);
        painter.line_segment([from, to], Stroke::new(6.0_f32, rgba(YELLOW, level * 40.0)));
        painter.line_segment([from, to], Stroke::new(2.5_f32, rgba(YELLOW, 50.0 + level * 205.0)));
    }

    // The skin.
    let base_d = h * 2 * NN;
    let base_v = (h * 2 + 1) * NN;
    for i in 0..NN {
        if !in_head(i) {
            continue;
        }
        let (x, y) = cell_pos(i);
        let pos = c + Vec2::new(x, y) * r;
        let (disp, vel) = (state.env[base_d + i], state.env[base_v + i]);
        let size = (vel * 0.9).powf(0.6).min(1.0);
        if size < 0.02 {
            painter.circle_filled(pos, 1.0, GRID);
            continue;
        }
        // Velocity running ahead of displacement means the cell is shaking faster than the fundamental.
        let over = ((vel / disp.max(1e-4) - 1.0) / 2.2).clamp(0.0, 1.0);
        let col = mix(CORAL, CYAN, over);
        if size > 0.3 {
            painter.circle_filled(pos, 2.0 + size * 6.0, rgba(col, size * 36.0));
        }
        painter.circle_filled(pos, 1.0 + size * 3.6, rgba(col, 70.0 + size * 185.0));
    }

    // The mallet: where it lands, and how much skin it touches.
    let at = Pos2::new(c.x + p.x.value() * r, c.y + p.y.value() * r);
    let reach = (6.0 - 4.7 * p.hard.value()) * k;
    painter.circle_filled(at, reach, rgba(CREAM, 7.0));
    painter.circle_stroke(at, reach, Stroke::new(1.0_f32, rgba(CREAM, 70.0)));

    let area = Rect::from_center_size(c, Vec2::splat((r + 8.0) * 2.0));
    let resp = ui.interact(area, ui.id().with(("head", h)), Sense::click_and_drag());
    let to_disc = |pos: Pos2| {
        let v = (pos - c) / r;
        let len = v.length();
        if len > 0.95 {
            v * (0.95 / len)
        } else {
            v
        }
    };
    let pointer = ui.input(|i| i.pointer.interact_pos());
    let (pressed, down, shift) = ui.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_down(), i.modifiers.shift));
    if resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
    }
    if pressed && resp.hovered() && (pointer.map(|p| (p - c).length() <= r + 8.0).unwrap_or(false)) {
        if let Some(pos) = pointer {
            let v = to_disc(pos);
            setter.begin_set_parameter(&p.x);
            setter.begin_set_parameter(&p.y);
            setter.set_parameter(&p.x, v.x);
            setter.set_parameter(&p.y, v.y);
            state.dragging[h] = true;
            meters.request_hit(h, v.x, v.y, if shift { 0.35 } else { 0.9 });
        }
    } else if state.dragging[h] {
        if down {
            if let Some(pos) = pointer {
                let v = to_disc(pos);
                setter.set_parameter(&p.x, v.x);
                setter.set_parameter(&p.y, v.y);
            }
        } else {
            setter.end_set_parameter(&p.x);
            setter.end_set_parameter(&p.y);
            state.dragging[h] = false;
        }
    }
    let lit = resp.hovered() || state.dragging[h];
    let ink = rgba(CREAM, if lit { 255.0 } else { 190.0 });
    painter.circle_stroke(at, 5.0, Stroke::new(1.5_f32, ink));
    for d in [Vec2::X, Vec2::Y] {
        painter.line_segment([at - d * 9.0, at - d * 6.5], Stroke::new(1.5_f32, ink));
        painter.line_segment([at + d * 6.5, at + d * 9.0], Stroke::new(1.5_f32, ink));
    }
    let _ = meters;
}

#[allow(clippy::too_many_arguments)]
fn draw_plate(ui: &mut Ui, painter: &Painter, rect: Rect, m: usize, params: &Arc<TympanParams>, meters: &Meters, setter: &ParamSetter, state: &UiState) {
    painter.rect_filled(rect, 8.0, PANEL);
    let p = &params.plates[m];
    let c = Pos2::new(rect.left() + 50.0, rect.top() + 50.0);
    let r = 38.0;

    // The plate, with a flash around the edge when it is struck.
    painter.circle_filled(c, r + 4.0, BG);
    let flash = state.plate_hit[m];
    painter.circle_stroke(c, r + 2.0, Stroke::new(6.0_f32, rgba(MINT, flash * 40.0)));
    painter.circle_stroke(c, r + 2.0, Stroke::new(1.5_f32, rgba(MINT, 60.0 + flash * 195.0)));
    painter.circle_stroke(c, r * 0.16, Stroke::new(1.0_f32, rgba(MINT, 50.0)));

    // Modes in a spiral from the bell out to the edge: low at the middle, high at the rim.
    for k in 0..MMODES {
        let a = k as f32 * 2.399_963;
        let rad = ((k as f32 + 0.5) / MMODES as f32).sqrt() * r * 0.92;
        let pos = c + Vec2::new(a.cos(), a.sin()) * rad;
        let level = (state.plate_modes[m][k] * 14.0).sqrt().min(1.0);
        if level < 0.03 {
            painter.circle_filled(pos, 1.0, GRID);
            continue;
        }
        if level > 0.3 {
            painter.circle_filled(pos, 2.0 + level * 4.0, rgba(MINT, level * 36.0));
        }
        painter.circle_filled(pos, 1.0 + level * 2.8, rgba(MINT, 80.0 + level * 175.0));
    }

    let resp = ui.interact(Rect::from_center_size(c, Vec2::splat((r + 4.0) * 2.0)), ui.id().with(("plate", m)), Sense::click());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        let (pressed, shift) = ui.input(|i| (i.pointer.primary_pressed(), i.modifiers.shift));
        if pressed {
            meters.request_plate_hit(m, if shift { 0.35 } else { 0.9 });
        }
    }

    painter.text(Pos2::new(rect.left() + 10.0, rect.top() + 106.0), Align2::LEFT_CENTER, METAL_NAMES[m], FontId::monospace(11.0), rgba(CREAM, 255.0));
    painter.text(Pos2::new(rect.left() + 10.0, rect.top() + 122.0), Align2::LEFT_CENTER, format!("MIDI {}", METAL_NOTES[m]), FontId::monospace(8.5), TEXT_DIM);
    painter.text(Pos2::new(rect.left() + 10.0, rect.top() + 138.0), Align2::LEFT_CENTER, p.diameter.to_string(), FontId::monospace(11.0), rgba(MINT, 255.0));

    let cw = (rect.width() - 100.0) / 2.0;
    let cell = |col: usize, row: usize| Rect::from_min_size(Pos2::new(rect.left() + 100.0 + col as f32 * cw, rect.top() + row as f32 * KNOB_H), Vec2::new(cw, KNOB_H));
    knob(ui, painter, cell(0, 0), setter, &p.diameter, "DIAM", MINT, false);
    knob(ui, painter, cell(1, 0), setter, &p.thickness, "THICK", MINT, false);
    knob(ui, painter, cell(0, 1), setter, &p.tone, "STICK", MINT, false);
    knob(ui, painter, cell(1, 1), setter, &p.level, "LEVEL", CREAM, false);
}

/// A selector that steps through an enum's values each time it is clicked.
fn cycler<T: Enum + PartialEq + 'static>(ui: &mut Ui, painter: &Painter, rect: Rect, setter: &ParamSetter, param: &EnumParam<T>, label: &str, color: [f32; 3]) {
    let r = ui.interact(rect, ui.id().with(("cycle", label, rect.min.x as i32, rect.min.y as i32)), Sense::click());
    if r.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    painter.rect_filled(rect, 5.0, if r.hovered() { PANEL_HI } else { PANEL });
    painter.rect_stroke(rect, 5.0, Stroke::new(1.0_f32, GRID), StrokeKind::Inside);
    painter.text(rect.left_center() + Vec2::new(8.0, 0.0), Align2::LEFT_CENTER, label, FontId::monospace(8.5), TEXT_DIM);
    painter.text(rect.right_center() - Vec2::new(8.0, 0.0), Align2::RIGHT_CENTER, param.to_string().to_uppercase(), FontId::monospace(10.0), rgba(color, 255.0));
    if r.clicked() {
        let next = (param.value().to_index() + 1) % T::variants().len();
        setter.begin_set_parameter(param);
        setter.set_parameter(param, T::from_index(next));
        setter.end_set_parameter(param);
    }
}

fn draw_strip(ui: &mut Ui, painter: &Painter, rect: Rect, params: &Arc<TympanParams>, setter: &ParamSetter, state: &UiState) {
    painter.rect_filled(rect, 8.0, PANEL);
    let cw = 64.0;
    let cell = |i: usize| Rect::from_min_size(Pos2::new(rect.left() + 14.0 + i as f32 * cw, rect.top() + 4.0), Vec2::new(cw, KNOB_H));
    knob(ui, painter, cell(0), setter, &params.skin, "SKIN LOSS", CORAL, false);
    knob(ui, painter, cell(1), setter, &params.wires, "SNARE WIRES", CYAN, false);
    knob(ui, painter, cell(2), setter, &params.level, "LEVEL", CREAM, false);

    // Output meter.
    let mx = rect.left() + 14.0 + 3.0 * cw + 20.0;
    let bar = Rect::from_min_size(Pos2::new(mx, rect.center().y - 4.0), Vec2::new(200.0, 8.0));
    painter.rect_filled(bar, 2.0, GRID);
    let level = state.peak.clamp(0.0, 1.0);
    let hot = Rect::from_min_size(bar.min, Vec2::new(bar.width() * level, bar.height()));
    painter.rect_filled(hot, 2.0, rgba(if level > 0.9 { CORAL } else { CREAM }, 255.0));
    painter.text(bar.left_top() + Vec2::new(0.0, -12.0), Align2::LEFT_CENTER, "OUT", FontId::monospace(8.5), TEXT_DIM);

    // What the colours mean.
    let lx = rect.right() - 390.0;
    for (i, (col, name, note)) in [(CORAL, "SKIN, FUNDAMENTAL", "the head's own pitch"), (CYAN, "SKIN, OVERTONES", "mallet and position"), (YELLOW, "SHELL TICKS", "ring modes of the shell"), (MINT, "PLATE MODES", "bell in, edge out")].iter().enumerate() {
        let (lx, y) = (lx + (i % 2) as f32 * 200.0, rect.top() + 26.0 + (i / 2) as f32 * 34.0);
        painter.circle_filled(Pos2::new(lx, y), 4.0, rgba(*col, 255.0));
        painter.text(Pos2::new(lx + 12.0, y - 5.0), Align2::LEFT_CENTER, *name, FontId::monospace(9.0), rgba(CREAM, 230.0));
        painter.text(Pos2::new(lx + 12.0, y + 6.0), Align2::LEFT_CENTER, *note, FontId::monospace(8.0), TEXT_DIM);
    }
    let _ = StrokeKind::Inside;
    let _ = egui::Id::NULL;
}
