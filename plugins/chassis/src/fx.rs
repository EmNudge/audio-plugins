//! The effects that sit in the three bands: the sub-octave, the vocoder, the frequency shifter,
//! the comb, the saturator, the cavity, the shine (a granular shifter) and the plate. Also the
//! filter bank behind the editor's spectrum.

use nih_plug::prelude::AtomicF32;
use std::f32::consts::{LN_2, PI, TAU};
use std::sync::atomic::Ordering;

use crate::filters::{coef_ms, Biquad, Cascade, Coeffs, MAX_CHANNELS};

/// A power-of-two ring buffer with fractional taps.
pub struct Delay {
    buf: Vec<f32>,
    mask: usize,
    pos: usize,
}

impl Delay {
    pub fn new(min_len: usize) -> Self {
        let n = min_len.next_power_of_two().max(16);
        Self { buf: vec![0.0; n], mask: n - 1, pos: 0 }
    }

    pub fn clear(&mut self) {
        self.buf.fill(0.0);
    }

    #[inline]
    pub fn push(&mut self, x: f32) {
        self.buf[self.pos] = x;
        self.pos = (self.pos + 1) & self.mask;
    }

    /// The sample pushed `k` pushes ago (1 is the newest).
    #[inline]
    pub fn at(&self, k: usize) -> f32 {
        self.buf[self.pos.wrapping_sub(k) & self.mask]
    }

    #[inline]
    pub fn tap_linear(&self, d: f32) -> f32 {
        let d = d.clamp(1.0, (self.mask - 2) as f32);
        let i = d as usize;
        let f = d - i as f32;
        self.at(i) * (1.0 - f) + self.at(i + 1) * f
    }

    /// Fractional read, `d` samples back, with 4-point Hermite interpolation.
    #[inline]
    pub fn tap(&self, d: f32) -> f32 {
        let d = d.clamp(2.0, (self.mask - 3) as f32);
        let i = d as usize;
        let t = 1.0 - (d - i as f32);
        let (y0, y1, y2, y3) = (self.at(i + 2), self.at(i + 1), self.at(i), self.at(i - 1));
        let c1 = 0.5 * (y2 - y0);
        let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
        let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
        ((c3 * t + c2) * t + c1) * t + y1
    }
}

/// White noise from a xorshift generator, in -1..1.
#[derive(Clone, Copy)]
pub struct Noise(u32);

impl Noise {
    pub fn new(seed: u32) -> Self {
        Self(seed.max(1))
    }

    #[inline]
    pub fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0 as f32 / u32::MAX as f32 * 2.0 - 1.0
    }
}

/// An octave-down generator for the low band: a flip-flop that toggles on every other rising
/// zero crossing of the low band's fundamental (found with a steep low-pass and a Schmitt trigger
/// whose threshold follows the level), smoothed to a near sine and scaled by the band's own
/// envelope, so the sub breathes with the voice.
pub struct Sub {
    pre_c: Coeffs,
    pre_s: Cascade,
    post_c: Coeffs,
    post_s: Cascade,
    peak: f32,
    peak_release: f32,
    armed: bool,
    flip: f32,
    env: f32,
    env_attack: f32,
    env_release: f32,
}

impl Sub {
    pub fn new(fs: f32) -> Self {
        Self {
            pre_c: Coeffs::lowpass(fs, 140.0, std::f64::consts::FRAC_1_SQRT_2),
            pre_s: Cascade::default(),
            post_c: Coeffs::lowpass(fs, 105.0, std::f64::consts::FRAC_1_SQRT_2),
            post_s: Cascade::default(),
            peak: 0.0,
            peak_release: coef_ms(80.0, fs),
            armed: false,
            flip: 1.0,
            env: 0.0,
            env_attack: coef_ms(4.0, fs),
            env_release: coef_ms(45.0, fs),
        }
    }

    pub fn reset(&mut self) {
        self.pre_s = Cascade::default();
        self.post_s = Cascade::default();
        self.peak = 0.0;
        self.armed = false;
        self.flip = 1.0;
        self.env = 0.0;
    }

    pub fn process(&mut self, x: f32) -> f32 {
        let l = self.pre_s.process(&self.pre_c, x);
        let a = l.abs();
        self.peak = if a > self.peak { a } else { self.peak + (a - self.peak) * self.peak_release };
        if self.peak > 1e-4 {
            let hi = 0.15 * self.peak;
            if l < -hi {
                self.armed = true;
            } else if self.armed && l > hi {
                self.armed = false;
                self.flip = -self.flip;
            }
        }
        let sine = self.post_s.process(&self.post_c, self.flip);
        let a = x.abs();
        let coef = if a > self.env { self.env_attack } else { self.env_release };
        self.env += (a - self.env) * coef;
        sine * self.env * 0.9
    }
}

fn poly_blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let x = t / dt;
        2.0 * x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + 2.0 * x + 1.0
    } else {
        0.0
    }
}

/// A band-limited saw to square blend.
fn wave(phase: f32, dt: f32, square: f32) -> f32 {
    let saw = 2.0 * phase - 1.0 - poly_blep(phase, dt);
    let sq = if phase < 0.5 { 1.0 } else { -1.0 } + poly_blep(phase, dt) - poly_blep((phase + 0.5).fract(), dt);
    saw + (sq - saw) * square
}

pub const VOC_BANDS: usize = 16;
const VOC_LO_HZ: f32 = 180.0;
const VOC_HI_HZ: f32 = 4800.0;
/// Sixteen overlapping bands, each narrower than the voice's own: measured on noise, the sum
/// comes out about 5 dB under the modulator without this.
const VOC_TRIM: f32 = 1.0;

pub fn vocoder_band_hz(b: usize) -> f32 {
    VOC_LO_HZ * (VOC_HI_HZ / VOC_LO_HZ).powf(b as f32 / (VOC_BANDS - 1) as f32)
}

/// A 16-band channel vocoder with a built-in carrier: two detuned band-limited oscillators
/// (saw to square) at the voice's pitch or a fixed note, and noise where the voice is unvoiced.
///
/// Each carrier band is divided by its own smoothed level before the modulator's envelope is
/// applied, so the output follows the voice's spectrum exactly whatever the carrier's pitch or
/// shape, and the level needs no tuning.
pub struct Vocoder {
    fs: f32,
    analysis: [Coeffs; VOC_BANDS],
    synthesis: [Coeffs; VOC_BANDS],
    analysis_s: [[Biquad; VOC_BANDS]; MAX_CHANNELS],
    synthesis_s: [Cascade; VOC_BANDS],
    mod_env: [[f32; VOC_BANDS]; MAX_CHANNELS],
    car_env: [f32; VOC_BANDS],
    attack: f32,
    release: f32,
    car_coef: f32,
    phase_a: f32,
    phase_b: f32,
    freq: f32,
    freq_coef: f32,
    voiced: f32,
    voiced_coef: f32,
    noise: Noise,
}

impl Vocoder {
    pub fn new(fs: f32) -> Self {
        let q = 4.5;
        Self {
            fs,
            analysis: std::array::from_fn(|b| Coeffs::bandpass(fs, vocoder_band_hz(b), q)),
            synthesis: std::array::from_fn(|b| Coeffs::bandpass(fs, vocoder_band_hz(b), q)),
            analysis_s: [[Biquad::default(); VOC_BANDS]; MAX_CHANNELS],
            synthesis_s: [Cascade::default(); VOC_BANDS],
            mod_env: [[0.0; VOC_BANDS]; MAX_CHANNELS],
            car_env: [0.0; VOC_BANDS],
            attack: coef_ms(2.0, fs),
            release: coef_ms(14.0, fs),
            car_coef: coef_ms(20.0, fs),
            phase_a: 0.0,
            phase_b: 0.3,
            freq: 110.0,
            freq_coef: coef_ms(7.0, fs),
            voiced: 0.0,
            voiced_coef: coef_ms(8.0, fs),
            noise: Noise::new(0x9e3779b9),
        }
    }

    pub fn reset(&mut self) {
        self.analysis_s = [[Biquad::default(); VOC_BANDS]; MAX_CHANNELS];
        self.synthesis_s = [Cascade::default(); VOC_BANDS];
        self.mod_env = [[0.0; VOC_BANDS]; MAX_CHANNELS];
        self.car_env = [0.0; VOC_BANDS];
        self.voiced = 0.0;
    }

    /// The carrier's frequency right now, for the editor.
    pub fn carrier_hz(&self) -> f32 {
        self.freq
    }

    /// `drone` keeps the tone going where the voice is unvoiced (a little noise joins it);
    /// otherwise the tone gives way to noise there.
    pub fn process(&mut self, x: [f32; MAX_CHANNELS], channels: usize, carrier_hz: f32, voiced: bool, drone: bool, square: f32) -> [f32; MAX_CHANNELS] {
        self.freq += (carrier_hz.clamp(40.0, 1200.0) - self.freq) * self.freq_coef;
        self.voiced += ((voiced as u8 as f32) - self.voiced) * self.voiced_coef;

        let dt_a = self.freq / self.fs;
        let dt_b = dt_a * 1.006;
        self.phase_a = (self.phase_a + dt_a).fract();
        self.phase_b = (self.phase_b + dt_b).fract();
        let tone = 0.5 * (wave(self.phase_a, dt_a, square) + wave(self.phase_b, dt_b, square));
        let hiss = (1.0 - self.voiced) * self.noise.next();
        let carrier = if drone { tone + 0.3 * hiss } else { self.voiced * tone + 0.6 * hiss };

        let mut bands = [0.0f32; VOC_BANDS];
        for b in 0..VOC_BANDS {
            let v = self.synthesis_s[b].process(&self.synthesis[b], carrier);
            self.car_env[b] += (v.abs() - self.car_env[b]) * self.car_coef;
            bands[b] = v / self.car_env[b].max(1e-3);
        }

        let mut out = [0.0; MAX_CHANNELS];
        for ch in 0..channels {
            let mut sum = 0.0;
            for b in 0..VOC_BANDS {
                let m = self.analysis_s[ch][b].process(&self.analysis[b], x[ch]).abs();
                let env = &mut self.mod_env[ch][b];
                *env += (m - *env) * if m > *env { self.attack } else { self.release };
                sum += bands[b] * *env;
            }
            out[ch] = sum * VOC_TRIM;
        }
        out
    }
}

const HILBERT_POLES: [f32; 12] =
    [0.3609, 2.7412, 11.1573, 44.7581, 179.6242, 798.4578, 1.2524, 5.5671, 22.3423, 89.6271, 364.7914, 2770.1114];

pub fn hilbert_coefs(fs: f32) -> [f32; 12] {
    std::array::from_fn(|j| {
        let alpha = TAU * HILBERT_POLES[j] * 15.0;
        let k = alpha * 0.5 / fs;
        -(1.0 - k) / (1.0 + k)
    })
}

/// Single-sideband frequency shifter: two all-pass chains 90 degrees apart (the pole set from
/// Bernie Hutchins' Musical Engineer's Handbook, as used by Csound's `hilbert`), mixed with a
/// quadrature oscillator.
#[derive(Clone, Copy, Default)]
pub struct FreqShifter {
    x1: [f32; 12],
    y1: [f32; 12],
}

impl FreqShifter {
    /// Shift up by the oscillator's frequency, whose phase is `(sin, cos)`.
    #[inline]
    pub fn process(&mut self, x: f32, coefs: &[f32; 12], sin: f32, cos: f32) -> f32 {
        let mut a = x;
        for j in 0..6 {
            let y = coefs[j] * (a - self.y1[j]) + self.x1[j];
            self.x1[j] = a;
            self.y1[j] = y;
            a = y;
        }
        let mut b = x;
        for j in 6..12 {
            let y = coefs[j] * (b - self.y1[j]) + self.x1[j];
            self.x1[j] = b;
            self.y1[j] = y;
            b = y;
        }
        a * cos + b * sin
    }
}

/// A feedback comb with a little damping in the loop: static ringing at one pitch.
pub struct Comb {
    line: Delay,
    damp: f32,
}

impl Comb {
    pub fn new(fs: f32) -> Self {
        Self { line: Delay::new((fs / 40.0) as usize + 8), damp: 0.0 }
    }

    pub fn clear(&mut self) {
        self.line.clear();
        self.damp = 0.0;
    }

    pub fn process(&mut self, x: f32, delay: f32, feedback: f32) -> f32 {
        let past = self.line.tap(delay);
        self.damp += (past - self.damp) * 0.6;
        let y = x + feedback * self.damp;
        self.line.push(y);
        y
    }
}

fn ln_cosh(x: f32) -> f32 {
    let a = x.abs();
    a + (-2.0 * a).exp().ln_1p() - LN_2
}

/// Tape-ish saturation: `tanh` with a touch of bias for even harmonics and first-order
/// antiderivative antialiasing, so it needs no oversampling and adds no latency.
pub struct Saturator {
    x1: f32,
    f1: f32,
    dc: f32,
    dc_coef: f32,
}

impl Saturator {
    pub fn new(fs: f32) -> Self {
        Self { x1: 0.0, f1: 0.0, dc: 0.0, dc_coef: 1.0 - (-TAU * 12.0 / fs).exp() }
    }

    pub fn reset(&mut self) {
        self.x1 = 0.0;
        self.f1 = 0.0;
        self.dc = 0.0;
    }

    pub fn process(&mut self, x: f32, grit: f32) -> f32 {
        if grit < 1e-3 {
            self.x1 = 0.0;
            self.f1 = 0.0;
            return x;
        }
        let g = 1.0 + 8.0 * grit;
        let bias = 0.12 * grit;
        let u = x * g + bias;
        let f = ln_cosh(u);
        let d = u - self.x1;
        let y = if d.abs() < 1e-4 { (0.5 * (u + self.x1)).tanh() } else { (f - self.f1) / d };
        self.x1 = u;
        self.f1 = f;
        let wet = (y - bias.tanh()) / g.powf(0.75);
        let wet = wet - self.dc;
        self.dc += wet * self.dc_coef;
        let blend = (grit * 3.0).min(1.0);
        x + (wet - x) * blend
    }
}

/// A short, slowly moving delay with no feedback: the sound of the inside of a metal head.
pub struct Cavity {
    fs: f32,
    lines: [Delay; MAX_CHANNELS],
    phase: f32,
}

impl Cavity {
    pub fn new(fs: f32) -> Self {
        let len = (fs * 0.06) as usize;
        Self { fs, lines: [Delay::new(len), Delay::new(len)], phase: 0.0 }
    }

    pub fn reset(&mut self) {
        for l in &mut self.lines {
            l.clear();
        }
    }

    pub fn process(&mut self, x: [f32; MAX_CHANNELS], channels: usize, ms: f32, amount: f32) -> [f32; MAX_CHANNELS] {
        self.phase = (self.phase + 0.35 / self.fs).fract();
        let base = ms * 0.001 * self.fs;
        let wobble = 0.00025 * self.fs;
        let mut out = x;
        for ch in 0..channels {
            let spread = if ch == 0 { 1.0 } else { 1.17 };
            let lfo = (TAU * (self.phase + 0.25 * ch as f32)).sin();
            self.lines[ch].push(x[ch]);
            let tap = self.lines[ch].tap(base * spread + wobble * lfo);
            out[ch] = x[ch] + amount * 0.9 * tap;
        }
        out
    }
}

const SHINE_GRAIN_S: f32 = 0.028;

/// Two-tap granular pitch shifter for the top band. The taps read a short buffer at delays that
/// sweep through `0..grain` at `1 - ratio` samples per sample and fade with `sin²`, so the two
/// always sum to one. On the high band the grains' seams are the metallic sheen.
pub struct Shine {
    buf: Vec<f32>,
    w: usize,
    phase: f32,
    grain: f32,
}

impl Shine {
    pub fn new(fs: f32) -> Self {
        let grain = (SHINE_GRAIN_S * fs).round();
        Self { buf: vec![0.0; grain as usize + 4], w: 0, phase: 0.0, grain }
    }

    pub fn reset(&mut self) {
        self.buf.fill(0.0);
    }

    pub fn process(&mut self, x: f32, ratio: f32) -> f32 {
        let len = self.buf.len();
        self.buf[self.w] = x;
        self.phase = (self.phase + (1.0 - ratio) / self.grain).rem_euclid(1.0);
        let mut y = 0.0;
        for k in 0..2 {
            let p = (self.phase + 0.5 * k as f32).fract();
            let d = p * self.grain;
            let i = d.floor();
            let f = d - i;
            let a = self.buf[(self.w + len - i as usize) % len];
            let b = self.buf[(self.w + len - i as usize - 1) % len];
            y += (PI * p).sin().powi(2) * (a + (b - a) * f);
        }
        self.w = (self.w + 1) % len;
        y
    }
}

struct Allpass {
    buf: Vec<f32>,
    i: usize,
}

impl Allpass {
    fn new(len: usize) -> Self {
        Self { buf: vec![0.0; len.max(1)], i: 0 }
    }

    fn process(&mut self, x: f32, g: f32) -> f32 {
        let d = self.buf[self.i];
        let w = x + g * d;
        self.buf[self.i] = w;
        self.i = (self.i + 1) % self.buf.len();
        d - g * w
    }
}

const PLATE_MS: [f32; 4] = [11.4, 14.6, 17.9, 22.1];
const PLATE_PREDELAY_MS: f32 = 4.0;

/// A small metal plate: a short pre-delay, two allpass diffusers and four short, bright delay
/// lines mixed through a Hadamard matrix. Four lines in under 25 ms keep the echoes sparse and
/// ringing, which is what makes it sound like sheet metal and not a room.
pub struct Plate {
    fs: f32,
    pre: Delay,
    diffusers: [Allpass; 2],
    lines: [Delay; 4],
    lengths: [usize; 4],
    damp: [f32; 4],
    damp_coef: f32,
}

impl Plate {
    pub fn new(fs: f32) -> Self {
        let lengths: [usize; 4] = std::array::from_fn(|i| ((PLATE_MS[i] * 0.001 * fs) as usize) | 1);
        Self {
            fs,
            pre: Delay::new((fs * 0.01) as usize),
            diffusers: [Allpass::new((fs * 0.0031) as usize | 1), Allpass::new((fs * 0.0047) as usize | 1)],
            lines: std::array::from_fn(|i| Delay::new(lengths[i] + 8)),
            lengths,
            damp: [0.0; 4],
            damp_coef: 1.0 - (-TAU * 9000.0 / fs).exp(),
        }
    }

    pub fn reset(&mut self) {
        self.pre.clear();
        for l in &mut self.lines {
            l.clear();
        }
        for a in &mut self.diffusers {
            a.buf.fill(0.0);
        }
        self.damp = [0.0; 4];
    }

    pub fn process(&mut self, x: f32, decay_s: f32) -> [f32; 2] {
        self.pre.push(x);
        let mut d = self.pre.tap_linear(PLATE_PREDELAY_MS * 0.001 * self.fs);
        d = self.diffusers[0].process(d, 0.6);
        d = self.diffusers[1].process(d, 0.6);

        let r: [f32; 4] = std::array::from_fn(|i| self.lines[i].at(self.lengths[i]));
        let h = [r[0] + r[1] + r[2] + r[3], r[0] - r[1] + r[2] - r[3], r[0] + r[1] - r[2] - r[3], r[0] - r[1] - r[2] + r[3]];
        for i in 0..4 {
            let g = 10f32.powf(-3.0 * self.lengths[i] as f32 / (self.fs * decay_s.max(0.05)));
            self.damp[i] += (h[i] * 0.5 * g - self.damp[i]) * self.damp_coef;
            self.lines[i].push(d * 0.5 + self.damp[i]);
        }
        [0.35 * (r[0] - r[1] + r[2] - r[3]), 0.35 * (r[0] + r[1] - r[2] - r[3])]
    }

    /// How long the plate rings after the input stops, in samples.
    pub fn tail(&self, decay_s: f32) -> u32 {
        (decay_s * 1.1 * self.fs) as u32 + (0.02 * self.fs) as u32
    }
}

pub const BINS: usize = 40;
pub const BIN_LO_HZ: f32 = 60.0;
pub const BIN_HI_HZ: f32 = 14000.0;

pub fn bin_hz(i: usize) -> f32 {
    BIN_LO_HZ * (BIN_HI_HZ / BIN_LO_HZ).powf(i as f32 / (BINS - 1) as f32)
}

/// A bank of narrow band-pass filters with envelope followers: the editor's spectrum.
pub struct Spectrum {
    coeffs: [Coeffs; BINS],
    state: [Biquad; BINS],
    env: [f32; BINS],
    peak: [f32; BINS],
    attack: f32,
    release: f32,
}

impl Spectrum {
    pub fn new(fs: f32) -> Self {
        Self {
            coeffs: std::array::from_fn(|i| Coeffs::bandpass(fs, bin_hz(i), 7.0)),
            state: [Biquad::default(); BINS],
            env: [0.0; BINS],
            peak: [0.0; BINS],
            attack: coef_ms(1.5, fs),
            release: coef_ms(25.0, fs),
        }
    }

    pub fn reset(&mut self) {
        self.state = [Biquad::default(); BINS];
        self.env = [0.0; BINS];
        self.peak = [0.0; BINS];
    }

    #[inline]
    pub fn push(&mut self, x: f32) {
        for i in 0..BINS {
            let y = self.state[i].process(&self.coeffs[i], x).abs();
            let coef = if y > self.env[i] { self.attack } else { self.release };
            self.env[i] += (y - self.env[i]) * coef;
            self.peak[i] = self.peak[i].max(self.env[i]);
        }
    }

    /// Hand the loudest each bin got since the last call to the editor. The mean of a rectified
    /// sine is 2/pi of its peak, so the scale puts a sine at a bin's centre at its own amplitude.
    pub fn publish(&mut self, out: &[AtomicF32; BINS]) {
        for i in 0..BINS {
            out[i].store(self.peak[i] * 1.57, Ordering::Relaxed);
            self.peak[i] = self.env[i];
        }
    }
}
