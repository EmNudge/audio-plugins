//! The editor: a vice closing on the signal.
//!
//! The graph is three columns, low / mid / high, laid out on a log frequency axis, each a live level
//! bar from -72 to 0 dB. The solid part is what the band does to the level, the orange block is
//! what the ceiling jaw took off the top, the cyan block is what the floor jaw added at the bottom.
//! The jaws are the two horizontal handles and their teeth grow with how hard they are working;
//! the two vertical dividers are the crossovers. Drag any of them, double-click to reset.
//!
//! Knobs: drag vertically (hold shift for fine), double-click to reset.

use nih_plug::prelude::*;
use nih_plug_egui::egui::{
    self, Align2, Color32, CursorIcon, FontId, Painter, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, Vec2,
};
use std::f32::consts::PI;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::dsp::{Meter, BANDS, MIN_WINDOW_DB};
use crate::presets;
use crate::ViceParams;

const BG: Color32 = Color32::from_rgb(0x0b, 0x0d, 0x11);
const PANEL: Color32 = Color32::from_rgb(0x14, 0x18, 0x1f);
const GRID: Color32 = Color32::from_rgb(0x25, 0x2b, 0x36);
const TEXT_DIM: Color32 = Color32::from_rgb(0x78, 0x84, 0x95);
const INK: [f32; 3] = [236.0, 238.0, 240.0];
const ORANGE: [f32; 3] = [255.0, 98.0, 24.0];
const CYAN: [f32; 3] = [34.0, 214.0, 255.0];

const DB_TOP: f32 = 0.0;
const DB_BOT: f32 = -72.0;
const F_MIN: f32 = 20.0;
const F_MAX: f32 = 20000.0;
const KNOB_ROW: f32 = 100.0;
const GRAB: f32 = 8.0;
const TOOTH: f32 = 14.0;
const NAMES: [&str; BANDS] = ["LOW", "MID", "HIGH"];

pub struct UiState {
    /// Output peak ticks per band, in dB, with the time left to hold each.
    tick: [f32; BANDS],
    hold: [f32; BANDS],
}

impl Default for UiState {
    fn default() -> Self {
        Self { tick: [DB_BOT; BANDS], hold: [0.0; BANDS] }
    }
}

pub fn background() -> Color32 {
    BG
}

fn rgba(c: [f32; 3], a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c[0] as u8, c[1] as u8, c[2] as u8, a.clamp(0.0, 255.0) as u8)
}

struct Snapshot {
    level_in: [f32; BANDS],
    level_out: [f32; BANDS],
    gain: [f32; BANDS],
}

fn snapshot(meter: &Meter, state: &mut UiState, dt: f32) -> Snapshot {
    let s = Snapshot {
        level_in: std::array::from_fn(|b| meter.level_in[b].load(Ordering::Relaxed)),
        level_out: std::array::from_fn(|b| meter.level_out[b].load(Ordering::Relaxed)),
        gain: std::array::from_fn(|b| meter.gain[b].load(Ordering::Relaxed)),
    };
    // Rise fast, fall slowly: the tick holds a moment, then drifts down.
    for b in 0..BANDS {
        if s.level_out[b] >= state.tick[b] {
            state.tick[b] = s.level_out[b];
            state.hold[b] = 0.8;
        } else if state.hold[b] > 0.0 {
            state.hold[b] -= dt;
        } else {
            state.tick[b] = (state.tick[b] - 14.0 * dt).max(s.level_out[b]);
        }
    }
    s
}

pub fn draw(ui: &mut Ui, params: &Arc<ViceParams>, setter: &ParamSetter, meter: &Arc<Meter>, state: &mut UiState) {
    ui.ctx().request_repaint();
    let dt = ui.input(|i| i.stable_dt).min(0.1);
    let snap = snapshot(meter, state, dt);

    header(ui, params, setter);
    ui.add_space(8.0);
    let height = ui.available_height() - KNOB_ROW - 8.0;
    graph(ui, height, params, setter, &snap, state);
    ui.add_space(8.0);
    knobs(ui, params, setter);
}

fn header(ui: &mut Ui, params: &ViceParams, setter: &ParamSetter) {
    let theme = preset_kit::ui::Theme { fill: PANEL, hover: GRID, ink: rgba(INK, 255.0), dim: TEXT_DIM };
    preset_kit::ui::picker_top_right(ui, 24.0, &presets::BANK, params, setter, &theme);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("VICE").monospace().size(11.0).color(TEXT_DIM));
        ui.label(egui::RichText::new("three-band squeeze").monospace().size(9.0).color(GRID));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(preset_kit::ui::WIDTH + 14.0);
            let floor = params.floor.value().min(params.ceil.value() - MIN_WINDOW_DB);
            for (text, color) in [(format!("{:>4.0}dB", floor), CYAN), (format!("{:>4.0}dB", params.ceil.value()), ORANGE)] {
                ui.label(egui::RichText::new(text).monospace().size(18.0).color(rgba(color, 255.0)));
                ui.add_space(10.0);
            }
        });
    });
}

fn hz(f: f32) -> String {
    if f >= 1000.0 {
        format!("{:.1}k", f / 1000.0)
    } else {
        format!("{f:.0}")
    }
}

/// A drag handle: edits `param` to whatever `target` makes of the pointer, double-click resets.
fn edit(r: &Response, setter: &ParamSetter, param: &FloatParam, target: impl FnOnce(Pos2) -> f32) {
    if r.drag_started() {
        setter.begin_set_parameter(param);
    }
    if r.dragged() {
        if let Some(pos) = r.interact_pointer_pos() {
            setter.set_parameter(param, target(pos));
        }
    }
    if r.drag_stopped() {
        setter.end_set_parameter(param);
    }
    if r.double_clicked() {
        setter.begin_set_parameter(param);
        setter.set_parameter(param, param.default_plain_value());
        setter.end_set_parameter(param);
    }
}

fn graph(ui: &mut Ui, height: f32, params: &ViceParams, setter: &ParamSetter, snap: &Snapshot, state: &UiState) {
    let size = Vec2::new(ui.available_width(), height);
    let (response, painter) = ui.allocate_painter(size, Sense::hover());
    let rect = response.rect;
    painter.rect_filled(rect, 6.0, PANEL);

    let plot = Rect::from_min_max(Pos2::new(rect.left() + 38.0, rect.top() + 12.0), Pos2::new(rect.right() - 16.0, rect.bottom() - 34.0));
    let y_of = |db: f32| plot.bottom() - (db.clamp(DB_BOT, DB_TOP) - DB_BOT) / (DB_TOP - DB_BOT) * plot.height();
    let db_of = |y: f32| DB_BOT + (plot.bottom() - y) / plot.height() * (DB_TOP - DB_BOT);
    let span = (F_MAX / F_MIN).ln();
    let x_of = |f: f32| plot.left() + (f / F_MIN).ln() / span * plot.width();
    let freq_of = |x: f32| F_MIN * ((x - plot.left()) / plot.width() * span).exp();

    let (low_hz, high_hz) = (params.cross_low.value(), params.cross_high.value());
    let (ceil, floor_set) = (params.ceil.value(), params.floor.value());
    let floor = floor_set.min(ceil - MIN_WINDOW_DB);
    let (ceil_y, floor_y) = (y_of(ceil), y_of(floor));
    let edges = [x_of(F_MIN), x_of(low_hz), x_of(high_hz), x_of(F_MAX)];

    // Interaction first, so the drawing below reads the freshest state.
    let strip = |y: f32| Rect::from_min_max(Pos2::new(plot.left(), y - GRAB), Pos2::new(plot.right(), y + GRAB));
    let column = |x: f32| Rect::from_min_max(Pos2::new(x - GRAB, plot.top()), Pos2::new(x + GRAB, plot.bottom() + 22.0));
    let ceil_r = ui.interact(strip(ceil_y), ui.id().with("ceil"), Sense::click_and_drag());
    let floor_r = ui.interact(strip(floor_y), ui.id().with("floor"), Sense::click_and_drag());
    let low_r = ui.interact(column(edges[1]), ui.id().with("x_low"), Sense::click_and_drag());
    let high_r = ui.interact(column(edges[2]), ui.id().with("x_high"), Sense::click_and_drag());
    edit(&ceil_r, setter, &params.ceil, |p| db_of(p.y).clamp(-40.0, 0.0).max(floor + MIN_WINDOW_DB));
    edit(&floor_r, setter, &params.floor, |p| db_of(p.y).clamp(-60.0, -10.0).min(ceil - MIN_WINDOW_DB));
    edit(&low_r, setter, &params.cross_low, |p| freq_of(p.x).clamp(40.0, 400.0));
    edit(&high_r, setter, &params.cross_high, |p| freq_of(p.x).clamp(1000.0, 10000.0));
    for (r, icon) in [(&ceil_r, CursorIcon::ResizeVertical), (&floor_r, CursorIcon::ResizeVertical), (&low_r, CursorIcon::ResizeHorizontal), (&high_r, CursorIcon::ResizeHorizontal)] {
        if r.hovered() || r.dragged() {
            ui.ctx().set_cursor_icon(icon);
        }
    }

    // Level grid.
    for db in (-72..=0).step_by(12) {
        let y = y_of(db as f32);
        painter.line_segment([Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)], Stroke::new(1.0_f32, GRID));
        painter.text(Pos2::new(plot.left() - 6.0, y), Align2::RIGHT_CENTER, format!("{db}"), FontId::monospace(8.0), TEXT_DIM);
    }

    // Bands.
    for b in 0..BANDS {
        let (xa, xb) = (edges[b] + 3.0, edges[b + 1] - 3.0);
        painter.rect_filled(Rect::from_min_max(Pos2::new(xa, plot.top()), Pos2::new(xb, plot.bottom())), 3.0, rgba(INK, 6.0));
        let (y_in, y_out) = (y_of(snap.level_in[b]), y_of(snap.level_out[b]));
        // The part of the bar both agree on, then the difference.
        let common = y_in.max(y_out);
        painter.rect_filled(Rect::from_min_max(Pos2::new(xa, common), Pos2::new(xb, plot.bottom())), 0.0, rgba(INK, 62.0));
        if y_out > y_in {
            // Level went down: orange is what the ceiling took.
            painter.rect_filled(Rect::from_min_max(Pos2::new(xa, y_in), Pos2::new(xb, y_out)), 0.0, rgba(ORANGE, 150.0));
        } else if y_out < y_in {
            painter.rect_filled(Rect::from_min_max(Pos2::new(xa, y_out), Pos2::new(xb, y_in)), 0.0, rgba(CYAN, 150.0));
        }
        // Output edge with a glow, the input as a dim tick, the held peak as a thin one.
        let mid = (xa + xb) * 0.5;
        painter.line_segment([Pos2::new(xa, y_out), Pos2::new(xb, y_out)], Stroke::new(5.0_f32, rgba(INK, 26.0)));
        painter.line_segment([Pos2::new(xa, y_out), Pos2::new(xb, y_out)], Stroke::new(2.0_f32, rgba(INK, 235.0)));
        painter.line_segment([Pos2::new(xa, y_in), Pos2::new(xb, y_in)], Stroke::new(1.0_f32, rgba(INK, 70.0)));
        let ty = y_of(state.tick[b]);
        painter.line_segment([Pos2::new(mid - 8.0, ty), Pos2::new(mid + 8.0, ty)], Stroke::new(1.5_f32, rgba(INK, 140.0)));

        let gain = snap.gain[b];
        let color = if gain < -0.5 { ORANGE } else if gain > 0.5 { CYAN } else { [120.0, 132.0, 149.0] };
        painter.text(Pos2::new(mid, plot.top() + 6.0), Align2::CENTER_TOP, NAMES[b], FontId::monospace(9.0), TEXT_DIM);
        painter.text(Pos2::new(mid, plot.top() + 20.0), Align2::CENTER_TOP, format!("{gain:+.1}"), FontId::monospace(12.0), rgba(color, 255.0));
    }

    // The jaws. Teeth grow with the strongest push on that side, so they chew in time with the audio.
    let down = snap.gain.iter().fold(0.0_f32, |m, g| m.min(*g));
    let up = snap.gain.iter().fold(0.0_f32, |m, g| m.max(*g));
    jaw(&painter, plot, ceil_y, ceil_r.hovered() || ceil_r.dragged(), 1.0, 3.0 + (-down).min(18.0) * 0.7, ORANGE);
    jaw(&painter, plot, floor_y, floor_r.hovered() || floor_r.dragged(), -1.0, 3.0 + up.min(18.0) * 0.7, CYAN);

    // Crossover dividers with their frequency flags.
    for (i, (r, f)) in [(&low_r, low_hz), (&high_r, high_hz)].into_iter().enumerate() {
        let x = edges[i + 1];
        let lit = r.hovered() || r.dragged();
        painter.line_segment([Pos2::new(x, plot.top()), Pos2::new(x, plot.bottom() + 6.0)], Stroke::new(if lit { 2.0_f32 } else { 1.0 }, rgba(INK, if lit { 200.0 } else { 70.0 })));
        let flag = Rect::from_center_size(Pos2::new(x, plot.bottom() + 15.0), Vec2::new(40.0, 14.0));
        painter.rect_filled(flag, 3.0, rgba(INK, if lit { 230.0 } else { 40.0 }));
        painter.text(flag.center(), Align2::CENTER_CENTER, hz(f), FontId::monospace(9.0), if lit { BG } else { rgba(INK, 220.0) });
    }
    for f in [100.0, 1000.0, 10000.0] {
        painter.text(Pos2::new(x_of(f), rect.bottom() - 2.0), Align2::CENTER_BOTTOM, hz(f), FontId::monospace(8.0), GRID);
    }
    painter.text(Pos2::new(plot.right(), rect.top() + 2.0), Align2::RIGHT_TOP, "drag the jaws and dividers", FontId::monospace(8.0), TEXT_DIM);
}

/// One jaw: a dim body, a glowing edge and a row of teeth. `dir` is 1 for the ceiling (body above,
/// teeth hanging down) and -1 for the floor.
fn jaw(painter: &Painter, plot: Rect, y: f32, lit: bool, dir: f32, tooth: f32, color: [f32; 3]) {
    let edge = if dir > 0.0 { plot.top() } else { plot.bottom() };
    let body = Rect::from_two_pos(Pos2::new(plot.left(), edge), Pos2::new(plot.right(), y));
    painter.rect_filled(body, 0.0, rgba(color, if lit { 26.0 } else { 14.0 }));
    let n = (plot.width() / TOOTH) as usize;
    let w = plot.width() / n as f32;
    for k in 0..n {
        let x = plot.left() + k as f32 * w;
        painter.add(Shape::convex_polygon(
            vec![Pos2::new(x + 1.0, y), Pos2::new(x + w - 1.0, y), Pos2::new(x + w * 0.5, y + dir * tooth)],
            rgba(color, if lit { 235.0 } else { 190.0 }),
            Stroke::NONE,
        ));
    }
    painter.line_segment([Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)], Stroke::new(5.0_f32, rgba(color, if lit { 60.0 } else { 30.0 })));
    painter.line_segment([Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)], Stroke::new(1.5_f32, rgba(color, 255.0)));
    let pill = Rect::from_center_size(Pos2::new(plot.right() - 10.0, y), Vec2::new(12.0, 14.0));
    painter.rect_filled(pill.expand(3.0), 5.0, rgba(color, if lit { 60.0 } else { 26.0 }));
    painter.rect_filled(pill, 3.0, rgba(color, if lit { 255.0 } else { 225.0 }));
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

fn knobs(ui: &mut Ui, params: &ViceParams, setter: &ParamSetter) {
    let (response, painter) = ui.allocate_painter(Vec2::new(ui.available_width(), KNOB_ROW), Sense::hover());
    let row = response.rect;
    let cells: [(&FloatParam, &str, [f32; 3]); 10] = [
        (&params.input, "INPUT", INK),
        (&params.depth, "DEPTH", INK),
        (&params.upward, "UP", CYAN),
        (&params.downward, "DOWN", ORANGE),
        (&params.time, "TIME", INK),
        (&params.gain_low, "LOW", INK),
        (&params.gain_mid, "MID", INK),
        (&params.gain_high, "HIGH", INK),
        (&params.mix, "MIX", INK),
        (&params.output, "OUTPUT", INK),
    ];
    let width = row.width() / cells.len() as f32;
    for (i, (param, label, color)) in cells.iter().enumerate() {
        let cell = Rect::from_min_size(Pos2::new(row.left() + i as f32 * width, row.top()), Vec2::new(width, KNOB_ROW));
        knob(ui, &painter, cell, setter, param, label, *color);
    }
}

fn knob(ui: &mut Ui, painter: &Painter, cell: Rect, setter: &ParamSetter, param: &FloatParam, label: &str, color: [f32; 3]) {
    let r = ui.interact(cell, ui.id().with(label), Sense::click_and_drag());
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

    let norm = param.unmodulated_normalized_value();
    let center = Pos2::new(cell.center().x, cell.top() + 34.0);
    let radius = 22.0;
    let (start, end) = (PI * 0.75, PI * 2.25);
    let lit = r.hovered() || r.dragged();
    let main = rgba(color, 255.0);

    painter.circle_filled(center, radius - 5.0, if lit { PANEL } else { Color32::from_rgb(0x10, 0x13, 0x18) });
    painter.add(Shape::line(arc(center, radius, start, end), Stroke::new(3.0_f32, GRID)));
    if norm > 0.002 {
        let to = start + (end - start) * norm;
        painter.add(Shape::line(arc(center, radius, start, to), Stroke::new(8.0_f32, rgba(color, 30.0))));
        painter.add(Shape::line(arc(center, radius, start, to), Stroke::new(3.0_f32, main)));
    }
    let angle = start + (end - start) * norm;
    let dir = Vec2::new(angle.cos(), angle.sin());
    painter.line_segment([center + dir * (radius - 13.0), center + dir * (radius - 6.0)], Stroke::new(2.0_f32, rgba(INK, 255.0)));

    painter.text(Pos2::new(cell.center().x, cell.top() + 67.0), Align2::CENTER_CENTER, label, FontId::monospace(9.0), TEXT_DIM);
    painter.text(Pos2::new(cell.center().x, cell.top() + 83.0), Align2::CENTER_CENTER, param.to_string(), FontId::monospace(10.0), main);
}
