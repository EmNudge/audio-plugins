//! Three-band upward and downward compression.
//!
//! The input is split into three bands with 4th-order Linkwitz-Riley crossovers (the low band gets
//! an all-pass at the upper crossover so the three always sum back flat in magnitude and phase).
//! Each band has a level detector (a short mean-square window, linked across channels so the stereo
//! image holds still) feeding a gain computer with two soft-knee branches: above the *ceiling* the
//! band is pushed down, below the *floor* it is pulled up, and between them it is left alone. The
//! resulting gain in dB is smoothed with separate attack (gain falling) and release (gain rising)
//! times, which scale together with the Time control.
//!
//! Upward gain fades out below a noise gate (-85 to -70 dB) and is capped, so digital silence stays
//! silent and a dither floor is not blown up to full scale.
//!
//! The dry path of the mix is the three bands summed *before* compression, not the raw input, so
//! mixing never comb-filters against the all-pass-shifted wet signal.

use nih_plug::prelude::AtomicF32;
use std::f64::consts::{FRAC_1_SQRT_2, TAU};
use std::sync::atomic::Ordering;
use std::sync::Arc;

pub const BANDS: usize = 3;
pub const MAX_CHANNELS: usize = 2;

/// How steeply a fully-open branch squeezes: 0.85 is roughly 6.7:1.
const SLOPE: f32 = 0.85;
/// Width of the soft knee around each threshold, in dB.
const KNEE_DB: f32 = 8.0;
/// The most upward gain a band can ever get, in dB.
const MAX_BOOST_DB: f32 = 24.0;
/// Upward gain fades in between these levels.
const GATE_LOW_DB: f32 = -85.0;
const GATE_HIGH_DB: f32 = -70.0;
/// Minimum distance between floor and ceiling, in dB.
pub const MIN_WINDOW_DB: f32 = 6.0;

/// Level detector time constants, per band (low frequencies need a longer window to ripple less).
const DETECTOR_MS: [f32; BANDS] = [10.0, 4.0, 1.0];
/// Attack (gain falling) and release (gain rising) at Time = 50%, per band.
const ATTACK_MS: [f32; BANDS] = [12.0, 6.0, 2.0];
const RELEASE_MS: [f32; BANDS] = [160.0, 100.0, 60.0];

/// The meters fall at this rate between buffers, in dB per second.
const METER_FALL_DB_S: f32 = 30.0;
const METER_FLOOR_DB: f32 = -120.0;

/// What the audio thread publishes for the editor, per band.
pub struct Meter {
    /// Band level going into the compressor and coming out of it (with the band gain), in dB.
    pub level_in: [AtomicF32; BANDS],
    pub level_out: [AtomicF32; BANDS],
    /// The compressor's gain in dB right now (negative: pushed down, positive: lifted).
    pub gain: [AtomicF32; BANDS],
}

impl Meter {
    fn new() -> Self {
        Self {
            level_in: std::array::from_fn(|_| AtomicF32::new(METER_FLOOR_DB)),
            level_out: std::array::from_fn(|_| AtomicF32::new(METER_FLOOR_DB)),
            gain: std::array::from_fn(|_| AtomicF32::new(0.0)),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub in_db: f32,
    pub out_db: f32,
    /// 0..1 scales the whole effect, 0 leaves only the band gains.
    pub depth: f32,
    /// 0..1: how much of each branch is applied.
    pub upward: f32,
    pub downward: f32,
    /// 0..1, 0.5 is the nominal times; the multiplier runs from 0.25x to 4x.
    pub time: f32,
    /// Below this level a band is lifted, above the ceiling it is pushed down.
    pub floor_db: f32,
    pub ceil_db: f32,
    pub band_db: [f32; BANDS],
    /// 0..1 wet.
    pub mix: f32,
    pub low_hz: f32,
    pub high_hz: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            in_db: 0.0,
            out_db: 0.0,
            depth: 1.0,
            upward: 1.0,
            downward: 1.0,
            time: 0.5,
            floor_db: -42.0,
            ceil_db: -18.0,
            band_db: [0.0; BANDS],
            mix: 1.0,
            low_hz: 120.0,
            high_hz: 2500.0,
        }
    }
}

fn db_to_lin(db: f32) -> f32 {
    10f32.powf(db * 0.05)
}

#[derive(Clone, Copy, Default)]
struct Coeffs {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

#[derive(Clone, Copy)]
enum Kind {
    Low,
    High,
    All,
}

/// RBJ biquad at Butterworth Q: two of these in series are a Linkwitz-Riley 4th-order filter.
fn design(kind: Kind, freq: f32, sample_rate: f32) -> Coeffs {
    let w = TAU * (freq as f64).min(sample_rate as f64 * 0.45) / sample_rate as f64;
    let (s, c) = w.sin_cos();
    let alpha = s / (2.0 * FRAC_1_SQRT_2);
    let a0 = 1.0 + alpha;
    let (b0, b1, b2) = match kind {
        Kind::Low => ((1.0 - c) * 0.5, 1.0 - c, (1.0 - c) * 0.5),
        Kind::High => ((1.0 + c) * 0.5, -(1.0 + c), (1.0 + c) * 0.5),
        Kind::All => (1.0 - alpha, -2.0 * c, 1.0 + alpha),
    };
    Coeffs { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: -2.0 * c / a0, a2: (1.0 - alpha) / a0 }
}

/// State of one biquad, for every channel (transposed direct form II).
#[derive(Clone, Copy, Default)]
struct Section {
    z: [[f64; 2]; MAX_CHANNELS],
}

impl Section {
    fn run(&mut self, c: &Coeffs, ch: usize, x: f64) -> f64 {
        let z = &mut self.z[ch];
        let y = c.b0 * x + z[0];
        z[0] = c.b1 * x - c.a1 * y + z[1];
        z[1] = c.b2 * x - c.a2 * y;
        y
    }
}

/// One LR4 crossover: a low-pass and a high-pass of two cascaded sections each.
#[derive(Default)]
struct Crossover {
    freq: f32,
    low: Coeffs,
    high: Coeffs,
    low_s: [Section; 2],
    high_s: [Section; 2],
}

impl Crossover {
    fn set(&mut self, freq: f32, sample_rate: f32) {
        if (freq - self.freq).abs() > self.freq * 1e-4 {
            self.freq = freq;
            self.low = design(Kind::Low, freq, sample_rate);
            self.high = design(Kind::High, freq, sample_rate);
        }
    }

    fn split(&mut self, ch: usize, x: f64) -> (f64, f64) {
        let l = self.low_s[0].run(&self.low, ch, x);
        let l = self.low_s[1].run(&self.low, ch, l);
        let h = self.high_s[0].run(&self.high, ch, x);
        let h = self.high_s[1].run(&self.high, ch, h);
        (l, h)
    }
}

#[derive(Clone, Copy)]
struct Band {
    /// Mean-square of the band, smoothed.
    power: f32,
    /// Smoothed compressor gain in dB.
    gain_db: f32,
}

pub struct Squeeze {
    pub meter: Arc<Meter>,
    sample_rate: f32,
    channels: usize,
    low_x: Crossover,
    high_x: Crossover,
    /// Aligns the low band with the upper crossover's phase.
    align: Coeffs,
    align_s: Section,
    align_freq: f32,
    bands: [Band; BANDS],
    detector: [f32; BANDS],
    attack: [f32; BANDS],
    release: [f32; BANDS],
    time: f32,
    /// Per-buffer maxima and the held values the meter shows.
    acc_in: [f32; BANDS],
    acc_out: [f32; BANDS],
    held_in: [f32; BANDS],
    held_out: [f32; BANDS],
}

fn one_pole(ms: f32, sample_rate: f32) -> f32 {
    1.0 - (-1.0 / (ms * 0.001 * sample_rate)).exp()
}

/// `max(x, 0)` with a quadratic knee of width `knee` centred on zero.
fn over(x: f32, knee: f32) -> f32 {
    if x <= -knee * 0.5 {
        0.0
    } else if x >= knee * 0.5 {
        x
    } else {
        let t = x + knee * 0.5;
        t * t / (2.0 * knee)
    }
}

fn smoothstep(lo: f32, hi: f32, x: f32) -> f32 {
    let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The static curve: gain in dB for a band at `level` dB.
pub fn gain_curve(level: f32, s: &Settings) -> f32 {
    let ceil = s.ceil_db;
    let floor = s.floor_db.min(ceil - MIN_WINDOW_DB);
    let down = over(level - ceil, KNEE_DB) * SLOPE * s.downward;
    let up = (over(floor - level, KNEE_DB) * SLOPE * s.upward).min(MAX_BOOST_DB)
        * smoothstep(GATE_LOW_DB, GATE_HIGH_DB, level);
    (up - down) * s.depth
}

impl Squeeze {
    pub fn new() -> Self {
        Self {
            meter: Arc::new(Meter::new()),
            sample_rate: 44100.0,
            channels: 2,
            low_x: Crossover::default(),
            high_x: Crossover::default(),
            align: Coeffs::default(),
            align_s: Section::default(),
            align_freq: 0.0,
            bands: [Band { power: 0.0, gain_db: 0.0 }; BANDS],
            detector: [0.0; BANDS],
            attack: [0.0; BANDS],
            release: [0.0; BANDS],
            time: -1.0,
            acc_in: [METER_FLOOR_DB; BANDS],
            acc_out: [METER_FLOOR_DB; BANDS],
            held_in: [METER_FLOOR_DB; BANDS],
            held_out: [METER_FLOOR_DB; BANDS],
        }
    }

    pub fn configure(&mut self, sample_rate: f32, channels: usize) {
        self.sample_rate = sample_rate;
        self.channels = channels.clamp(1, MAX_CHANNELS);
        for b in 0..BANDS {
            self.detector[b] = one_pole(DETECTOR_MS[b], sample_rate);
        }
        self.low_x.freq = 0.0;
        self.high_x.freq = 0.0;
        self.align_freq = 0.0;
        self.time = -1.0;
        self.reset();
    }

    pub fn reset(&mut self) {
        self.low_x.low_s = Default::default();
        self.low_x.high_s = Default::default();
        self.high_x.low_s = Default::default();
        self.high_x.high_s = Default::default();
        self.align_s = Section::default();
        self.bands = [Band { power: 0.0, gain_db: 0.0 }; BANDS];
        self.acc_in = [METER_FLOOR_DB; BANDS];
        self.acc_out = [METER_FLOOR_DB; BANDS];
        self.held_in = [METER_FLOOR_DB; BANDS];
        self.held_out = [METER_FLOOR_DB; BANDS];
    }

    fn update(&mut self, s: &Settings) {
        let low = s.low_hz.max(20.0);
        let high = s.high_hz.max(low * 2.0);
        self.low_x.set(low, self.sample_rate);
        self.high_x.set(high, self.sample_rate);
        if (high - self.align_freq).abs() > self.align_freq * 1e-4 {
            self.align_freq = high;
            self.align = design(Kind::All, high, self.sample_rate);
        }
        if (s.time - self.time).abs() > 1e-4 {
            self.time = s.time;
            let scale = 4f32.powf((s.time - 0.5) * 2.0);
            for b in 0..BANDS {
                self.attack[b] = one_pole(ATTACK_MS[b] * scale, self.sample_rate);
                self.release[b] = one_pole(RELEASE_MS[b] * scale, self.sample_rate);
            }
        }
    }

    pub fn process(&mut self, x: [f32; MAX_CHANNELS], s: &Settings) -> [f32; MAX_CHANNELS] {
        self.update(s);
        let in_gain = db_to_lin(s.in_db) as f64;

        // Split into low / mid / high; `bands[b][ch]`.
        let mut split = [[0.0f64; MAX_CHANNELS]; BANDS];
        for ch in 0..self.channels {
            let (low, rest) = self.low_x.split(ch, x[ch] as f64 * in_gain);
            let (mid, high) = self.high_x.split(ch, rest);
            split[0][ch] = self.align_s.run(&self.align, ch, low);
            split[1][ch] = mid;
            split[2][ch] = high;
        }

        let mut wet = [0.0f64; MAX_CHANNELS];
        let mut dry = [0.0f64; MAX_CHANNELS];
        for b in 0..BANDS {
            let mut ms = 0.0;
            for ch in 0..self.channels {
                let v = split[b][ch] as f32;
                ms += v * v;
            }
            let band = &mut self.bands[b];
            band.power += self.detector[b] * (ms / self.channels as f32 - band.power);
            // A full-scale sine reads 0 dB.
            let level = 10.0 * (2.0 * band.power + 1e-12).log10();
            let target = gain_curve(level, s);
            let rate = if target < band.gain_db { self.attack[b] } else { self.release[b] };
            band.gain_db += rate * (target - band.gain_db);

            let total_db = band.gain_db + s.band_db[b];
            let gain = db_to_lin(total_db) as f64;
            self.acc_in[b] = self.acc_in[b].max(level);
            self.acc_out[b] = self.acc_out[b].max(level + total_db);
            for ch in 0..self.channels {
                dry[ch] += split[b][ch];
                wet[ch] += split[b][ch] * gain;
            }
        }

        let out_gain = db_to_lin(s.out_db) as f64;
        let mix = s.mix as f64;
        let mut out = [0.0f32; MAX_CHANNELS];
        for ch in 0..self.channels {
            out[ch] = ((dry[ch] + (wet[ch] - dry[ch]) * mix) * out_gain) as f32;
        }
        out
    }

    /// Call once per buffer: publish the meters.
    pub fn publish(&mut self, frames: usize) {
        let fall = METER_FALL_DB_S * frames as f32 / self.sample_rate;
        for b in 0..BANDS {
            self.held_in[b] = self.acc_in[b].max(self.held_in[b] - fall).max(METER_FLOOR_DB);
            self.held_out[b] = self.acc_out[b].max(self.held_out[b] - fall).max(METER_FLOOR_DB);
            self.acc_in[b] = METER_FLOOR_DB;
            self.acc_out[b] = METER_FLOOR_DB;
            self.meter.level_in[b].store(self.held_in[b], Ordering::Relaxed);
            self.meter.level_out[b].store(self.held_out[b], Ordering::Relaxed);
            self.meter.gain[b].store(self.bands[b].gain_db, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 44100.0;

    fn squeeze(channels: usize) -> Squeeze {
        let mut s = Squeeze::new();
        s.configure(SR, channels);
        s
    }

    fn sine(freq: f32, secs: f32, db: f32) -> Vec<f32> {
        let amp = db_to_lin(db);
        (0..(secs * SR) as usize).map(|i| amp * (std::f32::consts::TAU * freq * i as f32 / SR).sin()).collect()
    }

    /// Deterministic noise in -1..1.
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

    fn run(d: &mut Squeeze, input: &[f32], s: &Settings) -> Vec<f32> {
        input.iter().map(|&v| d.process([v, v], s)[0]).collect()
    }

    fn rms_db(x: &[f32]) -> f32 {
        20.0 * ((x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt() + 1e-12).log10()
    }

    /// RMS in dB of the last half of a buffer, once everything has settled.
    fn tail_db(x: &[f32]) -> f32 {
        rms_db(&x[x.len() / 2..])
    }

    fn untouched() -> Settings {
        Settings { depth: 0.0, ..Settings::default() }
    }

    #[test]
    fn depth_zero_is_a_null_test() {
        let input = noise(SR as usize, 0.3);
        for mix in [1.0, 0.0] {
            let out = run(&mut squeeze(2), &input, &Settings { mix, ..untouched() });
            let diff = (tail_db(&out) - tail_db(&input)).abs();
            assert!(diff < 0.1, "mix {mix}: level moved by {diff} dB");
        }
    }

    #[test]
    fn bands_sum_flat() {
        for freq in [50.0, 120.0, 300.0, 1000.0, 2500.0, 5000.0, 12000.0] {
            let input = sine(freq, 0.6, -12.0);
            let out = run(&mut squeeze(2), &input, &untouched());
            let diff = tail_db(&out) - tail_db(&input);
            assert!(diff.abs() < 0.1, "{freq} Hz moved by {diff} dB");
        }
    }

    #[test]
    fn band_gain_only_moves_its_band() {
        let s = Settings { band_db: [0.0, 0.0, 12.0], ..untouched() };
        for (freq, expect) in [(60.0, 0.0), (500.0, 0.0), (10000.0, 12.0)] {
            let input = sine(freq, 0.6, -20.0);
            let out = run(&mut squeeze(2), &input, &s);
            let diff = tail_db(&out) - tail_db(&input);
            assert!((diff - expect).abs() < 0.5, "{freq} Hz moved by {diff} dB, expected {expect}");
        }
    }

    #[test]
    fn levels_inside_the_window_pass_untouched() {
        // Mid-band centre: off-centre tones leak into a neighbour band, which lifts the leak.
        let input = sine(550.0, 1.0, -30.0);
        let out = run(&mut squeeze(2), &input, &Settings::default());
        let diff = tail_db(&out) - tail_db(&input);
        assert!(diff.abs() < 0.5, "moved by {diff} dB");
    }

    #[test]
    fn loud_goes_down_and_quiet_goes_up() {
        let s = Settings::default();
        let loud = sine(1000.0, 1.0, -6.0);
        let quiet = sine(1000.0, 1.0, -55.0);
        let loud_out = run(&mut squeeze(2), &loud, &s);
        let quiet_out = run(&mut squeeze(2), &quiet, &s);
        let down = tail_db(&loud_out) - tail_db(&loud);
        let up = tail_db(&quiet_out) - tail_db(&quiet);
        assert!(down < -6.0, "loud only moved by {down} dB");
        assert!(up > 6.0, "quiet only moved by {up} dB");
    }

    #[test]
    fn squeezes_the_dynamic_range() {
        // Alternate loud and quiet half-second phrases and measure the settled end of each.
        let mut input = vec![];
        for _ in 0..3 {
            input.extend(sine(1000.0, 0.5, -6.0));
            input.extend(sine(1000.0, 0.5, -50.0));
        }
        let out = run(&mut squeeze(2), &input, &Settings::default());
        let half = (0.5 * SR) as usize;
        let end = |x: &[f32], phrase: usize| rms_db(&x[(phrase + 1) * half - half / 3..(phrase + 1) * half]);
        let before = end(&input, 4) - end(&input, 5);
        let after = end(&out, 4) - end(&out, 5);
        assert!((before - 44.0).abs() < 0.5);
        assert!(after < before - 12.0, "range only went from {before} dB to {after} dB");
        assert!(after > 0.0, "the loud phrase must stay louder");
    }

    #[test]
    fn each_branch_works_alone() {
        let loud = sine(1000.0, 1.0, -6.0);
        let quiet = sine(1000.0, 1.0, -55.0);

        let s = Settings { upward: 0.0, ..Settings::default() };
        let quiet_out = run(&mut squeeze(2), &quiet, &s);
        assert!((tail_db(&quiet_out) - tail_db(&quiet)).abs() < 0.5, "downward only touched a quiet signal");
        let loud_out = run(&mut squeeze(2), &loud, &s);
        assert!(tail_db(&loud_out) < tail_db(&loud) - 6.0);

        let s = Settings { downward: 0.0, ..Settings::default() };
        let loud_out = run(&mut squeeze(2), &loud, &s);
        assert!((tail_db(&loud_out) - tail_db(&loud)).abs() < 0.5, "upward only touched a loud signal");
        let quiet_out = run(&mut squeeze(2), &quiet, &s);
        assert!(tail_db(&quiet_out) > tail_db(&quiet) + 6.0);
    }

    #[test]
    fn depth_scales_the_effect() {
        let input = sine(1000.0, 1.0, -6.0);
        let moved = |depth: f32| {
            let out = run(&mut squeeze(2), &input, &Settings { depth, ..Settings::default() });
            tail_db(&input) - tail_db(&out)
        };
        let (full, half) = (moved(1.0), moved(0.5));
        assert!(full > 6.0);
        assert!((half - full * 0.5).abs() < 1.0, "half depth moved {half} dB against {full} dB");
    }

    #[test]
    fn mix_blends_between_dry_and_wet() {
        let input = sine(1000.0, 1.0, -6.0);
        let level = |mix: f32| tail_db(&run(&mut squeeze(2), &input, &Settings { mix, ..Settings::default() }));
        let (dry, half, wet) = (level(0.0), level(0.5), level(1.0));
        assert!((dry - tail_db(&input)).abs() < 0.1);
        assert!(wet < dry - 6.0);
        assert!(half < dry && half > wet);
    }

    #[test]
    fn faster_time_reacts_sooner() {
        // Step from quiet to loud: how far has the gain fallen after 20 ms?
        let mut input = sine(1000.0, 0.5, -50.0);
        input.extend(sine(1000.0, 0.5, -6.0));
        let after_step = |time: f32| {
            let mut d = squeeze(2);
            let out = run(&mut d, &input, &Settings { time, ..Settings::default() });
            let start = (0.5 * SR) as usize;
            rms_db(&out[start + 400..start + 900])
        };
        assert!(after_step(0.1) < after_step(0.9) - 1.0);
    }

    #[test]
    fn stereo_image_holds_still() {
        // R is half of L; linked detectors give both the same gain, so that stays true.
        let l = sine(1000.0, 1.0, -8.0);
        let mut d = squeeze(2);
        let s = Settings::default();
        let (mut ol, mut or) = (vec![], vec![]);
        for &v in &l {
            let o = d.process([v, v * 0.5], &s);
            ol.push(o[0]);
            or.push(o[1]);
        }
        assert!(tail_db(&ol) < tail_db(&l) - 3.0, "no compression happened");
        assert!((tail_db(&ol) - tail_db(&or) - 6.02).abs() < 0.1);
    }

    #[test]
    fn mono_layout_works() {
        let input = sine(1000.0, 1.0, -6.0);
        let mut d = squeeze(1);
        let out: Vec<f32> = input.iter().map(|&v| d.process([v, 0.0], &Settings::default())[0]).collect();
        assert!(tail_db(&out) < tail_db(&input) - 6.0);
    }

    #[test]
    fn silence_stays_silent() {
        let out = run(&mut squeeze(2), &vec![0.0; SR as usize], &Settings::default());
        assert!(out.iter().all(|&v| v == 0.0));
        // And so does a signal buried below the gate.
        let hiss = noise(SR as usize, 1e-6);
        let out = run(&mut squeeze(2), &hiss, &Settings::default());
        assert!(tail_db(&out) < -80.0, "a -120 dB floor was lifted to {} dB", tail_db(&out));
    }

    #[test]
    fn output_stays_finite_at_the_extremes() {
        let hot = Settings {
            in_db: 24.0,
            out_db: 12.0,
            band_db: [12.0; BANDS],
            time: 0.0,
            ceil_db: -40.0,
            floor_db: -10.0,
            low_hz: 40.0,
            high_hz: 10000.0,
            ..Settings::default()
        };
        let mut input = noise(SR as usize, 1.0);
        input.extend(vec![0.0; 5000]);
        input.extend(noise(SR as usize, 1e-5));
        for s in [hot, Settings { time: 1.0, floor_db: -60.0, ceil_db: 0.0, ..Settings::default() }] {
            let out = run(&mut squeeze(2), &input, &s);
            assert!(out.iter().all(|v| v.is_finite()));
        }
    }

    #[test]
    fn crossover_moves_are_clean() {
        // Sweeping the crossovers while audio plays must not blow up.
        let mut d = squeeze(2);
        let input = noise(SR as usize, 0.3);
        for (i, &v) in input.iter().enumerate() {
            let t = i as f32 / input.len() as f32;
            let s = Settings { low_hz: 40.0 + 360.0 * t, high_hz: 10000.0 - 9000.0 * t, ..Settings::default() };
            assert!(d.process([v, v], &s)[0].is_finite());
        }
    }

    #[test]
    fn meters_report_what_happened() {
        let mut d = squeeze(2);
        let input = sine(1000.0, 0.5, -6.0);
        let s = Settings::default();
        for chunk in input.chunks(256) {
            for &v in chunk {
                d.process([v, v], &s);
            }
            d.publish(chunk.len());
        }
        let m = &d.meter;
        let mid_in = m.level_in[1].load(Ordering::Relaxed);
        let mid_out = m.level_out[1].load(Ordering::Relaxed);
        assert!((mid_in + 6.0).abs() < 1.0, "input read {mid_in} dB");
        assert!(mid_out < mid_in - 3.0);
        assert!(m.gain[1].load(Ordering::Relaxed) < -3.0);
    }
}
