//! The groove browser: folders of built-in grooves and fills, plus any folders of `.mid` files
//! the user adds. Rows are drawn only while visible and their thumbnails are cached, so the list
//! scrolls the same however many files are in it.

use crate::engine::Meter;
use crate::grooves::{self, library, Kind};
use crate::import::{self, Dir};
use crate::midi::{self, Export};
use crate::pattern::*;
use crate::ui::*;
use crate::PocketParams;
use nih_plug::prelude::*;
use nih_plug_egui::egui::{self, Align2, Color32, CursorIcon, FontId, Pos2, Rect, ScrollArea, Sense, Stroke, StrokeKind, TextEdit, Ui, UiBuilder, Vec2};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const ROW_H: f32 = 38.0;
pub const USER_ROOT: &str = "MY MIDI";
const MAX_RESULTS: usize = 400;

#[derive(Default)]
pub struct UserLib {
    pub roots: Vec<Dir>,
    /// Counts up each time a scan finishes, so the entry list knows to rebuild.
    pub version: u32,
    pub scanning: bool,
}

/// The three places to browse. Grooves and fills are one flat, scrolling list each, filtered by
/// genre; My MIDI follows the folders the user added.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Tab {
    #[default]
    Grooves,
    Fills,
    MyMidi,
}

#[derive(Clone, PartialEq, Debug)]
pub enum Entry {
    /// A label in the flat list where the genre changes.
    Header(String),
    Folder { name: String, count: usize, root: Option<usize> },
    Builtin(usize),
    File { path: PathBuf, place: String },
    AddFolder,
}

type Thumb = [[u8; 16]; LANES];

#[derive(Default)]
pub struct Browser {
    pub tab: Tab,
    /// The genre filter on the grooves and fills tabs; `None` is all of them.
    pub genre: Option<String>,
    /// A sub-genre within the chosen genre, picked from the tag chips that appear under it. These
    /// exist so the list can be narrowed with the mouse alone, for hosts that keep the keyboard.
    pub sub: Option<String>,
    /// Where the user is inside My MIDI.
    pub path: Vec<String>,
    pub search: String,
    user: Arc<Mutex<UserLib>>,
    /// The folder list last handed to the scanner.
    scanned: Option<Vec<String>>,
    dialog_result: Arc<Mutex<Option<String>>>,
    dialog_open: Arc<AtomicBool>,
    entries: Vec<Entry>,
    key: Option<(Tab, Option<String>, Option<String>, Vec<String>, String, u32)>,
    thumbs: HashMap<PathBuf, Option<Box<Thumb>>>,
}

fn stem(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

fn matches(haystack: &str, query: &str) -> bool {
    let h = haystack.to_lowercase();
    query.split_whitespace().all(|word| h.contains(word))
}

/// The genres of a section, in library order.
pub fn genres(tab: Tab) -> Vec<String> {
    let section = match tab {
        Tab::Grooves => "GROOVES",
        Tab::Fills => "FILLS",
        Tab::MyMidi => return Vec::new(),
    };
    grooves::tree().find(&[section.into()]).map(|f| f.folders.iter().map(|g| g.name.clone()).collect()).unwrap_or_default()
}

/// The sub-genres inside a genre, in library order. Empty where a genre has none (the fills).
pub fn sub_genres(tab: Tab, genre: &str) -> Vec<String> {
    let section = match tab {
        Tab::Grooves => "GROOVES",
        Tab::Fills => "FILLS",
        Tab::MyMidi => return Vec::new(),
    };
    grooves::tree().find(&[section.into(), genre.into()]).map(|g| g.folders.iter().map(|f| f.name.clone()).collect()).unwrap_or_default()
}

/// What to list: a flat run of grooves or fills (with a label where the genre changes when all of
/// them are showing), the user's folders, or the results of a search across everything.
pub fn build(tab: Tab, genre: Option<&str>, sub: Option<&str>, path: &[String], search: &str, user: &UserLib) -> Vec<Entry> {
    let query = search.trim().to_lowercase();
    let lib = library();
    if !query.is_empty() {
        let mut out = Vec::new();
        for (i, g) in lib.iter().enumerate() {
            if matches(&format!("{} {}", g.name, g.path.join(" ")), &query) {
                out.push(Entry::Builtin(i));
            }
        }
        for root in &user.roots {
            let mut files = Vec::new();
            root.all_files(&mut files);
            for f in files {
                let place = f.parent().map(|p| p.display().to_string()).unwrap_or_default();
                if matches(&format!("{} {}", stem(&f), place), &query) {
                    out.push(Entry::File { place: root.name.clone(), path: f });
                }
            }
        }
        out.truncate(MAX_RESULTS);
        return out;
    }
    let section = match tab {
        Tab::MyMidi => return build_user(path, user),
        Tab::Grooves => "GROOVES",
        Tab::Fills => "FILLS",
    };
    let Some(root) = grooves::tree().find(&[section.into()]) else { return Vec::new() };
    let mut out = Vec::new();
    for g in root.folders.iter().filter(|g| genre.is_none_or(|want| g.name == want)) {
        if genre.is_none() {
            out.push(Entry::Header(g.name.clone()));
        }
        let mut items = Vec::new();
        match sub.filter(|_| genre.is_some()) {
            Some(want) => {
                if let Some(f) = g.folders.iter().find(|f| f.name == want) {
                    f.all_items(&mut items);
                }
            }
            None => g.all_items(&mut items),
        }
        out.extend(items.into_iter().map(Entry::Builtin));
    }
    out
}

fn build_user(path: &[String], user: &UserLib) -> Vec<Entry> {
    let dir = match path.split_first() {
        None => {
            let mut out = vec![Entry::AddFolder];
            out.extend(user.roots.iter().enumerate().map(|(i, r)| Entry::Folder { name: r.name.clone(), count: r.count(), root: Some(i) }));
            return out;
        }
        Some((root, rest)) => match user.roots.iter().find(|r| &r.name == root).and_then(|r| r.find(rest)) {
            Some(d) => d,
            None => return Vec::new(),
        },
    };
    let mut out: Vec<Entry> = dir.dirs.iter().map(|d| Entry::Folder { name: d.name.clone(), count: d.count(), root: None }).collect();
    out.extend(dir.files.iter().map(|f| Entry::File { path: f.clone(), place: dir.name.clone() }));
    out
}

enum Action {
    Enter(String),
    Load(usize),
    Drag(usize),
    Remove(usize),
    Add,
}

/// Show the system's folder chooser on a background thread. The chosen path lands in `result`.
pub fn pick_folder(result: &Arc<Mutex<Option<String>>>, open: &Arc<AtomicBool>, prompt: &'static str) {
    #[cfg(target_os = "macos")]
    {
        if open.swap(true, Ordering::SeqCst) {
            return;
        }
        let (result, open) = (result.clone(), open.clone());
        std::thread::spawn(move || {
            let out = std::process::Command::new("osascript").args(["-e", &format!("POSIX path of (choose folder with prompt \"{prompt}\")")]).output();
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
    let _ = (result, open, prompt);
}

/// A genre's name as it fits on a filter chip.
fn short_genre(name: &str) -> String {
    match name {
        "Drum & Bass" => "DNB".into(),
        "Breakcore & IDM" => "IDM".into(),
        "Funk & Soul" => "FUNK".into(),
        "Disco & Boogie" => "DISCO".into(),
        "Dubstep & Bass" => "DUBSTEP".into(),
        "Latin & World" => "LATIN".into(),
        "Jazz & Shuffle" => "JAZZ".into(),
        "Breakbeat" | "Breaks" => "BREAKS".into(),
        "Kick & Snare" => "KICK+SN".into(),
        other => other.to_uppercase().chars().take(8).collect(),
    }
}

fn folder_glyph(painter: &egui::Painter, at: Pos2, color: Color32) {
    painter.rect_filled(Rect::from_min_size(at + Vec2::new(0.0, 0.0), Vec2::new(8.0, 4.0)), 1.5, color);
    painter.rect_filled(Rect::from_min_size(at + Vec2::new(0.0, 3.0), Vec2::new(18.0, 12.0)), 2.5, color);
}

impl Browser {
    fn rescan(&mut self, folders: Vec<String>) {
        self.scanned = Some(folders.clone());
        let user = self.user.clone();
        if let Ok(mut u) = user.lock() {
            u.scanning = true;
        }
        std::thread::spawn(move || {
            let roots: Vec<Dir> = folders.iter().map(|f| import::scan(Path::new(f))).collect();
            if let Ok(mut u) = user.lock() {
                u.roots = roots;
                u.version += 1;
                u.scanning = false;
            }
        });
    }

    /// Ask the OS for a folder without blocking the editor.
    fn pick_folder(&self) {
        pick_folder(&self.dialog_result, &self.dialog_open, "Choose a folder of MIDI drum clips");
    }

    fn thumb(thumbs: &mut HashMap<PathBuf, Option<Box<Thumb>>>, path: &Path) -> Option<Thumb> {
        thumbs.entry(path.to_path_buf()).or_insert_with(|| import::read_clip(path).ok().map(|c| Box::new(c.thumbnail()))).as_deref().copied()
    }

    pub fn draw(&mut self, ui: &mut Ui, painter: &egui::Painter, rect: Rect, params: &PocketParams, setter: &ParamSetter, meter: &Meter, state: &mut UiState) {
        panel(painter, rect);
        let inner = rect.shrink(10.0);

        // Folders the user added: scan on first draw, and again when the list changes.
        let folders = params.folders.lock().map(|f| f.clone()).unwrap_or_default();
        if self.scanned.as_ref() != Some(&folders) {
            self.rescan(folders.clone());
        }
        if let Some(chosen) = self.dialog_result.lock().ok().and_then(|mut r| r.take()) {
            if let Ok(mut f) = params.folders.lock() {
                if !f.contains(&chosen) {
                    f.push(chosen);
                }
            }
            self.tab = Tab::MyMidi;
            self.path.clear();
        }

        let (user_version, scanning) = self.user.lock().map(|u| (u.version, u.scanning)).unwrap_or((0, false));
        let key = (self.tab, self.genre.clone(), self.sub.clone(), self.path.clone(), self.search.clone(), user_version);
        if self.key.as_ref() != Some(&key) {
            self.entries = match self.user.lock() {
                Ok(u) => build(self.tab, self.genre.as_deref(), self.sub.as_deref(), &self.path, &self.search, &u),
                Err(_) => Vec::new(),
            };
            self.key = Some(key);
        }

        // Title, and the preview switch.
        painter.text(inner.left_top() + Vec2::new(0.0, 6.0), Align2::LEFT_CENTER, "BROWSER", FontId::monospace(11.0), rgba(TEXT, 255.0));
        let searching = !self.search.trim().is_empty();
        let count = self.entries.iter().filter(|e| matches!(e, Entry::Builtin(_) | Entry::File { .. })).count();
        let auto = Rect::from_min_size(inner.right_top() + Vec2::new(-52.0, -2.0), Vec2::new(52.0, 17.0));
        let r = text_button(ui, painter, auto, "AUTO", state.auto_preview, TEXT);
        if r.clicked() {
            state.auto_preview = !state.auto_preview;
        }
        if r.hovered() {
            state.hint = "PREVIEW: PICKING A GROOVE OR FILL STARTS IT PLAYING ON THE PLUGIN'S OWN CLOCK WHEN YOUR HOST IS STOPPED. THE NOTES GO OUT AS MIDI, SO A SAMPLER AFTER POCKET PLAYS THEM.";
        }
        painter.text(inner.right_top() + Vec2::new(-60.0, 6.0), Align2::RIGHT_CENTER, if scanning { "SCANNING".to_owned() } else { format!("{count}") }, FontId::monospace(10.0), TEXT_DIM);

        // Tabs.
        let tab_w = (inner.width() - 2.0 * 4.0) / 3.0;
        for (i, (tab, label, hint)) in [
            (Tab::Grooves, "GROOVES", "FULL GROOVES, ONE LONG LIST. FILTER BY GENRE BELOW, OR SEARCH."),
            (Tab::Fills, "FILLS", "FILLS THAT GO ON THE END OF WHATEVER IS ON THE BOARD."),
            (Tab::MyMidi, "MY MIDI", "YOUR OWN FOLDERS OF .MID FILES."),
        ]
        .into_iter()
        .enumerate()
        {
            let cell = Rect::from_min_size(inner.left_top() + Vec2::new(i as f32 * (tab_w + 4.0), 22.0), Vec2::new(tab_w, 20.0));
            let r = text_button(ui, painter, cell, label, self.tab == tab && !searching, TEXT);
            if r.clicked() && self.tab != tab {
                self.tab = tab;
                self.genre = None;
                self.sub = None;
                self.search.clear();
            } else if r.clicked() {
                self.search.clear();
            }
            if r.hovered() {
                state.hint = hint;
            }
        }
        let mut y = 48.0;

        if self.tab == Tab::MyMidi {
            // Folders: a way back up, and where you are.
            let back = Rect::from_min_size(inner.left_top() + Vec2::new(0.0, y), Vec2::new(26.0, 20.0));
            if !self.path.is_empty() {
                let r = text_button(ui, painter, back, "<", false, TEXT);
                if r.clicked() {
                    self.path.pop();
                }
                if r.hovered() {
                    state.hint = "GO UP A FOLDER.";
                }
            } else {
                painter.rect_stroke(back, 4.0, Stroke::new(1.0_f32, GRID), StrokeKind::Inside);
            }
            let place = if self.path.is_empty() { "ALL FOLDERS".to_owned() } else { self.path.join(" / ").to_uppercase() };
            let crumb = if self.path.len() > 2 { format!(".. / {}", self.path[self.path.len() - 2..].join(" / ").to_uppercase()) } else { place };
            painter.text(back.right_center() + Vec2::new(8.0, 0.0), Align2::LEFT_CENTER, crumb, FontId::monospace(9.5), TEXT_DIM);
            y += 26.0;
        } else {
            // Genre filter: ALL and one chip per genre, five to a row.
            let names = genres(self.tab);
            let columns = 5usize;
            let chip_w = (inner.width() - (columns - 1) as f32 * 3.0) / columns as f32;
            let labels: Vec<(Option<&String>, String)> = std::iter::once((None, "ALL".to_owned())).chain(names.iter().map(|n| (Some(n), short_genre(n)))).collect();
            for (i, (name, label)) in labels.iter().enumerate() {
                let cell = Rect::from_min_size(
                    inner.left_top() + Vec2::new((i % columns) as f32 * (chip_w + 3.0), y + (i / columns) as f32 * 20.0),
                    Vec2::new(chip_w, 17.0),
                );
                let selected = self.genre.as_ref() == *name;
                let r = text_button(ui, painter, cell, label, selected && !searching, TEXT);
                if r.clicked() {
                    self.genre = name.cloned();
                    self.sub = None;
                    self.search.clear();
                }
                if r.hovered() {
                    state.hint = "SHOW ONLY THIS GENRE. SUB-GENRES SUCH AS METAL OR AMEN ARE TAGS: SEARCH FOR THEM.";
                }
            }
            y += labels.len().div_ceil(columns) as f32 * 20.0 + 3.0;

            // Tag chips for the sub-genres of the chosen genre: click one to narrow, again to undo.
            if let Some(genre) = self.genre.clone() {
                let subs = sub_genres(self.tab, &genre);
                let columns = 4usize;
                let chip_w = (inner.width() - (columns - 1) as f32 * 3.0) / columns as f32;
                for (i, name) in subs.iter().enumerate() {
                    let cell = Rect::from_min_size(
                        inner.left_top() + Vec2::new((i % columns) as f32 * (chip_w + 3.0), y + (i / columns) as f32 * 20.0),
                        Vec2::new(chip_w, 17.0),
                    );
                    let label: String = name.to_uppercase().chars().take(10).collect();
                    let selected = self.sub.as_ref() == Some(name);
                    let r = text_button(ui, painter, cell, &label, selected && !searching, SNARE);
                    if r.clicked() {
                        self.sub = if selected { None } else { Some(name.clone()) };
                        self.search.clear();
                    }
                    if r.hovered() {
                        state.hint = "A SUB-GENRE TAG. CLICK TO SHOW ONLY THESE, CLICK AGAIN FOR THE WHOLE GENRE.";
                    }
                }
                if !subs.is_empty() {
                    y += subs.len().div_ceil(columns) as f32 * 20.0 + 3.0;
                }
            }
        }

        // Search.
        let search = Rect::from_min_size(inner.left_top() + Vec2::new(0.0, y), Vec2::new(inner.width(), 22.0));
        painter.rect_filled(search, 4.0, BG);
        painter.rect_stroke(search, 4.0, Stroke::new(1.0_f32, if searching { rgba(TEXT, 160.0) } else { GRID }), StrokeKind::Inside);
        let edit_rect = search.shrink2(Vec2::new(8.0, 1.0));
        let mut child = ui.new_child(UiBuilder::new().max_rect(edit_rect));
        child.visuals_mut().extreme_bg_color = Color32::TRANSPARENT;
        child.visuals_mut().widgets.inactive.bg_stroke = Stroke::NONE;
        child.visuals_mut().widgets.hovered.bg_stroke = Stroke::NONE;
        child.visuals_mut().selection.stroke = Stroke::NONE;
        let response = child.add(
            TextEdit::singleline(&mut self.search)
                .hint_text("type to search (if your host allows)")
                .font(FontId::monospace(10.0))
                .text_color(rgba(TEXT, 255.0))
                .frame(false)
                .desired_width(edit_rect.width())
                .vertical_align(egui::Align::Center),
        );
        if response.hovered() {
            state.hint = "TYPE TO SEARCH EVERY GROOVE, FILL AND FILE AT ONCE: BY NAME, GENRE OR SUB-GENRE (E.G. METAL, AMEN, HYPER, DEEP).";
        }
        y += 28.0;

        let list = Rect::from_min_max(inner.left_top() + Vec2::new(0.0, y), inner.max);
        let current = params.pattern.name();
        let mut action: Option<Action> = None;
        let lib = library();
        let Browser { entries, thumbs, path, search, .. } = self;
        let id = ("browser", path.join("/"), search.trim().is_empty());
        let mut child = ui.new_child(UiBuilder::new().max_rect(list));
        child.set_clip_rect(list);
        if entries.is_empty() {
            painter.text(list.center_top() + Vec2::new(0.0, 30.0), Align2::CENTER_CENTER, if searching { "NOTHING MATCHES" } else { "EMPTY" }, FontId::monospace(10.0), TEXT_DIM);
        }
        ScrollArea::vertical().id_salt(id).drag_to_scroll(false).auto_shrink([false, false]).show_rows(&mut child, ROW_H, entries.len(), |ui, range| {
            ui.spacing_mut().item_spacing.y = 3.0;
            for index in range {
                let entry = &entries[index];
                let (row, r) = ui.allocate_exact_size(Vec2::new(ui.available_width() - 8.0, ROW_H), Sense::click_and_drag());
                let painter = ui.painter_at(row);
                let hot = r.hovered();
                match entry {
                    Entry::Header(name) => {
                        let y = row.center().y + 6.0;
                        painter.text(Pos2::new(row.left() + 4.0, y), Align2::LEFT_CENTER, name.to_uppercase(), FontId::monospace(9.5), rgba(TEXT, 200.0));
                        painter.line_segment([Pos2::new(row.left() + 4.0, y + 9.0), Pos2::new(row.right() - 4.0, y + 9.0)], Stroke::new(1.0_f32, GRID));
                    }
                    Entry::Folder { name, count, root } => {
                        painter.rect_filled(row, 5.0, if hot { PANEL_HI } else { BG });
                        folder_glyph(&painter, row.left_center() + Vec2::new(10.0, -8.0), if hot { rgba(TEXT, 230.0) } else { TEXT_DIM });
                        painter.text(row.left_center() + Vec2::new(38.0, 0.0), Align2::LEFT_CENTER, name.to_uppercase(), FontId::monospace(10.5), rgba(TEXT, if hot { 255.0 } else { 220.0 }));
                        let right = row.right_center() - Vec2::new(10.0, 0.0);
                        if let Some(i) = root {
                            let x = Rect::from_center_size(right - Vec2::new(10.0, 0.0), Vec2::splat(18.0));
                            let xr = ui.interact(x, ui.id().with(("remove", index)), Sense::click());
                            painter.text(x.center(), Align2::CENTER_CENTER, "x", FontId::monospace(11.0), if xr.hovered() { rgba(KICK, 255.0) } else { TEXT_DIM });
                            if xr.hovered() {
                                state.hint = "STOP LISTING THIS FOLDER. THE FILES ARE NOT TOUCHED.";
                            }
                            if xr.clicked() {
                                action = Some(Action::Remove(*i));
                            }
                            painter.text(right - Vec2::new(26.0, 0.0), Align2::RIGHT_CENTER, format!("{count}"), FontId::monospace(9.5), TEXT_DIM);
                        } else {
                            painter.text(right - Vec2::new(14.0, 0.0), Align2::RIGHT_CENTER, format!("{count}"), FontId::monospace(9.5), TEXT_DIM);
                            painter.text(right, Align2::RIGHT_CENTER, ">", FontId::monospace(11.0), TEXT_DIM);
                        }
                        if hot {
                            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                            state.hint = "OPEN THIS FOLDER.";
                        }
                        if r.clicked() && action.is_none() {
                            action = Some(Action::Enter(name.clone()));
                        }
                    }
                    Entry::AddFolder => {
                        painter.rect_filled(row, 5.0, if hot { rgba(TEXT, 255.0) } else { rgba(TEXT, 230.0) });
                        painter.text(row.center(), Align2::CENTER_CENTER, "+ ADD A FOLDER OF .MID FILES", FontId::monospace(10.0), BG);
                        if hot {
                            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                            state.hint = "POINT POCKET AT A FOLDER OF MIDI DRUM CLIPS, E.G. ONES YOU HAVE DRAGGED OUT OF ANOTHER DRUM PLUGIN. THEY ARE READ IN PLACE.";
                        }
                        if r.clicked() {
                            action = Some(Action::Add);
                        }
                    }
                    Entry::Builtin(_) | Entry::File { .. } => {
                        let (name, sub, thumb, selected) = match entry {
                            Entry::Builtin(i) => {
                                let g = &lib[*i];
                                // The genre and sub-genre ride along as tags on every row.
                                let tags = g.path[1..].join(" / ").to_lowercase();
                                let sub = if g.kind == Kind::Fill {
                                    format!("{} bar{}  {tags}", g.bars(), if g.bars() == 1 { "" } else { "s" })
                                } else {
                                    format!("{} bpm  {tags}", g.bpm)
                                };
                                (g.name.clone(), sub, Some(*g.thumbnail()), current == g.name)
                            }
                            Entry::File { path, place } => {
                                let name = stem(path);
                                (name.clone(), format!("midi  {}", place.to_lowercase()), Self::thumb(thumbs, path), current == name)
                            }
                            _ => unreachable!(),
                        };
                        painter.rect_filled(row, 5.0, if selected { PANEL_HI } else if hot { Color32::from_rgb(0x1b, 0x1e, 0x27) } else { BG });
                        if selected {
                            painter.rect_stroke(row, 5.0, Stroke::new(1.0_f32, rgba(TEXT, 200.0)), StrokeKind::Inside);
                        }
                        match thumb {
                            Some(t) => thumbnail(&painter, row.left_top() + Vec2::new(6.0, 6.0), &t, selected || hot),
                            None => {
                                painter.text(row.left_top() + Vec2::new(32.0, 19.0), Align2::CENTER_CENTER, "?", FontId::monospace(12.0), TEXT_DIM);
                            }
                        }
                        let shown: String = if name.chars().count() > 22 { name.chars().take(21).collect::<String>() + ".." } else { name };
                        painter.text(row.left_top() + Vec2::new(70.0, 12.0), Align2::LEFT_CENTER, shown.to_uppercase(), FontId::monospace(10.0), rgba(TEXT, if selected || hot { 255.0 } else { 215.0 }));
                        let sub: String = sub.chars().take(33).collect();
                        painter.text(row.left_top() + Vec2::new(70.0, 27.0), Align2::LEFT_CENTER, sub, FontId::monospace(8.5), TEXT_DIM);
                        if hot {
                            state.hint = match entry {
                                Entry::Builtin(i) if lib[*i].kind == Kind::Fill => "CLICK TO PUT THIS FILL ON THE END OF THE PATTERN. DRAG IT OUT TO TAKE IT AS A CLIP.",
                                _ => "CLICK TO LOAD. DRAG IT OUT OF THE PLUGIN AND DROP IT ON A TRACK IN YOUR DAW.",
                            };
                            ui.ctx().set_cursor_icon(CursorIcon::Grab);
                        }
                        if r.clicked() {
                            action = Some(Action::Load(index));
                        }
                        if r.drag_started() {
                            action = Some(Action::Drag(index));
                        }
                    }
                }
            }
        });

        // Apply what was clicked, now that the list is no longer borrowed.
        let Some(action) = action else { return };
        match action {
            Action::Enter(name) => {
                self.search.clear();
                self.path.push(name);
            }
            Action::Add => self.pick_folder(),
            Action::Remove(i) => {
                let name = self.user.lock().ok().and_then(|u| u.roots.get(i).map(|r| r.path.display().to_string()));
                if let (Some(name), Ok(mut f)) = (name, params.folders.lock()) {
                    f.retain(|p| p.trim_end_matches('/') != name.trim_end_matches('/'));
                }
            }
            Action::Load(i) => {
                if state.auto_preview && meter.playhead.load(Ordering::Relaxed) < 0.0 {
                    meter.audition.store(true, Ordering::Relaxed);
                }
                match self.entries[i].clone() {
                    Entry::Builtin(g) => {
                        let g = &library()[g];
                        g.load(&params.pattern, true);
                        if g.kind == Kind::Groove {
                            set_once(setter, &params.swing, g.swing);
                            set_once(setter, &params.swing_eighths, g.eighths);
                            note(state, ui, "LOADED", format!("{} bpm", g.bpm));
                        } else {
                            note(state, ui, "FILL", g.name.to_uppercase());
                        }
                        state.mark_clean(&params.pattern);
                    }
                    Entry::File { path, .. } => match import::read_clip(&path) {
                        Ok(clip) => {
                            clip.load(&params.pattern, true, &stem(&path));
                            state.mark_clean(&params.pattern);
                            let skipped = if clip.skipped > 0 { format!(", {} NOTES NOT DRUMS", clip.skipped) } else { String::new() };
                            toast(state, ui, format!("LOADED {} - {} BAR{}{}", stem(&path).to_uppercase(), clip.bars, if clip.bars == 1 { "" } else { "S" }, skipped));
                            if let Some(bpm) = clip.bpm {
                                note(state, ui, "FILE TEMPO", format!("{bpm:.0} bpm"));
                            }
                        }
                        Err(e) => toast(state, ui, format!("COULD NOT READ {}: {}", stem(&path).to_uppercase(), e.to_uppercase())),
                    },
                    _ => {}
                }
            }
            Action::Drag(i) => match self.entries[i].clone() {
                Entry::Builtin(g) => {
                    let g = &library()[g];
                    let tmp = PatternStore::default();
                    g.write(&tmp, false);
                    let feel = Feel { swing: g.swing, eighths: g.eighths, humanize: 0.0 };
                    let bytes = midi::to_midi(&tmp, &Export { map: params.map.value(), feel, bpm: meter.tempo.load(Ordering::Relaxed) as f64, only_lane: None, name: g.name.clone() });
                    drag_out(ui, state, &tmp, &bytes, &g.name);
                }
                Entry::File { path, .. } => match import::read_clip(&path) {
                    // Hand over the original file, so the DAW gets exactly what is on disk.
                    Ok(clip) => {
                        let tmp = PatternStore::default();
                        clip.load(&tmp, false, &stem(&path));
                        drag_file(ui, state, &path, &tmp);
                    }
                    Err(e) => toast(state, ui, format!("COULD NOT READ {}: {}", stem(&path).to_uppercase(), e.to_uppercase())),
                },
                _ => {}
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str, files: &[&str], dirs: Vec<Dir>) -> Dir {
        Dir { name: name.into(), path: PathBuf::from("/x").join(name), dirs, files: files.iter().map(|f| PathBuf::from("/x").join(name).join(f)).collect() }
    }

    fn user() -> UserLib {
        UserLib {
            roots: vec![dir("Mine", &["top.mid"], vec![dir("Rock", &["fast.mid", "slow.mid"], vec![]), dir("Jazz", &["swing.mid"], vec![])])],
            version: 1,
            scanning: false,
        }
    }

    fn builtins(e: &[Entry]) -> Vec<&'static str> {
        e.iter().filter_map(|e| if let Entry::Builtin(i) = e { Some(library()[*i].name.as_str()) } else { None }).collect()
    }

    #[test]
    fn grooves_are_one_flat_list_with_a_label_at_each_genre() {
        let e = build(Tab::Grooves, None, None, &[], "", &user());
        let headers: Vec<_> = e.iter().filter_map(|e| if let Entry::Header(h) = e { Some(h.as_str()) } else { None }).collect();
        assert_eq!(headers, genres(Tab::Grooves));
        assert!(headers.len() >= 10);
        // No folders anywhere: just labels and grooves, and every groove (no fills) appears once.
        assert!(e.iter().all(|e| matches!(e, Entry::Header(_) | Entry::Builtin(_))));
        let names = builtins(&e);
        assert_eq!(names.len(), library().iter().filter(|g| g.kind == grooves::Kind::Groove).count());
        assert!(names.contains(&"Blast Beat") && names.contains(&"Hyper Roll") && !names.contains(&"Snare Build"));
        // Sub-genres are not folders, so they never appear as entries of their own.
        assert!(matches!(e[0], Entry::Header(_)));
    }

    #[test]
    fn a_genre_filter_shows_all_of_it_with_no_labels() {
        let e = build(Tab::Grooves, Some("Rock"), None, &[], "", &user());
        assert!(e.iter().all(|e| matches!(e, Entry::Builtin(_))));
        let names = builtins(&e);
        // Every sub-genre of rock is in the one list.
        for n in ["Backbeat", "Half-Time", "D-Beat", "Blast Beat", "Straight Pop"] {
            assert_eq!(names.contains(&n), n != "Straight Pop", "{n}");
        }
        assert!(e.iter().all(|e| matches!(e, Entry::Builtin(i) if library()[*i].path[1] == "Rock")));
        let hh = build(Tab::Grooves, Some("Hip-Hop"), None, &[], "", &user());
        assert!(builtins(&hh).len() >= 15, "the hip-hop sub-genres share one list");
        assert!(build(Tab::Grooves, Some("Nope"), None, &[], "", &user()).is_empty());
    }

    #[test]
    fn sub_genre_tags_narrow_a_genre_without_the_keyboard() {
        assert_eq!(sub_genres(Tab::Grooves, "Rock"), ["Classic", "Half-Time", "Punk", "Metal"]);
        assert!(sub_genres(Tab::Fills, "Toms").is_empty());
        assert!(sub_genres(Tab::MyMidi, "Rock").is_empty());
        let metal = build(Tab::Grooves, Some("Rock"), Some("Metal"), &[], "", &user());
        let names = builtins(&metal);
        assert!(names.contains(&"Blast Beat") && names.contains(&"Double Kick"));
        assert!(!names.contains(&"Backbeat") && !names.contains(&"D-Beat"));
        assert!(names.len() >= 6);
        // Every sub-genre is reachable, and together they cover the whole genre.
        let all = builtins(&build(Tab::Grooves, Some("Hip-Hop"), None, &[], "", &user())).len();
        let parts: usize = sub_genres(Tab::Grooves, "Hip-Hop").iter().map(|s| builtins(&build(Tab::Grooves, Some("Hip-Hop"), Some(s), &[], "", &user())).len()).sum();
        assert_eq!(all, parts);
        // A tag without a genre does nothing: the whole list shows.
        let e = build(Tab::Grooves, None, Some("Metal"), &[], "", &user());
        assert!(builtins(&e).len() > builtins(&metal).len());
    }

    #[test]
    fn fills_are_their_own_flat_list() {
        let e = build(Tab::Fills, None, None, &[], "", &user());
        assert!(builtins(&e).iter().all(|n| library().iter().find(|g| g.name == *n).unwrap().kind == grooves::Kind::Fill));
        assert!(builtins(&e).contains(&"Tom Run Down"));
        assert!(!genres(Tab::Fills).is_empty());
        assert!(genres(Tab::MyMidi).is_empty());
        let toms = build(Tab::Fills, Some("Toms"), None, &[], "", &user());
        assert!(builtins(&toms).len() >= 5 && !builtins(&toms).contains(&"Snare Build"));
    }

    #[test]
    fn my_midi_offers_add_and_lists_roots_then_files() {
        let u = user();
        let e = build(Tab::MyMidi, None, None, &[], "", &u);
        assert_eq!(e[0], Entry::AddFolder);
        assert!(matches!(&e[1], Entry::Folder { name, count: 4, root: Some(0) } if name == "Mine"));
        let e = build(Tab::MyMidi, None, None, &["Mine".into()], "", &u);
        assert_eq!(e.len(), 3, "two sub-folders and one file");
        assert!(matches!(&e[2], Entry::File { path, .. } if path.ends_with("top.mid")));
        let e = build(Tab::MyMidi, None, None, &["Mine".into(), "Rock".into()], "", &u);
        assert_eq!(e.len(), 2);
        assert!(build(Tab::MyMidi, None, None, &["Nope".into()], "", &u).is_empty());
    }

    #[test]
    fn every_genre_fits_on_a_chip() {
        for tab in [Tab::Grooves, Tab::Fills] {
            let all = genres(tab);
            let shorts: Vec<_> = all.iter().map(|g| short_genre(g)).collect();
            assert!(shorts.iter().all(|s| s.len() <= 8), "{shorts:?}");
            let mut unique = shorts.clone();
            unique.sort();
            unique.dedup();
            assert_eq!(unique.len(), shorts.len(), "two genres share a chip label: {shorts:?}");
        }
    }

    #[test]
    fn search_spans_builtin_grooves_and_user_files() {
        let u = user();
        let e = build(Tab::Grooves, None, None, &[], "blast", &u);
        assert!(e.iter().any(|e| matches!(e, Entry::Builtin(i) if library()[*i].name == "Blast Beat")));
        let e = build(Tab::Grooves, None, None, &[], "swing", &u);
        assert!(e.iter().any(|e| matches!(e, Entry::File { path, .. } if path.ends_with("swing.mid"))));
        assert!(e.iter().any(|e| matches!(e, Entry::Builtin(i) if library()[*i].name == "Swing Ride")));
        // Every word has to match, in any order, against the name or the folders.
        let e = build(Tab::Grooves, None, None, &[], "metal blast", &u);
        assert!(!e.is_empty() && e.iter().all(|e| matches!(e, Entry::Builtin(_))));
        assert!(build(Tab::Grooves, None, None, &[], "zzzzqq", &u).is_empty());
        assert!(!build(Tab::Grooves, None, None, &[], "  ", &u).is_empty(), "a blank search is just the list");
    }

    #[test]
    fn search_is_capped() {
        let files: Vec<String> = (0..1000).map(|i| format!("loop{i}.mid")).collect();
        let refs: Vec<&str> = files.iter().map(String::as_str).collect();
        let u = UserLib { roots: vec![dir("Big", &refs, vec![])], version: 1, scanning: false };
        assert_eq!(build(Tab::Grooves, None, None, &[], "loop", &u).len(), MAX_RESULTS);
    }
}
