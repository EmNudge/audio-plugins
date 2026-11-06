//! The editor: a live spectrum drawn as a night-sky skyline.
//!
//! The dry signal is the mountain ridge. The air the plugin adds floats above the ridge as an
//! aurora ribbon, thicker where more is being added. The two bands are the glowing shelf line:
//! drag up or down in the left half for Mid High, or the right half for High.

use nih_plug::prelude::*;
use nih_plug_egui::egui::{
    self, Align2, Color32, FontId, Mesh, Painter, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2,
};
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::dsp::{Spectrum, BANDS, F_MAX, F_MIN};
use crate::presets;
use crate::{AirParams, HIGH_HZ, MID_HIGH_HZ};

const BG: Color32 = Color32::from_rgb(0x07, 0x08, 0x10);
const PANEL: Color32 = Color32::from_rgb(0x12, 0x14, 0x22);
const GRID: Color32 = Color32::from_rgb(0x24, 0x28, 0x3c);
/// Bar labels sit on top of the accent fill, so they need to be lighter than `TEXT_DIM`.
const BAR_LABEL: Color32 = Color32::from_rgb(0xb4, 0xba, 0xd4);
const TEXT_DIM: Color32 = Color32::from_rgb(0x6c, 0x72, 0x90);
const SKY_TOP: [f32; 3] = [12.0, 20.0, 54.0];
const SKY_BOTTOM: [f32; 3] = [5.0, 6.0, 13.0];
const AURORA_LOW: [f32; 3] = [53.0, 240.0, 192.0];
const AURORA_HIGH: [f32; 3] = [255.0, 79.0, 216.0];
const RIDGE: [f32; 3] = [90.0, 111.0, 176.0];
const LINE: [f32; 3] = [207.0, 232.0, 255.0];

/// Display range for the spectrum, in dB.
const DB_FLOOR: f32 = -100.0;
const DB_RANGE: f32 = 85.0;
/// Where the Mid High / High drag zones meet.
const SPLIT_HZ: f32 = 5600.0;

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
    Color32::from_rgba_unmultiplied(c[0] as u8, c[1] as u8, c[2] as u8, a as u8)
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

pub fn draw(
    ui: &mut Ui,
    params: &Arc<AirParams>,
    setter: &ParamSetter,
    spectrum: &Arc<Spectrum>,
    state: &mut UiState,
) {
    ui.ctx().request_repaint();
    follow(&mut state.dry_db, &spectrum.dry);
    follow(&mut state.wet_db, &spectrum.wet);

    let theme = preset_kit::ui::Theme { fill: PANEL, hover: GRID, ink: rgba(LINE, 255.0), dim: TEXT_DIM };
    preset_kit::ui::picker_top_right(ui, 20.0, &presets::BANK, &**params, setter, &theme);
    ui.label(egui::RichText::new("AURORA AIR").monospace().size(11.0).color(TEXT_DIM));
    ui.add_space(8.0);
    sky(ui, params, setter, state);
    ui.add_space(10.0);
    bar(ui, setter, &params.trim, "TRIM", rgba(AURORA_LOW, 255.0));
}

/// A smooth step in log-frequency: 0 well below `fc`, 1 well above.
fn shelf(f: f32, fc: f32) -> f32 {
    1.0 / (1.0 + (-3.0 * (f / fc).log2()).exp())
}

fn sky(ui: &mut Ui, params: &Arc<AirParams>, setter: &ParamSetter, state: &UiState) {
    let size = Vec2::new(ui.available_width(), ui.available_height() - 36.0);
    let (response, painter) = ui.allocate_painter(size, Sense::hover());
    let rect = response.rect;
    let bottom = rect.bottom() - 18.0;
    let h = bottom - rect.top() - 8.0;
    let x_of = |f: f32| rect.left() + (f / F_MIN).ln() / (F_MAX / F_MIN).ln() * rect.width();
    let y_of_db = |db: f32| bottom - ((db - DB_FLOOR) / DB_RANGE).clamp(0.0, 1.0) * h * 0.78;
    let band_x = |i: usize| rect.left() + (i as f32 + 0.5) / BANDS as f32 * rect.width();

    // Interaction first, so painting can react to hover and drag.
    let split = x_of(SPLIT_HZ);
    let columns = [
        (Rect::from_min_max(rect.min, Pos2::new(split, rect.bottom())), &params.mid_high, "MID HIGH", true),
        (Rect::from_min_max(Pos2::new(split, rect.top()), rect.max), &params.high, "HIGH", false),
    ];
    let mut active = [false; 2];
    for (i, (area, param, label, _)) in columns.iter().enumerate() {
        let r = ui.interact(*area, ui.id().with(*label), Sense::click_and_drag());
        active[i] = r.hovered() || r.dragged();
        if r.drag_started() {
            setter.begin_set_parameter(*param);
        }
        if r.dragged() {
            let v = param.unmodulated_normalized_value() - r.drag_delta().y / (h * 0.85);
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

    let a_mid = params.mid_high.value();
    let a_high = params.high.value();

    // Sky.
    strip(
        &painter,
        &[
            (rect.left_top(), rgba(SKY_TOP, 255.0), rect.left_bottom(), rgba(SKY_BOTTOM, 255.0)),
            (rect.right_top(), rgba(SKY_TOP, 255.0), rect.right_bottom(), rgba(SKY_BOTTOM, 255.0)),
        ],
    );
    for (i, (area, ..)) in columns.iter().enumerate() {
        if active[i] {
            painter.rect_filled(*area, 0.0, Color32::from_rgba_unmultiplied(255, 255, 255, 10));
        }
    }

    // Frequency scale.
    for (f, label) in [(1000.0, "1k"), (2000.0, "2k"), (5000.0, "5k"), (10000.0, "10k"), (20000.0, "20k")] {
        let x = x_of(f).min(rect.right() - 12.0);
        painter.line_segment([Pos2::new(x, bottom), Pos2::new(x, bottom + 4.0)], Stroke::new(1.0_f32, GRID));
        painter.text(Pos2::new(x, bottom + 5.0), Align2::CENTER_TOP, label, FontId::monospace(9.0), TEXT_DIM);
    }
    painter.line_segment([Pos2::new(rect.left(), bottom), Pos2::new(rect.right(), bottom)], Stroke::new(1.0_f32, GRID));

    // Ridge: the dry spectrum.
    let dry = blur(&state.dry_db);
    let wet = blur(&state.wet_db);
    let ridge: Vec<Pos2> = (0..BANDS).map(|i| Pos2::new(band_x(i), y_of_db(dry[i]))).collect();
    let floor = Pos2::new(0.0, bottom);
    strip(
        &painter,
        &ridge
            .iter()
            .map(|p| (*p, rgba(RIDGE, 90.0), Pos2::new(p.x, floor.y), rgba(SKY_BOTTOM, 255.0)))
            .collect::<Vec<_>>(),
    );
    painter.add(Shape::line(ridge.clone(), Stroke::new(1.5_f32, rgba(RIDGE, 200.0))));

    // Aurora: the added air, stacked above the ridge.
    let aurora_top: Vec<Pos2> = (0..BANDS)
        .map(|i| {
            let thickness = ((wet[i] - DB_FLOOR) / 70.0).clamp(0.0, 1.0) * h * 0.22;
            Pos2::new(ridge[i].x, ridge[i].y - thickness)
        })
        .collect();
    let aurora_color = |i: usize, a: f32| rgba(mix(AURORA_LOW, AURORA_HIGH, i as f32 / BANDS as f32), a);
    strip(
        &painter,
        &(0..BANDS).map(|i| (aurora_top[i], aurora_color(i, 0.0), ridge[i], aurora_color(i, 170.0))).collect::<Vec<_>>(),
    );
    for i in 0..BANDS - 1 {
        painter.line_segment([aurora_top[i], aurora_top[i + 1]], Stroke::new(5.0_f32, aurora_color(i, 40.0)));
        painter.line_segment([aurora_top[i], aurora_top[i + 1]], Stroke::new(1.5_f32, aurora_color(i, 230.0)));
    }

    // The shelf line: the combined boost of both bands.
    let usable = h * 0.85;
    let level = |f: f32| (a_mid * shelf(f, MID_HIGH_HZ) + a_high * shelf(f, HIGH_HZ)) / 2.0;
    const STEPS: usize = 160;
    let curve: Vec<Pos2> = (0..=STEPS)
        .map(|s| {
            let x = rect.left() + s as f32 / STEPS as f32 * rect.width();
            let f = F_MIN * (F_MAX / F_MIN).powf(s as f32 / STEPS as f32);
            Pos2::new(x, bottom - level(f) * usable)
        })
        .collect();
    strip(
        &painter,
        &curve.iter().map(|p| (*p, rgba(LINE, 38.0), Pos2::new(p.x, bottom), rgba(LINE, 0.0))).collect::<Vec<_>>(),
    );
    painter.add(Shape::line(curve.clone(), Stroke::new(6.0_f32, rgba(LINE, 35.0))));
    painter.add(Shape::line(curve, Stroke::new(2.0_f32, rgba(LINE, 255.0))));

    // Handles and labels.
    for (i, (_, param, label, left)) in columns.iter().enumerate() {
        let fc = if *left { MID_HIGH_HZ } else { HIGH_HZ };
        let handle = Pos2::new(x_of(fc), bottom - level(fc) * usable);
        let strength = if active[i] { 1.6 } else { 1.0 };
        painter.line_segment(
            [handle, Pos2::new(handle.x, bottom)],
            Stroke::new(1.0_f32, rgba(LINE, 60.0)),
        );
        painter.circle_filled(handle, 13.0 * strength, rgba(LINE, 22.0));
        painter.circle_filled(handle, 8.0 * strength, rgba(LINE, 55.0));
        painter.circle_filled(handle, 4.5, Color32::WHITE);

        let (anchor, align) = if *left {
            (Pos2::new(rect.left() + 12.0, rect.top() + 8.0), Align2::LEFT_TOP)
        } else {
            (Pos2::new(rect.right() - 12.0, rect.top() + 8.0), Align2::RIGHT_TOP)
        };
        painter.text(anchor, align, *label, FontId::monospace(10.0), TEXT_DIM);
        painter.text(
            anchor + Vec2::new(0.0, 14.0),
            align,
            format!("{:.0}%", param.value() * 100.0),
            FontId::monospace(22.0),
            rgba(LINE, 255.0),
        );
    }

    painter.text(
        Pos2::new(rect.center().x, rect.top() + 8.0),
        Align2::CENTER_TOP,
        "drag a side up / down",
        FontId::monospace(9.0),
        TEXT_DIM,
    );
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
    painter.rect_filled(
        Rect::from_min_size(Pos2::new(fill.right() - 3.0, rect.top()), Vec2::new(3.0, rect.height())),
        1.5,
        color,
    );
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
