//! Turns the pattern and the transport into note events, block by block, with no allocation.
//!
//! Every hit has an absolute time in beats, a pure function of its step and lane. A block covers
//! the beats `[b0, b1)`, so a hit belongs to exactly one block as long as the blocks tile, which
//! they do whether the transport is the host's or the plugin's own audition clock.

use crate::pattern::*;
use crate::samples::KitShare;
use nih_plug::prelude::AtomicF32;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

pub const MAX_EVENTS: usize = 512;
const MAX_PENDING: usize = 128;
/// A host position this close to where the last block ended counts as continuing.
const CONTINUITY: f64 = 0.01;

#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Event {
    /// Samples from the start of the block.
    pub timing: u32,
    pub on: bool,
    pub lane: u8,
    pub vel: u8,
}

pub struct EventBuf {
    events: [Event; MAX_EVENTS],
    len: usize,
}

impl Default for EventBuf {
    fn default() -> Self {
        Self { events: [Event::default(); MAX_EVENTS], len: 0 }
    }
}

impl EventBuf {
    pub fn clear(&mut self) {
        self.len = 0;
    }

    pub fn push(&mut self, e: Event) {
        if self.len < MAX_EVENTS {
            self.events[self.len] = e;
            self.len += 1;
        }
    }

    pub fn as_slice(&self) -> &[Event] {
        &self.events[..self.len]
    }

    /// In time order, with note offs ahead of note ons at the same sample.
    pub fn sort(&mut self) {
        self.events[..self.len].sort_unstable_by_key(|e| (e.timing, e.on, e.lane));
    }
}

/// What the audio thread tells the editor: where the playhead is and which drums just hit.
pub struct Meter {
    /// Position in steps within the loop, or negative when nothing is playing.
    pub playhead: AtomicF32,
    pub tempo: AtomicF32,
    /// Counts up on every hit of a lane, so the editor can tell a new one from the last it drew.
    pub hit_seq: [AtomicU32; LANES],
    pub hit_vel: [AtomicF32; LANES],
    /// The editor's play button: run on our own clock while the host is stopped.
    pub audition: AtomicBool,
    /// Sample kits on their way to the audio thread.
    pub samples: Arc<KitShare>,
    /// Notes played in are written into the pattern.
    pub record: AtomicBool,
    /// Bumped by the roll key so each press rolls a different groove.
    pub seed_bump: AtomicU32,
    /// The pattern as it was before the last take, for undo.
    pub undo: std::sync::Mutex<Option<Saved>>,
}

impl Default for Meter {
    fn default() -> Self {
        Self {
            playhead: AtomicF32::new(-1.0),
            tempo: AtomicF32::new(120.0),
            hit_seq: std::array::from_fn(|_| AtomicU32::new(0)),
            hit_vel: std::array::from_fn(|_| AtomicF32::new(0.0)),
            audition: AtomicBool::new(false),
            samples: Arc::new(KitShare::default()),
            record: AtomicBool::new(false),
            seed_bump: AtomicU32::new(0),
            undo: std::sync::Mutex::new(None),
        }
    }
}

/// Where the transport is for one block.
#[derive(Clone, Copy, Debug)]
pub struct Transport {
    /// The host is rolling, and at what beat.
    pub host: Option<f64>,
    /// Run on our own clock while the host is stopped.
    pub audition: bool,
    pub tempo: f64,
}

#[derive(Clone, Copy, Default)]
struct Off {
    lane: u8,
    /// Absolute sample the note ends at, and the sample it started at.
    at: i64,
    born: i64,
}

pub struct Sequencer {
    pending: [Off; MAX_PENDING],
    n_pending: usize,
    /// Samples rendered so far, which pending notes are timed against.
    clock: i64,
    /// Where the last block ended, to tell a continuing transport from a jump.
    expect: Option<f64>,
    internal_beat: f64,
    /// Where the current block started in beats, and how long a sample is, so a note arriving
    /// part-way through it can be placed on the grid. `None` while nothing is playing.
    block: Option<(f64, f64)>,
    pub meter: Arc<Meter>,
}

impl Default for Sequencer {
    fn default() -> Self {
        Self { pending: [Off::default(); MAX_PENDING], n_pending: 0, clock: 0, expect: None, internal_beat: 0.0, block: None, meter: Arc::new(Meter::default()) }
    }
}

impl Sequencer {
    pub fn reset(&mut self) {
        self.n_pending = 0;
        self.expect = None;
        self.internal_beat = 0.0;
        self.block = None;
        self.meter.playhead.store(-1.0, Ordering::Relaxed);
    }

    /// Write a note played during this block into the pattern, on the nearest step with the
    /// remainder kept as a timing nudge. Returns the step it landed on, or `None` if nothing is
    /// playing, so a note played while stopped is not recorded anywhere.
    pub fn record(&self, store: &PatternStore, feel: Feel, lane: usize, vel: u8, timing: u32) -> Option<usize> {
        let (b0, bps) = self.block?;
        let steps = store.steps();
        let pos = feel.unwarp((b0 + timing as f64 * bps) / STEP_BEATS).rem_euclid(steps as f64);
        let nearest = pos.round();
        let micro = ((pos - nearest) * 100.0).round() as i32;
        let micro = if micro.abs() < 4 { 0 } else { micro.clamp(-50, 50) } as i8;
        let step = nearest as usize % steps;
        store.set(lane, step, Cell::new(vel.max(1), micro, 1));
        self.meter.hit_vel[lane].store(vel as f32 / 127.0, Ordering::Relaxed);
        self.meter.hit_seq[lane].fetch_add(1, Ordering::Relaxed);
        Some(step)
    }

    fn flush(&mut self, out: &mut EventBuf) {
        for p in &self.pending[..self.n_pending] {
            out.push(Event { timing: 0, on: false, lane: p.lane, vel: 0 });
        }
        self.n_pending = 0;
    }

    /// Fill `out` with this block's events, in order.
    pub fn process(&mut self, store: &PatternStore, feel: Feel, transport: Transport, sample_rate: f64, len: usize, out: &mut EventBuf) {
        out.clear();
        let tempo = transport.tempo.clamp(20.0, 400.0);
        self.meter.tempo.store(tempo as f32, Ordering::Relaxed);

        let start = match (transport.host, transport.audition) {
            (Some(beat), _) => beat,
            (None, true) => self.internal_beat,
            (None, false) => {
                if self.expect.is_some() || self.n_pending > 0 {
                    self.flush(out);
                }
                self.reset();
                self.clock += len as i64;
                return;
            }
        };
        let host_driven = transport.host.is_some();
        let b0 = match self.expect {
            Some(e) if (e - start).abs() < CONTINUITY => e,
            Some(_) => {
                // The transport jumped, so whatever was ringing is cut.
                self.flush(out);
                start
            }
            None => start,
        };
        let beats_per_sample = tempo / 60.0 / sample_rate;
        let b1 = b0 + len as f64 * beats_per_sample;
        self.block = Some((b0, beats_per_sample));
        self.expect = Some(b1);
        if !host_driven {
            self.internal_beat = b1;
        } else if transport.audition {
            // Next time the host stops, the audition clock starts from the top.
            self.internal_beat = 0.0;
        }

        let steps = store.steps();
        let loop_beats = steps as f64 * STEP_BEATS;
        self.meter.playhead.store((b0.rem_euclid(loop_beats) / STEP_BEATS) as f32, Ordering::Relaxed);

        let first_on = out.as_slice().len();
        let n0 = ((b0 / STEP_BEATS).floor() as i64 - 3).max(0);
        let n1 = (b1 / STEP_BEATS).floor() as i64 + 2;
        for n in n0..=n1 {
            let step = (n % steps as i64) as usize;
            let pass = (n / steps as i64) as u32;
            let loop_start = (n / steps as i64) as f64 * loop_beats;
            for lane in 0..LANES {
                let cell = store.get(lane, step);
                if !cell.is_on() || !store.audible(lane) {
                    continue;
                }
                let (hits, count) = cell_hits(step, lane, cell, feel, pass);
                for hit in &hits[..count] {
                    // Decide the block by the sample the hit rounds to, so that a hit on a block
                    // boundary goes to exactly one side however the beats accumulate.
                    let abs = loop_start + hit.beat;
                    let timing = ((abs - b0) / beats_per_sample).round() as i64;
                    if timing < 0 || timing >= len as i64 {
                        continue;
                    }
                    out.push(Event { timing: timing as u32, on: true, lane: lane as u8, vel: hit.vel });
                    let length = ((hit.len / beats_per_sample).round() as i64).max(1);
                    let off = Off { lane: lane as u8, at: self.clock + timing + length, born: self.clock + timing };
                    if self.n_pending < MAX_PENDING {
                        self.pending[self.n_pending] = off;
                        self.n_pending += 1;
                    }
                }
            }
        }

        // A note that is still held when the same drum hits again ends just before the new hit.
        for e in &out.as_slice()[first_on..] {
            let t = self.clock + e.timing as i64;
            for p in &mut self.pending[..self.n_pending] {
                if p.lane == e.lane && p.born < t && p.at > t {
                    p.at = t;
                }
            }
        }

        // Notes ending in this block.
        let end = self.clock + len as i64;
        let mut i = 0;
        while i < self.n_pending {
            let p = self.pending[i];
            if p.at < end {
                out.push(Event { timing: (p.at - self.clock).max(0) as u32, on: false, lane: p.lane, vel: 0 });
                self.pending[i] = self.pending[self.n_pending - 1];
                self.n_pending -= 1;
            } else {
                i += 1;
            }
        }
        self.clock = end;
        out.sort();

        for e in out.as_slice() {
            if e.on {
                let lane = e.lane as usize;
                self.meter.hit_vel[lane].store(e.vel as f32 / 127.0, Ordering::Relaxed);
                self.meter.hit_seq[lane].fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FS: f64 = 48000.0;

    fn host(beat: f64, tempo: f64) -> Transport {
        Transport { host: Some(beat), audition: false, tempo }
    }

    fn backbeat() -> PatternStore {
        let s = PatternStore::default();
        s.set(LANE_KICK, 0, Cell::hit(120));
        s.set(LANE_KICK, 8, Cell::hit(110));
        s.set(LANE_SNARE, 4, Cell::hit(100));
        s.set(LANE_SNARE, 12, Cell::hit(100));
        s
    }

    /// Run `blocks` blocks of `len` samples from `beat`, collecting `(absolute sample, event)`.
    fn run(store: &PatternStore, feel: Feel, beat: f64, tempo: f64, len: usize, blocks: usize) -> Vec<(i64, Event)> {
        let mut seq = Sequencer::default();
        let mut buf = EventBuf::default();
        let mut all = Vec::new();
        let bps = tempo / 60.0 / FS;
        for b in 0..blocks {
            let pos = beat + (b * len) as f64 * bps;
            seq.process(store, feel, host(pos, tempo), FS, len, &mut buf);
            for e in buf.as_slice() {
                all.push(((b * len) as i64 + e.timing as i64, *e));
            }
        }
        all
    }

    fn ons(events: &[(i64, Event)]) -> Vec<(i64, u8)> {
        events.iter().filter(|(_, e)| e.on).map(|(t, e)| (*t, e.lane)).collect()
    }

    #[test]
    fn hits_land_on_the_exact_sample() {
        // 120 bpm: a beat is 24000 samples.
        let events = run(&backbeat(), Feel::default(), 0.0, 120.0, 480, 400);
        let got = ons(&events);
        assert_eq!(&got[..4], &[(0, 0), (24000, 1), (48000, 0), (72000, 1)]);
    }

    #[test]
    fn block_size_does_not_change_what_plays() {
        let reference = ons(&run(&backbeat(), Feel { humanize: 0.5, swing: 0.6, eighths: false }, 0.0, 133.0, 64, 3000));
        for len in [32usize, 100, 480, 1024, 2047] {
            let blocks = 64 * 3000 / len;
            let got: Vec<_> = ons(&run(&backbeat(), Feel { humanize: 0.5, swing: 0.6, eighths: false }, 0.0, 133.0, len, blocks))
                .into_iter()
                .filter(|(t, _)| *t < 64 * 3000)
                .collect();
            let want: Vec<_> = reference.iter().copied().filter(|(t, _)| *t < (len * blocks) as i64).collect();
            assert_eq!(got.len(), want.len(), "block {len}");
            for (a, b) in got.iter().zip(&want) {
                assert_eq!(a.1, b.1);
                assert!((a.0 - b.0).abs() <= 1, "block {len}: {a:?} vs {b:?}");
            }
        }
    }

    #[test]
    fn the_pattern_loops_every_bar() {
        let events = run(&backbeat(), Feel::default(), 0.0, 120.0, 512, 1000);
        let kicks: Vec<i64> = ons(&events).into_iter().filter(|e| e.1 == 0).map(|e| e.0).collect();
        // Two kicks per bar of 96000 samples.
        assert_eq!(&kicks[..4], &[0, 48000, 96000, 144000]);
    }

    #[test]
    fn every_note_on_gets_exactly_one_off() {
        let store = PatternStore::default();
        for s in 0..16 {
            store.set(LANE_HAT, s, Cell::new(90, if s % 3 == 0 { 40 } else { -40 }, 1 + (s % 4) as u8));
        }
        let events = run(&store, Feel { swing: 0.66, humanize: 1.0, eighths: false }, 0.0, 160.0, 256, 2000);
        let mut held = 0i32;
        for (_, e) in &events {
            held += if e.on { 1 } else { -1 };
            assert!((0..=1).contains(&held), "same drum must not stack: {held}");
        }
        assert!(events.iter().filter(|(_, e)| e.on).count() > 100);
    }

    #[test]
    fn a_jump_cuts_held_notes_and_restarts() {
        let store = backbeat();
        let mut seq = Sequencer::default();
        let mut buf = EventBuf::default();
        // Start just before the snare so its off is pending when the transport jumps.
        seq.process(&store, Feel::default(), host(0.99, 120.0), FS, 1024, &mut buf);
        assert!(buf.as_slice().iter().any(|e| e.on && e.lane == 1));
        seq.process(&store, Feel::default(), host(3.0, 120.0), FS, 64, &mut buf);
        assert!(buf.as_slice().iter().any(|e| !e.on && e.lane == 1 && e.timing == 0), "the held snare is released");
    }

    #[test]
    fn stopping_releases_everything() {
        let store = backbeat();
        let mut seq = Sequencer::default();
        let mut buf = EventBuf::default();
        seq.process(&store, Feel::default(), host(0.99, 120.0), FS, 1024, &mut buf);
        let held = buf.as_slice().iter().filter(|e| e.on).count() as i32 - buf.as_slice().iter().filter(|e| !e.on).count() as i32;
        assert_eq!(held, 1);
        let stopped = Transport { host: None, audition: false, tempo: 120.0 };
        seq.process(&store, Feel::default(), stopped, FS, 512, &mut buf);
        assert_eq!(buf.as_slice().iter().filter(|e| !e.on).count(), 1);
        seq.process(&store, Feel::default(), stopped, FS, 512, &mut buf);
        assert!(buf.as_slice().is_empty());
        assert!(seq.meter.playhead.load(Ordering::Relaxed) < 0.0);
    }

    #[test]
    fn the_audition_clock_runs_when_the_host_is_stopped() {
        let store = backbeat();
        let mut seq = Sequencer::default();
        let mut buf = EventBuf::default();
        let t = Transport { host: None, audition: true, tempo: 120.0 };
        let mut seen = Vec::new();
        for b in 0..200 {
            seq.process(&store, Feel::default(), t, FS, 480, &mut buf);
            for e in buf.as_slice().iter().filter(|e| e.on) {
                seen.push(b as i64 * 480 + e.timing as i64);
            }
        }
        assert_eq!(&seen[..3], &[0, 24000, 48000]);
        assert!(seq.meter.playhead.load(Ordering::Relaxed) >= 0.0);
    }

    #[test]
    fn muted_lanes_are_silent_and_solo_isolates() {
        let store = backbeat();
        store.toggle_mute(LANE_KICK);
        assert!(run(&store, Feel::default(), 0.0, 120.0, 512, 400).iter().all(|(_, e)| e.lane != 0));
        store.toggle_mute(LANE_KICK);
        store.toggle_solo(LANE_SNARE);
        assert!(run(&store, Feel::default(), 0.0, 120.0, 512, 400).iter().all(|(_, e)| e.lane == 1));
    }

    #[test]
    fn a_recorded_note_lands_where_it_would_play_back() {
        let store = PatternStore::default();
        let mut seq = Sequencer::default();
        let mut buf = EventBuf::default();
        // 120 bpm, a 512-sample block starting a hair before beat 1 (step 4).
        let start = 1.0 - 100.0 * 120.0 / 60.0 / FS;
        seq.process(&store, Feel::default(), host(start, 120.0), FS, 512, &mut buf);
        // A note at sample 100 is exactly on beat 1.
        assert_eq!(seq.record(&store, Feel::default(), LANE_SNARE, 99, 100), Some(4));
        assert_eq!(store.get(LANE_SNARE, 4).vel(), 99);
        assert_eq!(store.get(LANE_SNARE, 4).micro(), 0);
        // Played late by a third of a step (0.0833 beat = 2000 samples at 120 bpm) it keeps the nudge.
        let third = (0.25 / 3.0 * 24000.0) as u32;
        seq.record(&store, Feel::default(), LANE_KICK, 80, 100 + third);
        let c = store.get(LANE_KICK, 4);
        assert!((c.micro() as i32 - 33).abs() <= 1, "{}", c.micro());
        // More than half a step late is the next step, early by the same amount.
        seq.record(&store, Feel::default(), LANE_HAT, 80, 100 + 3500);
        assert_eq!(store.get(LANE_HAT, 5).vel(), 80);
        assert!(store.get(LANE_HAT, 5).micro() < 0);
        // And it plays back where it was played.
        let again = run(&store, Feel::default(), 0.0, 120.0, 480, 100);
        let snare = ons(&again).into_iter().find(|e| e.1 == LANE_SNARE as u8).unwrap();
        assert!((snare.0 - 24000).abs() <= 1);
    }

    #[test]
    fn recording_follows_the_swing() {
        let store = PatternStore::default();
        let mut seq = Sequencer::default();
        let mut buf = EventBuf::default();
        let feel = Feel { swing: 2.0 / 3.0, eighths: false, humanize: 0.0 };
        // Beat 1.3333 is where a swung step 5 sounds; playing there must write step 5, no nudge.
        let start = 1.3333333 - 100.0 * 120.0 / 60.0 / FS;
        seq.process(&store, feel, host(start, 120.0), FS, 512, &mut buf);
        assert_eq!(seq.record(&store, feel, LANE_HAT, 90, 100), Some(5));
        assert_eq!(store.get(LANE_HAT, 5).micro(), 0);
    }

    #[test]
    fn nothing_is_recorded_while_stopped_and_it_wraps_at_the_loop_end() {
        let store = PatternStore::default();
        let mut seq = Sequencer::default();
        let mut buf = EventBuf::default();
        assert_eq!(seq.record(&store, Feel::default(), LANE_KICK, 100, 0), None);
        // Just before the end of the bar, a note rounds forward onto step 0.
        let start = 3.99 - 100.0 * 120.0 / 60.0 / FS;
        seq.process(&store, Feel::default(), host(start, 120.0), FS, 512, &mut buf);
        assert_eq!(seq.record(&store, Feel::default(), LANE_KICK, 100, 100), Some(0));
        seq.process(&store, Feel::default(), Transport { host: None, audition: false, tempo: 120.0 }, FS, 512, &mut buf);
        assert_eq!(seq.record(&store, Feel::default(), LANE_KICK, 100, 0), None);
    }

    #[test]
    fn the_meter_reports_hits_and_the_playhead() {
        let store = backbeat();
        let mut seq = Sequencer::default();
        let mut buf = EventBuf::default();
        seq.process(&store, Feel::default(), host(1.0, 120.0), FS, 128, &mut buf);
        assert_eq!(seq.meter.hit_seq[LANE_SNARE].load(Ordering::Relaxed), 1);
        assert!((seq.meter.hit_vel[LANE_SNARE].load(Ordering::Relaxed) - 100.0 / 127.0).abs() < 1e-6);
        assert!((seq.meter.playhead.load(Ordering::Relaxed) - 4.0).abs() < 1e-3);
        assert_eq!(seq.meter.hit_seq[LANE_KICK].load(Ordering::Relaxed), 0);
    }

    #[test]
    fn a_negative_position_before_the_start_is_safe() {
        let events = run(&backbeat(), Feel::default(), -2.0, 120.0, 512, 200);
        assert!(ons(&events).iter().all(|(t, _)| *t >= 48000 - 512));
    }

    #[test]
    fn a_dense_pattern_does_not_overflow_the_buffers() {
        let store = PatternStore::default();
        for lane in 0..LANES {
            for s in 0..16 {
                store.set(lane, s, Cell::new(100, 0, 4));
            }
        }
        let events = run(&store, Feel::default(), 0.0, 200.0, 8192, 20);
        assert!(!events.is_empty());
    }
}
