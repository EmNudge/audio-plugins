//! A multi-voice chorus with analog (bucket-brigade) voicing.
//!
//! The input is summed to mono, as on the hardware this is modelled on, band-limited and softly
//! saturated, and written to one delay line. Up to three voices read it at delay times swept by
//! LFOs a fixed fraction of a cycle apart, and each voice is panned across the stereo field. The
//! sum of the dry signal and the wet voices is what makes the chorus: the moving comb filter
//! between them, plus a little pitch change wherever a delay is moving.
//!
//! Modes fix the voicing the way the units they are named after do:
//! - **Juno I / II / I+II**: two voices swept in anti-phase by a triangle (0.51 Hz, 0.86 Hz and a
//!   fast, shallow 9.75 Hz) around about 3.4 ms, hard left and right.
//! - **Dimension**: two anti-phase sine voices around 6.5 ms. The depth knob is the intensity
//!   (it sets rate and swing together, like the four buttons), and the wet is high-passed so the
//!   effect widens without thickening the low end or collapsing in mono.
//! - **Ensemble**: three voices 120 degrees apart, each swept by a slow sine plus a faster ripple,
//!   after the string-machine ensembles.
//! - **Free**: everything is a knob.
//!
//! The analog amount blends in the band-limiting a BBD needs (the bandwidth falls as the delay
//! grows, because the clock slows), a soft saturation before the line, and a little hiss on each
//! voice. It does not model the compander, so the noise stays constant instead of breathing.
//!
//! The wet path adds no latency: the dry signal is deliberately not delayed, because the
//! interference between the two is the effect.

use nih_plug::prelude::{AtomicF32, Enum};
use std::f32::consts::{FRAC_PI_2, TAU};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

pub const MAX_CHANNELS: usize = 2;
pub const MAX_VOICES: usize = 3;
pub const MIN_DELAY_MS: f32 = 0.5;
pub const MAX_DELAY_MS: f32 = 25.0;
/// At full depth a voice swings this fraction of its delay to either side.
const MAX_SWING: f32 = 0.8;
/// The furthest any voice can read from, in milliseconds.
pub const MAX_REACH_MS: f32 = MAX_DELAY_MS * (1.0 + MAX_SWING);
const LINE_MS: f32 = MAX_REACH_MS + 6.0;

/// Bandwidth of the modelled BBD at its reference delay, how that falls with delay, and its floor.
const BBD_CUT_HZ: f32 = 9000.0;
const BBD_REF_MS: f32 = 3.4;
const BBD_MIN_CUT_HZ: f32 = 3500.0;
/// Hiss on each voice at full analog, as a linear amplitude (about -64 dB).
const BBD_NOISE: f32 = 6.0e-4;
/// Wet high-pass corner in Dimension mode.
const DIMENSION_HP_HZ: f32 = 150.0;
/// Ensemble ripple: how much faster than the slow LFO it runs.
const RIPPLE_RATIO: f32 = 9.0;
/// Coefficients that depend on the delay are refreshed this often, in samples.
const COEFF_EVERY: u32 = 16;

/// Telemetry columns: the editor shows the last `COLS` of them, each `COL_MS` long.
pub const COLS: usize = 256;
pub const COL_MS: f32 = 6.0;

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    #[name = "Free"]
    Free,
    #[name = "Juno I"]
    JunoI,
    #[name = "Juno II"]
    JunoII,
    #[name = "Juno I+II"]
    JunoBoth,
    #[name = "Dimension"]
    Dimension,
    #[name = "Ensemble"]
    Ensemble,
}

impl Mode {
    /// Whether the rate (and tempo sync) and the delay knob do anything.
    pub fn free_rate(self) -> bool {
        matches!(self, Mode::Free | Mode::Ensemble)
    }
    pub fn free_delay(self) -> bool {
        self.free_rate()
    }
    /// Whether the depth knob does anything (in Dimension it is the intensity).
    pub fn free_depth(self) -> bool {
        !matches!(self, Mode::JunoI | Mode::JunoII | Mode::JunoBoth)
    }
    /// Whether the voice count and the LFO shape do anything.
    pub fn free_voices(self) -> bool {
        self == Mode::Free
    }
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    #[name = "Sine"]
    Sine,
    #[name = "Triangle"]
    Triangle,
    /// A new random target each cycle, glided to.
    #[name = "Random"]
    Random,
}

/// Tempo-synced LFO rate: one cycle per this note length.
#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Division {
    #[name = "Off"]
    Off,
    #[name = "1 bar"]
    Bar,
    #[name = "1/2"]
    Half,
    #[name = "1/4"]
    Quarter,
    #[name = "1/8"]
    Eighth,
    #[name = "1/16"]
    Sixteenth,
}

impl Division {
    fn beats(self) -> Option<f32> {
        match self {
            Division::Off => None,
            Division::Bar => Some(4.0),
            Division::Half => Some(2.0),
            Division::Quarter => Some(1.0),
            Division::Eighth => Some(0.5),
            Division::Sixteenth => Some(0.25),
        }
    }
}

/// LFO rate for one cycle per `division` at `bpm`, or `None` when sync is off.
pub fn synced_rate_hz(bpm: f64, division: Division) -> Option<f32> {
    division.beats().map(|beats| (bpm / 60.0) as f32 / beats)
}

#[derive(Clone, Copy)]
pub struct Settings {
    pub mode: Mode,
    pub rate_hz: f32,
    /// 0..1.
    pub depth: f32,
    pub delay_ms: f32,
    pub voices: usize,
    pub shape: Shape,
    /// 0..1: how far apart the voices are panned.
    pub spread: f32,
    /// 0..1: BBD band-limiting, saturation and hiss.
    pub analog: f32,
    /// 0..1: dry at 0, equal dry and wet at 0.5, wet only at 1.
    pub mix: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: Mode::Free,
            rate_hz: 0.6,
            depth: 0.4,
            delay_ms: 10.0,
            voices: 3,
            shape: Shape::Sine,
            spread: 1.0,
            analog: 0.4,
            mix: 0.5,
        }
    }
}

/// What the settings mean once the mode has fixed what it fixes.
#[derive(Clone, Copy)]
pub struct Voicing {
    pub voices: usize,
    pub rate_hz: f32,
    pub delay_ms: f32,
    /// Swing either side of the delay, in milliseconds.
    pub depth_ms: f32,
    pub shape: Shape,
    /// Weight of the faster sine mixed into each LFO.
    pub ripple: f32,
    pub wet_hp: bool,
    /// Each voice's LFO offset, in cycles.
    pub offset: [f32; MAX_VOICES],
    /// Each voice's position, -1 (left) to 1 (right).
    pub pan: [f32; MAX_VOICES],
    /// Each voice's gain into the left and right outputs.
    pub weight: [[f32; MAX_CHANNELS]; MAX_VOICES],
}

impl Voicing {
    pub fn of(s: &Settings) -> Self {
        let swing = s.depth * MAX_SWING * s.delay_ms;
        let (voices, rate_hz, delay_ms, depth_ms, shape, ripple, wet_hp) = match s.mode {
            Mode::Free => (s.voices.clamp(1, MAX_VOICES), s.rate_hz, s.delay_ms, swing, s.shape, 0.0, false),
            Mode::JunoI => (2, 0.513, 3.4, 1.9, Shape::Triangle, 0.0, false),
            Mode::JunoII => (2, 0.863, 3.4, 1.9, Shape::Triangle, 0.0, false),
            Mode::JunoBoth => (2, 9.75, 3.4, 0.4, Shape::Triangle, 0.0, false),
            Mode::Dimension => (2, 0.18 + 0.8 * s.depth, 6.5, 0.2 + 1.2 * s.depth, Shape::Sine, 0.0, true),
            Mode::Ensemble => (MAX_VOICES, s.rate_hz, s.delay_ms, swing, Shape::Sine, 0.25, false),
        };
        let (offset, layout) = match voices {
            1 => ([0.0; 3], [0.0; 3]),
            2 => ([0.0, 0.5, 0.0], [-1.0, 1.0, 0.0]),
            _ => ([0.0, 1.0 / 3.0, 2.0 / 3.0], [-1.0, 0.0, 1.0]),
        };
        let spread = s.spread.clamp(0.0, 1.0);
        let mut pan = [0.0; MAX_VOICES];
        let mut weight = [[0.0; MAX_CHANNELS]; MAX_VOICES];
        let mut energy = [0.0f32; MAX_CHANNELS];
        for i in 0..voices {
            pan[i] = layout[i] * spread;
            let theta = (pan[i] + 1.0) * 0.5 * FRAC_PI_2;
            weight[i] = [theta.cos(), theta.sin()];
            energy[0] += weight[i][0] * weight[i][0];
            energy[1] += weight[i][1] * weight[i][1];
        }
        // Keep the louder side's incoherent sum at unity however the voices are placed.
        let norm = 1.0 / energy[0].max(energy[1]).sqrt();
        for w in &mut weight[..voices] {
            w[0] *= norm;
            w[1] *= norm;
        }
        Self { voices, rate_hz, delay_ms, depth_ms, shape, ripple, wet_hp, offset, pan, weight }
    }
}

/// What the audio thread publishes for the editor: a ring of per-column voice positions.
pub struct Meter {
    /// Number of columns written so far; column `n` lives in slot `n % COLS`.
    pub head: AtomicUsize,
    /// Each voice's delay in milliseconds and LFO phase (0..1) at the end of each column.
    pub delay: [[AtomicF32; COLS]; MAX_VOICES],
    pub phase: [[AtomicF32; COLS]; MAX_VOICES],
    /// Peak of the dry input and of the wet voices over each column.
    pub dry: [AtomicF32; COLS],
    pub wet: [AtomicF32; COLS],
    /// Pitch deviation of the first voice, averaged over the latest column, in cents.
    pub cents: AtomicF32,
    /// The LFO rate in use, after the mode and tempo sync have had their say.
    pub rate_hz: AtomicF32,
}

impl Meter {
    fn new() -> Self {
        Self {
            head: AtomicUsize::new(0),
            delay: std::array::from_fn(|_| std::array::from_fn(|_| AtomicF32::new(0.0))),
            phase: std::array::from_fn(|_| std::array::from_fn(|_| AtomicF32::new(0.0))),
            dry: std::array::from_fn(|_| AtomicF32::new(0.0)),
            wet: std::array::from_fn(|_| AtomicF32::new(0.0)),
            cents: AtomicF32::new(0.0),
            rate_hz: AtomicF32::new(0.0),
        }
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

#[derive(Clone, Copy)]
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
    fn new() -> Self {
        Self { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0, z1: 0.0, z2: 0.0 }
    }

    /// Butterworth low-pass.
    fn set_lowpass(&mut self, hz: f32, sample_rate: f32) {
        let (s, c) = (TAU * hz / sample_rate).sin_cos();
        let alpha = s / (2.0 * std::f32::consts::FRAC_1_SQRT_2);
        let a0 = 1.0 + alpha;
        self.b0 = (1.0 - c) * 0.5 / a0;
        self.b1 = (1.0 - c) / a0;
        self.b2 = self.b0;
        self.a1 = -2.0 * c / a0;
        self.a2 = (1.0 - alpha) / a0;
    }

    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

fn one_pole(hz: f32, sample_rate: f32) -> f32 {
    1.0 - (-TAU * hz / sample_rate).exp()
}

/// One voice's LFO and the random walk behind the `Random` shape.
#[derive(Clone, Copy)]
struct Lfo {
    last_phase: f32,
    target: f32,
    held: f32,
    rng: u32,
}

impl Lfo {
    fn new(seed: u32) -> Self {
        Self { last_phase: 0.0, target: 0.0, held: 0.0, rng: seed }
    }

    /// -1..1.
    fn random(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        x as f32 / u32::MAX as f32 * 2.0 - 1.0
    }

    /// The wave at `phase` (0..1), -1..1. `glide` is the one-pole coefficient for `Random`.
    fn value(&mut self, shape: Shape, phase: f32, glide: f32) -> f32 {
        match shape {
            Shape::Sine => (TAU * phase).sin(),
            Shape::Triangle => 4.0 * (phase - 0.5).abs() - 1.0,
            Shape::Random => {
                if phase < self.last_phase {
                    self.target = self.random();
                }
                self.last_phase = phase;
                self.held += (self.target - self.held) * glide;
                self.held
            }
        }
    }
}

pub struct Chorus {
    sample_rate: f32,
    channels: usize,
    line: Vec<f32>,
    pos: usize,
    phase: f32,
    ripple_phase: f32,
    lfo: [Lfo; MAX_VOICES],
    band_limit: Biquad,
    /// Per-voice reconstruction filter state and its coefficient.
    post: [f32; MAX_VOICES],
    post_k: f32,
    noise: [Lfo; MAX_VOICES],
    noise_lp: [f32; MAX_VOICES],
    hp_lp: [f32; MAX_CHANNELS],
    hp_k: f32,
    tick: u32,
    last_delay: f32,
    pub meter: Arc<Meter>,
    col_len: usize,
    col_count: usize,
    col_dry: f32,
    col_wet: f32,
    col_cents: f32,
    col_delay: [f32; MAX_VOICES],
    col_phase: [f32; MAX_VOICES],
    col_rate: f32,
}

impl Chorus {
    pub fn new() -> Self {
        Self {
            sample_rate: 44100.0,
            channels: 2,
            line: Vec::new(),
            pos: 0,
            phase: 0.0,
            ripple_phase: 0.0,
            lfo: [Lfo::new(0x9e37_79b9), Lfo::new(0x85eb_ca6b), Lfo::new(0xc2b2_ae35)],
            band_limit: Biquad::new(),
            post: [0.0; MAX_VOICES],
            post_k: 1.0,
            noise: [Lfo::new(0x27d4_eb2f), Lfo::new(0x1656_67b1), Lfo::new(0xd3a2_646c)],
            noise_lp: [0.0; MAX_VOICES],
            hp_lp: [0.0; MAX_CHANNELS],
            hp_k: 0.0,
            tick: 0,
            last_delay: 0.0,
            meter: Arc::new(Meter::new()),
            col_len: 288,
            col_count: 0,
            col_dry: 0.0,
            col_wet: 0.0,
            col_cents: 0.0,
            col_delay: [0.0; MAX_VOICES],
            col_phase: [0.0; MAX_VOICES],
            col_rate: 0.0,
        }
    }

    pub fn configure(&mut self, sample_rate: f32, channels: usize) {
        self.sample_rate = sample_rate;
        self.channels = channels.clamp(1, MAX_CHANNELS);
        self.line.clear();
        self.line.resize((LINE_MS / 1000.0 * sample_rate).ceil() as usize + 8, 0.0);
        self.hp_k = one_pole(DIMENSION_HP_HZ, sample_rate);
        self.col_len = ((COL_MS / 1000.0 * sample_rate) as usize).max(1);
        self.reset();
    }

    pub fn reset(&mut self) {
        self.line.fill(0.0);
        self.pos = 0;
        self.band_limit = Biquad::new();
        self.post = [0.0; MAX_VOICES];
        self.noise_lp = [0.0; MAX_VOICES];
        self.hp_lp = [0.0; MAX_CHANNELS];
        self.tick = 0;
        self.col_count = 0;
        self.col_dry = 0.0;
        self.col_wet = 0.0;
        self.col_cents = 0.0;
    }

    /// The delay each voice read at in the last frame, in milliseconds.
    pub fn delays_ms(&self) -> [f32; MAX_VOICES] {
        self.col_delay
    }

    /// Process one frame. With one channel only `x[0]` and the returned `[0]` are used.
    pub fn process(&mut self, x: [f32; MAX_CHANNELS], s: &Settings) -> [f32; MAX_CHANNELS] {
        let sr = self.sample_rate;
        let len = self.line.len();
        let stereo = self.channels > 1;
        let v = Voicing::of(s);
        let analog = s.analog.clamp(0.0, 1.0);

        if self.tick == 0 {
            // A BBD's bandwidth falls as its delay grows, because its clock slows down.
            let cut = (BBD_CUT_HZ * BBD_REF_MS / v.delay_ms.max(0.1)).clamp(BBD_MIN_CUT_HZ, BBD_CUT_HZ).min(0.45 * sr);
            self.band_limit.set_lowpass(cut, sr);
            self.post_k = one_pole((cut * 1.5).min(0.45 * sr), sr);
        }
        self.tick = (self.tick + 1) % COEFF_EVERY;

        // Into the line: mono sum, band-limited and softly saturated by the analog amount.
        let mono = if stereo { 0.5 * (x[0] + x[1]) } else { x[0] };
        let limited = mono + (self.band_limit.process(mono) - mono) * analog;
        let drive = 1.0 + 3.0 * analog;
        let saturated = (limited * drive).tanh() / drive;
        self.line[self.pos] = limited + (saturated - limited) * analog;

        self.phase = (self.phase + v.rate_hz / sr).fract();
        self.ripple_phase = (self.ripple_phase + v.rate_hz * RIPPLE_RATIO / sr).fract();
        let glide = one_pole(v.rate_hz * 1.5, sr);

        let mut wet = [0.0f32; MAX_CHANNELS];
        let mono_gain = 1.0 / (v.voices as f32).sqrt();
        for i in 0..v.voices {
            let phase = (self.phase + v.offset[i]).fract();
            let mut sweep = self.lfo[i].value(v.shape, phase, glide);
            if v.ripple > 0.0 {
                let ripple = (TAU * (self.ripple_phase + v.offset[i]).fract()).sin();
                sweep = sweep * (1.0 - v.ripple) + ripple * v.ripple;
            }
            let delay_ms = v.delay_ms + v.depth_ms * sweep;
            let delay = (delay_ms * sr / 1000.0).clamp(2.0, (len - 8) as f32);
            let mut read = self.pos as f32 - delay;
            if read < 0.0 {
                read += len as f32;
            }
            let raw = read_hermite(&self.line, read);
            self.post[i] += self.post_k * (raw - self.post[i]);
            let mut y = raw + (self.post[i] - raw) * analog;
            let hiss = self.noise[i].random();
            self.noise_lp[i] += 0.5 * (hiss - self.noise_lp[i]);
            y += self.noise_lp[i] * BBD_NOISE * analog;

            if stereo {
                wet[0] += y * v.weight[i][0];
                wet[1] += y * v.weight[i][1];
            } else {
                wet[0] += y * mono_gain;
            }
            self.col_delay[i] = delay_ms;
            self.col_phase[i] = phase;
        }
        self.pos = (self.pos + 1) % len;

        if v.wet_hp {
            for c in 0..self.channels {
                self.hp_lp[c] += self.hp_k * (wet[c] - self.hp_lp[c]);
                wet[c] -= self.hp_lp[c];
            }
        }

        let dry_gain = (2.0 * (1.0 - s.mix)).min(1.0);
        let wet_gain = (2.0 * s.mix).min(1.0);
        let mut out = x;
        for c in 0..self.channels {
            out[c] = x[c] * dry_gain + wet[c] * wet_gain;
            self.col_dry = self.col_dry.max(x[c].abs());
            self.col_wet = self.col_wet.max((wet[c] * wet_gain).abs());
        }

        // Speed of the first voice's read head relative to the write head is `1 - d(delay)/dt`.
        let dd = (self.col_delay[0] - self.last_delay) * sr / 1000.0;
        self.last_delay = self.col_delay[0];
        self.col_cents += 1200.0 * (1.0 - dd.clamp(-0.5, 0.5)).log2();
        self.col_rate = v.rate_hz;
        self.col_count += 1;
        if self.col_count >= self.col_len {
            self.publish();
        }
        out
    }

    fn publish(&mut self) {
        let m = &self.meter;
        let slot = m.head.load(Ordering::Relaxed) % COLS;
        for i in 0..MAX_VOICES {
            m.delay[i][slot].store(self.col_delay[i], Ordering::Relaxed);
            m.phase[i][slot].store(self.col_phase[i], Ordering::Relaxed);
        }
        m.dry[slot].store(self.col_dry, Ordering::Relaxed);
        m.wet[slot].store(self.col_wet, Ordering::Relaxed);
        m.cents.store(self.col_cents / self.col_count as f32, Ordering::Relaxed);
        m.rate_hz.store(self.col_rate, Ordering::Relaxed);
        m.head.fetch_add(1, Ordering::Release);
        self.col_count = 0;
        self.col_dry = 0.0;
        self.col_wet = 0.0;
        self.col_cents = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48000.0;

    fn chorus(channels: usize) -> Chorus {
        let mut c = Chorus::new();
        c.configure(SR, channels);
        c
    }

    /// One plain voice, no modulation, wet only, no analog colour.
    fn plain() -> Settings {
        Settings {
            mode: Mode::Free,
            voices: 1,
            depth: 0.0,
            delay_ms: 10.0,
            analog: 0.0,
            spread: 0.0,
            mix: 1.0,
            ..Settings::default()
        }
    }

    fn noise(n: usize, seed: u32) -> Vec<f32> {
        let mut state = seed;
        (0..n)
            .map(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                (state >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
            })
            .collect()
    }

    fn run(c: &mut Chorus, input: &[f32], s: &Settings) -> Vec<[f32; 2]> {
        input.iter().map(|&x| c.process([x, x], s)).collect()
    }

    fn rms(x: impl Iterator<Item = f32>) -> f32 {
        let (sum, n) = x.fold((0.0f32, 0usize), |(s, n), v| (s + v * v, n + 1));
        (sum / n.max(1) as f32).sqrt()
    }

    #[test]
    fn zero_mix_is_a_null_test() {
        let input = noise(20000, 1);
        for mode in [Mode::Free, Mode::JunoI, Mode::Dimension, Mode::Ensemble] {
            let mut c = chorus(2);
            let s = Settings { mode, mix: 0.0, analog: 1.0, ..Settings::default() };
            for (y, x) in run(&mut c, &input, &s).iter().zip(&input) {
                assert_eq!(y[0], *x);
                assert_eq!(y[1], *x);
            }
        }
    }

    #[test]
    fn an_unmodulated_voice_is_the_input_delayed_by_the_delay() {
        let mut input = vec![0.0; 4000];
        input[0] = 1.0;
        let out = run(&mut chorus(2), &input, &plain());
        let at = (10.0 * SR / 1000.0) as usize;
        assert!((out[at][0] - 1.0).abs() < 1e-5, "{}", out[at][0]);
        assert!((out[at][1] - 1.0).abs() < 1e-5);
        let elsewhere = out.iter().enumerate().filter(|(i, _)| *i != at).map(|(_, o)| o[0].abs()).fold(0.0, f32::max);
        assert!(elsewhere < 1e-5);
    }

    #[test]
    fn depth_swings_the_delay_by_that_fraction() {
        let mut c = chorus(2);
        let s = Settings { depth: 1.0, rate_hz: 1.0, ..plain() };
        let delays: Vec<f32> = (0..(2.0 * SR) as usize)
            .map(|_| {
                c.process([0.0, 0.0], &s);
                c.delays_ms()[0]
            })
            .collect();
        let (lo, hi) = delays.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &d| (lo.min(d), hi.max(d)));
        assert!((lo - 2.0).abs() < 0.05 && (hi - 18.0).abs() < 0.05, "{lo} {hi}");
    }

    #[test]
    fn three_voices_sit_a_third_of_a_cycle_apart() {
        let mut c = chorus(2);
        let s = Settings { voices: 3, depth: 0.5, rate_hz: 1.0, ..plain() };
        for _ in 0..12345 {
            c.process([0.0, 0.0], &s);
            let d = c.delays_ms();
            // Three sines 120 degrees apart cancel, leaving the centre delay.
            assert!((d.iter().sum::<f32>() - 30.0).abs() < 0.01, "{d:?}");
        }
    }

    #[test]
    fn random_shape_stays_inside_the_swing_and_actually_moves() {
        let mut c = chorus(1);
        let s = Settings { shape: Shape::Random, depth: 0.5, rate_hz: 2.0, ..plain() };
        let delays: Vec<f32> = (0..(6.0 * SR) as usize)
            .map(|_| {
                c.process([0.0, 0.0], &s);
                c.delays_ms()[0]
            })
            .collect();
        let (lo, hi) = delays.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &d| (lo.min(d), hi.max(d)));
        assert!(lo >= 10.0 - 4.0 - 1e-3 && hi <= 10.0 + 4.0 + 1e-3, "{lo} {hi}");
        assert!(hi - lo > 3.0, "{lo} {hi}");
    }

    #[test]
    fn fixed_modes_ignore_rate_and_delay() {
        let a = Settings { mode: Mode::JunoII, rate_hz: 0.1, delay_ms: 3.0, ..Settings::default() };
        let b = Settings { mode: Mode::JunoII, rate_hz: 9.0, delay_ms: 20.0, ..Settings::default() };
        let (va, vb) = (Voicing::of(&a), Voicing::of(&b));
        assert_eq!((va.rate_hz, va.delay_ms, va.depth_ms), (vb.rate_hz, vb.delay_ms, vb.depth_ms));
        assert_eq!(va.voices, 2);
        assert_eq!(va.pan[..2], [-1.0, 1.0]);
    }

    #[test]
    fn dimension_depth_is_the_intensity() {
        let soft = Voicing::of(&Settings { mode: Mode::Dimension, depth: 0.1, ..Settings::default() });
        let hard = Voicing::of(&Settings { mode: Mode::Dimension, depth: 1.0, ..Settings::default() });
        assert!(hard.depth_ms > soft.depth_ms * 3.0 && hard.rate_hz > soft.rate_hz * 2.0);
    }

    #[test]
    fn stereo_follows_spread_and_is_identical_at_zero() {
        let input = noise(30000, 2);
        let s = Settings { voices: 2, depth: 0.5, mix: 0.5, ..Settings::default() };
        let narrow = run(&mut chorus(2), &input, &Settings { spread: 0.0, ..s });
        assert!(narrow.iter().all(|o| o[0] == o[1]));
        let wide = run(&mut chorus(2), &input, &Settings { spread: 1.0, ..s });
        let side = rms(wide.iter().map(|o| o[0] - o[1]));
        let mid = rms(wide.iter().map(|o| o[0] + o[1]));
        assert!(side > 0.2 * mid, "side {side} mid {mid}");
    }

    #[test]
    fn dimension_is_mono_compatible() {
        // The mono sum keeps nearly all the stereo energy: no deep comb notches or cancellation.
        let input = noise(60000, 3);
        let s = Settings { mode: Mode::Dimension, depth: 0.5, mix: 0.5, spread: 1.0, analog: 0.4, ..Settings::default() };
        let out = run(&mut chorus(2), &input, &s);
        let mono = rms(out.iter().map(|o| 0.5 * (o[0] + o[1])));
        let stereo = rms(out.iter().flat_map(|o| [o[0], o[1]]));
        assert!(mono > 0.7 * stereo, "mono {mono} stereo {stereo}");
    }

    #[test]
    fn wet_level_matches_dry_level() {
        let input = noise(60000, 4);
        let dry = rms(input.iter().copied());
        for (voices, spread) in [(1, 0.0), (2, 1.0), (3, 1.0), (3, 0.0)] {
            let s = Settings { voices, spread, depth: 0.3, mix: 1.0, analog: 0.0, ..Settings::default() };
            let out = run(&mut chorus(2), &input, &s);
            let wet = rms(out.iter().flat_map(|o| [o[0], o[1]]));
            assert!(wet > 0.6 * dry && wet < 1.25 * dry, "voices {voices}: wet {wet} dry {dry}");
        }
    }

    #[test]
    fn analog_darkens_and_adds_hiss_only_when_asked() {
        let input = noise(40000, 5);
        let brightness = |analog: f32| {
            let out = run(&mut chorus(1), &input, &Settings { analog, ..plain() });
            let x: Vec<f32> = out.iter().map(|o| o[0]).skip(2000).collect();
            rms(x.windows(2).map(|w| w[1] - w[0])) / rms(x.iter().copied())
        };
        assert!(brightness(1.0) < 0.7 * brightness(0.0));

        let silence = vec![0.0; 20000];
        let clean = run(&mut chorus(2), &silence, &plain());
        assert!(clean.iter().all(|o| o[0] == 0.0 && o[1] == 0.0));
        let hiss = run(&mut chorus(2), &silence, &Settings { analog: 1.0, ..plain() });
        let level = rms(hiss.iter().map(|o| o[0]));
        assert!(level > 1e-5 && level < 1e-3, "{level}");
    }

    #[test]
    fn dimension_removes_the_low_end_from_the_wet() {
        let tone: Vec<f32> = (0..48000).map(|n| (TAU * 40.0 * n as f32 / SR).sin()).collect();
        let s = |mode| Settings { mode, mix: 1.0, analog: 0.0, depth: 0.2, ..Settings::default() };
        let level = |mode| rms(run(&mut chorus(2), &tone, &s(mode)).iter().skip(24000).map(|o| o[0]));
        assert!(level(Mode::Dimension) < 0.5 * level(Mode::Free));
    }

    #[test]
    fn tempo_sync_gives_one_cycle_per_division() {
        assert_eq!(synced_rate_hz(120.0, Division::Quarter), Some(2.0));
        assert_eq!(synced_rate_hz(120.0, Division::Bar), Some(0.5));
        assert_eq!(synced_rate_hz(120.0, Division::Sixteenth), Some(8.0));
        assert_eq!(synced_rate_hz(120.0, Division::Off), None);
    }

    #[test]
    fn output_stays_finite_at_extremes() {
        let input = noise(30000, 6);
        for mode in [Mode::Free, Mode::JunoI, Mode::JunoII, Mode::JunoBoth, Mode::Dimension, Mode::Ensemble] {
            for channels in [1, 2] {
                for (rate_hz, depth, delay_ms, shape) in [(10.0, 1.0, 25.0, Shape::Random), (0.05, 1.0, 0.5, Shape::Triangle), (5.0, 0.0, 25.0, Shape::Sine)] {
                    let mut c = chorus(channels);
                    let s = Settings { mode, rate_hz, depth, delay_ms, shape, voices: 3, spread: 1.0, analog: 1.0, mix: 1.0 };
                    for &x in &input {
                        let out = c.process([x, -x], &s);
                        assert!(out[0].is_finite() && out[1].is_finite() && out[0].abs() < 4.0 && out[1].abs() < 4.0);
                    }
                }
            }
        }
    }

    #[test]
    fn meter_publishes_columns() {
        let mut c = chorus(2);
        let s = Settings { depth: 0.5, ..Settings::default() };
        for n in 0..(SR * 0.1) as usize {
            c.process([(TAU * 220.0 * n as f32 / SR).sin() * 0.5; 2], &s);
        }
        let m = &c.meter;
        assert!(m.head.load(Ordering::Relaxed) >= 15);
        assert!(m.delay[2][3].load(Ordering::Relaxed) > 1.0);
        assert!((m.dry[3].load(Ordering::Relaxed) - 0.5).abs() < 0.01);
        assert!((m.rate_hz.load(Ordering::Relaxed) - 0.6).abs() < 1e-6);
    }
}
