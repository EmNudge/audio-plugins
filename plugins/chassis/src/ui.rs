//! The editor: a robot's torso seen side-on, as a spectrum.
//!
//! Frequency runs left to right on a log axis and the signal is drawn as vent-grille slats, one
//! column per filter-bank bin: lit segments are the output, and a short white tick over each
//! column is what went in. The grille is divided into three bays by two bulkheads, the splits,
//! and each bay is lit in its own colour: orange for the chest (the low band: sub and squash),
//! lime for the body (the mid band: ring mod, vocoder, shifter, comb, grit) and cyan for the shell
//! (the high band: cavity, shine, plate). Drag a bulkhead to move a split.
//!
//! Two needles mark the voice's pitch going in (dashed) and coming out (solid), so the pitch knob
//! can be read as a distance. In Drone mode a lime diamond marks the vocoder's carrier. In the
//! header, a pair of eyes light with the output and shut when the gate closes, and the value of
//! the knob under the pointer is written large.
//!
//! Knobs: drag vertically (hold shift for fine), double-click to reset.

use nih_plug::prelude::*;
use nih_plug_egui::egui::{
    self, Align2, Color32, FontId, Painter, Pos2, Rect, Sense, Shape as EguiShape, Stroke, Ui, Vec2,
};
use std::f32::consts::PI;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::dsp::{Carrier, Meter, MAX_HIGH_HZ, MAX_LOW_HZ, MIN_HIGH_HZ, MIN_LOW_HZ, MIN_SPLIT_RATIO};
use crate::filters::lin_to_db;
use crate::fx::{bin_hz, BINS, BIN_HI_HZ, BIN_LO_HZ};
use crate::presets;
use crate::ChassisParams;

const BG: Color32 = Color32::from_rgb(0x0a, 0x0d, 0x12);
const PANEL: Color32 = Color32::from_rgb(0x12, 0x17, 0x1f);
const GRID: Color32 = Color32::from_rgb(0x23, 0x2b, 0x37);
const TEXT_DIM: Color32 = Color32::from_rgb(0x7b, 0x87, 0x94);
const TEXT: [f32; 3] = [238.0, 240.0, 236.0];
const ORANGE: [f32; 3] = [255.0, 92.0, 40.0];
const LIME: [f32; 3] = [190.0, 245.0, 30.0];
const CYAN: [f32; 3] = [45.0, 226.0, 230.0];

/// The frequency axis of the display.
const F_LO: f32 = 40.0;
const F_HI: f32 = 16000.0;
/// The level axis: the bottom and top of a column, in dB.
const DB_LO: f32 = -66.0;
const DB_HI: f32 = -6.0;
/// How fast a column falls, in dB per second.
const FALL_DB_S: f32 = 40.0;
const FLOOR_DB: f32 = -100.0;

const SEG_H: f32 = 6.0;
const SEG_GAP: f32 = 2.0;
const CHIP_H: f32 = 22.0;
const KNOB_H: f32 = 78.0;
const SECTION_HEAD: f32 = 24.0;
const SECTION_H: f32 = SECTION_HEAD + KNOB_H * 2.0 + 8.0;
const SECTION_GAP: f32 = 8.0;
/// The strip above the grille: bay names, what is in them, and the bulkhead handles.
const TOP_STRIP: f32 = 56.0;

#[derive(Default)]
pub struct UiState {
    /// Column levels in dB, rising at once and falling slowly.
    input: Vec<f32>,
    output: Vec<f32>,
    /// The knob under the pointer: label, value, colour. Set while drawing the knobs and read by
    /// the header on the next frame.
    editing: Option<(&'static str, String, [f32; 3])>,
    eyes: f32,
}

pub fn background() -> Color32 {
    BG
}

fn rgba(c: [f32; 3], a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c[0] as u8, c[1] as u8, c[2] as u8, a.clamp(0.0, 255.0) as u8)
}

/// What the meter held when the frame began.
struct Snapshot {
    gate: f32,
    squash: f32,
    glue: f32,
    limiter: f32,
    pitch_hz: f32,
    carrier_hz: f32,
    latency_ms: f32,
}

fn snapshot(meter: &Meter, state: &mut UiState, dt: f32) -> Snapshot {
    state.input.resize(BINS, FLOOR_DB);
    state.output.resize(BINS, FLOOR_DB);
    let fall = FALL_DB_S * dt;
    for i in 0..BINS {
        let (i_db, o_db) = (lin_to_db(meter.input[i].load(Ordering::Relaxed)), lin_to_db(meter.output[i].load(Ordering::Relaxed)));
        state.input[i] = i_db.max(state.input[i] - fall).max(FLOOR_DB);
        state.output[i] = o_db.max(state.output[i] - fall).max(FLOOR_DB);
    }
    let sample_rate = meter.sample_rate.load(Ordering::Relaxed).max(1.0);
    let level_db = lin_to_db(meter.level.load(Ordering::Relaxed));
    let target = ((level_db + 54.0) / 48.0).clamp(0.0, 1.0);
    state.eyes = if target > state.eyes { target } else { (state.eyes - 2.0 * dt).max(target) };
    Snapshot {
        gate: meter.gate.load(Ordering::Relaxed),
        squash: meter.squash.load(Ordering::Relaxed),
        glue: meter.glue.load(Ordering::Relaxed),
        limiter: meter.limiter.load(Ordering::Relaxed),
        pitch_hz: meter.pitch_hz.load(Ordering::Relaxed),
        carrier_hz: meter.carrier_hz.load(Ordering::Relaxed),
        latency_ms: meter.latency.load(Ordering::Relaxed) / sample_rate * 1000.0,
    }
}

pub fn draw(ui: &mut Ui, params: &Arc<ChassisParams>, setter: &ParamSetter, meter: &Arc<Meter>, state: &mut UiState) {
    ui.ctx().request_repaint();
    let dt = ui.input(|i| i.stable_dt).clamp(0.001, 0.1);
    let snap = snapshot(meter, state, dt);
    let settings = params.settings();

    header(ui, params, setter, &snap, state);
    ui.add_space(8.0);

    let display_h = (ui.available_height() - SECTION_H - 8.0).max(200.0);
    let (area, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), display_h), Sense::hover());
    display(ui, area, params, setter, &snap, &settings, state);

    ui.add_space(8.0);
    state.editing = None;
    sections(ui, params, setter, state);
}

fn header(ui: &mut Ui, params: &ChassisParams, setter: &ParamSetter, snap: &Snapshot, state: &UiState) {
    let theme = preset_kit::ui::Theme { fill: PANEL, hover: GRID, ink: rgba(TEXT, 255.0), dim: TEXT_DIM };
    preset_kit::ui::picker_top_right(ui, 24.0, &presets::BANK, params, setter, &theme);
    let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.0), Sense::hover());
    let painter = ui.painter().clone();

    // Eyes: lit by the output, shut by the gate.
    let awake = snap.gate > 0.25;
    for k in 0..2 {
        let eye = Rect::from_center_size(Pos2::new(row.left() + 7.0 + k as f32 * 22.0, row.center().y), Vec2::new(12.0, 12.0));
        if awake {
            let glow = 20.0 + 70.0 * state.eyes;
            painter.rect_filled(eye.expand(3.0), 3.0, rgba(LIME, glow * 0.5));
            painter.rect_filled(eye, 1.5, rgba(LIME, 70.0 + 185.0 * state.eyes));
        } else {
            painter.rect_filled(Rect::from_center_size(eye.center(), Vec2::new(12.0, 2.0)), 1.0, rgba(LIME, 50.0));
        }
    }
    let x = row.left() + 54.0;
    painter.text(Pos2::new(x, row.center().y), Align2::LEFT_CENTER, "CHASSIS", FontId::monospace(11.0), TEXT_DIM);
    painter.text(Pos2::new(x + 66.0, row.center().y), Align2::LEFT_CENTER, "robot voice rig", FontId::monospace(9.0), GRID);

    // The value under the pointer, big, else the voice's pitch.
    let right = row.right() - preset_kit::ui::WIDTH - 16.0;
    let (label, value, color) = match &state.editing {
        Some((label, value, color)) => (label.to_string(), value.clone(), *color),
        None if snap.pitch_hz > 0.0 => ("F0".to_owned(), format!("{:.0}Hz", snap.pitch_hz), LIME),
        None => ("F0".to_owned(), "--".to_owned(), TEXT),
    };
    let big = painter.text(Pos2::new(right, row.center().y), Align2::RIGHT_CENTER, value, FontId::monospace(18.0), rgba(color, 255.0));
    painter.text(Pos2::new(big.left() - 8.0, row.center().y), Align2::RIGHT_CENTER, label, FontId::monospace(9.0), TEXT_DIM);
    let small = format!("LAT {:.0}ms   GLUE {:.1}   LIM {:.1}", snap.latency_ms, snap.glue, snap.limiter);
    painter.text(Pos2::new(big.left() - 70.0, row.center().y), Align2::RIGHT_CENTER, small, FontId::monospace(9.0), TEXT_DIM);
}

fn x_of(plot: Rect, hz: f32) -> f32 {
    plot.left() + plot.width() * (hz.max(1.0) / F_LO).ln() / (F_HI / F_LO).ln()
}

fn hz_at(plot: Rect, x: f32) -> f32 {
    F_LO * (F_HI / F_LO).powf(((x - plot.left()) / plot.width()).clamp(0.0, 1.0))
}

fn level01(db: f32) -> f32 {
    ((db - DB_LO) / (DB_HI - DB_LO)).clamp(0.0, 1.0)
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

fn display(ui: &mut Ui, rect: Rect, params: &ChassisParams, setter: &ParamSetter, snap: &Snapshot, s: &crate::dsp::Settings, state: &UiState) {
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 6.0, PANEL);
    let plot = Rect::from_min_max(Pos2::new(rect.left() + 14.0, rect.top() + TOP_STRIP), Pos2::new(rect.right() - 14.0, rect.bottom() - 26.0));
    let (low, high) = s.splits();

    // Bays.
    let edges = [F_LO, low, high, F_HI];
    let bays: [(&str, &str, [f32; 3]); 3] = [
        ("CHEST", "sub / squash", ORANGE),
        ("BODY", "ring / vocoder / shift / comb / grit", LIME),
        ("SHELL", "cavity / shine / plate", CYAN),
    ];
    for (k, (name, what, color)) in bays.iter().enumerate() {
        let (xa, xb) = (x_of(plot, edges[k]), x_of(plot, edges[k + 1]));
        painter.rect_filled(Rect::from_min_max(Pos2::new(xa, rect.top() + 4.0), Pos2::new(xb, plot.bottom())), 0.0, rgba(*color, 6.0));
        painter.rect_filled(Rect::from_min_max(Pos2::new(xa, rect.top() + 4.0), Pos2::new(xb, rect.top() + 6.0)), 0.0, rgba(*color, 170.0));
        let label = if k == 0 && snap.squash > 0.3 { format!("{name}  {:.1} dB", -snap.squash) } else { name.to_string() };
        painter.text(Pos2::new(xa + 10.0, rect.top() + 10.0), Align2::LEFT_TOP, label, FontId::monospace(11.0), rgba(*color, 235.0));
        if xb - xa > 150.0 {
            painter.text(Pos2::new(xa + 10.0, rect.top() + 25.0), Align2::LEFT_TOP, what, FontId::monospace(8.0), TEXT_DIM);
        }
    }

    // Level and frequency guides.
    for db in [-12.0f32, -24.0, -36.0, -48.0] {
        let y = plot.bottom() - level01(db) * plot.height();
        painter.line_segment([Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)], Stroke::new(1.0_f32, rgba(TEXT, 9.0)));
        painter.text(Pos2::new(plot.left() + 2.0, y - 1.0), Align2::LEFT_BOTTOM, format!("{db:.0}"), FontId::monospace(7.0), GRID);
    }
    for (hz, label) in [(50.0, "50"), (100.0, "100"), (200.0, "200"), (500.0, "500"), (1000.0, "1k"), (2000.0, "2k"), (5000.0, "5k"), (10000.0, "10k")] {
        let x = x_of(plot, hz);
        painter.line_segment([Pos2::new(x, plot.bottom()), Pos2::new(x, plot.bottom() + 4.0)], Stroke::new(1.0_f32, TEXT_DIM));
        painter.text(Pos2::new(x, plot.bottom() + 6.0), Align2::CENTER_TOP, label, FontId::monospace(8.0), TEXT_DIM);
    }
    painter.line_segment([Pos2::new(plot.left(), plot.bottom()), Pos2::new(plot.right(), plot.bottom())], Stroke::new(1.0_f32, GRID));

    // The grille.
    let ratio = (BIN_HI_HZ / BIN_LO_HZ).powf(1.0 / (BINS - 1) as f32).sqrt();
    let segments = ((plot.height() / (SEG_H + SEG_GAP)) as usize).max(4);
    for i in 0..BINS {
        let hz = bin_hz(i);
        let (xa, xb) = (x_of(plot, hz / ratio) + 1.0, x_of(plot, hz * ratio) - 1.0);
        if xb <= xa {
            continue;
        }
        let color = if hz < low { ORANGE } else if hz < high { LIME } else { CYAN };
        let lit = (level01(state.output.get(i).copied().unwrap_or(FLOOR_DB)) * segments as f32).round() as usize;
        if lit > 0 {
            let top = plot.bottom() - lit as f32 * (SEG_H + SEG_GAP);
            painter.rect_filled(Rect::from_min_max(Pos2::new(xa - 1.0, top - 2.0), Pos2::new(xb + 1.0, plot.bottom())), 2.0, rgba(color, 14.0));
        }
        for seg in 0..lit {
            let y = plot.bottom() - (seg + 1) as f32 * (SEG_H + SEG_GAP);
            let heat = seg as f32 / segments as f32;
            let alpha = 95.0 + 160.0 * heat + if seg + 1 == lit { 40.0 } else { 0.0 };
            painter.rect_filled(Rect::from_min_size(Pos2::new(xa, y), Vec2::new(xb - xa, SEG_H)), 1.0, rgba(color, alpha));
        }
        let tick = plot.bottom() - level01(state.input.get(i).copied().unwrap_or(FLOOR_DB)) * plot.height();
        if state.input.get(i).copied().unwrap_or(FLOOR_DB) > DB_LO {
            painter.line_segment([Pos2::new(xa, tick), Pos2::new(xb, tick)], Stroke::new(2.0_f32, rgba(TEXT, 140.0)));
        }
    }

    // Pitch needles: in (dashed) and out (solid), and the drone's carrier.
    if snap.pitch_hz > 0.0 {
        let out_hz = snap.pitch_hz * 2f32.powf(s.pitch / 12.0);
        let (xi, xo) = (x_of(plot, snap.pitch_hz), x_of(plot, out_hz));
        dashed(&painter, Pos2::new(xi, plot.top()), Pos2::new(xi, plot.bottom()), Stroke::new(1.0_f32, rgba(TEXT, 90.0)));
        painter.line_segment([Pos2::new(xo, plot.top()), Pos2::new(xo, plot.bottom())], Stroke::new(1.0_f32, rgba(TEXT, 210.0)));
        painter.line_segment([Pos2::new(xi, plot.bottom() - 6.0), Pos2::new(xo, plot.bottom() - 6.0)], Stroke::new(2.0_f32, rgba(TEXT, 150.0)));
        let (left, right) = if xi < xo { (xi, xo) } else { (xo, xi) };
        painter.text(Pos2::new(left - 4.0, plot.top() + 2.0), Align2::RIGHT_TOP, format!("{:.0}", if xi < xo { snap.pitch_hz } else { out_hz }), FontId::monospace(8.0), rgba(TEXT, 160.0));
        painter.text(Pos2::new(right + 4.0, plot.top() + 2.0), Align2::LEFT_TOP, format!("{:.0}", if xi < xo { out_hz } else { snap.pitch_hz }), FontId::monospace(8.0), rgba(TEXT, 255.0));
    }
    if s.carrier == Carrier::Drone && s.vocoder > 0.01 && snap.carrier_hz > 0.0 {
        let c = Pos2::new(x_of(plot, snap.carrier_hz), plot.bottom() - 18.0);
        painter.add(EguiShape::convex_polygon(
            vec![c + Vec2::new(0.0, -7.0), c + Vec2::new(6.0, 0.0), c + Vec2::new(0.0, 7.0), c + Vec2::new(-6.0, 0.0)],
            rgba(LIME, 255.0),
            Stroke::NONE,
        ));
    }

    // Bulkheads.
    bulkhead(ui, &painter, rect, plot, params, setter, true, low, high);
    bulkhead(ui, &painter, rect, plot, params, setter, false, low, high);
}

/// One split: a bolted divider with a handle plate showing its frequency.
#[allow(clippy::too_many_arguments)]
fn bulkhead(ui: &mut Ui, painter: &Painter, rect: Rect, plot: Rect, params: &ChassisParams, setter: &ParamSetter, is_low: bool, low: f32, high: f32) {
    let (param, hz) = if is_low { (&params.low, low) } else { (&params.high, high) };
    let x = x_of(plot, hz);
    let plate = Rect::from_center_size(Pos2::new(x, rect.top() + 44.0), Vec2::new(50.0, 16.0));
    let hit = Rect::from_min_max(Pos2::new(x - 9.0, plate.top()), Pos2::new(x + 9.0, plot.bottom())).union(plate);
    let r = ui.interact(hit, ui.id().with(("bulkhead", is_low)), Sense::click_and_drag());
    let lit = r.hovered() || r.dragged();
    if lit {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    if r.drag_started() {
        setter.begin_set_parameter(param);
    }
    if r.dragged() {
        if let Some(pos) = r.interact_pointer_pos() {
            let wanted = hz_at(plot, pos.x);
            let value = if is_low {
                wanted.clamp(MIN_LOW_HZ, MAX_LOW_HZ.min(high / MIN_SPLIT_RATIO))
            } else {
                wanted.clamp(MIN_HIGH_HZ.max(low * MIN_SPLIT_RATIO), MAX_HIGH_HZ)
            };
            setter.set_parameter(param, value);
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

    let ink = rgba(TEXT, if lit { 255.0 } else { 150.0 });
    painter.line_segment([Pos2::new(x, plate.bottom()), Pos2::new(x, plot.bottom())], Stroke::new(if lit { 5.0_f32 } else { 3.0 }, rgba(TEXT, if lit { 30.0 } else { 14.0 })));
    painter.line_segment([Pos2::new(x, plate.bottom()), Pos2::new(x, plot.bottom())], Stroke::new(1.5_f32, ink));
    let mut y = plate.bottom() + 14.0;
    while y < plot.bottom() - 4.0 {
        painter.circle_filled(Pos2::new(x, y), 2.0, BG);
        painter.circle_stroke(Pos2::new(x, y), 2.0, Stroke::new(1.0_f32, ink));
        y += 28.0;
    }
    painter.rect_filled(plate, 3.0, if lit { GRID } else { BG });
    painter.rect_stroke(plate, 3.0, Stroke::new(1.0_f32, ink), egui::StrokeKind::Inside);
    let label = if hz >= 1000.0 { format!("{:.2}k", hz / 1000.0) } else { format!("{hz:.0}") };
    painter.text(plate.center(), Align2::CENTER_CENTER, label, FontId::monospace(10.0), ink);
}

struct Section<'a> {
    title: &'static str,
    color: [f32; 3],
    cols: usize,
    knobs: Vec<(&'a FloatParam, &'static str, bool)>,
}

fn sections(ui: &mut Ui, params: &ChassisParams, setter: &ParamSetter, state: &mut UiState) {
    let drone = params.carrier.value() == Carrier::Drone;
    let list = [
        Section {
            title: "IN",
            color: TEXT,
            cols: 3,
            knobs: vec![(&params.pitch, "PITCH", true), (&params.formant, "FORMANT", true), (&params.gate, "GATE", true), (&params.boxy, "BOX", true), (&params.weight, "WEIGHT", true)],
        },
        Section {
            title: "CHEST",
            color: ORANGE,
            cols: 2,
            knobs: vec![(&params.sub, "SUB", true), (&params.squash, "SQUASH", true), (&params.chest, "LEVEL", true)],
        },
        Section {
            title: "BODY",
            color: LIME,
            cols: 5,
            knobs: vec![
                (&params.ring, "RING", true),
                (&params.ring_hz, "RING HZ", true),
                (&params.vocoder, "VOCODER", true),
                (&params.note, "NOTE", drone),
                (&params.wave, "WAVE", true),
                (&params.shift, "SHIFT", true),
                (&params.comb, "COMB", true),
                (&params.comb_hz, "COMB HZ", true),
                (&params.grit, "GRIT", true),
                (&params.body, "LEVEL", true),
            ],
        },
        Section {
            title: "SHELL",
            color: CYAN,
            cols: 4,
            knobs: vec![
                (&params.cavity, "CAVITY", true),
                (&params.cavity_ms, "CAV MS", true),
                (&params.shine, "SHINE", true),
                (&params.shine_st, "SHN ST", true),
                (&params.plate, "PLATE", true),
                (&params.decay, "DECAY", true),
                (&params.shell, "LEVEL", true),
            ],
        },
        Section {
            title: "OUT",
            color: TEXT,
            cols: 3,
            knobs: vec![(&params.glue, "GLUE", true), (&params.tame, "TAME", true), (&params.push, "PUSH", true), (&params.ceiling, "CEIL", true), (&params.mix, "MIX", true)],
        },
    ];
    let columns: usize = list.iter().map(|s| s.cols).sum();
    let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), SECTION_H), Sense::hover());
    let col_w = (row.width() - SECTION_GAP * (list.len() - 1) as f32) / columns as f32;
    let painter = ui.painter().clone();

    let mut x = row.left();
    for section in &list {
        let w = col_w * section.cols as f32;
        let panel = Rect::from_min_size(Pos2::new(x, row.top()), Vec2::new(w, SECTION_H));
        painter.rect_filled(panel, 6.0, PANEL);
        painter.rect_filled(Rect::from_min_size(panel.min + Vec2::new(8.0, 0.0), Vec2::new(w - 16.0, 2.0)), 1.0, rgba(section.color, 200.0));
        painter.text(panel.min + Vec2::new(10.0, 13.0), Align2::LEFT_CENTER, section.title, FontId::monospace(9.0), rgba(section.color, 230.0));

        if section.title == "BODY" {
            let labels: Vec<String> = ["TRACK", "DRONE"].iter().map(|s| s.to_string()).collect();
            let chips_w = 2.0 * (5.0 * 6.4 + 18.0) + 4.0;
            let selected = params.carrier.value().to_index();
            let (clicked, _) = chips(ui, Pos2::new(panel.right() - chips_w - 8.0, panel.top() + 5.0), "carrier", &labels, selected, true, LIME);
            if let Some(i) = clicked {
                setter.begin_set_parameter(&params.carrier);
                setter.set_parameter_normalized(&params.carrier, i as f32);
                setter.end_set_parameter(&params.carrier);
            }
            painter.text(Pos2::new(panel.right() - chips_w - 16.0, panel.top() + 16.0), Align2::RIGHT_CENTER, "CARRIER", FontId::monospace(8.0), TEXT_DIM);
        }

        for (k, (param, label, enabled)) in section.knobs.iter().enumerate() {
            let (col, line) = (k % section.cols, k / section.cols);
            let cell = Rect::from_min_size(
                Pos2::new(x + col as f32 * col_w, panel.top() + SECTION_HEAD + line as f32 * KNOB_H),
                Vec2::new(col_w, KNOB_H),
            );
            knob(ui, &painter, cell, setter, param, (section.title, label), section.color, *enabled, state);
        }
        x += w + SECTION_GAP;
    }
}

/// A row of chips starting at `pos`. Returns the clicked index and the row's right edge.
fn chips(ui: &mut Ui, pos: Pos2, id: &str, labels: &[String], selected: usize, enabled: bool, color: [f32; 3]) -> (Option<usize>, f32) {
    let painter = ui.painter().clone();
    let mut x = pos.x;
    let mut clicked = None;
    for (i, label) in labels.iter().enumerate() {
        let w = label.chars().count() as f32 * 6.4 + 18.0;
        let rect = Rect::from_min_size(Pos2::new(x, pos.y), Vec2::new(w, CHIP_H - 4.0));
        let r = ui.interact(rect, ui.id().with((id, i)), if enabled { Sense::click() } else { Sense::hover() });
        let lit = enabled && r.hovered();
        let on = i == selected;
        if on {
            painter.rect_filled(rect.expand(2.0), 6.0, rgba(color, 30.0));
            painter.rect_filled(rect, 4.0, rgba(color, 235.0));
        } else {
            painter.rect_filled(rect, 4.0, if lit { GRID } else { BG });
        }
        let ink = if on { BG } else if lit { rgba(TEXT, 255.0) } else { TEXT_DIM };
        painter.text(rect.center(), Align2::CENTER_CENTER, label, FontId::monospace(9.0), ink);
        if r.clicked() {
            clicked = Some(i);
        }
        x += w + 4.0;
    }
    (clicked, x)
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
fn knob(ui: &mut Ui, painter: &Painter, cell: Rect, setter: &ParamSetter, param: &FloatParam, id: (&'static str, &'static str), color: [f32; 3], enabled: bool, state: &mut UiState) {
    let r = ui.interact(cell, ui.id().with(id), if enabled { Sense::click_and_drag() } else { Sense::hover() });
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

    let fade = if enabled { 1.0 } else { 0.3 };
    let norm = param.unmodulated_normalized_value();
    let center = Pos2::new(cell.center().x, cell.top() + 24.0);
    let radius = 18.0;
    let (start, end) = (PI * 0.75, PI * 2.25);
    let lit = enabled && (r.hovered() || r.dragged());
    let main = rgba(color, 255.0 * fade);
    if lit {
        state.editing = Some((id.1, param.to_string(), color));
    }

    painter.circle_filled(center, radius - 4.0, if lit { PANEL } else { Color32::from_rgb(0x0e, 0x12, 0x18) });
    painter.add(EguiShape::line(arc(center, radius, start, end), Stroke::new(2.5_f32, GRID)));
    // Parameters that straddle zero draw their arc out from zero; the others from the left stop.
    let origin = if param.preview_plain(0.0) < 0.0 && param.preview_plain(1.0) > 0.0 { param.preview_normalized(0.0) } else { 0.0 };
    if (norm - origin).abs() > 0.002 {
        let (a, b) = (start + (end - start) * origin, start + (end - start) * norm);
        let (from, to) = if a < b { (a, b) } else { (b, a) };
        painter.add(EguiShape::line(arc(center, radius, from, to), Stroke::new(7.0_f32, rgba(color, 28.0 * fade))));
        painter.add(EguiShape::line(arc(center, radius, from, to), Stroke::new(2.5_f32, main)));
    }
    if origin > 0.0 {
        let zero = start + (end - start) * origin;
        let z = Vec2::new(zero.cos(), zero.sin());
        painter.line_segment([center + z * (radius + 2.0), center + z * (radius + 5.0)], Stroke::new(1.0_f32, TEXT_DIM));
    }
    let angle = start + (end - start) * norm;
    let dir = Vec2::new(angle.cos(), angle.sin());
    painter.line_segment([center + dir * (radius - 11.0), center + dir * (radius - 5.0)], Stroke::new(2.0_f32, rgba(TEXT, 255.0 * fade)));

    painter.text(Pos2::new(cell.center().x, cell.top() + 51.0), Align2::CENTER_CENTER, id.1, FontId::monospace(8.0), TEXT_DIM);
    painter.text(Pos2::new(cell.center().x, cell.top() + 64.0), Align2::CENTER_CENTER, param.to_string(), FontId::monospace(10.0), main);
}
