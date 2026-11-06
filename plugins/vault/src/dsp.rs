//! Vault: an algorithmic reverb.
//!
//! The signal path is: pre-delay, low and high cut, then two parallel paths.
//!
//! * Early reflections: ten taps per channel read from a delay line, spread over the first tens of
//!   milliseconds and scaled with the room size. They go straight to the output.
//! * The late field: four Schroeder all-passes per channel smear the input, which is injected into
//!   an 8-line feedback delay network (Householder matrix, mutually inharmonic line lengths). Each
//!   line is read with a Hermite interpolator at a position wandered by its own slow sine, which
//!   removes the metallic ring of a fixed network. Inside each loop a three-band decay filter sets
//!   the reverb time separately for lows, mids and highs: the signal
//!   is split with two 12 dB/octave crossovers (pairs of one-poles) whose bands sum back to the
//!   input, so equal gains are exactly flat. Gains are `10^(-3 * delay / RT60)` per band, which makes every band decay at the
//!   asked rate whatever the line length. An optional single-sideband frequency shifter (a
//!   12-pole Hilbert all-pass network) sits in the loop, so the tail
//!   spirals up or down by a fixed number of Hz on every pass.
//!
//! Freeze mutes the input and sets every loop gain to one. Ducking lowers the wet signal while
//! the input is loud. Delay line lengths and the pre-delay are slewed, so moving Size or switching
//! space glides rather than clicks.
//!
//! The audio thread also publishes a history of input and wet levels split into low, mid and high,
//! and a 20-band spectrum of the wet signal, for the editor.

use nih_plug::prelude::{AtomicF32, Enum};
use std::f32::consts::{LN_10, TAU};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

pub const MAX_CHANNELS: usize = 2;
pub const LINES: usize = 8;

/// Telemetry columns: the editor shows the last `COLS` of them, each `COL_MS` long.
pub const COLS: usize = 96;
pub const COL_MS: f32 = 50.0;
pub const BANDS: usize = 20;
pub const BAND_LO_HZ: f32 = 50.0;
pub const BAND_HI_HZ: f32 = 14000.0;
/// Bands below this index are "low", up to `MID_END` "mid", the rest "high".
const LOW_END: usize = 8;
const MID_END: usize = 14;

pub const MIN_DECAY_S: f32 = 0.2;
pub const MAX_DECAY_S: f32 = 40.0;

/// Base delay line lengths in milliseconds at size 1: mutually inharmonic.
const LINE_MS: [f32; LINES] = [29.7, 37.1, 41.1, 43.7, 53.3, 59.9, 67.7, 73.1];
/// Early reflection tap times in milliseconds at size 1, left and right.
const ER_MS: [[f32; 10]; 2] = [
    [5.3, 8.9, 13.1, 17.9, 23.7, 29.3, 36.1, 43.9, 52.3, 61.7],
    [6.7, 10.3, 15.1, 20.3, 26.9, 33.7, 40.9, 48.1, 57.7, 66.3],
];
const ER_SIGN: [f32; 10] = [1.0, -1.0, 1.0, 1.0, -1.0, 1.0, -1.0, -1.0, 1.0, -1.0];
/// Input all-pass lengths in samples at 48 kHz, left and right.
const AP_LEN: [[usize; 4]; 2] = [[113, 162, 241, 336], [127, 184, 253, 359]];
/// How the stereo input and output are spread across the eight lines.
const IN_MID: [f32; LINES] = [1.0, -1.0, 1.0, 1.0, -1.0, 1.0, -1.0, -1.0];
const SIGN_L: [f32; LINES] = [1.0, 1.0, -1.0, -1.0, 1.0, 1.0, -1.0, -1.0];
const SIGN_R: [f32; LINES] = [1.0, -1.0, -1.0, 1.0, 1.0, -1.0, -1.0, 1.0];

/// What kind of space the network models.
#[derive(Enum, Debug, PartialEq, Clone, Copy)]
pub enum Space {
    #[name = "Room"]
    Room,
    #[name = "Hall"]
    Hall,
    #[name = "Plate"]
    Plate,
    #[name = "Chamber"]
    Chamber,
    #[name = "Abyss"]
    Abyss,
}

struct Shape {
    /// Multiplier on the line lengths.
    line: f32,
    /// Multiplier on early reflection times; zero removes them.
    er: f32,
    /// Multiplier on modulation depth.
    modulation: f32,
    /// Added to the diffusion amount.
    diffusion: f32,
}

impl Space {
    fn shape(self) -> Shape {
        let (line, er, modulation, diffusion) = match self {
            Space::Room => (1.0, 1.0, 1.0, 0.0),
            Space::Hall => (1.7, 1.6, 1.0, 0.0),
            Space::Plate => (0.75, 0.0, 1.4, 0.2),
            Space::Chamber => (1.25, 1.2, 0.7, 0.1),
            Space::Abyss => (3.0, 2.0, 1.8, 0.1),
        };
        Shape { line, er, modulation, diffusion }
    }
}

/// The size knob (0..1) as a multiplier on every length: 0.3x to 2.4x.
pub fn size_scale(size: f32) -> f32 {
    0.3 * 8.0_f32.powf(size.clamp(0.0, 1.0))
}

/// Mean line length in seconds, which the decay curve needs.
pub fn mean_delay_s(space: Space, size: f32) -> f32 {
    LINE_MS.iter().sum::<f32>() / LINES as f32 * 0.001 * space.shape().line * size_scale(size)
}

/// Per-pass gain that makes a signal decay by 60 dB in `rt60` seconds through a delay of `tau`.
fn pass_gain(tau: f32, rt60: f32) -> f32 {
    (-3.0 * LN_10 * tau / rt60.max(0.02)).exp()
}

/// The reverb time at `freq`, as the loop filters really give it: the magnitude of the three-band
/// filter at that frequency, converted back from a per-pass gain. The editor draws this.
pub fn decay_at(freq: f32, tau: f32, decay: f32, low_mult: f32, high_mult: f32, xo_low: f32, xo_high: f32) -> f32 {
    // Each crossover is two one-poles in series: (1 / (1 + jx))^2.
    let lp = |fc: f32| {
        let x = freq / fc;
        let d = 1.0 + x * x;
        let (a, b) = (1.0 / d, -x / d);
        (a * a - b * b, 2.0 * a * b)
    };
    let (ll, lh) = (lp(xo_low), lp(xo_high));
    let (gl, gm, gh) = (pass_gain(tau, decay * low_mult), pass_gain(tau, decay), pass_gain(tau, decay * high_mult));
    let re = gl * ll.0 + gm * (lh.0 - ll.0) + gh * (1.0 - lh.0);
    let im = gl * ll.1 + gm * (lh.1 - ll.1) + gh * -lh.1;
    let mag = re.hypot(im).clamp(1e-6, 0.999_999);
    (-3.0 * LN_10 * tau / mag.ln()).clamp(0.01, 200.0)
}

/// What the audio thread publishes for the editor: a ring of per-column levels and the spectrum.
pub struct Meter {
    /// Number of columns written so far; column `n` lives in slot `n % COLS`.
    pub head: AtomicUsize,
    pub input: [AtomicF32; COLS],
    pub wet: [AtomicF32; COLS],
    /// RMS of the wet signal in the low, mid and high groups of bands, per column.
    pub low: [AtomicF32; COLS],
    pub mid: [AtomicF32; COLS],
    pub high: [AtomicF32; COLS],
    /// RMS of the wet signal in each band over the latest column.
    pub bands: [AtomicF32; BANDS],
    /// Gain applied by the ducker, 1 = none.
    pub duck: AtomicF32,
}

impl Meter {
    fn new() -> Self {
        let zeros = || std::array::from_fn(|_| AtomicF32::new(0.0));
        Self {
            head: AtomicUsize::new(0),
            input: zeros(),
            wet: zeros(),
            low: zeros(),
            mid: zeros(),
            high: zeros(),
            bands: std::array::from_fn(|_| AtomicF32::new(0.0)),
            duck: AtomicF32::new(1.0),
        }
    }
}

#[derive(Clone, Copy)]
pub struct Settings {
    pub space: Space,
    /// 0..1, mapped by `size_scale`.
    pub size: f32,
    pub predelay_ms: f32,
    /// Reverb time of the mids in seconds.
    pub decay_s: f32,
    pub low_mult: f32,
    pub high_mult: f32,
    pub xo_low: f32,
    pub xo_high: f32,
    pub diffusion: f32,
    pub mod_depth: f32,
    pub mod_rate: f32,
    /// Frequency shift of the tail in Hz per pass; zero turns the shifter off.
    pub shift_hz: f32,
    /// 0..1 level of the early reflections.
    pub early: f32,
    /// 0..1.5 stereo width of the wet signal.
    pub width: f32,
    /// 0..1 how far the wet signal ducks under the input.
    pub duck: f32,
    pub low_cut: f32,
    pub high_cut: f32,
    pub mix: f32,
    pub freeze: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            space: Space::Hall,
            size: 0.5,
            predelay_ms: 12.0,
            decay_s: 2.5,
            low_mult: 1.0,
            high_mult: 0.7,
            xo_low: 300.0,
            xo_high: 4000.0,
            diffusion: 0.7,
            mod_depth: 0.3,
            mod_rate: 0.6,
            shift_hz: 0.0,
            early: 0.5,
            width: 1.0,
            duck: 0.0,
            low_cut: 20.0,
            high_cut: 20000.0,
            mix: 0.3,
            freeze: false,
        }
    }
}

struct Delay {
    buf: Vec<f32>,
    mask: usize,
    pos: usize,
}

impl Delay {
    fn new(min_len: usize) -> Self {
        let n = min_len.next_power_of_two().max(16);
        Self { buf: vec![0.0; n], mask: n - 1, pos: 0 }
    }

    #[inline]
    fn push(&mut self, x: f32) {
        self.buf[self.pos] = x;
        self.pos = (self.pos + 1) & self.mask;
    }

    /// The sample pushed `k` pushes ago (1 is the newest).
    #[inline]
    fn at(&self, k: usize) -> f32 {
        self.buf[self.pos.wrapping_sub(k) & self.mask]
    }

    #[inline]
    fn tap_linear(&self, d: f32) -> f32 {
        let d = d.clamp(1.0, (self.mask - 2) as f32);
        let i = d as usize;
        let f = d - i as f32;
        self.at(i) * (1.0 - f) + self.at(i + 1) * f
    }

    /// Fractional read, `d` samples back, with 4-point Hermite interpolation.
    #[inline]
    fn tap(&self, d: f32) -> f32 {
        let d = d.clamp(2.0, (self.mask - 3) as f32);
        let i = d as usize;
        let t = 1.0 - (d - i as f32);
        let (y0, y1, y2, y3) = (self.at(i + 2), self.at(i + 1), self.at(i), self.at(i - 1));
        let c1 = 0.5 * (y2 - y0);
        let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
        let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
        ((c3 * t + c2) * t + c1) * t + y1
    }

    fn clear(&mut self) {
        self.buf.fill(0.0);
    }
}

struct Allpass {
    buf: Vec<f32>,
    pos: usize,
}

impl Allpass {
    fn new(len: usize) -> Self {
        Self { buf: vec![0.0; len.max(2)], pos: 0 }
    }

    #[inline]
    fn process(&mut self, x: f32, g: f32) -> f32 {
        let d = self.buf[self.pos];
        let w = x + g * d;
        self.buf[self.pos] = w;
        self.pos += 1;
        if self.pos == self.buf.len() {
            self.pos = 0;
        }
        d - g * w
    }
}

/// Single-sideband frequency shifter: two all-pass chains 90 degrees apart (the pole set from
/// Bernie Hutchins' Musical Engineer's Handbook, as used by Csound's `hilbert`), mixed with a
/// quadrature oscillator.
#[derive(Clone, Copy, Default)]
struct Shifter {
    x1: [f32; 12],
    y1: [f32; 12],
}

const HILBERT_POLES: [f32; 12] =
    [0.3609, 2.7412, 11.1573, 44.7581, 179.6242, 798.4578, 1.2524, 5.5671, 22.3423, 89.6271, 364.7914, 2770.1114];

fn hilbert_coefs(sample_rate: f32) -> [f32; 12] {
    std::array::from_fn(|j| {
        let alpha = TAU * HILBERT_POLES[j] * 15.0;
        let k = alpha * 0.5 / sample_rate;
        -(1.0 - k) / (1.0 + k)
    })
}

impl Shifter {
    /// Shift up by the oscillator's frequency, whose phase is `(cos, sin)`.
    #[inline]
    fn process(&mut self, x: f32, coefs: &[f32; 12], cos: f32, sin: f32) -> f32 {
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

/// One band of the analysis bank: a constant-peak band-pass.
#[derive(Clone, Copy, Default)]
struct Band {
    b0: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
    energy: f32,
}

impl Band {
    fn new(freq: f32, sample_rate: f32) -> Self {
        let q = 3.5;
        let w = TAU * freq / sample_rate;
        let alpha = w.sin() / (2.0 * q);
        let a0 = 1.0 + alpha;
        Self { b0: alpha / a0, a1: -2.0 * w.cos() / a0, a2: (1.0 - alpha) / a0, z1: 0.0, z2: 0.0, energy: 0.0 }
    }

    #[inline]
    fn process(&mut self, x: f32) {
        let y = self.b0 * x + self.z1;
        self.z1 = -self.a1 * y + self.z2;
        self.z2 = -self.b0 * x - self.a2 * y;
        self.energy += y * y;
    }
}

fn one_pole(freq: f32, sample_rate: f32) -> f32 {
    1.0 - (-TAU * freq / sample_rate).exp()
}

pub struct Reverb {
    pub meter: Arc<Meter>,
    sr: f32,
    channels: usize,
    first: bool,

    // Targets from the settings.
    space: Space,
    line_target: [f32; LINES],
    er_scale_target: f32,
    pre_target: f32,
    gain_target: [[f32; 3]; LINES],
    ap_gain: f32,
    mod_samples: f32,
    lfo_inc: [f32; LINES],
    xo_a: [f32; 2],
    inject: f32,
    shift_inc: f32,
    shifting: bool,
    cut_a: [f32; 2],
    cut_on: [bool; 2],
    mix_target: f32,
    early: f32,
    width: f32,
    duck: f32,
    freeze: bool,

    // State.
    line_len: [f32; LINES],
    er_scale: f32,
    pre_len: f32,
    gain: [[f32; 3]; LINES],
    mix: f32,
    freeze_amt: f32,
    pre: [Delay; 2],
    er: [Delay; 2],
    allpass: [[Allpass; 4]; 2],
    lines: [Delay; LINES],
    lfo: [f32; LINES],
    /// The two crossover filters, each two one-poles in series.
    lp: [[[f32; 2]; 2]; LINES],
    shifters: [Shifter; LINES],
    hilbert: [f32; 12],
    shift_phase: f32,
    cut_state: [[[f32; 2]; 2]; 2],
    env: f32,
    duck_gain: f32,

    // Analysis.
    bands: [Band; BANDS],
    col_len: usize,
    col_count: usize,
    col_input: f32,
    col_wet: f32,
}

impl Reverb {
    pub fn new() -> Self {
        let mut r = Self {
            meter: Arc::new(Meter::new()),
            sr: 48000.0,
            channels: 2,
            first: true,
            space: Space::Hall,
            line_target: [100.0; LINES],
            er_scale_target: 1.0,
            pre_target: 2.0,
            gain_target: [[0.9; 3]; LINES],
            ap_gain: 0.5,
            mod_samples: 0.0,
            lfo_inc: [0.0; LINES],
            xo_a: [0.01; 2],
            inject: 0.3,
            shift_inc: 0.0,
            shifting: false,
            cut_a: [0.0; 2],
            cut_on: [false; 2],
            mix_target: 0.0,
            early: 0.0,
            width: 1.0,
            duck: 0.0,
            freeze: false,
            line_len: [100.0; LINES],
            er_scale: 1.0,
            pre_len: 2.0,
            gain: [[0.9; 3]; LINES],
            mix: 0.0,
            freeze_amt: 0.0,
            pre: [Delay::new(16), Delay::new(16)],
            er: [Delay::new(16), Delay::new(16)],
            allpass: std::array::from_fn(|_| std::array::from_fn(|_| Allpass::new(2))),
            lines: std::array::from_fn(|_| Delay::new(16)),
            lfo: [0.0; LINES],
            lp: [[[0.0; 2]; 2]; LINES],
            shifters: [Shifter::default(); LINES],
            hilbert: [0.0; 12],
            shift_phase: 0.0,
            cut_state: [[[0.0; 2]; 2]; 2],
            env: 0.0,
            duck_gain: 1.0,
            bands: [Band::default(); BANDS],
            col_len: 1,
            col_count: 0,
            col_input: 0.0,
            col_wet: 0.0,
        };
        r.configure(48000.0, 2);
        r
    }

    /// Allocates for the sample rate. Not for the audio thread.
    pub fn configure(&mut self, sample_rate: f32, channels: usize) {
        self.sr = sample_rate;
        self.channels = channels.clamp(1, MAX_CHANNELS);
        let secs = |s: f32| (s * sample_rate) as usize + 8;
        self.pre = [Delay::new(secs(0.32)), Delay::new(secs(0.32))];
        self.er = [Delay::new(secs(0.5)), Delay::new(secs(0.5))];
        self.lines = std::array::from_fn(|_| Delay::new(secs(0.75)));
        let scale = sample_rate / 48000.0;
        self.allpass = std::array::from_fn(|c| std::array::from_fn(|k| Allpass::new((AP_LEN[c][k] as f32 * scale) as usize)));
        self.hilbert = hilbert_coefs(sample_rate);
        for (b, band) in self.bands.iter_mut().enumerate() {
            let f = BAND_LO_HZ * (BAND_HI_HZ / BAND_LO_HZ).powf(b as f32 / (BANDS - 1) as f32);
            *band = Band::new(f.min(sample_rate * 0.45), sample_rate);
        }
        self.col_len = ((COL_MS * 0.001 * sample_rate) as usize).max(1);
        self.first = true;
        self.reset();
    }

    pub fn reset(&mut self) {
        for d in self.pre.iter_mut().chain(self.er.iter_mut()).chain(self.lines.iter_mut()) {
            d.clear();
        }
        for ap in self.allpass.iter_mut().flatten() {
            ap.buf.fill(0.0);
            ap.pos = 0;
        }
        self.lp = [[[0.0; 2]; 2]; LINES];
        self.shifters = [Shifter::default(); LINES];
        self.cut_state = [[[0.0; 2]; 2]; 2];
        self.env = 0.0;
        self.freeze_amt = 0.0;
        for k in 0..LINES {
            self.lfo[k] = k as f32 * TAU / LINES as f32;
        }
        for b in self.bands.iter_mut() {
            b.z1 = 0.0;
            b.z2 = 0.0;
            b.energy = 0.0;
        }
        self.col_count = 0;
        self.col_input = 0.0;
        self.col_wet = 0.0;
    }

    /// How long the host should keep calling us after the input stops, in samples.
    pub fn tail_samples(&self, s: &Settings) -> u32 {
        (s.decay_s * s.low_mult.max(s.high_mult).max(1.0) * self.sr) as u32
    }

    /// Takes new settings. Call every few samples (the plugin calls it every 32): it only sets
    /// targets, and the per-sample code glides towards them.
    pub fn set(&mut self, s: &Settings) {
        let sr = self.sr;
        let shape = s.space.shape();
        let cap = (self.lines[0].mask - 8) as f32;
        let scale = size_scale(s.size);
        for i in 0..LINES {
            // Whole samples: an interpolated read is slightly lossy, and a frozen loop should not leak.
            self.line_target[i] = (LINE_MS[i] * 0.001 * sr * shape.line * scale).min(cap * 0.85).round();
        }
        self.space = s.space;
        self.er_scale_target = scale * shape.er;
        self.pre_target = (s.predelay_ms * 0.001 * sr).max(2.0);
        self.ap_gain = 0.78 * (s.diffusion + shape.diffusion).clamp(0.0, 1.0);

        // Decay gains per band, from each line's own length.
        let decay = s.decay_s.clamp(MIN_DECAY_S, MAX_DECAY_S);
        let mut mean_gain = 0.0;
        for i in 0..LINES {
            let tau = self.line_target[i] / sr;
            let g = [
                pass_gain(tau, decay * s.low_mult),
                pass_gain(tau, decay),
                pass_gain(tau, decay * s.high_mult),
            ];
            mean_gain += g[1] / LINES as f32;
            self.gain_target[i] = if s.freeze { [1.0; 3] } else { g };
        }
        // Steady-state level of the loop grows as 1/sqrt(1 - g^2); undo it so that Decay changes
        // the length of the tail, not how loud it is.
        self.inject = 0.5 * (1.0 - mean_gain * mean_gain).max(0.002).sqrt();

        let xo_high = s.xo_high.max(s.xo_low * 1.2);
        self.xo_a = [one_pole(s.xo_low, sr), one_pole(xo_high, sr)];

        let depth_ms = 2.0 * s.mod_depth * shape.modulation;
        self.mod_samples = depth_ms * 0.001 * sr;
        for i in 0..LINES {
            self.lfo_inc[i] = TAU * s.mod_rate * (0.85 + 0.045 * i as f32) / sr;
        }

        self.shifting = s.shift_hz.abs() > 0.02;
        self.shift_inc = TAU * s.shift_hz / sr;

        self.cut_on = [s.low_cut > 21.0, s.high_cut < 19500.0];
        self.cut_a = [one_pole(s.low_cut.max(1.0), sr), one_pole(s.high_cut.min(sr * 0.45), sr)];

        self.mix_target = s.mix.clamp(0.0, 1.0);
        self.early = s.early;
        self.width = s.width;
        self.duck = s.duck;
        self.freeze = s.freeze;

        if self.first {
            self.first = false;
            self.line_len = self.line_target;
            self.er_scale = self.er_scale_target;
            self.pre_len = self.pre_target;
            self.gain = self.gain_target;
            self.mix = self.mix_target;
            self.freeze_amt = if s.freeze { 1.0 } else { 0.0 };
        }
    }

    pub fn process(&mut self, x: [f32; MAX_CHANNELS]) -> [f32; MAX_CHANNELS] {
        let sr = self.sr;
        let (l, r) = if self.channels > 1 { (x[0], x[1]) } else { (x[0], x[0]) };
        let input = [l, r];

        // Smoothed controls.
        let slew = 1.0 - (-1.0 / (0.12 * sr)).exp();
        self.er_scale += (self.er_scale_target - self.er_scale) * slew;
        self.pre_len += (self.pre_target - self.pre_len) * slew;
        self.mix += (self.mix_target - self.mix) * 0.005;
        let fz_target = if self.freeze { 1.0 } else { 0.0 };
        self.freeze_amt += (fz_target - self.freeze_amt) * (1.0 - (-1.0 / (0.03 * sr)).exp());

        // Ducking follower: fast attack, slow release.
        let peak = l.abs().max(r.abs());
        let coef = if peak > self.env { 1.0 - (-1.0 / (0.004 * sr)).exp() } else { 1.0 - (-1.0 / (0.2 * sr)).exp() };
        self.env += (peak - self.env) * coef;
        let duck_gain = 1.0 - self.duck * (self.env * 3.0).min(1.0);
        self.duck_gain += (duck_gain - self.duck_gain) * 0.01;

        // Pre-delay, then the cuts.
        let mut u = [0.0; 2];
        for c in 0..2 {
            self.pre[c].push(input[c]);
            let mut v = self.pre[c].tap(self.pre_len + 1.0);
            if self.cut_on[0] {
                for st in 0..2 {
                    self.cut_state[c][0][st] += self.cut_a[0] * (v - self.cut_state[c][0][st]);
                    v -= self.cut_state[c][0][st];
                }
            }
            if self.cut_on[1] {
                for st in 0..2 {
                    self.cut_state[c][1][st] += self.cut_a[1] * (v - self.cut_state[c][1][st]);
                    v = self.cut_state[c][1][st];
                }
            }
            u[c] = v * (1.0 - self.freeze_amt);
        }

        // Early reflections: even taps from their own channel, odd taps from the other.
        let mut early = [0.0; 2];
        if self.er_scale_target > 0.0 || self.er_scale > 0.001 {
            for c in 0..2 {
                self.er[c].push(u[c]);
            }
            let to_samples = 0.001 * sr * self.er_scale;
            for c in 0..2 {
                let mut sum = 0.0;
                for k in 0..10 {
                    let src = if k % 2 == 0 { c } else { 1 - c };
                    sum += ER_SIGN[k] * 0.8_f32.powi(k as i32) * self.er[src].tap_linear(ER_MS[c][k] * to_samples);
                }
                early[c] = sum * 0.45 * self.early;
            }
        }

        // Diffusion, then injection into the network.
        let mut diffused = [0.0; 2];
        for c in 0..2 {
            let mut v = u[c];
            for ap in self.allpass[c].iter_mut() {
                v = ap.process(v, self.ap_gain);
            }
            diffused[c] = v;
        }
        let mid = (diffused[0] + diffused[1]) * 0.5;
        let side = (diffused[0] - diffused[1]) * 0.5;

        // The network.
        let shift = if self.shifting {
            self.shift_phase = (self.shift_phase + self.shift_inc) % TAU;
            Some(self.shift_phase.sin_cos())
        } else {
            None
        };
        let mut out = [0.0; LINES];
        let mut sum = 0.0;
        for i in 0..LINES {
            self.lfo[i] += self.lfo_inc[i];
            if self.lfo[i] > TAU {
                self.lfo[i] -= TAU;
            }
            self.line_len[i] += (self.line_target[i] - self.line_len[i]) * slew;
            let depth = self.mod_samples.min(self.line_len[i] * 0.08);
            let y = self.lines[i].tap(self.line_len[i] + depth * self.lfo[i].sin());

            for b in 0..3 {
                self.gain[i][b] += (self.gain_target[i][b] - self.gain[i][b]) * 0.05;
            }
            let mut band_lp = [0.0; 2];
            for (xo, lp) in self.lp[i].iter_mut().enumerate() {
                lp[0] += self.xo_a[xo] * (y - lp[0]);
                lp[1] += self.xo_a[xo] * (lp[0] - lp[1]);
                band_lp[xo] = lp[1];
            }
            let g = self.gain[i];
            let mut v = g[0] * band_lp[0] + g[1] * (band_lp[1] - band_lp[0]) + g[2] * (y - band_lp[1]);
            if let Some((sin, cos)) = shift {
                v = self.shifters[i].process(v, &self.hilbert, cos, sin);
            }
            let v = v.clamp(-16.0, 16.0);
            out[i] = v;
            sum += v;
        }
        let k = sum * (2.0 / LINES as f32);
        let mut late = [0.0; 2];
        for i in 0..LINES {
            let inj = self.inject * (mid * IN_MID[i] + side * SIGN_L[i]);
            self.lines[i].push(out[i] - k + inj);
            late[0] += SIGN_L[i] * out[i];
            late[1] += SIGN_R[i] * out[i];
        }
        let norm = 0.354;
        let (wl, wr) = (early[0] + late[0] * norm, early[1] + late[1] * norm);

        // Width, duck.
        let m = (wl + wr) * 0.5;
        let s = (wl - wr) * 0.5 * self.width;
        let g = self.duck_gain;
        let wet = [(m + s) * g, (m - s) * g];

        // Analysis.
        let mono = (wet[0] + wet[1]) * 0.5;
        for b in self.bands.iter_mut() {
            b.process(mono);
        }
        self.col_input = self.col_input.max(peak);
        self.col_wet = self.col_wet.max(wet[0].abs().max(wet[1].abs()));
        self.col_count += 1;
        if self.col_count >= self.col_len {
            self.publish();
        }

        let dry = 1.0 - self.mix;
        if self.channels > 1 {
            [x[0] * dry + wet[0] * self.mix, x[1] * dry + wet[1] * self.mix]
        } else {
            [x[0] * dry + mono * self.mix, 0.0]
        }
    }

    fn publish(&mut self) {
        let m = &self.meter;
        let n = self.col_count as f32;
        let slot = m.head.load(Ordering::Relaxed) % COLS;
        let (mut low, mut mid, mut high) = (0.0, 0.0, 0.0);
        for (b, band) in self.bands.iter_mut().enumerate() {
            m.bands[b].store((band.energy / n).sqrt(), Ordering::Relaxed);
            match b {
                b if b < LOW_END => low += band.energy,
                b if b < MID_END => mid += band.energy,
                _ => high += band.energy,
            }
            band.energy = 0.0;
        }
        m.input[slot].store(self.col_input, Ordering::Relaxed);
        m.wet[slot].store(self.col_wet, Ordering::Relaxed);
        m.low[slot].store((low / n).sqrt(), Ordering::Relaxed);
        m.mid[slot].store((mid / n).sqrt(), Ordering::Relaxed);
        m.high[slot].store((high / n).sqrt(), Ordering::Relaxed);
        m.duck.store(self.duck_gain, Ordering::Relaxed);
        m.head.fetch_add(1, Ordering::Release);
        self.col_count = 0;
        self.col_input = 0.0;
        self.col_wet = 0.0;
    }
}

/// Centre frequency of an analysis band.
pub fn band_freq(b: usize) -> f32 {
    BAND_LO_HZ * (BAND_HI_HZ / BAND_LO_HZ).powf(b as f32 / (BANDS - 1) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48000.0;
    const CTRL: usize = 32;

    /// A dry, steady setup the tests tweak: fully wet, nothing modulated, no early reflections.
    fn plain() -> Settings {
        Settings { mix: 1.0, mod_depth: 0.0, early: 0.0, high_mult: 1.0, predelay_ms: 0.0, ..Settings::default() }
    }

    fn reverb(channels: usize) -> Reverb {
        Reverb::new_at(SR, channels)
    }

    impl Reverb {
        fn new_at(sr: f32, channels: usize) -> Self {
            let mut r = Reverb::new();
            r.configure(sr, channels);
            r
        }
    }

    /// Runs `input` (left; right is the same unless `right` is given) in 32-sample blocks.
    fn run(r: &mut Reverb, s: &Settings, left: &[f32], right: Option<&[f32]>) -> (Vec<f32>, Vec<f32>) {
        let (mut ol, mut or) = (Vec::with_capacity(left.len()), Vec::with_capacity(left.len()));
        for (n, &l) in left.iter().enumerate() {
            if n % CTRL == 0 {
                r.set(s);
            }
            let rr = right.map_or(l, |x| x[n]);
            let o = r.process([l, rr]);
            ol.push(o[0]);
            or.push(o[1]);
        }
        (ol, or)
    }

    fn impulse(n: usize) -> Vec<f32> {
        let mut x = vec![0.0; n];
        x[0] = 1.0;
        x
    }

    fn noise(n: usize, amp: f32) -> Vec<f32> {
        let mut state = 0x1234_5678_u32;
        (0..n)
            .map(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                ((state >> 8) as f32 / (1 << 23) as f32 - 1.0) * amp
            })
            .collect()
    }

    fn sine(freq: f32, n: usize) -> Vec<f32> {
        (0..n).map(|i| (TAU * freq * i as f32 / SR).sin()).collect()
    }

    fn energy(x: &[f32]) -> f32 {
        x.iter().map(|v| v * v).sum()
    }

    fn rms(x: &[f32]) -> f32 {
        (energy(x) / x.len() as f32).sqrt()
    }

    fn db(x: f32) -> f32 {
        20.0 * x.max(1e-12).log10()
    }

    /// Single-bin DFT power at `freq`.
    fn power_at(x: &[f32], freq: f32) -> f32 {
        let (mut re, mut im) = (0.0, 0.0);
        for (i, v) in x.iter().enumerate() {
            let w = TAU * freq * i as f32 / SR;
            re += v * w.cos();
            im += v * w.sin();
        }
        (re * re + im * im) / (x.len() * x.len()) as f32
    }

    /// Reverb time from the Schroeder backward integral: the slope between -5 and -25 dB,
    /// extrapolated to 60 dB.
    fn rt60(ir: &[f32]) -> f32 {
        let mut edc = vec![0.0; ir.len()];
        let mut acc = 0.0;
        for i in (0..ir.len()).rev() {
            acc += ir[i] * ir[i];
            edc[i] = acc;
        }
        let total = edc[0];
        let at = |level: f32| edc.iter().position(|&e| 10.0 * (e / total).log10() < level).unwrap() as f32 / SR;
        3.0 * (at(-25.0) - at(-5.0))
    }

    #[test]
    fn zero_mix_is_a_null_test() {
        let mut r = reverb(2);
        let x = noise(20000, 0.5);
        let (l, rr) = run(&mut r, &Settings { mix: 0.0, ..Settings::default() }, &x, None);
        assert_eq!(l, x);
        assert_eq!(rr, x);
    }

    #[test]
    fn decay_time_matches_the_setting() {
        for (space, decay) in [(Space::Room, 1.0), (Space::Hall, 2.0), (Space::Abyss, 4.0)] {
            let mut r = reverb(2);
            let s = Settings { space, size: 0.4, decay_s: decay, ..plain() };
            let n = (SR * (decay * 2.0 + 1.0)) as usize;
            let (l, _) = run(&mut r, &s, &impulse(n), None);
            let measured = rt60(&l);
            assert!(
                (measured / decay - 1.0).abs() < 0.2,
                "{space:?}: asked {decay} s, measured {measured:.2} s"
            );
        }
    }

    #[test]
    fn high_decay_multiplier_darkens_the_tail() {
        let n = (SR * 3.0) as usize;
        let ir = |high_mult: f32| {
            let mut r = reverb(2);
            let (l, _) = run(&mut r, &Settings { decay_s: 2.0, high_mult, ..plain() }, &impulse(n), None);
            let t = &l[(SR * 1.0) as usize..];
            let hf: f32 = t.windows(2).map(|w| (w[1] - w[0]).powi(2)).sum();
            hf / energy(t)
        };
        let (flat, dark) = (ir(1.0), ir(0.15));
        assert!(dark < flat * 0.5, "flat {flat}, dark {dark}");
    }

    #[test]
    fn decay_curve_matches_the_settings() {
        let tau = mean_delay_s(Space::Hall, 0.5);
        let flat = decay_at(1000.0, tau, 3.0, 1.0, 1.0, 300.0, 4000.0);
        assert!((flat / 3.0 - 1.0).abs() < 0.01, "{flat}");
        let (low, mid, high) = (
            decay_at(25.0, tau, 3.0, 0.4, 0.2, 300.0, 4000.0),
            decay_at(1000.0, tau, 3.0, 0.4, 0.2, 300.0, 4000.0),
            decay_at(19000.0, tau, 3.0, 0.4, 0.2, 300.0, 4000.0),
        );
        // The crossovers are 12 dB/octave, so the mids still feel the high band a little.
        assert!(low < 1.4 && (2.0..3.2).contains(&mid) && high < 0.8, "{low} {mid} {high}");
    }

    #[test]
    fn freeze_holds_the_tail() {
        let mut r = reverb(2);
        let n = (SR * 4.0) as usize;
        let mut x = noise(n, 0.5);
        x[(SR * 0.5) as usize..].fill(0.0);
        let mut s = Settings { decay_s: 1.0, ..plain() };
        let (a, _) = run(&mut r, &s, &x[..(SR * 0.5) as usize], None);
        assert!(rms(&a[a.len() / 2..]) > 0.01);
        s.freeze = true;
        let (b, _) = run(&mut r, &s, &x[(SR * 0.5) as usize..], None);
        let w = (SR * 0.5) as usize;
        let early = rms(&b[(SR * 0.5) as usize..][..w]);
        let late = rms(&b[(SR * 3.0) as usize..][..w]);
        assert!(early > 0.01);
        assert!((db(late) - db(early)).abs() < 1.5, "{} dB vs {} dB", db(early), db(late));
    }

    #[test]
    fn without_freeze_the_same_tail_dies() {
        let mut r = reverb(2);
        let n = (SR * 4.0) as usize;
        let mut x = noise(n, 0.5);
        x[(SR * 0.5) as usize..].fill(0.0);
        let (b, _) = run(&mut r, &Settings { decay_s: 1.0, ..plain() }, &x, None);
        let w = (SR * 0.5) as usize;
        assert!(db(rms(&b[(SR * 3.0) as usize..][..w])) < db(rms(&b[(SR * 0.6) as usize..][..w])) - 30.0);
    }

    #[test]
    fn early_reflections_come_first_and_stop() {
        let n = (SR * 0.3) as usize;
        let s = Settings { space: Space::Room, size: 0.0, ..plain() };
        let (off, _) = run(&mut reverb(2), &s, &impulse(n), None);
        let (on, _) = run(&mut reverb(2), &Settings { early: 1.0, ..s }, &impulse(n), None);
        let diff: Vec<f32> = on.iter().zip(&off).map(|(a, b)| a - b).collect();
        // Size 0 puts the last tap at 66.3 ms * 0.3 = 20 ms.
        assert!(energy(&diff[..(SR * 0.025) as usize]) > 1e-3);
        assert_eq!(energy(&diff[(SR * 0.03) as usize..]), 0.0);
    }

    #[test]
    fn plate_has_no_early_reflections() {
        let n = (SR * 0.2) as usize;
        let s = Settings { space: Space::Plate, ..plain() };
        let (off, _) = run(&mut reverb(2), &s, &impulse(n), None);
        let (on, _) = run(&mut reverb(2), &Settings { early: 1.0, ..s }, &impulse(n), None);
        assert_eq!(off, on);
    }

    #[test]
    fn predelay_delays_the_wet_signal() {
        let n = (SR * 0.3) as usize;
        let (l, _) = run(&mut reverb(2), &Settings { predelay_ms: 50.0, ..plain() }, &impulse(n), None);
        let quiet = (SR * 0.049) as usize;
        assert_eq!(energy(&l[..quiet]), 0.0);
        assert!(energy(&l[quiet..]) > 1e-4);
    }

    #[test]
    fn width_zero_is_mono() {
        let mut r = reverb(2);
        let n = (SR * 0.5) as usize;
        let s = Settings { width: 0.0, ..plain() };
        let (l, rr) = run(&mut r, &s, &noise(n, 0.5), Some(&noise(n, 0.5).iter().rev().cloned().collect::<Vec<_>>()));
        for (a, b) in l.iter().zip(&rr) {
            assert!((a - b).abs() < 1e-5);
        }
        let mut r = reverb(2);
        let (l, rr) = run(&mut r, &Settings { width: 1.0, ..plain() }, &impulse(n), None);
        let diff: f32 = l.iter().zip(&rr).map(|(a, b)| (a - b).powi(2)).sum();
        assert!(diff > 0.01 * energy(&l));
    }

    #[test]
    fn frequency_shifter_moves_a_sine_up_without_an_image() {
        let mut sh = Shifter::default();
        let coefs = hilbert_coefs(SR);
        let n = SR as usize;
        let mut phase = 0.0_f32;
        let x = sine(1000.0, n);
        let y: Vec<f32> = x
            .iter()
            .map(|&v| {
                phase += TAU * 100.0 / SR;
                let (s, c) = phase.sin_cos();
                sh.process(v, &coefs, c, s)
            })
            .collect();
        let (up, down, orig) = (power_at(&y[4800..], 1100.0), power_at(&y[4800..], 900.0), power_at(&y[4800..], 1000.0));
        assert!(db(up.sqrt()) > -7.0, "wanted unit amplitude at 1100: {} dB", db(up.sqrt()));
        assert!(db(up.sqrt()) - db(down.sqrt()) > 25.0, "image only {} dB down", db(up.sqrt()) - db(down.sqrt()));
        assert!(db(up.sqrt()) - db(orig.sqrt()) > 25.0);
    }

    #[test]
    fn shift_moves_the_tail_and_stays_stable() {
        let n = (SR * 3.0) as usize;
        let s = Settings { decay_s: 3.0, shift_hz: 20.0, ..plain() };
        let (l, rr) = run(&mut reverb(2), &s, &noise(n, 0.5), None);
        assert!(l.iter().chain(&rr).all(|v| v.is_finite() && v.abs() < 10.0));
        assert!(rms(&l[n / 2..]) > 0.02);
    }

    #[test]
    fn duck_lowers_the_wet_under_loud_input() {
        let n = (SR * 1.0) as usize;
        let x = noise(n, 0.8);
        let level = |duck: f32| rms(&run(&mut reverb(2), &Settings { duck, ..plain() }, &x, None).0[n / 2..]);
        assert!(db(level(1.0)) < db(level(0.0)) - 6.0);
    }

    #[test]
    fn modulation_changes_the_tail_without_collapsing_its_level() {
        let n = (SR * 2.0) as usize;
        let x = noise(n, 0.5);
        let still = run(&mut reverb(2), &Settings { mod_depth: 0.0, ..plain() }, &x, None).0;
        let moving = run(&mut reverb(2), &Settings { mod_depth: 1.0, ..plain() }, &x, None).0;
        assert_ne!(still, moving);
        assert!((db(rms(&still[n / 2..])) - db(rms(&moving[n / 2..]))).abs() < 4.0);
    }

    #[test]
    fn cuts_remove_the_bands_they_name() {
        let n = (SR * 1.0) as usize;
        let x = noise(n, 0.5);
        let band = |s: Settings, f: f32| power_at(&run(&mut reverb(2), &s, &x, None).0[n / 2..], f);
        let hp = band(Settings { low_cut: 800.0, ..plain() }, 60.0) / band(plain(), 60.0);
        let lp = band(Settings { high_cut: 1500.0, ..plain() }, 9000.0) / band(plain(), 9000.0);
        assert!(hp < 0.05 && lp < 0.05, "hp {hp} lp {lp}");
    }

    #[test]
    fn output_stays_finite_at_extremes() {
        let n = (SR * 3.0) as usize;
        let x = noise(n, 1.0);
        for space in [Space::Room, Space::Hall, Space::Plate, Space::Chamber, Space::Abyss] {
            for (size, decay, shift, freeze) in [(0.0, 0.2, 0.0, false), (1.0, 40.0, 0.0, false), (1.0, 40.0, 500.0, true), (0.5, 10.0, -500.0, false)] {
                let s = Settings {
                    space,
                    size,
                    decay_s: decay,
                    shift_hz: shift,
                    freeze,
                    low_mult: 2.0,
                    high_mult: 2.0,
                    mod_depth: 1.0,
                    mod_rate: 5.0,
                    diffusion: 1.0,
                    early: 1.0,
                    width: 1.5,
                    predelay_ms: 300.0,
                    ..plain()
                };
                let (l, rr) = run(&mut reverb(2), &s, &x, Some(&x.iter().rev().cloned().collect::<Vec<_>>()));
                let peak = l.iter().chain(&rr).fold(0.0_f32, |m, v| m.max(v.abs()));
                assert!(l.iter().chain(&rr).all(|v| v.is_finite()) && peak < 30.0, "{space:?} {size} {decay}: peak {peak}");
            }
        }
    }

    #[test]
    fn sudden_changes_do_not_click() {
        // Switch space and size mid-tail: the glide keeps the output bounded.
        let mut r = reverb(2);
        let x = noise((SR * 2.0) as usize, 0.5);
        let mut out = Vec::new();
        for (k, space) in [Space::Room, Space::Abyss, Space::Plate, Space::Hall].into_iter().enumerate() {
            let s = Settings { space, size: if k % 2 == 0 { 0.1 } else { 0.9 }, ..plain() };
            out.extend(run(&mut r, &s, &x[..(SR * 0.5) as usize], None).0);
        }
        assert!(out.iter().all(|v| v.is_finite() && v.abs() < 5.0));
    }

    #[test]
    fn works_in_mono() {
        let mut r = reverb(1);
        let x = noise((SR * 1.0) as usize, 0.5);
        let (l, _) = run(&mut r, &plain(), &x, None);
        assert!(l.iter().all(|v| v.is_finite()) && rms(&l[l.len() / 2..]) > 0.02);
    }

    #[test]
    fn other_sample_rates_keep_the_decay_time() {
        for sr in [44100.0_f32, 96000.0] {
            let mut r = Reverb::new_at(sr, 2);
            let s = Settings { decay_s: 1.5, ..plain() };
            let n = (sr * 4.0) as usize;
            let mut out = Vec::new();
            for (i, chunk) in impulse(n).chunks(CTRL).enumerate() {
                r.set(&s);
                for &v in chunk {
                    out.push(r.process([v, v])[0]);
                }
                let _ = i;
            }
            let mut edc = vec![0.0; out.len()];
            let mut acc = 0.0;
            for i in (0..out.len()).rev() {
                acc += out[i] * out[i];
                edc[i] = acc;
            }
            let at = |level: f32| edc.iter().position(|&e| 10.0 * (e / edc[0]).log10() < level).unwrap() as f32 / sr;
            let measured = 3.0 * (at(-25.0) - at(-5.0));
            assert!((measured / 1.5 - 1.0).abs() < 0.2, "{sr}: {measured}");
        }
    }

    #[test]
    fn meter_publishes_columns_and_a_spectrum() {
        let mut r = reverb(2);
        let (_, _) = run(&mut r, &plain(), &noise(SR as usize, 0.5), None);
        let m = &r.meter;
        let head = m.head.load(Ordering::Relaxed);
        assert!(head >= 19, "{head}");
        assert!(m.input[(head - 1) % COLS].load(Ordering::Relaxed) > 0.3);
        assert!(m.wet[(head - 1) % COLS].load(Ordering::Relaxed) > 0.01);
        assert!(m.mid[(head - 1) % COLS].load(Ordering::Relaxed) > 0.0);
        assert!(m.bands.iter().all(|b| b.load(Ordering::Relaxed) > 0.0));
    }
}
