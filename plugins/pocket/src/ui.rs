//! The editor: a pegboard. Every drum is a row of holes and every hit is a peg pushed into one,
//! its size the velocity and its position the timing nudge. A glowing comb sweeps across, and
//! each peg it strikes pops a marble out of the board in that drum's colour, thrown higher the
//! harder the hit.
//!
//! Left: the groove browser. Click a groove to load it; drag it out of the plugin and drop it on a
//! DAW track to get the MIDI clip. Bottom: the generator (style, complexity, dynamics, fill, swing,
//! humanize, seed) and the clip itself, which drags out the same way.
//!
//! On the board: click a hole for a peg, drag across holes to paint. Drag a peg up and down for
//! velocity (shift for its timing). Right-click a peg to cycle its roll, right-drag to erase.
//! Drag a drum's name to take just that drum as a clip. M mutes, S solos, L locks the drum so
//! re-rolling the generator leaves it alone.

use nih_plug::prelude::*;
use nih_plug_egui::egui::{self, Align2, Color32, CursorIcon, FontId, Mesh, Painter, PointerButton, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, Vec2};
use std::f32::consts::PI;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::browser::{pick_folder, Browser};
use crate::engine::Meter;
use crate::generator::{self, STYLES};
use crate::midi::{self, Export};
use crate::pattern::*;
use crate::synth::{KitKind, KITS};
use crate::{drag, PocketParams};

pub const BG: Color32 = Color32::from_rgb(0x0d, 0x0e, 0x12);
pub const PANEL: Color32 = Color32::from_rgb(0x16, 0x18, 0x1f);
pub const PANEL_HI: Color32 = Color32::from_rgb(0x20, 0x23, 0x2d);
pub const GRID: Color32 = Color32::from_rgb(0x2b, 0x2f, 0x3b);
pub const HOLE: Color32 = Color32::from_rgb(0x2a, 0x2d, 0x38);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x7b, 0x81, 0x95);
pub const TEXT: [f32; 3] = [236.0, 238.0, 245.0];
pub const KICK: [f32; 3] = [255.0, 85.0, 61.0];
pub const SNARE: [f32; 3] = [255.0, 207.0, 31.0];
pub const CYMBAL: [f32; 3] = [46.0, 226.0, 166.0];
pub const TOM: [f32; 3] = [58.0, 160.0, 255.0];
/// The editor background as an array, for lamps that sit on a lit button.
const BG_F: [f32; 3] = [13.0, 14.0, 18.0];

const HEADER_H: f32 = 52.0;
const BROWSER_W: f32 = 268.0;
const FOOTER_H: f32 = 162.0;
const HINT_H: f32 = 14.0;
const RULER_H: f32 = 20.0;
const LABEL_W: f32 = 136.0;
const GAP: f32 = 12.0;
const GRAVITY: f32 = 1500.0;

pub fn background() -> Color32 {
    BG
}

pub fn rgba(c: [f32; 3], a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c[0] as u8, c[1] as u8, c[2] as u8, a.clamp(0.0, 255.0) as u8)
}

pub fn family_color(f: Family) -> [f32; 3] {
    match f {
        Family::Kick => KICK,
        Family::Snare => SNARE,
        Family::Cymbal => CYMBAL,
        Family::Tom => TOM,
    }
}

fn lane_color(lane: usize) -> [f32; 3] {
    family_color(LANE_INFO[lane].family)
}

struct Marble {
    lane: usize,
    x: f32,
    y: f32,
    born: f64,
    /// Initial upward speed in px/s.
    speed: f32,
    vel: f32,
}

enum Edit {
    /// Painting pegs in (or, with right-drag, out of) holes.
    Paint { on: bool, last: (usize, usize) },
    /// Pulling a peg: its velocity, or with shift its timing.
    Peg { lane: usize, step: usize, vel: f32, micro: f32, origin: Pos2 },
}

pub struct UiState {
    browser: Browser,
    /// The folder dialog for the My Samples kit, and the last load the editor has seen.
    kit_dialog: (Arc<std::sync::Mutex<Option<String>>>, Arc<std::sync::atomic::AtomicBool>),
    kit_gen: u32,
    /// Which pair of bars the board shows when the pattern is longer than two.
    page: usize,
    edit: Option<Edit>,
    /// Re-roll the generator next frame, once the parameter that asked for it has landed.
    regen: bool,
    /// The pattern revision at the last load or roll, to tell when it has been hand-edited.
    clean_rev: Option<u32>,
    last_seq: Option<[u32; LANES]>,
    marbles: Vec<Marble>,
    pulse: [[f32; MAX_STEPS]; LANES],
    /// The playhead as drawn: it glides between the audio thread's reports.
    head: f32,
    last_time: f64,
    readout: (String, String),
    readout_time: f64,
    toast: (String, f64),
    pub hint: &'static str,
    /// Start the groove playing when one is picked in the browser, if nothing is running.
    pub auto_preview: bool,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            browser: Browser::default(),
            kit_dialog: Default::default(),
            kit_gen: 0,
            page: 0,
            edit: None,
            regen: false,
            clean_rev: None,
            last_seq: None,
            marbles: Vec::new(),
            pulse: [[0.0; MAX_STEPS]; LANES],
            head: -1.0,
            last_time: 0.0,
            readout: (String::new(), String::new()),
            readout_time: -10.0,
            toast: (String::new(), -10.0),
            hint: "",
            auto_preview: true,
        }
    }
}

impl UiState {
    /// Open the browser on a tab ("grooves", "fills", "my midi") filtered to a genre.
    pub fn browse(&mut self, tab: &str, genre: Option<&str>) {
        use crate::browser::Tab;
        self.browser.tab = match tab {
            "fills" => Tab::Fills,
            "my midi" => Tab::MyMidi,
            _ => Tab::Grooves,
        };
        self.browser.genre = genre.map(str::to_owned);
        self.browser.sub = None;
    }

    /// The pattern as it stands counts as untouched: a load or a roll, not a hand edit.
    pub fn mark_clean(&mut self, store: &PatternStore) {
        self.clean_rev = Some(store.rev());
    }
}

pub fn note(state: &mut UiState, ui: &Ui, label: &str, value: String) {
    state.readout = (label.to_owned(), value);
    state.readout_time = ui.input(|i| i.time);
}

pub fn toast(state: &mut UiState, ui: &Ui, text: impl Into<String>) {
    state.toast = (text.into(), ui.input(|i| i.time));
}

pub fn set_once<P: Param>(setter: &ParamSetter, param: &P, value: P::Plain) {
    setter.begin_set_parameter(param);
    setter.set_parameter(param, value);
    setter.end_set_parameter(param);
}

// ----------------------------------------------------------------------------------------------
// Frame

pub fn draw(ui: &mut Ui, params: &Arc<PocketParams>, setter: &ParamSetter, meter: &Arc<Meter>, state: &mut UiState) {
    ui.ctx().request_repaint();
    let now = ui.input(|i| i.time);
    let dt = ((now - state.last_time) as f32).clamp(0.0, 0.1);
    state.last_time = now;
    state.hint = "";
    let store = &params.pattern;
    if state.clean_rev.is_none() {
        state.clean_rev = Some(store.rev());
    }
    if state.regen {
        state.regen = false;
        generator::generate(store, &params.generator());
        state.clean_rev = Some(store.rev());
    }
    follow(state, meter, store.steps(), dt);

    let area = ui.available_rect_before_wrap();
    let painter = ui.painter().clone();
    let header = Rect::from_min_size(area.min, Vec2::new(area.width(), HEADER_H));
    let hint = Rect::from_min_max(Pos2::new(area.left(), area.bottom() - HINT_H), area.max);
    let footer = Rect::from_min_max(Pos2::new(area.left(), hint.top() - 8.0 - FOOTER_H), Pos2::new(area.right(), hint.top() - 8.0));
    let middle = Rect::from_min_max(Pos2::new(area.left(), header.bottom() + 8.0), Pos2::new(area.right(), footer.top() - 10.0));
    let browser = Rect::from_min_max(middle.min, Pos2::new(middle.left() + BROWSER_W, middle.bottom()));
    let board = Rect::from_min_max(Pos2::new(browser.right() + GAP, middle.top()), middle.max);

    draw_header(ui, &painter, header, params, meter, state);
    let mut b = std::mem::take(&mut state.browser);
    b.draw(ui, &painter, browser, params, setter, meter, state);
    state.browser = b;
    draw_board(ui, &painter, board, params, meter, state, dt);
    draw_footer(ui, &painter, footer, params, setter, meter, state);

    // The line of help under the cursor, or the last thing that happened.
    let shown = if now - state.toast.1 < 4.0 { state.toast.0.as_str() } else { state.hint };
    painter.text(hint.left_center(), Align2::LEFT_CENTER, shown, FontId::monospace(9.5), if now - state.toast.1 < 4.0 { rgba(TEXT, 255.0) } else { TEXT_DIM });
}

/// Smooth the playhead, and note which drums have fired since the last frame.
fn follow(state: &mut UiState, meter: &Meter, steps: usize, dt: f32) {
    let reported = meter.playhead.load(Ordering::Relaxed);
    if reported < 0.0 {
        state.head = -1.0;
    } else {
        let tempo = meter.tempo.load(Ordering::Relaxed);
        let advance = dt * tempo / 60.0 * 4.0;
        let span = steps as f32;
        if state.head < 0.0 {
            state.head = reported;
        } else {
            let predicted = (state.head + advance).rem_euclid(span);
            // Shortest signed distance round the loop.
            let mut err = reported - predicted;
            if err > span / 2.0 {
                err -= span;
            } else if err < -span / 2.0 {
                err += span;
            }
            state.head = if err.abs() > 1.5 { reported } else { (predicted + err * 0.25).rem_euclid(span) };
        }
    }
    for row in state.pulse.iter_mut() {
        for p in row.iter_mut() {
            *p = (*p - dt * 4.5).max(0.0);
        }
    }
}

// ----------------------------------------------------------------------------------------------
// Small widgets

pub fn text_button(ui: &mut Ui, painter: &Painter, rect: Rect, label: &str, active: bool, color: [f32; 3]) -> egui::Response {
    let r = ui.interact(rect, ui.id().with(("button", label, rect.min.x as i32, rect.min.y as i32)), Sense::click());
    let fill = if active { rgba(color, 255.0) } else if r.hovered() { PANEL_HI } else { PANEL };
    painter.rect_filled(rect, 4.0, fill);
    painter.rect_stroke(rect, 4.0, Stroke::new(1.0_f32, if active { rgba(color, 255.0) } else { GRID }), StrokeKind::Inside);
    let ink = if active { BG } else if r.hovered() { rgba(TEXT, 255.0) } else { TEXT_DIM };
    painter.text(rect.center(), Align2::CENTER_CENTER, label, FontId::monospace(9.5), ink);
    if r.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    r
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

/// A hardware knob: vertical drag, shift for fine, double-click to reset. Returns whether it moved.
#[allow(clippy::too_many_arguments)]
fn knob(ui: &mut Ui, painter: &Painter, cell: Rect, setter: &ParamSetter, state: &mut UiState, param: &FloatParam, label: &str, color: [f32; 3], hint: &'static str) -> bool {
    let r = ui.interact(cell, ui.id().with(("knob", label)), Sense::click_and_drag());
    let mut moved = false;
    if r.drag_started() {
        setter.begin_set_parameter(param);
    }
    if r.dragged() {
        let fine = ui.input(|i| i.modifiers.shift);
        let speed = if fine { 1.0 / 1200.0 } else { 1.0 / 160.0 };
        let v = param.unmodulated_normalized_value() - r.drag_delta().y * speed;
        setter.set_parameter_normalized(param, v.clamp(0.0, 1.0));
        note(state, ui, label, param.to_string());
        moved = true;
    }
    if r.drag_stopped() {
        setter.end_set_parameter(param);
    }
    if r.double_clicked() {
        set_once(setter, param, param.default_plain_value());
        moved = true;
    }
    if r.hovered() {
        state.hint = hint;
        ui.ctx().set_cursor_icon(CursorIcon::ResizeVertical);
    }

    let norm = param.unmodulated_normalized_value();
    let center = Pos2::new(cell.center().x, cell.top() + 31.0);
    let radius = 22.0;
    let (start, end) = (PI * 0.75, PI * 2.25);
    let lit = r.hovered() || r.dragged();
    painter.circle_filled(center, radius - 5.0, if lit { PANEL_HI } else { PANEL });
    painter.add(Shape::line(arc(center, radius, start, end), Stroke::new(3.0_f32, GRID)));
    let to = start + (end - start) * norm;
    if norm > 0.005 {
        painter.add(Shape::line(arc(center, radius, start, to), Stroke::new(8.0_f32, rgba(color, 28.0))));
        painter.add(Shape::line(arc(center, radius, start, to), Stroke::new(3.0_f32, rgba(color, 255.0))));
    }
    let dir = Vec2::new(to.cos(), to.sin());
    painter.line_segment([center + dir * (radius - 13.0), center + dir * (radius - 6.0)], Stroke::new(2.0_f32, rgba(TEXT, 255.0)));
    painter.text(Pos2::new(cell.center().x, cell.top() + 63.0), Align2::CENTER_CENTER, label, FontId::monospace(9.0), TEXT_DIM);
    painter.text(Pos2::new(cell.center().x, cell.top() + 78.0), Align2::CENTER_CENTER, param.to_string(), FontId::monospace(11.0), rgba(color, 255.0));
    moved
}

pub fn panel(painter: &Painter, rect: Rect) {
    painter.rect_filled(rect, 8.0, PANEL);
    painter.rect_stroke(rect, 8.0, Stroke::new(1.0_f32, GRID), StrokeKind::Inside);
}

fn colored_strip(painter: &Painter, cols: &[(Pos2, Color32, Pos2, Color32)]) {
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

// ----------------------------------------------------------------------------------------------
// Header

fn draw_header(ui: &mut Ui, painter: &Painter, rect: Rect, params: &PocketParams, meter: &Meter, state: &mut UiState) {
    let store = &params.pattern;
    let now = ui.input(|i| i.time);
    // The mark: four pegs in a 2x2, one of each drum colour.
    for (i, c) in [KICK, SNARE, CYMBAL, TOM].into_iter().enumerate() {
        let p = Pos2::new(rect.left() + 9.0 + (i % 2) as f32 * 16.0, rect.top() + 14.0 + (i / 2) as f32 * 16.0);
        painter.circle_filled(p, 9.0, rgba(c, 40.0));
        painter.circle_filled(p, 5.5, rgba(c, 255.0));
    }
    painter.text(Pos2::new(rect.left() + 42.0, rect.top() + 14.0), Align2::LEFT_CENTER, "POCKET", FontId::monospace(20.0), rgba(TEXT, 255.0));
    painter.text(Pos2::new(rect.left() + 42.0, rect.top() + 33.0), Align2::LEFT_CENTER, "groove sequencer", FontId::monospace(9.5), TEXT_DIM);

    // Name of what is on the board, starred once it has been touched.
    let edited = state.clean_rev != Some(store.rev());
    let name = store.name();
    let name = if name.is_empty() { "Untitled".to_owned() } else { name };
    let title = format!("{}{}", name.to_uppercase(), if edited { " *" } else { "" });
    painter.text(Pos2::new(rect.left() + 232.0, rect.top() + 14.0), Align2::LEFT_CENTER, title, FontId::monospace(17.0), rgba(TEXT, 255.0));
    let bars = store.bars();
    painter.text(
        Pos2::new(rect.left() + 232.0, rect.top() + 33.0),
        Align2::LEFT_CENTER,
        format!("{} hits   {} bar{}   {} drums", store.hit_count(), bars, if bars == 1 { "" } else { "s" }, active_lanes(store)),
        FontId::monospace(9.5),
        TEXT_DIM,
    );

    // Right side: play, bars, and a big readout of whatever is being edited, else the tempo.
    let play = Rect::from_min_size(Pos2::new(rect.right() - 82.0, rect.top() + 4.0), Vec2::new(82.0, 40.0));
    let audition = meter.audition.load(Ordering::Relaxed);
    let r = ui.interact(play, ui.id().with("play"), Sense::click());
    if r.clicked() || (!ui.ctx().wants_keyboard_input() && ui.input(|i| i.key_pressed(egui::Key::Space))) {
        meter.audition.store(!audition, Ordering::Relaxed);
    }
    if r.hovered() {
        state.hint = "PLAY RUNS THE GROOVE ON THE PLUGIN'S OWN CLOCK WHEN YOUR HOST IS STOPPED. SPACE DOES THE SAME.";
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    let fill = if audition { rgba(TEXT, 255.0) } else if r.hovered() { PANEL_HI } else { PANEL };
    painter.rect_filled(play, 8.0, fill);
    painter.rect_stroke(play, 8.0, Stroke::new(1.0_f32, if audition { rgba(TEXT, 255.0) } else { GRID }), StrokeKind::Inside);
    let c = play.center() + Vec2::new(-16.0, 0.0);
    let ink = if audition { BG } else { rgba(TEXT, 255.0) };
    if audition {
        painter.rect_filled(Rect::from_center_size(c, Vec2::splat(11.0)), 2.0, ink);
    } else {
        painter.add(Shape::convex_polygon(vec![c + Vec2::new(-5.0, -7.0), c + Vec2::new(-5.0, 7.0), c + Vec2::new(7.0, 0.0)], ink, Stroke::NONE));
    }
    painter.text(play.center() + Vec2::new(14.0, 0.0), Align2::CENTER_CENTER, if audition { "STOP" } else { "PLAY" }, FontId::monospace(11.0), ink);

    let bars_rect = Rect::from_min_size(Pos2::new(play.left() - 8.0 - 138.0, rect.top() + 14.0), Vec2::new(138.0, 24.0));
    for (i, (label, n)) in [("1", 1usize), ("2", 2), ("3", 3), ("4", 4)].into_iter().enumerate() {
        let cell = Rect::from_min_size(bars_rect.min + Vec2::new(i as f32 * 35.0, 0.0), Vec2::new(33.0, 24.0));
        let r = text_button(ui, painter, cell, label, bars == n, TEXT);
        if r.hovered() {
            state.hint = "HOW MANY BARS LONG THE LOOP IS. LONGER REPEATS WHAT IS THERE, SHORTER CUTS IT OFF.";
        }
        if r.clicked() {
            store.set_bars(n);
        }
    }
    painter.text(bars_rect.left_top() + Vec2::new(0.0, -2.0), Align2::LEFT_BOTTOM, "BARS", FontId::monospace(8.0), TEXT_DIM);

    // Record and undo, left of the bars.
    let recording = meter.record.load(Ordering::Relaxed);
    let rec = Rect::from_min_size(Pos2::new(bars_rect.left() - 8.0 - 62.0, rect.top() + 14.0), Vec2::new(62.0, 24.0));
    let undo = Rect::from_min_size(Pos2::new(rec.left() - 4.0 - 52.0, rect.top() + 14.0), Vec2::new(52.0, 24.0));
    let r = text_button(ui, painter, rec, "REC", recording, KICK);
    if recording {
        // A lamp that breathes while armed.
        let pulse = 0.55 + 0.45 * ((now * 5.0).sin() as f32 * 0.5 + 0.5);
        painter.circle_filled(rec.left_center() + Vec2::new(9.0, 0.0), 3.5, rgba(BG_F, 255.0 * pulse));
    } else {
        painter.circle_filled(rec.left_center() + Vec2::new(9.0, 0.0), 3.5, rgba(KICK, 255.0));
    }
    if r.hovered() {
        state.hint = "RECORD: PLAY THE DRUM KEYS ON YOUR KEYBOARD WHILE THE GROOVE RUNS AND THEY LAND ON THE GRID, TIMING KEPT AS A NUDGE. KEY C#7 TOGGLES IT.";
    }
    if r.clicked() {
        if !recording {
            if let Ok(mut u) = meter.undo.lock() {
                *u = Some(store.save());
            }
        }
        meter.record.store(!recording, Ordering::Relaxed);
        if !recording && state.head < 0.0 {
            toast(state, ui, "ARMED. PRESS PLAY, OR START YOUR HOST, THEN PLAY THE KEYS.");
        }
    }
    let can_undo = meter.undo.try_lock().map(|u| u.is_some()).unwrap_or(false);
    let r = text_button(ui, painter, undo, "UNDO", false, TEXT);
    if r.hovered() {
        state.hint = "PUT THE PATTERN BACK AS IT WAS BEFORE THE LAST TAKE. KEY E7 DOES THE SAME.";
    }
    if r.clicked() && can_undo {
        let saved = meter.undo.lock().ok().and_then(|mut u| u.take());
        if let Some(saved) = saved {
            store.load(&saved);
            toast(state, ui, "TAKE UNDONE.");
        }
    }

    let readout = Rect::from_min_max(Pos2::new(undo.left() - 190.0, rect.top()), Pos2::new(undo.left() - 8.0, rect.bottom()));
    if now - state.readout_time < 1.4 {
        painter.text(Pos2::new(readout.right(), readout.top() + 12.0), Align2::RIGHT_CENTER, &state.readout.0, FontId::monospace(9.5), TEXT_DIM);
        painter.text(Pos2::new(readout.right(), readout.top() + 33.0), Align2::RIGHT_CENTER, &state.readout.1, FontId::monospace(22.0), rgba(TEXT, 255.0));
    } else {
        painter.text(Pos2::new(readout.right(), readout.top() + 12.0), Align2::RIGHT_CENTER, "TEMPO", FontId::monospace(9.5), TEXT_DIM);
        painter.text(Pos2::new(readout.right(), readout.top() + 33.0), Align2::RIGHT_CENTER, format!("{:.1}", meter.tempo.load(Ordering::Relaxed)), FontId::monospace(22.0), rgba(TEXT, 255.0));
    }
}

fn active_lanes(store: &PatternStore) -> usize {
    (0..LANES).filter(|&l| (0..store.steps()).any(|s| store.get(l, s).is_on())).count()
}

// ----------------------------------------------------------------------------------------------
// Browser

pub fn thumbnail(painter: &Painter, origin: Pos2, grid: &[[u8; 16]; LANES], accent: bool) {
    // Four rows, one per family of drum, sixteen columns.
    let fams = [Family::Kick, Family::Snare, Family::Cymbal, Family::Tom];
    for (row, fam) in fams.into_iter().enumerate() {
        for step in 0..16 {
            let vel = (0..LANES).filter(|&l| LANE_INFO[l].family == fam).map(|l| grid[l][step]).max().unwrap_or(0);
            let p = origin + Vec2::new(step as f32 * 3.5 + 1.5, row as f32 * 4.2 + 1.5);
            if vel > 0 {
                painter.circle_filled(p, 1.5, rgba(family_color(fam), 110.0 + vel as f32 / 127.0 * 145.0));
            } else {
                painter.circle_filled(p, 0.8, if accent { rgba(TEXT, 40.0) } else { HOLE });
            }
        }
    }
}

// ----------------------------------------------------------------------------------------------
// Dragging clips out

/// Write the clip and hand it to the OS as a file drag; if that cannot start, save it where it
/// can be found instead.
pub fn drag_out(ui: &Ui, state: &mut UiState, store: &PatternStore, bytes: &[u8], stem: &str) {
    match midi::write_file(&midi::drag_dir(), stem, bytes) {
        Ok(path) => {
            if drag::start(&path, store) {
                // The drag swallows the mouse-up, so tell egui the button is not held any more.
                ui.ctx().input_mut(|i| i.pointer = egui::PointerState::default());
                ui.ctx().stop_dragging();
                toast(state, ui, format!("DRAGGING {}.MID", midi::file_stem(stem).to_uppercase()));
            } else {
                save_clip(ui, state, bytes, stem);
            }
        }
        Err(e) => toast(state, ui, format!("COULD NOT WRITE THE CLIP: {e}")),
    }
}

/// Drag a file that already exists, such as one of the user's own clips.
pub fn drag_file(ui: &Ui, state: &mut UiState, path: &std::path::Path, store: &PatternStore) {
    if drag::start(path, store) {
        ui.ctx().input_mut(|i| i.pointer = egui::PointerState::default());
        ui.ctx().stop_dragging();
        toast(state, ui, format!("DRAGGING {}", path.file_name().map(|n| n.to_string_lossy().to_uppercase()).unwrap_or_default()));
    } else {
        toast(state, ui, "THE HOST WOULD NOT TAKE THE DRAG. THE FILE IS ALREADY ON YOUR DISK.");
    }
}

/// Save into ~/Music/Clowd Pocket and show it in the file manager.
fn save_clip(ui: &Ui, state: &mut UiState, bytes: &[u8], stem: &str) {
    match midi::write_file(&midi::save_dir(), stem, bytes) {
        Ok(path) => {
            #[cfg(target_os = "macos")]
            let _ = std::process::Command::new("open").arg("-R").arg(&path).spawn();
            toast(state, ui, format!("SAVED {}", path.display()));
        }
        Err(e) => toast(state, ui, format!("COULD NOT SAVE: {e}")),
    }
}

fn export_current(params: &PocketParams, meter: &Meter, only_lane: Option<usize>) -> (Vec<u8>, String) {
    let name = params.pattern.name();
    let name = if name.is_empty() { "Groove".to_owned() } else { name };
    let stem = match only_lane {
        Some(l) => format!("{name} - {}", LANE_INFO[l].name),
        None => name,
    };
    let bytes = midi::to_midi(
        &params.pattern,
        &Export { map: params.map.value(), feel: params.feel(), bpm: meter.tempo.load(Ordering::Relaxed) as f64, only_lane, name: stem.clone() },
    );
    (bytes, stem)
}

// ----------------------------------------------------------------------------------------------
// Board

struct Geometry {
    /// The first step on the board, and how many it shows.
    base: usize,
    row_h: f32,
    x0: f32,
    top: f32,
    cell_w: f32,
    visible: usize,
}

impl Geometry {
    fn x_of(&self, step: f32) -> f32 {
        self.x0 + (step - self.base as f32) * self.cell_w
    }
    fn y_of(&self, lane: usize) -> f32 {
        self.top + lane as f32 * self.row_h
    }
    fn centre(&self, lane: usize, step: usize, micro: i8) -> Pos2 {
        Pos2::new(self.x_of(step as f32 + 0.5 + micro as f32 / 100.0), self.y_of(lane) + self.row_h / 2.0)
    }
    fn cell_at(&self, pos: Pos2) -> Option<(usize, usize)> {
        let step = ((pos.x - self.x0) / self.cell_w).floor();
        let lane = ((pos.y - self.top) / self.row_h).floor();
        (step >= 0.0 && (step as usize) < self.visible && lane >= 0.0 && (lane as usize) < LANES).then(|| (lane as usize, self.base + step as usize))
    }
}

fn draw_board(ui: &mut Ui, painter: &Painter, rect: Rect, params: &PocketParams, meter: &Meter, state: &mut UiState, dt: f32) {
    let store = &params.pattern;
    let now = ui.input(|i| i.time);
    panel(painter, rect);
    let total = store.steps();
    let pages = total.div_ceil(32);
    // While playing the board follows the playhead; otherwise the page buttons choose.
    if state.head >= 0.0 && pages > 1 {
        state.page = (state.head as usize / 32).min(pages - 1);
    }
    state.page = state.page.min(pages - 1);
    let base = state.page * 32;
    let steps = (total - base).min(32);
    let x0 = rect.left() + LABEL_W;
    let grid_right = rect.right() - 10.0;
    let top = rect.top() + RULER_H + 6.0;
    let row_h = ((rect.height() - RULER_H - 6.0 - 12.0) / LANES as f32).min(48.0);
    let g = Geometry { base, row_h, x0, top, cell_w: (grid_right - x0) / total.min(32) as f32, visible: steps };
    let grid_rect = Rect::from_min_max(Pos2::new(x0, top), Pos2::new(grid_right, top + LANES as f32 * row_h));

    // The audio thread's hits become marbles and pulses.
    let seqs: [u32; LANES] = std::array::from_fn(|l| meter.hit_seq[l].load(Ordering::Relaxed));
    if let Some(last) = state.last_seq {
        for lane in 0..LANES {
            let fired = seqs[lane].wrapping_sub(last[lane]).min(3);
            if fired > 0 && state.head >= 0.0 {
                let vel = meter.hit_vel[lane].load(Ordering::Relaxed);
                let step = (state.head.round() as usize) % total;
                state.pulse[lane][step] = 1.0;
                if (base..base + steps).contains(&step) {
                    let p = g.centre(lane, step, store.get(lane, step).micro());
                    state.marbles.push(Marble { lane, x: p.x, y: p.y, born: now, speed: 150.0 + 320.0 * vel, vel });
                }
            }
        }
    }
    state.last_seq = Some(seqs);
    state.marbles.retain(|m| now - m.born < 1.2);
    let _ = dt;

    // Rows, tinted by the drum, then the beat stripes.
    for lane in 0..LANES {
        let row = Rect::from_min_size(Pos2::new(rect.left() + 6.0, g.y_of(lane) + 1.0), Vec2::new(rect.width() - 12.0, g.row_h - 2.0));
        painter.rect_filled(row, 4.0, rgba(lane_color(lane), if store.audible(lane) { 3.0 } else { 1.0 }));
    }
    for beat in 0..steps / 4 {
        if beat % 2 == 1 {
            let stripe = Rect::from_min_max(Pos2::new(g.x_of((base + beat * 4) as f32), grid_rect.top()), Pos2::new(g.x_of((base + beat * 4 + 4) as f32), grid_rect.bottom()));
            painter.rect_filled(stripe, 0.0, rgba([255.0, 255.0, 255.0], 2.5));
        }
    }
    for local in 0..=steps {
        let s = base + local;
        let bar = s % 16 == 0;
        if s % 4 == 0 {
            painter.line_segment(
                [Pos2::new(g.x_of(s as f32), grid_rect.top() - if bar { 6.0 } else { 0.0 }), Pos2::new(g.x_of(s as f32), grid_rect.bottom())],
                Stroke::new(if bar { 1.5_f32 } else { 1.0_f32 }, if bar { Color32::from_rgb(0x4a, 0x50, 0x62) } else { GRID }),
            );
        }
    }
    for beat in 0..steps / 4 {
        let label = format!("{}", beat % 4 + 1);
        painter.text(
            Pos2::new(g.x_of((base + beat * 4) as f32) + 5.0, rect.top() + RULER_H / 2.0 + 4.0),
            Align2::LEFT_CENTER,
            label,
            FontId::monospace(if beat % 4 == 0 { 11.0 } else { 9.0 }),
            if beat % 4 == 0 { rgba(TEXT, 255.0) } else { TEXT_DIM },
        );
    }

    // Holes and pegs.
    let hover = ui.input(|i| i.pointer.hover_pos()).and_then(|p| g.cell_at(p));
    for lane in 0..LANES {
        let col = lane_color(lane);
        let live = store.audible(lane);
        for step in base..base + steps {
            let cell = store.get(lane, step);
            let home = g.centre(lane, step, 0);
            if !cell.is_on() {
                let beat = step % 4 == 0;
                let lit = hover == Some((lane, step));
                painter.circle_filled(home, if lit { 4.0 } else if beat { 3.0 } else { 2.3 }, if lit { rgba(col, 120.0) } else { HOLE });
                continue;
            }
            let p = g.centre(lane, step, cell.micro());
            let v = cell.vel() as f32 / 127.0;
            let max_r = (g.cell_w * 0.46).min(g.row_h * 0.44);
            let pulse = state.pulse[lane][step];
            let r = (3.0 + (max_r - 3.0) * v) * (1.0 + 0.5 * pulse);
            let a = if live { 255.0 } else { 70.0 };
            if cell.micro() != 0 {
                painter.circle_stroke(home, 2.3, Stroke::new(1.0_f32, rgba(col, 90.0)));
            }
            painter.circle_filled(p, r + 5.0 + 6.0 * pulse, rgba(col, (26.0 + 80.0 * pulse) * a / 255.0));
            painter.circle_filled(p, r, rgba(col, a * (0.55 + 0.45 * v)));
            painter.circle_filled(p - Vec2::new(r * 0.28, r * 0.3), r * 0.28, rgba([255.0, 255.0, 255.0], 70.0 * a / 255.0));
            // A roll shows as pips under the peg.
            for k in 1..cell.ratchet() {
                let off = (k as f32 - (cell.ratchet() as f32 - 1.0) / 2.0) * 4.5;
                painter.circle_filled(p + Vec2::new(off, r + 4.0), 1.6, rgba(TEXT, 220.0 * a / 255.0));
            }
        }
    }

    // Marbles.
    for m in &state.marbles {
        let col = lane_color(m.lane);
        let t = (now - m.born) as f32;
        let floor = g.row_h * 0.5;
        for k in (0..4).rev() {
            let tk = (t - k as f32 * 0.018).max(0.0);
            let dy = -m.speed * tk + 0.5 * GRAVITY * tk * tk;
            if dy > floor {
                continue;
            }
            let fade = (1.0 - t / 1.2).clamp(0.0, 1.0);
            let p = Pos2::new(m.x, m.y + dy);
            let rad = 2.2 + 2.2 * m.vel;
            if k == 0 {
                painter.circle_filled(p, rad + 5.0, rgba(col, 40.0 * fade));
                painter.circle_filled(p, rad, rgba(col, 255.0 * fade));
                painter.circle_filled(p - Vec2::new(rad * 0.3, rad * 0.3), rad * 0.35, rgba([255.0, 255.0, 255.0], 200.0 * fade));
            } else {
                painter.circle_filled(p, rad * (1.0 - k as f32 * 0.2), rgba(col, 70.0 * fade / k as f32));
            }
        }
    }

    // The comb.
    if state.head >= base as f32 && state.head <= (base + steps) as f32 {
        let x = g.x_of(state.head);
        let tail = 46.0;
        colored_strip(
            painter,
            &[
                (Pos2::new((x - tail).max(g.x0), grid_rect.top()), rgba(TEXT, 0.0), Pos2::new((x - tail).max(g.x0), grid_rect.bottom()), rgba(TEXT, 0.0)),
                (Pos2::new(x, grid_rect.top()), rgba(TEXT, 30.0), Pos2::new(x, grid_rect.bottom()), rgba(TEXT, 30.0)),
            ],
        );
        painter.line_segment([Pos2::new(x, grid_rect.top() - 6.0), Pos2::new(x, grid_rect.bottom() + 2.0)], Stroke::new(7.0_f32, rgba(TEXT, 28.0)));
        painter.line_segment([Pos2::new(x, grid_rect.top() - 6.0), Pos2::new(x, grid_rect.bottom() + 2.0)], Stroke::new(1.5_f32, rgba(TEXT, 255.0)));
        painter.add(Shape::convex_polygon(
            vec![Pos2::new(x - 5.0, grid_rect.top() - 11.0), Pos2::new(x + 5.0, grid_rect.top() - 11.0), Pos2::new(x, grid_rect.top() - 4.0)],
            rgba(TEXT, 255.0),
            Stroke::NONE,
        ));
    }

    // Page buttons for patterns longer than two bars.
    if pages > 1 {
        for p in 0..pages {
            let first = p * 2 + 1;
            let last = ((p + 1) * 2).min(store.bars());
            let label = if first == last { format!("BAR {first}") } else { format!("BARS {first}-{last}") };
            let cell = Rect::from_min_size(Pos2::new(rect.right() - 10.0 - (pages - p) as f32 * 76.0, rect.top() + 4.0), Vec2::new(72.0, 16.0));
            let r = text_button(ui, painter, cell, &label, state.page == p, TEXT);
            if r.clicked() {
                state.page = p;
            }
            if r.hovered() {
                state.hint = "THE BOARD SHOWS TWO BARS AT A TIME. IT FOLLOWS THE PLAYHEAD WHILE PLAYING.";
            }
        }
    }

    draw_lane_labels(ui, painter, rect, &g, params, meter, state);
    edit_board(ui, &g, grid_rect, params, state);
}

fn draw_lane_labels(ui: &mut Ui, painter: &Painter, rect: Rect, g: &Geometry, params: &PocketParams, meter: &Meter, state: &mut UiState) {
    let store = &params.pattern;
    for lane in 0..LANES {
        let y = g.y_of(lane);
        let col = lane_color(lane);
        let live = store.audible(lane);
        let name_rect = Rect::from_min_size(Pos2::new(rect.left() + 10.0, y + 2.0), Vec2::new(LABEL_W - 74.0, g.row_h - 4.0));
        let r = ui.interact(name_rect, ui.id().with(("lane", lane)), Sense::click_and_drag());
        if r.hovered() {
            state.hint = "DRAG THE NAME TO TAKE JUST THIS DRUM AS A CLIP.";
            ui.ctx().set_cursor_icon(CursorIcon::Grab);
        }
        if r.drag_started() {
            if store.hit_count() > 0 {
                let (bytes, stem) = export_current(params, meter, Some(lane));
                drag_out(ui, state, store, &bytes, &stem);
            }
        }
        painter.circle_filled(Pos2::new(name_rect.left() + 4.0, y + g.row_h / 2.0), 4.0, rgba(col, if live { 255.0 } else { 70.0 }));
        painter.text(
            Pos2::new(name_rect.left() + 15.0, y + g.row_h / 2.0),
            Align2::LEFT_CENTER,
            LANE_INFO[lane].name,
            FontId::monospace(10.0),
            if r.hovered() { rgba(TEXT, 255.0) } else if live { rgba(TEXT, 210.0) } else { TEXT_DIM },
        );
        // M, S, L.
        for (i, (label, hint)) in [
            ("M", "MUTE THIS DRUM. IT STAYS IN THE PATTERN BUT NOT IN THE OUTPUT OR THE CLIP."),
            ("S", "SOLO THIS DRUM."),
            ("L", "LOCK THIS DRUM: ROLLING THE GENERATOR OR LOADING A GROOVE LEAVES IT ALONE."),
        ]
        .into_iter()
        .enumerate()
        {
            let b = Rect::from_min_size(Pos2::new(rect.left() + LABEL_W - 66.0 + i as f32 * 20.0, y + (g.row_h - 16.0) / 2.0), Vec2::new(17.0, 16.0));
            let on = match i {
                0 => store.muted(lane),
                1 => store.soloed(lane),
                _ => store.locked(lane),
            };
            let br = ui.interact(b, ui.id().with(("lane_button", lane, i)), Sense::click());
            if br.clicked() {
                match i {
                    0 => store.toggle_mute(lane),
                    1 => store.toggle_solo(lane),
                    _ => store.toggle_lock(lane),
                }
            }
            if br.hovered() {
                state.hint = hint;
                ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
            }
            let fill = if on { if i == 2 { rgba(col, 255.0) } else { rgba(TEXT, 255.0) } } else if br.hovered() { PANEL_HI } else { BG };
            painter.rect_filled(b, 3.0, fill);
            painter.rect_stroke(b, 3.0, Stroke::new(1.0_f32, if on { fill } else { GRID }), StrokeKind::Inside);
            painter.text(b.center(), Align2::CENTER_CENTER, label, FontId::monospace(9.0), if on { BG } else { TEXT_DIM });
        }
    }
}

/// Clicks, painting, velocity and timing drags, rolls and erasing on the grid.
fn edit_board(ui: &mut Ui, g: &Geometry, grid: Rect, params: &PocketParams, state: &mut UiState) {
    let store = &params.pattern;
    let r = ui.interact(grid, ui.id().with("grid"), Sense::click_and_drag());
    let pos = r.interact_pointer_pos().or_else(|| r.hover_pos());
    if r.hovered() {
        state.hint = "CLICK A HOLE FOR A PEG, DRAG ACROSS TO PAINT. DRAG A PEG UP OR DOWN FOR VELOCITY, WITH SHIFT FOR TIMING. RIGHT-CLICK FOR A ROLL, RIGHT-DRAG TO ERASE.";
        ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
    }
    let default_vel = 100u8;

    if r.clicked_by(PointerButton::Primary) {
        if let Some((lane, step)) = pos.and_then(|p| g.cell_at(p)) {
            let cell = store.get(lane, step);
            store.set(lane, step, if cell.is_on() { Cell::default() } else { Cell::hit(default_vel) });
        }
    }
    if r.clicked_by(PointerButton::Secondary) {
        if let Some((lane, step)) = pos.and_then(|p| g.cell_at(p)) {
            let cell = store.get(lane, step);
            if cell.is_on() {
                let next = cell.ratchet() % MAX_RATCHET + 1;
                store.set(lane, step, cell.with_ratchet(next));
                note(state, ui, "ROLL", format!("x{next}"));
            }
        }
    }

    if r.drag_started() {
        let origin = ui.input(|i| i.pointer.press_origin()).or(pos);
        if let Some(origin) = origin {
            if let Some((lane, step)) = g.cell_at(origin) {
                let cell = store.get(lane, step);
                if r.drag_started_by(PointerButton::Secondary) {
                    store.set(lane, step, Cell::default());
                    state.edit = Some(Edit::Paint { on: false, last: (lane, step) });
                } else if cell.is_on() {
                    state.edit = Some(Edit::Peg { lane, step, vel: cell.vel() as f32, micro: cell.micro() as f32, origin });
                } else {
                    store.set(lane, step, Cell::hit(default_vel));
                    state.edit = Some(Edit::Paint { on: true, last: (lane, step) });
                }
            }
        }
    }
    if r.dragged() {
        if let (Some(p), Some(edit)) = (r.interact_pointer_pos(), state.edit.as_mut()) {
            let shift = ui.input(|i| i.modifiers.shift);
            match edit {
                Edit::Paint { on, last } => {
                    if let Some((lane, step)) = g.cell_at(p) {
                        // Fill any cells the pointer skipped on the way, along the same drum.
                        let (from, to) = if last.0 == lane { (last.1.min(step), last.1.max(step)) } else { (step, step) };
                        for s in from..=to {
                            let cell = store.get(lane, s);
                            if *on && !cell.is_on() {
                                store.set(lane, s, Cell::hit(default_vel));
                            } else if !*on && cell.is_on() {
                                store.set(lane, s, Cell::default());
                            }
                        }
                        *last = (lane, step);
                    }
                }
                Edit::Peg { lane, step, vel, micro, origin } => {
                    let cell = store.get(*lane, *step);
                    if cell.is_on() {
                        if shift {
                            let m = (*micro + (p.x - origin.x) / g.cell_w * 100.0).round().clamp(-50.0, 50.0) as i8;
                            store.set(*lane, *step, cell.with_micro(m));
                            let value = format!("{:+}%", m);
                            note(state, ui, "NUDGE", value);
                        } else {
                            let v = (*vel + (origin.y - p.y) * 0.8).round().clamp(1.0, 127.0) as u8;
                            store.set(*lane, *step, cell.with_vel(v));
                            let value = format!("{v}");
                            note(state, ui, "VELOCITY", value);
                        }
                    }
                }
            }
        }
    }
    if r.drag_stopped() {
        state.edit = None;
    }
    // The wheel over a peg nudges its velocity.
    if r.hovered() {
        let scroll = ui.input(|i| i.raw_scroll_delta.y);
        if scroll != 0.0 {
            if let Some((lane, step)) = r.hover_pos().and_then(|p| g.cell_at(p)) {
                let cell = store.get(lane, step);
                if cell.is_on() {
                    let v = (cell.vel() as f32 + scroll.signum() * 4.0).clamp(1.0, 127.0) as u8;
                    store.set(lane, step, cell.with_vel(v));
                    note(state, ui, "VELOCITY", format!("{v}"));
                }
            }
        }
    }
}

// ----------------------------------------------------------------------------------------------
// Footer: the generator and the clip

fn draw_footer(ui: &mut Ui, painter: &Painter, rect: Rect, params: &PocketParams, setter: &ParamSetter, meter: &Meter, state: &mut UiState) {
    panel(painter, rect);

    // Style and seed.
    let col_a = Rect::from_min_size(rect.min + Vec2::new(14.0, 26.0), Vec2::new(206.0, rect.height() - 40.0));
    painter.text(col_a.left_top() + Vec2::new(0.0, 6.0), Align2::LEFT_CENTER, "STYLE", FontId::monospace(9.0), TEXT_DIM);
    let style = params.style.value();
    let index = style.index();
    for (dir, label) in [(-1i32, "<"), (1, ">")] {
        let b = Rect::from_min_size(col_a.left_top() + Vec2::new(if dir < 0 { 0.0 } else { 182.0 }, 18.0), Vec2::new(24.0, 30.0));
        let r = text_button(ui, painter, b, label, false, TEXT);
        if r.hovered() {
            state.hint = "PICK THE STYLE THE GENERATOR WRITES IN. THE SWING SETS ITSELF TO MATCH.";
        }
        if r.clicked() {
            let next = STYLES[(index as i32 + dir).rem_euclid(STYLES.len() as i32) as usize];
            set_once(setter, &params.style, next);
            let t = next.template();
            set_once(setter, &params.swing, t.swing);
            set_once(setter, &params.swing_eighths, t.eighths);
            state.regen = true;
        }
    }
    painter.text(col_a.left_top() + Vec2::new(103.0, 33.0), Align2::CENTER_CENTER, style.template().name.to_uppercase(), FontId::monospace(15.0), rgba(TEXT, 255.0));
    painter.text(col_a.left_top() + Vec2::new(0.0, 62.0), Align2::LEFT_CENTER, "SEED", FontId::monospace(9.0), TEXT_DIM);
    let seed = params.seed.value();
    for (dir, label) in [(-1i32, "<"), (1, ">")] {
        let b = Rect::from_min_size(col_a.left_top() + Vec2::new(if dir < 0 { 0.0 } else { 100.0 }, 72.0), Vec2::new(22.0, 24.0));
        let r = text_button(ui, painter, b, label, false, TEXT);
        if r.clicked() {
            set_once(setter, &params.seed, (seed + dir - 1).rem_euclid(9999) + 1);
            state.regen = true;
        }
        if r.hovered() {
            state.hint = "STEP THROUGH SEEDS. THE SAME SEED, STYLE AND SETTINGS ALWAYS ROLL THE SAME GROOVE.";
        }
    }
    painter.text(col_a.left_top() + Vec2::new(61.0, 84.0), Align2::CENTER_CENTER, format!("{seed:04}"), FontId::monospace(15.0), rgba(TEXT, 255.0));
    let roll = Rect::from_min_size(col_a.left_top() + Vec2::new(130.0, 66.0), Vec2::new(76.0, 36.0));
    let r = ui.interact(roll, ui.id().with("roll"), Sense::click());
    if r.hovered() {
        state.hint = "ROLL A NEW GROOVE. LOCKED DRUMS STAY PUT, SO LOCK THE KICK AND ROLL THE REST.";
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    if r.clicked() {
        let t = ui.input(|i| i.time);
        let new = (crate::pattern::hash01((t * 1000.0) as u32, seed as u32, 7) * 9999.0) as i32 + 1;
        set_once(setter, &params.seed, new.clamp(1, 9999));
        state.regen = true;
    }
    painter.rect_filled(roll, 6.0, if r.hovered() { rgba(TEXT, 255.0) } else { rgba(TEXT, 235.0) });
    // A die: three pips on the diagonal.
    let d = Rect::from_center_size(roll.left_center() + Vec2::new(20.0, 0.0), Vec2::splat(20.0));
    painter.rect_stroke(d, 4.0, Stroke::new(1.5_f32, BG), StrokeKind::Inside);
    for k in [-1.0f32, 0.0, 1.0] {
        painter.circle_filled(d.center() + Vec2::new(k * 5.0, -k * 5.0), 1.8, BG);
    }
    painter.text(roll.center() + Vec2::new(14.0, 0.0), Align2::CENTER_CENTER, "ROLL", FontId::monospace(11.0), BG);

    // The five knobs.
    let knobs_left = col_a.right() + 14.0;
    let kw = 92.0;
    let specs: [(&FloatParam, &str, bool, &'static str); 5] = [
        (&params.complexity, "COMPLEX", true, "HOW MANY OF THE STYLE'S OPTIONAL NOTES FIRE: GHOST SNARES, SIXTEENTH HATS, EXTRA KICKS."),
        (&params.dynamics, "DYNAMICS", true, "HOW FAR APART LOUD AND QUIET HITS ARE."),
        (&params.fill, "FILL", true, "ADDS A FILL AT THE END OF THE LAST BAR, LONGER AND BUSIER THE HIGHER IT GOES."),
        (&params.swing, "SWING", false, "PLAYBACK SWING. 50% IS STRAIGHT, 67% IS A TRIPLET SHUFFLE. IT ALSO GOES INTO THE CLIP."),
        (&params.humanize, "HUMANIZE", false, "LOOSENS TIMING AND VELOCITY A LITTLE DIFFERENTLY EACH TIME ROUND. THE CLIP GETS ONE TAKE OF IT."),
    ];
    for (i, (param, label, rolls, hint)) in specs.into_iter().enumerate() {
        let cell = Rect::from_min_size(Pos2::new(knobs_left + i as f32 * kw, rect.top() + 26.0), Vec2::new(kw, 96.0));
        if knob(ui, painter, cell, setter, state, param, label, TEXT, hint) && rolls {
            state.regen = true;
        }
    }
    // Swing pairs sixteenths or eighths.
    let eighths = params.swing_eighths.value();
    let pill = Rect::from_min_size(Pos2::new(knobs_left + 3.0 * kw + kw / 2.0 - 30.0, rect.top() + 122.0), Vec2::new(60.0, 16.0));
    let r = ui.interact(pill, ui.id().with("swing8"), Sense::click());
    if r.clicked() {
        set_once(setter, &params.swing_eighths, !eighths);
    }
    if r.hovered() {
        state.hint = "WHAT SWING BENDS: SIXTEENTHS GIVE A HIP-HOP BOUNCE, EIGHTHS A BLUES SHUFFLE.";
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    painter.rect_filled(pill, 8.0, if r.hovered() { PANEL_HI } else { BG });
    painter.rect_stroke(pill, 8.0, Stroke::new(1.0_f32, GRID), StrokeKind::Inside);
    painter.text(pill.center(), Align2::CENTER_CENTER, if eighths { "8TH" } else { "16TH" }, FontId::monospace(9.0), rgba(TEXT, 255.0));

    // The clip, and what to do with it.
    let col_c = Rect::from_min_max(Pos2::new(knobs_left + 5.0 * kw + 12.0, rect.top() + 12.0), Pos2::new(rect.right() - 12.0, rect.bottom() - 12.0));
    draw_clip(ui, painter, col_c, params, setter, meter, state);
}

fn draw_clip(ui: &mut Ui, painter: &Painter, rect: Rect, params: &PocketParams, setter: &ParamSetter, meter: &Meter, state: &mut UiState) {
    let store = &params.pattern;
    let chip = Rect::from_min_size(rect.min, Vec2::new(rect.width(), 50.0));
    let r = ui.interact(chip, ui.id().with("chip"), Sense::click_and_drag());
    if r.hovered() {
        state.hint = "DRAG THIS CLIP OUT OF THE PLUGIN AND DROP IT ON A MIDI OR DRUM TRACK IN YOUR DAW. CLICK TO SAVE IT AS A .MID FILE.";
        ui.ctx().set_cursor_icon(CursorIcon::Grab);
    }
    let empty = store.hit_count() == 0;
    if r.drag_started() && !empty {
        let (bytes, stem) = export_current(params, meter, None);
        drag_out(ui, state, store, &bytes, &stem);
    }
    if r.clicked() && !empty {
        let (bytes, stem) = export_current(params, meter, None);
        save_clip(ui, state, &bytes, &stem);
    }
    painter.rect_filled(chip, 8.0, if r.hovered() { PANEL_HI } else { BG });
    painter.rect_stroke(chip, 8.0, Stroke::new(1.5_f32, if r.hovered() { rgba(TEXT, 230.0) } else { GRID }), StrokeKind::Inside);
    // The clip as it will land in the DAW: a pegboard in miniature.
    let steps = store.steps();
    let inner = Rect::from_min_max(chip.min + Vec2::new(12.0, 10.0), chip.max - Vec2::new(86.0, 10.0));
    let cw = inner.width() / steps as f32;
    let rh = inner.height() / LANES as f32;
    for lane in 0..LANES {
        for s in 0..steps {
            let cell = store.get(lane, s);
            let p = Pos2::new(inner.left() + (s as f32 + 0.5) * cw, inner.top() + (lane as f32 + 0.5) * rh);
            if cell.is_on() {
                painter.circle_filled(p, (rh * 0.45).max(1.3), rgba(lane_color(lane), 90.0 + cell.vel() as f32 / 127.0 * 165.0));
            } else if s % 4 == 0 {
                painter.circle_filled(p, 0.7, HOLE);
            }
        }
    }
    let hx = chip.right() - 42.0;
    // The grip: six dots, as on a drag handle.
    for (i, j) in [(0, 0), (1, 0), (0, 1), (1, 1), (0, 2), (1, 2)] {
        painter.circle_filled(Pos2::new(hx - 4.0 + i as f32 * 8.0, chip.center().y - 19.0 + j as f32 * 6.0), 1.6, TEXT_DIM);
    }
    painter.text(Pos2::new(hx, chip.center().y + 8.0), Align2::CENTER_CENTER, "DRAG", FontId::monospace(9.5), rgba(TEXT, 255.0));
    painter.text(Pos2::new(hx, chip.center().y + 19.0), Align2::CENTER_CENTER, ".MID", FontId::monospace(9.5), TEXT_DIM);

    // The kit: what you hear from Pocket itself.
    if let Some(chosen) = state.kit_dialog.0.lock().ok().and_then(|mut r| r.take()) {
        if let Ok(mut f) = params.kit_folder.lock() {
            *f = chosen.clone();
        }
        meter.samples.spawn_load(chosen.into());
        set_once(setter, &params.kit, KitKind::Samples);
    }
    let generation = meter.samples.generation.load(Ordering::Relaxed);
    if generation != state.kit_gen {
        state.kit_gen = generation;
        let status = meter.samples.status();
        if !status.is_empty() {
            toast(state, ui, status);
        }
    }
    let kit = params.kit.value();
    let row = Rect::from_min_size(Pos2::new(rect.left(), chip.bottom() + 6.0), Vec2::new(rect.width(), 22.0));
    for (dir, label) in [(-1i32, "<"), (1, ">")] {
        let b = Rect::from_min_size(Pos2::new(if dir < 0 { row.left() } else { row.right() - 24.0 }, row.top()), Vec2::new(24.0, 22.0));
        let r = text_button(ui, painter, b, label, false, TEXT);
        if r.hovered() {
            state.hint = kit.blurb();
        }
        if r.clicked() {
            let index = KITS.iter().position(|k| *k == kit).unwrap_or(0) as i32;
            set_once(setter, &params.kit, KITS[(index + dir).rem_euclid(KITS.len() as i32) as usize]);
        }
    }
    let name = Rect::from_min_max(Pos2::new(row.left() + 28.0, row.top()), Pos2::new(row.right() - 28.0, row.bottom()));
    painter.text(Pos2::new(name.left() + 4.0, name.center().y), Align2::LEFT_CENTER, "KIT", FontId::monospace(8.5), TEXT_DIM);
    painter.text(name.center() + Vec2::new(8.0, 0.0), Align2::CENTER_CENTER, kit.name(), FontId::monospace(12.0), rgba(TEXT, 255.0));
    if ui.rect_contains_pointer(name) {
        state.hint = kit.blurb();
    }

    // Room, tune, decay and level.
    let y = row.bottom() + 6.0;
    let w = (rect.width() - 3.0 * 6.0) / 4.0;
    let slot = |i: usize, y: f32| Rect::from_min_size(Pos2::new(rect.left() + i as f32 * (w + 6.0), y), Vec2::new(w, 20.0));
    for (i, (param, label, hint)) in [
        (&params.room, "ROOM", "HOW MUCH OF A ROOM THE KIT SITS IN. DRAG LEFT AND RIGHT, DOUBLE-CLICK TO RESET."),
        (&params.tune, "TUNE", "TUNES EVERY DRUM UP OR DOWN, IN SEMITONES."),
        (&params.decay, "DECAY", "HOW LONG EVERY DRUM RINGS. SHORT FOR TIGHT AND DEAD, LONG FOR OPEN."),
        (&params.level, "LEVEL", "LEVEL OF THE BUILT-IN KIT."),
    ]
    .into_iter()
    .enumerate()
    {
        hslider(ui, painter, slot(i, y), setter, state, param, label, hint);
    }

    // Clear, map, sound, samples, thru.
    let y = y + 26.0;
    let w5 = (rect.width() - 4.0 * 6.0) / 5.0;
    let slot = |i: usize, y: f32| Rect::from_min_size(Pos2::new(rect.left() + i as f32 * (w5 + 6.0), y), Vec2::new(w5, 20.0));
    let r = text_button(ui, painter, slot(0, y), "CLEAR", false, TEXT);
    if r.hovered() {
        state.hint = "EMPTY THE BOARD (LOCKED DRUMS STAY).";
    }
    if r.clicked() {
        for lane in 0..LANES {
            if !store.locked(lane) {
                store.clear_lane(lane);
            }
        }
        store.set_name("Empty");
        state.clean_rev = Some(store.rev());
    }
    let map = params.map.value();
    let r = text_button(ui, painter, slot(1, y), if map == NoteMap::Gm { "MAP GM" } else { "MAP PADS" }, false, TEXT);
    if r.hovered() {
        state.hint = "WHICH NOTES THE DRUMS PLAY: GENERAL MIDI (KICK 36, SNARE 38...) OR ELEVEN PADS UP FROM C1.";
    }
    if r.clicked() {
        set_once(setter, &params.map, if map == NoteMap::Gm { NoteMap::Pads } else { NoteMap::Gm });
    }
    let sound = params.sound.value();
    let r = text_button(ui, painter, slot(2, y), if sound { "SOUND ON" } else { "SOUND OFF" }, sound, TEXT);
    if r.hovered() {
        state.hint = "THE BUILT-IN KIT FOR AUDITIONING. TURN IT OFF TO ROUTE THE MIDI TO YOUR OWN DRUMS.";
    }
    if r.clicked() {
        set_once(setter, &params.sound, !sound);
    }
    let r = text_button(ui, painter, slot(3, y), "SAMPLES", kit == KitKind::Samples, TEXT);
    if r.hovered() {
        state.hint = "PICK A FOLDER OF YOUR OWN WAV DRUMS FOR THE MY SAMPLES KIT. FILES ARE FILED BY NAME: KICK, SNARE, HAT, HAT OPEN, TOM LOW, CRASH...";
    }
    if r.clicked() {
        pick_folder(&state.kit_dialog.0, &state.kit_dialog.1, "Choose a folder of WAV drum samples");
    }
    let thru = params.thru.value();
    let r = text_button(ui, painter, slot(4, y), if thru { "THRU ON" } else { "THRU OFF" }, thru, TEXT);
    if r.hovered() {
        state.hint = "PASS THE NOTES YOU PLAY ON TO THE NEXT PLUGIN, SO YOU CAN PLAY YOUR SAMPLER BY HAND OVER THE GROOVE.";
    }
    if r.clicked() {
        set_once(setter, &params.thru, !thru);
    }
}

/// A horizontal drag bar for a parameter: drag left and right, shift for fine, double-click to reset.
#[allow(clippy::too_many_arguments)]
fn hslider(ui: &mut Ui, painter: &Painter, rect: Rect, setter: &ParamSetter, state: &mut UiState, param: &FloatParam, label: &str, hint: &'static str) {
    let r = ui.interact(rect, ui.id().with(("hslider", label)), Sense::click_and_drag());
    painter.rect_filled(rect, 4.0, if r.hovered() { PANEL_HI } else { PANEL });
    painter.rect_stroke(rect, 4.0, Stroke::new(1.0_f32, GRID), StrokeKind::Inside);
    let frac = param.unmodulated_normalized_value();
    let fill = Rect::from_min_max(rect.min, Pos2::new(rect.left() + rect.width() * frac, rect.bottom()));
    painter.rect_filled(fill.shrink(1.0), 3.0, rgba(TEXT, 26.0));
    painter.text(rect.left_center() + Vec2::new(5.0, 0.0), Align2::LEFT_CENTER, label, FontId::monospace(7.5), TEXT_DIM);
    painter.text(rect.right_center() - Vec2::new(5.0, 0.0), Align2::RIGHT_CENTER, param.to_string(), FontId::monospace(8.5), rgba(TEXT, 230.0));
    if r.hovered() {
        state.hint = hint;
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
    }
    if r.drag_started() {
        setter.begin_set_parameter(param);
    }
    if r.dragged() {
        let fine = ui.input(|i| i.modifiers.shift);
        let v = param.unmodulated_normalized_value() + r.drag_delta().x / if fine { 1500.0 } else { 120.0 };
        setter.set_parameter_normalized(param, v.clamp(0.0, 1.0));
        note(state, ui, label, param.to_string());
    }
    if r.drag_stopped() {
        setter.end_set_parameter(param);
    }
    if r.double_clicked() {
        set_once(setter, param, param.default_plain_value());
    }
}
