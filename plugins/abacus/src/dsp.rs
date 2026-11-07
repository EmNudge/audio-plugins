//! Sine resynthesis. Every `HOP` samples the last `FFT_SIZE` samples are windowed and transformed;
//! the strongest spectral peaks (parabolic interpolation of the log magnitude, frequency refined
//! from the phase advance since the previous frame) become partials. Partials are tracked from
//! frame to frame by nearest frequency, with births and deaths, so each keeps an identity, and are
//! played by a preallocated bank of phase-continuous oscillators whose amplitude and frequency are
//! interpolated linearly across the next hop. What the sines leave unexplained is rebuilt as noise
//! by an inverse FFT with random phase.
//!
//! Latency: the analysis frame is centred `FFT_SIZE / 2` samples back and its values are reached
//! one hop later, so `LATENCY = FFT_SIZE / 2 + HOP`.

use nih_plug::prelude::AtomicF32;
use spectral::Fft;
use std::f32::consts::TAU;
use std::sync::atomic::Ordering;
use std::sync::Arc;

pub const FFT_SIZE: usize = 4096;
pub const HOP: usize = 512;
pub const LATENCY: usize = FFT_SIZE / 2 + HOP;
pub const MAX_PARTIALS: usize = 64;
/// Oscillators: the partials plus the ones fading out while their replacements fade in.
pub const SLOTS: usize = 128;
pub const MAX_CHANNELS: usize = 2;
pub const NOISE_BANDS: usize = 48;
pub const F_MIN: f32 = 40.0;
pub const F_MAX: f32 = 16000.0;

const BINS: usize = FFT_SIZE / 2 + 1;
const NOISE_FFT: usize = 2048;
const MAX_CANDIDATES: usize = BINS / 2 + 8;
/// Detune range at full Spread, in cents either way.
const SPREAD_CENTS: f32 = 70.0;
/// Pivot of the harmonic stretch.
const STRETCH_PIVOT: f32 = 220.0;

#[derive(Clone, Copy)]
pub struct Settings {
    pub partials: usize,
    pub freeze_freq: bool,
    pub freeze_amp: bool,
    /// 0 to 1: analysis window from 512 to 4096 samples.
    pub resolution: f32,
    pub glide_ms: f32,
    pub pitch_semitones: f32,
    /// -1 to 1: partials spread apart (or squeezed together) around a pivot.
    pub stretch: f32,
    pub noise: f32,
    pub spread: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            partials: 16,
            freeze_freq: false,
            freeze_amp: false,
            resolution: 1.0,
            glide_ms: 80.0,
            pitch_semitones: 0.0,
            stretch: 0.0,
            noise: 0.0,
            spread: 0.0,
        }
    }
}

/// Written by the audio thread (channel 0), read by the editor.
pub struct Meter {
    /// Played frequency per oscillator slot, in Hz.
    pub freq: [AtomicF32; SLOTS],
    /// Linear amplitude per slot, 0 for an idle slot.
    pub amp: [AtomicF32; SLOTS],
    /// Residual level per log band, dB.
    pub noise: [AtomicF32; NOISE_BANDS],
}

impl Default for Meter {
    fn default() -> Self {
        Self {
            freq: std::array::from_fn(|_| AtomicF32::new(0.0)),
            amp: std::array::from_fn(|_| AtomicF32::new(0.0)),
            noise: std::array::from_fn(|_| AtomicF32::new(-120.0)),
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Slot {
    alive: bool,
    /// Fading out, to be freed at the next frame.
    dying: bool,
    id: u32,
    /// Frequency measured by the tracker, and what the oscillator follows after Glide and Freeze.
    tracked_freq: f32,
    held_freq: f32,
    held_amp: f32,
    /// Random detune in -1..1, scaled by Spread.
    detune: f32,
    phase: f64,
    /// Running values (cycles per sample, amplitude) and their per-sample steps.
    inc: f32,
    amp: f32,
    d_inc: f32,
    d_amp: f32,
    /// Where the running values arrive at the end of this hop.
    inc_end: f32,
    amp_end: f32,
}

#[derive(Clone, Copy, Default)]
struct Cand {
    bin: u16,
    mag: f32,
}

#[derive(Clone, Copy, Default)]
struct Peak {
    freq: f32,
    amp: f32,
    bin: u16,
    taken: bool,
}

#[derive(Clone, Copy, Default)]
struct Pair {
    cost: f32,
    slot: u16,
    peak: u16,
}

struct Channel {
    ring: Vec<f32>,
    write: usize,
    count: usize,
    prev_re: Vec<f32>,
    prev_im: Vec<f32>,
    slots: Vec<Slot>,
    live: Vec<u16>,
    noise_seg: Vec<f32>,
    ola: Vec<f32>,
    rng: u32,
    next_id: u32,
}

impl Channel {
    fn new(seed: u32) -> Self {
        Self {
            ring: vec![0.0; FFT_SIZE],
            write: 0,
            count: 0,
            prev_re: vec![0.0; BINS],
            prev_im: vec![0.0; BINS],
            slots: vec![Slot::default(); SLOTS],
            live: Vec::with_capacity(SLOTS),
            noise_seg: vec![0.0; HOP],
            ola: vec![0.0; NOISE_FFT],
            rng: seed,
            next_id: 1,
        }
    }

    fn clear(&mut self) {
        self.ring.fill(0.0);
        self.write = 0;
        self.count = 0;
        self.prev_re.fill(0.0);
        self.prev_im.fill(0.0);
        self.slots.fill(Slot::default());
        self.live.clear();
        self.noise_seg.fill(0.0);
        self.ola.fill(0.0);
    }

    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng as f32 / u32::MAX as f32
    }
}

pub struct Resynth {
    pub settings: Settings,
    pub meter: Arc<Meter>,
    fs: f32,
    channels: usize,
    ch: Vec<Channel>,
    fft: Fft,
    fft_noise: Fft,
    re: Vec<f32>,
    im: Vec<f32>,
    mag: Vec<f32>,
    window: Vec<f32>,
    window_len: usize,
    window_sum: f32,
    window_sq: f32,
    hann_noise: Vec<f32>,
    nre: Vec<f32>,
    nim: Vec<f32>,
    prefix_pow: Vec<f64>,
    prefix_count: Vec<u32>,
    smooth: Vec<f32>,
    cands: Vec<Cand>,
    peaks: Vec<Peak>,
    pairs: Vec<Pair>,
}

impl Resynth {
    pub fn new() -> Self {
        let mut s = Self {
            settings: Settings::default(),
            meter: Arc::new(Meter::default()),
            fs: 48000.0,
            channels: 1,
            ch: Vec::new(),
            fft: Fft::new(FFT_SIZE),
            fft_noise: Fft::new(NOISE_FFT),
            re: vec![0.0; FFT_SIZE],
            im: vec![0.0; FFT_SIZE],
            mag: vec![0.0; BINS],
            window: vec![0.0; FFT_SIZE],
            window_len: 0,
            window_sum: 1.0,
            window_sq: 1.0,
            hann_noise: (0..NOISE_FFT).map(|i| 0.5 - 0.5 * (TAU * i as f32 / NOISE_FFT as f32).cos()).collect(),
            nre: vec![0.0; NOISE_FFT],
            nim: vec![0.0; NOISE_FFT],
            prefix_pow: vec![0.0; BINS + 1],
            prefix_count: vec![0; BINS + 1],
            smooth: vec![0.0; BINS],
            cands: Vec::with_capacity(MAX_CANDIDATES),
            peaks: Vec::with_capacity(MAX_PARTIALS),
            pairs: Vec::with_capacity(SLOTS * MAX_PARTIALS),
        };
        s.configure(48000.0, 2);
        s
    }

    pub fn configure(&mut self, fs: f32, channels: usize) {
        self.fs = fs;
        self.channels = channels.clamp(1, MAX_CHANNELS);
        self.ch = (0..self.channels).map(|c| Channel::new(0x9E37_79B9u32.wrapping_mul(c as u32 + 1) | 1)).collect();
        self.window_len = 0;
        self.reset();
    }

    pub fn reset(&mut self) {
        for c in &mut self.ch {
            c.clear();
        }
        for i in 0..SLOTS {
            self.meter.freq[i].store(0.0, Ordering::Relaxed);
            self.meter.amp[i].store(0.0, Ordering::Relaxed);
        }
    }

    /// Partials of channel `c`, as `(identity, played frequency, amplitude)`.
    pub fn tracked(&self, c: usize) -> impl Iterator<Item = (u32, f32, f32)> + '_ {
        let s = self.settings;
        self.ch[c].slots.iter().filter(|s| s.alive && !s.dying).map(move |sl| (sl.id, shape(sl.held_freq, sl.detune, &s), sl.held_amp))
    }

    /// One sample per channel in; `(wet, dry)` out, dry delayed by `LATENCY`.
    pub fn process(&mut self, x: [f32; MAX_CHANNELS]) -> ([f32; MAX_CHANNELS], [f32; MAX_CHANNELS]) {
        let mut wet = [0.0; MAX_CHANNELS];
        let mut dry = [0.0; MAX_CHANNELS];
        let noise_gain = self.settings.noise;
        for c in 0..self.channels {
            let ch = &mut self.ch[c];
            ch.ring[ch.write] = x[c];
            ch.write = (ch.write + 1) & (FFT_SIZE - 1);
            dry[c] = ch.ring[(ch.write + FFT_SIZE - 1 - LATENCY) & (FFT_SIZE - 1)];

            let mut sum = 0.0;
            for &i in &ch.live {
                let s = &mut ch.slots[i as usize];
                s.amp += s.d_amp;
                s.inc += s.d_inc;
                s.phase += s.inc as f64;
                s.phase -= s.phase.floor();
                sum += s.amp * (s.phase as f32 * TAU).sin();
            }
            wet[c] = sum + ch.noise_seg[ch.count] * noise_gain;

            ch.count += 1;
            if ch.count == HOP {
                ch.count = 0;
                self.analyse(c);
            }
        }
        (wet, dry)
    }

    fn update_window(&mut self) {
        let t = self.settings.resolution.clamp(0.0, 1.0);
        // 512 to 4096 samples, geometric.
        let len = ((512.0 * 8.0f32.powf(t)) as usize).clamp(512, FFT_SIZE) & !1;
        if len == self.window_len {
            return;
        }
        self.window_len = len;
        self.window.iter_mut().for_each(|v| *v = 0.0);
        let (mut sum, mut sq) = (0.0, 0.0);
        for i in 0..len {
            let w = 0.5 - 0.5 * (TAU * i as f32 / len as f32).cos();
            self.window[i] = w;
            sum += w;
            sq += w * w;
        }
        self.window_sum = sum;
        self.window_sq = sq;
    }

    fn analyse(&mut self, c: usize) {
        self.update_window();
        let s = self.settings;
        let fs = self.fs;
        let bin_hz = fs / FFT_SIZE as f32;
        let len = self.window_len;
        let start = (FFT_SIZE - len) / 2;
        let spread_bins = FFT_SIZE as f32 / len as f32;

        // Window the newest FFT_SIZE samples, oldest first.
        {
            let ch = &self.ch[c];
            for i in 0..FFT_SIZE {
                let v = ch.ring[(ch.write + i) & (FFT_SIZE - 1)];
                let w = if i >= start && i < start + len { self.window[i - start] } else { 0.0 };
                self.re[i] = v * w;
                self.im[i] = 0.0;
            }
        }
        self.fft.forward(&mut self.re, &mut self.im);
        for k in 0..BINS {
            self.mag[k] = (self.re[k] * self.re[k] + self.im[k] * self.im[k]).sqrt();
        }

        // Peaks: maxima over the width of the window's main lobe, so sidelobes are not partials.
        let hw = ((2.0 * spread_bins) as usize).max(2);
        let k_lo = ((25.0 / bin_hz).ceil() as usize).max(hw + 1);
        let k_hi = (((fs * 0.45 / bin_hz) as usize).min(BINS - 2 - hw)).max(k_lo + 1);
        let max_mag = self.mag[k_lo..=k_hi].iter().cloned().fold(0.0f32, f32::max);
        let floor = (1e-5 * 0.5 * self.window_sum).max(max_mag * 1e-4);
        self.cands.clear();
        for k in k_lo..=k_hi {
            let m = self.mag[k];
            if m <= floor {
                continue;
            }
            let mut is_peak = true;
            for j in 1..=hw {
                if self.mag[k - j] >= m || self.mag[k + j] > m {
                    is_peak = false;
                    break;
                }
            }
            if is_peak && self.cands.len() < MAX_CANDIDATES {
                self.cands.push(Cand { bin: k as u16, mag: m });
            }
        }
        self.cands.sort_unstable_by(|a, b| b.mag.total_cmp(&a.mag));
        let wanted = s.partials.clamp(1, MAX_PARTIALS);

        self.peaks.clear();
        let tol_bins = 0.5 * spread_bins + 0.75;
        for cand in self.cands.iter().take(wanted) {
            let k = cand.bin as usize;
            let (a, b, cc) = (
                (self.mag[k - 1] + 1e-20).ln(),
                (self.mag[k] + 1e-20).ln(),
                (self.mag[k + 1] + 1e-20).ln(),
            );
            let denom = a - 2.0 * b + cc;
            let d = if denom.abs() > 1e-9 { (0.5 * (a - cc) / denom).clamp(-0.5, 0.5) } else { 0.0 };
            let peak_mag = (b - 0.25 * (a - cc) * d).exp();
            let mut bin = k as f32 + d;
            // Phase advance since the previous frame says where inside the bin the sinusoid is.
            let (pr, pi) = (self.ch[c].prev_re[k], self.ch[c].prev_im[k]);
            let cross_re = self.re[k] * pr + self.im[k] * pi;
            let cross_im = self.im[k] * pr - self.re[k] * pi;
            if cross_re != 0.0 || cross_im != 0.0 {
                let expected = TAU * k as f32 * HOP as f32 / FFT_SIZE as f32;
                let mut dev = cross_im.atan2(cross_re) - expected;
                dev -= TAU * (dev / TAU).round();
                let refined = k as f32 + dev * FFT_SIZE as f32 / (TAU * HOP as f32);
                if (refined - bin).abs() < tol_bins {
                    bin = refined;
                }
            }
            self.peaks.push(Peak { freq: bin * bin_hz, amp: peak_mag * 2.0 / self.window_sum, bin: k as u16, taken: false });
        }
        {
            let ch = &mut self.ch[c];
            ch.prev_re.copy_from_slice(&self.re[..BINS]);
            ch.prev_im.copy_from_slice(&self.im[..BINS]);
        }

        self.track(c, &s, bin_hz, spread_bins);
        self.residual(c, &s, spread_bins);
    }

    /// Match peaks to the partials already playing, give the rest births, and set up the ramps.
    fn track(&mut self, c: usize, s: &Settings, bin_hz: f32, spread_bins: f32) {
        let fs = self.fs;
        let ch = &mut self.ch[c];
        for sl in ch.slots.iter_mut() {
            if sl.dying {
                *sl = Slot::default();
            }
        }

        self.pairs.clear();
        let tol_floor = 3.0 * bin_hz * spread_bins.max(1.0) * 0.5;
        for (si, sl) in ch.slots.iter().enumerate() {
            if !sl.alive {
                continue;
            }
            let tol = (0.035 * sl.tracked_freq).max(tol_floor);
            for (pi, p) in self.peaks.iter().enumerate() {
                let dist = (p.freq - sl.tracked_freq).abs();
                if dist <= tol {
                    self.pairs.push(Pair { cost: dist / tol, slot: si as u16, peak: pi as u16 });
                }
            }
        }
        self.pairs.sort_unstable_by(|a, b| a.cost.total_cmp(&b.cost));

        let mut matched = [false; SLOTS];
        let glide = if s.glide_ms < 0.5 { 1.0 } else { 1.0 - (-(HOP as f32) / fs / (s.glide_ms * 1e-3)).exp() };
        for pair in &self.pairs {
            let (si, pi) = (pair.slot as usize, pair.peak as usize);
            if matched[si] || self.peaks[pi].taken {
                continue;
            }
            matched[si] = true;
            self.peaks[pi].taken = true;
            let sl = &mut ch.slots[si];
            sl.tracked_freq = self.peaks[pi].freq;
            if !s.freeze_freq {
                sl.held_freq += (sl.tracked_freq - sl.held_freq) * glide;
            }
            if !s.freeze_amp {
                sl.held_amp = self.peaks[pi].amp;
            }
        }

        // Partials the analysis lost: fade out, unless the amplitude is frozen.
        for (si, sl) in ch.slots.iter_mut().enumerate() {
            if sl.alive && !matched[si] && !s.freeze_amp {
                sl.dying = true;
                sl.held_amp = 0.0;
            }
        }
        // New partials. Frozen amplitude means the set is fixed.
        if !s.freeze_amp {
            for pi in 0..self.peaks.len() {
                if self.peaks[pi].taken {
                    continue;
                }
                let Some(free) = ch.slots.iter().position(|sl| !sl.alive) else { break };
                let detune = ch.random() + ch.random() - 1.0;
                let phase = ch.random() as f64;
                let id = ch.next_id;
                ch.next_id = ch.next_id.wrapping_add(1);
                let p = self.peaks[pi];
                self.peaks[pi].taken = true;
                ch.slots[free] = Slot {
                    alive: true,
                    id,
                    tracked_freq: p.freq,
                    held_freq: p.freq,
                    held_amp: p.amp,
                    detune,
                    phase,
                    inc: shape(p.freq, detune, s) / fs,
                    amp: 0.0,
                    ..Slot::default()
                };
            }
        } else {
            // Cut a frozen set down to the requested count, weakest first.
            let wanted = s.partials.clamp(1, MAX_PARTIALS);
            loop {
                let n = ch.slots.iter().filter(|sl| sl.alive && !sl.dying).count();
                if n <= wanted {
                    break;
                }
                let weakest = ch
                    .slots
                    .iter()
                    .enumerate()
                    .filter(|(_, sl)| sl.alive && !sl.dying)
                    .min_by(|a, b| a.1.held_amp.total_cmp(&b.1.held_amp))
                    .map(|(i, _)| i);
                if let Some(i) = weakest {
                    ch.slots[i].dying = true;
                    ch.slots[i].held_amp = 0.0;
                } else {
                    break;
                }
            }
        }

        // Ramps across the next hop.
        ch.live.clear();
        let nyquist_guard = fs * 0.45;
        for (si, sl) in ch.slots.iter_mut().enumerate() {
            if !sl.alive {
                continue;
            }
            let f = shape(sl.held_freq, sl.detune, s);
            let amp_end = if f > nyquist_guard { 0.0 } else { sl.held_amp };
            // The running values reached the previous targets; start from them exactly.
            if sl.amp_end != 0.0 || sl.inc_end != 0.0 {
                sl.amp = sl.amp_end;
                sl.inc = sl.inc_end;
            }
            sl.inc_end = f.min(nyquist_guard) / fs;
            sl.amp_end = amp_end;
            sl.d_amp = (sl.amp_end - sl.amp) / HOP as f32;
            sl.d_inc = (sl.inc_end - sl.inc) / HOP as f32;
            ch.live.push(si as u16);
        }

        if c == 0 {
            for i in 0..SLOTS {
                let sl = &ch.slots[i];
                let (f, a) = if sl.alive && !sl.dying { (sl.inc_end * fs, sl.amp_end) } else { (0.0, 0.0) };
                self.meter.freq[i].store(f, Ordering::Relaxed);
                self.meter.amp[i].store(a, Ordering::Relaxed);
            }
        }
    }

    /// The unexplained spectrum: power with the partials cut out and the gaps smoothed over,
    /// turned back into noise for the next hop.
    fn residual(&mut self, c: usize, s: &Settings, spread_bins: f32) {
        let hx = ((2.0 * spread_bins) as usize).max(2) + 1;
        self.prefix_pow[0] = 0.0;
        self.prefix_count[0] = 0;
        // `smooth` doubles as the mask of bins that belong to a partial.
        self.smooth.fill(0.0);
        for p in &self.peaks {
            let k = p.bin as usize;
            for j in k.saturating_sub(hx)..=(k + hx).min(BINS - 1) {
                self.smooth[j] = 1.0;
            }
        }
        for k in 0..BINS {
            let keep = self.smooth[k] == 0.0;
            let pw = self.mag[k] * self.mag[k];
            self.prefix_pow[k + 1] = self.prefix_pow[k] + if keep { pw as f64 } else { 0.0 };
            self.prefix_count[k + 1] = self.prefix_count[k] + keep as u32;
        }
        let w = ((4.0 * spread_bins) as usize).max(10);
        for k in 0..BINS {
            let (lo, hi) = (k.saturating_sub(w), (k + w + 1).min(BINS));
            let n = self.prefix_count[hi] - self.prefix_count[lo];
            self.smooth[k] = if n > 0 { ((self.prefix_pow[hi] - self.prefix_pow[lo]) / n as f64) as f32 / self.window_sq } else { 0.0 };
        }

        let bin_hz = self.fs / FFT_SIZE as f32;
        if c == 0 {
            let ln_span = (F_MAX / F_MIN).ln();
            for b in 0..NOISE_BANDS {
                let f0 = F_MIN * (ln_span * b as f32 / NOISE_BANDS as f32).exp();
                let f1 = F_MIN * (ln_span * (b + 1) as f32 / NOISE_BANDS as f32).exp();
                let (k0, k1) = (((f0 / bin_hz) as usize).min(BINS - 2), ((f1 / bin_hz).ceil() as usize).clamp(1, BINS - 1));
                let k1 = k1.max(k0 + 1);
                let mean = self.smooth[k0..k1].iter().sum::<f32>() / (k1 - k0) as f32;
                self.meter.noise[b].store(10.0 * (mean + 1e-12).log10(), Ordering::Relaxed);
            }
        }

        let ch = &mut self.ch[c];
        if s.noise < 0.0005 {
            ch.noise_seg.fill(0.0);
            ch.ola.fill(0.0);
            return;
        }
        // Random phase, magnitude from the smoothed power. 2048-point frames at the hop of 512
        // overlap four times, which sums Hann squared to 1.5.
        let scale = NOISE_FFT as f32 / 1.5;
        self.nre[0] = 0.0;
        self.nim[0] = 0.0;
        self.nre[NOISE_FFT / 2] = 0.0;
        self.nim[NOISE_FFT / 2] = 0.0;
        for j in 1..NOISE_FFT / 2 {
            let m = (scale * self.smooth[2 * j]).sqrt();
            let th = ch.random() * TAU;
            let (sn, cs) = th.sin_cos();
            self.nre[j] = m * cs;
            self.nim[j] = m * sn;
            self.nre[NOISE_FFT - j] = m * cs;
            self.nim[NOISE_FFT - j] = -m * sn;
        }
        self.fft_noise.inverse(&mut self.nre, &mut self.nim);
        for i in 0..NOISE_FFT {
            ch.ola[i] += self.nre[i] * self.hann_noise[i];
        }
        ch.noise_seg.copy_from_slice(&ch.ola[..HOP]);
        ch.ola.copy_within(HOP.., 0);
        let n = ch.ola.len();
        ch.ola[n - HOP..].fill(0.0);
    }
}

impl Default for Resynth {
    fn default() -> Self {
        Self::new()
    }
}

/// Played frequency for a partial: random detune, harmonic stretch about a pivot, then transpose.
pub fn shape(freq: f32, detune: f32, s: &Settings) -> f32 {
    let mut f = freq * (detune * s.spread * SPREAD_CENTS / 1200.0).exp2();
    if s.stretch != 0.0 {
        f = STRETCH_PIVOT * (f / STRETCH_PIVOT).max(1e-6).powf(1.0 + 0.25 * s.stretch);
    }
    f * (s.pitch_semitones / 12.0).exp2()
}

#[cfg(test)]
mod tests;
