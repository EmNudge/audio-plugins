//! The editor: a rack of four glass microscope slides.
//!
//! Each slide holds a captured spectrum as a pink silhouette. Above the rack, the stage shows what
//! comes out as an acid-yellow shape that morphs between the slides, over the live input in cyan,
//! with the two slides being blended ghosted behind it. Click a slide to capture the input into
//! it (right-click clears), and drag across the rack or along the rail to move the objective lens,
//! which is the Position. Knobs: drag vertically (hold shift for fine), double-click to reset.

use nih_plug::prelude::*;
use nih_plug_egui::egui::{
    self, Align2, Color32, FontId, Mesh, Painter, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, Vec2,
};
use std::f32::consts::PI;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::dsp::{plan, Meter, BANDS, F_HI, F_LO, SLOTS};
use crate::{presets, EvolveMode, Mode, SpecimenParams};

const BG: Color32 = Color32::from_rgb(0x0a, 0x0f, 0x24);
const PANEL: Color32 = Color32::from_rgb(0x13, 0x1a, 0x3a);
const GRID: Color32 = Color32::from_rgb(0x25, 0x30, 0x5c);
const TEXT_DIM: Color32 = Color32::from_rgb(0x6f, 0x7a, 0xa8);
const TEXT: [f32; 3] = [232.0, 236.0, 255.0];
const GLOW_TOP: [f32; 3] = [20.0, 28.0, 66.0];
const GLOW_BOTTOM: [f32; 3] = [10.0, 15.0, 36.0];
const YELLOW: [f32; 3] = [230.0, 255.0, 46.0];
const PINK: [f32; 3] = [255.0, 61.0, 154.0];
const CYAN: [f32; 3] = [53.0, 216.0, 255.0];

const DB_FLOOR: f32 = -100.0;
const DB_RANGE: f32 = 86.0;
const KNOB_ROW: f32 = 100.0;
const RACK_H: f32 = 176.0;
const SLIDE_H: f32 = 124.0;
const SLIDE_GAP: f32 = 14.0;

pub struct UiState {
    input: [f32; BANDS],
    output: [f32; BANDS],
    slots: [[f32; BANDS]; SLOTS],
    filled: [bool; SLOTS],
    progress: [f32; SLOTS],
    flash: [f32; SLOTS],
    position: f32,
    holding: bool,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            input: [-120.0; BANDS],
            output: [-120.0; BANDS],
            slots: [[-120.0; BANDS]; SLOTS],
            filled: [false; SLOTS],
            progress: [0.0; SLOTS],
            flash: [0.0; SLOTS],
            position: 0.0,
            holding: false,
        }
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

/// Rise fast, fall slowly, so peaks stay readable.
fn follow(state: &mut UiState, meter: &Meter) {
    for i in 0..BANDS {
        let db = meter.input[i].load(Ordering::Relaxed);
        state.input[i] = if db > state.input[i] { db } else { (state.input[i] - 1.2).max(db) };
        let db = meter.output[i].load(Ordering::Relaxed);
        state.output[i] = if db > state.output[i] { db } else { (state.output[i] - 1.2).max(db) };
    }
    for s in 0..SLOTS {
        for b in 0..BANDS {
            state.slots[s][b] = meter.slots[s][b].load(Ordering::Relaxed);
        }
        let filled = meter.filled[s].load(Ordering::Relaxed);
        if filled && !state.filled[s] {
            state.flash[s] = 1.0;
        }
        state.flash[s] = (state.flash[s] - 0.03).max(0.0);
        state.filled[s] = filled;
        state.progress[s] = meter.progress[s].load(Ordering::Relaxed);
    }
    state.position = meter.position.load(Ordering::Relaxed);
    state.holding = meter.holding.load(Ordering::Relaxed);
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

/// A filled spectrum with a glowing edge: gradient from `colour` at the line to clear at `bottom`.
fn silhouette(painter: &Painter, line: &[Pos2], bottom: f32, colour: [f32; 3], fill: f32, glow: f32, edge: f32) {
    strip(
        painter,
        &line.iter().map(|p| (*p, rgba(colour, fill), Pos2::new(p.x, bottom), rgba(colour, 0.0))).collect::<Vec<_>>(),
    );
    painter.add(Shape::line(line.to_vec(), Stroke::new(5.0_f32, rgba(colour, glow))));
    painter.add(Shape::line(line.to_vec(), Stroke::new(1.5_f32, rgba(colour, edge))));
}

pub fn draw(ui: &mut Ui, params: &Arc<SpecimenParams>, setter: &ParamSetter, meter: &Arc<Meter>, state: &mut UiState) {
    ui.ctx().request_repaint();
    follow(state, meter);

    header(ui, params, setter);
    ui.add_space(8.0);
    let stage_height = ui.available_height() - RACK_H - KNOB_ROW - 20.0;
    stage(ui, params, state, stage_height);
    ui.add_space(10.0);
    rack(ui, params, setter, meter, state);
    ui.add_space(10.0);
    knobs(ui, params, setter);
}

fn toggle(ui: &mut Ui, painter: &Painter, rect: Rect, id: &str, label: &str, colour: [f32; 3], lit: bool) -> Response {
    let r = ui.interact(rect, ui.id().with(id), Sense::click());
    let fill = if lit { rgba(colour, 55.0) } else if r.hovered() { PANEL } else { BG };
    painter.rect_filled(rect, 4.0, fill);
    painter.rect_stroke(rect, 4.0, Stroke::new(1.0_f32, if lit { rgba(colour, 255.0) } else { GRID }), egui::StrokeKind::Inside);
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::monospace(10.0),
        if lit { rgba(colour, 255.0) } else if r.hovered() { rgba(TEXT, 255.0) } else { TEXT_DIM },
    );
    r
}

fn header(ui: &mut Ui, params: &SpecimenParams, setter: &ParamSetter) {
    let (response, painter) = ui.allocate_painter(Vec2::new(ui.available_width(), 24.0), Sense::hover());
    let rect = response.rect;
    painter.text(rect.left_center(), Align2::LEFT_CENTER, "S P E C I M E N", FontId::monospace(15.0), rgba(TEXT, 255.0));
    painter.text(rect.left_center() + Vec2::new(142.0, 1.0), Align2::LEFT_CENTER, "spectral freeze", FontId::monospace(10.0), TEXT_DIM);

    let theme = preset_kit::ui::Theme { fill: PANEL, hover: GRID, ink: rgba(TEXT, 255.0), dim: TEXT_DIM };
    preset_kit::ui::picker(ui, Pos2::new(rect.left() + 292.0, rect.center().y - 11.0), 22.0, &presets::BANK, params, setter, &theme);

    let cy = rect.center().y;
    let button = |right: f32, w: f32| Rect::from_center_size(Pos2::new(right - w / 2.0, cy), Vec2::new(w, 22.0));

    let freeze = button(rect.right(), 70.0);
    let on = params.freeze.value();
    if toggle(ui, &painter, freeze, "freeze", "FREEZE", YELLOW, on).clicked() {
        setter.begin_set_parameter(&params.freeze);
        setter.set_parameter(&params.freeze, !on);
        setter.end_set_parameter(&params.freeze);
    }

    let evolve = button(freeze.left() - 6.0, 84.0);
    let mode_now = params.evolve.value();
    let label = match mode_now {
        EvolveMode::Off => "EVOLVE OFF",
        EvolveMode::Sine => "EVOLVE SIN",
        EvolveMode::Walk => "EVOLVE RND",
    };
    if toggle(ui, &painter, evolve, "evolve", label, YELLOW, mode_now != EvolveMode::Off).clicked() {
        let next = match mode_now {
            EvolveMode::Off => EvolveMode::Sine,
            EvolveMode::Sine => EvolveMode::Walk,
            EvolveMode::Walk => EvolveMode::Off,
        };
        setter.begin_set_parameter(&params.evolve);
        setter.set_parameter(&params.evolve, next);
        setter.end_set_parameter(&params.evolve);
    }

    let mode = button(evolve.left() - 6.0, 84.0);
    let resynth = params.mode.value() == Mode::Resynth;
    let (label, colour) = if resynth { ("RESYNTH", PINK) } else { ("CROSS", CYAN) };
    if toggle(ui, &painter, mode, "mode", label, colour, true).clicked() {
        setter.begin_set_parameter(&params.mode);
        setter.set_parameter(&params.mode, if resynth { Mode::Cross } else { Mode::Resynth });
        setter.end_set_parameter(&params.mode);
    }
}

fn band_x(rect: Rect, b: usize) -> f32 {
    rect.left() + (b as f32 + 0.5) / BANDS as f32 * rect.width()
}

fn stage(ui: &mut Ui, params: &SpecimenParams, state: &UiState, height: f32) {
    let (response, painter) = ui.allocate_painter(Vec2::new(ui.available_width(), height), Sense::hover());
    let rect = response.rect;
    let bottom = rect.bottom() - 18.0;
    let top = rect.top() + 8.0;
    let h = bottom - top;
    let y_of = |db: f32| bottom - ((db - DB_FLOOR) / DB_RANGE).clamp(0.0, 1.0) * h;
    let ln_span = (F_HI / F_LO).ln();
    let x_of = |f: f32| rect.left() + (f / F_LO).ln() / ln_span * rect.width();

    strip(
        &painter,
        &[
            (rect.left_top(), rgba(GLOW_TOP, 255.0), rect.left_bottom(), rgba(GLOW_BOTTOM, 255.0)),
            (rect.right_top(), rgba(GLOW_TOP, 255.0), rect.right_bottom(), rgba(GLOW_BOTTOM, 255.0)),
        ],
    );
    painter.rect_stroke(rect, 4.0, Stroke::new(1.0_f32, GRID), egui::StrokeKind::Inside);
    for db in [-20.0, -40.0, -60.0, -80.0] {
        let y = y_of(db);
        painter.line_segment([Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)], Stroke::new(1.0_f32, rgba([255.0, 255.0, 255.0], 8.0)));
        painter.text(Pos2::new(rect.left() + 6.0, y - 2.0), Align2::LEFT_BOTTOM, format!("{db:.0}"), FontId::monospace(8.0), TEXT_DIM);
    }
    for (f, label) in [(50.0, "50"), (100.0, "100"), (200.0, "200"), (500.0, "500"), (1000.0, "1k"), (2000.0, "2k"), (5000.0, "5k"), (10000.0, "10k")] {
        let x = x_of(f);
        painter.line_segment([Pos2::new(x, top), Pos2::new(x, bottom)], Stroke::new(1.0_f32, rgba([255.0, 255.0, 255.0], 6.0)));
        painter.text(Pos2::new(x, bottom + 5.0), Align2::CENTER_TOP, label, FontId::monospace(9.0), TEXT_DIM);
    }
    painter.line_segment([Pos2::new(rect.left(), bottom), Pos2::new(rect.right(), bottom)], Stroke::new(1.0_f32, GRID));

    let line = |values: &[f32; BANDS]| -> Vec<Pos2> {
        let v = blur(values);
        (0..BANDS).map(|b| Pos2::new(band_x(rect, b), y_of(v[b]))).collect()
    };

    // The two slides being blended, ghosted by how much of each is in the output.
    let pos = state.position;
    let blend = plan(state.filled, pos, params.curve.value());
    if let Some(p) = blend {
        let weights: &[(usize, f32)] = if p.a == p.b { &[(p.a, 1.0)] } else { &[(p.a, 1.0 - p.t), (p.b, p.t)] };
        for (slot, w) in weights.iter() {
            let l = line(&state.slots[*slot]);
            painter.add(Shape::line(l.clone(), Stroke::new(4.0_f32, rgba(PINK, 18.0 * w))));
            painter.add(Shape::line(l, Stroke::new(1.0_f32, rgba(PINK, 60.0 + 150.0 * w))));
        }
    }

    // Output: acid yellow. Input: cyan, thin.
    let out = line(&state.output);
    silhouette(&painter, &out, bottom, YELLOW, 120.0, 34.0, 240.0);
    let live = line(&state.input);
    painter.add(Shape::line(live.clone(), Stroke::new(4.0_f32, rgba(CYAN, 22.0))));
    painter.add(Shape::line(live, Stroke::new(1.0_f32, rgba(CYAN, 190.0))));

    // Legend, and the position as a big numeral.
    let mut x = rect.left() + 38.0;
    for (label, colour) in [("OUTPUT", YELLOW), ("INPUT", CYAN), ("SLIDES", PINK)] {
        painter.rect_filled(Rect::from_center_size(Pos2::new(x, rect.top() + 17.0), Vec2::splat(7.0)), 1.0, rgba(colour, 255.0));
        let galley = painter.text(Pos2::new(x + 8.0, rect.top() + 17.0), Align2::LEFT_CENTER, label, FontId::monospace(9.0), TEXT_DIM);
        x += 8.0 + galley.width() + 16.0;
    }
    let coordinate = pos * (SLOTS - 1) as f32 + 1.0;
    painter.text(Pos2::new(rect.right() - 16.0, rect.top() + 28.0), Align2::RIGHT_CENTER, format!("{coordinate:.2}"), FontId::monospace(30.0), rgba(YELLOW, 255.0));
    let caption = match blend {
        None => "no slides: dry".to_owned(),
        Some(p) if p.a == p.b => format!("slide {}", p.a + 1),
        Some(p) => format!("slide {} > {}  {:.0}%", p.a + 1, p.b + 1, p.t * 100.0),
    };
    painter.text(Pos2::new(rect.right() - 16.0, rect.top() + 50.0), Align2::RIGHT_CENTER, caption, FontId::monospace(9.0), TEXT_DIM);
    if state.holding {
        painter.text(Pos2::new(rect.right() - 16.0, rect.top() + 64.0), Align2::RIGHT_CENTER, "FROZEN", FontId::monospace(9.0), rgba(YELLOW, 255.0));
    }
}

fn rack(ui: &mut Ui, params: &SpecimenParams, setter: &ParamSetter, meter: &Meter, state: &UiState) {
    let (response, painter) = ui.allocate_painter(Vec2::new(ui.available_width(), RACK_H), Sense::hover());
    let rect = response.rect;
    let sw = (rect.width() - SLIDE_GAP * 3.0) / 4.0;
    let slide_rect = |i: usize| Rect::from_min_size(Pos2::new(rect.left() + i as f32 * (sw + SLIDE_GAP), rect.top()), Vec2::new(sw, SLIDE_H));
    let c0 = slide_rect(0).center().x;
    let c3 = slide_rect(3).center().x;
    let x_of_pos = |p: f32| c0 + p * (c3 - c0);
    let pos_of_x = |x: f32| ((x - c0) / (c3 - c0)).clamp(0.0, 1.0);
    let blend = plan(state.filled, state.position, params.curve.value());
    let weight = |i: usize| match blend {
        Some(p) if p.a == p.b => (p.a == i) as u32 as f32,
        Some(p) if p.a == i => 1.0 - p.t,
        Some(p) if p.b == i => p.t,
        _ => 0.0,
    };

    // Dragging anywhere on the rack moves the lens; a click on a slide captures.
    let drag = |r: &Response| {
        if r.drag_started() {
            setter.begin_set_parameter(&params.position);
        }
        if r.dragged() {
            if let Some(pos) = r.interact_pointer_pos() {
                setter.set_parameter(&params.position, pos_of_x(pos.x));
            }
        }
        if r.drag_stopped() {
            setter.end_set_parameter(&params.position);
        }
    };

    for i in 0..SLOTS {
        let r = slide_rect(i);
        let resp = ui.interact(r, ui.id().with(("slide", i)), Sense::click_and_drag());
        drag(&resp);
        if resp.clicked() {
            meter.request_capture(i);
        }
        if resp.secondary_clicked() {
            meter.request_clear(i);
        }
        slide(&painter, r, i, state, weight(i), resp.hovered());
    }

    // The rail with the objective lens.
    let rail_y = rect.top() + SLIDE_H + 22.0;
    let rail = Rect::from_min_max(Pos2::new(rect.left(), rail_y - 14.0), Pos2::new(rect.right(), rail_y + 14.0));
    let resp = ui.interact(rail, ui.id().with("rail"), Sense::click_and_drag());
    drag(&resp);
    if resp.double_clicked() {
        setter.begin_set_parameter(&params.position);
        setter.set_parameter(&params.position, params.position.default_plain_value());
        setter.end_set_parameter(&params.position);
    }
    painter.line_segment([Pos2::new(c0, rail_y), Pos2::new(c3, rail_y)], Stroke::new(2.0_f32, GRID));
    for i in 0..SLOTS {
        let x = slide_rect(i).center().x;
        painter.line_segment([Pos2::new(x, rail_y - 5.0), Pos2::new(x, rail_y + 5.0)], Stroke::new(2.0_f32, if state.filled[i] { rgba(PINK, 220.0) } else { GRID }));
    }
    painter.text(Pos2::new(rect.left(), rail_y + 19.0), Align2::LEFT_CENTER, "POSITION", FontId::monospace(9.0), TEXT_DIM);
    painter.text(
        Pos2::new(rect.right(), rail_y + 19.0),
        Align2::RIGHT_CENTER,
        "drag to morph / click capture / right-click clear",
        FontId::monospace(9.0),
        TEXT_DIM,
    );

    // The base position, when evolve is moving the lens away from it.
    if params.evolve.value() != EvolveMode::Off {
        let bx = x_of_pos(params.position.value());
        painter.circle_stroke(Pos2::new(bx, rail_y), 6.0, Stroke::new(1.0_f32, rgba(YELLOW, 110.0)));
    }
    let lens = x_of_pos(state.position);
    let active = resp.hovered() || resp.dragged();
    let beam_top = rect.top() + 4.0;
    painter.line_segment([Pos2::new(lens, beam_top), Pos2::new(lens, rail_y)], Stroke::new(7.0_f32, rgba(YELLOW, 14.0)));
    painter.line_segment([Pos2::new(lens, beam_top), Pos2::new(lens, rail_y)], Stroke::new(1.0_f32, rgba(YELLOW, 150.0)));
    painter.circle_stroke(Pos2::new(lens, rail_y), 15.0, Stroke::new(5.0_f32, rgba(YELLOW, if active { 40.0 } else { 20.0 })));
    painter.circle_filled(Pos2::new(lens, rail_y), 11.0, BG);
    painter.circle_stroke(Pos2::new(lens, rail_y), 11.0, Stroke::new(2.0_f32, rgba(YELLOW, 255.0)));
    painter.circle_filled(Pos2::new(lens, rail_y), 3.5, rgba(YELLOW, 255.0));
}

fn slide(painter: &Painter, r: Rect, i: usize, state: &UiState, weight: f32, hovered: bool) {
    let filled = state.filled[i];
    let progress = state.progress[i];
    let capturing = progress > 0.0;
    let flash = state.flash[i];

    // Glass: a slightly lit slab with a bright top edge and a corner sheen.
    strip(
        painter,
        &[
            (r.left_top(), rgba(mix([22.0, 31.0, 70.0], PINK, 0.06), 255.0), r.left_bottom(), rgba([14.0, 20.0, 46.0], 255.0)),
            (r.right_top(), rgba(mix([22.0, 31.0, 70.0], PINK, 0.06), 255.0), r.right_bottom(), rgba([14.0, 20.0, 46.0], 255.0)),
        ],
    );
    let sheen = vec![r.left_top(), r.left_top() + Vec2::new(r.width() * 0.42, 0.0), r.left_top() + Vec2::new(0.0, r.height() * 0.55)];
    painter.add(Shape::convex_polygon(sheen, Color32::from_white_alpha(9), Stroke::NONE));
    painter.line_segment([r.left_top() + Vec2::new(2.0, 0.5), r.right_top() + Vec2::new(-2.0, 0.5)], Stroke::new(1.0_f32, Color32::from_white_alpha(34)));

    let border = if filled {
        rgba(PINK, 70.0 + 185.0 * weight + if hovered { 30.0 } else { 0.0 })
    } else if hovered {
        rgba(PINK, 150.0)
    } else {
        GRID
    };
    painter.rect_stroke(r, 4.0, Stroke::new(1.0_f32 + weight, border), egui::StrokeKind::Inside);

    // The specimen well, with the silhouette inside it.
    let well = Rect::from_min_max(r.left_top() + Vec2::new(8.0, 24.0), r.right_bottom() - Vec2::new(8.0, 28.0));
    painter.rect_filled(well, 2.0, Color32::from_rgba_unmultiplied(6, 9, 24, 150));
    let well_bottom = well.bottom();
    let y_of = |db: f32| well_bottom - ((db - DB_FLOOR) / DB_RANGE).clamp(0.0, 1.0) * well.height();
    if filled {
        let v = blur(&state.slots[i]);
        let line: Vec<Pos2> = (0..BANDS).map(|b| Pos2::new(band_x(well, b), y_of(v[b]))).collect();
        silhouette(painter, &line, well_bottom, PINK, 70.0 + 110.0 * weight, 26.0, 150.0 + 105.0 * weight);
    } else {
        let y = well.center().y;
        painter.extend(Shape::dashed_line(&[Pos2::new(well.left() + 6.0, y), Pos2::new(well.right() - 6.0, y)], Stroke::new(1.0_f32, GRID), 4.0, 4.0));
        painter.text(well.center() + Vec2::new(0.0, -10.0), Align2::CENTER_CENTER, "EMPTY", FontId::monospace(10.0), TEXT_DIM);
    }

    // Captured: a cyan scan line sweeps the well with the real capture progress.
    if capturing {
        let x = well.left() + well.width() * progress;
        painter.rect_filled(Rect::from_min_max(well.left_top(), Pos2::new(x, well.bottom())), 2.0, rgba(CYAN, 22.0));
        painter.line_segment([Pos2::new(x, well.top()), Pos2::new(x, well.bottom())], Stroke::new(5.0_f32, rgba(CYAN, 40.0)));
        painter.line_segment([Pos2::new(x, well.top()), Pos2::new(x, well.bottom())], Stroke::new(1.5_f32, rgba(CYAN, 255.0)));
    }
    if flash > 0.0 {
        painter.rect_filled(r, 4.0, rgba(CYAN, 55.0 * flash));
        painter.rect_stroke(r, 4.0, Stroke::new(2.0_f32, rgba(CYAN, 255.0 * flash)), egui::StrokeKind::Inside);
    }

    // Slide number and how much of it is in the output.
    painter.text(r.left_top() + Vec2::new(10.0, 12.0), Align2::LEFT_CENTER, format!("0{}", i + 1), FontId::monospace(11.0), if filled { rgba(PINK, 255.0) } else { TEXT_DIM });
    if filled && weight > 0.005 {
        painter.text(r.right_top() + Vec2::new(-10.0, 12.0), Align2::RIGHT_CENTER, format!("{:.0}%", weight * 100.0), FontId::monospace(10.0), rgba(YELLOW, 120.0 + 135.0 * weight));
    }

    // The frosted label strip is the capture button.
    let label = Rect::from_min_max(Pos2::new(r.left() + 1.0, r.bottom() - 22.0), r.right_bottom() - Vec2::new(1.0, 1.0));
    painter.rect_filled(label, 2.0, Color32::from_white_alpha(if hovered { 26 } else { 12 }));
    if capturing {
        painter.rect_filled(Rect::from_min_size(label.left_top(), Vec2::new(label.width() * progress, label.height())), 2.0, rgba(CYAN, 70.0));
    }
    let (text, colour) = if capturing {
        ("SCANNING".to_owned(), rgba(CYAN, 255.0))
    } else if hovered {
        (if filled { "RECAPTURE".to_owned() } else { "CAPTURE".to_owned() }, rgba(TEXT, 255.0))
    } else {
        (if filled { "HELD".to_owned() } else { "CAPTURE".to_owned() }, TEXT_DIM)
    };
    painter.circle_filled(label.left_center() + Vec2::new(11.0, 0.0), 3.5, if capturing { rgba(CYAN, 255.0) } else { rgba(PINK, if hovered { 255.0 } else { 150.0 }) });
    painter.text(label.left_center() + Vec2::new(22.0, 0.0), Align2::LEFT_CENTER, text, FontId::monospace(9.0), colour);
}

/// The row of knobs under the rack. Pink shapes the morph, cyan is analysis and time, yellow is
/// the evolve motion (dim while it is off), and plain is the output.
fn knobs(ui: &mut Ui, params: &SpecimenParams, setter: &ParamSetter) {
    let (response, painter) = ui.allocate_painter(Vec2::new(ui.available_width(), KNOB_ROW), Sense::hover());
    let row = response.rect;
    let pink = rgba(PINK, 255.0);
    let cyan = rgba(CYAN, 255.0);
    let evolving = params.evolve.value() != EvolveMode::Off;
    let yellow = if evolving { rgba(YELLOW, 255.0) } else { rgba(YELLOW, 90.0) };
    let cells: [(&FloatParam, &str, Color32, bool); 9] = [
        (&params.fade, "FADE", pink, false),
        (&params.curve, "CURVE", pink, false),
        (&params.scale, "SCALE", pink, true),
        (&params.shift, "SHIFT", pink, true),
        (&params.detail, "DETAIL", cyan, false),
        (&params.glide, "GLIDE", cyan, false),
        (&params.rate, "RATE", yellow, false),
        (&params.depth, "DEPTH", yellow, false),
        (&params.mix, "MIX", rgba(TEXT, 255.0), false),
    ];
    let width = row.width() / cells.len() as f32;
    for (i, (param, label, color, bipolar)) in cells.iter().enumerate() {
        let cell = Rect::from_min_size(Pos2::new(row.left() + i as f32 * width, row.top()), Vec2::new(width, KNOB_ROW));
        knob(ui, &painter, cell, setter, *param, label, *color, *bipolar);
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

#[allow(clippy::too_many_arguments)]
fn knob(ui: &mut Ui, painter: &Painter, cell: Rect, setter: &ParamSetter, param: &FloatParam, label: &str, color: Color32, bipolar: bool) {
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

    painter.circle_filled(center, radius - 5.0, if lit { PANEL } else { Color32::from_rgb(0x0e, 0x14, 0x2e) });
    painter.add(Shape::line(arc(center, radius, start, end), Stroke::new(3.0_f32, GRID)));
    let (from, to) = if bipolar { (start + (end - start) * 0.5, start + (end - start) * norm) } else { (start, start + (end - start) * norm) };
    if (to - from).abs() > 0.01 {
        painter.add(Shape::line(arc(center, radius, from, to), Stroke::new(8.0_f32, rgba(rgb, 30.0))));
        painter.add(Shape::line(arc(center, radius, from, to), Stroke::new(3.0_f32, color)));
    }
    let angle = start + (end - start) * norm;
    let dir = Vec2::new(angle.cos(), angle.sin());
    painter.line_segment([center + dir * (radius - 14.0), center + dir * (radius - 7.0)], Stroke::new(2.0_f32, rgba(TEXT, 255.0)));

    painter.text(Pos2::new(cell.center().x, cell.top() + 69.0), Align2::CENTER_CENTER, label, FontId::monospace(9.0), TEXT_DIM);
    painter.text(Pos2::new(cell.center().x, cell.top() + 85.0), Align2::CENTER_CENTER, param.to_string(), FontId::monospace(11.0), color);
}
