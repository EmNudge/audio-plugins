//! The acoustic drums: modal synthesis.
//!
//! A real drum is a few vibrating things at once, and each hit sets them going a little
//! differently. A voice here is built from the same parts:
//!
//! * **modes**, damped sines at the inharmonic frequencies of a membrane or plate (a circular
//!   head rings at 1, 1.59, 2.14, 2.30, 2.65, 2.92 times its fundamental, not at multiples of it);
//! * **pitched** partials whose pitch falls right after the strike, as a struck head's does;
//! * **noise layers** through a filter: the snare wires, the stick, the beater, the air in a hat.
//!
//! How hard the hit is changes the balance, not just the volume: a soft ghost note is mostly head
//! with a little wire buzz, a hard one has the stick crack, a deeper pitch drop and more wires.
//! Every hit also gets small random differences in pitch, decay and noise, so repeats are never
//! identical.

use std::f32::consts::TAU;

use crate::pattern::*;

pub const MAX_MODES: usize = 30;
const MAX_PITCHED: usize = 3;
const MAX_NOISE: usize = 6;

/// A damped resonator advanced by a rotation, so it costs a multiply per sample and no `sin`.
#[derive(Clone, Copy, Default)]
struct Mode {
    re: f32,
    im: f32,
    c: f32,
    s: f32,
}

#[derive(Clone, Copy, Default)]
struct Pitched {
    ph: f32,
    f0: f32,
    /// Fractional pitch above `f0` right now; falls to nothing.
    drop: f32,
    drop_k: f32,
    env: f32,
    k: f32,
    amp: f32,
}

#[derive(Clone, Copy, Default)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

#[derive(Clone, Copy)]
pub enum Shape {
    Band,
    High,
    Low,
}

impl Biquad {
    /// Robert Bristow-Johnson's cookbook filters.
    fn new(shape: Shape, fc: f32, q: f32, sr: f32) -> Self {
        let w = TAU * fc.clamp(20.0, sr * 0.45) / sr;
        let (sin, cos) = w.sin_cos();
        let alpha = sin / (2.0 * q.max(0.1));
        let (b0, b1, b2) = match shape {
            Shape::Band => (alpha, 0.0, -alpha),
            Shape::High => ((1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0),
            Shape::Low => ((1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0),
        };
        let a0 = 1.0 + alpha;
        Biquad { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: -2.0 * cos / a0, a2: (1.0 - alpha) / a0, z1: 0.0, z2: 0.0 }
    }

    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

#[derive(Clone, Copy, Default)]
struct Noise {
    filter: Biquad,
    env: f32,
    k: f32,
    amp: f32,
    /// Seconds to wait before it starts, for the staggered hands of a clap.
    start: f32,
}

#[derive(Clone, Copy)]
pub struct Layered {
    pub on: bool,
    age: f32,
    life: f32,
    gain: f32,
    drive: f32,
    modes: [Mode; MAX_MODES],
    nm: usize,
    pitched: [Pitched; MAX_PITCHED],
    np: usize,
    noise: [Noise; MAX_NOISE],
    nn: usize,
}

impl Default for Layered {
    fn default() -> Self {
        Layered {
            on: false,
            age: 0.0,
            life: 0.0,
            gain: 1.0,
            drive: 0.0,
            modes: [Mode::default(); MAX_MODES],
            nm: 0,
            pitched: [Pitched::default(); MAX_PITCHED],
            np: 0,
            noise: [Noise::default(); MAX_NOISE],
            nn: 0,
        }
    }
}

/// A per-sample factor for a decay with this time constant (it falls 60 dB in 6.9 of them).
fn k_of(tau: f32, dt: f32) -> f32 {
    (-dt / tau.max(1e-4)).exp()
}

impl Layered {
    fn mode(&mut self, f: f32, amp: f32, tau: f32, sr: f32) {
        if self.nm >= MAX_MODES || f >= sr * 0.45 {
            return;
        }
        let dt = 1.0 / sr;
        let (s, c) = (TAU * f * dt).sin_cos();
        let r = k_of(tau, dt);
        self.modes[self.nm] = Mode { re: amp, im: 0.0, c: r * c, s: r * s };
        self.nm += 1;
        self.life = self.life.max(tau * 6.9);
    }

    fn pitched(&mut self, f0: f32, drop: f32, drop_tau: f32, amp: f32, tau: f32, sr: f32) {
        if self.np >= MAX_PITCHED {
            return;
        }
        let dt = 1.0 / sr;
        self.pitched[self.np] = Pitched { ph: 0.0, f0, drop, drop_k: k_of(drop_tau, dt), env: 1.0, k: k_of(tau, dt), amp };
        self.np += 1;
        self.life = self.life.max(tau * 6.9);
    }

    fn noise(&mut self, shape: Shape, fc: f32, q: f32, amp: f32, tau: f32, start: f32, sr: f32) {
        if self.nn >= MAX_NOISE {
            return;
        }
        self.noise[self.nn] = Noise { filter: Biquad::new(shape, fc, q, sr), env: 1.0, k: k_of(tau, 1.0 / sr), amp, start };
        self.nn += 1;
        self.life = self.life.max(start + tau * 6.9);
    }

    /// `n` plate modes spread over `f_lo..f_hi` by a golden-ratio sequence, which gives the uneven,
    /// never-quite-harmonic spacing of a cymbal. High modes die faster than low ones.
    #[allow(clippy::too_many_arguments)]
    fn plate(&mut self, n: usize, f_lo: f32, f_hi: f32, tau_lo: f32, tau_hi: f32, amp: f32, offset: f32, sr: f32) {
        for i in 0..n {
            let u = (offset + i as f32 * 0.618_034).fract();
            let f = f_lo * (f_hi / f_lo).powf(u);
            let tau = tau_lo + (tau_hi - tau_lo) * u;
            // Brighter modes are a little quieter, with a spread so no two sit at the same level.
            let a = amp * (1.0 - 0.5 * u) * (0.6 + 0.8 * ((i * 7 + 3) % 5) as f32 / 4.0);
            self.mode(f, a, tau, sr);
        }
    }

    /// Scale the whole voice, e.g. by how hard the hit was.
    pub fn set_level(&mut self, level: f32) {
        self.gain *= level;
    }

    /// One sample. `white` is a fresh noise value in -1..1.
    pub fn next(&mut self, dt: f32, white: f32) -> f32 {
        let mut y = 0.0;
        for p in &mut self.pitched[..self.np] {
            p.ph += p.f0 * (1.0 + p.drop) * dt;
            p.ph -= p.ph.floor();
            y += (TAU * p.ph).sin() * p.amp * p.env;
            p.env *= p.k;
            p.drop *= p.drop_k;
        }
        for m in &mut self.modes[..self.nm] {
            let re = m.re * m.c - m.im * m.s;
            let im = m.re * m.s + m.im * m.c;
            m.re = re;
            m.im = im;
            y += im;
        }
        for n in &mut self.noise[..self.nn] {
            if self.age < n.start {
                continue;
            }
            y += n.filter.process(white) * n.amp * n.env;
            n.env *= n.k;
        }
        self.age += dt;
        if self.age > self.life {
            self.on = false;
        }
        if self.drive > 0.0 {
            y = (y * (1.0 + self.drive)).tanh() / (1.0 + self.drive * 0.35);
        }
        y * self.gain
    }
}

/// What makes one kit sound unlike another: tunings, decays and how bright the metal is.
pub struct Voicing {
    pub kick_f: f32,
    pub kick_tau: f32,
    pub kick_drop: f32,
    pub kick_click: f32,
    pub snare_f: f32,
    pub snare_tau: f32,
    pub wire: f32,
    pub wire_tau: f32,
    pub wire_fc: f32,
    pub tom_lo: f32,
    pub tom_hi: f32,
    pub tom_tau: f32,
    pub rim_f: f32,
    pub hat_fc: f32,
    pub hat_closed: f32,
    pub hat_open: f32,
    /// Scales the decay of crash and ride.
    pub cymbal: f32,
    pub ride_level: f32,
}

pub const STUDIO: Voicing = Voicing {
    kick_f: 58.0, kick_tau: 0.22, kick_drop: 1.1, kick_click: 1.0,
    snare_f: 205.0, snare_tau: 0.12, wire: 0.7, wire_tau: 0.2, wire_fc: 5200.0,
    tom_lo: 98.0, tom_hi: 150.0, tom_tau: 0.28, rim_f: 520.0,
    hat_fc: 8500.0, hat_closed: 0.05, hat_open: 0.38, cymbal: 1.0, ride_level: 1.0,
};

pub const ROCK: Voicing = Voicing {
    kick_f: 50.0, kick_tau: 0.3, kick_drop: 1.0, kick_click: 1.3,
    snare_f: 178.0, snare_tau: 0.16, wire: 0.85, wire_tau: 0.26, wire_fc: 4600.0,
    tom_lo: 80.0, tom_hi: 122.0, tom_tau: 0.4, rim_f: 470.0,
    hat_fc: 8000.0, hat_closed: 0.055, hat_open: 0.45, cymbal: 1.3, ride_level: 0.9,
};

pub const JAZZ: Voicing = Voicing {
    kick_f: 66.0, kick_tau: 0.16, kick_drop: 0.6, kick_click: 0.4,
    snare_f: 250.0, snare_tau: 0.09, wire: 0.5, wire_tau: 0.14, wire_fc: 6500.0,
    tom_lo: 118.0, tom_hi: 178.0, tom_tau: 0.22, rim_f: 600.0,
    hat_fc: 9500.0, hat_closed: 0.065, hat_open: 0.3, cymbal: 0.8, ride_level: 1.5,
};

pub const TIGHT: Voicing = Voicing {
    kick_f: 62.0, kick_tau: 0.12, kick_drop: 0.9, kick_click: 1.0,
    snare_f: 215.0, snare_tau: 0.07, wire: 0.6, wire_tau: 0.1, wire_fc: 5600.0,
    tom_lo: 105.0, tom_hi: 160.0, tom_tau: 0.16, rim_f: 540.0,
    hat_fc: 9000.0, hat_closed: 0.035, hat_open: 0.18, cymbal: 0.6, ride_level: 0.8,
};

/// Ratios of a circular membrane's modes to its fundamental, and how loud and long each rings.
const MEMBRANE: [(f32, f32, f32); 6] = [(1.0, 1.0, 1.0), (1.594, 0.55, 0.75), (2.136, 0.38, 0.6), (2.296, 0.3, 0.55), (2.653, 0.22, 0.45), (2.917, 0.16, 0.4)];

/// A random number in -1..1, advancing `state`.
pub fn jitter(state: &mut u32) -> f32 {
    let mut x = (*state).max(1);
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    (x >> 8) as f32 / (1u32 << 23) as f32 - 1.0
}

/// Set up `v` to play `lane` as a hit of strength `hard` (0..1) in `voicing`.
#[allow(clippy::too_many_arguments)]
pub fn build(v: &mut Layered, lane: usize, vo: &Voicing, hard: f32, tune: f32, decay: f32, rng: &mut u32, sr: f32) {
    *v = Layered { on: true, ..Layered::default() };
    // Differences between one hit and the next.
    let (jp, jd, jn) = (1.0 + 0.012 * jitter(rng), 1.0 + 0.08 * jitter(rng), 1.0 + 0.12 * jitter(rng));
    let tau = |t: f32| t * decay * jd;
    match lane {
        LANE_KICK => {
            let f = vo.kick_f * tune * jp;
            // The head's pitch falls as the strike's tension relaxes; harder hits start higher.
            v.pitched(f, vo.kick_drop * (0.35 + 0.65 * hard), 0.02, 1.0, tau(vo.kick_tau), sr);
            // The shell and the air in it.
            v.mode(f * 2.7, 0.14, tau(0.07), sr);
            v.noise(Shape::Low, 170.0, 0.7, 0.45 * jn, tau(0.05), 0.0, sr);
            // The beater on the head.
            v.noise(Shape::Band, 3200.0 + 2400.0 * hard, 1.1, vo.kick_click * (0.15 + 0.85 * hard) * jn, 0.0028, 0.0, sr);
            v.drive = 0.3 + 0.9 * hard;
            v.gain = 1.2;
        }
        LANE_SNARE => {
            let f = vo.snare_f * tune * jp;
            let tone = 0.55 + 0.45 * hard;
            v.pitched(f, 0.1 * hard, 0.012, tone, tau(vo.snare_tau), sr);
            for &(ratio, amp, t) in &MEMBRANE[1..] {
                v.mode(f * ratio * (1.0 + 0.004 * jitter(rng)), tone * amp * 0.8, tau(vo.snare_tau * t), sr);
            }
            // The wires answer the head: hard hits wake more of them.
            let wires = vo.wire * (0.45 + 0.75 * hard) * jn;
            v.noise(Shape::Band, vo.wire_fc, 0.55, wires, tau(vo.wire_tau), 0.0, sr);
            v.noise(Shape::High, 2200.0, 0.7, wires * 0.35, tau(vo.wire_tau * 0.5), 0.0, sr);
            v.noise(Shape::Band, 2600.0, 1.0, 0.45 * hard * hard, 0.0016, 0.0, sr);
            v.gain = 0.42;
        }
        LANE_LTOM | LANE_HTOM => {
            let base = if lane == LANE_LTOM { vo.tom_lo } else { vo.tom_hi };
            let f = base * tune * jp;
            v.pitched(f, 0.22 * (0.4 + 0.6 * hard), 0.035, 1.0, tau(vo.tom_tau), sr);
            for &(ratio, amp, t) in &MEMBRANE[1..4] {
                v.mode(f * ratio, amp * 0.5, tau(vo.tom_tau * t), sr);
            }
            v.noise(Shape::Band, 2200.0, 1.0, 0.3 * hard * jn, 0.003, 0.0, sr);
            v.noise(Shape::Low, 400.0, 0.7, 0.15, tau(0.04), 0.0, sr);
            v.gain = 0.55;
        }
        LANE_RIM => {
            // A side stick: the stick across the head and rim, a short woody knock.
            let f = vo.rim_f * tune * jp;
            v.mode(f, 0.7, 0.022 * decay, sr);
            v.mode(f * 3.7, 0.45, 0.014 * decay, sr);
            v.mode(f * 1.62, 0.3, 0.03 * decay, sr);
            v.noise(Shape::Band, 3600.0, 1.2, 0.5 * jn, 0.002, 0.0, sr);
            v.gain = 0.6;
        }
        LANE_CLAP => {
            // Several hands, a few milliseconds apart and never evenly, then the room.
            let mut start = 0.0;
            for _ in 0..4 {
                v.noise(Shape::Band, 1150.0 * jp, 1.4, 0.7 * jn, 0.0045, start, sr);
                start += 0.009 + 0.004 * (jitter(rng) * 0.5 + 0.5);
            }
            v.noise(Shape::Band, 1050.0, 0.9, 0.4, tau(0.13), start - 0.009, sr);
            v.noise(Shape::High, 2600.0, 0.7, 0.12, tau(0.1), start - 0.009, sr);
            v.gain = 2.3;
        }
        LANE_HAT | LANE_OPEN => {
            let open = lane == LANE_OPEN;
            let t = if open { vo.hat_open } else { vo.hat_closed };
            // A hard hit on closed hats is longer and brighter; a soft one is a tick.
            let t = if open { t } else { t * (0.7 + 0.7 * hard) };
            v.plate(12, 3200.0, 14000.0, tau(t * 0.9), tau(t * 0.35), 0.05, (jitter(rng) * 0.5 + 0.5) * 0.3, sr);
            v.noise(Shape::Band, vo.hat_fc * (0.9 + 0.2 * hard), 0.8, 0.55 * jn, tau(t), 0.0, sr);
            v.noise(Shape::High, 5500.0, 0.7, 0.3, tau(t * 0.6), 0.0, sr);
            v.noise(Shape::High, 7000.0, 0.8, 0.45 * hard, 0.0016, 0.0, sr);
            v.gain = 1.1;
        }
        LANE_CRASH => {
            let c = vo.cymbal * decay;
            v.plate(28, 340.0, 14500.0, 0.7 * c * jd, 0.15 * c, 0.05, (jitter(rng) * 0.5 + 0.5) * 0.3, sr);
            v.noise(Shape::Band, 6200.0, 0.5, 0.5 * jn, 0.4 * c, 0.0, sr);
            v.noise(Shape::High, 9000.0, 0.7, 0.2, 0.2 * c, 0.0, sr);
            v.noise(Shape::Band, 4200.0, 1.0, 0.5 * hard, 0.004, 0.0, sr);
            v.gain = 0.7;
        }
        LANE_RIDE => {
            let c = vo.cymbal * decay;
            v.plate(22, 650.0, 12500.0, 0.5 * c * jd, 0.1 * c, 0.045, (jitter(rng) * 0.5 + 0.5) * 0.3, sr);
            // The bell and the ping of the stick.
            v.mode(690.0 * tune, 0.3, 0.3 * c, sr);
            v.mode(2950.0 * tune, 0.22, 0.35 * c, sr);
            v.noise(Shape::Band, 5200.0, 1.0, 0.4 * (0.4 + 0.6 * hard), 0.0045, 0.0, sr);
            v.noise(Shape::High, 7500.0, 0.7, 0.13, 0.3 * c, 0.0, sr);
            v.gain = 0.7 * vo.ride_level;
        }
        _ => {
            // A wood block: two quick modes and a knock.
            let f = 810.0 * tune * jp;
            v.mode(f, 0.8, 0.055 * decay, sr);
            v.mode(f * 2.11, 0.4, 0.035 * decay, sr);
            v.mode(f * 3.6, 0.2, 0.02 * decay, sr);
            v.noise(Shape::Band, 3000.0, 1.2, 0.35 * jn, 0.0025, 0.0, sr);
            v.gain = 0.55;
        }
    }
}
