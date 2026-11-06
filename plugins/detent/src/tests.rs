use crate::control::Settings;
use crate::engine::Engine;
use crate::pitch::PitchTracker;
use std::f32::consts::PI;

const FS: f32 = 48000.0;

/// A synthetic voice: harmonics shaped by fixed formants (so formants do not follow f0).
fn voice(secs: f32, f0_of: impl Fn(f32) -> f32) -> Vec<f32> {
    let formants = [(700.0f32, 110.0f32, 1.0f32), (1200.0, 130.0, 0.6), (2600.0, 200.0, 0.35)];
    let mut phase = 0.0f32;
    (0..(secs * FS) as usize)
        .map(|i| {
            let t = i as f32 / FS;
            let f0 = f0_of(t);
            phase += 2.0 * PI * f0 / FS;
            let mut sample = 0.0;
            let mut h = 1;
            while (h as f32) * f0 < 6000.0 {
                let f = h as f32 * f0;
                let env: f32 = formants.iter().map(|(fc, bw, g)| g / (1.0 + ((f - fc) / bw).powi(2))).sum();
                sample += (env + 0.05 / h as f32) * (h as f32 * phase).sin();
                h += 1;
            }
            sample * 0.25
        })
        .collect()
}

fn midi(f: f32) -> f32 {
    69.0 + 12.0 * (f / 440.0).log2()
}

fn hz(m: f32) -> f32 {
    440.0 * 2f32.powf((m - 69.0) / 12.0)
}

/// Independent f0 estimate: plain autocorrelation with parabolic refinement.
fn estimate_f0(x: &[f32], lo: f32, hi: f32) -> f32 {
    let (min_lag, max_lag) = ((FS / hi) as usize, (FS / lo) as usize);
    let n = x.len() - max_lag - 1;
    let corr = |lag: usize| (0..n).map(|i| x[i] * x[i + lag]).sum::<f32>();
    let (mut best, mut best_v) = (min_lag, f32::MIN);
    for lag in min_lag..=max_lag {
        let c = corr(lag);
        if c > best_v {
            best_v = c;
            best = lag;
        }
    }
    let (a, b, c) = (corr(best - 1), corr(best), corr(best + 1));
    let delta = 0.5 * (a - c) / (a - 2.0 * b + c);
    FS / (best as f32 + delta)
}

fn cents(a: f32, b: f32) -> f32 {
    1200.0 * (a / b).log2()
}

fn chromatic(retune_ms: f32, humanize: f32) -> Settings {
    Settings { mask: 0x0fff, retune_ms, humanize, amount: 1.0, detune_cents: 0.0, formant_semitones: 0.0 }
}

fn run(input: &[f32], settings: &Settings, f_min: f32) -> (Vec<f32>, usize) {
    run_mode(input, settings, f_min, false)
}

fn run_mode(input: &[f32], settings: &Settings, f_min: f32, live: bool) -> (Vec<f32>, usize) {
    let mut engine = Engine::new(FS, f_min, live);
    let latency = engine.latency();
    let out = input.iter().map(|&x| engine.process([x, x], 2, settings)[0]).collect();
    (out, latency)
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
}

#[test]
fn tracker_reads_steady_pitch() {
    let input = voice(1.0, |_| 220.0);
    let mut tracker = PitchTracker::new(FS, 80.0);
    let mut errors = Vec::new();
    for (i, &x) in input.iter().enumerate() {
        if let Some(frame) = tracker.push(x) {
            if i > 12000 {
                assert!(frame.voiced, "frame at {i} not voiced");
                errors.push(cents(frame.f0, 220.0).abs());
            }
        }
    }
    let worst = errors.iter().cloned().fold(0.0, f32::max);
    assert!(worst < 3.0, "worst pitch error {worst} cents");
}

#[test]
fn tracker_ignores_noise() {
    let mut seed = 12345u32;
    let mut tracker = PitchTracker::new(FS, 80.0);
    let mut voiced_frames = 0;
    for i in 0..48000 {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let x = ((seed >> 8) as f32 / 8388608.0 - 1.0) * 0.1;
        if let Some(frame) = tracker.push(x) {
            if i > 12000 && frame.voiced {
                voiced_frames += 1;
            }
        }
    }
    assert_eq!(voiced_frames, 0);
}

#[test]
fn snaps_a_flat_note_to_pitch() {
    let sung = 225.0;
    let target = hz(midi(sung).round());
    let input = voice(2.5, |_| sung);
    let (out, _) = run(&input, &chromatic(0.0, 0.0), 80.0);
    let tail = &out[out.len() - 12000..out.len() - 1000];
    let got = estimate_f0(tail, 150.0, 400.0);
    let err = cents(got, target);
    assert!(err.abs() < 8.0, "sung {sung} Hz -> {got} Hz, target {target} Hz, error {err} cents");
}

#[test]
fn in_tune_input_stays_in_tune_without_clicks() {
    let f = hz(57.0);
    let input = voice(2.0, |_| f);
    let (out, latency) = run(&input, &chromatic(0.0, 0.0), 80.0);
    let a = &input[60000..90000];
    let b = &out[60000 + latency..90000 + latency];

    let err = cents(estimate_f0(&b[..8192], 150.0, 400.0), f);
    assert!(err.abs() < 5.0, "in-tune note drifted {err} cents");

    let level = 20.0 * (rms(b) / rms(a)).log10();
    assert!(level.abs() < 1.0, "level changed by {level} dB");

    let max_step = |x: &[f32]| x.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max);
    let ratio = max_step(b) / max_step(a);
    assert!(ratio < 1.5, "largest sample step grew {ratio}x, which would click");
}

#[test]
fn keeps_vibrato_when_humanize_is_high_and_flattens_it_when_low() {
    let base = hz(57.0);
    let vibrato = |t: f32| base * 2f32.powf(40.0 * (2.0 * PI * 5.5 * t).sin() / 1200.0);
    let input = voice(3.0, vibrato);
    let spread = |signal: &[f32], offset: usize| -> f32 {
        let track: Vec<f32> = (0..30)
            .map(|k| {
                let start = offset + k * 1024;
                cents(estimate_f0(&signal[start..start + 1536], 150.0, 400.0), base)
            })
            .collect();
        let mean = track.iter().sum::<f32>() / track.len() as f32;
        (track.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / track.len() as f32).sqrt()
    };
    let (kept, lat) = run(&input, &chromatic(0.0, 1.0), 80.0);
    let (flat, _) = run(&input, &chromatic(0.0, 0.0), 80.0);
    let original = spread(&input, 60000);
    let kept_spread = spread(&kept, 60000 + lat);
    let flat_spread = spread(&flat, 60000 + lat);
    assert!(kept_spread > 0.6 * original, "humanize=1 kept {kept_spread} of {original} cents");
    assert!(flat_spread < 0.5 * original, "humanize=0 left {flat_spread} of {original} cents");
}

/// Energy of `x` between `lo` and `hi` Hz, from a Hann-windowed DFT sampled every 10 Hz.
fn band_energy(x: &[f32], lo: f32, hi: f32) -> f32 {
    let n = x.len();
    let mut total = 0.0;
    let mut f = lo;
    while f <= hi {
        let (mut re, mut im) = (0.0f32, 0.0f32);
        for (i, &v) in x.iter().enumerate() {
            let w = 0.5 - 0.5 * (2.0 * PI * i as f32 / n as f32).cos();
            let a = 2.0 * PI * f * i as f32 / FS;
            re += v * w * a.cos();
            im += v * w * a.sin();
        }
        total += re * re + im * im;
        f += 10.0;
    }
    total
}

#[test]
fn shifting_moves_pitch_but_not_formants() {
    let f = 220.0;
    let semitones = 4.0;
    let input = voice(2.5, |_| f);
    let mut shifter = crate::psola::Shifter::new(FS, 80.0, false);
    let out: Vec<f32> = input
        .iter()
        .map(|&x| {
            shifter.set_control(FS / f, true, semitones, 0.0);
            shifter.process([x, x], 1)[0]
        })
        .collect();
    let tail = &out[out.len() - 16384..];
    let expected = f * 2f32.powf(semitones / 12.0);
    let got = estimate_f0(&tail[..8192], 150.0, 500.0);
    assert!(cents(got, expected).abs() < 10.0, "shifted pitch {got} Hz, expected {expected} Hz");

    // Formant 1 sits near 700 Hz and formant 3 near 2600 Hz; their balance should not move.
    let reference = &input[input.len() - 16384..];
    let balance = |x: &[f32]| 10.0 * (band_energy(x, 500.0, 950.0) / band_energy(x, 2200.0, 3000.0)).log10();
    let drift = balance(tail) - balance(reference);
    assert!(drift.abs() < 4.0, "formant balance moved by {drift} dB");
}

#[test]
fn unvoiced_sound_passes_through_at_level() {
    let mut seed = 99u32;
    let input: Vec<f32> = (0..48000)
        .map(|_| {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            ((seed >> 8) as f32 / 8388608.0 - 1.0) * 0.1
        })
        .collect();
    let (out, latency) = run(&input, &chromatic(0.0, 0.0), 80.0);
    let ratio = rms(&out[20000 + latency..40000 + latency]) / rms(&input[20000..40000]);
    assert!((ratio - 1.0).abs() < 0.1, "noise level changed by {ratio}x");
}

#[test]
fn sings_a_melody_into_the_scale() {
    // Off-pitch notes with 40 ms glides and a little vibrato, snapped to C major.
    let notes = [60.4f32, 62.35, 64.7, 62.0, 59.6, 67.45];
    let expected = [60.0f32, 62.0, 65.0, 62.0, 60.0, 67.0];
    let seg = 0.8;
    let pitch_at = |t: f32| {
        let k = ((t / seg) as usize).min(notes.len() - 1);
        let into = t - k as f32 * seg;
        let from = if k == 0 { notes[0] } else { notes[k - 1] };
        let glide = (into / 0.04).clamp(0.0, 1.0);
        let m = from + (notes[k] - from) * (0.5 - 0.5 * (PI * glide).cos());
        hz(m + 0.2 * (2.0 * PI * 5.5 * t).sin())
    };
    let input = voice(seg * notes.len() as f32, pitch_at);
    let settings = Settings { mask: 0xAB5, retune_ms: 15.0, humanize: 0.0, amount: 1.0, detune_cents: 0.0, formant_semitones: 0.0 };
    let (out, latency) = run(&input, &settings, 80.0);

    for (k, want) in expected.iter().enumerate() {
        let end = ((k as f32 + 1.0) * seg * FS) as usize + latency;
        let window = &out[end - 12000..end - 3000];
        let got = midi(estimate_f0(&window[..8192], 150.0, 500.0));
        assert!((got - want).abs() < 0.12, "note {k}: sang {} -> got MIDI {got}, wanted {want}", notes[k]);
    }

    let max_step = |x: &[f32]| x.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max);
    let ratio = max_step(&out) / max_step(&input);
    assert!(ratio < 2.0, "melody output has a click: largest step {ratio}x the input's");
}

#[test]
fn slow_retune_takes_time_and_fast_retune_does_not() {
    // A note a quarter tone flat of C: after 80 ms a slow setting should still be mostly flat.
    let sung = hz(59.7);
    let input = voice(1.5, |_| sung);
    let error_after = |retune_ms: f32, seconds: f32| {
        let (out, latency) = run(&input, &chromatic(retune_ms, 0.0), 80.0);
        let at = (seconds * FS) as usize + latency;
        cents(estimate_f0(&out[at..at + 4096], 150.0, 500.0), hz(60.0)).abs()
    };
    let slow_early = error_after(300.0, 0.10);
    let fast_early = error_after(0.0, 0.10);
    let slow_late = error_after(300.0, 1.2);
    assert!(fast_early < 10.0, "fast retune still {fast_early} cents off");
    assert!(slow_early > 12.0, "slow retune finished too early ({slow_early} cents)");
    assert!(slow_late < 10.0, "slow retune never arrived ({slow_late} cents)");
}

/// Runs the engine over real recordings: `DETENT_FILES=a,b cargo test offline -- --ignored --nocapture`.
/// Inputs and outputs are raw mono f32le at 44.1 kHz in /tmp/detent.
#[test]
#[ignore]
fn offline_files() {
    let names = std::env::var("DETENT_FILES").unwrap_or_default();
    for name in names.split(',').filter(|n| !n.is_empty()) {
        let bytes = std::fs::read(format!("/tmp/detent/{name}.raw")).expect("raw input");
        let input: Vec<f32> = bytes.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
        let fs = 44100.0;
        let f_min: f32 = std::env::var("DETENT_FMIN").ok().and_then(|v| v.parse().ok()).unwrap_or(80.0);
        let retune: f32 = std::env::var("DETENT_RETUNE").ok().and_then(|v| v.parse().ok()).unwrap_or(0.0);
        let humanize: f32 = std::env::var("DETENT_HUMANIZE").ok().and_then(|v| v.parse().ok()).unwrap_or(0.0);
        let mask: u16 = std::env::var("DETENT_MASK").ok().and_then(|v| u16::from_str_radix(&v, 16).ok()).unwrap_or(0x0fff);
        let settings = Settings { mask, retune_ms: retune, humanize, amount: 1.0, detune_cents: 0.0, formant_semitones: 0.0 };

        let mut engine = Engine::new(fs, f_min, false);
        let latency = engine.latency();
        let (mut out, mut voiced, mut shifts, mut midis) = (Vec::new(), 0usize, Vec::new(), Vec::new());
        for &x in &input {
            out.push(engine.process([x, x], 1, &settings)[0]);
            if engine.last.voiced {
                voiced += 1;
                shifts.push(engine.last.shift * 100.0);
                midis.push(engine.last.input_midi);
            }
        }
        out.drain(..latency.min(out.len()));
        let bytes: Vec<u8> = out.iter().flat_map(|v| v.to_le_bytes()).collect();
        std::fs::write(format!("/tmp/detent/{name}.out.raw"), bytes).unwrap();

        let abs_mean = shifts.iter().map(|s: &f32| s.abs()).sum::<f32>() / shifts.len().max(1) as f32;
        midis.sort_by(|a, b| a.total_cmp(b));
        let pct = |p: f32| midis.get(((midis.len() as f32 - 1.0) * p) as usize).copied().unwrap_or(0.0);
        let max_step_in = input.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max);
        let max_step_out = out.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max);
        println!(
            "{name}: {:.1}s, latency {:.1} ms, voiced {:.0}% of frames, input MIDI 5th/50th/95th pct {:.1}/{:.1}/{:.1}, mean |correction| {:.0} cents, peak in {:.2} out {:.2}, max step in {:.3} out {:.3}",
            input.len() as f32 / fs,
            latency as f32 / fs * 1000.0,
            100.0 * voiced as f32 / input.len() as f32 * 0.0027 * fs / (0.0027 * fs).max(1.0) * (1.0),
            pct(0.05), pct(0.5), pct(0.95),
            abs_mean,
            input.iter().fold(0.0f32, |m, v| m.max(v.abs())),
            out.iter().fold(0.0f32, |m, v| m.max(v.abs())),
            max_step_in, max_step_out
        );
    }
}

#[test]
fn survives_hostile_input_without_nans() {
    let mut seed = 7u32;
    let mut noise = move || {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        (seed >> 8) as f32 / 8388608.0 - 1.0
    };
    let mut input = Vec::new();
    input.extend(std::iter::repeat(0.0).take(20000)); // silence
    input.extend(std::iter::repeat(0.8).take(20000)); // DC
    input.extend((0..20000).map(|i| if (i / 240) % 2 == 0 { 1.0 } else { -1.0 })); // clipped square
    input.extend((0..20000).map(|i| (2.0 * PI * 1000.0 * i as f32 / FS).sin())); // 1 kHz, above range
    input.extend((0..20000).map(|i| if i % 997 == 0 { 1.0 } else { 0.0 })); // sparse clicks
    input.extend((0..20000).map(|_| noise())); // full-scale noise
    input.extend(voice(1.0, |t| 100.0 + 400.0 * t)); // a fast sweep through the range
    input.extend((0..5000).map(|_| 0.0));
    for retune in [0.0, 50.0] {
        for f_min in [55.0, 170.0] {
            let (out, _) = run(&input, &chromatic(retune, 0.5), f_min);
            let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            assert!(out.iter().all(|v| v.is_finite()), "non-finite output (retune {retune}, f_min {f_min})");
            assert!(peak < 6.0, "runaway output {peak} (retune {retune}, f_min {f_min})");
        }
    }
}

#[test]
fn runs_much_faster_than_real_time() {
    let input = voice(4.0, |t| 180.0 + 20.0 * (2.0 * PI * 5.0 * t).sin());
    let mut engine = Engine::new(FS, 55.0, false);
    let settings = chromatic(10.0, 0.3);
    let start = std::time::Instant::now();
    for &x in &input {
        std::hint::black_box(engine.process([x, x], 2, &settings));
    }
    let elapsed = start.elapsed().as_secs_f32();
    let load = elapsed / 4.0;
    println!("DSP load: {:.1}% of one core (4 s of stereo audio in {:.3} s)", load * 100.0, elapsed);
    assert!(load < 0.5, "too slow: {:.0}% of real time", load * 100.0);
}

#[test]
fn formant_control_moves_formants_and_keeps_pitch() {
    let f = 220.0;
    let input = voice(2.5, |_| f);
    let mut shifter = crate::psola::Shifter::new(FS, 80.0, false);
    let out: Vec<f32> = input
        .iter()
        .map(|&x| {
            shifter.set_control(FS / f, true, 0.0, 4.0);
            shifter.process([x, x], 1)[0]
        })
        .collect();
    let tail = &out[out.len() - 16384..];
    let reference = &input[input.len() - 16384..];
    let got = estimate_f0(&tail[..8192], 150.0, 500.0);
    assert!(cents(got, f).abs() < 15.0, "formant shift moved the pitch to {got} Hz");

    // Formant 1 sits near 700 Hz. Raising formants by 4 semitones pushes its energy up in frequency.
    let upward = |x: &[f32]| 10.0 * (band_energy(x, 800.0, 1100.0) / band_energy(x, 500.0, 800.0)).log10();
    let moved = upward(tail) - upward(reference);
    assert!(moved > 3.0, "formants only moved {moved} dB toward the high side");
}

#[test]
fn live_mode_has_lower_latency_and_still_tunes() {
    let quality = crate::psola::latency_samples(FS, 75.0, false);
    let live = crate::psola::latency_samples(FS, 75.0, true);
    assert!(live < quality * 8 / 10, "live {live} vs quality {quality} samples");

    let sung = 225.0;
    let target = hz(midi(sung).round());
    let input = voice(2.5, |_| sung);
    let (out, _) = run_mode(&input, &chromatic(0.0, 0.0), 110.0, true);
    let got = estimate_f0(&out[out.len() - 12000..out.len() - 1000], 150.0, 400.0);
    assert!(cents(got, target).abs() < 10.0, "live mode: {sung} Hz -> {got} Hz, wanted {target} Hz");

    let max_step = |x: &[f32]| x.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max);
    let ratio = max_step(&out[30000..]) / max_step(&input[30000..]);
    assert!(ratio < 2.0, "live mode clicks: largest step {ratio}x");
}

#[test]
fn live_mode_sings_a_melody_into_the_scale() {
    let notes = [60.4f32, 62.35, 64.7, 62.0, 59.6];
    let expected = [60.0f32, 62.0, 65.0, 62.0, 60.0];
    let seg = 0.8;
    let pitch_at = |t: f32| {
        let k = ((t / seg) as usize).min(notes.len() - 1);
        let into = t - k as f32 * seg;
        let from = if k == 0 { notes[0] } else { notes[k - 1] };
        let glide = (into / 0.04).clamp(0.0, 1.0);
        hz(from + (notes[k] - from) * (0.5 - 0.5 * (PI * glide).cos()) + 0.2 * (2.0 * PI * 5.5 * t).sin())
    };
    let input = voice(seg * notes.len() as f32, pitch_at);
    let settings = Settings { mask: 0xAB5, retune_ms: 15.0, humanize: 0.0, amount: 1.0, detune_cents: 0.0, formant_semitones: 0.0 };
    let (out, latency) = run_mode(&input, &settings, 110.0, true);
    for (k, want) in expected.iter().enumerate() {
        let end = ((k as f32 + 1.0) * seg * FS) as usize + latency;
        let got = midi(estimate_f0(&out[end - 12000..end - 3800], 150.0, 500.0));
        assert!((got - want).abs() < 0.15, "live note {k}: got MIDI {got}, wanted {want}");
    }
}

#[test]
fn an_empty_note_set_leaves_the_voice_alone() {
    // Compare output to input with the same estimator, so the estimator's own bias cancels.
    for sung in [110.0f32, 150.0, 225.0, 261.6, 440.0] {
        let input = voice(2.0, |_| sung);
        let settings = Settings { mask: 0, ..chromatic(0.0, 0.0) };
        let (out, _) = run(&input, &settings, 80.0);
        let before = estimate_f0(&input[input.len() - 12000..input.len() - 1000], 90.0, 600.0);
        let after = estimate_f0(&out[out.len() - 12000..out.len() - 1000], 90.0, 600.0);
        assert!(cents(after, before).abs() < 1.0, "no-notes mode moved {sung} Hz by {} cents", cents(after, before));
    }
}

#[test]
fn midi_notes_become_a_pitch_class_mask() {
    let mut held = [false; 128];
    held[60] = true; // C4
    held[67] = true; // G4
    held[76] = true; // E5
    assert_eq!(crate::scale::midi_mask(&held), 1 | 1 << 4 | 1 << 7);
}


#[test]
fn learn_hears_the_key_of_a_sung_melody() {
    // A slightly out-of-tune melody around G major, sampled the way the editor samples it (~60 Hz).
    let melody = [67.2f32, 69.1, 71.3, 72.0, 74.2, 71.0, 69.2, 67.0, 71.1, 74.0, 72.2, 69.0, 67.1];
    let seg = 0.45;
    let input = voice(seg * melody.len() as f32, |t| hz(melody[((t / seg) as usize).min(melody.len() - 1)]));
    let mut engine = Engine::new(FS, 110.0, false);
    let settings = chromatic(60.0, 0.5);
    let mut histogram = [0.0f32; 12];
    let mut frames = 0;
    for (i, &x) in input.iter().enumerate() {
        engine.process([x, x], 1, &settings);
        if i % 800 == 0 && engine.last.voiced {
            crate::scale::add_to_histogram(&mut histogram, engine.last.input_midi);
            frames += 1;
        }
    }
    assert!(frames > 40, "only {frames} voiced readings");
    let (key, scale, _) = crate::scale::detect_key(&histogram).unwrap();
    assert_eq!((key, scale), (7, crate::scale::Scale::Major), "heard key {key} {scale:?}");
}
