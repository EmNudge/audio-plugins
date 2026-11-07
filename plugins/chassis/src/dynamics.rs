//! Dynamics and EQ: the gate, the body EQ, the chest compressor, the glue compressor, the surgical
//! notches and the true-peak limiter.

use crate::filters::{coef_ms, db_to_lin, lin_to_db, Biquad, Coeffs, MAX_CHANNELS};

/// Gate threshold at or below which the gate is out of circuit.
pub const GATE_OFF_DB: f32 = -79.0;

/// A gate whose gain is meant to be applied late: the caller delays it by the latency of the
/// path it gates, so the gate opens before the first word arrives.
pub struct Gate {
    env: f32,
    open: bool,
    hold: u32,
    gain: f32,
    release: f32,
    gain_up: f32,
    gain_down: f32,
    hold_samples: u32,
}

/// How far a closed gate pulls the signal down.
const GATE_FLOOR: f32 = 0.003;

impl Gate {
    pub fn new(fs: f32) -> Self {
        Self {
            env: 0.0,
            open: true,
            hold: 0,
            gain: 1.0,
            release: coef_ms(20.0, fs),
            gain_up: coef_ms(1.5, fs),
            gain_down: coef_ms(25.0, fs),
            hold_samples: (0.03 * fs) as u32,
        }
    }

    pub fn reset(&mut self) {
        self.env = 0.0;
        self.open = true;
        self.hold = 0;
        self.gain = 1.0;
    }

    /// `level` is the peak magnitude of the input across channels.
    pub fn process(&mut self, level: f32, threshold_db: f32) -> f32 {
        let target = if threshold_db <= GATE_OFF_DB {
            self.open = true;
            1.0
        } else {
            self.env = if level > self.env { level } else { self.env + (level - self.env) * self.release };
            let db = lin_to_db(self.env);
            if db > threshold_db {
                self.open = true;
                self.hold = self.hold_samples;
            } else if db < threshold_db - 4.0 {
                if self.hold > 0 {
                    self.hold -= 1;
                } else {
                    self.open = false;
                }
            }
            if self.open {
                1.0
            } else {
                GATE_FLOOR
            }
        };
        let coef = if target > self.gain { self.gain_up } else { self.gain_down };
        self.gain += (target - self.gain) * coef;
        self.gain
    }
}

/// Control-rate period of the EQ updates, in samples.
pub const CONTROL: u32 = 32;
const BOX_HZ: f32 = 400.0;
const BOX_MAX_DB: f32 = 9.0;
const WEIGHT_HZ: f32 = 100.0;
const WEIGHT_MAX_DB: f32 = 7.0;

/// Pre-processing EQ: a bell that cuts room boxiness at 300 to 500 Hz, and only while there is
/// something there to cut, and a low shelf at 100 Hz for chest weight.
pub struct BodyEq {
    fs: f32,
    detect: Coeffs,
    detect_s: Biquad,
    env: f32,
    attack: f32,
    release: f32,
    cut_db: f32,
    weight_db: f32,
    bell: Coeffs,
    shelf: Coeffs,
    bell_s: [Biquad; MAX_CHANNELS],
    shelf_s: [Biquad; MAX_CHANNELS],
}

impl BodyEq {
    pub fn new(fs: f32) -> Self {
        Self {
            fs,
            detect: Coeffs::bandpass(fs, BOX_HZ, 1.2),
            detect_s: Biquad::default(),
            env: 0.0,
            attack: coef_ms(6.0, fs),
            release: coef_ms(90.0, fs),
            cut_db: 0.0,
            weight_db: 0.0,
            bell: Coeffs::default(),
            shelf: Coeffs::default(),
            bell_s: [Biquad::default(); MAX_CHANNELS],
            shelf_s: [Biquad::default(); MAX_CHANNELS],
        }
    }

    pub fn reset(&mut self) {
        self.detect_s = Biquad::default();
        self.env = 0.0;
        self.cut_db = 0.0;
        self.bell_s = [Biquad::default(); MAX_CHANNELS];
        self.shelf_s = [Biquad::default(); MAX_CHANNELS];
    }

    /// Feed the sidechain; call once per sample with the mono signal.
    #[inline]
    pub fn detect(&mut self, mono: f32) {
        let band = self.detect_s.process(&self.detect, mono).abs();
        let coef = if band > self.env { self.attack } else { self.release };
        self.env += (band - self.env) * coef;
    }

    /// Call every `CONTROL` samples.
    pub fn update(&mut self, boxy: f32, weight: f32) {
        // Full cut once the band sits above about -26 dB, none below -46.
        let engaged = ((lin_to_db(self.env) + 46.0) / 20.0).clamp(0.0, 1.0);
        let wanted = boxy * BOX_MAX_DB * engaged;
        self.cut_db += (wanted - self.cut_db) * 0.15;
        let shelf_db = weight * WEIGHT_MAX_DB;
        if (shelf_db - self.weight_db).abs() > 0.01 {
            self.weight_db = shelf_db;
            self.shelf = Coeffs::low_shelf(self.fs, WEIGHT_HZ, shelf_db);
        }
        self.bell = Coeffs::peaking(self.fs, BOX_HZ, 1.0, -self.cut_db);
    }

    #[inline]
    pub fn process(&mut self, ch: usize, x: f32) -> f32 {
        let y = self.bell_s[ch].process(&self.bell, x);
        self.shelf_s[ch].process(&self.shelf, y)
    }
}

/// A downward compressor's static curve: the gain reduction in dB (positive) at `level_db`.
pub fn reduction_db(level_db: f32, threshold_db: f32, ratio: f32, knee_db: f32) -> f32 {
    let over = level_db - threshold_db;
    let slope = 1.0 - 1.0 / ratio;
    if 2.0 * over < -knee_db {
        0.0
    } else if 2.0 * over.abs() <= knee_db {
        slope * (over + knee_db * 0.5).powi(2) / (2.0 * knee_db)
    } else {
        slope * over
    }
}

/// The chest compressor: 8:1, fast attack, fast release, with the threshold set by one knob and
/// part of the gain it takes away given back.
pub struct Squash {
    env: f32,
    gr: f32,
    detect_attack: f32,
    detect_release: f32,
    attack: f32,
    release: f32,
}

const SQUASH_RATIO: f32 = 8.0;
const SQUASH_KNEE_DB: f32 = 6.0;
const SQUASH_MAX_DB: f32 = 36.0;

impl Squash {
    pub fn new(fs: f32) -> Self {
        Self {
            env: 0.0,
            gr: 0.0,
            detect_attack: coef_ms(0.4, fs),
            detect_release: coef_ms(30.0, fs),
            attack: coef_ms(1.0, fs),
            release: coef_ms(55.0, fs),
        }
    }

    pub fn reset(&mut self) {
        self.env = 0.0;
        self.gr = 0.0;
    }

    /// Gain reduction in dB (positive) right now.
    pub fn reduction(&self) -> f32 {
        self.gr
    }

    pub fn process(&mut self, x: [f32; MAX_CHANNELS], channels: usize, amount: f32) -> [f32; MAX_CHANNELS] {
        if amount < 1e-3 {
            self.gr += (0.0 - self.gr) * self.release;
            self.env = 0.0;
            return x;
        }
        let level = x[..channels].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        let coef = if level > self.env { self.detect_attack } else { self.detect_release };
        self.env += (level - self.env) * coef;
        let threshold = -amount * SQUASH_MAX_DB;
        let target = reduction_db(lin_to_db(self.env), threshold, SQUASH_RATIO, SQUASH_KNEE_DB);
        let coef = if target > self.gr { self.attack } else { self.release };
        self.gr += (target - self.gr) * coef;
        let makeup = 0.4 * (1.0 - 1.0 / SQUASH_RATIO) * -threshold;
        let gain = db_to_lin(makeup - self.gr);
        [x[0] * gain, x[1] * gain]
    }
}

/// The master glue compressor: 4:1, a medium attack, and a release that lengthens the longer the
/// gain has been held down.
pub struct Glue {
    env: f32,
    gr: f32,
    average: f32,
    detect_attack: f32,
    detect_release: f32,
    attack: f32,
    average_coef: f32,
    fs: f32,
}

const GLUE_RATIO: f32 = 4.0;
const GLUE_KNEE_DB: f32 = 8.0;

impl Glue {
    pub fn new(fs: f32) -> Self {
        Self {
            env: 0.0,
            gr: 0.0,
            average: 0.0,
            detect_attack: coef_ms(3.0, fs),
            detect_release: coef_ms(60.0, fs),
            attack: coef_ms(18.0, fs),
            average_coef: coef_ms(500.0, fs),
            fs,
        }
    }

    pub fn reset(&mut self) {
        self.env = 0.0;
        self.gr = 0.0;
        self.average = 0.0;
    }

    pub fn reduction(&self) -> f32 {
        self.gr
    }

    pub fn process(&mut self, x: [f32; MAX_CHANNELS], channels: usize, amount: f32) -> [f32; MAX_CHANNELS] {
        let level = x[..channels].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        let coef = if level > self.env { self.detect_attack } else { self.detect_release };
        self.env += (level - self.env) * coef;
        let target = if amount < 1e-3 {
            0.0
        } else {
            reduction_db(lin_to_db(self.env), -2.0 - 14.0 * amount, GLUE_RATIO, GLUE_KNEE_DB)
        };
        self.average += (self.gr - self.average) * self.average_coef;
        let coef = if target > self.gr {
            self.attack
        } else {
            // 70 ms after a brief squeeze, up to 600 ms after a sustained one.
            coef_ms(70.0 + 530.0 * (self.average / 5.0).clamp(0.0, 1.0), self.fs)
        };
        self.gr += (target - self.gr) * coef;
        let gain = db_to_lin(-self.gr);
        [x[0] * gain, x[1] * gain]
    }
}

/// Three narrow cuts at the frequencies where a metallic voice turns harsh.
pub struct Notches {
    fs: f32,
    depth_db: f32,
    coeffs: [Coeffs; 3],
    state: [[Biquad; 3]; MAX_CHANNELS],
}

pub const NOTCH_HZ: [f32; 3] = [2500.0, 4000.0, 6000.0];

impl Notches {
    pub fn new(fs: f32) -> Self {
        Self { fs, depth_db: 0.0, coeffs: [Coeffs::default(); 3], state: [[Biquad::default(); 3]; MAX_CHANNELS] }
    }

    pub fn reset(&mut self) {
        self.state = [[Biquad::default(); 3]; MAX_CHANNELS];
    }

    pub fn set(&mut self, amount: f32) {
        let depth = amount * 10.0;
        if (depth - self.depth_db).abs() > 0.01 {
            self.depth_db = depth;
            for (c, hz) in self.coeffs.iter_mut().zip(NOTCH_HZ) {
                *c = Coeffs::peaking(self.fs, hz, 4.0, -depth);
            }
        }
    }

    #[inline]
    pub fn process(&mut self, ch: usize, x: f32) -> f32 {
        let mut y = x;
        for (s, c) in self.state[ch].iter_mut().zip(&self.coeffs) {
            y = s.process(c, y);
        }
        y
    }
}

const LIMITER_MS: f32 = 1.5;
/// Taps of the interpolation filter behind the true-peak detector, and how far into its history
/// (counted from the oldest) the interval it measures sits.
const TP_TAPS: usize = 12;
const TP_CENTRE: usize = TP_TAPS / 2 - 1;
/// An interval's peak is known this many samples after the audio sample it follows.
const TP_LAG: usize = TP_TAPS / 2;

/// Windowed-sinc kernels that read the signal a quarter, a half and three quarters of a sample
/// past the centre tap: with the samples themselves, a 4x oversampled peak detector.
fn true_peak_kernels() -> [[f32; TP_TAPS]; 3] {
    std::array::from_fn(|p| {
        let frac = (p + 1) as f32 * 0.25;
        let mut k: [f32; TP_TAPS] = std::array::from_fn(|m| {
            let x = m as f32 - TP_CENTRE as f32 - frac;
            let sinc = if x.abs() < 1e-6 { 1.0 } else { (std::f32::consts::PI * x).sin() / (std::f32::consts::PI * x) };
            let r = x / (TP_TAPS as f32 * 0.5);
            let window = 0.42 + 0.5 * (std::f32::consts::PI * r).cos() + 0.08 * (2.0 * std::f32::consts::PI * r).cos();
            sinc * window
        });
        let sum: f32 = k.iter().sum();
        k.iter_mut().for_each(|v| *v /= sum);
        k
    })
}

/// A lookahead brickwall limiter that watches for inter-sample peaks.
///
/// Peaks are measured the way a true-peak meter does, on a 4x oversampled signal. The gain they
/// ask for is held for the length of the window (a running minimum), then smoothed by a running
/// mean of the same length, which gives a gain that is already down by the time the peak arrives
/// and never overshoots; the release follows slowly.
pub struct Limiter {
    window: usize,
    delay: usize,
    audio: [Vec<f32>; MAX_CHANNELS],
    audio_pos: usize,
    kernels: [[f32; TP_TAPS]; 3],
    history: [[f32; TP_TAPS]; MAX_CHANNELS],
    wanted: Vec<f32>,
    wanted_pos: usize,
    held: Vec<f32>,
    held_pos: usize,
    sum: f64,
    gain: f32,
    release: f32,
}

impl Limiter {
    pub fn new(fs: f32) -> Self {
        let window = ((LIMITER_MS * 0.001 * fs).ceil() as usize).max(4);
        // The minimum reaches back far enough to cover the peaks that are found late, and the
        // audio waits until the whole mean sits on them.
        let delay = window + TP_LAG - 1;
        Self {
            window,
            delay,
            audio: [vec![0.0; delay + 1], vec![0.0; delay + 1]],
            audio_pos: 0,
            kernels: true_peak_kernels(),
            history: [[0.0; TP_TAPS]; MAX_CHANNELS],
            wanted: vec![1.0; window + TP_LAG],
            wanted_pos: 0,
            held: vec![1.0; window],
            held_pos: 0,
            sum: window as f64,
            gain: 1.0,
            release: coef_ms(90.0, fs),
        }
    }

    pub fn reset(&mut self) {
        for a in &mut self.audio {
            a.fill(0.0);
        }
        self.history = [[0.0; TP_TAPS]; MAX_CHANNELS];
        self.wanted.fill(1.0);
        self.held.fill(1.0);
        self.sum = self.window as f64;
        self.gain = 1.0;
    }

    pub fn latency(&self) -> usize {
        self.delay
    }

    /// Gain reduction in dB (positive) right now.
    pub fn reduction(&self) -> f32 {
        (-lin_to_db(self.gain)).max(0.0)
    }

    pub fn process(&mut self, x: [f32; MAX_CHANNELS], channels: usize, ceiling: f32) -> [f32; MAX_CHANNELS] {
        let mut peak = 0.0f32;
        for ch in 0..channels {
            let h = &mut self.history[ch];
            h.rotate_left(1);
            h[TP_TAPS - 1] = x[ch];
            peak = peak.max(x[ch].abs());
            for kernel in &self.kernels {
                let v: f32 = kernel.iter().zip(h.iter()).map(|(k, s)| k * s).sum();
                peak = peak.max(v.abs());
            }
        }
        let wanted = if peak > ceiling { ceiling / peak } else { 1.0 };
        self.wanted[self.wanted_pos] = wanted;
        self.wanted_pos = (self.wanted_pos + 1) % self.wanted.len();
        let min = self.wanted.iter().fold(1.0f32, |m, &v| m.min(v));

        self.sum += (min - self.held[self.held_pos]) as f64;
        self.held[self.held_pos] = min;
        self.held_pos = (self.held_pos + 1) % self.window;
        let smooth = (self.sum / self.window as f64) as f32;
        self.gain = if smooth < self.gain { smooth } else { self.gain + (smooth - self.gain) * self.release };

        let len = self.audio[0].len();
        let mut out = [0.0; MAX_CHANNELS];
        for ch in 0..channels {
            self.audio[ch][self.audio_pos] = x[ch];
            let delayed = self.audio[ch][(self.audio_pos + 1) % len];
            out[ch] = (delayed * self.gain).clamp(-ceiling, ceiling);
        }
        self.audio_pos = (self.audio_pos + 1) % len;
        out
    }
}
