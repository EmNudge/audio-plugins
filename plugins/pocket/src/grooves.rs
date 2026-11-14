//! The built-in library: grooves and fills in folders, written in `library.txt`.
//!
//! A lane is sixteenth-note steps, one bar per 16 characters (up to four bars). Spaces are
//! ignored so beats can be grouped. `X` accent, `x` hit, `o` ghost note, `r` two hits in the step,
//! `R` three, `q` four, `.` rest.

use crate::pattern::{Cell, PatternStore, LANES, LANE_CRASH, LANE_PERC, MAX_BARS};
use std::sync::OnceLock;

pub const VEL_ACCENT: u8 = 122;
pub const VEL_HIT: u8 = 100;
pub const VEL_GHOST: u8 = 50;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Groove,
    /// A fill replaces the end of whatever is on the board instead of all of it.
    Fill,
}

pub struct Groove {
    /// Folders from the root, e.g. `["GROOVES", "Rock", "Classic"]`.
    pub path: Vec<String>,
    pub kind: Kind,
    pub name: String,
    pub bpm: u16,
    /// Swing as the fraction of a pair the first note takes, and whether it pairs eighths.
    pub swing: f32,
    pub eighths: bool,
    /// Kick, snare, rim, clap, hat, open hat, low tom, high tom, crash, ride, perc.
    pub lanes: [Vec<char>; LANES],
    thumb: [[u8; 16]; LANES],
}

/// The cell a library character stands for.
pub fn cell_of(c: char) -> Option<Cell> {
    Some(match c {
        'X' => Cell::hit(VEL_ACCENT),
        'x' => Cell::hit(VEL_HIT),
        'o' => Cell::hit(VEL_GHOST),
        'r' => Cell::new(VEL_HIT - 6, 0, 2),
        'R' => Cell::new(VEL_HIT - 6, 0, 3),
        'q' => Cell::new(VEL_HIT - 10, 0, 4),
        '.' => Cell::default(),
        _ => return None,
    })
}

/// Lane characters with the spacing removed.
pub fn steps_of(lane: &str) -> Vec<char> {
    lane.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Which lane a letter in `library.txt` names.
fn lane_of_key(key: char) -> Option<usize> {
    "KSRCHOLTXDP".find(key)
}

impl Groove {
    pub fn bars(&self) -> usize {
        self.lanes.iter().map(Vec::len).max().unwrap_or(16).div_ceil(16).clamp(1, MAX_BARS)
    }

    pub fn folder(&self) -> String {
        self.path.join(" / ")
    }

    /// The first bar as a grid of hit strengths, for the browser's thumbnails.
    pub fn thumbnail(&self) -> &[[u8; 16]; LANES] {
        &self.thumb
    }

    /// Write the groove into `store`, replacing everything but the locked lanes when `keep_locked`.
    /// A fill goes onto the end of the pattern instead.
    pub fn load(&self, store: &PatternStore, keep_locked: bool) {
        match self.kind {
            Kind::Fill => self.load_fill(store),
            Kind::Groove => self.write(store, keep_locked),
        }
    }

    /// Write the clip as it stands into `store`, fill or not: for dragging a fill out as a clip.
    pub fn write(&self, store: &PatternStore, keep_locked: bool) {
        let bars = self.bars();
        store.set_bars(1);
        store.set_bars(bars);
        let steps = store.steps();
        for (lane, chars) in self.lanes.iter().enumerate() {
            if keep_locked && store.locked(lane) {
                continue;
            }
            store.clear_lane(lane);
            if chars.is_empty() {
                continue;
            }
            for s in 0..steps {
                // A one-bar lane repeats through a longer groove.
                if let Some(cell) = cell_of(chars[s % chars.len()]) {
                    store.set(lane, s, cell);
                }
            }
        }
        store.set_name(&self.name);
    }

    fn load_fill(&self, store: &PatternStore) {
        let bars = self.bars();
        if store.bars() < bars {
            store.set_bars(bars);
        }
        let fill_steps = bars * 16;
        let base = store.steps() - fill_steps;
        let first = (0..fill_steps)
            .find(|&s| self.lanes.iter().any(|l| l.get(s).and_then(|c| cell_of(*c)).is_some_and(|c| c.is_on())))
            .unwrap_or(fill_steps);
        // The fill takes over the timekeeping and the drums; crash and percussion carry on.
        for lane in (0..LANES).filter(|&l| l != LANE_CRASH && l != LANE_PERC) {
            if store.locked(lane) {
                continue;
            }
            for s in first..fill_steps {
                store.set(lane, base + s, Cell::default());
            }
        }
        for (lane, chars) in self.lanes.iter().enumerate() {
            if store.locked(lane) {
                continue;
            }
            for s in first..fill_steps {
                if let Some(cell) = chars.get(s).and_then(|c| cell_of(*c)).filter(|c| c.is_on()) {
                    store.set(lane, base + s, cell);
                }
            }
        }
    }
}

/// Parse the library text. Returns the grooves and a message for every line that could not be read.
pub fn parse(text: &str) -> (Vec<Groove>, Vec<String>) {
    let (mut grooves, mut errors) = (Vec::new(), Vec::new());
    let mut path: Vec<String> = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(header) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            path = header.split('/').map(|p| p.trim().to_owned()).collect();
            continue;
        }
        match parse_line(line, &path) {
            Ok(g) => grooves.push(g),
            Err(e) => errors.push(format!("line {}: {e}", n + 1)),
        }
    }
    (grooves, errors)
}

fn parse_line(line: &str, path: &[String]) -> Result<Groove, String> {
    if path.is_empty() {
        return Err("no folder header above it".into());
    }
    let mut parts = line.split('|');
    let head = parts.next().unwrap_or("");
    let (mut name, mut bpm, mut swing, mut eighths) = (Vec::new(), 120u16, 0.5f32, false);
    for word in head.split_whitespace() {
        if let Some(b) = word.strip_prefix('@') {
            bpm = b.parse().map_err(|_| format!("bad tempo {word:?}"))?;
        } else if let Some(s) = word.strip_prefix('~') {
            let (s, e) = s.strip_suffix('e').map_or((s, false), |s| (s, true));
            swing = format!("0{s}").parse().map_err(|_| format!("bad swing {word:?}"))?;
            eighths = e;
        } else {
            name.push(word);
        }
    }
    let name = name.join(" ");
    if name.is_empty() {
        return Err("no name".into());
    }
    let mut lanes: [Vec<char>; LANES] = std::array::from_fn(|_| Vec::new());
    for part in parts {
        let part = part.trim();
        let mut chars = part.chars();
        let key = chars.next().ok_or("empty lane")?;
        let lane = lane_of_key(key).ok_or_else(|| format!("{name}: no lane {key:?}"))?;
        let steps = steps_of(chars.as_str());
        if steps.is_empty() || steps.len() % 16 != 0 || steps.len() > MAX_BARS * 16 {
            return Err(format!("{name}: lane {key} has {} steps", steps.len()));
        }
        if let Some(bad) = steps.iter().find(|c| cell_of(**c).is_none()) {
            return Err(format!("{name}: bad character {bad:?} in lane {key}"));
        }
        lanes[lane] = steps;
    }
    if lanes.iter().all(Vec::is_empty) {
        return Err(format!("{name} is empty"));
    }
    let mut thumb = [[0u8; 16]; LANES];
    for (lane, chars) in lanes.iter().enumerate() {
        for (s, c) in chars.iter().take(16).enumerate() {
            thumb[lane][s] = cell_of(*c).map_or(0, |cell| cell.vel());
        }
    }
    let kind = if path[0] == "FILLS" { Kind::Fill } else { Kind::Groove };
    Ok(Groove { path: path.to_vec(), kind, name, bpm, swing, eighths, lanes, thumb })
}

static LIBRARY: OnceLock<Vec<Groove>> = OnceLock::new();

pub fn library() -> &'static [Groove] {
    LIBRARY.get_or_init(|| parse(include_str!("library.txt")).0)
}

/// A folder of the browser: sub-folders and the grooves directly in it.
#[derive(Default)]
pub struct Folder {
    pub name: String,
    pub folders: Vec<Folder>,
    /// Indices into `library()`.
    pub items: Vec<usize>,
}

impl Folder {
    pub fn count(&self) -> usize {
        self.items.len() + self.folders.iter().map(Folder::count).sum::<usize>()
    }

    pub fn find(&self, path: &[String]) -> Option<&Folder> {
        match path.split_first() {
            None => Some(self),
            Some((head, rest)) => self.folders.iter().find(|f| &f.name == head)?.find(rest),
        }
    }

    /// Every item below this folder.
    pub fn all_items(&self, out: &mut Vec<usize>) {
        out.extend(&self.items);
        for f in &self.folders {
            f.all_items(out);
        }
    }
}

static TREE: OnceLock<Folder> = OnceLock::new();

pub fn tree() -> &'static Folder {
    TREE.get_or_init(|| {
        let mut root = Folder::default();
        for (index, g) in library().iter().enumerate() {
            let mut node = &mut root;
            for part in &g.path {
                let at = match node.folders.iter().position(|f| &f.name == part) {
                    Some(at) => at,
                    None => {
                        node.folders.push(Folder { name: part.clone(), ..Folder::default() });
                        node.folders.len() - 1
                    }
                };
                node = &mut node.folders[at];
            }
            node.items.push(index);
        }
        root
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pattern::*;

    #[test]
    fn the_whole_library_parses_without_errors() {
        let (grooves, errors) = parse(include_str!("library.txt"));
        assert!(errors.is_empty(), "{errors:#?}");
        assert!(grooves.len() >= 200, "only {} entries", grooves.len());
    }

    #[test]
    fn every_entry_is_sane() {
        for g in library() {
            assert!((40..=210).contains(&g.bpm), "{}: {} bpm", g.name, g.bpm);
            assert!((0.5..=0.75).contains(&g.swing), "{}: swing {}", g.name, g.swing);
            assert!(g.bars() >= 1 && g.bars() <= MAX_BARS);
            assert!(g.path.len() >= 2, "{} is not in a genre folder", g.name);
            if g.kind == Kind::Groove {
                // A groove needs a pulse: a kick or a snare of some kind.
                assert!((0..4).any(|l| !g.lanes[l].is_empty()), "{} has no kick or snare", g.name);
            }
        }
    }

    #[test]
    fn nothing_is_listed_twice_in_the_same_folder() {
        let mut seen = std::collections::HashSet::new();
        for g in library() {
            assert!(seen.insert((g.folder(), g.name.clone())), "{} / {}", g.folder(), g.name);
        }
    }

    #[test]
    fn the_tree_has_sections_genres_and_fills() {
        let root = tree();
        let sections: Vec<_> = root.folders.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(sections, ["GROOVES", "FILLS"]);
        assert_eq!(root.count(), library().len());
        let grooves = root.find(&["GROOVES".into()]).unwrap();
        assert!(grooves.folders.len() >= 10);
        for genre in ["Rock", "Drum & Bass", "Breakcore & IDM", "Trap", "House", "Techno"] {
            assert!(grooves.folders.iter().any(|f| f.name == genre), "no {genre} folder");
        }
        assert!(root.find(&["FILLS".into(), "Toms".into()]).is_some());
        assert!(root.find(&["GROOVES".into(), "Nope".into()]).is_none());
    }

    #[test]
    fn fills_are_in_the_fill_section_and_grooves_are_not() {
        for g in library() {
            assert_eq!(g.kind == Kind::Fill, g.path[0] == "FILLS", "{}", g.name);
        }
    }

    #[test]
    fn the_hyper_folder_really_is_fast_and_rolled() {
        let hyper = tree().find(&["GROOVES".into(), "Breakcore & IDM".into(), "Hyper".into()]).unwrap();
        assert!(hyper.items.len() >= 8);
        let rolls = |g: &Groove| g.lanes.iter().flatten().filter(|c| matches!(c, 'r' | 'R' | 'q')).count();
        for &i in &hyper.items {
            let g = &library()[i];
            assert!(g.bpm >= 160, "{} is only {} bpm", g.name, g.bpm);
            assert!(rolls(g) >= 3, "{} has few rolls", g.name);
        }
    }

    #[test]
    fn parse_reports_bad_lines_and_reads_options() {
        let (g, e) = parse("[GROOVES/T/U]\nOne @97 ~.667e | K X... .... .... ....\nBad | K X...\nAlso Bad | Z X... .... .... ....\nNo Bars | K X... .... .... ..x. ..x.\n");
        assert_eq!(g.len(), 1);
        assert_eq!(e.len(), 3);
        assert_eq!((g[0].bpm, g[0].eighths), (97, true));
        assert!((g[0].swing - 0.667).abs() < 1e-6);
        let (_, e) = parse("Orphan | K X... .... .... ....");
        assert_eq!(e.len(), 1);
    }

    #[test]
    fn loading_writes_cells_and_respects_locks() {
        let store = PatternStore::default();
        let groove = library().iter().find(|g| g.name == "Backbeat").unwrap();
        groove.load(&store, false);
        assert_eq!(store.get(LANE_KICK, 0).vel(), VEL_ACCENT);
        assert_eq!(store.get(LANE_SNARE, 4).vel(), VEL_ACCENT);
        assert_eq!(store.name(), "Backbeat");
        store.toggle_lock(LANE_KICK);
        store.set(LANE_KICK, 1, Cell::hit(77));
        let other = library().iter().find(|g| g.name == "Half-Time").unwrap();
        other.load(&store, true);
        assert_eq!(store.get(LANE_KICK, 1).vel(), 77, "locked kick lane must be left alone");
        assert_eq!(store.get(LANE_SNARE, 4).vel(), 0);
        assert_eq!(store.get(LANE_SNARE, 8).vel(), VEL_ACCENT);
    }

    #[test]
    fn long_grooves_set_the_pattern_length() {
        let store = PatternStore::default();
        let g = library().iter().find(|g| g.name == "Three Against Four").unwrap();
        assert_eq!(g.bars(), 3);
        g.load(&store, false);
        assert_eq!(store.steps(), 48);
        // The kick lands every third step across the whole three bars.
        assert!((0..48).step_by(3).all(|s| store.get(LANE_KICK, s).is_on()));
        assert!((0..48).filter(|s| s % 3 != 0).all(|s| !store.get(LANE_KICK, s).is_on()));
        // Loading a one-bar groove afterwards shrinks it back.
        library().iter().find(|g| g.name == "Backbeat").unwrap().load(&store, false);
        assert_eq!(store.steps(), 16);
    }

    #[test]
    fn a_fill_replaces_only_the_end_of_the_last_bar() {
        let store = PatternStore::default();
        library().iter().find(|g| g.name == "Backbeat").unwrap().load(&store, false);
        let before: Vec<_> = (0..LANES).flat_map(|l| (0..12).map(move |s| (l, s))).map(|(l, s)| store.get(l, s)).collect();
        let fill = library().iter().find(|g| g.name == "Snare Build").unwrap();
        assert_eq!(fill.kind, Kind::Fill);
        fill.load(&store, true);
        let after: Vec<_> = (0..LANES).flat_map(|l| (0..12).map(move |s| (l, s))).map(|(l, s)| store.get(l, s)).collect();
        assert_eq!(before, after, "the first three beats are untouched");
        assert!((12..16).all(|s| store.get(LANE_SNARE, s).is_on()));
        assert!((12..16).all(|s| !store.get(LANE_HAT, s).is_on()), "hats stop for the fill");
        assert_eq!(store.name(), "Backbeat", "a fill does not rename the groove");
    }

    #[test]
    fn a_fill_leaves_locked_drums_and_pads_the_pattern_to_its_length() {
        let store = PatternStore::default();
        library().iter().find(|g| g.name == "Backbeat").unwrap().load(&store, false);
        store.toggle_lock(LANE_HAT);
        library().iter().find(|g| g.name == "Four Bar Build").unwrap().load(&store, true);
        assert_eq!(store.steps(), 64, "a four-bar fill grows the pattern to four bars");
        assert!(store.get(LANE_HAT, 60).is_on(), "locked hats are not cleared");
        assert!(store.get(LANE_SNARE, 63).ratchet() >= 3);
    }

    #[test]
    fn thumbnails_match_the_first_bar() {
        let g = library().iter().find(|g| g.name == "Backbeat").unwrap();
        let t = g.thumbnail();
        assert_eq!(t[0][0], VEL_ACCENT);
        assert_eq!(t[1][4], VEL_ACCENT);
        assert_eq!(t[1][5], 0);
    }

    #[test]
    fn rolls_become_ratchets() {
        assert_eq!(cell_of('r').unwrap().ratchet(), 2);
        assert_eq!(cell_of('R').unwrap().ratchet(), 3);
        assert_eq!(cell_of('q').unwrap().ratchet(), 4);
    }
}
