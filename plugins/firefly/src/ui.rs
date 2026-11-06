//! The editor: a lamp, and a photocell that remembers.
//!
//! The display scrolls right to left over about six seconds. The top half hangs the gain reduction
//! from the ceiling like icicles of light: the lime part is the fast stage of the photocell, the
//! magenta part under it is the slow stage, the memory, which is what lingers after the signal has
//! gone. A thin white line is what the lamp is asking for. The glowing strip along the very top is
//! the electro-luminescent panel itself, as bright as the lamp is driven.
//!
//! The bottom half is the input level, and the horizontal handle is the threshold: drag it to set
//! Peak Reduction. Whatever sticks up above it turns lime, and that is what the top half answers.
//!
//! Knobs: drag vertically (hold shift for fine), double-click to reset.

use nih_plug::prelude::*;
use nih_plug_egui::egui::{
    self, Align2, Color32, CursorIcon, FontId, Painter, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2,
};
use std::collections::VecDeque;
use std::f32::consts::PI;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::dsp::{threshold_db, Meter, Mode, METER_FLOOR_DB};
use crate::presets;
use crate::FireflyParams;

const BG: Color32 = Color32::from_rgb(0x0a, 0x0c, 0x0a);
const PANEL: Color32 = Color32::from_rgb(0x12, 0x16, 0x13);
const GRID: Color32 = Color32::from_rgb(0x25, 0x2c, 0x26);
const TEXT_DIM: Color32 = Color32::from_rgb(0x7c, 0x88, 0x7d);
const INK: [f32; 3] = [236.0, 240.0, 232.0];
const LIME: [f32; 3] = [184.0, 255.0, 46.0];
const MAGENTA: [f32; 3] = [255.0, 61.0, 139.0];

/// The top half shows this much reduction, the bottom half this level range.
const GR_RANGE: f32 = 24.0;
const DB_TOP: f32 = 0.0;
const DB_BOT: f32 = -60.0;
const HISTORY: usize = 360;
const STEP: f32 = 1.0 / 60.0;
const KNOB_ROW: f32 = 100.0;
const GRAB: f32 = 9.0;

#[derive(Clone, Copy, Default)]
struct Column {
    fast: f32,
    slow: f32,
    lamp: f32,
    input: f32,
}

pub struct UiState {
    history: VecDeque<Column>,
    clock: f32,
}

impl Default for UiState {
    fn default() -> Self {
        Self { history: VecDeque::from(vec![Column { input: METER_FLOOR_DB, ..Column::default() }; HISTORY]), clock: 0.0 }
    }
}

pub fn background() -> Color32 {
    BG
}

fn rgba(c: [f32; 3], a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c[0] as u8, c[1] as u8, c[2] as u8, a.clamp(0.0, 255.0) as u8)
}

fn sample(meter: &Meter, state: &mut UiState, dt: f32) -> Column {
    let now = Column {
        fast: meter.fast.load(Ordering::Relaxed),
        slow: meter.slow.load(Ordering::Relaxed),
        lamp: meter.lamp.load(Ordering::Relaxed),
        input: meter.input_db.load(Ordering::Relaxed),
    };
    state.clock += dt;
    let mut pushed = 0;
    while state.clock >= STEP && pushed < 6 {
        state.clock -= STEP;
        state.history.pop_front();
        state.history.push_back(now);
        pushed += 1;
    }
    state.clock = state.clock.min(STEP);
    now
}

pub fn draw(ui: &mut Ui, params: &Arc<FireflyParams>, setter: &ParamSetter, meter: &Arc<Meter>, state: &mut UiState) {
    ui.ctx().request_repaint();
    let dt = ui.input(|i| i.stable_dt).min(0.1);
    let now = sample(meter, state, dt);

    header(ui, params, setter, now);
    ui.add_space(8.0);
    let height = ui.available_height() - KNOB_ROW - 8.0;
    display(ui, height, params, setter, state, now);
    ui.add_space(8.0);
    knobs(ui, params, setter);
}

fn header(ui: &mut Ui, params: &FireflyParams, setter: &ParamSetter, now: Column) {
    let theme = preset_kit::ui::Theme { fill: PANEL, hover: GRID, ink: rgba(INK, 255.0), dim: TEXT_DIM };
    preset_kit::ui::picker_top_right(ui, 24.0, &presets::BANK, params, setter, &theme);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("FIREFLY").monospace().size(11.0).color(TEXT_DIM));
        ui.label(egui::RichText::new("opto leveller").monospace().size(9.0).color(GRID));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(preset_kit::ui::WIDTH + 14.0);
            let gr = now.fast + now.slow;
            ui.label(egui::RichText::new(format!("{:>5.1} dB", -gr)).monospace().size(18.0).color(rgba(LIME, 255.0)));
            ui.add_space(10.0);
            ui.label(egui::RichText::new("GR").monospace().size(9.0).color(TEXT_DIM));
        });
    });
}

/// A drag handle: edits `param` to whatever `target` makes of the pointer, double-click resets.
fn edit(r: &egui::Response, setter: &ParamSetter, param: &FloatParam, target: impl FnOnce(Pos2) -> f32) {
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

fn display(ui: &mut Ui, height: f32, params: &FireflyParams, setter: &ParamSetter, state: &UiState, now: Column) {
    let size = Vec2::new(ui.available_width(), height);
    let (response, painter) = ui.allocate_painter(size, Sense::hover());
    let rect = response.rect;
    painter.rect_filled(rect, 6.0, PANEL);

    let plot = Rect::from_min_max(Pos2::new(rect.left() + 36.0, rect.top() + 10.0), Pos2::new(rect.right() - 74.0, rect.bottom() - 10.0));
    let split = plot.top() + plot.height() * 0.54;
    let gr_h = split - plot.top() - 6.0;
    let gr_y = |db: f32| plot.top() + db.clamp(0.0, GR_RANGE) / GR_RANGE * gr_h;
    let lvl_top = split + 6.0;
    let lvl_y = |db: f32| plot.bottom() - (db.clamp(DB_BOT, DB_TOP) - DB_BOT) / (DB_TOP - DB_BOT) * (plot.bottom() - lvl_top);
    let db_of = |y: f32| DB_BOT + (plot.bottom() - y) / (plot.bottom() - lvl_top) * (DB_TOP - DB_BOT);
    let col_w = plot.width() / HISTORY as f32;
    let x_of = |i: usize| plot.left() + i as f32 * col_w;

    let (mode, pr) = (params.mode.value(), params.peak_reduction.value());
    let thr = threshold_db(pr);
    let thr_y = lvl_y(thr);

    // The threshold handle.
    let strip = Rect::from_min_max(Pos2::new(plot.left(), thr_y - GRAB), Pos2::new(rect.right() - 4.0, thr_y + GRAB));
    let thr_r = ui.interact(strip, ui.id().with("thr"), Sense::click_and_drag());
    edit(&thr_r, setter, &params.peak_reduction, |p| ((threshold_db(0.0) - db_of(p.y)) / (threshold_db(0.0) - threshold_db(1.0))).clamp(0.0, 1.0));
    let lit = thr_r.hovered() || thr_r.dragged();
    if lit {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeVertical);
    }

    // Grid and labels.
    for db in [0.0, 6.0, 12.0, 18.0, 24.0] {
        let y = gr_y(db);
        painter.line_segment([Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)], Stroke::new(1.0_f32, rgba(INK, 9.0)));
        painter.text(Pos2::new(plot.left() - 6.0, y), Align2::RIGHT_CENTER, format!("-{db:.0}"), FontId::monospace(8.0), TEXT_DIM);
    }
    for db in [0, -12, -24, -36, -48] {
        let y = lvl_y(db as f32);
        painter.line_segment([Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)], Stroke::new(1.0_f32, rgba(INK, 9.0)));
        painter.text(Pos2::new(plot.left() - 6.0, y), Align2::RIGHT_CENTER, format!("{db}"), FontId::monospace(8.0), TEXT_DIM);
    }
    painter.line_segment([Pos2::new(plot.left(), split), Pos2::new(plot.right(), split)], Stroke::new(1.0_f32, GRID));

    // Input level, and what pokes above the threshold in lime. The knee is a faint band around the line.
    let (_, knee) = mode.curve();
    let knee_band = Rect::from_min_max(Pos2::new(plot.left(), lvl_y(thr + knee * 0.5)), Pos2::new(plot.right(), lvl_y(thr - knee * 0.5)));
    painter.rect_filled(knee_band, 0.0, rgba(LIME, 10.0));
    for (i, c) in state.history.iter().enumerate() {
        let (x0, x1) = (x_of(i), x_of(i) + col_w + 0.5);
        let y = lvl_y(c.input);
        painter.rect_filled(Rect::from_min_max(Pos2::new(x0, y.max(thr_y)), Pos2::new(x1, plot.bottom())), 0.0, rgba(INK, 44.0));
        if y < thr_y {
            painter.rect_filled(Rect::from_min_max(Pos2::new(x0, y), Pos2::new(x1, thr_y)), 0.0, rgba(LIME, 150.0));
        }
    }

    // The photocell: gain reduction hanging from the top, fast stage then memory.
    let mut total = Vec::with_capacity(HISTORY);
    let mut lamp = Vec::with_capacity(HISTORY);
    for (i, c) in state.history.iter().enumerate() {
        let (x0, x1) = (x_of(i), x_of(i) + col_w + 0.5);
        let (yf, yt) = (gr_y(c.fast), gr_y(c.fast + c.slow));
        painter.rect_filled(Rect::from_min_max(Pos2::new(x0, plot.top()), Pos2::new(x1, yf)), 0.0, rgba(LIME, 90.0));
        painter.rect_filled(Rect::from_min_max(Pos2::new(x0, yf), Pos2::new(x1, yt)), 0.0, rgba(MAGENTA, 140.0));
        total.push(Pos2::new(x0 + col_w * 0.5, yt));
        lamp.push(Pos2::new(x0 + col_w * 0.5, gr_y(c.lamp)));
    }
    painter.add(Shape::line(total.clone(), Stroke::new(5.0_f32, rgba(MAGENTA, 34.0))));
    painter.add(Shape::line(total, Stroke::new(1.5_f32, rgba(MAGENTA, 255.0))));
    painter.add(Shape::line(lamp, Stroke::new(1.0_f32, rgba(INK, 150.0))));

    // The electro-luminescent panel: a strip whose glow follows the lamp.
    let glow = (now.lamp / 12.0).clamp(0.0, 1.0);
    let panel = Rect::from_min_max(Pos2::new(plot.left(), plot.top() - 4.0), Pos2::new(plot.right(), plot.top() + 10.0));
    let mut mesh = egui::Mesh::default();
    let (bright, clear) = (rgba(LIME, 40.0 + 200.0 * glow), rgba(LIME, 0.0));
    mesh.colored_vertex(panel.left_top(), bright);
    mesh.colored_vertex(panel.right_top(), bright);
    mesh.colored_vertex(panel.left_bottom(), clear);
    mesh.colored_vertex(panel.right_bottom(), clear);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 2, 3);
    painter.add(Shape::mesh(mesh));
    painter.line_segment([panel.left_top() + Vec2::new(0.0, 4.0), panel.right_top() + Vec2::new(0.0, 4.0)], Stroke::new(2.0_f32, rgba(LIME, 70.0 + 185.0 * glow)));

    // The threshold line, its pill and its label.
    painter.line_segment([Pos2::new(plot.left(), thr_y), Pos2::new(plot.right(), thr_y)], Stroke::new(if lit { 6.0_f32 } else { 4.0 }, rgba(LIME, if lit { 60.0 } else { 30.0 })));
    painter.line_segment([Pos2::new(plot.left(), thr_y), Pos2::new(plot.right(), thr_y)], Stroke::new(1.5_f32, rgba(LIME, 255.0)));
    let pill = Rect::from_center_size(Pos2::new(plot.right() + 14.0, thr_y), Vec2::new(40.0, 16.0));
    painter.rect_filled(pill.expand(3.0), 6.0, rgba(LIME, if lit { 60.0 } else { 26.0 }));
    painter.rect_filled(pill, 4.0, rgba(LIME, if lit { 255.0 } else { 225.0 }));
    painter.text(pill.center(), Align2::CENTER_CENTER, format!("{thr:.0}"), FontId::monospace(10.0), BG);
    painter.text(Pos2::new(pill.center().x, pill.bottom() + 6.0), Align2::CENTER_TOP, "THRESH", FontId::monospace(7.0), TEXT_DIM);

    // Legend.
    let mut x = plot.left() + 8.0;
    for (color, text) in [(LIME, "FAST"), (MAGENTA, "MEMORY"), (INK, "LAMP")] {
        painter.circle_filled(Pos2::new(x, split - 9.0), 3.0, rgba(color, 220.0));
        let galley = painter.text(Pos2::new(x + 8.0, split - 9.0), Align2::LEFT_CENTER, text, FontId::monospace(8.0), TEXT_DIM);
        x += 20.0 + galley.width();
    }
    painter.text(Pos2::new(plot.right(), plot.bottom() - 2.0), Align2::RIGHT_BOTTOM, "6 s  >", FontId::monospace(8.0), TEXT_DIM);
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

fn knobs(ui: &mut Ui, params: &FireflyParams, setter: &ParamSetter) {
    let (response, painter) = ui.allocate_painter(Vec2::new(ui.available_width(), KNOB_ROW), Sense::hover());
    let row = response.rect;
    let width = row.width() / 7.0;
    let cell = |i: usize| Rect::from_min_size(Pos2::new(row.left() + i as f32 * width, row.top()), Vec2::new(width, KNOB_ROW));
    knob(ui, &painter, cell(0), setter, &params.peak_reduction, "PEAK RED", LIME);
    knob(ui, &painter, cell(1), setter, &params.gain, "GAIN", INK);
    mode_switch(ui, &painter, cell(2), setter, &params.mode);
    knob(ui, &painter, cell(3), setter, &params.emphasis, "EMPHASIS", INK);
    knob(ui, &painter, cell(4), setter, &params.tube, "TUBE", INK);
    knob(ui, &painter, cell(5), setter, &params.link, "LINK", INK);
    knob(ui, &painter, cell(6), setter, &params.mix, "MIX", INK);
}

/// A two-position lever: click anywhere on it to flip.
fn mode_switch(ui: &mut Ui, painter: &Painter, cell: Rect, setter: &ParamSetter, param: &EnumParam<Mode>) {
    let r = ui.interact(cell, ui.id().with("mode"), Sense::click());
    let mode = param.value();
    if r.clicked() {
        setter.begin_set_parameter(param);
        setter.set_parameter(param, if mode == Mode::Compress { Mode::Limit } else { Mode::Compress });
        setter.end_set_parameter(param);
    }
    let lit = r.hovered();
    let center = Pos2::new(cell.center().x, cell.top() + 34.0);
    let slot = Rect::from_center_size(center, Vec2::new(18.0, 46.0));
    painter.rect_filled(slot, 9.0, if lit { PANEL } else { Color32::from_rgb(0x0f, 0x12, 0x10) });
    painter.rect_stroke(slot, 9.0, Stroke::new(2.0_f32, GRID), egui::StrokeKind::Inside);
    let up = mode == Mode::Limit;
    let thumb = Pos2::new(center.x, if up { slot.top() + 11.0 } else { slot.bottom() - 11.0 });
    let color = if up { MAGENTA } else { LIME };
    painter.circle_filled(thumb, 11.0, rgba(color, 40.0));
    painter.circle_filled(thumb, 8.0, rgba(color, 255.0));
    painter.text(Pos2::new(center.x + 24.0, slot.top() + 11.0), Align2::LEFT_CENTER, "LIMIT", FontId::monospace(8.0), if up { rgba(MAGENTA, 255.0) } else { TEXT_DIM });
    painter.text(Pos2::new(center.x + 24.0, slot.bottom() - 11.0), Align2::LEFT_CENTER, "COMP", FontId::monospace(8.0), if up { TEXT_DIM } else { rgba(LIME, 255.0) });
    painter.text(Pos2::new(cell.center().x, cell.top() + 67.0), Align2::CENTER_CENTER, "MODE", FontId::monospace(9.0), TEXT_DIM);
    painter.text(Pos2::new(cell.center().x, cell.top() + 83.0), Align2::CENTER_CENTER, if up { "LIMIT" } else { "COMPRESS" }, FontId::monospace(10.0), rgba(color, 255.0));
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

    painter.circle_filled(center, radius - 5.0, if lit { PANEL } else { Color32::from_rgb(0x0f, 0x12, 0x10) });
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
