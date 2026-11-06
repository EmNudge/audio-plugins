//! Pitch detection: a McLeod-style normalised autocorrelation tracker on a decimated signal.
//!
//! The input is lowpassed and decimated to roughly 12 kHz, which keeps the lag search cheap
//! while parabolic interpolation still resolves pitch to well under a cent. Octave errors are
//! suppressed by preferring the candidate closest to the previous pitch, and the voiced decision
//! uses the peak clarity with hysteresis so consonants don't flicker the state.

use std::f32::consts::{FRAC_1_SQRT_2, PI};

const MAX_WINDOW: usize = 2048;
const MAX_TAU: usize = 1000;
const HOP_SECONDS: f32 = 0.0027;

/// Clarity (NSDF peak height) needed to call a frame voiced, and to keep it voiced.
const VOICED_ON: f32 = 0.86;
const VOICED_OFF: f32 = 0.72;
/// Linear level below which input is treated as silence (about -58 dBFS).
const GATE: f32 = 0.0012;
const MAX_F0: f32 = 1000.0;

#[derive(Clone, Copy, Default)]
struct Lowpass {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Lowpass {
    fn new(fs: f32, fc: f32) -> Self {
        let w0 = 2.0 * PI * fc / fs;
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * FRAC_1_SQRT_2);
        let a0 = 1.0 + alpha;
        Self {
            b0: (1.0 - cos) / 2.0 / a0,
            b1: (1.0 - cos) / a0,
            b2: (1.0 - cos) / 2.0 / a0,
            a1: -2.0 * cos / a0,
            a2: (1.0 - alpha) / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    fn clear(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }

    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// One analysis result, produced every hop.
#[derive(Clone, Copy, Debug)]
pub struct PitchFrame {
    /// Fundamental in Hz. Only meaningful when `voiced`.
    pub f0: f32,
    /// Period in input samples at the full sample rate.
    pub period: f32,
    #[allow(dead_code)]
    pub clarity: f32,
    pub voiced: bool,
}

pub struct PitchTracker {
    factor: usize,
    fd: f32,
    filters: [Lowpass; 2],
    phase: usize,
    ring: Vec<f32>,
    ring_pos: usize,
    since_hop: usize,
    hop: usize,
    window: usize,
    tau_min: usize,
    tau_max: usize,
    scratch: Vec<f32>,
    nsdf: Vec<f32>,
    prev_tau: f32,
    on_count: u32,
    off_count: u32,
    voiced: bool,
    last: PitchFrame,
}

impl PitchTracker {
    pub fn new(fs: f32, f_min: f32) -> Self {
        let factor = ((fs / 12000.0).round() as usize).clamp(1, 8);
        let fd = fs / factor as f32;
        let tau_max = ((fd / f_min).ceil() as usize + 2).min(MAX_TAU);
        let window = (tau_max * 2).min(MAX_WINDOW);
        Self {
            factor,
            fd,
            filters: [Lowpass::new(fs, 3000.0_f32.min(fs * 0.4)); 2],
            phase: 0,
            ring: vec![0.0; MAX_WINDOW],
            ring_pos: 0,
            since_hop: 0,
            hop: ((HOP_SECONDS * fd) as usize).max(8),
            window,
            tau_min: ((fd / MAX_F0) as usize).max(2),
            tau_max,
            scratch: vec![0.0; MAX_WINDOW],
            nsdf: vec![0.0; MAX_TAU + 2],
            prev_tau: 0.0,
            on_count: 0,
            off_count: 0,
            voiced: false,
            last: PitchFrame { f0: 0.0, period: 0.0, clarity: 0.0, voiced: false },
        }
    }

    pub fn reset(&mut self) {
        for f in &mut self.filters {
            f.clear();
        }
        self.ring.fill(0.0);
        self.ring_pos = 0;
        self.phase = 0;
        self.since_hop = 0;
        self.prev_tau = 0.0;
        self.on_count = 0;
        self.off_count = 0;
        self.voiced = false;
        self.last = PitchFrame { f0: 0.0, period: 0.0, clarity: 0.0, voiced: false };
    }

    pub fn hop_seconds(&self) -> f32 {
        (self.hop * self.factor) as f32 / (self.fd * self.factor as f32)
    }

    /// Feed one input sample. Returns a frame once per hop.
    pub fn push(&mut self, x: f32) -> Option<PitchFrame> {
        let first = self.filters[0].process(x);
        let y = self.filters[1].process(first);
        self.phase += 1;
        if self.phase < self.factor {
            return None;
        }
        self.phase = 0;
        self.ring[self.ring_pos] = y;
        self.ring_pos = (self.ring_pos + 1) % MAX_WINDOW;
        self.since_hop += 1;
        if self.since_hop < self.hop {
            return None;
        }
        self.since_hop = 0;
        Some(self.analyze())
    }

    fn analyze(&mut self) -> PitchFrame {
        let w = self.window;
        let mut energy = 0.0;
        for i in 0..w {
            let v = self.ring[(self.ring_pos + MAX_WINDOW - w + i) % MAX_WINDOW];
            self.scratch[i] = v;
            energy += v * v;
        }
        let rms = (energy / w as f32).sqrt();

        // NSDF: 2 r'(tau) / m'(tau).
        let x = &self.scratch[..w];
        let mut m = 2.0 * energy;
        for tau in 0..=self.tau_max {
            if tau > 0 {
                m -= x[tau - 1] * x[tau - 1] + x[w - tau] * x[w - tau];
            }
            let mut r = 0.0;
            for j in 0..w - tau {
                r += x[j] * x[j + tau];
            }
            self.nsdf[tau] = if m > 1e-12 { 2.0 * r / m } else { 0.0 };
        }

        // Key maxima: the highest point of each positive lobe after the first zero crossing.
        let mut peaks = [(0usize, 0.0f32); 24];
        let mut n_peaks = 0;
        let mut tau = 1;
        while tau < self.tau_max && self.nsdf[tau] > 0.0 {
            tau += 1;
        }
        while tau < self.tau_max && n_peaks < peaks.len() {
            while tau < self.tau_max && self.nsdf[tau] <= 0.0 {
                tau += 1;
            }
            let mut best = (0usize, f32::MIN);
            while tau < self.tau_max && self.nsdf[tau] > 0.0 {
                if self.nsdf[tau] > best.1 {
                    best = (tau, self.nsdf[tau]);
                }
                tau += 1;
            }
            if best.0 >= self.tau_min && best.0 > 0 && best.1 > f32::MIN {
                peaks[n_peaks] = best;
                n_peaks += 1;
            }
        }

        let mut clarity = 0.0;
        let mut period_d = 0.0;
        if n_peaks > 0 {
            let max_val = peaks[..n_peaks].iter().map(|p| p.1).fold(0.0, f32::max);
            let mut chosen = peaks[..n_peaks].iter().find(|p| p.1 >= 0.9 * max_val).copied().unwrap();
            if self.prev_tau > 0.0 {
                let near = peaks[..n_peaks]
                    .iter()
                    .filter(|p| p.1 >= 0.75 * max_val)
                    .min_by(|a, b| {
                        let da = (a.0 as f32 / self.prev_tau).ln().abs();
                        let db = (b.0 as f32 / self.prev_tau).ln().abs();
                        da.total_cmp(&db)
                    })
                    .copied();
                if let Some(p) = near {
                    if (p.0 as f32 / self.prev_tau).ln().abs() < 0.08 {
                        chosen = p;
                    }
                }
            }
            // Parabolic interpolation around the chosen lag.
            let t = chosen.0;
            let (a, b, c) = (self.nsdf[t - 1], self.nsdf[t], self.nsdf[t + 1]);
            let denom = a - 2.0 * b + c;
            let delta = if denom.abs() > 1e-9 { (0.5 * (a - c) / denom).clamp(-1.0, 1.0) } else { 0.0 };
            period_d = t as f32 + delta;
            clarity = b - 0.25 * (a - c) * delta;
        }

        let f0 = if period_d > 0.0 { self.fd / period_d } else { 0.0 };
        let plausible = f0 >= self.fd / self.tau_max as f32 && f0 <= MAX_F0 && rms > GATE;
        let raw_voiced = plausible && clarity >= if self.voiced { VOICED_OFF } else { VOICED_ON };
        if raw_voiced {
            self.on_count += 1;
            self.off_count = 0;
        } else {
            self.off_count += 1;
            self.on_count = 0;
        }
        if !self.voiced && self.on_count >= 2 {
            self.voiced = true;
        } else if self.voiced && self.off_count >= 3 {
            self.voiced = false;
            self.prev_tau = 0.0;
        }
        if raw_voiced {
            self.prev_tau = period_d;
        }

        self.last = if raw_voiced {
            PitchFrame { f0, period: period_d * self.factor as f32, clarity, voiced: self.voiced }
        } else {
            PitchFrame { voiced: self.voiced, clarity, ..self.last }
        };
        self.last
    }
}
