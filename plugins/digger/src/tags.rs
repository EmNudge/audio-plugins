//! Tags: words that group samples so a whole library can be filtered and kits built from it.
//!
//! Three sources feed one vocabulary of up to 64 tags. Words repeated across file names
//! (`808`, `acoustic`, `vinyl`), measurements of the audio itself (`short` and `long` by how fast
//! it dies away, `dark` and `bright` by its zero-crossing rate, each against the rest of its own
//! category), and tags the user adds by hand. Only the last kind, and the kits, are saved, in a
//! `.digger.json` beside the samples so they travel with the folder.

use crate::library::{Category, Library, Sample};
use crate::state::Kit;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::Ordering;

pub const MAX_TAGS: usize = 64;
/// Room kept for the user's own tags.
const MAX_AUTO: usize = 44;
const SIDECAR: &str = ".digger.json";
/// Names that describe the drum itself, which the category already says.
const SKIP: &[&str] = &[
    "kick", "snare", "hat", "hats", "hihat", "openhat", "open", "clap", "claps", "crash", "ride", "tom", "toms", "tomtom", "perc", "shaker", "cowbell", "cymbal", "the", "and", "oh", "for", "its", "wav", "sample",
];

/// Seconds for the envelope to fall 30 dB below its peak, and the zero-crossing rate in hertz over
/// the first 80 ms.
pub fn measure(left: &[f32], sr: u32) -> (f32, f32) {
    let win = ((0.005 * sr as f32) as usize).max(1);
    let env: Vec<f32> = left.chunks(win).map(|c| c.iter().fold(0.0f32, |m, v| m.max(v.abs()))).collect();
    let peak = env.iter().cloned().fold(0.0, f32::max);
    let last = env.iter().rposition(|&e| e > peak * 0.0316).unwrap_or(0);
    let decay = (last + 1) as f32 * win as f32 / sr as f32;
    let head = &left[..left.len().min((0.08 * sr as f32) as usize)];
    let crossings = head.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count();
    let bright = crossings as f32 / (2.0 * (head.len().max(1) as f32 / sr as f32));
    (decay, bright)
}

/// Words in a file name that could name a group: lower case, trailing numbers dropped
/// (`acoustic01` is `acoustic`), three-digit machine names kept (`808`), the rest of the numbers
/// and the drum's own name dropped.
pub fn name_words(stem: &str, category: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in stem.to_lowercase().split(|c: char| !c.is_ascii_alphanumeric()).filter(|t| !t.is_empty()) {
        let word = if raw.len() == 3 && raw.chars().all(|c| c.is_ascii_digit()) { raw.to_owned() } else { raw.trim_end_matches(|c: char| c.is_ascii_digit()).to_owned() };
        if word.len() < 3 || SKIP.contains(&word.as_str()) || word.eq_ignore_ascii_case(category) || out.contains(&word) {
            continue;
        }
        out.push(word);
    }
    out
}

impl Library {
    pub fn tag_count(&self) -> usize {
        self.tag_names.lock().map(|n| n.len()).unwrap_or(0)
    }

    pub fn tag_name(&self, bit: usize) -> String {
        self.tag_names.lock().ok().and_then(|n| n.get(bit).cloned()).unwrap_or_default()
    }

    /// The bit for a tag, adding it to the vocabulary if `create` and there is room.
    pub fn tag_bit(&self, name: &str, create: bool) -> Option<usize> {
        let name = name.trim().to_lowercase();
        if name.is_empty() {
            return None;
        }
        let mut names = self.tag_names.lock().ok()?;
        if let Some(i) = names.iter().position(|n| *n == name) {
            return Some(i);
        }
        if create && names.len() < MAX_TAGS {
            names.push(name);
            return Some(names.len() - 1);
        }
        None
    }

    pub fn sample_tags(&self, cat: usize, idx: usize) -> Vec<(usize, String, bool)> {
        let Some(s) = self.sample(cat, idx) else { return vec![] };
        let (tags, user) = (s.tags.load(Ordering::Relaxed), s.user.load(Ordering::Relaxed));
        (0..self.tag_count()).filter(|b| tags >> b & 1 == 1).map(|b| (b, self.tag_name(b), user >> b & 1 == 1)).collect()
    }

    /// Tags present in a category with how many samples carry each, commonest first.
    pub fn category_tags(&self, cat: usize) -> Vec<(usize, usize)> {
        let Some(c) = self.cats.get(cat) else { return vec![] };
        let mut counts: Vec<(usize, usize)> = (0..self.tag_count())
            .map(|b| (b, c.samples.iter().filter(|s| s.tags.load(Ordering::Relaxed) >> b & 1 == 1).count()))
            .filter(|(_, n)| *n > 0)
            .collect();
        counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        counts
    }

    /// Add or remove a tag the user chose. Returns whether it is now set.
    pub fn toggle_user_tag(&self, cat: usize, idx: usize, name: &str) -> Option<bool> {
        let s = self.sample(cat, idx)?;
        let bit = self.tag_bit(name, true)?;
        let m = 1u64 << bit;
        let on = s.user.load(Ordering::Relaxed) & m == 0;
        if on {
            s.user.fetch_or(m, Ordering::Relaxed);
            s.tags.fetch_or(m, Ordering::Relaxed);
        } else {
            s.user.fetch_and(!m, Ordering::Relaxed);
            s.tags.fetch_and(!m, Ordering::Relaxed);
        }
        Some(on)
    }
}

fn set(lib: &Library, s: &Sample, name: &str) {
    if let Some(bit) = lib.tag_bit(name, true) {
        s.tags.fetch_or(1 << bit, Ordering::Relaxed);
    }
}

/// Tag a category by its measurements: the quickest-dying third `short`, the slowest `long`, the
/// lowest zero-crossing rate `dark`, the highest `bright`.
fn tag_by_measurement(lib: &Library, c: &Category) {
    let n = c.samples.len();
    if n < 6 {
        return;
    }
    let rank = |key: &dyn Fn(&Sample) -> f32| -> Vec<usize> {
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by(|&a, &b| key(&c.samples[a]).partial_cmp(&key(&c.samples[b])).unwrap_or(std::cmp::Ordering::Equal));
        order
    };
    let third = n / 3;
    let by_decay = rank(&|s| s.decay);
    for &i in &by_decay[..third] {
        set(lib, &c.samples[i], "short");
    }
    for &i in &by_decay[n - third..] {
        set(lib, &c.samples[i], "long");
    }
    let by_bright = rank(&|s| s.bright);
    for &i in &by_bright[..third] {
        set(lib, &c.samples[i], "dark");
    }
    for &i in &by_bright[n - third..] {
        set(lib, &c.samples[i], "bright");
    }
}

/// Tag every sample from its name and its audio.
pub fn auto_tag(lib: &Library) {
    // Words that recur are worth a tag; one-offs are just names.
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for c in &lib.cats {
        for s in &c.samples {
            for w in name_words(&s.name, &c.name) {
                *counts.entry(w).or_default() += 1;
            }
        }
    }
    let mut common: Vec<(String, usize)> = counts.into_iter().filter(|(_, n)| *n >= 2).collect();
    common.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    common.truncate(MAX_AUTO - 4);
    // The measured tags are added first so they always have a place.
    for name in ["short", "long", "dark", "bright"] {
        lib.tag_bit(name, true);
    }
    for c in &lib.cats {
        tag_by_measurement(lib, c);
        for s in &c.samples {
            for w in name_words(&s.name, &c.name) {
                if common.iter().any(|(k, _)| *k == w) {
                    set(lib, s, &w);
                }
            }
        }
    }
}

// ----------------------------------------------------------------------------------------------
// Saved beside the samples

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct Sidecar {
    /// Tags the user added, by `category/sample`.
    #[serde(default)]
    pub tags: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub kits: Vec<Kit>,
}

fn key(cat: &Category, s: &Sample) -> String {
    format!("{}/{}", cat.name, s.name)
}

pub fn load_sidecar(root: &Path) -> Sidecar {
    std::fs::read_to_string(root.join(SIDECAR)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn save_sidecar(root: &Path, side: &Sidecar) -> Result<(), String> {
    let text = serde_json::to_string_pretty(side).map_err(|e| e.to_string())?;
    std::fs::write(root.join(SIDECAR), text).map_err(|e| e.to_string())
}

pub fn apply_sidecar(lib: &Library, side: &Sidecar) {
    for c in &lib.cats {
        for s in &c.samples {
            for name in side.tags.get(&key(c, s)).into_iter().flatten() {
                if let Some(bit) = lib.tag_bit(name, true) {
                    s.user.fetch_or(1 << bit, Ordering::Relaxed);
                    s.tags.fetch_or(1 << bit, Ordering::Relaxed);
                }
            }
        }
    }
}

/// The user's tags gathered for saving.
pub fn user_tags(lib: &Library) -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::new();
    for c in &lib.cats {
        for s in &c.samples {
            let user = s.user.load(Ordering::Relaxed);
            if user != 0 {
                let names: Vec<String> = (0..lib.tag_count()).filter(|b| user >> b & 1 == 1).map(|b| lib.tag_name(b)).collect();
                out.insert(key(c, s), names);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::tests::Scratch;

    fn cat(name: &str, files: &[(&str, f32, f32)]) -> Category {
        // (name, decay seconds, zero-crossing hertz) stand in for measured audio.
        Category {
            name: name.into(),
            samples: files
                .iter()
                .map(|(n, decay, bright)| {
                    let mut s = Sample::plain(*n, vec![0.0; 4], None, vec![]);
                    s.decay = *decay;
                    s.bright = *bright;
                    s
                })
                .collect(),
        }
    }

    #[test]
    fn file_names_give_words_minus_numbers_and_the_drum_itself() {
        assert_eq!(name_words("[I] kick-acoustic01", "kick"), ["acoustic"]);
        assert_eq!(name_words("Snare - Eight Oh Eight!", "snare"), ["eight"], "a repeat and a stop word go");
        assert_eq!(name_words("kick-808", "kick"), ["808"]);
        assert_eq!(name_words("Kick - 1985", "kick"), Vec::<String>::new(), "years are not tags");
        assert_eq!(name_words("hat-vinyl vinyl2", "hat"), ["vinyl"], "no repeats");
    }

    #[test]
    fn the_audio_is_measured_for_decay_and_brightness() {
        let sr = 44100u32;
        let n = sr as usize / 2;
        let ring = |decay: f32, hz: f32| -> Vec<f32> { (0..n).map(|i| (i as f32 / sr as f32 * hz * std::f32::consts::TAU).sin() * (-(i as f32) / sr as f32 * decay).exp()).collect() };
        let (short, dark) = measure(&ring(60.0, 100.0), sr);
        let (long, bright) = measure(&ring(6.0, 4000.0), sr);
        assert!(short < long * 0.5, "{short} vs {long}");
        assert!((dark - 100.0).abs() < 30.0, "{dark}");
        assert!((bright - 4000.0).abs() < 400.0, "{bright}");
        // Silence and a handful of samples do not break it.
        let (d, b) = measure(&[0.0; 10], sr);
        assert!(d.is_finite() && b.is_finite());
    }

    #[test]
    fn measured_tags_are_relative_to_the_category() {
        let files: Vec<(String, f32, f32)> = (0..9).map(|i| (format!("s{i}"), 0.1 + i as f32 * 0.1, 200.0 + i as f32 * 300.0)).collect();
        let refs: Vec<(&str, f32, f32)> = files.iter().map(|(n, d, b)| (n.as_str(), *d, *b)).collect();
        let lib = Library::new(String::new(), 44100, vec![cat("kick", &refs)]);
        auto_tag(&lib);
        let names = |i: usize| lib.sample_tags(0, i).into_iter().map(|t| t.1).collect::<Vec<_>>();
        assert_eq!(names(0), ["short", "dark"]);
        assert_eq!(names(8), ["long", "bright"]);
        assert!(names(4).is_empty(), "the middle third carries neither");
        // Too few samples to rank: no measured tags.
        let small = Library::new(String::new(), 44100, vec![cat("kick", &refs[..4])]);
        auto_tag(&small);
        assert!(small.sample_tags(0, 0).is_empty());
    }

    #[test]
    fn repeated_words_become_tags_and_one_offs_do_not() {
        let lib = Library::new(String::new(), 44100, vec![cat("kick", &[("kick-808", 0.0, 0.0), ("[II] Kick - 808 Deep", 0.0, 0.0), ("kick-lonely", 0.0, 0.0)])]);
        auto_tag(&lib);
        assert!(lib.tag_bit("808", false).is_some());
        assert!(lib.tag_bit("lonely", false).is_none());
        assert!(lib.sample_tags(0, 0).iter().any(|t| t.1 == "808"));
        assert!(!lib.sample_tags(0, 2).iter().any(|t| t.1 == "808"));
    }

    #[test]
    fn user_tags_toggle_and_the_vocabulary_has_a_ceiling() {
        let lib = Library::new(String::new(), 44100, vec![cat("kick", &[("a", 0.0, 0.0), ("b", 0.0, 0.0)])]);
        assert_eq!(lib.toggle_user_tag(0, 0, "  Fav "), Some(true));
        assert_eq!(lib.sample_tags(0, 0), vec![(0, "fav".to_string(), true)]);
        assert_eq!(lib.category_tags(0), vec![(0, 1)]);
        assert_eq!(lib.toggle_user_tag(0, 0, "FAV"), Some(false));
        assert!(lib.sample_tags(0, 0).is_empty());
        assert_eq!(lib.toggle_user_tag(0, 9, "x"), None, "no such sample");
        assert_eq!(lib.toggle_user_tag(0, 0, "   "), None, "a blank is not a tag");
        for i in 0..MAX_TAGS + 5 {
            lib.tag_bit(&format!("t{i}"), true);
        }
        assert_eq!(lib.tag_count(), MAX_TAGS);
        assert_eq!(lib.toggle_user_tag(0, 1, "one too many"), None);
    }

    #[test]
    fn only_user_tags_are_saved_and_they_come_back() {
        let make = || Library::new("/x".into(), 44100, vec![cat("kick", &[("a", 0.0, 0.0), ("b", 0.0, 0.0)]), cat("snare", &[("c", 0.0, 0.0)])]);
        let lib = make();
        lib.toggle_user_tag(0, 1, "punchy");
        lib.toggle_user_tag(1, 0, "punchy");
        lib.toggle_user_tag(1, 0, "wet");
        set(&lib, &lib.cats[0].samples[0], "auto");
        let saved = user_tags(&lib);
        assert_eq!(saved.len(), 2);
        assert_eq!(saved["kick/b"], ["punchy"]);
        assert_eq!(saved["snare/c"], ["punchy", "wet"]);

        let dir = Scratch::new("sidecar");
        save_sidecar(&dir.0, &Sidecar { tags: saved, kits: vec![] }).unwrap();
        let fresh = make();
        apply_sidecar(&fresh, &load_sidecar(&dir.0));
        assert_eq!(fresh.sample_tags(0, 1).iter().map(|t| t.1.clone()).collect::<Vec<_>>(), ["punchy"]);
        assert!(fresh.sample_tags(0, 0).is_empty());
        assert!(fresh.sample_tags(1, 0).iter().all(|t| t.2), "restored tags are the user's");
        // A missing or damaged file is an empty sidecar, not an error.
        assert!(load_sidecar(&dir.0.join("nowhere")).tags.is_empty());
        std::fs::write(dir.0.join(SIDECAR), "{broken").unwrap();
        assert!(load_sidecar(&dir.0).kits.is_empty());
    }
}
