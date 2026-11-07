//! Biquads in f64 (the corners here are tens of hertz, where f32 state runs out of precision).

use std::f64::consts::{FRAC_1_SQRT_2, TAU};

/// One-pole smoothing coefficient for a time constant: `y += (x - y) * coef`.
pub fn coef_ms(ms: f32, fs: f32) -> f32 {
    1.0 - (-1.0 / (ms * 0.001 * fs)).exp()
}

pub fn lin_to_db(x: f32) -> f32 {
    20.0 * x.max(1e-9).log10()
}

/// Biquad coefficients, kept apart from the state so a design can be replaced without a click.
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
    (TAU * (freq as f64).clamp(1.0, fs as f64 * 0.45) / fs as f64).sin_cos()
}

impl Coeffs {
    fn rbj(b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) -> Self {
        Self { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0 }
    }

    /// Butterworth-Q low-pass: two in a `Cascade` make a 4th-order Linkwitz-Riley.
    pub fn lowpass(fs: f32, freq: f32) -> Self {
        let (s, c) = trig(fs, freq);
        let alpha = s / (2.0 * FRAC_1_SQRT_2);
        Self::rbj((1.0 - c) * 0.5, 1.0 - c, (1.0 - c) * 0.5, 1.0 + alpha, -2.0 * c, 1.0 - alpha)
    }

    pub fn highpass(fs: f32, freq: f32) -> Self {
        let (s, c) = trig(fs, freq);
        let alpha = s / (2.0 * FRAC_1_SQRT_2);
        Self::rbj((1.0 + c) * 0.5, -(1.0 + c), (1.0 + c) * 0.5, 1.0 + alpha, -2.0 * c, 1.0 - alpha)
    }

    /// Band-pass with 0 dB gain at the centre.
    pub fn bandpass(fs: f32, freq: f32, q: f64) -> Self {
        let (s, c) = trig(fs, freq);
        let alpha = s / (2.0 * q);
        Self::rbj(alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * c, 1.0 - alpha)
    }
}

#[derive(Clone, Copy, Default)]
pub struct Biquad {
    z1: f64,
    z2: f64,
}

impl Biquad {
    #[inline]
    pub fn process(&mut self, c: &Coeffs, x: f32) -> f32 {
        let x = x as f64;
        let y = c.b0 * x + self.z1;
        self.z1 = c.b1 * x - c.a1 * y + self.z2;
        self.z2 = c.b2 * x - c.a2 * y;
        y as f32
    }
}

/// Two sections of one design in series.
#[derive(Clone, Copy, Default)]
pub struct Cascade([Biquad; 2]);

impl Cascade {
    #[inline]
    pub fn process(&mut self, c: &Coeffs, x: f32) -> f32 {
        let y = self.0[0].process(c, x);
        self.0[1].process(c, y)
    }
}
