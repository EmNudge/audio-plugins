//! Reading drum clips from `.mid` files, and finding a folder of them.
//!
//! The notes are sorted into Pocket's eleven drums by their General MIDI number, snapped to the
//! sixteenth grid with the remainder kept as a timing nudge, and two hits landing on one step
//! become a roll. The clip is cut to four bars.

use crate::pattern::*;
use std::path::{Path, PathBuf};

/// A note-on read from a file: ticks from the start, MIDI note, velocity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Note {
    pub tick: u64,
    pub note: u8,
    pub vel: u8,
}

pub struct Parsed {
    pub notes: Vec<Note>,
    pub ppq: u32,
    pub bpm: Option<f64>,
    /// Ticks per bar in the file's first time signature, if it states one.
    pub bar_ticks: Option<u64>,
    /// The tick of the last event, so a clip with a long tail still gets its length.
    pub end_tick: u64,
}

fn be16(b: &[u8]) -> Option<u32> {
    Some(u16::from_be_bytes(b.get(..2)?.try_into().ok()?) as u32)
}

fn be32(b: &[u8]) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(..4)?.try_into().ok()?))
}

/// A standard MIDI file, any format. Notes from every track are merged.
pub fn parse_smf(bytes: &[u8]) -> Result<Parsed, String> {
    if bytes.get(..4) != Some(b"MThd") {
        return Err("not a MIDI file".into());
    }
    let header_len = be32(&bytes[4..]).ok_or("short header")? as usize;
    let tracks = be16(&bytes[10..]).ok_or("short header")? as usize;
    let division = be16(&bytes[12..]).ok_or("short header")?;
    if division & 0x8000 != 0 {
        return Err("SMPTE timing is not supported".into());
    }
    let ppq = division.max(1);
    let (mut notes, mut bpm, mut bar_ticks, mut end_tick) = (Vec::new(), None, None, 0u64);
    let mut i = 8 + header_len;
    for _ in 0..tracks {
        if bytes.get(i..i + 4) != Some(b"MTrk") {
            break;
        }
        let len = be32(&bytes[i + 4..]).ok_or("short track")? as usize;
        let end = (i + 8 + len).min(bytes.len());
        let mut p = i + 8;
        let (mut tick, mut running) = (0u64, 0u8);
        while p < end {
            let mut delta = 0u64;
            loop {
                let b = *bytes.get(p).ok_or("truncated")?;
                p += 1;
                delta = delta << 7 | (b & 0x7f) as u64;
                if b & 0x80 == 0 {
                    break;
                }
            }
            tick += delta;
            end_tick = end_tick.max(tick);
            let first = *bytes.get(p).ok_or("truncated")?;
            let status = if first & 0x80 != 0 {
                p += 1;
                first
            } else {
                running
            };
            match status {
                0xff => {
                    let kind = *bytes.get(p).ok_or("truncated")?;
                    p += 1;
                    let mut n = 0usize;
                    loop {
                        let b = *bytes.get(p).ok_or("truncated")?;
                        p += 1;
                        n = n << 7 | (b & 0x7f) as usize;
                        if b & 0x80 == 0 {
                            break;
                        }
                    }
                    let data = bytes.get(p..p + n).ok_or("truncated")?;
                    match kind {
                        0x51 if n == 3 && bpm.is_none() => {
                            let us = (data[0] as u32) << 16 | (data[1] as u32) << 8 | data[2] as u32;
                            if us > 0 {
                                bpm = Some(60_000_000.0 / us as f64);
                            }
                        }
                        0x58 if n >= 2 && bar_ticks.is_none() => {
                            let (num, den) = (data[0] as u64, 1u64 << data[1].min(6));
                            bar_ticks = Some(num * ppq as u64 * 4 / den);
                        }
                        _ => {}
                    }
                    p += n;
                }
                0xf0 | 0xf7 => {
                    let mut n = 0usize;
                    loop {
                        let b = *bytes.get(p).ok_or("truncated")?;
                        p += 1;
                        n = n << 7 | (b & 0x7f) as usize;
                        if b & 0x80 == 0 {
                            break;
                        }
                    }
                    p += n;
                }
                s if s >= 0x80 => {
                    running = s;
                    let data_len = if matches!(s & 0xf0, 0xc0 | 0xd0) { 1 } else { 2 };
                    let d = bytes.get(p..p + data_len).ok_or("truncated")?;
                    if s & 0xf0 == 0x90 && d[1] > 0 {
                        notes.push(Note { tick, note: d[0], vel: d[1] });
                    }
                    p += data_len;
                }
                _ => return Err("bad status byte".into()),
            }
        }
        i = end;
    }
    notes.sort_by_key(|n| (n.tick, n.note));
    Ok(Parsed { notes, ppq, bpm, bar_ticks, end_tick })
}

/// The drum a General MIDI note belongs to. Broad on purpose: other cymbals, congas and shakers
/// fall into the nearest of Pocket's eleven.
pub fn lane_of_gm(note: u8) -> Option<usize> {
    Some(match note {
        35 | 36 => LANE_KICK,
        38 | 40 => LANE_SNARE,
        37 => LANE_RIM,
        39 => LANE_CLAP,
        42 | 44 => LANE_HAT,
        46 => LANE_OPEN,
        41 | 43 | 45 => LANE_LTOM,
        47 | 48 | 50 => LANE_HTOM,
        49 | 52 | 55 | 57 => LANE_CRASH,
        51 | 53 | 59 => LANE_RIDE,
        54 | 56 | 58 | 60..=81 => LANE_PERC,
        _ => return None,
    })
}

pub struct Clip {
    pub cells: Vec<(usize, usize, Cell)>,
    pub bars: usize,
    pub bpm: Option<f64>,
    /// Notes that matched none of the drums.
    pub skipped: usize,
}

/// Snap parsed notes onto the grid.
pub fn to_clip(p: &Parsed) -> Clip {
    let ticks_per_step = p.ppq as f64 / 4.0;
    let bar_ticks = p.bar_ticks.unwrap_or(p.ppq as u64 * 4) as f64;
    // The clip's length: the last note's bar, so a file that ends on a long tail is not padded.
    let last = p.notes.last().map_or(0, |n| n.tick) as f64;
    let bars = (((last + ticks_per_step * 0.5) / bar_ticks).floor() as usize + 1).clamp(1, MAX_BARS);
    let steps = bars * 16;
    let mut grid: Vec<Option<Cell>> = vec![None; LANES * steps];
    // Positions in sixteenths of the bar, however the file divides it.
    let mut per_lane: Vec<Vec<(f64, u8)>> = vec![Vec::new(); LANES];
    let mut skipped = 0;
    for n in &p.notes {
        match lane_of_gm(n.note) {
            Some(lane) => per_lane[lane].push((n.tick as f64 * 16.0 / bar_ticks, n.vel)),
            None => skipped += 1,
        }
    }
    for (lane, hits) in per_lane.iter().enumerate() {
        let mut i = 0;
        while i < hits.len() {
            // Hits closer than about half a step belong to one roll, up to four, within one step.
            let mut j = i + 1;
            while j < hits.len() && j - i < MAX_RATCHET as usize && hits[j].0 - hits[j - 1].0 < 0.55 && hits[j].0 - hits[i].0 < 1.0 {
                j += 1;
            }
            let (pos, _) = hits[i];
            let vel = hits[i..j].iter().map(|h| h.1).max().unwrap_or(1);
            let step = pos.round() as i64;
            if (0..steps as i64).contains(&step) {
                let micro = ((pos - step as f64) * 100.0).round() as i32;
                let micro = if micro.abs() < 8 { 0 } else { micro.clamp(-50, 50) } as i8;
                let slot = &mut grid[lane * steps + step as usize];
                // Two clips landing on one step: keep the louder.
                if slot.is_none_or(|old| old.vel() < vel) {
                    *slot = Some(Cell::new(vel, micro, (j - i) as u8));
                }
            }
            i = j;
        }
    }
    let mut cells = Vec::new();
    for lane in 0..LANES {
        for step in 0..steps {
            if let Some(c) = grid[lane * steps + step] {
                cells.push((lane, step, c));
            }
        }
    }
    Clip { cells, bars, bpm: p.bpm, skipped }
}

pub fn read_clip(path: &Path) -> Result<Clip, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let clip = to_clip(&parse_smf(&bytes)?);
    if clip.cells.is_empty() {
        return Err("no drum notes in it".into());
    }
    Ok(clip)
}

impl Clip {
    /// Replace the pattern, apart from locked drums when `keep_locked`.
    pub fn load(&self, store: &PatternStore, keep_locked: bool, name: &str) {
        store.set_bars(1);
        store.set_bars(self.bars);
        for lane in 0..LANES {
            if !(keep_locked && store.locked(lane)) {
                store.clear_lane(lane);
            }
        }
        for &(lane, step, cell) in &self.cells {
            if !(keep_locked && store.locked(lane)) {
                store.set(lane, step, cell);
            }
        }
        store.set_name(name);
    }

    /// The first bar for a browser thumbnail.
    pub fn thumbnail(&self) -> [[u8; 16]; LANES] {
        let mut t = [[0u8; 16]; LANES];
        for &(lane, step, cell) in &self.cells {
            if step < 16 {
                t[lane][step] = cell.vel();
            }
        }
        t
    }
}

// ----------------------------------------------------------------------------------------------
// Folders

/// A folder of MIDI files, as the browser shows it.
#[derive(Default, Clone)]
pub struct Dir {
    pub name: String,
    pub path: PathBuf,
    pub dirs: Vec<Dir>,
    pub files: Vec<PathBuf>,
}

impl Dir {
    pub fn count(&self) -> usize {
        self.files.len() + self.dirs.iter().map(Dir::count).sum::<usize>()
    }

    pub fn find(&self, path: &[String]) -> Option<&Dir> {
        match path.split_first() {
            None => Some(self),
            Some((head, rest)) => self.dirs.iter().find(|d| &d.name == head)?.find(rest),
        }
    }

    pub fn all_files(&self, out: &mut Vec<PathBuf>) {
        out.extend(self.files.iter().cloned());
        for d in &self.dirs {
            d.all_files(out);
        }
    }
}

const MAX_DEPTH: usize = 8;
const MAX_FILES: usize = 20_000;

/// Walk `root` for `.mid` and `.midi` files. Folders with none below them are left out, and
/// hidden entries are skipped.
pub fn scan(root: &Path) -> Dir {
    let mut budget = MAX_FILES;
    let mut dir = walk(root, 0, &mut budget);
    dir.name = root.file_name().map_or_else(|| root.display().to_string(), |n| n.to_string_lossy().into_owned());
    dir
}

fn walk(path: &Path, depth: usize, budget: &mut usize) -> Dir {
    let mut dir = Dir {
        name: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        path: path.to_path_buf(),
        ..Dir::default()
    };
    let Ok(read) = std::fs::read_dir(path) else { return dir };
    let mut entries: Vec<_> = read.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name().to_string_lossy().to_lowercase());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let p = entry.path();
        // `file_type` does not follow symlinks, so a loop of links cannot trap the scan.
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_dir() && depth < MAX_DEPTH {
            let sub = walk(&p, depth + 1, budget);
            if sub.count() > 0 {
                dir.dirs.push(sub);
            }
        } else if kind.is_file() && *budget > 0 {
            let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase());
            if matches!(ext.as_deref(), Some("mid" | "midi")) {
                *budget -= 1;
                dir.files.push(p);
            }
        }
    }
    dir
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::midi::{self, Export};

    fn exported(store: &PatternStore) -> Vec<u8> {
        midi::to_midi(store, &Export { map: NoteMap::Gm, feel: Feel::default(), bpm: 133.0, only_lane: None, name: "T".into() })
    }

    #[test]
    fn our_own_export_comes_back_unchanged() {
        let store = PatternStore::default();
        store.set_bars(2);
        for (lane, step, cell) in [
            (LANE_KICK, 0, Cell::hit(120)),
            (LANE_SNARE, 4, Cell::hit(100)),
            (LANE_HAT, 6, Cell::new(80, 20, 1)),
            (LANE_HAT, 22, Cell::new(90, 0, 3)),
            (LANE_OPEN, 31, Cell::hit(70)),
            (LANE_CRASH, 16, Cell::hit(110)),
            (LANE_PERC, 9, Cell::hit(60)),
        ] {
            store.set(lane, step, cell);
        }
        let clip = to_clip(&parse_smf(&exported(&store)).unwrap());
        assert_eq!(clip.bars, 2);
        assert!((clip.bpm.unwrap() - 133.0).abs() < 0.01);
        let back = PatternStore::default();
        clip.load(&back, false, "Back");
        for lane in 0..LANES {
            for step in 0..32 {
                let (a, b) = (store.get(lane, step), back.get(lane, step));
                assert_eq!(a.vel() > 0, b.vel() > 0, "lane {lane} step {step}");
                if a.is_on() {
                    assert_eq!(a.vel(), b.vel().max(a.vel().min(b.vel())), "velocity of lane {lane} step {step}");
                    assert!((a.micro() as i32 - b.micro() as i32).abs() <= 2, "nudge of lane {lane} step {step}: {} vs {}", a.micro(), b.micro());
                }
            }
        }
        assert_eq!(back.get(LANE_HAT, 22).ratchet(), 3, "a roll survives the round trip");
        assert_eq!(back.name(), "Back");
    }

    #[test]
    fn gm_notes_land_on_the_right_drums() {
        for (note, lane) in [(36, LANE_KICK), (38, LANE_SNARE), (42, LANE_HAT), (46, LANE_OPEN), (49, LANE_CRASH), (51, LANE_RIDE), (45, LANE_LTOM), (50, LANE_HTOM), (39, LANE_CLAP), (37, LANE_RIM), (56, LANE_PERC)] {
            assert_eq!(lane_of_gm(note), Some(lane), "note {note}");
        }
        assert_eq!(lane_of_gm(10), None);
        // Every lane's own GM note is understood.
        for (lane, info) in LANE_INFO.iter().enumerate() {
            assert_eq!(lane_of_gm(info.gm), Some(lane), "{}", info.name);
        }
    }

    /// A tiny hand-built file: one track, 96 ticks per quarter, running status and a velocity-0 off.
    fn tiny_file() -> Vec<u8> {
        let mut t = vec![];
        t.extend([0x00, 0xff, 0x51, 0x03, 0x0a, 0x2c, 0x2a]); // 90 bpm
        t.extend([0x00, 0xff, 0x58, 0x04, 0x03, 0x02, 0x18, 0x08]); // 3/4
        t.extend([0x00, 0x99, 36, 110]); // kick on the downbeat
        t.extend([0x0c, 0x99, 36, 0]); // note off as velocity 0
        t.extend([0x24, 38, 90]); // running status: snare 48 ticks later (two sixteenths)
        t.extend([0x00, 0xff, 0x2f, 0x00]);
        let mut f = b"MThd".to_vec();
        f.extend([0, 0, 0, 6, 0, 1, 0, 1, 0, 96]);
        f.extend(b"MTrk");
        f.extend((t.len() as u32).to_be_bytes());
        f.extend(t);
        f
    }

    #[test]
    fn a_hand_built_file_with_running_status_and_a_waltz_bar_reads() {
        let p = parse_smf(&tiny_file()).unwrap();
        assert_eq!(p.ppq, 96);
        assert!((p.bpm.unwrap() - 90.0).abs() < 0.01);
        assert_eq!(p.bar_ticks, Some(288));
        assert_eq!(p.notes, vec![Note { tick: 0, note: 36, vel: 110 }, Note { tick: 48, note: 38, vel: 90 }]);
        let clip = to_clip(&p);
        // In 3/4 a bar is 288 ticks, so the snare 48 ticks in is a sixth of the way: step 2.67 -> 3
        // of the 16 the bar is divided into.
        assert_eq!(clip.bars, 1);
        assert!(clip.cells.iter().any(|c| c.0 == LANE_KICK && c.1 == 0));
        assert!(clip.cells.iter().any(|c| c.0 == LANE_SNARE && c.1 == 3));
    }

    #[test]
    fn two_hits_on_one_step_become_a_roll_and_off_grid_hits_become_nudges() {
        let p = Parsed {
            notes: vec![
                Note { tick: 0, note: 38, vel: 80 },
                Note { tick: 10, note: 38, vel: 100 },
                Note { tick: 120, note: 42, vel: 90 },
                Note { tick: 120 + 15, note: 42, vel: 90 },
            ],
            ppq: 480,
            bpm: None,
            bar_ticks: None,
            end_tick: 135,
        };
        let clip = to_clip(&p);
        let snare = clip.cells.iter().find(|c| c.0 == LANE_SNARE).unwrap();
        assert_eq!((snare.2.ratchet(), snare.2.vel()), (2, 100));
        let hats: Vec<_> = clip.cells.iter().filter(|c| c.0 == LANE_HAT).collect();
        assert_eq!(hats.len(), 1, "two hits 15 ticks apart are one roll: {hats:?}");
        assert_eq!(hats[0].2.ratchet(), 2);
    }

    #[test]
    fn unknown_notes_are_counted_and_long_files_are_cut_to_four_bars() {
        let mut notes = vec![Note { tick: 0, note: 3, vel: 100 }];
        for bar in 0..8u64 {
            notes.push(Note { tick: bar * 1920, note: 36, vel: 100 });
        }
        let clip = to_clip(&Parsed { notes, ppq: 480, bpm: None, bar_ticks: None, end_tick: 7 * 1920 });
        assert_eq!(clip.skipped, 1);
        assert_eq!(clip.bars, MAX_BARS);
        assert_eq!(clip.cells.len(), 4);
    }

    #[test]
    fn broken_files_are_errors_not_panics() {
        assert!(parse_smf(b"nope").is_err());
        assert!(parse_smf(b"MThd\0\0\0\x06\0\0\0\x01\x80\x01").is_err(), "SMPTE timing");
        let mut cut = tiny_file();
        cut.truncate(cut.len() - 6);
        assert!(parse_smf(&cut).is_err());
        // Random bytes after a valid header never panic.
        let mut seed = 7u32;
        for _ in 0..200 {
            let mut f = tiny_file();
            for b in f.iter_mut().skip(14) {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                if seed % 5 == 0 {
                    *b = seed as u8;
                }
            }
            let _ = parse_smf(&f);
        }
    }

    #[test]
    fn locked_drums_survive_an_import() {
        let store = PatternStore::default();
        store.set(LANE_KICK, 3, Cell::hit(77));
        store.toggle_lock(LANE_KICK);
        let clip = to_clip(&parse_smf(&tiny_file()).unwrap());
        clip.load(&store, true, "X");
        assert_eq!(store.get(LANE_KICK, 3).vel(), 77);
        assert!(!store.get(LANE_KICK, 0).is_on());
        assert!(store.get(LANE_SNARE, 3).is_on());
    }

    #[test]
    fn scanning_finds_midi_files_in_folders_and_skips_the_rest() {
        let root = std::env::temp_dir().join(format!("pocket-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (dir, file) in [("Rock/Fast", "a.mid"), ("Rock", "b.MID"), ("Jazz", "c.midi"), ("Empty", "notes.txt"), (".hidden", "d.mid"), ("Jazz", "e.wav")] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
            std::fs::write(root.join(dir).join(file), b"x").unwrap();
        }
        let d = scan(&root);
        assert_eq!(d.count(), 3);
        let names: Vec<_> = d.dirs.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["Jazz", "Rock"], "sorted, with the empty and hidden folders left out");
        assert_eq!(d.find(&["Rock".into(), "Fast".into()]).unwrap().files.len(), 1);
        let mut all = Vec::new();
        d.all_files(&mut all);
        assert_eq!(all.len(), 3);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_midi_file_found_on_this_machine_reads_if_present() {
        // A local smoke test against a real file; silently skipped elsewhere.
        if let Ok(bytes) = std::fs::read("/private/tmp/mtpdk.mid") {
            let p = parse_smf(&bytes).expect("a real file parses");
            let clip = to_clip(&p);
            assert!(clip.bars >= 1);
        }
    }
}
