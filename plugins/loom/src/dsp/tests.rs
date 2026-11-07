use super::*;
use std::f32::consts::TAU;

const FS: f32 = 48000.0;

fn noise(n: usize, seed: u32) -> Vec<f32> {
    let mut s = seed;
    (0..n)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            (s as f32 / u32::MAX as f32 - 0.5) * 0.8
        })
        .collect()
}

fn sine(f: f32, n: usize, amp: f32) -> Vec<f32> {
    (0..n).map(|i| amp * (TAU * f * i as f32 / FS).sin()).collect()
}

fn head(delay_ms: f32, level_db: f32) -> HeadSettings {
    HeadSettings { delay_ms, level_db, ..HeadSettings::default() }
}

fn make(channels: usize, heads: &[HeadSettings], feedback: f32) -> Loom {
    let mut dsp = Loom::new();
    dsp.configure(FS, channels);
    let mut s = Settings { feedback, smooth_ms: 0.0, ..Settings::default() };
    for (i, h) in heads.iter().enumerate() {
        s.heads[i] = *h;
    }
    dsp.settings = s;
    dsp
}

/// Runs a mono signal and returns (wet, dry).
fn run(dsp: &mut Loom, x: &[f32]) -> (Vec<f32>, Vec<f32>) {
    let (mut wet, mut dry) = (Vec::with_capacity(x.len()), Vec::with_capacity(x.len()));
    for &v in x {
        let (w, d) = dsp.process([v, 0.0]);
        wet.push(w[0]);
        dry.push(d[0]);
    }
    (wet, dry)
}

fn correlation(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let (na, nb): (f32, f32) = (a.iter().map(|x| x * x).sum(), b.iter().map(|x| x * x).sum());
    dot / (na * nb).sqrt().max(1e-12)
}

/// Amplitude of the component at `f` Hz (exact when `f` is a whole number of Hz and x is 1 s).
fn tone(x: &[f32], f: f32) -> f32 {
    let (mut re, mut im) = (0.0f64, 0.0f64);
    for (i, &v) in x.iter().enumerate() {
        let a = std::f64::consts::TAU * f as f64 * i as f64 / FS as f64;
        re += v as f64 * a.cos();
        im += v as f64 * a.sin();
    }
    (2.0 * (re * re + im * im).sqrt() / x.len() as f64) as f32
}

#[test]
fn mix_zero_is_the_exact_delayed_dry_signal() {
    let x = noise(20000, 1);
    let mut dsp = make(1, &[head(300.0, 0.0), head(900.0, -3.0)], 0.5);
    let (wet, dry) = run(&mut dsp, &x);
    for n in 0..x.len() - LATENCY {
        assert_eq!(dry[n + LATENCY], x[n]);
        assert_eq!(mix(wet[n + LATENCY], dry[n + LATENCY], 0.0), x[n]);
    }
}

#[test]
fn single_head_is_the_input_delayed() {
    let frames = 20;
    let d = frames * HOP;
    let x = noise(60000, 7);
    let mut dsp = make(1, &[head(frames as f32 * HOP as f32 / FS * 1000.0, 0.0)], 0.0);
    let (wet, _) = run(&mut dsp, &x);
    let lag = LATENCY + d;
    let n = x.len() - lag;
    let c = correlation(&wet[lag + 4096..lag + n], &x[4096..n]);
    assert!(c > 0.999, "correlation {c}");
}

#[test]
fn head_at_zero_delay_has_the_reported_latency() {
    let mut x = vec![0.0; 12000];
    x[1000] = 1.0;
    let mut dsp = make(1, &[head(0.0, 0.0)], 0.0);
    let (wet, dry) = run(&mut dsp, &x);
    let peak = |v: &[f32]| v.iter().enumerate().max_by(|a, b| a.1.abs().total_cmp(&b.1.abs())).unwrap().0;
    assert_eq!(peak(&dry), 1000 + LATENCY);
    assert_eq!(peak(&wet), 1000 + LATENCY);
}

#[test]
fn fractional_delay_between_frames_stays_a_clean_tone() {
    let x = sine(1000.0, 96000, 0.5);
    let mut dsp = make(1, &[head(333.0, 0.0)], 0.0);
    let (wet, _) = run(&mut dsp, &x);
    let seg = &wet[48000..96000 - 2048];
    assert!((tone(&seg[..48000 - 2048], 1000.0) - 0.5).abs() < 0.03);
}

#[test]
fn octave_up_moves_the_peak_to_double_frequency() {
    let x = sine(1000.0, 96000, 0.5);
    let mut h = head(0.0, 0.0);
    h.shift = 12.0;
    let mut dsp = make(1, &[h], 0.0);
    let (wet, _) = run(&mut dsp, &x);
    let seg = &wet[40000..40000 + 32768];
    let at = |f: f32| tone(&seg[..32000], f);
    let peak = (1900..2100).step_by(2).map(|f| (f, at(f as f32))).max_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
    let dump: Vec<_> = (1850..2150).step_by(10).map(|f| (f, at(f as f32))).collect();
    assert!((peak.0 as f32 - 2000.0).abs() <= 6.0, "peak at {} Hz {dump:?}", peak.0);
    assert!(peak.1 > 0.2, "amplitude {}", peak.1);
    assert!(at(1000.0) < 0.05 * peak.1, "residue at 1 kHz {}", at(1000.0));
}

#[test]
fn octave_down_moves_the_peak_to_half_frequency() {
    let x = sine(2000.0, 96000, 0.5);
    let mut h = head(0.0, 0.0);
    h.shift = -12.0;
    let mut dsp = make(1, &[h], 0.0);
    let (wet, _) = run(&mut dsp, &x);
    let seg = &wet[40000..40000 + 32000];
    let peak = (900..1100).step_by(2).map(|f| (f, tone(seg, f as f32))).max_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
    assert!((peak.0 as f32 - 1000.0).abs() <= 6.0, "peak at {} Hz", peak.0);
    assert!(peak.1 > 0.2);
}

#[test]
fn band_pass_window_removes_a_tone_outside_it() {
    let x: Vec<f32> = sine(500.0, 96000, 0.3).iter().zip(sine(4000.0, 96000, 0.3)).map(|(a, b)| a + b).collect();
    let mut h = head(0.0, 0.0);
    h.centre_hz = 500.0;
    h.width_oct = 1.0;
    let mut dsp = make(1, &[h], 0.0);
    let (wet, _) = run(&mut dsp, &x);
    let seg = &wet[40000..88000];
    assert!(tone(seg, 500.0) > 0.25, "kept {}", tone(seg, 500.0));
    assert!(tone(seg, 4000.0) < 0.003, "leaked {}", tone(seg, 4000.0));
}

#[test]
fn feedback_at_max_on_noise_stays_finite_and_bounded() {
    let x = noise(48000 * 6, 99);
    let mut heads = Vec::new();
    for i in 0..HEADS {
        let mut h = head(100.0 + 230.0 * i as f32, 3.0);
        h.shift = [7.0, -5.0, 12.0, 0.0][i % 4];
        h.slide = [0.0, 9.0, -14.0][i % 3];
        h.pan = (i as f32 - 3.5) / 3.5;
        heads.push(h);
    }
    let mut dsp = make(2, &heads, MAX_FEEDBACK);
    dsp.settings.smooth_ms = 50.0;
    let mut peak = 0.0f32;
    for &v in &x {
        let (w, d) = dsp.process([v, -v]);
        for s in w.iter().chain(d.iter()) {
            assert!(s.is_finite());
            peak = peak.max(s.abs());
        }
    }
    assert!(peak < 200.0, "peak {peak}");
}

#[test]
fn stereo_heads_pan_and_mono_works() {
    let x = noise(30000, 5);
    let mut h = head(0.0, 0.0);
    h.pan = -1.0;
    let mut dsp = make(2, &[h], 0.0);
    let (mut l, mut r) = (Vec::new(), Vec::new());
    for &v in &x {
        let (w, _) = dsp.process([v, v]);
        l.push(w[0]);
        r.push(w[1]);
    }
    let energy = |v: &[f32]| v[LATENCY..].iter().map(|x| x * x).sum::<f32>();
    assert!(energy(&r) < 1e-6 * energy(&l));

    let mut dsp = make(1, &[head(0.0, 0.0)], 0.0);
    let (wet, _) = run(&mut dsp, &x);
    let c = correlation(&wet[LATENCY + 4096..], &x[4096..x.len() - LATENCY]);
    assert!(c > 0.999, "mono correlation {c}");
}

#[test]
fn glide_moves_the_pitch_and_bounces() {
    let x = sine(1000.0, 96000, 0.5);
    let mut h = head(0.0, 0.0);
    h.slide = 12.0;
    let mut dsp = make(1, &[h], 0.0);
    let mut seen = (f32::MAX, f32::MIN);
    for &v in &x {
        dsp.process([v, 0.0]);
        let g = dsp.meter.glide[0].load(Ordering::Relaxed);
        seen = (seen.0.min(g), seen.1.max(g));
    }
    assert!(seen.1 > 11.0 && seen.0 > -0.1, "{seen:?}");
}

#[test]
fn history_is_a_scrolling_spectrogram() {
    let x = sine(1000.0, 48000, 0.5);
    let mut dsp = make(1, &[], 0.0);
    run(&mut dsp, &x);
    let cursor = dsp.meter.cursor.load(Ordering::Relaxed);
    assert!(cursor > 25);
    let col = (cursor - 1) % COLS;
    let band = ((1000.0f32 / F_MIN).ln() / (F_MAX / F_MIN).ln() * BANDS as f32) as usize;
    let level = dsp.meter.spec[col * BANDS + band].load(Ordering::Relaxed);
    assert!(level > -9.0 && level < 0.0, "level {level}");
}
