use crate::dsp::{plan, Engine, Evolve, Plan, Settings, MAX_CHANNELS};
use std::f32::consts::TAU;
use std::sync::atomic::Ordering;

const FS: f32 = 48000.0;

fn engine(channels: usize) -> Engine {
    let mut e = Engine::new();
    e.configure(FS, channels);
    e.settings = Settings { glide_ms: 5.0, ..Settings::default() };
    e
}

fn run_mono(e: &mut Engine, input: &[f32], mix: f32) -> Vec<f32> {
    input.iter().map(|x| e.process([*x, 0.0], mix)[0]).collect()
}

fn sine(freq: f32, amp: f32, len: usize) -> Vec<f32> {
    (0..len).map(|i| amp * (TAU * freq * i as f32 / FS).sin()).collect()
}

fn noise(len: usize, amp: f32, seed: u32) -> Vec<f32> {
    let mut s = seed;
    (0..len)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            (s as f32 / u32::MAX as f32 * 2.0 - 1.0) * amp
        })
        .collect()
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
}

/// Hann-windowed single-bin DFT magnitude (amplitude of a sine at `f`).
fn tone(x: &[f32], f: f32) -> f32 {
    let n = x.len();
    let (mut re, mut im, mut wsum) = (0.0f64, 0.0f64, 0.0f64);
    for (i, v) in x.iter().enumerate() {
        let w = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n as f64).cos();
        let ph = std::f64::consts::TAU * f as f64 * i as f64 / FS as f64;
        re += *v as f64 * w * ph.cos();
        im -= *v as f64 * w * ph.sin();
        wsum += w;
    }
    (2.0 * (re * re + im * im).sqrt() / wsum) as f32
}

/// Frequency of the strongest tone between `lo` and `hi`, 0.1 Hz resolution.
fn peak_freq(x: &[f32], lo: f32, hi: f32) -> f32 {
    let mut best = (0.0, lo);
    let mut f = lo;
    while f <= hi {
        let m = tone(x, f);
        if m > best.0 {
            best = (m, f);
        }
        f += 0.1;
    }
    best.1
}

fn capture(e: &mut Engine, slot: usize, signal: &[f32]) {
    e.meter.request_capture(slot);
    run_mono(e, signal, 1.0);
    assert!(e.is_filled(slot), "slot {slot} should be filled");
}

#[test]
fn passthrough_is_delayed_input_and_latency_matches() {
    let mut e = engine(1);
    let lat = e.latency();
    assert_eq!(lat, 2048);
    let x = noise(20000, 0.5, 7);
    let y = run_mono(&mut e, &x, 1.0);
    for i in lat..x.len() {
        assert!((y[i] - x[i - lat]).abs() < 2e-4, "sample {i}: {} vs {}", y[i], x[i - lat]);
    }
    // An impulse comes out exactly at the reported latency.
    let mut e = engine(1);
    let mut imp = vec![0.0; 6000];
    imp[100] = 1.0;
    let y = run_mono(&mut e, &imp, 1.0);
    let peak = y.iter().enumerate().max_by(|a, b| a.1.abs().total_cmp(&b.1.abs())).unwrap().0;
    assert_eq!(peak, 100 + lat);
}

#[test]
fn zero_mix_is_exactly_the_delayed_dry_signal() {
    let mut e = engine(1);
    let lat = e.latency();
    capture(&mut e, 0, &sine(700.0, 0.4, 8000));
    e.reset();
    e.settings.resynth = true;
    e.settings.freeze = true;
    let x = noise(30000, 0.7, 3);
    let y = run_mono(&mut e, &x, 0.0);
    for i in lat..x.len() {
        assert_eq!(y[i], x[i - lat], "sample {i}");
    }
    assert!(y[..lat].iter().all(|v| *v == 0.0));
}

#[test]
fn frozen_sine_sustains_at_a_steady_level_and_frequency() {
    let freq = 1013.3;
    let amp = 0.5;
    let mut e = engine(1);
    e.settings.resynth = true;
    run_mono(&mut e, &sine(freq, amp, 12000), 1.0);
    e.meter.request_capture(0);
    run_mono(&mut e, &sine(freq, amp, 12000), 1.0);
    assert!(e.is_filled(0));
    // Different input afterwards: the frozen sine must not depend on it.
    let x = noise(FS as usize * 4, 0.05, 11);
    let y = run_mono(&mut e, &x, 1.0);
    let block = FS as usize / 2;
    let levels: Vec<f32> = (1..7).map(|b| tone(&y[b * block..(b + 1) * block], freq)).collect();
    for l in &levels {
        assert!((l / amp - 1.0).abs() < 0.12, "level {l} should stay near {amp}: {levels:?}");
    }
    assert!(levels[5] / levels[0] < 1.1 && levels[5] / levels[0] > 0.9, "no growth or decay: {levels:?}");
    // Right frequency, to a fraction of a bin (bin = 23 Hz).
    let seg = &y[2 * block..3 * block];
    let f = peak_freq(seg, freq - 8.0, freq + 8.0);
    assert!((f - freq).abs() < 0.5, "frequency {f} vs {freq}");
    // No buzz: nearly all the energy is in the sine.
    let total = rms(seg);
    let sine_rms = tone(seg, f) / 2f32.sqrt();
    assert!(sine_rms / total > 0.99, "buzz: tone {sine_rms} of total {total}");
}

#[test]
fn freeze_holds_the_live_frame() {
    let freq = 2200.0;
    let mut e = engine(1);
    e.settings.resynth = true;
    run_mono(&mut e, &sine(freq, 0.4, 10000), 1.0);
    e.settings.freeze = true;
    run_mono(&mut e, &sine(freq, 0.4, 6000), 1.0);
    // Input switches to a different tone, the held sine goes on.
    let y = run_mono(&mut e, &sine(500.0, 0.4, FS as usize), 1.0);
    let seg = &y[FS as usize / 2..];
    assert!(tone(seg, freq) > 0.3, "held tone {}", tone(seg, freq));
    assert!(tone(seg, 500.0) < 0.02);
    assert!(e.meter.holding.load(Ordering::Relaxed));
    // Releasing returns to the live input (Cross mode with nothing captured is passthrough).
    e.settings.freeze = false;
    e.settings.resynth = false;
    let y = run_mono(&mut e, &sine(500.0, 0.4, FS as usize), 1.0);
    let seg = &y[FS as usize / 2..];
    assert!(tone(seg, 500.0) > 0.35 && tone(seg, freq) < 0.02);
}

fn four_slides(e: &mut Engine) {
    capture(e, 0, &sine(500.0, 0.4, 9000));
    capture(e, 1, &sine(3000.0, 0.4, 9000));
    capture(e, 2, &noise(12000, 0.3, 5));
    capture(e, 3, &sine(1500.0, 0.2, 9000));
}

#[test]
fn morph_endpoints_reproduce_each_slide() {
    let mut e = engine(1);
    four_slides(&mut e);
    let bins = e.fft_len() / 2 + 1;
    let mut out = vec![0.0; bins];
    for slot in 0..4 {
        let p = plan([true; 4], slot as f32 / 3.0, 0.0).unwrap();
        assert_eq!(p, Plan { a: slot, b: slot, t: 0.0 });
        e.morph_target(0, p, 0.0, 0.0, 0.5, &mut out);
        for k in 0..bins {
            assert_eq!(out[k], e.slot_lmag(0, slot)[k], "slot {slot} bin {k}");
        }
    }
    // The spectrum the meter shows for slide 1 peaks at 500 Hz.
    let bands: Vec<f32> = (0..crate::dsp::BANDS).map(|b| e.meter.slots[0][b].load(Ordering::Relaxed)).collect();
    let top = bands.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0;
    let f = crate::dsp::F_LO * (crate::dsp::F_HI / crate::dsp::F_LO).powf((top as f32 + 0.5) / crate::dsp::BANDS as f32);
    assert!((f / 500.0 - 1.0).abs() < 0.15, "slide 1 peak band at {f} Hz");
}

#[test]
fn midpoint_lies_between_the_slides() {
    let mut e = engine(1);
    four_slides(&mut e);
    let bins = e.fft_len() / 2 + 1;
    let (mut mid, mut linear) = (vec![0.0; bins], vec![0.0; bins]);
    let p = plan([true; 4], 0.5 / 3.0, 0.0).unwrap();
    assert_eq!((p.a, p.b), (0, 1));
    assert!((p.t - 0.5).abs() < 1e-6);
    e.morph_target(0, p, 0.0, 0.0, 0.0, &mut mid);
    for k in 0..bins {
        let (a, b) = (e.slot_lmag(0, 0)[k], e.slot_lmag(0, 1)[k]);
        assert!(mid[k] >= a.min(b) - 1e-4 && mid[k] <= a.max(b) + 1e-4);
        linear[k] = 0.5 * (a + b);
        assert!((mid[k] - linear[k]).abs() < 1e-4);
    }
    // With the cross-fade the midpoint is still between the slides, and above the log morph.
    let mut faded = vec![0.0; bins];
    e.morph_target(0, p, 0.0, 0.0, 1.0, &mut faded);
    for k in 0..bins {
        let (a, b) = (e.slot_lmag(0, 0)[k], e.slot_lmag(0, 1)[k]);
        assert!(faded[k] >= a.min(b) - 1e-4 && faded[k] <= a.max(b) + 1e-4 && faded[k] >= mid[k] - 1e-4);
    }
    // The curve keeps the ends and the middle but dwells near the slides.
    assert!((plan([true; 4], 0.5 / 3.0, 1.0).unwrap().t - 0.5).abs() < 1e-6);
    assert!(plan([true; 4], 0.25 / 3.0, 1.0).unwrap().t < 0.25);
    // Empty slides are skipped: with 0 and 3 filled, the middle blends them.
    let p = plan([true, false, false, true], 0.5, 0.0).unwrap();
    assert_eq!((p.a, p.b), (0, 3));
    assert!((p.t - 0.5).abs() < 1e-6);
    assert!(plan([false; 4], 0.5, 0.0).is_none());
}

#[test]
fn scale_and_shift_move_the_timbre() {
    let mut e = engine(1);
    capture(&mut e, 0, &sine(1000.0, 0.4, 9000));
    let bins = e.fft_len() / 2 + 1;
    let bin_hz = FS / e.fft_len() as f32;
    let peak = |v: &[f32]| v.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0 as f32 * bin_hz;
    let p = plan([true, false, false, false], 0.0, 0.0).unwrap();
    let mut out = vec![0.0; bins];
    e.morph_target(0, p, 1.0, 0.0, 0.5, &mut out);
    assert!((peak(&out) / 2000.0 - 1.0).abs() < 0.03, "scaled peak {}", peak(&out));
    e.morph_target(0, p, 0.0, 300.0 / bin_hz, 0.5, &mut out);
    assert!((peak(&out) - 1300.0).abs() < 2.0 * bin_hz, "shifted peak {}", peak(&out));
}

#[test]
fn position_morphs_the_resynthesised_tone_between_slides() {
    let mut e = engine(1);
    e.settings.resynth = true;
    e.settings.blend = 1.0;
    capture(&mut e, 0, &sine(500.0, 0.4, 9000));
    capture(&mut e, 1, &sine(3000.0, 0.4, 9000));
    let silence = vec![0.0; FS as usize / 2];
    for (pos, loud, quiet) in [(0.0, 500.0, 3000.0), (1.0 / 3.0, 3000.0, 500.0)] {
        e.settings.position = pos;
        run_mono(&mut e, &silence, 1.0);
        let y = run_mono(&mut e, &silence, 1.0);
        assert!(tone(&y, loud) > 0.3, "pos {pos}: {}", tone(&y, loud));
        assert!(tone(&y, quiet) < 0.01, "pos {pos}: {}", tone(&y, quiet));
    }
    // Halfway there is a bit of each, quieter than a full slide.
    e.settings.position = 0.5 / 3.0;
    run_mono(&mut e, &silence, 1.0);
    let y = run_mono(&mut e, &silence, 1.0);
    let (a, b) = (tone(&y, 500.0), tone(&y, 3000.0));
    assert!(a > 0.05 && a < 0.3 && b > 0.05 && b < 0.3, "{a} {b}");
}

#[test]
fn cross_mode_imposes_the_envelope_and_keeps_the_level() {
    let mut e = engine(1);
    // A dark snapshot: noise through a strong one-pole low-pass.
    let mut lp = 0.0;
    let dark: Vec<f32> = noise(14000, 1.0, 21).iter().map(|x| { lp += 0.05 * (x - lp); lp * 3.0 }).collect();
    capture(&mut e, 0, &dark);
    let input = noise(FS as usize, 0.3, 99);
    let y = run_mono(&mut e, &input, 1.0);
    let lat = e.latency();
    let high = |s: &[f32]| {
        // Energy above 6 kHz relative to the whole, crudely by first difference.
        let d: Vec<f32> = s.windows(2).map(|w| w[1] - w[0]).collect();
        rms(&d) / rms(s)
    };
    let (seg_in, seg_out) = (&input[10000..40000], &y[10000 + lat..40000 + lat]);
    assert!(high(seg_out) < 0.5 * high(seg_in), "{} vs {}", high(seg_out), high(seg_in));
    let ratio = rms(seg_out) / rms(seg_in);
    assert!(ratio > 0.5 && ratio < 1.5, "level ratio {ratio}");
}

#[test]
fn output_stays_finite_on_noise_in_every_mode() {
    for channels in [1, 2] {
        let mut e = engine(channels);
        let l = noise(FS as usize * 3, 1.0, 5);
        let r = noise(FS as usize * 3, 1.0, 6);
        e.settings = Settings {
            resynth: false,
            evolve: Evolve::Walk,
            rate_hz: 4.0,
            depth: 1.0,
            scale_oct: 1.0,
            shift_hz: -1500.0,
            glide_ms: 5.0,
            curve: 0.7,
            detail: 1.0,
            ..Settings::default()
        };
        for slot in 0..4 {
            e.meter.request_capture(slot);
            for i in 0..9000 {
                e.process([l[slot * 9000 + i], r[slot * 9000 + i]], 1.0);
            }
        }
        let mut peak = 0.0f32;
        for i in 0..l.len() {
            // Flip modes and toggles as it runs.
            if i % 20000 == 0 {
                e.settings.resynth = !e.settings.resynth;
                e.settings.freeze = (i / 20000) % 3 == 1;
                e.settings.scale_oct = [1.0, -1.0, 0.0][(i / 20000) % 3];
                e.settings.evolve = [Evolve::Walk, Evolve::Sine, Evolve::Off][(i / 40000) % 3];
            }
            let y = e.process([l[i], r[i]], 0.8);
            for c in 0..MAX_CHANNELS.min(channels) {
                assert!(y[c].is_finite(), "non-finite at {i}");
                peak = peak.max(y[c].abs());
            }
        }
        assert!(peak < 50.0, "peak {peak}");
    }
}

#[test]
fn stereo_channels_are_captured_separately() {
    let mut e = engine(2);
    e.meter.request_capture(0);
    for i in 0..9000 {
        let t = i as f32 / FS;
        e.process([0.4 * (TAU * 800.0 * t).sin(), 0.4 * (TAU * 4000.0 * t).sin()], 1.0);
    }
    e.settings.resynth = true;
    let mut out = Vec::new();
    for _ in 0..FS as usize / 2 {
        out.push(e.process([0.0, 0.0], 1.0));
    }
    let l: Vec<f32> = out.iter().map(|v| v[0]).collect();
    let r: Vec<f32> = out.iter().map(|v| v[1]).collect();
    assert!(tone(&l, 800.0) > 0.3 && tone(&l, 4000.0) < 0.01);
    assert!(tone(&r, 4000.0) > 0.3 && tone(&r, 800.0) < 0.01);
}

#[test]
fn evolve_moves_the_position_and_capture_progress_is_reported() {
    let mut e = engine(1);
    e.settings.evolve = Evolve::Sine;
    e.settings.rate_hz = 2.0;
    e.settings.depth = 1.0;
    e.settings.position = 0.5;
    let (mut lo, mut hi) = (1.0f32, 0.0f32);
    for _ in 0..FS as usize {
        e.process([0.0, 0.0], 1.0);
        let p = e.meter.position.load(Ordering::Relaxed);
        lo = lo.min(p);
        hi = hi.max(p);
    }
    assert!(lo < 0.1 && hi > 0.9, "{lo}..{hi}");
    e.meter.request_capture(2);
    let mut seen = false;
    for _ in 0..FS as usize / 4 {
        e.process([0.1, 0.0], 1.0);
        let p = e.meter.progress[2].load(Ordering::Relaxed);
        seen |= p > 0.0 && p < 1.0;
    }
    assert!(seen && e.meter.filled[2].load(Ordering::Relaxed));
    e.meter.request_clear(2);
    for _ in 0..4096 {
        e.process([0.0, 0.0], 1.0);
    }
    assert!(!e.is_filled(2));
}
