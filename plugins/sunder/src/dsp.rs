//! Ring modulator DSP.
//!
//! The signal path, per channel and at the oversampled rate:
//!
//! 1. The input goes through a 12-pole Hilbert all-pass pair, giving an analytic signal
//!    `(xi, xq)`: two copies of the input 90 degrees apart.
//! 2. The **carrier** comes from an oscillator (sine, triangle, square or saw, the last two
//!    PolyBLEP band-limited), from the sidechain input, with its pitch set by the knob, the last
//!    MIDI note or the pitch tracked from the input. An envelope follower and an LFO bend it.
//!    Sines are generated as a quadrature pair; anything else goes through its own Hilbert pair.
//! 3. The **ring** is `xi * ci`, either an ideal multiplier or Julian Parker's diode ring model
//!    (DAFx-11, "A simple digital model of the diode-based ring-modulator"): four diodes in a
//!    bridge, each a soft-knee static non-linearity, fed with the carrier and half the input.
//!    The carrier level into the diodes is the Drive; a low drive starves the diodes (gritty,
//!    gated, quiet) and a high one turns the ring into a multiplication by a square wave.
//! 4. **Sideband** blends toward single-sideband shifting: `xi*ci - shift * xq*cq` is the upper
//!    sideband alone at +1 (the sum frequency), the lower alone at -1 and the plain ring at 0,
//!    which is a Bode-style frequency shifter when the carrier is below the audio band.
//!    **Bias** adds the carrier's DC, leaking the original signal back in (amplitude modulation).
//!
//! The non-linear part runs oversampled with cascaded halfband filters, and the latency is
//! reported so the dry path can be delayed to match.

use nih_plug::prelude::{AtomicF32, Enum};
use std::f32::consts::{PI, TAU};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use crate::pitch::PitchTracker;

pub const MAX_CHANNELS: usize = 2;
/// Highest carrier frequency, in Hz.
pub const MAX_CARRIER_HZ: f32 = 16000.0;
/// Spectrum display: FFT size, hop, and the number of linear bins published (from 0 Hz up).
pub const FFT_N: usize = 2048;
pub const HOP: usize = 512;
pub const SPEC_BINS: usize = 256;
/// Points in one scope lane.
pub const SCOPE: usize = 256;
/// Level the spectra are published at, below which a bin reads as silence.
pub const FLOOR_DB: f32 = -100.0;

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The Freq knob.
    #[name = "Free"]
    Free,
    /// The last MIDI note played, times Ratio.
    #[name = "MIDI"]
    Midi,
    /// The pitch of the input, times Ratio: the carrier is always in tune with what it modulates.
    #[name = "Track"]
    Track,
    /// The sidechain input is the carrier.
    #[name = "Sidechain"]
    Sidechain,
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Circuit {
    #[name = "Diode"]
    Diode,
    #[name = "Ideal"]
    Ideal,
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wave {
    #[name = "Sine"]
    Sine,
    #[name = "Triangle"]
    Triangle,
    #[name = "Square"]
    Square,
    #[name = "Saw"]
    Saw,
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Oversampling {
    #[name = "Off"]
    Off,
    #[name = "2x"]
    X2,
    #[name = "4x"]
    X4,
    #[name = "8x"]
    X8,
}

impl Oversampling {
    pub fn stages(self) -> usize {
        self as usize
    }
}

/// Everything the audio thread needs for one sample.
#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub source: Source,
    pub circuit: Circuit,
    pub wave: Wave,
    pub oversampling: Oversampling,
    pub freq_hz: f32,
    pub ratio: f32,
    /// Carrier level into the diodes, 0..1.
    pub drive: f32,
    /// Carrier DC offset, 0 (ring) to 1 (amplitude modulation).
    pub bias: f32,
    /// -1 lower sideband only, 0 both (a ring modulator), +1 upper sideband only.
    pub shift: f32,
    /// Octaves of carrier change per unit of input level, -1..1 (times two).
    pub follow: f32,
    /// LFO depth on the carrier, 0..1 (one octave each way).
    pub wobble: f32,
    pub wobble_hz: f32,
    /// Phase offset of the right carrier, 0..1 (a half cycle at 1).
    pub spread: f32,
    /// Low-pass on the wet signal.
    pub tone_hz: f32,
    pub mix: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            source: Source::Free,
            circuit: Circuit::Diode,
            wave: Wave::Sine,
            oversampling: Oversampling::X4,
            freq_hz: 440.0,
            ratio: 1.0,
            drive: 0.4,
            bias: 0.0,
            shift: 0.0,
            follow: 0.0,
            wobble: 0.0,
            wobble_hz: 1.5,
            spread: 0.0,
            tone_hz: 20000.0,
            mix: 1.0,
        }
    }
}

/// Carrier and sidebands as the editor should show them, without the smoothing or modulation.
impl Settings {
    /// Amplitude weight of the upper (sum) sideband.
    pub fn upper_weight(&self) -> f32 {
        0.5 + 0.5 * self.shift
    }

    /// Amplitude weight of the lower (difference) sideband.
    pub fn lower_weight(&self) -> f32 {
        0.5 - 0.5 * self.shift
    }
}

// ---------------------------------------------------------------------------------------------
// Oversampling
// ---------------------------------------------------------------------------------------------

/// Round-trip delay, in samples at the base rate, of each oversampling setting.
pub const LATENCY: [usize; 4] = [0, 16, 24, 28];
pub const MAX_STAGES: usize = 3;
const MAX_FACTOR: usize = 1 << MAX_STAGES;

const TAPS: usize = 33;
const CENTER: usize = TAPS / 2;

/// Windowed-sinc halfband lowpass, normalised to unity gain at DC. Every second tap away from the
/// centre is zero, which the stages below rely on.
fn halfband() -> [f32; TAPS] {
    let mut h = [0.0; TAPS];
    let mut sum = 0.0;
    for (j, tap) in h.iter_mut().enumerate() {
        let d = j as f32 - CENTER as f32;
        let sinc = if d == 0.0 {
            1.0
        } else if (d as i32) % 2 == 0 {
            0.0
        } else {
            (PI * d / 2.0).sin() / (PI * d / 2.0)
        };
        let n = TAPS as f32 - 1.0;
        let w = 0.42 - 0.5 * (2.0 * PI * j as f32 / n).cos() + 0.08 * (4.0 * PI * j as f32 / n).cos();
        *tap = 0.5 * sinc * w;
        sum += *tap;
    }
    for tap in &mut h {
        *tap /= sum;
    }
    h
}

/// 2x upsampler, polyphase.
#[derive(Clone)]
struct Up {
    hist: [f32; 16],
    pos: usize,
    even: f32,
    odd: [f32; 16],
}

impl Up {
    fn new(h: &[f32; TAPS]) -> Self {
        Self { hist: [0.0; 16], pos: 0, even: 2.0 * h[CENTER], odd: std::array::from_fn(|k| 2.0 * h[2 * k + 1]) }
    }

    fn push(&mut self, x: f32) -> (f32, f32) {
        self.pos = (self.pos + 1) & 15;
        self.hist[self.pos] = x;
        let even = self.even * self.hist[self.pos.wrapping_sub(CENTER / 2) & 15];
        let odd = self.odd.iter().enumerate().map(|(k, t)| t * self.hist[self.pos.wrapping_sub(k) & 15]).sum();
        (even, odd)
    }
}

/// 2x decimator: filters only at the samples it keeps.
#[derive(Clone)]
struct Down {
    hist: [f32; 64],
    pos: usize,
    center: f32,
    odd: [f32; 16],
}

impl Down {
    fn new(h: &[f32; TAPS]) -> Self {
        Self { hist: [0.0; 64], pos: 0, center: h[CENTER], odd: std::array::from_fn(|k| h[2 * k + 1]) }
    }

    fn push(&mut self, a: f32, b: f32) -> f32 {
        self.pos = (self.pos + 1) & 63;
        self.hist[self.pos] = a;
        let mut y = self.center * self.hist[self.pos.wrapping_sub(CENTER) & 63];
        for (k, t) in self.odd.iter().enumerate() {
            y += t * self.hist[self.pos.wrapping_sub(2 * k + 1) & 63];
        }
        self.pos = (self.pos + 1) & 63;
        self.hist[self.pos] = b;
        y
    }
}

/// Splits one base-rate stream into 1, 2, 4 or 8 samples per input, and folds them back. The two
/// halves are separate calls so several streams can be combined at the high rate in between.
#[derive(Clone)]
pub struct Oversampler {
    up: [Up; MAX_STAGES],
    down: [Down; MAX_STAGES],
    stages: usize,
}

impl Oversampler {
    pub fn new() -> Self {
        let h = halfband();
        Self { up: std::array::from_fn(|_| Up::new(&h)), down: std::array::from_fn(|_| Down::new(&h)), stages: 0 }
    }

    pub fn set_stages(&mut self, stages: usize) {
        if stages != self.stages {
            *self = Self { stages, ..Self::new() };
        }
    }

    pub fn clear(&mut self) {
        *self = Self { stages: self.stages, ..Self::new() };
    }

    /// Writes `1 << stages` samples to `out`.
    pub fn upsample(&mut self, x: f32, out: &mut [f32; MAX_FACTOR]) {
        out[0] = x;
        let mut n = 1;
        for stage in 0..self.stages {
            let src = *out;
            for i in 0..n {
                let (even, odd) = self.up[stage].push(src[i]);
                out[2 * i] = even;
                out[2 * i + 1] = odd;
            }
            n *= 2;
        }
    }

    /// Folds `1 << stages` samples of `buf` down to one.
    pub fn downsample(&mut self, buf: &mut [f32; MAX_FACTOR]) -> f32 {
        let mut n = 1 << self.stages;
        for stage in (0..self.stages).rev() {
            n /= 2;
            for i in 0..n {
                buf[i] = self.down[stage].push(buf[2 * i], buf[2 * i + 1]);
            }
        }
        buf[0]
    }
}

// ---------------------------------------------------------------------------------------------
// Hilbert pair
// ---------------------------------------------------------------------------------------------

/// Two all-pass chains 90 degrees apart (the pole set from Bernie Hutchins' Musical Engineer's
/// Handbook, as used by Csound's `hilbert`).
#[derive(Clone, Copy, Default)]
struct Hilbert {
    x1: [f32; 12],
    y1: [f32; 12],
}

const HILBERT_POLES: [f32; 12] =
    [0.3609, 2.7412, 11.1573, 44.7581, 179.6242, 798.4578, 1.2524, 5.5671, 22.3423, 89.6271, 364.7914, 2770.1114];

/// Which way round the quadrature output has to be taken so `xi * ci - xq * cq` is the upper
/// sideband. Pinned down by `shift_up_moves_the_sine_up`.
const QUADRATURE_SIGN: f32 = -1.0;

fn hilbert_coefs(sample_rate: f32) -> [f32; 12] {
    std::array::from_fn(|j| {
        let alpha = TAU * HILBERT_POLES[j] * 15.0;
        let k = alpha * 0.5 / sample_rate;
        -(1.0 - k) / (1.0 + k)
    })
}

impl Hilbert {
    /// `(in-phase, quadrature)` of `x`.
    #[inline]
    fn process(&mut self, x: f32, coefs: &[f32; 12]) -> (f32, f32) {
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
        (a, QUADRATURE_SIGN * b)
    }
}

// ---------------------------------------------------------------------------------------------
// Diodes
// ---------------------------------------------------------------------------------------------

/// Parker's diode: dead below `VB`, a quadratic knee up to `VL`, then linear.
const VB: f32 = 0.2;
const VL: f32 = 0.4;
const KNEE: f32 = (VL - VB) / 2.0;

#[inline]
fn diode(v: f32) -> f32 {
    if v <= VB {
        0.0
    } else if v <= VL {
        (v - VB) * (v - VB) / (2.0 * (VL - VB))
    } else {
        v - VL + KNEE
    }
}

#[inline]
fn diode_slope(v: f32) -> f32 {
    if v <= VB {
        0.0
    } else if v <= VL {
        (v - VB) / (VL - VB)
    } else {
        1.0
    }
}

/// The diode bridge: `E(c + x/2) - E(c - x/2)` with `E(u) = D(u) + D(-u)`. For a small input this
/// is `E'(c) * x`, which is odd in the carrier, so it multiplies.
#[inline]
pub fn diode_ring(x: f32, c: f32) -> f32 {
    let a = c + 0.5 * x;
    let b = c - 0.5 * x;
    diode(a) + diode(-a) - diode(b) - diode(-b)
}

/// Carrier amplitude at the diodes for a Drive of 0..1.
#[inline]
pub fn carrier_level(drive: f32) -> f32 {
    0.35 + 1.65 * drive
}

/// Scales the bridge so that, for a small input, its RMS gain over a carrier cycle matches an
/// ideal multiplier's (1/sqrt 2), whatever the carrier level.
pub fn diode_norm(level: f32) -> f32 {
    const N: usize = 32;
    let mut sum = 0.0;
    for i in 0..N {
        let u = level * (TAU * (i as f32 + 0.5) / N as f32).sin();
        let g = diode_slope(u) - diode_slope(-u);
        sum += g * g;
    }
    let rms = (sum / N as f32).sqrt();
    std::f32::consts::FRAC_1_SQRT_2 / rms.max(0.06)
}

// ---------------------------------------------------------------------------------------------
// Oscillator
// ---------------------------------------------------------------------------------------------

#[inline]
fn blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let t = t / dt;
        2.0 * t - t * t - 1.0
    } else if t > 1.0 - dt {
        let t = (t - 1.0) / dt;
        t * t + 2.0 * t + 1.0
    } else {
        0.0
    }
}

/// A non-sine carrier at phase `p` (0..1) with phase increment `dt`.
#[inline]
fn shape(wave: Wave, p: f32, dt: f32) -> f32 {
    match wave {
        Wave::Sine => (TAU * p).cos(),
        Wave::Triangle => 1.0 - 4.0 * (p - 0.5).abs(),
        Wave::Square => {
            let v = if p < 0.5 { 1.0 } else { -1.0 };
            v + blep(p, dt) - blep((p + 0.5).fract(), dt)
        }
        Wave::Saw => 2.0 * p - 1.0 - blep(p, dt),
    }
}

// ---------------------------------------------------------------------------------------------
// Meter
// ---------------------------------------------------------------------------------------------

/// What the audio thread publishes for the editor.
pub struct Meter {
    /// Spectrum frames published so far.
    pub frames: AtomicUsize,
    /// Magnitude in dB (full-scale sine = 0) of the dry input and the wet output, bin `k` at
    /// `k * sample_rate / FFT_N`.
    pub spec_in: [AtomicF32; SPEC_BINS],
    pub spec_out: [AtomicF32; SPEC_BINS],
    pub sample_rate: AtomicF32,
    /// The carrier as it is right now (after follow and wobble), and before them.
    pub carrier_hz: AtomicF32,
    pub target_hz: AtomicF32,
    /// The pitch (tracked) or note (MIDI) that Ratio multiplies; zero when there is none.
    pub base_hz: AtomicF32,
    pub env: AtomicF32,
    /// One captured frame: input, carrier and output, triggered on the carrier phase. Two
    /// buffers, so the editor reads the finished one while the next is written.
    pub scope: [[[AtomicF32; SCOPE]; 3]; 2],
    pub scope_ready: AtomicUsize,
    pub scope_ms: AtomicF32,
}

impl Meter {
    fn new() -> Self {
        Self {
            frames: AtomicUsize::new(0),
            spec_in: std::array::from_fn(|_| AtomicF32::new(FLOOR_DB)),
            spec_out: std::array::from_fn(|_| AtomicF32::new(FLOOR_DB)),
            sample_rate: AtomicF32::new(48000.0),
            carrier_hz: AtomicF32::new(0.0),
            target_hz: AtomicF32::new(0.0),
            base_hz: AtomicF32::new(0.0),
            env: AtomicF32::new(0.0),
            scope: std::array::from_fn(|_| std::array::from_fn(|_| std::array::from_fn(|_| AtomicF32::new(0.0)))),
            scope_ready: AtomicUsize::new(0),
            scope_ms: AtomicF32::new(10.0),
        }
    }
}

/// Windowed FFT of the input and the wet output, published every `HOP` samples.
struct Analyzer {
    ring: [Vec<f32>; 2],
    pos: usize,
    count: usize,
    window: Vec<f32>,
    re: Vec<f32>,
    im: Vec<f32>,
    tw_re: Vec<f32>,
    tw_im: Vec<f32>,
}

impl Analyzer {
    fn new() -> Self {
        Self {
            ring: [vec![0.0; FFT_N], vec![0.0; FFT_N]],
            pos: 0,
            count: 0,
            window: (0..FFT_N).map(|i| 0.5 - 0.5 * (TAU * i as f32 / FFT_N as f32).cos()).collect(),
            re: vec![0.0; FFT_N],
            im: vec![0.0; FFT_N],
            tw_re: (0..FFT_N / 2).map(|k| (TAU * k as f32 / FFT_N as f32).cos()).collect(),
            tw_im: (0..FFT_N / 2).map(|k| -(TAU * k as f32 / FFT_N as f32).sin()).collect(),
        }
    }

    fn clear(&mut self) {
        for r in &mut self.ring {
            r.fill(0.0);
        }
        self.count = 0;
    }

    fn push(&mut self, input: f32, output: f32, meter: &Meter) {
        self.ring[0][self.pos] = input;
        self.ring[1][self.pos] = output;
        self.pos = (self.pos + 1) % FFT_N;
        self.count += 1;
        if self.count < HOP {
            return;
        }
        self.count = 0;
        for which in 0..2 {
            for i in 0..FFT_N {
                self.re[i] = self.ring[which][(self.pos + i) % FFT_N] * self.window[i];
                self.im[i] = 0.0;
            }
            fft(&mut self.re, &mut self.im, &self.tw_re, &self.tw_im);
            let target = if which == 0 { &meter.spec_in } else { &meter.spec_out };
            for k in 0..SPEC_BINS {
                let mag = (self.re[k] * self.re[k] + self.im[k] * self.im[k]).sqrt() / (FFT_N as f32 * 0.25);
                target[k].store((20.0 * (mag + 1e-7).log10()).max(FLOOR_DB), Ordering::Relaxed);
            }
        }
        meter.frames.fetch_add(1, Ordering::Release);
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

/// Captures one scope frame at a time, starting each when the carrier phase wraps.
struct ScopeCapture {
    recording: bool,
    index: usize,
    skip: usize,
    every: usize,
    idle: usize,
    writing: usize,
}

impl ScopeCapture {
    fn new() -> Self {
        Self { recording: false, index: 0, skip: 0, every: 1, idle: 0, writing: 1 }
    }
}

// ---------------------------------------------------------------------------------------------
// The effect
// ---------------------------------------------------------------------------------------------

pub struct RingMod {
    pub meter: Arc<Meter>,
    fs: f32,
    fs_os: f32,
    channels: usize,
    stages: usize,

    os: [Oversampler; MAX_CHANNELS],
    os_side: [Oversampler; MAX_CHANNELS],
    hil: [f32; 12],
    hx: [Hilbert; MAX_CHANNELS],
    hc: [Hilbert; MAX_CHANNELS],

    phase: f64,
    lfo_phase: f32,
    env: f32,
    env_attack: f32,
    env_release: f32,
    glide: f32,
    freq: f32,
    last_pitch: f32,
    note_hz: Option<f32>,
    tracker: PitchTracker,

    norm_level: f32,
    norm: f32,

    tone: [[f32; 2]; MAX_CHANNELS],
    dry: [[f32; 64]; MAX_CHANNELS],
    dry_pos: usize,

    analyzer: Analyzer,
    scope: ScopeCapture,
}

impl RingMod {
    pub fn new() -> Self {
        let mut me = Self {
            meter: Arc::new(Meter::new()),
            fs: 48000.0,
            fs_os: 48000.0,
            channels: 2,
            stages: 0,
            os: [Oversampler::new(), Oversampler::new()],
            os_side: [Oversampler::new(), Oversampler::new()],
            hil: [0.0; 12],
            hx: [Hilbert::default(); MAX_CHANNELS],
            hc: [Hilbert::default(); MAX_CHANNELS],
            phase: 0.0,
            lfo_phase: 0.0,
            env: 0.0,
            env_attack: 0.0,
            env_release: 0.0,
            glide: 0.0,
            freq: 440.0,
            last_pitch: 0.0,
            note_hz: None,
            tracker: PitchTracker::new(48000.0, 50.0),
            norm_level: -1.0,
            norm: 1.0,
            tone: [[0.0; 2]; MAX_CHANNELS],
            dry: [[0.0; 64]; MAX_CHANNELS],
            dry_pos: 0,
            analyzer: Analyzer::new(),
            scope: ScopeCapture::new(),
        };
        me.configure(48000.0, 2);
        me
    }

    pub fn configure(&mut self, sample_rate: f32, channels: usize) {
        self.fs = sample_rate;
        self.channels = channels.clamp(1, MAX_CHANNELS);
        self.tracker = PitchTracker::new(sample_rate, 50.0);
        self.env_attack = 1.0 - (-1.0 / (0.003 * sample_rate)).exp();
        self.env_release = 1.0 - (-1.0 / (0.12 * sample_rate)).exp();
        self.glide = 1.0 - (-1.0 / (0.03 * sample_rate)).exp();
        self.meter.sample_rate.store(sample_rate, Ordering::Relaxed);
        self.set_oversampling(self.stages);
        self.reset();
    }

    pub fn reset(&mut self) {
        for o in self.os.iter_mut().chain(self.os_side.iter_mut()) {
            o.clear();
        }
        self.hx = [Hilbert::default(); MAX_CHANNELS];
        self.hc = [Hilbert::default(); MAX_CHANNELS];
        self.phase = 0.0;
        self.lfo_phase = 0.0;
        self.env = 0.0;
        self.last_pitch = 0.0;
        self.tracker.reset();
        self.tone = [[0.0; 2]; MAX_CHANNELS];
        self.dry = [[0.0; 64]; MAX_CHANNELS];
        self.analyzer.clear();
        self.scope = ScopeCapture::new();
    }

    pub fn set_oversampling(&mut self, stages: usize) {
        self.stages = stages.min(MAX_STAGES);
        for o in self.os.iter_mut().chain(self.os_side.iter_mut()) {
            o.set_stages(self.stages);
        }
        self.fs_os = self.fs * (1 << self.stages) as f32;
        self.hil = hilbert_coefs(self.fs_os);
    }

    pub fn stages(&self) -> usize {
        self.stages
    }

    /// Latency in samples at the current oversampling.
    pub fn latency(&self) -> u32 {
        LATENCY[self.stages] as u32
    }

    /// The MIDI note that sets the carrier in `Source::Midi`.
    pub fn note_on(&mut self, note: u8) {
        self.note_hz = Some(440.0 * 2.0_f32.powf((note as f32 - 69.0) / 12.0));
    }

    /// Processes one frame. `side` is the sidechain input (ignored unless the source is
    /// `Sidechain`). Channels beyond `channels` are passed through untouched.
    pub fn process(&mut self, input: [f32; MAX_CHANNELS], side: [f32; MAX_CHANNELS], s: &Settings) -> [f32; MAX_CHANNELS] {
        let n = 1usize << self.stages;
        let channels = self.channels;
        let mono = input[..channels].iter().sum::<f32>() / channels as f32;

        // Envelope follower, for Follow and the editor.
        let level = input[..channels].iter().fold(0.0_f32, |m, x| m.max(x.abs()));
        let k = if level > self.env { self.env_attack } else { self.env_release };
        self.env += (level - self.env) * k;
        let env = (self.env * 2.0).min(1.0);

        // Where the carrier wants to be.
        if s.source == Source::Track {
            if let Some(frame) = self.tracker.push(mono) {
                if frame.voiced {
                    self.last_pitch = frame.f0;
                }
            }
        }
        let base = match s.source {
            Source::Midi => self.note_hz.unwrap_or(0.0),
            Source::Track => self.last_pitch,
            _ => 0.0,
        };
        let target = if base > 0.0 { base * s.ratio } else { s.freq_hz }.clamp(0.05, MAX_CARRIER_HZ);
        if s.source == Source::Free || s.source == Source::Sidechain {
            self.freq = target;
        } else {
            self.freq += (target - self.freq) * self.glide;
        }

        self.lfo_phase += s.wobble_hz / self.fs;
        self.lfo_phase -= self.lfo_phase.floor();
        let octaves = 2.0 * s.follow * env + s.wobble * (TAU * self.lfo_phase).sin();
        let carrier = (self.freq * 2.0_f32.powf(octaves)).clamp(0.01, MAX_CARRIER_HZ);
        let inc = carrier as f64 / self.fs_os as f64;

        // The carrier's phase at each oversampled step.
        let mut phases = [0.0_f64; MAX_FACTOR];
        let mut wrapped = false;
        for p in phases.iter_mut().take(n) {
            self.phase += inc;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
                wrapped = true;
            }
            *p = self.phase;
        }

        let level = carrier_level(s.drive);
        if (level - self.norm_level).abs() > 1e-3 {
            self.norm_level = level;
            self.norm = diode_norm(level);
        }
        let shift = s.shift;
        let shift_gain = 1.0 / (1.0 + shift.abs()).sqrt();
        let bias_gain = 1.0 / (1.0 + s.bias);
        let dt = inc as f32;
        let tone_k = 1.0 - (-TAU * s.tone_hz.min(0.45 * self.fs) / self.fs).exp();
        let tone_on = s.tone_hz < 19900.0;
        let sidechain = s.source == Source::Sidechain;

        let delay = LATENCY[self.stages];
        self.dry_pos = (self.dry_pos + 1) & 63;
        let mut out = input;
        let mut wet_mono = 0.0;
        let mut scope_carrier = 0.0;
        let mut scope_wet = 0.0;

        for ch in 0..channels {
            let mut x = [0.0; MAX_FACTOR];
            self.os[ch].upsample(input[ch], &mut x);
            let mut side_os = [0.0; MAX_FACTOR];
            if sidechain {
                self.os_side[ch].upsample(side[ch], &mut side_os);
            }
            let offset = if ch == 1 { 0.5 * s.spread } else { 0.0 };

            let mut y = [0.0; MAX_FACTOR];
            for i in 0..n {
                let (xi, xq) = self.hx[ch].process(x[i], &self.hil);
                let (ci, cq) = if sidechain {
                    self.hc[ch].process(side_os[i], &self.hil)
                } else {
                    let p = (phases[i] as f32 + offset).fract();
                    match s.wave {
                        Wave::Sine => {
                            let (sin, cos) = (TAU * p).sin_cos();
                            (cos, sin)
                        }
                        wave => self.hc[ch].process(shape(wave, p, dt), &self.hil),
                    }
                };
                let ring = match s.circuit {
                    Circuit::Ideal => xi * (ci + s.bias) * bias_gain,
                    Circuit::Diode => diode_ring(xi, level * (ci + s.bias) * bias_gain) * self.norm,
                };
                y[i] = (ring - shift * xq * cq) * shift_gain;
                if ch == 0 && i == 0 {
                    scope_carrier = ci;
                }
            }
            let mut wet = self.os[ch].downsample(&mut y);
            if tone_on {
                for stage in &mut self.tone[ch] {
                    *stage += (wet - *stage) * tone_k;
                    wet = *stage;
                }
            } else {
                self.tone[ch] = [wet; 2];
            }

            self.dry[ch][self.dry_pos] = input[ch];
            let dry = self.dry[ch][self.dry_pos.wrapping_sub(delay) & 63];
            out[ch] = dry + (wet - dry) * s.mix;
            wet_mono += wet / channels as f32;
            if ch == 0 {
                scope_wet = wet;
            }
        }

        self.publish(mono, wet_mono, input[0], scope_carrier, scope_wet, wrapped, carrier, target, base, env);
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn publish(&mut self, input: f32, wet: f32, scope_in: f32, scope_carrier: f32, scope_wet: f32, wrapped: bool, carrier: f32, target: f32, base: f32, env: f32) {
        let m = &self.meter;
        m.carrier_hz.store(carrier, Ordering::Relaxed);
        m.target_hz.store(target, Ordering::Relaxed);
        m.base_hz.store(base, Ordering::Relaxed);
        m.env.store(env, Ordering::Relaxed);
        self.analyzer.push(input, wet, m);

        let sc = &mut self.scope;
        if !sc.recording {
            sc.idle += 1;
            // Start on the carrier's wrap, or after a while if it is too slow to wrap.
            if wrapped || sc.idle as f32 > 0.06 * self.fs {
                let span = (3.0 / carrier).clamp(0.002, 0.04);
                sc.every = ((span * self.fs / SCOPE as f32).round() as usize).clamp(1, 64);
                sc.recording = true;
                sc.index = 0;
                sc.skip = 0;
                sc.writing = 1 - m.scope_ready.load(Ordering::Relaxed);
                m.scope_ms.store(sc.every as f32 * SCOPE as f32 / self.fs * 1000.0, Ordering::Relaxed);
            }
        }
        if sc.recording {
            if sc.skip == 0 {
                let frame = &m.scope[sc.writing];
                frame[0][sc.index].store(scope_in, Ordering::Relaxed);
                frame[1][sc.index].store(scope_carrier, Ordering::Relaxed);
                frame[2][sc.index].store(scope_wet, Ordering::Relaxed);
                sc.index += 1;
                if sc.index == SCOPE {
                    m.scope_ready.store(sc.writing, Ordering::Release);
                    sc.recording = false;
                    sc.idle = 0;
                }
            }
            sc.skip = (sc.skip + 1) % sc.every;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FS: f32 = 48000.0;

    fn sine(f: f32, n: usize, amp: f32) -> Vec<f32> {
        (0..n).map(|i| amp * (TAU * f * i as f32 / FS).sin()).collect()
    }

    fn noise(n: usize) -> Vec<f32> {
        let mut s = 0x1234_5678u32;
        (0..n)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                (s as f32 / u32::MAX as f32 - 0.5) * 1.8
            })
            .collect()
    }

    fn run(fx: &mut RingMod, x: &[f32], s: &Settings) -> Vec<f32> {
        x.iter().map(|&v| fx.process([v, v], [0.0, 0.0], s)[0]).collect()
    }

    /// Amplitude of the component at `f` in the second half of `x` (so the filters have settled).
    fn level(x: &[f32], f: f32) -> f32 {
        let tail = &x[x.len() / 2..];
        let (mut re, mut im) = (0.0, 0.0);
        for (i, v) in tail.iter().enumerate() {
            let a = TAU * f * (i + x.len() / 2) as f32 / FS;
            re += v * a.cos();
            im += v * a.sin();
        }
        2.0 * (re * re + im * im).sqrt() / tail.len() as f32
    }

    fn db(x: f32) -> f32 {
        20.0 * (x + 1e-9).log10()
    }

    fn settings() -> Settings {
        Settings { circuit: Circuit::Ideal, freq_hz: 300.0, ..Settings::default() }
    }

    #[test]
    fn oversampler_latency_matches_the_table() {
        for stages in 0..=MAX_STAGES {
            let mut o = Oversampler::new();
            o.set_stages(stages);
            let mut peak = (0, 0.0_f32);
            for i in 0..200 {
                let mut buf = [0.0; MAX_FACTOR];
                o.upsample(if i == 0 { 1.0 } else { 0.0 }, &mut buf);
                let y = o.downsample(&mut buf);
                if y.abs() > peak.1 {
                    peak = (i, y.abs());
                }
            }
            assert_eq!(peak.0, LATENCY[stages], "stages {stages}");
            // Unity gain at DC.
            let mut dc = 0.0;
            for _ in 0..200 {
                let mut buf = [0.0; MAX_FACTOR];
                o.upsample(1.0, &mut buf);
                dc = o.downsample(&mut buf);
            }
            assert!((dc - 1.0).abs() < 0.01, "stages {stages}: dc gain {dc}");
        }
    }

    #[test]
    fn mix_zero_is_a_null_test() {
        for stages in 0..=MAX_STAGES {
            let mut fx = RingMod::new();
            fx.set_oversampling(stages);
            let s = Settings { mix: 0.0, ..Settings::default() };
            let x = noise(4000);
            let y = run(&mut fx, &x, &s);
            let d = LATENCY[stages];
            for i in d..x.len() {
                assert_eq!(y[i], x[i - d], "stages {stages} sample {i}");
            }
        }
    }

    #[test]
    fn ideal_ring_makes_sum_and_difference_and_no_carrier() {
        let mut fx = RingMod::new();
        let s = settings();
        let y = run(&mut fx, &sine(1000.0, 24000, 0.5), &s);
        let (sum, diff) = (level(&y, 1300.0), level(&y, 700.0));
        // Each sideband is half the input amplitude.
        assert!((sum - 0.25).abs() < 0.02 && (diff - 0.25).abs() < 0.02, "sum {sum} diff {diff}");
        assert!(db(level(&y, 1000.0)) < db(sum) - 40.0, "input leaked: {}", level(&y, 1000.0));
        assert!(db(level(&y, 300.0)) < db(sum) - 40.0, "carrier leaked: {}", level(&y, 300.0));
    }

    #[test]
    fn shift_up_moves_the_sine_up() {
        let mut fx = RingMod::new();
        let s = Settings { shift: 1.0, ..settings() };
        let y = run(&mut fx, &sine(1000.0, 24000, 0.5), &s);
        let (up, down) = (level(&y, 1300.0), level(&y, 700.0));
        assert!(db(up) > db(down) + 30.0, "up {up} down {down}");
        assert!(up > 0.1, "{up}");
    }

    #[test]
    fn shift_down_moves_the_sine_down() {
        let mut fx = RingMod::new();
        let s = Settings { shift: -1.0, ..settings() };
        let y = run(&mut fx, &sine(1000.0, 24000, 0.5), &s);
        let (up, down) = (level(&y, 1300.0), level(&y, 700.0));
        assert!(db(down) > db(up) + 30.0, "up {up} down {down}");
    }

    #[test]
    fn shifter_holds_its_level() {
        // The single sideband has about the power of the two sidebands of the ring.
        let x = sine(1000.0, 24000, 0.5);
        let ring = run(&mut RingMod::new(), &x, &settings());
        let ssb = run(&mut RingMod::new(), &x, &Settings { shift: 1.0, ..settings() });
        let rms = |v: &[f32]| (v[12000..].iter().map(|x| x * x).sum::<f32>() / 12000.0).sqrt();
        assert!((db(rms(&ssb)) - db(rms(&ring))).abs() < 1.0, "{} vs {}", rms(&ssb), rms(&ring));
    }

    #[test]
    fn bias_leaks_the_input_back_in() {
        let mut fx = RingMod::new();
        let s = Settings { bias: 1.0, ..settings() };
        let y = run(&mut fx, &sine(1000.0, 24000, 0.5), &s);
        // Full bias is amplitude modulation: x * (1 + c) / 2, so the input comes back at half.
        assert!((level(&y, 1000.0) - 0.25).abs() < 0.02, "{}", level(&y, 1000.0));
        assert!((level(&y, 1300.0) - 0.125).abs() < 0.01, "{}", level(&y, 1300.0));
    }

    #[test]
    fn diode_ring_is_a_ring_with_extra_harmonics() {
        let x = sine(1000.0, 24000, 0.4);
        let ideal = run(&mut RingMod::new(), &x, &settings());
        let diode = run(&mut RingMod::new(), &x, &Settings { circuit: Circuit::Diode, ..settings() });
        // Sidebands at about the same level, thanks to the normalisation.
        let (a, b) = (level(&ideal, 1300.0), level(&diode, 1300.0));
        assert!((db(a) - db(b)).abs() < 3.0, "ideal {a} diode {b}");
        // Still no carrier or input to speak of, but the diodes add intermodulation (x + 3c).
        assert!(db(level(&diode, 1000.0)) < db(b) - 30.0);
        assert!(level(&diode, 1900.0) > level(&ideal, 1900.0) * 3.0 + 1e-4);
    }

    #[test]
    fn diode_drive_changes_the_character_not_the_level_much() {
        let x = sine(1000.0, 24000, 0.2);
        for drive in [0.0, 0.2, 0.5, 1.0] {
            let y = run(&mut RingMod::new(), &x, &Settings { circuit: Circuit::Diode, drive, ..settings() });
            let sum = level(&y, 1300.0);
            assert!(sum > 0.04 && sum < 0.4, "drive {drive}: {sum}");
        }
    }

    #[test]
    fn oversampling_removes_aliasing() {
        // 15 kHz times 15 kHz has a sum at 30 kHz, which folds to 18 kHz at 48 kHz.
        let x = sine(15000.0, 24000, 0.5);
        let s = Settings { freq_hz: 15000.0, ..settings() };
        let mut plain = RingMod::new();
        plain.set_oversampling(0);
        let mut over = RingMod::new();
        over.set_oversampling(2);
        let a = level(&run(&mut plain, &x, &s), 18000.0);
        let b = level(&run(&mut over, &x, &s), 18000.0);
        assert!(a > 0.2, "{a}");
        assert!(db(b) < db(a) - 30.0, "aliased {a}, oversampled {b}");
    }

    #[test]
    fn every_setting_stays_finite() {
        let x = noise(6000);
        let side = noise(6000);
        for source in [Source::Free, Source::Midi, Source::Track, Source::Sidechain] {
            for circuit in [Circuit::Diode, Circuit::Ideal] {
                for wave in [Wave::Sine, Wave::Triangle, Wave::Square, Wave::Saw] {
                    for (shift, bias, drive, follow, wobble) in [(0.0, 0.0, 0.4, 0.0, 0.0), (1.0, 1.0, 1.0, 1.0, 1.0), (-1.0, 0.5, 0.0, -1.0, 1.0)] {
                        let mut fx = RingMod::new();
                        fx.note_on(60);
                        let s = Settings {
                            source, circuit, wave, shift, bias, drive, follow, wobble,
                            wobble_hz: 8.0,
                            freq_hz: 7000.0,
                            ratio: 8.0,
                            spread: 1.0,
                            tone_hz: 800.0,
                            ..Settings::default()
                        };
                        for i in 0..x.len() {
                            let o = fx.process([x[i], -x[i]], [side[i], side[i]], &s);
                            assert!(o[0].is_finite() && o[1].is_finite() && o[0].abs() < 8.0, "{source:?} {circuit:?} {wave:?}: {o:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn mono_runs_alone() {
        let mut fx = RingMod::new();
        fx.configure(FS, 1);
        let y: Vec<f32> = sine(500.0, 4000, 0.5).iter().map(|&v| fx.process([v, 0.0], [0.0; 2], &Settings::default())[0]).collect();
        assert!(y.iter().all(|v| v.is_finite()) && y.iter().any(|v| v.abs() > 0.05));
    }

    #[test]
    fn sidechain_is_the_carrier() {
        let x = sine(1000.0, 24000, 0.5);
        let side = sine(300.0, 24000, 0.9);
        let s = Settings { source: Source::Sidechain, ..settings() };
        let mut fx = RingMod::new();
        let y: Vec<f32> = (0..x.len()).map(|i| fx.process([x[i], x[i]], [side[i], side[i]], &s)[0]).collect();
        let (sum, diff) = (level(&y, 1300.0), level(&y, 700.0));
        // Half of 0.5 * 0.9 each.
        assert!((sum - 0.225).abs() < 0.03 && (diff - 0.225).abs() < 0.03, "sum {sum} diff {diff}");
    }

    #[test]
    fn square_carrier_adds_odd_harmonic_sidebands() {
        let x = sine(1000.0, 24000, 0.5);
        let sine_y = run(&mut RingMod::new(), &x, &settings());
        let sq_y = run(&mut RingMod::new(), &x, &Settings { wave: Wave::Square, ..settings() });
        // Third harmonic of the carrier: 1000 + 900.
        assert!(level(&sq_y, 1900.0) > 0.03, "{}", level(&sq_y, 1900.0));
        assert!(level(&sine_y, 1900.0) < 0.005);
    }

    #[test]
    fn tracking_follows_the_input_pitch() {
        let mut fx = RingMod::new();
        let s = Settings { source: Source::Track, ratio: 2.0, ..settings() };
        let x: Vec<f32> = (0..48000).map(|i| 0.4 * (TAU * 220.0 * i as f32 / FS).sin() + 0.2 * (TAU * 440.0 * i as f32 / FS).sin()).collect();
        run(&mut fx, &x, &s);
        let carrier = fx.meter.carrier_hz.load(Ordering::Relaxed);
        assert!((carrier - 440.0).abs() < 5.0, "{carrier}");
        assert!((fx.meter.base_hz.load(Ordering::Relaxed) - 220.0).abs() < 3.0);
    }

    #[test]
    fn midi_note_sets_the_carrier() {
        let mut fx = RingMod::new();
        fx.note_on(69);
        let s = Settings { source: Source::Midi, ratio: 0.5, ..settings() };
        run(&mut fx, &sine(1000.0, 24000, 0.3), &s);
        assert!((fx.meter.carrier_hz.load(Ordering::Relaxed) - 220.0).abs() < 1.0);
    }

    #[test]
    fn follow_bends_the_carrier_with_level() {
        let s = Settings { follow: 0.5, ..settings() };
        let mut loud = RingMod::new();
        run(&mut loud, &sine(1000.0, 12000, 0.6), &s);
        let mut quiet = RingMod::new();
        run(&mut quiet, &sine(1000.0, 12000, 0.01), &s);
        let (l, q) = (loud.meter.carrier_hz.load(Ordering::Relaxed), quiet.meter.carrier_hz.load(Ordering::Relaxed));
        assert!(l > 1.8 * q, "loud {l} quiet {q}");
    }

    #[test]
    fn spectra_and_scope_are_published() {
        let mut fx = RingMod::new();
        run(&mut fx, &sine(1000.0, 24000, 0.5), &settings());
        let m = &fx.meter;
        let bin = |hz: f32| (hz * FFT_N as f32 / FS).round() as usize;
        let inp = m.spec_in[bin(1000.0)].load(Ordering::Relaxed);
        assert!((inp - db(0.5)).abs() < 1.5, "{inp}");
        let wet = m.spec_out[bin(1300.0)].load(Ordering::Relaxed);
        assert!((wet - db(0.25)).abs() < 2.0, "{wet}");
        assert!(m.spec_out[bin(1000.0)].load(Ordering::Relaxed) < wet - 30.0);
        let ready = m.scope_ready.load(Ordering::Relaxed);
        let carrier: f32 = (0..SCOPE).map(|i| m.scope[ready][1][i].load(Ordering::Relaxed).abs()).fold(0.0, f32::max);
        assert!(carrier > 0.9, "{carrier}");
    }
}
