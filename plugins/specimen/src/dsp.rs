//! Spectral freeze and morph.
//!
//! A 4x-overlap STFT (sqrt-Hann on both sides) with four snapshot slots plus a hidden hold slot.
//! A capture averages the input's magnitude spectrum over a few frames and stores its log
//! magnitude, plus the instantaneous frequency and phase of every bin from the last frame. The
//! output spectrum is a morph between slots (log magnitude interpolated, then optionally stretched
//! and shifted along the frequency axis), applied one of two ways:
//!
//! * **Cross**: the snapshot's spectral envelope is imposed on the input. Both envelopes are
//!   smoothed across bins, the input's bins are scaled by their ratio and the frame is
//!   renormalised to the input's own energy, so the input keeps its phase, fine structure and level.
//! * **Resynth**: the morphed magnitude is played back with its own phase, advanced every hop by
//!   the stored instantaneous frequency of each bin (a phase vocoder), so a frozen sine stays a
//!   steady sine instead of buzzing.
//!
//! Everything is allocated in `configure`; the audio thread only touches preallocated buffers and
//! atomics. The editor requests captures through [`Meter`] flags.

use nih_plug::prelude::AtomicF32;
use spectral::{sqrt_hann, Fft};
use std::f32::consts::TAU;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub const MAX_CHANNELS: usize = 2;
pub const SLOTS: usize = 4;
/// Resolution of the spectra shared with the editor.
pub const BANDS: usize = 96;
pub const F_LO: f32 = 30.0;
pub const F_HI: f32 = 18000.0;

/// Index of the hidden slot Freeze writes the live frame into.
const HOLD: usize = SLOTS;
const ALL: usize = SLOTS + 1;
const CAPTURE_FRAMES: usize = 8;
const EPS: f32 = 1e-9;
const NO_DOMINANT: usize = usize::MAX;

/// FFT length for a sample rate: 2048 up to 70 kHz, 4096 above.
pub fn fft_len(sample_rate: f32) -> usize {
    if sample_rate > 70000.0 {
        4096
    } else {
        2048
    }
}

/// Latency in samples, one FFT length.
pub fn latency(sample_rate: f32) -> usize {
    fft_len(sample_rate)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Evolve {
    Off,
    Sine,
    Walk,
}

#[derive(Clone, Copy, Debug)]
pub struct Settings {
    /// 0..1 along the four slides.
    pub position: f32,
    /// 0 is a linear blend between neighbouring slides, 1 dwells on each slide.
    pub curve: f32,
    /// 0 morphs log magnitude (timbres slide into each other, but slides with nothing in common
    /// dip towards silence halfway), 1 cross-fades linear magnitude.
    pub blend: f32,
    pub resynth: bool,
    /// Frequency-axis stretch in octaves, -1..1.
    pub scale_oct: f32,
    /// Frequency-axis shift in Hz.
    pub shift_hz: f32,
    /// 0..1, how finely the spectral envelope is resolved in Cross mode.
    pub detail: f32,
    pub evolve: Evolve,
    pub rate_hz: f32,
    pub depth: f32,
    pub glide_ms: f32,
    pub freeze: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            position: 0.0,
            curve: 0.0,
            blend: 0.5,
            resynth: false,
            scale_oct: 0.0,
            shift_hz: 0.0,
            detail: 0.5,
            evolve: Evolve::Off,
            rate_hz: 0.25,
            depth: 0.5,
            glide_ms: 120.0,
            freeze: false,
        }
    }
}

/// State shared with the editor. The editor sets the request flags; the audio thread clears them
/// and writes the spectra (in dB, per log-spaced band).
pub struct Meter {
    pub input: [AtomicF32; BANDS],
    pub output: [AtomicF32; BANDS],
    pub slots: [[AtomicF32; BANDS]; SLOTS],
    pub filled: [AtomicBool; SLOTS],
    pub capture: [AtomicBool; SLOTS],
    pub clear: [AtomicBool; SLOTS],
    /// 0..1 while a capture is running.
    pub progress: [AtomicF32; SLOTS],
    /// The position actually in use: base position plus evolve, after glide.
    pub position: AtomicF32,
    pub holding: AtomicBool,
}

impl Meter {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            input: std::array::from_fn(|_| AtomicF32::new(-120.0)),
            output: std::array::from_fn(|_| AtomicF32::new(-120.0)),
            slots: std::array::from_fn(|_| std::array::from_fn(|_| AtomicF32::new(-120.0))),
            filled: std::array::from_fn(|_| AtomicBool::new(false)),
            capture: std::array::from_fn(|_| AtomicBool::new(false)),
            clear: std::array::from_fn(|_| AtomicBool::new(false)),
            progress: std::array::from_fn(|_| AtomicF32::new(0.0)),
            position: AtomicF32::new(0.0),
            holding: AtomicBool::new(false),
        })
    }

    pub fn request_capture(&self, slot: usize) {
        self.capture[slot].store(true, Ordering::Relaxed);
    }

    pub fn request_clear(&self, slot: usize) {
        self.clear[slot].store(true, Ordering::Relaxed);
    }
}

/// Which two slots to blend, and how far from the first to the second (already curved).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plan {
    pub a: usize,
    pub b: usize,
    pub t: f32,
}

/// Blend plan for `position` (0..1 over the four slide positions) across the filled slides only,
/// holding the nearest one beyond the ends. `None` if nothing is filled.
pub fn plan(filled: [bool; SLOTS], position: f32, curve: f32) -> Option<Plan> {
    let mut idx = [0usize; SLOTS];
    let mut count = 0;
    for (i, f) in filled.iter().enumerate() {
        if *f {
            idx[count] = i;
            count += 1;
        }
    }
    if count == 0 {
        return None;
    }
    let p = position.clamp(0.0, 1.0) * (SLOTS - 1) as f32;
    let (first, last) = (idx[0], idx[count - 1]);
    if p <= first as f32 {
        return Some(Plan { a: first, b: first, t: 0.0 });
    }
    if p >= last as f32 {
        return Some(Plan { a: last, b: last, t: 0.0 });
    }
    for j in 0..count - 1 {
        let (lo, hi) = (idx[j], idx[j + 1]);
        if p >= lo as f32 && p < hi as f32 {
            let t = shape((p - lo as f32) / (hi - lo) as f32, curve);
            if t <= 0.0 {
                return Some(Plan { a: lo, b: lo, t: 0.0 });
            }
            if t >= 1.0 {
                return Some(Plan { a: hi, b: hi, t: 0.0 });
            }
            return Some(Plan { a: lo, b: hi, t });
        }
    }
    Some(Plan { a: last, b: last, t: 0.0 })
}

/// Linear at curve 0, a double smoothstep (long dwell at each slide, quick crossing) at 1.
fn shape(t: f32, curve: f32) -> f32 {
    let s = t * t * (3.0 - 2.0 * t);
    let s2 = s * s * (3.0 - 2.0 * s);
    t + (s2 - t) * curve.clamp(0.0, 1.0)
}

struct Slot {
    lmag: Vec<f32>,
    omega: Vec<f32>,
    phase: Vec<f32>,
}

impl Slot {
    fn new(bins: usize) -> Self {
        Self { lmag: vec![EPS.ln(); bins], omega: vec![0.0; bins], phase: vec![0.0; bins] }
    }
}

struct Channel {
    in_buf: Vec<f32>,
    out_acc: Vec<f32>,
    dry: Vec<f32>,
    prev_phase: Vec<f32>,
    syn_phase: Vec<f32>,
    dominant: Vec<usize>,
    acc: Vec<f32>,
    slots: Vec<Slot>,
}

impl Channel {
    fn new(n: usize) -> Self {
        let bins = n / 2 + 1;
        Self {
            in_buf: vec![0.0; n],
            out_acc: vec![0.0; n],
            dry: vec![0.0; n],
            prev_phase: vec![0.0; bins],
            syn_phase: vec![0.0; bins],
            dominant: vec![NO_DOMINANT; bins],
            acc: vec![0.0; bins],
            slots: (0..ALL).map(|_| Slot::new(bins)).collect(),
        }
    }
}

struct Scratch {
    re: Vec<f32>,
    im: Vec<f32>,
    mag: Vec<f32>,
    phase: Vec<f32>,
    omega: Vec<f32>,
    tgt: Vec<f32>,
    tgt_env: Vec<f32>,
    in_log: Vec<f32>,
    in_env: Vec<f32>,
    out_mag: Vec<f32>,
    prefix: Vec<f32>,
    db: Vec<f32>,
    locked: Vec<f32>,
    left: Vec<usize>,
    right: Vec<usize>,
}

impl Scratch {
    fn new(n: usize) -> Self {
        let bins = n / 2 + 1;
        let v = |len: usize| vec![0.0f32; len];
        Self {
            re: v(n),
            im: v(n),
            mag: v(bins),
            phase: v(bins),
            omega: v(bins),
            tgt: v(bins),
            tgt_env: v(bins),
            in_log: v(bins),
            in_env: v(bins),
            out_mag: v(bins),
            prefix: v(bins + 1),
            db: v(bins),
            locked: v(bins),
            left: vec![0; bins],
            right: vec![0; bins],
        }
    }
}

/// What the current frame is doing, decided once for all channels.
struct FrameJob {
    plan: Option<Plan>,
    resynth: bool,
    scale: f32,
    shift_bins: f32,
    blend: f32,
    env_half_width: usize,
    capture: Option<usize>,
    capture_last: bool,
    hold_now: bool,
}

pub struct Engine {
    pub settings: Settings,
    pub meter: Arc<Meter>,
    fs: f32,
    n: usize,
    hop: usize,
    bins: usize,
    channels: usize,
    fft: Fft,
    window: Vec<f32>,
    ch: Vec<Channel>,
    scratch: Scratch,
    band_ranges: Vec<(usize, usize, f32)>,
    ref_db: f32,
    pos: usize,
    count: usize,
    // Frame-rate state.
    filled: [bool; ALL],
    capture: Option<usize>,
    capture_left: usize,
    frozen: bool,
    primed: bool,
    pos_s: f32,
    scale_s: f32,
    shift_s: f32,
    lfo: f32,
    walk: f32,
    rng: u32,
}

impl Engine {
    pub fn new() -> Self {
        let mut e = Self {
            settings: Settings::default(),
            meter: Meter::new(),
            fs: 48000.0,
            n: 0,
            hop: 0,
            bins: 0,
            channels: 0,
            fft: Fft::new(2),
            window: Vec::new(),
            ch: Vec::new(),
            scratch: Scratch::new(2),
            band_ranges: Vec::new(),
            ref_db: 0.0,
            pos: 0,
            count: 0,
            filled: [false; ALL],
            capture: None,
            capture_left: 0,
            frozen: false,
            primed: false,
            pos_s: 0.0,
            scale_s: 0.0,
            shift_s: 0.0,
            lfo: 0.0,
            walk: 0.0,
            rng: 0x2545_f491,
        };
        e.configure(48000.0, 2);
        e
    }

    /// Allocates everything. Captured slides are lost.
    pub fn configure(&mut self, sample_rate: f32, channels: usize) {
        let n = fft_len(sample_rate);
        self.fs = sample_rate;
        self.n = n;
        self.hop = n / 4;
        self.bins = n / 2 + 1;
        self.channels = channels.clamp(1, MAX_CHANNELS);
        self.fft = Fft::new(n);
        self.window = sqrt_hann(n);
        self.ch = (0..self.channels).map(|_| Channel::new(n)).collect();
        self.scratch = Scratch::new(n);
        // A full-scale sine peaks at amplitude/2 * sum(window) = 0.318 n.
        self.ref_db = 20.0 * (0.3183 * n as f32).log10();
        let bin_hz = sample_rate / n as f32;
        let ratio = F_HI / F_LO;
        self.band_ranges = (0..BANDS)
            .map(|b| {
                let f0 = F_LO * ratio.powf(b as f32 / BANDS as f32);
                let f1 = F_LO * ratio.powf((b + 1) as f32 / BANDS as f32);
                let fc = (f0 * f1).sqrt();
                let max = self.bins - 1;
                let lo = ((f0 / bin_hz).floor() as usize).min(max);
                let hi = ((f1 / bin_hz).ceil() as usize).clamp(lo, max);
                (lo, hi, (fc / bin_hz).min(max as f32))
            })
            .collect();
        self.reset();
        self.filled = [false; ALL];
        for s in 0..SLOTS {
            self.meter.filled[s].store(false, Ordering::Relaxed);
            self.meter.progress[s].store(0.0, Ordering::Relaxed);
            for b in 0..BANDS {
                self.meter.slots[s][b].store(-120.0, Ordering::Relaxed);
            }
        }
    }

    /// Clears audio history but keeps the captured slides.
    pub fn reset(&mut self) {
        for ch in &mut self.ch {
            ch.in_buf.fill(0.0);
            ch.out_acc.fill(0.0);
            ch.dry.fill(0.0);
            ch.prev_phase.fill(0.0);
            ch.dominant.fill(NO_DOMINANT);
        }
        self.pos = 0;
        self.count = 0;
        self.primed = false;
        self.capture = None;
        self.frozen = false;
        self.filled[HOLD] = false;
        self.meter.holding.store(false, Ordering::Relaxed);
    }

    pub fn latency(&self) -> usize {
        self.n
    }

    pub fn fft_len(&self) -> usize {
        self.n
    }

    pub fn sample_rate(&self) -> f32 {
        self.fs
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    pub fn is_filled(&self, slot: usize) -> bool {
        self.filled[slot]
    }

    /// One sample in, one sample out. `mix` 0 gives exactly the dry input delayed by the latency.
    pub fn process(&mut self, x: [f32; MAX_CHANNELS], mix: f32) -> [f32; MAX_CHANNELS] {
        let p = self.pos;
        let mut out = [0.0; MAX_CHANNELS];
        for c in 0..self.channels {
            let ch = &mut self.ch[c];
            ch.in_buf[p] = x[c];
            let wet = ch.out_acc[p];
            ch.out_acc[p] = 0.0;
            let dry = ch.dry[p];
            ch.dry[p] = x[c];
            out[c] = dry + (wet - dry) * mix;
        }
        self.pos = (p + 1) % self.n;
        self.count += 1;
        if self.count == self.hop {
            self.count = 0;
            self.frame();
        }
        out
    }

    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng as f32 / u32::MAX as f32 * 2.0 - 1.0
    }

    fn frame(&mut self) {
        let s = self.settings;
        let dt = self.hop as f32 / self.fs;

        // Requests from the editor.
        for slot in 0..SLOTS {
            if self.meter.clear[slot].swap(false, Ordering::Relaxed) {
                self.filled[slot] = false;
                self.meter.filled[slot].store(false, Ordering::Relaxed);
                for b in 0..BANDS {
                    self.meter.slots[slot][b].store(-120.0, Ordering::Relaxed);
                }
                if self.capture == Some(slot) {
                    self.capture = None;
                    self.meter.progress[slot].store(0.0, Ordering::Relaxed);
                }
            }
            if self.meter.capture[slot].swap(false, Ordering::Relaxed) {
                if let Some(old) = self.capture {
                    self.meter.progress[old].store(0.0, Ordering::Relaxed);
                }
                self.capture = Some(slot);
                self.capture_left = CAPTURE_FRAMES;
                for ch in &mut self.ch {
                    ch.acc.fill(0.0);
                }
            }
        }
        let hold_now = s.freeze && !self.frozen;
        if hold_now {
            self.frozen = true;
        } else if !s.freeze && self.frozen {
            self.frozen = false;
            self.filled[HOLD] = false;
        }
        self.meter.holding.store(self.frozen, Ordering::Relaxed);

        // Position: evolve, then glide.
        let mut target = s.position;
        match s.evolve {
            Evolve::Off => {}
            Evolve::Sine => {
                self.lfo = (self.lfo + s.rate_hz * dt).fract();
                target += s.depth * 0.5 * (TAU * self.lfo).sin();
            }
            Evolve::Walk => {
                let step = (self.random() + self.random()) * 1.5 * s.rate_hz.sqrt() * dt.sqrt();
                self.walk += step;
                if self.walk > 1.0 {
                    self.walk = 2.0 - self.walk;
                }
                if self.walk < -1.0 {
                    self.walk = -2.0 - self.walk;
                }
                self.walk = self.walk.clamp(-1.0, 1.0);
                target += s.depth * 0.5 * self.walk;
            }
        }
        let target = fold01(target);
        let scale_t = s.scale_oct.clamp(-1.0, 1.0);
        let shift_t = s.shift_hz * self.n as f32 / self.fs;
        if !self.primed {
            self.primed = true;
            self.pos_s = target;
            self.scale_s = scale_t;
            self.shift_s = shift_t;
        } else {
            let alpha = 1.0 - (-dt / (s.glide_ms.max(1.0) * 0.001)).exp();
            self.pos_s += (target - self.pos_s) * alpha;
            self.scale_s += (scale_t - self.scale_s) * alpha;
            self.shift_s += (shift_t - self.shift_s) * alpha;
        }
        self.meter.position.store(self.pos_s, Ordering::Relaxed);

        let filled4 = [self.filled[0], self.filled[1], self.filled[2], self.filled[3]];
        let plan = if self.frozen && (self.filled[HOLD] || hold_now) {
            Some(Plan { a: HOLD, b: HOLD, t: 0.0 })
        } else {
            plan(filled4, self.pos_s, s.curve)
        };
        let capture_last = self.capture.is_some() && self.capture_left == 1;
        let job = FrameJob {
            plan,
            resynth: s.resynth,
            scale: 2f32.powf(self.scale_s),
            shift_bins: self.shift_s,
            blend: s.blend.clamp(0.0, 1.0),
            env_half_width: 2 + ((1.0 - s.detail.clamp(0.0, 1.0)).powi(2) * 40.0) as usize,
            capture: self.capture,
            capture_last,
            hold_now,
        };

        for c in 0..self.channels {
            frame_channel(
                &job,
                c == 0,
                self.pos,
                &self.fft,
                &self.window,
                self.hop,
                &mut self.ch[c],
                &mut self.scratch,
                &self.band_ranges,
                self.ref_db,
                &self.meter,
            );
        }
        if hold_now {
            self.filled[HOLD] = true;
        }
        if let Some(slot) = self.capture {
            if capture_last {
                self.filled[slot] = true;
                self.meter.filled[slot].store(true, Ordering::Relaxed);
                self.meter.progress[slot].store(0.0, Ordering::Relaxed);
                // Slide silhouette from channel 0.
                let bins = self.bins;
                let sc = &mut self.scratch;
                for k in 0..bins {
                    sc.db[k] = self.ch[0].slots[slot].lmag[k] * (20.0 / std::f32::consts::LN_10) - self.ref_db;
                }
                for b in 0..BANDS {
                    self.meter.slots[slot][b].store(band_level(&sc.db, self.band_ranges[b]), Ordering::Relaxed);
                }
                self.capture = None;
            } else {
                self.capture_left -= 1;
                let done = (CAPTURE_FRAMES - self.capture_left) as f32 / CAPTURE_FRAMES as f32;
                self.meter.progress[slot].store(done, Ordering::Relaxed);
            }
        }
    }

    /// The morphed target log-magnitude for `plan`, `scale` (octaves) and `shift_bins`, for channel
    /// `ch`. Exposed so tests can compare it with the captured slides.
    pub fn morph_target(&mut self, ch: usize, plan: Plan, scale_oct: f32, shift_bins: f32, blend: f32, out: &mut [f32]) {
        let slots = &self.ch[ch].slots;
        morph(&slots[plan.a], &slots[plan.b], plan, 2f32.powf(scale_oct), shift_bins, blend, out);
    }

    /// A captured slide's log magnitude (natural log of linear magnitude).
    pub fn slot_lmag(&self, ch: usize, slot: usize) -> &[f32] {
        &self.ch[ch].slots[slot].lmag
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

/// Reflects into 0..1.
fn fold01(x: f32) -> f32 {
    let p = x.rem_euclid(2.0);
    if p > 1.0 {
        2.0 - p
    } else {
        p
    }
}

fn wrap_pi(x: f32) -> f32 {
    x - TAU * (x / TAU).round()
}

/// Blend `sa` and `sb` (log domain, towards linear by `blend`), sampling the source at `k / scale - shift`.
fn morph(sa: &Slot, sb: &Slot, plan: Plan, scale: f32, shift: f32, blend: f32, out: &mut [f32]) {
    let bins = out.len();
    let floor = EPS.ln();
    let max = (bins - 1) as f32;
    for (k, o) in out.iter_mut().enumerate() {
        let sf = k as f32 / scale - shift;
        if sf < 0.0 || sf > max {
            *o = floor;
            continue;
        }
        let i0 = sf as usize;
        let fr = sf - i0 as f32;
        let i1 = (i0 + 1).min(bins - 1);
        let at = |s: &Slot| s.lmag[i0] * (1.0 - fr) + s.lmag[i1] * fr;
        *o = if plan.a == plan.b || plan.t <= 0.0 {
            at(sa)
        } else if plan.t >= 1.0 {
            at(sb)
        } else {
            let (la, lb) = (at(sa), at(sb));
            let geometric = la * (1.0 - plan.t) + lb * plan.t;
            if blend > 0.0 {
                let linear = la.exp() * (1.0 - plan.t) + lb.exp() * plan.t;
                geometric + (linear.max(EPS).ln() - geometric) * blend
            } else {
                geometric
            }
        };
    }
}

/// Mean of `values` over a box of half-width `w` around each bin.
fn box_smooth(values: &[f32], out: &mut [f32], prefix: &mut [f32], w: usize) {
    let bins = values.len();
    prefix[0] = 0.0;
    for k in 0..bins {
        prefix[k + 1] = prefix[k] + values[k];
    }
    for (k, o) in out.iter_mut().enumerate() {
        let lo = k.saturating_sub(w);
        let hi = (k + w).min(bins - 1);
        *o = (prefix[hi + 1] - prefix[lo]) / (hi - lo + 1) as f32;
    }
}

/// Level of one display band: the peak over wide bands, an interpolation over narrow ones.
fn band_level(db: &[f32], (lo, hi, centre): (usize, usize, f32)) -> f32 {
    if hi - lo <= 1 {
        let i0 = (centre as usize).min(db.len() - 1);
        let i1 = (i0 + 1).min(db.len() - 1);
        let fr = centre - i0 as f32;
        db[i0] * (1.0 - fr) + db[i1] * fr
    } else {
        db[lo..=hi].iter().cloned().fold(-200.0, f32::max)
    }
}

#[allow(clippy::too_many_arguments)]
fn frame_channel(
    job: &FrameJob,
    meter_channel: bool,
    pos: usize,
    fft: &Fft,
    window: &[f32],
    hop: usize,
    ch: &mut Channel,
    sc: &mut Scratch,
    band_ranges: &[(usize, usize, f32)],
    ref_db: f32,
    meter: &Meter,
) {
    let n = window.len();
    let bins = n / 2 + 1;
    let db_scale = 20.0 / std::f32::consts::LN_10;

    // Analysis.
    for i in 0..n {
        sc.re[i] = ch.in_buf[(pos + i) % n] * window[i];
        sc.im[i] = 0.0;
    }
    fft.forward(&mut sc.re, &mut sc.im);
    for k in 0..bins {
        let (re, im) = (sc.re[k], sc.im[k]);
        sc.mag[k] = (re * re + im * im).sqrt();
        let ph = im.atan2(re);
        let expected = TAU * k as f32 * hop as f32 / n as f32;
        let dev = wrap_pi(ph - ch.prev_phase[k] - expected);
        sc.omega[k] = TAU * k as f32 / n as f32 + dev / hop as f32;
        ch.prev_phase[k] = ph;
        sc.phase[k] = ph;
    }

    // Captures. Every bin of a spectral peak takes the peak's frequency (identity phase locking),
    // which keeps the phases of a partial's bins moving together when the frame is frozen.
    if (job.capture.is_some() && job.capture_last) || job.hold_now {
        lock_peaks(sc, bins);
    }
    if let Some(slot) = job.capture {
        for k in 0..bins {
            ch.acc[k] += sc.mag[k];
        }
        if job.capture_last {
            let s = &mut ch.slots[slot];
            for k in 0..bins {
                s.lmag[k] = (ch.acc[k] / CAPTURE_FRAMES as f32 + EPS).ln();
                s.omega[k] = sc.locked[k];
                s.phase[k] = sc.phase[k];
            }
        }
    }
    if job.hold_now {
        let s = &mut ch.slots[HOLD];
        for k in 0..bins {
            s.lmag[k] = (sc.mag[k] + EPS).ln();
            s.omega[k] = sc.locked[k];
            s.phase[k] = sc.phase[k];
        }
    }

    if meter_channel {
        for k in 0..bins {
            sc.db[k] = 20.0 * (sc.mag[k] + EPS).log10() - ref_db;
        }
        for (b, range) in band_ranges.iter().enumerate() {
            meter.input[b].store(band_level(&sc.db, *range), Ordering::Relaxed);
        }
    }

    // Spectral processing.
    match job.plan {
        None => {
            ch.dominant.fill(NO_DOMINANT);
            for k in 0..bins {
                sc.out_mag[k] = sc.mag[k];
            }
        }
        Some(plan) => {
            morph(&ch.slots[plan.a], &ch.slots[plan.b], plan, job.scale, job.shift_bins, job.blend, &mut sc.tgt);
            if job.resynth {
                resynth(job, plan, hop, n, ch, sc);
            } else {
                ch.dominant.fill(NO_DOMINANT);
                cross(job, bins, sc);
            }
        }
    }

    if meter_channel {
        for k in 0..bins {
            sc.db[k] = db_scale * (sc.out_mag[k] + EPS).ln() - ref_db;
        }
        for (b, range) in band_ranges.iter().enumerate() {
            meter.output[b].store(band_level(&sc.db, *range), Ordering::Relaxed);
        }
    }

    // Synthesis. The envelope path scales the input's own bins; the resynth and passthrough paths
    // rebuild them from magnitude and phase (the input's own phase when passing through).
    let polar = matches!(job.plan, Some(_) if job.resynth);
    for k in 0..bins {
        if polar {
            let ph = ch.syn_phase[k];
            sc.re[k] = sc.out_mag[k] * ph.cos();
            sc.im[k] = sc.out_mag[k] * ph.sin();
        } else if sc.mag[k] > 0.0 {
            let g = sc.out_mag[k] / sc.mag[k];
            sc.re[k] *= g;
            sc.im[k] *= g;
        }
    }
    sc.im[0] = 0.0;
    sc.im[n / 2] = 0.0;
    for k in 1..n / 2 {
        sc.re[n - k] = sc.re[k];
        sc.im[n - k] = -sc.im[k];
    }
    fft.inverse(&mut sc.re, &mut sc.im);
    for i in 0..n {
        ch.out_acc[(pos + i) % n] += sc.re[i] * window[i] * 0.5;
    }
}

/// Fills `sc.locked` with each bin's frequency replaced by that of the nearest spectral peak.
fn lock_peaks(sc: &mut Scratch, bins: usize) {
    let m = |i: isize| if i < 0 || i >= bins as isize { 0.0 } else { sc.mag[i as usize] };
    let is_peak = |k: usize| {
        let (c, k) = (sc.mag[k], k as isize);
        c > m(k - 1) && c > m(k + 1) && c > m(k - 2) && c > m(k + 2)
    };
    let mut last = usize::MAX;
    for k in 0..bins {
        if is_peak(k) {
            last = k;
        }
        sc.left[k] = last;
    }
    let mut next = usize::MAX;
    for k in (0..bins).rev() {
        if is_peak(k) {
            next = k;
        }
        sc.right[k] = next;
    }
    for k in 0..bins {
        let (l, r) = (sc.left[k], sc.right[k]);
        let owner = match (l == usize::MAX, r == usize::MAX) {
            (true, true) => k,
            (true, false) => r,
            (false, true) => l,
            (false, false) => {
                if k - l < r - k || (k - l == r - k && sc.mag[l] >= sc.mag[r]) {
                    l
                } else {
                    r
                }
            }
        };
        // Offset from the peak's own bin centre is kept, so the bin sits at its true frequency
        // relative to the partial.
        sc.locked[k] = sc.omega[owner];
    }
}

/// Impose the target's envelope on the input: scale each bin by the ratio of the smoothed target
/// to the smoothed input, then renormalise the frame to the input's energy.
fn cross(job: &FrameJob, bins: usize, sc: &mut Scratch) {
    for k in 0..bins {
        sc.in_log[k] = (sc.mag[k] + EPS).ln();
    }
    let w = job.env_half_width;
    box_smooth(&sc.tgt, &mut sc.tgt_env, &mut sc.prefix, w);
    box_smooth(&sc.in_log, &mut sc.in_env, &mut sc.prefix, w);
    let (mut e_in, mut e_out) = (0.0f32, 0.0f32);
    for k in 0..bins {
        let g = (sc.tgt_env[k] - sc.in_env[k]).clamp(-6.9, 6.9).exp();
        sc.out_mag[k] = sc.mag[k] * g;
        e_in += sc.mag[k] * sc.mag[k];
        e_out += sc.out_mag[k] * sc.out_mag[k];
    }
    if e_out > 1e-18 {
        let norm = (e_in / e_out).sqrt();
        for k in 0..bins {
            sc.out_mag[k] *= norm;
        }
    }
}

/// Play the morphed magnitude with phases that advance by each bin's own frequency every hop.
fn resynth(job: &FrameJob, plan: Plan, hop: usize, n: usize, ch: &mut Channel, sc: &mut Scratch) {
    let bins = n / 2 + 1;
    for k in 0..bins {
        sc.out_mag[k] = sc.tgt[k].exp() - EPS;
        if sc.out_mag[k] < 0.0 {
            sc.out_mag[k] = 0.0;
        }
    }
    // Each bin takes its phase and frequency from whichever slide contributes more to it (with
    // some hysteresis), so a partial from either slide keeps its own coherent phases.
    let lw_a = (1.0 - plan.t).max(1e-6).ln();
    let lw_b = plan.t.max(1e-6).ln();
    const STICK: f32 = 0.405;
    let max = (bins - 1) as f32;
    for k in 0..bins {
        let sf = (k as f32 / job.scale - job.shift_bins).clamp(0.0, max);
        let ns = sf.round() as usize;
        let (sa, sb) = (&ch.slots[plan.a], &ch.slots[plan.b]);
        let (va, vb) = if plan.a == plan.b { (1.0, 0.0) } else { (sa.lmag[ns] + lw_a, sb.lmag[ns] + lw_b) };
        let current = ch.dominant[k];
        let pick = if current == plan.a {
            if vb > va + STICK {
                plan.b
            } else {
                plan.a
            }
        } else if current == plan.b {
            if va > vb + STICK {
                plan.a
            } else {
                plan.b
            }
        } else if va >= vb {
            plan.a
        } else {
            plan.b
        };
        let src = &ch.slots[pick];
        let omega = (src.omega[ns] + job.shift_bins * TAU / n as f32) * job.scale;
        if pick != current {
            ch.dominant[k] = pick;
            ch.syn_phase[k] = src.phase[ns];
        } else {
            ch.syn_phase[k] = wrap_pi(ch.syn_phase[k] + omega * hop as f32);
        }
    }
}
