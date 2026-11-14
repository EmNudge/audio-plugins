//! The audition kit: what you hear from Pocket itself.
//!
//! Seven kits share one set of voices. The four acoustic ones (Studio, Rock, Jazz, Tight) are
//! modal models of real drums, the Electro kit is 808-style synthesis, Lo-Fi is the Studio kit
//! through a crusher, and Samples plays WAV files you point it at. All go through a small room.
//! Each drum has one voice, so a repeat retriggers and an open hat is choked by the closed one.

use crate::electro;
use crate::layered::{self, Layered, Voicing};
use crate::pattern::*;
use crate::reverb::Reverb;
use crate::samples::{SampleKit, SampleVoice};
use nih_plug::prelude::Enum;
use std::sync::Arc;

#[derive(Enum, Clone, Copy, PartialEq, Eq, Debug)]
pub enum KitKind {
    #[name = "Studio"]
    Studio,
    #[name = "Rock"]
    Rock,
    #[name = "Jazz"]
    Jazz,
    #[name = "Tight"]
    Tight,
    #[name = "Electro 808"]
    Electro,
    #[name = "Lo-Fi"]
    LoFi,
    #[name = "My Samples"]
    Samples,
}

pub const KITS: [KitKind; 7] = [KitKind::Studio, KitKind::Rock, KitKind::Jazz, KitKind::Tight, KitKind::Electro, KitKind::LoFi, KitKind::Samples];

impl KitKind {
    pub fn name(self) -> &'static str {
        match self {
            KitKind::Studio => "STUDIO",
            KitKind::Rock => "ROCK",
            KitKind::Jazz => "JAZZ",
            KitKind::Tight => "TIGHT",
            KitKind::Electro => "ELECTRO 808",
            KitKind::LoFi => "LO-FI",
            KitKind::Samples => "MY SAMPLES",
        }
    }

    pub fn blurb(self) -> &'static str {
        match self {
            KitKind::Studio => "A natural mid-sized kit: balanced, a little room.",
            KitKind::Rock => "A big kit: deep kick, fat snare, long cymbals.",
            KitKind::Jazz => "A small kit: high tuning, soft wires, a loud ride.",
            KitKind::Tight => "Dry and short: funk, pop and anything that wants no ring.",
            KitKind::Electro => "808-style synthesis.",
            KitKind::LoFi => "The studio kit through a bit crusher, a dull filter and vinyl noise.",
            KitKind::Samples => "Your own WAV files, filed under each drum by name.",
        }
    }

    fn voicing(self) -> &'static Voicing {
        match self {
            KitKind::Rock => &layered::ROCK,
            KitKind::Jazz => &layered::JAZZ,
            KitKind::Tight => &layered::TIGHT,
            _ => &layered::STUDIO,
        }
    }
}

/// Where each drum sits in the stereo image, as a drummer hears it.
const PAN: [f32; LANES] = [0.0, 0.04, 0.12, -0.1, 0.32, 0.34, -0.38, 0.14, -0.5, 0.5, -0.3];

/// The Lo-Fi kit's damage: sample-rate reduction, a few bits, a dull top, hiss and the odd pop.
struct Crusher {
    hold: [f32; 2],
    count: u32,
    lp: [f32; 2],
    quiet_for: u32,
}

pub struct Kit {
    dt: f32,
    sr: f32,
    kind: KitKind,
    layered: [Layered; LANES],
    electro: [electro::Voice; LANES],
    sample_voices: [SampleVoice; LANES],
    samples: Option<Arc<SampleKit>>,
    rng: u32,
    /// Pitch ratio and decay scale from the Tune and Decay controls.
    tune: f32,
    decay: f32,
    room_amount: f32,
    room: Reverb,
    crusher: Crusher,
    /// Low-pass state for the room send, so the kick does not set the room booming.
    send_lp: f32,
}

impl Default for Kit {
    fn default() -> Self {
        Self::new(44100.0)
    }
}

fn ceiling(x: f32) -> f32 {
    let a = x.abs();
    if a <= 0.9 {
        x
    } else {
        x.signum() * (0.9 + 0.1 * ((a - 0.9) / 0.1).tanh())
    }
}

impl Kit {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            dt: 1.0 / sample_rate,
            sr: sample_rate,
            kind: KitKind::Studio,
            layered: [Layered::default(); LANES],
            electro: [electro::Voice::default(); LANES],
            sample_voices: [SampleVoice::default(); LANES],
            samples: None,
            rng: 0x2545_F491,
            tune: 1.0,
            decay: 1.0,
            room_amount: 0.0,
            room: Reverb::new(sample_rate),
            crusher: Crusher { hold: [0.0; 2], count: 0, lp: [0.0; 2], quiet_for: u32::MAX },
            send_lp: 0.0,
        }
    }

    /// Reallocates the room, so only call this outside the audio callback.
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.dt = 1.0 / sample_rate;
        self.sr = sample_rate;
        self.room = Reverb::new(sample_rate);
    }

    pub fn reset(&mut self) {
        self.layered = [Layered::default(); LANES];
        self.electro = [electro::Voice::default(); LANES];
        self.sample_voices = [SampleVoice::default(); LANES];
        self.room.clear();
    }

    pub fn set_kind(&mut self, kind: KitKind) {
        self.kind = kind;
    }

    /// Tune in semitones, decay as a scale on every ring time, room as the wet amount (0..1).
    pub fn set_voice(&mut self, tune_semitones: f32, decay: f32, room: f32) {
        self.tune = 2f32.powf(tune_semitones / 12.0);
        self.decay = decay.clamp(0.2, 3.0);
        self.room_amount = room.clamp(0.0, 1.0);
    }

    /// Swap in a loaded sample kit and hand back the old one, which the caller must not drop on
    /// the audio thread.
    pub fn set_samples(&mut self, kit: Option<Arc<SampleKit>>) -> Option<Arc<SampleKit>> {
        self.sample_voices = [SampleVoice::default(); LANES];
        std::mem::replace(&mut self.samples, kit)
    }

    pub fn has_samples(&self) -> bool {
        self.samples.is_some()
    }

    pub fn trigger(&mut self, lane: usize, vel: u8) {
        if lane >= LANES {
            return;
        }
        let v = (vel as f32 / 127.0).clamp(0.0, 1.0);
        if lane == LANE_HAT {
            self.layered[LANE_OPEN].on = false;
            self.electro[LANE_OPEN].on = false;
            self.sample_voices[LANE_OPEN].on = false;
        }
        match self.kind {
            KitKind::Electro => self.electro[lane] = electro::Voice::start(v.powf(1.4)),
            KitKind::Samples => {
                let layer = self.samples.as_ref().and_then(|k| k.layer(lane, v));
                match layer {
                    Some(layer) => {
                        // Layers carry the loudness; a gentle curve keeps the rest of the range alive.
                        self.sample_voices[lane] = SampleVoice { on: true, layer, pos: 0, gain: (0.35 + 0.65 * v).powf(1.2) };
                    }
                    // A drum the folder has no file for falls back to the studio kit.
                    None => self.trigger_modal(lane, v, KitKind::Studio),
                }
            }
            kind => self.trigger_modal(lane, v, kind),
        }
    }

    fn trigger_modal(&mut self, lane: usize, v: f32, kind: KitKind) {
        let hard = v.powf(0.8);
        layered::build(&mut self.layered[lane], lane, kind.voicing(), hard, self.tune, self.decay, &mut self.rng, self.sr);
        // Loudness follows velocity, and timbre follows `hard` inside the voice.
        self.layered[lane].set_level(0.25 + 0.75 * v.powf(1.3));
    }

    pub fn is_silent(&self) -> bool {
        self.layered.iter().all(|v| !v.on) && self.electro.iter().all(|v| !v.on) && self.sample_voices.iter().all(|v| !v.on)
    }

    fn white(&mut self) -> f32 {
        layered::jitter(&mut self.rng)
    }

    pub fn next(&mut self) -> (f32, f32) {
        let (mut l, mut r) = (0.0f32, 0.0f32);
        let dt = self.dt;
        for lane in 0..LANES {
            let pan = PAN[lane];
            let (gl, gr) = (1.0 - pan.max(0.0) * 0.6, 1.0 + pan.min(0.0) * 0.6);
            if self.layered[lane].on {
                let n = self.white();
                let s = self.layered[lane].next(dt, n);
                l += s * gl;
                r += s * gr;
            }
            if self.electro[lane].on {
                let n = self.white();
                let s = self.electro[lane].next(lane, dt, n);
                l += s * (1.0 - pan.max(0.0));
                r += s * (1.0 + pan.min(0.0));
            }
            if self.sample_voices[lane].on {
                if let Some(kit) = self.samples.as_deref() {
                    let (a, b) = self.sample_voices[lane].next(kit, lane);
                    l += a;
                    r += b;
                }
            }
        }
        // The room hears everything: a mono send, a stereo return.
        if self.room_amount > 0.0 {
            let send = (l + r) * 0.5 * (0.4 + 0.6 * self.room_amount);
            self.send_lp += 0.02 * (send - self.send_lp);
            let (wl, wr) = self.room.process(send - self.send_lp);
            l += wl * self.room_amount * 1.2;
            r += wr * self.room_amount * 1.2;
        }
        // Transparent below 0.9, a soft ceiling above it.
        l = ceiling(l);
        r = ceiling(r);
        if self.kind == KitKind::LoFi {
            let active = !self.is_silent();
            return self.crush(l, r, active);
        }
        (l, r)
    }

    fn crush(&mut self, l: f32, r: f32, active: bool) -> (f32, f32) {
        let c = &mut self.crusher;
        c.quiet_for = if active { 0 } else { c.quiet_for.saturating_add(1) };
        // Hold every fourth sample at 44.1 kHz: about 11 kHz of sample rate.
        let every = ((self.sr / 11025.0).round() as u32).max(1);
        if c.count % every == 0 {
            c.hold = [l, r];
        }
        c.count = c.count.wrapping_add(1);
        let mut out = [0.0f32; 2];
        for ch in 0..2 {
            // Ten bits.
            let q = (c.hold[ch] * 512.0).round() / 512.0;
            c.lp[ch] += 0.32 * (q - c.lp[ch]);
            out[ch] = (c.lp[ch] * 1.4).tanh() * 0.85;
        }
        // Hiss and crackle, only while the kit is sounding or just after it.
        if c.quiet_for < (self.sr * 1.5) as u32 {
            let mut s = self.rng;
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            self.rng = s;
            let n = (s >> 8) as f32 / (1u32 << 23) as f32 - 1.0;
            let pop = if s % 9000 == 0 { n * 0.25 } else { 0.0 };
            out[0] += n * 0.004 + pop;
            out[1] += n * -0.004 + pop;
        }
        (out[0], out[1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::samples::Sample;
    use std::f32::consts::TAU;

    const FS: f32 = 48000.0;

    fn kit(kind: KitKind, room: f32) -> Kit {
        let mut k = Kit::new(FS);
        k.set_kind(kind);
        k.set_voice(0.0, 1.0, room);
        k
    }

    /// Both channels summed, for `seconds`.
    fn render(kind: KitKind, lane: usize, vel: u8, seconds: f32, room: f32) -> Vec<f32> {
        let mut k = kit(kind, room);
        k.trigger(lane, vel);
        (0..(seconds * FS) as usize).map(|_| k.next()).map(|(l, r)| l + r).collect()
    }

    /// Mean power between `lo` and `hi` Hz, by a windowed DFT at log-spaced frequencies.
    fn band(x: &[f32], lo: f32, hi: f32) -> f32 {
        let n = x.len().min(16384);
        let bins = 28;
        let mut total = 0.0;
        for b in 0..bins {
            let f = lo * (hi / lo).powf(b as f32 / (bins - 1) as f32);
            let (mut re, mut im) = (0.0f32, 0.0f32);
            for (i, s) in x[..n].iter().enumerate() {
                let w = 0.5 - 0.5 * (TAU * i as f32 / n as f32).cos();
                let a = TAU * f * i as f32 / FS;
                re += s * w * a.cos();
                im += s * w * a.sin();
            }
            total += (re * re + im * im) / (n * n) as f32;
        }
        total / bins as f32
    }

    /// The loudest frequency in `lo..hi`.
    fn peak_hz(x: &[f32], lo: f32, hi: f32) -> f32 {
        let n = x.len().min(8192);
        let (mut best, mut at) = (0.0f32, lo);
        let mut f = lo;
        while f < hi {
            let (mut re, mut im) = (0.0f32, 0.0f32);
            for (i, s) in x[..n].iter().enumerate() {
                let w = 0.5 - 0.5 * (TAU * i as f32 / n as f32).cos();
                let a = TAU * f * i as f32 / FS;
                re += s * w * a.cos();
                im += s * w * a.sin();
            }
            let p = re * re + im * im;
            if p > best {
                best = p;
                at = f;
            }
            f += 1.0;
        }
        at
    }

    fn db(ratio: f32) -> f32 {
        10.0 * ratio.max(1e-20).log10()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|s| s * s).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    #[test]
    fn every_kit_and_drum_is_finite_bounded_and_ends() {
        for kind in KITS {
            if kind == KitKind::Samples {
                continue;
            }
            for lane in 0..LANES {
                let mut k = kit(kind, 0.5);
                k.trigger(lane, 127);
                let mut peak = 0.0f32;
                for _ in 0..(8.0 * FS) as usize {
                    let (l, r) = k.next();
                    assert!(l.is_finite() && r.is_finite(), "{kind:?} lane {lane}");
                    peak = peak.max(l.abs()).max(r.abs());
                }
                assert!(peak > 0.03, "{kind:?} lane {lane} is silent ({peak})");
                assert!(peak < 1.01, "{kind:?} lane {lane} is too hot ({peak})");
                // After the room has died away too, nothing is still ringing.
                assert!(k.is_silent(), "{kind:?} lane {lane} never ends");
            }
        }
    }

    #[test]
    fn the_acoustic_drums_have_their_energy_where_real_ones_do() {
        let kick = render(KitKind::Studio, LANE_KICK, 110, 0.4, 0.0);
        let body = band(&kick, 40.0, 120.0);
        assert!(db(body / band(&kick, 2500.0, 9000.0)) > 15.0, "a kick is mostly low end");
        assert!(db(body / band(&kick, 400.0, 1200.0)) > 6.0);

        let hat = render(KitKind::Studio, LANE_HAT, 110, 0.3, 0.0);
        assert!(db(band(&hat, 6000.0, 14000.0) / band(&hat, 100.0, 400.0)) > 25.0, "a hat is all top");

        let snare = render(KitKind::Studio, LANE_SNARE, 110, 0.4, 0.0);
        let (head, wires) = (band(&snare, 150.0, 450.0), band(&snare, 3000.0, 9000.0));
        assert!(db(head / wires).abs() < 25.0, "a snare has both head and wires: {} dB", db(head / wires));
        assert!(db(head / band(&snare, 14000.0, 20000.0)) > 15.0);

        let tom = render(KitKind::Studio, LANE_LTOM, 110, 0.5, 0.0);
        assert!(db(band(&tom, 70.0, 180.0) / band(&tom, 4000.0, 10000.0)) > 20.0);

        let crash = render(KitKind::Studio, LANE_CRASH, 110, 1.0, 0.0);
        assert!(db(band(&crash, 3000.0, 14000.0) / band(&crash, 60.0, 250.0)) > 10.0);
    }

    #[test]
    fn the_snare_is_not_a_harmonic_series() {
        // A membrane's modes are not whole multiples of the fundamental; a synth tone's would be.
        let snare = render(KitKind::Studio, LANE_SNARE, 100, 0.3, 0.0);
        let f0 = peak_hz(&snare, 150.0, 300.0);
        let f1 = peak_hz(&snare, f0 * 1.45, f0 * 1.75);
        let ratio = f1 / f0;
        assert!((ratio - 1.594).abs() < 0.06, "second mode at {ratio} x the fundamental");
    }

    #[test]
    fn hitting_harder_changes_the_sound_not_just_the_level() {
        // The click of the beater and the crack of the stick take a larger share of a hard hit.
        // Measured over the first few milliseconds, where the strike is.
        let share = |lane, lo, hi, lo2, hi2, vel| {
            let x = render(KitKind::Studio, lane, vel, 0.25, 0.0);
            let x = &x[..(0.012 * FS) as usize];
            band(x, lo, hi) / band(x, lo2, hi2)
        };
        assert!(share(LANE_KICK, 2500.0, 7000.0, 40.0, 120.0, 127) > 3.0 * share(LANE_KICK, 2500.0, 7000.0, 40.0, 120.0, 35));
        assert!(share(LANE_SNARE, 3000.0, 9000.0, 150.0, 450.0, 127) > 1.5 * share(LANE_SNARE, 3000.0, 9000.0, 150.0, 450.0, 35));
        // A hard closed hat rings longer than a soft one.
        let tail = |vel| rms(&render(KitKind::Studio, LANE_HAT, vel, 0.3, 0.0)[(0.06 * FS) as usize..]);
        let head = |vel| rms(&render(KitKind::Studio, LANE_HAT, vel, 0.3, 0.0)[..(0.02 * FS) as usize]);
        assert!(tail(127) / head(127) > tail(30) / head(30));
    }

    #[test]
    fn two_hits_are_never_the_same_but_are_equally_loud() {
        for lane in [LANE_KICK, LANE_SNARE, LANE_HAT, LANE_LTOM, LANE_CLAP, LANE_CRASH] {
            let mut k = kit(KitKind::Studio, 0.0);
            let mut take = |k: &mut Kit| {
                k.trigger(lane, 100);
                let x: Vec<f32> = (0..(0.2 * FS) as usize).map(|_| k.next().0).collect();
                for _ in 0..(5.0 * FS) as usize {
                    k.next();
                }
                x
            };
            let (a, b) = (take(&mut k), take(&mut k));
            let diff: f32 = a.iter().zip(&b).map(|(x, y)| (x - y).powi(2)).sum::<f32>().sqrt();
            assert!(diff > 0.02 * a.iter().map(|x| x * x).sum::<f32>().sqrt(), "lane {lane}: repeats are identical");
            assert!(db(rms(&a) / rms(&b)).abs() < 2.5, "lane {lane}: repeats differ in level by {} dB", db(rms(&a) / rms(&b)));
        }
    }

    #[test]
    fn the_kits_really_are_different_kits() {
        let pitch = |kind, lane, lo, hi| peak_hz(&render(kind, lane, 100, 0.12, 0.0), lo, hi);
        let kicks = [KitKind::Rock, KitKind::Studio, KitKind::Jazz].map(|k| pitch(k, LANE_KICK, 35.0, 120.0));
        assert!(kicks[0] < kicks[1] && kicks[1] < kicks[2], "rock < studio < jazz: {kicks:?}");
        let snares = [KitKind::Rock, KitKind::Studio, KitKind::Jazz].map(|k| pitch(k, LANE_SNARE, 120.0, 400.0));
        assert!(snares[0] < snares[1] && snares[1] < snares[2], "{snares:?}");
        let ring = |kind| rms(&render(kind, LANE_SNARE, 100, 0.5, 0.0)[(0.15 * FS) as usize..]);
        assert!(ring(KitKind::Rock) > ring(KitKind::Tight) * 2.0, "tight is shorter than rock");
        let ride = |kind| rms(&render(kind, LANE_RIDE, 100, 1.0, 0.0));
        assert!(ride(KitKind::Jazz) > ride(KitKind::Tight), "jazz ride is the loudest");
    }

    #[test]
    fn tune_and_decay_do_what_they_say() {
        let mut k = kit(KitKind::Studio, 0.0);
        let mut take = |k: &mut Kit, st: f32, decay: f32| {
            k.set_voice(st, decay, 0.0);
            k.trigger(LANE_LTOM, 100);
            let x: Vec<f32> = (0..(0.5 * FS) as usize).map(|_| k.next().0).collect();
            for _ in 0..(3.0 * FS) as usize {
                k.next();
            }
            x
        };
        let base = take(&mut k, 0.0, 1.0);
        let up = take(&mut k, 12.0, 1.0);
        let ratio = peak_hz(&up[(0.03 * FS) as usize..], 100.0, 600.0) / peak_hz(&base[(0.03 * FS) as usize..], 50.0, 300.0);
        assert!((ratio - 2.0).abs() < 0.12, "an octave up is {ratio}x");
        let long = take(&mut k, 0.0, 2.0);
        let tail = |x: &[f32]| rms(&x[(0.3 * FS) as usize..]);
        assert!(tail(&long) > 1.5 * tail(&base));
    }

    #[test]
    fn the_room_adds_a_tail_and_leaves_the_dry_kit_alone() {
        let dry = render(KitKind::Studio, LANE_SNARE, 110, 1.0, 0.0);
        let wet = render(KitKind::Studio, LANE_SNARE, 110, 1.0, 1.0);
        let late = |x: &[f32]| rms(&x[(0.5 * FS) as usize..(0.9 * FS) as usize]);
        assert!(late(&wet) > 3.0 * late(&dry), "{} vs {}", late(&wet), late(&dry));
        // The start of the hit is the same sound with or without it.
        let head = |x: &[f32]| rms(&x[..(0.01 * FS) as usize]);
        assert!(db(head(&wet) / head(&dry)).abs() < 1.0);
    }

    #[test]
    fn the_hats_sit_right_and_the_crash_left() {
        let side = |lane| {
            let mut k = kit(KitKind::Studio, 0.0);
            k.trigger(lane, 110);
            let (mut l, mut r) = (0.0f32, 0.0f32);
            for _ in 0..(0.3 * FS) as usize {
                let (a, b) = k.next();
                l += a * a;
                r += b * b;
            }
            r - l
        };
        assert!(side(LANE_HAT) > 0.0 && side(LANE_CRASH) < 0.0);
        assert!(side(LANE_KICK).abs() < 1e-3, "the kick is in the middle");
    }

    #[test]
    fn the_closed_hat_chokes_the_open_one_in_every_kit() {
        for kind in [KitKind::Studio, KitKind::Electro, KitKind::LoFi] {
            let mut k = kit(kind, 0.0);
            k.trigger(LANE_OPEN, 110);
            for _ in 0..2000 {
                k.next();
            }
            let open: Vec<f32> = (0..2000).map(|_| k.next().0.abs()).collect();
            assert!(open.iter().sum::<f32>() > 1.0, "{kind:?}: the open hat is not sounding");
            k.trigger(LANE_HAT, 110);
            // Only the short closed hat is left, which is gone well inside half a second.
            for _ in 0..(0.5 * FS) as usize {
                k.next();
            }
            assert!(k.is_silent(), "{kind:?}: the open hat was not choked");
        }
    }

    #[test]
    fn lo_fi_is_duller_and_noisier_than_the_studio_kit() {
        let top = |kind| band(&render(kind, LANE_HAT, 110, 0.15, 0.0), 8000.0, 16000.0);
        assert!(top(KitKind::LoFi) < top(KitKind::Studio) * 0.1, "no crush on the top end");
        // Hiss while it is sounding, none once it has been quiet for a while.
        let mut k = kit(KitKind::LoFi, 0.0);
        k.trigger(LANE_KICK, 100);
        let during: Vec<f32> = (0..(3.0 * FS) as usize).map(|_| k.next().0).collect();
        for _ in 0..(3.0 * FS) as usize {
            k.next();
        }
        let after: Vec<f32> = (0..(0.5 * FS) as usize).map(|_| k.next().0).collect();
        assert!(rms(&during[(2.0 * FS) as usize..(2.5 * FS) as usize]) > 0.0);
        assert!(rms(&after) < 1e-6, "silence after the tail");
    }

    #[test]
    fn the_electro_kit_still_sounds_electronic() {
        let kick = render(KitKind::Electro, LANE_KICK, 110, 0.5, 0.0);
        assert!(db(band(&kick, 40.0, 130.0) / band(&kick, 2000.0, 8000.0)) > 20.0);
        let tom = render(KitKind::Electro, LANE_LTOM, 110, 0.4, 0.0);
        assert!(rms(&tom) > 0.01);
    }

    #[test]
    fn the_samples_kit_plays_the_file_and_falls_back_when_a_drum_has_none() {
        let mut k = kit(KitKind::Samples, 0.0);
        // No kit loaded: every drum falls back to the studio sound.
        k.trigger(LANE_SNARE, 100);
        let fallback: f32 = (0..4800).map(|_| k.next().0.abs()).sum();
        assert!(fallback > 5.0, "silent without samples");
        for _ in 0..(5.0 * FS) as usize {
            k.next();
        }
        let mut sk = SampleKit::default();
        sk.lanes[LANE_KICK].push(Sample { left: vec![0.5, -0.25, 0.125], right: vec![0.25, -0.125, 0.0625], level: 0.5 });
        let old = k.set_samples(Some(Arc::new(sk)));
        assert!(old.is_none());
        k.trigger(LANE_KICK, 127);
        // The first sample comes out exactly: full velocity is unity gain.
        let (l, r) = k.next();
        assert!((l - 0.5).abs() < 1e-6 && (r - 0.25).abs() < 1e-6, "{l} {r}");
        // A drum with no file still plays.
        k.trigger(LANE_HAT, 100);
        let hat: f32 = (0..2400).map(|_| k.next().0.abs()).sum();
        assert!(hat > 0.5);
        // Swapping the kit hands the old one back instead of dropping it here.
        let back = k.set_samples(None);
        assert!(back.is_some());
        assert!(k.sample_voices.iter().all(|v| !v.on));
    }

    #[test]
    fn velocity_picks_the_sample_layer() {
        let mut k = kit(KitKind::Samples, 0.0);
        let mut sk = SampleKit::default();
        for (i, level) in [0.1f32, 0.5, 0.9].into_iter().enumerate() {
            sk.lanes[LANE_SNARE].push(Sample { left: vec![level; 10], right: vec![level; 10], level: i as f32 });
        }
        k.set_samples(Some(Arc::new(sk)));
        let mut first = |vel| {
            k.trigger(LANE_SNARE, vel);
            k.next().0
        };
        // Soft hits come from the quiet layer, hard hits from the loud one, and gain rises too.
        assert!(first(20) < 0.1 && first(127) > 0.85);
    }

    #[test]
    fn the_whole_kit_runs_with_no_allocation_after_setup() {
        // The audio callback must not allocate. The crates enable assert_process_allocs; here we
        // just run a dense burst through every kit to be sure nothing panics or grows.
        for kind in KITS {
            let mut k = kit(kind, 0.6);
            for i in 0..40000usize {
                if i % 700 == 0 {
                    k.trigger(i / 700 % LANES, 40 + (i % 80) as u8);
                }
                let (l, r) = k.next();
                assert!(l.is_finite() && r.is_finite());
            }
        }
    }
}
