//! Allocation-free radix-2 FFT and window helpers for the spectral plugins.
//! Build an [`Fft`] in `new`/`configure` (it allocates), then call it on the audio thread.

use std::f32::consts::PI;

pub struct Fft {
    n: usize,
    tw_re: Vec<f32>,
    tw_im: Vec<f32>,
}

impl Fft {
    /// `n` must be a power of two.
    pub fn new(n: usize) -> Self {
        assert!(n.is_power_of_two() && n >= 2);
        Self {
            n,
            tw_re: (0..n / 2).map(|k| (2.0 * PI * k as f32 / n as f32).cos()).collect(),
            tw_im: (0..n / 2).map(|k| -(2.0 * PI * k as f32 / n as f32).sin()).collect(),
        }
    }

    pub fn len(&self) -> usize {
        self.n
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    /// In-place forward transform (e^-i).
    pub fn forward(&self, re: &mut [f32], im: &mut [f32]) {
        self.run(re, im, false);
    }

    /// In-place inverse transform, scaled by 1/n so `inverse(forward(x)) == x`.
    pub fn inverse(&self, re: &mut [f32], im: &mut [f32]) {
        // Conjugate trick: ifft(x) = conj(fft(conj(x))) / n.
        for v in im.iter_mut() {
            *v = -*v;
        }
        self.run(re, im, true);
        let s = 1.0 / self.n as f32;
        for (r, i) in re.iter_mut().zip(im.iter_mut()) {
            *r *= s;
            *i = -*i * s;
        }
    }

    fn run(&self, re: &mut [f32], im: &mut [f32], _inverse: bool) {
        let n = self.n;
        debug_assert!(re.len() == n && im.len() == n);
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
                    let (wr, wi) = (self.tw_re[k * step], self.tw_im[k * step]);
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
}

/// Periodic sqrt-Hann of length `n`. Applied on analysis and synthesis it gives Hann, which sums
/// to 2 at 4x overlap (divide the overlap-added output by 2).
pub fn sqrt_hann(n: usize) -> Vec<f32> {
    (0..n).map(|i| (0.5 - 0.5 * (2.0 * PI * i as f32 / n as f32).cos()).sqrt()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let fft = Fft::new(256);
        let x: Vec<f32> = (0..256).map(|i| (i as f32 * 0.37).sin() + 0.2 * (i as f32 * 1.9).cos()).collect();
        let (mut re, mut im) = (x.clone(), vec![0.0; 256]);
        fft.forward(&mut re, &mut im);
        fft.inverse(&mut re, &mut im);
        for i in 0..256 {
            assert!((re[i] - x[i]).abs() < 1e-4 && im[i].abs() < 1e-4);
        }
    }

    #[test]
    fn sine_lands_in_its_bin() {
        let fft = Fft::new(512);
        let (mut re, mut im): (Vec<f32>, Vec<f32>) =
            ((0..512).map(|i| (2.0 * PI * 20.0 * i as f32 / 512.0).sin()).collect(), vec![0.0; 512]);
        fft.forward(&mut re, &mut im);
        let mag = |k: usize| (re[k] * re[k] + im[k] * im[k]).sqrt();
        assert!((mag(20) - 256.0).abs() < 0.1 && mag(21) < 0.1);
    }
}
