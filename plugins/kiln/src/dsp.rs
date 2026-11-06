//! Saturation DSP: a three-band split, and per band a 4x oversampled waveshaper wrapped in
//! pre/de-emphasis filters so each style saturates some frequencies harder than others.
//!
//! Around that core sits a tube-console chain: input transformer, calibration tilt, a series
//! pentode and triode ahead of each band's curve, a parallel 12AX7 path that only saturates
//! above a chosen frequency, an air shelf and an output transformer.

use nih_plug::prelude::{AtomicF32, Enum};
use std::f32::consts::{FRAC_1_SQRT_2, PI};

/// Points kept for the live scatter on the transfer curve, and the spacing between them.
pub const SCOPE_N: usize = 160;
pub const SCOPE_STRIDE: usize = 5;

const STAGE1_TAPS: usize = 65;
const STAGE2_TAPS: usize = 33;
/// Round-trip delay of the 4x oversampler, in base-rate samples. The dry path is delayed by this much.
pub const LATENCY: usize = (STAGE1_TAPS - 1) / 2 + (STAGE2_TAPS - 1) / 4;
const RING: usize = 128;
/// Corner of the first stage as a fraction of its 2x rate. Half-band is 0.25; a little lower
/// trades some top-octave level for rejecting harmonics that would fold back.
const STAGE1_CUTOFF: f32 = 0.235;

#[derive(Enum, Debug, Clone, Copy, PartialEq)]
pub enum Style {
    Tube,
    Tape,
    Iron,
    Diode,
}

impl Style {
    pub const ALL: [Style; 4] = [Style::Tube, Style::Tape, Style::Iron, Style::Diode];

    pub fn name(self) -> &'static str {
        match self {
            Style::Tube => "TUBE",
            Style::Tape => "TAPE",
            Style::Iron => "IRON",
            Style::Diode => "DIODE",
        }
    }

    /// Asymmetry the style has on its own, before the Asym knob.
    fn native_asym(self) -> f32 {
        match self {
            Style::Tube => 0.5,
            Style::Tape => 0.0,
            Style::Iron => 0.2,
            Style::Diode => 0.35,
        }
    }

    /// Pre-emphasis as `(corner Hz, high-frequency gain)`. The signal is shaped like this before
    /// the curve and the inverse is applied after, so the curve sees more of whatever is boosted.
    fn emphasis(self) -> (f32, f32) {
        match self {
            Style::Tube => (1000.0, 1.0),
            Style::Tape => (2500.0, 2.8),
            Style::Iron => (350.0, 0.3),
            Style::Diode => (1200.0, 2.0),
        }
    }

    /// The symmetric curve: slope 1 at the origin, ceiling at 1.
    fn core(self, x: f32) -> f32 {
        match self {
            Style::Tube => x.tanh(),
            Style::Tape => (PI / 2.0 * x).atan() * (2.0 / PI),
            Style::Iron => x / (1.0 + x * x).sqrt(),
            Style::Diode => x / (1.0 + x.abs().powi(4)).powf(0.25),
        }
    }
}

#[derive(Enum, Debug, Clone, Copy, PartialEq)]
pub enum Voice {
    Warm,
    Hot,
}

impl Voice {
    /// `(gain, positive ceiling, negative ceiling)` of the 12AX7 stage.
    fn stage(self) -> (f32, f32, f32) {
        match self {
            Voice::Warm => (3.0, 1.0, 0.75),
            Voice::Hot => (9.0, 1.0, 0.45),
        }
    }
}

/// The hardware's internal trim: tilts the highs before everything else.
#[derive(Enum, Debug, Clone, Copy, PartialEq)]
pub enum Calibration {
    Dark,
    Normal,
    Bright,
}

impl Calibration {
    fn shelf_gain(self) -> f32 {
        match self {
            Calibration::Dark => 0.6,
            Calibration::Normal => 1.0,
            Calibration::Bright => 1.6,
        }
    }
}

/// Level at which the tube stages keep their output, however hard they are pushed.
const TUBE_REF: f32 = 0.5;
/// Pentode: a sharp knee and odd harmonics.
fn pentode_curve(v: f32) -> f32 {
    v / (1.0 + v.abs().powi(3)).cbrt()
}
/// Triode: a soft knee with unequal ceilings, so it adds even harmonics.
fn triode_curve(v: f32) -> f32 {
    if v >= 0.0 {
        1.2 * (v / 1.2).tanh()
    } else {
        0.8 * (v / 0.8).tanh()
    }
}

/// One tube stage. Gain `g` pushes the curve; the output is scaled so a `TUBE_REF` signal
/// comes out at the same level, which makes density a change of colour and not of volume.
/// Below a hair of gain it is exactly a wire.
fn tube_stage(curve: fn(f32) -> f32, x: f32, g: f32) -> f32 {
    if g < 1e-3 {
        return x;
    }
    curve(g * x) * TUBE_REF / curve(g * TUBE_REF)
}

/// Pentode into triode, with their gains.
pub fn tube_stack(x: f32, gains: [f32; 2]) -> f32 {
    tube_stage(triode_curve, tube_stage(pentode_curve, x, gains[0]), gains[1])
}

/// The 12AX7 with unity gain for small signals, so `curve(a) - a` is purely what it adds.
fn sat_curve(voice: Voice, a: f32) -> f32 {
    let (g, cp, cn) = voice.stage();
    let c = if a >= 0.0 { cp } else { cn };
    let v = g * a / c;
    let y = match voice {
        Voice::Warm => v.tanh(),
        Voice::Hot => v / (1.0 + v.abs().powi(3)).cbrt(),
    };
    c * y / g
}

/// Positive and negative ceilings. Unequal ceilings are what give a curve even harmonics.
pub fn ceilings(style: Style, asym: f32) -> (f32, f32) {
    let a = (style.native_asym() + asym).clamp(-1.0, 1.0);
    (1.0 + 0.5 * a, 1.0 - 0.5 * a)
}

/// The full waveshaper. `x` already includes the drive.
pub fn shape(style: Style, x: f32, ceil_pos: f32, ceil_neg: f32) -> f32 {
    let c = if x >= 0.0 { ceil_pos } else { ceil_neg };
    c * style.core(x / c)
}

#[derive(Clone, Copy)]
pub struct Settings {
    pub style: Style,
    pub asym: f32,
    /// -1..1. Positive: louder input drives harder. Negative: louder input drives softer.
    pub dynamics: f32,
    /// Pentode and triode gains. Zero is a wire.
    pub tubes: [f32; 2],
    /// Level of the parallel 12AX7 path.
    pub sat: f32,
    pub voice: Voice,
    /// Transformer amount for both ends.
    pub xfmr: f32,
}

/// Magnitudes the audio thread publishes for the editor.
pub struct Scope {
    pub input: [[AtomicF32; SCOPE_N]; 3],
    pub output: [[AtomicF32; SCOPE_N]; 3],
    pub level: [AtomicF32; 3],
}

impl Default for Scope {
    fn default() -> Self {
        let ring = || std::array::from_fn(|_| std::array::from_fn(|_| AtomicF32::new(0.0)));
        Self { input: ring(), output: ring(), level: std::array::from_fn(|_| AtomicF32::new(0.0)) }
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
    /// Second-order Butterworth. Keeps the state, so the corner can move while audio runs.
    fn set(&mut self, fs: f32, fc: f32, high: bool) {
        let w0 = 2.0 * PI * fc.min(fs * 0.45) / fs;
        let (sin, cos) = w0.sin_cos();
        let alpha = sin * FRAC_1_SQRT_2;
        let a0 = 1.0 + alpha;
        let (b0, b1) = if high { ((1.0 + cos) / 2.0, -(1.0 + cos)) } else { ((1.0 - cos) / 2.0, 1.0 - cos) };
        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b0 / a0;
        self.a1 = -2.0 * cos / a0;
        self.a2 = (1.0 - alpha) / a0;
    }

    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// Linkwitz-Riley 4th order: two cascaded Butterworth sections.
#[derive(Clone, Copy, Default)]
struct Lr4([Biquad; 2]);

impl Lr4 {
    fn set(&mut self, fs: f32, fc: f32, high: bool) {
        for b in &mut self.0 {
            b.set(fs, fc, high);
        }
    }

    fn process(&mut self, x: f32) -> f32 {
        let y = self.0[0].process(x);
        self.0[1].process(y)
    }
}

/// Three-way split whose bands sum back to an allpass-filtered copy of the input.
#[derive(Clone, Copy, Default)]
struct Crossover {
    lp1: Lr4,
    hp1: Lr4,
    lp2: Lr4,
    hp2: Lr4,
    /// The low band also passes the upper crossover so all three bands share one phase response.
    ap_lp: Lr4,
    ap_hp: Lr4,
}

impl Crossover {
    fn set(&mut self, fs: f32, f1: f32, f2: f32) {
        self.lp1.set(fs, f1, false);
        self.hp1.set(fs, f1, true);
        for (lp, hp) in [(&mut self.lp2, &mut self.hp2), (&mut self.ap_lp, &mut self.ap_hp)] {
            lp.set(fs, f2, false);
            hp.set(fs, f2, true);
        }
    }

    fn split(&mut self, x: f32) -> [f32; 3] {
        let low = self.lp1.process(x);
        let rest = self.hp1.process(x);
        let low = self.ap_lp.process(low) + self.ap_hp.process(low);
        [low, self.lp2.process(rest), self.hp2.process(rest)]
    }
}

/// First-order filter with a zero and a pole.
#[derive(Clone, Copy)]
struct Filter1 {
    b0: f32,
    b1: f32,
    a1: f32,
    z: f32,
}

impl Filter1 {
    const FLAT: Self = Self { b0: 1.0, b1: 0.0, a1: 0.0, z: 0.0 };

    /// Gain 1 at DC, `gain` at high frequencies, geometrically centred on `f0`.
    fn shelf(fs: f32, f0: f32, gain: f32) -> Self {
        let r = gain.sqrt();
        let c = 1.0 / (PI * f0 / fs).tan();
        let d0 = 1.0 + c / r;
        Self { b0: (1.0 + r * c) / d0, b1: (1.0 - r * c) / d0, a1: (1.0 - c / r) / d0, z: 0.0 }
    }

    /// The exact inverse. Stable because the original is minimum phase.
    fn inverse(&self) -> Self {
        Self { b0: 1.0 / self.b0, b1: self.a1 / self.b0, a1: self.b1 / self.b0, z: 0.0 }
    }

    /// Swap in new coefficients but keep the running state, so changes do not click.
    fn retune(&mut self, new: Self) {
        *self = Self { z: self.z, ..new };
    }

    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z;
        self.z = self.b1 * x - self.a1 * y;
        y
    }
}

/// Audio transformer: its core saturates the lows first, and it leans a little into them.
#[derive(Clone, Copy, Default)]
struct Transformer {
    low: f32,
    coef: f32,
}

impl Transformer {
    fn new(fs: f32, corner: f32) -> Self {
        Self { low: 0.0, coef: 1.0 - (-2.0 * PI * corner / fs).exp() }
    }

    fn process(&mut self, x: f32, amount: f32) -> f32 {
        self.low += self.coef * (x - self.low);
        let k = 1.0 + 3.0 * amount;
        let flux = (self.low * k).tanh() / k;
        x + amount * (flux - self.low + 0.3 * self.low)
    }
}

/// Windowed-sinc lowpass with unity DC gain. `cutoff` is a fraction of the output rate.
fn kernel<const N: usize>(cutoff: f32) -> [f32; N] {
    let mut h = [0.0; N];
    let mid = (N - 1) as f32 / 2.0;
    let n = (N - 1) as f32;
    let mut sum = 0.0;
    for (j, tap) in h.iter_mut().enumerate() {
        let t = j as f32 - mid;
        let sinc = if t == 0.0 { 1.0 } else { (2.0 * PI * cutoff * t).sin() / (2.0 * PI * cutoff * t) };
        let w = 0.42 - 0.5 * (2.0 * PI * j as f32 / n).cos() + 0.08 * (4.0 * PI * j as f32 / n).cos();
        *tap = sinc * w;
        sum += *tap;
    }
    for tap in &mut h {
        *tap /= sum;
    }
    h
}

/// One 2x up/down stage (polyphase on the way up).
#[derive(Clone)]
struct Stage<const N: usize> {
    h: [f32; N],
    up: [f32; RING],
    down: [f32; RING],
    up_pos: usize,
    down_pos: usize,
}

impl<const N: usize> Stage<N> {
    fn new(cutoff: f32) -> Self {
        Self { h: kernel::<N>(cutoff), up: [0.0; RING], down: [0.0; RING], up_pos: 0, down_pos: 0 }
    }

    fn up(&mut self, x: f32) -> [f32; 2] {
        self.up_pos = (self.up_pos + 1) & (RING - 1);
        self.up[self.up_pos] = x;
        let mut out = [0.0f32; 2];
        for (j, tap) in self.h.iter().enumerate() {
            out[j & 1] += tap * self.up[self.up_pos.wrapping_sub(j >> 1) & (RING - 1)];
        }
        [out[0] * 2.0, out[1] * 2.0]
    }

    fn down(&mut self, a: f32, b: f32) -> f32 {
        // Keep the earlier sample of the pair: that is the one the filter's centre tap lines up with.
        self.down_pos = (self.down_pos + 1) & (RING - 1);
        self.down[self.down_pos] = a;
        let mut y = 0.0;
        for (j, tap) in self.h.iter().enumerate() {
            y += tap * self.down[self.down_pos.wrapping_sub(j) & (RING - 1)];
        }
        self.down_pos = (self.down_pos + 1) & (RING - 1);
        self.down[self.down_pos] = b;
        y
    }
}

/// 4x oversampling as two cascaded 2x stages. The first is sharp, the second can be short.
#[derive(Clone)]
struct Oversampler {
    s1: Stage<STAGE1_TAPS>,
    s2: Stage<STAGE2_TAPS>,
}

impl Oversampler {
    fn new() -> Self {
        Self { s1: Stage::new(STAGE1_CUTOFF), s2: Stage::new(0.25) }
    }

    fn process(&mut self, x: f32, mut f: impl FnMut(f32) -> f32) -> f32 {
        let mut halves = [0.0; 2];
        for (half, s) in halves.iter_mut().zip(self.s1.up(x)) {
            let [a, b] = self.s2.up(s);
            *half = self.s2.down(f(a), f(b));
        }
        self.s1.down(halves[0], halves[1])
    }
}

/// One band for one channel.
#[derive(Clone)]
struct Band {
    os: Oversampler,
    pre: Filter1,
    post: Filter1,
    style: Style,
    env: f32,
    dc_in: f32,
    dc_out: f32,
    fs: f32,
    attack: f32,
    release: f32,
    dc_pole: f32,
}

impl Band {
    fn new(fs: f32) -> Self {
        let coef = |seconds: f32| 1.0 - (-1.0 / (seconds * fs)).exp();
        let mut band = Self {
            os: Oversampler::new(),
            pre: Filter1::FLAT,
            post: Filter1::FLAT,
            style: Style::Tube,
            env: 0.0,
            dc_in: 0.0,
            dc_out: 0.0,
            fs,
            attack: coef(0.002),
            release: coef(0.12),
            dc_pole: 1.0 - 2.0 * PI * 10.0 / fs,
        };
        band.set_style(Style::Tube);
        band
    }

    fn set_style(&mut self, style: Style) {
        let (f0, gain) = style.emphasis();
        let pre = Filter1::shelf(self.fs * 4.0, f0, gain);
        self.pre.retune(pre);
        self.post.retune(pre.inverse());
        self.style = style;
    }

    fn process(&mut self, x: f32, drive: f32, set: Settings) -> f32 {
        let tubes = set.tubes;
        if set.style != self.style {
            self.set_style(set.style);
        }
        let level = x.abs();
        self.env += (if level > self.env { self.attack } else { self.release }) * (level - self.env);
        let drive = drive * (set.dynamics * 2.0 * self.env.min(1.5)).exp();
        // Below unity drive the curve is nearly linear, so lift it back to unity gain.
        let makeup = 1.0 / drive.min(1.0);
        let (cp, cn) = ceilings(set.style, set.asym);
        let (pre, post, style) = (&mut self.pre, &mut self.post, set.style);
        let y = self.os.process(x, |s| post.process(shape(style, tube_stack(pre.process(s), tubes) * drive, cp, cn) * makeup));
        // The asymmetric curve adds DC.
        let out = y - self.dc_in + self.dc_pole * self.dc_out;
        self.dc_in = y;
        self.dc_out = out;
        out
    }
}

pub struct Rendered {
    /// Sum of the three bands after the tone tilt.
    pub wet: f32,
    pub input: [f32; 3],
    pub output: [f32; 3],
}

#[derive(Clone)]
pub struct Channel {
    fs: f32,
    xo: Crossover,
    bands: [Band; 3],
    tone: Filter1,
    tone_trim: f32,
    cal: Filter1,
    air: Filter1,
    xfmr_in: Transformer,
    xfmr_out: Transformer,
    sat_os: Oversampler,
    sat_hp_in: Biquad,
    sat_hp_out: Biquad,
}

impl Channel {
    pub fn new(fs: f32) -> Self {
        let mut c = Self {
            fs,
            xo: Crossover::default(),
            bands: std::array::from_fn(|_| Band::new(fs)),
            tone: Filter1::FLAT,
            tone_trim: 1.0,
            cal: Filter1::FLAT,
            air: Filter1::FLAT,
            xfmr_in: Transformer::new(fs, 90.0),
            xfmr_out: Transformer::new(fs, 60.0),
            sat_os: Oversampler::new(),
            sat_hp_in: Biquad::default(),
            sat_hp_out: Biquad::default(),
        };
        c.set_sat_freq(1500.0);
        c
    }

    pub fn set_crossover(&mut self, low: f32, high: f32) {
        self.xo.set(self.fs, low, high);
    }

    /// `amount` of -1..1 tilts the wet signal around 1 kHz by up to 6 dB at each end.
    pub fn set_tone(&mut self, amount: f32) {
        let gain = 10.0f32.powf(amount * 12.0 / 20.0);
        self.tone.retune(Filter1::shelf(self.fs, 1000.0, gain));
        self.tone_trim = 1.0 / gain.sqrt();
    }

    pub fn set_calibration(&mut self, cal: Calibration) {
        self.cal.retune(Filter1::shelf(self.fs, 2500.0, cal.shelf_gain()));
    }

    /// `amount` of 0..1 lifts everything above about 10 kHz by up to 10 dB.
    pub fn set_air(&mut self, amount: f32) {
        self.air.retune(Filter1::shelf(self.fs, 10000.0, 10.0f32.powf(amount * 10.0 / 20.0)));
    }

    /// Corner of the highpass that feeds, and cleans up after, the 12AX7 path.
    pub fn set_sat_freq(&mut self, hz: f32) {
        self.sat_hp_in.set(self.fs, hz, true);
        self.sat_hp_out.set(self.fs, hz, true);
    }

    pub fn process(&mut self, x: f32, drive: [f32; 3], set: Settings) -> Rendered {
        let x = self.cal.process(self.xfmr_in.process(x, set.xfmr));
        let input = self.xo.split(x);
        let output: [f32; 3] = std::array::from_fn(|b| self.bands[b].process(input[b], drive[b], set));

        // The 12AX7 path: only the highs go in, only what the tube adds comes out. The oversampler
        // delays it by the same amount as the bands, so it lines up.
        let voice = set.voice;
        let high = self.sat_hp_in.process(x);
        let added = self.sat_os.process(high, |a| sat_curve(voice, a) - a);
        let added = self.sat_hp_out.process(added);

        let wet = self.tone.process(output.iter().sum()) * self.tone_trim + set.sat * added;
        let wet = self.xfmr_out.process(self.air.process(wet), set.xfmr);
        Rendered { wet, input, output }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FS: f32 = 48000.0;

    fn sine(f: f32, amp: f32, n: usize) -> Vec<f32> {
        (0..n).map(|i| amp * (2.0 * PI * f * i as f32 / FS).sin()).collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    /// Amplitude of one frequency (single-bin DFT). `f` should fit a whole number of cycles.
    fn tone(x: &[f32], f: f32) -> f32 {
        let (mut re, mut im) = (0.0, 0.0);
        for (n, v) in x.iter().enumerate() {
            let w = 2.0 * PI * f * n as f32 / FS;
            re += v * w.cos();
            im -= v * w.sin();
        }
        2.0 * (re * re + im * im).sqrt() / x.len() as f32
    }

    fn settings(style: Style) -> Settings {
        Settings { style, asym: 0.0, dynamics: 0.0, tubes: [0.0; 2], sat: 0.0, voice: Voice::Warm, xfmr: 0.0 }
    }

    fn run(channel: &mut Channel, input: &[f32], db: f32, set: Settings) -> Vec<f32> {
        let g = 10.0f32.powf(db / 20.0);
        input.iter().map(|&x| channel.process(x, [g; 3], set).wet).collect()
    }

    fn channel() -> Channel {
        let mut c = Channel::new(FS);
        c.set_crossover(150.0, 2500.0);
        c
    }

    #[test]
    fn crossover_sums_flat() {
        let mut xo = Crossover::default();
        xo.set(FS, 150.0, 2500.0);
        for f in [60.0, 150.0, 1000.0, 2500.0, 6000.0, 15000.0] {
            let input = sine(f, 0.5, 24000);
            let out: Vec<f32> = input.iter().map(|&x| xo.split(x).iter().sum()).collect();
            let ratio = rms(&out[12000..]) / rms(&input[12000..]);
            assert!((ratio - 1.0).abs() < 0.02, "{f} Hz came out at {ratio}");
        }
    }

    #[test]
    fn oversampler_latency_matches_constant() {
        let mut os = Oversampler::new();
        let out: Vec<f32> = (0..200).map(|i| os.process(if i == 0 { 1.0 } else { 0.0 }, |s| s)).collect();
        let peak = out.iter().enumerate().max_by(|a, b| a.1.abs().total_cmp(&b.1.abs())).unwrap().0;
        assert_eq!(peak, LATENCY);
    }

    #[test]
    fn oversampler_keeps_the_top_octave() {
        for (f, tolerance_db) in [(12000.0, 0.05), (18000.0, 0.1), (20000.0, 1.0)] {
            let mut os = Oversampler::new();
            let input = sine(f, 0.1, 24000);
            let out: Vec<f32> = input.iter().map(|&x| os.process(x, |s| s)).collect();
            let db = 20.0 * (rms(&out[12000..]) / rms(&input[12000..])).log10();
            assert!(db.abs() < tolerance_db, "{f} Hz changed by {db} dB");
        }
    }

    #[test]
    fn low_drive_is_nearly_transparent() {
        for f in [100.0, 1000.0, 8000.0, 15000.0] {
            let input = sine(f, 0.1, 24000);
            let out = run(&mut channel(), &input, -24.0, settings(Style::Tape));
            let db = 20.0 * (rms(&out[12000..]) / rms(&input[12000..])).log10();
            assert!(db.abs() < 0.5, "{f} Hz changed by {db} dB");
        }
    }

    #[test]
    fn symmetric_styles_make_odd_harmonics_and_tube_adds_even() {
        let input = sine(1000.0, 0.5, 48 * 400);
        let tube = run(&mut channel(), &input, 12.0, settings(Style::Tube));
        let tape = run(&mut channel(), &input, 12.0, Settings { asym: -0.0, ..settings(Style::Tape) });
        let (tube, tape) = (&tube[4800..], &tape[4800..]);
        let h2 = |x: &[f32]| tone(x, 2000.0) / tone(x, 1000.0);
        let h3 = |x: &[f32]| tone(x, 3000.0) / tone(x, 1000.0);
        assert!(h2(tube) > 0.02, "tube 2nd harmonic {}", h2(tube));
        assert!(h2(tape) < 0.005, "tape 2nd harmonic {}", h2(tape));
        assert!(h3(tape) > 0.02, "tape 3rd harmonic {}", h3(tape));
    }

    #[test]
    fn oversampling_cuts_aliasing() {
        let n = 16384;
        let input = sine(9000.0, 0.5, n);
        let set = settings(Style::Tube);
        let wet = run(&mut channel(), &input, 30.0, set);
        // The 3rd harmonic at 27 kHz folds down to 21 kHz.
        let ours = tone(&wet[4096..], 21000.0) / tone(&wet[4096..], 9000.0);
        let (cp, cn) = ceilings(set.style, 0.0);
        let g = 10.0f32.powf(30.0 / 20.0);
        let naive: Vec<f32> = input.iter().map(|&x| shape(set.style, x * g, cp, cn)).collect();
        let theirs = tone(&naive[4096..], 21000.0) / tone(&naive[4096..], 9000.0);
        println!("alias at 21k: {:.1} dB, without oversampling {:.1} dB", 20.0 * ours.log10(), 20.0 * theirs.log10());
        assert!(ours < theirs * 0.1, "oversampled {ours}, naive {theirs}");
    }

    #[test]
    fn stays_finite_and_bounded_at_extremes() {
        let mut c = channel();
        let mut seed = 12345u32;
        for style in Style::ALL {
            for i in 0..20000 {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                let x = (seed >> 8) as f32 / (1 << 23) as f32 * 4.0 - 4.0;
                let set = Settings {
                    asym: 1.0,
                    dynamics: if i % 2 == 0 { 1.0 } else { -1.0 },
                    tubes: [8.0; 2],
                    sat: 1.0,
                    voice: if i % 3 == 0 { Voice::Hot } else { Voice::Warm },
                    xfmr: 1.0,
                    ..settings(style)
                };
                let out = c.process(x, [250.0; 3], set);
                assert!(out.wet.is_finite() && out.wet.abs() < 20.0, "{style:?} gave {}", out.wet);
            }
        }
    }

    #[test]
    fn shelf_inverse_cancels() {
        let pre = Filter1::shelf(FS * 4.0, 2500.0, 2.8);
        let (mut a, mut b) = (pre, pre.inverse());
        for x in sine(3000.0, 0.5, 4000) {
            let y = b.process(a.process(x));
            assert!((y - x).abs() < 1e-3);
        }
    }

    fn db(x: f32) -> f32 {
        20.0 * x.log10()
    }

    #[test]
    fn tube_stages_are_wires_at_zero_and_keep_the_reference_level() {
        for x in [-0.8, -0.1, 0.3, 0.9] {
            assert_eq!(tube_stack(x, [0.0, 0.0]), x);
        }
        for g in [1.0, 3.0, 8.0] {
            assert!((tube_stage(pentode_curve, TUBE_REF, g) - TUBE_REF).abs() < 1e-5);
            assert!((tube_stage(triode_curve, TUBE_REF, g) - TUBE_REF).abs() < 1e-5);
        }
        // Tiny gain is continuous with the wire.
        assert!((tube_stack(0.7, [2e-3, 2e-3]) - 0.7).abs() < 1e-3);
    }

    #[test]
    fn triode_adds_even_harmonics_and_pentode_odd() {
        let input = sine(1000.0, 0.5, 48 * 400);
        let h = |gains: [f32; 2], f: f32| {
            let out: Vec<f32> = input.iter().map(|&x| tube_stack(x, gains)).collect();
            tone(&out[4800..], f) / tone(&out[4800..], 1000.0)
        };
        assert!(h([0.0, 6.0], 2000.0) > 0.03, "triode h2 {}", h([0.0, 6.0], 2000.0));
        assert!(h([6.0, 0.0], 2000.0) < 0.001, "pentode h2 {}", h([6.0, 0.0], 2000.0));
        assert!(h([6.0, 0.0], 3000.0) > 0.03, "pentode h3 {}", h([6.0, 0.0], 3000.0));
    }

    #[test]
    fn density_style_push_keeps_loudness_near_constant() {
        let input = sine(500.0, 0.5, 48 * 200);
        let level = |g: f32| {
            let out: Vec<f32> = input.iter().map(|&x| tube_stack(x, [g, g])).collect();
            rms(&out[4800..])
        };
        let (soft, hard) = (level(1.0), level(6.0));
        assert!((db(hard / soft)).abs() < 1.5, "level moved {} dB", db(hard / soft));
    }

    #[test]
    fn sat_path_adds_harmonics_only_above_its_frequency() {
        let run_sat = |f: f32, freq: f32| {
            let mut c = channel();
            c.set_sat_freq(freq);
            let set = Settings { sat: 1.0, voice: Voice::Hot, ..settings(Style::Tube) };
            let input = sine(f, 0.5, 48 * 400);
            // Strip the dry band path out by comparing against the same channel with sat off.
            let mut base = channel();
            let with: Vec<f32> = input.iter().map(|&x| c.process(x, [1.0; 3], set).wet).collect();
            let without: Vec<f32> = input.iter().map(|&x| base.process(x, [1.0; 3], settings(Style::Tube)).wet).collect();
            let diff: Vec<f32> = with.iter().zip(&without).map(|(a, b)| a - b).collect();
            rms(&diff[4800..])
        };
        let above = run_sat(4000.0, 1500.0);
        let below = run_sat(200.0, 1500.0);
        assert!(above > 0.02, "added {above}");
        assert!(below < above * 0.1, "below the corner added {below}, above {above}");
    }

    #[test]
    fn sat_path_lines_up_with_the_bands() {
        // The harmonic generator and the bands share a latency, so a click through both peaks together.
        let mut c = channel();
        let set = Settings { sat: 1.0, voice: Voice::Hot, ..settings(Style::Tape) };
        let out: Vec<f32> = (0..300).map(|i| c.process(if i == 0 { 0.9 } else { 0.0 }, [1.0; 3], set).wet).collect();
        let peak = out.iter().enumerate().max_by(|a, b| a.1.abs().total_cmp(&b.1.abs())).unwrap().0;
        assert!(peak.abs_diff(LATENCY) <= 12, "peak at {peak}");
    }

    #[test]
    fn transformer_is_a_wire_at_zero_and_bumps_then_squashes_lows() {
        let mut t = Transformer::new(FS, 90.0);
        for x in sine(80.0, 0.5, 4800) {
            assert_eq!(t.process(x, 0.0), x);
        }
        let gain = |amp: f32| {
            let mut t = Transformer::new(FS, 90.0);
            let input = sine(60.0, amp, 48000);
            let out: Vec<f32> = input.iter().map(|&x| t.process(x, 1.0)).collect();
            rms(&out[24000..]) / rms(&input[24000..])
        };
        assert!(gain(0.02) > 1.15, "quiet lows should be lifted, got {}", gain(0.02));
        assert!(gain(0.9) < gain(0.02) - 0.1, "loud lows should saturate");
        let mut t = Transformer::new(FS, 90.0);
        let input = sine(8000.0, 0.5, 4800);
        let out: Vec<f32> = input.iter().map(|&x| t.process(x, 1.0)).collect();
        assert!(db(rms(&out[2400..]) / rms(&input[2400..])).abs() < 0.2, "highs should pass untouched");
    }

    #[test]
    fn air_lifts_only_the_top_and_calibration_tilts_it() {
        let gain = |f: f32, apply: &dyn Fn(&mut Channel)| {
            let mut c = channel();
            apply(&mut c);
            let input = sine(f, 0.1, 24000);
            let out = run(&mut c, &input, -24.0, settings(Style::Tape));
            db(rms(&out[12000..]) / rms(&input[12000..]))
        };
        let flat_hi = gain(14000.0, &|_| {});
        let air_hi = gain(14000.0, &|c| c.set_air(1.0));
        let air_lo = gain(200.0, &|c| c.set_air(1.0));
        assert!(air_hi - flat_hi > 5.0, "air gave {} dB at 14k", air_hi - flat_hi);
        assert!(air_lo.abs() < 0.5, "air moved the lows by {air_lo} dB");
        let dark = gain(10000.0, &|c| c.set_calibration(Calibration::Dark));
        let bright = gain(10000.0, &|c| c.set_calibration(Calibration::Bright));
        assert!(bright - dark > 5.0, "dark {dark}, bright {bright}");
    }
}
