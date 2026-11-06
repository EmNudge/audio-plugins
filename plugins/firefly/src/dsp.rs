//! Opto compressor DSP: an electro-luminescent panel lighting a photocell, then a tube output stage.
//!
//! The sidechain is the input through a gentle high-pass and a high-frequency shelf (the
//! *emphasis*, so bright content lights the panel harder than bass at the same level), a short
//! mean-square window and a soft-knee gain computer. The computer sets how much gain reduction the
//! lamp *asks* for, and the photocell answers slowly and with a memory:
//!
//! * a fast stage (about 8 ms attack, 40 ms release) carries 70% of the reduction, so about half of
//!   it lets go within 60 ms of the signal dropping away, as on the hardware;
//! * a slow stage (300 ms attack) carries the other 30% and releases over 0.6 to 6 s, with a time
//!   constant that grows with how much and how long the cell has been working. A short peak leaves
//!   almost no tail, a loud sustained passage leaves seconds of it.
//!
//! The reduction is smoothed in dB, applied, then the make-up gain feeds a triode-style tube stage
//! with unequal ceilings (even harmonics) that runs at 4x oversampling. Only what the tube *adds* is
//! DC-blocked, so with the tube at zero the stage is an exact wire. The dry path of the mix is
//! delayed by the oversampler's latency.

use nih_plug::prelude::{AtomicF32, Enum};
use std::f32::consts::PI;
use std::sync::atomic::Ordering;
use std::sync::Arc;

pub const MAX_CHANNELS: usize = 2;

const STAGE1_TAPS: usize = 65;
const STAGE2_TAPS: usize = 33;
/// Round-trip delay of the 4x oversampler, in base-rate samples. The dry path is delayed by this much.
pub const LATENCY: usize = (STAGE1_TAPS - 1) / 2 + (STAGE2_TAPS - 1) / 4;
const RING: usize = 128;
/// Corner of the first stage as a fraction of its 2x rate (half-band is 0.25).
const STAGE1_CUTOFF: f32 = 0.235;

/// Threshold at Peak Reduction 0 and how far it falls by 100%, in dB. At 0 even a full-scale
/// signal stays below the knee, so nothing is reduced.
const THRESH_TOP_DB: f32 = 6.0;
const THRESH_SPAN_DB: f32 = 48.0;
const MAX_GR_DB: f32 = 40.0;

/// Share of the reduction carried by the fast stage.
const W_FAST: f32 = 0.7;
const FAST_ATTACK_MS: [f32; 2] = [8.0, 4.0];
const FAST_RELEASE_MS: f32 = 40.0;
const SLOW_ATTACK_MS: f32 = 300.0;
/// Slow release time at an empty and a fully-worked cell, in seconds.
const SLOW_RELEASE_S: [f32; 2] = [0.6, 6.0];
/// How fast the cell's history fills and drains, in seconds, and the reduction that counts as full.
const HIST_UP_S: f32 = 4.0;
const HIST_DOWN_S: f32 = 8.0;
const HIST_FULL_DB: f32 = 15.0;

const DETECTOR_MS: f32 = 3.0;
const SC_HIGHPASS_HZ: f32 = 35.0;
const SHELF_HZ: f32 = 3000.0;
const SHELF_MAX_DB: f32 = 9.0;
const TUBE_DC_HZ: f32 = 8.0;

const METER_FALL_DB_S: f32 = 30.0;
pub const METER_FLOOR_DB: f32 = -96.0;

#[derive(Enum, Debug, Clone, Copy, PartialEq)]
pub enum Mode {
    Compress,
    Limit,
}

impl Mode {
    /// Ratio and knee width in dB.
    pub fn curve(self) -> (f32, f32) {
        match self {
            Mode::Compress => (3.0, 12.0),
            Mode::Limit => (10.0, 6.0),
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// Where the threshold sits for a Peak Reduction setting 0..1, in dB.
pub fn threshold_db(peak_reduction: f32) -> f32 {
    THRESH_TOP_DB - THRESH_SPAN_DB * peak_reduction
}

/// The gain reduction (dB, positive) the lamp asks for at a sidechain level.
pub fn target_reduction_db(level_db: f32, threshold: f32, mode: Mode) -> f32 {
    let (ratio, knee) = mode.curve();
    let over = level_db - threshold;
    let eff = if over <= -knee * 0.5 {
        0.0
    } else if over >= knee * 0.5 {
        over
    } else {
        let t = over + knee * 0.5;
        t * t / (2.0 * knee)
    };
    (eff * (1.0 - 1.0 / ratio)).min(MAX_GR_DB)
}

/// What the audio thread publishes for the editor.
pub struct Meter {
    /// Input and output peak, in dB.
    pub input_db: AtomicF32,
    pub output_db: AtomicF32,
    /// What the lamp is asking for, in dB of reduction.
    pub lamp: AtomicF32,
    /// The two photocell stages' shares of the reduction. They add up to the gain reduction.
    pub fast: AtomicF32,
    pub slow: AtomicF32,
}

impl Meter {
    fn new() -> Self {
        Self {
            input_db: AtomicF32::new(METER_FLOOR_DB),
            output_db: AtomicF32::new(METER_FLOOR_DB),
            lamp: AtomicF32::new(0.0),
            fast: AtomicF32::new(0.0),
            slow: AtomicF32::new(0.0),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Settings {
    /// 0..1: how hard the lamp is driven (lowers the threshold).
    pub peak_reduction: f32,
    /// Make-up gain, in dB.
    pub gain_db: f32,
    pub mode: Mode,
    /// 0..1: high-frequency tilt in the sidechain.
    pub emphasis: f32,
    /// 0..1: tube drive.
    pub tube: f32,
    /// 0..1: how much the channels share one detector.
    pub link: f32,
    /// 0..1 wet.
    pub mix: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self { peak_reduction: 0.5, gain_db: 0.0, mode: Mode::Compress, emphasis: 0.35, tube: 0.3, link: 1.0, mix: 1.0 }
    }
}

fn db_to_lin(db: f32) -> f32 {
    10f32.powf(db * 0.05)
}

fn lin_to_db(x: f32) -> f32 {
    20.0 * x.max(1e-6).log10()
}

/// Smoothing coefficient for a one-pole with time constant `secs`.
fn coef(secs: f32, fs: f32) -> f32 {
    1.0 - (-1.0 / (secs * fs)).exp()
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

/// Triode-style curve with unequal ceilings, so it adds even harmonics. Unity slope at zero.
fn triode(v: f32) -> f32 {
    if v >= 0.0 {
        1.2 * (v / 1.2).tanh()
    } else {
        0.8 * (v / 0.8).tanh()
    }
}

/// What the tube does to a sample: the input pushed by `1 + 2.5 * amount`, curved, and brought back.
fn tube_curve(x: f32, amount: f32) -> f32 {
    if amount < 1e-4 {
        return x;
    }
    let g = 1.0 + 2.5 * amount;
    triode(g * x) / g
}

/// Time constants that depend on the sample rate.
#[derive(Clone, Copy)]
struct Coeffs {
    detector: f32,
    highpass: f32,
    fast_attack: [f32; 2],
    fast_release: f32,
    slow_attack: f32,
    hist_up: f32,
    hist_down: f32,
    tube_dc: f32,
    fs: f32,
}

impl Coeffs {
    fn new(fs: f32) -> Self {
        Self {
            detector: coef(DETECTOR_MS * 1e-3, fs),
            highpass: 1.0 - (-2.0 * PI * SC_HIGHPASS_HZ / fs).exp(),
            fast_attack: [coef(FAST_ATTACK_MS[0] * 1e-3, fs), coef(FAST_ATTACK_MS[1] * 1e-3, fs)],
            fast_release: coef(FAST_RELEASE_MS * 1e-3, fs),
            slow_attack: coef(SLOW_ATTACK_MS * 1e-3, fs),
            hist_up: coef(HIST_UP_S, fs),
            hist_down: coef(HIST_DOWN_S, fs),
            tube_dc: 1.0 - (-2.0 * PI * TUBE_DC_HZ / fs).exp(),
            fs,
        }
    }
}

/// The photocell: two stages and a memory of how hard it has been worked.
#[derive(Clone, Copy, Default)]
struct Cell {
    fast: f32,
    slow: f32,
    history: f32,
}

impl Cell {
    /// Move towards a requested reduction in dB; returns the total reduction.
    fn step(&mut self, target: f32, c: &Coeffs, mode: Mode) -> f32 {
        let a = if target > self.fast { c.fast_attack[mode.index()] } else { c.fast_release };
        self.fast += a * (target - self.fast);

        let h = (target / HIST_FULL_DB).min(1.0);
        self.history += (if h > self.history { c.hist_up } else { c.hist_down }) * (h - self.history);

        let a = if target > self.slow {
            c.slow_attack
        } else {
            let tau = SLOW_RELEASE_S[0] + (SLOW_RELEASE_S[1] - SLOW_RELEASE_S[0]) * self.history;
            1.0 / (tau * c.fs)
        };
        self.slow += a * (target - self.slow);
        self.total()
    }

    fn total(&self) -> f32 {
        self.fast * W_FAST + self.slow * (1.0 - W_FAST)
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


/// One channel's sidechain filters, photocell, tube and oversampler.
#[derive(Clone)]
struct Channel {
    hp_low: f32,
    shelf: Filter1,
    power: f32,
    cell: Cell,
    os: Oversampler,
    tube_dc: f32,
}

impl Channel {
    fn new() -> Self {
        Self { hp_low: 0.0, shelf: Filter1::FLAT, power: 0.0, cell: Cell::default(), os: Oversampler::new(), tube_dc: 0.0 }
    }
}

pub struct Compressor {
    pub meter: Arc<Meter>,
    channels: [Channel; MAX_CHANNELS],
    count: usize,
    c: Coeffs,
    emphasis: f32,
    dry: [[f32; 64]; MAX_CHANNELS],
    pos: usize,
    // Per-buffer accumulators for the meter.
    in_peak: f32,
    out_peak: f32,
    lamp: f32,
    held_in: f32,
    held_out: f32,
}

impl Compressor {
    pub fn new() -> Self {
        Self {
            meter: Arc::new(Meter::new()),
            channels: [Channel::new(), Channel::new()],
            count: 2,
            c: Coeffs::new(44100.0),
            emphasis: -1.0,
            dry: [[0.0; 64]; MAX_CHANNELS],
            pos: 0,
            in_peak: 0.0,
            out_peak: 0.0,
            lamp: 0.0,
            held_in: METER_FLOOR_DB,
            held_out: METER_FLOOR_DB,
        }
    }

    pub fn configure(&mut self, sample_rate: f32, channels: usize) {
        self.c = Coeffs::new(sample_rate);
        self.count = channels.clamp(1, MAX_CHANNELS);
        self.emphasis = -1.0;
        self.reset();
    }

    pub fn reset(&mut self) {
        self.channels = [Channel::new(), Channel::new()];
        self.dry = [[0.0; 64]; MAX_CHANNELS];
        self.pos = 0;
        self.emphasis = -1.0;
    }

    fn set_emphasis(&mut self, emphasis: f32) {
        if (emphasis - self.emphasis).abs() < 0.002 {
            return;
        }
        self.emphasis = emphasis;
        let shelf = Filter1::shelf(self.c.fs, SHELF_HZ, db_to_lin(SHELF_MAX_DB * emphasis));
        for ch in &mut self.channels {
            ch.shelf.retune(shelf);
        }
    }

    pub fn process(&mut self, x: [f32; MAX_CHANNELS], s: &Settings) -> [f32; MAX_CHANNELS] {
        self.set_emphasis(s.emphasis);
        let c = self.c;
        let n = self.count;

        // Sidechain levels, then the linked detector.
        let mut level = [0.0f32; MAX_CHANNELS];
        for i in 0..n {
            let ch = &mut self.channels[i];
            ch.hp_low += c.highpass * (x[i] - ch.hp_low);
            let sc = ch.shelf.process(x[i] - ch.hp_low);
            ch.power += c.detector * (sc * sc - ch.power);
            level[i] = ch.power.sqrt();
        }
        let loudest = level[..n].iter().fold(0.0f32, |m, l| m.max(*l));
        let threshold = threshold_db(s.peak_reduction);
        let makeup = db_to_lin(s.gain_db);
        let amount = s.tube;

        let mut out = [0.0f32; MAX_CHANNELS];
        for i in 0..n {
            let ch = &mut self.channels[i];
            let linked = level[i] + s.link * (loudest - level[i]);
            let target = target_reduction_db(lin_to_db(linked), threshold, s.mode);
            self.lamp = self.lamp.max(target);
            let reduction = ch.cell.step(target, &c, s.mode);
            let gain = (-reduction * std::f32::consts::LN_10 / 20.0).exp() * makeup;

            let y = x[i] * gain;
            let dc = c.tube_dc;
            let tube_dc = &mut ch.tube_dc;
            let wet = ch.os.process(y, |v| {
                let added = tube_curve(v, amount) - v;
                *tube_dc += dc * (added - *tube_dc);
                v + added - *tube_dc
            });

            self.dry[i][self.pos] = x[i];
            let dry = self.dry[i][(self.pos + 64 - LATENCY) % 64];
            out[i] = dry + (wet - dry) * s.mix;
            self.in_peak = self.in_peak.max(x[i].abs());
            self.out_peak = self.out_peak.max(out[i].abs());
        }
        self.pos = (self.pos + 1) % 64;
        out
    }

    /// Call once per buffer: publish the meters.
    pub fn publish(&mut self, frames: usize) {
        let fall = METER_FALL_DB_S * frames as f32 / self.c.fs;
        self.held_in = lin_to_db(self.in_peak).max(self.held_in - fall).max(METER_FLOOR_DB);
        self.held_out = lin_to_db(self.out_peak).max(self.held_out - fall).max(METER_FLOOR_DB);
        self.in_peak = 0.0;
        self.out_peak = 0.0;
        let (mut fast, mut slow) = (0.0f32, 0.0f32);
        for ch in &self.channels[..self.count] {
            fast = fast.max(ch.cell.fast * W_FAST);
            slow = slow.max(ch.cell.slow * (1.0 - W_FAST));
        }
        let m = &self.meter;
        m.input_db.store(self.held_in, Ordering::Relaxed);
        m.output_db.store(self.held_out, Ordering::Relaxed);
        m.lamp.store(self.lamp, Ordering::Relaxed);
        m.fast.store(fast, Ordering::Relaxed);
        m.slow.store(slow, Ordering::Relaxed);
        self.lamp = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 44100.0;

    fn sine(freq: f32, secs: f32, db: f32) -> Vec<f32> {
        let amp = db_to_lin(db);
        (0..(secs * SR) as usize).map(|i| amp * (TAU32 * freq * i as f32 / SR).sin()).collect()
    }
    const TAU32: f32 = std::f32::consts::TAU;

    fn noise(n: usize, amp: f32) -> Vec<f32> {
        let mut state = 12345u32;
        (0..n)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                amp * (state as f32 / u32::MAX as f32 * 2.0 - 1.0)
            })
            .collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    fn run(input: &[f32], s: &Settings) -> Vec<f32> {
        let mut c = Compressor::new();
        c.configure(SR, 1);
        input.iter().map(|&v| c.process([v, 0.0], s)[0]).collect()
    }

    /// Settings with the tube off and no make-up.
    fn clean() -> Settings {
        Settings { tube: 0.0, ..Settings::default() }
    }

    /// Reduction in dB measured over a stretch of a processed sine against its delayed input.
    fn reduction(input: &[f32], out: &[f32], from: usize, to: usize) -> f32 {
        let delayed: Vec<f32> = (from..to).map(|i| input[i - LATENCY]).collect();
        lin_to_db(rms(&delayed)) - lin_to_db(rms(&out[from..to]))
    }

    fn tone(x: &[f32], f: f32) -> f32 {
        let (mut re, mut im) = (0.0, 0.0);
        for (i, v) in x.iter().enumerate() {
            let a = TAU32 * f * i as f32 / SR;
            re += v * a.cos();
            im += v * a.sin();
        }
        2.0 * (re * re + im * im).sqrt() / x.len() as f32
    }

    #[test]
    fn oversampler_latency_matches_constant() {
        let mut os = Oversampler::new();
        let out: Vec<f32> = (0..200).map(|i| os.process(if i == 0 { 1.0 } else { 0.0 }, |s| s)).collect();
        let peak = out.iter().enumerate().max_by(|a, b| a.1.abs().total_cmp(&b.1.abs())).unwrap().0;
        assert_eq!(peak, LATENCY);
    }

    #[test]
    fn null_at_zero_peak_reduction_with_no_tube() {
        let s = Settings { peak_reduction: 0.0, ..clean() };
        // Band-limited and near full scale: the oversampler rolls off the top octave, and nothing
        // below the knee at 0 dBFS may be reduced.
        let (a, b, c) = (sine(300.0, 0.5, -6.0), sine(2100.0, 0.5, -9.0), sine(7300.0, 0.5, -12.0));
        let input: Vec<f32> = (0..a.len()).map(|i| a[i] + b[i] + c[i]).collect();
        let out = run(&input, &s);
        let err: Vec<f32> = (4000..input.len()).map(|i| out[i] - input[i - LATENCY]).collect();
        assert!(rms(&err) < 0.01 * rms(&input), "error {}", rms(&err) / rms(&input));
    }

    #[test]
    fn mix_zero_is_the_input_delayed_by_the_latency() {
        let s = Settings { mix: 0.0, peak_reduction: 1.0, tube: 1.0, ..Settings::default() };
        let input = noise(5000, 0.8);
        let out = run(&input, &s);
        for i in LATENCY..5000 {
            assert!((out[i] - input[i - LATENCY]).abs() < 1e-6);
        }
    }

    #[test]
    fn compresses_and_limit_is_firmer() {
        let input = sine(1000.0, 4.0, -6.0);
        let n = input.len();
        let c = reduction(&input, &run(&input, &Settings { peak_reduction: 0.6, ..clean() }), n - 20000, n);
        let l = reduction(&input, &run(&input, &Settings { peak_reduction: 0.6, mode: Mode::Limit, ..clean() }), n - 20000, n);
        assert!(c > 4.0, "compress reduced {c} dB");
        assert!(l > c + 1.0, "limit {l} dB vs compress {c} dB");
    }

    #[test]
    fn more_peak_reduction_means_more_reduction() {
        let input = sine(1000.0, 3.0, -10.0);
        let n = input.len();
        let mut last = -1.0;
        for pr in [0.2, 0.5, 0.8] {
            let r = reduction(&input, &run(&input, &Settings { peak_reduction: pr, ..clean() }), n - 20000, n);
            assert!(r > last, "pr {pr}: {r} dB after {last} dB");
            last = r;
        }
    }

    /// Reduction at `at` seconds after a loud burst of `burst` seconds, read off a quiet probe.
    fn tail(burst: f32, at: f32) -> f32 {
        let s = Settings { peak_reduction: 0.7, ..clean() };
        let mut input = sine(1000.0, burst, -6.0);
        let release_at = input.len();
        input.extend(sine(1000.0, at + 0.3, -50.0));
        let out = run(&input, &s);
        let from = release_at + (at * SR) as usize;
        reduction(&input, &out, from, from + 2000)
    }

    #[test]
    fn release_is_two_stage_and_remembers_the_program() {
        let short = tail(0.1, 0.5);
        let long = tail(6.0, 0.5);
        assert!(long > short + 1.5, "after 0.5 s: short burst {short} dB, long burst {long} dB");

        // After a long burst the first part lets go quickly, then a tail lingers for seconds.
        let held = tail(6.0, 0.0);
        let soon = tail(6.0, 0.1);
        let later = tail(6.0, 1.5);
        assert!(soon < held * 0.75, "0.1 s: {soon} dB of {held} dB");
        assert!(later > 1.0 && later < soon, "1.5 s: {later} dB");
    }

    #[test]
    fn emphasis_makes_highs_drive_harder() {
        let level = -18.0;
        let measure = |f: f32, emphasis: f32| {
            let input = sine(f, 2.0, level);
            let n = input.len();
            let s = Settings { peak_reduction: 0.5, emphasis, ..clean() };
            reduction(&input, &run(&input, &s), n - 10000, n)
        };
        let flat = measure(8000.0, 0.0) - measure(200.0, 0.0);
        let tilted = measure(8000.0, 1.0) - measure(200.0, 1.0);
        assert!(flat.abs() < 1.0, "flat sidechain differs by {flat} dB");
        assert!(tilted > flat + 2.0, "tilted {tilted} dB vs flat {flat} dB");
    }

    #[test]
    fn tube_adds_even_harmonics_and_no_dc() {
        let input = sine(1000.0, 1.0, -8.0);
        let s = Settings { peak_reduction: 0.0, tube: 1.0, ..Settings::default() };
        let out = run(&input, &s);
        let tail = &out[SR as usize / 2..];
        let (h1, h2) = (tone(tail, 1000.0), tone(tail, 2000.0));
        assert!(h2 / h1 > 0.01, "second harmonic only {:.4} of the fundamental", h2 / h1);
        let dc = tail.iter().sum::<f32>() / tail.len() as f32;
        assert!(dc.abs() < 0.01, "dc {dc}");

        let wire = run(&input, &Settings { tube: 0.0, ..s });
        let wt = &wire[SR as usize / 2..];
        assert!(tone(wt, 2000.0) / tone(wt, 1000.0) < 0.002);
    }

    #[test]
    fn stereo_link_gives_both_channels_one_gain() {
        let quiet = sine(500.0, 2.0, -40.0);
        let loud = sine(500.0, 2.0, -3.0);
        let run2 = |link: f32| {
            let mut c = Compressor::new();
            c.configure(SR, 2);
            let s = Settings { peak_reduction: 0.7, link, ..clean() };
            let mut out = vec![];
            for i in 0..quiet.len() {
                out.push(c.process([loud[i], quiet[i]], &s));
            }
            let from = out.len() - 10000;
            let ratio = |ch: usize, src: &[f32]| rms(&out[from..].iter().map(|o| o[ch]).collect::<Vec<_>>()) / rms(&src[from - LATENCY..out.len() - LATENCY]);
            (ratio(0, &loud), ratio(1, &quiet))
        };
        let (a, b) = run2(1.0);
        assert!((a / b - 1.0).abs() < 0.02, "linked gains differ: {a} vs {b}");
        let (_, b) = run2(0.0);
        assert!(b > 0.98, "unlinked quiet channel was reduced to {b}");
    }

    #[test]
    fn output_stays_finite_at_the_extremes() {
        let s = Settings { peak_reduction: 1.0, gain_db: 24.0, mode: Mode::Limit, emphasis: 1.0, tube: 1.0, link: 1.0, mix: 1.0 };
        let mut input = noise(30000, 1.0);
        input.extend(vec![0.0; 5000]);
        input.extend(sine(60.0, 0.5, 6.0));
        let out = run(&input, &s);
        assert!(out.iter().all(|v| v.is_finite() && v.abs() < 20.0));
    }
}
