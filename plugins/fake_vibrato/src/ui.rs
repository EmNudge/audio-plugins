//! The editor: the vibrato is drawn as a scrolling pitch trace that you drag to control.
//!
//! The trace shows pitch deviation in cents over time. The playhead sits at "now" and is driven by
//! the real LFO phase from the audio thread; the left side is the recent past and the right side
//! is the near future. Horizontal drag sets the rate, vertical drag sets the depth.

use nih_plug::prelude::*;
use nih_plug_egui::egui::{
    self, Align2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2,
};
use std::f32::consts::TAU;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::{presets, VibratoParams};

const BG: Color32 = Color32::from_rgb(0x14, 0x14, 0x17);
const PANEL: Color32 = Color32::from_rgb(0x1e, 0x1e, 0x23);
const GRID: Color32 = Color32::from_rgb(0x2c, 0x2c, 0x33);
/// Bar labels sit on top of the accent fill, so they need to be lighter than `TEXT_DIM`.
const BAR_LABEL: Color32 = Color32::from_rgb(0xb8, 0xb8, 0xc4);
const TEXT_DIM: Color32 = Color32::from_rgb(0x6a, 0x6a, 0x75);
const ACCENT: Color32 = Color32::from_rgb(0xff, 0x5a, 0x1f);
const CYAN: Color32 = Color32::from_rgb(0x2e, 0xe6, 0xd6);

/// Seconds of trace shown behind and ahead of the playhead.
const PAST_SECS: f32 = 0.75;
const FUTURE_SECS: f32 = 0.25;
/// How much of the half-height 100 cents occupies.
const CENTS_FILL: f32 = 0.9;
const MAX_CENTS: f32 = 100.0;

pub fn draw(ui: &mut Ui, params: &Arc<VibratoParams>, setter: &ParamSetter, phase: &Arc<AtomicF32>) {
    ui.ctx().request_repaint();

    let rate = params.rate.value();
    let depth = params.depth.value();
    let mix = params.mix.value();

    header(ui, params, setter, rate, depth, mix);
    ui.add_space(8.0);
    trace(ui, params, setter, rate, depth, mix, phase.load(Ordering::Relaxed));
    ui.add_space(10.0);
    bar(ui, setter, &params.rate, "RATE", ACCENT);
    ui.add_space(6.0);
    bar(ui, setter, &params.mix, "MIX", CYAN);
}

fn header(ui: &mut Ui, params: &Arc<VibratoParams>, setter: &ParamSetter, rate: f32, depth: f32, mix: f32) {
    // The readouts below fill the right of their row, so the preset picker gets a row of its own,
    // shared with the title (about 80px wide; the picker is 206px of the 492px content width).
    ui.horizontal(|ui| {
        ui.set_min_height(20.0);
        let theme = preset_kit::ui::Theme { fill: PANEL, hover: GRID, ink: Color32::WHITE, dim: TEXT_DIM };
        preset_kit::ui::picker_top_right(ui, 20.0, &presets::BANK, &**params, setter, &theme);
        ui.label(egui::RichText::new("FAKE VIBRATO").monospace().size(11.0).color(TEXT_DIM));
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            for (text, color) in [
                (format!("{:>3.0}%", mix * 100.0), TEXT_DIM),
                (format!("{:>3.0}¢", depth), CYAN),
                (format!("{:>4.1}Hz", rate), ACCENT),
            ] {
                ui.label(egui::RichText::new(text).monospace().size(18.0).color(color));
                ui.add_space(10.0);
            }
        });
    });
}

fn trace(
    ui: &mut Ui,
    params: &Arc<VibratoParams>,
    setter: &ParamSetter,
    rate: f32,
    depth: f32,
    mix: f32,
    phase: f32,
) {
    let size = Vec2::new(ui.available_width(), ui.available_height() - 82.0);
    let (response, painter) = ui.allocate_painter(size, Sense::click_and_drag());
    let rect = response.rect;
    let cy = rect.center().y;
    let half_h = rect.height() * 0.5 * CENTS_FILL;
    let y_of = |cents: f32| cy - cents / MAX_CENTS * half_h;
    let now_x = rect.left() + rect.width() * PAST_SECS / (PAST_SECS + FUTURE_SECS);
    let x_of = |t: f32| now_x + t / (PAST_SECS + FUTURE_SECS) * rect.width();

    painter.rect_filled(rect, 6.0, PANEL);

    // Grid: 0 and +-50 / +-100 cents, with the centre line stronger.
    for cents in [-100.0, -50.0, 0.0, 50.0, 100.0] {
        let y = y_of(cents);
        let stroke = Stroke::new(if cents == 0.0 { 1.5_f32 } else { 1.0 }, GRID);
        painter.line_segment([Pos2::new(rect.left() + 6.0, y), Pos2::new(rect.right() - 6.0, y)], stroke);
        if cents != 0.0 {
            painter.text(
                Pos2::new(rect.left() + 8.0, y - 2.0),
                Align2::LEFT_BOTTOM,
                format!("{:+.0}", cents),
                FontId::monospace(9.0),
                TEXT_DIM,
            );
        }
    }

    // Depth envelope: the bounds the pitch swings between.
    let env = Color32::from_rgba_unmultiplied(0x2e, 0xe6, 0xd6, 90);
    for sign in [1.0, -1.0] {
        let y = y_of(sign * depth);
        painter.add(Shape::dashed_line(
            &[Pos2::new(rect.left() + 6.0, y), Pos2::new(rect.right() - 14.0, y)],
            Stroke::new(1.0_f32, env),
            4.0,
            4.0,
        ));
        painter.circle_filled(Pos2::new(rect.right() - 8.0, y), 3.5, CYAN);
    }

    // Pitch deviation at `t` seconds from now. A longer delay lowers the pitch, so the deviation
    // follows -cos of the LFO phase, matching what the DSP does.
    let cents_at = |t: f32| -depth * (TAU * (phase + rate * t)).cos();
    let alpha = |a: f32| (a * (0.25 + 0.75 * mix)) as u8;

    // Future (dim), then past (fading in toward the playhead), each with a soft glow underneath.
    const N: usize = 220;
    let segment = |t0: f32, t1: f32, a: f32, strong: bool| {
        let p0 = Pos2::new(x_of(t0), y_of(cents_at(t0)));
        let p1 = Pos2::new(x_of(t1), y_of(cents_at(t1)));
        let glow = Color32::from_rgba_unmultiplied(0xff, 0x5a, 0x1f, alpha(a * 0.25));
        let core = Color32::from_rgba_unmultiplied(0xff, 0x5a, 0x1f, alpha(a));
        if strong {
            painter.line_segment([p0, p1], Stroke::new(7.0_f32, glow));
        }
        painter.line_segment([p0, p1], Stroke::new(if strong { 2.5_f32 } else { 1.5 }, core));
    };
    for i in 0..N {
        let u0 = i as f32 / N as f32;
        let u1 = (i + 1) as f32 / N as f32;
        // Past: u=0 is oldest, u=1 is now.
        segment(-PAST_SECS * (1.0 - u0), -PAST_SECS * (1.0 - u1), 255.0 * u0.powf(1.5), true);
        // Future: u=0 is now, u=1 is furthest ahead.
        segment(FUTURE_SECS * u0, FUTURE_SECS * u1, 110.0 * (1.0 - u0), false);
    }

    // Playhead.
    painter.line_segment(
        [Pos2::new(now_x, rect.top() + 6.0), Pos2::new(now_x, rect.bottom() - 6.0)],
        Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 40)),
    );
    let head = Pos2::new(now_x, y_of(cents_at(0.0)));
    for (r, a) in [(14.0, 25), (9.0, 55)] {
        painter.circle_filled(head, r, Color32::from_rgba_unmultiplied(0xff, 0x5a, 0x1f, a));
    }
    painter.circle_filled(head, 5.0, Color32::WHITE);

    painter.text(
        Pos2::new(rect.right() - 8.0, rect.bottom() - 6.0),
        Align2::RIGHT_BOTTOM,
        "drag here: x = rate  y = depth",
        FontId::monospace(9.0),
        TEXT_DIM,
    );

    // Interaction: the pointer position inside the trace is the setting.
    if response.drag_started() {
        setter.begin_set_parameter(&params.rate);
        setter.begin_set_parameter(&params.depth);
    }
    if response.dragged() || response.drag_started() {
        if let Some(pos) = response.interact_pointer_pos() {
            let x = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
            let cents = ((pos.y - cy).abs() / half_h * MAX_CENTS).clamp(0.0, MAX_CENTS);
            setter.set_parameter_normalized(&params.rate, x);
            setter.set_parameter(&params.depth, cents);
        }
    }
    if response.drag_stopped() {
        setter.end_set_parameter(&params.rate);
        setter.end_set_parameter(&params.depth);
    }
    if response.double_clicked() {
        reset(setter, &params.rate);
        reset(setter, &params.depth);
    }
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
    painter.text(
        Pos2::new(rect.left() + 8.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::monospace(11.0),
        BAR_LABEL,
    );
    painter.text(
        Pos2::new(rect.right() - 8.0, rect.center().y),
        Align2::RIGHT_CENTER,
        param.to_string(),
        FontId::monospace(12.0),
        color,
    );
    painter.rect_stroke(rect, 4.0, Stroke::new(1.0, GRID), egui::StrokeKind::Inside);

    if response.drag_started() {
        setter.begin_set_parameter(param);
    }
    if response.dragged() || response.drag_started() {
        if let Some(pos) = response.interact_pointer_pos() {
            let v = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
            setter.set_parameter_normalized(param, v);
        }
    }
    if response.drag_stopped() {
        setter.end_set_parameter(param);
    }
    if response.double_clicked() {
        reset(setter, param);
    }
}

fn reset(setter: &ParamSetter, param: &FloatParam) {
    setter.begin_set_parameter(param);
    setter.set_parameter(param, param.default_plain_value());
    setter.end_set_parameter(param);
}

pub fn background() -> Color32 {
    BG
}
