//! A delay whose feedback loop is an instrument.
//!
//! A plain delay feeds back clean copies. A character delay instead processes the signal on every trip round the loop, so
//! each repeat is a little further from the original than the one before. This does the same: the delayed signal
//! passes through
//!
//! 1. a four-stage allpass **diffusion** network that smears transients into a cloud,
//! 2. a granular **pitch shifter** (two Hann-windowed taps), so repeats climb or fall,
//! 3. a high-pass and a 12 dB/octave low-pass, so repeats thin out and darken,
//! 4. an antialiased **tanh saturator**, so loud repeats compress as on tape,
//!
//! and only then is it heard and fed back. Because the processed signal is also the wet output,
//! the first repeat already carries the colour. The delay time is read with a four-point
//! Catmull-Rom interpolator and slides with a one-pole, so changing it bends the pitch as a tape
//! machine's speed knob does, and a sine plus a smoothed random walk modulates it (wow and
//! flutter). Ping-pong crosses the channels in the feedback and steers the input into one side.
//! The wet signal is **ducked** by the input level, so repeats bloom in the gaps.
//!
//! The delay line adds no latency, so the dry path needs none.

use nih_plug::prelude::{AtomicF32, Enum};
use std::f32::consts::{LN_2, PI, TAU};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

pub const MAX_CHANNELS: usize = 2;
pub const MIN_TIME_MS: f32 = 1.0;
pub const MAX_TIME_MS: f32 = 4000.0;
/// Largest modulation excursion in milliseconds, reached at low rates.
const MAX_WOW_MS: f32 = 8.0;
/// The excursion is also limited to `WOW_BUDGET / rate`, which bounds the pitch deviation.
const WOW_BUDGET: f32 = 6.0;
/// Time constant of the delay-time slide, in seconds.
const TIME_SLIDE_S: f32 = 0.06;
/// Allpass delays of the diffusion network, in milliseconds.
const DIFFUSER_MS: [f32; 4] = [2.3, 4.7, 8.9, 14.3];
/// Grain length of the pitch shifter, in seconds.
const GRAIN_S: f32 = 0.05;

/// Telemetry: the editor shows the last `WINDOW_S` seconds in `COLS` columns.
pub const COLS: usize = 240;
pub const WINDOW_S: f32 = 2.4;

/// A note length for the synced delay time.
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Division {
    #[name = "1/16"]
    Sixteenth,
    #[name = "1/8T"]
    EighthTriplet,
    #[name = "1/8"]
    Eighth,
    #[name = "1/8."]
    DottedEighth,
    #[name = "1/4T"]
    QuarterTriplet,
    #[name = "1/4"]
    Quarter,
    #[name = "1/4."]
    DottedQuarter,
    #[name = "1/2"]
    Half,
    #[name = "1/1"]
    Whole,
}

impl Division {
    pub const ALL: [Division; 9] = [
        Division::Sixteenth,
        Division::EighthTriplet,
        Division::Eighth,
        Division::DottedEighth,
        Division::QuarterTriplet,
        Division::Quarter,
        Division::DottedQuarter,
        Division::Half,
        Division::Whole,
    ];

    /// Length in quarter notes.
    pub fn beats(self) -> f32 {
        match self {
            Division::Sixteenth => 0.25,
            Division::EighthTriplet => 1.0 / 3.0,
            Division::Eighth => 0.5,
            Division::DottedEighth => 0.75,
            Division::QuarterTriplet => 2.0 / 3.0,
            Division::Quarter => 1.0,
            Division::DottedQuarter => 1.5,
            Division::Half => 2.0,
            Division::Whole => 4.0,
        }
    }

    pub fn time_ms(self, bpm: f32) -> f32 {
        (60_000.0 / bpm.max(1.0) * self.beats()).clamp(MIN_TIME_MS, MAX_TIME_MS)
    }
}

/// What the audio thread publishes for the editor: rings of per-column peaks.
pub struct Meter {
    /// Number of columns written so far; column `n` lives in slot `n % COLS`.
    pub head: AtomicUsize,
    pub input: [AtomicF32; COLS],
    /// The wet signal as heard, after ducking.
    pub wet: [AtomicF32; COLS],
    /// Gain the ducker is applying to the wet signal right now (1 = none).
    pub duck_gain: AtomicF32,
}

impl Meter {
    fn new() -> Self {
        Self {
            head: AtomicUsize::new(0),
            input: std::array::from_fn(|_| AtomicF32::new(0.0)),
            wet: std::array::from_fn(|_| AtomicF32::new(0.0)),
            duck_gain: AtomicF32::new(1.0),
        }
    }
}

#[derive(Clone, Copy)]
pub struct Settings {
    pub time_ms: f32,
    /// Loop gain; above 1 the saturator is what keeps it from running away.
    pub feedback: f32,
    /// 0..1: how far the repeats cross between the channels.
    pub pingpong: f32,
    pub hp_hz: f32,
    pub lp_hz: f32,
    /// 0..1: saturation in the loop.
    pub drive: f32,
    /// 0..1: allpass smear in the loop.
    pub diffusion: f32,
    /// Pitch shift per trip round the loop, in semitones.
    pub pitch: f32,
    /// 0..1: wow and flutter depth.
    pub mod_depth: f32,
    pub mod_rate_hz: f32,
    /// 0..1: how far the input level pushes the wet signal down.
    pub duck: f32,
    /// 0..1, equal power.
    pub mix: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            time_ms: 350.0,
            feedback: 0.5,
            pingpong: 0.0,
            hp_hz: 120.0,
            lp_hz: 6000.0,
            drive: 0.2,
            diffusion: 0.0,
            pitch: 0.0,
            mod_depth: 0.2,
            mod_rate_hz: 0.6,
            duck: 0.0,
            mix: 0.35,
        }
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
        (self.s2 * 1.5).clamp(-1.0, 1.0)
    }
}

fn ln_cosh(x: f32) -> f32 {
    let a = x.abs();
    a + (-2.0 * a).exp().ln_1p() - LN_2
}

/// `tanh(g x) / g`, with first-order antiderivative antialiasing.
#[derive(Default)]
struct Saturator {
    x1: f32,
    f1: f32,
}

impl Saturator {
    fn process(&mut self, x: f32, drive: f32) -> f32 {
        let g = 1.0 + 3.0 * drive;
        let u = x * g;
        let f = ln_cosh(u);
        let d = u - self.x1;
        let y = if d.abs() < 1e-4 { (0.5 * (u + self.x1)).tanh() } else { (f - self.f1) / d };
        self.x1 = u;
        self.f1 = f;
        y / g
    }
}

/// Schroeder allpass: `w = x + g d; y = d - g w`.
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

    fn clear(&mut self) {
        self.buf.fill(0.0);
    }
}

/// Two-tap granular pitch shifter. The taps read a short buffer at delays that sweep through
/// `0..grain` at `1 - ratio` samples per sample, and fade with `sin²` so the two always sum to one.
struct PitchShifter {
    buf: Vec<f32>,
    w: usize,
    phase: f32,
    grain: f32,
    /// 0 when bypassed, 1 when shifting: a smoothed switch so engaging it doesn't click.
    engaged: f32,
}

impl PitchShifter {
    fn new(sample_rate: f32) -> Self {
        let grain = (GRAIN_S * sample_rate).round();
        Self { buf: vec![0.0; grain as usize + 4], w: 0, phase: 0.0, grain, engaged: 0.0 }
    }

    fn process(&mut self, x: f32, ratio: f32, shifting: bool) -> f32 {
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
        self.engaged += 0.002 * (if shifting { 1.0 } else { 0.0 } - self.engaged);
        x + (y - x) * self.engaged
    }

    fn clear(&mut self) {
        self.buf.fill(0.0);
        self.engaged = 0.0;
    }
}

/// Four-point Catmull-Rom read, `delay` samples behind the newest write at `pos - 1`.
fn read_delayed(line: &[f32], pos: usize, delay: f32) -> f32 {
    let len = line.len();
    let i = delay.floor();
    let f = delay - i;
    let at = |back: usize| line[(pos + len - back) % len];
    let i = i as usize;
    let (ym1, y0, y1, y2) = (at(i - 1), at(i), at(i + 1), at(i + 2));
    let c1 = 0.5 * (y1 - ym1);
    let c2 = ym1 - 2.5 * y0 + 2.0 * y1 - 0.5 * y2;
    let c3 = 0.5 * (y2 - ym1) + 1.5 * (y0 - y1);
    ((c3 * f + c2) * f + c1) * f + y0
}

fn one_pole(cutoff_hz: f32, sample_rate: f32) -> f32 {
    1.0 - (-TAU * cutoff_hz.min(sample_rate * 0.45) / sample_rate).exp()
}

pub struct Echo {
    sample_rate: f32,
    channels: usize,
    lines: [Vec<f32>; MAX_CHANNELS],
    pos: usize,
    /// The delay time as slid towards its target, in samples; `None` until the first sample.
    time: Option<f32>,
    lfo_phase: f32,
    wander: [Wander; MAX_CHANNELS],
    wander_coeff: f32,
    diffusers: [[Allpass; 4]; MAX_CHANNELS],
    pitch: [PitchShifter; MAX_CHANNELS],
    hp_lp: [f32; MAX_CHANNELS],
    lp1: [f32; MAX_CHANNELS],
    lp2: [f32; MAX_CHANNELS],
    sat: [Saturator; MAX_CHANNELS],
    /// Filter coefficients, cached against the cutoffs they were made for.
    coeffs: (f32, f32, f32, f32),
    duck_env: f32,
    duck_attack: f32,
    duck_release: f32,
    slide: f32,
    pub meter: Arc<Meter>,
    col_len: usize,
    col_count: usize,
    col_input: f32,
    col_wet: f32,
}

impl Echo {
    pub fn new() -> Self {
        let mut echo = Self {
            sample_rate: 44100.0,
            channels: 2,
            lines: [Vec::new(), Vec::new()],
            pos: 0,
            time: None,
            lfo_phase: 0.0,
            wander: [Wander::new(0x9E37_79B9), Wander::new(0x85EB_CA6B)],
            wander_coeff: 0.0,
            diffusers: std::array::from_fn(|_| std::array::from_fn(|_| Allpass::new(1))),
            pitch: [PitchShifter::new(44100.0), PitchShifter::new(44100.0)],
            hp_lp: [0.0; MAX_CHANNELS],
            lp1: [0.0; MAX_CHANNELS],
            lp2: [0.0; MAX_CHANNELS],
            sat: Default::default(),
            coeffs: (0.0, 0.0, 0.0, 0.0),
            duck_env: 0.0,
            duck_attack: 0.0,
            duck_release: 0.0,
            slide: 0.0,
            meter: Arc::new(Meter::new()),
            col_len: 1,
            col_count: 0,
            col_input: 0.0,
            col_wet: 0.0,
        };
        echo.configure(44100.0, 2);
        echo
    }

    /// Allocates for a sample rate; call from `initialize`, never from the audio thread.
    pub fn configure(&mut self, sample_rate: f32, channels: usize) {
        self.sample_rate = sample_rate;
        self.channels = channels.clamp(1, MAX_CHANNELS);
        let len = (sample_rate * (MAX_TIME_MS / 1000.0 + 0.05)) as usize + 8;
        self.lines = [vec![0.0; len], vec![0.0; len]];
        self.diffusers = std::array::from_fn(|_| {
            std::array::from_fn(|k| Allpass::new((DIFFUSER_MS[k] * 0.001 * sample_rate).round() as usize))
        });
        self.pitch = [PitchShifter::new(sample_rate), PitchShifter::new(sample_rate)];
        self.wander_coeff = 1.0 - (-TAU * 2.0 / sample_rate).exp();
        self.duck_attack = 1.0 - (-1.0 / (0.004 * sample_rate)).exp();
        self.duck_release = 1.0 - (-1.0 / (0.22 * sample_rate)).exp();
        self.slide = 1.0 - (-1.0 / (TIME_SLIDE_S * sample_rate)).exp();
        self.col_len = ((WINDOW_S * sample_rate / COLS as f32) as usize).max(1);
        self.reset();
    }

    pub fn reset(&mut self) {
        for line in &mut self.lines {
            line.fill(0.0);
        }
        for chain in &mut self.diffusers {
            for ap in chain {
                ap.clear();
            }
        }
        for p in &mut self.pitch {
            p.clear();
        }
        self.pos = 0;
        self.time = None;
        self.hp_lp = [0.0; MAX_CHANNELS];
        self.lp1 = [0.0; MAX_CHANNELS];
        self.lp2 = [0.0; MAX_CHANNELS];
        self.sat = Default::default();
        self.duck_env = 0.0;
        self.col_count = 0;
        self.col_input = 0.0;
        self.col_wet = 0.0;
    }

    /// How long the tail rings after the input stops, in samples, for the host's tail handling.
    pub fn tail_samples(&self, s: &Settings) -> u32 {
        let t = s.time_ms * 0.001 * self.sample_rate;
        if s.feedback >= 0.99 {
            return u32::MAX;
        }
        // Repeats until the loop has fallen 80 dB, plus the allpass ring-out.
        let repeats = (4.0 / (-s.feedback.max(0.05).log10())).ceil().min(400.0);
        (t * (repeats + 1.0)) as u32
    }

    /// One trip through the loop's character stages.
    fn colour(&mut self, ch: usize, x: f32, s: &Settings) -> f32 {
        let (a_hp, a_lp) = (self.coeffs.1, self.coeffs.3);

        let g = 0.72 * s.diffusion;
        let mut d = x;
        for ap in &mut self.diffusers[ch] {
            d = ap.process(d, g);
        }
        let x = x + (d - x) * s.diffusion;

        let shifting = s.pitch.abs() > 0.005;
        let ratio = (s.pitch / 12.0).exp2();
        let x = self.pitch[ch].process(x, ratio, shifting);

        self.hp_lp[ch] += a_hp * (x - self.hp_lp[ch]);
        let x = x - self.hp_lp[ch];
        self.lp1[ch] += a_lp * (x - self.lp1[ch]);
        self.lp2[ch] += a_lp * (self.lp1[ch] - self.lp2[ch]);

        self.sat[ch].process(self.lp2[ch], s.drive)
    }

    pub fn process(&mut self, x: [f32; MAX_CHANNELS], s: &Settings) -> [f32; MAX_CHANNELS] {
        let fs = self.sample_rate;
        if self.coeffs.0 != s.hp_hz || self.coeffs.2 != s.lp_hz {
            self.coeffs = (s.hp_hz, one_pole(s.hp_hz, fs), s.lp_hz, one_pole(s.lp_hz, fs));
        }
        let len = self.lines[0].len();

        // Delay time: slide towards the target, then add the wow and flutter.
        let target = (s.time_ms * 0.001 * fs).clamp(MIN_TIME_MS * 0.001 * fs, len as f32 - 8.0);
        let time = self.time.get_or_insert(target);
        *time += self.slide * (target - *time);
        let time = *time;
        self.lfo_phase = (self.lfo_phase + s.mod_rate_hz / fs).fract();
        let amp = s.mod_depth * MAX_WOW_MS.min(WOW_BUDGET / s.mod_rate_hz.max(0.05)) * 0.001 * fs;

        let channels = self.channels;
        let mut tap = [0.0_f32; MAX_CHANNELS];
        for ch in 0..channels {
            let sine = (TAU * (self.lfo_phase + 0.25 * ch as f32)).sin();
            let noise = self.wander[ch].next(s.mod_rate_hz, self.wander_coeff, fs);
            let delay = (time + amp * (0.7 * sine + 0.3 * noise)).clamp(3.0, len as f32 - 6.0);
            tap[ch] = self.colour(ch, read_delayed(&self.lines[ch], self.pos, delay), s);
        }

        // Ping-pong: feedback crosses over and the input is steered into the left line.
        let pp = if channels == 2 { s.pingpong } else { 0.0 };
        let mut inject = x;
        let mut feed = tap;
        if channels == 2 {
            inject = [(x[0] + pp * x[1]) / (1.0 + pp), x[1] * (1.0 - pp)];
            feed = [tap[0] + pp * (tap[1] - tap[0]), tap[1] + pp * (tap[0] - tap[1])];
        }
        for ch in 0..channels {
            let v = inject[ch] + s.feedback * feed[ch];
            self.lines[ch][self.pos] = if v.is_finite() { v } else { 0.0 };
        }
        self.pos = (self.pos + 1) % len;

        // Ducking: the input level pulls the wet signal down.
        let level = x[..channels].iter().fold(0.0_f32, |m, v| m.max(v.abs()));
        let coeff = if level > self.duck_env { self.duck_attack } else { self.duck_release };
        self.duck_env += coeff * (level - self.duck_env);
        let duck_gain = 1.0 - s.duck * self.duck_env / (self.duck_env + 0.06);

        let (dry, wet) = ((s.mix * PI * 0.5).cos(), (s.mix * PI * 0.5).sin() * duck_gain);
        let mut out = [0.0; MAX_CHANNELS];
        let mut peak = 0.0_f32;
        for ch in 0..channels {
            out[ch] = dry * x[ch] + wet * tap[ch];
            peak = peak.max((wet * tap[ch]).abs());
        }

        self.col_input = self.col_input.max(level);
        self.col_wet = self.col_wet.max(peak);
        self.col_count += 1;
        if self.col_count >= self.col_len {
            let head = self.meter.head.load(Ordering::Relaxed);
            let slot = head % COLS;
            self.meter.input[slot].store(self.col_input, Ordering::Relaxed);
            self.meter.wet[slot].store(self.col_wet, Ordering::Relaxed);
            self.meter.duck_gain.store(duck_gain, Ordering::Relaxed);
            self.meter.head.store(head.wrapping_add(1), Ordering::Release);
            self.col_count = 0;
            self.col_input = 0.0;
            self.col_wet = 0.0;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FS: f32 = 48000.0;

    fn clean() -> Settings {
        Settings {
            time_ms: 100.0,
            feedback: 0.5,
            hp_hz: 20.0,
            lp_hz: 20000.0,
            drive: 0.0,
            mod_depth: 0.0,
            mix: 1.0,
            ..Settings::default()
        }
    }

    fn run(s: &Settings, n: usize, mut input: impl FnMut(usize) -> f32) -> Vec<f32> {
        let mut echo = Echo::new();
        echo.configure(FS, 2);
        (0..n)
            .map(|i| {
                let v = input(i);
                echo.process([v, v], s)[0]
            })
            .collect()
    }

    fn sine(hz: f32, amp: f32) -> impl FnMut(usize) -> f32 {
        move |i| amp * (TAU * hz * i as f32 / FS).sin()
    }

    fn peak(x: &[f32], from: usize, to: usize) -> f32 {
        x[from..to].iter().fold(0.0_f32, |m, v| m.max(v.abs()))
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    #[test]
    fn mix_zero_is_a_null_test() {
        let s = Settings { mix: 0.0, ..Settings::default() };
        let mut phase = 0.0_f32;
        let input = move |_| {
            phase += 0.05;
            0.4 * phase.sin()
        };
        let mut reference = input.clone();
        let out = run(&s, 20000, input);
        for (i, y) in out.iter().enumerate() {
            assert_eq!(*y, reference(i));
        }
    }

    #[test]
    fn first_repeat_arrives_at_the_set_time() {
        let s = clean();
        let t = (100.0 * 0.001 * FS) as usize;
        let out = run(&s, 3 * t, |i| if i == 0 { 0.05 } else { 0.0 });
        let at = (1..out.len()).max_by(|&a, &b| out[a].abs().total_cmp(&out[b].abs())).unwrap();
        assert!((at as i64 - t as i64).abs() <= 1, "echo at {at}, expected {t}");
    }

    #[test]
    fn feedback_sets_how_fast_repeats_die() {
        let t = (100.0 * 0.001 * FS) as usize;
        let ratio = |fb: f32| {
            let s = Settings { feedback: fb, ..clean() };
            let out = run(&s, 5 * t, |i| if i == 0 { 0.05 } else { 0.0 });
            peak(&out, 3 * t - 20, 3 * t + 20) / peak(&out, t - 20, t + 20)
        };
        let (low, high) = (ratio(0.3), ratio(0.8));
        assert!(low < 0.6 * high, "{low} vs {high}");
        assert!(high < 1.0 && low > 0.0);
    }

    #[test]
    fn low_pass_in_the_loop_darkens_repeats() {
        let t = (100.0 * 0.001 * FS) as usize;
        let burst = |i: usize| if i < 480 { 0.2 * (TAU * 8000.0 * i as f32 / FS).sin() } else { 0.0 };
        let bright = run(&clean(), 3 * t, burst);
        let dark = run(&Settings { lp_hz: 500.0, ..clean() }, 3 * t, burst);
        let (b, d) = (peak(&bright, t, t + 600), peak(&dark, t, t + 600));
        assert!(d < 0.1 * b, "{d} vs {b}");
    }

    #[test]
    fn high_pass_in_the_loop_thins_repeats() {
        let t = (100.0 * 0.001 * FS) as usize;
        let burst = |i: usize| if i < 4800 { 0.2 * (TAU * 60.0 * i as f32 / FS).sin() } else { 0.0 };
        let full = run(&clean(), 3 * t, burst);
        let thin = run(&Settings { hp_hz: 1000.0, ..clean() }, 3 * t, burst);
        assert!(peak(&thin, t, t + 4800) < 0.1 * peak(&full, t, t + 4800));
    }

    #[test]
    fn pitch_shifts_the_repeats_an_octave() {
        let s = Settings { pitch: 12.0, feedback: 0.0, ..clean() };
        let out = run(&s, 96000, sine(1000.0, 0.1));
        let tail = &out[48000..];
        let crossings = tail.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        let hz = crossings as f32 / (tail.len() as f32 / FS);
        assert!((hz - 2000.0).abs() < 100.0, "{hz} Hz");
        assert!(rms(tail) > 0.03, "shifted signal too quiet");
    }

    #[test]
    fn pitch_zero_leaves_the_pitch_alone() {
        let s = Settings { feedback: 0.0, ..clean() };
        let out = run(&s, 96000, sine(1000.0, 0.1));
        let tail = &out[48000..];
        let crossings = tail.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        let hz = crossings as f32 / (tail.len() as f32 / FS);
        assert!((hz - 1000.0).abs() < 20.0, "{hz} Hz");
    }

    #[test]
    fn ducking_pulls_the_wet_down_under_loud_input() {
        let loud = |amount: f32| {
            let s = Settings { duck: amount, ..clean() };
            rms(&run(&s, 48000, sine(1000.0, 0.5))[24000..])
        };
        assert!(loud(1.0) < 0.3 * loud(0.0));
    }

    #[test]
    fn ping_pong_alternates_the_repeats() {
        let s = Settings { pingpong: 1.0, ..clean() };
        let mut echo = Echo::new();
        echo.configure(FS, 2);
        let t = (100.0 * 0.001 * FS) as usize;
        let mut l = vec![];
        let mut r = vec![];
        for i in 0..4 * t {
            let v = if i == 0 { 0.05 } else { 0.0 };
            let y = echo.process([v, v], &s);
            l.push(y[0]);
            r.push(y[1]);
        }
        // Repeats alternate: right is silent at the first, left at the second.
        assert!(peak(&l, t - 5, t + 5) > 0.02 && peak(&r, t - 5, t + 5) < 1e-4);
        assert!(peak(&r, 2 * t - 5, 2 * t + 5) > 0.01 && peak(&l, 2 * t - 5, 2 * t + 5) < 1e-4);
    }

    #[test]
    fn stays_finite_and_bounded_at_extremes() {
        let s = Settings {
            time_ms: 37.0,
            feedback: 1.1,
            pingpong: 0.7,
            drive: 1.0,
            diffusion: 1.0,
            pitch: 7.0,
            mod_depth: 1.0,
            mod_rate_hz: 5.0,
            hp_hz: 20.0,
            lp_hz: 20000.0,
            duck: 0.5,
            mix: 0.8,
        };
        let mut rng = 12345u32;
        let out = run(&s, 5 * 48000, |_| {
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            (rng as f32 / u32::MAX as f32 * 2.0 - 1.0) * 0.5
        });
        assert!(out.iter().all(|v| v.is_finite() && v.abs() < 8.0));
    }

    #[test]
    fn mono_layout_runs() {
        let mut echo = Echo::new();
        echo.configure(FS, 1);
        let s = Settings { pingpong: 1.0, ..clean() };
        let t = (100.0 * 0.001 * FS) as usize;
        let mut best = 0.0_f32;
        for i in 0..2 * t {
            let y = echo.process([if i == 0 { 0.05 } else { 0.0 }, 0.0], &s)[0];
            if i > t - 5 {
                best = best.max(y.abs());
            }
        }
        assert!(best > 0.02);
    }

    #[test]
    fn changing_the_time_bends_the_pitch_without_clicks() {
        let mut echo = Echo::new();
        echo.configure(FS, 2);
        let mut s = Settings { feedback: 0.0, ..clean() };
        let mut last = 0.0_f32;
        let mut worst = 0.0_f32;
        for i in 0..48000 {
            if i == 20000 {
                s.time_ms = 300.0;
            }
            let v = 0.2 * (TAU * 200.0 * i as f32 / FS).sin();
            let y = echo.process([v, v], &s)[0];
            if i > 10000 {
                worst = worst.max((y - last).abs());
            }
            last = y;
        }
        // A 200 Hz tone at 0.2 can step at most about 0.005 per sample, even when bent.
        assert!(worst < 0.03, "{worst}");
    }

    #[test]
    fn synced_times_follow_the_tempo() {
        assert!((Division::Quarter.time_ms(120.0) - 500.0).abs() < 1e-3);
        assert!((Division::DottedEighth.time_ms(120.0) - 375.0).abs() < 1e-3);
        assert!((Division::EighthTriplet.time_ms(90.0) - 222.2222).abs() < 1e-2);
        assert_eq!(Division::Whole.time_ms(20.0), MAX_TIME_MS);
    }
}
