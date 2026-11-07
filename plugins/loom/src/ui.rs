//! The editor: a loom. The orange cloth is the spectrogram, a warp of vertical threads that
//! scrolls left as time passes. Each playhead is a heddle you drag across it: the hollow slat marks
//! the band it reads, the teal slat is where it writes, and the threads under a head light up
//! teal. What the heads weave comes out on the right as the finished cloth, the live output
//! spectrum.
//!
//! On the graph: drag the teal slat for delay (across) and pitch offset (up and down, snapping to
//! semitones, shift for fine); drag the hollow slat up and down to move the band; drag the end of
//! either slat for the band width. Double-click a slat to reset what it moves. Click a head, or its
//! tab, to pick the head the knobs edit; the lamp on the tab switches it on and off.
//! Knobs: drag vertically (shift for fine), double-click to reset.

use nih_plug::prelude::*;
use nih_plug_egui::egui::{
    self, Align2, Color32, FontId, Mesh, Painter, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, Vec2,
};
use std::f32::consts::PI;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::dsp::{
    window_gain, Meter, BANDS, COLS, F_MAX, F_MIN, FULL_WIDTH, HEADS, MAX_DELAY_MS, MAX_SHIFT, MIN_WIDTH, OFF_DB,
};
use crate::{presets, HeadParams, LoomParams};

const BG: Color32 = Color32::from_rgb(0x0c, 0x0a, 0x1e);
const PANEL: Color32 = Color32::from_rgb(0x18, 0x15, 0x36);
const GRID: Color32 = Color32::from_rgb(0x2b, 0x26, 0x58);
const TEXT_DIM: Color32 = Color32::from_rgb(0x87, 0x80, 0xb8);
const TEXT: [f32; 3] = [236.0, 232.0, 255.0];
const GLOW_TOP: [f32; 3] = [24.0, 19.0, 54.0];
const GLOW_BOTTOM: [f32; 3] = [12.0, 10.0, 30.0];
const THREAD: [f32; 3] = [86.0, 62.0, 124.0];
const ORANGE: [f32; 3] = [255.0, 122.0, 26.0];
const HOT: [f32; 3] = [255.0, 214.0, 120.0];
const TEAL: [f32; 3] = [24.0, 230.0, 200.0];
const PINK: [f32; 3] = [255.0, 79.0, 154.0];

const KNOB_ROW: f32 = 100.0;
const TABS_ROW: f32 = 26.0;
const AXIS_W: f32 = 34.0;
const STRIP_W: f32 = 78.0;
const SLAT_HALF: f32 = 4.0;
const SLAT_GAP: f32 = 12.0;
const MAX_S: f32 = MAX_DELAY_MS / 1000.0;

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    /// The teal slat's body: delay and pitch offset.
    Out,
    /// The hollow slat's body: the band's centre.
    Src,
    /// An end of a slat: the band's width.
    Edge(bool),
}

struct Drag {
    head: usize,
    kind: Kind,
    delay: f32,
    shift: f32,
    centre_oct: f32,
}

pub struct UiState {
    selected: usize,
    drag: Option<Drag>,
    out_db: [f32; BANDS],
    act: [f32; HEADS],
    /// What is being edited and when it last changed, for the big numerals.
    readout: Vec<(String, String)>,
    readout_time: f64,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            selected: 0,
            drag: None,
            out_db: [-120.0; BANDS],
            act: [0.0; HEADS],
            readout: Vec::new(),
            readout_time: -10.0,
        }
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

/// Rises fast, falls slowly, so peaks stay readable.
fn follow(state: &mut UiState, meter: &Meter) {
    for b in 0..BANDS {
        let db = meter.out[b].load(Ordering::Relaxed);
        state.out_db[b] = if db > state.out_db[b] { db } else { (state.out_db[b] - 1.2).max(db) };
    }
    for h in 0..HEADS {
        // 0 at -70 dB, 1 at -12 dB.
        let a = ((meter.head_level[h].load(Ordering::Relaxed) + 70.0) / 58.0).clamp(0.0, 1.0);
        state.act[h] = if a > state.act[h] { state.act[h] + (a - state.act[h]) * 0.6 } else { (state.act[h] - 0.025).max(a) };
    }
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

/// The cloth's coordinates: delay across (now on the right), log frequency up.
#[derive(Clone, Copy)]
struct Plot {
    left: f32,
    right: f32,
    top: f32,
    bottom: f32,
}

impl Plot {
    fn o_min() -> f32 {
        F_MIN.log2()
    }
    fn o_span() -> f32 {
        (F_MAX / F_MIN).log2()
    }
    fn x_of_ms(&self, ms: f32) -> f32 {
        self.right - ms / MAX_DELAY_MS * (self.right - self.left)
    }
    fn ms_per_px(&self) -> f32 {
        MAX_DELAY_MS / (self.right - self.left)
    }
    fn y_of_oct(&self, o: f32) -> f32 {
        self.bottom - (o - Self::o_min()) / Self::o_span() * (self.bottom - self.top)
    }
    fn oct_of_y(&self, y: f32) -> f32 {
        Self::o_min() + (self.bottom - y) / (self.bottom - self.top) * Self::o_span()
    }
    fn oct_per_px(&self) -> f32 {
        Self::o_span() / (self.bottom - self.top)
    }
    fn clip(&self, y: f32) -> f32 {
        y.clamp(self.top, self.bottom)
    }
}

/// Where a head's two slats are, with the glide it is currently on added to its offset.
#[derive(Clone, Copy)]
struct Geom {
    out_x: f32,
    src_x: f32,
    out_top: f32,
    out_bottom: f32,
    src_top: f32,
    src_bottom: f32,
    out_centre: f32,
    src_centre: f32,
    on: bool,
}

fn geom(plot: &Plot, p: &HeadParams, glide: f32) -> Geom {
    let c = p.centre.value().log2();
    let half = p.width.value() * 0.5;
    let shift = (p.shift.value() + glide) / 12.0;
    let x = plot.x_of_ms(p.delay.value());
    Geom {
        out_x: x,
        src_x: x - SLAT_GAP,
        out_top: plot.clip(plot.y_of_oct(c + shift + half)),
        out_bottom: plot.clip(plot.y_of_oct(c + shift - half)),
        src_top: plot.clip(plot.y_of_oct(c + half)),
        src_bottom: plot.clip(plot.y_of_oct(c - half)),
        out_centre: plot.clip(plot.y_of_oct(c + shift)),
        src_centre: plot.clip(plot.y_of_oct(c)),
        on: p.is_on(),
    }
}

fn hit_test(geoms: &[Geom; HEADS], selected: usize, pos: Pos2) -> Option<(usize, Kind)> {
    let mut best: Option<(f32, usize, Kind)> = None;
    for (h, g) in geoms.iter().enumerate() {
        for (is_out, x, top, bottom) in [(true, g.out_x, g.out_top, g.out_bottom), (false, g.src_x, g.src_top, g.src_bottom)] {
            let dx = (pos.x - x).abs();
            if dx > SLAT_HALF + 4.0 || pos.y < top - 7.0 || pos.y > bottom + 7.0 {
                continue;
            }
            let at_edge = (pos.y - top).abs() < 7.0 || (pos.y - bottom).abs() < 7.0;
            let kind = if at_edge { Kind::Edge(is_out) } else if is_out { Kind::Out } else { Kind::Src };
            // The nearest slat wins; the selected head wins ties.
            let score = dx - if h == selected { 2.0 } else { 0.0 };
            if best.map_or(true, |b| score < b.0) {
                best = Some((score, h, kind));
            }
        }
    }
    best.map(|(_, h, k)| (h, k))
}

fn note(state: &mut UiState, ui: &Ui, lines: Vec<(String, String)>) {
    state.readout = lines;
    state.readout_time = ui.input(|i| i.time);
}

fn set(setter: &ParamSetter, param: &FloatParam, value: f32) {
    setter.set_parameter(param, value);
}

fn set_once(setter: &ParamSetter, param: &FloatParam, value: f32) {
    setter.begin_set_parameter(param);
    setter.set_parameter(param, value);
    setter.end_set_parameter(param);
}

pub fn draw(ui: &mut Ui, params: &Arc<LoomParams>, setter: &ParamSetter, meter: &Arc<Meter>, state: &mut UiState) {
    ui.ctx().request_repaint();
    follow(state, meter);

    header(ui, params, setter);
    ui.add_space(8.0);
    let graph_height = ui.available_height() - TABS_ROW - KNOB_ROW - 16.0;
    graph(ui, params, setter, meter, state, graph_height);
    ui.add_space(6.0);
    tabs(ui, params, setter, state);
    ui.add_space(4.0);
    knobs(ui, params, setter, state);
}

fn header(ui: &mut Ui, params: &LoomParams, setter: &ParamSetter) {
    let (response, painter) = ui.allocate_painter(Vec2::new(ui.available_width(), 24.0), Sense::hover());
    let rect = response.rect;
    painter.text(rect.left_center(), Align2::LEFT_CENTER, "L O O M", FontId::monospace(15.0), rgba(TEXT, 255.0));
    painter.text(
        rect.left_center() + Vec2::new(86.0, 1.0),
        Align2::LEFT_CENTER,
        "spectral playheads",
        FontId::monospace(10.0),
        TEXT_DIM,
    );
    let theme = preset_kit::ui::Theme { fill: PANEL, hover: GRID, ink: rgba(TEXT, 255.0), dim: TEXT_DIM };
    preset_kit::ui::picker(ui, Pos2::new(rect.left() + 250.0, rect.center().y - 11.0), 22.0, &presets::BANK, params, setter, &theme);

    let live = params.heads.iter().filter(|h| h.is_on()).count();
    painter.text(
        rect.right_center(),
        Align2::RIGHT_CENTER,
        format!("{live}/{HEADS} heads on   fb {:.0}%", params.feedback.value() * 100.0),
        FontId::monospace(10.0),
        if live > 0 { rgba(TEAL, 255.0) } else { TEXT_DIM },
    );
}

fn graph(ui: &mut Ui, params: &Arc<LoomParams>, setter: &ParamSetter, meter: &Meter, state: &mut UiState, height: f32) {
    let size = Vec2::new(ui.available_width(), height);
    let (response, painter) = ui.allocate_painter(size, Sense::click_and_drag());
    let rect = response.rect;
    let plot = Plot { left: rect.left() + AXIS_W, right: rect.right() - STRIP_W - 8.0, top: rect.top() + 8.0, bottom: rect.bottom() - 20.0 };
    let strip_left = plot.right + 8.0;

    let glides: [f32; HEADS] = std::array::from_fn(|h| meter.glide[h].load(Ordering::Relaxed));
    let geoms: [Geom; HEADS] = std::array::from_fn(|h| geom(&plot, &params.heads[h], glides[h]));

    interact(ui, &response, &plot, params, setter, state, &geoms);
    let hover = if state.drag.is_none() { response.hover_pos().and_then(|p| hit_test(&geoms, state.selected, p)) } else { None };

    // Backdrop.
    strip(
        &painter,
        &[
            (rect.left_top(), rgba(GLOW_TOP, 255.0), rect.left_bottom(), rgba(GLOW_BOTTOM, 255.0)),
            (rect.right_top(), rgba(GLOW_TOP, 255.0), rect.right_bottom(), rgba(GLOW_BOTTOM, 255.0)),
        ],
    );

    cloth(&painter, &plot, meter, state, params, &geoms);

    // Frequency and time scales.
    let faint = Stroke::new(1.0_f32, rgba([255.0, 255.0, 255.0], 9.0));
    for (f, label) in [(50.0, "50"), (100.0, "100"), (200.0, "200"), (500.0, "500"), (1000.0, "1k"), (2000.0, "2k"), (5000.0, "5k"), (10000.0, "10k")] {
        let y = plot.y_of_oct(f32::log2(f));
        painter.line_segment([Pos2::new(plot.left, y), Pos2::new(strip_left + STRIP_W, y)], faint);
        painter.text(Pos2::new(plot.left - 6.0, y), Align2::RIGHT_CENTER, label, FontId::monospace(8.0), TEXT_DIM);
    }
    for s in 0..=4 {
        let x = plot.x_of_ms(s as f32 * 1000.0);
        painter.line_segment([Pos2::new(x, plot.bottom), Pos2::new(x, plot.bottom + 4.0)], Stroke::new(1.0_f32, GRID));
        let label = if s == 0 { "now".to_string() } else { format!("-{s}s") };
        painter.text(Pos2::new(x, plot.bottom + 7.0), if s == 4 { Align2::LEFT_TOP } else { Align2::CENTER_TOP }, label, FontId::monospace(9.0), TEXT_DIM);
    }
    painter.line_segment([Pos2::new(plot.left, plot.bottom), Pos2::new(strip_left + STRIP_W, plot.bottom)], Stroke::new(1.0_f32, GRID));

    // The beater: where the cloth is made.
    painter.line_segment([Pos2::new(plot.right + 4.0, plot.top), Pos2::new(plot.right + 4.0, plot.bottom)], Stroke::new(6.0_f32, rgba(ORANGE, 26.0)));
    painter.line_segment([Pos2::new(plot.right + 4.0, plot.top), Pos2::new(plot.right + 4.0, plot.bottom)], Stroke::new(1.5_f32, rgba(HOT, 200.0)));

    finished_cloth(&painter, &plot, strip_left, state);

    // Weft: each head throws a thread from its teal slat to the finished cloth.
    for h in 0..HEADS {
        let g = geoms[h];
        if !g.on {
            continue;
        }
        let a = 0.25 + 0.75 * state.act[h];
        let y = g.out_centre;
        painter.line_segment([Pos2::new(g.out_x, y), Pos2::new(plot.right + 4.0, y)], Stroke::new(5.0_f32, rgba(TEAL, 22.0 * a)));
        painter.line_segment([Pos2::new(g.out_x, y), Pos2::new(plot.right + 4.0, y)], Stroke::new(1.0_f32, rgba(TEAL, 150.0 * a)));
    }

    // Heddles. The selected head is drawn last so it sits on top.
    let order = (0..HEADS).filter(|&h| h != state.selected).chain(std::iter::once(state.selected));
    for h in order {
        heddle(&painter, &geoms[h], h, h == state.selected, state.act[h], hover.filter(|(hh, _)| *hh == h).map(|(_, k)| k), state.drag.as_ref().filter(|d| d.head == h).map(|d| d.kind));
    }

    // The value being edited, in big numerals.
    let now = ui.input(|i| i.time);
    if now - state.readout_time < 1.1 && !state.readout.is_empty() {
        let fade = (1.0 - ((now - state.readout_time - 0.7) / 0.4).clamp(0.0, 1.0)) as f32;
        let width = 150.0 * state.readout.len() as f32 + 12.0;
        let back = Rect::from_min_size(Pos2::new(plot.left + 8.0, plot.top + 8.0), Vec2::new(width, 66.0));
        painter.rect_filled(back, 6.0, Color32::from_rgba_unmultiplied(12, 10, 30, (200.0 * fade) as u8));
        for (i, (label, value)) in state.readout.iter().enumerate() {
            let x = back.left() + 12.0 + i as f32 * 150.0;
            painter.text(Pos2::new(x, back.top() + 9.0), Align2::LEFT_TOP, label, FontId::monospace(9.0), rgba(TEXT, 130.0 * fade));
            painter.text(Pos2::new(x, back.top() + 22.0), Align2::LEFT_TOP, value, FontId::monospace(30.0), rgba(PINK, 255.0 * fade));
        }
    }
}

fn interact(ui: &Ui, response: &egui::Response, plot: &Plot, params: &LoomParams, setter: &ParamSetter, state: &mut UiState, geoms: &[Geom; HEADS]) {
    let fine = ui.input(|i| i.modifiers.shift);
    let speed = if fine { 0.1 } else { 1.0 };

    if response.drag_started() {
        if let Some((h, kind)) = response.interact_pointer_pos().and_then(|p| hit_test(geoms, state.selected, p)) {
            let hp = &params.heads[h];
            state.selected = h;
            if !hp.is_on() {
                set_once(setter, &hp.level, 0.0);
            }
            match kind {
                Kind::Out => {
                    setter.begin_set_parameter(&hp.delay);
                    setter.begin_set_parameter(&hp.shift);
                }
                Kind::Src => setter.begin_set_parameter(&hp.centre),
                Kind::Edge(_) => setter.begin_set_parameter(&hp.width),
            }
            state.drag = Some(Drag { head: h, kind, delay: hp.delay.value(), shift: hp.shift.value(), centre_oct: hp.centre.value().log2() });
        }
    }
    if response.dragged() {
        if let Some(drag) = state.drag.as_mut() {
            let hp = &params.heads[drag.head];
            let delta = response.drag_delta();
            let mut lines = Vec::new();
            match drag.kind {
                Kind::Out => {
                    drag.delay = (drag.delay - delta.x * plot.ms_per_px() * speed).clamp(0.0, MAX_DELAY_MS);
                    drag.shift = (drag.shift - delta.y * plot.oct_per_px() * 12.0 * speed).clamp(-MAX_SHIFT, MAX_SHIFT);
                    set(setter, &hp.delay, drag.delay);
                    let shift = if fine { drag.shift } else { drag.shift.round() };
                    set(setter, &hp.shift, shift);
                    lines.push(("DELAY".to_string(), format!("{:.0} ms", drag.delay)));
                    lines.push(("SHIFT".to_string(), format!("{shift:+.1} st")));
                }
                Kind::Src => {
                    drag.centre_oct -= delta.y * plot.oct_per_px() * speed;
                    let hz = drag.centre_oct.exp2().clamp(40.0, 12000.0);
                    drag.centre_oct = hz.log2();
                    set(setter, &hp.centre, hz);
                    lines.push(("CENTRE".to_string(), format!("{hz:.0} Hz")));
                }
                Kind::Edge(on_out) => {
                    if let Some(pos) = response.interact_pointer_pos() {
                        let centre = hp.centre.value().log2() + if on_out { hp.shift.value() / 12.0 } else { 0.0 };
                        let at_limit = pos.y <= plot.top + 4.0 || pos.y >= plot.bottom - 4.0;
                        let width = (2.0 * (plot.oct_of_y(pos.y) - centre).abs()).clamp(MIN_WIDTH, FULL_WIDTH);
                        let width = if at_limit { FULL_WIDTH } else { width };
                        set(setter, &hp.width, width);
                        let text = if width >= FULL_WIDTH - 0.01 { "full".to_string() } else { format!("{width:.2} oct") };
                        lines.push(("WIDTH".to_string(), text));
                    }
                }
            }
            note(state, ui, lines);
        }
    }
    if response.drag_stopped() {
        if let Some(drag) = state.drag.take() {
            let hp = &params.heads[drag.head];
            match drag.kind {
                Kind::Out => {
                    setter.end_set_parameter(&hp.delay);
                    setter.end_set_parameter(&hp.shift);
                }
                Kind::Src => setter.end_set_parameter(&hp.centre),
                Kind::Edge(_) => setter.end_set_parameter(&hp.width),
            }
        }
    }
    if response.clicked() || response.double_clicked() {
        if let Some((h, kind)) = response.interact_pointer_pos().and_then(|p| hit_test(geoms, state.selected, p)) {
            state.selected = h;
            if response.double_clicked() {
                let hp = &params.heads[h];
                match kind {
                    Kind::Out => {
                        set_once(setter, &hp.shift, hp.shift.default_plain_value());
                        set_once(setter, &hp.slide, hp.slide.default_plain_value());
                    }
                    Kind::Src => set_once(setter, &hp.centre, hp.centre.default_plain_value()),
                    Kind::Edge(_) => set_once(setter, &hp.width, hp.width.default_plain_value()),
                }
            }
        }
    }
}

/// The warp: one thread per column of spectrogram, brighter and hotter where the input is louder,
/// teal where a head is reading it.
fn cloth(painter: &Painter, plot: &Plot, meter: &Meter, state: &UiState, params: &LoomParams, geoms: &[Geom; HEADS]) {
    let cursor = meter.cursor.load(Ordering::Relaxed);
    let progress = meter.progress.load(Ordering::Relaxed);
    let col_s = meter.col_seconds.load(Ordering::Relaxed).max(1e-3);
    let width = plot.right - plot.left;
    let col_px = col_s / MAX_S * width;
    let thread_w = (col_px * 0.72).max(1.5);
    let band_oct = |b: usize| f32::log2(F_MIN) + (b as f32 + 0.5) / BANDS as f32 * Plot::o_span();

    // Heads that light threads: x, strength, centre (octaves), width.
    let lights: Vec<(f32, f32, f32, f32)> = (0..HEADS)
        .filter(|&h| geoms[h].on)
        .map(|h| (geoms[h].out_x, 0.35 + 0.65 * state.act[h], params.heads[h].centre.value().log2(), params.heads[h].width.value()))
        .collect();
    let max_cols = (width / col_px).ceil() as usize + 1;
    for a in 0..max_cols {
        let x = plot.right - (a as f32 + 0.5 + progress) * col_px;
        if x - thread_w * 0.5 < plot.left {
            break;
        }
        let have = a < cursor;
        let col = if have { (cursor - 1 - a) % COLS } else { 0 };
        let mut mesh = Mesh::default();
        let mut under = Mesh::default();
        let push = |y: f32, colour: Color32, mesh: &mut Mesh, under: &mut Mesh| {
            mesh.colored_vertex(Pos2::new(x - thread_w * 0.5, y), colour);
            mesh.colored_vertex(Pos2::new(x + thread_w * 0.5, y), colour);
            let [r, g, b, a] = colour.to_array();
            let soft = Color32::from_rgba_unmultiplied(r, g, b, (a as f32 * 0.4) as u8);
            under.colored_vertex(Pos2::new(x - col_px * 0.5, y), soft);
            under.colored_vertex(Pos2::new(x + col_px * 0.5, y), soft);
        };
        let mut first: Option<Color32> = None;
        let mut last = Color32::TRANSPARENT;
        for b in 0..BANDS {
            let db = if have { meter.spec[col * BANDS + b].load(Ordering::Relaxed) } else { -120.0 };
            let t = ((db + 78.0) / 70.0).clamp(0.0, 1.0).powf(1.25);
            let base = if t < 0.5 { mix(THREAD, ORANGE, t * 2.0) } else { mix(ORANGE, HOT, (t - 0.5) * 2.0) };
            let alpha = 60.0 + 195.0 * t.powf(0.7);
            let o = band_oct(b);
            let mut lit = 0.0f32;
            for &(hx, strength, c, w) in &lights {
                let falloff = (1.0 - (x - hx).abs() / (col_px * 3.2)).max(0.0);
                if falloff > 0.0 {
                    lit = lit.max(falloff * strength * window_gain(o - c, w));
                }
            }
            let colour = rgba(mix(base, mix(TEAL, [220.0, 255.0, 248.0], t * 0.6), lit * 0.9), alpha.max(110.0 * lit + 40.0 * (lit > 0.0) as u32 as f32));
            if first.is_none() {
                first = Some(colour);
            }
            last = colour;
            push(plot.y_of_oct(o), colour, &mut mesh, &mut under);
        }
        // Extend the ends of the thread to the edges of the cloth.
        if let Some(f) = first {
            let mut m2 = Mesh::default();
            m2.colored_vertex(Pos2::new(x - thread_w * 0.5, plot.bottom), f);
            m2.colored_vertex(Pos2::new(x + thread_w * 0.5, plot.bottom), f);
            m2.colored_vertex(Pos2::new(x - thread_w * 0.5, plot.y_of_oct(band_oct(0))), f);
            m2.colored_vertex(Pos2::new(x + thread_w * 0.5, plot.y_of_oct(band_oct(0))), f);
            m2.add_triangle(0, 1, 2);
            m2.add_triangle(1, 2, 3);
            painter.add(Shape::mesh(m2));
            let mut m3 = Mesh::default();
            let ytop = plot.y_of_oct(band_oct(BANDS - 1));
            m3.colored_vertex(Pos2::new(x - thread_w * 0.5, plot.top), last);
            m3.colored_vertex(Pos2::new(x + thread_w * 0.5, plot.top), last);
            m3.colored_vertex(Pos2::new(x - thread_w * 0.5, ytop), last);
            m3.colored_vertex(Pos2::new(x + thread_w * 0.5, ytop), last);
            m3.add_triangle(0, 1, 2);
            m3.add_triangle(1, 2, 3);
            painter.add(Shape::mesh(m3));
        }
        for i in 0..BANDS.saturating_sub(1) {
            let k = (i * 2) as u32;
            mesh.add_triangle(k, k + 1, k + 2);
            mesh.add_triangle(k + 1, k + 2, k + 3);
            under.add_triangle(k, k + 1, k + 2);
            under.add_triangle(k + 1, k + 2, k + 3);
        }
        painter.add(Shape::mesh(under));
        painter.add(Shape::mesh(mesh));
    }
}

/// The output spectrum as weft: one horizontal bar per band, longer where it is louder.
fn finished_cloth(painter: &Painter, plot: &Plot, left: f32, state: &UiState) {
    painter.text(Pos2::new(left, plot.top - 1.0), Align2::LEFT_BOTTOM, "", FontId::monospace(8.0), TEXT_DIM);
    let h = (plot.bottom - plot.top) / BANDS as f32;
    let mut mesh = Mesh::default();
    for b in 0..BANDS {
        let o = f32::log2(F_MIN) + (b as f32 + 0.5) / BANDS as f32 * Plot::o_span();
        let y = plot.y_of_oct(o);
        let t = ((state.out_db[b] + 84.0) / 72.0).clamp(0.0, 1.0);
        let len = 3.0 + t * (STRIP_W - 3.0);
        let thick = (h * 0.62).max(1.2);
        let k = mesh.vertices.len() as u32;
        let dim = rgba(mix(TEAL, [12.0, 10.0, 30.0], 0.7), 90.0);
        let bright = rgba(mix(TEAL, [220.0, 255.0, 248.0], t * 0.5), 60.0 + 195.0 * t);
        mesh.colored_vertex(Pos2::new(left, y - thick * 0.5), dim);
        mesh.colored_vertex(Pos2::new(left + len, y - thick * 0.5), bright);
        mesh.colored_vertex(Pos2::new(left, y + thick * 0.5), dim);
        mesh.colored_vertex(Pos2::new(left + len, y + thick * 0.5), bright);
        mesh.add_triangle(k, k + 1, k + 2);
        mesh.add_triangle(k + 1, k + 2, k + 3);
    }
    painter.add(Shape::mesh(mesh));
    painter.text(Pos2::new(left + STRIP_W, plot.bottom + 7.0), Align2::RIGHT_TOP, "out", FontId::monospace(9.0), rgba(TEAL, 200.0));
}

#[allow(clippy::too_many_arguments)]
fn heddle(painter: &Painter, g: &Geom, index: usize, selected: bool, act: f32, hover: Option<Kind>, drag: Option<Kind>) {
    let live = if g.on { 1.0 } else { 0.2 };
    let lit = |k: Kind| (hover == Some(k) || drag == Some(k)) as u32 as f32;
    let accent = if selected { PINK } else { TEAL };
    let sel = if selected { 1.0 } else { 0.0 };

    // A thread from where the head reads to where it writes; slanted when the head shifts pitch.
    painter.line_segment([Pos2::new(g.src_x, g.src_centre), Pos2::new(g.out_x, g.out_centre)], Stroke::new(4.0_f32, rgba(accent, 36.0 * live)));
    painter.line_segment([Pos2::new(g.src_x, g.src_centre), Pos2::new(g.out_x, g.out_centre)], Stroke::new(1.25_f32, rgba(accent, 200.0 * live)));

    // Hollow slat: the band it reads.
    let src = Rect::from_min_max(Pos2::new(g.src_x - SLAT_HALF, g.src_top), Pos2::new(g.src_x + SLAT_HALF, g.src_bottom.max(g.src_top + 1.0)));
    painter.rect_filled(src, 3.0, rgba(HOT, (20.0 + 30.0 * lit(Kind::Src) + 20.0 * sel) * live));
    painter.rect_stroke(src, 3.0, Stroke::new(1.25_f32, rgba(HOT, (120.0 + 120.0 * lit(Kind::Src) + 60.0 * lit(Kind::Edge(false))) * live)), StrokeKind::Inside);

    // Teal slat: where it writes, glowing with what it is putting out.
    let out = Rect::from_min_max(Pos2::new(g.out_x - SLAT_HALF, g.out_top), Pos2::new(g.out_x + SLAT_HALF, g.out_bottom.max(g.out_top + 1.0)));
    painter.rect_filled(out.expand(4.0), 6.0, rgba(accent, (10.0 + 40.0 * act) * live));
    painter.rect_filled(out, 3.0, rgba(accent, (70.0 + 120.0 * act + 50.0 * lit(Kind::Out) + 30.0 * sel) * live));
    painter.rect_stroke(out, 3.0, Stroke::new(1.5_f32, rgba(mix(accent, [255.0, 255.0, 255.0], 0.25 * sel + 0.25 * lit(Kind::Out)), (170.0 + 85.0 * lit(Kind::Out)) * live)), StrokeKind::Inside);

    // Grips at the ends, so the width handles show even when the slat runs off the cloth.
    for (x, y, k) in [(g.out_x, g.out_top, Kind::Edge(true)), (g.out_x, g.out_bottom, Kind::Edge(true)), (g.src_x, g.src_top, Kind::Edge(false)), (g.src_x, g.src_bottom, Kind::Edge(false))] {
        let r = Rect::from_center_size(Pos2::new(x, y), Vec2::new(SLAT_HALF * 2.0 + 4.0, 3.0));
        painter.rect_filled(r, 1.0, rgba(if x == g.out_x { accent } else { HOT }, (150.0 + 105.0 * lit(k)) * live));
    }

    // The head's number, on a chip at the top of the teal slat.
    let chip = Rect::from_center_size(Pos2::new(g.out_x, g.out_top + 11.0), Vec2::new(14.0, 14.0));
    painter.rect_filled(chip, 3.0, rgba(if selected { PINK } else { [12.0, 10.0, 30.0] }, 255.0 * live.max(0.5)));
    painter.rect_stroke(chip, 3.0, Stroke::new(1.0_f32, rgba(accent, 200.0 * live.max(0.4))), StrokeKind::Inside);
    painter.text(chip.center(), Align2::CENTER_CENTER, format!("{}", index + 1), FontId::monospace(9.0), if selected { rgba([12.0, 10.0, 30.0], 255.0) } else { rgba(TEXT, 230.0 * live.max(0.5)) });
}

/// Eight tabs: pick the head the knobs edit, or switch it on and off with its lamp.
fn tabs(ui: &mut Ui, params: &LoomParams, setter: &ParamSetter, state: &mut UiState) {
    let (response, painter) = ui.allocate_painter(Vec2::new(ui.available_width(), TABS_ROW), Sense::hover());
    let row = response.rect;
    let w = 66.0;
    for h in 0..HEADS {
        let tab = Rect::from_min_size(Pos2::new(row.left() + h as f32 * (w + 4.0), row.top() + 2.0), Vec2::new(w, 22.0));
        let lamp = Rect::from_center_size(Pos2::new(tab.left() + 13.0, tab.center().y), Vec2::new(20.0, 22.0));
        let hp = &params.heads[h];
        let r = ui.interact(tab, ui.id().with(("tab", h)), Sense::click());
        let l = ui.interact(lamp, ui.id().with(("lamp", h)), Sense::click());
        if l.clicked() {
            set_once(setter, &hp.level, if hp.is_on() { OFF_DB } else { 0.0 });
            state.selected = h;
        } else if r.clicked() {
            state.selected = h;
        }
        let selected = state.selected == h;
        let on = hp.is_on();
        let fill = if selected { rgba(PINK, 40.0) } else if r.hovered() { PANEL } else { BG };
        painter.rect_filled(tab, 4.0, fill);
        painter.rect_stroke(tab, 4.0, Stroke::new(1.0_f32, if selected { rgba(PINK, 255.0) } else { GRID }), StrokeKind::Inside);
        let c = lamp.center();
        if on {
            painter.circle_filled(c, 6.0, rgba(TEAL, 40.0 + 60.0 * state.act[h]));
            painter.circle_filled(c, 3.0, rgba(TEAL, 255.0));
        } else {
            painter.circle_stroke(c, 3.0, Stroke::new(1.0_f32, if l.hovered() { rgba(TEAL, 200.0) } else { TEXT_DIM }));
        }
        painter.text(
            Pos2::new(tab.left() + 28.0, tab.center().y),
            Align2::LEFT_CENTER,
            format!("HEAD {}", h + 1),
            FontId::monospace(9.0),
            if selected { rgba(TEXT, 255.0) } else { TEXT_DIM },
        );
    }
    let sel = &params.heads[state.selected];
    painter.text(
        Pos2::new(row.right(), row.center().y),
        Align2::RIGHT_CENTER,
        format!(
            "{:.0} ms  {:+.1} st  {:.0} Hz x {}",
            sel.delay.value(),
            sel.shift.value(),
            sel.centre.value(),
            if sel.width.value() >= FULL_WIDTH - 0.01 { "full".to_string() } else { format!("{:.1} oct", sel.width.value()) }
        ),
        FontId::monospace(10.0),
        TEXT_DIM,
    );
}

/// The selected head's seven knobs in teal, then the three that belong to the whole loom.
fn knobs(ui: &mut Ui, params: &LoomParams, setter: &ParamSetter, state: &mut UiState) {
    let (response, painter) = ui.allocate_painter(Vec2::new(ui.available_width(), KNOB_ROW), Sense::hover());
    let row = response.rect;
    let hp = &params.heads[state.selected];
    let teal = rgba(TEAL, 255.0);
    let orange = rgba(ORANGE, 255.0);
    let cells: [(&FloatParam, &str, Color32, bool); 10] = [
        (&hp.delay, "DELAY", teal, false),
        (&hp.shift, "SHIFT", teal, true),
        (&hp.slide, "SLIDE", teal, true),
        (&hp.centre, "CENTRE", teal, false),
        (&hp.width, "WIDTH", teal, false),
        (&hp.level, "LEVEL", teal, false),
        (&hp.pan, "PAN", teal, true),
        (&params.feedback, "FEEDBACK", orange, false),
        (&params.smooth, "SMOOTH", orange, false),
        (&params.mix, "MIX", rgba(TEXT, 255.0), false),
    ];
    let width = row.width() / cells.len() as f32;
    for (i, (param, label, color, bipolar)) in cells.iter().enumerate() {
        let cell = Rect::from_min_size(Pos2::new(row.left() + i as f32 * width, row.top()), Vec2::new(width, KNOB_ROW));
        knob(ui, &painter, cell, setter, state, *param, label, *color, *bipolar);
    }
    // A rule between the head's knobs and the loom's.
    let x = row.left() + 7.0 * width;
    painter.line_segment([Pos2::new(x, row.top() + 6.0), Pos2::new(x, row.bottom() - 6.0)], Stroke::new(1.0_f32, GRID));
    painter.text(Pos2::new(row.left() + 2.0, row.top() + 2.0), Align2::LEFT_TOP, format!("HEAD {}", state.selected + 1), FontId::monospace(8.0), rgba(PINK, 220.0));
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
fn knob(ui: &mut Ui, painter: &Painter, cell: Rect, setter: &ParamSetter, state: &mut UiState, param: &FloatParam, label: &str, color: Color32, bipolar: bool) {
    let r = ui.interact(cell, ui.id().with(label), Sense::click_and_drag());
    if r.drag_started() {
        setter.begin_set_parameter(param);
    }
    if r.dragged() {
        let fine = ui.input(|i| i.modifiers.shift);
        let speed = if fine { 1.0 / 1200.0 } else { 1.0 / 160.0 };
        let v = param.unmodulated_normalized_value() - r.drag_delta().y * speed;
        setter.set_parameter_normalized(param, v.clamp(0.0, 1.0));
        note(state, ui, vec![(label.to_string(), param.to_string())]);
    }
    if r.drag_stopped() {
        setter.end_set_parameter(param);
    }
    if r.double_clicked() {
        set_once(setter, param, param.default_plain_value());
    }

    let norm = param.unmodulated_normalized_value();
    let center = Pos2::new(cell.center().x, cell.top() + 34.0);
    let radius = 23.0;
    let (start, end) = (PI * 0.75, PI * 2.25);
    let lit = r.hovered() || r.dragged();
    let [cr, cg, cb, _] = color.to_array();
    let rgb = [cr as f32, cg as f32, cb as f32];

    painter.circle_filled(center, radius - 5.0, if lit { PANEL } else { Color32::from_rgb(0x12, 0x10, 0x2a) });
    painter.add(Shape::line(arc(center, radius, start, end), Stroke::new(3.0_f32, GRID)));
    let from = if bipolar { (start + end) * 0.5 } else { start };
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
