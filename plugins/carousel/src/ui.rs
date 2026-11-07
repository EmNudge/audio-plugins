//! The editor: voices riding a carousel.
//!
//! Seen from above, each voice is a horse on the platform. Its angle is the phase of its LFO and
//! its distance from the hub is its delay (on a square-root scale, so short delays stay readable),
//! so it spirals in and out as the delay sweeps, trailing a glowing path of the last 1.5 s. The
//! band between the two outer rings is the sweep; the bright ring in the middle of it is the
//! delay, and dragging the platform sets it. Voices are coloured by where they sit in the stereo
//! field: orange left, yellow centre, cyan right.
//!
//! Beside it the same delays scroll as traces (drag up and down for depth), over a mirror of the
//! dry input (above the line) and the wet voices (below it). Chips pick the mode, shape, voice
//! count and tempo sync; locked controls dim, and show the value the mode has fixed.
//!
//! Knobs: drag vertically (hold shift for fine), double-click to reset.

use nih_plug::prelude::*;
use nih_plug_egui::egui::{
    self, Align2, Color32, FontId, Painter, Pos2, Rect, Sense, Shape as EguiShape, Stroke, Ui, Vec2,
};
use std::f32::consts::{FRAC_PI_2, PI, TAU};
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::dsp::{Meter, Mode, Shape, Voicing, COLS, MAX_DELAY_MS, MAX_REACH_MS, MAX_VOICES, MIN_DELAY_MS};
use crate::dsp::Division;
use crate::presets;
use crate::CarouselParams;

const BG: Color32 = Color32::from_rgb(0x0a, 0x0e, 0x14);
const PANEL: Color32 = Color32::from_rgb(0x13, 0x19, 0x22);
const GRID: Color32 = Color32::from_rgb(0x23, 0x2c, 0x39);
const TEXT_DIM: Color32 = Color32::from_rgb(0x7b, 0x87, 0x94);
const TEXT: [f32; 3] = [238.0, 240.0, 236.0];
const ORANGE: [f32; 3] = [255.0, 106.0, 31.0];
const YELLOW: [f32; 3] = [255.0, 214.0, 10.0];
const CYAN: [f32; 3] = [45.0, 226.0, 230.0];

const CHIP_H: f32 = 22.0;
const SELECT_H: f32 = 30.0;
const KNOB_ROW: f32 = 100.0;
/// Delay marks drawn as rings and gridlines.
const MARKS_MS: [f32; 4] = [5.0, 10.0, 20.0, 40.0];

#[derive(Default)]
pub struct UiState {
    last_head: usize,
    /// Peak pitch deviation of the first voice, held and released slowly, in cents.
    cents_peak: f32,
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

/// Orange at the left of the stereo field, yellow in the middle, cyan at the right.
fn voice_color(pan: f32) -> [f32; 3] {
    if pan < 0.0 {
        mix(ORANGE, YELLOW, 1.0 + pan)
    } else {
        mix(YELLOW, CYAN, pan)
    }
}

/// One snapshot of the meter, oldest column first.
struct Columns {
    delay: [[f32; COLS]; MAX_VOICES],
    phase: [[f32; COLS]; MAX_VOICES],
    dry: [f32; COLS],
    wet: [f32; COLS],
    rate_hz: f32,
}

fn snapshot(meter: &Meter, state: &mut UiState) -> Columns {
    let head = meter.head.load(Ordering::Acquire);
    let mut c = Columns {
        delay: [[0.0; COLS]; MAX_VOICES],
        phase: [[0.0; COLS]; MAX_VOICES],
        dry: [0.0; COLS],
        wet: [0.0; COLS],
        rate_hz: meter.rate_hz.load(Ordering::Relaxed),
    };
    for k in 0..COLS {
        let slot = (head + k) % COLS;
        for i in 0..MAX_VOICES {
            c.delay[i][k] = meter.delay[i][slot].load(Ordering::Relaxed);
            c.phase[i][k] = meter.phase[i][slot].load(Ordering::Relaxed);
        }
        c.dry[k] = meter.dry[slot].load(Ordering::Relaxed);
        c.wet[k] = meter.wet[slot].load(Ordering::Relaxed);
    }
    // Rise fast, fall slowly; only while the audio is running.
    let advanced = head.wrapping_sub(state.last_head).min(COLS) as f32;
    state.last_head = head;
    let cents = meter.cents.load(Ordering::Relaxed).abs();
    state.cents_peak = if advanced > 0.0 { cents.max(state.cents_peak * 0.985) } else { state.cents_peak };
    c
}

pub fn draw(ui: &mut Ui, params: &Arc<CarouselParams>, setter: &ParamSetter, meter: &Arc<Meter>, state: &mut UiState) {
    ui.ctx().request_repaint();
    let cols = snapshot(meter, state);
    let settings = params.settings();
    let voicing = Voicing::of(&settings);

    header(ui, params, setter, &cols, state);
    ui.add_space(8.0);
    mode_row(ui, params, setter);
    ui.add_space(8.0);

    let display_h = ui.available_height() - SELECT_H - KNOB_ROW - 16.0;
    let (area, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), display_h), Sense::hover());
    let carousel_w = display_h.min(area.width() * 0.5);
    let left = Rect::from_min_size(area.min, Vec2::new(carousel_w, display_h));
    let right = Rect::from_min_max(Pos2::new(left.right() + 8.0, area.top()), area.max);
    carousel(ui, left, params, setter, &cols, &voicing);
    traces(ui, right, params, setter, &cols, &voicing);

    ui.add_space(8.0);
    selectors(ui, params, setter);
    ui.add_space(8.0);
    knobs(ui, params, setter, &cols, &voicing);
}

fn header(ui: &mut Ui, params: &CarouselParams, setter: &ParamSetter, c: &Columns, state: &UiState) {
    let theme = preset_kit::ui::Theme { fill: PANEL, hover: GRID, ink: rgba(TEXT, 255.0), dim: TEXT_DIM };
    preset_kit::ui::picker_top_right(ui, 24.0, &presets::BANK, params, setter, &theme);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("CAROUSEL").monospace().size(11.0).color(TEXT_DIM));
        ui.label(egui::RichText::new("three-voice bbd chorus").monospace().size(9.0).color(GRID));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(preset_kit::ui::WIDTH + 14.0);
            for (text, color) in [
                (format!("{:>3.0}¢", state.cents_peak), CYAN),
                (format!("{:>5.2}Hz", c.rate_hz), ORANGE),
            ] {
                ui.label(egui::RichText::new(text).monospace().size(18.0).color(rgba(color, 255.0)));
                ui.add_space(10.0);
            }
        });
    });
}

/// A row of chips starting at `pos`. Returns the clicked index and the row's right edge. Locked
/// rows dim and ignore the pointer.
fn chips(ui: &mut Ui, pos: Pos2, id: &str, labels: &[String], selected: usize, enabled: bool, color: [f32; 3]) -> (Option<usize>, f32) {
    let painter = ui.painter().clone();
    let mut x = pos.x;
    let mut clicked = None;
    for (i, label) in labels.iter().enumerate() {
        let w = label.chars().count() as f32 * 6.4 + 18.0;
        let rect = Rect::from_min_size(Pos2::new(x, pos.y), Vec2::new(w, CHIP_H));
        let r = ui.interact(rect, ui.id().with((id, i)), if enabled { Sense::click() } else { Sense::hover() });
        let lit = enabled && r.hovered();
        let on = i == selected;
        let fade = if enabled { 1.0 } else { 0.4 };
        if on {
            painter.rect_filled(rect.expand(2.0), 6.0, rgba(color, 30.0 * fade));
            painter.rect_filled(rect, 4.0, rgba(color, 235.0 * fade));
        } else {
            painter.rect_filled(rect, 4.0, if lit { GRID } else { PANEL });
        }
        let ink = if on { BG } else if lit { rgba(TEXT, 255.0) } else { TEXT_DIM };
        painter.text(rect.center(), Align2::CENTER_CENTER, label, FontId::monospace(10.0), ink);
        if r.clicked() {
            clicked = Some(i);
        }
        x += w + 4.0;
    }
    (clicked, x)
}

fn pick<T: Enum + Copy + PartialEq>(setter: &ParamSetter, param: &EnumParam<T>, index: usize) {
    setter.begin_set_parameter(param);
    setter.set_parameter_normalized(param, index as f32 / (T::variants().len() - 1) as f32);
    setter.end_set_parameter(param);
}

fn mode_row(ui: &mut Ui, params: &CarouselParams, setter: &ParamSetter) {
    let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), CHIP_H), Sense::hover());
    ui.painter().text(Pos2::new(row.left(), row.center().y), Align2::LEFT_CENTER, "MODE", FontId::monospace(9.0), TEXT_DIM);
    let labels: Vec<String> = Mode::variants().iter().map(|n| n.to_uppercase()).collect();
    let (clicked, _) = chips(ui, Pos2::new(row.left() + 52.0, row.top()), "mode", &labels, params.mode.value().to_index(), true, ORANGE);
    if let Some(i) = clicked {
        pick(setter, &params.mode, i);
    }
}

fn selectors(ui: &mut Ui, params: &CarouselParams, setter: &ParamSetter) {
    let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), SELECT_H), Sense::hover());
    let mode = params.mode.value();
    let y = row.top() + (SELECT_H - CHIP_H) * 0.5;
    let painter = ui.painter().clone();
    let label = |x: f32, text: &str| {
        painter.text(Pos2::new(x, row.center().y), Align2::LEFT_CENTER, text, FontId::monospace(9.0), TEXT_DIM);
    };

    let mut x = row.left();
    label(x, "SHAPE");
    let shapes: Vec<String> = Shape::variants().iter().map(|n| n.to_uppercase()).collect();
    let (clicked, end) = chips(ui, Pos2::new(x + 52.0, y), "shape", &shapes, params.shape.value().to_index(), mode.free_voices(), YELLOW);
    if let Some(i) = clicked {
        pick(setter, &params.shape, i);
    }

    x = end + 22.0;
    label(x, "VOICES");
    let counts: Vec<String> = (1..=MAX_VOICES).map(|n| n.to_string()).collect();
    let selected = Voicing::of(&params.settings()).voices - 1;
    let (clicked, end) = chips(ui, Pos2::new(x + 58.0, y), "voices", &counts, selected, mode.free_voices(), CYAN);
    if let Some(i) = clicked {
        setter.begin_set_parameter(&params.voices);
        setter.set_parameter(&params.voices, i as i32 + 1);
        setter.end_set_parameter(&params.voices);
    }

    x = end + 22.0;
    label(x, "SYNC");
    let divisions: Vec<String> = Division::variants().iter().map(|n| n.to_uppercase()).collect();
    let (clicked, _) = chips(ui, Pos2::new(x + 40.0, y), "sync", &divisions, params.sync.value().to_index(), mode.free_rate(), ORANGE);
    if let Some(i) = clicked {
        pick(setter, &params.sync, i);
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

/// A glowing trail through `points`, oldest first, brightening toward the end.
fn trail(painter: &Painter, points: &[Option<Pos2>], color: [f32; 3]) {
    let n = points.len() as f32;
    for k in 1..points.len() {
        let (Some(a), Some(b)) = (points[k - 1], points[k]) else { continue };
        let age = k as f32 / n;
        painter.line_segment([a, b], Stroke::new(6.0_f32, rgba(color, 30.0 * age)));
        painter.line_segment([a, b], Stroke::new(1.6_f32, rgba(color, 235.0 * age * age)));
    }
}

/// Distance from the hub as a fraction of the platform radius: square root of the delay.
fn radial(ms: f32) -> f32 {
    0.06 + 0.94 * (ms / MAX_REACH_MS).clamp(0.0, 1.0).sqrt()
}

fn carousel(ui: &mut Ui, rect: Rect, params: &CarouselParams, setter: &ParamSetter, c: &Columns, v: &Voicing) {
    let response = ui.interact(rect, ui.id().with("carousel"), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 6.0, PANEL);
    let center = rect.center();
    let radius = rect.width().min(rect.height()) * 0.5 - 14.0;
    let editable = params.mode.value().free_delay();
    let lit = editable && (response.hovered() || response.dragged());

    // The sweep band, then the guide rings with their delays.
    let (lo, hi) = (v.delay_ms - v.depth_ms, v.delay_ms + v.depth_ms);
    let (r_lo, r_hi) = (radial(lo.max(0.0)) * radius, radial(hi) * radius);
    painter.circle_stroke(center, (r_lo + r_hi) * 0.5, Stroke::new((r_hi - r_lo).max(1.0), rgba(ORANGE, 12.0)));
    painter.circle_stroke(center, radius, Stroke::new(1.0_f32, GRID));
    for ms in MARKS_MS {
        let r = radial(ms) * radius;
        painter.circle_stroke(center, r, Stroke::new(1.0_f32, rgba(TEXT, 14.0)));
        painter.text(Pos2::new(center.x + 3.0, center.y - r - 1.0), Align2::LEFT_BOTTOM, format!("{ms:.0}"), FontId::monospace(8.0), TEXT_DIM);
    }
    painter.text(Pos2::new(rect.left() + 10.0, rect.top() + 8.0), Align2::LEFT_TOP, "delay, ms", FontId::monospace(9.0), TEXT_DIM);
    if editable {
        painter.text(Pos2::new(rect.right() - 10.0, rect.top() + 8.0), Align2::RIGHT_TOP, "drag: delay", FontId::monospace(9.0), TEXT_DIM);
    }

    // The delay: the ring the voices circle around.
    let r_base = radial(v.delay_ms) * radius;
    painter.circle_stroke(center, r_base, Stroke::new(5.0_f32, rgba(ORANGE, if lit { 40.0 } else { 18.0 })));
    painter.circle_stroke(center, r_base, Stroke::new(1.0_f32, rgba(ORANGE, if lit { 190.0 } else { 90.0 })));

    for i in 0..v.voices {
        let color = voice_color(v.pan[i]);
        let points: Vec<Option<Pos2>> = (0..COLS)
            .map(|k| {
                let d = c.delay[i][k];
                (d > 0.0).then(|| {
                    let a = TAU * c.phase[i][k] - FRAC_PI_2;
                    center + Vec2::new(a.cos(), a.sin()) * radial(d) * radius
                })
            })
            .collect();
        trail(&painter, &points, color);
        if let Some(head) = points[COLS - 1] {
            painter.line_segment([center, head], Stroke::new(1.0_f32, rgba(color, 40.0)));
            painter.circle_filled(head, 13.0, rgba(color, 40.0));
            painter.circle_filled(head, 6.5, rgba(color, 255.0));
            painter.circle_filled(head, 2.0, BG);
        }
    }
    painter.circle_filled(center, 5.0, rgba(TEXT, 255.0));

    if editable {
        if response.drag_started() {
            setter.begin_set_parameter(&params.delay);
        }
        if response.dragged() || response.drag_started() {
            if let Some(pos) = response.interact_pointer_pos() {
                let frac = (((pos - center).length() / radius - 0.06) / 0.94).clamp(0.0, 1.0);
                setter.set_parameter(&params.delay, (frac * frac * MAX_REACH_MS).clamp(MIN_DELAY_MS, MAX_DELAY_MS));
            }
        }
        if response.drag_stopped() {
            setter.end_set_parameter(&params.delay);
        }
        if response.double_clicked() {
            setter.begin_set_parameter(&params.delay);
            setter.set_parameter(&params.delay, params.delay.default_plain_value());
            setter.end_set_parameter(&params.delay);
        }
    }
}

fn traces(ui: &mut Ui, rect: Rect, params: &CarouselParams, setter: &ParamSetter, c: &Columns, v: &Voicing) {
    let response = ui.interact(rect, ui.id().with("traces"), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 6.0, PANEL);
    let editable = params.mode.value().free_depth();
    let lit = editable && (response.hovered() || response.dragged());

    let plot = Rect::from_min_max(Pos2::new(rect.left() + 36.0, rect.top() + 22.0), Pos2::new(rect.right() - 14.0, rect.bottom() - 84.0));
    let strip = Rect::from_min_max(Pos2::new(plot.left(), plot.bottom() + 18.0), Pos2::new(plot.right(), rect.bottom() - 10.0));
    let x_of = |k: usize| plot.left() + plot.width() * k as f32 / (COLS - 1) as f32;
    let y_of = |ms: f32| plot.bottom() - plot.height() * (ms / MAX_REACH_MS).clamp(0.0, 1.0).sqrt();

    painter.text(Pos2::new(rect.left() + 10.0, rect.top() + 8.0), Align2::LEFT_TOP, "voice delays", FontId::monospace(9.0), TEXT_DIM);
    if editable {
        painter.text(Pos2::new(rect.right() - 10.0, rect.top() + 8.0), Align2::RIGHT_TOP, "drag: depth", FontId::monospace(9.0), TEXT_DIM);
    }
    for ms in MARKS_MS {
        let y = y_of(ms);
        painter.line_segment([Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)], Stroke::new(1.0_f32, rgba(TEXT, 14.0)));
        painter.text(Pos2::new(plot.left() - 6.0, y), Align2::RIGHT_CENTER, format!("{ms:.0}"), FontId::monospace(8.0), TEXT_DIM);
    }
    // The sweep limits, so the depth reads against the traces.
    for ms in [v.delay_ms - v.depth_ms, v.delay_ms + v.depth_ms] {
        let y = y_of(ms.max(0.0));
        painter.line_segment([Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)], Stroke::new(1.0_f32, rgba(ORANGE, if lit { 90.0 } else { 40.0 })));
    }
    for i in 0..v.voices {
        let points: Vec<Option<Pos2>> = (0..COLS).map(|k| (c.delay[i][k] > 0.0).then(|| Pos2::new(x_of(k), y_of(c.delay[i][k])))).collect();
        trail(&painter, &points, voice_color(v.pan[i]));
    }

    // Dry above the line, wet below it.
    let cy = strip.center().y;
    let half = strip.height() * 0.5;
    painter.line_segment([Pos2::new(strip.left(), cy), Pos2::new(strip.right(), cy)], Stroke::new(1.0_f32, GRID));
    painter.text(Pos2::new(strip.left() - 6.0, cy - half * 0.5), Align2::RIGHT_CENTER, "dry", FontId::monospace(8.0), TEXT_DIM);
    painter.text(Pos2::new(strip.left() - 6.0, cy + half * 0.5), Align2::RIGHT_CENTER, "wet", FontId::monospace(8.0), TEXT_DIM);
    let colw = strip.width() / COLS as f32;
    for k in 0..COLS {
        let age = k as f32 / COLS as f32;
        let x = x_of(k);
        let dry = c.dry[k].min(1.0).powf(0.6) * half;
        let wet = c.wet[k].min(1.0).powf(0.6) * half;
        let w = (colw * 0.8).max(1.0);
        if dry > 0.5 {
            painter.rect_filled(Rect::from_min_max(Pos2::new(x - w * 0.5, cy - dry), Pos2::new(x + w * 0.5, cy)), 0.0, rgba(TEXT, 40.0 + 150.0 * age * age));
        }
        if wet > 0.5 {
            painter.rect_filled(Rect::from_min_max(Pos2::new(x - w * 0.5, cy), Pos2::new(x + w * 0.5, cy + wet)), 0.0, rgba(YELLOW, 40.0 + 190.0 * age * age));
        }
    }

    if editable {
        if response.drag_started() {
            setter.begin_set_parameter(&params.depth);
        }
        if response.dragged() {
            let v = params.depth.unmodulated_normalized_value() - response.drag_delta().y / plot.height().max(1.0);
            setter.set_parameter_normalized(&params.depth, v.clamp(0.0, 1.0));
        }
        if response.drag_stopped() {
            setter.end_set_parameter(&params.depth);
        }
        if response.double_clicked() {
            setter.begin_set_parameter(&params.depth);
            setter.set_parameter(&params.depth, params.depth.default_plain_value());
            setter.end_set_parameter(&params.depth);
        }
    }
}

fn knobs(ui: &mut Ui, params: &CarouselParams, setter: &ParamSetter, c: &Columns, v: &Voicing) {
    let (response, painter) = ui.allocate_painter(Vec2::new(ui.available_width(), KNOB_ROW), Sense::hover());
    let row = response.rect;
    let mode = params.mode.value();
    // Locked knobs show the value the mode has fixed.
    let fixed_rate = (!mode.free_rate() || params.sync.value() != Division::Off).then(|| format!("{:.2} Hz", c.rate_hz));
    let fixed_delay = (!mode.free_delay()).then(|| format!("{:.1} ms", v.delay_ms));
    let cells: [(&FloatParam, &str, [f32; 3], bool, Option<String>); 6] = [
        (&params.rate, "RATE", ORANGE, mode.free_rate() && params.sync.value() == Division::Off, fixed_rate),
        (&params.depth, "DEPTH", ORANGE, mode.free_depth(), None),
        (&params.delay, "DELAY", ORANGE, mode.free_delay(), fixed_delay),
        (&params.spread, "SPREAD", CYAN, true, None),
        (&params.analog, "ANALOG", YELLOW, true, None),
        (&params.mix, "MIX", TEXT, true, None),
    ];
    let width = row.width() / cells.len() as f32;
    for (i, (param, label, color, enabled, shown)) in cells.iter().enumerate() {
        let cell = Rect::from_min_size(Pos2::new(row.left() + i as f32 * width, row.top()), Vec2::new(width, KNOB_ROW));
        knob(ui, &painter, cell, setter, param, label, *color, *enabled, shown.as_deref());
    }
}

#[allow(clippy::too_many_arguments)]
fn knob(ui: &mut Ui, painter: &Painter, cell: Rect, setter: &ParamSetter, param: &FloatParam, label: &str, color: [f32; 3], enabled: bool, shown: Option<&str>) {
    let r = ui.interact(cell, ui.id().with(label), if enabled { Sense::click_and_drag() } else { Sense::hover() });
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
    let center = Pos2::new(cell.center().x, cell.top() + 34.0);
    let radius = 25.0;
    let (start, end) = (PI * 0.75, PI * 2.25);
    let lit = enabled && (r.hovered() || r.dragged());
    let main = rgba(color, 255.0 * fade);

    painter.circle_filled(center, radius - 5.0, if lit { PANEL } else { Color32::from_rgb(0x0f, 0x14, 0x1b) });
    painter.add(EguiShape::line(arc(center, radius, start, end), Stroke::new(3.0_f32, GRID)));
    if norm > 0.002 {
        let to = start + (end - start) * norm;
        painter.add(EguiShape::line(arc(center, radius, start, to), Stroke::new(8.0_f32, rgba(color, 30.0 * fade))));
        painter.add(EguiShape::line(arc(center, radius, start, to), Stroke::new(3.0_f32, main)));
    }
    let angle = start + (end - start) * norm;
    let dir = Vec2::new(angle.cos(), angle.sin());
    painter.line_segment([center + dir * (radius - 14.0), center + dir * (radius - 7.0)], Stroke::new(2.0_f32, rgba(TEXT, 255.0 * fade)));

    painter.text(Pos2::new(cell.center().x, cell.top() + 69.0), Align2::CENTER_CENTER, label, FontId::monospace(9.0), TEXT_DIM);
    let text = shown.map_or_else(|| param.to_string(), str::to_owned);
    painter.text(Pos2::new(cell.center().x, cell.top() + 85.0), Align2::CENTER_CENTER, text, FontId::monospace(11.0), if shown.is_some() { rgba(color, 150.0) } else { main });
}
