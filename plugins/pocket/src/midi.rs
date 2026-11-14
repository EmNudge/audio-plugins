//! Standard MIDI files, for dragging a groove into a DAW or saving it to disk.

use crate::pattern::*;
use std::path::{Path, PathBuf};

pub const PPQ: u32 = 480;
/// MIDI channel 10, which General MIDI reserves for drums.
const CHANNEL: u8 = 9;

pub struct Export {
    pub map: NoteMap,
    pub feel: Feel,
    pub bpm: f64,
    /// Only this lane, for dragging one drum out on its own.
    pub only_lane: Option<usize>,
    pub name: String,
}

fn var_len(out: &mut Vec<u8>, mut value: u32) {
    let mut bytes = [0u8; 5];
    let mut n = 0;
    loop {
        bytes[n] = (value & 0x7f) as u8 | if n > 0 { 0x80 } else { 0 };
        n += 1;
        value >>= 7;
        if value == 0 {
            break;
        }
    }
    out.extend(bytes[..n].iter().rev());
}

struct Ev {
    tick: u32,
    /// Note offs sort before ons at the same tick so repeated notes retrigger cleanly.
    on: bool,
    note: u8,
    vel: u8,
}

/// A format-0 file holding one pass through the pattern, as long as the pattern is, so the clip
/// loops cleanly in the host. Muted and un-soloed lanes are left out.
pub fn to_midi(store: &PatternStore, e: &Export) -> Vec<u8> {
    let steps = store.steps();
    let loop_ticks = steps as u32 * PPQ / 4;
    let mut events = Vec::new();
    for lane in 0..LANES {
        if e.only_lane.is_some_and(|l| l != lane) || !(e.only_lane.is_some() || store.audible(lane)) {
            continue;
        }
        let note = e.map.note(lane);
        for step in 0..steps {
            let (hits, n) = cell_hits(step, lane, store.get(lane, step), e.feel, 0);
            for hit in &hits[..n] {
                let on = ((hit.beat * PPQ as f64).round().max(0.0) as u32).min(loop_ticks.saturating_sub(2));
                let len = ((hit.len * PPQ as f64).round() as u32).max(10);
                let off = (on + len).min(loop_ticks.saturating_sub(1)).max(on + 1);
                events.push(Ev { tick: on, on: true, note, vel: hit.vel });
                events.push(Ev { tick: off, on: false, note, vel: 0 });
            }
        }
    }
    events.sort_by_key(|ev| (ev.tick, ev.on, ev.note));

    let mut track = Vec::new();
    // Track name.
    var_len(&mut track, 0);
    track.extend([0xff, 0x03]);
    var_len(&mut track, e.name.len() as u32);
    track.extend(e.name.as_bytes());
    // Tempo, in microseconds per quarter note.
    let us = (60_000_000.0 / e.bpm.clamp(20.0, 400.0)).round() as u32;
    var_len(&mut track, 0);
    track.extend([0xff, 0x51, 0x03, (us >> 16) as u8, (us >> 8) as u8, us as u8]);
    // 4/4.
    var_len(&mut track, 0);
    track.extend([0xff, 0x58, 0x04, 4, 2, 24, 8]);
    let mut last = 0;
    for ev in &events {
        var_len(&mut track, ev.tick - last);
        last = ev.tick;
        track.extend([if ev.on { 0x90 } else { 0x80 } | CHANNEL, ev.note, ev.vel]);
    }
    var_len(&mut track, loop_ticks.saturating_sub(last));
    track.extend([0xff, 0x2f, 0x00]);

    let mut file = Vec::new();
    file.extend(b"MThd");
    file.extend(6u32.to_be_bytes());
    file.extend(0u16.to_be_bytes());
    file.extend(1u16.to_be_bytes());
    file.extend((PPQ as u16).to_be_bytes());
    file.extend(b"MTrk");
    file.extend((track.len() as u32).to_be_bytes());
    file.extend(track);
    file
}

/// A file name that is safe on every platform.
pub fn file_stem(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == ' ' { c } else { '_' })
        .collect();
    let trimmed = cleaned.trim().trim_matches('_');
    if trimmed.is_empty() { "Groove".into() } else { trimmed.to_owned() }
}

/// Write the file under `dir` and return its path.
pub fn write_file(dir: &Path, stem: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("{}.mid", file_stem(stem)));
    std::fs::write(&path, bytes)?;
    Ok(path)
}

/// Where dragged-out clips are written. The OS cleans temporary folders up.
pub fn drag_dir() -> PathBuf {
    std::env::temp_dir().join("Clowd Pocket")
}

/// Where "save" puts clips, so they can be found again.
pub fn save_dir() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    home.join("Music").join("Clowd Pocket")
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Minimal reader: `(tick, on, note, vel)` plus tempo and end-of-track tick.
    pub struct Parsed {
        pub notes: Vec<(u32, bool, u8, u8)>,
        pub tempo_us: u32,
        pub end: u32,
        pub division: u16,
        pub format: u16,
    }

    pub fn parse(bytes: &[u8]) -> Parsed {
        assert_eq!(&bytes[..4], b"MThd");
        let format = u16::from_be_bytes([bytes[8], bytes[9]]);
        let division = u16::from_be_bytes([bytes[12], bytes[13]]);
        assert_eq!(&bytes[14..18], b"MTrk");
        let len = u32::from_be_bytes(bytes[18..22].try_into().unwrap()) as usize;
        assert_eq!(bytes.len(), 22 + len, "track length must cover the file");
        let mut i = 22;
        let (mut tick, mut notes, mut tempo_us, mut end) = (0u32, Vec::new(), 0, 0);
        while i < bytes.len() {
            let mut delta = 0u32;
            loop {
                let b = bytes[i];
                i += 1;
                delta = delta << 7 | (b & 0x7f) as u32;
                if b & 0x80 == 0 {
                    break;
                }
            }
            tick += delta;
            match bytes[i] {
                0xff => {
                    let kind = bytes[i + 1];
                    let n = bytes[i + 2] as usize;
                    let data = &bytes[i + 3..i + 3 + n];
                    match kind {
                        0x51 => tempo_us = (data[0] as u32) << 16 | (data[1] as u32) << 8 | data[2] as u32,
                        0x2f => end = tick,
                        _ => {}
                    }
                    i += 3 + n;
                }
                status => {
                    assert_eq!(status & 0x0f, CHANNEL, "drums belong on channel 10");
                    notes.push((tick, status & 0xf0 == 0x90, bytes[i + 1], bytes[i + 2]));
                    i += 3;
                }
            }
        }
        Parsed { notes, tempo_us, end, division, format }
    }

    fn export(store: &PatternStore) -> Vec<u8> {
        to_midi(store, &Export { map: NoteMap::Gm, feel: Feel::default(), bpm: 120.0, only_lane: None, name: "T".into() })
    }

    #[test]
    fn variable_length_quantities_match_the_spec_examples() {
        for (value, bytes) in [(0u32, vec![0u8]), (0x7f, vec![0x7f]), (0x80, vec![0x81, 0x00]), (0x3fff, vec![0xff, 0x7f]), (0x4000, vec![0x81, 0x80, 0x00])] {
            let mut out = Vec::new();
            var_len(&mut out, value);
            assert_eq!(out, bytes, "{value:#x}");
        }
    }

    #[test]
    fn a_backbeat_exports_the_right_notes_at_the_right_ticks() {
        let store = PatternStore::default();
        store.set(LANE_KICK, 0, Cell::hit(120));
        store.set(LANE_SNARE, 4, Cell::hit(100));
        store.set(LANE_SNARE, 12, Cell::hit(90));
        let p = parse(&export(&store));
        assert_eq!((p.format, p.division), (0, 480));
        assert_eq!(p.tempo_us, 500_000);
        let ons: Vec<_> = p.notes.iter().filter(|n| n.1).map(|n| (n.0, n.2, n.3)).collect();
        assert_eq!(ons, vec![(0, 36, 120), (480, 38, 100), (1440, 38, 90)]);
        assert_eq!(p.notes.len(), 6, "every note on has an off");
        assert_eq!(p.end, 1920, "one bar is four quarter notes");
    }

    #[test]
    fn two_bars_make_a_two_bar_clip() {
        let store = PatternStore::default();
        store.set_bars(2);
        store.set(LANE_KICK, 16, Cell::hit(100));
        let p = parse(&export(&store));
        assert_eq!(p.end, 3840);
        assert!(p.notes.iter().any(|n| n.1 && n.0 == 1920));
    }

    #[test]
    fn swing_and_ratchets_reach_the_file() {
        let store = PatternStore::default();
        store.set(LANE_HAT, 1, Cell::hit(100));
        store.set(LANE_HAT, 8, Cell::new(100, 0, 2));
        let feel = Feel { swing: 2.0 / 3.0, ..Feel::default() };
        let p = parse(&to_midi(&store, &Export { map: NoteMap::Gm, feel, bpm: 90.0, only_lane: None, name: "S".into() }));
        let ticks: Vec<u32> = p.notes.iter().filter(|n| n.1).map(|n| n.0).collect();
        assert_eq!(ticks, vec![160, 960, 960 + 60]);
        assert_eq!(p.tempo_us, 666_667);
    }

    #[test]
    fn mutes_solos_and_single_lanes_filter_the_file() {
        let store = PatternStore::default();
        store.set(LANE_KICK, 0, Cell::hit(100));
        store.set(LANE_SNARE, 4, Cell::hit(100));
        store.toggle_mute(LANE_KICK);
        let notes = |p: &Parsed| p.notes.iter().filter(|n| n.1).map(|n| n.2).collect::<Vec<_>>();
        assert_eq!(notes(&parse(&export(&store))), vec![38]);
        store.toggle_mute(LANE_KICK);
        let only = to_midi(&store, &Export { map: NoteMap::Pads, feel: Feel::default(), bpm: 120.0, only_lane: Some(LANE_KICK), name: "K".into() });
        assert_eq!(notes(&parse(&only)), vec![36]);
        // A single lane dragged out ignores a mute: you asked for it by name.
        store.toggle_mute(LANE_KICK);
        let again = to_midi(&store, &Export { map: NoteMap::Gm, feel: Feel::default(), bpm: 120.0, only_lane: Some(LANE_KICK), name: "K".into() });
        assert_eq!(notes(&parse(&again)), vec![36]);
    }

    #[test]
    fn repeated_notes_retrigger_off_before_on() {
        let store = PatternStore::default();
        store.set(LANE_HAT, 0, Cell::new(100, 0, 4));
        let p = parse(&export(&store));
        let mut held = false;
        for (_, on, ..) in &p.notes {
            if *on {
                assert!(!held, "note on while the same note is still held");
            }
            held = *on;
        }
    }

    #[test]
    fn events_never_run_past_the_end_of_the_loop() {
        let store = PatternStore::default();
        store.set(LANE_SNARE, 15, Cell::new(100, 50, 4));
        store.set(LANE_KICK, 0, Cell::new(100, -50, 1));
        let p = parse(&export(&store));
        assert!(p.notes.iter().all(|n| n.0 < p.end));
    }

    #[test]
    fn file_names_are_sanitised() {
        assert_eq!(file_stem("Boom Bap #12"), "Boom Bap _12");
        assert_eq!(file_stem("../../x"), "x");
        assert_eq!(file_stem(""), "Groove");
    }
}
