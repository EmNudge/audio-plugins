//! The editor: a spectrum split by the carrier.
//!
//! The upper half is the input's spectrum in white, rising from the centre line. The lower half is
//! what came out, hanging below it. A ring modulator turns every input partial into two: one at
//! the sum of its frequency and the carrier (cyan) and one at the difference (magenta), and
//! wherever that difference would be negative it folds back off 0 Hz (dashed). Faint outlines
//! under the centre line are where that arithmetic says the sidebands should be, drawn from the
//! live input and the live carrier; the filled bars are what actually came out, so the diodes'
//! extra intermodulation shows up as yellow, the bars the sums and differences cannot explain.
//! The carrier is the yellow blade, and dragging the display moves it.
//!
//! Below, a scope shows the multiplication itself: the input, the carrier and the product, captured
//! on the carrier's phase. Chips pick the carrier source, circuit, wave and oversampling.
//!
//! Knobs: drag vertically (hold shift for fine), double-click to reset.

use nih_plug::prelude::*;
use nih_plug_egui::egui::{
    self, Align2, Color32, FontId, Mesh, Painter, Pos2, Rect, Sense, Shape as EguiShape, Stroke, Ui, Vec2,
};
use std::f32::consts::PI;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::dsp::{Circuit, Meter, Oversampling, Settings, Source, Wave, FFT_N, FLOOR_DB, SCOPE, SPEC_BINS};
use crate::presets;
use crate::SunderParams;

const BG: Color32 = Color32::from_rgb(0x0b, 0x0b, 0x13);
const PANEL: Color32 = Color32::from_rgb(0x14, 0x15, 0x20);
const GRID: Color32 = Color32::from_rgb(0x26, 0x28, 0x3a);
const TEXT_DIM: Color32 = Color32::from_rgb(0x7f, 0x82, 0x96);
const TEXT: [f32; 3] = [238.0, 238.0, 244.0];
/// The carrier.
const YELLOW: [f32; 3] = [255.0, 214.0, 10.0];
/// The sum sideband.
const CYAN: [f32; 3] = [40.0, 226.0, 236.0];
/// The difference sideband.
const MAGENTA: [f32; 3] = [255.0, 64.0, 168.0];

const CHIP_H: f32 = 22.0;
const SELECT_H: f32 = 30.0;
const KNOB_ROW: f32 = 100.0;
const SCOPE_H: f32 = 96.0;
/// Decibels the spectrum display spans below full scale.
const RANGE_DB: f32 = 84.0;
/// Decibels a held bin falls per repaint.
const FALL_DB: f32 = 0.9;

pub struct UiState {
    last_frames: usize,
    in_db: [f32; SPEC_BINS],
    out_db: [f32; SPEC_BINS],
}

impl Default for UiState {
    fn default() -> Self {
        Self { last_frames: 0, in_db: [FLOOR_DB; SPEC_BINS], out_db: [FLOOR_DB; SPEC_BINS] }
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

/// Everything the audio thread published, with the spectra held: rise fast, fall slowly.
struct Frame {
    fs: f32,
    carrier_hz: f32,
    target_hz: f32,
    base_hz: f32,
    scope: [[f32; SCOPE]; 3],
    scope_ms: f32,
}

fn snapshot(meter: &Meter, state: &mut UiState) -> Frame {
    let frames = meter.frames.load(Ordering::Acquire);
    if frames != state.last_frames {
        state.last_frames = frames;
        for k in 0..SPEC_BINS {
            for (held, new) in [(&mut state.in_db[k], meter.spec_in[k].load(Ordering::Relaxed)), (&mut state.out_db[k], meter.spec_out[k].load(Ordering::Relaxed))] {
                *held = new.max(*held - FALL_DB);
            }
        }
    }
    let ready = meter.scope_ready.load(Ordering::Acquire);
    let mut scope = [[0.0; SCOPE]; 3];
    for (lane, row) in scope.iter_mut().enumerate() {
        for (i, v) in row.iter_mut().enumerate() {
            *v = meter.scope[ready][lane][i].load(Ordering::Relaxed);
        }
    }
    Frame {
        fs: meter.sample_rate.load(Ordering::Relaxed).max(8000.0),
        carrier_hz: meter.carrier_hz.load(Ordering::Relaxed),
        target_hz: meter.target_hz.load(Ordering::Relaxed),
        base_hz: meter.base_hz.load(Ordering::Relaxed),
        scope,
        scope_ms: meter.scope_ms.load(Ordering::Relaxed),
    }
}

pub fn draw(ui: &mut Ui, params: &Arc<SunderParams>, setter: &ParamSetter, meter: &Arc<Meter>, state: &mut UiState) {
    ui.ctx().request_repaint();
    let f = snapshot(meter, state);
    let settings = params.settings();

    header(ui, params, setter, &f, state);
    ui.add_space(8.0);
    source_row(ui, params, setter);
    ui.add_space(8.0);

    let display_h = ui.available_height() - SELECT_H - KNOB_ROW - 16.0;
    let (area, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), display_h), Sense::hover());
    let spectrum_rect = Rect::from_min_max(area.min, Pos2::new(area.right(), area.bottom() - SCOPE_H - 8.0));
    let scope_rect = Rect::from_min_max(Pos2::new(area.left(), area.bottom() - SCOPE_H), area.max);
    spectrum(ui, spectrum_rect, params, setter, &f, state, &settings);
    scope(ui, scope_rect, &f);

    ui.add_space(8.0);
    selectors(ui, params, setter);
    ui.add_space(8.0);
    knobs(ui, params, setter, &f);
}

/// The strongest partial of the input, in Hz, if there is one.
fn input_peak(state: &UiState, fs: f32) -> Option<f32> {
    let (bin, db) = state.in_db.iter().enumerate().skip(2).fold((0, FLOOR_DB), |best, (k, &v)| if v > best.1 { (k, v) } else { best });
    (db > -60.0).then(|| bin as f32 * fs / FFT_N as f32)
}

fn header(ui: &mut Ui, params: &SunderParams, setter: &ParamSetter, f: &Frame, state: &UiState) {
    let theme = preset_kit::ui::Theme { fill: PANEL, hover: GRID, ink: rgba(TEXT, 255.0), dim: TEXT_DIM };
    preset_kit::ui::picker_top_right(ui, 24.0, &presets::BANK, params, setter, &theme);
    let external = params.source.value() == Source::Sidechain;
    let peak = input_peak(state, f.fs);
    let cell = |hz: Option<f32>| hz.map_or("    --".to_string(), |hz| format!("{hz:>6.0}"));
    let (sum, diff) = match peak {
        Some(p) if !external => (Some(p + f.carrier_hz), Some((p - f.carrier_hz).abs())),
        _ => (None, None),
    };
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("SUNDER").monospace().size(11.0).color(TEXT_DIM));
        ui.label(egui::RichText::new("ring modulator and frequency shifter").monospace().size(9.0).color(GRID));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(preset_kit::ui::WIDTH + 14.0);
            for (caption, text, color) in [
                ("DIFF", cell(diff), MAGENTA),
                ("SUM", cell(sum), CYAN),
                ("CARRIER", if external { "   ext".to_string() } else { cell(Some(f.carrier_hz)) }, YELLOW),
            ] {
                ui.label(egui::RichText::new(format!("{text}Hz")).monospace().size(18.0).color(rgba(color, 255.0)));
                ui.label(egui::RichText::new(caption).monospace().size(9.0).color(TEXT_DIM));
                ui.add_space(12.0);
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

fn upper<T: Enum>() -> Vec<String> {
    T::variants().iter().map(|n| n.to_uppercase()).collect()
}

fn source_row(ui: &mut Ui, params: &SunderParams, setter: &ParamSetter) {
    let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), CHIP_H), Sense::hover());
    let painter = ui.painter().clone();
    let label = |x: f32, text: &str| {
        painter.text(Pos2::new(x, row.center().y), Align2::LEFT_CENTER, text, FontId::monospace(9.0), TEXT_DIM);
    };
    label(row.left(), "CARRIER");
    let (clicked, end) = chips(ui, Pos2::new(row.left() + 64.0, row.top()), "source", &upper::<Source>(), params.source.value().to_index(), true, YELLOW);
    if let Some(i) = clicked {
        pick(setter, &params.source, i);
    }
    let x = end + 22.0;
    label(x, "CIRCUIT");
    let (clicked, _) = chips(ui, Pos2::new(x + 58.0, row.top()), "circuit", &upper::<Circuit>(), params.circuit.value().to_index(), true, MAGENTA);
    if let Some(i) = clicked {
        pick(setter, &params.circuit, i);
    }
}

fn selectors(ui: &mut Ui, params: &SunderParams, setter: &ParamSetter) {
    let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), SELECT_H), Sense::hover());
    let y = row.top() + (SELECT_H - CHIP_H) * 0.5;
    let painter = ui.painter().clone();
    let label = |x: f32, text: &str| {
        painter.text(Pos2::new(x, row.center().y), Align2::LEFT_CENTER, text, FontId::monospace(9.0), TEXT_DIM);
    };
    let internal = params.source.value() != Source::Sidechain;

    label(row.left(), "WAVE");
    let (clicked, end) = chips(ui, Pos2::new(row.left() + 40.0, y), "wave", &upper::<Wave>(), params.wave.value().to_index(), internal, YELLOW);
    if let Some(i) = clicked {
        pick(setter, &params.wave, i);
    }
    let x = end + 22.0;
    label(x, "OVERSAMPLE");
    let (clicked, _) = chips(ui, Pos2::new(x + 82.0, y), "os", &upper::<Oversampling>(), params.os.value().to_index(), true, CYAN);
    if let Some(i) = clicked {
        pick(setter, &params.os, i);
    }
}

fn height(db: f32) -> f32 {
    ((db + RANGE_DB) / RANGE_DB).clamp(0.0, 1.0)
}

fn amp(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

/// Where the sums and differences of the input and the carrier should land, as linear amplitudes
/// per display bin: the upper sideband, the lower, the lower folded back off 0 Hz, and the input
/// leaking through the bias.
struct Ghosts {
    up: [f32; SPEC_BINS],
    down: [f32; SPEC_BINS],
    fold: [f32; SPEC_BINS],
    leak: [f32; SPEC_BINS],
}

fn ghosts(in_db: &[f32; SPEC_BINS], carrier_bin: f32, s: &Settings) -> Ghosts {
    let at = |bin: f32| -> f32 {
        if bin < 0.0 || bin > (SPEC_BINS - 1) as f32 {
            return 0.0;
        }
        let i = bin.floor() as usize;
        let t = bin - i as f32;
        let j = (i + 1).min(SPEC_BINS - 1);
        amp(in_db[i]) * (1.0 - t) + amp(in_db[j]) * t
    };
    let gain = 1.0 / (1.0 + s.shift.abs()).sqrt();
    let (wu, wl) = (s.upper_weight() * gain, s.lower_weight() * gain);
    let leak = if s.circuit == Circuit::Ideal { s.bias / (1.0 + s.bias) * gain } else { 0.0 };
    let mut g = Ghosts { up: [0.0; SPEC_BINS], down: [0.0; SPEC_BINS], fold: [0.0; SPEC_BINS], leak: [0.0; SPEC_BINS] };
    for k in 0..SPEC_BINS {
        let b = k as f32;
        g.up[k] = at(b - carrier_bin) * wu;
        g.down[k] = at(b + carrier_bin) * wl;
        g.fold[k] = if b < carrier_bin { at(carrier_bin - b) * wl } else { 0.0 };
        g.leak[k] = at(b) * leak;
    }
    g
}

fn db_of(amp: f32) -> f32 {
    20.0 * (amp + 1e-7).log10()
}

/// A filled skyline: columns at `xs`, rising (`dir` = -1) or hanging (`dir` = 1) from `base` by
/// `tips`, shaded from `base_alpha` at the base to `tip_alpha` at the tip.
fn skyline(painter: &Painter, xs: &[f32], tips: &[f32], base: f32, dir: f32, colors: &[[f32; 3]], base_alpha: f32, tip_alpha: f32) {
    let mut mesh = Mesh::default();
    for i in 0..xs.len() {
        mesh.colored_vertex(Pos2::new(xs[i], base), rgba(colors[i], base_alpha));
        mesh.colored_vertex(Pos2::new(xs[i], base + dir * tips[i]), rgba(colors[i], tip_alpha));
    }
    for i in 0..xs.len().saturating_sub(1) as u32 {
        let v = 2 * i;
        mesh.add_triangle(v, v + 1, v + 3);
        mesh.add_triangle(v, v + 3, v + 2);
    }
    painter.add(EguiShape::mesh(mesh));
}

/// A glowing polyline through the tips, coloured per column.
fn outline(painter: &Painter, xs: &[f32], tips: &[f32], base: f32, dir: f32, colors: &[[f32; 3]], strength: f32) {
    for i in 1..xs.len() {
        let a = Pos2::new(xs[i - 1], base + dir * tips[i - 1]);
        let b = Pos2::new(xs[i], base + dir * tips[i]);
        let lit = (tips[i - 1].max(tips[i]) / 4.0).min(1.0);
        painter.line_segment([a, b], Stroke::new(5.0_f32, rgba(colors[i], 26.0 * strength * lit)));
        painter.line_segment([a, b], Stroke::new(1.4_f32, rgba(colors[i], 235.0 * strength * lit)));
    }
}

fn spectrum(ui: &mut Ui, rect: Rect, params: &SunderParams, setter: &ParamSetter, f: &Frame, state: &UiState, s: &Settings) {
    let response = ui.interact(rect, ui.id().with("spectrum"), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 6.0, PANEL);
    let editable = s.source != Source::Sidechain;
    let lit = editable && (response.hovered() || response.dragged());

    let plot = Rect::from_min_max(Pos2::new(rect.left() + 16.0, rect.top() + 30.0), Pos2::new(rect.right() - 16.0, rect.bottom() - 18.0));
    let mid = plot.center().y;
    let half = plot.height() * 0.5 - 4.0;
    let bin_hz = f.fs / FFT_N as f32;
    let f_max = (SPEC_BINS - 1) as f32 * bin_hz;
    let x_of_bin = |b: f32| plot.left() + plot.width() * b / (SPEC_BINS - 1) as f32;
    let x_of_hz = |hz: f32| plot.left() + plot.width() * hz / f_max;
    let xs: Vec<f32> = (0..SPEC_BINS).map(|k| x_of_bin(k as f32)).collect();

    painter.text(Pos2::new(rect.left() + 10.0, rect.top() + 8.0), Align2::LEFT_TOP, "input above, output below", FontId::monospace(9.0), TEXT_DIM);
    if editable {
        painter.text(Pos2::new(rect.right() - 10.0, rect.top() + 8.0), Align2::RIGHT_TOP, "drag: carrier", FontId::monospace(9.0), TEXT_DIM);
    }

    // Level guides and the frequency ruler.
    for db in [-20.0, -40.0, -60.0] {
        let h = height(db) * half;
        for dir in [-1.0, 1.0] {
            painter.line_segment([Pos2::new(plot.left(), mid + dir * h), Pos2::new(plot.right(), mid + dir * h)], Stroke::new(1.0_f32, rgba(TEXT, 8.0)));
        }
    }
    painter.line_segment([Pos2::new(plot.left(), mid), Pos2::new(plot.right(), mid)], Stroke::new(1.0_f32, GRID));
    let mut hz = 0.0;
    while hz <= f_max {
        let x = x_of_hz(hz);
        let major = (hz as i32) % 1000 == 0;
        painter.line_segment([Pos2::new(x, mid - if major { 4.0 } else { 2.0 }), Pos2::new(x, mid + if major { 4.0 } else { 2.0 })], Stroke::new(1.0_f32, TEXT_DIM));
        if major && hz > 0.0 {
            painter.text(Pos2::new(x, mid + 6.0), Align2::CENTER_TOP, format!("{:.0}k", hz / 1000.0), FontId::monospace(8.0), TEXT_DIM);
        }
        hz += 500.0;
    }
    // 0 Hz is a mirror: differences below it come back up.
    painter.line_segment([Pos2::new(plot.left(), mid + 6.0), Pos2::new(plot.left(), plot.bottom())], Stroke::new(1.0_f32, rgba(MAGENTA, 90.0)));
    painter.text(Pos2::new(plot.left() + 5.0, plot.bottom()), Align2::LEFT_BOTTOM, "0 Hz fold", FontId::monospace(8.0), rgba(MAGENTA, 150.0));

    // The input, rising.
    let in_tips: Vec<f32> = state.in_db.iter().map(|&db| height(db) * half).collect();
    let white = vec![TEXT; SPEC_BINS];
    skyline(&painter, &xs, &in_tips, mid, -1.0, &white, 6.0, 110.0);
    outline(&painter, &xs, &in_tips, mid, -1.0, &white, 0.9);

    // Where the sums and differences should land: thin outlines under the bars.
    let carrier_bin = f.carrier_hz / bin_hz;
    let g = ghosts(&state.in_db, carrier_bin, s);
    let tip = |amp: f32| height(db_of(amp)) * half;
    let curve = |values: &[f32; SPEC_BINS]| -> Vec<Pos2> { (0..SPEC_BINS).map(|k| Pos2::new(xs[k], mid + tip(values[k]))).collect() };
    if editable {
        painter.add(EguiShape::line(curve(&g.up), Stroke::new(1.0_f32, rgba(CYAN, 150.0))));
        painter.add(EguiShape::line(curve(&g.down), Stroke::new(1.0_f32, rgba(MAGENTA, 150.0))));
        let fold: Vec<Pos2> = curve(&g.fold).into_iter().take(carrier_bin.ceil().max(0.0) as usize + 1).collect();
        if fold.len() > 1 {
            painter.add(EguiShape::Vec(EguiShape::dashed_line(&fold, Stroke::new(1.0_f32, rgba(MAGENTA, 190.0)), 3.0, 3.0)));
        }
    }

    // The output, hanging, coloured by which sideband explains each bar.
    let mut colors = Vec::with_capacity(SPEC_BINS);
    let out_tips: Vec<f32> = state.out_db.iter().map(|&db| height(db) * half).collect();
    for k in 0..SPEC_BINS {
        let (u, d, l) = (g.up[k], g.down[k] + g.fold[k], g.leak[k]);
        let expected = u + d + l;
        let actual = amp(state.out_db[k]);
        let mut c = if expected > 1e-6 {
            let w = [u / expected, d / expected, l / expected];
            [
                CYAN[0] * w[0] + MAGENTA[0] * w[1] + TEXT[0] * w[2],
                CYAN[1] * w[0] + MAGENTA[1] * w[1] + TEXT[1] * w[2],
                CYAN[2] * w[0] + MAGENTA[2] * w[1] + TEXT[2] * w[2],
            ]
        } else {
            YELLOW
        };
        // Output the sums and differences cannot account for: intermodulation, carrier leak.
        let unexplained = if actual > expected * 1.4 { 1.0 - expected * 1.4 / actual } else { 0.0 };
        c = mix(c, YELLOW, unexplained.clamp(0.0, 1.0));
        colors.push(c);
    }
    skyline(&painter, &xs, &out_tips, mid, 1.0, &colors, 6.0, 120.0);
    outline(&painter, &xs, &out_tips, mid, 1.0, &colors, 1.0);

    // The carrier: a blade through both halves. The dim one is where the knob (or the pitch
    // tracker) puts it, the bright one is where it is right now, after Follow and Wobble.
    if s.source != Source::Sidechain {
        let target_x = x_of_hz(f.target_hz).clamp(plot.left(), plot.right());
        let live_x = x_of_hz(f.carrier_hz).clamp(plot.left(), plot.right());
        painter.line_segment([Pos2::new(target_x, plot.top()), Pos2::new(target_x, plot.bottom())], Stroke::new(1.0_f32, rgba(YELLOW, 40.0)));
        let strength = if lit { 1.0 } else { 0.8 };
        painter.line_segment([Pos2::new(live_x, plot.top() - 4.0), Pos2::new(live_x, plot.bottom())], Stroke::new(7.0_f32, rgba(YELLOW, 22.0 * strength)));
        painter.line_segment([Pos2::new(live_x, plot.top() - 4.0), Pos2::new(live_x, plot.bottom())], Stroke::new(1.6_f32, rgba(YELLOW, 235.0 * strength)));
        let top = Pos2::new(live_x, plot.top() - 4.0);
        painter.add(EguiShape::convex_polygon(
            vec![top + Vec2::new(-6.0, -8.0), top + Vec2::new(6.0, -8.0), top],
            rgba(YELLOW, 255.0 * strength),
            Stroke::NONE,
        ));
        let flip = live_x > plot.right() - 90.0;
        painter.text(
            Pos2::new(live_x + if flip { -10.0 } else { 10.0 }, plot.top() + 2.0),
            if flip { Align2::RIGHT_TOP } else { Align2::LEFT_TOP },
            format!("{:.0} Hz", f.carrier_hz),
            FontId::monospace(10.0),
            rgba(YELLOW, 220.0),
        );
    }

    if editable {
        if response.drag_started() {
            begin(params, setter, s, f);
        }
        if response.dragged() || response.drag_started() {
            if let Some(pos) = response.interact_pointer_pos() {
                let hz = ((pos.x - plot.left()) / plot.width() * f_max).clamp(0.5, f_max);
                set_carrier(params, setter, s, f, hz);
            }
        }
        if response.drag_stopped() {
            end(params, setter, s, f);
        }
        if response.double_clicked() {
            let param = carrier_param(params, s, f);
            setter.begin_set_parameter(param);
            setter.set_parameter(param, param.default_plain_value());
            setter.end_set_parameter(param);
        }
    }
}

/// The parameter dragging the display edits: Ratio when a note or the tracked pitch is the
/// reference, Freq otherwise.
fn carrier_param<'a>(params: &'a SunderParams, s: &Settings, f: &Frame) -> &'a FloatParam {
    if s.source != Source::Free && f.base_hz > 0.0 {
        &params.ratio
    } else {
        &params.freq
    }
}

fn begin(params: &SunderParams, setter: &ParamSetter, s: &Settings, f: &Frame) {
    setter.begin_set_parameter(carrier_param(params, s, f));
}

fn end(params: &SunderParams, setter: &ParamSetter, s: &Settings, f: &Frame) {
    setter.end_set_parameter(carrier_param(params, s, f));
}

fn set_carrier(params: &SunderParams, setter: &ParamSetter, s: &Settings, f: &Frame, hz: f32) {
    if s.source != Source::Free && f.base_hz > 0.0 {
        setter.set_parameter(&params.ratio, (hz / f.base_hz).clamp(0.25, 8.0));
    } else {
        setter.set_parameter(&params.freq, hz.clamp(0.5, 8000.0));
    }
}

/// The multiplication itself: input, carrier and product, captured on the carrier's phase.
fn scope(ui: &mut Ui, rect: Rect, f: &Frame) {
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 6.0, PANEL);
    let lanes = [("IN", TEXT, 0.15), ("CARRIER", YELLOW, 1.0), ("OUT", CYAN, 0.15)];
    let plot = Rect::from_min_max(Pos2::new(rect.left() + 66.0, rect.top() + 8.0), Pos2::new(rect.right() - 14.0, rect.bottom() - 8.0));
    let lane_h = plot.height() / 3.0;
    for (i, (name, color, floor)) in lanes.iter().enumerate() {
        let cy = plot.top() + lane_h * (i as f32 + 0.5);
        painter.text(Pos2::new(rect.left() + 10.0, cy), Align2::LEFT_CENTER, name, FontId::monospace(9.0), TEXT_DIM);
        painter.line_segment([Pos2::new(plot.left(), cy), Pos2::new(plot.right(), cy)], Stroke::new(1.0_f32, rgba(TEXT, 10.0)));
        // Quiet lanes are scaled up to fill, but not so far that silence looks like signal.
        let peak = f.scope[i].iter().fold(0.0_f32, |m, v| m.max(v.abs())).max(*floor);
        let scale = lane_h * 0.45 / peak;
        let points: Vec<Pos2> = (0..SCOPE)
            .map(|k| Pos2::new(plot.left() + plot.width() * k as f32 / (SCOPE - 1) as f32, cy - f.scope[i][k] * scale))
            .collect();
        painter.add(EguiShape::line(points.clone(), Stroke::new(5.0_f32, rgba(*color, 26.0))));
        painter.add(EguiShape::line(points, Stroke::new(1.4_f32, rgba(*color, 235.0))));
    }
    painter.text(Pos2::new(rect.right() - 10.0, rect.top() + 6.0), Align2::RIGHT_TOP, format!("{:.1} ms", f.scope_ms), FontId::monospace(8.0), TEXT_DIM);
}

fn knobs(ui: &mut Ui, params: &SunderParams, setter: &ParamSetter, f: &Frame) {
    let (response, painter) = ui.allocate_painter(Vec2::new(ui.available_width(), KNOB_ROW), Sense::hover());
    let row = response.rect;
    let source = params.source.value();
    let tracked = source != Source::Free && f.base_hz > 0.0;
    let external = source == Source::Sidechain;
    let diode = params.circuit.value() == Circuit::Diode;
    // Locked knobs show what is in charge instead.
    let fixed_freq = if external { Some("ext".to_string()) } else { tracked.then(|| format!("{:.1} Hz", f.carrier_hz)) };
    let fixed_ratio = (source == Source::Free || external).then(|| "--".to_string());
    let fixed_drive = (!diode && !external).then(|| "ideal".to_string());
    let cells: [(&FloatParam, &str, [f32; 3], bool, bool, Option<String>); 11] = [
        (&params.freq, "FREQ", YELLOW, !external && !tracked, false, fixed_freq),
        (&params.ratio, "RATIO", YELLOW, tracked || (source != Source::Free && !external), false, fixed_ratio),
        (&params.drive, "DRIVE", YELLOW, diode, false, fixed_drive),
        (&params.bias, "BIAS", TEXT, true, false, None),
        (&params.shift, "SIDEBAND", CYAN, true, true, None),
        (&params.follow, "FOLLOW", YELLOW, !external, true, None),
        (&params.wobble, "WOBBLE", YELLOW, !external, false, None),
        (&params.rate, "RATE", YELLOW, !external && params.wobble.value() > 0.0, false, None),
        (&params.spread, "SPREAD", CYAN, !external, false, None),
        (&params.tone, "TONE", TEXT, true, false, None),
        (&params.mix, "MIX", TEXT, true, false, None),
    ];
    let width = row.width() / cells.len() as f32;
    for (i, (param, label, color, enabled, bipolar, shown)) in cells.iter().enumerate() {
        let cell = Rect::from_min_size(Pos2::new(row.left() + i as f32 * width, row.top()), Vec2::new(width, KNOB_ROW));
        knob(ui, &painter, cell, setter, param, label, *color, *enabled, *bipolar, shown.as_deref());
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
fn knob(ui: &mut Ui, painter: &Painter, cell: Rect, setter: &ParamSetter, param: &FloatParam, label: &str, color: [f32; 3], enabled: bool, bipolar: bool, shown: Option<&str>) {
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

    painter.circle_filled(center, radius - 5.0, if lit { PANEL } else { Color32::from_rgb(0x0f, 0x10, 0x18) });
    painter.add(EguiShape::line(arc(center, radius, start, end), Stroke::new(3.0_f32, GRID)));
    let angle = start + (end - start) * norm;
    let from = if bipolar { start + (end - start) * 0.5 } else { start };
    if (angle - from).abs() > 0.01 {
        painter.add(EguiShape::line(arc(center, radius, from, angle), Stroke::new(8.0_f32, rgba(color, 30.0 * fade))));
        painter.add(EguiShape::line(arc(center, radius, from, angle), Stroke::new(3.0_f32, main)));
    }
    let dir = Vec2::new(angle.cos(), angle.sin());
    painter.line_segment([center + dir * (radius - 14.0), center + dir * (radius - 7.0)], Stroke::new(2.0_f32, rgba(TEXT, 255.0 * fade)));

    painter.text(Pos2::new(cell.center().x, cell.top() + 69.0), Align2::CENTER_CENTER, label, FontId::monospace(9.0), TEXT_DIM);
    let text = shown.map_or_else(|| param.to_string(), str::to_owned);
    painter.text(Pos2::new(cell.center().x, cell.top() + 85.0), Align2::CENTER_CENTER, text, FontId::monospace(11.0), if shown.is_some() { rgba(color, 150.0) } else { main });
}
