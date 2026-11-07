//! The editor: Aurora Air's night sky turned upside down.
//!
//! The dry spectrum is the mountain ridge above the ground line. What the plugin adds hangs below
//! it as magma, deeper where more is being added, violet for the octave below and vermilion for the
//! harmonics. The glowing line under the ground is the amount: drag the left half down for Sub and
//! the right half down for Body. When the sub engine is locked on a note, a bridge arcs from that
//! note on the ridge to the octave below it. The shaded rock left of the dashed Floor line is
//! where nothing is added; drag the line to move it.

use nih_plug::prelude::*;
use nih_plug_egui::egui::{
    self, Align2, Color32, FontId, Mesh, Painter, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2,
};
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::dsp::{Meter, BANDS, F_MAX, F_MIN, MAX_FLOOR_HZ, MIN_FLOOR_HZ};
use crate::presets;
use crate::BedrockParams;

const BG: Color32 = Color32::from_rgb(0x0b, 0x07, 0x10);
const PANEL: Color32 = Color32::from_rgb(0x19, 0x11, 0x20);
const GRID: Color32 = Color32::from_rgb(0x30, 0x22, 0x3a);
/// Bar labels sit on top of the accent fill, so they need to be lighter than `TEXT_DIM`.
const BAR_LABEL: Color32 = Color32::from_rgb(0xd0, 0xb8, 0xc8);
const TEXT_DIM: Color32 = Color32::from_rgb(0x8a, 0x78, 0x98);
const SKY_TOP: [f32; 3] = [10.0, 7.0, 18.0];
const SKY_HORIZON: [f32; 3] = [62.0, 26.0, 56.0];
const ROCK_TOP: [f32; 3] = [30.0, 15.0, 34.0];
const ROCK_BOTTOM: [f32; 3] = [7.0, 4.0, 10.0];
const SUB_COLOR: [f32; 3] = [140.0, 84.0, 255.0];
const BODY_COLOR: [f32; 3] = [255.0, 92.0, 52.0];
const RIDGE: [f32; 3] = [176.0, 140.0, 206.0];
const LINE: [f32; 3] = [255.0, 226.0, 204.0];

/// Display range for the spectrum, in dB: the top of the ridge is -6.
const DB_FLOOR: f32 = -84.0;
const DB_RANGE: f32 = 78.0;
/// Where the Sub and Body drag zones meet, in Hz.
const SPLIT_HZ: f32 = 80.0;
/// The notes the amount curve is centred on, and how wide each bump is, in octaves.
const SUB_HZ: f32 = 42.0;
const BODY_HZ: f32 = 150.0;
const SUB_WIDTH: f32 = 0.7;
const BODY_WIDTH: f32 = 1.1;
/// Height of the scale strip that sits on the ground line.
const SCALE_H: f32 = 16.0;

pub struct UiState {
    dry_db: [f32; BANDS],
    wet_db: [f32; BANDS],
}

impl Default for UiState {
    fn default() -> Self {
        Self { dry_db: [-120.0; BANDS], wet_db: [-120.0; BANDS] }
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

fn to_db(lin: f32) -> f32 {
    20.0 * lin.max(1e-6).log10()
}

/// Fast attack, slow release, so peaks stay readable.
fn follow(current: &mut [f32; BANDS], source: &[AtomicF32; BANDS]) {
    for (c, s) in current.iter_mut().zip(source) {
        let db = to_db(s.load(Ordering::Relaxed));
        *c = if db > *c { db } else { (*c - 1.5).max(db) };
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

/// A bump in log-frequency, 1 at `fc`, falling off over `width` octaves.
fn bump(f: f32, fc: f32, width: f32) -> f32 {
    let octaves = (f / fc).log2() / width;
    (-0.5 * octaves * octaves).exp()
}

fn dashed(painter: &Painter, from: Pos2, to: Pos2, stroke: Stroke) {
    let len = (to - from).length();
    let steps = (len / 8.0) as usize;
    for k in 0..steps {
        let a = from + (to - from) * (k as f32 * 8.0 / len);
        let b = from + (to - from) * ((k as f32 * 8.0 + 4.0) / len);
        painter.line_segment([a, b], stroke);
    }
}

pub fn draw(ui: &mut Ui, params: &Arc<BedrockParams>, setter: &ParamSetter, meter: &Arc<Meter>, state: &mut UiState) {
    ui.ctx().request_repaint();
    follow(&mut state.dry_db, &meter.dry);
    follow(&mut state.wet_db, &meter.wet);

    let theme = preset_kit::ui::Theme { fill: PANEL, hover: GRID, ink: rgba(LINE, 255.0), dim: TEXT_DIM };
    preset_kit::ui::picker_top_right(ui, 20.0, &presets::BANK, &**params, setter, &theme);
    ui.label(egui::RichText::new("BEDROCK").monospace().size(11.0).color(TEXT_DIM));
    ui.add_space(8.0);
    ground(ui, params, setter, meter, state);
    ui.add_space(10.0);
    bar(ui, setter, &params.trim, "TRIM", rgba(BODY_COLOR, 255.0));
}

fn ground(ui: &mut Ui, params: &Arc<BedrockParams>, setter: &ParamSetter, meter: &Meter, state: &UiState) {
    let size = Vec2::new(ui.available_width(), ui.available_height() - 36.0);
    let (response, painter) = ui.allocate_painter(size, Sense::hover());
    let rect = response.rect;
    // The ground line, with the scale strip hanging from it.
    let surface_top = rect.top() + 6.0;
    let ground_y = surface_top + (rect.height() - SCALE_H) * 0.5;
    let rock_top = ground_y + SCALE_H;
    let surface_h = ground_y - surface_top;
    let rock_h = rect.bottom() - rock_top - 4.0;
    let x_of = |f: f32| rect.left() + (f / F_MIN).ln() / (F_MAX / F_MIN).ln() * rect.width();
    let hz_at = |x: f32| F_MIN * (F_MAX / F_MIN).powf(((x - rect.left()) / rect.width()).clamp(0.0, 1.0));
    let band_x = |i: usize| rect.left() + (i as f32 + 0.5) / BANDS as f32 * rect.width();
    let ridge_y = |db: f32| ground_y - ((db - DB_FLOOR) / DB_RANGE).clamp(0.0, 1.0) * surface_h * 0.86;
    let usable = rock_h * 0.8;

    let a_sub = params.sub.value();
    let a_body = params.body.value();
    let floor_hz = params.floor.value();
    let depth = |f: f32| (a_sub * bump(f, SUB_HZ, SUB_WIDTH) + a_body * bump(f, BODY_HZ, BODY_WIDTH)).min(1.0);

    // Interaction first, so painting can react to hover and drag.
    let split = x_of(SPLIT_HZ);
    let zones = [
        (Rect::from_min_max(Pos2::new(rect.left(), rock_top), Pos2::new(split, rect.bottom())), &params.sub, "SUB"),
        (Rect::from_min_max(Pos2::new(split, rock_top), rect.max), &params.body, "BODY"),
    ];
    let mut active = [false; 2];
    for (i, (area, param, label)) in zones.iter().enumerate() {
        let r = ui.interact(*area, ui.id().with(*label), Sense::click_and_drag());
        active[i] = r.hovered() || r.dragged();
        if r.drag_started() {
            setter.begin_set_parameter(*param);
        }
        if r.dragged() {
            // Down is more: the magma goes deeper.
            let v = param.unmodulated_normalized_value() + r.drag_delta().y / (usable * 0.9);
            setter.set_parameter_normalized(*param, v.clamp(0.0, 1.0));
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
    // The floor is registered last so it wins where it overlaps a zone.
    let floor_x = x_of(floor_hz);
    let grip = Rect::from_center_size(Pos2::new(floor_x, rect.bottom() - 16.0), Vec2::new(46.0, 16.0));
    let floor_hit = Rect::from_min_max(Pos2::new(floor_x - 7.0, rock_top), Pos2::new(floor_x + 7.0, rect.bottom())).union(grip);
    let floor_r = ui.interact(floor_hit, ui.id().with("floor"), Sense::click_and_drag());
    let floor_lit = floor_r.hovered() || floor_r.dragged();
    if floor_lit {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    if floor_r.drag_started() {
        setter.begin_set_parameter(&params.floor);
    }
    if floor_r.dragged() {
        if let Some(pos) = floor_r.interact_pointer_pos() {
            setter.set_parameter(&params.floor, hz_at(pos.x).clamp(MIN_FLOOR_HZ, MAX_FLOOR_HZ));
        }
    }
    if floor_r.drag_stopped() {
        setter.end_set_parameter(&params.floor);
    }
    if floor_r.double_clicked() {
        setter.begin_set_parameter(&params.floor);
        setter.set_parameter(&params.floor, params.floor.default_plain_value());
        setter.end_set_parameter(&params.floor);
    }

    // Sky and rock.
    strip(
        &painter,
        &[
            (Pos2::new(rect.left(), surface_top), rgba(SKY_TOP, 255.0), Pos2::new(rect.left(), ground_y), rgba(SKY_HORIZON, 255.0)),
            (Pos2::new(rect.right(), surface_top), rgba(SKY_TOP, 255.0), Pos2::new(rect.right(), ground_y), rgba(SKY_HORIZON, 255.0)),
        ],
    );
    strip(
        &painter,
        &[
            (Pos2::new(rect.left(), rock_top), rgba(ROCK_TOP, 255.0), Pos2::new(rect.left(), rect.bottom()), rgba(ROCK_BOTTOM, 255.0)),
            (Pos2::new(rect.right(), rock_top), rgba(ROCK_TOP, 255.0), Pos2::new(rect.right(), rect.bottom()), rgba(ROCK_BOTTOM, 255.0)),
        ],
    );
    let mut y = rock_top + 22.0;
    while y < rect.bottom() {
        painter.line_segment([Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)], Stroke::new(1.0_f32, rgba(LINE, 7.0)));
        y += 22.0;
    }
    for (i, (area, ..)) in zones.iter().enumerate() {
        if active[i] {
            painter.rect_filled(*area, 0.0, Color32::from_rgba_unmultiplied(255, 255, 255, 10));
        }
    }

    // The sealed rock below the floor.
    painter.rect_filled(Rect::from_min_max(Pos2::new(rect.left(), rock_top), Pos2::new(floor_x, rect.bottom())), 0.0, Color32::from_black_alpha(110));

    // Ground line and frequency scale.
    painter.rect_filled(Rect::from_min_max(Pos2::new(rect.left(), ground_y), Pos2::new(rect.right(), rock_top)), 0.0, BG);
    for (f, label) in [(20.0, "20"), (30.0, "30"), (40.0, "40"), (60.0, "60"), (80.0, "80"), (100.0, "100"), (150.0, "150"), (200.0, "200"), (300.0, "300"), (400.0, "400")] {
        let x = x_of(f).clamp(rect.left() + 8.0, rect.right() - 10.0);
        painter.line_segment([Pos2::new(x, ground_y), Pos2::new(x, ground_y + 4.0)], Stroke::new(1.0_f32, GRID));
        painter.text(Pos2::new(x, ground_y + 5.0), Align2::CENTER_TOP, label, FontId::monospace(8.0), TEXT_DIM);
    }
    painter.line_segment([Pos2::new(rect.left(), ground_y), Pos2::new(rect.right(), ground_y)], Stroke::new(1.0_f32, GRID));
    painter.line_segment([Pos2::new(rect.left(), rock_top), Pos2::new(rect.right(), rock_top)], Stroke::new(1.0_f32, GRID));

    // Ridge: the dry spectrum.
    let dry = blur(&state.dry_db);
    let wet = blur(&state.wet_db);
    let ridge: Vec<Pos2> = (0..BANDS).map(|i| Pos2::new(band_x(i), ridge_y(dry[i]))).collect();
    strip(
        &painter,
        &ridge.iter().map(|p| (*p, rgba(RIDGE, 100.0), Pos2::new(p.x, ground_y), rgba(SKY_HORIZON, 0.0))).collect::<Vec<_>>(),
    );
    painter.add(Shape::line(ridge.clone(), Stroke::new(1.5_f32, rgba(RIDGE, 210.0))));

    // Magma: what is added, hanging under the ground.
    let tips: Vec<Pos2> = (0..BANDS)
        .map(|i| {
            let thickness = ((wet[i] - DB_FLOOR) / 70.0).clamp(0.0, 1.0) * rock_h * 0.62;
            Pos2::new(band_x(i), rock_top + thickness)
        })
        .collect();
    let magma = |i: usize, a: f32| rgba(mix(SUB_COLOR, BODY_COLOR, i as f32 / BANDS as f32), a);
    strip(&painter, &(0..BANDS).map(|i| (Pos2::new(tips[i].x, rock_top), magma(i, 150.0), tips[i], magma(i, 0.0))).collect::<Vec<_>>());
    for i in 0..BANDS - 1 {
        painter.line_segment([tips[i], tips[i + 1]], Stroke::new(5.0_f32, magma(i, 40.0)));
        painter.line_segment([tips[i], tips[i + 1]], Stroke::new(1.5_f32, magma(i, 230.0)));
    }

    // The octave bridge: from the note the sub is following, down to the note it adds.
    let note = meter.pitch_hz.load(Ordering::Relaxed);
    if note > 0.0 {
        let (from, to) = (Pos2::new(x_of(note), ground_y + 1.0), Pos2::new(x_of(note * 0.5), rock_top + 1.0));
        let control = Pos2::new((from.x + to.x) * 0.5, rock_top + 26.0);
        let curve: Vec<Pos2> = (0..=24)
            .map(|s| {
                let t = s as f32 / 24.0;
                let (a, b, c) = ((1.0 - t) * (1.0 - t), 2.0 * t * (1.0 - t), t * t);
                Pos2::new(a * from.x + b * control.x + c * to.x, a * from.y + b * control.y + c * to.y)
            })
            .collect();
        painter.add(Shape::line(curve.clone(), Stroke::new(5.0_f32, rgba(SUB_COLOR, 50.0))));
        painter.add(Shape::line(curve, Stroke::new(1.5_f32, rgba(LINE, 230.0))));
        painter.circle_filled(from, 3.5, Color32::WHITE);
        painter.circle_filled(to, 3.5, rgba(SUB_COLOR, 255.0));
        painter.text(from + Vec2::new(5.0, -3.0), Align2::LEFT_BOTTOM, format!("{note:.0}"), FontId::monospace(8.0), rgba(LINE, 200.0));
        painter.text(to + Vec2::new(-5.0, 9.0), Align2::RIGHT_TOP, format!("{:.0}", note * 0.5), FontId::monospace(8.0), rgba(SUB_COLOR, 255.0));
    }

    // The amount line: how deep the magma is asked to go.
    const STEPS: usize = 160;
    let curve: Vec<Pos2> = (0..=STEPS)
        .map(|s| {
            let f = F_MIN * (F_MAX / F_MIN).powf(s as f32 / STEPS as f32);
            Pos2::new(rect.left() + s as f32 / STEPS as f32 * rect.width(), rock_top + depth(f) * usable)
        })
        .collect();
    strip(&painter, &curve.iter().map(|p| (Pos2::new(p.x, rock_top), rgba(LINE, 0.0), *p, rgba(LINE, 16.0))).collect::<Vec<_>>());
    painter.add(Shape::line(curve.clone(), Stroke::new(5.0_f32, rgba(LINE, 28.0))));
    painter.add(Shape::line(curve, Stroke::new(1.5_f32, rgba(LINE, 230.0))));

    // Handles and labels.
    for (i, (_, param, label)) in zones.iter().enumerate() {
        let fc = if i == 0 { SUB_HZ } else { BODY_HZ };
        let handle = Pos2::new(x_of(fc), rock_top + depth(fc) * usable);
        let strength = if active[i] { 1.6 } else { 1.0 };
        painter.line_segment([Pos2::new(handle.x, rock_top), handle], Stroke::new(1.0_f32, rgba(LINE, 60.0)));
        painter.circle_filled(handle, 13.0 * strength, rgba(LINE, 22.0));
        painter.circle_filled(handle, 8.0 * strength, rgba(LINE, 55.0));
        painter.circle_filled(handle, 4.5, Color32::WHITE);

        let (anchor, align) = if i == 0 {
            (Pos2::new(rect.left() + 12.0, rect.top() + 12.0), Align2::LEFT_TOP)
        } else {
            (Pos2::new(rect.right() - 12.0, rect.top() + 12.0), Align2::RIGHT_TOP)
        };
        painter.text(anchor, align, *label, FontId::monospace(10.0), TEXT_DIM);
        painter.text(anchor + Vec2::new(0.0, 14.0), align, format!("{:.0}%", param.value() * 100.0), FontId::monospace(22.0), rgba(LINE, 255.0));
    }
    let lock = meter.lock.load(Ordering::Relaxed);
    let pill = Rect::from_min_size(Pos2::new(rect.left() + 10.0, rect.top() + 50.0), Vec2::new(122.0, 16.0));
    painter.rect_filled(pill, 8.0, Color32::from_black_alpha(150));
    painter.circle_filled(Pos2::new(pill.left() + 9.0, pill.center().y), 3.5, rgba(SUB_COLOR, 50.0 + 205.0 * lock));
    painter.text(
        Pos2::new(pill.left() + 18.0, pill.center().y),
        Align2::LEFT_CENTER,
        if note > 0.0 { format!("tracking {note:.0} Hz") } else { "no single note".to_owned() },
        FontId::monospace(9.0),
        rgba(LINE, 200.0),
    );

    // The floor.
    let ink = rgba(LINE, if floor_lit { 255.0 } else { 150.0 });
    dashed(&painter, Pos2::new(floor_x, rock_top + 4.0), Pos2::new(floor_x, grip.top()), Stroke::new(1.0_f32, ink));
    painter.rect_filled(grip, 3.0, if floor_lit { GRID } else { BG });
    painter.rect_stroke(grip, 3.0, Stroke::new(1.0_f32, ink), egui::StrokeKind::Inside);
    painter.text(grip.center(), Align2::CENTER_CENTER, format!("{floor_hz:.0} Hz"), FontId::monospace(9.0), ink);
    painter.text(Pos2::new(grip.right() + 5.0, grip.center().y), Align2::LEFT_CENTER, "floor", FontId::monospace(8.0), TEXT_DIM);

    painter.text(Pos2::new(rect.center().x, rect.top() + 12.0), Align2::CENTER_TOP, "drag the magma down", FontId::monospace(9.0), TEXT_DIM);
}

/// A horizontal bar fader for a float parameter: drag to set, double-click to reset.
fn bar(ui: &mut Ui, setter: &ParamSetter, param: &FloatParam, label: &str, color: Color32) {
    let size = Vec2::new(ui.available_width(), 26.0);
    let (response, painter) = ui.allocate_painter(size, Sense::click_and_drag());
    let rect = response.rect;
    let norm = param.unmodulated_normalized_value();

    painter.rect_filled(rect, 4.0, PANEL);
    let fill = Rect::from_min_size(rect.min, Vec2::new(rect.width() * norm, rect.height()));
    let [r, g, b, _] = color.to_array();
    painter.rect_filled(fill, 4.0, Color32::from_rgba_unmultiplied(r, g, b, 36));
    painter.rect_filled(Rect::from_min_size(Pos2::new(fill.right() - 3.0, rect.top()), Vec2::new(3.0, rect.height())), 1.5, color);
    painter.text(Pos2::new(rect.left() + 8.0, rect.center().y), Align2::LEFT_CENTER, label, FontId::monospace(11.0), BAR_LABEL);
    painter.text(Pos2::new(rect.right() - 8.0, rect.center().y), Align2::RIGHT_CENTER, param.to_string(), FontId::monospace(12.0), color);
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
