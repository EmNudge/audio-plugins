//! The editor: an abacus. Every tracked partial is a bead on a wire, left to right by log
//! frequency, strongest on the top wire. A bead's size and brightness are its level, and it drags a
//! trail behind it as it moves. Frozen partials are ringed in vermilion. Under the wires the
//! residual (what the sines do not explain) is a teal ridge.
//!
//! On the board: drag sideways to transpose every bead (shift for fine, double-click to reset);
//! drag the cut line up or down to set how many partials are kept.
//! Knobs: drag vertically (hold shift for fine), double-click to reset.

use nih_plug::prelude::*;
use nih_plug_egui::egui::{
    self, Align2, Color32, FontId, Mesh, Painter, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2,
};
use std::f32::consts::PI;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::dsp::{Meter, F_MAX, F_MIN, MAX_PARTIALS, NOISE_BANDS, SLOTS};
use crate::{presets, AbacusParams};

const BG: Color32 = Color32::from_rgb(0x14, 0x0e, 0x0b);
const PANEL: Color32 = Color32::from_rgb(0x22, 0x18, 0x13);
const GRID: Color32 = Color32::from_rgb(0x3a, 0x2b, 0x22);
const TEXT_DIM: Color32 = Color32::from_rgb(0x95, 0x82, 0x72);
const TEXT: [f32; 3] = [240.0, 230.0, 210.0];
const BONE: [f32; 3] = [236.0, 224.0, 196.0];
const VERMILION: [f32; 3] = [255.0, 78.0, 40.0];
const TEAL: [f32; 3] = [64.0, 214.0, 200.0];
const WOOD: [f32; 3] = [78.0, 54.0, 38.0];
const BOARD_TOP: [f32; 3] = [38.0, 27.0, 20.0];
const BOARD_BOTTOM: [f32; 3] = [24.0, 17.0, 13.0];

const KNOB_ROW: f32 = 100.0;
const TRAIL: usize = 36;
const TRAY: f32 = 46.0;
/// Level shown at the smallest and largest bead, dB.
const DB_LOW: f32 = -78.0;
const DB_HIGH: f32 = -8.0;
const SEMITONES: f32 = 24.0;

#[derive(Clone, Copy)]
struct Bead {
    db: f32,
    x: f32,
    y: f32,
    /// Position on the board in 0..1 (log frequency), kept while the bead fades out.
    pos: f32,
    trail: [Pos2; TRAIL],
    head: usize,
    len: usize,
}

impl Default for Bead {
    fn default() -> Self {
        Self { db: -120.0, x: 0.0, y: 0.0, pos: 0.0, trail: [Pos2::ZERO; TRAIL], head: 0, len: 0 }
    }
}

pub struct UiState {
    beads: Vec<Bead>,
    noise_db: [f32; NOISE_BANDS],
}

impl Default for UiState {
    fn default() -> Self {
        Self { beads: vec![Bead::default(); SLOTS], noise_db: [-120.0; NOISE_BANDS] }
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

/// Levels rise fast and fall slowly; each bead remembers where it has been.
fn follow(state: &mut UiState, meter: &Meter) {
    for (i, bead) in state.beads.iter_mut().enumerate() {
        let f = meter.freq[i].load(Ordering::Relaxed);
        let a = meter.amp[i].load(Ordering::Relaxed);
        let db = if a > 1e-6 { 20.0 * a.log10() } else { -120.0 };
        let fresh = bead.db < DB_LOW - 6.0;
        bead.db = if db > bead.db { db } else { (bead.db - 1.1).max(db) };
        if f > 1.0 && a > 1e-6 {
            let pos = ((f / F_MIN).ln() / (F_MAX / F_MIN).ln()).clamp(0.0, 1.0);
            bead.pos = if fresh { pos } else { bead.pos + (pos - bead.pos) * 0.5 };
        }
    }
    for (i, n) in state.noise_db.iter_mut().enumerate() {
        let db = meter.noise[i].load(Ordering::Relaxed);
        *n = if db > *n { db } else { (*n - 0.8).max(db) };
    }
}

/// Top edge of wire `r` (a fractional rank), as a 0..1 fraction of the wire area.
/// Strong partials get wide rows and the weak crowd together below, but never thinner than a hair.
fn row_edge(r: f32) -> f32 {
    let x = (r / MAX_PARTIALS as f32).clamp(0.0, 1.0);
    0.4 * x + 0.6 * x.sqrt()
}

/// Inverse of `row_edge`, by bisection.
fn rank_at(frac: f32) -> f32 {
    let (mut lo, mut hi) = (0.0f32, MAX_PARTIALS as f32);
    for _ in 0..24 {
        let mid = 0.5 * (lo + hi);
        if row_edge(mid) < frac {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

pub fn draw(ui: &mut Ui, params: &Arc<AbacusParams>, setter: &ParamSetter, meter: &Arc<Meter>, state: &mut UiState) {
    ui.ctx().request_repaint();
    follow(state, meter);

    header(ui, params, setter, state);
    ui.add_space(8.0);
    let board_height = ui.available_height() - KNOB_ROW - 10.0;
    board(ui, params, setter, state, board_height);
    ui.add_space(10.0);
    knobs(ui, params, setter);
}

fn toggle(ui: &mut Ui, painter: &Painter, rect: Rect, id: &str, param: &BoolParam, setter: &ParamSetter, label: &str) {
    let r = ui.interact(rect, ui.id().with(id), Sense::click());
    let on = param.value();
    if r.clicked() {
        setter.begin_set_parameter(param);
        setter.set_parameter(param, !on);
        setter.end_set_parameter(param);
    }
    let fill = if on { rgba(VERMILION, 55.0) } else if r.hovered() { PANEL } else { BG };
    painter.rect_filled(rect, 4.0, fill);
    painter.rect_stroke(rect, 4.0, Stroke::new(1.0_f32, if on { rgba(VERMILION, 255.0) } else { GRID }), egui::StrokeKind::Inside);
    // A little latch: the lamp is lit when frozen.
    let lamp = Pos2::new(rect.left() + 12.0, rect.center().y);
    painter.circle_filled(lamp, 3.0, if on { rgba(VERMILION, 255.0) } else { GRID });
    if on {
        painter.circle_filled(lamp, 6.0, rgba(VERMILION, 40.0));
    }
    painter.text(
        Pos2::new(rect.center().x + 6.0, rect.center().y),
        Align2::CENTER_CENTER,
        label,
        FontId::monospace(10.0),
        if on { rgba(VERMILION, 255.0) } else { TEXT_DIM },
    );
}

fn header(ui: &mut Ui, params: &AbacusParams, setter: &ParamSetter, state: &UiState) {
    let (response, painter) = ui.allocate_painter(Vec2::new(ui.available_width(), 24.0), Sense::hover());
    let rect = response.rect;
    painter.text(rect.left_center(), Align2::LEFT_CENTER, "A B A C U S", FontId::monospace(15.0), rgba(TEXT, 255.0));
    painter.text(rect.left_center() + Vec2::new(116.0, 1.0), Align2::LEFT_CENTER, "sine resynth", FontId::monospace(10.0), TEXT_DIM);

    let theme = preset_kit::ui::Theme { fill: PANEL, hover: GRID, ink: rgba(TEXT, 255.0), dim: TEXT_DIM };
    preset_kit::ui::picker(ui, Pos2::new(rect.left() + 250.0, rect.center().y - 11.0), 22.0, &presets::BANK, params, setter, &theme);

    let size = Vec2::new(112.0, 22.0);
    let amp = Rect::from_center_size(Pos2::new(rect.right() - size.x * 0.5, rect.center().y), size);
    let freq = amp.translate(Vec2::new(-size.x - 8.0, 0.0));
    toggle(ui, &painter, freq, "ffreeze", &params.freeze_freq, setter, "FREEZE FREQ");
    toggle(ui, &painter, amp, "afreeze", &params.freeze_amp, setter, "FREEZE AMP");

    let live = state.beads.iter().filter(|b| b.db > DB_LOW).count().min(params.partials.value() as usize);
    painter.text(
        Pos2::new(freq.left() - 14.0, rect.center().y),
        Align2::RIGHT_CENTER,
        format!("{live} beads"),
        FontId::monospace(10.0),
        TEXT_DIM,
    );
}

fn board(ui: &mut Ui, params: &Arc<AbacusParams>, setter: &ParamSetter, state: &mut UiState, height: f32) {
    let size = Vec2::new(ui.available_width(), height);
    let (response, painter) = ui.allocate_painter(size, Sense::hover());
    let rect = response.rect;
    let post = 14.0;
    let axis = 16.0;
    let wires = Rect::from_min_max(
        Pos2::new(rect.left() + post, rect.top() + 12.0),
        Pos2::new(rect.right() - post, rect.bottom() - TRAY - axis - 6.0),
    );
    let tray = Rect::from_min_max(Pos2::new(wires.left(), wires.bottom() + 6.0), Pos2::new(wires.right(), rect.bottom() - axis));
    let ln_span = (F_MAX / F_MIN).ln();
    let x_of = |pos: f32| wires.left() + pos * wires.width();
    let y_edge = |r: f32| wires.top() + row_edge(r) * wires.height();
    let n = params.partials.value() as f32;
    let octave_px = wires.width() / (ln_span / std::f32::consts::LN_2);

    // Interaction. The board first (sideways drag is a transpose), the cut line on top of it.
    let board_hit = ui.interact(Rect::from_min_max(wires.min, Pos2::new(wires.right(), tray.bottom())), ui.id().with("board"), Sense::click_and_drag());
    let pitch = &params.pitch;
    if board_hit.drag_started() {
        setter.begin_set_parameter(pitch);
    }
    if board_hit.dragged() {
        let fine = ui.input(|i| i.modifiers.shift);
        let st = board_hit.drag_delta().x / octave_px * 12.0 * if fine { 0.1 } else { 1.0 };
        setter.set_parameter(pitch, (pitch.value() + st).clamp(-SEMITONES, SEMITONES));
    }
    if board_hit.drag_stopped() {
        setter.end_set_parameter(pitch);
    }
    if board_hit.double_clicked() {
        setter.begin_set_parameter(pitch);
        setter.set_parameter(pitch, pitch.default_plain_value());
        setter.end_set_parameter(pitch);
    }
    let cut_y = y_edge(n);
    let grab = Rect::from_min_max(Pos2::new(rect.left(), cut_y - 7.0), Pos2::new(rect.right(), cut_y + 7.0));
    let cut_hit = ui.interact(grab, ui.id().with("cut"), Sense::click_and_drag());
    if cut_hit.drag_started() {
        setter.begin_set_parameter(&params.partials);
    }
    if cut_hit.dragged() {
        if let Some(pos) = cut_hit.interact_pointer_pos() {
            let rank = rank_at((pos.y - wires.top()) / wires.height()).round().clamp(1.0, MAX_PARTIALS as f32);
            setter.set_parameter(&params.partials, rank as i32);
        }
    }
    if cut_hit.drag_stopped() {
        setter.end_set_parameter(&params.partials);
    }
    if cut_hit.double_clicked() {
        setter.begin_set_parameter(&params.partials);
        setter.set_parameter(&params.partials, params.partials.default_plain_value());
        setter.end_set_parameter(&params.partials);
    }
    let cut_active = cut_hit.hovered() || cut_hit.dragged();
    let pitch_active = board_hit.dragged() && !cut_hit.dragged();

    // The frame: two posts and a board between them.
    painter.rect_filled(rect, 6.0, rgba(WOOD, 255.0));
    painter.rect_filled(rect.shrink(2.0), 5.0, rgba(mix(WOOD, [0.0; 3], 0.45), 255.0));
    for x in [rect.left() + post * 0.5, rect.right() - post * 0.5] {
        for k in 0..4 {
            let y = rect.top() + 24.0 + k as f32 * (rect.height() - 48.0) / 3.0;
            painter.circle_filled(Pos2::new(x, y), 2.5, rgba(mix(WOOD, [0.0; 3], 0.7), 255.0));
            painter.circle_filled(Pos2::new(x - 0.6, y - 0.6), 1.2, rgba(mix(WOOD, TEXT, 0.25), 255.0));
        }
    }
    let inner = Rect::from_min_max(Pos2::new(wires.left(), rect.top() + 4.0), Pos2::new(wires.right(), rect.bottom() - 4.0));
    strip(
        &painter,
        &[
            (inner.left_top(), rgba(BOARD_TOP, 255.0), inner.left_bottom(), rgba(BOARD_BOTTOM, 255.0)),
            (inner.right_top(), rgba(BOARD_TOP, 255.0), inner.right_bottom(), rgba(BOARD_BOTTOM, 255.0)),
        ],
    );

    // Frequency scale, as faint verticals through every wire.
    for (f, label) in [(50.0, "50"), (100.0, "100"), (200.0, "200"), (500.0, "500"), (1000.0, "1k"), (2000.0, "2k"), (5000.0, "5k"), (10000.0, "10k")] {
        let x = x_of((f / F_MIN as f32).ln() / ln_span);
        painter.line_segment([Pos2::new(x, wires.top()), Pos2::new(x, tray.bottom())], Stroke::new(1.0_f32, rgba([255.0, 240.0, 220.0], 7.0)));
        painter.text(Pos2::new(x, tray.bottom() + 4.0), Align2::CENTER_TOP, label, FontId::monospace(9.0), TEXT_DIM);
    }

    // Wires: one per rank; the ones past the cut are dim.
    for r in 0..MAX_PARTIALS {
        let y = wires.top() + row_edge(r as f32 + 0.5) * wires.height();
        let kept = (r as f32) < n;
        let a = if kept { 120.0 } else { 36.0 };
        painter.line_segment([Pos2::new(wires.left(), y), Pos2::new(wires.right(), y)], Stroke::new(1.0_f32, rgba(mix(WOOD, TEXT, 0.35), a)));
    }

    // Rank the beads by level, strongest first.
    let mut order: [(f32, usize); SLOTS] = std::array::from_fn(|i| (state.beads[i].db, i));
    order.sort_by(|a, b| b.0.total_cmp(&a.0));
    let frozen_amp = params.freeze_amp.value();
    let frozen_freq = params.freeze_freq.value();
    let mut rank = 0usize;
    let mut drawn: Vec<(usize, f32)> = Vec::with_capacity(SLOTS);
    let n_rows = params.partials.value() as usize;
    for (db, i) in order {
        // Past the cut there is nothing to show but partials fading out.
        if db <= DB_LOW - 4.0 || rank >= n_rows {
            break;
        }
        drawn.push((i, rank as f32));
        rank += 1;
    }

    // Trails first so beads sit on top.
    for &(i, r) in &drawn {
        let bead = &mut state.beads[i];
        let target_y = wires.top() + row_edge(r + 0.5) * wires.height();
        let target_x = x_of(bead.pos);
        if bead.len == 0 {
            bead.x = target_x;
            bead.y = target_y;
        }
        bead.x += (target_x - bead.x) * 0.6;
        bead.y += (target_y - bead.y) * 0.3;
        bead.trail[bead.head] = Pos2::new(bead.x, bead.y);
        bead.head = (bead.head + 1) % TRAIL;
        bead.len = (bead.len + 1).min(TRAIL);
    }
    for (i, bead) in state.beads.iter_mut().enumerate() {
        if !drawn.iter().any(|(j, _)| *j == i) {
            bead.len = 0;
        }
    }
    for &(i, r) in &drawn {
        let bead = &state.beads[i];
        let level = ((bead.db - DB_LOW) / (DB_HIGH - DB_LOW)).clamp(0.0, 1.0);
        let pitch_row = wires.height() * (row_edge(r + 1.0) - row_edge(r)).max(0.002);
        let radius = (pitch_row * 0.46).clamp(1.5, 13.0) * (0.4 + 0.6 * level);
        for k in 1..bead.len {
            let a = bead.trail[(bead.head + TRAIL - k) % TRAIL];
            let b = bead.trail[(bead.head + TRAIL - k - 1) % TRAIL];
            if (a - b).length() < 0.2 {
                continue;
            }
            let fade = 1.0 - k as f32 / TRAIL as f32;
            painter.line_segment([a, b], Stroke::new((radius * 1.3 * fade).max(0.8), rgba(BONE, 70.0 * fade * level)));
        }
    }

    // Residual tray.
    painter.rect_filled(tray, 3.0, Color32::from_rgba_unmultiplied(0, 0, 0, 70));
    painter.line_segment([tray.left_bottom(), tray.right_bottom()], Stroke::new(1.0_f32, GRID));
    let ridge: Vec<Pos2> = (0..NOISE_BANDS)
        .map(|b| {
            let x = tray.left() + (b as f32 + 0.5) / NOISE_BANDS as f32 * tray.width();
            let lvl = ((state.noise_db[b] + 100.0) / 70.0).clamp(0.0, 1.0);
            Pos2::new(x, tray.bottom() - lvl * (tray.height() - 4.0))
        })
        .collect();
    strip(
        &painter,
        &ridge.iter().map(|p| (*p, rgba(TEAL, 110.0), Pos2::new(p.x, tray.bottom()), rgba(TEAL, 0.0))).collect::<Vec<_>>(),
    );
    painter.add(Shape::line(ridge.clone(), Stroke::new(4.0_f32, rgba(TEAL, 30.0))));
    painter.add(Shape::line(ridge, Stroke::new(1.25_f32, rgba(TEAL, 230.0))));
    painter.text(tray.left_top() + Vec2::new(5.0, 3.0), Align2::LEFT_TOP, "residual", FontId::monospace(8.0), rgba(TEAL, 150.0));

    // Beads.
    for &(i, r) in &drawn {
        let bead = &state.beads[i];
        let level = ((bead.db - DB_LOW) / (DB_HIGH - DB_LOW)).clamp(0.0, 1.0);
        let pitch_row = wires.height() * (row_edge(r + 1.0) - row_edge(r)).max(0.002);
        let radius = (pitch_row * 0.46).clamp(1.5, 13.0) * (0.4 + 0.6 * level);
        let center = Pos2::new(bead.x, bead.y);
        let beyond = r >= n;
        let tone = mix(WOOD, BONE, 0.35 + 0.65 * level);
        let dim = if beyond { 0.45 } else { 1.0 };
        painter.circle_filled(center, radius * 2.4, rgba(BONE, 28.0 * level * dim));
        painter.circle_filled(center, radius, rgba(tone, 255.0 * dim));
        painter.circle_filled(center + Vec2::new(-radius * 0.3, -radius * 0.3), radius * 0.38, rgba([255.0, 252.0, 240.0], 150.0 * level * dim));
        if frozen_amp {
            painter.circle_stroke(center, radius + 1.8, Stroke::new(1.2_f32, rgba(VERMILION, 255.0 * dim)));
            painter.circle_stroke(center, radius + 3.6, Stroke::new(2.5_f32, rgba(VERMILION, 28.0 * dim)));
        }
        if frozen_freq {
            // A pin through the wire: the bead is not going anywhere.
            let h = radius + 5.0;
            painter.line_segment([center + Vec2::new(0.0, -h), center + Vec2::new(0.0, h)], Stroke::new(1.5_f32, rgba(VERMILION, 220.0 * dim)));
        }
    }

    // Everything past the cut is out: dim it and draw the line.
    let veil = Rect::from_min_max(Pos2::new(wires.left(), cut_y), Pos2::new(wires.right(), wires.bottom()));
    painter.rect_filled(veil, 0.0, Color32::from_rgba_unmultiplied(10, 6, 4, 110));
    let line_a = if cut_active { 255.0 } else { 150.0 };
    let mut x = wires.left();
    while x < wires.right() {
        painter.line_segment([Pos2::new(x, cut_y), Pos2::new((x + 7.0).min(wires.right()), cut_y)], Stroke::new(1.5_f32, rgba(BONE, line_a)));
        x += 12.0;
    }
    let tab = Rect::from_center_size(Pos2::new(wires.left() + 20.0, cut_y), Vec2::new(32.0, 14.0));
    painter.rect_filled(tab, 3.0, rgba(BONE, line_a));
    painter.text(tab.center(), Align2::CENTER_CENTER, format!("{}", n as i32), FontId::monospace(10.0), BG);
    if cut_active {
        painter.text(tab.right_center() + Vec2::new(8.0, 0.0), Align2::LEFT_CENTER, "PARTIALS", FontId::monospace(9.0), rgba(BONE, 255.0));
    }

    // The value being dragged, big.
    if pitch_active {
        painter.text(
            Pos2::new(wires.center().x, wires.top() + 30.0),
            Align2::CENTER_CENTER,
            format!("{:+.2} st", pitch.value()),
            FontId::monospace(34.0),
            rgba(BONE, 230.0),
        );
    } else if pitch.value().abs() > 0.005 {
        painter.text(
            Pos2::new(wires.right() - 8.0, wires.top() + 6.0),
            Align2::RIGHT_TOP,
            format!("{:+.2} st", pitch.value()),
            FontId::monospace(10.0),
            rgba(BONE, 200.0),
        );
    }
}

fn knobs(ui: &mut Ui, params: &AbacusParams, setter: &ParamSetter) {
    let (response, painter) = ui.allocate_painter(Vec2::new(ui.available_width(), KNOB_ROW), Sense::hover());
    let row = response.rect;
    let bone = rgba(BONE, 255.0);
    let width = row.width() / 8.0;
    let cell = |i: usize| Rect::from_min_size(Pos2::new(row.left() + i as f32 * width, row.top()), Vec2::new(width, KNOB_ROW));
    knob(ui, &painter, cell(0), setter, &params.partials, "PARTIALS", bone, false);
    knob(ui, &painter, cell(1), setter, &params.resolution, "RESOLUTION", bone, false);
    knob(ui, &painter, cell(2), setter, &params.glide, "GLIDE", bone, false);
    knob(ui, &painter, cell(3), setter, &params.pitch, "PITCH", bone, true);
    knob(ui, &painter, cell(4), setter, &params.stretch, "STRETCH", bone, true);
    knob(ui, &painter, cell(5), setter, &params.noise, "NOISE", rgba(TEAL, 255.0), false);
    knob(ui, &painter, cell(6), setter, &params.spread, "SPREAD", bone, false);
    knob(ui, &painter, cell(7), setter, &params.mix, "MIX", rgba(TEXT, 255.0), false);
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
fn knob<P: Param>(ui: &mut Ui, painter: &Painter, cell: Rect, setter: &ParamSetter, param: &P, label: &str, color: Color32, bipolar: bool) {
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

    painter.circle_filled(center, radius - 5.0, if lit { PANEL } else { Color32::from_rgb(0x1a, 0x12, 0x0e) });
    painter.add(Shape::line(arc(center, radius, start, end), Stroke::new(3.0_f32, GRID)));
    let from = if bipolar { start + (end - start) * 0.5 } else { start };
    let to = start + (end - start) * norm;
    if (to - from).abs() > 0.01 {
        painter.add(Shape::line(arc(center, radius, from, to), Stroke::new(8.0_f32, rgba(rgb, 30.0))));
        painter.add(Shape::line(arc(center, radius, from, to), Stroke::new(3.0_f32, color)));
    }
    let dir = Vec2::new(to.cos(), to.sin());
    painter.line_segment([center + dir * (radius - 14.0), center + dir * (radius - 7.0)], Stroke::new(2.0_f32, rgba(TEXT, 255.0)));

    painter.text(Pos2::new(cell.center().x, cell.top() + 69.0), Align2::CENTER_CENTER, label, FontId::monospace(9.0), TEXT_DIM);
    painter.text(Pos2::new(cell.center().x, cell.top() + 85.0), Align2::CENTER_CENTER, param.to_string(), FontId::monospace(11.0), color);
}
