//! The editor: a rotary pitch dial and a scrolling pitch tape.
//!
//! The dial is one octave bent into a circle. Each of the twelve notes is a key that is lit when
//! the plugin may snap to it; click one to switch it on or off. A thin grey needle shows the pitch
//! being sung and a bold orange needle shows the pitch coming out, so the retune speed can be seen
//! as the lag between them. The tape beside it is the same story over the last few seconds.

use nih_plug::prelude::*;
use nih_plug_egui::egui::{
    self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Shape, Stroke, Ui, Vec2,
};
use std::f32::consts::{PI, TAU};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;

use crate::scale::{self, Scale, NOTE_NAMES};
use crate::{DetentParams, Key, Target, Telemetry};

const BG: Color32 = Color32::from_rgb(0x10, 0x10, 0x12);
const PANEL: Color32 = Color32::from_rgb(0x1b, 0x1b, 0x1f);
const KEY_OFF: Color32 = Color32::from_rgb(0x26, 0x26, 0x2c);
const GRID: Color32 = Color32::from_rgb(0x30, 0x30, 0x38);
const TEXT_DIM: Color32 = Color32::from_rgb(0x72, 0x72, 0x7e);
const LINE: Color32 = Color32::from_rgb(0xea, 0xea, 0xee);
const RAW: Color32 = Color32::from_rgb(0x8a, 0x8a, 0x96);
const ACCENT: [u8; 3] = [0xff, 0x5a, 0x1f];
const CYAN: [u8; 3] = [0x2e, 0xe6, 0xd6];

const HISTORY: usize = 360;
/// Semitones shown above and below the centre of the tape.
const TAPE_SPAN: f32 = 8.0;

fn accent(a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(ACCENT[0], ACCENT[1], ACCENT[2], a)
}

fn cyan(a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(CYAN[0], CYAN[1], CYAN[2], a)
}

pub fn background() -> Color32 {
    BG
}

pub struct UiState {
    /// Ring of (input, target, output) MIDI pitches. NaN while unvoiced.
    history: Vec<[f32; 3]>,
    head: usize,
    center: f32,
    last: [f32; 3],
    /// Learn: while listening, a histogram of the pitch classes being sung.
    learning: bool,
    learn_hist: [f32; 12],
    learn_frames: u32,
    learn_message: String,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            history: vec![[f32::NAN; 3]; HISTORY],
            head: 0,
            center: 60.0,
            last: [f32::NAN; 3],
            learning: false,
            learn_hist: [0.0; 12],
            learn_frames: 0,
            learn_message: String::new(),
        }
    }
}

fn note_name(midi: f32) -> String {
    let n = midi.round() as i32;
    format!("{}{}", NOTE_NAMES[n.rem_euclid(12) as usize], n.div_euclid(12) - 1)
}

fn pc_angle(pitch_class: f32) -> f32 {
    -PI / 2.0 + pitch_class / 12.0 * TAU
}

pub fn draw(ui: &mut Ui, params: &Arc<DetentParams>, setter: &ParamSetter, telemetry: &Arc<Telemetry>, state: &mut UiState) {
    ui.ctx().request_repaint();

    let voiced = telemetry.voiced.load(Relaxed);
    let frame = if voiced {
        [telemetry.input_midi.load(Relaxed), telemetry.target_midi.load(Relaxed), telemetry.output_midi.load(Relaxed)]
    } else {
        [f32::NAN; 3]
    };
    state.history[state.head] = frame;
    state.head = (state.head + 1) % HISTORY;
    if voiced {
        state.last = frame;
        state.center += (frame[2] - state.center) * 0.04;
        if state.learning {
            scale::add_to_histogram(&mut state.learn_hist, frame[0]);
            state.learn_frames += 1;
        }
    }

    let midi_mode = params.target.value() == Target::Midi;
    let mask = if midi_mode {
        telemetry.held_mask.load(Relaxed) as u16
    } else {
        let flips: [bool; 12] = std::array::from_fn(|i| params.notes[i].flip.value());
        scale::mask(params.key.value().to_index(), params.scale.value(), flips)
    };
    let key_pc = params.key.value().to_index();

    ui.label(RichText::new("DETENT").monospace().size(11.0).color(TEXT_DIM));
    ui.add_space(6.0);

    let controls_height = 88.0 + 12.0 + 28.0 + 6.0 + 28.0;
    let top_height = (ui.available_height() - controls_height - 12.0).max(120.0);
    let (area, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), top_height), Sense::hover());
    let dial_area = Rect::from_min_size(area.min, Vec2::splat(top_height));
    let tape_area = Rect::from_min_max(Pos2::new(dial_area.right() + 14.0, area.top()), area.max);

    dial(ui, dial_area, params, setter, mask, key_pc, state, voiced);
    tape(ui, tape_area, mask, state);

    ui.add_space(12.0);
    ui.columns(2, |cols| {
        bar(&mut cols[0], setter, &params.retune, "RETUNE");
        cols[0].add_space(5.0);
        bar(&mut cols[0], setter, &params.humanize, "HUMANIZE");
        cols[0].add_space(5.0);
        bar(&mut cols[0], setter, &params.formant, "FORMANT");
        bar(&mut cols[1], setter, &params.amount, "AMOUNT");
        cols[1].add_space(5.0);
        bar(&mut cols[1], setter, &params.detune, "DETUNE");
        cols[1].add_space(5.0);
        let fs = telemetry.sample_rate.load(Relaxed).max(1.0);
        let ms = telemetry.latency_samples.load(Relaxed) / fs * 1000.0;
        readout(&mut cols[1], "LATENCY", &format!("{ms:.0} ms"));
    });
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        stepper(ui, setter, &params.key, "KEY", 40.0);
        ui.add_space(10.0);
        stepper(ui, setter, &params.scale, "SCALE", 130.0);
        ui.add_space(10.0);
        segmented(ui, setter, &params.range, "RANGE");
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        segmented(ui, setter, &params.target, "TARGET");
        ui.add_space(10.0);
        segmented(ui, setter, &params.mode, "MODE");
        ui.add_space(10.0);
        learn_button(ui, setter, params, state);
    });
}

/// A read-only panel in the style of the bars.
fn readout(ui: &mut Ui, label: &str, value: &str) {
    let (response, painter) = ui.allocate_painter(Vec2::new(ui.available_width(), 26.0), Sense::hover());
    let rect = response.rect;
    painter.rect_filled(rect, 4.0, PANEL);
    painter.rect_stroke(rect, 4.0, Stroke::new(1.0_f32, GRID), egui::StrokeKind::Inside);
    painter.text(Pos2::new(rect.left() + 8.0, rect.center().y), Align2::LEFT_CENTER, label, FontId::monospace(10.0), TEXT_DIM);
    painter.text(Pos2::new(rect.right() - 8.0, rect.center().y), Align2::RIGHT_CENTER, value, FontId::monospace(11.0), LINE);
}

/// Learn: press, sing for a while, press again. The notes sung decide the key and scale.
fn learn_button(ui: &mut Ui, setter: &ParamSetter, params: &Arc<DetentParams>, state: &mut UiState) {
    let pulse = (ui.input(|i| i.time) * 4.0).sin() as f32 * 0.5 + 0.5;
    let (text, fill, color) = if state.learning {
        ("LISTENING  - press to finish", cyan(60 + (pulse * 120.0) as u8), LINE)
    } else {
        ("LEARN KEY", PANEL, LINE)
    };
    let button = egui::Button::new(RichText::new(text).monospace().size(11.0).color(color))
        .fill(fill)
        .stroke(Stroke::new(1.0_f32, if state.learning { cyan(255) } else { GRID }))
        .min_size(Vec2::new(110.0, 22.0));
    if ui.add(button).clicked() {
        if !state.learning {
            state.learning = true;
            state.learn_hist = [0.0; 12];
            state.learn_frames = 0;
            state.learn_message.clear();
        } else {
            state.learning = false;
            // About a second of voiced singing is the least that says anything about a key.
            match scale::detect_key(&state.learn_hist).filter(|_| state.learn_frames >= 60) {
                Some((key, scale, _)) => {
                    setter.begin_set_parameter(&params.key);
                    setter.set_parameter(&params.key, Key::from_index(key));
                    setter.end_set_parameter(&params.key);
                    setter.begin_set_parameter(&params.scale);
                    setter.set_parameter(&params.scale, scale);
                    setter.end_set_parameter(&params.scale);
                    setter.begin_set_parameter(&params.target);
                    setter.set_parameter(&params.target, Target::Scale);
                    setter.end_set_parameter(&params.target);
                    // Start from the plain scale: clear any hand-edited notes.
                    for note in &params.notes {
                        if note.flip.value() {
                            setter.begin_set_parameter(&note.flip);
                            setter.set_parameter(&note.flip, false);
                            setter.end_set_parameter(&note.flip);
                        }
                    }
                    let mode = if scale == Scale::Major { "major" } else { "minor" };
                    state.learn_message = format!("heard {} {mode}", NOTE_NAMES[key]);
                }
                None => state.learn_message = "not enough singing heard".to_string(),
            }
        }
    }
    let message = if state.learning {
        format!("{:.1}s", state.learn_frames as f32 / 60.0)
    } else {
        state.learn_message.clone()
    };
    ui.label(RichText::new(message).monospace().size(10.0).color(TEXT_DIM));
}

#[allow(clippy::too_many_arguments)]
fn dial(
    ui: &mut Ui,
    area: Rect,
    params: &Arc<DetentParams>,
    setter: &ParamSetter,
    mask: u16,
    key_pc: usize,
    state: &UiState,
    voiced: bool,
) {
    let response = ui.interact(area, ui.id().with("dial"), Sense::click());
    let painter = ui.painter_at(area);
    let c = area.center();
    let radius = area.width().min(area.height()) * 0.5 - 2.0;
    let (r_in, r_out) = (radius * 0.60, radius * 0.97);

    painter.circle_filled(c, radius, PANEL);
    painter.circle_stroke(c, radius, Stroke::new(1.0_f32, GRID));

    let [raw, target, out] = state.last;
    let target_pc = if voiced { target.rem_euclid(12.0).round() as usize % 12 } else { usize::MAX };

    // The twelve keys.
    for pc in 0..12 {
        let centre = pc_angle(pc as f32);
        let half = 0.245;
        let mut pts = Vec::with_capacity(14);
        for k in 0..=6 {
            let a = centre - half + 2.0 * half * k as f32 / 6.0;
            pts.push(c + Vec2::angled(a) * r_out);
        }
        for k in (0..=6).rev() {
            let a = centre - half + 2.0 * half * k as f32 / 6.0;
            pts.push(c + Vec2::angled(a) * r_in);
        }
        let allowed = mask & (1 << pc) != 0;
        let hit = pc == target_pc;
        let fill = match (allowed, hit) {
            (true, true) => accent(255),
            (true, false) => accent(70),
            _ => KEY_OFF,
        };
        let stroke = if pc == key_pc { Stroke::new(2.0_f32, LINE) } else { Stroke::new(1.0_f32, GRID) };
        painter.add(Shape::convex_polygon(pts, fill, stroke));

        let label = c + Vec2::angled(centre) * (r_in + r_out) * 0.5;
        let color = if hit { BG } else if allowed { LINE } else { TEXT_DIM };
        painter.text(label, Align2::CENTER_CENTER, NOTE_NAMES[pc], FontId::monospace(if NOTE_NAMES[pc].len() > 1 { 10.0 } else { 13.0 }), color);
    }

    if response.clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            let v = pos - c;
            if v.length() > r_in * 0.9 && v.length() < radius {
                let turns = (v.y.atan2(v.x) + PI / 2.0).rem_euclid(TAU) / TAU;
                let pc = (turns * 12.0).round() as usize % 12;
                let flip = &params.notes[pc].flip;
                setter.begin_set_parameter(flip);
                setter.set_parameter(flip, !flip.value());
                setter.end_set_parameter(flip);
            }
        }
    }

    // While learning, show how often each note has been sung as petals inside the ring.
    if state.learning {
        let peak = state.learn_hist.iter().cloned().fold(0.0, f32::max).max(1.0);
        for (pc, count) in state.learn_hist.iter().enumerate() {
            let angle = pc_angle(pc as f32);
            let len = radius * 0.17 + (r_in - 6.0 - radius * 0.17) * count / peak;
            painter.line_segment([c + Vec2::angled(angle) * (radius * 0.17), c + Vec2::angled(angle) * len], Stroke::new(9.0_f32, cyan(150)));
        }
    }

    // Needles: grey for what is sung, orange for what comes out.
    if voiced {
        let raw_angle = pc_angle(raw.rem_euclid(12.0));
        let out_angle = pc_angle(out.rem_euclid(12.0));
        let raw_tip = c + Vec2::angled(raw_angle) * (r_in - 6.0);
        painter.line_segment([c + Vec2::angled(raw_angle) * (radius * 0.16), raw_tip], Stroke::new(1.5_f32, RAW));
        painter.circle_stroke(raw_tip, 4.0, Stroke::new(1.5_f32, RAW));

        let tip = c + Vec2::angled(out_angle) * (r_in - 3.0);
        painter.line_segment([c, tip], Stroke::new(6.0_f32, accent(40)));
        painter.line_segment([c, tip], Stroke::new(3.0_f32, accent(255)));
        painter.circle_filled(tip, 4.5, Color32::WHITE);
    }
    painter.circle_filled(c, radius * 0.13, BG);
    painter.circle_stroke(c, radius * 0.13, Stroke::new(1.0_f32, GRID));

    // Readout.
    let (name, detail) = if voiced {
        (note_name(out), format!("{:+.0}c in", (raw - target) * 100.0))
    } else {
        ("--".to_string(), "no pitch".to_string())
    };
    painter.text(c + Vec2::new(0.0, radius * 0.32), Align2::CENTER_CENTER, name, FontId::monospace(radius * 0.17), if voiced { LINE } else { TEXT_DIM });
    painter.text(c + Vec2::new(0.0, radius * 0.45), Align2::CENTER_CENTER, detail, FontId::monospace(9.0), TEXT_DIM);
    painter.circle_filled(c + Vec2::new(0.0, -radius * 0.34), 3.5, if voiced { accent(255) } else { GRID });
}

fn tape(ui: &mut Ui, area: Rect, mask: u16, state: &UiState) {
    let painter = ui.painter_at(area);
    painter.rect_filled(area, 6.0, PANEL);

    let half_h = area.height() * 0.5;
    let y_of = |m: f32| area.center().y - (m - state.center) / TAPE_SPAN * half_h * 0.96;
    let x_of = |i: usize| area.left() + 6.0 + i as f32 / (HISTORY - 1) as f32 * (area.width() - 12.0);

    // Note lanes: lit where the plugin may land.
    let lo = (state.center - TAPE_SPAN - 1.0).floor() as i32;
    let hi = (state.center + TAPE_SPAN + 1.0).ceil() as i32;
    for n in lo..=hi {
        let y = y_of(n as f32);
        if y < area.top() + 2.0 || y > area.bottom() - 2.0 {
            continue;
        }
        let allowed = mask & (1 << n.rem_euclid(12)) != 0;
        let (stroke, color) = if allowed { (Stroke::new(1.5_f32, accent(60)), LINE) } else { (Stroke::new(1.0_f32, GRID), TEXT_DIM) };
        painter.line_segment([Pos2::new(area.left() + 4.0, y), Pos2::new(area.right() - 4.0, y)], stroke);
        if allowed {
            painter.text(Pos2::new(area.left() + 8.0, y - 1.0), Align2::LEFT_BOTTOM, note_name(n as f32), FontId::monospace(9.0), color);
        }
    }

    // Ordered oldest to newest.
    let frames = |channel: usize| -> Vec<(usize, f32)> {
        (0..HISTORY).map(|i| (i, state.history[(state.head + i) % HISTORY][channel])).collect()
    };
    let polyline = |channel: usize, width: f32, color: Color32| {
        let mut run: Vec<Pos2> = Vec::new();
        let flush = |run: &mut Vec<Pos2>| {
            if run.len() >= 2 {
                painter.add(Shape::line(std::mem::take(run), Stroke::new(width, color)));
            } else {
                run.clear();
            }
        };
        for (i, m) in frames(channel) {
            if m.is_nan() {
                flush(&mut run);
            } else {
                run.push(Pos2::new(x_of(i), y_of(m)));
            }
        }
        flush(&mut run);
    };
    polyline(1, 1.0, cyan(150));
    polyline(0, 1.5, RAW);
    polyline(2, 5.0, accent(40));
    polyline(2, 2.5, accent(255));

    painter.text(Pos2::new(area.right() - 8.0, area.top() + 6.0), Align2::RIGHT_TOP, "IN  OUT  TARGET", FontId::monospace(9.0), TEXT_DIM);
}

/// A horizontal bar fader for a float parameter: drag to set, double-click to reset.
fn bar(ui: &mut Ui, setter: &ParamSetter, param: &FloatParam, label: &str) {
    let size = Vec2::new(ui.available_width(), 26.0);
    let (response, painter) = ui.allocate_painter(size, Sense::click_and_drag());
    let rect = response.rect;
    let norm = param.unmodulated_normalized_value();

    painter.rect_filled(rect, 4.0, PANEL);
    let fill = Rect::from_min_size(rect.min, Vec2::new(rect.width() * norm, rect.height()));
    painter.rect_filled(fill, 4.0, accent(70));
    painter.rect_filled(Rect::from_min_size(Pos2::new(fill.right() - 3.0, rect.top()), Vec2::new(3.0, rect.height())), 1.5, accent(255));
    painter.text(Pos2::new(rect.left() + 8.0, rect.center().y), Align2::LEFT_CENTER, label, FontId::monospace(10.0), TEXT_DIM);
    painter.text(Pos2::new(rect.right() - 8.0, rect.center().y), Align2::RIGHT_CENTER, param.to_string(), FontId::monospace(11.0), accent(255));
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

fn small_button(text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.to_string()).monospace().size(12.0).color(LINE))
        .fill(PANEL)
        .stroke(Stroke::new(1.0_f32, GRID))
        .min_size(Vec2::new(24.0, 22.0))
}

/// `< value >` selector for an enum parameter.
fn stepper<E: Enum + PartialEq + 'static>(ui: &mut Ui, setter: &ParamSetter, param: &EnumParam<E>, label: &str, width: f32) {
    ui.label(RichText::new(label).monospace().size(10.0).color(TEXT_DIM));
    let count = E::variants().len();
    let current = param.value().to_index();
    let step = |delta: isize| {
        let next = (current as isize + delta).rem_euclid(count as isize) as usize;
        setter.begin_set_parameter(param);
        setter.set_parameter(param, E::from_index(next));
        setter.end_set_parameter(param);
    };
    if ui.add(small_button("<")).clicked() {
        step(-1);
    }
    ui.add_sized([width, 22.0], egui::Label::new(RichText::new(param.to_string()).monospace().size(12.0).color(accent(255))));
    if ui.add(small_button(">")).clicked() {
        step(1);
    }
}

/// A row of buttons, one per variant, with the current one lit.
fn segmented<E: Enum + PartialEq + 'static>(ui: &mut Ui, setter: &ParamSetter, param: &EnumParam<E>, label: &str) {
    ui.label(RichText::new(label).monospace().size(10.0).color(TEXT_DIM));
    let current = param.value().to_index();
    for (i, name) in E::variants().iter().enumerate() {
        let short = name.split_whitespace().next().unwrap_or(name);
        let selected = i == current;
        let button = egui::Button::new(RichText::new(short.to_string()).monospace().size(11.0).color(if selected { BG } else { LINE }))
            .fill(if selected { accent(255) } else { PANEL })
            .stroke(Stroke::new(1.0_f32, GRID))
            .min_size(Vec2::new(44.0, 22.0));
        if ui.add(button).clicked() && !selected {
            setter.begin_set_parameter(param);
            setter.set_parameter(param, E::from_index(i));
            setter.end_set_parameter(param);
        }
    }
}
