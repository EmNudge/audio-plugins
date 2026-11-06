//! The editor: two tape lanes tied together by threads.
//!
//! The top lane is take 1 (the input), the bottom lane is take 2 (the copy), both scrolling right
//! to left with "now" at the right edge. Every thread runs from a moment in the lead to the moment
//! the copy replays it, so their slant is the delay in the audio thread right now and they
//! visibly wander as the tape drifts. Below, a delay ruler draws the recent delay history of each
//! channel as a plumb line; drag it to set the time. The reels turn with the audio, the copy's
//! reel at the speed its read head is running.
//!
//! Knobs: drag vertically (hold shift for fine), double-click to reset.

use nih_plug::prelude::*;
use nih_plug_egui::egui::{
    self, Align2, Color32, FontId, Painter, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2,
};
use std::f32::consts::{PI, TAU};
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::dsp::{Meter, COLS, COL_MS, MAX_TIME_MS, MIN_TIME_MS};
use crate::presets;
use crate::DoppelParams;

const BG: Color32 = Color32::from_rgb(0x0e, 0x0c, 0x15);
const PANEL: Color32 = Color32::from_rgb(0x18, 0x15, 0x23);
const GRID: Color32 = Color32::from_rgb(0x2a, 0x26, 0x3b);
const TEXT_DIM: Color32 = Color32::from_rgb(0x84, 0x7d, 0x9a);
const TEXT: [f32; 3] = [240.0, 236.0, 228.0];
const LIME: [f32; 3] = [200.0, 255.0, 61.0];
const PINK: [f32; 3] = [255.0, 61.0, 154.0];
const VIOLET: [f32; 3] = [143.0, 107.0, 255.0];

const LANE_H: f32 = 78.0;
const REEL_ZONE: f32 = 64.0;
const RULER_H: f32 = 70.0;
const KNOB_ROW: f32 = 100.0;
/// Reel turns per column of audio: one turn every 120 columns (0.3 s).
const REEL_TURN_PER_COL: f32 = TAU / 120.0;

pub struct UiState {
    last_head: usize,
    reel: [f32; 2],
    /// Peak pitch deviation of the copy, held and released slowly, in cents.
    cents_peak: f32,
}

impl Default for UiState {
    fn default() -> Self {
        Self { last_head: 0, reel: [0.0, 0.0], cents_peak: 0.0 }
    }
}

pub fn background() -> Color32 {
    BG
}

fn rgba(c: [f32; 3], a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c[0] as u8, c[1] as u8, c[2] as u8, a.clamp(0.0, 255.0) as u8)
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// One snapshot of the meter, oldest column first.
struct Columns {
    lead: [f32; COLS],
    double: [f32; COLS],
    delay: [f32; COLS],
    delay_right: [f32; COLS],
    cents: f32,
}

fn snapshot(meter: &Meter, state: &mut UiState) -> Columns {
    let head = meter.head.load(Ordering::Acquire);
    let mut c = Columns { lead: [0.0; COLS], double: [0.0; COLS], delay: [0.0; COLS], delay_right: [0.0; COLS], cents: 0.0 };
    for k in 0..COLS {
        let slot = (head + k) % COLS;
        c.lead[k] = meter.lead[slot].load(Ordering::Relaxed);
        c.double[k] = meter.double[slot].load(Ordering::Relaxed);
        c.delay[k] = meter.delay[slot].load(Ordering::Relaxed);
        c.delay_right[k] = meter.delay_right[slot].load(Ordering::Relaxed);
    }
    c.cents = meter.cents.load(Ordering::Relaxed);

    // Reels turn only as the audio advances.
    let advanced = head.wrapping_sub(state.last_head).min(COLS) as f32;
    state.last_head = head;
    state.reel[0] = (state.reel[0] + advanced * REEL_TURN_PER_COL) % TAU;
    state.reel[1] = (state.reel[1] + advanced * REEL_TURN_PER_COL * (c.cents / 1200.0).exp2()) % TAU;
    // Rise fast, fall slowly.
    state.cents_peak = if advanced > 0.0 { c.cents.abs().max(state.cents_peak * 0.985) } else { state.cents_peak };
    c
}

pub fn draw(ui: &mut Ui, params: &Arc<DoppelParams>, setter: &ParamSetter, meter: &Arc<Meter>, state: &mut UiState) {
    ui.ctx().request_repaint();
    let cols = snapshot(meter, state);

    header(ui, params, setter, state);
    ui.add_space(8.0);
    let lanes_h = ui.available_height() - RULER_H - KNOB_ROW - 16.0;
    lanes(ui, lanes_h, params, &cols, state);
    ui.add_space(8.0);
    ruler(ui, params, setter, &cols);
    ui.add_space(8.0);
    knobs(ui, params, setter);
}

fn header(ui: &mut Ui, params: &DoppelParams, setter: &ParamSetter, state: &UiState) {
    let theme = preset_kit::ui::Theme { fill: PANEL, hover: GRID, ink: rgba(TEXT, 255.0), dim: TEXT_DIM };
    preset_kit::ui::picker_top_right(ui, 24.0, &presets::BANK, params, setter, &theme);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("DOPPEL").monospace().size(11.0).color(TEXT_DIM));
        ui.label(egui::RichText::new("automatic double tracking").monospace().size(9.0).color(GRID));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(preset_kit::ui::WIDTH + 14.0);
            for (text, color) in [
                (format!("{:>3.0}¢", state.cents_peak), VIOLET),
                (format!("{:>5.1}ms", params.time.value()), PINK),
            ] {
                ui.label(egui::RichText::new(text).monospace().size(18.0).color(rgba(color, 255.0)));
                ui.add_space(10.0);
            }
        });
    });
}

fn lanes(ui: &mut Ui, height: f32, params: &DoppelParams, c: &Columns, state: &UiState) {
    let size = Vec2::new(ui.available_width(), height);
    let (response, painter) = ui.allocate_painter(size, Sense::hover());
    let rect = response.rect;
    painter.rect_filled(rect, 6.0, PANEL);

    let lane_a = Rect::from_min_size(Pos2::new(rect.left(), rect.top() + 10.0), Vec2::new(rect.width(), LANE_H));
    let lane_b = Rect::from_min_size(Pos2::new(rect.left(), rect.bottom() - 10.0 - LANE_H), Vec2::new(rect.width(), LANE_H));
    let x0 = rect.left() + REEL_ZONE + 8.0;
    let x1 = rect.right() - 14.0;
    let colw = (x1 - x0) / COLS as f32;
    let x_of = |k: f32| x0 + (k + 0.5) * colw;

    // Threads first, so the lanes sit on top: lead moment -> the moment the copy replays it.
    for k in (3..COLS).step_by(4) {
        let s = k as f32 - c.delay[k] / COL_MS;
        if s < 0.0 {
            continue;
        }
        let (i, f) = (s as usize, s.fract());
        let amp = c.lead[i] + (c.lead[(i + 1).min(COLS - 1)] - c.lead[i]) * f;
        let strength = (amp * 4.0).min(1.0);
        if strength < 0.03 {
            continue;
        }
        let age = k as f32 / COLS as f32;
        let (a, b) = (Pos2::new(x_of(s), lane_a.center().y + LANE_H * 0.5), Pos2::new(x_of(k as f32), lane_b.center().y - LANE_H * 0.5));
        let color = mix(LIME, PINK, 0.5);
        painter.line_segment([a, b], Stroke::new(4.0_f32, rgba(color, 22.0 * strength * age)));
        painter.line_segment([a, b], Stroke::new(1.0_f32, rgba(mix(color, TEXT, 0.25), 200.0 * strength * age)));
    }

    for (lane, label, data, color, reel) in [
        (lane_a, "TAKE 1  you", &c.lead, LIME, state.reel[0]),
        (lane_b, "TAKE 2  not you", &c.double, PINK, state.reel[1]),
    ] {
        waveform(&painter, lane, x0, x1, colw, data, color);
        painter.text(Pos2::new(x0 + 2.0, lane.top() - 2.0), Align2::LEFT_TOP, label, FontId::monospace(9.0), TEXT_DIM);
        painter.line_segment(
            [Pos2::new(x1 + 5.0, lane.top() + 4.0), Pos2::new(x1 + 5.0, lane.bottom() - 4.0)],
            Stroke::new(1.0_f32, rgba(TEXT, 40.0)),
        );
        reel_icon(&painter, Pos2::new(rect.left() + REEL_ZONE * 0.5 + 6.0, lane.center().y), 24.0, reel, color);
    }

    // The copy's level shows as a dim ceiling on its lane.
    let ceiling = params.level.value().min(1.0).powf(0.6);
    painter.line_segment(
        [Pos2::new(x0, lane_b.center().y - ceiling * LANE_H * 0.5), Pos2::new(x1, lane_b.center().y - ceiling * LANE_H * 0.5)],
        Stroke::new(1.0_f32, rgba(PINK, 24.0)),
    );
}

fn waveform(painter: &Painter, lane: Rect, x0: f32, x1: f32, colw: f32, data: &[f32; COLS], color: [f32; 3]) {
    let cy = lane.center().y;
    painter.line_segment([Pos2::new(x0, cy), Pos2::new(x1, cy)], Stroke::new(1.0_f32, GRID));
    for (k, &peak) in data.iter().enumerate() {
        let age = k as f32 / COLS as f32;
        let h = peak.min(1.0).powf(0.6) * LANE_H * 0.5;
        if h < 0.6 {
            continue;
        }
        let x = x0 + (k as f32 + 0.5) * colw;
        let glow = Rect::from_center_size(Pos2::new(x, cy), Vec2::new(colw * 2.6, h * 2.0 + 6.0));
        painter.rect_filled(glow, 3.0, rgba(color, 14.0 + 22.0 * age));
        let bar = Rect::from_center_size(Pos2::new(x, cy), Vec2::new((colw * 0.75).max(1.0), h * 2.0));
        painter.rect_filled(bar, 0.0, rgba(color, 70.0 + 185.0 * age * age));
    }
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

/// A tape reel: a flange, three spokes and a hub.
fn reel_icon(painter: &Painter, center: Pos2, radius: f32, angle: f32, color: [f32; 3]) {
    painter.circle_filled(center, radius, rgba(color, 14.0));
    painter.circle_stroke(center, radius, Stroke::new(1.5_f32, rgba(color, 150.0)));
    painter.circle_stroke(center, radius * 0.62, Stroke::new(1.0_f32, rgba(color, 50.0)));
    for k in 0..3 {
        let a = angle + k as f32 * TAU / 3.0;
        let dir = Vec2::new(a.cos(), a.sin());
        painter.line_segment([center + dir * 5.0, center + dir * (radius - 3.0)], Stroke::new(5.0_f32, rgba(color, 28.0)));
        painter.line_segment([center + dir * 5.0, center + dir * (radius - 3.0)], Stroke::new(2.0_f32, rgba(color, 230.0)));
    }
    painter.circle_filled(center, 4.0, rgba(TEXT, 255.0));
}

fn ruler(ui: &mut Ui, params: &DoppelParams, setter: &ParamSetter, c: &Columns) {
    let size = Vec2::new(ui.available_width(), RULER_H);
    let (response, painter) = ui.allocate_painter(size, Sense::click_and_drag());
    let rect = response.rect;
    painter.rect_filled(rect, 6.0, PANEL);

    let (left, right) = (rect.left() + 16.0, rect.right() - 16.0);
    let x_ms = |ms: f32| left + ms / MAX_TIME_MS * (right - left);
    let base = rect.bottom() - 18.0;
    let top = rect.top() + 8.0;

    for ms in (0..=120).step_by(10) {
        let x = x_ms(ms as f32);
        let major = ms % 30 == 0;
        painter.line_segment([Pos2::new(x, base + 4.0), Pos2::new(x, base + if major { 10.0 } else { 7.0 })], Stroke::new(1.0_f32, GRID));
        if major {
            painter.text(Pos2::new(x, rect.bottom() - 1.0), Align2::CENTER_BOTTOM, format!("{ms}"), FontId::monospace(8.0), TEXT_DIM);
        }
    }
    painter.line_segment([Pos2::new(left, base + 4.0), Pos2::new(right, base + 4.0)], Stroke::new(1.0_f32, GRID));
    painter.text(Pos2::new(right, top - 2.0), Align2::RIGHT_TOP, "drag: time", FontId::monospace(9.0), TEXT_DIM);

    // The lead is at zero.
    painter.line_segment([Pos2::new(x_ms(0.0), top), Pos2::new(x_ms(0.0), base + 4.0)], Stroke::new(2.0_f32, rgba(LIME, 200.0)));

    // Recent delay history as plumb lines: newest at the bottom.
    let y_of = |k: usize| top + (base - top) * k as f32 / (COLS - 1) as f32;
    let stereo = c.delay_right[COLS - 1] > 0.0;
    for (data, color, active) in [(&c.delay_right, VIOLET, stereo), (&c.delay, PINK, true)] {
        if !active {
            continue;
        }
        for k in 1..COLS {
            if data[k] <= 0.0 || data[k - 1] <= 0.0 {
                continue;
            }
            let age = k as f32 / COLS as f32;
            let seg = [Pos2::new(x_ms(data[k - 1]), y_of(k - 1)), Pos2::new(x_ms(data[k]), y_of(k))];
            painter.line_segment(seg, Stroke::new(5.0_f32, rgba(color, 26.0 * age)));
            painter.line_segment(seg, Stroke::new(1.5_f32, rgba(color, 230.0 * age * age)));
        }
    }

    // The handle: where the time is set.
    let hx = x_ms(params.time.value());
    let lit = response.hovered() || response.dragged();
    painter.line_segment(
        [Pos2::new(hx, top), Pos2::new(hx, base)],
        Stroke::new(1.0_f32, rgba(PINK, if lit { 90.0 } else { 40.0 })),
    );
    let pill = Rect::from_center_size(Pos2::new(hx, base + 1.0), Vec2::new(12.0, 14.0));
    painter.rect_filled(pill.expand(3.0), 5.0, rgba(PINK, if lit { 50.0 } else { 22.0 }));
    painter.rect_filled(pill, 3.0, rgba(PINK, if lit { 255.0 } else { 220.0 }));

    if response.drag_started() {
        setter.begin_set_parameter(&params.time);
    }
    if response.dragged() || response.drag_started() {
        if let Some(pos) = response.interact_pointer_pos() {
            let ms = (pos.x - left) / (right - left) * MAX_TIME_MS;
            setter.set_parameter(&params.time, ms.clamp(MIN_TIME_MS, MAX_TIME_MS));
        }
    }
    if response.drag_stopped() {
        setter.end_set_parameter(&params.time);
    }
    if response.double_clicked() {
        setter.begin_set_parameter(&params.time);
        setter.set_parameter(&params.time, params.time.default_plain_value());
        setter.end_set_parameter(&params.time);
    }
}

fn knobs(ui: &mut Ui, params: &DoppelParams, setter: &ParamSetter) {
    let (response, painter) = ui.allocate_painter(Vec2::new(ui.available_width(), KNOB_ROW), Sense::hover());
    let row = response.rect;
    let cells: [(&FloatParam, &str, [f32; 3]); 6] = [
        (&params.time, "TIME", PINK),
        (&params.drift, "DRIFT", VIOLET),
        (&params.rate, "RATE", VIOLET),
        (&params.tape, "TAPE", PINK),
        (&params.width, "WIDTH", VIOLET),
        (&params.level, "DOUBLE", PINK),
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
    let radius = 25.0;
    let (start, end) = (PI * 0.75, PI * 2.25);
    let lit = r.hovered() || r.dragged();
    let main = rgba(color, 255.0);

    painter.circle_filled(center, radius - 5.0, if lit { PANEL } else { Color32::from_rgb(0x12, 0x10, 0x1b) });
    painter.add(Shape::line(arc(center, radius, start, end), Stroke::new(3.0_f32, GRID)));
    if norm > 0.002 {
        let to = start + (end - start) * norm;
        painter.add(Shape::line(arc(center, radius, start, to), Stroke::new(8.0_f32, rgba(color, 30.0))));
        painter.add(Shape::line(arc(center, radius, start, to), Stroke::new(3.0_f32, main)));
    }
    let angle = start + (end - start) * norm;
    let dir = Vec2::new(angle.cos(), angle.sin());
    painter.line_segment([center + dir * (radius - 14.0), center + dir * (radius - 7.0)], Stroke::new(2.0_f32, rgba(TEXT, 255.0)));

    painter.text(Pos2::new(cell.center().x, cell.top() + 69.0), Align2::CENTER_CENTER, label, FontId::monospace(9.0), TEXT_DIM);
    painter.text(Pos2::new(cell.center().x, cell.top() + 85.0), Align2::CENTER_CENTER, param.to_string(), FontId::monospace(11.0), main);
}
