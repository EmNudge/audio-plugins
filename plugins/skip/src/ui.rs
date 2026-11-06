//! The editor: a stone skipping across a pond.
//!
//! The pond plots time on a log axis from 5 ms to 10 s, so the bounces crowd together the way a
//! real stone's do. The yellow hand at the left throws the input; every arc is one trip round the
//! feedback loop, landing at `n x` the delay time. Arcs shrink with the feedback and shift from
//! mint toward pink as the loop's drive, diffusion and pitch shift wear on them, and their
//! reflections hang in the water. Where the pitch shift is on, each splash is labelled with how
//! far that repeat has climbed. Drag the white stone sideways to set the time (it is locked to
//! the grid when synced) and the grip on the second arc up and down to set the feedback.
//!
//! Under it, the last 2.4 seconds of real signal scroll past: the input above the water line in
//! yellow, the echoes as actually heard below it in mint, with ticks at every multiple of the
//! delay time so you can see the repeats land on them. The arcs dim while the ducker is pushing
//! the wet signal down.
//!
//! Knobs: drag vertically (hold shift for fine), double-click to reset.

use nih_plug::prelude::*;
use nih_plug_egui::egui::{
    self, Align2, Color32, FontId, Mesh, Painter, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, Vec2,
};
use std::f32::consts::{PI, TAU};
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::dsp::{Division, Meter, COLS, MAX_TIME_MS, MIN_TIME_MS, WINDOW_S};
use crate::presets;
use crate::SkipParams;

const BG: Color32 = Color32::from_rgb(0x06, 0x0d, 0x12);
const PANEL: Color32 = Color32::from_rgb(0x0e, 0x18, 0x1f);
const GRID: Color32 = Color32::from_rgb(0x22, 0x34, 0x3d);
const TEXT_DIM: Color32 = Color32::from_rgb(0x70, 0x8a, 0x94);
const TEXT: [f32; 3] = [232.0, 244.0, 246.0];
/// The echo: timing and the repeats themselves.
const MINT: [f32; 3] = [53.0, 240.0, 192.0];
/// What each trip round the loop does to a repeat.
const PINK: [f32; 3] = [255.0, 77.0, 141.0];
/// The input that throws the stone.
const SUN: [f32; 3] = [255.0, 228.0, 94.0];

const POND_MIN_H: f32 = 200.0;
const HISTORY_H: f32 = 96.0;
const KNOB_H: f32 = 84.0;
/// The pond's time axis in seconds.
const T0: f32 = 0.005;
const T1: f32 = 10.0;
/// Most bounces drawn.
const MAX_BOUNCES: usize = 96;

pub struct UiState {
    last_head: usize,
    /// Input level (0..1), risen at once and fallen over about a third of a second.
    hand: f32,
}

impl Default for UiState {
    fn default() -> Self {
        Self { last_head: 0, hand: 0.0 }
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

/// A peak level on a -60..0 dB scale, as 0..1.
fn lvl(peak: f32) -> f32 {
    ((20.0 * (peak + 1e-6).log10() + 60.0) / 60.0).clamp(0.0, 1.0)
}

struct Columns {
    input: [f32; COLS],
    wet: [f32; COLS],
    duck_gain: f32,
}

fn snapshot(meter: &Meter, state: &mut UiState) -> Columns {
    let head = meter.head.load(Ordering::Acquire);
    let mut c = Columns { input: [0.0; COLS], wet: [0.0; COLS], duck_gain: meter.duck_gain.load(Ordering::Relaxed) };
    for k in 0..COLS {
        let slot = (head + k) % COLS;
        c.input[k] = lvl(meter.input[slot].load(Ordering::Relaxed));
        c.wet[k] = lvl(meter.wet[slot].load(Ordering::Relaxed));
    }
    // Rise fast, fall slowly: once per new column.
    if head != state.last_head {
        state.hand = c.input[COLS - 1].max(state.hand * 0.93);
        state.last_head = head;
    }
    c
}

pub fn draw(ui: &mut Ui, params: &Arc<SkipParams>, setter: &ParamSetter, meter: &Arc<Meter>, state: &mut UiState) {
    ui.ctx().request_repaint();
    let cols = snapshot(meter, state);

    header(ui, params, setter);
    ui.add_space(6.0);
    mode_row(ui, params, setter);
    ui.add_space(8.0);
    let pond_h = (ui.available_height() - HISTORY_H - KNOB_H - 16.0).max(POND_MIN_H);
    pond(ui, pond_h, params, setter, &cols, state);
    ui.add_space(8.0);
    history(ui, params, &cols);
    ui.add_space(8.0);
    knobs(ui, params, setter);
}

fn header(ui: &mut Ui, params: &SkipParams, setter: &ParamSetter) {
    let theme = preset_kit::ui::Theme { fill: PANEL, hover: GRID, ink: rgba(TEXT, 255.0), dim: TEXT_DIM };
    preset_kit::ui::picker_top_right(ui, 24.0, &presets::BANK, params, setter, &theme);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("SKIP").monospace().size(11.0).color(TEXT_DIM));
        ui.label(egui::RichText::new("a delay whose loop changes things").monospace().size(9.0).color(GRID));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(preset_kit::ui::WIDTH + 14.0);
            for (text, color) in [
                (format!("{:>3.0}%", params.feedback.value() * 100.0), PINK),
                (format!("{:>6.0}ms", params.time_ms(None)), MINT),
            ] {
                ui.label(egui::RichText::new(text).monospace().size(18.0).color(rgba(color, 255.0)));
                ui.add_space(10.0);
            }
        });
    });
}

/// Note-length chips and the sync key. Clicking a chip also switches sync on.
fn mode_row(ui: &mut Ui, params: &SkipParams, setter: &ParamSetter) {
    let (response, painter) = ui.allocate_painter(Vec2::new(ui.available_width(), 24.0), Sense::hover());
    let row = response.rect;
    let synced = params.sync.value();
    let current = params.division.value();
    for (i, division) in Division::ALL.iter().enumerate() {
        let rect = Rect::from_min_size(row.min + Vec2::new(i as f32 * 62.0, 0.0), Vec2::new(56.0, 24.0));
        let r = ui.interact(rect, ui.id().with(("division", i)), Sense::click());
        let active = synced && current == *division;
        let fill = if active { rgba(MINT, 255.0) } else if r.hovered() { GRID } else { PANEL };
        painter.rect_filled(rect, 4.0, fill);
        let ink = if active {
            BG
        } else if r.hovered() {
            rgba(TEXT, 255.0)
        } else if current == *division {
            rgba(MINT, 200.0)
        } else {
            TEXT_DIM
        };
        let label = Division::variants()[i];
        painter.text(rect.center(), Align2::CENTER_CENTER, label, FontId::monospace(10.0), ink);
        if r.clicked() {
            setter.begin_set_parameter(&params.division);
            setter.set_parameter(&params.division, *division);
            setter.end_set_parameter(&params.division);
            if !synced {
                setter.begin_set_parameter(&params.sync);
                setter.set_parameter(&params.sync, true);
                setter.end_set_parameter(&params.sync);
            }
        }
    }

    let rect = Rect::from_min_size(Pos2::new(row.right() - 96.0, row.top()), Vec2::new(96.0, 24.0));
    let r = ui.interact(rect, ui.id().with("sync"), Sense::click());
    if synced {
        painter.rect_filled(rect.expand(3.0), 6.0, rgba(MINT, 36.0));
    }
    painter.rect_filled(rect, 4.0, if synced { rgba(MINT, 255.0) } else if r.hovered() { GRID } else { PANEL });
    let ink = if synced { BG } else if r.hovered() { rgba(TEXT, 255.0) } else { TEXT_DIM };
    painter.text(rect.center(), Align2::CENTER_CENTER, if synced { "SYNCED" } else { "FREE" }, FontId::monospace(10.0), ink);
    if r.clicked() {
        setter.begin_set_parameter(&params.sync);
        setter.set_parameter(&params.sync, !synced);
        setter.end_set_parameter(&params.sync);
    }
}

/// `ui.interact` for a rect that is not laid out, with an id derived from a key.
fn ui_interact(ui: &Ui, rect: Rect, key: &str) -> Response {
    ui.interact(rect, ui.id().with(key), Sense::click_and_drag())
}

/// Sets a parameter from the pointer while dragging; double-click resets it.
fn drag_param(r: &Response, setter: &ParamSetter, param: &FloatParam, value: impl Fn(Pos2) -> f32) {
    if r.drag_started() {
        setter.begin_set_parameter(param);
    }
    if r.dragged() {
        if let Some(p) = r.interact_pointer_pos() {
            let (lo, hi) = (param.preview_plain(0.0), param.preview_plain(1.0));
            setter.set_parameter(param, value(p).clamp(lo, hi));
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

fn ellipse(center: Pos2, rx: f32, ry: f32) -> Vec<Pos2> {
    (0..=24).map(|i| center + Vec2::new((TAU * i as f32 / 24.0).cos() * rx, (TAU * i as f32 / 24.0).sin() * ry)).collect()
}

/// How a bounce's arc is flattened when the log axis squeezes it.
fn squash(width_px: f32) -> f32 {
    (width_px / 70.0).clamp(0.28, 1.0)
}

fn pond(ui: &mut Ui, height: f32, params: &SkipParams, setter: &ParamSetter, cols: &Columns, state: &UiState) {
    let (response, painter) = ui.allocate_painter(Vec2::new(ui.available_width(), height), Sense::hover());
    let rect = response.rect;
    painter.rect_filled(rect, 6.0, PANEL);

    let (left, right) = (rect.left() + 36.0, rect.right() - 24.0);
    let water = rect.top() + height * 0.64;
    let reach = water - rect.top() - 34.0;
    let fx = |t: f32| left + ((t / T0).ln() / (T1 / T0).ln()).clamp(0.0, 1.0) * (right - left);
    let inv_t = |x: f32| T0 * (((x - left) / (right - left)).clamp(0.0, 1.0) * (T1 / T0).ln()).exp();

    let time_s = params.time_ms(None) * 0.001;
    let feedback = params.feedback.value();
    let synced = params.sync.value();
    let wet_gain = (params.mix.value() * PI * 0.5).sin();
    let scale = reach * (0.45 + 0.55 * wet_gain) / 1.15;
    // Wear per bounce: drive, diffusion and pitch shift all change a repeat.
    let wear = 0.06 + 0.5 * params.drive.value() + 0.3 * params.diffusion.value() + 0.5 * params.pitch.value().abs() / 12.0;
    let dim = 0.3 + 0.7 * cols.duck_gain;

    // Water: a gradient down from the surface, and the time grid.
    let mut mesh = Mesh::default();
    mesh.colored_vertex(Pos2::new(rect.left(), water), rgba(MINT, 22.0));
    mesh.colored_vertex(Pos2::new(rect.right(), water), rgba(MINT, 22.0));
    mesh.colored_vertex(Pos2::new(rect.left(), rect.bottom()), rgba(MINT, 0.0));
    mesh.colored_vertex(Pos2::new(rect.right(), rect.bottom()), rgba(MINT, 0.0));
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 2, 3);
    painter.add(Shape::mesh(mesh));
    for (t, label) in [(0.01, "10ms"), (0.1, "100ms"), (1.0, "1s"), (10.0, "10s")] {
        let x = fx(t);
        painter.line_segment([Pos2::new(x, water), Pos2::new(x, rect.bottom() - 16.0)], Stroke::new(1.0_f32, rgba(TEXT, 5.0)));
        painter.text(Pos2::new(x, rect.bottom() - 5.0), Align2::CENTER_BOTTOM, label, FontId::monospace(8.0), TEXT_DIM);
    }
    painter.line_segment([Pos2::new(rect.left() + 8.0, water), Pos2::new(rect.right() - 8.0, water)], Stroke::new(1.5_f32, rgba(MINT, 90.0)));
    painter.text(rect.left_top() + Vec2::new(12.0, 8.0), Align2::LEFT_TOP, "SKIPS ON A LOG TIME AXIS", FontId::monospace(9.0), TEXT_DIM);

    // Bounces: x of each landing, amplitude of the arc that ends there.
    let mut landings: Vec<(f32, f32)> = vec![(fx(T0), 1.0)];
    let mut amp = 1.0_f32;
    for k in 1..=MAX_BOUNCES {
        let t = k as f32 * time_s;
        if t > T1 || amp < 0.004 {
            break;
        }
        landings.push((fx(t), amp));
        amp = (amp * feedback).min(1.15);
    }
    let skips = if feedback >= 0.999 { None } else { Some((0.001_f32.ln() / feedback.max(0.001).ln()).ceil() as usize + 1) };

    // Arcs and their reflections, drawn far to near so the front ones sit on top.
    let live = 0.35 + 0.65 * cols.wet[COLS - 1];
    for k in (1..landings.len()).rev() {
        let ((x0, _), (x1, a)) = (landings[k - 1], landings[k]);
        let width = x1 - x0;
        // The first arc is the throw, from the hand at the left edge: kept low so it doesn't
        // swamp the bounces.
        let h = scale * a * squash(width) * if k == 1 { 0.6 } else { 1.0 };
        let damage = (k as f32 * wear).min(1.0);
        let color = mix(TEXT, mix(MINT, PINK, damage), (k as f32 * 0.5).min(1.0));
        let arc: Vec<Pos2> = (0..=28)
            .map(|i| {
                let u = i as f32 / 28.0;
                Pos2::new(x0 + width * u, water - h * 4.0 * u * (1.0 - u))
            })
            .collect();
        let fade = (1.0 - k as f32 / (landings.len() as f32 + 6.0)).max(0.25) * dim;
        let mirror: Vec<Pos2> = arc.iter().map(|p| Pos2::new(p.x, water + (water - p.y) * 0.35)).collect();
        painter.add(Shape::line(mirror, Stroke::new(1.5_f32, rgba(color, 40.0 * fade))));
        painter.add(Shape::line(arc.clone(), Stroke::new(7.0_f32, rgba(color, 22.0 * fade))));
        painter.add(Shape::line(arc, Stroke::new(2.0_f32, rgba(color, 235.0 * fade))));

        // The splash where it lands.
        let rx = 3.0 + 12.0 * a.min(1.0);
        for (ring, alpha) in [(1.0, 150.0), (1.8, 70.0)] {
            painter.add(Shape::line(
                ellipse(Pos2::new(x1, water), rx * ring, rx * ring * 0.28),
                Stroke::new(1.2_f32, rgba(color, alpha * live * fade)),
            ));
        }
        let gap = if k + 1 < landings.len() { landings[k + 1].0 - x1 } else { 99.0 };
        if k <= 8 && params.pitch.value().abs() > 0.05 && gap > 30.0 {
            let semis = k as f32 * params.pitch.value();
            painter.text(
                Pos2::new(x1, water + 18.0),
                Align2::CENTER_CENTER,
                format!("{semis:+.0}"),
                FontId::monospace(8.5),
                rgba(PINK, 200.0 * fade),
            );
        }
    }
    if landings.len() >= MAX_BOUNCES {
        painter.text(Pos2::new(right, water + 18.0), Align2::RIGHT_CENTER, "...", FontId::monospace(9.0), TEXT_DIM);
    }

    // The hand: the input, sized by how loud it is right now.
    let hand = Pos2::new(left, water);
    let r = 4.0 + 8.0 * state.hand;
    painter.circle_filled(hand, r + 7.0, rgba(SUN, 18.0));
    painter.circle_filled(hand, r, rgba(SUN, 255.0));
    painter.text(hand + Vec2::new(0.0, -r - 8.0), Align2::CENTER_BOTTOM, "IN", FontId::monospace(8.5), rgba(SUN, 200.0));

    let count = match skips {
        Some(n) if n < MAX_BOUNCES => format!("{n} skips to silence"),
        Some(_) => "skips for ages".to_string(),
        None => "never stops skipping".to_string(),
    };
    painter.text(rect.right_top() + Vec2::new(-12.0, 8.0), Align2::RIGHT_TOP, count, FontId::monospace(9.0), rgba(PINK, 200.0));

    // Grips: the first landing is the time, the apex of the second arc is the feedback. The
    // feedback grip is drawn first so the stone wins where they overlap.
    if landings.len() >= 3 {
        let ((x1, _), (x2, a2)) = (landings[1], landings[2]);
        let base = scale * squash(x2 - x1);
        let apex = Pos2::new((x1 + x2) * 0.5, water - base * a2);
        let r = ui_interact(ui, Rect::from_center_size(apex, Vec2::splat(26.0)), "feedback_grip");
        drag_param(&r, setter, &params.feedback, |p| (water - p.y) / base.max(1.0));
        grip(&painter, apex, PINK, &r, &format!("{:.0}%", feedback * 100.0));
    }
    if landings.len() >= 2 {
        let stone = Pos2::new(landings[1].0, water);
        let r = ui_interact(ui, Rect::from_center_size(stone, Vec2::splat(26.0)), "time_stone");
        if !synced {
            drag_param(&r, setter, &params.time, |p| (inv_t(p.x) * 1000.0).clamp(MIN_TIME_MS, MAX_TIME_MS));
        }
        let lit = (r.hovered() || r.dragged()) && !synced;
        let radius = if lit { 8.5 } else { 7.0 };
        painter.circle_filled(stone, radius + 6.0, rgba(TEXT, if lit { 24.0 } else { 10.0 }));
        painter.circle_filled(stone, radius, rgba(TEXT, 255.0));
        painter.circle_stroke(stone, radius, Stroke::new(1.5_f32, BG));
        let text = if time_s >= 1.0 { format!("{:.2}s", time_s) } else { format!("{:.0}ms", time_s * 1000.0) };
        painter.text(stone + Vec2::new(0.0, 14.0), Align2::CENTER_TOP, text, FontId::monospace(9.0), rgba(TEXT, 220.0));
    }
}

fn grip(painter: &Painter, center: Pos2, color: [f32; 3], r: &Response, text: &str) {
    let lit = r.hovered() || r.dragged();
    let radius = if lit { 7.5 } else { 6.0 };
    painter.circle_filled(center, radius + 6.0, rgba(color, if lit { 20.0 } else { 8.0 }));
    painter.circle_filled(center, radius, rgba(color, 255.0));
    painter.circle_stroke(center, radius, Stroke::new(1.5_f32, BG));
    painter.text(center + Vec2::new(0.0, -12.0), Align2::CENTER_BOTTOM, text, FontId::monospace(9.0), rgba(color, 230.0));
}

/// The last `WINDOW_S` seconds of input (up) and echoes as heard (down), with the delay grid.
fn history(ui: &mut Ui, params: &SkipParams, cols: &Columns) {
    let (response, painter) = ui.allocate_painter(Vec2::new(ui.available_width(), HISTORY_H), Sense::hover());
    let rect = response.rect;
    painter.rect_filled(rect, 6.0, PANEL);
    let plot = Rect::from_min_max(Pos2::new(rect.left() + 12.0, rect.top() + 18.0), Pos2::new(rect.right() - 12.0, rect.bottom() - 8.0));
    let mid = plot.center().y;
    let half = plot.height() * 0.5;

    // A tick at every multiple of the delay time, counted back from now.
    let time_s = params.time_ms(None) * 0.001;
    for k in 1..=64 {
        let age = k as f32 * time_s;
        let x = plot.right() - age / WINDOW_S * plot.width();
        if x < plot.left() {
            break;
        }
        if time_s / WINDOW_S * plot.width() >= 5.0 {
            painter.line_segment([Pos2::new(x, plot.top()), Pos2::new(x, plot.bottom())], Stroke::new(1.0_f32, rgba(TEXT, 9.0)));
        }
    }

    let step = plot.width() / COLS as f32;
    for i in 0..COLS {
        let x = plot.left() + (i as f32 + 0.5) * step;
        let recent = 0.35 + 0.65 * i as f32 / COLS as f32;
        let (up, down) = (cols.input[i] * half, cols.wet[i] * half);
        if up > 0.5 {
            painter.line_segment([Pos2::new(x, mid - 1.0), Pos2::new(x, mid - up)], Stroke::new(step.max(1.0), rgba(SUN, 200.0 * recent)));
        }
        if down > 0.5 {
            painter.line_segment([Pos2::new(x, mid + 1.0), Pos2::new(x, mid + down)], Stroke::new(step.max(1.0), rgba(MINT, 200.0 * recent)));
        }
    }
    painter.line_segment([Pos2::new(plot.left(), mid), Pos2::new(plot.right(), mid)], Stroke::new(1.0_f32, rgba(TEXT, 30.0)));
    painter.text(rect.left_top() + Vec2::new(12.0, 5.0), Align2::LEFT_TOP, "INPUT", FontId::monospace(8.0), rgba(SUN, 160.0));
    painter.text(rect.left_top() + Vec2::new(60.0, 5.0), Align2::LEFT_TOP, "ECHOES AS HEARD", FontId::monospace(8.0), rgba(MINT, 160.0));
    painter.text(rect.right_top() + Vec2::new(-12.0, 5.0), Align2::RIGHT_TOP, format!("last {WINDOW_S:.1}s"), FontId::monospace(8.0), TEXT_DIM);
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

fn knobs(ui: &mut Ui, params: &SkipParams, setter: &ParamSetter) {
    let (response, painter) = ui.allocate_painter(Vec2::new(ui.available_width(), KNOB_H), Sense::hover());
    let row = response.rect;
    let cells: [(&FloatParam, &str, [f32; 3]); 12] = [
        (&params.time, "TIME", MINT),
        (&params.feedback, "FEEDBACK", MINT),
        (&params.pingpong, "PING-PONG", MINT),
        (&params.mod_depth, "WOW", MINT),
        (&params.mod_rate, "RATE", MINT),
        (&params.hp, "LO CUT", PINK),
        (&params.lp, "HI CUT", PINK),
        (&params.drive, "DRIVE", PINK),
        (&params.diffusion, "DIFFUSE", PINK),
        (&params.pitch, "PITCH", PINK),
        (&params.duck, "DUCK", SUN),
        (&params.mix, "MIX", TEXT),
    ];
    let width = row.width() / cells.len() as f32;
    for (i, (param, label, color)) in cells.iter().enumerate() {
        let cell = Rect::from_min_size(Pos2::new(row.left() + i as f32 * width, row.top()), Vec2::new(width, KNOB_H));
        // The time knob is the note length's job while synced.
        let locked = *label == "TIME" && params.sync.value();
        let text = if locked { format!("{:.0} ms", params.time_ms(None)) } else { param.to_string() };
        knob(ui, &painter, cell, setter, param, label, *color, &text, locked);
    }
}

#[allow(clippy::too_many_arguments)]
fn knob(ui: &mut Ui, painter: &Painter, cell: Rect, setter: &ParamSetter, param: &FloatParam, label: &str, color: [f32; 3], text: &str, locked: bool) {
    let r = ui.interact(cell, ui.id().with(label), Sense::click_and_drag());
    if !locked {
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
    }

    let norm = param.unmodulated_normalized_value();
    let center = Pos2::new(cell.center().x, cell.top() + 26.0);
    let radius = 19.0;
    let (start, end) = (PI * 0.75, PI * 2.25);
    let lit = (r.hovered() || r.dragged()) && !locked;
    let main = rgba(color, if locked { 90.0 } else { 255.0 });

    painter.circle_filled(center, radius - 4.0, if lit { PANEL } else { Color32::from_rgb(0x09, 0x12, 0x17) });
    painter.add(Shape::line(arc(center, radius, start, end), Stroke::new(2.5_f32, GRID)));
    // The pitch is bipolar and lights from the centre.
    let from: f32 = if label == "PITCH" { 0.5 } else { 0.0 };
    let (lo, hi) = (from.min(norm), from.max(norm));
    if hi - lo > 0.002 {
        let (a, b) = (start + (end - start) * lo, start + (end - start) * hi);
        painter.add(Shape::line(arc(center, radius, a, b), Stroke::new(7.0_f32, rgba(color, if locked { 8.0 } else { 30.0 }))));
        painter.add(Shape::line(arc(center, radius, a, b), Stroke::new(2.5_f32, main)));
    }
    let angle = start + (end - start) * norm;
    let dir = Vec2::new(angle.cos(), angle.sin());
    painter.line_segment([center + dir * (radius - 11.0), center + dir * (radius - 5.0)], Stroke::new(2.0_f32, rgba(TEXT, if locked { 90.0 } else { 255.0 })));

    painter.text(Pos2::new(cell.center().x, cell.top() + 55.0), Align2::CENTER_CENTER, label, FontId::monospace(8.5), TEXT_DIM);
    painter.text(Pos2::new(cell.center().x, cell.top() + 69.0), Align2::CENTER_CENTER, text, FontId::monospace(10.0), main);
}
