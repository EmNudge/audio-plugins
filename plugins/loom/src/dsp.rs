//! Spectral playheads.
//!
//! The input (plus the fed-back output) is analysed by a 4x-overlap STFT and every frame is kept
//! in a ring of history as magnitude, phase and phase deviation (how far each bin's phase moved
//! from where a steady tone at the bin centre would have put it). Up to eight playheads read that
//! history at their own delay. A head's delay may fall between two frames, so magnitudes are
//! interpolated; its band-pass window picks which source bins it hears; and its pitch offset maps
//! output bin `k` to source position `k / ratio`. Phases are not copied but accumulated in the
//! phase-vocoder way: each output bin advances by the instantaneous frequency of the source bin it
//! reads, scaled by the ratio. That keeps shifted, gliding and delayed heads continuous from frame
//! to frame, and at ratio 1 it reproduces the source frame's phases exactly, so a head with no
//! offset is a plain delay. The heads' complex spectra are summed per channel, inverse-transformed
//! once and overlap-added.

use nih_plug::prelude::AtomicF32;
use spectral::{sqrt_hann, Fft};
use std::f32::consts::{FRAC_PI_2, TAU};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

pub const FFT_SIZE: usize = 2048;
pub const HOP: usize = FFT_SIZE / 4;
/// Index of the Nyquist bin; per-bin arrays have `NB` entries.
pub const BINS: usize = FFT_SIZE / 2;
pub const NB: usize = BINS + 1;
/// Samples between input and output: one whole frame has to be collected before it can be output.
pub const LATENCY: usize = FFT_SIZE;
pub const MAX_CHANNELS: usize = 2;
pub const HEADS: usize = 8;

/// Longest head delay, and the span of history the display shows.
pub const MAX_DELAY_MS: f32 = 4000.0;
pub const MAX_SHIFT: f32 = 24.0;
/// Level at or under which a head is off.
pub const OFF_DB: f32 = -60.0;
/// A window this wide (octaves) hears everything.
pub const FULL_WIDTH: f32 = 10.0;
pub const MIN_WIDTH: f32 = 0.25;
pub const MAX_FEEDBACK: f32 = 0.95;
/// A gliding head bounces between plus and minus this many semitones.
const GLIDE_LIMIT: f32 = 12.0;

/// Display bands (log spaced) and scrolling spectrogram columns shared with the editor.
pub const BANDS: usize = 64;
pub const COLS: usize = 128;
pub const F_MIN: f32 = 30.0;
pub const F_MAX: f32 = 16000.0;

#[derive(Clone, Copy)]
pub struct HeadSettings {
    pub delay_ms: f32,
    /// Pitch offset in semitones.
    pub shift: f32,
    /// Glide in semitones per second, added on top of the offset and bouncing at +-12.
    pub slide: f32,
    pub centre_hz: f32,
    pub width_oct: f32,
    pub level_db: f32,
    pub pan: f32,
}

impl Default for HeadSettings {
    fn default() -> Self {
        Self { delay_ms: 400.0, shift: 0.0, slide: 0.0, centre_hz: 1000.0, width_oct: FULL_WIDTH, level_db: OFF_DB, pan: 0.0 }
    }
}

#[derive(Clone, Copy)]
pub struct Settings {
    pub heads: [HeadSettings; HEADS],
    pub feedback: f32,
    /// Time constant for head parameter changes.
    pub smooth_ms: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self { heads: [HeadSettings::default(); HEADS], feedback: 0.0, smooth_ms: 100.0 }
    }
}

pub fn level_gain(db: f32) -> f32 {
    if db <= OFF_DB + 0.05 {
        0.0
    } else {
        10f32.powf(db / 20.0)
    }
}

/// Equal-power balance; both gains are 1 at the centre.
pub fn pan_gains(pan: f32) -> (f32, f32) {
    let a = (pan.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
    (a.cos() * std::f32::consts::SQRT_2, a.sin() * std::f32::consts::SQRT_2)
}

/// Gain of a head's band-pass window at `distance` octaves from its centre: flat over the middle
/// 60% of the width, then a smooth fall to nothing at the edge.
pub fn window_gain(distance: f32, width: f32) -> f32 {
    if width >= FULL_WIDTH - 1e-3 {
        return 1.0;
    }
    let half = width * 0.5;
    let t = ((distance.abs() - half * 0.6) / (half * 0.4)).clamp(0.0, 1.0);
    1.0 - t * t * (3.0 - 2.0 * t)
}

pub fn mix(wet: f32, dry: f32, amount: f32) -> f32 {
    dry + (wet - dry) * amount
}

fn soft(x: f32) -> f32 {
    let x = x.clamp(-3.0, 3.0);
    x * (27.0 + x * x) / (27.0 + 9.0 * x * x)
}

fn wrap(x: f32) -> f32 {
    x - TAU * (x / TAU).round()
}

/// Everything the editor draws, written by the audio thread.
pub struct Meter {
    /// Spectrogram columns of input level in dB (full-scale sine = 0), `col * BANDS + band`.
    pub spec: Vec<AtomicF32>,
    /// Columns completed so far; the newest is `(cursor - 1) % COLS`.
    pub cursor: AtomicUsize,
    /// How far into the next column the audio is, 0 to 1, for smooth scrolling.
    pub progress: AtomicF32,
    /// Seconds per column.
    pub col_seconds: AtomicF32,
    /// Output spectrum in dB per band.
    pub out: [AtomicF32; BANDS],
    /// Each head's output level in dB (-120 when off).
    pub head_level: [AtomicF32; HEADS],
    /// Each head's current glide in semitones.
    pub glide: [AtomicF32; HEADS],
}

impl Default for Meter {
    fn default() -> Self {
        Self {
            spec: (0..COLS * BANDS).map(|_| AtomicF32::new(-120.0)).collect(),
            cursor: AtomicUsize::new(0),
            progress: AtomicF32::new(0.0),
            col_seconds: AtomicF32::new(MAX_DELAY_MS / 1000.0 / COLS as f32),
            out: std::array::from_fn(|_| AtomicF32::new(-120.0)),
            head_level: std::array::from_fn(|_| AtomicF32::new(-120.0)),
            glide: std::array::from_fn(|_| AtomicF32::new(0.0)),
        }
    }
}

struct Head {
    active: bool,
    fresh: bool,
    /// Smoothed values: delay in frames, pitch and glide in semitones, centre in log2 Hz.
    delay: f32,
    shift: f32,
    centre: f32,
    width: f32,
    gain: f32,
    pan: f32,
    glide: f32,
    dir: f32,
    /// Accumulated output phase per channel and bin.
    phase: [Vec<f32>; MAX_CHANNELS],
    /// Output magnitude of the last frame, to find which peak continues which.
    pmag: [Vec<f32>; MAX_CHANNELS],
    window: Vec<f32>,
}

impl Head {
    fn new() -> Self {
        Self {
            active: false,
            fresh: true,
            delay: 0.0,
            shift: 0.0,
            centre: 10.0,
            width: FULL_WIDTH,
            gain: 0.0,
            pan: 0.0,
            glide: 0.0,
            dir: 1.0,
            phase: std::array::from_fn(|_| vec![0.0; NB]),
            pmag: std::array::from_fn(|_| vec![0.0; NB]),
            window: vec![1.0; NB],
        }
    }
}

fn per_channel(len: usize) -> [Vec<f32>; MAX_CHANNELS] {
    std::array::from_fn(|_| vec![0.0; len])
}

pub struct Loom {
    fs: f32,
    channels: usize,
    pub settings: Settings,
    fft: Fft,
    window: Vec<f32>,
    /// Converts a windowed FFT magnitude into the amplitude of the sine that produced it.
    norm: f32,
    bin_oct: Vec<f32>,
    /// Input plus feedback, and the plain input for the latency-matched dry path.
    input: [Vec<f32>; MAX_CHANNELS],
    dry: [Vec<f32>; MAX_CHANNELS],
    pos: usize,
    since: usize,
    acc: [Vec<f32>; MAX_CHANNELS],
    out: [[f32; HOP]; MAX_CHANNELS],
    out_pos: usize,
    re: Vec<f32>,
    im: Vec<f32>,
    sum_re: [Vec<f32>; MAX_CHANNELS],
    sum_im: [Vec<f32>; MAX_CHANNELS],
    /// History ring: `fr` frames of `NB` bins per channel.
    fr: usize,
    frames: usize,
    mag: [Vec<f32>; MAX_CHANNELS],
    dev: [Vec<f32>; MAX_CHANNELS],
    ph: [Vec<f32>; MAX_CHANNELS],
    zeros: Vec<f32>,
    scratch_mag: Vec<f32>,
    scratch_phase: Vec<f32>,
    scratch_out_mag: Vec<f32>,
    scratch_peaks: Vec<usize>,
    heads: Vec<Head>,
    ranges: [(usize, usize); BANDS],
    pending: [f32; BANDS],
    hops_in_col: usize,
    hops_per_col: usize,
    pub meter: Arc<Meter>,
}

impl Default for Loom {
    fn default() -> Self {
        Self::new()
    }
}

impl Loom {
    pub fn new() -> Self {
        let window = sqrt_hann(FFT_SIZE);
        let norm = window.iter().sum::<f32>() / 2.0;
        let mut this = Self {
            fs: 48000.0,
            channels: 2,
            settings: Settings::default(),
            fft: Fft::new(FFT_SIZE),
            window,
            norm,
            bin_oct: (0..NB).map(|_| 0.0).collect(),
            input: per_channel(FFT_SIZE),
            dry: per_channel(FFT_SIZE),
            pos: 0,
            since: 0,
            acc: per_channel(FFT_SIZE),
            out: [[0.0; HOP]; MAX_CHANNELS],
            out_pos: 0,
            re: vec![0.0; FFT_SIZE],
            im: vec![0.0; FFT_SIZE],
            sum_re: per_channel(NB),
            sum_im: per_channel(NB),
            fr: 0,
            frames: 0,
            mag: per_channel(0),
            dev: per_channel(0),
            ph: per_channel(0),
            zeros: vec![0.0; NB],
            scratch_mag: vec![0.0; NB],
            scratch_phase: vec![0.0; NB],
            scratch_out_mag: vec![0.0; NB],
            scratch_peaks: vec![0; NB],
            heads: (0..HEADS).map(|_| Head::new()).collect(),
            ranges: [(1, 1); BANDS],
            pending: [-120.0; BANDS],
            hops_in_col: 0,
            hops_per_col: 3,
            meter: Arc::new(Meter::default()),
        };
        this.configure(48000.0, 2);
        this
    }

    /// Allocates the history for this sample rate. Call from `initialize`, never from `process`.
    pub fn configure(&mut self, fs: f32, channels: usize) {
        self.fs = fs;
        self.channels = channels.clamp(1, MAX_CHANNELS);
        let bin_hz = fs / FFT_SIZE as f32;
        for (k, o) in self.bin_oct.iter_mut().enumerate() {
            *o = (k.max(1) as f32 * bin_hz).log2();
        }
        for (i, range) in self.ranges.iter_mut().enumerate() {
            let edge = |k: usize| F_MIN * (F_MAX / F_MIN).powf(k as f32 / BANDS as f32);
            let lo = ((edge(i) / bin_hz).floor() as usize).clamp(1, BINS);
            let hi = ((edge(i + 1) / bin_hz).ceil() as usize).clamp(lo, BINS);
            *range = (lo, hi);
        }
        let frames_for_max = (MAX_DELAY_MS * 0.001 * fs / HOP as f32).ceil() as usize;
        self.fr = frames_for_max + 4;
        for ch in 0..MAX_CHANNELS {
            self.mag[ch] = vec![0.0; self.fr * NB];
            self.dev[ch] = vec![0.0; self.fr * NB];
            self.ph[ch] = vec![0.0; self.fr * NB];
        }
        self.hops_per_col = (frames_for_max as f32 / COLS as f32).ceil().max(1.0) as usize;
        self.meter.col_seconds.store(self.hops_per_col as f32 * HOP as f32 / fs, Ordering::Relaxed);
        self.reset();
    }

    pub fn reset(&mut self) {
        for ch in 0..MAX_CHANNELS {
            self.input[ch].fill(0.0);
            self.dry[ch].fill(0.0);
            self.acc[ch].fill(0.0);
            self.out[ch].fill(0.0);
            self.mag[ch].fill(0.0);
            self.dev[ch].fill(0.0);
            self.ph[ch].fill(0.0);
        }
        for h in self.heads.iter_mut() {
            h.active = false;
            h.fresh = true;
            h.glide = 0.0;
            h.dir = 1.0;
        }
        self.pos = 0;
        self.since = 0;
        self.out_pos = 0;
        self.frames = 0;
        self.hops_in_col = 0;
        self.pending = [-120.0; BANDS];
        self.meter.cursor.store(0, Ordering::Relaxed);
        for a in self.meter.spec.iter() {
            a.store(-120.0, Ordering::Relaxed);
        }
    }

    /// Returns `(wet, dry)` for one frame of samples; both are delayed by `LATENCY`.
    pub fn process(&mut self, x: [f32; MAX_CHANNELS]) -> ([f32; MAX_CHANNELS], [f32; MAX_CHANNELS]) {
        let (mut wet, mut dry) = ([0.0; MAX_CHANNELS], [0.0; MAX_CHANNELS]);
        let fb = self.settings.feedback.clamp(0.0, MAX_FEEDBACK);
        for ch in 0..self.channels {
            // Before it is overwritten, this slot holds the sample from exactly `FFT_SIZE` ago.
            dry[ch] = self.dry[ch][self.pos];
            self.dry[ch][self.pos] = x[ch];
            wet[ch] = self.out[ch][self.out_pos];
            // The output goes back into the history through a soft clipper, so the loop is bounded.
            self.input[ch][self.pos] = x[ch] + fb * soft(wet[ch]);
        }
        self.out_pos += 1;
        self.pos = (self.pos + 1) % FFT_SIZE;
        self.since += 1;
        if self.since == HOP {
            self.since = 0;
            self.frame();
            self.out_pos = 0;
        }
        (wet, dry)
    }

    fn frame(&mut self) {
        self.analyse();
        self.synthesise();
        self.frames += 1;

        self.hops_in_col += 1;
        if self.hops_in_col >= self.hops_per_col {
            self.hops_in_col = 0;
            let col = self.meter.cursor.load(Ordering::Relaxed);
            for (b, p) in self.pending.iter_mut().enumerate() {
                self.meter.spec[(col % COLS) * BANDS + b].store(*p, Ordering::Relaxed);
                *p = -120.0;
            }
            self.meter.cursor.store(col + 1, Ordering::Relaxed);
        }
        self.meter.progress.store(self.hops_in_col as f32 / self.hops_per_col as f32, Ordering::Relaxed);
    }

    /// Transform the newest frame and file it in the history.
    fn analyse(&mut self) {
        let t = self.frames;
        let slot = (t % self.fr) * NB;
        let prev = if t > 0 { Some(((t - 1) % self.fr) * NB) } else { None };
        let mut band_max = [0.0f32; BANDS];
        for ch in 0..self.channels {
            for i in 0..FFT_SIZE {
                self.re[i] = self.input[ch][(self.pos + i) % FFT_SIZE] * self.window[i];
                self.im[i] = 0.0;
            }
            self.fft.forward(&mut self.re, &mut self.im);
            for k in 0..=BINS {
                let m = self.re[k].hypot(self.im[k]);
                let p = self.im[k].atan2(self.re[k]);
                // A steady tone at the bin centre advances k * pi/2 per hop (mod 2 pi).
                self.dev[ch][slot + k] = match prev {
                    Some(pb) => wrap(p - self.ph[ch][pb + k] - (k & 3) as f32 * FRAC_PI_2),
                    None => 0.0,
                };
                self.mag[ch][slot + k] = m;
                self.ph[ch][slot + k] = p;
            }
            for (b, &(lo, hi)) in self.ranges.iter().enumerate() {
                for k in lo..=hi {
                    band_max[b] = band_max[b].max(self.mag[ch][slot + k]);
                }
            }
        }
        for (b, m) in band_max.iter().enumerate() {
            let db = 20.0 * (m / self.norm).max(1e-6).log10();
            self.pending[b] = self.pending[b].max(db);
        }
    }

    fn synthesise(&mut self) {
        let s = self.settings;
        let c = self.channels;
        let t = self.frames;
        let hop_s = HOP as f32 / self.fs;
        let coef = if s.smooth_ms <= 0.0 { 1.0 } else { 1.0 - (-hop_s / (s.smooth_ms * 0.001)).exp() };
        let max_delay = (self.fr - 3) as f32;
        for ch in 0..c {
            self.sum_re[ch].fill(0.0);
            self.sum_im[ch].fill(0.0);
        }

        for h in 0..HEADS {
            let hs = s.heads[h];
            let target_gain = level_gain(hs.level_db);
            let head = &mut self.heads[h];
            if target_gain <= 1e-5 {
                head.active = false;
                head.fresh = true;
                self.meter.head_level[h].store(-120.0, Ordering::Relaxed);
                self.meter.glide[h].store(0.0, Ordering::Relaxed);
                continue;
            }
            let target_delay = (hs.delay_ms * 0.001 * self.fs / HOP as f32).clamp(0.0, max_delay);
            let target_centre = hs.centre_hz.max(10.0).log2();
            let target_shift = hs.shift.clamp(-MAX_SHIFT, MAX_SHIFT);
            let target_width = hs.width_oct.clamp(MIN_WIDTH, FULL_WIDTH);
            if !head.active {
                head.active = true;
                head.fresh = true;
                head.delay = target_delay;
                head.shift = target_shift;
                head.centre = target_centre;
                head.width = target_width;
                head.gain = target_gain;
                head.pan = hs.pan;
                head.glide = 0.0;
                head.dir = 1.0;
            } else {
                head.delay += (target_delay - head.delay) * coef;
                head.shift += (target_shift - head.shift) * coef;
                head.centre += (target_centre - head.centre) * coef;
                head.width += (target_width - head.width) * coef;
                head.gain += (target_gain - head.gain) * coef;
                head.pan += (hs.pan - head.pan) * coef;
            }
            // The glide bounces between the limits; with no slide it relaxes back to zero.
            if hs.slide.abs() < 1e-3 {
                let step = 6.0 * hop_s;
                head.glide -= head.glide.clamp(-step, step);
            } else {
                head.glide += hs.slide * head.dir * hop_s;
                if head.glide > GLIDE_LIMIT {
                    head.glide = 2.0 * GLIDE_LIMIT - head.glide;
                    head.dir = -head.dir;
                } else if head.glide < -GLIDE_LIMIT {
                    head.glide = -2.0 * GLIDE_LIMIT - head.glide;
                    head.dir = -head.dir;
                }
                head.glide = head.glide.clamp(-GLIDE_LIMIT, GLIDE_LIMIT);
            }
            self.meter.glide[h].store(head.glide, Ordering::Relaxed);

            let ratio = ((head.shift + head.glide).clamp(-36.0, 36.0) / 12.0).exp2();
            let unity = ratio == 1.0;
            for k in 0..NB {
                head.window[k] = window_gain(self.bin_oct[k] - head.centre, head.width);
            }

            // Which two history frames the delay falls between, and how much of the newer one.
            let dc = head.delay.ceil();
            let w_new = dc - head.delay;
            let a = t as isize - dc as isize;
            let b = a + 1;
            let slot = |i: isize| if i >= 0 && i <= t as isize { Some((i as usize % self.fr) * NB) } else { None };
            let (sa, sb) = (slot(a), slot(b));
            let nearest = if w_new > 0.5 { sb.or(sa) } else { sa.or(sb) };

            let (gl, gr) = if c == 2 { pan_gains(head.pan) } else { (1.0, 1.0) };
            let mut energy = 0.0f32;
            for ch in 0..c {
                let g = head.gain * if ch == 0 { gl } else { gr };
                let mag_of = |sl: Option<usize>| match sl {
                    Some(o) => &self.mag[ch][o..o + NB],
                    None => &self.zeros[..],
                };
                let (ma, mb) = (mag_of(sa), mag_of(sb));
                let (dv, ph) = match nearest {
                    Some(o) => (&self.dev[ch][o..o + NB], &self.ph[ch][o..o + NB]),
                    None => (&self.zeros[..], &self.zeros[..]),
                };
                let (wa, wb) = (1.0 - w_new, w_new);
                let (sre, sim) = (&mut self.sum_re[ch], &mut self.sum_im[ch]);
                let mg = &mut self.scratch_mag;
                let mut maxm = 0.0f32;
                let mut e = 0.0f32;
                for i in 0..NB {
                    mg[i] = (ma[i] * wa + mb[i] * wb) * head.window[i];
                    maxm = maxm.max(mg[i]);
                    e += mg[i] * mg[i];
                }
                energy = energy.max(e * head.gain * head.gain);
                let (phase, pmag) = (&mut head.phase[ch], &mut head.pmag[ch]);

                if unity {
                    // No offset: every bin continues its own phase, which reproduces the source
                    // frame's phases exactly, so the head is a plain delay.
                    for k in 0..NB {
                        if head.fresh {
                            phase[k] = ph[k];
                        } else {
                            phase[k] = wrap(phase[k] + ((k & 3) as f32 * FRAC_PI_2 + dv[k]));
                        }
                        pmag[k] = mg[k];
                        let (sn, cs) = phase[k].sin_cos();
                        sre[k] += g * mg[k] * cs;
                        sim[k] += g * mg[k] * sn;
                    }
                    continue;
                }

                // Shifted: find the spectral peaks and move each one's whole region of influence
                // by the same whole number of bins, so a partial keeps its lobe shape. The peak
                // continues the phase of the nearest peak of the last frame, advanced by its own
                // instantaneous frequency times the ratio, and the other bins keep their phase
                // relative to their peak (identity phase locking).
                let threshold = (maxm * 1e-4).max(1e-7);
                let peaks = &mut self.scratch_peaks;
                let mut np = 0;
                for i in 1..BINS {
                    let m = mg[i];
                    if m > threshold && m > mg[i - 1] && m >= mg[i + 1] && (i < 2 || m >= mg[i - 2]) && (i + 2 > BINS || m >= mg[i + 2]) {
                        peaks[np] = i;
                        np += 1;
                    }
                }
                let (new_phase, new_mag) = (&mut self.scratch_phase, &mut self.scratch_out_mag);
                new_phase.fill(0.0);
                new_mag.fill(0.0);
                let mut lo = 0;
                for j in 0..np {
                    let p = peaks[j];
                    let hi = if j + 1 < np {
                        let mut at = p;
                        for k in p..=peaks[j + 1] {
                            if mg[k] < mg[at] || at == p {
                                at = k;
                            }
                        }
                        at.max(p + 1)
                    } else {
                        NB
                    };
                    let target = (p as f32 * ratio).round() as isize;
                    let shift = target - p as isize;
                    let start = if head.fresh {
                        ph[p]
                    } else {
                        let mut q = target.clamp(0, BINS as isize) as usize;
                        for c in target.max(1) - 1..=(target + 1).min(BINS as isize) {
                            if pmag[c as usize] > pmag[q] {
                                q = c as usize;
                            }
                        }
                        phase[q] + ratio * (p as f32 * FRAC_PI_2 + dv[p])
                    };
                    let start = wrap(start);
                    for k in lo..hi {
                        let kp = k as isize + shift;
                        if kp < 0 || kp > BINS as isize {
                            continue;
                        }
                        let kp = kp as usize;
                        let theta = wrap(start + ph[k] - ph[p]);
                        new_phase[kp] = theta;
                        new_mag[kp] = mg[k];
                        let (sn, cs) = theta.sin_cos();
                        sre[kp] += g * mg[k] * cs;
                        sim[kp] += g * mg[k] * sn;
                    }
                    lo = hi;
                }
                phase.copy_from_slice(new_phase);
                pmag.copy_from_slice(new_mag);
            }
            // Until a real frame has been read there is no phase to continue from.
            head.fresh = nearest.is_none();
            let amp2 = 8.0 * energy / (FFT_SIZE * FFT_SIZE) as f32;
            self.meter.head_level[h].store(10.0 * amp2.max(1e-12).log10(), Ordering::Relaxed);
        }

        let mut band_max = [0.0f32; BANDS];
        for ch in 0..c {
            for k in 0..=BINS {
                self.re[k] = self.sum_re[ch][k];
                self.im[k] = self.sum_im[ch][k];
            }
            for (b, &(lo, hi)) in self.ranges.iter().enumerate() {
                for k in lo..=hi {
                    band_max[b] = band_max[b].max(self.re[k].hypot(self.im[k]));
                }
            }
            self.im[0] = 0.0;
            self.im[BINS] = 0.0;
            for k in 1..BINS {
                self.re[FFT_SIZE - k] = self.re[k];
                self.im[FFT_SIZE - k] = -self.im[k];
            }
            self.fft.inverse(&mut self.re, &mut self.im);
            for i in 0..FFT_SIZE {
                // sqrt-Hann twice is Hann, which sums to 2 at this overlap.
                self.acc[ch][i] += self.re[i] * self.window[i] * 0.5;
            }
            self.out[ch].copy_from_slice(&self.acc[ch][..HOP]);
            self.acc[ch].copy_within(HOP.., 0);
            self.acc[ch][FFT_SIZE - HOP..].fill(0.0);
        }
        for (b, m) in band_max.iter().enumerate() {
            let db = 20.0 * (m / self.norm).max(1e-6).log10();
            self.meter.out[b].store(db, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests;
