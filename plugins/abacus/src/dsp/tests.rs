use super::*;

const FS: f32 = 48000.0;

fn sine(f: f32, a: f32, n: usize, start: usize) -> Vec<f32> {
    (0..n).map(|i| a * (TAU * f * (i + start) as f32 / FS).sin()).collect()
}

fn noise(n: usize, amp: f32) -> Vec<f32> {
    let mut s = 0x1234_5678u32;
    (0..n)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            (s as f32 / u32::MAX as f32 - 0.5) * 2.0 * amp
        })
        .collect()
}

fn make(settings: Settings, channels: usize) -> Resynth {
    let mut r = Resynth::new();
    r.configure(FS, channels);
    r.settings = settings;
    r
}

/// Mono run: `(wet, dry)`.
fn run_mono(r: &mut Resynth, x: &[f32]) -> (Vec<f32>, Vec<f32>) {
    let (mut wet, mut dry) = (Vec::new(), Vec::new());
    for &v in x {
        let (w, d) = r.process([v, 0.0]);
        wet.push(w[0]);
        dry.push(d[0]);
    }
    (wet, dry)
}

/// Spectral peaks of the last 16384 samples: `(frequency, amplitude)`, strongest first.
fn peaks(x: &[f32], count: usize) -> Vec<(f32, f32)> {
    const N: usize = 16384;
    let seg = &x[x.len() - N..];
    let fft = Fft::new(N);
    let w: Vec<f32> = (0..N).map(|i| 0.5 - 0.5 * (TAU * i as f32 / N as f32).cos()).collect();
    let sum: f32 = w.iter().sum();
    let mut re: Vec<f32> = seg.iter().zip(&w).map(|(a, b)| a * b).collect();
    let mut im = vec![0.0; N];
    fft.forward(&mut re, &mut im);
    let mag: Vec<f32> = (0..N / 2).map(|k| (re[k] * re[k] + im[k] * im[k]).sqrt()).collect();
    let mut found = Vec::new();
    for k in 3..N / 2 - 3 {
        if (1..=3).all(|j| mag[k] > mag[k - j] && mag[k] >= mag[k + j]) && mag[k] > 1e-3 * sum {
            let (a, b, c) = (mag[k - 1].ln(), mag[k].ln(), mag[k + 1].ln());
            let d = 0.5 * (a - c) / (a - 2.0 * b + c);
            found.push(((k as f32 + d) * FS / N as f32, (b - 0.25 * (a - c) * d).exp() * 2.0 / sum));
        }
    }
    found.sort_by(|a, b| b.1.total_cmp(&a.1));
    found.truncate(count);
    found
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
}

fn three_sines(n: usize) -> Vec<f32> {
    let (a, b, c) = (sine(440.0, 0.3, n, 0), sine(1234.0, 0.2, n, 0), sine(3100.0, 0.1, n, 0));
    (0..n).map(|i| a[i] + b[i] + c[i]).collect()
}

#[test]
fn mix_zero_is_the_dry_signal_delayed_exactly() {
    let x = noise(20000, 0.5);
    let mut r = make(Settings { partials: 32, noise: 1.0, spread: 1.0, ..Settings::default() }, 1);
    let (_, dry) = run_mono(&mut r, &x);
    for i in 0..x.len() {
        let expect = if i >= LATENCY { x[i - LATENCY] } else { 0.0 };
        assert_eq!(dry[i], expect, "sample {i}");
    }
}

#[test]
fn three_steady_sines_come_back_at_their_frequencies_and_levels() {
    let mut r = make(Settings { partials: 8, glide_ms: 0.0, ..Settings::default() }, 1);
    let (wet, _) = run_mono(&mut r, &three_sines(FS as usize));
    let found = peaks(&wet, 3);
    assert_eq!(found.len(), 3, "{found:?}");
    for (f, a) in [(440.0, 0.3), (1234.0, 0.2), (3100.0, 0.1)] {
        let (pf, pa) = found.iter().cloned().find(|(pf, _)| (pf - f).abs() < 5.0).unwrap_or_else(|| panic!("no peak near {f}: {found:?}"));
        assert!((pf - f).abs() < 0.5, "{f}: got {pf}");
        assert!((pa / a - 1.0).abs() < 0.15, "{f}: amplitude {pa} vs {a}");
    }
}

#[test]
fn freeze_amp_sustains_the_tone_after_the_input_stops() {
    let mut r = make(Settings { partials: 4, ..Settings::default() }, 1);
    let tone = sine(1000.0, 0.4, 24000, 0);
    let (before, _) = run_mono(&mut r, &tone);
    r.settings.freeze_amp = true;
    let (held, _) = run_mono(&mut r, &vec![0.0; 48000]);
    let level = rms(&before[before.len() - 8000..]);
    let after = rms(&held[held.len() - 8000..]);
    assert!(after > 0.8 * level, "{after} vs {level}");
    // Without the freeze the tone dies.
    let mut r = make(Settings { partials: 4, ..Settings::default() }, 1);
    run_mono(&mut r, &tone);
    let (gone, _) = run_mono(&mut r, &vec![0.0; 48000]);
    assert!(rms(&gone[gone.len() - 8000..]) < 1e-4);
}

#[test]
fn one_partial_on_a_chord_is_the_strongest_note_alone() {
    let n = FS as usize;
    let x: Vec<f32> = (0..n).map(|i| {
        let t = i as f32 / FS;
        0.3 * (TAU * 220.0 * t).sin() + 0.2 * (TAU * 277.18 * t).sin() + 0.1 * (TAU * 329.63 * t).sin()
    }).collect();
    let mut r = make(Settings { partials: 1, glide_ms: 0.0, ..Settings::default() }, 1);
    let (wet, _) = run_mono(&mut r, &x);
    let found = peaks(&wet, 2);
    assert!((found[0].0 - 220.0).abs() < 0.5, "{found:?}");
    assert!(found.get(1).map_or(true, |p| p.1 < 0.03 * found[0].1), "{found:?}");
}

#[test]
fn transpose_moves_the_peak_by_the_ratio() {
    for semis in [7.0f32, -12.0, 3.5] {
        let mut r = make(Settings { partials: 2, pitch_semitones: semis, ..Settings::default() }, 1);
        let (wet, _) = run_mono(&mut r, &sine(1000.0, 0.3, FS as usize, 0));
        let f = peaks(&wet, 1)[0].0;
        let want = 1000.0 * (semis / 12.0).exp2();
        assert!((f - want).abs() < 0.5, "{semis}: {f} vs {want}");
    }
}

#[test]
fn stretch_moves_upper_partials_further() {
    let mut r = make(Settings { partials: 4, stretch: 1.0, ..Settings::default() }, 1);
    let (wet, _) = run_mono(&mut r, &sine(2200.0, 0.3, FS as usize, 0));
    let f = peaks(&wet, 1)[0].0;
    let want = STRETCH_PIVOT * (2200.0f32 / STRETCH_PIVOT).powf(1.25);
    assert!((f - want).abs() < 1.0, "{f} vs {want}");
}

#[test]
fn partials_keep_their_identity_across_a_slow_glide() {
    let mut r = make(Settings { partials: 4, glide_ms: 0.0, ..Settings::default() }, 1);
    let n = FS as usize * 2;
    let mut phases = [0.0f32; 2];
    let mut ids_seen: Vec<Vec<(u32, f32)>> = Vec::new();
    for i in 0..n {
        let t = i as f32 / n as f32;
        // 500 Hz -> 540 Hz, and 1500 Hz -> 1380 Hz.
        let f = [500.0 + 40.0 * t, 1500.0 - 120.0 * t];
        let mut x = 0.0;
        for k in 0..2 {
            phases[k] = (phases[k] + f[k] / FS).fract();
            x += 0.25 * (TAU * phases[k]).sin();
        }
        r.process([x, 0.0]);
        if i % HOP == HOP - 1 && i > 12000 {
            ids_seen.push(r.tracked(0).map(|(id, f, _)| (id, f)).collect());
        }
    }
    let first = &ids_seen[0];
    assert_eq!(first.len(), 2, "{first:?}");
    for frame in &ids_seen {
        assert_eq!(frame.len(), 2, "{frame:?}");
        for (id, _) in first {
            assert!(frame.iter().any(|(i, _)| i == id), "lost {id}: {frame:?}");
        }
    }
    // And the frequencies actually moved.
    let moved = |a: &[(u32, f32)], b: &[(u32, f32)]| a.iter().all(|(id, f)| (b.iter().find(|(j, _)| j == id).unwrap().1 - f).abs() > 5.0);
    assert!(moved(first, ids_seen.last().unwrap()));
}

#[test]
fn output_is_finite_on_noise_and_silence() {
    for settings in [
        Settings { partials: 64, noise: 1.0, spread: 1.0, stretch: 1.0, pitch_semitones: 24.0, ..Settings::default() },
        Settings { partials: 64, noise: 1.0, resolution: 0.0, glide_ms: 0.0, freeze_freq: true, ..Settings::default() },
        Settings { partials: 1, noise: 1.0, freeze_amp: true, pitch_semitones: -24.0, ..Settings::default() },
    ] {
        let mut r = make(settings, 2);
        let mut x = noise(60000, 0.8);
        x.extend(vec![0.0; 30000]);
        x.extend(noise(30000, 1.0));
        for &v in &x {
            let (w, d) = r.process([v, -v]);
            assert!(w.iter().chain(d.iter()).all(|s| s.is_finite() && s.abs() < 100.0));
        }
    }
    let mut r = make(Settings { noise: 1.0, ..Settings::default() }, 1);
    let (wet, _) = run_mono(&mut r, &vec![0.0; 20000]);
    assert!(wet.iter().all(|&v| v == 0.0));
}

#[test]
fn stereo_channels_are_independent_and_mono_works() {
    let mut r = make(Settings { partials: 4, ..Settings::default() }, 2);
    let (a, b) = (sine(500.0, 0.3, 48000, 0), sine(900.0, 0.3, 48000, 0));
    let (mut l, mut rt) = (Vec::new(), Vec::new());
    for i in 0..a.len() {
        let (w, _) = r.process([a[i], b[i]]);
        l.push(w[0]);
        rt.push(w[1]);
    }
    assert!((peaks(&l, 1)[0].0 - 500.0).abs() < 0.5);
    assert!((peaks(&rt, 1)[0].0 - 900.0).abs() < 0.5);
    let mut m = make(Settings { partials: 4, ..Settings::default() }, 1);
    let (wet, _) = run_mono(&mut m, &a);
    assert!((peaks(&wet, 1)[0].0 - 500.0).abs() < 0.5);
}

#[test]
fn latency_matches_the_reported_value() {
    // A smooth burst: its energy centroid in the wet output sits LATENCY after the input's.
    let n = 60000;
    let len = 20000usize;
    let at = 15000usize;
    let mut x = vec![0.0; n];
    for i in 0..len {
        let env = 0.5 - 0.5 * (TAU * i as f32 / len as f32).cos();
        x[at + i] = 0.5 * env * (TAU * 1000.0 * i as f32 / FS).sin();
    }
    let mut r = make(Settings { partials: 4, glide_ms: 0.0, ..Settings::default() }, 1);
    let (wet, dry) = run_mono(&mut r, &x);
    let centroid = |v: &[f32]| {
        let (mut num, mut den) = (0.0f64, 0.0f64);
        for (i, s) in v.iter().enumerate() {
            num += i as f64 * (*s as f64).powi(2);
            den += (*s as f64).powi(2);
        }
        num / den
    };
    let shift = centroid(&wet) - centroid(&x);
    assert!((shift - LATENCY as f64).abs() < 200.0, "wet shifted {shift}, reported {LATENCY}");
    assert!((centroid(&dry) - centroid(&x) - LATENCY as f64).abs() < 1.0);
}

#[test]
fn residual_noise_follows_the_unexplained_spectrum() {
    // Broadband noise with one partial taken out: the noise tracks the input level.
    let x = noise(96000, 0.3);
    let level = rms(&x[48000..]);
    let mut r = make(Settings { partials: 1, noise: 1.0, ..Settings::default() }, 1);
    let (wet, _) = run_mono(&mut r, &x);
    let out = rms(&wet[48000..]);
    assert!(out > 0.5 * level && out < 1.6 * level, "{out} vs {level}");
    // And with the knob at zero there is none.
    let mut r = make(Settings { partials: 1, noise: 0.0, ..Settings::default() }, 1);
    let (wet, _) = run_mono(&mut r, &x);
    assert!(rms(&wet[48000..]) < 0.2 * level);
}
