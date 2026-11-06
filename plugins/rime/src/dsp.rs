//! Resonance suppressor DSP.
//!
//! A 4x-overlapped STFT. In every frame each bin is compared with the mean level of its
//! neighbourhood (in dB, over a width that grows with frequency). Bins that stick out above it are
//! resonances and get a gain cut. The cut is smoothed across frequency (sharpness) and over time
//! (attack / release), optionally linked between channels, then applied as a zero-phase gain.

use nih_plug::prelude::AtomicF32;
use std::f32::consts::PI;
use std::sync::atomic::Ordering;
use std::sync::Arc;

pub const FFT_SIZE: usize = 2048;
const HOP: usize = FFT_SIZE / 4;
/// Index of the Nyquist bin; per-bin arrays have `BINS + 1` entries.
const BINS: usize = FFT_SIZE / 2;
/// Samples between input and output: one full frame has to be collected before it can be output.
pub const LATENCY: usize = FFT_SIZE;
pub const MAX_CHANNELS: usize = 2;

pub const BANDS: usize = 96;
/// Lower and upper edge of the displayed range, log spaced across `BANDS`.
pub const F_MIN: f32 = 40.0;
pub const F_MAX: f32 = 20000.0;

/// A bin has to stand this far above its surroundings before it counts as a resonance.
const KNEE_DB: f32 = 3.0;
/// At full depth the cut is this multiple of the excess; the local mean includes the peak itself,
/// so 1.0 would not quite flatten it.
const DEPTH_GAIN: f32 = 1.5;
const MAX_REDUCTION_DB: f32 = 24.0;
/// Raw bin power (dB, before normalisation) under which a bin is treated as silence.
const SILENCE_DB: f32 = -20.0;
const DB_TO_NEPER: f32 = 0.115_129_25;

/// Per-band levels, written by the audio thread and read by the editor.
pub struct Meter {
    /// Input level in dB relative to a full-scale sine.
    pub level: [AtomicF32; BANDS],
    /// Gain reduction in dB, zero or positive.
    pub reduction: [AtomicF32; BANDS],
}

impl Default for Meter {
    fn default() -> Self {
        Self {
            level: std::array::from_fn(|_| AtomicF32::new(-120.0)),
            reduction: std::array::from_fn(|_| AtomicF32::new(0.0)),
        }
    }
}

#[derive(Clone, Copy)]
pub struct Settings {
    pub depth: f32,
    pub sharpness: f32,
    pub selectivity: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
    pub low_hz: f32,
    pub high_hz: f32,
    pub link: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            depth: 0.5,
            sharpness: 0.5,
            selectivity: 0.5,
            attack_ms: 8.0,
            release_ms: 150.0,
            low_hz: F_MIN,
            high_hz: F_MAX,
            link: 1.0,
        }
    }
}

pub struct Suppressor {
    fs: f32,
    channels: usize,
    pub settings: Settings,
    window: Vec<f32>,
    tw_re: Vec<f32>,
    tw_im: Vec<f32>,
    /// Converts a windowed FFT magnitude into the amplitude of the sine that produced it.
    norm_db: f32,
    /// The last `FFT_SIZE` input samples; the oldest one doubles as the latency-matched dry signal.
    input: [Vec<f32>; MAX_CHANNELS],
    pos: usize,
    since: usize,
    acc: [Vec<f32>; MAX_CHANNELS],
    out: [[f32; HOP]; MAX_CHANNELS],
    out_pos: usize,
    re: [Vec<f32>; MAX_CHANNELS],
    im: [Vec<f32>; MAX_CHANNELS],
    power: [Vec<f32>; MAX_CHANNELS],
    db: [Vec<f32>; MAX_CHANNELS],
    target: [Vec<f32>; MAX_CHANNELS],
    state: [Vec<f32>; MAX_CHANNELS],
    prefix: Vec<f32>,
    weight: Vec<f32>,
    weight_for: (f32, f32),
    ranges: [(usize, usize); BANDS],
    pub meter: Arc<Meter>,
}

fn per_channel(len: usize) -> [Vec<f32>; MAX_CHANNELS] {
    std::array::from_fn(|_| vec![0.0; len])
}

impl Suppressor {
    pub fn new() -> Self {
        let n = FFT_SIZE;
        // Periodic sqrt-Hann: applied on analysis and synthesis it gives Hann, which sums to 2
        // at 4x overlap.
        let window: Vec<f32> = (0..n).map(|i| (0.5 - 0.5 * (2.0 * PI * i as f32 / n as f32).cos()).sqrt()).collect();
        let norm_db = 20.0 * (window.iter().sum::<f32>() / 2.0).log10();
        let mut this = Self {
            fs: 44100.0,
            channels: 2,
            settings: Settings::default(),
            window,
            tw_re: (0..n / 2).map(|k| (2.0 * PI * k as f32 / n as f32).cos()).collect(),
            tw_im: (0..n / 2).map(|k| -(2.0 * PI * k as f32 / n as f32).sin()).collect(),
            norm_db,
            input: per_channel(n),
            pos: 0,
            since: 0,
            acc: per_channel(n),
            out: [[0.0; HOP]; MAX_CHANNELS],
            out_pos: 0,
            re: per_channel(n),
            im: per_channel(n),
            power: per_channel(BINS + 1),
            db: per_channel(BINS + 1),
            target: per_channel(BINS + 1),
            state: per_channel(BINS + 1),
            prefix: vec![0.0; BINS + 2],
            weight: vec![1.0; BINS + 1],
            weight_for: (0.0, 0.0),
            ranges: [(1, 1); BANDS],
            meter: Arc::new(Meter::default()),
        };
        this.configure(44100.0, 2);
        this
    }

    pub fn configure(&mut self, fs: f32, channels: usize) {
        self.fs = fs;
        self.channels = channels.clamp(1, MAX_CHANNELS);
        let bin_hz = fs / FFT_SIZE as f32;
        for (i, range) in self.ranges.iter_mut().enumerate() {
            let edge = |k: usize| F_MIN * (F_MAX / F_MIN).powf(k as f32 / BANDS as f32);
            let lo = ((edge(i) / bin_hz).floor() as usize).clamp(1, BINS);
            let hi = ((edge(i + 1) / bin_hz).ceil() as usize).clamp(lo, BINS);
            *range = (lo, hi);
        }
        self.weight_for = (0.0, 0.0);
        self.reset();
    }

    pub fn reset(&mut self) {
        for ch in 0..MAX_CHANNELS {
            self.input[ch].fill(0.0);
            self.acc[ch].fill(0.0);
            self.out[ch].fill(0.0);
            self.power[ch].fill(0.0);
            self.state[ch].fill(0.0);
        }
        self.pos = 0;
        self.since = 0;
        self.out_pos = 0;
    }

    /// Returns `(processed, dry)` for one frame of samples; both are delayed by `LATENCY`.
    pub fn process(&mut self, x: [f32; MAX_CHANNELS]) -> ([f32; MAX_CHANNELS], [f32; MAX_CHANNELS]) {
        let (mut wet, mut dry) = ([0.0; MAX_CHANNELS], [0.0; MAX_CHANNELS]);
        for ch in 0..self.channels {
            // Before it is overwritten, this slot holds the sample from exactly `FFT_SIZE` ago.
            dry[ch] = self.input[ch][self.pos];
            self.input[ch][self.pos] = x[ch];
            wet[ch] = self.out[ch][self.out_pos];
        }
        self.out_pos += 1;
        self.pos = (self.pos + 1) % FFT_SIZE;
        self.since += 1;
        if self.since == HOP {
            self.since = 0;
            self.frame();
            self.out_pos = 0;
        }
        (wet, dry)
    }

    fn update_weights(&mut self) {
        let (low, high) = (self.settings.low_hz, self.settings.high_hz);
        if (low, high) == self.weight_for {
            return;
        }
        self.weight_for = (low, high);
        let bin_hz = self.fs / FFT_SIZE as f32;
        for (k, w) in self.weight.iter_mut().enumerate() {
            let octaves_up = (k as f32 * bin_hz).max(1.0).log2();
            let above_low = 1.0 / (1.0 + (-8.0 * (octaves_up - low.log2())).exp());
            let below_high = 1.0 / (1.0 + (8.0 * (octaves_up - high.log2())).exp());
            *w = above_low * below_high;
        }
    }

    fn frame(&mut self) {
        let s = self.settings;
        let c = self.channels;
        self.update_weights();

        for ch in 0..c {
            for i in 0..FFT_SIZE {
                self.re[ch][i] = self.input[ch][(self.pos + i) % FFT_SIZE] * self.window[i];
                self.im[ch][i] = 0.0;
            }
            fft(&mut self.re[ch], &mut self.im[ch], &self.tw_re, &self.tw_im);
            for k in 0..=BINS {
                let p = self.re[ch][k] * self.re[ch][k] + self.im[ch][k] * self.im[ch][k];
                // A little smoothing over time keeps noise from flickering in and out of "resonance".
                self.power[ch][k] += (p - self.power[ch][k]) * 0.5;
                self.db[ch][k] = 10.0 * self.power[ch][k].max(1e-18).log10();
            }
            detect(&self.db[ch], &mut self.prefix, &mut self.target[ch], &s);
        }

        let frame_s = HOP as f32 / self.fs;
        let coef = |ms: f32| if ms <= 0.0 { 1.0 } else { 1.0 - (-frame_s / (ms * 0.001)).exp() };
        let (attack, release) = (coef(s.attack_ms), coef(s.release_ms));
        for k in 0..=BINS {
            let mean = (0..c).map(|ch| self.target[ch][k]).sum::<f32>() / c as f32;
            for ch in 0..c {
                let own = self.target[ch][k];
                let t = own + (mean - own) * s.link;
                let st = &mut self.state[ch][k];
                *st += (t - *st) * if t > *st { attack } else { release };
            }
        }

        for ch in 0..c {
            for k in 0..=BINS {
                let g = (-self.state[ch][k] * self.weight[k] * DB_TO_NEPER).exp();
                self.re[ch][k] *= g;
                self.im[ch][k] *= g;
                if k > 0 && k < BINS {
                    self.re[ch][FFT_SIZE - k] = self.re[ch][k];
                    self.im[ch][FFT_SIZE - k] = -self.im[ch][k];
                }
            }
            // Inverse FFT through the forward one: conjugate in, take the real part out.
            for v in self.im[ch].iter_mut() {
                *v = -*v;
            }
            fft(&mut self.re[ch], &mut self.im[ch], &self.tw_re, &self.tw_im);
            let scale = 0.5 / FFT_SIZE as f32;
            for i in 0..FFT_SIZE {
                self.acc[ch][i] += self.re[ch][i] * self.window[i] * scale;
            }
            self.out[ch].copy_from_slice(&self.acc[ch][..HOP]);
            self.acc[ch].copy_within(HOP.., 0);
            self.acc[ch][FFT_SIZE - HOP..].fill(0.0);
        }

        self.publish();
    }

    fn publish(&self) {
        let c = self.channels;
        for (band, &(lo, hi)) in self.ranges.iter().enumerate() {
            let (mut level, mut reduction) = (-120.0f32, 0.0f32);
            for k in lo..=hi {
                for ch in 0..c {
                    level = level.max(self.db[ch][k] - self.norm_db);
                }
                let r = (0..c).map(|ch| self.state[ch][k]).sum::<f32>() / c as f32;
                reduction = reduction.max(r * self.weight[k]);
            }
            self.meter.level[band].store(level, Ordering::Relaxed);
            self.meter.reduction[band].store(reduction, Ordering::Relaxed);
        }
    }
}

/// `prefix[i]` is the sum of the first `i` values, so any window mean is two lookups.
fn accumulate(values: &[f32], prefix: &mut [f32]) {
    prefix[0] = 0.0;
    for (i, v) in values.iter().enumerate() {
        prefix[i + 1] = prefix[i] + v;
    }
}

fn window_mean(prefix: &[f32], k: usize, half: usize) -> f32 {
    let lo = k.saturating_sub(half);
    let hi = (k + half).min(BINS);
    (prefix[hi + 1] - prefix[lo]) / (hi + 1 - lo) as f32
}

/// Per-bin target gain reduction in dB for one channel, from its spectrum in dB.
fn detect(db: &[f32], prefix: &mut [f32], target: &mut [f32], s: &Settings) {
    // Narrow neighbourhoods only see off peaks that are narrower than themselves.
    let neighbourhood = 0.35 + (0.06 - 0.35) * s.selectivity;
    accumulate(db, prefix);
    for k in 0..=BINS {
        if db[k] < SILENCE_DB {
            target[k] = 0.0;
            continue;
        }
        let half = ((k as f32 * neighbourhood).round() as usize).max(3);
        let excess = db[k] - window_mean(prefix, k, half) - KNEE_DB;
        target[k] = (s.depth * DEPTH_GAIN * excess.max(0.0)).min(MAX_REDUCTION_DB);
    }

    // Sharpness: low values blur each cut over its neighbours.
    accumulate(target, prefix);
    let blur = 0.1 * (1.0 - s.sharpness);
    for k in 0..=BINS {
        let half = (k as f32 * blur).round() as usize;
        if half > 0 {
            target[k] = window_mean(prefix, k, half);
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

    /// White noise from a fixed-seed xorshift, roughly +-0.1.
    fn noise(n: usize) -> Vec<f32> {
        let mut s = 0x1234_5678u32;
        (0..n)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                (s as f32 / u32::MAX as f32 - 0.5) * 0.2
            })
            .collect()
    }

    /// Frequency of the FFT bin, so test tones sit exactly on a bin.
    fn bin_freq(bin: usize) -> f32 {
        bin as f32 * FS / FFT_SIZE as f32
    }

    fn run(settings: Settings, x: &[f32]) -> Vec<f32> {
        let mut sup = Suppressor::new();
        sup.configure(FS, 1);
        sup.settings = settings;
        x.iter().map(|&v| sup.process([v, 0.0]).0[0]).collect()
    }

    /// Mean power around `f` over several nearby frequencies (single Goertzel bins of noise are noisy).
    fn power_near(x: &[f32], f: f32) -> f32 {
        let goertzel = |f: f32| {
            let w = 2.0 * PI * f / FS;
            let (mut re, mut im) = (0.0, 0.0);
            for (i, v) in x.iter().enumerate() {
                re += v * (w * i as f32).cos();
                im += v * (w * i as f32).sin();
            }
            (re * re + im * im) / (x.len() * x.len()) as f32
        };
        (-4..=4).map(|d| goertzel(f + d as f32 * 37.0)).sum::<f32>() / 9.0
    }

    fn db(ratio: f32) -> f32 {
        10.0 * ratio.log10()
    }

    #[test]
    fn zero_depth_is_a_pure_delay() {
        let x: Vec<f32> = noise(20000);
        let y = run(Settings { depth: 0.0, ..Settings::default() }, &x);
        let worst = (LATENCY..x.len()).map(|i| (y[i] - x[i - LATENCY]).abs()).fold(0.0, f32::max);
        assert!(worst < 1e-4, "reconstruction error {worst}");
    }

    #[test]
    fn resonance_is_pulled_down_and_the_rest_is_left_alone() {
        let tone = bin_freq(43);
        let x: Vec<f32> = noise(48000)
            .into_iter()
            .enumerate()
            .map(|(i, n)| n + 0.2 * (2.0 * PI * tone * i as f32 / FS).sin())
            .collect();
        let y = run(Settings { depth: 1.0, ..Settings::default() }, &x);
        let (xin, yout) = (&x[16000..40000], &y[16000 + LATENCY..40000 + LATENCY]);

        let tone_change = db(power_near(yout, tone) / power_near(xin, tone));
        let rest_change = db(power_near(yout, 6000.0) / power_near(xin, 6000.0));
        println!("tone {tone_change:.1} dB, rest {rest_change:.1} dB");
        assert!(tone_change < -10.0, "resonance only moved {tone_change} dB");
        assert!(rest_change > -2.5, "broadband content dropped {rest_change} dB");
    }

    #[test]
    fn depth_scales_the_cut() {
        let tone = bin_freq(60);
        let x: Vec<f32> = noise(48000)
            .into_iter()
            .enumerate()
            .map(|(i, n)| n + 0.2 * (2.0 * PI * tone * i as f32 / FS).sin())
            .collect();
        let cut = |depth| {
            let y = run(Settings { depth, ..Settings::default() }, &x);
            db(power_near(&y[16000 + LATENCY..40000 + LATENCY], tone) / power_near(&x[16000..40000], tone))
        };
        let (light, heavy) = (cut(0.3), cut(0.9));
        println!("depth 0.3: {light:.1} dB, depth 0.9: {heavy:.1} dB");
        assert!(heavy < light - 3.0);
    }

    #[test]
    fn focus_range_protects_what_is_outside() {
        let tone = bin_freq(43);
        let x: Vec<f32> = noise(48000)
            .into_iter()
            .enumerate()
            .map(|(i, n)| n + 0.2 * (2.0 * PI * tone * i as f32 / FS).sin())
            .collect();
        let focus = Settings { depth: 1.0, low_hz: 4000.0, high_hz: 20000.0, ..Settings::default() };
        let y = run(focus, &x);
        let change = db(power_near(&y[16000 + LATENCY..40000 + LATENCY], tone) / power_near(&x[16000..40000], tone));
        assert!(change > -1.0, "tone below the focus range moved {change} dB");
    }

    #[test]
    fn stereo_stays_finite_and_meters_report() {
        let mut sup = Suppressor::new();
        sup.configure(FS, 2);
        sup.settings = Settings { depth: 1.0, ..Settings::default() };
        let (l, r) = (noise(30000), noise(30000).into_iter().rev().collect::<Vec<_>>());
        for i in 0..30000 {
            let (wet, dry) = sup.process([l[i] + 0.5 * (i as f32 * 0.07).sin(), r[i]]);
            assert!(wet.iter().chain(dry.iter()).all(|v| v.is_finite() && v.abs() < 4.0));
        }
        let reduction: f32 = sup.meter.reduction.iter().map(|r| r.load(Ordering::Relaxed)).fold(0.0, f32::max);
        let level: f32 = sup.meter.level.iter().map(|r| r.load(Ordering::Relaxed)).fold(-120.0, f32::max);
        assert!(reduction > 3.0, "meter shows no reduction ({reduction})");
        assert!(level > -30.0 && level < 0.0, "meter level {level}");
    }
}
