//! Measuring sounds, so the simulation can be checked against real ones without listening.
//!
//! A sound is boiled down to a handful of numbers that say what you would hear: how fast it dies,
//! where its energy sits in the spectrum (and how that changes), how noise-like it is, and what
//! pitch it has. The same measurements run on a recording and on the plugin's output, and
//! [`distance`] says how far apart they are.

use std::f32::consts::PI;
use std::path::Path;

pub const BANDS: usize = 9;
/// The time-frequency fingerprint: this many log-spaced bands from 40 Hz to 20 kHz...
pub const TF_BANDS: usize = 32;
/// ...at this many log-spaced moments from the peak, from 4 ms out to about 3 s.
pub const TF_FRAMES: usize = 28;
/// The tonal fingerprint: fine bands from 40 Hz to 2.5 kHz in a long window, which shows the
/// modes and so the tuning and the character of the head.
pub const TONAL_BANDS: usize = 64;
/// Band edges in Hz: roughly octaves from 80 Hz up.
pub const EDGES: [f32; BANDS + 1] = [20.0, 80.0, 160.0, 320.0, 640.0, 1280.0, 2560.0, 5120.0, 10240.0, 20000.0];
const FLOOR_DB: f32 = -90.0;

/// What kind of sound it is, which decides which measurements matter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Kick,
    Snare,
    Tom,
    Hat,
    Cymbal,
}

impl Kind {
    /// How much the pitch counts: drums are tuned, hats and cymbals are not.
    fn pitched(self) -> f32 {
        match self {
            Kind::Kick | Kind::Tom => 1.0,
            Kind::Snare => 0.3,
            _ => 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Features {
    /// Seconds from the peak until the level last stays below -20 and -40 dB, and the time a 60 dB
    /// fall would take at the slope of the decay.
    pub t20: f32,
    pub t40: f32,
    pub t60: f32,
    /// Energy per band in the first 100 ms after the peak, in dB relative to the total.
    pub early: [f32; BANDS],
    /// The same for 100 ms starting a quarter second later, relative to the early total.
    pub late: [f32; BANDS],
    /// Spectral centroid of the early window, Hz.
    pub centroid: f32,
    /// Geometric over arithmetic mean of the early spectrum between 200 Hz and 16 kHz: 1 is white
    /// noise, near 0 is a few pure tones.
    pub flatness: f32,
    /// Strongest partial between 30 and 800 Hz from 10 to 250 ms after the peak, Hz.
    pub fund: f32,
    /// The same strongest partial in the first 40 ms and from 120 to 350 ms. A drop is the pitch
    /// glide of a kick or tom.
    pub fund_early: f32,
    pub fund_late: f32,
    /// Level in dB of each band at each moment, relative to the loudest cell: how the spectrum
    /// evolves, which is what separates a drum from a noise burst with the same overall shape.
    pub tf: [f32; TF_BANDS * TF_FRAMES],
    /// Level in dB of each fine band in the first 190 ms, relative to the loudest.
    pub tonal: [f32; TONAL_BANDS],
}

impl Default for Features {
    fn default() -> Self {
        Self {
            t20: 0.0,
            t40: 0.0,
            t60: 0.0,
            early: [0.0; BANDS],
            late: [0.0; BANDS],
            centroid: 0.0,
            flatness: 0.0,
            fund: 0.0,
            fund_early: 0.0,
            fund_late: 0.0,
            tf: [FLOOR_DB; TF_BANDS * TF_FRAMES],
            tonal: [FLOOR_DB; TONAL_BANDS],
        }
    }
}

/// In-place radix-2 FFT. `re.len()` must be a power of two.
pub fn fft(re: &mut [f32], im: &mut [f32]) {
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
        let ang = -2.0 * PI / len as f32;
        let (wr, wi) = (ang.cos(), ang.sin());
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0f32, 0.0f32);
            for k in 0..len / 2 {
                let (a, b) = (start + k, start + k + len / 2);
                let (tr, ti) = (re[b] * cr - im[b] * ci, re[b] * ci + im[b] * cr);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let nr = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = nr;
            }
        }
        len <<= 1;
    }
}

/// Power spectrum of `x[start..start+len]` under a Hann window, zero-padded to `nfft`.
pub fn power_spectrum(x: &[f32], start: usize, len: usize, nfft: usize) -> Vec<f32> {
    let mut re = vec![0.0f32; nfft];
    let mut im = vec![0.0f32; nfft];
    let end = (start + len).min(x.len());
    let n = end.saturating_sub(start).min(nfft);
    for i in 0..n {
        let w = 0.5 - 0.5 * (2.0 * PI * i as f32 / n.max(2) as f32).cos();
        re[i] = x[start + i] * w;
    }
    fft(&mut re, &mut im);
    (0..=nfft / 2).map(|k| re[k] * re[k] + im[k] * im[k]).collect()
}

fn db(power: f32) -> f32 {
    (10.0 * power.max(1e-20).log10()).max(FLOOR_DB)
}

/// Strongest peak of the spectrum of a window between `lo` and `hi` Hz.
fn strongest(x: &[f32], sr: f32, start: usize, len: usize, lo: f32, hi: f32) -> f32 {
    let nfft = 1 << 17;
    let p = power_spectrum(x, start, len, nfft);
    let bin = |hz: f32| (hz * nfft as f32 / sr) as usize;
    let (a, b) = (bin(lo), bin(hi).min(p.len() - 1));
    let k = (a..=b).max_by(|i, j| p[*i].total_cmp(&p[*j])).unwrap_or(a);
    // Parabolic interpolation between the bins.
    let (l, c, r) = (p[k.saturating_sub(1)].max(1e-20).ln(), p[k].max(1e-20).ln(), p[(k + 1).min(p.len() - 1)].max(1e-20).ln());
    let denom = l - 2.0 * c + r;
    let shift = if denom.abs() > 1e-9 { 0.5 * (l - r) / denom } else { 0.0 };
    (k as f32 + shift.clamp(-1.0, 1.0)) * sr / nfft as f32
}

/// Measure a sound. `x` is mono at sample rate `sr`.
pub fn features(x: &[f32], sr: f32) -> Features {
    let x = &x[..x.len().min((4.0 * sr) as usize)];
    let peak = x.iter().enumerate().max_by(|a, b| a.1.abs().total_cmp(&b.1.abs())).map(|(i, _)| i).unwrap_or(0);
    let ms = |t: f32| (t * sr) as usize;

    // Level in 4 ms windows, in dB below its own peak.
    let hop = ms(0.002).max(1);
    let win = ms(0.004).max(2);
    let levels: Vec<f32> = (0..x.len().saturating_sub(win) / hop)
        .map(|i| {
            let seg = &x[i * hop..i * hop + win];
            (seg.iter().map(|v| v * v).sum::<f32>() / win as f32).sqrt()
        })
        .collect();
    let top = levels.iter().cloned().fold(1e-9f32, f32::max);
    let from = peak / hop;
    let last_above = |thr_db: f32| {
        let thr = top * 10f32.powf(thr_db / 20.0);
        let idx = (from..levels.len()).rev().find(|i| levels[*i] > thr).unwrap_or(from);
        (idx.saturating_sub(from) * hop) as f32 / sr
    };

    let nfft = 1 << 13;
    let band_energy = |start: usize| -> ([f32; BANDS], f32, Vec<f32>) {
        let p = power_spectrum(x, start, ms(0.1), nfft);
        let hz = |k: usize| k as f32 * sr / nfft as f32;
        let mut bands = [0.0f32; BANDS];
        for (k, v) in p.iter().enumerate() {
            let f = hz(k);
            if let Some(b) = (0..BANDS).find(|b| f >= EDGES[*b] && f < EDGES[b + 1]) {
                bands[b] += v;
            }
        }
        let total: f32 = bands.iter().sum();
        (bands, total, p)
    };
    let (early_p, early_total, spec) = band_energy(peak);
    let (late_p, _, _) = band_energy(peak + ms(0.25));
    // The time to fall 60 dB is read off the slope of the decay between -5 and -35 dB, as in room
    // acoustics, because "the last moment above -60 dB" jumps by hundreds of milliseconds when a
    // quiet, slow resonance happens to sit right at that level.
    let slope_t60 = {
        let pts: Vec<(f32, f32)> = (from..levels.len())
            .map(|i| (((i - from) * hop) as f32 / sr, 20.0 * (levels[i] / top).max(1e-9).log10()))
            .filter(|(_, d)| (-35.0..=-5.0).contains(d))
            .collect();
        // Only the first descent: stop at the first sample below -35 dB.
        let first_below = (from..levels.len()).find(|i| 20.0 * (levels[*i] / top).max(1e-9).log10() < -35.0).map(|i| ((i - from) * hop) as f32 / sr);
        let pts: Vec<(f32, f32)> = pts.into_iter().filter(|(t, _)| first_below.map(|b| *t <= b).unwrap_or(true)).collect();
        let n = pts.len() as f32;
        if n < 4.0 {
            None
        } else {
            let (mt, md) = (pts.iter().map(|p| p.0).sum::<f32>() / n, pts.iter().map(|p| p.1).sum::<f32>() / n);
            let var: f32 = pts.iter().map(|p| (p.0 - mt).powi(2)).sum();
            let cov: f32 = pts.iter().map(|p| (p.0 - mt) * (p.1 - md)).sum();
            let slope = cov / var.max(1e-12);
            (slope < -1.0).then(|| (60.0 / -slope).min(8.0))
        }
    };
    let mut f = Features { t20: last_above(-20.0), t40: last_above(-40.0), t60: slope_t60.unwrap_or_else(|| last_above(-60.0)), ..Default::default() };
    for b in 0..BANDS {
        f.early[b] = db(early_p[b] / early_total.max(1e-20));
        f.late[b] = db(late_p[b] / early_total.max(1e-20));
    }
    let hz = |k: usize| k as f32 * sr / nfft as f32;
    let (mut num, mut den) = (0.0f32, 0.0f32);
    // Up to 20 kHz only, so the same sound measures the same at 44.1 and 96 kHz.
    for (k, v) in spec.iter().enumerate().filter(|(k, _)| hz(*k) < 20000.0) {
        num += hz(k) * v.sqrt();
        den += v.sqrt();
    }
    f.centroid = num / den.max(1e-20);
    let band: Vec<f32> = spec.iter().enumerate().filter(|(k, _)| hz(*k) > 200.0 && hz(*k) < 16000.0).map(|(_, v)| *v + 1e-20).collect();
    let geo = (band.iter().map(|v| v.ln()).sum::<f32>() / band.len().max(1) as f32).exp();
    f.flatness = geo / (band.iter().sum::<f32>() / band.len().max(1) as f32);
    f.fund = strongest(x, sr, peak + ms(0.01), ms(0.24), 30.0, 800.0);
    f.fund_early = strongest(x, sr, peak, ms(0.04), 30.0, 800.0);
    f.fund_late = strongest(x, sr, peak + ms(0.12), ms(0.23), 30.0, 800.0);
    fingerprints(x, sr, peak, &mut f);
    f
}

/// Band edges `n + 1` log-spaced from `lo` to `hi`.
fn log_edges(n: usize, lo: f32, hi: f32) -> Vec<f32> {
    (0..=n).map(|i| lo * (hi / lo).powf(i as f32 / n as f32)).collect()
}

/// Fill in the time-frequency and tonal fingerprints of `x`, with its peak at `peak`.
fn fingerprints(x: &[f32], sr: f32, peak: usize, f: &mut Features) {
    // Spectrum in dB per band, summing the power of the bins that fall in it (or the nearest bin
    // for bands narrower than the bin spacing).
    let bands_db = |start: usize, nfft: usize, win: usize, edges: &[f32], use_max: bool| -> Vec<f32> {
        let p = power_spectrum(x, start, win, nfft);
        let bin = |hz: f32| hz * nfft as f32 / sr;
        (0..edges.len() - 1)
            .map(|b| {
                let (lo, hi) = (bin(edges[b]), bin(edges[b + 1]));
                let (a, z) = (lo.floor() as usize, (hi.ceil() as usize).max(lo.floor() as usize + 1));
                let z = z.min(p.len() - 1);
                let a = a.min(z);
                let v = if use_max { p[a..=z].iter().cloned().fold(0.0, f32::max) } else { p[a..=z].iter().sum() };
                db(v)
            })
            .collect()
    };
    // Time-frequency: 2048-sample windows centred on log-spaced moments after the peak.
    let edges = log_edges(TF_BANDS, 40.0, 20000.0f32.min(sr * 0.45));
    let win = 2048;
    for k in 0..TF_FRAMES {
        let t = 0.004 * 1.28f32.powi(k as i32);
        let centre = peak + (t * sr) as usize;
        let start = centre.saturating_sub(win / 2);
        let row = if start + win / 4 < x.len() { bands_db(start, 2048, win, &edges, false) } else { vec![FLOOR_DB; TF_BANDS] };
        for (b, v) in row.iter().enumerate() {
            f.tf[k * TF_BANDS + b] = *v;
        }
    }
    let top = f.tf.iter().cloned().fold(FLOOR_DB, f32::max);
    for v in f.tf.iter_mut() {
        *v = (*v - top).max(FLOOR_DB);
    }
    let edges = log_edges(TONAL_BANDS, 40.0, 2500.0);
    let tonal = bands_db(peak + (0.005 * sr) as usize, 1 << 15, (0.19 * sr) as usize, &edges, true);
    let top = tonal.iter().cloned().fold(FLOOR_DB, f32::max);
    for (b, v) in tonal.iter().enumerate() {
        f.tonal[b] = (*v - top).max(FLOOR_DB);
    }
}

/// The parts of [`distance`], so a failure says what is wrong.
#[derive(Clone, Copy, Debug, Default)]
pub struct Parts {
    /// How fast it dies.
    pub decay: f32,
    /// Where the energy sits overall (nine bands, early and late), and its centroid.
    pub colour: f32,
    /// How noise-like it is.
    pub noise: f32,
    /// How the spectrum evolves over time.
    pub evolution: f32,
    /// Where the modes are.
    pub modes: f32,
    /// Pitch and glide.
    pub pitch: f32,
}

impl Parts {
    pub fn total(&self) -> f32 {
        self.decay + self.colour + self.noise + self.evolution + self.modes + self.pitch
    }
}

/// How far a candidate is from a target, in rough units of "just noticeable": about 1 is a
/// difference you would hear but call the same kind of drum.
pub fn distance(target: &Features, got: &Features, kind: Kind) -> f32 {
    distance_parts(target, got, kind).total()
}

pub fn distance_parts(target: &Features, got: &Features, kind: Kind) -> Parts {
    let octaves = |a: f32, b: f32| (a.max(1e-3) / b.max(1e-3)).log2().abs();
    let mut p = Parts::default();
    // How fast it dies.
    for (a, b) in [(target.t20, got.t20), (target.t40, got.t40), (target.t60, got.t60)] {
        p.decay += octaves(a, b).min(3.0) * 1.2;
    }
    // Where the energy sits, and how that changes.
    let mut spec = 0.0;
    for k in 0..BANDS {
        spec += ((target.early[k] - got.early[k]) / 6.0).powi(2);
        // Only the late bands that are really there in the target.
        if target.late[k] > -60.0 || got.late[k] > -60.0 {
            spec += 0.5 * ((target.late[k] - got.late[k]) / 8.0).powi(2);
        }
    }
    p.colour = (spec / BANDS as f32).sqrt() * 2.0 + octaves(target.centroid, got.centroid) * 1.5;
    p.noise = (target.flatness - got.flatness).abs() * 4.0;
    // How the spectrum evolves: mean error in dB over the cells that matter, with big misses
    // capped so one wrong partial does not swamp everything.
    let (mut sum, mut n) = (0.0f32, 0.0f32);
    for (a, b) in target.tf.iter().zip(got.tf.iter()) {
        // Cells more than 55 dB below the loudest are at the level of the recording's own noise
        // floor and room, and not part of the drum.
        if *a > -55.0 || *b > -55.0 {
            sum += (a - b).abs().min(25.0);
            n += 1.0;
        }
    }
    p.evolution = sum / n.max(1.0) / 4.0;
    // Where the modes are: the long, fine spectrum, only where the target has something.
    let (mut sum, mut n) = (0.0f32, 0.0f32);
    for (a, b) in target.tonal.iter().zip(got.tonal.iter()) {
        if *a > -40.0 {
            sum += (a - b).abs().min(30.0);
            n += 1.0;
        }
    }
    p.modes = sum / n.max(1.0) / 6.0;
    let w = kind.pitched();
    if w > 0.0 {
        // The strongest partial can flip between two of nearly equal level, so a miss is capped at
        // a fifth: the mode-structure term (`modes`) is what really checks the tuning.
        let semis = |a: f32, b: f32| (12.0 * octaves(a, b)).min(7.0);
        p.pitch = w * (semis(target.fund, got.fund) / 2.0 + 0.25 * semis(target.fund_early, got.fund_early) / 2.0 + 0.25 * semis(target.fund_late, got.fund_late) / 2.0);
    }
    p
}

/// Read a PCM WAV (16, 24 or 32 bit) as mono.
pub fn read_wav(path: &Path) -> Result<(Vec<f32>, f32), String> {
    let b = std::fs::read(path).map_err(|e| e.to_string())?;
    if b.len() < 44 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return Err("not a WAV".into());
    }
    let (mut pos, mut ch, mut sr, mut bits, mut data) = (12, 1usize, 44100u32, 16usize, None);
    while pos + 8 <= b.len() {
        let size = u32::from_le_bytes(b[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let body = pos + 8;
        match &b[pos..pos + 4] {
            b"fmt " => {
                ch = u16::from_le_bytes(b[body + 2..body + 4].try_into().unwrap()) as usize;
                sr = u32::from_le_bytes(b[body + 4..body + 8].try_into().unwrap());
                bits = u16::from_le_bytes(b[body + 14..body + 16].try_into().unwrap()) as usize;
            }
            b"data" => data = Some(&b[body..(body + size).min(b.len())]),
            _ => {}
        }
        pos = body + size + (size & 1);
    }
    let data = data.ok_or("no data chunk")?;
    let bytes = bits / 8;
    let frame = bytes * ch;
    let sample = |i: usize| -> f32 {
        let s = &data[i..i + bytes];
        match bits {
            16 => i16::from_le_bytes([s[0], s[1]]) as f32 / 32768.0,
            24 => (((s[0] as i32) << 8 | (s[1] as i32) << 16 | (s[2] as i32) << 24) >> 8) as f32 / 8388608.0,
            32 => i32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f32 / 2147483648.0,
            _ => 0.0,
        }
    };
    if !matches!(bits, 16 | 24 | 32) {
        return Err(format!("{bits}-bit WAV not supported"));
    }
    let n = data.len() / frame;
    Ok(((0..n).map(|f| (0..ch).map(|c| sample(f * frame + c * bytes)).sum::<f32>() / ch as f32).collect(), sr as f32))
}

/// Write mono 16-bit PCM, so a render can be opened in anything.
pub fn write_wav(path: &Path, x: &[f32], sr: f32) -> std::io::Result<()> {
    let data: Vec<u8> = x.iter().flat_map(|v| ((v.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes()).collect();
    let mut out = Vec::with_capacity(44 + data.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&(sr as u32).to_le_bytes());
    out.extend_from_slice(&(sr as u32 * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 44100.0;

    fn tone(hz: f32, decay: f32, seconds: f32) -> Vec<f32> {
        (0..(seconds * SR) as usize).map(|i| (2.0 * PI * hz * i as f32 / SR).sin() * (-(i as f32 / SR) / decay).exp()).collect()
    }

    fn noise(seconds: f32, decay: f32) -> Vec<f32> {
        let mut s = 0x1234_5678u32;
        (0..(seconds * SR) as usize)
            .map(|i| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                (s as f32 / u32::MAX as f32 * 2.0 - 1.0) * (-(i as f32 / SR) / decay).exp()
            })
            .collect()
    }

    #[test]
    fn the_fft_finds_a_tone() {
        let x = tone(1000.0, 10.0, 0.2);
        let p = power_spectrum(&x, 0, x.len(), 1 << 14);
        let k = (0..p.len()).max_by(|a, b| p[*a].partial_cmp(&p[*b]).unwrap()).unwrap();
        assert!((k as f32 * SR / (1 << 14) as f32 - 1000.0).abs() < 5.0);
    }

    #[test]
    fn decay_times_follow_the_envelope() {
        // Amplitude e^(-t/tau) falls by 20 dB at tau * ln(10) and 60 dB at tau * ln(1000).
        let f = features(&tone(200.0, 0.1, 2.0), SR);
        assert!((f.t20 - 0.1 * 10f32.ln()).abs() < 0.03, "t20 {}", f.t20);
        assert!((f.t60 - 0.1 * 1000f32.ln()).abs() < 0.08, "t60 {}", f.t60);
    }

    #[test]
    fn pitch_flatness_and_brightness_tell_tones_from_noise() {
        let t = features(&tone(150.0, 0.3, 1.0), SR);
        assert!((t.fund - 150.0).abs() < 3.0, "fund {}", t.fund);
        assert!(t.flatness < 0.1 && t.centroid < 400.0, "tone: flat {} centroid {}", t.flatness, t.centroid);
        let n = features(&noise(1.0, 0.3), SR);
        assert!(n.flatness > 0.5 && n.centroid > 4000.0, "noise: flat {} centroid {}", n.flatness, n.centroid);
    }

    #[test]
    fn a_falling_pitch_shows_in_early_and_late_fundamentals() {
        let x: Vec<f32> = (0..(0.6 * SR) as usize)
            .map(|i| {
                let t = i as f32 / SR;
                // 200 Hz gliding to 100 Hz.
                let phase = 2.0 * PI * (100.0 * t + 100.0 * 0.05 * (1.0 - (-t / 0.05).exp()));
                phase.sin() * (-t / 0.4).exp()
            })
            .collect();
        let f = features(&x, SR);
        assert!(f.fund_early > f.fund_late * 1.3, "early {} late {}", f.fund_early, f.fund_late);
    }

    #[test]
    fn distance_is_zero_for_itself_and_grows_with_difference() {
        let a = features(&tone(150.0, 0.3, 1.0), SR);
        assert!(distance(&a, &a, Kind::Tom) < 1e-4);
        let b = features(&tone(150.0, 0.1, 1.0), SR);
        let c = features(&noise(1.0, 0.1), SR);
        assert!(distance(&a, &b, Kind::Tom) > 0.5);
        assert!(distance(&a, &c, Kind::Tom) > distance(&a, &b, Kind::Tom));
    }

    #[test]
    fn wavs_round_trip() {
        let x = tone(300.0, 0.2, 0.2);
        let path = std::env::temp_dir().join("tympan-analysis-test.wav");
        write_wav(&path, &x, SR).unwrap();
        let (y, sr) = read_wav(&path).unwrap();
        assert_eq!(sr, SR);
        assert_eq!(y.len(), x.len());
        assert!(x.iter().zip(&y).all(|(a, b)| (a - b).abs() < 1e-3));
        let _ = std::fs::remove_file(path);
    }
}
