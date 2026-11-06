//! The editor: a transfer curve you bend by hand.
//!
//! The left square is the waveshaper itself, input on x and output on y, one glowing curve per
//! band. The bright dots are the audio: real samples from each band, so you watch them ride the
//! curve and see tape and iron smear off it. Drag up and down for drive, left and right for asym.
//! The right panel is the three bands as heat columns: drag a column up or down to push that band
//! harder, drag the glowing dividers to move the crossovers. The teal flag is the 12AX7 path: drag
//! it to choose the frequency above which the parallel tube adds its harmonics. Below the display
//! sit the tube-console controls: pentode, triode, density, transformer, air and calibration.

use nih_plug::prelude::*;
use nih_plug_egui::egui::{self, Align2, Color32, CursorIcon, FontId, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2};
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::dsp::{ceilings, shape, tube_stack, Calibration, Scope, Style, Voice, SCOPE_N};
use crate::{presets, tube_gain, KilnParams, TRIM_MAX, TRIM_MIN};

const BG: Color32 = Color32::from_rgb(0x0b, 0x09, 0x07);
const WELL: Color32 = Color32::from_rgb(0x11, 0x0e, 0x0b);
const PANEL: Color32 = Color32::from_rgb(0x1b, 0x16, 0x11);
const GRID: Color32 = Color32::from_rgb(0x2c, 0x25, 0x1d);
const TEXT_DIM: Color32 = Color32::from_rgb(0x8c, 0x7b, 0x66);
/// Bar labels sit on top of the heat fill, so they need to be lighter than `TEXT_DIM`.
const BAR_LABEL: Color32 = Color32::from_rgb(0xd0, 0xc2, 0xb0);
/// The parallel 12AX7 path, kept apart from the heat colours so it reads as its own circuit.
const TEAL: [f32; 3] = [88.0, 204.0, 216.0];
const CREAM: [f32; 3] = [255.0, 236.0, 214.0];
/// Low, mid and high: from deep red to near-white heat.
const HEAT: [[f32; 3]; 3] = [[226.0, 62.0, 34.0], [255.0, 140.0, 30.0], [255.0, 218.0, 110.0]];

const MAIN_H: f32 = 300.0;
const ROW_H: f32 = 26.0;
/// The curve plot shows -RANGE..RANGE on both axes.
const RANGE: f32 = 1.25;
const BAND_NAMES: [&str; 3] = ["LOW", "MID", "HIGH"];
const F_MIN: f32 = 20.0;
const F_MAX: f32 = 20000.0;

pub struct UiState {
    /// Band output level in dB, fast attack and slow release.
    level_db: [f32; 3],
}

impl Default for UiState {
    fn default() -> Self {
        Self { level_db: [-120.0; 3] }
    }
}

pub fn background() -> Color32 {
    BG
}

fn rgba(c: [f32; 3], a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c[0] as u8, c[1] as u8, c[2] as u8, a as u8)
}

fn text(painter: &egui::Painter, pos: Pos2, align: Align2, s: impl ToString, size: f32, color: Color32) {
    painter.text(pos, align, s.to_string(), FontId::monospace(size), color);
}

fn reset<P: Param>(setter: &ParamSetter, param: &P) {
    setter.begin_set_parameter(param);
    setter.set_parameter(param, param.default_plain_value());
    setter.end_set_parameter(param);
}

pub fn draw(ui: &mut Ui, params: &Arc<KilnParams>, setter: &ParamSetter, scope: &Arc<Scope>, state: &mut UiState) {
    ui.ctx().request_repaint();
    for (level, source) in state.level_db.iter_mut().zip(&scope.level) {
        let db = 20.0 * source.load(Ordering::Relaxed).max(1e-6).log10();
        *level = if db > *level { db } else { (*level - 1.2).max(db) };
    }

    header(ui, params, setter);
    ui.add_space(8.0);

    let width = ui.available_width();
    let (main, _) = ui.allocate_exact_size(Vec2::new(width, MAIN_H), Sense::hover());
    let half = (width - 12.0) / 2.0;
    let left = Rect::from_min_size(main.min, Vec2::new(half, MAIN_H));
    let right = Rect::from_min_size(Pos2::new(main.right() - half, main.top()), Vec2::new(half, MAIN_H));
    curve(ui, left, params, setter, scope);
    bands(ui, right, params, setter, state);
    ui.add_space(10.0);

    let gap = 12.0;
    // `None` in a slot means the cell is filled by hand below.
    let rows: [[Option<(&FloatParam, &str, [f32; 3], bool)>; 2]; 7] = [
        [bar_cell(&params.drive, "DRIVE", HEAT[1], false), bar_cell(&params.mix, "MIX", HEAT[2], false)],
        [bar_cell(&params.asym, "ASYM", HEAT[1], true), bar_cell(&params.output, "OUTPUT", HEAT[2], false)],
        [bar_cell(&params.dynamics, "DYNAMICS", HEAT[1], true), bar_cell(&params.tone, "TONE", HEAT[2], true)],
        [bar_cell(&params.pentode, "PENTODE", HEAT[0], false), bar_cell(&params.triode, "TRIODE", HEAT[1], false)],
        [bar_cell(&params.density, "DENSITY", HEAT[1], false), bar_cell(&params.xfmr, "TRANSFORMER", HEAT[0], false)],
        [bar_cell(&params.sat, "12AX7 SAT", TEAL, false), bar_cell(&params.sat_freq, "SAT FREQ", TEAL, false)],
        [bar_cell(&params.air, "AIR", HEAT[2], false), None],
    ];
    for row in rows {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, ROW_H), Sense::hover());
        let w = (width - gap) / 2.0;
        for (i, cell_def) in row.into_iter().enumerate() {
            let x = rect.left() + i as f32 * (w + gap);
            let cell = Rect::from_min_size(Pos2::new(x, rect.top()), Vec2::new(w, ROW_H));
            match cell_def {
                Some((param, label, color, bipolar)) => bar(ui, cell, setter, param, label, rgba(color, 255.0), bipolar),
                None => segmented(
                    ui,
                    cell,
                    setter,
                    &params.calibration,
                    "CAL",
                    &[("DARK", Calibration::Dark), ("NORMAL", Calibration::Normal), ("BRIGHT", Calibration::Bright)],
                    HEAT[1],
                ),
            }
        }
        ui.add_space(6.0);
    }
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, ROW_H), Sense::hover());
    let w = (width - gap) / 2.0;
    let voice_cell = Rect::from_min_size(rect.min, Vec2::new(w, ROW_H));
    let match_cell = Rect::from_min_size(Pos2::new(rect.left() + w + gap, rect.top()), Vec2::new(w, ROW_H));
    segmented(ui, voice_cell, setter, &params.voice, "12AX7", &[("WARM", Voice::Warm), ("HOT", Voice::Hot)], TEAL);
    toggle(ui, match_cell, setter, &params.gain_match, "GAIN MATCH");
}

fn bar_cell<'a>(param: &'a FloatParam, label: &'a str, color: [f32; 3], bipolar: bool) -> Option<(&'a FloatParam, &'a str, [f32; 3], bool)> {
    Some((param, label, color, bipolar))
}

fn header(ui: &mut Ui, params: &Arc<KilnParams>, setter: &ParamSetter) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.0), Sense::hover());
    let painter = ui.painter_at(rect);
    text(&painter, rect.left_center(), Align2::LEFT_CENTER, "KILN", 11.0, TEXT_DIM);
    let theme = preset_kit::ui::Theme { fill: PANEL, hover: GRID, ink: rgba(CREAM, 255.0), dim: TEXT_DIM };
    preset_kit::ui::picker(ui, Pos2::new(rect.left() + 56.0, rect.top()), rect.height(), &presets::BANK, &**params, setter, &theme);

    let (w, gap) = (62.0, 4.0);
    let current = params.style.value();
    for (i, style) in Style::ALL.into_iter().enumerate() {
        let x = rect.right() - (4 - i) as f32 * (w + gap) + gap;
        let cell = Rect::from_min_size(Pos2::new(x, rect.top()), Vec2::new(w, rect.height()));
        let r = ui.interact(cell, ui.id().with(("style", i)), Sense::click());
        let on = style == current;
        let fill = if on { rgba(HEAT[1], 255.0) } else if r.hovered() { GRID } else { PANEL };
        painter.rect_filled(cell, 4.0, fill);
        let ink = if on { BG } else { TEXT_DIM };
        text(&painter, cell.center(), Align2::CENTER_CENTER, style.name(), 10.0, ink);
        if r.clicked() {
            setter.begin_set_parameter(&params.style);
            setter.set_parameter(&params.style, style);
            setter.end_set_parameter(&params.style);
        }
    }
}

fn curve(ui: &mut Ui, rect: Rect, params: &Arc<KilnParams>, setter: &ParamSetter, scope: &Arc<Scope>) {
    let response = ui.interact(rect, ui.id().with("curve"), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 6.0, WELL);

    let side = rect.width().min(rect.height()) / 2.0 - 4.0;
    let center = rect.center();
    let at = |x: f32, y: f32| Pos2::new(center.x + x / RANGE * side, center.y - y / RANGE * side);

    for v in [-1.0, -0.5, 0.5, 1.0] {
        painter.line_segment([at(v, -RANGE), at(v, RANGE)], Stroke::new(1.0_f32, GRID));
        painter.line_segment([at(-RANGE, v), at(RANGE, v)], Stroke::new(1.0_f32, GRID));
    }
    let axis = Stroke::new(1.0_f32, rgba(CREAM, 50.0));
    painter.line_segment([at(0.0, -RANGE), at(0.0, RANGE)], axis);
    painter.line_segment([at(-RANGE, 0.0), at(RANGE, 0.0)], axis);
    // Straight line: what a clean signal would do.
    painter.line_segment([at(-RANGE, -RANGE), at(RANGE, RANGE)], Stroke::new(1.0_f32, rgba(CREAM, 40.0)));

    let style = params.style.value();
    let (ceil_pos, ceil_neg) = ceilings(style, params.asym.value());
    let tubes = [tube_gain(params.pentode.value(), params.density.value()), tube_gain(params.triode.value(), params.density.value())];
    const STEPS: usize = 140;
    for b in 0..3 {
        let g = 10.0f32.powf(params.drive_db(b) / 20.0);
        let makeup = 1.0 / g.min(1.0);
        let points: Vec<Pos2> = (0..=STEPS)
            .map(|i| {
                let x = -RANGE + 2.0 * RANGE * i as f32 / STEPS as f32;
                at(x, shape(style, g * tube_stack(x, tubes), ceil_pos, ceil_neg) * makeup)
            })
            .collect();
        painter.add(Shape::line(points.clone(), Stroke::new(7.0_f32, rgba(HEAT[b], 26.0))));
        painter.add(Shape::line(points, Stroke::new(1.8_f32, rgba(HEAT[b], 235.0))));
    }

    for b in 0..3 {
        for i in 0..SCOPE_N {
            let x = scope.input[b][i].load(Ordering::Relaxed);
            let y = scope.output[b][i].load(Ordering::Relaxed);
            if x == 0.0 && y == 0.0 {
                continue;
            }
            let p = at(x, y);
            painter.circle_filled(p, 3.4, rgba(HEAT[b], 22.0));
            painter.circle_filled(p, 1.3, rgba(HEAT[b], 200.0));
        }
    }

    text(&painter, rect.left_top() + Vec2::new(10.0, 8.0), Align2::LEFT_TOP, "TRANSFER", 10.0, TEXT_DIM);
    text(&painter, rect.right_top() + Vec2::new(-10.0, 8.0), Align2::RIGHT_TOP, params.drive.to_string(), 14.0, rgba(CREAM, 255.0));
    text(
        &painter,
        rect.center_bottom() + Vec2::new(0.0, -8.0),
        Align2::CENTER_BOTTOM,
        "drag up/down: drive   left/right: asym",
        9.0,
        TEXT_DIM,
    );

    if response.hovered() || response.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::Grab);
    }
    if response.drag_started() {
        setter.begin_set_parameter(&params.drive);
        setter.begin_set_parameter(&params.asym);
    }
    if response.dragged() {
        let d = response.drag_delta();
        let drive = params.drive.unmodulated_normalized_value() - d.y / (side * 1.6);
        let asym = params.asym.unmodulated_normalized_value() + d.x / (side * 2.4);
        setter.set_parameter_normalized(&params.drive, drive.clamp(0.0, 1.0));
        setter.set_parameter_normalized(&params.asym, asym.clamp(0.0, 1.0));
    }
    if response.drag_stopped() {
        setter.end_set_parameter(&params.drive);
        setter.end_set_parameter(&params.asym);
    }
    if response.double_clicked() {
        reset(setter, &params.drive);
        reset(setter, &params.asym);
    }
}

fn bands(ui: &mut Ui, rect: Rect, params: &Arc<KilnParams>, setter: &ParamSetter, state: &UiState) {
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 6.0, WELL);

    let x_of = |f: f32| rect.left() + (f / F_MIN).ln() / (F_MAX / F_MIN).ln() * rect.width();
    let f_of = |x: f32| F_MIN * (F_MAX / F_MIN).powf(((x - rect.left()) / rect.width()).clamp(0.0, 1.0));
    let crossovers = [&params.xover_low, &params.xover_high];
    let edges = [rect.left(), x_of(crossovers[0].value()), x_of(crossovers[1].value()), rect.right()];
    let bottom = rect.bottom() - 26.0;
    let usable = bottom - rect.top() - 44.0;

    for b in 0..3 {
        let zone = Rect::from_min_max(Pos2::new(edges[b], rect.top()), Pos2::new(edges[b + 1], rect.bottom()));
        let trim = params.trim(b);
        let r = ui.interact(zone, ui.id().with(("zone", b)), Sense::click_and_drag());
        if r.drag_started() {
            setter.begin_set_parameter(trim);
        }
        if r.dragged() {
            let db = trim.value() - r.drag_delta().y / usable * 72.0;
            setter.set_parameter(trim, db.clamp(TRIM_MIN, TRIM_MAX));
        }
        if r.drag_stopped() {
            setter.end_set_parameter(trim);
        }
        if r.double_clicked() {
            reset(setter, trim);
        }
        if r.hovered() || r.dragged() {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeVertical);
        }

        // The column glows with the band's output level and fills to its effective drive.
        let heat = ((state.level_db[b] + 54.0) / 54.0).clamp(0.0, 1.0);
        painter.rect_filled(zone, 0.0, rgba(HEAT[b], 6.0 + heat * 34.0));
        let drive = (params.drive_db(b) + 24.0) / 72.0;
        let top = bottom - drive * usable;
        let fill = Rect::from_min_max(Pos2::new(zone.left() + 1.0, top), Pos2::new(zone.right() - 1.0, bottom));
        painter.rect_filled(fill, 0.0, rgba(HEAT[b], 55.0 + heat * 110.0));
        painter.line_segment([fill.left_top(), fill.right_top()], Stroke::new(5.0_f32, rgba(HEAT[b], 40.0)));
        painter.line_segment([fill.left_top(), fill.right_top()], Stroke::new(2.0_f32, rgba(HEAT[b], 255.0)));

        let cx = zone.center().x;
        text(&painter, Pos2::new(cx, rect.top() + 10.0), Align2::CENTER_TOP, BAND_NAMES[b], 10.0, TEXT_DIM);
        text(
            &painter,
            Pos2::new(cx, rect.top() + 24.0),
            Align2::CENTER_TOP,
            format!("{:+.0}", params.drive_db(b)),
            16.0,
            rgba(CREAM, 255.0),
        );
    }
    painter.line_segment([Pos2::new(rect.left(), bottom), Pos2::new(rect.right(), bottom)], Stroke::new(1.0_f32, GRID));

    // Crossover dividers, registered last so they win the overlap with the columns.
    for (i, param) in crossovers.into_iter().enumerate() {
        let x = edges[i + 1];
        let hit = Rect::from_min_max(Pos2::new(x - 8.0, rect.top()), Pos2::new(x + 8.0, rect.bottom()));
        let r = ui.interact(hit, ui.id().with(("crossover", i)), Sense::click_and_drag());
        let active = r.hovered() || r.dragged();
        if active {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
        }
        if r.drag_started() {
            setter.begin_set_parameter(param);
        }
        if r.dragged() {
            if let Some(pos) = r.interact_pointer_pos() {
                setter.set_parameter_normalized(param, param.preview_normalized(f_of(pos.x)));
            }
        }
        if r.drag_stopped() {
            setter.end_set_parameter(param);
        }
        if r.double_clicked() {
            reset(setter, param);
        }

        let glow = if active { 1.0 } else { 0.55 };
        painter.line_segment([Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())], Stroke::new(5.0_f32, rgba(CREAM, 20.0 * glow)));
        painter.line_segment([Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())], Stroke::new(1.5_f32, rgba(CREAM, 150.0 * glow)));
        let grip = Rect::from_center_size(Pos2::new(x, bottom - usable * 0.5), Vec2::new(6.0, 26.0));
        painter.rect_filled(grip, 3.0, rgba(CREAM, 70.0 + 120.0 * glow));
        let f = param.value();
        let label = if f >= 1000.0 { format!("{:.1}k", f / 1000.0) } else { format!("{:.0}", f) };
        text(&painter, Pos2::new(x, rect.bottom() - 12.0), Align2::CENTER_CENTER, label, 10.0, rgba(CREAM, 255.0));
    }

    // The 12AX7 flag, registered after everything else. Only its head grabs, so it never steals a crossover.
    let sat = &params.sat_freq;
    let x = x_of(sat.value());
    let head = Rect::from_center_size(Pos2::new(x, rect.top() + 7.0), Vec2::new(18.0, 14.0));
    let r = ui.interact(head, ui.id().with("satfreq"), Sense::click_and_drag());
    let active = r.hovered() || r.dragged();
    if active {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
    }
    if r.drag_started() {
        setter.begin_set_parameter(sat);
    }
    if r.dragged() {
        if let Some(pos) = r.interact_pointer_pos() {
            setter.set_parameter_normalized(sat, sat.preview_normalized(f_of(pos.x)));
        }
    }
    if r.drag_stopped() {
        setter.end_set_parameter(sat);
    }
    if r.double_clicked() {
        reset(setter, sat);
    }
    // Brighter the more of the path is blended in; everything right of the flag is where it works.
    let live = 0.3 + 0.7 * params.sat.value() + if active { 0.2 } else { 0.0 };
    painter.rect_filled(Rect::from_min_max(Pos2::new(x, rect.top()), Pos2::new(rect.right(), bottom)), 0.0, rgba(TEAL, 10.0 * live));
    let mut y = rect.top() + 14.0;
    while y < bottom {
        painter.line_segment([Pos2::new(x, y), Pos2::new(x, (y + 4.0).min(bottom))], Stroke::new(1.2_f32, rgba(TEAL, 200.0 * live.min(1.0))));
        y += 8.0;
    }
    painter.add(Shape::convex_polygon(
        vec![Pos2::new(x - 6.0, rect.top()), Pos2::new(x + 6.0, rect.top()), Pos2::new(x, rect.top() + 11.0)],
        rgba(TEAL, 230.0 * live.min(1.0)),
        Stroke::NONE,
    ));
    let f = sat.value();
    let label = if f >= 1000.0 { format!("12AX7 {:.1}k", f / 1000.0) } else { format!("12AX7 {:.0}", f) };
    let (align, dx) = if x > rect.right() - 80.0 { (Align2::RIGHT_BOTTOM, -5.0) } else { (Align2::LEFT_BOTTOM, 5.0) };
    text(&painter, Pos2::new(x + dx, bottom - 4.0), align, label, 9.0, rgba(TEAL, 255.0));
}

/// A horizontal bar fader: drag to set, double-click to reset. Bipolar bars fill from the middle.
fn bar(ui: &mut Ui, rect: Rect, setter: &ParamSetter, param: &FloatParam, label: &str, color: Color32, bipolar: bool) {
    let response = ui.interact(rect, ui.id().with(label), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    let norm = param.unmodulated_normalized_value();

    painter.rect_filled(rect, 4.0, PANEL);
    let from: f32 = if bipolar { 0.5 } else { 0.0 };
    let (lo, hi) = (from.min(norm), from.max(norm));
    let fill = Rect::from_min_max(
        Pos2::new(rect.left() + rect.width() * lo, rect.top()),
        Pos2::new(rect.left() + rect.width() * hi, rect.bottom()),
    );
    let [r, g, b, _] = color.to_array();
    painter.rect_filled(fill, 4.0, Color32::from_rgba_unmultiplied(r, g, b, 36));
    let edge = rect.left() + rect.width() * norm;
    painter.rect_filled(Rect::from_min_max(Pos2::new(edge - 1.5, rect.top()), Pos2::new(edge + 1.5, rect.bottom())), 1.5, color);
    if bipolar {
        let mid = rect.center().x;
        painter.line_segment([Pos2::new(mid, rect.top() + 5.0), Pos2::new(mid, rect.bottom() - 5.0)], Stroke::new(1.0_f32, GRID));
    }
    text(&painter, Pos2::new(rect.left() + 8.0, rect.center().y), Align2::LEFT_CENTER, label, 11.0, BAR_LABEL);
    text(&painter, Pos2::new(rect.right() - 8.0, rect.center().y), Align2::RIGHT_CENTER, param.to_string(), 12.0, rgba(CREAM, 255.0));
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
        reset(setter, param);
    }
}

fn toggle(ui: &mut Ui, rect: Rect, setter: &ParamSetter, param: &BoolParam, label: &str) {
    let response = ui.interact(rect, ui.id().with(label), Sense::click());
    let painter = ui.painter_at(rect);
    let on = param.value();
    painter.rect_filled(rect, 4.0, if on { rgba(HEAT[1], 28.0) } else { PANEL });
    painter.rect_stroke(rect, 4.0, Stroke::new(1.0_f32, if on { rgba(HEAT[1], 160.0) } else { GRID }), egui::StrokeKind::Inside);
    text(&painter, Pos2::new(rect.left() + 8.0, rect.center().y), Align2::LEFT_CENTER, label, 11.0, BAR_LABEL);
    let (state, ink) = if on { ("ON", rgba(HEAT[1], 255.0)) } else { ("OFF", TEXT_DIM) };
    text(&painter, Pos2::new(rect.right() - 8.0, rect.center().y), Align2::RIGHT_CENTER, state, 11.0, ink);
    if response.clicked() {
        setter.begin_set_parameter(param);
        setter.set_parameter(param, !on);
        setter.end_set_parameter(param);
    }
}

/// A row of mutually exclusive buttons for an enum parameter, under a small label.
fn segmented<E: Enum + PartialEq + Copy + 'static>(
    ui: &mut Ui,
    rect: Rect,
    setter: &ParamSetter,
    param: &EnumParam<E>,
    label: &str,
    options: &[(&str, E)],
    color: [f32; 3],
) {
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, PANEL);
    painter.rect_stroke(rect, 4.0, Stroke::new(1.0_f32, GRID), egui::StrokeKind::Inside);
    text(&painter, Pos2::new(rect.left() + 8.0, rect.center().y), Align2::LEFT_CENTER, label, 11.0, BAR_LABEL);
    let current = param.value();
    let (w, gap) = (62.0, 3.0);
    for (i, (name, value)) in options.iter().enumerate() {
        let x = rect.right() - 3.0 - (options.len() - i) as f32 * (w + gap) + gap;
        let cell = Rect::from_min_size(Pos2::new(x, rect.top() + 3.0), Vec2::new(w, rect.height() - 6.0));
        let r = ui.interact(cell, ui.id().with((label, i)), Sense::click());
        let on = *value == current;
        let fill = if on { rgba(color, 255.0) } else if r.hovered() { GRID } else { WELL };
        painter.rect_filled(cell, 3.0, fill);
        text(&painter, cell.center(), Align2::CENTER_CENTER, name, 10.0, if on { BG } else { TEXT_DIM });
        if r.clicked() {
            setter.begin_set_parameter(param);
            setter.set_parameter(param, *value);
            setter.end_set_parameter(param);
        }
    }
}
