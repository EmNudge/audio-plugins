//! Exciter DSP: per-band highpass -> 2x oversampled saturation -> highpass, plus a spectrum analyser.

use nih_plug::prelude::AtomicF32;
use std::f32::consts::PI;
use std::sync::atomic::Ordering;
use std::sync::Arc;

pub const BANDS: usize = 64;
/// Lower and upper edge of the analysed range, log spaced across `BANDS`.
pub const F_MIN: f32 = 500.0;
pub const F_MAX: f32 = 20000.0;

const FFT_SIZE: usize = 1024;
const HOP: usize = 512;
const TAPS: usize = 33;
/// Base-rate latency of the up/down resampling pair. The dry path is delayed by this much.
pub const LATENCY: usize = (TAPS - 1) / 2;

/// Linear magnitude per band, written by the audio thread and read by the editor.
pub struct Spectrum {
    pub dry: [AtomicF32; BANDS],
    pub wet: [AtomicF32; BANDS],
}

impl Default for Spectrum {
    fn default() -> Self {
        Self {
            dry: std::array::from_fn(|_| AtomicF32::new(0.0)),
            wet: std::array::from_fn(|_| AtomicF32::new(0.0)),
        }
    }
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

impl Biquad {
    /// Second-order Butterworth highpass.
    fn highpass(fs: f32, fc: f32) -> Self {
        let w0 = 2.0 * PI * fc.min(fs * 0.45) / fs;
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * std::f32::consts::FRAC_1_SQRT_2);
        let a0 = 1.0 + alpha;
        Self {
            b0: (1.0 + cos) / 2.0 / a0,
            b1: -(1.0 + cos) / a0,
            b2: (1.0 + cos) / 2.0 / a0,
            a1: -2.0 * cos / a0,
            a2: (1.0 - alpha) / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// Blackman-windowed half-band lowpass, unity DC gain, cutoff at the base-rate Nyquist.
fn kernel() -> [f32; TAPS] {
    let mut h = [0.0; TAPS];
    let mid = (TAPS - 1) as f32 / 2.0;
    let n = (TAPS - 1) as f32;
    let mut sum = 0.0;
    for (j, tap) in h.iter_mut().enumerate() {
        let t = j as f32 - mid;
        let sinc = if t == 0.0 { 1.0 } else { (PI * 0.5 * t).sin() / (PI * 0.5 * t) };
        let w = 0.42 - 0.5 * (2.0 * PI * j as f32 / n).cos() + 0.08 * (4.0 * PI * j as f32 / n).cos();
        *tap = 0.5 * sinc * w;
        sum += *tap;
    }
    for tap in &mut h {
        *tap /= sum;
    }
    h
}

/// Asymmetric soft clipper. The bias makes even harmonics as well as odd ones; the DC it
/// introduces is removed by the highpass that follows.
fn saturate(x: f32) -> f32 {
    const BIAS: f32 = 0.3;
    const DRIVE: f32 = 4.0;
    ((DRIVE * x + BIAS).tanh() - BIAS.tanh()) / DRIVE
}

/// One excitation band for one channel.
#[derive(Clone)]
pub struct Chain {
    hp_in: Biquad,
    hp_out: Biquad,
    h: [f32; TAPS],
    up_hist: [f32; 32],
    up_pos: usize,
    down_hist: [f32; 64],
    down_pos: usize,
}

impl Chain {
    pub fn new(fs: f32, fc: f32) -> Self {
        Self {
            hp_in: Biquad::highpass(fs, fc),
            hp_out: Biquad::highpass(fs, fc),
            h: kernel(),
            up_hist: [0.0; 32],
            up_pos: 0,
            down_hist: [0.0; 64],
            down_pos: 0,
        }
    }

    pub fn process(&mut self, x: f32) -> f32 {
        let x = self.hp_in.process(x);

        // 2x upsample (polyphase): even and odd output phases use even and odd taps.
        self.up_pos = (self.up_pos + 1) & 31;
        self.up_hist[self.up_pos] = x;
        let (mut even, mut odd) = (0.0, 0.0);
        for k in 0..=(TAPS - 1) / 2 {
            let v = self.up_hist[self.up_pos.wrapping_sub(k) & 31];
            even += self.h[2 * k] * v;
            if 2 * k + 1 < TAPS {
                odd += self.h[2 * k + 1] * v;
            }
        }

        for s in [even * 2.0, odd * 2.0] {
            self.down_pos = (self.down_pos + 1) & 63;
            self.down_hist[self.down_pos] = saturate(s);
        }

        // Decimate: lowpass at the oversampled rate, keep the latest sample.
        let mut y = 0.0;
        for (j, tap) in self.h.iter().enumerate() {
            y += tap * self.down_hist[self.down_pos.wrapping_sub(j) & 63];
        }
        self.hp_out.process(y)
    }
}

/// Windowed FFT of the last `FFT_SIZE` samples of the dry and wet signals, folded into log bands.
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
    pub spectrum: Arc<Spectrum>,
}

impl Analyzer {
    pub fn new() -> Self {
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
            spectrum: Arc::new(Spectrum::default()),
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
            (&self.dry_ring, &self.spectrum.dry)
        } else {
            (&self.wet_ring, &self.spectrum.wet)
        };
        for i in 0..FFT_SIZE {
            self.re[i] = ring[(self.pos + i) % FFT_SIZE] * self.window[i];
            self.im[i] = 0.0;
        }
        fft(&mut self.re, &mut self.im, &self.tw_re, &self.tw_im);
        let scale = 4.0 / FFT_SIZE as f32;
        for (band, &(lo, hi)) in self.ranges.iter().enumerate() {
            let mut peak = 0.0f32;
            for bin in lo..=hi {
                peak = peak.max((self.re[bin] * self.re[bin] + self.im[bin] * self.im[bin]).sqrt());
            }
            out[band].store(peak * scale, Ordering::Relaxed);
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

    fn sine(fs: f32, f: f32, amp: f32, n: usize) -> Vec<f32> {
        (0..n).map(|i| amp * (2.0 * PI * f * i as f32 / fs).sin()).collect()
    }

    #[test]
    fn chain_is_finite_and_bounded() {
        let fs = 48000.0;
        let mut chain = Chain::new(fs, 3500.0);
        let out: Vec<f32> = sine(fs, 6000.0, 0.3, 48000).into_iter().map(|x| chain.process(x)).collect();
        assert!(out.iter().all(|v| v.is_finite() && v.abs() < 2.0));
        let rms = (out[24000..].iter().map(|v| v * v).sum::<f32>() / 24000.0).sqrt();
        assert!(rms > 0.01, "wet signal should not be silent, rms={rms}");
    }

    #[test]
    fn chain_ignores_low_frequencies() {
        let fs = 48000.0;
        let mut chain = Chain::new(fs, 9000.0);
        let out: Vec<f32> = sine(fs, 200.0, 0.5, 48000).into_iter().map(|x| chain.process(x)).collect();
        let rms = (out[24000..].iter().map(|v| v * v).sum::<f32>() / 24000.0).sqrt();
        assert!(rms < 0.01, "bass should not leak into the air band, rms={rms}");
    }

    #[test]
    fn analyzer_finds_a_tone() {
        let fs = 48000.0;
        let mut a = Analyzer::new();
        a.configure(fs);
        for x in sine(fs, 6000.0, 0.5, 8192) {
            a.push(x, 0.0);
        }
        let mags: Vec<f32> = a.spectrum.dry.iter().map(|m| m.load(Ordering::Relaxed)).collect();
        let peak = mags.iter().cloned().enumerate().max_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
        let expected = ((6000.0f32 / F_MIN).ln() / (F_MAX / F_MIN).ln() * BANDS as f32) as usize;
        assert!(peak.0.abs_diff(expected) <= 1, "peak band {} expected {}", peak.0, expected);
        assert!((peak.1 - 0.5).abs() < 0.1, "tone amplitude read as {}", peak.1);
    }
}
