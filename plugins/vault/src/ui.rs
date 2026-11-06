//! The editor: a vaulted tunnel of arches, and the decay-by-frequency graph.
//!
//! The tunnel is the tail seen from the listener's end. Each arch is one 50 ms slice of the wet
//! signal, the newest at the front and the oldest far away, so the sound visibly flies off into
//! the distance as it decays, and the arches dim with it. The floor of an arch is the low band
//! (orange), its pillars the mids (lime) and its ceiling the highs (cyan). A small white arch
//! inside is the input of the same moment, so the pre-delay and the early reflections show up
//! as the gap and the echoes between the white arches and the coloured ones. The tunnel widens
//! with Size (drag it sideways to set it), twists with the frequency shift, and turns to ice when
//! frozen.
//!
//! Below it, the graph shows the reverb time at every frequency, which is exactly what the loop
//! filters do. The live spectrum of the wet signal rises behind the curve. Drag the white handle
//! for Decay, the orange and cyan ones for the low and high multipliers, and the lime grips for
//! the two crossovers.
//!
//! Knobs: drag vertically (hold shift for fine), double-click to reset.

use nih_plug::prelude::*;
use nih_plug_egui::egui::{
    self, Align2, Color32, FontId, Mesh, Painter, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, Vec2,
};
use std::f32::consts::PI;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::dsp::{band_freq, decay_at, mean_delay_s, Meter, Space, BANDS, COLS, COL_MS, MAX_DECAY_S, MIN_DECAY_S};
use crate::presets;
use crate::VaultParams;

const BG: Color32 = Color32::from_rgb(0x07, 0x09, 0x12);
const PANEL: Color32 = Color32::from_rgb(0x10, 0x13, 0x1f);
const GRID: Color32 = Color32::from_rgb(0x25, 0x2a, 0x3d);
const TEXT_DIM: Color32 = Color32::from_rgb(0x7a, 0x82, 0x9c);
const TEXT: [f32; 3] = [236.0, 240.0, 248.0];
const ORANGE: [f32; 3] = [255.0, 122.0, 41.0];
const LIME: [f32; 3] = [200.0, 255.0, 61.0];
const CYAN: [f32; 3] = [56.0, 214.0, 255.0];
const ICE: [f32; 3] = [190.0, 225.0, 255.0];

const DECAY_H: f32 = 170.0;
const KNOB_H: f32 = 84.0;
const SPACES: [(Space, &str); 5] =
    [(Space::Room, "ROOM"), (Space::Hall, "HALL"), (Space::Plate, "PLATE"), (Space::Chamber, "CHAMBER"), (Space::Abyss, "ABYSS")];

/// The decay graph's vertical range in seconds.
const RT_MIN: f32 = 0.05;
const RT_MAX: f32 = 80.0;

pub struct UiState {
    last_head: usize,
    /// Band levels (0..1), risen at once and fallen over about half a second.
    bands: [f32; BANDS],
}

impl Default for UiState {
    fn default() -> Self {
        Self { last_head: 0, bands: [0.0; BANDS] }
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

/// Orange at the lows through lime to cyan at the highs.
fn spectrum_color(t: f32) -> [f32; 3] {
    if t < 0.5 {
        mix(ORANGE, LIME, t * 2.0)
    } else {
        mix(LIME, CYAN, (t - 0.5) * 2.0)
    }
}

/// An RMS level on a -66..0 dB scale, as 0..1.
fn lvl(rms: f32) -> f32 {
    ((20.0 * (rms + 1e-6).log10() + 66.0) / 66.0).clamp(0.0, 1.0)
}

struct Columns {
    input: [f32; COLS],
    low: [f32; COLS],
    mid: [f32; COLS],
    high: [f32; COLS],
    wet: f32,
    duck: f32,
}

fn snapshot(meter: &Meter, state: &mut UiState) -> Columns {
    let head = meter.head.load(Ordering::Acquire);
    let mut c = Columns { input: [0.0; COLS], low: [0.0; COLS], mid: [0.0; COLS], high: [0.0; COLS], wet: 0.0, duck: 1.0 };
    for k in 0..COLS {
        let slot = (head + k) % COLS;
        c.input[k] = lvl(meter.input[slot].load(Ordering::Relaxed));
        c.low[k] = lvl(meter.low[slot].load(Ordering::Relaxed));
        c.mid[k] = lvl(meter.mid[slot].load(Ordering::Relaxed));
        c.high[k] = lvl(meter.high[slot].load(Ordering::Relaxed));
    }
    c.wet = c.low[COLS - 1].max(c.mid[COLS - 1]).max(c.high[COLS - 1]);
    c.duck = meter.duck.load(Ordering::Relaxed);

    // Rise fast, fall slowly: once per new column.
    if head != state.last_head {
        for (b, level) in state.bands.iter_mut().enumerate() {
            *level = lvl(meter.bands[b].load(Ordering::Relaxed)).max(*level * 0.88);
        }
        state.last_head = head;
    }
    c
}

pub fn draw(ui: &mut Ui, params: &Arc<VaultParams>, setter: &ParamSetter, meter: &Arc<Meter>, state: &mut UiState) {
    ui.ctx().request_repaint();
    let cols = snapshot(meter, state);

    header(ui, params, setter);
    ui.add_space(6.0);
    mode_row(ui, params, setter);
    ui.add_space(8.0);
    let tunnel_h = ui.available_height() - DECAY_H - KNOB_H - 16.0;
    tunnel(ui, tunnel_h, params, setter, &cols);
    ui.add_space(8.0);
    decay_panel(ui, params, setter, state);
    ui.add_space(8.0);
    knobs(ui, params, setter);
}

fn header(ui: &mut Ui, params: &VaultParams, setter: &ParamSetter) {
    let theme = preset_kit::ui::Theme { fill: PANEL, hover: GRID, ink: rgba(TEXT, 255.0), dim: TEXT_DIM };
    preset_kit::ui::picker_top_right(ui, 24.0, &presets::BANK, params, setter, &theme);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("VAULT").monospace().size(11.0).color(TEXT_DIM));
        ui.label(egui::RichText::new("reverb architecture").monospace().size(9.0).color(GRID));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(preset_kit::ui::WIDTH + 14.0);
            for (text, color) in [
                (format!("{:>5.1}ms", params.predelay.value()), CYAN),
                (format!("{:>4.1}s", params.decay.value()), ORANGE),
            ] {
                ui.label(egui::RichText::new(text).monospace().size(18.0).color(rgba(color, 255.0)));
                ui.add_space(10.0);
            }
        });
    });
}

fn mode_row(ui: &mut Ui, params: &VaultParams, setter: &ParamSetter) {
    let (response, painter) = ui.allocate_painter(Vec2::new(ui.available_width(), 24.0), Sense::hover());
    let row = response.rect;
    let current = params.space.value();
    for (i, (space, label)) in SPACES.iter().enumerate() {
        let rect = Rect::from_min_size(row.min + Vec2::new(i as f32 * 80.0, 0.0), Vec2::new(74.0, 24.0));
        let r = ui.interact(rect, ui.id().with(("space", i)), Sense::click());
        let active = current == *space;
        let fill = if active { rgba(ORANGE, 255.0) } else if r.hovered() { GRID } else { PANEL };
        painter.rect_filled(rect, 4.0, fill);
        let ink = if active { BG } else if r.hovered() { rgba(TEXT, 255.0) } else { TEXT_DIM };
        painter.text(rect.center(), Align2::CENTER_CENTER, label, FontId::monospace(10.0), ink);
        if r.clicked() && !active {
            setter.begin_set_parameter(&params.space);
            setter.set_parameter(&params.space, *space);
            setter.end_set_parameter(&params.space);
        }
    }

    // Freeze, a latching key at the right.
    let rect = Rect::from_min_size(Pos2::new(row.right() - 96.0, row.top()), Vec2::new(96.0, 24.0));
    let r = ui.interact(rect, ui.id().with("freeze"), Sense::click());
    let on = params.freeze.value();
    if on {
        painter.rect_filled(rect.expand(3.0), 6.0, rgba(ICE, 36.0));
    }
    painter.rect_filled(rect, 4.0, if on { rgba(ICE, 255.0) } else if r.hovered() { GRID } else { PANEL });
    let ink = if on { BG } else if r.hovered() { rgba(TEXT, 255.0) } else { TEXT_DIM };
    painter.text(rect.center(), Align2::CENTER_CENTER, if on { "FROZEN" } else { "FREEZE" }, FontId::monospace(10.0), ink);
    if r.clicked() {
        setter.begin_set_parameter(&params.freeze);
        setter.set_parameter(&params.freeze, !on);
        setter.end_set_parameter(&params.freeze);
    }
}

/// The shape of one arch in its own coordinates, relative to the vanishing point, at the front.
struct Vault {
    vp: Pos2,
    half_w: f32,
    floor: f32,
    spring: f32,
    rise: f32,
    /// Radians the far end is twisted by.
    twist: f32,
}

impl Vault {
    /// A point of the arch at depth `age` (0 front, 1 far end), `scale` shrinking it inside itself.
    fn at(&self, x: f32, y: f32, age: f32, scale: f32) -> Pos2 {
        let s = depth_scale(age) * scale;
        let (sin, cos) = (self.twist * age).sin_cos();
        let (x, y) = (x * s, y * s);
        self.vp + Vec2::new(x * cos - y * sin, x * sin + y * cos)
    }

    fn floor_line(&self, age: f32, scale: f32) -> Vec<Pos2> {
        vec![self.at(-self.half_w, self.floor, age, scale), self.at(self.half_w, self.floor, age, scale)]
    }

    fn pillars(&self, age: f32, scale: f32) -> [Vec<Pos2>; 2] {
        [-1.0_f32, 1.0].map(|side| {
            (0..=4)
                .map(|i| {
                    let y = self.floor + (self.spring - self.floor) * i as f32 / 4.0;
                    self.at(side * self.half_w, y, age, scale)
                })
                .collect()
        })
    }

    fn arch(&self, age: f32, scale: f32) -> Vec<Pos2> {
        (0..=16)
            .map(|i| {
                let a = PI + PI * i as f32 / 16.0;
                self.at(a.cos() * self.half_w, self.spring + a.sin() * self.rise, age, scale)
            })
            .collect()
    }
}

/// Perspective: 1 at the front, a fifth at the far end.
fn depth_scale(age: f32) -> f32 {
    1.0 / (1.0 + 4.0 * age)
}

fn glow_line(painter: &Painter, points: Vec<Pos2>, color: [f32; 3], alpha: f32, depth: f32) {
    if alpha < 6.0 {
        return;
    }
    painter.add(Shape::line(points.clone(), Stroke::new(2.0 + 4.0 * depth, rgba(color, alpha * 0.07))));
    painter.add(Shape::line(points, Stroke::new(0.6 + 1.2 * depth, rgba(color, alpha))));
}

fn tunnel(ui: &mut Ui, height: f32, params: &VaultParams, setter: &ParamSetter, c: &Columns) {
    let size = Vec2::new(ui.available_width(), height);
    let (response, painter) = ui.allocate_painter(size, Sense::click_and_drag());
    let rect = response.rect;
    let painter = painter.with_clip_rect(rect);
    painter.rect_filled(rect, 6.0, PANEL);

    // Dragging sideways sets the size.
    let lit = response.hovered() || response.dragged();
    if response.drag_started() {
        setter.begin_set_parameter(&params.size);
    }
    if response.dragged() {
        let fine = ui.input(|i| i.modifiers.shift);
        let v = params.size.unmodulated_normalized_value() + response.drag_delta().x / if fine { 2400.0 } else { 360.0 };
        setter.set_parameter_normalized(&params.size, v.clamp(0.0, 1.0));
    }
    if response.drag_stopped() {
        setter.end_set_parameter(&params.size);
    }
    if response.double_clicked() {
        setter.begin_set_parameter(&params.size);
        setter.set_parameter(&params.size, params.size.default_plain_value());
        setter.end_set_parameter(&params.size);
    }

    let size_norm = params.size.unmodulated_normalized_value();
    let frozen = params.freeze.value();
    let tint = |color: [f32; 3]| if frozen { mix(color, ICE, 0.75) } else { color };
    let vault = Vault {
        vp: Pos2::new(rect.center().x, rect.top() + rect.height() * 0.40),
        half_w: rect.width() * (0.2 + 0.26 * size_norm),
        floor: rect.height() * 0.55,
        spring: -rect.height() * 0.02,
        rise: rect.height() * 0.38,
        twist: (params.shift.value() / 60.0).clamp(-1.0, 1.0) * 1.1,
    };

    // The light at the far end, as bright as the newest wet level.
    for i in 0..10 {
        let r = 70.0 - 6.0 * i as f32;
        painter.circle_filled(vault.vp, r, rgba(tint(ICE), 0.4 + 1.2 * c.wet));
    }

    // Rails along the corners and the crown, curving with the twist.
    for (x, y, is_arch) in [(-1.0, vault.floor, false), (1.0, vault.floor, false), (-1.0, vault.spring, false), (1.0, vault.spring, false), (0.0, vault.spring - vault.rise, true)] {
        let pts: Vec<Pos2> = (0..=12)
            .map(|i| {
                let age = i as f32 / 12.0;
                vault.at(x * vault.half_w, y, age, 1.0)
            })
            .collect();
        painter.add(Shape::line(pts, Stroke::new(1.0_f32, rgba(tint(TEXT), if is_arch { 5.0 } else { 7.0 }))));
    }

    // Arches, far to near so the near ones draw over.
    for k in 0..COLS {
        let age = (COLS - 1 - k) as f32 / COLS as f32;
        let depth = depth_scale(age);
        let fade = 1.0 - age * 0.55;

        if k % 3 == 0 {
            let base = rgba(tint(TEXT), 14.0 * (1.0 - age * 0.7));
            painter.add(Shape::line(vault.arch(age, 1.0), Stroke::new(0.8_f32, base)));
            for p in vault.pillars(age, 1.0) {
                painter.add(Shape::line(p, Stroke::new(0.8_f32, base)));
            }
            painter.add(Shape::line(vault.floor_line(age, 1.0), Stroke::new(0.8_f32, base)));
        }

        // Contrast: the quiet end of the tail should fall dark, not just dim.
        let alpha = |amp: f32| if amp < 0.12 { 0.0 } else { (20.0 + 235.0 * ((amp - 0.12) / 0.88).powf(2.2)) * fade };
        glow_line(&painter, vault.floor_line(age, 1.0), tint(ORANGE), alpha(c.low[k]), depth);
        for p in vault.pillars(age, 1.0) {
            glow_line(&painter, p, tint(LIME), alpha(c.mid[k]), depth);
        }
        glow_line(&painter, vault.arch(age, 1.0), tint(CYAN), alpha(c.high[k]), depth);
        // The input of the same moment, a small white arch inside.
        glow_line(&painter, vault.arch(age, 0.5), tint(TEXT), alpha(c.input[k]) * 0.9, depth);
    }

    // Time ticks on the right-hand floor rail.
    let span_s = COLS as f32 * COL_MS * 0.001;
    for t in 1..=4 {
        let age = t as f32 / span_s;
        if age >= 1.0 {
            break;
        }
        let p = vault.at(vault.half_w, vault.floor, age, 1.0);
        painter.text(p + Vec2::new(8.0, 0.0), Align2::LEFT_CENTER, format!("{t}s"), FontId::monospace(9.0), TEXT_DIM);
    }

    painter.text(rect.left_top() + Vec2::new(12.0, 10.0), Align2::LEFT_TOP, "THE VAULT", FontId::monospace(9.0), TEXT_DIM);
    painter.text(
        rect.left_top() + Vec2::new(12.0, 24.0),
        Align2::LEFT_TOP,
        "drag sideways: size",
        FontId::monospace(9.0),
        if lit { rgba(TEXT, 200.0) } else { rgba(TEXT, 70.0) },
    );
    let status = if frozen {
        Some(("FROZEN".to_owned(), ICE))
    } else if c.duck < 0.97 {
        Some((format!("DUCK {:.0} dB", 20.0 * c.duck.max(0.001).log10()), CYAN))
    } else {
        None
    };
    if let Some((text, color)) = status {
        painter.text(rect.right_top() + Vec2::new(-12.0, 10.0), Align2::RIGHT_TOP, text, FontId::monospace(10.0), rgba(color, 230.0));
    }
}

/// Set a parameter from a drag on `response`; double-click resets.
fn drag_param(response: &Response, setter: &ParamSetter, param: &FloatParam, value: impl Fn(Pos2) -> f32) {
    if response.drag_started() {
        setter.begin_set_parameter(param);
    }
    if response.dragged() {
        if let Some(pos) = response.interact_pointer_pos() {
            setter.set_parameter(param, value(pos));
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

fn decay_panel(ui: &mut Ui, params: &VaultParams, setter: &ParamSetter, state: &UiState) {
    let (response, painter) = ui.allocate_painter(Vec2::new(ui.available_width(), DECAY_H), Sense::hover());
    let rect = response.rect;
    painter.rect_filled(rect, 6.0, PANEL);
    let plot = Rect::from_min_max(Pos2::new(rect.left() + 44.0, rect.top() + 26.0), Pos2::new(rect.right() - 16.0, rect.bottom() - 20.0));

    let fx = |f: f32| plot.left() + (f / 20.0).ln() / 1000.0_f32.ln() * plot.width();
    let ry = |rt: f32| plot.bottom() - ((rt / RT_MIN).ln() / (RT_MAX / RT_MIN).ln()).clamp(0.0, 1.0) * plot.height();
    let inv_rt = |y: f32| RT_MIN * (((plot.bottom() - y) / plot.height()).clamp(0.0, 1.0) * (RT_MAX / RT_MIN).ln()).exp();
    let inv_f = |x: f32| 20.0 * (((x - plot.left()) / plot.width()).clamp(0.0, 1.0) * 1000.0_f32.ln()).exp();

    // Grid.
    for (rt, label) in [(0.1, "0.1s"), (0.3, ""), (1.0, "1s"), (3.0, ""), (10.0, "10s"), (30.0, "")] {
        let y = ry(rt);
        painter.line_segment([Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)], Stroke::new(1.0_f32, rgba(TEXT, 3.0)));
        if !label.is_empty() {
            painter.text(Pos2::new(plot.left() - 6.0, y), Align2::RIGHT_CENTER, label, FontId::monospace(8.0), TEXT_DIM);
        }
    }
    for (f, label) in [(50.0, ""), (100.0, "100"), (200.0, ""), (500.0, ""), (1000.0, "1k"), (2000.0, ""), (5000.0, ""), (10000.0, "10k")] {
        let x = fx(f);
        painter.line_segment([Pos2::new(x, plot.top()), Pos2::new(x, plot.bottom())], Stroke::new(1.0_f32, rgba(TEXT, 3.0)));
        if !label.is_empty() {
            painter.text(Pos2::new(x, rect.bottom() - 4.0), Align2::CENTER_BOTTOM, label, FontId::monospace(8.0), TEXT_DIM);
        }
    }

    // The live spectrum of the wet signal.
    for (b, &level) in state.bands.iter().enumerate() {
        let f = band_freq(b);
        let (x0, x1) = (fx(f / 1.16), fx(f * 1.16));
        let h = level * plot.height() * 0.92;
        if h < 1.0 {
            continue;
        }
        let color = spectrum_color(b as f32 / (BANDS - 1) as f32);
        let bar = Rect::from_min_max(Pos2::new(x0 + 1.0, plot.bottom() - h), Pos2::new(x1 - 1.0, plot.bottom()));
        painter.rect_filled(bar.expand(2.0), 2.0, rgba(color, 5.0));
        painter.rect_filled(bar, 1.0, rgba(color, 30.0 + 40.0 * level));
        painter.line_segment([bar.left_top(), bar.right_top()], Stroke::new(2.0_f32, rgba(color, 230.0)));
    }

    // The curve.
    let (decay, low_mult, high_mult) = (params.decay.value(), params.low_mult.value(), params.high_mult.value());
    let (xo_low, xo_high) = (params.xo_low.value(), params.xo_high.value().max(params.xo_low.value() * 1.2));
    let tau = mean_delay_s(params.space.value(), params.size.value());
    let frozen = params.freeze.value();
    let curve: Vec<Pos2> = (0..=140)
        .map(|i| {
            let f = 20.0 * 1000.0_f32.powf(i as f32 / 140.0);
            let rt = if frozen { RT_MAX } else { decay_at(f, tau, decay, low_mult, high_mult, xo_low, xo_high) };
            Pos2::new(fx(f), ry(rt))
        })
        .collect();
    let ink = if frozen { ICE } else { TEXT };
    let mut mesh = Mesh::default();
    for p in &curve {
        mesh.colored_vertex(*p, rgba(ink, 10.0));
        mesh.colored_vertex(Pos2::new(p.x, plot.bottom()), rgba(ink, 0.0));
    }
    for k in 0..(curve.len() as u32 - 1) {
        mesh.add_triangle(2 * k, 2 * k + 1, 2 * k + 2);
        mesh.add_triangle(2 * k + 1, 2 * k + 2, 2 * k + 3);
    }
    painter.add(Shape::mesh(mesh));
    painter.add(Shape::line(curve.clone(), Stroke::new(6.0_f32, rgba(ink, 8.0))));
    painter.add(Shape::line(curve, Stroke::new(2.0_f32, rgba(ink, 240.0))));

    painter.text(rect.left_top() + Vec2::new(12.0, 8.0), Align2::LEFT_TOP, "DECAY BY FREQUENCY", FontId::monospace(9.0), TEXT_DIM);

    // Crossover strips come first so the handles sit on top of them.
    let mut strips = Vec::new();
    for (i, (param, hz)) in [(&params.xo_low, xo_low), (&params.xo_high, xo_high)].into_iter().enumerate() {
        let x = fx(hz);
        let r = ui_interact(ui, Rect::from_x_y_ranges(x - 8.0..=x + 8.0, rect.top()..=plot.bottom()), ("xo", i));
        drag_param(&r, setter, param, |p| inv_f(p.x));
        strips.push((x, hz, r));
    }
    for (x, hz, r) in strips {
        let lit = r.hovered() || r.dragged();
        painter.line_segment([Pos2::new(x, plot.top() + 6.0), Pos2::new(x, plot.bottom())], Stroke::new(1.0_f32, rgba(LIME, if lit { 120.0 } else { 40.0 })));
        let tri = vec![Pos2::new(x - 6.0, plot.top() - 4.0), Pos2::new(x + 6.0, plot.top() - 4.0), Pos2::new(x, plot.top() + 7.0)];
        painter.add(Shape::convex_polygon(tri, rgba(LIME, if lit { 255.0 } else { 200.0 }), Stroke::NONE));
        let text = if hz >= 1000.0 { format!("{:.1}k", hz / 1000.0) } else { format!("{hz:.0}") };
        painter.text(Pos2::new(x + 9.0, plot.top() - 5.0), Align2::LEFT_BOTTOM, text, FontId::monospace(9.0), rgba(LIME, 200.0));
    }

    // Handles: decay (mids), low multiplier, high multiplier. Each sits on the curve at its own
    // frequency, and a drag moves the band's nominal time by the same ratio as the pointer moves
    // the curve, so the handle stays under the pointer even where the crossovers blur the bands.
    let handles: [(f32, f32, [f32; 3], &FloatParam); 3] = [
        ((xo_low * xo_high).sqrt(), decay, ink, &params.decay),
        ((20.0 * xo_low).sqrt(), decay * low_mult, ORANGE, &params.low_mult),
        ((xo_high * 20000.0).sqrt(), decay * high_mult, CYAN, &params.high_mult),
    ];
    for (i, (f, nominal, color, param)) in handles.iter().enumerate() {
        let on_curve = if frozen { RT_MAX } else { decay_at(*f, tau, decay, low_mult, high_mult, xo_low, xo_high) };
        let ratio = nominal / on_curve;
        let center = Pos2::new(fx(*f), ry(on_curve.min(RT_MAX)));
        let r = ui_interact(ui, Rect::from_center_size(center, Vec2::splat(26.0)), ("decay_handle", i));
        drag_param(&r, setter, param, |p| {
            let target = inv_rt(p.y) * ratio;
            match i {
                0 => target.clamp(MIN_DECAY_S, MAX_DECAY_S),
                _ => (target / decay).clamp(0.1, 2.0),
            }
        });
        let lit = r.hovered() || r.dragged();
        let radius = if lit { 7.5 } else { 6.0 };
        painter.circle_filled(center, radius + 6.0, rgba(*color, if lit { 16.0 } else { 7.0 }));
        painter.circle_filled(center, radius, rgba(*color, 255.0));
        painter.circle_stroke(center, radius, Stroke::new(1.5_f32, BG));
        let text = match i {
            0 => format!("{decay:.1}s"),
            1 => format!("x{low_mult:.2}"),
            _ => format!("x{high_mult:.2}"),
        };
        painter.text(center + Vec2::new(0.0, -12.0), Align2::CENTER_BOTTOM, text, FontId::monospace(9.0), rgba(*color, 230.0));
    }
}

/// `ui.interact` for a rect that is not laid out, with an id derived from a key.
fn ui_interact(ui: &Ui, rect: Rect, key: (&str, usize)) -> Response {
    ui.interact(rect, ui.id().with(key), Sense::click_and_drag())
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

fn knobs(ui: &mut Ui, params: &VaultParams, setter: &ParamSetter) {
    let (response, painter) = ui.allocate_painter(Vec2::new(ui.available_width(), KNOB_H), Sense::hover());
    let row = response.rect;
    let cells: [(&FloatParam, &str, [f32; 3]); 12] = [
        (&params.size, "SIZE", ORANGE),
        (&params.predelay, "PRE-DLY", ORANGE),
        (&params.diffusion, "DIFFUSE", ORANGE),
        (&params.mod_depth, "MOD", LIME),
        (&params.mod_rate, "RATE", LIME),
        (&params.shift, "SHIFT", LIME),
        (&params.early, "EARLY", CYAN),
        (&params.width, "WIDTH", CYAN),
        (&params.duck, "DUCK", CYAN),
        (&params.low_cut, "LO CUT", TEXT),
        (&params.high_cut, "HI CUT", TEXT),
        (&params.mix, "MIX", TEXT),
    ];
    let width = row.width() / cells.len() as f32;
    for (i, (param, label, color)) in cells.iter().enumerate() {
        let cell = Rect::from_min_size(Pos2::new(row.left() + i as f32 * width, row.top()), Vec2::new(width, KNOB_H));
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
    let center = Pos2::new(cell.center().x, cell.top() + 26.0);
    let radius = 19.0;
    let (start, end) = (PI * 0.75, PI * 2.25);
    let lit = r.hovered() || r.dragged();
    let main = rgba(color, 255.0);

    painter.circle_filled(center, radius - 4.0, if lit { PANEL } else { Color32::from_rgb(0x0c, 0x0f, 0x19) });
    painter.add(Shape::line(arc(center, radius, start, end), Stroke::new(2.5_f32, GRID)));
    // The shift is bipolar and lights from the centre.
    let from: f32 = if label == "SHIFT" { 0.5 } else { 0.0 };
    let (lo, hi) = (from.min(norm), from.max(norm));
    if hi - lo > 0.002 {
        let (a, b) = (start + (end - start) * lo, start + (end - start) * hi);
        painter.add(Shape::line(arc(center, radius, a, b), Stroke::new(7.0_f32, rgba(color, 30.0))));
        painter.add(Shape::line(arc(center, radius, a, b), Stroke::new(2.5_f32, main)));
    }
    let angle = start + (end - start) * norm;
    let dir = Vec2::new(angle.cos(), angle.sin());
    painter.line_segment([center + dir * (radius - 11.0), center + dir * (radius - 5.0)], Stroke::new(2.0_f32, rgba(TEXT, 255.0)));

    painter.text(Pos2::new(cell.center().x, cell.top() + 55.0), Align2::CENTER_CENTER, label, FontId::monospace(8.5), TEXT_DIM);
    painter.text(Pos2::new(cell.center().x, cell.top() + 69.0), Align2::CENTER_CENTER, param.to_string(), FontId::monospace(10.0), main);
}
