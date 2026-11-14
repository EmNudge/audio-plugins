//! The pattern the editor draws, the generator fills and the audio thread plays.
//!
//! Every cell is one `AtomicU32`, so the UI can edit while the audio thread reads without a lock.
//! Time is counted in beats (quarter notes) and the grid is sixteenths, so one bar is 16 steps.

use nih_plug::params::persist::PersistentField;
use nih_plug::prelude::Enum;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

pub const LANES: usize = 11;
pub const MAX_STEPS: usize = 64;
pub const MAX_BARS: usize = 4;
pub const STEP_BEATS: f64 = 0.25;
/// Ratchets repeat a hit this many times inside its step.
pub const MAX_RATCHET: u8 = 4;

/// Which colour a lane takes: the family of drum it is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Family {
    Kick,
    Snare,
    Cymbal,
    Tom,
}

pub struct Lane {
    pub name: &'static str,
    pub short: &'static str,
    pub family: Family,
    /// General MIDI drum note.
    pub gm: u8,
}

pub const LANE_INFO: [Lane; LANES] = [
    Lane { name: "KICK", short: "KCK", family: Family::Kick, gm: 36 },
    Lane { name: "SNARE", short: "SNR", family: Family::Snare, gm: 38 },
    Lane { name: "RIM", short: "RIM", family: Family::Snare, gm: 37 },
    Lane { name: "CLAP", short: "CLP", family: Family::Snare, gm: 39 },
    Lane { name: "HAT", short: "HAT", family: Family::Cymbal, gm: 42 },
    Lane { name: "OPEN", short: "OPN", family: Family::Cymbal, gm: 46 },
    Lane { name: "LO TOM", short: "LTM", family: Family::Tom, gm: 45 },
    Lane { name: "HI TOM", short: "HTM", family: Family::Tom, gm: 50 },
    Lane { name: "CRASH", short: "CRS", family: Family::Cymbal, gm: 49 },
    Lane { name: "RIDE", short: "RDE", family: Family::Cymbal, gm: 51 },
    Lane { name: "PERC", short: "PRC", family: Family::Tom, gm: 56 },
];

pub const LANE_KICK: usize = 0;
pub const LANE_SNARE: usize = 1;
pub const LANE_RIM: usize = 2;
pub const LANE_CLAP: usize = 3;
pub const LANE_HAT: usize = 4;
pub const LANE_OPEN: usize = 5;
pub const LANE_LTOM: usize = 6;
pub const LANE_HTOM: usize = 7;
pub const LANE_CRASH: usize = 8;
pub const LANE_RIDE: usize = 9;
pub const LANE_PERC: usize = 10;

/// How lanes map to MIDI notes.
#[derive(Enum, Clone, Copy, PartialEq, Eq, Debug)]
pub enum NoteMap {
    /// General MIDI drums, which is what most drum VSTs and GM kits expect.
    #[name = "General MIDI"]
    Gm,
    /// Eleven pads from C1 upward, as on a drum rack or an MPC.
    #[name = "Pads from C1"]
    Pads,
}

impl NoteMap {
    pub fn note(self, lane: usize) -> u8 {
        match self {
            NoteMap::Gm => LANE_INFO[lane].gm,
            NoteMap::Pads => 36 + lane as u8,
        }
    }

    /// The lane a note plays, for the audition kit.
    pub fn lane_of(self, note: u8) -> Option<usize> {
        (0..LANES).find(|&l| self.note(l) == note)
    }
}

/// One step of one lane. 0 velocity is empty.
///
/// Bits 0-7 velocity, 8-15 timing nudge as a signed percent of a step, 16-17 ratchet count - 1.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct Cell(pub u32);

impl Cell {
    pub fn new(vel: u8, micro: i8, ratchet: u8) -> Self {
        let vel = vel.min(127) as u32;
        let micro = micro.clamp(-50, 50) as u8 as u32;
        let ratchet = (ratchet.clamp(1, MAX_RATCHET) - 1) as u32;
        Cell(vel | micro << 8 | ratchet << 16)
    }

    pub fn hit(vel: u8) -> Self {
        Self::new(vel, 0, 1)
    }

    pub fn vel(self) -> u8 {
        (self.0 & 0xff) as u8
    }

    pub fn micro(self) -> i8 {
        ((self.0 >> 8) & 0xff) as u8 as i8
    }

    pub fn ratchet(self) -> u8 {
        ((self.0 >> 16) & 3) as u8 + 1
    }

    pub fn is_on(self) -> bool {
        self.vel() > 0
    }

    pub fn with_vel(self, vel: u8) -> Self {
        Self::new(vel, self.micro(), self.ratchet())
    }

    pub fn with_micro(self, micro: i8) -> Self {
        Self::new(self.vel(), micro, self.ratchet())
    }

    pub fn with_ratchet(self, ratchet: u8) -> Self {
        Self::new(self.vel(), self.micro(), ratchet)
    }
}

fn default_stride() -> u32 {
    32
}

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct Saved {
    pub steps: u32,
    /// Steps per lane in `cells`. Saves from before four-bar patterns used 32.
    #[serde(default = "default_stride")]
    pub stride: u32,
    pub cells: Vec<u32>,
    pub mute: u32,
    pub solo: u32,
    pub lock: u32,
    pub name: String,
}

pub struct PatternStore {
    cells: [AtomicU32; LANES * MAX_STEPS],
    steps: AtomicU32,
    mute: AtomicU32,
    solo: AtomicU32,
    lock: AtomicU32,
    /// Bumped on every edit so the UI can tell when to redraw cached things.
    rev: AtomicU32,
    /// Only the editor touches this; the audio thread never locks it.
    name: Mutex<String>,
}

impl Default for PatternStore {
    fn default() -> Self {
        Self {
            cells: std::array::from_fn(|_| AtomicU32::new(0)),
            steps: AtomicU32::new(16),
            mute: AtomicU32::new(0),
            solo: AtomicU32::new(0),
            lock: AtomicU32::new(0),
            rev: AtomicU32::new(0),
            name: Mutex::new(String::new()),
        }
    }
}

impl PatternStore {
    pub fn steps(&self) -> usize {
        self.steps.load(Ordering::Relaxed).clamp(1, MAX_STEPS as u32) as usize
    }

    pub fn bars(&self) -> usize {
        self.steps().div_ceil(16)
    }

    /// Change the length to 1-4 bars. Growing repeats what is already there.
    pub fn set_bars(&self, bars: usize) {
        let steps = bars.clamp(1, MAX_BARS) * 16;
        let old = self.steps();
        if steps == old {
            return;
        }
        if steps > old {
            for lane in 0..LANES {
                for s in old..steps {
                    self.set(lane, s, self.get(lane, s % old));
                }
            }
        }
        self.steps.store(steps as u32, Ordering::Relaxed);
        self.touch();
    }

    pub fn get(&self, lane: usize, step: usize) -> Cell {
        Cell(self.cells[lane * MAX_STEPS + step].load(Ordering::Relaxed))
    }

    pub fn set(&self, lane: usize, step: usize, cell: Cell) {
        self.cells[lane * MAX_STEPS + step].store(cell.0, Ordering::Relaxed);
        self.touch();
    }

    pub fn clear_lane(&self, lane: usize) {
        for s in 0..MAX_STEPS {
            self.cells[lane * MAX_STEPS + s].store(0, Ordering::Relaxed);
        }
        self.touch();
    }

    pub fn clear(&self) {
        for lane in 0..LANES {
            self.clear_lane(lane);
        }
    }

    pub fn touch(&self) {
        self.rev.fetch_add(1, Ordering::Relaxed);
    }

    pub fn rev(&self) -> u32 {
        self.rev.load(Ordering::Relaxed)
    }

    pub fn is_empty(&self) -> bool {
        self.cells.iter().all(|c| c.load(Ordering::Relaxed) & 0xff == 0)
    }

    pub fn hit_count(&self) -> usize {
        let steps = self.steps();
        (0..LANES).map(|l| (0..steps).filter(|&s| self.get(l, s).is_on()).count()).sum()
    }

    pub fn name(&self) -> String {
        self.name.lock().map(|n| n.clone()).unwrap_or_default()
    }

    pub fn set_name(&self, name: &str) {
        if let Ok(mut n) = self.name.lock() {
            *n = name.to_owned();
        }
    }

    fn mask(atom: &AtomicU32, lane: usize) -> bool {
        atom.load(Ordering::Relaxed) >> lane & 1 == 1
    }

    fn flip(atom: &AtomicU32, lane: usize) {
        atom.fetch_xor(1 << lane, Ordering::Relaxed);
    }

    pub fn muted(&self, lane: usize) -> bool {
        Self::mask(&self.mute, lane)
    }
    pub fn soloed(&self, lane: usize) -> bool {
        Self::mask(&self.solo, lane)
    }
    pub fn locked(&self, lane: usize) -> bool {
        Self::mask(&self.lock, lane)
    }
    pub fn toggle_mute(&self, lane: usize) {
        Self::flip(&self.mute, lane);
    }
    pub fn toggle_solo(&self, lane: usize) {
        Self::flip(&self.solo, lane);
    }
    pub fn toggle_lock(&self, lane: usize) {
        Self::flip(&self.lock, lane);
    }

    /// Whether the lane should sound, given the mutes and solos.
    pub fn audible(&self, lane: usize) -> bool {
        let solo = self.solo.load(Ordering::Relaxed);
        !self.muted(lane) && (solo == 0 || solo >> lane & 1 == 1)
    }

    pub fn save(&self) -> Saved {
        Saved {
            steps: self.steps() as u32,
            stride: MAX_STEPS as u32,
            cells: self.cells.iter().map(|c| c.load(Ordering::Relaxed)).collect(),
            mute: self.mute.load(Ordering::Relaxed),
            solo: self.solo.load(Ordering::Relaxed),
            lock: self.lock.load(Ordering::Relaxed),
            name: self.name(),
        }
    }

    pub fn load(&self, saved: &Saved) {
        self.steps.store((saved.steps.clamp(16, MAX_STEPS as u32).div_ceil(16) * 16) as u32, Ordering::Relaxed);
        let stride = (saved.stride as usize).max(1);
        for lane in 0..LANES {
            for step in 0..MAX_STEPS {
                let value = if step < stride { saved.cells.get(lane * stride + step).copied().unwrap_or(0) } else { 0 };
                self.cells[lane * MAX_STEPS + step].store(value, Ordering::Relaxed);
            }
        }
        self.mute.store(saved.mute, Ordering::Relaxed);
        self.solo.store(saved.solo, Ordering::Relaxed);
        self.lock.store(saved.lock, Ordering::Relaxed);
        self.set_name(&saved.name);
        self.touch();
    }
}

impl<'a> PersistentField<'a, Saved> for PatternStore {
    fn set(&self, new_value: Saved) {
        self.load(&new_value);
    }

    fn map<F, R>(&self, f: F) -> R
    where
        F: Fn(&Saved) -> R,
    {
        f(&self.save())
    }
}

/// A cheap deterministic hash to a number in 0..1, for humanising without a random state.
pub fn hash01(a: u32, b: u32, c: u32) -> f32 {
    let mut h = a.wrapping_mul(0x9E37_79B1) ^ b.wrapping_mul(0x85EB_CA77) ^ c.wrapping_mul(0xC2B2_AE3D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^= h >> 15;
    (h >> 8) as f32 / (1u32 << 24) as f32
}

/// One sounding hit, in beats from the start of the loop.
#[derive(Clone, Copy, Default, Debug)]
pub struct Hit {
    pub beat: f64,
    pub len: f64,
    pub vel: u8,
}

pub const MAX_HITS: usize = MAX_RATCHET as usize;

/// How a pattern is played: the feel on top of the grid.
#[derive(Clone, Copy, Debug)]
pub struct Feel {
    /// Length of the first of each pair as a fraction of the pair: 0.5 straight, 0.667 triplet.
    pub swing: f32,
    /// Swing pairs up eighths (a shuffle) instead of sixteenths (an MPC-style bounce).
    pub eighths: bool,
    /// 0..1.
    pub humanize: f32,
}

impl Default for Feel {
    fn default() -> Self {
        Self { swing: 0.5, eighths: false, humanize: 0.0 }
    }
}

impl Feel {
    /// Bend a position in steps so the second half of each pair lands late (or early).
    pub fn warp(&self, pos: f64) -> f64 {
        let swing = self.swing as f64;
        let unit = if self.eighths { 2.0 } else { 1.0 };
        let pair = unit * 2.0;
        let k = (pos / pair).floor();
        let local = pos - k * pair;
        let bent = if local < unit { local * 2.0 * swing } else { pair * swing + (local - unit) * 2.0 * (1.0 - swing) };
        k * pair + bent
    }
}

impl Feel {
    /// The inverse of `warp`: where on the straight grid a hit played at `pos` (in steps of the
    /// swung groove) belongs.
    pub fn unwarp(&self, pos: f64) -> f64 {
        let swing = (self.swing as f64).clamp(0.01, 0.99);
        let unit = if self.eighths { 2.0 } else { 1.0 };
        let pair = unit * 2.0;
        let k = (pos / pair).floor();
        let local = pos - k * pair;
        let straight = if local < pair * swing { local / (2.0 * swing) } else { unit + (local - pair * swing) / (2.0 * (1.0 - swing)) };
        k * pair + straight
    }
}

/// The hits a cell makes. `pass` is the loop count, so humanising differs each time round.
pub fn cell_hits(step: usize, lane: usize, cell: Cell, feel: Feel, pass: u32) -> ([Hit; MAX_HITS], usize) {
    let mut out = [Hit::default(); MAX_HITS];
    if !cell.is_on() {
        return (out, 0);
    }
    let n = cell.ratchet() as usize;
    let base = feel.warp(step as f64 + cell.micro() as f64 / 100.0) * STEP_BEATS;
    // Humanising moves a hit by up to about 12 ms at 120 bpm and its velocity by up to 18.
    let h = feel.humanize as f64;
    let time_jitter = (hash01(step as u32, lane as u32, pass) as f64 - 0.5) * 2.0 * h * 0.024;
    let vel_jitter = (hash01(step as u32 + 97, lane as u32, pass) - 0.5) * 2.0 * feel.humanize * 18.0;
    let slot = STEP_BEATS / n as f64;
    for (j, hit) in out.iter_mut().enumerate().take(n) {
        // Ratchets fade: a roll that gets quieter reads as a drag, not a machine gun.
        let fade = 1.0 - 0.14 * j as f32;
        hit.beat = base + slot * j as f64 + if j == 0 { time_jitter } else { 0.0 };
        hit.len = (slot * 0.8).min(0.125);
        hit.vel = (cell.vel() as f32 * fade + vel_jitter).round().clamp(1.0, 127.0) as u8;
    }
    (out, n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_pack_and_unpack() {
        let c = Cell::new(99, -30, 3);
        assert_eq!((c.vel(), c.micro(), c.ratchet()), (99, -30, 3));
        assert!(!Cell::default().is_on());
        assert_eq!(c.with_vel(0), Cell::new(0, -30, 3));
        assert_eq!(Cell::new(200, 90, 9).vel(), 127);
        assert_eq!(Cell::new(200, 90, 9).micro(), 50);
        assert_eq!(Cell::new(200, 90, 9).ratchet(), MAX_RATCHET);
    }

    #[test]
    fn store_round_trips_through_saved() {
        let a = PatternStore::default();
        a.set_bars(2);
        a.set(LANE_KICK, 0, Cell::hit(120));
        a.set(LANE_SNARE, 20, Cell::new(80, 12, 2));
        a.toggle_mute(LANE_HAT);
        a.toggle_lock(LANE_KICK);
        a.set_name("Test");
        let json = serde_json::to_string(&a.save()).unwrap();
        let b = PatternStore::default();
        b.load(&serde_json::from_str(&json).unwrap());
        assert_eq!(b.steps(), 32);
        assert_eq!(b.get(LANE_SNARE, 20), Cell::new(80, 12, 2));
        assert!(b.muted(LANE_HAT) && b.locked(LANE_KICK) && !b.soloed(LANE_KICK));
        assert_eq!(b.name(), "Test");
    }

    #[test]
    fn old_saves_with_a_32_step_stride_still_load() {
        let mut saved = Saved { steps: 16, stride: 32, cells: vec![0; LANES * 32], ..Saved::default() };
        saved.cells[LANE_SNARE * 32 + 4] = Cell::hit(99).0;
        let a = PatternStore::default();
        a.load(&saved);
        assert_eq!(a.get(LANE_SNARE, 4).vel(), 99);
        assert_eq!(a.get(LANE_KICK, 0).vel(), 0);
        let json = r#"{"steps":16,"cells":[],"mute":0,"solo":0,"lock":0,"name":""}"#;
        let parsed: Saved = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.stride, 32);
    }

    #[test]
    fn four_bars_repeat_the_pattern_and_lengths_are_whole_bars() {
        let a = PatternStore::default();
        a.set(LANE_KICK, 3, Cell::hit(100));
        a.set_bars(4);
        assert_eq!((a.steps(), a.bars()), (64, 4));
        assert_eq!(a.get(LANE_KICK, 3 + 48).vel(), 100);
        a.set_bars(3);
        assert_eq!(a.steps(), 48);
        a.set_bars(9);
        assert_eq!(a.steps(), 64);
    }

    #[test]
    fn going_to_two_bars_copies_and_back_truncates() {
        let a = PatternStore::default();
        a.set(LANE_KICK, 4, Cell::hit(100));
        a.set_bars(2);
        assert_eq!(a.get(LANE_KICK, 20), Cell::hit(100));
        a.set(LANE_KICK, 21, Cell::hit(90));
        a.set_bars(1);
        assert_eq!(a.steps(), 16);
        assert_eq!(a.bars(), 1);
    }

    #[test]
    fn solo_beats_unmuted_lanes() {
        let a = PatternStore::default();
        assert!(a.audible(LANE_KICK));
        a.toggle_solo(LANE_SNARE);
        assert!(!a.audible(LANE_KICK) && a.audible(LANE_SNARE));
        a.toggle_mute(LANE_SNARE);
        assert!(!a.audible(LANE_SNARE));
    }

    #[test]
    fn swing_delays_only_the_offbeat_sixteenths() {
        let feel = Feel { swing: 2.0 / 3.0, ..Feel::default() };
        let (on, _) = cell_hits(4, 0, Cell::hit(100), feel, 0);
        let (off, _) = cell_hits(5, 0, Cell::hit(100), feel, 0);
        assert!((on[0].beat - 1.0).abs() < 1e-6);
        // A triplet shuffle puts the offbeat sixteenth a twelfth of a beat late.
        assert!((off[0].beat - (1.25 + 1.0 / 12.0)).abs() < 1e-6);
    }

    #[test]
    fn eighth_swing_makes_a_triplet_shuffle() {
        let feel = Feel { swing: 2.0 / 3.0, eighths: true, humanize: 0.0 };
        let (off, _) = cell_hits(2, 0, Cell::hit(100), feel, 0);
        let (next, _) = cell_hits(4, 0, Cell::hit(100), feel, 0);
        assert!((off[0].beat - 2.0 / 3.0).abs() < 1e-6);
        assert!((next[0].beat - 1.0).abs() < 1e-6);
    }

    #[test]
    fn warp_is_monotonic_and_keeps_the_pair_length() {
        for (swing, eighths) in [(0.5, false), (0.62, false), (0.75, true), (0.4, true)] {
            let feel = Feel { swing, eighths, humanize: 0.0 };
            let mut last = -1.0;
            for i in 0..640 {
                let p = i as f64 * 0.05;
                let w = feel.warp(p);
                assert!(w > last);
                last = w;
            }
            let pair = if eighths { 4.0 } else { 2.0 };
            assert!((feel.warp(pair) - pair).abs() < 1e-9);
        }
    }

    #[test]
    fn unwarp_undoes_warp() {
        for (swing, eighths) in [(0.5, false), (0.62, false), (0.75, true), (0.4, true), (0.667, true)] {
            let feel = Feel { swing, eighths, humanize: 0.0 };
            for i in 0..400 {
                let p = i as f64 * 0.137;
                assert!((feel.unwarp(feel.warp(p)) - p).abs() < 1e-9, "swing {swing} eighths {eighths} at {p}");
            }
        }
    }

    #[test]
    fn ratchets_split_the_step_and_fade() {
        let (hits, n) = cell_hits(2, 0, Cell::new(100, 0, 4), Feel::default(), 0);
        assert_eq!(n, 4);
        for j in 1..n {
            assert!((hits[j].beat - hits[j - 1].beat - 0.0625).abs() < 1e-9);
            assert!(hits[j].vel < hits[j - 1].vel);
        }
    }

    #[test]
    fn humanize_is_bounded_and_varies_by_pass() {
        let feel = Feel { humanize: 1.0, ..Feel::default() };
        let mut distinct = std::collections::HashSet::new();
        for pass in 0..50 {
            let (h, _) = cell_hits(8, 1, Cell::hit(100), feel, pass);
            assert!((h[0].beat - 2.0).abs() <= 0.024 + 1e-9);
            assert!((h[0].vel as i32 - 100).abs() <= 18);
            distinct.insert(h[0].beat.to_bits());
        }
        assert!(distinct.len() > 10);
        let (a, _) = cell_hits(8, 1, Cell::hit(100), Feel::default(), 3);
        assert_eq!((a[0].beat, a[0].vel), (2.0, 100));
    }
}
