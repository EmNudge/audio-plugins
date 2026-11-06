//! The editor: a scrolling sibilance waterfall with a snake that hunts the "s".
//!
//! The waterfall is time across and frequency up. Green is what the plugin lets through; red-hot
//! is what it took out, so a de-essed "s" shows as a bright bite in the signal. The snake is the
//! tracked centre of the detector: it stays thin and green while idle and swells and flushes red
//! when it is reducing. Below it, two strips share the same time axis: the detector against its
//! threshold, and the reduction hanging from the ceiling.
//!
//! Drag in the waterfall to move the centre, drag the ribbon edges for width, drag the detector
//! strip for the threshold and the reduction strip for the range.

use nih_plug::prelude::*;
use nih_plug_egui::egui::{
    self, Align2, Color32, CursorIcon, FontId, Mesh, Painter, Pos2, Rect, RichText, Sense, Shape, Stroke, Ui, Vec2,
};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;

use crate::dsp::{Telemetry, BANDS, F_MAX, F_MIN};
use crate::SerpentParams;

const BG: Color32 = Color32::from_rgb(0x07, 0x0a, 0x0a);
const PANEL: Color32 = Color32::from_rgb(0x11, 0x16, 0x16);
const GRID: Color32 = Color32::from_rgb(0x22, 0x2b, 0x2a);
const TEXT_DIM: Color32 = Color32::from_rgb(0xa4, 0xbb, 0xb5);
const LINE: Color32 = Color32::from_rgb(0xe8, 0xf4, 0xee);
const LIME: [u8; 3] = [182, 255, 59];
const CORAL: [u8; 3] = [255, 74, 61];

/// Waterfall colour ramps: what is kept, and what was removed.
const KEPT: [(f32, [f32; 3]); 5] = [
    (0.0, [6.0, 10.0, 11.0]),
    (0.3, [8.0, 52.0, 50.0]),
    (0.6, [22.0, 140.0, 104.0]),
    (0.85, [150.0, 232.0, 84.0]),
    (1.0, [236.0, 255.0, 200.0]),
];
const REMOVED: [(f32, [f32; 3]); 3] = [(0.0, [110.0, 12.0, 36.0]), (0.5, [255.0, 72.0, 52.0]), (1.0, [255.0, 214.0, 110.0])];

/// Band level range shown by the waterfall, in dB.
const LEVEL_FLOOR: f32 = -90.0;
const LEVEL_SPAN: f32 = 66.0;
/// A cut of this many dB is drawn fully hot.
const CUT_FULL_DB: f32 = 10.0;

const COLS: usize = 240;
const STEP: f32 = 1.0 / 60.0;
/// Room on the right of the waterfall for the snake's head and tongue.
const HEAD_PAD: f32 = 28.0;

const DETECT_LO: f32 = -60.0;
const DETECT_HI: f32 = 6.0;
const DETECT_H: f32 = 62.0;
const REDUCTION_H: f32 = 44.0;
const REDUCTION_SCALE_DB: f32 = 24.0;

pub struct UiState {
    wet_db: Vec<f32>,
    cut_db: Vec<f32>,
    center: Vec<f32>,
    gr: Vec<f32>,
    detect: Vec<f32>,
    head: usize,
    accum: f32,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            wet_db: vec![-120.0; COLS * BANDS],
            cut_db: vec![0.0; COLS * BANDS],
            center: vec![7000.0; COLS],
            gr: vec![0.0; COLS],
            detect: vec![-120.0; COLS],
            head: 0,
            accum: 0.0,
        }
    }
}

impl UiState {
    fn push(&mut self, telemetry: &Telemetry) {
        let h = self.head;
        for b in 0..BANDS {
            let dry = to_db(telemetry.dry[b].load(Relaxed));
            let wet = to_db(telemetry.wet[b].load(Relaxed));
            self.wet_db[h * BANDS + b] = wet;
            self.cut_db[h * BANDS + b] = if dry > -85.0 { (dry - wet).max(0.0) } else { 0.0 };
        }
        self.center[h] = telemetry.center_hz.load(Relaxed);
        self.gr[h] = telemetry.gr_db.load(Relaxed);
        self.detect[h] = telemetry.detect_db.load(Relaxed);
        self.head = (h + 1) % COLS;
    }

    /// Ring index of the `c`th column, oldest first.
    fn index(&self, c: usize) -> usize {
        (self.head + c) % COLS
    }

    fn newest(&self) -> usize {
        (self.head + COLS - 1) % COLS
    }
}

pub fn background() -> Color32 {
    BG
}

fn to_db(lin: f32) -> f32 {
    20.0 * lin.max(1e-6).log10()
}

fn rgba(c: [u8; 3], a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(c[0], c[1], c[2], a)
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
}

fn ramp(stops: &[(f32, [f32; 3])], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    for pair in stops.windows(2) {
        let ((t0, c0), (t1, c1)) = (pair[0], pair[1]);
        if t <= t1 {
            return mix(c0, c1, (t - t0) / (t1 - t0));
        }
    }
    stops[stops.len() - 1].1
}

fn cell_color(wet_db: f32, cut_db: f32) -> Color32 {
    let base = ramp(&KEPT, (wet_db - LEVEL_FLOOR) / LEVEL_SPAN);
    let bite = (cut_db / CUT_FULL_DB).clamp(0.0, 1.0);
    let c = if bite > 0.02 { mix(base, ramp(&REMOVED, bite), (bite * 1.4).min(0.92)) } else { base };
    Color32::from_rgb(c[0] as u8, c[1] as u8, c[2] as u8)
}

/// The snake goes from lime to coral as it works harder.
fn snake_color(gr_db: f32, alpha: u8) -> Color32 {
    let c = mix(LIME.map(f32::from), CORAL.map(f32::from), (gr_db / 10.0).clamp(0.0, 1.0));
    Color32::from_rgba_unmultiplied(c[0] as u8, c[1] as u8, c[2] as u8, alpha)
}

fn data_rect(rect: Rect) -> Rect {
    Rect::from_min_max(rect.min, Pos2::new(rect.right() - HEAD_PAD, rect.bottom()))
}

/// Triangle strip along a path, with a width and colour at each point.
fn tube(painter: &Painter, pts: &[(Pos2, f32, Color32)]) {
    let mut mesh = Mesh::default();
    for (i, (p, width, color)) in pts.iter().enumerate() {
        let tangent = pts[(i + 1).min(pts.len() - 1)].0 - pts[i.saturating_sub(1)].0;
        let normal = Vec2::new(-tangent.y, tangent.x) / tangent.length().max(1e-3);
        mesh.colored_vertex(*p + normal * (width * 0.5), *color);
        mesh.colored_vertex(*p - normal * (width * 0.5), *color);
    }
    for i in 0..pts.len().saturating_sub(1) {
        let k = (i * 2) as u32;
        mesh.add_triangle(k, k + 1, k + 2);
        mesh.add_triangle(k + 1, k + 2, k + 3);
    }
    painter.add(Shape::mesh(mesh));
}

/// Drag, click or double-click a float parameter. `to_normalized` maps the pointer to 0..1.
fn drive(setter: &ParamSetter, param: &FloatParam, r: &egui::Response, to_normalized: impl Fn(Pos2) -> f32) {
    if r.drag_started() {
        setter.begin_set_parameter(param);
    }
    if r.dragged() || r.drag_started() {
        if let Some(pos) = r.interact_pointer_pos() {
            setter.set_parameter_normalized(param, to_normalized(pos).clamp(0.0, 1.0));
        }
    }
    if r.drag_stopped() {
        setter.end_set_parameter(param);
    }
    if r.double_clicked() {
        setter.begin_set_parameter(param);
        setter.set_parameter(param, param.default_plain_value());
        setter.end_set_parameter(param);
    } else if r.clicked() {
        if let Some(pos) = r.interact_pointer_pos() {
            setter.begin_set_parameter(param);
            setter.set_parameter_normalized(param, to_normalized(pos).clamp(0.0, 1.0));
            setter.end_set_parameter(param);
        }
    }
    if r.hovered() || r.dragged() {
        r.ctx.set_cursor_icon(CursorIcon::ResizeVertical);
    }
}

pub fn draw(ui: &mut Ui, params: &Arc<SerpentParams>, setter: &ParamSetter, telemetry: &Arc<Telemetry>, state: &mut UiState) {
    ui.ctx().request_repaint();

    // One column per 1/60 s, whatever the real frame rate is.
    state.accum += ui.input(|i| i.unstable_dt).min(0.1);
    let mut pushed = 0;
    while state.accum >= STEP && pushed < 6 {
        state.push(telemetry);
        state.accum -= STEP;
        pushed += 1;
    }
    if pushed == 6 {
        state.accum = 0.0;
    }

    let (title, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.0), Sense::hover());
    let painter = ui.painter().clone();
    painter.text(title.left_center(), Align2::LEFT_CENTER, "SERPENT", FontId::monospace(11.0), TEXT_DIM);
    // Picker sits just right of the title (about 206 px wide); the latency readout is on the right.
    let theme = preset_kit::ui::Theme { fill: PANEL, hover: GRID, ink: LINE, dim: TEXT_DIM };
    preset_kit::ui::picker(ui, Pos2::new(title.left() + 80.0, title.top()), title.height(), &crate::presets::BANK, &**params, setter, &theme);
    let latency_ms = telemetry.latency_samples.load(Relaxed) / telemetry.sample_rate.load(Relaxed).max(1.0) * 1000.0;
    painter.text(
        title.right_center(),
        Align2::RIGHT_CENTER,
        format!("LATENCY {latency_ms:.1} ms"),
        FontId::monospace(10.0),
        TEXT_DIM,
    );
    ui.add_space(6.0);

    let below = DETECT_H + REDUCTION_H + 4.0 + 12.0 + 88.0 + 12.0 + 24.0 + 36.0;
    let plot_height = (ui.available_height() - below).max(120.0);
    waterfall(ui, plot_height, params, setter, state);
    ui.add_space(6.0);
    strips(ui, params, setter, state);

    ui.add_space(10.0);
    ui.columns(2, |cols| {
        bar(&mut cols[0], setter, &params.freq, "FREQUENCY");
        cols[0].add_space(5.0);
        bar(&mut cols[0], setter, &params.width, "WIDTH");
        cols[0].add_space(5.0);
        bar(&mut cols[0], setter, &params.follow, "FOLLOW");
        bar(&mut cols[1], setter, &params.attack, "ATTACK");
        cols[1].add_space(5.0);
        bar(&mut cols[1], setter, &params.release, "RELEASE");
        cols[1].add_space(5.0);
        bar(&mut cols[1], setter, &params.lookahead, "LOOKAHEAD");
    });
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        segmented(ui, setter, &params.shape, "SHAPE");
        ui.add_space(14.0);
        segmented(ui, setter, &params.detect, "DETECT");
        ui.add_space(14.0);
        segmented(ui, setter, &params.listen, "LISTEN");
    });
}

fn waterfall(ui: &mut Ui, height: f32, params: &Arc<SerpentParams>, setter: &ParamSetter, state: &UiState) {
    let (response, painter) = ui.allocate_painter(Vec2::new(ui.available_width(), height), Sense::hover());
    let rect = response.rect;
    let data = data_rect(rect);
    let span = (F_MAX / F_MIN).ln();
    let y_of = |f: f32| rect.bottom() - (f / F_MIN).ln() / span * rect.height();
    let f_of = |y: f32| F_MIN * (span * ((rect.bottom() - y) / rect.height()).clamp(0.0, 1.0)).exp();
    let center = params.freq.value();
    let width = params.width.value();
    let half = 2f32.powf(width / 2.0);
    let (top_y, bottom_y) = (y_of(center * half), y_of(center / half));

    // Interaction first, so painting can react.
    let plot = ui.interact(data, ui.id().with("plot"), Sense::click_and_drag());
    drive(setter, &params.freq, &plot, |p| params.freq.preview_normalized(f_of(p.y)));
    let mut edge_active = false;
    for (name, y) in [("edge-top", top_y), ("edge-bottom", bottom_y)] {
        let edge = Rect::from_center_size(Pos2::new(data.center().x, y), Vec2::new(data.width(), 10.0));
        let r = ui.interact(edge, ui.id().with(name), Sense::drag());
        edge_active |= r.hovered() || r.dragged();
        drive(setter, &params.width, &r, |p| {
            let octaves = 2.0 * (f_of(p.y) / center).log2().abs();
            params.width.preview_normalized(octaves.clamp(0.3, 3.0))
        });
    }

    painter.rect_filled(rect, 0.0, Color32::from_rgb(6, 10, 11));

    // The waterfall itself: one smooth-shaded vertex per band per column.
    let mut mesh = Mesh::default();
    mesh.vertices.reserve(COLS * BANDS);
    for c in 0..COLS {
        let idx = state.index(c);
        let x = data.left() + c as f32 / (COLS - 1) as f32 * data.width();
        for b in 0..BANDS {
            let y = rect.bottom() - (b as f32 + 0.5) / BANDS as f32 * rect.height();
            mesh.colored_vertex(Pos2::new(x, y), cell_color(state.wet_db[idx * BANDS + b], state.cut_db[idx * BANDS + b]));
        }
    }
    for c in 0..COLS - 1 {
        for b in 0..BANDS - 1 {
            let i0 = (c * BANDS + b) as u32;
            let (i1, i2, i3) = (i0 + 1, i0 + BANDS as u32, i0 + BANDS as u32 + 1);
            mesh.add_triangle(i0, i1, i2);
            mesh.add_triangle(i1, i2, i3);
        }
    }
    painter.add(Shape::mesh(mesh));

    // Frequency scale.
    for (f, label) in [(2000.0, "2k"), (3000.0, "3k"), (5000.0, "5k"), (8000.0, "8k"), (12000.0, "12k"), (16000.0, "16k")] {
        let y = y_of(f);
        painter.line_segment([Pos2::new(data.left(), y), Pos2::new(data.right(), y)], Stroke::new(1.0_f32, rgba([255, 255, 255], 12)));
        painter.text(Pos2::new(data.right() + 4.0, y), Align2::LEFT_CENTER, label, FontId::monospace(9.0), TEXT_DIM);
    }

    // The detector band.
    let edge_alpha = if edge_active { 170 } else { 70 };
    painter.rect_filled(
        Rect::from_min_max(Pos2::new(data.left(), top_y), Pos2::new(data.right(), bottom_y)),
        0.0,
        rgba(LIME, 9),
    );
    for y in [top_y, bottom_y] {
        painter.line_segment([Pos2::new(data.left(), y), Pos2::new(data.right(), y)], Stroke::new(1.0_f32, rgba(LIME, edge_alpha)));
    }
    painter.text(
        Pos2::new(data.left() + 8.0, top_y - 3.0),
        Align2::LEFT_BOTTOM,
        format!("{}  {:.1} oct", params.freq, width),
        FontId::monospace(10.0),
        rgba(LIME, 200),
    );

    snake(ui, &painter, data, &y_of, state);

    // Legend and listen tag.
    let legend_y = rect.bottom() - 12.0;
    let mut x = data.left() + 8.0;
    for (label, color) in [("KEPT", Color32::from_rgb(22, 140, 104)), ("REMOVED", rgba(CORAL, 255))] {
        painter.rect_filled(Rect::from_center_size(Pos2::new(x + 3.0, legend_y), Vec2::splat(7.0)), 1.5, color);
        let galley = painter.text(Pos2::new(x + 11.0, legend_y), Align2::LEFT_CENTER, label, FontId::monospace(9.0), TEXT_DIM);
        x += 11.0 + galley.width() + 12.0;
    }
    let listen = params.listen.value();
    if listen != crate::dsp::Listen::Off {
        painter.text(
            Pos2::new(data.right() - 8.0, rect.top() + 8.0),
            Align2::RIGHT_TOP,
            format!("LISTENING: {}", params.listen.to_string().to_uppercase()),
            FontId::monospace(10.0),
            rgba(CORAL, 255),
        );
    }
    painter.rect_stroke(rect, 0.0, Stroke::new(1.0_f32, GRID), egui::StrokeKind::Inside);
}

/// The tracked centre over time as a tapered tube whose girth is the reduction, with a head.
fn snake(ui: &Ui, painter: &Painter, data: Rect, y_of: &impl Fn(f32) -> f32, state: &UiState) {
    let point = |c: usize| {
        let idx = state.index(c);
        let taper = 0.25 + 0.75 * (c as f32 / (COLS - 1) as f32).powf(0.7);
        let width = (2.0 + state.gr[idx] * 0.9).min(20.0) * taper;
        let pos = Pos2::new(data.left() + c as f32 / (COLS - 1) as f32 * data.width(), y_of(state.center[idx]));
        (pos, width, state.gr[idx])
    };
    let pts: Vec<(Pos2, f32, f32)> = (0..COLS).map(point).collect();
    tube(painter, &pts.iter().map(|&(p, w, gr)| (p, w * 2.8, snake_color(gr, 22))).collect::<Vec<_>>());
    tube(painter, &pts.iter().map(|&(p, w, gr)| (p, w, snake_color(gr, 235))).collect::<Vec<_>>());

    let (head, width, gr) = pts[COLS - 1];
    let r = width * 0.5 + 2.5;
    painter.circle_filled(head, r * 1.9, snake_color(gr, 28));
    painter.circle_filled(head, r, snake_color(gr, 255));
    for side in [-1.0, 1.0] {
        painter.circle_filled(head + Vec2::new(r * 0.25, side * r * 0.45), 1.4, BG);
    }
    if gr > 0.3 {
        let flick = 6.0 + 3.0 * (ui.input(|i| i.time) as f32 * 28.0).sin();
        let base = head + Vec2::new(r, 0.0);
        let tip = base + Vec2::new(flick, 0.0);
        let stroke = Stroke::new(1.2_f32, snake_color(gr, 255));
        painter.line_segment([base, tip], stroke);
        for angle in [-0.5f32, 0.5] {
            painter.line_segment([tip, tip + Vec2::new(angle.cos(), angle.sin()) * 4.5], stroke);
        }
    }
}

/// Detector against threshold, and the reduction hanging from the top, on the waterfall's time axis.
fn strips(ui: &mut Ui, params: &Arc<SerpentParams>, setter: &ParamSetter, state: &UiState) {
    let width = ui.available_width();
    let (detect_response, detect_painter) = ui.allocate_painter(Vec2::new(width, DETECT_H), Sense::click_and_drag());
    ui.add_space(4.0);
    let (reduction_response, reduction_painter) = ui.allocate_painter(Vec2::new(width, REDUCTION_H), Sense::click_and_drag());
    let cw = data_rect(detect_response.rect).width() / COLS as f32;
    let newest = state.newest();

    // Detector.
    let rect = detect_response.rect;
    let data = data_rect(rect);
    let y_of = |db: f32| rect.bottom() - (db.clamp(DETECT_LO, DETECT_HI) - DETECT_LO) / (DETECT_HI - DETECT_LO) * rect.height();
    let db_of = |y: f32| DETECT_LO + (rect.bottom() - y) / rect.height() * (DETECT_HI - DETECT_LO);
    drive(setter, &params.threshold, &detect_response, |p| params.threshold.preview_normalized(db_of(p.y)));
    let p = &detect_painter;
    p.rect_filled(rect, 3.0, PANEL);
    let threshold = params.threshold.value();
    let thr_y = y_of(threshold);
    for c in 0..COLS {
        let x = data.left() + c as f32 * cw;
        let y = y_of(state.detect[state.index(c)]);
        let col = |top: f32, bottom: f32| Rect::from_min_max(Pos2::new(x, top), Pos2::new(x + cw + 0.5, bottom));
        p.rect_filled(col(y.max(thr_y), rect.bottom()), 0.0, rgba(LIME, 70));
        if y < thr_y {
            p.rect_filled(col(y, thr_y), 0.0, rgba(CORAL, 200));
        }
    }
    let hot = detect_response.hovered() || detect_response.dragged();
    p.line_segment([Pos2::new(rect.left(), thr_y), Pos2::new(rect.right(), thr_y)], Stroke::new(if hot { 2.0_f32 } else { 1.0_f32 }, LINE));
    let mode = if params.detect.value() == crate::dsp::Detect::Relative { "vs. mix" } else { "level" };
    p.text(rect.left_top() + Vec2::new(8.0, 5.0), Align2::LEFT_TOP, format!("SIBILANCE {mode}"), FontId::monospace(9.0), TEXT_DIM);
    p.text(
        Pos2::new(rect.right() - 8.0, (thr_y - 3.0).max(rect.top() + 10.0)),
        Align2::RIGHT_BOTTOM,
        format!("THRESHOLD {}", params.threshold),
        FontId::monospace(10.0),
        LINE,
    );
    p.rect_stroke(rect, 3.0, Stroke::new(1.0_f32, GRID), egui::StrokeKind::Inside);

    // Reduction.
    let rect = reduction_response.rect;
    let data = data_rect(rect);
    let y_of = |db: f32| rect.top() + db.clamp(0.0, REDUCTION_SCALE_DB) / REDUCTION_SCALE_DB * rect.height();
    drive(setter, &params.range, &reduction_response, |p| params.range.preview_normalized((p.y - rect.top()) / rect.height() * REDUCTION_SCALE_DB));
    let p = &reduction_painter;
    p.rect_filled(rect, 3.0, PANEL);
    for c in 0..COLS {
        let x = data.left() + c as f32 * cw;
        let gr = state.gr[state.index(c)];
        if gr > 0.02 {
            let bar = Rect::from_min_max(Pos2::new(x, rect.top()), Pos2::new(x + cw + 0.5, y_of(gr)));
            p.rect_filled(bar, 0.0, snake_color(gr, 210));
        }
    }
    let range = params.range.value();
    let hot = reduction_response.hovered() || reduction_response.dragged();
    let range_y = y_of(range);
    p.line_segment([Pos2::new(rect.left(), range_y), Pos2::new(rect.right(), range_y)], Stroke::new(if hot { 2.0_f32 } else { 1.0_f32 }, rgba(CORAL, 150)));
    p.text(rect.left_top() + Vec2::new(8.0, 5.0), Align2::LEFT_TOP, "REDUCTION", FontId::monospace(9.0), TEXT_DIM);
    p.text(
        Pos2::new(rect.right() - 8.0, (range_y + 3.0).min(rect.bottom() - 10.0)),
        Align2::RIGHT_TOP,
        format!("RANGE {}", params.range),
        FontId::monospace(10.0),
        rgba(CORAL, 255),
    );
    p.text(
        Pos2::new(data.right() - 6.0, rect.top() + 5.0),
        Align2::RIGHT_TOP,
        format!("-{:.1} dB", state.gr[newest]),
        FontId::monospace(10.0),
        snake_color(state.gr[newest], 255),
    );
    p.rect_stroke(rect, 3.0, Stroke::new(1.0_f32, GRID), egui::StrokeKind::Inside);
}

/// A horizontal bar fader for a float parameter: drag to set, double-click to reset.
fn bar(ui: &mut Ui, setter: &ParamSetter, param: &FloatParam, label: &str) {
    let (response, painter) = ui.allocate_painter(Vec2::new(ui.available_width(), 26.0), Sense::click_and_drag());
    let rect = response.rect;
    let norm = param.unmodulated_normalized_value();

    painter.rect_filled(rect, 4.0, PANEL);
    let fill = Rect::from_min_size(rect.min, Vec2::new(rect.width() * norm, rect.height()));
    painter.rect_filled(fill, 4.0, rgba(LIME, 34));
    painter.rect_filled(Rect::from_min_size(Pos2::new(fill.right() - 3.0, rect.top()), Vec2::new(3.0, rect.height())), 1.5, rgba(LIME, 255));
    painter.text(Pos2::new(rect.left() + 8.0, rect.center().y), Align2::LEFT_CENTER, label, FontId::monospace(10.5), LINE);
    painter.text(Pos2::new(rect.right() - 8.0, rect.center().y), Align2::RIGHT_CENTER, param.to_string(), FontId::monospace(11.0), rgba(LIME, 255));
    painter.rect_stroke(rect, 4.0, Stroke::new(1.0_f32, GRID), egui::StrokeKind::Inside);

    drive(setter, param, &response, |p| (p.x - rect.left()) / rect.width());
    // `drive` shows a vertical-resize cursor; a bar moves sideways.
    if response.hovered() || response.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
    }
}

/// A row of buttons, one per variant, with the current one lit.
fn segmented<E: Enum + PartialEq + 'static>(ui: &mut Ui, setter: &ParamSetter, param: &EnumParam<E>, label: &str) {
    ui.label(RichText::new(label).monospace().size(10.0).color(TEXT_DIM));
    let current = param.value().to_index();
    for (i, name) in E::variants().iter().enumerate() {
        let selected = i == current;
        let button = egui::Button::new(RichText::new(name.to_string()).monospace().size(11.0).color(if selected { BG } else { LINE }))
            .fill(if selected { rgba(LIME, 255) } else { PANEL })
            .stroke(Stroke::new(1.0_f32, GRID))
            .min_size(Vec2::new(44.0, 22.0));
        if ui.add(button).clicked() && !selected {
            setter.begin_set_parameter(param);
            setter.set_parameter(param, E::from_index(i));
            setter.end_set_parameter(param);
        }
    }
}
