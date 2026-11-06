//! De-esser DSP.
//!
//! Detection: a band-pass on the mono sidechain, measured *relative* to the broadband level by
//! default, so a quiet take and a loud take trip the same threshold. The band centre follows the
//! sibilance: two flanking band-passes steer it until they hear equal energy.
//!
//! Reduction: a dynamic bell (or shelf) made by subtracting part of a band-pass from the input, so
//! the signal passes bit-for-bit when nothing is being reduced. The audio path is delayed by the
//! lookahead so the gain is already down when a sibilant arrives.

use nih_plug::prelude::{AtomicF32, Enum};
use std::f32::consts::{FRAC_1_SQRT_2, PI};
use std::sync::atomic::Ordering;
use std::sync::Arc;

pub const BANDS: usize = 48;
/// Lower and upper edge of the analysed range, log spaced across `BANDS`.
pub const F_MIN: f32 = 1500.0;
pub const F_MAX: f32 = 18000.0;
pub const MAX_LOOKAHEAD_MS: f32 = 5.0;

const FFT_SIZE: usize = 1024;
const HOP: usize = 512;
const DELAY_LEN: usize = 1024;
const RATIO: f32 = 5.0;
const KNEE_DB: f32 = 8.0;
/// How fast the tracker may move the centre at full imbalance, in octaves per second.
const TRACK_RATE: f32 = 12.0;
/// Update filter coefficients and the tracker every this many samples.
const CONTROL_RATE: u32 = 8;

#[derive(Enum, Debug, PartialEq, Clone, Copy)]
pub enum Shape {
    Bell,
    Shelf,
    Wideband,
}

#[derive(Enum, Debug, PartialEq, Clone, Copy)]
pub enum Detect {
    Relative,
    Absolute,
}

#[derive(Enum, Debug, PartialEq, Clone, Copy)]
pub enum Listen {
    Off,
    Trigger,
    Delta,
}

#[derive(Clone, Copy)]
pub struct Settings {
    pub threshold_db: f32,
    pub range_db: f32,
    pub freq: f32,
    pub width_oct: f32,
    /// Maximum travel of the tracked centre, in octaves each way.
    pub follow_oct: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
    pub lookahead: usize,
    pub shape: Shape,
    pub detect: Detect,
    pub listen: Listen,
}

/// Shared with the editor: band magnitudes before and after, and what the detector is doing.
pub struct Telemetry {
    pub dry: [AtomicF32; BANDS],
    pub wet: [AtomicF32; BANDS],
    pub gr_db: AtomicF32,
    pub detect_db: AtomicF32,
    pub center_hz: AtomicF32,
    pub sample_rate: AtomicF32,
    pub latency_samples: AtomicF32,
}

impl Default for Telemetry {
    fn default() -> Self {
        Self {
            dry: std::array::from_fn(|_| AtomicF32::new(0.0)),
            wet: std::array::from_fn(|_| AtomicF32::new(0.0)),
            gr_db: AtomicF32::new(0.0),
            detect_db: AtomicF32::new(-120.0),
            center_hz: AtomicF32::new(7000.0),
            sample_rate: AtomicF32::new(44100.0),
            latency_samples: AtomicF32::new(0.0),
        }
    }
}

/// Zavalishin topology-preserving state variable filter. Stable while its frequency moves.
#[derive(Clone, Copy, Default)]
struct Svf {
    k: f32,
    a1: f32,
    a2: f32,
    a3: f32,
    ic1: f32,
    ic2: f32,
}

impl Svf {
    fn set(&mut self, fs: f32, fc: f32, q: f32) {
        let g = (PI * fc.clamp(20.0, fs * 0.45) / fs).tan();
        self.k = 1.0 / q;
        self.a1 = 1.0 / (1.0 + g * (g + self.k));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
    }

    /// Returns `[band-pass with unity gain at the centre, high-pass]`.
    fn tick(&mut self, x: f32) -> [f32; 2] {
        let v3 = x - self.ic2;
        let v1 = self.a1 * self.ic1 + self.a2 * v3;
        let v2 = self.ic2 + self.a2 * self.ic1 + self.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        [self.k * v1, x - self.k * v1 - v2]
    }
}

fn coef(fs: f32, ms: f32) -> f32 {
    1.0 - (-1.0 / (ms * 0.001 * fs)).exp()
}

/// Q of a band-pass whose bandwidth is `octaves` wide.
fn q_from_octaves(octaves: f32) -> f32 {
    1.0 / (2.0 * (std::f32::consts::LN_2 / 2.0 * octaves).sinh())
}

fn db(power: f32) -> f32 {
    10.0 * power.log10()
}

/// Soft-knee downward compression curve: dB of reduction for a given dB above threshold.
fn reduction(over: f32) -> f32 {
    let slope = 1.0 - 1.0 / RATIO;
    if over <= -KNEE_DB / 2.0 {
        0.0
    } else if over >= KNEE_DB / 2.0 {
        over * slope
    } else {
        slope * (over + KNEE_DB / 2.0).powi(2) / (2.0 * KNEE_DB)
    }
}

pub struct Engine {
    fs: f32,
    det: Svf,
    lo: Svf,
    hi: Svf,
    proc: [Svf; 2],
    delay: [Vec<f32>; 2],
    pos: usize,
    tick: u32,
    p_sib: f32,
    p_full: f32,
    p_lo: f32,
    p_hi: f32,
    c_sib: f32,
    c_full: f32,
    c_track: f32,
    c_att: f32,
    c_rel: f32,
    cached_times: (f32, f32),
    last_relative_db: f32,
    /// Tracked offset of the centre from `Settings::freq`, in octaves.
    offset: f32,
    gr: f32,
    /// Samples left in which detection is ignored, so the filters' startup ring cannot trigger.
    settle: u32,

    pub center_hz: f32,
    pub detect_db: f32,
    pub gr_db: f32,
    /// The input delayed by the lookahead, and the processed signal before any listen mode.
    pub dry: [f32; 2],
    pub wet: [f32; 2],
}

impl Engine {
    pub fn new(fs: f32) -> Self {
        let mut engine = Self {
            fs,
            det: Svf::default(),
            lo: Svf::default(),
            hi: Svf::default(),
            proc: [Svf::default(); 2],
            delay: [vec![0.0; DELAY_LEN], vec![0.0; DELAY_LEN]],
            pos: 0,
            tick: 0,
            p_sib: 0.0,
            p_full: 0.0,
            p_lo: 0.0,
            p_hi: 0.0,
            c_sib: coef(fs, 1.2),
            c_full: coef(fs, 5.0),
            c_track: coef(fs, 12.0),
            c_att: 0.0,
            c_rel: 0.0,
            cached_times: (f32::NAN, f32::NAN),
            last_relative_db: -60.0,
            offset: 0.0,
            gr: 0.0,
            settle: (fs * 0.01) as u32,
            center_hz: 7000.0,
            detect_db: -120.0,
            gr_db: 0.0,
            dry: [0.0; 2],
            wet: [0.0; 2],
        };
        engine.update_filters(&Settings {
            threshold_db: -16.0,
            range_db: 9.0,
            freq: 7000.0,
            width_oct: 1.2,
            follow_oct: 0.5,
            attack_ms: 0.5,
            release_ms: 80.0,
            lookahead: 0,
            shape: Shape::Bell,
            detect: Detect::Relative,
            listen: Listen::Off,
        });
        engine
    }

    pub fn reset(&mut self) {
        for line in &mut self.delay {
            line.fill(0.0);
        }
        self.p_sib = 0.0;
        self.p_full = 0.0;
        self.p_lo = 0.0;
        self.p_hi = 0.0;
        self.offset = 0.0;
        self.gr = 0.0;
        self.settle = (self.fs * 0.01) as u32;
        let [p0, p1] = &mut self.proc;
        for f in [&mut self.det, &mut self.lo, &mut self.hi, p0, p1] {
            f.ic1 = 0.0;
            f.ic2 = 0.0;
        }
    }

    fn update_filters(&mut self, s: &Settings) {
        let fc = (s.freq * self.offset.exp2()).clamp(1000.0, (self.fs * 0.42).max(1000.0));
        let q = q_from_octaves(s.width_oct);
        self.det.set(self.fs, fc, q);
        self.lo.set(self.fs, fc / 1.4, 2.0);
        self.hi.set(self.fs, fc * 1.4, 2.0);
        let proc_q = if s.shape == Shape::Shelf { FRAC_1_SQRT_2 } else { q };
        for p in &mut self.proc {
            p.set(self.fs, fc, proc_q);
        }
        self.center_hz = fc;
    }

    /// Move the centre toward the side of the band that holds more energy, and drift home when
    /// there is no sibilance to follow.
    fn track(&mut self, s: &Settings) {
        let dt = CONTROL_RATE as f32 / self.fs;
        let activity = ((self.last_relative_db + 24.0) / 12.0).clamp(0.0, 1.0);
        let balance = (self.p_hi - self.p_lo) / (self.p_hi + self.p_lo + 1e-12);
        self.offset += balance * activity * TRACK_RATE * dt;
        self.offset -= self.offset * (1.0 - activity) * 2.0 * dt;
        self.offset = self.offset.clamp(-s.follow_oct, s.follow_oct);
    }

    pub fn process(&mut self, input: [f32; 2], channels: usize, s: &Settings) -> [f32; 2] {
        if self.tick.is_multiple_of(CONTROL_RATE) {
            self.track(s);
            self.update_filters(s);
        }
        self.tick = self.tick.wrapping_add(1);
        if self.cached_times != (s.attack_ms, s.release_ms) {
            self.cached_times = (s.attack_ms, s.release_ms);
            self.c_att = coef(self.fs, s.attack_ms);
            self.c_rel = coef(self.fs, s.release_ms);
        }

        // Detection.
        let mono = if channels == 2 { 0.5 * (input[0] + input[1]) } else { input[0] };
        let sib = self.det.tick(mono)[0];
        let lo = self.lo.tick(mono)[0];
        let hi = self.hi.tick(mono)[0];
        self.p_sib += (sib * sib - self.p_sib) * self.c_sib;
        self.p_full += (mono * mono - self.p_full) * self.c_full;
        self.p_lo += (lo * lo - self.p_lo) * self.c_track;
        self.p_hi += (hi * hi - self.p_hi) * self.c_track;

        // Sibilant energy against everything. The floor keeps silence and noise from triggering.
        let relative = db((self.p_sib + 1e-11) / (self.p_full + 1e-7)).min(6.0);
        self.last_relative_db = relative;
        self.detect_db = match s.detect {
            Detect::Relative => relative,
            Detect::Absolute => db(2.0 * self.p_sib + 1e-10),
        };

        let target = if self.settle > 0 {
            self.settle -= 1;
            0.0
        } else {
            reduction(self.detect_db - s.threshold_db).min(s.range_db)
        };
        let c = if target > self.gr { self.c_att } else { self.c_rel };
        self.gr += (target - self.gr) * c;
        if self.gr < 0.005 {
            self.gr = 0.0;
        }
        self.gr_db = self.gr;

        // Audio path.
        let g = 10f32.powf(-self.gr / 20.0);
        let delay = s.lookahead.min(DELAY_LEN - 1);
        let mut out = [0.0; 2];
        for ch in 0..channels.min(2) {
            self.delay[ch][self.pos] = input[ch];
            let d = self.delay[ch][(self.pos + DELAY_LEN - delay) % DELAY_LEN];
            let [bell, shelf] = self.proc[ch].tick(d);
            let y = match s.shape {
                Shape::Bell => d - (1.0 - g) * bell,
                Shape::Shelf => d - (1.0 - g) * shelf,
                Shape::Wideband => d * g,
            };
            self.dry[ch] = d;
            self.wet[ch] = y;
            out[ch] = match s.listen {
                Listen::Off => y,
                Listen::Trigger => sib,
                Listen::Delta => d - y,
            };
        }
        self.pos = (self.pos + 1) % DELAY_LEN;
        out
    }
}

/// Windowed FFT of the last `FFT_SIZE` samples of the dry and processed signals, folded into log
/// bands as RMS so the difference between the two reads as the amount removed.
pub struct Analyzer {
    window: Vec<f32>,
    tw_re: Vec<f32>,
    tw_im: Vec<f32>,
    re: Vec<f32>,
    im: Vec<f32>,
    dry_ring: Vec<f32>,
    wet_ring: Vec<f32>,
    pos: usize,
    since: usize,
    ranges: [(usize, usize); BANDS],
    telemetry: Arc<Telemetry>,
}

impl Analyzer {
    pub fn new(telemetry: Arc<Telemetry>) -> Self {
        let n = FFT_SIZE;
        Self {
            window: (0..n).map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / n as f32).cos()).collect(),
            tw_re: (0..n / 2).map(|k| (2.0 * PI * k as f32 / n as f32).cos()).collect(),
            tw_im: (0..n / 2).map(|k| -(2.0 * PI * k as f32 / n as f32).sin()).collect(),
            re: vec![0.0; n],
            im: vec![0.0; n],
            dry_ring: vec![0.0; n],
            wet_ring: vec![0.0; n],
            pos: 0,
            since: 0,
            ranges: [(0, 0); BANDS],
            telemetry,
        }
    }

    pub fn configure(&mut self, fs: f32) {
        let bin_hz = fs / FFT_SIZE as f32;
        let max_bin = FFT_SIZE / 2 - 1;
        for (i, range) in self.ranges.iter_mut().enumerate() {
            let edge = |k: usize| F_MIN * (F_MAX / F_MIN).powf(k as f32 / BANDS as f32);
            let lo = ((edge(i) / bin_hz).floor() as usize).min(max_bin);
            let hi = ((edge(i + 1) / bin_hz).ceil() as usize).clamp(lo, max_bin);
            *range = (lo, hi);
        }
        self.dry_ring.fill(0.0);
        self.wet_ring.fill(0.0);
    }

    pub fn push(&mut self, dry: f32, wet: f32) {
        self.dry_ring[self.pos] = dry;
        self.wet_ring[self.pos] = wet;
        self.pos = (self.pos + 1) % FFT_SIZE;
        self.since += 1;
        if self.since >= HOP {
            self.since = 0;
            self.analyze(true);
            self.analyze(false);
        }
    }

    fn analyze(&mut self, dry: bool) {
        let (ring, out) = if dry {
            (&self.dry_ring, &self.telemetry.dry)
        } else {
            (&self.wet_ring, &self.telemetry.wet)
        };
        for i in 0..FFT_SIZE {
            self.re[i] = ring[(self.pos + i) % FFT_SIZE] * self.window[i];
            self.im[i] = 0.0;
        }
        fft(&mut self.re, &mut self.im, &self.tw_re, &self.tw_im);
        let scale = 4.0 / FFT_SIZE as f32;
        for (band, &(lo, hi)) in self.ranges.iter().enumerate() {
            let mut power = 0.0;
            for bin in lo..=hi {
                power += self.re[bin] * self.re[bin] + self.im[bin] * self.im[bin];
            }
            out[band].store((power / (hi - lo + 1) as f32).sqrt() * scale, Ordering::Relaxed);
        }
    }
}

/// In-place iterative radix-2 FFT.
fn fft(re: &mut [f32], im: &mut [f32], tw_re: &[f32], tw_im: &[f32]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let step = n / len;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (wr, wi) = (tw_re[k * step], tw_im[k * step]);
                let (a, b) = (start + k, start + k + len / 2);
                let tr = re[b] * wr - im[b] * wi;
                let ti = re[b] * wi + im[b] * wr;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        len <<= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FS: f32 = 48000.0;

    fn settings() -> Settings {
        Settings {
            threshold_db: -16.0,
            range_db: 9.0,
            freq: 7000.0,
            width_oct: 1.2,
            follow_oct: 0.0,
            attack_ms: 0.5,
            release_ms: 80.0,
            lookahead: 96,
            shape: Shape::Bell,
            detect: Detect::Relative,
            listen: Listen::Off,
        }
    }

    fn run(input: &[f32], s: &Settings) -> (Engine, Vec<f32>) {
        let mut engine = Engine::new(FS);
        let out = input.iter().map(|&x| engine.process([x, x], 1, s)[0]).collect();
        (engine, out)
    }

    fn tone(f: f32, amp: f32, n: usize) -> Vec<f32> {
        (0..n).map(|i| amp * (2.0 * PI * f * i as f32 / FS).sin()).collect()
    }

    /// Highpassed noise: a stand-in for an "s".
    fn hiss(amp: f32, n: usize) -> Vec<f32> {
        let mut state = 12345u32;
        let mut hp = Svf::default();
        hp.set(FS, 5000.0, FRAC_1_SQRT_2);
        (0..n)
            .map(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                let white = (state >> 8) as f32 / (1u32 << 23) as f32 - 1.0;
                hp.tick(white)[1] * amp
            })
            .collect()
    }

    /// A vowel-ish tone, an "s" (the voice stops while it hisses), then the tone again.
    fn phrase(level: f32) -> Vec<f32> {
        let n = 14400;
        let mut x = tone(200.0, 0.3 * level, 3 * n);
        for (v, h) in x[n..2 * n].iter_mut().zip(hiss(0.25 * level, n)) {
            *v = h;
        }
        x
    }

    /// Samples to skip so the envelope followers have settled after the start.
    const WARMUP: usize = 4800;

    fn band_rms(x: &[f32], fc: f32) -> f32 {
        let mut f = Svf::default();
        f.set(FS, fc, 1.2);
        let sum: f32 = x.iter().map(|&v| f.tick(v)[0].powi(2)).sum();
        (sum / x.len() as f32).sqrt()
    }

    fn band_reduction_db(level: f32) -> f32 {
        let s = settings();
        let input = phrase(level);
        let (_, out) = run(&input, &s);
        let n = 14400;
        // Late in the sibilant, once attack has settled; the output lags by the lookahead.
        let range = n + 6000..2 * n - 1000;
        let delayed = &input[range.start - s.lookahead..range.end - s.lookahead];
        20.0 * (band_rms(&out[range], 7000.0) / band_rms(delayed, 7000.0)).log10()
    }

    #[test]
    fn passes_untouched_when_there_is_no_sibilance() {
        let s = settings();
        let input = tone(200.0, 0.3, 24000);
        let (engine, out) = run(&input, &s);
        assert_eq!(engine.gr_db, 0.0);
        for i in WARMUP..input.len() {
            assert_eq!(out[i], input[i - s.lookahead], "sample {i}");
        }
    }

    #[test]
    fn cuts_the_sibilant_band_not_the_body() {
        let s = settings();
        let input = phrase(1.0);
        let (_, out) = run(&input, &s);
        let n = 14400;
        let cut = band_reduction_db(1.0);
        assert!(cut < -3.0, "7 kHz band only came down {cut:.1} dB");
        for i in WARMUP..n {
            assert_eq!(out[i], input[i - s.lookahead]);
        }
    }

    #[test]
    fn bell_reaches_the_full_range_at_its_centre() {
        let s = settings();
        let (_, out) = run(&tone(7000.0, 0.2, 24000), &s);
        let late = 12000..23000;
        let cut = 20.0 * (band_rms(&out[late.clone()], 7000.0) / band_rms(&tone(7000.0, 0.2, 24000)[late], 7000.0)).log10();
        assert!((cut + s.range_db).abs() < 1.0, "cut {cut:.2} dB, range {}", s.range_db);
    }

    #[test]
    fn relative_detection_ignores_level() {
        let loud = band_reduction_db(1.0);
        let quiet = band_reduction_db(0.1);
        assert!((loud - quiet).abs() < 2.5, "loud {loud:.1} dB vs quiet {quiet:.1} dB");
    }

    #[test]
    fn wideband_mode_ducks_the_whole_signal() {
        let mut s = settings();
        s.shape = Shape::Wideband;
        let input = phrase(1.0);
        let (_, out) = run(&input, &s);
        let n = 14400;
        let range = n + 6000..2 * n - 1000;
        let rms = |x: &[f32]| (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt();
        let before = rms(&input[range.start - s.lookahead..range.end - s.lookahead]);
        let after = rms(&out[range]);
        assert!(after < before * 0.5, "the whole signal should duck: {before} -> {after}");
    }

    #[test]
    fn centre_follows_the_sibilance() {
        let mut s = settings();
        s.freq = 6500.0;
        s.follow_oct = 1.0;
        let (engine, _) = run(&tone(9000.0, 0.3, 48000), &s);
        assert!(engine.center_hz > 8000.0, "centre only reached {}", engine.center_hz);

        s.follow_oct = 0.0;
        let (engine, _) = run(&tone(9000.0, 0.3, 48000), &s);
        assert_eq!(engine.center_hz, 6500.0);
    }

    #[test]
    fn delta_is_what_was_removed() {
        let mut s = settings();
        let input = phrase(1.0);
        let (_, processed) = run(&input, &s);
        s.listen = Listen::Delta;
        let (_, delta) = run(&input, &s);
        for i in s.lookahead..input.len() {
            let removed = input[i - s.lookahead] - processed[i];
            assert!((delta[i] - removed).abs() < 1e-6, "sample {i}");
        }
    }

    #[test]
    fn analyzer_sees_the_cut() {
        let telemetry = Arc::new(Telemetry::default());
        let mut a = Analyzer::new(telemetry.clone());
        a.configure(FS);
        for x in tone(7000.0, 0.5, 8192) {
            a.push(x, x * 0.5);
        }
        let band = ((7000.0f32 / F_MIN).ln() / (F_MAX / F_MIN).ln() * BANDS as f32) as usize;
        let dry = telemetry.dry[band].load(Ordering::Relaxed);
        let wet = telemetry.wet[band].load(Ordering::Relaxed);
        assert!(dry > 0.1, "dry read {dry}");
        assert!((wet / dry - 0.5).abs() < 0.05, "wet/dry {}", wet / dry);
    }
}
