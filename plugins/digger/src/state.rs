//! What is selected in each slot, and the atomics the editor and the audio thread share.
//! Selections are saved with the project by name, so they survive a library that has changed.

use crate::engine::SLOTS;
use crate::library::{Library, Share};
use nih_plug::prelude::*;
use nih_plug::params::persist::PersistentField;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;

pub struct SlotInfo {
    pub name: &'static str,
    /// The General MIDI note the slot answers to.
    pub note: u8,
    /// Category names that fit it, best first.
    pub hints: &'static [&'static str],
    /// 0 lows, 1 snaps, 2 metal, 3 everything else.
    pub tone: usize,
}

pub const SLOT_INFO: [SlotInfo; SLOTS] = [
    SlotInfo { name: "KICK", note: 36, hints: &["kick", "kicks", "bd"], tone: 0 },
    SlotInfo { name: "SNARE", note: 38, hints: &["snare", "snares", "sd"], tone: 1 },
    SlotInfo { name: "CLAP", note: 39, hints: &["clap", "claps"], tone: 1 },
    SlotInfo { name: "HAT", note: 42, hints: &["hat", "hats", "hihat", "closed"], tone: 2 },
    SlotInfo { name: "OPEN", note: 46, hints: &["hat-open", "openhat", "open"], tone: 2 },
    SlotInfo { name: "TOM", note: 45, hints: &["tom", "toms"], tone: 0 },
    SlotInfo { name: "PERC", note: 37, hints: &["perc", "percussion"], tone: 3 },
    SlotInfo { name: "CYMBAL", note: 49, hints: &["cymbal", "crash", "ride"], tone: 2 },
];

/// Which slot a drum note plays: the General MIDI drum map folded onto the eight slots.
pub fn note_slot(note: u8) -> Option<usize> {
    Some(match note {
        35 | 36 => 0,
        38 | 40 => 1,
        39 => 2,
        42 | 44 => 3,
        46 => 4,
        41 | 43 | 45 | 47 | 48 | 50 => 5,
        37 | 54 | 56 | 58 | 69 | 70 | 75 | 76 | 77 | 80 | 81 => 6,
        49 | 51 | 52 | 53 | 55 | 57 | 59 => 7,
        _ => return None,
    })
}

pub fn note_name(note: u8) -> String {
    const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    format!("{}{}", NAMES[(note % 12) as usize], note as i32 / 12 - 2)
}

/// Keys from C7 that browse without touching the mouse or the window: with Reaper's virtual MIDI
/// keyboard, or any controller, they steer the plugin while the groove plays.
pub const NAV_BASE: u8 = 96;
pub const NAV_KEYS: u8 = 11;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nav {
    SlotPrev,
    SlotNext,
    SamplePrev,
    SampleNext,
    SampleRandom,
    CatPrev,
    CatNext,
    Audition,
    KitPrev,
    KitNext,
    KitRoll,
}

impl Nav {
    pub fn from_note(note: u8) -> Option<Nav> {
        use Nav::*;
        Some(match note.checked_sub(NAV_BASE)? {
            0 => SlotPrev,
            1 => SlotNext,
            2 => SamplePrev,
            3 => SampleNext,
            4 => SampleRandom,
            5 => CatPrev,
            6 => CatNext,
            7 => Audition,
            8 => KitPrev,
            9 => KitNext,
            10 => KitRoll,
            _ => return None,
        })
    }
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct SavedSlot {
    pub cat: String,
    pub file: String,
}

/// A kit: the sample chosen in every slot, by name.
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Kit {
    pub name: String,
    pub slots: Vec<SavedSlot>,
}

/// No kit is selected: the slots hold a mix.
pub const NO_KIT: usize = usize::MAX;

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Saved {
    pub root: String,
    pub slots: Vec<SavedSlot>,
}

pub struct Shared {
    pub lib: Share,
    /// The folder the library is loaded from.
    pub root: Mutex<String>,
    pending: Mutex<Saved>,
    pub cat: [AtomicUsize; SLOTS],
    pub idx: [AtomicUsize; SLOTS],
    pub focus: AtomicUsize,
    /// Bumped by the editor to sound a slot: always, or only when the transport is stopped.
    pub audition: [AtomicU32; SLOTS],
    pub browse: [AtomicU32; SLOTS],
    /// Set by the audio thread on each hit and cleared by the editor, which smooths it.
    pub hit: [AtomicF32; SLOTS],
    /// Playhead through the slot's newest voice, or -1.
    pub pos: [AtomicF32; SLOTS],
    pub out_peak: AtomicF32,
    pub playing: AtomicBool,
    /// Saved kits, kept in the `.digger.json` beside the samples.
    pub kits: Mutex<Vec<Kit>>,
    pub kit: AtomicUsize,
    /// One bit set: only samples with that tag are stepped to and rolled. Zero: everything.
    pub filter: AtomicU64,
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            lib: Share::default(),
            root: Mutex::new(String::new()),
            pending: Mutex::new(Saved::default()),
            cat: std::array::from_fn(|_| AtomicUsize::new(0)),
            idx: std::array::from_fn(|_| AtomicUsize::new(0)),
            focus: AtomicUsize::new(0),
            audition: std::array::from_fn(|_| AtomicU32::new(0)),
            browse: std::array::from_fn(|_| AtomicU32::new(0)),
            hit: std::array::from_fn(|_| AtomicF32::new(0.0)),
            pos: std::array::from_fn(|_| AtomicF32::new(-1.0)),
            out_peak: AtomicF32::new(0.0),
            playing: AtomicBool::new(false),
            kits: Mutex::new(Vec::new()),
            kit: AtomicUsize::new(NO_KIT),
            filter: AtomicU64::new(0),
        }
    }
}

impl Shared {
    pub fn selection(&self, slot: usize) -> (usize, usize) {
        (self.cat[slot].load(Ordering::Relaxed), self.idx[slot].load(Ordering::Relaxed))
    }

    pub fn focus(&self) -> usize {
        self.focus.load(Ordering::Relaxed).min(SLOTS - 1)
    }

    pub fn set_focus(&self, slot: usize) {
        self.focus.store(slot.min(SLOTS - 1), Ordering::Relaxed);
    }

    pub fn move_focus(&self, delta: isize) {
        let f = (self.focus() as isize + delta).rem_euclid(SLOTS as isize);
        self.focus.store(f as usize, Ordering::Relaxed);
    }

    fn passes(&self, lib: &Library, cat: usize, idx: usize) -> bool {
        let mask = self.filter.load(Ordering::Relaxed);
        mask == 0 || lib.sample(cat, idx).is_some_and(|s| s.tags.load(Ordering::Relaxed) & mask != 0)
    }

    /// Switch the tag filter to `bit`, or off if it is already the one in force.
    pub fn toggle_filter(&self, bit: usize) {
        let m = 1u64 << bit;
        let now = self.filter.load(Ordering::Relaxed);
        self.filter.store(if now == m { 0 } else { m }, Ordering::Relaxed);
    }

    /// Step through a slot's samples, wrapping at the ends and skipping what the filter hides.
    pub fn step_sample(&self, slot: usize, delta: isize, lib: &Library) {
        let (cat, idx) = self.selection(slot);
        let n = lib.cats.get(cat).map_or(0, |c| c.samples.len()) as isize;
        if n == 0 {
            return;
        }
        if self.filter.load(Ordering::Relaxed) == 0 {
            self.idx[slot].store((idx as isize + delta).rem_euclid(n) as usize, Ordering::Relaxed);
            return;
        }
        let dir = if delta < 0 { -1 } else { 1 };
        let mut cur = idx as isize;
        for _ in 0..delta.abs() {
            match (1..=n).map(|j| (cur + dir * j).rem_euclid(n) as usize).find(|&k| self.passes(lib, cat, k)) {
                Some(k) => cur = k as isize,
                None => break,
            }
        }
        self.idx[slot].store(cur as usize, Ordering::Relaxed);
    }

    pub fn set_sample(&self, slot: usize, idx: usize, lib: &Library) {
        let n = lib.cats.get(self.selection(slot).0).map_or(0, |c| c.samples.len());
        if n > 0 {
            self.idx[slot].store(idx.min(n - 1), Ordering::Relaxed);
        }
    }

    /// A different sample from a number the caller supplies; never the one already chosen.
    pub fn random_sample(&self, slot: usize, seed: u32, lib: &Library) {
        let (cat, idx) = self.selection(slot);
        let n = lib.cats.get(cat).map_or(0, |c| c.samples.len());
        let ok = |k: &usize| *k != idx && self.passes(lib, cat, *k);
        let m = (0..n).filter(ok).count();
        if m > 0 {
            if let Some(k) = (0..n).filter(ok).nth(seed as usize % m) {
                self.idx[slot].store(k, Ordering::Relaxed);
            }
        }
    }

    pub fn step_cat(&self, slot: usize, delta: isize, lib: &Library) {
        let n = lib.cats.len();
        if n > 0 {
            let cat = (self.selection(slot).0 as isize + delta).rem_euclid(n as isize) as usize;
            self.set_cat(slot, cat, lib);
        }
    }

    /// Move a slot to a category, on its first sample the filter allows.
    pub fn set_cat(&self, slot: usize, cat: usize, lib: &Library) {
        if let Some(c) = lib.cats.get(cat) {
            self.cat[slot].store(cat, Ordering::Relaxed);
            let first = (0..c.samples.len()).find(|&k| self.passes(lib, cat, k)).unwrap_or(0);
            self.idx[slot].store(first, Ordering::Relaxed);
        }
    }

    // ------------------------------------------------------------------------------------------
    // Kits

    /// Today's selection in every slot, as a kit.
    pub fn kit_from_selection(&self, name: &str, lib: &Library) -> Kit {
        let slots = (0..SLOTS)
            .map(|s| {
                let (cat, idx) = self.selection(s);
                match lib.cats.get(cat) {
                    Some(c) => SavedSlot { cat: c.name.clone(), file: c.samples.get(idx).map(|x| x.name.clone()).unwrap_or_default() },
                    None => SavedSlot::default(),
                }
            })
            .collect();
        Kit { name: name.to_owned(), slots }
    }

    /// Put every slot on the sample the kit names. Slots whose sample is gone are left as they are.
    /// Never blocks: if the editor holds the kit list at this instant, nothing happens.
    pub fn apply_kit(&self, i: usize, lib: &Library) -> bool {
        let Ok(kits) = self.kits.try_lock() else { return false };
        let Some(kit) = kits.get(i) else { return false };
        for (s, saved) in kit.slots.iter().enumerate().take(SLOTS) {
            let Some(cat) = lib.find_cat(&saved.cat) else { continue };
            let Some(idx) = lib.cats[cat].samples.iter().position(|x| x.name == saved.file) else { continue };
            self.cat[s].store(cat, Ordering::Relaxed);
            self.idx[s].store(idx, Ordering::Relaxed);
        }
        self.kit.store(i, Ordering::Relaxed);
        self.sound_the_kit();
        true
    }

    /// Browsing a kit speaks as a kick, snare and hat together, like a bar of a beat.
    fn sound_the_kit(&self) {
        for slot in [0, 1, 3] {
            self.request_browse(slot);
        }
    }

    pub fn step_kit(&self, delta: isize, lib: &Library) {
        let n = self.kits.try_lock().map(|k| k.len()).unwrap_or(0) as isize;
        if n == 0 {
            return;
        }
        let cur = self.kit.load(Ordering::Relaxed);
        let next = if cur == NO_KIT { if delta > 0 { 0 } else { n - 1 } } else { (cur as isize + delta).rem_euclid(n) };
        self.apply_kit(next as usize, lib);
    }

    /// A new kit: every slot takes a random sample from its own category, from among those the
    /// tag filter lets through (or from all of them, if none do).
    pub fn roll_kit(&self, seed: u32, lib: &Library) {
        let mut x = seed | 1;
        for s in 0..SLOTS {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            let cat = self.selection(s).0;
            let n = lib.cats.get(cat).map_or(0, |c| c.samples.len());
            let m = (0..n).filter(|&k| self.passes(lib, cat, k)).count();
            if m > 0 {
                if let Some(k) = (0..n).filter(|&k| self.passes(lib, cat, k)).nth(x as usize % m) {
                    self.idx[s].store(k, Ordering::Relaxed);
                }
            }
        }
        self.kit.store(NO_KIT, Ordering::Relaxed);
        self.sound_the_kit();
    }

    /// Write the user's tags and the kits beside the samples.
    pub fn save_sidecar(&self) -> Result<(), String> {
        let lib = self.lib.library().ok_or("no library")?;
        let root = self.root.lock().map(|r| r.clone()).unwrap_or_default();
        let kits = self.kits.lock().map(|k| k.clone()).unwrap_or_default();
        crate::tags::save_sidecar(std::path::Path::new(&root), &crate::tags::Sidecar { tags: crate::tags::user_tags(&lib), kits })
    }

    pub fn request_browse(&self, slot: usize) {
        self.browse[slot].fetch_add(1, Ordering::Relaxed);
    }

    pub fn request_audition(&self, slot: usize) {
        self.audition[slot].fetch_add(1, Ordering::Relaxed);
    }

    /// Run a navigation command on the focused slot. `seed` feeds the random pick.
    pub fn navigate(&self, nav: Nav, seed: u32, lib: &Library) {
        let slot = self.focus();
        match nav {
            Nav::SlotPrev => self.move_focus(-1),
            Nav::SlotNext => self.move_focus(1),
            Nav::SamplePrev => {
                self.step_sample(slot, -1, lib);
                self.request_browse(slot);
            }
            Nav::SampleNext => {
                self.step_sample(slot, 1, lib);
                self.request_browse(slot);
            }
            Nav::SampleRandom => {
                self.random_sample(slot, seed, lib);
                self.request_browse(slot);
            }
            Nav::CatPrev => {
                self.step_cat(slot, -1, lib);
                self.request_browse(slot);
            }
            Nav::CatNext => {
                self.step_cat(slot, 1, lib);
                self.request_browse(slot);
            }
            Nav::Audition => self.request_audition(slot),
            Nav::KitPrev => self.step_kit(-1, lib),
            Nav::KitNext => self.step_kit(1, lib),
            Nav::KitRoll => self.roll_kit(seed, lib),
        }
    }

    pub fn set_root(&self, root: &str) {
        if let Ok(mut r) = self.root.lock() {
            *r = root.to_owned();
        }
        remember_folder(root);
    }

    /// Settle the slots on a freshly loaded library. `keep` holds on to what is selected now (or
    /// what the project saved); otherwise every slot starts on the category that suits it.
    pub fn adopt(&self, lib: &Library, keep: bool) {
        let fresh = if keep { self.snapshot() } else { Saved { root: self.root.lock().map(|r| r.clone()).unwrap_or_default(), slots: vec![] } };
        if let Ok(mut p) = self.pending.lock() {
            *p = fresh;
        }
        self.apply_pending(lib);
    }

    /// Point every slot at what the project saved, or at the category that suits it.
    pub fn apply_pending(&self, lib: &Library) {
        let saved = self.pending.lock().map(|p| p.clone()).unwrap_or_default();
        for (s, info) in SLOT_INFO.iter().enumerate() {
            let want = saved.slots.get(s);
            let cat = want
                .filter(|w| !w.cat.is_empty())
                .and_then(|w| lib.find_cat(&w.cat))
                .unwrap_or_else(|| lib.default_cat(info.hints, s));
            let idx = want
                .filter(|w| !w.file.is_empty())
                .and_then(|w| lib.cats.get(cat).and_then(|c| c.samples.iter().position(|x| x.name == w.file)))
                .unwrap_or(0);
            self.cat[s].store(cat, Ordering::Relaxed);
            self.idx[s].store(idx, Ordering::Relaxed);
        }
    }

    fn snapshot(&self) -> Saved {
        let mut saved = self.pending.lock().map(|p| p.clone()).unwrap_or_default();
        saved.root = self.root.lock().map(|r| r.clone()).unwrap_or_default();
        saved.slots.resize(SLOTS, SavedSlot::default());
        if let Some(lib) = self.lib.library() {
            for s in 0..SLOTS {
                let (cat, idx) = self.selection(s);
                if let Some(c) = lib.cats.get(cat) {
                    saved.slots[s] = SavedSlot { cat: c.name.clone(), file: c.samples.get(idx).map(|x| x.name.clone()).unwrap_or_default() };
                }
            }
        }
        saved
    }
}

impl<'a> PersistentField<'a, String> for Shared {
    fn set(&self, new_value: String) {
        if let Ok(saved) = serde_json::from_str::<Saved>(&new_value) {
            if let Ok(mut root) = self.root.lock() {
                *root = saved.root.clone();
            }
            if let Ok(mut p) = self.pending.lock() {
                *p = saved;
            }
        }
    }

    fn map<F, R>(&self, f: F) -> R
    where
        F: Fn(&String) -> R,
    {
        f(&serde_json::to_string(&self.snapshot()).unwrap_or_default())
    }
}

// ----------------------------------------------------------------------------------------------
// The last folder used, so a new instance does not start empty.

fn config_path() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(std::path::PathBuf::from(home).join("Library/Application Support/Clowd/digger-folder.txt"))
}

pub fn remembered_folder() -> String {
    config_path().and_then(|p| std::fs::read_to_string(p).ok()).map(|s| s.trim().to_owned()).filter(|s| std::path::Path::new(s).is_dir()).unwrap_or_default()
}

pub fn remember_folder(folder: &str) {
    if let Some(p) = config_path() {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(p, folder);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::{Category, Sample};

    pub fn lib(cats: &[(&str, &[&str])]) -> Library {
        let cats = cats
            .iter()
            .map(|(n, files)| Category {
                name: n.to_string(),
                samples: files.iter().map(|f| Sample::plain(*f, vec![0.0; 4], None, vec![])).collect(),
            })
            .collect();
        Library::new("/x".into(), 44100, cats)
    }

    #[test]
    fn gm_drum_notes_fold_onto_the_eight_slots() {
        for (s, info) in SLOT_INFO.iter().enumerate() {
            assert_eq!(note_slot(info.note), Some(s), "{} answers to its own note", info.name);
        }
        assert_eq!(note_slot(40), Some(1));
        assert_eq!(note_slot(51), Some(7));
        assert_eq!(note_slot(60), None);
        // The browsing keys never collide with a drum.
        for n in NAV_BASE..NAV_BASE + NAV_KEYS {
            assert!(note_slot(n).is_none() && Nav::from_note(n).is_some());
        }
        assert_eq!(Nav::from_note(NAV_BASE + NAV_KEYS), None);
        assert_eq!(Nav::from_note(10), None);
        assert_eq!(note_name(36), "C1");
        assert_eq!(note_name(61), "C#3");
    }

    #[test]
    fn stepping_wraps_and_a_new_category_starts_at_the_top() {
        let l = lib(&[("kick", &["a", "b", "c"]), ("snare", &["x", "y"])]);
        let sh = Shared::default();
        sh.step_sample(0, -1, &l);
        assert_eq!(sh.selection(0), (0, 2));
        sh.step_sample(0, 1, &l);
        assert_eq!(sh.selection(0), (0, 0));
        sh.step_sample(0, 10, &l);
        assert_eq!(sh.selection(0), (0, 1));
        sh.step_cat(0, 1, &l);
        assert_eq!(sh.selection(0), (1, 0));
        sh.step_cat(0, 1, &l);
        assert_eq!(sh.selection(0), (0, 0));
        sh.set_sample(0, 99, &l);
        assert_eq!(sh.selection(0).1, 2, "clamped to the last sample");
    }

    #[test]
    fn a_random_pick_is_always_a_different_sample_in_range() {
        let l = lib(&[("kick", &["a", "b", "c", "d"])]);
        let sh = Shared::default();
        for seed in 0..200u32 {
            let before = sh.selection(0).1;
            sh.random_sample(0, seed.wrapping_mul(2654435761), &l);
            let after = sh.selection(0).1;
            assert!(after < 4 && after != before);
        }
        let one = lib(&[("kick", &["a"])]);
        sh.set_sample(0, 0, &one);
        sh.random_sample(0, 7, &one);
        assert_eq!(sh.selection(0).1, 0);
    }

    #[test]
    fn navigation_commands_move_the_focused_slot_and_ask_to_audition() {
        let l = lib(&[("kick", &["a", "b"]), ("snare", &["x", "y"])]);
        let sh = Shared::default();
        sh.navigate(Nav::SlotNext, 0, &l);
        assert_eq!(sh.focus(), 1);
        sh.navigate(Nav::SlotPrev, 0, &l);
        sh.navigate(Nav::SlotPrev, 0, &l);
        assert_eq!(sh.focus(), SLOTS - 1, "the focus wraps");
        sh.set_focus(0);
        sh.navigate(Nav::SampleNext, 0, &l);
        assert_eq!(sh.selection(0), (0, 1));
        assert_eq!(sh.browse[0].load(Ordering::Relaxed), 1);
        sh.navigate(Nav::CatNext, 0, &l);
        assert_eq!(sh.selection(0), (1, 0));
        sh.navigate(Nav::Audition, 0, &l);
        assert_eq!(sh.audition[0].load(Ordering::Relaxed), 1);
    }

    #[test]
    fn selections_round_trip_by_name_and_survive_a_reordered_library() {
        let before = lib(&[("kick", &["a", "b", "c"]), ("snare", &["x", "y"])]);
        let sh = Shared::default();
        *sh.root.lock().unwrap() = "/x".into();
        sh.apply_pending(&before);
        sh.set_cat(1, 1, &before);
        sh.set_sample(1, 1, &before);
        *sh.lib.current.lock().unwrap() = Some(std::sync::Arc::new(before));
        let json = PersistentField::map(&sh, |s: &String| s.clone());

        let restored = Shared::default();
        PersistentField::set(&restored, json);
        assert_eq!(*restored.root.lock().unwrap(), "/x");
        // The same folder, categories in another order and a sample added at the front.
        let after = lib(&[("snare", &["new", "x", "y"]), ("kick", &["a", "b", "c"])]);
        restored.apply_pending(&after);
        assert_eq!(restored.selection(1), (0, 2), "snare slot found 'y' in the moved category");
        assert_eq!(restored.selection(0), (1, 0), "an unsaved slot falls back to its default category");
    }

    #[test]
    fn nonsense_in_a_saved_project_is_ignored() {
        let sh = Shared::default();
        PersistentField::set(&sh, "{not json".to_string());
        let l = lib(&[("kick", &["a"])]);
        sh.apply_pending(&l);
        assert_eq!(sh.selection(0), (0, 0));
    }

    fn tagged() -> Library {
        let l = lib(&[("kick", &["a", "b", "c", "d", "e"]), ("snare", &["x", "y", "z"])]);
        for (c, k, t) in [(0, 1, "808"), (0, 3, "808"), (0, 4, "808"), (1, 2, "808")] {
            l.toggle_user_tag(c, k, t);
        }
        l
    }

    #[test]
    fn a_tag_filter_hides_the_rest_from_stepping_random_picks_and_new_categories() {
        let l = tagged();
        let sh = Shared::default();
        sh.toggle_filter(l.tag_bit("808", false).unwrap());
        // From the top, forward lands on b, d, e and then wraps back round to b.
        let mut seen = vec![];
        for _ in 0..4 {
            sh.step_sample(0, 1, &l);
            seen.push(sh.selection(0).1);
        }
        assert_eq!(seen, [1, 3, 4, 1]);
        sh.step_sample(0, -1, &l);
        assert_eq!(sh.selection(0).1, 4, "backwards skips too, and wraps");
        sh.set_sample(0, 2, &l);
        sh.step_sample(0, 1, &l);
        assert_eq!(sh.selection(0).1, 3, "from a hidden sample it finds the next one that shows");
        for seed in 0..60u32 {
            sh.random_sample(0, seed.wrapping_mul(2654435761), &l);
            assert!([1, 3, 4].contains(&sh.selection(0).1));
        }
        sh.set_cat(0, 1, &l);
        assert_eq!(sh.selection(0), (1, 2), "a new category starts on its first match");
        // Turning the same tag off again shows everything.
        sh.toggle_filter(l.tag_bit("808", false).unwrap());
        assert_eq!(sh.filter.load(Ordering::Relaxed), 0);
        sh.set_cat(0, 0, &l);
        assert_eq!(sh.selection(0), (0, 0));
    }

    #[test]
    fn a_tag_nothing_carries_leaves_stepping_where_it_is() {
        let l = tagged();
        l.tag_bit("ghost", true);
        let sh = Shared::default();
        sh.toggle_filter(l.tag_bit("ghost", false).unwrap());
        sh.step_sample(0, 1, &l);
        sh.random_sample(0, 5, &l);
        assert_eq!(sh.selection(0), (0, 0));
    }

    #[test]
    fn a_kit_saves_every_slot_and_switching_restores_them_and_sounds_a_beat() {
        let l = lib(&[("kick", &["a", "b", "c"]), ("snare", &["x", "y"])]);
        let sh = Shared::default();
        sh.apply_pending(&l);
        sh.set_sample(0, 2, &l);
        sh.set_cat(1, 1, &l);
        sh.set_sample(1, 1, &l);
        let kit = sh.kit_from_selection("ONE", &l);
        assert_eq!((kit.slots[0].cat.as_str(), kit.slots[0].file.as_str()), ("kick", "c"));
        assert_eq!((kit.slots[1].cat.as_str(), kit.slots[1].file.as_str()), ("snare", "y"));
        sh.kits.lock().unwrap().push(kit);
        sh.set_sample(0, 0, &l);
        sh.set_sample(1, 0, &l);
        let before = sh.browse[0].load(Ordering::Relaxed);
        assert!(sh.apply_kit(0, &l));
        assert_eq!((sh.selection(0), sh.selection(1)), ((0, 2), (1, 1)));
        assert_eq!(sh.kit.load(Ordering::Relaxed), 0);
        assert_eq!(sh.browse[0].load(Ordering::Relaxed), before + 1, "kick, snare and hat speak");
        assert_eq!(sh.browse[1].load(Ordering::Relaxed), 1);
        assert_eq!(sh.browse[3].load(Ordering::Relaxed), 1);
        assert_eq!(sh.browse[2].load(Ordering::Relaxed), 0);
        assert!(!sh.apply_kit(7, &l), "no such kit");
    }

    #[test]
    fn kits_step_round_and_a_kit_naming_a_missing_sample_keeps_that_slot() {
        let l = lib(&[("kick", &["a", "b"])]);
        let sh = Shared::default();
        sh.apply_pending(&l);
        let kit = |name: &str, file: &str| Kit { name: name.into(), slots: vec![SavedSlot { cat: "kick".into(), file: file.into() }] };
        *sh.kits.lock().unwrap() = vec![kit("one", "a"), kit("two", "b"), kit("gone", "deleted.wav")];
        sh.step_kit(1, &l);
        assert_eq!(sh.kit.load(Ordering::Relaxed), 0, "from no kit, forward is the first");
        sh.step_kit(1, &l);
        assert_eq!(sh.selection(0).1, 1);
        sh.step_kit(1, &l);
        assert_eq!(sh.selection(0).1, 1, "the missing sample leaves the slot alone");
        sh.step_kit(1, &l);
        assert_eq!(sh.kit.load(Ordering::Relaxed), 0, "and it wraps");
        sh.step_kit(-1, &l);
        assert_eq!(sh.kit.load(Ordering::Relaxed), 2);
        // No kits at all: nothing happens.
        let empty = Shared::default();
        empty.step_kit(1, &l);
        assert_eq!(empty.kit.load(Ordering::Relaxed), NO_KIT);
    }

    #[test]
    fn rolling_a_kit_draws_each_slot_from_the_tagged_samples_of_its_own_category() {
        let l = tagged();
        let sh = Shared::default();
        sh.apply_pending(&l);
        sh.toggle_filter(l.tag_bit("808", false).unwrap());
        let mut kicks = std::collections::HashSet::new();
        for seed in 0..80u32 {
            sh.roll_kit(seed.wrapping_mul(2654435761), &l);
            assert_eq!(sh.kit.load(Ordering::Relaxed), NO_KIT, "a rolled kit is not a saved one");
            let (cat, idx) = sh.selection(0);
            assert_eq!(cat, 0, "slots stay in their category");
            assert!([1, 3, 4].contains(&idx));
            kicks.insert(idx);
            assert_eq!(sh.selection(1), (1, 2), "the only tagged snare");
        }
        assert_eq!(kicks.len(), 3, "and it does vary");
        // Slots whose category has no tagged sample stay put.
        assert_eq!(sh.selection(2).1, sh.selection(2).1);
    }

    #[test]
    fn the_sidecar_round_trips_kits_and_user_tags_through_a_folder() {
        let dir = crate::library::tests::Scratch::new("shared");
        let l = tagged();
        let sh = Shared::default();
        *sh.root.lock().unwrap() = dir.0.to_string_lossy().into_owned();
        assert!(sh.save_sidecar().is_err(), "nothing loaded yet");
        *sh.lib.current.lock().unwrap() = Some(std::sync::Arc::new(l));
        sh.kits.lock().unwrap().push(Kit { name: "mine".into(), slots: vec![] });
        sh.save_sidecar().unwrap();
        let back = crate::tags::load_sidecar(&dir.0);
        assert_eq!(back.kits.len(), 1);
        assert_eq!(back.tags["kick/b"], ["808"]);
    }
}
