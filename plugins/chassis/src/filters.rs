//! Filters and small helpers shared by the rest of the DSP.

use std::f64::consts::{FRAC_1_SQRT_2, TAU};

pub const MAX_CHANNELS: usize = 2;

pub fn db_to_lin(db: f32) -> f32 {
    10f32.powf(db * 0.05)
}

pub fn lin_to_db(x: f32) -> f32 {
    20.0 * x.max(1e-9).log10()
}

/// One-pole smoothing coefficient for a time constant: `y += (x - y) * coef`.
pub fn coef_ms(ms: f32, fs: f32) -> f32 {
    1.0 - (-1.0 / (ms * 0.001 * fs)).exp()
}

/// Biquad coefficients, kept apart from the state so one design can serve every channel and be
/// replaced without a click.
#[derive(Clone, Copy)]
pub struct Coeffs {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

impl Default for Coeffs {
    fn default() -> Self {
        Self { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0 }
    }
}

fn trig(fs: f32, freq: f32) -> (f64, f64) {
    let w = TAU * (freq as f64).clamp(1.0, fs as f64 * 0.45) / fs as f64;
    w.sin_cos()
}

impl Coeffs {
    fn rbj(b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) -> Self {
        Self { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0 }
    }

    pub fn lowpass(fs: f32, freq: f32, q: f64) -> Self {
        let (s, c) = trig(fs, freq);
        let alpha = s / (2.0 * q);
        Self::rbj((1.0 - c) * 0.5, 1.0 - c, (1.0 - c) * 0.5, 1.0 + alpha, -2.0 * c, 1.0 - alpha)
    }

    pub fn highpass(fs: f32, freq: f32, q: f64) -> Self {
        let (s, c) = trig(fs, freq);
        let alpha = s / (2.0 * q);
        Self::rbj((1.0 + c) * 0.5, -(1.0 + c), (1.0 + c) * 0.5, 1.0 + alpha, -2.0 * c, 1.0 - alpha)
    }

    /// Band-pass with 0 dB gain at the centre.
    pub fn bandpass(fs: f32, freq: f32, q: f64) -> Self {
        let (s, c) = trig(fs, freq);
        let alpha = s / (2.0 * q);
        Self::rbj(alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * c, 1.0 - alpha)
    }

    pub fn peaking(fs: f32, freq: f32, q: f64, gain_db: f32) -> Self {
        let (s, c) = trig(fs, freq);
        let a = 10f64.powf(gain_db as f64 / 40.0);
        let alpha = s / (2.0 * q);
        Self::rbj(1.0 + alpha * a, -2.0 * c, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * c, 1.0 - alpha / a)
    }

    pub fn low_shelf(fs: f32, freq: f32, gain_db: f32) -> Self {
        let (s, c) = trig(fs, freq);
        let a = 10f64.powf(gain_db as f64 / 40.0);
        let alpha = s * 0.5 * ((a + 1.0 / a) * (1.0 / 0.9 - 1.0) + 2.0).sqrt();
        let k = 2.0 * a.sqrt() * alpha;
        Self::rbj(
            a * ((a + 1.0) - (a - 1.0) * c + k),
            2.0 * a * ((a - 1.0) - (a + 1.0) * c),
            a * ((a + 1.0) - (a - 1.0) * c - k),
            (a + 1.0) + (a - 1.0) * c + k,
            -2.0 * ((a - 1.0) + (a + 1.0) * c),
            (a + 1.0) + (a - 1.0) * c - k,
        )
    }
}

/// Biquad state (transposed direct form II).
#[derive(Clone, Copy, Default)]
pub struct Biquad {
    z1: f64,
    z2: f64,
}

impl Biquad {
    #[inline]
    pub fn run(&mut self, c: &Coeffs, x: f64) -> f64 {
        let y = c.b0 * x + self.z1;
        self.z1 = c.b1 * x - c.a1 * y + self.z2;
        self.z2 = c.b2 * x - c.a2 * y;
        y
    }

    #[inline]
    pub fn process(&mut self, c: &Coeffs, x: f32) -> f32 {
        self.run(c, x as f64) as f32
    }
}

/// Two sections with one design in series: with Butterworth coefficients, a Linkwitz-Riley
/// 4th-order filter.
#[derive(Clone, Copy, Default)]
pub struct Cascade([Biquad; 2]);

impl Cascade {
    #[inline]
    pub fn run(&mut self, c: &Coeffs, x: f64) -> f64 {
        let y = self.0[0].run(c, x);
        self.0[1].run(c, y)
    }

    #[inline]
    pub fn process(&mut self, c: &Coeffs, x: f32) -> f32 {
        self.run(c, x as f64) as f32
    }
}

/// Splits a signal into low, mid and high bands with 4th-order Linkwitz-Riley crossovers (24 dB per
/// octave). The low band also passes through the upper crossover's low- and high-pass, an all-pass
/// overall, so all three bands share one phase response and sum back flat.
pub struct Crossover {
    low_hz: f32,
    high_hz: f32,
    lp1: Coeffs,
    hp1: Coeffs,
    lp2: Coeffs,
    hp2: Coeffs,
    state: [[Cascade; 6]; MAX_CHANNELS],
}

impl Crossover {
    pub fn new() -> Self {
        Self {
            low_hz: 0.0,
            high_hz: 0.0,
            lp1: Coeffs::default(),
            hp1: Coeffs::default(),
            lp2: Coeffs::default(),
            hp2: Coeffs::default(),
            state: [[Cascade::default(); 6]; MAX_CHANNELS],
        }
    }

    pub fn clear(&mut self) {
        self.state = [[Cascade::default(); 6]; MAX_CHANNELS];
    }

    pub fn set(&mut self, low_hz: f32, high_hz: f32, fs: f32) {
        if (low_hz - self.low_hz).abs() > low_hz * 1e-4 {
            self.low_hz = low_hz;
            self.lp1 = Coeffs::lowpass(fs, low_hz, FRAC_1_SQRT_2);
            self.hp1 = Coeffs::highpass(fs, low_hz, FRAC_1_SQRT_2);
        }
        if (high_hz - self.high_hz).abs() > high_hz * 1e-4 {
            self.high_hz = high_hz;
            self.lp2 = Coeffs::lowpass(fs, high_hz, FRAC_1_SQRT_2);
            self.hp2 = Coeffs::highpass(fs, high_hz, FRAC_1_SQRT_2);
        }
    }

    pub fn split(&mut self, ch: usize, x: f32) -> [f32; 3] {
        let s = &mut self.state[ch];
        let x = x as f64;
        let low = s[0].run(&self.lp1, x);
        let rest = s[1].run(&self.hp1, x);
        let low = s[4].run(&self.lp2, low) + s[5].run(&self.hp2, low);
        let mid = s[2].run(&self.lp2, rest);
        let high = s[3].run(&self.hp2, rest);
        [low as f32, mid as f32, high as f32]
    }
}
