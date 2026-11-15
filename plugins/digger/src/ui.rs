//! The editor: eight drum slots along the top, and under them a record crate for the slot in
//! focus. Its samples stand upright like sleeves; the chosen one is pulled up and opened out, and
//! stepping through them slides the crate along. Keys, the wheel, a drag or the navigation notes
//! all do the same thing.

use crate::engine::SLOTS;
use crate::library::{Library, PEAKS};
use crate::state::{note_name, Nav, NO_KIT, SLOT_INFO};
use crate::{spawn_load, DiggerParams};
use nih_plug::prelude::*;
use nih_plug_egui::egui::{self, epaint::TextShape, Align2, Color32, CursorIcon, FontId, Key, Modifiers, Painter, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, Vec2};
use std::f32::consts::{FRAC_PI_2, PI};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub const BG: Color32 = Color32::from_rgb(0x12, 0x0f, 0x15);
pub const PANEL: Color32 = Color32::from_rgb(0x1c, 0x18, 0x22);
pub const PANEL_HI: Color32 = Color32::from_rgb(0x28, 0x23, 0x30);
pub const GRID: Color32 = Color32::from_rgb(0x38, 0x32, 0x43);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x8c, 0x84, 0x99);
/// Sleeve stock and type: warm paper.
pub const PAPER: [f32; 3] = [244.0, 236.0, 218.0];
pub const INK: [f32; 3] = [28.0, 22.0, 34.0];
/// One colour per kind of drum: lows, snaps, metal, everything else.
pub const TONES: [[f32; 3]; 4] = [[255.0, 74.0, 140.0], [190.0, 242.0, 52.0], [62.0, 222.0, 236.0], [255.0, 168.0, 52.0]];

const HEADER_H: f32 = 50.0;
const CARD_H: f32 = 150.0;
const GAP: f32 = 12.0;
const SIDE_W: f32 = 232.0;
const KIT_H: f32 = 34.0;
const FRONT_H: f32 = 78.0;

pub fn background() -> Color32 {
    BG
}

pub fn rgba(c: [f32; 3], a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c[0] as u8, c[1] as u8, c[2] as u8, a.clamp(0.0, 255.0) as u8)
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn tone(slot: usize) -> [f32; 3] {
    TONES[SLOT_INFO[slot].tone]
}

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// `text` cut to `max` characters, with a dot where it was shortened.
fn fit(text: &str, max: usize) -> String {
    let n = text.chars().count();
    if n <= max {
        text.to_owned()
    } else {
        let mut s: String = text.chars().take(max.saturating_sub(1)).collect();
        s.push('.');
        s
    }
}

#[derive(Default)]
pub struct UiState {
    lib: Option<Arc<Library>>,
    generation: u32,
    status: String,
    /// Eased position of each slot's crate, in samples.
    scroll: [f32; SLOTS],
    hit: [f32; SLOTS],
    peak: f32,
    drag_acc: f32,
    wheel_acc: f32,
    picked: Arc<Mutex<Option<String>>>,
    picker_open: Arc<AtomicBool>,
    /// Text being typed into the "add tag" box and the kit name box.
    tag_input: String,
    kit_name: String,
    /// The kit whose name the box currently shows, so it reloads when another is picked.
    kit_shown: Option<usize>,
    toast: (String, f64),
}

/// Show the system's folder chooser on a background thread. The chosen path lands in `result`.
fn pick_folder(result: &Arc<Mutex<Option<String>>>, open: &Arc<AtomicBool>) {
    #[cfg(target_os = "macos")]
    {
        if open.swap(true, Ordering::SeqCst) {
            return;
        }
        let (result, open) = (result.clone(), open.clone());
        std::thread::spawn(move || {
            let out = std::process::Command::new("osascript").args(["-e", "POSIX path of (choose folder with prompt \"Choose the folder of drum samples\")"]).output();
            if let Ok(o) = out {
                let chosen = String::from_utf8_lossy(&o.stdout).trim().to_owned();
                if o.status.success() && !chosen.is_empty() {
                    if let Ok(mut r) = result.lock() {
                        *r = Some(chosen);
                    }
                }
            }
            open.store(false, Ordering::SeqCst);
        });
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (result, open);
}

/// A hardware knob: vertical drag, shift for fine, double-click to reset.
#[allow(clippy::too_many_arguments)]
fn knob(ui: &mut Ui, painter: &Painter, cell: Rect, setter: &ParamSetter, param: &FloatParam, label: &str, color: [f32; 3], bipolar: bool) {
    let r = ui.interact(cell, ui.id().with(("knob", label, param.name())), Sense::click_and_drag());
    if r.drag_started() {
        setter.begin_set_parameter(param);
    }
    if r.dragged() {
        let speed = if ui.input(|i| i.modifiers.shift) { 1.0 / 1200.0 } else { 1.0 / 160.0 };
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
    if r.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeVertical);
    }
    let norm = param.unmodulated_normalized_value();
    let center = Pos2::new(cell.center().x, cell.top() + 26.0);
    let radius = 19.0;
    let (start, end) = (PI * 0.75, PI * 2.25);
    let arc = |from: f32, to: f32| -> Vec<Pos2> {
        let steps = (((to - from).abs() / 0.08) as usize).max(2);
        (0..=steps).map(|s| center + Vec2::new((from + (to - from) * s as f32 / steps as f32).cos(), (from + (to - from) * s as f32 / steps as f32).sin()) * radius).collect()
    };
    let lit = r.hovered() || r.dragged();
    painter.circle_filled(center, radius - 4.0, if lit { PANEL_HI } else { PANEL });
    painter.add(Shape::line(arc(start, end), Stroke::new(3.0_f32, GRID)));
    // Bipolar knobs light from the middle, so "centre" reads as nothing.
    let to = start + (end - start) * norm;
    let from = if bipolar { start + (end - start) * 0.5 } else { start };
    let (a, b) = if to >= from { (from, to) } else { (to, from) };
    if b - a > 0.01 {
        painter.add(Shape::line(arc(a, b), Stroke::new(8.0_f32, rgba(color, 28.0))));
        painter.add(Shape::line(arc(a, b), Stroke::new(3.0_f32, rgba(color, 255.0))));
    }
    let dir = Vec2::new(to.cos(), to.sin());
    painter.line_segment([center + dir * (radius - 11.0), center + dir * (radius - 5.0)], Stroke::new(2.0_f32, rgba(PAPER, 255.0)));
    painter.text(Pos2::new(cell.center().x, cell.top() + 54.0), Align2::CENTER_CENTER, label, FontId::monospace(8.5), TEXT_DIM);
    painter.text(Pos2::new(cell.center().x, cell.top() + 68.0), Align2::CENTER_CENTER, param.to_string(), FontId::monospace(10.5), rgba(color, 255.0));
}

fn button(ui: &mut Ui, painter: &Painter, rect: Rect, label: &str, on: bool, color: [f32; 3]) -> bool {
    let r = ui.interact(rect, ui.id().with(("button", label)), Sense::click());
    if r.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    let fill = if on { rgba(color, 255.0) } else if r.hovered() { PANEL_HI } else { PANEL };
    painter.rect_filled(rect, 5.0, fill);
    painter.rect_stroke(rect, 5.0, Stroke::new(1.0_f32, if on { rgba(color, 255.0) } else { GRID }), StrokeKind::Inside);
    painter.text(rect.center(), Align2::CENTER_CENTER, label, FontId::monospace(9.5), if on { rgba(INK, 255.0) } else { rgba(PAPER, 230.0) });
    r.clicked()
}

pub fn draw(ui: &mut Ui, params: &Arc<DiggerParams>, setter: &ParamSetter, state: &mut UiState) {
    let shared = &params.shared;
    let dt = ui.input(|i| i.unstable_dt).clamp(0.001, 0.1);
    ui.ctx().request_repaint();

    // A new library, or a folder the user just chose.
    let generation = shared.lib.generation.load(Ordering::Relaxed);
    if generation != state.generation || state.lib.is_none() {
        state.generation = generation;
        state.lib = shared.lib.library();
        state.status = shared.lib.status();
    }
    let chosen = state.picked.lock().ok().and_then(|mut p| p.take());
    if let Some(path) = chosen {
        shared.set_root(&path);
        spawn_load(params, path.into(), false);
    }

    // Meters: rise fast, fall slow.
    for s in 0..SLOTS {
        state.hit[s] = (state.hit[s] * (-dt * 4.0).exp()).max(shared.hit[s].swap(0.0, Ordering::Relaxed));
    }
    state.peak = (state.peak - dt * 1.4).max(shared.out_peak.swap(0.0, Ordering::Relaxed));

    handle_keys(ui, params, state);

    // The crates slide to wherever the selection is.
    for s in 0..SLOTS {
        let target = shared.selection(s).1 as f32;
        let diff = target - state.scroll[s];
        state.scroll[s] = if diff.abs() < 0.002 { target } else { state.scroll[s] + diff * (1.0 - (-dt * 15.0).exp()) };
    }

    let painter = ui.painter().clone();
    let full = ui.max_rect();
    let header = Rect::from_min_size(full.min, Vec2::new(full.width(), HEADER_H));
    let cards = Rect::from_min_size(Pos2::new(full.left(), header.bottom() + GAP), Vec2::new(full.width(), CARD_H));
    let foot = Rect::from_min_size(Pos2::new(full.left(), full.bottom() - 14.0), Vec2::new(full.width(), 14.0));
    let kits = Rect::from_min_size(Pos2::new(full.left(), cards.bottom() + 10.0), Vec2::new(full.width(), KIT_H));
    let body = Rect::from_min_max(Pos2::new(full.left(), kits.bottom() + 10.0), Pos2::new(full.right(), foot.top() - 8.0));

    draw_header(ui, &painter, header, params, state);
    draw_cards(ui, &painter, cards, params, state);
    draw_kits(ui, &painter, kits, params, state);
    let side = Rect::from_min_max(Pos2::new(body.right() - SIDE_W, body.top()), body.max);
    let crate_rect = Rect::from_min_max(body.min, Pos2::new(side.left() - GAP, body.bottom()));
    draw_crate(ui, &painter, crate_rect, params, state);
    draw_side(ui, &painter, side, params, setter, state);

    painter.text(
        foot.left_center(),
        Align2::LEFT_CENTER,
        "KEYS: < > SAMPLE  ^ v CATEGORY  TAB 1-8 SLOT  R RANDOM  SPACE HIT  [ ] KIT  K ROLL KIT      MIDI C7-A#7 STEERS IT FROM YOUR KEYBOARD",
        FontId::monospace(9.0),
        TEXT_DIM,
    );
}

fn handle_keys(ui: &mut Ui, params: &Arc<DiggerParams>, state: &mut UiState) {
    // Typing into a text box is not a shortcut.
    if ui.ctx().wants_keyboard_input() {
        return;
    }
    let Some(lib) = state.lib.clone() else { return };
    let shared = &params.shared;
    let slot = shared.focus();
    let seed = ui.input(|i| (i.time * 1000.0) as u32).wrapping_mul(2654435761);
    let take = |mods: Modifiers, key: Key| ui.input_mut(|i| i.consume_key(mods, key));
    let step = |n: isize| {
        shared.step_sample(slot, n, &lib);
        shared.request_browse(slot);
    };
    if take(Modifiers::NONE, Key::ArrowRight) {
        step(1);
    }
    if take(Modifiers::NONE, Key::ArrowLeft) {
        step(-1);
    }
    if take(Modifiers::SHIFT, Key::ArrowRight) {
        step(10);
    }
    if take(Modifiers::SHIFT, Key::ArrowLeft) {
        step(-10);
    }
    if take(Modifiers::NONE, Key::ArrowDown) {
        shared.navigate(Nav::CatNext, seed, &lib);
    }
    if take(Modifiers::NONE, Key::ArrowUp) {
        shared.navigate(Nav::CatPrev, seed, &lib);
    }
    if take(Modifiers::NONE, Key::Tab) {
        shared.move_focus(1);
    }
    if take(Modifiers::SHIFT, Key::Tab) {
        shared.move_focus(-1);
    }
    if take(Modifiers::NONE, Key::R) {
        shared.navigate(Nav::SampleRandom, seed, &lib);
    }
    if take(Modifiers::NONE, Key::Space) {
        shared.navigate(Nav::Audition, seed, &lib);
    }
    if take(Modifiers::NONE, Key::OpenBracket) {
        shared.navigate(Nav::KitPrev, seed, &lib);
    }
    if take(Modifiers::NONE, Key::CloseBracket) {
        shared.navigate(Nav::KitNext, seed, &lib);
    }
    if take(Modifiers::NONE, Key::K) {
        shared.navigate(Nav::KitRoll, seed, &lib);
    }
    for (n, key) in [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8].into_iter().enumerate() {
        if take(Modifiers::NONE, key) {
            shared.set_focus(n);
        }
    }
}

fn draw_header(ui: &mut Ui, painter: &Painter, rect: Rect, params: &Arc<DiggerParams>, state: &mut UiState) {
    painter.text(rect.left_top() + Vec2::new(0.0, 14.0), Align2::LEFT_CENTER, "DIGGER", FontId::monospace(21.0), rgba(PAPER, 255.0));
    painter.text(rect.left_top() + Vec2::new(0.0, 35.0), Align2::LEFT_CENTER, "crate-digging drum sampler", FontId::monospace(9.5), TEXT_DIM);

    let root = params.shared.root.lock().map(|r| r.clone()).unwrap_or_default();
    let button_rect = Rect::from_min_size(Pos2::new(rect.right() - 128.0, rect.top() + 6.0), Vec2::new(128.0, 24.0));
    if button(ui, painter, button_rect, if root.is_empty() { "CHOOSE FOLDER" } else { "CHANGE FOLDER" }, false, TONES[0]) {
        pick_folder(&state.picked, &state.picker_open);
    }
    let shown = if root.is_empty() {
        "NO FOLDER".to_owned()
    } else {
        let name = std::path::Path::new(root.trim_end_matches('/')).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(root.clone());
        name.to_uppercase()
    };
    painter.text(Pos2::new(button_rect.left() - 14.0, rect.top() + 11.0), Align2::RIGHT_CENTER, shown, FontId::monospace(11.0), rgba(PAPER, 235.0));
    let recent = ui.input(|i| i.time) - state.toast.1 < 4.0 && !state.toast.0.is_empty();
    let (text, ink) = if recent { (&state.toast.0, rgba(TONES[1], 255.0)) } else { (&state.status, TEXT_DIM) };
    painter.text(Pos2::new(button_rect.left() - 14.0, rect.top() + 29.0), Align2::RIGHT_CENTER, text, FontId::monospace(8.5), ink);
}

/// A text box in the editor's own colours.
fn text_box(ui: &mut Ui, rect: Rect, text: &mut String, hint: &str, id: &str) -> egui::Response {
    ui.put(
        rect,
        egui::TextEdit::singleline(text)
            .id(ui.id().with(id))
            .font(FontId::monospace(10.0))
            .hint_text(hint)
            .text_color(rgba(PAPER, 240.0))
            .desired_width(rect.width())
            .margin(egui::Margin::symmetric(8, 5)),
    )
}

fn toast(state: &mut UiState, ui: &Ui, text: impl Into<String>) {
    state.toast = (text.into(), ui.input(|i| i.time));
}

/// Writes the sidecar and says so if it could not.
fn save(ui: &Ui, state: &mut UiState, params: &Arc<DiggerParams>) {
    if let Err(e) = params.shared.save_sidecar() {
        toast(state, ui, format!("COULD NOT SAVE TAGS AND KITS: {}", e.to_uppercase()));
    }
}

/// The kit strip: step through saved kits, name and save the current one, roll a new one.
fn draw_kits(ui: &mut Ui, painter: &Painter, area: Rect, params: &Arc<DiggerParams>, state: &mut UiState) {
    let shared = &params.shared;
    let Some(lib) = state.lib.clone() else { return };
    let color = tone(shared.focus());
    painter.rect_filled(area, 7.0, PANEL);
    painter.rect_stroke(area, 7.0, Stroke::new(1.0_f32, GRID), StrokeKind::Inside);
    let seed = ui.input(|i| (i.time * 1000.0) as u32).wrapping_mul(2654435761);
    let current = shared.kit.load(Ordering::Relaxed);
    let (kit_count, current_name) = shared.kits.lock().map(|k| (k.len(), k.get(current).map(|x| x.name.clone()))).unwrap_or((0, None));

    // The name box follows the selected kit.
    let shown = (current != NO_KIT).then_some(current);
    if state.kit_shown != shown {
        state.kit_shown = shown;
        state.kit_name = current_name.clone().unwrap_or_default();
    }

    let y = area.top() + 5.0;
    let h = area.height() - 10.0;
    let mut x = area.left() + 10.0;
    painter.text(Pos2::new(x, area.center().y), Align2::LEFT_CENTER, "KIT", FontId::monospace(11.0), rgba(PAPER, 255.0));
    x += 36.0;
    for (label, d) in [("<", -1isize), (">", 1)] {
        if button(ui, painter, Rect::from_min_size(Pos2::new(x, y), Vec2::new(26.0, h)), label, false, color) {
            shared.step_kit(d, &lib);
        }
        x += 30.0;
    }
    x += 4.0;
    let name_rect = Rect::from_min_size(Pos2::new(x, y), Vec2::new(150.0, h));
    painter.rect_filled(name_rect, 5.0, BG);
    painter.rect_stroke(name_rect, 5.0, Stroke::new(1.0_f32, GRID), StrokeKind::Inside);
    let resp = text_box(ui, name_rect, &mut state.kit_name, if kit_count == 0 { "NO KITS YET" } else { "UNSAVED MIX" }, "kitname");
    if resp.lost_focus() && current != NO_KIT && !state.kit_name.trim().is_empty() {
        if let Ok(mut kits) = shared.kits.lock() {
            if let Some(k) = kits.get_mut(current) {
                k.name = state.kit_name.trim().to_uppercase();
            }
        }
        save(ui, state, params);
    }
    x += 158.0;

    if button(ui, painter, Rect::from_min_size(Pos2::new(x, y), Vec2::new(78.0, h)), "SAVE NEW", false, color) {
        let name = format!("KIT {:02}", kit_count + 1);
        let kit = shared.kit_from_selection(&name, &lib);
        if let Ok(mut kits) = shared.kits.lock() {
            kits.push(kit);
            shared.kit.store(kits.len() - 1, Ordering::Relaxed);
        }
        state.kit_shown = None;
        save(ui, state, params);
        toast(state, ui, format!("SAVED {name}"));
    }
    x += 84.0;
    if current != NO_KIT && current_name.is_some() {
        if button(ui, painter, Rect::from_min_size(Pos2::new(x, y), Vec2::new(64.0, h)), "UPDATE", false, color) {
            let name = current_name.clone().unwrap_or_default();
            let kit = shared.kit_from_selection(&name, &lib);
            if let Ok(mut kits) = shared.kits.lock() {
                if let Some(k) = kits.get_mut(current) {
                    *k = kit;
                }
            }
            save(ui, state, params);
            toast(state, ui, format!("{name} NOW HOLDS WHAT IS IN THE SLOTS"));
        }
        x += 70.0;
        if button(ui, painter, Rect::from_min_size(Pos2::new(x, y), Vec2::new(64.0, h)), "DELETE", false, TONES[0]) {
            if let Ok(mut kits) = shared.kits.lock() {
                if current < kits.len() {
                    kits.remove(current);
                }
            }
            shared.kit.store(NO_KIT, Ordering::Relaxed);
            state.kit_shown = None;
            save(ui, state, params);
        }
        x += 70.0;
    }

    // Roll sits at the right edge; saved kits fill the space between.
    let roll_w = 104.0;
    let roll = Rect::from_min_size(Pos2::new(area.right() - roll_w - 6.0, y), Vec2::new(roll_w, h));
    let filter = shared.filter.load(Ordering::Relaxed);
    let label = if filter != 0 { format!("ROLL {}", lib.tag_name(filter.trailing_zeros() as usize).to_uppercase()) } else { "ROLL KIT".to_owned() };
    if button(ui, painter, roll, &fit(&label, 13), true, color) {
        shared.roll_kit(seed, &lib);
    }
    x += 8.0;
    let names: Vec<String> = shared.kits.lock().map(|k| k.iter().map(|x| x.name.clone()).collect()).unwrap_or_default();
    for (i, name) in names.iter().enumerate() {
        let w = name.chars().count() as f32 * 6.4 + 16.0;
        if x + w > roll.left() - 10.0 {
            painter.text(Pos2::new(x, area.center().y), Align2::LEFT_CENTER, "...", FontId::monospace(10.0), TEXT_DIM);
            break;
        }
        if button(ui, painter, Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, h)), name, i == current, color) {
            shared.apply_kit(i, &lib);
        }
        x += w + 5.0;
    }
    if names.is_empty() {
        painter.text(Pos2::new(x, area.center().y), Align2::LEFT_CENTER, "SAVE A KIT TO SWITCH BETWEEN SETS OF SAMPLES", FontId::monospace(8.5), TEXT_DIM);
    }
}

fn draw_cards(ui: &mut Ui, painter: &Painter, area: Rect, params: &Arc<DiggerParams>, state: &mut UiState) {
    let shared = &params.shared;
    let w = (area.width() - GAP * (SLOTS as f32 - 1.0)) / SLOTS as f32;
    for (s, info) in SLOT_INFO.iter().enumerate() {
        let rect = Rect::from_min_size(Pos2::new(area.left() + s as f32 * (w + GAP), area.top()), Vec2::new(w, area.height()));
        let color = tone(s);
        let focused = shared.focus() == s;
        let r = ui.interact(rect, ui.id().with(("card", s)), Sense::click());
        if r.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        if r.clicked() {
            shared.set_focus(s);
            shared.request_audition(s);
        }
        let flash = state.hit[s].clamp(0.0, 1.0);
        if flash > 0.01 {
            painter.rect_stroke(rect.expand(3.0), 9.0, Stroke::new(6.0_f32, rgba(color, 40.0 * flash)), StrokeKind::Outside);
        }
        painter.rect_filled(rect, 7.0, if r.hovered() { PANEL_HI } else { PANEL });
        painter.rect_stroke(rect, 7.0, Stroke::new(if focused { 2.0_f32 } else { 1.0_f32 }, if focused { rgba(color, 255.0) } else { mix_color(GRID, color, flash) }), StrokeKind::Inside);

        let inner = rect.shrink(9.0);
        painter.text(inner.left_top() + Vec2::new(0.0, 6.0), Align2::LEFT_CENTER, info.name, FontId::monospace(12.0), rgba(color, 255.0));
        painter.text(inner.right_top() + Vec2::new(0.0, 6.0), Align2::RIGHT_CENTER, note_name(info.note), FontId::monospace(9.0), TEXT_DIM);

        let (cat, idx) = shared.selection(s);
        let sample = state.lib.as_ref().and_then(|l| l.sample(cat, idx));
        let wave = Rect::from_min_max(inner.left_top() + Vec2::new(0.0, 22.0), inner.right_bottom() - Vec2::new(0.0, 36.0));
        if let Some(sm) = sample {
            let pos = shared.pos[s].load(Ordering::Relaxed);
            waveform(painter, wave, &sm.peaks, color, 0.28 + 0.6 * flash, pos);
            let cat_name = state.lib.as_ref().map(|l| l.cats[cat].name.to_uppercase()).unwrap_or_default();
            let chars = (inner.width() / 6.0) as usize;
            painter.text(inner.left_bottom() + Vec2::new(0.0, -24.0), Align2::LEFT_CENTER, fit(&cat_name, chars), FontId::monospace(8.5), TEXT_DIM);
            painter.text(inner.left_bottom() + Vec2::new(0.0, -9.0), Align2::LEFT_CENTER, fit(&sm.name, chars), FontId::monospace(10.0), rgba(PAPER, 235.0));
        } else {
            painter.line_segment([wave.left_center(), wave.right_center()], Stroke::new(1.0_f32, GRID));
            painter.text(inner.left_bottom() + Vec2::new(0.0, -9.0), Align2::LEFT_CENTER, "EMPTY", FontId::monospace(10.0), TEXT_DIM);
        }
    }
}

fn mix_color(a: Color32, b: [f32; 3], t: f32) -> Color32 {
    let a = [a.r() as f32, a.g() as f32, a.b() as f32];
    let m = mix(a, b, t.clamp(0.0, 1.0));
    Color32::from_rgb(m[0] as u8, m[1] as u8, m[2] as u8)
}

/// The sample's shape as mirrored bars. What has already played is lit; a line marks the head.
fn waveform(painter: &Painter, rect: Rect, peaks: &[f32], color: [f32; 3], alpha: f32, playhead: f32) {
    if peaks.is_empty() {
        return;
    }
    let mid = rect.center().y;
    let step = rect.width() / peaks.len() as f32;
    for (i, p) in peaks.iter().enumerate() {
        let x = rect.left() + (i as f32 + 0.5) * step;
        let h = (p.powf(0.7) * rect.height() * 0.5).max(1.0);
        let played = playhead >= 0.0 && (i as f32 / peaks.len() as f32) <= playhead;
        let a = if played { 255.0 } else { alpha * 255.0 };
        if played {
            painter.line_segment([Pos2::new(x, mid - h), Pos2::new(x, mid + h)], Stroke::new((step - 1.0).max(1.0) + 3.0, rgba(color, 40.0)));
        }
        painter.line_segment([Pos2::new(x, mid - h), Pos2::new(x, mid + h)], Stroke::new((step - 1.0).max(1.0), rgba(color, a)));
    }
    if playhead >= 0.0 {
        let x = rect.left() + playhead * rect.width();
        painter.line_segment([Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())], Stroke::new(1.0_f32, rgba(PAPER, 200.0)));
    }
}

fn draw_crate(ui: &mut Ui, painter: &Painter, area: Rect, params: &Arc<DiggerParams>, state: &mut UiState) {
    let shared = &params.shared;
    let slot = shared.focus();
    let color = tone(slot);

    let Some(lib) = state.lib.clone() else {
        painter.rect_filled(area, 8.0, PANEL);
        painter.text(area.center() + Vec2::new(0.0, -22.0), Align2::CENTER_CENTER, if state.status.is_empty() { "NO SAMPLES YET" } else { &state.status }, FontId::monospace(13.0), rgba(PAPER, 255.0));
        let b = Rect::from_center_size(area.center() + Vec2::new(0.0, 14.0), Vec2::new(200.0, 30.0));
        if button(ui, painter, b, "CHOOSE A FOLDER OF WAVS", true, TONES[0]) {
            pick_folder(&state.picked, &state.picker_open);
        }
        painter.text(area.center() + Vec2::new(0.0, 52.0), Align2::CENTER_CENTER, "each sub-folder becomes a category", FontId::monospace(9.0), TEXT_DIM);
        return;
    };
    let (cat_i, idx) = shared.selection(slot);
    let Some(cat) = lib.cats.get(cat_i) else { return };

    // Category tabs.
    let tabs = Rect::from_min_size(area.min, Vec2::new(area.width(), 24.0));
    let mut x = tabs.left();
    for (i, c) in lib.cats.iter().enumerate() {
        let label = c.name.to_uppercase();
        let wd = label.chars().count() as f32 * 6.4 + 18.0;
        if x + wd > tabs.right() {
            break;
        }
        let r = Rect::from_min_size(Pos2::new(x, tabs.top()), Vec2::new(wd, tabs.height()));
        if button(ui, painter, r, &label, i == cat_i, color) {
            shared.set_cat(slot, i, &lib);
            shared.request_browse(slot);
        }
        x += wd + 6.0;
    }

    // Tag filter: the tags in this category, commonest first. One at a time.
    let filter = shared.filter.load(Ordering::Relaxed);
    let tag_row = Rect::from_min_size(Pos2::new(area.left(), tabs.bottom() + 6.0), Vec2::new(area.width(), 22.0));
    painter.text(tag_row.left_center(), Align2::LEFT_CENTER, "TAGS", FontId::monospace(8.5), TEXT_DIM);
    let mut x = tag_row.left() + 34.0;
    let category_tags = lib.category_tags(cat_i);
    if category_tags.is_empty() {
        painter.text(Pos2::new(x, tag_row.center().y), Align2::LEFT_CENTER, "NONE YET: ADD ONE FROM THE PANEL ON THE RIGHT", FontId::monospace(8.5), TEXT_DIM);
    }
    for (bit, count) in category_tags {
        let label = format!("{} {}", lib.tag_name(bit).to_uppercase(), count);
        let wd = label.chars().count() as f32 * 6.0 + 14.0;
        if x + wd > tag_row.right() {
            break;
        }
        let r = Rect::from_min_size(Pos2::new(x, tag_row.top()), Vec2::new(wd, tag_row.height()));
        if button(ui, painter, r, &label, filter == 1u64 << bit, color) {
            shared.toggle_filter(bit);
            // Land on something that shows.
            let (c, i) = shared.selection(slot);
            if lib.sample(c, i).is_some_and(|sm| filter != 1u64 << bit && sm.tags.load(Ordering::Relaxed) >> bit & 1 == 0) {
                shared.step_sample(slot, 1, &lib);
            }
        }
        x += wd + 5.0;
    }

    let crate_area = Rect::from_min_max(Pos2::new(area.left(), tag_row.bottom() + 8.0), area.max);
    let clip = painter.with_clip_rect(crate_area);
    clip.rect_filled(crate_area, 8.0, PANEL);
    let rim = crate_area.bottom() - FRONT_H;
    let pos = state.scroll[slot];
    let cx = crate_area.center().x;
    let n = cat.samples.len();

    // Sleeves, farthest first so the chosen one lands on top.
    let mut order: Vec<(f32, usize)> = (0..n).map(|i| (i as f32 - pos, i)).filter(|(d, _)| d.abs() < 12.0).collect();
    order.sort_by(|a, b| b.0.abs().partial_cmp(&a.0.abs()).unwrap_or(std::cmp::Ordering::Equal));
    let mut rects: Vec<(f32, usize, Rect)> = Vec::with_capacity(order.len());
    for &(d, i) in &order {
        let near = smooth(1.0 - d.abs());
        let w = 46.0 + 132.0 * near;
        let centre = cx + d.signum() * (d.abs() * 50.0 + 66.0 * smooth(d.abs()));
        let top = crate_area.top() + 30.0 - 26.0 * near + d.abs().min(6.0) * 3.0;
        let rect = Rect::from_min_max(Pos2::new(centre - w * 0.5, top), Pos2::new(centre + w * 0.5, rim + 40.0));
        let sample = &cat.samples[i];
        // Sleeves the tag filter hides stay in the crate, greyed back.
        let shown = filter == 0 || sample.tags.load(Ordering::Relaxed) & filter != 0;
        let fade = (1.0 - (d.abs() - 6.0).max(0.0) / 5.0).clamp(0.0, 1.0) * if shown { 1.0 } else { 0.28 };

        // Paper for the open one, dark stock tinted with the slot's colour for the rest.
        let body = mix(mix([40.0, 34.0, 50.0], color, 0.16), PAPER, near);
        let body = if shown { body } else { mix(body, [28.0, 24.0, 34.0], 0.6) };
        clip.rect_filled(rect.translate(Vec2::new(3.0, 3.0)), 5.0, Color32::from_black_alpha((90.0 * fade) as u8));
        clip.rect_filled(rect, 5.0, rgba(body, 255.0 * fade));
        clip.rect_filled(Rect::from_min_size(rect.min, Vec2::new(rect.width(), 5.0)), 5.0, rgba(color, 255.0 * fade));
        if near > 0.02 {
            clip.rect_stroke(rect.expand(2.0), 6.0, Stroke::new(5.0_f32, rgba(color, 45.0 * near)), StrokeKind::Outside);
        }

        // Spine: the name running up the narrow sleeve; it gives way to the open face.
        let spine_a = (1.0 - near * 1.6).clamp(0.0, 1.0) * fade;
        if spine_a > 0.02 {
            let chars = ((rim - rect.top() - 30.0) / 6.0) as usize;
            let galley = ui.fonts(|f| f.layout_no_wrap(fit(&sample.name, chars), FontId::monospace(10.0), rgba(PAPER, 200.0 * spine_a)));
            clip.add(TextShape::new(Pos2::new(rect.left() + 8.0, rim - 8.0), galley, Color32::WHITE).with_angle(-FRAC_PI_2));
            clip.text(Pos2::new(rect.center().x, rect.top() + 17.0), Align2::CENTER_CENTER, format!("{:02}", i + 1), FontId::monospace(8.5), rgba(color, 220.0 * spine_a));
        }
        let face_a = ((near - 0.35) / 0.65).clamp(0.0, 1.0);
        if face_a > 0.02 {
            let chars = ((rect.width() - 16.0) / 6.6) as usize;
            clip.text(rect.left_top() + Vec2::new(10.0, 22.0), Align2::LEFT_CENTER, fit(&sample.name.to_uppercase(), chars), FontId::monospace(11.0), rgba(INK, 255.0 * face_a));
            clip.text(rect.left_top() + Vec2::new(10.0, 38.0), Align2::LEFT_CENTER, format!("{:03}/{:03}", i + 1, n), FontId::monospace(8.5), rgba(mix(INK, PAPER, 0.4), 255.0 * face_a));
            let wave = Rect::from_min_max(rect.left_top() + Vec2::new(8.0, 56.0), Pos2::new(rect.right() - 8.0, rim - 14.0));
            if wave.height() > 10.0 {
                waveform_face(&clip, wave, &sample.peaks, INK, face_a, shared.pos[slot].load(Ordering::Relaxed), color);
            }
        }
        rects.push((d.abs(), i, rect));
    }

    // The front of the crate, over the bottoms of the sleeves.
    let front = Rect::from_min_max(Pos2::new(crate_area.left(), rim), crate_area.max);
    clip.rect_filled(front, 8.0, PANEL_HI);
    clip.rect_filled(Rect::from_min_size(front.min, Vec2::new(front.width(), 3.0)), 0.0, rgba(color, 255.0));
    clip.rect_filled(Rect::from_min_size(front.min + Vec2::new(0.0, 3.0), Vec2::new(front.width(), 10.0)), 0.0, rgba(color, 14.0));
    clip.text(front.left_top() + Vec2::new(18.0, 30.0), Align2::LEFT_CENTER, cat.name.to_uppercase(), FontId::monospace(22.0), rgba(color, 255.0));
    clip.text(front.left_top() + Vec2::new(18.0, 56.0), Align2::LEFT_CENTER, format!("{} SAMPLES", n), FontId::monospace(9.5), TEXT_DIM);
    clip.text(front.right_top() + Vec2::new(-18.0, 30.0), Align2::RIGHT_CENTER, format!("{:03}", idx + 1), FontId::monospace(28.0), rgba(PAPER, 255.0));
    clip.text(front.right_top() + Vec2::new(-18.0, 56.0), Align2::RIGHT_CENTER, format!("OF {:03}", n), FontId::monospace(9.5), TEXT_DIM);

    // Interaction: click a sleeve, drag the crate along, or use the wheel.
    let r = ui.interact(crate_area, ui.id().with("crate"), Sense::click_and_drag());
    if r.hovered() {
        ui.ctx().set_cursor_icon(if r.dragged() { CursorIcon::Grabbing } else { CursorIcon::Grab });
        let scroll = ui.input(|i| i.raw_scroll_delta);
        state.wheel_acc += if scroll.x.abs() > scroll.y.abs() { scroll.x } else { scroll.y };
        while state.wheel_acc.abs() >= 24.0 {
            let dir = if state.wheel_acc > 0.0 { -1 } else { 1 };
            state.wheel_acc -= 24.0 * state.wheel_acc.signum();
            shared.step_sample(slot, dir, &lib);
            shared.request_browse(slot);
        }
    }
    if r.dragged() {
        state.drag_acc += r.drag_delta().x;
        while state.drag_acc.abs() >= 34.0 {
            let dir = if state.drag_acc > 0.0 { -1 } else { 1 };
            state.drag_acc -= 34.0 * state.drag_acc.signum();
            shared.step_sample(slot, dir, &lib);
            shared.request_browse(slot);
        }
    } else {
        state.drag_acc = 0.0;
    }
    if r.clicked() {
        if let Some(p) = r.interact_pointer_pos() {
            if let Some(&(_, i, _)) = rects.iter().rev().find(|(_, _, rc)| rc.contains(p) && p.y < rim) {
                shared.set_sample(slot, i, &lib);
                shared.request_browse(slot);
            }
        }
    }

    // On-screen controls on the front of the crate. They are added after the crate's own
    // interaction so they sit on top of it and take their clicks.
    let seed = ui.input(|i| (i.time * 1000.0) as u32).wrapping_mul(2654435761);
    let (bw, bh) = (46.0, 28.0);
    let row_y = front.top() + 14.0;
    let left = front.center().x - 2.5 * bw - 12.0 - 40.0;
    let mut step = None;
    for (k, (label, delta)) in [("<<", -10isize), ("<", -1), (">", 1), (">>", 10)].into_iter().enumerate() {
        let b = Rect::from_min_size(Pos2::new(left + k as f32 * (bw + 6.0), row_y), Vec2::new(bw, bh));
        if button(ui, &clip, b, label, false, color) {
            step = Some(delta);
        }
    }
    if let Some(d) = step {
        shared.step_sample(slot, d, &lib);
        shared.request_browse(slot);
    }
    let rand = Rect::from_min_size(Pos2::new(left + 4.0 * (bw + 6.0) + 8.0, row_y), Vec2::new(76.0, bh));
    if button(ui, &clip, rand, "RANDOM", false, color) {
        shared.navigate(Nav::SampleRandom, seed, &lib);
    }
    painter.text(Pos2::new(left, front.top() + 56.0), Align2::LEFT_CENTER, "STEP THROUGH THE CRATE, OR CLICK A SLEEVE", FontId::monospace(8.0), TEXT_DIM);
}

/// The open sleeve's waveform, drawn in ink on paper.
fn waveform_face(painter: &Painter, rect: Rect, peaks: &[f32], ink: [f32; 3], alpha: f32, playhead: f32, accent: [f32; 3]) {
    let mid = rect.center().y;
    let step = rect.width() / peaks.len().max(1) as f32;
    for (i, p) in peaks.iter().enumerate() {
        let x = rect.left() + (i as f32 + 0.5) * step;
        let h = (p.powf(0.7) * rect.height() * 0.5).max(1.0);
        let played = playhead >= 0.0 && (i as f32 / PEAKS as f32) <= playhead;
        painter.line_segment([Pos2::new(x, mid - h), Pos2::new(x, mid + h)], Stroke::new((step - 1.5).max(1.0), rgba(if played { mix(accent, ink, 0.2) } else { ink }, 255.0 * alpha * if played { 1.0 } else { 0.85 })));
    }
    if playhead >= 0.0 {
        let x = rect.left() + playhead * rect.width();
        painter.line_segment([Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())], Stroke::new(1.5_f32, rgba(accent, 255.0 * alpha)));
    }
}

fn draw_side(ui: &mut Ui, painter: &Painter, area: Rect, params: &Arc<DiggerParams>, setter: &ParamSetter, state: &mut UiState) {
    let shared = &params.shared;
    let slot = shared.focus();
    let color = tone(slot);
    painter.rect_filled(area, 8.0, PANEL);
    painter.rect_stroke(area, 8.0, Stroke::new(1.0_f32, GRID), StrokeKind::Inside);
    let inner = area.shrink(14.0);

    painter.text(inner.left_top() + Vec2::new(0.0, 8.0), Align2::LEFT_CENTER, SLOT_INFO[slot].name, FontId::monospace(18.0), rgba(color, 255.0));
    painter.text(inner.right_top() + Vec2::new(0.0, 8.0), Align2::RIGHT_CENTER, format!("PLAYS {}", note_name(SLOT_INFO[slot].note)), FontId::monospace(9.0), TEXT_DIM);

    let (cat, idx) = shared.selection(slot);
    if let Some(sm) = state.lib.as_ref().and_then(|l| l.sample(cat, idx)) {
        let chars = (inner.width() / 8.6) as usize;
        let name = sm.name.to_uppercase();
        // Long names break over two lines rather than losing their ends.
        let (a, b): (String, String) = if name.chars().count() > chars { (name.chars().take(chars).collect(), fit(&name.chars().skip(chars).collect::<String>(), chars)) } else { (name, String::new()) };
        painter.text(inner.left_top() + Vec2::new(0.0, 36.0), Align2::LEFT_CENTER, a, FontId::monospace(14.0), rgba(PAPER, 255.0));
        painter.text(inner.left_top() + Vec2::new(0.0, 54.0), Align2::LEFT_CENTER, b, FontId::monospace(14.0), rgba(PAPER, 255.0));
        let secs = sm.len() as f32 / state.lib.as_ref().map_or(44100.0, |l| l.sample_rate as f32);
        painter.text(inner.left_top() + Vec2::new(0.0, 76.0), Align2::LEFT_CENTER, format!("{:.0} MS  {}", secs * 1000.0, if sm.right.is_some() { "STEREO" } else { "MONO" }), FontId::monospace(9.0), TEXT_DIM);
    }

    // Tags on the selected sample. Ones the user added carry an x to remove them.
    let lib = state.lib.clone();
    let mut tags_bottom = inner.top() + 88.0;
    if let Some(lib) = lib.as_ref().filter(|l| l.sample(cat, idx).is_some()) {
        let (mut x, mut y) = (inner.left(), inner.top() + 88.0);
        let filter = shared.filter.load(Ordering::Relaxed);
        for (bit, name, user) in lib.sample_tags(cat, idx) {
            let label = name.to_uppercase();
            let wd = label.chars().count() as f32 * 6.0 + if user { 26.0 } else { 14.0 };
            if x + wd > inner.right() {
                x = inner.left();
                y += 21.0;
            }
            if y > inner.top() + 88.0 + 21.0 {
                break;
            }
            let r = Rect::from_min_size(Pos2::new(x, y), Vec2::new(wd, 18.0));
            if button(ui, painter, r, &label, filter == 1u64 << bit, color) {
                shared.toggle_filter(bit);
            }
            if user {
                let xr = Rect::from_min_size(Pos2::new(r.right() - 16.0, y), Vec2::new(16.0, 18.0));
                let hit = ui.interact(xr, ui.id().with(("untag", bit)), Sense::click());
                painter.text(xr.center(), Align2::CENTER_CENTER, "x", FontId::monospace(10.0), if hit.hovered() { rgba(TONES[0], 255.0) } else { TEXT_DIM });
                if hit.clicked() {
                    lib.toggle_user_tag(cat, idx, &name);
                    save(ui, state, params);
                }
            }
            x += wd + 4.0;
        }
        tags_bottom = y + 22.0;
        let add = Rect::from_min_size(Pos2::new(inner.left(), tags_bottom), Vec2::new(inner.width(), 22.0));
        painter.rect_filled(add, 5.0, BG);
        painter.rect_stroke(add, 5.0, Stroke::new(1.0_f32, GRID), StrokeKind::Inside);
        let resp = text_box(ui, add, &mut state.tag_input, "+ ADD A TAG, PRESS ENTER", "addtag");
        if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) && !state.tag_input.trim().is_empty() {
            let word = state.tag_input.trim().to_lowercase();
            if lib.sample_tags(cat, idx).iter().any(|t| t.1 == word) {
                // Already there: leave it be.
            } else if lib.toggle_user_tag(cat, idx, &word).is_some() {
                save(ui, state, params);
            } else {
                toast(state, ui, "TAG LIST IS FULL");
            }
            state.tag_input.clear();
        }
        tags_bottom = add.bottom() + 6.0;
    }

    let sp = &params.slots[slot];
    let cell_w = inner.width() / 3.0;
    let knobs_top = tags_bottom + 6.0;
    for (k, (param, label, bipolar)) in [(&sp.level, "LEVEL", false), (&sp.tune, "TUNE", true), (&sp.pan, "PAN", true)].into_iter().enumerate() {
        let cell = Rect::from_min_size(Pos2::new(inner.left() + k as f32 * cell_w, knobs_top), Vec2::new(cell_w, 78.0));
        knob(ui, painter, cell, setter, param, label, color, bipolar);
    }

    let hit = Rect::from_min_size(Pos2::new(inner.left(), knobs_top + 84.0), Vec2::new(inner.width(), 28.0));
    if button(ui, painter, hit, "HIT  (SPACE)", false, color) {
        shared.request_audition(slot);
    }
    let on = params.audition_browse.value();
    let toggle = Rect::from_min_size(Pos2::new(inner.left(), hit.bottom() + 8.0), Vec2::new(inner.width(), 24.0));
    if button(ui, painter, toggle, "SOUND WHILE BROWSING", on, color) {
        setter.begin_set_parameter(&params.audition_browse);
        setter.set_parameter(&params.audition_browse, !on);
        setter.end_set_parameter(&params.audition_browse);
    }
    painter.text(toggle.left_bottom() + Vec2::new(0.0, 12.0), Align2::LEFT_CENTER, "ONLY WHILE THE TRANSPORT IS STOPPED", FontId::monospace(7.5), TEXT_DIM);

    // Master level and the output meter along the bottom.
    let base = Rect::from_min_size(Pos2::new(inner.left(), inner.bottom() - 78.0), Vec2::new(cell_w, 78.0));
    knob(ui, painter, base, setter, &params.level, "MASTER", PAPER, false);
    let meter = Rect::from_min_max(Pos2::new(base.right() + 16.0, inner.bottom() - 52.0), Pos2::new(inner.right(), inner.bottom() - 40.0));
    painter.rect_filled(meter, 3.0, BG);
    let level = state.peak.clamp(0.0, 1.0).powf(0.5);
    let lit = Rect::from_min_size(meter.min, Vec2::new(meter.width() * level, meter.height()));
    painter.rect_filled(lit.expand2(Vec2::new(0.0, 3.0)), 3.0, rgba(color, 30.0));
    painter.rect_filled(lit, 3.0, rgba(if state.peak > 0.98 { TONES[0] } else { color }, 255.0));
    painter.text(meter.left_bottom() + Vec2::new(0.0, 12.0), Align2::LEFT_CENTER, "OUT", FontId::monospace(8.0), TEXT_DIM);
}
