//! Automatic double tracking.
//!
//! The original ADT trick (Abbey Road, 1966) played the vocal back from a second tape machine whose
//! speed was wobbled by an oscillator, so the copy lagged the lead by a few tens of milliseconds
//! and kept drifting in time and pitch, like a second take by a human. This models that: a long
//! delay line read with a smoothed random walk added to the delay time, a slow sine "flutter" for
//! the tape's speed ripple, and a tape stage on the copy (antialiased tanh saturation and a
//! high-frequency roll-off). In stereo the two channels wander independently by `width`, and the
//! left and right copies sit at slightly different offsets.
//!
//! Reading the line at a moving position shifts pitch by `-d(delay)/dt`, so the wander's
//! amplitude is limited by its rate to keep the pitch deviation within about 25 cents.

use nih_plug::prelude::AtomicF32;
use std::f32::consts::{LN_2, TAU};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

pub const MAX_CHANNELS: usize = 2;
pub const MIN_TIME_MS: f32 = 5.0;
pub const MAX_TIME_MS: f32 = 120.0;
/// Longest delay the line must hold: the maximum time, the stereo spread on it and the wander.
const MAX_DELAY_MS: f32 = 145.0;
/// Largest wander excursion in milliseconds, reached at low rates.
const MAX_WANDER_MS: f32 = 6.0;
/// Wander amplitude in ms is `PITCH_BUDGET / rate`: that keeps the peak pitch deviation constant.
const PITCH_BUDGET: f32 = 3.0;
/// Peak amplitude of the tape's speed ripple at full tape amount, in milliseconds, and its rate.
const FLUTTER_MS: f32 = 0.06;
const FLUTTER_HZ: f32 = 9.3;
/// Stereo offset between the two copies at full width, as a fraction of the time.
const SPREAD: f32 = 0.12;

/// Telemetry columns: the editor shows the last `COLS` of them, each `COL_MS` long.
pub const COLS: usize = 240;
pub const COL_MS: f32 = 2.5;

/// What the audio thread publishes for the editor: a ring of per-column peaks and delay times.
pub struct Meter {
    /// Number of columns written so far; column `n` lives in slot `n % COLS`.
    pub head: AtomicUsize,
    pub lead: [AtomicF32; COLS],
    pub double: [AtomicF32; COLS],
    /// Delay of the left / right copy at the end of each column, in milliseconds.
    pub delay: [AtomicF32; COLS],
    pub delay_right: [AtomicF32; COLS],
    /// Pitch deviation of the left copy, averaged over the latest column, in cents.
    pub cents: AtomicF32,
}

impl Meter {
    fn new() -> Self {
        Self {
            head: AtomicUsize::new(0),
            lead: std::array::from_fn(|_| AtomicF32::new(0.0)),
            double: std::array::from_fn(|_| AtomicF32::new(0.0)),
            delay: std::array::from_fn(|_| AtomicF32::new(0.0)),
            delay_right: std::array::from_fn(|_| AtomicF32::new(0.0)),
            cents: AtomicF32::new(0.0),
        }
    }
}

#[derive(Clone, Copy)]
pub struct Settings {
    pub time_ms: f32,
    /// 0..1: how far the delay wanders.
    pub drift: f32,
    /// Wander speed in Hz.
    pub rate_hz: f32,
    /// 0..1: saturation, roll-off and speed ripple of the copy.
    pub tape: f32,
    /// 0..1: how independent the two channels are.
    pub width: f32,
    /// Linear gain of the copy.
    pub level: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self { time_ms: 45.0, drift: 0.35, rate_hz: 0.7, tape: 0.4, width: 0.5, level: 0.7 }
    }
}

/// Slowly varying noise in about -1..1: random targets twice per cycle, smoothed by two one-poles.
struct Wander {
    rng: u32,
    target: f32,
    s1: f32,
    s2: f32,
    phase: f32,
}

impl Wander {
    fn new(seed: u32) -> Self {
        Self { rng: seed, target: 0.0, s1: 0.0, s2: 0.0, phase: 0.0 }
    }

    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng as f32 / u32::MAX as f32 * 2.0 - 1.0
    }

    fn next(&mut self, rate_hz: f32, coeff: f32, sample_rate: f32) -> f32 {
        self.phase += 2.0 * rate_hz / sample_rate;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
            self.target = self.random();
        }
        self.s1 += coeff * (self.target - self.s1);
        self.s2 += coeff * (self.s1 - self.s2);
        // Two smoothing stages shrink the swing, so scale back up.
        (self.s2 * 1.5).clamp(-1.0, 1.0)
    }
}

fn ln_cosh(x: f32) -> f32 {
    let a = x.abs();
    a + (-2.0 * a).exp().ln_1p() - LN_2
}

/// What the tape does to the copy.
#[derive(Default)]
struct TapeStage {
    x1: f32,
    f1: f32,
    lp1: f32,
    lp2: f32,
}

impl TapeStage {
    /// First-order antiderivative antialiasing of `tanh`: the average of tanh over the step.
    fn saturate(&mut self, u: f32) -> f32 {
        let f = ln_cosh(u);
        let d = u - self.x1;
        let y = if d.abs() < 1e-4 { (0.5 * (u + self.x1)).tanh() } else { (f - self.f1) / d };
        self.x1 = u;
        self.f1 = f;
        y
    }

    fn process(&mut self, x: f32, tape: f32, sample_rate: f32) -> f32 {
        if tape <= 0.0 {
            // Keep the state current so engaging the tape doesn't click.
            self.saturate(x);
            self.lp1 = x;
            self.lp2 = x;
            return x;
        }
        let drive = 1.0 + 3.0 * tape;
        let sat = x + (self.saturate(x * drive) / drive - x) * tape;
        let cutoff = (16_000.0 * 0.3_f32.powf(tape)).min(sample_rate * 0.45);
        let coeff = 1.0 - (-TAU * cutoff / sample_rate).exp();
        self.lp1 += coeff * (sat - self.lp1);
        self.lp2 += coeff * (self.lp1 - self.lp2);
        sat + (self.lp2 - sat) * tape
    }
}

/// Four-point Catmull-Rom read of `line` at fractional position `pos`.
fn read_hermite(line: &[f32], pos: f32) -> f32 {
    let len = line.len();
    let i = pos.floor();
    let f = pos - i;
    let i = i as usize;
    let ym1 = line[(i + len - 1) % len];
    let y0 = line[i % len];
    let y1 = line[(i + 1) % len];
    let y2 = line[(i + 2) % len];
    let c1 = 0.5 * (y1 - ym1);
    let c2 = ym1 - 2.5 * y0 + 2.0 * y1 - 0.5 * y2;
    let c3 = 0.5 * (y2 - ym1) + 1.5 * (y0 - y1);
    ((c3 * f + c2) * f + c1) * f + y0
}

pub struct Doubler {
    sample_rate: f32,
    channels: usize,
    lines: [Vec<f32>; MAX_CHANNELS],
    pos: usize,
    wander: [Wander; MAX_CHANNELS],
    flutter_phase: f32,
    tape: [TapeStage; MAX_CHANNELS],
    last_delay: f32,
    pub meter: Arc<Meter>,
    col_len: usize,
    col_count: usize,
    col_lead: f32,
    col_double: f32,
    col_cents: f32,
    col_delay: [f32; MAX_CHANNELS],
}

impl Doubler {
    pub fn new() -> Self {
        Self {
            sample_rate: 44100.0,
            channels: 2,
            lines: [Vec::new(), Vec::new()],
            pos: 0,
            wander: [Wander::new(0x9e37_79b9), Wander::new(0x85eb_ca6b)],
            flutter_phase: 0.0,
            tape: Default::default(),
            last_delay: 0.0,
            meter: Arc::new(Meter::new()),
            col_len: 110,
            col_count: 0,
            col_lead: 0.0,
            col_double: 0.0,
            col_cents: 0.0,
            col_delay: [0.0; MAX_CHANNELS],
        }
    }

    pub fn configure(&mut self, sample_rate: f32, channels: usize) {
        self.sample_rate = sample_rate;
        self.channels = channels.clamp(1, MAX_CHANNELS);
        let len = (MAX_DELAY_MS / 1000.0 * sample_rate).ceil() as usize + 8;
        for line in &mut self.lines {
            line.clear();
            line.resize(len, 0.0);
        }
        self.col_len = ((COL_MS / 1000.0 * sample_rate) as usize).max(1);
        self.reset();
    }

    pub fn reset(&mut self) {
        for line in &mut self.lines {
            line.fill(0.0);
        }
        self.tape = Default::default();
        self.pos = 0;
        self.col_count = 0;
        self.col_lead = 0.0;
        self.col_double = 0.0;
        self.col_cents = 0.0;
    }

    /// Process one frame: the lead passes through untouched and the copy is added.
    pub fn process(&mut self, x: [f32; MAX_CHANNELS], s: &Settings) -> [f32; MAX_CHANNELS] {
        let sr = self.sample_rate;
        let len = self.lines[0].len();
        let stereo = self.channels > 1;

        // Cap the wander so its pitch deviation stays musical at fast rates.
        let amp_ms = (PITCH_BUDGET / s.rate_hz.max(0.05)).min(MAX_WANDER_MS) * s.drift;
        let coeff = 1.0 - (-TAU * s.rate_hz * 0.8 / sr).exp();
        let n0 = self.wander[0].next(s.rate_hz, coeff, sr);
        let n1 = if stereo { self.wander[1].next(s.rate_hz, coeff, sr) } else { 0.0 };
        let noise = [n0, n0 * (1.0 - s.width) + n1 * s.width];
        let spread = if stereo { [1.0 - SPREAD * s.width, 1.0 + SPREAD * s.width] } else { [1.0, 1.0] };
        self.flutter_phase = (self.flutter_phase + FLUTTER_HZ / sr).fract();

        let mut out = x;
        let mut delay_ms = [0.0; MAX_CHANNELS];
        for ch in 0..self.channels {
            let flutter = (TAU * (self.flutter_phase + 0.37 * ch as f32)).sin();
            delay_ms[ch] = s.time_ms * spread[ch] + amp_ms * noise[ch] + s.tape * FLUTTER_MS * flutter;
            let delay = (delay_ms[ch] * sr / 1000.0).clamp(2.0, (len - 8) as f32);

            self.lines[ch][self.pos] = x[ch];
            let mut read = self.pos as f32 - delay;
            if read < 0.0 {
                read += len as f32;
            }
            let copy = read_hermite(&self.lines[ch], read);
            let copy = self.tape[ch].process(copy, s.tape, sr) * s.level;
            out[ch] = x[ch] + copy;
            self.col_double = self.col_double.max(copy.abs());
            self.col_lead = self.col_lead.max(x[ch].abs());
        }
        self.pos = (self.pos + 1) % len;

        // Speed of the left read head relative to the write head is `1 - d(delay)/dt`.
        let dd = (delay_ms[0] - self.last_delay) * sr / 1000.0;
        self.last_delay = delay_ms[0];
        self.col_cents += 1200.0 * (1.0 - dd.clamp(-0.5, 0.5)).log2();
        self.col_delay = delay_ms;
        self.col_count += 1;
        if self.col_count >= self.col_len {
            self.publish();
        }
        out
    }

    fn publish(&mut self) {
        let m = &self.meter;
        let slot = m.head.load(Ordering::Relaxed) % COLS;
        m.lead[slot].store(self.col_lead, Ordering::Relaxed);
        m.double[slot].store(self.col_double, Ordering::Relaxed);
        m.delay[slot].store(self.col_delay[0], Ordering::Relaxed);
        m.delay_right[slot].store(if self.channels > 1 { self.col_delay[1] } else { 0.0 }, Ordering::Relaxed);
        m.cents.store(self.col_cents / self.col_count as f32, Ordering::Relaxed);
        m.head.fetch_add(1, Ordering::Release);
        self.col_count = 0;
        self.col_lead = 0.0;
        self.col_double = 0.0;
        self.col_cents = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 44100.0;

    fn doubler(channels: usize) -> Doubler {
        let mut d = Doubler::new();
        d.configure(SR, channels);
        d
    }

    fn plain() -> Settings {
        Settings { time_ms: 40.0, drift: 0.0, tape: 0.0, width: 0.0, level: 1.0, ..Settings::default() }
    }

    /// Deterministic noise in -1..1.
    fn noise(n: usize) -> Vec<f32> {
        let mut state = 12345u32;
        (0..n)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                state as f32 / u32::MAX as f32 * 2.0 - 1.0
            })
            .collect()
    }

    fn sine(freq: f32, n: usize, amp: f32) -> Vec<f32> {
        (0..n).map(|i| amp * (TAU * freq * i as f32 / SR).sin()).collect()
    }

    fn run(d: &mut Doubler, input: &[f32], s: &Settings) -> Vec<f32> {
        input.iter().map(|&v| d.process([v, v], s)[0]).collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    #[test]
    fn zero_level_is_a_null_test() {
        let mut d = doubler(2);
        let input = noise(20_000);
        let s = Settings { level: 0.0, drift: 1.0, tape: 1.0, width: 1.0, ..Settings::default() };
        let out = run(&mut d, &input, &s);
        assert_eq!(out, input);
    }

    #[test]
    fn copy_is_the_input_delayed_by_the_time() {
        let mut d = doubler(1);
        let mut input = vec![0.0; 6000];
        input[100] = 1.0;
        let out = run(&mut d, &input, &plain());
        let lag = (40.0 * SR / 1000.0) as usize;
        assert!((out[100] - 1.0).abs() < 1e-6, "lead must pass untouched");
        assert!((out[100 + lag] - 1.0).abs() < 1e-4, "copy should land {lag} samples later");
        let stray: f32 = out.iter().enumerate().filter(|&(i, _)| i != 100 && i != 100 + lag).map(|(_, v)| v.abs()).sum();
        assert!(stray < 1e-3, "no other energy, got {stray}");
    }

    #[test]
    fn level_scales_only_the_copy() {
        let mut d = doubler(1);
        let mut input = vec![0.0; 6000];
        input[100] = 1.0;
        let s = Settings { level: 0.5, ..plain() };
        let out = run(&mut d, &input, &s);
        let lag = (40.0 * SR / 1000.0) as usize;
        assert!((out[100] - 1.0).abs() < 1e-6);
        assert!((out[100 + lag] - 0.5).abs() < 1e-4);
    }

    #[test]
    fn drift_bends_pitch_within_budget_and_is_off_at_zero() {
        let input = sine(1000.0, (SR * 12.0) as usize, 0.5);
        for (drift, expect_motion) in [(0.0, false), (1.0, true)] {
            let mut d = doubler(1);
            let s = Settings { drift, ..plain() };
            let mut peak = 0.0_f32;
            for (i, &v) in input.iter().enumerate() {
                d.process([v, v], &s);
                if i > SR as usize {
                    peak = peak.max(d.col_cents.abs() / d.col_count.max(1) as f32);
                }
            }
            let stat = d.meter.cents.load(Ordering::Relaxed).abs();
            assert!(stat < 40.0, "cents {stat}");
            if expect_motion {
                assert!(peak > 2.0 && peak < 60.0, "peak cents {peak}");
            } else {
                assert!(peak < 0.01, "peak cents {peak}");
            }
        }
    }

    #[test]
    fn wander_covers_a_range_and_stays_bounded() {
        let mut w = Wander::new(1);
        let coeff = 1.0 - (-TAU * 0.7 * 0.8 / SR).exp();
        let vals: Vec<f32> = (0..(SR as usize * 60)).map(|_| w.next(0.7, coeff, SR)).collect();
        let (lo, hi) = vals.iter().fold((f32::MAX, f32::MIN), |(l, h), &v| (l.min(v), h.max(v)));
        assert!(lo >= -1.0 && hi <= 1.0);
        assert!(hi - lo > 0.8, "range {lo}..{hi}");
        assert!(vals.iter().sum::<f32>().abs() / (vals.len() as f32) < 0.3, "should be centred");
    }

    #[test]
    fn mono_in_stereo_stays_identical_at_zero_width_and_decorrelates_at_full_width() {
        let input = noise(SR as usize * 4);
        for (width, identical) in [(0.0, true), (1.0, false)] {
            let mut d = doubler(2);
            let s = Settings { drift: 1.0, width, ..plain() };
            let mut max_diff = 0.0_f32;
            for &v in &input {
                let o = d.process([v, v], &s);
                max_diff = max_diff.max((o[0] - o[1]).abs());
            }
            if identical {
                assert!(max_diff < 1e-6, "width 0 diff {max_diff}");
            } else {
                assert!(max_diff > 0.05, "width 1 diff {max_diff}");
            }
        }
    }

    #[test]
    fn tape_saturates_and_rolls_off_highs() {
        let loud = sine(200.0, 20_000, 0.9);
        let mut a = doubler(1);
        let mut b = doubler(1);
        let clean = run(&mut a, &loud, &Settings { tape: 0.0, ..plain() });
        let worn = run(&mut b, &loud, &Settings { tape: 1.0, ..plain() });
        // Drive adds harmonics, so the waveform departs from the clean sine.
        let diff: Vec<f32> = clean.iter().zip(&worn).skip(8000).map(|(c, w)| c - w).collect();
        assert!(rms(&diff) > 0.02, "tape should change the waveform, rms diff {}", rms(&diff));

        let hi = sine(12_000.0, 20_000, 0.1);
        let mut a = doubler(1);
        let mut b = doubler(1);
        let clean = run(&mut a, &hi, &Settings { tape: 0.0, ..plain() });
        let worn = run(&mut b, &hi, &Settings { tape: 1.0, ..plain() });
        assert!(rms(&worn[8000..]) < rms(&clean[8000..]) * 0.8, "highs should roll off");
    }

    #[test]
    fn output_stays_finite_at_extremes() {
        for channels in [1, 2] {
            let mut d = doubler(channels);
            let s = Settings { time_ms: MAX_TIME_MS, drift: 1.0, rate_hz: 0.1, tape: 1.0, width: 1.0, level: 2.0 };
            let input: Vec<f32> = noise(SR as usize * 5).iter().map(|v| v * 4.0).collect();
            for (i, &v) in input.iter().enumerate() {
                let s = if i % 20_000 < 10_000 { s } else { Settings { time_ms: MIN_TIME_MS, rate_hz: 4.0, ..s } };
                let o = d.process([v, -v], &s);
                assert!(o[0].is_finite() && o[1].is_finite());
            }
        }
    }

    #[test]
    fn meter_publishes_columns_with_the_delay() {
        let mut d = doubler(1);
        let input = sine(500.0, (SR * 0.5) as usize, 0.5);
        run(&mut d, &input, &plain());
        let head = d.meter.head.load(Ordering::Relaxed);
        assert!(head > 150, "columns written: {head}");
        let slot = (head - 1) % COLS;
        assert!((d.meter.delay[slot].load(Ordering::Relaxed) - 40.0).abs() < 0.5);
        assert!((d.meter.lead[slot].load(Ordering::Relaxed) - 0.5).abs() < 0.05);
        assert!(d.meter.double[slot].load(Ordering::Relaxed) > 0.4);
    }
}
