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

#[derive(Clone, PartialEq, Debug)]
pub enum Entry {
    Folder { name: String, count: usize, root: Option<usize> },
    Builtin(usize),
    File { path: PathBuf, place: String },
    AddFolder,
}

type Thumb = [[u8; 16]; LANES];

#[derive(Default)]
pub struct Browser {
    pub path: Vec<String>,
    pub search: String,
    user: Arc<Mutex<UserLib>>,
    /// The folder list last handed to the scanner.
    scanned: Option<Vec<String>>,
    dialog_result: Arc<Mutex<Option<String>>>,
    dialog_open: Arc<AtomicBool>,
    entries: Vec<Entry>,
    key: Option<(Vec<String>, String, u32)>,
    thumbs: HashMap<PathBuf, Option<Box<Thumb>>>,
}

fn stem(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

fn matches(haystack: &str, query: &str) -> bool {
    let h = haystack.to_lowercase();
    query.split_whitespace().all(|word| h.contains(word))
}

/// What to list for a place in the folder tree, or for a search.
pub fn build(path: &[String], search: &str, user: &UserLib) -> Vec<Entry> {
    let query = search.trim().to_lowercase();
    let lib = library();
    let user_count: usize = user.roots.iter().map(Dir::count).sum();
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
    let Some(section) = path.first() else {
        let tree = grooves::tree();
        let mut out: Vec<Entry> = tree.folders.iter().map(|f| Entry::Folder { name: f.name.clone(), count: f.count(), root: None }).collect();
        out.push(Entry::Folder { name: USER_ROOT.into(), count: user_count, root: None });
        return out;
    };
    if section == USER_ROOT {
        return build_user(&path[1..], user);
    }
    let Some(folder) = grooves::tree().find(path) else { return Vec::new() };
    let mut out: Vec<Entry> = folder.folders.iter().map(|f| Entry::Folder { name: f.name.clone(), count: f.count(), root: None }).collect();
    out.extend(folder.items.iter().map(|&i| Entry::Builtin(i)));
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
            self.path = vec![USER_ROOT.into()];
        }

        let (user_version, scanning) = self.user.lock().map(|u| (u.version, u.scanning)).unwrap_or((0, false));
        let key = (self.path.clone(), self.search.clone(), user_version);
        if self.key.as_ref() != Some(&key) {
            self.entries = match self.user.lock() {
                Ok(u) => build(&self.path, &self.search, &u),
                Err(_) => Vec::new(),
            };
            self.key = Some(key);
        }

        // Title and where we are.
        painter.text(inner.left_top() + Vec2::new(0.0, 6.0), Align2::LEFT_CENTER, "BROWSER", FontId::monospace(11.0), rgba(TEXT, 255.0));
        let searching = !self.search.trim().is_empty();
        let place = if searching { "SEARCH".to_owned() } else if self.path.is_empty() { "ALL".into() } else { self.path.join(" / ").to_uppercase() };
        let count = self.entries.iter().filter(|e| !matches!(e, Entry::Folder { .. } | Entry::AddFolder)).count();
        painter.text(inner.right_top() + Vec2::new(0.0, 6.0), Align2::RIGHT_CENTER, if scanning { "SCANNING".to_owned() } else if count > 0 { format!("{count}") } else { String::new() }, FontId::monospace(10.0), TEXT_DIM);

        let back = Rect::from_min_size(inner.left_top() + Vec2::new(0.0, 20.0), Vec2::new(26.0, 22.0));
        let can_back = !self.path.is_empty() || searching;
        if can_back {
            let r = text_button(ui, painter, back, "<", false, TEXT);
            if r.clicked() {
                if searching {
                    self.search.clear();
                } else {
                    self.path.pop();
                }
            }
            if r.hovered() {
                state.hint = "GO UP A FOLDER.";
            }
        } else {
            painter.rect_stroke(back, 4.0, Stroke::new(1.0_f32, GRID), StrokeKind::Inside);
        }
        // Long paths show their last two folders.
        let crumb = if !searching && self.path.len() > 2 {
            format!(".. / {}", self.path[self.path.len() - 2..].join(" / ").to_uppercase())
        } else {
            place
        };
        painter.text(back.right_center() + Vec2::new(8.0, 0.0), Align2::LEFT_CENTER, crumb, FontId::monospace(9.5), TEXT_DIM);

        // Search.
        let search = Rect::from_min_size(inner.left_top() + Vec2::new(0.0, 48.0), Vec2::new(inner.width(), 22.0));
        painter.rect_filled(search, 4.0, BG);
        painter.rect_stroke(search, 4.0, Stroke::new(1.0_f32, GRID), StrokeKind::Inside);
        let edit_rect = search.shrink2(Vec2::new(8.0, 1.0));
        let mut child = ui.new_child(UiBuilder::new().max_rect(edit_rect));
        child.visuals_mut().extreme_bg_color = Color32::TRANSPARENT;
        child.visuals_mut().widgets.inactive.bg_stroke = Stroke::NONE;
        child.visuals_mut().widgets.hovered.bg_stroke = Stroke::NONE;
        child.visuals_mut().selection.stroke = Stroke::NONE;
        let response = child.add(
            TextEdit::singleline(&mut self.search)
                .hint_text("search grooves")
                .font(FontId::monospace(10.0))
                .text_color(rgba(TEXT, 255.0))
                .frame(false)
                .desired_width(edit_rect.width())
                .vertical_align(egui::Align::Center),
        );
        if response.hovered() {
            state.hint = "TYPE TO SEARCH EVERY GROOVE, FILL AND FILE BY NAME OR FOLDER.";
        }

        let list = Rect::from_min_max(inner.left_top() + Vec2::new(0.0, 78.0), inner.max);
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
                                let sub = if searching {
                                    format!("{} bpm  {}", g.bpm, g.path.last().map(|s| s.to_lowercase()).unwrap_or_default())
                                } else if g.kind == Kind::Fill {
                                    format!("{} bar{}  fill", g.bars(), if g.bars() == 1 { "" } else { "s" })
                                } else {
                                    format!("{} bpm  {} bar{}", g.bpm, g.bars(), if g.bars() == 1 { "" } else { "s" })
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
                        let sub: String = sub.chars().take(30).collect();
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
            Action::Load(i) => match self.entries[i].clone() {
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
            },
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

    #[test]
    fn the_root_lists_sections_and_my_midi() {
        let e = build(&[], "", &user());
        let names: Vec<_> = e.iter().filter_map(|e| if let Entry::Folder { name, .. } = e { Some(name.as_str()) } else { None }).collect();
        assert_eq!(names, ["GROOVES", "FILLS", USER_ROOT]);
        assert!(matches!(e.last(), Some(Entry::Folder { count: 4, .. })));
    }

    #[test]
    fn genre_folders_list_sub_folders_then_grooves() {
        let e = build(&["GROOVES".into(), "Rock".into()], "", &user());
        assert!(e.iter().all(|e| matches!(e, Entry::Folder { .. })), "Rock holds sub-folders");
        assert!(e.len() >= 4);
        let e = build(&["GROOVES".into(), "Rock".into(), "Metal".into()], "", &user());
        assert!(e.iter().all(|e| matches!(e, Entry::Builtin(_))));
        let names: Vec<_> = e.iter().map(|e| if let Entry::Builtin(i) = e { library()[*i].name.as_str() } else { "" }).collect();
        assert!(names.contains(&"Blast Beat"));
    }

    #[test]
    fn my_midi_offers_add_and_lists_roots_then_files() {
        let u = user();
        let e = build(&[USER_ROOT.into()], "", &u);
        assert_eq!(e[0], Entry::AddFolder);
        assert!(matches!(&e[1], Entry::Folder { name, count: 4, root: Some(0) } if name == "Mine"));
        let e = build(&[USER_ROOT.into(), "Mine".into()], "", &u);
        assert_eq!(e.len(), 3, "two sub-folders and one file");
        assert!(matches!(&e[2], Entry::File { path, .. } if path.ends_with("top.mid")));
        let e = build(&[USER_ROOT.into(), "Mine".into(), "Rock".into()], "", &u);
        assert_eq!(e.len(), 2);
        assert!(build(&[USER_ROOT.into(), "Nope".into()], "", &u).is_empty());
    }

    #[test]
    fn search_spans_builtin_grooves_and_user_files() {
        let u = user();
        let e = build(&[], "blast", &u);
        assert!(e.iter().any(|e| matches!(e, Entry::Builtin(i) if library()[*i].name == "Blast Beat")));
        let e = build(&[], "swing", &u);
        assert!(e.iter().any(|e| matches!(e, Entry::File { path, .. } if path.ends_with("swing.mid"))));
        assert!(e.iter().any(|e| matches!(e, Entry::Builtin(i) if library()[*i].name == "Swing Ride")));
        // Every word has to match, in any order, against the name or the folders.
        let e = build(&[], "metal blast", &u);
        assert!(!e.is_empty() && e.iter().all(|e| matches!(e, Entry::Builtin(_))));
        assert!(build(&[], "zzzzqq", &u).is_empty());
        assert!(build(&[], "  ", &u).len() == 3, "blank search is the root");
    }

    #[test]
    fn search_is_capped() {
        let files: Vec<String> = (0..1000).map(|i| format!("loop{i}.mid")).collect();
        let refs: Vec<&str> = files.iter().map(String::as_str).collect();
        let u = UserLib { roots: vec![dir("Big", &refs, vec![])], version: 1, scanning: false };
        assert_eq!(build(&[], "loop", &u).len(), MAX_RESULTS);
    }

    #[test]
    fn fills_open_as_a_folder_of_folders() {
        let e = build(&["FILLS".into()], "", &user());
        assert!(e.len() >= 5 && e.iter().all(|e| matches!(e, Entry::Folder { .. })));
    }
}
