//! Real-time pitch-synchronous overlap-add (PSOLA) pitch shifter.
//!
//! Analysis marks are placed one pitch period apart, each aligned to its predecessor by waveform
//! similarity so they land on the same point of the cycle. To shift pitch, two-period Hann grains
//! are cut around the marks and overlap-added at a spacing of `period / ratio`. Each grain keeps
//! its own spectral envelope, so formants stay put while the pitch moves.
//!
//! Output is delayed by `delay` samples so a grain's source is always fully available. Unvoiced
//! input is passed through the same delay untouched, and the two paths are crossfaded.

const RING: usize = 1 << 15;
const MASK: usize = RING - 1;
const MARKS: usize = 32;
const MAX_GRAIN: usize = 8192;

/// Live mode trades some low-frequency body for latency: shorter grains and a shorter mark search.
const QUALITY: Profile = Profile { grain: 1.0, mark_half: 0.5, periods: 2.4 };
const LIVE: Profile = Profile { grain: 0.6, mark_half: 0.4, periods: 1.7 };

#[derive(Clone, Copy)]
struct Profile {
    /// Grain half-length in periods.
    grain: f32,
    /// Half-length of the waveform-similarity window used to place marks, in periods.
    mark_half: f32,
    /// Output delay in periods of the lowest pitch.
    periods: f32,
}

pub fn latency_samples(fs: f32, f_min: f32, live: bool) -> usize {
    let profile = if live { LIVE } else { QUALITY };
    ((profile.periods / f_min + 0.0015) * fs).ceil() as usize
}

pub struct Shifter {
    profile: Profile,
    formant: f32,
    delay: i64,
    n: i64,
    inbuf: [Vec<f32>; 2],
    mono: Vec<f32>,
    outacc: [Vec<f32>; 2],
    winacc: Vec<f32>,
    window: Vec<f32>,
    reference: Vec<f32>,
    marks: [i64; MARKS],
    mark_count: usize,
    period: f32,
    voiced: bool,
    ratio: f32,
    ratio_target: f32,
    ratio_coef: f32,
    next_syn: f64,
    syn_active: bool,
    grains: u32,
    last_grain_u: i64,
    gain: f32,
    gain_target: f32,
    gain_up: f32,
    gain_down: f32,
}

impl Shifter {
    pub fn new(fs: f32, f_min: f32, live: bool) -> Self {
        let coef = |seconds: f32| 1.0 - (-1.0 / (fs * seconds)).exp();
        Self {
            profile: if live { LIVE } else { QUALITY },
            formant: 1.0,
            delay: latency_samples(fs, f_min, live) as i64,
            n: 0,
            inbuf: [vec![0.0; RING], vec![0.0; RING]],
            mono: vec![0.0; RING],
            outacc: [vec![0.0; RING], vec![0.0; RING]],
            winacc: vec![0.0; RING],
            window: vec![0.0; MAX_GRAIN],
            reference: vec![0.0; MAX_GRAIN],
            marks: [0; MARKS],
            mark_count: 0,
            period: 0.0,
            voiced: false,
            ratio: 1.0,
            ratio_target: 1.0,
            ratio_coef: coef(0.002),
            next_syn: 0.0,
            syn_active: false,
            grains: 0,
            last_grain_u: 0,
            gain: 0.0,
            gain_target: 0.0,
            gain_up: coef(0.004),
            gain_down: coef(0.006),
        }
    }

    pub fn reset(&mut self) {
        for buf in self.inbuf.iter_mut().chain(self.outacc.iter_mut()) {
            buf.fill(0.0);
        }
        self.mono.fill(0.0);
        self.winacc.fill(0.0);
        self.n = 0;
        self.mark_count = 0;
        self.period = 0.0;
        self.voiced = false;
        self.ratio = 1.0;
        self.ratio_target = 1.0;
        self.formant = 1.0;
        self.next_syn = 0.0;
        self.syn_active = false;
        self.grains = 0;
        self.gain = 0.0;
        self.gain_target = 0.0;
    }

    pub fn latency(&self) -> usize {
        self.delay as usize
    }

    /// Called once per pitch frame.
    pub fn set_control(&mut self, period: f32, voiced: bool, shift_semitones: f32, formant_semitones: f32) {
        self.formant = 2f32.powf(formant_semitones / 12.0).clamp(0.5, 2.0);
        if voiced {
            self.period = period;
        }
        self.voiced = voiced;
        self.ratio_target = 2f32.powf(shift_semitones / 12.0).clamp(0.6, 1.7);
    }

    pub fn process(&mut self, x: [f32; 2], channels: usize) -> [f32; 2] {
        let i = self.n;
        let idx = i as usize & MASK;
        let mut sum = 0.0;
        for c in 0..channels {
            self.inbuf[c][idx] = x[c];
            sum += x[c];
        }
        self.mono[idx] = sum / channels as f32;
        self.n += 1;

        self.update_marks();
        let u = i - self.delay;
        self.synthesize(u);

        self.ratio += (self.ratio_target - self.ratio) * self.ratio_coef;
        let coef = if self.gain_target > self.gain { self.gain_up } else { self.gain_down };
        self.gain += (self.gain_target - self.gain) * coef;

        let mut out = [0.0; 2];
        if u >= 0 {
            let ui = u as usize & MASK;
            let norm = self.winacc[ui].max(0.3);
            for c in 0..channels {
                let wet = self.outacc[c][ui] / norm;
                out[c] = self.gain * wet + (1.0 - self.gain) * self.inbuf[c][ui];
                self.outacc[c][ui] = 0.0;
            }
            self.winacc[ui] = 0.0;
        }
        out
    }

    fn push_mark(&mut self, t: i64) {
        self.marks[self.mark_count % MARKS] = t;
        self.mark_count += 1;
    }

    fn latest_mark(&self) -> i64 {
        self.marks[(self.mark_count - 1) % MARKS]
    }

    fn update_marks(&mut self) {
        if !self.voiced || self.period < 16.0 || self.period > 3500.0 {
            self.mark_count = 0;
            self.syn_active = false;
            self.grains = 0;
            return;
        }
        let p = self.period;
        let h = (self.profile.mark_half * p).round() as i64;
        let have = self.n - 1;
        loop {
            if self.mark_count == 0 {
                let hi = have - h;
                let lo = hi - p.round() as i64;
                if lo < 0 {
                    break;
                }
                let mut best = (lo, f32::MIN);
                for t in lo..=hi {
                    let v = self.mono[t as usize & MASK];
                    if v > best.1 {
                        best = (t, v);
                    }
                }
                self.push_mark(best.0);
                continue;
            }

            let last = self.latest_mark();
            let nominal = last + p.round() as i64;
            let (lo, hi) = (last + (0.9 * p).round() as i64, last + (1.1 * p).round() as i64);
            if hi + h > have {
                break;
            }
            let len = (2 * h + 1) as usize;
            let mut ref_energy = 0.0;
            for (j, r) in self.reference[..len].iter_mut().enumerate() {
                *r = self.mono[(last + j as i64 - h) as usize & MASK];
                ref_energy += *r * *r;
            }
            let mut best = (nominal, f32::MIN);
            for c in lo..=hi {
                let (mut num, mut energy) = (0.0, 0.0);
                for (j, r) in self.reference[..len].iter().enumerate() {
                    let v = self.mono[(c + j as i64 - h) as usize & MASK];
                    num += r * v;
                    energy += v * v;
                }
                let score = num / (ref_energy * energy).sqrt().max(1e-9) - 0.03 * (c - nominal).abs() as f32 / p;
                if score > best.1 {
                    best = (c, score);
                }
            }
            self.push_mark(best.0);
        }
    }

    /// The period measured from the spacing of recent marks. Marks are aligned by waveform
    /// similarity at the full sample rate, so their average spacing is more accurate than the
    /// tracker's estimate, and the output pitch follows what the voice is really doing.
    fn effective_period(&self) -> f32 {
        let n = self.mark_count.min(7);
        if n >= 4 {
            let newest = self.marks[(self.mark_count - 1) % MARKS];
            let oldest = self.marks[(self.mark_count - n) % MARKS];
            let measured = (newest - oldest) as f32 / (n - 1) as f32;
            if (measured / self.period).ln().abs() < 0.1 {
                return measured;
            }
        }
        self.period
    }

    fn pick_source(&self, t: i64, half: i64, have: i64) -> Option<i64> {
        let mut best: Option<(i64, i64)> = None;
        for k in 0..self.mark_count.min(MARKS) {
            let m = self.marks[(self.mark_count - 1 - k) % MARKS];
            if m + half > have {
                continue;
            }
            let dist = (m - t).abs();
            if best.map_or(true, |b| dist < b.1) {
                best = Some((m, dist));
            }
        }
        best.map(|b| b.0)
    }

    fn synthesize(&mut self, u: i64) {
        if self.mark_count < 2 || !self.voiced {
            self.gain_target = 0.0;
            self.syn_active = false;
            return;
        }
        let p = self.effective_period();
        if !self.syn_active {
            self.syn_active = true;
            self.next_syn = (u + 1) as f64 + p as f64;
            self.grains = 0;
            self.last_grain_u = u;
        }
        let have = self.n - 1;
        let out_period = p / self.ratio;
        let half = ((self.profile.grain * p.max(out_period)).round() as i64).clamp(8, (MAX_GRAIN as i64 - 1) / 2);

        while self.next_syn - half as f64 <= u as f64 {
            let t = self.next_syn.round() as i64;
            let Some(src) = self.pick_source(t, half, have) else { break };

            let len = (2 * half + 1) as usize;
            for (k, w) in self.window[..len].iter_mut().enumerate() {
                *w = 0.5 + 0.5 * (std::f32::consts::PI * (k as i64 - half) as f32 / half as f32).cos();
            }
            let formant = self.formant;
            for k in 0..len {
                let off = k as i64 - half;
                if t + off < u {
                    continue;
                }
                let oi = (t + off) as usize & MASK;
                let w = self.window[k];
                self.winacc[oi] += w;
                if formant == 1.0 {
                    let ii = (src + off) as usize & MASK;
                    for c in 0..2 {
                        self.outacc[c][oi] += w * self.inbuf[c][ii];
                    }
                } else {
                    // Reading the grain faster or slower moves its spectral envelope (the formants)
                    // without touching the spacing that sets the pitch. Reads past the newest input
                    // are clamped; they only ever land in the faint tail of the window.
                    let pos = off as f32 * formant;
                    let base = pos.floor();
                    let frac = pos - base;
                    let i0 = ((src + base as i64).min(have)) as usize & MASK;
                    let i1 = ((src + base as i64 + 1).min(have)) as usize & MASK;
                    for c in 0..2 {
                        let a = self.inbuf[c][i0];
                        let b = self.inbuf[c][i1];
                        self.outacc[c][oi] += w * (a + (b - a) * frac);
                    }
                }
            }
            self.next_syn += out_period as f64;
            self.grains += 1;
            self.last_grain_u = u;
        }

        if u - self.last_grain_u > 3 * half {
            // The source ran dry: drop back to the dry path and restart cleanly.
            self.syn_active = false;
            self.gain_target = 0.0;
        } else {
            self.gain_target = if self.grains >= 2 { 1.0 } else { 0.0 };
        }
    }
}
