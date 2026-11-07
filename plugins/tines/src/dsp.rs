//! Tines: a bank of tuned modal resonators.
//!
//! Every resonator is a complex one-pole filter, `z <- p z + x` with `p = r e^{jw}`, read out as
//! `Re(z)`. That is exactly one decaying cosine per input sample (a "mode"), it is unconditionally
//! stable, it keeps its state when `p` changes, so pitch glides and decay changes never click, and
//! it runs in `f64` so a 30 Hz mode with a twenty second tail keeps its precision.
//!
//! A *note* is a set of up to sixteen modes whose ratios come from a model:
//!
//! - **String**: the harmonic series, with a stiffness term `f_n = n f0 sqrt(1 + B n^2)` that
//!   stretches the upper partials sharp, as in a piano.
//! - **Pipe**: odd harmonics only (a stopped pipe, a clarinet).
//! - **Bar**: a free-free bar, `((2n+3) pi / 2)^2 / 4.73^2` for n >= 1 (marimba, vibraphone).
//! - **Bell**: a church bell's hum, prime, tierce, quint and nominal.
//! - **Drum**: the circular membrane's Bessel zeros.
//!
//! Up to four notes sound at once, from a chord on the Note knob, from the MIDI keys held (the
//! notes latch when released, so the resonance does not stop when your hand does) or from the
//! pitch tracked in the input. Each mode has its own decay: `Decay` is the 60 dB time of the root
//! and `Damping` makes the upper modes die faster (loss grows with the square of frequency (relative to 500 Hz), like a
//! real string or tube). `Tilt` sets how bright the modes are, `Detune` pulls the left and right
//! banks apart by a few cents, and `Drift` wanders each note slowly out of tune.
//!
//! A feedback path puts the resonators' own (soft-clipped) output back in front of them, so with
//! enough of it the bank sustains and self-oscillates; `Pluck` injects a burst of noise at every
//! onset of the input so even a sine or a bass will excite the upper modes, and `Freeze` closes the
//! input and stretches the decay to forty seconds, holding whatever is ringing.
//!
//! Level: each mode is scaled so white noise in gives the same power out whatever the decay,
//! blended with the unity-peak scaling that makes a tuned input ring loudest. The wet path ends in
//! a soft limiter, so feedback and long decays cannot run away.

use nih_plug::prelude::{AtomicF32, Enum};
use std::f32::consts::TAU;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use crate::pitch::PitchTracker;

pub const MAX_CHANNELS: usize = 2;
pub const MAX_NOTES: usize = 4;
pub const MAX_PARTIALS: usize = 16;
pub const MAX_MODES: usize = MAX_NOTES * MAX_PARTIALS;
/// Samples between coefficient updates.
const CONTROL: usize = 32;
/// Spectrum display: FFT size, hop, and log-spaced bands from `BAND_LO` to `BAND_HI`.
pub const FFT_N: usize = 4096;
pub const HOP: usize = 1024;
pub const BANDS: usize = 192;
pub const BAND_LO: f32 = 30.0;
pub const BAND_HI: f32 = 18000.0;
pub const FLOOR_DB: f32 = -100.0;
/// Lowest root the bank will play, in Hz.
const MIN_HZ: f64 = 16.0;
/// How the unity-peak and equal-noise-power scalings blend (0.5 is equal power).
const LEVEL_EXPONENT: f64 = 0.65;

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The Note knob (and Fine, Octave).
    #[name = "Note"]
    Note,
    /// The keys held; they latch on release.
    #[name = "MIDI"]
    Midi,
    /// The pitch of the input.
    #[name = "Track"]
    Track,
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chord {
    #[name = "Single"]
    Single,
    #[name = "Octave"]
    Octave,
    #[name = "Power"]
    Power,
    #[name = "Major"]
    Major,
    #[name = "Minor"]
    Minor,
    #[name = "Sus4"]
    Sus4,
    #[name = "Min7"]
    Min7,
    #[name = "Maj7"]
    Maj7,
}

impl Chord {
    /// Semitones above the root.
    pub fn intervals(self) -> &'static [f64] {
        match self {
            Chord::Single => &[0.0],
            Chord::Octave => &[0.0, 12.0],
            Chord::Power => &[0.0, 7.0, 12.0],
            Chord::Major => &[0.0, 4.0, 7.0, 12.0],
            Chord::Minor => &[0.0, 3.0, 7.0, 12.0],
            Chord::Sus4 => &[0.0, 5.0, 7.0, 12.0],
            Chord::Min7 => &[0.0, 3.0, 7.0, 10.0],
            Chord::Maj7 => &[0.0, 4.0, 7.0, 11.0],
        }
    }
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Model {
    #[name = "String"]
    String,
    #[name = "Pipe"]
    Pipe,
    #[name = "Bar"]
    Bar,
    #[name = "Bell"]
    Bell,
    #[name = "Drum"]
    Drum,
}

const BELL: [f64; MAX_PARTIALS] =
    [0.5, 1.0, 1.183, 1.506, 2.0, 2.514, 2.662, 3.011, 4.166, 5.433, 6.796, 8.215, 9.653, 11.2, 12.9, 14.5];
const DRUM: [f64; MAX_PARTIALS] =
    [1.0, 1.593, 2.135, 2.295, 2.653, 2.917, 3.156, 3.500, 3.600, 3.652, 4.060, 4.154, 4.231, 4.601, 4.832, 4.903];

impl Model {
    /// Frequency of partial `n` (0-based) over the root.
    pub fn ratio(self, n: usize) -> f64 {
        let k = n as f64;
        match self {
            Model::String => k + 1.0,
            Model::Pipe => 2.0 * k + 1.0,
            Model::Bar => {
                if n == 0 {
                    1.0
                } else {
                    let b = (2.0 * k + 3.0) * std::f64::consts::FRAC_PI_2 / 4.730_040_7;
                    b * b
                }
            }
            Model::Bell => BELL[n],
            Model::Drum => DRUM[n],
        }
    }
}

#[derive(Clone, Copy)]
pub struct Settings {
    pub source: Source,
    pub chord: Chord,
    pub model: Model,
    /// MIDI note number, fractional.
    pub note: f32,
    pub octave: i32,
    pub fine_cents: f32,
    /// 60 dB decay time of the root, in seconds.
    pub decay_s: f32,
    pub damping: f32,
    /// -1 dark, 0 natural, +1 bright.
    pub tilt: f32,
    pub partials: usize,
    pub stiffness: f32,
    pub detune: f32,
    pub drift: f32,
    pub feedback: f32,
    pub pluck: f32,
    pub tone_hz: f32,
    pub level_db: f32,
    pub mix: f32,
    pub freeze: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            source: Source::Note,
            chord: Chord::Single,
            model: Model::String,
            note: 45.0,
            octave: 0,
            fine_cents: 0.0,
            decay_s: 1.2,
            damping: 0.35,
            tilt: 0.0,
            partials: 8,
            stiffness: 0.0,
            detune: 0.2,
            drift: 0.15,
            feedback: 0.0,
            pluck: 0.0,
            tone_hz: 20000.0,
            level_db: 0.0,
            mix: 0.5,
            freeze: false,
        }
    }
}

/// Frequency of a (fractional) MIDI note.
pub fn mtof(note: f64) -> f64 {
    440.0 * ((note - 69.0) / 12.0).exp2()
}

pub fn ftom(hz: f64) -> f64 {
    69.0 + 12.0 * (hz / 440.0).log2()
}

// ---------------------------------------------------------------------------------------------
// Meter
// ---------------------------------------------------------------------------------------------

/// What the audio thread publishes for the editor.
pub struct Meter {
    pub frames: AtomicUsize,
    /// Level in dB (full-scale sine = 0) of the dry input and the output, in `BANDS` log bands.
    pub spec_in: [AtomicF32; BANDS],
    pub spec_out: [AtomicF32; BANDS],
    pub sample_rate: AtomicF32,
    /// The root of the first note as it sounds now (after glide and drift); 0 when silent.
    pub root_hz: AtomicF32,
    /// Frequency of each mode (0 when it is not sounding) and the amplitude it is ringing at.
    pub mode_hz: [AtomicF32; MAX_MODES],
    pub mode_level: [AtomicF32; MAX_MODES],
    /// The pitch found in the input, or 0.
    pub pitch_hz: AtomicF32,
}

impl Meter {
    fn new() -> Self {
        Self {
            frames: AtomicUsize::new(0),
            spec_in: std::array::from_fn(|_| AtomicF32::new(FLOOR_DB)),
            spec_out: std::array::from_fn(|_| AtomicF32::new(FLOOR_DB)),
            sample_rate: AtomicF32::new(48000.0),
            root_hz: AtomicF32::new(0.0),
            mode_hz: std::array::from_fn(|_| AtomicF32::new(0.0)),
            mode_level: std::array::from_fn(|_| AtomicF32::new(0.0)),
            pitch_hz: AtomicF32::new(0.0),
        }
    }
}

pub fn band_hz(b: usize) -> f32 {
    BAND_LO * (BAND_HI / BAND_LO).powf(b as f32 / (BANDS - 1) as f32)
}

/// Windowed FFT of the dry and the output, folded into log-spaced bands, every `HOP` samples.
struct Analyzer {
    ring: [Vec<f32>; 2],
    pos: usize,
    count: usize,
    window: Vec<f32>,
    re: Vec<f32>,
    im: Vec<f32>,
    tw_re: Vec<f32>,
    tw_im: Vec<f32>,
    /// Per band: the centre bin and the inclusive range of whole bins it covers (empty when the
    /// band is narrower than a bin, which is then interpolated).
    bands: Vec<(f32, usize, usize)>,
}

impl Analyzer {
    fn new(fs: f32) -> Self {
        let step = (BAND_HI / BAND_LO).powf(1.0 / (BANDS - 1) as f32).sqrt();
        let top = FFT_N / 2 - 1;
        let bands = (0..BANDS)
            .map(|b| {
                let f = band_hz(b);
                let to_bin = |hz: f32| hz * FFT_N as f32 / fs;
                let centre = to_bin(f).min(top as f32 - 1.0);
                let lo = (to_bin(f / step)).ceil() as usize;
                let hi = (to_bin(f * step)).floor().min(top as f32) as usize;
                (centre, lo, hi)
            })
            .collect();
        Self {
            ring: [vec![0.0; FFT_N], vec![0.0; FFT_N]],
            pos: 0,
            count: 0,
            window: (0..FFT_N).map(|i| 0.5 - 0.5 * (TAU * i as f32 / FFT_N as f32).cos()).collect(),
            re: vec![0.0; FFT_N],
            im: vec![0.0; FFT_N],
            tw_re: (0..FFT_N / 2).map(|k| (TAU * k as f32 / FFT_N as f32).cos()).collect(),
            tw_im: (0..FFT_N / 2).map(|k| -(TAU * k as f32 / FFT_N as f32).sin()).collect(),
            bands,
        }
    }

    fn clear(&mut self) {
        for r in &mut self.ring {
            r.fill(0.0);
        }
        self.count = 0;
    }

    fn push(&mut self, input: f32, output: f32, meter: &Meter) {
        self.ring[0][self.pos] = input;
        self.ring[1][self.pos] = output;
        self.pos = (self.pos + 1) % FFT_N;
        self.count += 1;
        if self.count < HOP {
            return;
        }
        self.count = 0;
        let norm = FFT_N as f32 * 0.25;
        for which in 0..2 {
            for i in 0..FFT_N {
                self.re[i] = self.ring[which][(self.pos + i) % FFT_N] * self.window[i];
                self.im[i] = 0.0;
            }
            fft(&mut self.re, &mut self.im, &self.tw_re, &self.tw_im);
            let mag = |k: usize| (self.re[k] * self.re[k] + self.im[k] * self.im[k]).sqrt() / norm;
            let target = if which == 0 { &meter.spec_in } else { &meter.spec_out };
            for (b, &(centre, lo, hi)) in self.bands.iter().enumerate() {
                let m = if hi >= lo {
                    (lo..=hi).fold(0.0_f32, |m, k| m.max(mag(k)))
                } else {
                    let i = centre.floor() as usize;
                    let t = centre - i as f32;
                    mag(i) * (1.0 - t) + mag(i + 1) * t
                };
                target[b].store((20.0 * (m + 1e-7).log10()).max(FLOOR_DB), Ordering::Relaxed);
            }
        }
        meter.frames.fetch_add(1, Ordering::Release);
    }
}

/// In-place iterative radix-2 FFT.
fn fft(re: &mut [f32], im: &mut [f32], tw_re: &[f32], tw_im: &[f32]) {
    let n = re.len();
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
                let (wr, wi) = (tw_re[k * step], tw_im[k * step]);
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

// ---------------------------------------------------------------------------------------------
// The bank
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct Slot {
    /// Smoothed frequency in Hz and the level the note is faded to.
    hz: f64,
    gain: f32,
    drift_phase: [f64; 2],
}

pub struct Resonator {
    fs: f32,
    channels: usize,
    pub meter: Arc<Meter>,
    slots: [Slot; MAX_NOTES],
    zr: [[f64; MAX_MODES]; MAX_CHANNELS],
    zi: [[f64; MAX_MODES]; MAX_CHANNELS],
    pr: [[f64; MAX_MODES]; MAX_CHANNELS],
    pi: [[f64; MAX_MODES]; MAX_CHANNELS],
    gain: [[f64; MAX_MODES]; MAX_CHANNELS],
    live: [usize; MAX_MODES],
    n_live: usize,
    tick: usize,
    held: [u8; MAX_NOTES],
    n_held: usize,
    down: u128,
    tracker: PitchTracker,
    last_pitch: f32,
    env_fast: f32,
    env_slow: f32,
    noise: u32,
    fb: [f32; MAX_CHANNELS],
    input_gain: f32,
    tone: [[f32; 2]; MAX_CHANNELS],
    analyzer: Analyzer,
}

fn soft_limit(x: f32) -> f32 {
    const KNEE: f32 = 0.7;
    let a = x.abs();
    if a <= KNEE {
        x
    } else {
        x.signum() * (KNEE + (1.0 - KNEE) * ((a - KNEE) / (1.0 - KNEE)).tanh())
    }
}

impl Resonator {
    pub fn new() -> Self {
        let slot = Slot { hz: 0.0, gain: 0.0, drift_phase: [0.0; 2] };
        Self {
            fs: 48000.0,
            channels: 2,
            meter: Arc::new(Meter::new()),
            slots: [slot; MAX_NOTES],
            zr: [[0.0; MAX_MODES]; MAX_CHANNELS],
            zi: [[0.0; MAX_MODES]; MAX_CHANNELS],
            pr: [[0.0; MAX_MODES]; MAX_CHANNELS],
            pi: [[0.0; MAX_MODES]; MAX_CHANNELS],
            gain: [[0.0; MAX_MODES]; MAX_CHANNELS],
            live: [0; MAX_MODES],
            n_live: 0,
            tick: 0,
            held: [0; MAX_NOTES],
            n_held: 0,
            down: 0,
            tracker: PitchTracker::new(48000.0, 50.0),
            last_pitch: 0.0,
            env_fast: 0.0,
            env_slow: 0.0,
            noise: 0x1234_5678,
            fb: [0.0; MAX_CHANNELS],
            input_gain: 1.0,
            tone: [[0.0; 2]; MAX_CHANNELS],
            analyzer: Analyzer::new(48000.0),
        }
    }

    pub fn configure(&mut self, fs: f32, channels: usize) {
        self.fs = fs;
        self.channels = channels.clamp(1, MAX_CHANNELS);
        self.tracker = PitchTracker::new(fs, 50.0);
        self.analyzer = Analyzer::new(fs);
        self.meter.sample_rate.store(fs, Ordering::Relaxed);
        self.reset();
    }

    pub fn reset(&mut self) {
        for ch in 0..MAX_CHANNELS {
            self.zr[ch].fill(0.0);
            self.zi[ch].fill(0.0);
            self.gain[ch].fill(0.0);
        }
        for (i, s) in self.slots.iter_mut().enumerate() {
            s.gain = 0.0;
            s.hz = 0.0;
            // Spread the drift phases so the notes never wander together.
            s.drift_phase = [i as f64 * 1.7, i as f64 * 2.9 + 0.5];
        }
        self.n_live = 0;
        self.tick = 0;
        self.tracker.reset();
        self.last_pitch = 0.0;
        self.env_fast = 0.0;
        self.env_slow = 0.0;
        self.fb = [0.0; MAX_CHANNELS];
        self.input_gain = 1.0;
        self.tone = [[0.0; 2]; MAX_CHANNELS];
        self.analyzer.clear();
    }

    /// A key went down. When none was held, the new key starts a fresh chord.
    pub fn note_on(&mut self, note: u8) {
        if self.down == 0 {
            self.n_held = 0;
        }
        self.down |= 1u128 << (note & 127);
        if self.held[..self.n_held].contains(&note) {
            return;
        }
        if self.n_held == MAX_NOTES {
            self.held.copy_within(1.., 0);
            self.n_held -= 1;
        }
        self.held[self.n_held] = note;
        self.n_held += 1;
    }

    pub fn note_off(&mut self, note: u8) {
        self.down &= !(1u128 << (note & 127));
    }

    /// The notes to sound, as frequencies, and how many there are.
    fn targets(&self, s: &Settings) -> ([f64; MAX_NOTES], usize) {
        let shift = s.octave as f64 * 12.0 + s.fine_cents as f64 / 100.0;
        let mut out = [0.0; MAX_NOTES];
        let mut n = 0;
        let root = match s.source {
            Source::Midi if self.n_held > 0 => {
                for &note in &self.held[..self.n_held] {
                    out[n] = mtof(note as f64 + shift);
                    n += 1;
                }
                None
            }
            Source::Track if self.last_pitch > 0.0 => Some(ftom(self.last_pitch as f64)),
            _ => Some(s.note as f64),
        };
        if let Some(root) = root {
            for &interval in s.chord.intervals() {
                out[n] = mtof(root + interval + shift);
                n += 1;
            }
        }
        let top = self.fs as f64 * 0.45;
        for hz in &mut out[..n] {
            *hz = hz.clamp(MIN_HZ, top);
        }
        (out, n)
    }

    /// Smooth the notes towards their targets and recompute every mode's coefficients.
    fn control(&mut self, s: &Settings) {
        let fs = self.fs as f64;
        let dt = CONTROL as f64 / fs;
        let a_hz = 1.0 - (-dt / 0.012).exp();
        let a_gain = 1.0 - (-dt / 0.02).exp() as f32;
        let (targets, count) = self.targets(s);

        // The detune splits the two banks; a mono instance has only one.
        let spread = if self.channels == 2 { s.detune as f64 * 40.0 * 0.5 } else { 0.0 };
        let t60 = if s.freeze { (s.decay_s as f64).max(40.0) } else { s.decay_s as f64 };
        let loss = 3.0 * s.damping as f64;
        let stiff = (s.stiffness as f64).powi(2) * 0.02;
        let tilt = 0.8 - 1.2 * s.tilt as f64;
        let partials = s.partials.clamp(1, MAX_PARTIALS);
        let nyquist = fs * 0.45;
        let chord_norm = 1.0 / (count.max(1) as f64).sqrt();

        self.n_live = 0;
        for i in 0..MAX_NOTES {
            let want = i < count;
            let slot = &mut self.slots[i];
            if want {
                if slot.gain < 1e-4 {
                    slot.hz = targets[i];
                } else {
                    slot.hz *= ((targets[i] / slot.hz).ln() * a_hz).exp();
                }
            }
            slot.gain += (if want { 1.0 } else { 0.0 } - slot.gain) * a_gain;
            if !want && slot.gain < 1e-4 {
                slot.gain = 0.0;
            }
            // Two slow, unrelated sines per note: a wander that never repeats audibly.
            slot.drift_phase[0] += TAU as f64 * 0.23 * dt * (1.0 + 0.19 * i as f64);
            slot.drift_phase[1] += TAU as f64 * 0.61 * dt * (1.0 + 0.13 * i as f64);

            let base = i * MAX_PARTIALS;
            if slot.gain == 0.0 {
                for ch in 0..MAX_CHANNELS {
                    for m in base..base + MAX_PARTIALS {
                        self.zr[ch][m] = 0.0;
                        self.zi[ch][m] = 0.0;
                        self.gain[ch][m] = 0.0;
                    }
                }
                self.meter_slot(i, 0.0);
                continue;
            }

            let wander = s.drift as f64 * 25.0 * (slot.drift_phase[0].sin() + 0.6 * slot.drift_phase[1].sin()) / 1.6;
            let root = slot.hz * (wander / 1200.0).exp2();

            // The partials' frequencies and weights, before the per-channel detune.
            let mut hz = [0.0; MAX_PARTIALS];
            let mut weight = [0.0; MAX_PARTIALS];
            let mut power = 0.0;
            for n in 0..partials {
                let ratio = s.model.ratio(n);
                let f = root * ratio * (1.0 + stiff * ratio * ratio).sqrt();
                if f < nyquist {
                    hz[n] = f;
                    weight[n] = ratio.powf(-tilt);
                    power += weight[n] * weight[n];
                }
            }
            let norm = chord_norm * slot.gain as f64 / power.max(1e-9).sqrt();

            for ch in 0..self.channels {
                let cents = if ch == 0 { spread } else { -spread };
                let detune = (cents / 1200.0).exp2();
                for n in 0..MAX_PARTIALS {
                    let m = base + n;
                    if hz[n] == 0.0 {
                        self.zr[ch][m] = 0.0;
                        self.zi[ch][m] = 0.0;
                        self.gain[ch][m] = 0.0;
                        continue;
                    }
                    let f = (hz[n] * detune).min(nyquist);
                    let sigma = 6.9078 / t60.max(0.005) * (1.0 + loss * (f / 500.0).powi(2));
                    let r = (-sigma / fs).exp();
                    let w = std::f64::consts::TAU * f / fs;
                    let (sin, cos) = w.sin_cos();
                    self.pr[ch][m] = r * cos;
                    self.pi[ch][m] = r * sin;
                    self.gain[ch][m] = norm * weight[n] * (2.0 * (1.0 - r)).powf(LEVEL_EXPONENT);
                    if ch == 0 {
                        self.live[self.n_live] = m;
                        self.n_live += 1;
                    }
                    // Flush tails the CPU would handle as denormals.
                    let e = self.zr[ch][m] * self.zr[ch][m] + self.zi[ch][m] * self.zi[ch][m];
                    if e < 1e-60 {
                        self.zr[ch][m] = 0.0;
                        self.zi[ch][m] = 0.0;
                    }
                }
            }
            self.meter_slot(i, root);
        }
        self.meter.root_hz.store(if self.slots[0].gain > 0.0 { self.slots[0].hz as f32 } else { 0.0 }, Ordering::Relaxed);
        self.meter.pitch_hz.store(self.last_pitch, Ordering::Relaxed);
    }

    /// Publish the modes of one note (all zero when `root` is).
    fn meter_slot(&self, i: usize, root: f64) {
        for n in 0..MAX_PARTIALS {
            let m = i * MAX_PARTIALS + n;
            let g = self.gain[0][m];
            let live = root > 0.0 && g > 0.0;
            let level = if live {
                let mut sum = 0.0;
                for ch in 0..self.channels {
                    sum += (self.zr[ch][m].powi(2) + self.zi[ch][m].powi(2)).sqrt() * self.gain[ch][m];
                }
                (sum / self.channels as f64) as f32
            } else {
                0.0
            };
            let f = if live { (self.pi[0][m].atan2(self.pr[0][m]) as f32 / TAU) * self.fs } else { 0.0 };
            self.meter.mode_hz[m].store(f, Ordering::Relaxed);
            self.meter.mode_level[m].store(level, Ordering::Relaxed);
        }
    }

    fn noise(&mut self) -> f32 {
        // xorshift32, scaled to +-1.
        self.noise ^= self.noise << 13;
        self.noise ^= self.noise >> 17;
        self.noise ^= self.noise << 5;
        self.noise as f32 / 2_147_483_648.0 - 1.0
    }

    pub fn process(&mut self, input: [f32; MAX_CHANNELS], s: &Settings) -> [f32; MAX_CHANNELS] {
        let channels = self.channels;
        let mono = if channels == 2 { (input[0] + input[1]) * 0.5 } else { input[0] };

        if s.source == Source::Track {
            if let Some(frame) = self.tracker.push(mono) {
                if frame.voiced {
                    self.last_pitch = frame.f0;
                }
            }
        }
        if self.tick == 0 {
            self.control(s);
        }
        self.tick = (self.tick + 1) % CONTROL;

        // Onsets: the fast envelope jumping over the slow one.
        let a = mono.abs();
        self.env_fast += (a - self.env_fast) * if a > self.env_fast { 0.05 } else { 0.004 };
        self.env_slow += (a - self.env_slow) * if a > self.env_slow { 0.002 } else { 0.0006 };
        let onset = ((self.env_fast - 1.4 * self.env_slow).max(0.0) * 8.0).min(1.0);

        self.input_gain += ((if s.freeze { 0.0 } else { 1.0 }) - self.input_gain) * 0.002;
        let fb_amount = s.feedback * s.feedback * 0.12;
        let tone_k = 1.0 - (-TAU * s.tone_hz.min(19900.0) / self.fs).exp();
        let tone_on = s.tone_hz < 19900.0;
        let level = 10.0_f32.powf(s.level_db / 20.0) * 2.0;

        let mut out = [0.0; MAX_CHANNELS];
        for ch in 0..channels {
            let burst = self.noise() * onset * s.pluck * 0.5;
            let x = ((input[ch] + burst) * self.input_gain + fb_amount * self.fb[ch]) as f64;
            let mut acc = 0.0_f64;
            for &m in &self.live[..self.n_live] {
                let (zr, zi) = (self.zr[ch][m], self.zi[ch][m]);
                let (pr, pi) = (self.pr[ch][m], self.pi[ch][m]);
                let nr = pr * zr - pi * zi + x;
                let ni = pr * zi + pi * zr;
                self.zr[ch][m] = nr;
                self.zi[ch][m] = ni;
                acc += nr * self.gain[ch][m];
            }
            let mut wet = acc as f32;
            self.fb[ch] = (wet * 2.0).tanh();
            if tone_on {
                for stage in 0..2 {
                    let y = &mut self.tone[ch][stage];
                    *y += (wet - *y) * tone_k;
                    wet = *y;
                }
            } else {
                self.tone[ch] = [wet; 2];
            }
            wet = soft_limit(wet * level);
            out[ch] = input[ch] * (1.0 - s.mix) + wet * s.mix;
        }
        let mono_out = if channels == 2 { (out[0] + out[1]) * 0.5 } else { out[0] };
        self.analyzer.push(mono, mono_out, &self.meter);
        out
    }
}

impl Default for Resonator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    const FS: f32 = 48000.0;

    fn fresh(channels: usize) -> Resonator {
        let mut r = Resonator::new();
        r.configure(FS, channels);
        r
    }

    fn dry() -> Settings {
        // One undamped, undetuned partial of a plain note, fully wet.
        Settings { mix: 1.0, damping: 0.0, detune: 0.0, drift: 0.0, partials: 1, ..Settings::default() }
    }

    /// Run `n` samples of mono input from `f`, returning the left output.
    fn run(r: &mut Resonator, s: &Settings, n: usize, mut f: impl FnMut(usize) -> f32) -> Vec<f32> {
        (0..n)
            .map(|i| {
                let x = f(i);
                r.process([x, x], s)[0]
            })
            .collect()
    }

    /// Let the notes fade in, then strike once and return `n` samples of the left output.
    fn hit(r: &mut Resonator, s: &Settings, n: usize) -> Vec<f32> {
        run(r, s, 4800, |_| 0.0);
        run(r, s, n, impulse)
    }

    fn impulse(i: usize) -> f32 {
        if i == 0 {
            1.0
        } else {
            0.0
        }
    }

    /// Amplitude of the component of `x` at `hz` (a single-bin DFT).
    fn amp(x: &[f32], hz: f32) -> f32 {
        let (mut re, mut im) = (0.0_f64, 0.0_f64);
        for (i, &v) in x.iter().enumerate() {
            let p = std::f64::consts::TAU * hz as f64 * i as f64 / FS as f64;
            re += v as f64 * p.cos();
            im += v as f64 * p.sin();
        }
        (2.0 * (re * re + im * im).sqrt() / x.len() as f64) as f32
    }

    fn db(a: f32) -> f32 {
        20.0 * (a + 1e-12).log10()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    fn noise_source(seed: u32) -> impl FnMut(usize) -> f32 {
        let mut state = seed;
        move |_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state as f32 / 2_147_483_648.0 - 1.0) * 0.3
        }
    }

    #[test]
    fn zero_mix_is_a_null_test() {
        let mut r = fresh(2);
        let s = Settings { mix: 0.0, feedback: 1.0, pluck: 1.0, ..Settings::default() };
        let mut noise = noise_source(7);
        for i in 0..20000 {
            let x = noise(i);
            let out = r.process([x, -x * 0.5], &s);
            assert_eq!(out, [x, -x * 0.5]);
        }
    }

    #[test]
    fn an_impulse_rings_at_the_root() {
        let mut r = fresh(2);
        let hz = mtof(45.0) as f32;
        let y = hit(&mut r, &dry(), 24000);
        let at = amp(&y[2000..], hz);
        assert!(db(at) - db(amp(&y[2000..], hz * 1.5)) > 30.0, "{at}");
        assert!(db(at) - db(amp(&y[2000..], hz * 0.7)) > 30.0);
    }

    #[test]
    fn decay_is_a_sixty_db_time() {
        let mut r = fresh(2);
        let s = Settings { decay_s: 1.0, ..dry() };
        let y = hit(&mut r, &s, 60000);
        let period = (FS / mtof(45.0) as f32) as usize + 1;
        let peak = |from: usize| y[from..from + period].iter().fold(0.0_f32, |m, v| m.max(v.abs()));
        let ratio = peak(48000 - period) / peak(100);
        assert!((0.0005..0.0022).contains(&ratio), "{ratio}");
    }

    #[test]
    fn damping_makes_the_upper_modes_die_first() {
        let tail = |damping: f32| {
            let mut r = fresh(2);
            let s = Settings { partials: 8, damping, decay_s: 2.0, ..dry() };
            let y = hit(&mut r, &s, 48000);
            // Compare the 6th partial in the second half against the root.
            let root = mtof(45.0) as f32;
            db(amp(&y[24000..], root * 6.0)) - db(amp(&y[24000..], root))
        };
        assert!(tail(0.0) - tail(0.8) > 20.0, "{} {}", tail(0.0), tail(0.8));
    }

    #[test]
    fn pipe_has_only_odd_harmonics() {
        let spectrum = |model: Model| {
            let mut r = fresh(2);
            let s = Settings { model, partials: 6, ..dry() };
            let y = hit(&mut r, &s, 24000);
            let root = mtof(45.0) as f32;
            (db(amp(&y[1000..], root * 2.0)), db(amp(&y[1000..], root * 3.0)))
        };
        let (even, odd) = spectrum(Model::Pipe);
        assert!(odd - even > 30.0, "{even} {odd}");
        let (even, odd) = spectrum(Model::String);
        assert!((odd - even).abs() < 12.0, "{even} {odd}");
    }

    #[test]
    fn bar_partials_are_inharmonic() {
        let mut r = fresh(2);
        let s = Settings { model: Model::Bar, partials: 3, ..dry() };
        hit(&mut r, &s, 200);
        let root = mtof(45.0) as f32;
        let hz = |n: usize| r.meter.mode_hz[n].load(Ordering::Relaxed) / root;
        assert!((hz(1) - 2.756).abs() < 0.01, "{}", hz(1));
        assert!((hz(2) - 5.404).abs() < 0.01, "{}", hz(2));
    }

    #[test]
    fn stiffness_stretches_the_upper_partials_sharp() {
        let top = |stiffness: f32| {
            let mut r = fresh(2);
            let s = Settings { partials: 8, stiffness, ..dry() };
            hit(&mut r, &s, 200);
            r.meter.mode_hz[7].load(Ordering::Relaxed) / mtof(45.0) as f32
        };
        assert!((top(0.0) - 8.0).abs() < 0.01);
        assert!(top(1.0) > 8.0 * 1.2, "{}", top(1.0));
    }

    #[test]
    fn chords_add_their_notes() {
        let ring = |chord: Chord| {
            let mut r = fresh(2);
            let s = Settings { chord, ..dry() };
            let y = hit(&mut r, &s, 24000);
            let third = mtof(45.0 + 4.0) as f32;
            db(amp(&y[3000..], third))
        };
        // The floor is the root's own skirt in the DFT, about 80 dB down.
        assert!(ring(Chord::Major) - ring(Chord::Single) > 18.0);
        assert!(ring(Chord::Minor) - ring(Chord::Major) < -18.0);
    }

    #[test]
    fn midi_notes_set_the_pitch_and_latch() {
        let mut r = fresh(2);
        let s = Settings { source: Source::Midi, ..dry() };
        r.note_on(57);
        let y = hit(&mut r, &s, 24000);
        assert!(db(amp(&y[2000..], 220.0)) - db(amp(&y[2000..], 110.0)) > 30.0);
        // After release the note keeps sounding: a new impulse still rings at 220 Hz.
        r.note_off(57);
        let y = hit(&mut r, &s, 24000);
        assert!(db(amp(&y[2000..], 220.0)) - db(amp(&y[2000..], 110.0)) > 30.0);
        // A new key then replaces it.
        r.note_on(60);
        let y = hit(&mut r, &s, 24000);
        let c4 = mtof(60.0) as f32;
        assert!(db(amp(&y[3000..], c4)) - db(amp(&y[3000..], 220.0)) > 30.0);
    }

    #[test]
    fn octave_and_fine_transpose() {
        let mut r = fresh(2);
        let s = Settings { octave: 1, fine_cents: 0.0, ..dry() };
        let y = hit(&mut r, &s, 24000);
        assert!(db(amp(&y[2000..], 220.0)) - db(amp(&y[2000..], 110.0)) > 30.0);
    }

    #[test]
    fn track_follows_the_input_pitch() {
        let mut r = fresh(2);
        let s = Settings { source: Source::Track, mix: 1.0, ..Settings::default() };
        let mut phase = 0.0_f32;
        run(&mut r, &s, 24000, |_| {
            phase = (phase + TAU * 196.0 / FS) % TAU;
            phase.sin() * 0.5
        });
        let root = r.meter.root_hz.load(Ordering::Relaxed);
        assert!((root / 196.0 - 1.0).abs() < 0.02, "{root}");
    }

    #[test]
    fn feedback_sustains_past_the_decay() {
        let tail = |feedback: f32| {
            let mut r = fresh(2);
            let s = Settings { decay_s: 0.3, feedback, ..dry() };
            let y = hit(&mut r, &s, 96000);
            rms(&y[72000..])
        };
        let (open, closed) = (tail(0.0), tail(1.0));
        assert!(db(closed) - db(open) > 40.0, "{} {}", db(open), db(closed));
    }

    #[test]
    fn freeze_holds_the_ring_after_the_input_stops() {
        let tail = |freeze: bool| {
            let mut r = fresh(2);
            let s = Settings { decay_s: 0.2, ..dry() };
            let mut noise = noise_source(3);
            run(&mut r, &s, 12000, |i| noise(i));
            let s = Settings { freeze, ..s };
            let y = run(&mut r, &s, 96000, |_| 0.0);
            rms(&y[84000..])
        };
        assert!(db(tail(true)) - db(tail(false)) > 40.0);
    }

    #[test]
    fn pluck_rings_the_upper_modes_from_a_pure_low_sine() {
        let tail = |pluck: f32| {
            let mut r = fresh(2);
            let s = Settings { partials: 8, pluck, decay_s: 0.5, mix: 1.0, ..Settings::default() };
            run(&mut r, &s, 4800, |_| 0.0);
            let mut phase = 0.0_f32;
            // Bursts of a 55 Hz sine with a smooth 125 ms envelope: nothing in it near 550 Hz.
            let y = run(&mut r, &s, 48000, |i| {
                phase = (phase + TAU * 55.0 / FS) % TAU;
                let k = i % 24000;
                let env = if k < 6000 { (PI * k as f32 / 6000.0).sin().powi(2) } else { 0.0 };
                phase.sin() * 0.5 * env
            });
            amp(&y[..24000], mtof(45.0) as f32 * 5.0)
        };
        assert!(db(tail(1.0)) - db(tail(0.0)) > 20.0, "{} {}", db(tail(1.0)), db(tail(0.0)));
    }

    #[test]
    fn detune_decorrelates_the_channels_and_zero_detune_does_not() {
        let spread = |detune: f32| {
            let mut r = fresh(2);
            let s = Settings { detune, partials: 4, ..dry() };
            let mut noise = noise_source(5);
            let mut diff = 0.0;
            let mut sum = 0.0;
            for i in 0..24000 {
                let x = noise(i);
                let o = r.process([x, x], &s);
                if i > 12000 {
                    diff += (o[0] - o[1]).powi(2);
                    sum += (o[0] + o[1]).powi(2);
                }
            }
            diff / sum
        };
        assert!(spread(0.0) < 1e-9);
        assert!(spread(1.0) > 0.01);
    }

    #[test]
    fn mono_works() {
        let mut r = fresh(1);
        let s = Settings { chord: Chord::Major, ..dry() };
        let y: Vec<f32> = (0..24000).map(|i| r.process([impulse(i), 0.0], &s)[0]).collect();
        assert!(db(amp(&y[2000..], mtof(45.0) as f32)) > -60.0);
        assert!(y.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn every_setting_stays_finite_and_bounded() {
        for model in [Model::String, Model::Pipe, Model::Bar, Model::Bell, Model::Drum] {
            for chord in [Chord::Single, Chord::Major, Chord::Maj7, Chord::Octave] {
                for (note, fs) in [(12.0, 44100.0), (96.0, 44100.0), (60.0, 96000.0)] {
                    let mut r = Resonator::new();
                    r.configure(fs, 2);
                    let s = Settings {
                        model,
                        chord,
                        note,
                        partials: 16,
                        decay_s: 20.0,
                        feedback: 1.0,
                        pluck: 1.0,
                        stiffness: 1.0,
                        drift: 1.0,
                        detune: 1.0,
                        damping: 0.0,
                        level_db: 12.0,
                        mix: 1.0,
                        ..Settings::default()
                    };
                    let mut noise = noise_source(11);
                    for i in 0..30000 {
                        let x = noise(i) * 3.0;
                        let o = r.process([x, -x], &s);
                        assert!(o.iter().all(|v| v.is_finite() && v.abs() < 4.0), "{model:?} {chord:?} {note} {o:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn going_silent_leaves_silence() {
        let mut r = fresh(2);
        let s = Settings { decay_s: 0.5, mix: 1.0, ..Settings::default() };
        let mut noise = noise_source(2);
        run(&mut r, &s, 12000, |i| noise(i));
        let y = run(&mut r, &s, 96000, |_| 0.0);
        assert!(db(rms(&y[80000..])) < -120.0, "{}", db(rms(&y[80000..])));
    }

    #[test]
    fn broadband_input_comes_out_at_a_sane_level() {
        // A full-wet noise input should land within a few dB of the input level for a range of
        // decays: the scaling is meant to keep decay from changing loudness much.
        let mut levels = vec![];
        for decay in [0.1, 1.0, 8.0] {
            let mut r = fresh(2);
            let s = Settings { decay_s: decay, mix: 1.0, ..Settings::default() };
            let mut noise = noise_source(9);
            let y = run(&mut r, &s, 96000, |i| noise(i));
            let x_rms = 0.3 / 3.0_f32.sqrt();
            levels.push(db(rms(&y[48000..])) - db(x_rms));
        }
        for l in &levels {
            assert!((-24.0..=6.0).contains(l), "{levels:?}");
        }
        let (lo, hi) = levels.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &l| (lo.min(l), hi.max(l)));
        assert!(hi - lo < 14.0, "{levels:?}");
    }

    #[test]
    fn the_spectrum_shows_the_ring() {
        let mut r = fresh(2);
        let s = Settings { decay_s: 5.0, ..dry() };
        hit(&mut r, &s, 40000);
        let band = (0..BANDS).min_by(|&a, &b| (band_hz(a) - 110.0).abs().total_cmp(&(band_hz(b) - 110.0).abs())).unwrap();
        let at = r.meter.spec_out[band].load(Ordering::Relaxed);
        let away = r.meter.spec_out[band + 30].load(Ordering::Relaxed);
        assert!(at - away > 30.0, "{at} {away}");
    }
}
