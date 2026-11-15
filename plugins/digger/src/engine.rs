//! Sample playback: a pool of voices reading from the current library. A voice remembers which
//! sample it plays as a (category, index) pair rather than holding a reference, so nothing is
//! cloned or dropped on the audio thread. A new library stops every voice first.

use crate::library::Library;

pub const SLOTS: usize = 8;
const VOICES: usize = 24;

#[derive(Clone, Copy, Default)]
struct Voice {
    on: bool,
    slot: u8,
    cat: u16,
    idx: u16,
    pos: f64,
    rate: f64,
    gl: f32,
    gr: f32,
    /// 1 while playing normally; falls to 0 when the voice is choked.
    fade: f32,
    choked: bool,
    age: u32,
}

pub struct Engine {
    voices: [Voice; VOICES],
    clock: u32,
    choke_step: f32,
}

impl Default for Engine {
    fn default() -> Self {
        Self { voices: [Voice::default(); VOICES], clock: 0, choke_step: 1.0 / (0.006 * 44100.0) }
    }
}

impl Engine {
    pub fn set_sample_rate(&mut self, sr: f32) {
        self.choke_step = 1.0 / (0.006 * sr).max(1.0);
    }

    pub fn stop_all(&mut self) {
        for v in self.voices.iter_mut() {
            v.on = false;
        }
    }

    pub fn active(&self) -> usize {
        self.voices.iter().filter(|v| v.on).count()
    }

    /// Start a hit. `tune` is in semitones, `gain` linear, `pan` -1 (left) to 1 (right).
    #[allow(clippy::too_many_arguments)]
    pub fn trigger(&mut self, slot: usize, cat: usize, idx: usize, velocity: f32, tune: f32, gain: f32, pan: f32) {
        self.clock = self.clock.wrapping_add(1);
        // A free voice, else steal the oldest (the largest wrapping distance from now).
        let at = self.voices.iter().position(|v| !v.on).unwrap_or_else(|| {
            self.voices.iter().enumerate().max_by_key(|(_, v)| self.clock.wrapping_sub(v.age)).map(|(i, _)| i).unwrap_or(0)
        });
        let vel = velocity.clamp(0.0, 1.0);
        let pan = pan.clamp(-1.0, 1.0);
        self.voices[at] = Voice {
            on: true,
            slot: slot as u8,
            cat: cat as u16,
            idx: idx as u16,
            pos: 0.0,
            rate: 2f64.powf(tune as f64 / 12.0),
            gl: gain * vel * if pan > 0.0 { 1.0 - pan } else { 1.0 },
            gr: gain * vel * if pan < 0.0 { 1.0 + pan } else { 1.0 },
            fade: 1.0,
            choked: false,
            age: self.clock,
        };
    }

    /// Fade out everything the slot is playing, in a few milliseconds.
    pub fn choke(&mut self, slot: usize) {
        for v in self.voices.iter_mut().filter(|v| v.on && v.slot as usize == slot) {
            v.choked = true;
        }
    }

    pub fn next(&mut self, lib: &Library) -> (f32, f32) {
        let (mut l, mut r) = (0.0, 0.0);
        for v in self.voices.iter_mut().filter(|v| v.on) {
            let Some(s) = lib.sample(v.cat as usize, v.idx as usize) else {
                v.on = false;
                continue;
            };
            let i = v.pos as usize;
            if i >= s.len() {
                v.on = false;
                continue;
            }
            let frac = (v.pos - i as f64) as f32;
            let j = (i + 1).min(s.len() - 1);
            let a = s.left[i] + (s.left[j] - s.left[i]) * frac;
            let b = match s.right.as_deref() {
                Some(rr) => rr[i] + (rr[j] - rr[i]) * frac,
                None => a,
            };
            if v.choked {
                v.fade -= self.choke_step;
                if v.fade <= 0.0 {
                    v.on = false;
                    continue;
                }
            }
            l += a * v.gl * v.fade;
            r += b * v.gr * v.fade;
            v.pos += v.rate;
        }
        (l, r)
    }

    /// How far through its sample the newest voice in `slot` is, 0 to 1, or -1 when silent.
    pub fn playhead(&self, lib: &Library, slot: usize) -> f32 {
        let newest = self.voices.iter().filter(|v| v.on && v.slot as usize == slot).max_by_key(|v| v.age);
        match newest.and_then(|v| lib.sample(v.cat as usize, v.idx as usize).map(|s| (v, s.len()))) {
            Some((v, len)) if len > 0 => (v.pos / len as f64).clamp(0.0, 1.0) as f32,
            _ => -1.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::{Category, Sample};

    fn lib_of(samples: Vec<Vec<f32>>) -> Library {
        let samples = samples.into_iter().enumerate().map(|(i, left)| Sample::plain(format!("s{i}"), left, None, vec![])).collect();
        Library::new(String::new(), 44100, vec![Category { name: "kick".into(), samples }])
    }

    fn run(e: &mut Engine, lib: &Library, n: usize) -> Vec<(f32, f32)> {
        (0..n).map(|_| e.next(lib)).collect()
    }

    #[test]
    fn a_hit_plays_its_sample_once_and_stops() {
        let lib = lib_of(vec![vec![0.5; 100]]);
        let mut e = Engine::default();
        e.trigger(0, 0, 0, 1.0, 0.0, 1.0, 0.0);
        let out = run(&mut e, &lib, 150);
        assert!(out[..100].iter().all(|&(l, r)| (l - 0.5).abs() < 1e-6 && (r - 0.5).abs() < 1e-6));
        assert!(out[100..].iter().all(|&(l, _)| l == 0.0));
        assert_eq!(e.active(), 0);
    }

    #[test]
    fn velocity_scales_and_pan_moves_the_hit() {
        let lib = lib_of(vec![vec![1.0; 50]]);
        let mut e = Engine::default();
        e.trigger(0, 0, 0, 0.5, 0.0, 1.0, 0.0);
        assert!((e.next(&lib).0 - 0.5).abs() < 1e-6);
        e.stop_all();
        e.trigger(0, 0, 0, 1.0, 0.0, 1.0, 1.0);
        let (l, r) = e.next(&lib);
        assert!(l.abs() < 1e-6 && (r - 1.0).abs() < 1e-6, "hard right silences the left");
    }

    #[test]
    fn tuning_up_an_octave_halves_the_length() {
        let lib = lib_of(vec![vec![0.3; 1000]]);
        let (mut a, mut b) = (Engine::default(), Engine::default());
        a.trigger(0, 0, 0, 1.0, 0.0, 1.0, 0.0);
        b.trigger(0, 0, 0, 1.0, 12.0, 1.0, 0.0);
        let len = |e: &mut Engine| run(e, &lib, 1200).iter().filter(|(l, _)| *l != 0.0).count();
        let (la, lb) = (len(&mut a), len(&mut b));
        assert_eq!(la, 1000);
        assert!((lb as i32 - 500).abs() <= 1, "{lb}");
    }

    #[test]
    fn a_choke_fades_only_that_slot_out_within_a_few_milliseconds() {
        let lib = lib_of(vec![vec![0.5; 20000]]);
        let mut e = Engine::default();
        e.set_sample_rate(44100.0);
        e.trigger(4, 0, 0, 1.0, 0.0, 1.0, 0.0);
        e.trigger(1, 0, 0, 1.0, 0.0, 1.0, 0.0);
        run(&mut e, &lib, 10);
        e.choke(4);
        let out = run(&mut e, &lib, 400);
        // Falls smoothly rather than cutting off, and the other slot carries on at 0.5.
        assert!(out[0].0 < 1.0 && out[0].0 > 0.5);
        assert!((out[399].0 - 0.5).abs() < 1e-5);
        assert_eq!(e.active(), 1);
    }

    #[test]
    fn running_out_of_voices_steals_the_oldest_and_never_overflows() {
        let lib = lib_of(vec![vec![0.01; 100000]]);
        let mut e = Engine::default();
        for k in 0..VOICES * 3 {
            e.trigger(k % SLOTS, 0, 0, 1.0, 0.0, 1.0, 0.0);
            e.next(&lib);
        }
        assert_eq!(e.active(), VOICES);
    }

    #[test]
    fn a_voice_whose_sample_vanished_goes_quiet_and_the_playhead_tracks_progress() {
        let lib = lib_of(vec![vec![0.2; 1000]]);
        let mut e = Engine::default();
        assert_eq!(e.playhead(&lib, 2), -1.0);
        e.trigger(2, 0, 0, 1.0, 0.0, 1.0, 0.0);
        run(&mut e, &lib, 500);
        assert!((e.playhead(&lib, 2) - 0.5).abs() < 0.01);
        let empty = Library::default();
        assert_eq!(e.next(&empty), (0.0, 0.0));
        assert_eq!(e.active(), 0);
    }
}
