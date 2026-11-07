//! Low-end exciter DSP: the counterpart of Aurora Air.
//!
//! Aurora Air grows content above the top of a signal by saturating it. This grows content around
//! the bottom of one, with two engines fed by the mono sum of the lows (30 to 130 Hz):
//!
//! - **Body** makes harmonics of the bass, so it is heard on speakers that cannot reproduce it.
//!   The band is normalised by its own envelope and fed through Chebyshev polynomials: `T2(sin t)`
//!   is `cos 2t` and `T3(sin t)` is `-sin 3t`, so a note gains its second and third harmonics and
//!   nothing else, at the same proportion whether it is loud or soft. Where several notes sound at
//!   once, they intermodulate a little, as any waveshaper does; a high-pass keeps the difference
//!   tones out. Body is polyphonic and works on whole mixes.
//! - **Sub** makes an octave below. A Schmitt trigger finds the rising zero crossings of the low
//!   band and a flip-flop divides them by two. That only means something while there is one note,
//!   so the engine also watches the interval between crossings: it fades in once three cycles in a
//!   row agree (within 7%, between 36 and 120 Hz) and out as soon as they stop, which is what
//!   happens under a chord or a dense low end. The square wave is filtered to a near sine and
//!   scaled by the band's envelope, then high-passed at the Floor so nothing is spent on
//!   frequencies below it.
//!
//! Both engines are minimum-phase filters and memoryless shapers, so the plugin adds no latency and
//! the dry signal is not touched: the added content is summed on top, to both channels alike.

use nih_plug::prelude::AtomicF32;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::filters::{coef_ms, lin_to_db, Biquad, Cascade, Coeffs};

pub const BANDS: usize = 36;
/// The range of the spectrum display and its analysis bank, log spaced across `BANDS`.
pub const F_MIN: f32 = 20.0;
pub const F_MAX: f32 = 400.0;

pub const MIN_FLOOR_HZ: f32 = 20.0;
pub const MAX_FLOOR_HZ: f32 = 60.0;

/// The low band both engines listen to.
const BAND_HP_HZ: f32 = 30.0;
const BAND_LP_HZ: f32 = 130.0;
/// What body keeps: nothing under this (so no fundamentals and no difference tones), nothing over that.
const BODY_HP_HZ: f32 = 55.0;
const BODY_LP_HZ: f32 = 700.0;
/// Levels of the second and third harmonics at full Body, relative to the note.
const BODY_H2: f32 = 0.5;
const BODY_H3: f32 = 0.35;
/// Below this band level (dB) body fades out, so it does not make harmonics of rumble and hum.
const BODY_GATE_DB: (f32, f32) = (-66.0, -50.0);

/// The fundamentals sub will follow, in Hz, and how closely consecutive cycles must agree.
const SUB_RANGE_HZ: (f32, f32) = (36.0, 120.0);
const SUB_TOLERANCE: f32 = 0.07;
const SUB_CYCLES: u32 = 3;
const SUB_LP_HZ: f32 = 80.0;

/// What the audio thread publishes for the editor.
pub struct Meter {
    /// Level of each analysis band, linear: the input, and what the plugin adds.
    pub dry: [AtomicF32; BANDS],
    pub wet: [AtomicF32; BANDS],
    /// The note the sub engine is following in Hz, 0 while it is not locked.
    pub pitch_hz: AtomicF32,
    /// How firmly it is locked, 0 to 1.
    pub lock: AtomicF32,
}

impl Meter {
    fn new() -> Self {
        Self {
            dry: std::array::from_fn(|_| AtomicF32::new(0.0)),
            wet: std::array::from_fn(|_| AtomicF32::new(0.0)),
            pitch_hz: AtomicF32::new(0.0),
            lock: AtomicF32::new(0.0),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub body: f32,
    pub sub: f32,
    pub floor_hz: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self { body: 0.5, sub: 0.35, floor_hz: 28.0 }
    }
}

fn smoothstep(lo: f32, hi: f32, x: f32) -> f32 {
    let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Both engines, for one mono signal.
pub struct Bass {
    fs: f32,
    // The low band: a high-pass against rumble, then a 4th-order low-pass.
    hp_c: Coeffs,
    hp_s: Biquad,
    lp_c: Coeffs,
    lp_s: Cascade,
    // Body.
    env: f32,
    env_attack: f32,
    env_release: f32,
    body_hp_c: Coeffs,
    body_hp_s: Cascade,
    body_lp_c: Coeffs,
    body_lp_s: Biquad,
    // Sub.
    peak: f32,
    peak_release: f32,
    armed: bool,
    flip: f32,
    since: u32,
    last_period: f32,
    cycles: u32,
    hz: f32,
    lock: f32,
    lock_up: f32,
    lock_down: f32,
    post_c: Coeffs,
    post_s: Cascade,
    sub_env: f32,
    sub_env_attack: f32,
    sub_env_release: f32,
    floor_hz: f32,
    floor_c: Coeffs,
    floor_s: Cascade,
}

impl Bass {
    pub fn new(fs: f32) -> Self {
        Self {
            fs,
            hp_c: Coeffs::highpass(fs, BAND_HP_HZ),
            hp_s: Biquad::default(),
            lp_c: Coeffs::lowpass(fs, BAND_LP_HZ),
            lp_s: Cascade::default(),
            env: 0.0,
            env_attack: coef_ms(2.0, fs),
            env_release: coef_ms(80.0, fs),
            body_hp_c: Coeffs::highpass(fs, BODY_HP_HZ),
            body_hp_s: Cascade::default(),
            body_lp_c: Coeffs::lowpass(fs, BODY_LP_HZ),
            body_lp_s: Biquad::default(),
            peak: 0.0,
            peak_release: coef_ms(80.0, fs),
            armed: false,
            flip: 1.0,
            since: 0,
            last_period: 0.0,
            cycles: 0,
            hz: 0.0,
            lock: 0.0,
            lock_up: coef_ms(25.0, fs),
            lock_down: coef_ms(12.0, fs),
            post_c: Coeffs::lowpass(fs, SUB_LP_HZ),
            post_s: Cascade::default(),
            sub_env: 0.0,
            sub_env_attack: coef_ms(4.0, fs),
            sub_env_release: coef_ms(45.0, fs),
            floor_hz: 0.0,
            floor_c: Coeffs::default(),
            floor_s: Cascade::default(),
        }
    }

    pub fn reset(&mut self) {
        let fresh = Self::new(self.fs);
        *self = Self { floor_hz: 0.0, ..fresh };
    }

    /// The sub engine's note in Hz while it is locked, else 0.
    pub fn pitch_hz(&self) -> f32 {
        if self.lock > 0.5 {
            self.hz
        } else {
            0.0
        }
    }

    pub fn lock(&self) -> f32 {
        self.lock
    }

    fn set_floor(&mut self, hz: f32) {
        if (hz - self.floor_hz).abs() > self.floor_hz * 5e-3 {
            self.floor_hz = hz;
            self.floor_c = Coeffs::highpass(self.fs, hz);
        }
    }

    /// One sample of the mono low end in; the body and sub contributions out, already scaled by
    /// their amounts (0 to 1).
    pub fn process(&mut self, x: f32, body: f32, sub: f32, floor_hz: f32) -> (f32, f32) {
        self.set_floor(floor_hz.clamp(MIN_FLOOR_HZ, MAX_FLOOR_HZ));
        let band = self.lp_s.process(&self.lp_c, self.hp_s.process(&self.hp_c, x));
        let a = band.abs();

        // Body.
        self.env += (a - self.env) * if a > self.env { self.env_attack } else { self.env_release };
        let u = (band / self.env.max(1e-4)).clamp(-1.2, 1.2);
        let gate = smoothstep(BODY_GATE_DB.0, BODY_GATE_DB.1, lin_to_db(self.env));
        let harmonics = BODY_H2 * (2.0 * u * u - 1.0) + BODY_H3 * (4.0 * u * u * u - 3.0 * u);
        let shaped = harmonics * self.env * gate;
        let shaped = self.body_hp_s.process(&self.body_hp_c, shaped);
        let body_out = self.body_lp_s.process(&self.body_lp_c, shaped) * body;

        // Sub: count cycles, divide by two, and judge whether there is one note to follow.
        self.peak = if a > self.peak { a } else { self.peak + (a - self.peak) * self.peak_release };
        self.since = self.since.saturating_add(1);
        if self.peak > 1e-4 {
            let hi = 0.15 * self.peak;
            if band < -hi {
                self.armed = true;
            } else if self.armed && band > hi {
                self.armed = false;
                let period = self.since as f32;
                self.since = 0;
                let hz = self.fs / period;
                let steady = (SUB_RANGE_HZ.0..=SUB_RANGE_HZ.1).contains(&hz)
                    && self.last_period > 0.0
                    && ((period - self.last_period) / self.last_period).abs() < SUB_TOLERANCE;
                self.cycles = if steady { (self.cycles + 1).min(8) } else { 0 };
                self.last_period = period;
                self.hz += (hz - self.hz) * if self.hz > 0.0 { 0.5 } else { 1.0 };
                self.flip = -self.flip;
            }
        }
        if self.since as f32 > self.fs / (SUB_RANGE_HZ.0 * 0.85) {
            // Too long without a crossing: silence, or something under the range.
            self.cycles = 0;
            self.last_period = 0.0;
        }
        let target = if self.cycles >= SUB_CYCLES { 1.0 } else { 0.0 };
        self.lock += (target - self.lock) * if target > self.lock { self.lock_up } else { self.lock_down };

        let sine = self.post_s.process(&self.post_c, self.flip);
        self.sub_env += (a - self.sub_env) * if a > self.sub_env { self.sub_env_attack } else { self.sub_env_release };
        let octave = sine * self.sub_env * 0.9 * self.lock;
        let sub_out = self.floor_s.process(&self.floor_c, octave) * sub;
        (body_out, sub_out)
    }
}

/// A bank of narrow band-pass filters with envelope followers over the display range, run on the
/// input and on what the plugin adds.
pub struct Analyzer {
    coeffs: [Coeffs; BANDS],
    dry_s: [Biquad; BANDS],
    wet_s: [Biquad; BANDS],
    dry_env: [f32; BANDS],
    wet_env: [f32; BANDS],
    dry_peak: [f32; BANDS],
    wet_peak: [f32; BANDS],
    attack: f32,
    release: f32,
}

pub fn band_hz(i: usize) -> f32 {
    F_MIN * (F_MAX / F_MIN).powf((i as f32 + 0.5) / BANDS as f32)
}

impl Analyzer {
    pub fn new(fs: f32) -> Self {
        Self {
            coeffs: std::array::from_fn(|i| Coeffs::bandpass(fs, band_hz(i), 11.0)),
            dry_s: [Biquad::default(); BANDS],
            wet_s: [Biquad::default(); BANDS],
            dry_env: [0.0; BANDS],
            wet_env: [0.0; BANDS],
            dry_peak: [0.0; BANDS],
            wet_peak: [0.0; BANDS],
            attack: coef_ms(3.0, fs),
            release: coef_ms(40.0, fs),
        }
    }

    #[inline]
    pub fn push(&mut self, dry: f32, wet: f32) {
        for i in 0..BANDS {
            let d = self.dry_s[i].process(&self.coeffs[i], dry).abs();
            let w = self.wet_s[i].process(&self.coeffs[i], wet).abs();
            self.dry_env[i] += (d - self.dry_env[i]) * if d > self.dry_env[i] { self.attack } else { self.release };
            self.wet_env[i] += (w - self.wet_env[i]) * if w > self.wet_env[i] { self.attack } else { self.release };
            self.dry_peak[i] = self.dry_peak[i].max(self.dry_env[i]);
            self.wet_peak[i] = self.wet_peak[i].max(self.wet_env[i]);
        }
    }

    /// Hand the editor the loudest each band got since the last call. The mean of a rectified sine
    /// is 2/pi of its peak, so the scale reads a sine at a band's centre at its own amplitude.
    pub fn publish(&mut self, meter: &Meter) {
        for i in 0..BANDS {
            meter.dry[i].store(self.dry_peak[i] * 1.57, Ordering::Relaxed);
            meter.wet[i].store(self.wet_peak[i] * 1.57, Ordering::Relaxed);
            self.dry_peak[i] = self.dry_env[i];
            self.wet_peak[i] = self.wet_env[i];
        }
    }
}

/// The whole effect: the engines, the analyser, and the sum.
pub struct Bedrock {
    pub meter: Arc<Meter>,
    bass: Bass,
    analyzer: Analyzer,
    channels: usize,
}

impl Bedrock {
    pub fn new() -> Self {
        Self::build(48000.0, 2, Arc::new(Meter::new()))
    }

    pub fn configure(&mut self, fs: f32, channels: usize) {
        *self = Self::build(fs, channels, self.meter.clone());
    }

    fn build(fs: f32, channels: usize, meter: Arc<Meter>) -> Self {
        Self { meter, bass: Bass::new(fs), analyzer: Analyzer::new(fs), channels: channels.clamp(1, 2) }
    }

    pub fn reset(&mut self) {
        let fs = self.bass.fs;
        *self = Self::build(fs, self.channels, self.meter.clone());
    }

    /// Add the body and the sub to the input, to both channels alike. Returns the new sample and
    /// the amount added (for the display).
    pub fn process(&mut self, x: [f32; 2], s: &Settings) -> [f32; 2] {
        let mono = if self.channels == 2 { 0.5 * (x[0] + x[1]) } else { x[0] };
        let (body, sub) = self.bass.process(mono, s.body, s.sub, s.floor_hz);
        let wet = body + sub;
        self.analyzer.push(mono, wet);
        [x[0] + wet, x[1] + if self.channels == 2 { wet } else { 0.0 }]
    }

    pub fn publish(&mut self) {
        self.analyzer.publish(&self.meter);
        self.meter.pitch_hz.store(self.bass.pitch_hz(), Ordering::Relaxed);
        self.meter.lock.store(self.bass.lock(), Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    const FS: f32 = 48000.0;

    fn sine(f: f32, amp: f32, n: usize) -> Vec<f32> {
        (0..n).map(|i| amp * (TAU * f * i as f32 / FS).sin()).collect()
    }

    fn noise(amp: f32, n: usize, seed: u32) -> Vec<f32> {
        let mut s = seed;
        (0..n)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                amp * (s as f32 / u32::MAX as f32 * 2.0 - 1.0)
            })
            .collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    /// Amplitude at `freq`, Hann-windowed.
    fn tone(x: &[f32], freq: f32) -> f32 {
        let n = x.len();
        let (mut re, mut im, mut wsum) = (0.0f64, 0.0f64, 0.0f64);
        for (i, v) in x.iter().enumerate() {
            let w = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n as f64).cos();
            let p = std::f64::consts::TAU * freq as f64 * i as f64 / FS as f64;
            re += *v as f64 * w * p.cos();
            im += *v as f64 * w * p.sin();
            wsum += w;
        }
        (2.0 * re.hypot(im) / wsum) as f32
    }

    /// What the engines add for `input`, as (body, sub).
    fn added(input: &[f32], body: f32, sub: f32, floor: f32) -> (Vec<f32>, Vec<f32>) {
        let mut bass = Bass::new(FS);
        input.iter().map(|&x| bass.process(x, body, sub, floor)).unzip()
    }

    #[test]
    fn body_adds_the_second_and_third_harmonic_and_not_the_note() {
        let (body, _) = added(&sine(50.0, 0.5, 48000), 1.0, 0.0, 28.0);
        let x = &body[24000..];
        assert!(tone(x, 100.0) > 0.1, "{}", tone(x, 100.0));
        assert!(tone(x, 150.0) > 0.07, "{}", tone(x, 150.0));
        assert!(tone(x, 50.0) < 0.03, "{}", tone(x, 50.0));
        assert!(tone(x, 200.0) < 0.03, "{}", tone(x, 200.0));
    }

    #[test]
    fn body_keeps_its_proportions_at_any_level() {
        let ratio = |amp: f32| {
            let (body, _) = added(&sine(60.0, amp, 48000), 1.0, 0.0, 28.0);
            tone(&body[24000..], 120.0) / amp
        };
        let (loud, soft) = (ratio(0.8), ratio(0.03));
        assert!((loud / soft - 1.0).abs() < 0.25, "{loud} vs {soft}");
    }

    #[test]
    fn body_ignores_everything_above_the_low_band_and_rumble() {
        let (body, _) = added(&sine(1000.0, 0.5, 24000), 1.0, 1.0, 28.0);
        assert!(rms(&body[12000..]) < 0.005, "{}", rms(&body[12000..]));
        let (body, _) = added(&sine(8.0, 0.5, 96000), 1.0, 1.0, 28.0);
        assert!(rms(&body[48000..]) < 0.01, "{}", rms(&body[48000..]));
    }

    #[test]
    fn body_stays_silent_below_the_noise_floor() {
        let (body, _) = added(&sine(50.0, 0.0005, 48000), 1.0, 1.0, 28.0);
        assert!(rms(&body[24000..]) < 1e-4, "{}", rms(&body[24000..]));
    }

    #[test]
    fn body_works_on_two_notes_at_once() {
        let two: Vec<f32> = sine(55.0, 0.3, 96000).iter().zip(sine(73.0, 0.3, 96000)).map(|(a, b)| a + b).collect();
        let (body, _) = added(&two, 1.0, 0.0, 28.0);
        let x = &body[48000..];
        assert!(tone(x, 110.0) > 0.03 && tone(x, 146.0) > 0.03, "{} {}", tone(x, 110.0), tone(x, 146.0));
    }

    #[test]
    fn sub_makes_an_octave_down_and_locks_on() {
        let (_, sub) = added(&sine(80.0, 0.4, 48000), 0.0, 1.0, 20.0);
        let x = &sub[24000..];
        assert!(tone(x, 40.0) > 0.2, "{}", tone(x, 40.0));
        assert!(tone(x, 80.0) < 0.05, "{}", tone(x, 80.0));
    }

    /// A bass with a harmonic series: a saw (1/k), a square (odd 1/k), and a sine with a strong second.
    fn rich(f: f32, kind: usize, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| {
                let t = i as f32 / FS;
                let h = |k: usize| (TAU * f * k as f32 * t).sin();
                0.4 * match kind {
                    0 => (1..=10).map(|k| h(k) / k as f32).sum::<f32>() * 0.7,
                    1 => (1..=9).step_by(2).map(|k| h(k) / k as f32).sum::<f32>() * 0.8,
                    _ => h(1) + 0.7 * (TAU * 2.0 * f * t + 1.0).sin(),
                }
            })
            .collect()
    }

    #[test]
    fn sub_locks_on_basses_with_harmonics() {
        for kind in 0..3 {
            for f in [41.0f32, 55.0, 73.0, 98.0] {
                let (_, sub) = added(&rich(f, kind, 48000), 0.0, 1.0, 20.0);
                let (octave, note) = (tone(&sub[24000..], f * 0.5), tone(&sub[24000..], f));
                assert!(octave > 0.12 && octave > 4.0 * note, "kind {kind}, {f} Hz: octave {octave}, note {note}");
            }
        }
    }

    #[test]
    fn sub_follows_a_change_of_note() {
        let input: Vec<f32> = sine(60.0, 0.4, 48000).into_iter().chain(sine(90.0, 0.4, 48000)).collect();
        let (_, sub) = added(&input, 0.0, 1.0, 20.0);
        assert!(tone(&sub[24000..48000], 30.0) > 0.15);
        assert!(tone(&sub[72000..], 45.0) > 0.15, "{}", tone(&sub[72000..], 45.0));
    }

    #[test]
    fn sub_stands_down_under_a_chord_and_under_noise() {
        let single = rms(&added(&sine(55.0, 0.4, 96000), 0.0, 1.0, 20.0).1[48000..]);
        let chord: Vec<f32> = sine(55.0, 0.3, 96000).iter().zip(sine(73.0, 0.3, 96000)).map(|(a, b)| a + b).collect();
        let under_chord = rms(&added(&chord, 0.0, 1.0, 20.0).1[48000..]);
        assert!(under_chord < 0.25 * single, "{under_chord} vs {single}");
        let under_noise = rms(&added(&noise(0.3, 96000, 5), 0.0, 1.0, 20.0).1[48000..]);
        assert!(under_noise < 0.25 * single, "{under_noise} vs {single}");
    }

    #[test]
    fn sub_stays_inside_its_range() {
        // A 150 Hz note is above the range; a 30 Hz one is below it.
        for f in [150.0, 30.0] {
            let (_, sub) = added(&sine(f, 0.4, 48000), 0.0, 1.0, 20.0);
            assert!(rms(&sub[24000..]) < 0.02, "{f} Hz: {}", rms(&sub[24000..]));
        }
    }

    #[test]
    fn the_floor_keeps_the_sub_out_of_the_subsonic() {
        let level = |floor: f32| tone(&added(&sine(48.0, 0.4, 48000), 0.0, 1.0, floor).1[24000..], 24.0);
        assert!(level(20.0) > 3.0 * level(60.0), "{} vs {}", level(20.0), level(60.0));
    }

    #[test]
    fn nothing_is_added_at_zero() {
        let (body, sub) = added(&noise(0.5, 48000, 3), 0.0, 0.0, 28.0);
        assert!(body.iter().chain(&sub).all(|&v| v == 0.0));
    }

    #[test]
    fn output_is_finite_and_bounded_on_hostile_input() {
        let mut input = noise(1.0, 20000, 9);
        input.extend(std::iter::repeat(0.0).take(5000));
        input.extend(sine(45.0, 1.0, 20000));
        input.extend((0..10000).map(|i| if i % 500 == 0 { 1.0 } else { 0.0 }));
        input.extend(sine(70.0, 2.0, 10000));
        let (body, sub) = added(&input, 1.0, 1.0, 20.0);
        assert!(body.iter().chain(&sub).all(|v| v.is_finite() && v.abs() < 3.0));
    }

    #[test]
    fn dry_is_untouched_and_stereo_gets_the_same_addition() {
        let mut b = Bedrock::new();
        b.configure(FS, 2);
        let s = Settings::default();
        let input = sine(50.0, 0.4, 48000);
        let out: Vec<[f32; 2]> = input.iter().map(|&x| b.process([x, x], &s)).collect();
        for (o, x) in out.iter().zip(&input) {
            assert!(((o[0] - x) - (o[1] - x)).abs() < 1e-6);
        }
        let added: Vec<f32> = out.iter().zip(&input).map(|(o, x)| o[0] - x).collect();
        assert!(rms(&added[24000..]) > 0.02);
        // A mono layout gets the same addition on its one channel.
        let mut m = Bedrock::new();
        m.configure(FS, 1);
        let mono: Vec<f32> = input.iter().map(|&x| m.process([x, 0.0], &s)[0] - x).collect();
        assert!((rms(&mono[24000..]) / rms(&added[24000..]) - 1.0).abs() < 0.05);
    }

    #[test]
    fn analyzer_finds_a_tone_and_the_wet_shows_the_harmonics() {
        let mut b = Bedrock::new();
        b.configure(FS, 1);
        let s = Settings { body: 1.0, sub: 0.0, floor_hz: 28.0 };
        for x in sine(60.0, 0.5, 48000) {
            b.process([x, 0.0], &s);
        }
        b.publish();
        let peak = |a: &[AtomicF32; BANDS]| {
            a.iter().map(|v| v.load(Ordering::Relaxed)).enumerate().max_by(|x, y| x.1.total_cmp(&y.1)).unwrap()
        };
        let expected = |hz: f32| ((hz / F_MIN).ln() / (F_MAX / F_MIN).ln() * BANDS as f32) as usize;
        let (dry_band, dry_level) = peak(&b.meter.dry);
        assert!(dry_band.abs_diff(expected(60.0)) <= 1, "{dry_band}");
        assert!((dry_level - 0.5).abs() < 0.15, "{dry_level}");
        let (wet_band, _) = peak(&b.meter.wet);
        assert!(wet_band.abs_diff(expected(120.0)) <= 2, "{wet_band}");
    }
}
