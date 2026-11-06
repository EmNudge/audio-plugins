//! Distortion DSP: waveshapers, an oversampler built from cascaded halfband stages, and the
//! small filters around them.

use nih_plug::prelude::Enum;
use std::f32::consts::PI;

#[derive(Enum, Clone, Copy, PartialEq, Eq)]
pub enum Model {
    /// Soft on the positive side, softer still on the negative: even and odd harmonics.
    #[name = "Tube"]
    Tube,
    /// Gentle arctangent saturation.
    #[name = "Tape"]
    Tape,
    /// Exponential knee, with the negative half clipping harder than the positive half.
    #[name = "Diode"]
    Diode,
    /// Sine wavefolder: past full scale the wave turns back on itself.
    #[name = "Fold"]
    Fold,
    /// Hard clipper.
    #[name = "Clip"]
    Clip,
    /// Sharp-kneed saturation that stays near full scale: dense odd harmonics.
    #[name = "Crunch"]
    Crunch,
    /// The negative half barely moves, which leaves mostly even harmonics.
    #[name = "Rectify"]
    Rectify,
    /// Quantises to a few levels. Steps are harsh and alias without oversampling.
    #[name = "Crush"]
    Crush,
}

/// The waveshaper. `x` already includes the drive gain; `bias` is in -1..1. Small signals pass at
/// unity gain, and the output with zero input is zero whatever the bias.
pub fn shape(model: Model, x: f32, bias: f32) -> f32 {
    let b = bias * 0.5;
    f(model, x + b) - f(model, b)
}

fn f(model: Model, u: f32) -> f32 {
    match model {
        Model::Tube => {
            if u >= 0.0 {
                u.tanh()
            } else {
                u / (1.0 + u.abs())
            }
        }
        Model::Tape => (u * PI / 2.0).atan() * (2.0 / PI),
        Model::Diode => {
            if u >= 0.0 {
                1.0 - (-u).exp()
            } else {
                -(1.0 - (1.4 * u).exp()) / 1.4
            }
        }
        Model::Fold => u.sin(),
        Model::Clip => u.clamp(-1.0, 1.0),
        Model::Crunch => u / (1.0 + u.abs().powi(3)).cbrt(),
        Model::Rectify => {
            if u >= 0.0 {
                u.tanh()
            } else {
                0.2 * (u / 0.2).tanh()
            }
        }
        Model::Crush => (u.clamp(-1.0, 1.0) * 8.0).round() / 8.0,
    }
}

#[derive(Enum, Clone, Copy, PartialEq, Eq)]
pub enum Bands {
    #[name = "Single"]
    Single,
    #[name = "3-Band"]
    Multi,
}

#[derive(Enum, Clone, Copy, PartialEq, Eq)]
pub enum ModSource {
    #[name = "Off"]
    Off,
    #[name = "LFO"]
    Lfo,
    #[name = "Envelope"]
    Envelope,
}

#[derive(Enum, Clone, Copy, PartialEq, Eq)]
pub enum ModTarget {
    #[name = "Drive"]
    Drive,
    #[name = "Bias"]
    Bias,
    #[name = "Tone"]
    Tone,
}

#[derive(Enum, Clone, Copy, PartialEq, Eq)]
pub enum LfoShape {
    #[name = "Sine"]
    Sine,
    #[name = "Triangle"]
    Triangle,
    #[name = "Square"]
    Square,
}

#[derive(Enum, Clone, Copy, PartialEq, Eq)]
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

/// Round-trip delay, in samples at the base rate, of each oversampling setting.
pub const LATENCY: [usize; 4] = [0, 16, 24, 28];
pub const MAX_STAGES: usize = 3;

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

/// Runs a function at 1x, 2x, 4x or 8x the sample rate.
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

    /// Forgets the signal history but keeps the oversampling setting.
    pub fn clear(&mut self) {
        *self = Self { stages: self.stages, ..Self::new() };
    }

    pub fn set_stages(&mut self, stages: usize) {
        if stages != self.stages {
            *self = Self { stages, ..Self::new() };
        }
    }

    pub fn process<F: FnMut(f32) -> f32>(&mut self, x: f32, f: &mut F) -> f32 {
        self.run(0, x, f)
    }

    fn run<F: FnMut(f32) -> f32>(&mut self, stage: usize, x: f32, f: &mut F) -> f32 {
        if stage == self.stages {
            return f(x);
        }
        let (even, odd) = self.up[stage].push(x);
        let a = self.run(stage + 1, even, f);
        let b = self.run(stage + 1, odd, f);
        self.down[stage].push(a, b)
    }
}

/// One-pole filter coefficient for a cutoff in Hz.
pub fn one_pole(fs: f32, fc: f32) -> f32 {
    1.0 - (-2.0 * PI * fc / fs).exp()
}

/// Per-channel filters: a lowpass-derived highpass before the shaper, a DC blocker and a tilt
/// control after it.
#[derive(Clone, Default)]
pub struct Filters {
    pre_lp: f32,
    dc_x: f32,
    dc_y: f32,
    tone_lp: f32,
}

const TONE_HZ: f32 = 1000.0;
const TONE_DB: f32 = 9.0;

impl Filters {
    pub fn low_cut(&mut self, x: f32, a: f32) -> f32 {
        self.pre_lp += a * (x - self.pre_lp);
        x - self.pre_lp
    }

    pub fn dc_block(&mut self, x: f32, fs: f32) -> f32 {
        let r = 1.0 - 2.0 * PI * 10.0 / fs;
        self.dc_y = x - self.dc_x + r * self.dc_y;
        self.dc_x = x;
        self.dc_y
    }

    /// `tone` in -1..1: negative darkens (lows up, highs down), positive brightens.
    pub fn tilt(&mut self, x: f32, a: f32, tone: f32) -> f32 {
        self.tone_lp += a * (x - self.tone_lp);
        let g = 10f32.powf(TONE_DB * tone / 20.0);
        self.tone_lp / g + (x - self.tone_lp) * g
    }
}

pub fn tone_coeff(fs: f32) -> f32 {
    one_pole(fs, TONE_HZ)
}

/// Modulation sources. Both produce a value for the caller to scale: the LFO in -1..1, the
/// envelope in 0..1.
#[derive(Clone, Default)]
pub struct Lfo {
    phase: f32,
    smooth: f32,
}

impl Lfo {
    pub fn next(&mut self, fs: f32, rate: f32, shape: LfoShape) -> f32 {
        self.phase = (self.phase + rate / fs).fract();
        let raw = match shape {
            LfoShape::Sine => (self.phase * 2.0 * PI).sin(),
            LfoShape::Triangle => 4.0 * (self.phase - 0.5).abs() - 1.0,
            LfoShape::Square => {
                if self.phase < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
        };
        // Takes the click out of the square's edges.
        self.smooth += one_pole(fs, 300.0) * (raw - self.smooth);
        self.smooth
    }
}

/// Peak follower: fast attack, slow release.
#[derive(Clone, Default)]
pub struct Envelope {
    value: f32,
}

impl Envelope {
    pub fn next(&mut self, fs: f32, x: f32) -> f32 {
        let level = x.abs();
        let a = if level > self.value { one_pole_ms(fs, 5.0) } else { one_pole_ms(fs, 150.0) };
        self.value += a * (level - self.value);
        self.value.min(1.0)
    }
}

fn one_pole_ms(fs: f32, ms: f32) -> f32 {
    1.0 - (-1.0 / (ms * 0.001 * fs)).exp()
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
    /// Butterworth-Q lowpass or highpass; two in series make a Linkwitz-Riley filter.
    fn new(fs: f32, fc: f32, highpass: bool) -> Self {
        let w0 = 2.0 * PI * fc.min(fs * 0.45) / fs;
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * std::f32::consts::FRAC_1_SQRT_2);
        let a0 = 1.0 + alpha;
        let (b0, b1) = if highpass { ((1.0 + cos) / 2.0, -(1.0 + cos)) } else { ((1.0 - cos) / 2.0, 1.0 - cos) };
        Self { b0: b0 / a0, b1: b1 / a0, b2: b0 / a0, a1: -2.0 * cos / a0, a2: (1.0 - alpha) / a0, z1: 0.0, z2: 0.0 }
    }

    fn with_state(mut self, old: &Biquad) -> Self {
        self.z1 = old.z1;
        self.z2 = old.z2;
        self
    }

    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// Fourth-order Linkwitz-Riley lowpass or highpass.
#[derive(Clone, Copy, Default)]
struct Lr4([Biquad; 2]);

impl Lr4 {
    fn new(fs: f32, fc: f32, highpass: bool) -> Self {
        Self([Biquad::new(fs, fc, highpass); 2])
    }

    fn retune(&mut self, fs: f32, fc: f32, highpass: bool) {
        for b in &mut self.0 {
            *b = Biquad::new(fs, fc, highpass).with_state(b);
        }
    }

    fn process(&mut self, x: f32) -> f32 {
        let y = self.0[0].process(x);
        self.0[1].process(y)
    }
}

/// Splits a signal into low, mid and high bands that sum back to the input's magnitude. The low
/// band also passes through the upper crossover's filters, so all three share its phase response.
#[derive(Clone, Default)]
pub struct Crossover {
    low: Lr4,
    rest: Lr4,
    mid: Lr4,
    high: Lr4,
    low_fix_lp: Lr4,
    low_fix_hp: Lr4,
}

impl Crossover {
    pub fn new(fs: f32, f1: f32, f2: f32) -> Self {
        Self {
            low: Lr4::new(fs, f1, false),
            rest: Lr4::new(fs, f1, true),
            mid: Lr4::new(fs, f2, false),
            high: Lr4::new(fs, f2, true),
            low_fix_lp: Lr4::new(fs, f2, false),
            low_fix_hp: Lr4::new(fs, f2, true),
        }
    }

    pub fn retune(&mut self, fs: f32, f1: f32, f2: f32) {
        self.low.retune(fs, f1, false);
        self.rest.retune(fs, f1, true);
        self.mid.retune(fs, f2, false);
        self.high.retune(fs, f2, true);
        self.low_fix_lp.retune(fs, f2, false);
        self.low_fix_hp.retune(fs, f2, true);
    }

    pub fn split(&mut self, x: f32) -> [f32; 3] {
        let low = self.low.process(x);
        let rest = self.rest.process(x);
        [self.low_fix_lp.process(low) + self.low_fix_hp.process(low), self.mid.process(rest), self.high.process(rest)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODELS: [Model; 8] = [
        Model::Tube,
        Model::Tape,
        Model::Diode,
        Model::Fold,
        Model::Clip,
        Model::Crunch,
        Model::Rectify,
        Model::Crush,
    ];

    #[test]
    fn shapers_pass_small_signals_at_unity_and_stay_finite() {
        for m in MODELS {
            for bias in [-1.0, 0.0, 0.7] {
                assert!(shape(m, 0.0, bias).abs() < 1e-6);
                // Bias moves the operating point along the curve, so only check the slope without it.
                // Crush has a dead zone around zero, so it only has unity gain on average.
                let slope = (shape(m, 1e-3, 0.0) - shape(m, -1e-3, 0.0)) / 2e-3;
                assert!(m == Model::Crush || (slope - 1.0).abs() < 0.01, "model {} slope {slope}", m as usize);
                for i in -200..=200 {
                    assert!(shape(m, i as f32 * 0.1, bias).is_finite());
                }
            }
        }
    }

    #[test]
    fn oversampler_latency_and_gain() {
        for stages in 0..=MAX_STAGES {
            let mut os = Oversampler::new();
            os.set_stages(stages);
            let mut out = vec![];
            for n in 0..200 {
                out.push(os.process(if n == 0 { 1.0 } else { 0.0 }, &mut |v| v));
            }
            let peak = out.iter().enumerate().max_by(|a, b| a.1.abs().total_cmp(&b.1.abs())).unwrap().0;
            assert_eq!(peak, LATENCY[stages], "stages {stages}");

            let mut os = Oversampler::new();
            os.set_stages(stages);
            let mut last = 0.0;
            for _ in 0..400 {
                last = os.process(1.0, &mut |v| v);
            }
            assert!((last - 1.0).abs() < 1e-3, "dc gain {last}");
        }
    }

    #[test]
    fn oversampling_reduces_aliasing() {
        // A hard-clipped 9 kHz sine at 44.1 kHz: its 5th harmonic (45 kHz) folds to 900 Hz at 1x.
        let fs = 44100.0;
        let energy = |stages: usize| {
            let mut os = Oversampler::new();
            os.set_stages(stages);
            let n = 4096;
            let y: Vec<f32> = (0..n)
                .map(|i| os.process(3.0 * (2.0 * PI * 9000.0 * i as f32 / fs).sin(), &mut |v| shape(Model::Clip, v, 0.0)))
                .collect();
            let w = 2.0 * PI * 900.0 / fs;
            let (re, im) = y.iter().enumerate().skip(256).fold((0.0, 0.0), |(r, i), (k, v)| {
                (r + v * (w * k as f32).cos(), i + v * (w * k as f32).sin())
            });
            (re * re + im * im).sqrt() / n as f32
        };
        assert!(energy(2) < energy(0) * 0.3, "{} vs {}", energy(2), energy(0));
    }

    #[test]
    fn crossover_bands_sum_to_a_flat_response() {
        let fs = 48000.0;
        for freq in [60.0, 200.0, 800.0, 2500.0, 6000.0, 15000.0] {
            let mut xo = Crossover::new(fs, 300.0, 3000.0);
            let (mut input, mut output) = (0.0, 0.0);
            for i in 0..20000 {
                let x = (2.0 * PI * freq * i as f32 / fs).sin();
                let bands = xo.split(x);
                if i > 10000 {
                    input += x * x;
                    let y: f32 = bands.iter().sum();
                    output += y * y;
                }
            }
            let db = 10.0 * (output / input).log10();
            assert!(db.abs() < 0.3, "{freq} Hz: {db} dB");
        }
    }

    #[test]
    fn crossover_separates_bands() {
        let fs = 48000.0;
        let level = |freq: f32| {
            let mut xo = Crossover::new(fs, 300.0, 3000.0);
            let mut energy = [0.0f32; 3];
            for i in 0..20000 {
                let b = xo.split((2.0 * PI * freq * i as f32 / fs).sin());
                if i > 10000 {
                    for k in 0..3 {
                        energy[k] += b[k] * b[k];
                    }
                }
            }
            energy
        };
        let (lo, mid, hi) = (level(60.0), level(1000.0), level(12000.0));
        assert!(lo[0] > 100.0 * (lo[1] + lo[2]));
        assert!(mid[1] > 100.0 * (mid[0] + mid[2]));
        assert!(hi[2] > 100.0 * (hi[0] + hi[1]));
    }
}
