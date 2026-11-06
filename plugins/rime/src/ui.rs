//! The editor: the input spectrum as a ridge of embers, with every suppressed resonance hanging
//! beneath it as an icicle whose tip is the level after the cut.
//!
//! The two grab bars on the graph are the focus range: drag them to limit where suppression works.
//! Knobs: drag vertically (hold shift for fine), double-click to reset.

use nih_plug::prelude::*;
use nih_plug_egui::egui::{
    self, Align2, Color32, FontId, Mesh, Painter, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2,
};
use std::f32::consts::PI;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::dsp::{Meter, BANDS, F_MAX, F_MIN};
use crate::{presets, RimeParams};

const BG: Color32 = Color32::from_rgb(0x11, 0x0f, 0x0e);
const PANEL: Color32 = Color32::from_rgb(0x1c, 0x19, 0x17);
const GRID: Color32 = Color32::from_rgb(0x2f, 0x2a, 0x27);
const TEXT_DIM: Color32 = Color32::from_rgb(0x8c, 0x80, 0x76);
const TEXT: [f32; 3] = [244.0, 236.0, 224.0];
const GLOW_TOP: [f32; 3] = [36.0, 24.0, 20.0];
const GLOW_BOTTOM: [f32; 3] = [15.0, 13.0, 12.0];
const EMBER_LOW: [f32; 3] = [255.0, 176.0, 64.0];
const EMBER_HIGH: [f32; 3] = [255.0, 96.0, 56.0];
const ICE: [f32; 3] = [150.0, 226.0, 255.0];
const ICE_DEEP: [f32; 3] = [64.0, 140.0, 255.0];

/// Display range for the spectrum, in dB relative to a full-scale sine.
const DB_FLOOR: f32 = -96.0;
const DB_RANGE: f32 = 90.0;
/// Reduction that makes the longest icicle, in dB. Matches the DSP limit.
const MAX_ICICLE_DB: f32 = 24.0;
const KNOB_ROW: f32 = 100.0;

pub struct UiState {
    level_db: [f32; BANDS],
    reduction_db: [f32; BANDS],
}

impl Default for UiState {
    fn default() -> Self {
        Self { level_db: [-120.0; BANDS], reduction_db: [0.0; BANDS] }
    }
}

pub fn background() -> Color32 {
    BG
}

fn rgba(c: [f32; 3], a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c[0] as u8, c[1] as u8, c[2] as u8, a as u8)
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// Level falls slowly so peaks stay readable; reduction follows the DSP, which already smooths it.
fn follow(state: &mut UiState, meter: &Meter) {
    for i in 0..BANDS {
        let db = meter.level[i].load(Ordering::Relaxed);
        state.level_db[i] = if db > state.level_db[i] { db } else { (state.level_db[i] - 1.5).max(db) };
        let r = meter.reduction[i].load(Ordering::Relaxed);
        state.reduction_db[i] = if r > state.reduction_db[i] { r } else { (state.reduction_db[i] - 1.0).max(r) };
    }
}

fn blur(values: &[f32; BANDS]) -> [f32; BANDS] {
    std::array::from_fn(|i| {
        let l = values[i.saturating_sub(1)];
        let r = values[(i + 1).min(BANDS - 1)];
        (l + 2.0 * values[i] + r) / 4.0
    })
}

/// Triangle strip between a top and a bottom edge, each vertex with its own colour.
fn strip(painter: &Painter, cols: &[(Pos2, Color32, Pos2, Color32)]) {
    let mut mesh = Mesh::default();
    for (top, top_color, bottom, bottom_color) in cols {
        mesh.colored_vertex(*top, *top_color);
        mesh.colored_vertex(*bottom, *bottom_color);
    }
    for i in 0..cols.len().saturating_sub(1) {
        let k = (i * 2) as u32;
        mesh.add_triangle(k, k + 1, k + 2);
        mesh.add_triangle(k + 1, k + 2, k + 3);
    }
    painter.add(Shape::mesh(mesh));
}

pub fn draw(ui: &mut Ui, params: &Arc<RimeParams>, setter: &ParamSetter, meter: &Arc<Meter>, state: &mut UiState) {
    ui.ctx().request_repaint();
    follow(state, meter);

    header(ui, params, setter, state);
    ui.add_space(8.0);
    let graph_height = ui.available_height() - KNOB_ROW - 10.0;
    graph(ui, params, setter, state, graph_height);
    ui.add_space(10.0);
    knobs(ui, params, setter);
}

fn header(ui: &mut Ui, params: &RimeParams, setter: &ParamSetter, state: &UiState) {
    let (response, painter) = ui.allocate_painter(Vec2::new(ui.available_width(), 24.0), Sense::hover());
    let rect = response.rect;
    painter.text(rect.left_center(), Align2::LEFT_CENTER, "R I M E", FontId::monospace(15.0), rgba(TEXT, 255.0));
    painter.text(
        rect.left_center() + Vec2::new(92.0, 1.0),
        Align2::LEFT_CENTER,
        "resonance suppressor",
        FontId::monospace(10.0),
        TEXT_DIM,
    );

    // Preset picker, between the subtitle (ends near x+215) and the reduction readout (starts near x+600).
    let theme = preset_kit::ui::Theme { fill: PANEL, hover: GRID, ink: rgba(TEXT, 255.0), dim: TEXT_DIM };
    preset_kit::ui::picker(ui, Pos2::new(rect.left() + 250.0, rect.center().y - 11.0), 22.0, &presets::BANK, params, setter, &theme);

    // Delta toggle.
    let button = Rect::from_center_size(Pos2::new(rect.right() - 30.0, rect.center().y), Vec2::new(60.0, 22.0));
    let r = ui.interact(button, ui.id().with("delta"), Sense::click());
    let on = params.delta.value();
    if r.clicked() {
        setter.begin_set_parameter(&params.delta);
        setter.set_parameter(&params.delta, !on);
        setter.end_set_parameter(&params.delta);
    }
    let fill = if on { rgba(ICE, 60.0) } else if r.hovered() { PANEL } else { BG };
    painter.rect_filled(button, 4.0, fill);
    painter.rect_stroke(button, 4.0, Stroke::new(1.0_f32, if on { rgba(ICE, 255.0) } else { GRID }), egui::StrokeKind::Inside);
    painter.text(
        button.center(),
        Align2::CENTER_CENTER,
        "DELTA",
        FontId::monospace(10.0),
        if on { rgba(ICE, 255.0) } else { TEXT_DIM },
    );

    let deepest = state.reduction_db.iter().cloned().fold(0.0, f32::max);
    painter.text(
        Pos2::new(button.left() - 16.0, rect.center().y),
        Align2::RIGHT_CENTER,
        format!("cooling  -{deepest:.1} dB"),
        FontId::monospace(10.0),
        if deepest > 0.5 { rgba(ICE, 255.0) } else { TEXT_DIM },
    );
}

fn graph(ui: &mut Ui, params: &Arc<RimeParams>, setter: &ParamSetter, state: &UiState, height: f32) {
    let size = Vec2::new(ui.available_width(), height);
    let (response, painter) = ui.allocate_painter(size, Sense::hover());
    let rect = response.rect;
    let bottom = rect.bottom() - 18.0;
    let top = rect.top() + 6.0;
    let h = bottom - top;
    let ln_span = (F_MAX / F_MIN).ln();
    let x_of = |f: f32| rect.left() + (f / F_MIN).ln() / ln_span * rect.width();
    let f_of = |x: f32| F_MIN * (((x - rect.left()) / rect.width()).clamp(0.0, 1.0) * ln_span).exp();
    let y_of = |db: f32| bottom - ((db - DB_FLOOR) / DB_RANGE).clamp(0.0, 1.0) * h;
    let band_x = |i: usize| rect.left() + (i as f32 + 0.5) / BANDS as f32 * rect.width();
    let slot = rect.width() / BANDS as f32;

    // Focus handles: interaction first so the painting can react.
    let handles = [(&params.low, "low"), (&params.high, "high")];
    let mut active = [false; 2];
    for (i, (param, id)) in handles.iter().enumerate() {
        let x = x_of(param.value());
        let grab = Rect::from_min_max(Pos2::new(x - 9.0, rect.top()), Pos2::new(x + 9.0, rect.bottom()));
        let r = ui.interact(grab, ui.id().with(*id), Sense::click_and_drag());
        active[i] = r.hovered() || r.dragged();
        if r.drag_started() {
            setter.begin_set_parameter(*param);
        }
        if r.dragged() {
            if let Some(pos) = r.interact_pointer_pos() {
                setter.set_parameter(*param, f_of(pos.x).clamp(F_MIN, F_MAX));
            }
        }
        if r.drag_stopped() {
            setter.end_set_parameter(*param);
        }
        if r.double_clicked() {
            setter.begin_set_parameter(*param);
            setter.set_parameter(*param, param.default_plain_value());
            setter.end_set_parameter(*param);
        }
    }
    let (low, high) = {
        let (a, b) = (params.low.value(), params.high.value());
        (a.min(b), a.max(b))
    };

    // Backdrop and scales.
    strip(
        &painter,
        &[
            (rect.left_top(), rgba(GLOW_TOP, 255.0), rect.left_bottom(), rgba(GLOW_BOTTOM, 255.0)),
            (rect.right_top(), rgba(GLOW_TOP, 255.0), rect.right_bottom(), rgba(GLOW_BOTTOM, 255.0)),
        ],
    );
    for db in [-20.0, -40.0, -60.0, -80.0] {
        let y = y_of(db);
        painter.line_segment([Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)], Stroke::new(1.0_f32, rgba([255.0, 255.0, 255.0], 8.0)));
        painter.text(Pos2::new(rect.left() + 5.0, y - 2.0), Align2::LEFT_BOTTOM, format!("{db:.0}"), FontId::monospace(8.0), TEXT_DIM);
    }
    for (f, label) in [(50.0, "50"), (100.0, "100"), (200.0, "200"), (500.0, "500"), (1000.0, "1k"), (2000.0, "2k"), (5000.0, "5k"), (10000.0, "10k"), (20000.0, "20k")] {
        let x = x_of(f).min(rect.right() - 12.0);
        painter.line_segment([Pos2::new(x, top), Pos2::new(x, bottom)], Stroke::new(1.0_f32, rgba([255.0, 255.0, 255.0], 6.0)));
        painter.text(Pos2::new(x, bottom + 5.0), Align2::CENTER_TOP, label, FontId::monospace(9.0), TEXT_DIM);
    }
    painter.line_segment([Pos2::new(rect.left(), bottom), Pos2::new(rect.right(), bottom)], Stroke::new(1.0_f32, GRID));

    // Embers: the input spectrum.
    let level = blur(&state.level_db);
    let reduction = blur(&state.reduction_db);
    let ridge: Vec<Pos2> = (0..BANDS).map(|i| Pos2::new(band_x(i), y_of(level[i]))).collect();
    let ember = |i: usize, a: f32| rgba(mix(EMBER_LOW, EMBER_HIGH, i as f32 / BANDS as f32), a);
    strip(
        &painter,
        &ridge.iter().enumerate().map(|(i, p)| (*p, ember(i, 120.0), Pos2::new(p.x, bottom), ember(i, 0.0))).collect::<Vec<_>>(),
    );
    for i in 0..BANDS - 1 {
        painter.line_segment([ridge[i], ridge[i + 1]], Stroke::new(4.0_f32, ember(i, 30.0)));
        painter.line_segment([ridge[i], ridge[i + 1]], Stroke::new(1.5_f32, ember(i, 235.0)));
    }

    // Icicles: each cut hangs from the ridge, its tip the level after suppression.
    let tip_of = |i: usize| {
        let cut = reduction[i].min(MAX_ICICLE_DB);
        Pos2::new(ridge[i].x, y_of(level[i] - cut).max(ridge[i].y))
    };
    for i in 0..BANDS {
        if reduction[i] < 0.3 {
            continue;
        }
        let strength = (reduction[i] / 12.0).clamp(0.25, 1.0);
        let tip = tip_of(i);
        let half = slot * 0.5;
        let mut mesh = Mesh::default();
        mesh.colored_vertex(Pos2::new(ridge[i].x - half, ridge[i].y), rgba(ICE_DEEP, 150.0 * strength));
        mesh.colored_vertex(Pos2::new(ridge[i].x + half, ridge[i].y), rgba(ICE_DEEP, 150.0 * strength));
        mesh.colored_vertex(tip, rgba(ICE, 255.0 * strength));
        mesh.add_triangle(0, 1, 2);
        painter.add(Shape::mesh(mesh));
    }
    let tips: Vec<Pos2> = (0..BANDS).map(tip_of).collect();
    painter.add(Shape::line(tips.clone(), Stroke::new(5.0_f32, rgba(ICE, 28.0))));
    painter.add(Shape::line(tips.clone(), Stroke::new(1.25_f32, rgba(ICE, 210.0))));
    // A glint on the deepest cuts.
    for (i, tip) in tips.iter().enumerate() {
        if reduction[i] > 5.0 {
            let (r, a) = (2.0 + reduction[i] / 8.0, (reduction[i] / 12.0).clamp(0.3, 1.0) * 255.0);
            let diamond = vec![
                *tip + Vec2::new(0.0, -r),
                *tip + Vec2::new(r * 0.6, 0.0),
                *tip + Vec2::new(0.0, r),
                *tip + Vec2::new(-r * 0.6, 0.0),
            ];
            painter.add(Shape::convex_polygon(diamond, rgba(TEXT, a), Stroke::NONE));
        }
    }

    // Focus range: dim what is outside it, then the grab bars.
    let (x_low, x_high) = (x_of(low), x_of(high));
    let veil = Color32::from_rgba_unmultiplied(8, 6, 5, 170);
    painter.rect_filled(Rect::from_min_max(rect.min, Pos2::new(x_low, bottom)), 0.0, veil);
    painter.rect_filled(Rect::from_min_max(Pos2::new(x_high, rect.top()), Pos2::new(rect.right(), bottom)), 0.0, veil);
    for (i, ((param, _), x)) in handles.iter().zip([x_of(params.low.value()), x_of(params.high.value())]).enumerate() {
        let strength = if active[i] { 1.0 } else { 0.55 };
        painter.line_segment([Pos2::new(x, top), Pos2::new(x, bottom)], Stroke::new(1.0_f32, rgba(ICE, 110.0 * strength)));
        let tab = Rect::from_center_size(Pos2::new(x, top + 7.0), Vec2::new(8.0, 14.0));
        painter.rect_filled(tab, 2.0, rgba(ICE, 255.0 * strength));
        if active[i] {
            painter.text(
                Pos2::new(x + if x < rect.center().x { 8.0 } else { -8.0 }, top + 7.0),
                if x < rect.center().x { Align2::LEFT_CENTER } else { Align2::RIGHT_CENTER },
                format!("{:.0} Hz", param.value()),
                FontId::monospace(10.0),
                rgba(ICE, 255.0),
            );
        }
    }
}

/// The row of knobs under the graph; ice for detection, amber for timing, plain for output.
fn knobs(ui: &mut Ui, params: &RimeParams, setter: &ParamSetter) {
    let (response, painter) = ui.allocate_painter(Vec2::new(ui.available_width(), KNOB_ROW), Sense::hover());
    let row = response.rect;
    let amber = rgba(EMBER_LOW, 255.0);
    let ice = rgba(ICE, 255.0);
    let cells: [(&FloatParam, &str, Color32); 8] = [
        (&params.depth, "DEPTH", ice),
        (&params.sharpness, "SHARPNESS", ice),
        (&params.selectivity, "SELECTIVITY", ice),
        (&params.attack, "ATTACK", amber),
        (&params.release, "RELEASE", amber),
        (&params.link, "LINK", amber),
        (&params.mix, "MIX", rgba(TEXT, 255.0)),
        (&params.trim, "TRIM", rgba(TEXT, 255.0)),
    ];
    let width = row.width() / cells.len() as f32;
    for (i, (param, label, color)) in cells.iter().enumerate() {
        let cell = Rect::from_min_size(Pos2::new(row.left() + i as f32 * width, row.top()), Vec2::new(width, KNOB_ROW));
        knob(ui, &painter, cell, setter, *param, label, *color);
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

fn knob(ui: &mut Ui, painter: &Painter, cell: Rect, setter: &ParamSetter, param: &FloatParam, label: &str, color: Color32) {
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
    let [cr, cg, cb, _] = color.to_array();
    let rgb = [cr as f32, cg as f32, cb as f32];

    painter.circle_filled(center, radius - 5.0, if lit { PANEL } else { Color32::from_rgb(0x17, 0x14, 0x13) });
    painter.add(Shape::line(arc(center, radius, start, end), Stroke::new(3.0_f32, GRID)));
    if norm > 0.002 {
        let to = start + (end - start) * norm;
        painter.add(Shape::line(arc(center, radius, start, to), Stroke::new(8.0_f32, rgba(rgb, 30.0))));
        painter.add(Shape::line(arc(center, radius, start, to), Stroke::new(3.0_f32, color)));
    }
    let angle = start + (end - start) * norm;
    let dir = Vec2::new(angle.cos(), angle.sin());
    painter.line_segment([center + dir * (radius - 14.0), center + dir * (radius - 7.0)], Stroke::new(2.0_f32, rgba(TEXT, 255.0)));

    painter.text(Pos2::new(cell.center().x, cell.top() + 69.0), Align2::CENTER_CENTER, label, FontId::monospace(9.0), TEXT_DIM);
    painter.text(Pos2::new(cell.center().x, cell.top() + 85.0), Align2::CENTER_CENTER, param.to_string(), FontId::monospace(11.0), color);
}
