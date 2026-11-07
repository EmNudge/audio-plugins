//! DSP tests on synthetic signals: the split sums flat, every stage does what it claims, the
//! limiter holds its ceiling (inter-sample peaks included), and nothing goes non-finite.

use std::f32::consts::TAU;

use crate::dsp::{Carrier, Chassis, Settings};
use crate::dynamics::{BodyEq, Glue, Limiter, Notches, Squash};
use crate::filters::{db_to_lin, lin_to_db, Biquad, Coeffs, Crossover};
use crate::fx::{hilbert_coefs, Cavity, FreqShifter, Plate, Saturator, Shine, Sub};

const FS: f32 = 48000.0;

fn sine(freq: f32, amp: f32, n: usize) -> Vec<f32> {
    (0..n).map(|i| amp * (TAU * freq * i as f32 / FS).sin()).collect()
}

/// A voice-like tone: a harmonic series with a falling spectrum.
fn voice(f0: f32, amp: f32, n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| {
            let t = i as f32 / FS;
            let sum: f32 = (1..=24).map(|k| (TAU * f0 * k as f32 * t).sin() / (k as f32).powf(1.3)).sum();
            amp * sum * 0.5
        })
        .collect()
}

fn noise(amp: f32, n: usize, seed: u32) -> Vec<f32> {
    let mut state = seed.max(1);
    (0..n)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            amp * (state as f32 / u32::MAX as f32 * 2.0 - 1.0)
        })
        .collect()
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0, |m, v| m.max(v.abs()))
}

/// Amplitude of the component at `freq`, with a Hann window so neighbours do not leak in.
fn tone(x: &[f32], freq: f32) -> f32 {
    let n = x.len();
    let (mut re, mut im, mut wsum) = (0.0f64, 0.0f64, 0.0f64);
    for (i, v) in x.iter().enumerate() {
        let w = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n as f64).cos();
        let phase = std::f64::consts::TAU * freq as f64 * i as f64 / FS as f64;
        re += *v as f64 * w * phase.cos();
        im += *v as f64 * w * phase.sin();
        wsum += w;
    }
    (2.0 * re.hypot(im) / wsum) as f32
}

/// RMS of the content between `lo` and `hi` Hz (a sum over Hann-windowed bins a hertz apart, so
/// a tone that slips in phase part-way through still counts in full).
fn band_rms(x: &[f32], lo: f32, hi: f32) -> f32 {
    let sum: f32 = (lo as usize..=hi as usize).map(|f| tone(x, f as f32).powi(2)).sum();
    (sum / 2.0 / 1.5).sqrt()
}

/// The signal through a wide band-pass centred on `centre`.
fn bandpassed(x: &[f32], centre: f32, q: f64) -> Vec<f32> {
    let c = Coeffs::bandpass(FS, centre, q);
    let mut s = Biquad::default();
    x.iter().map(|&v| s.process(&c, v)).collect()
}

/// Normalised autocorrelation at `lag`.
fn autocorr(x: &[f32], lag: usize) -> f32 {
    let n = x.len() - lag;
    let num: f32 = (0..n).map(|i| x[i] * x[i + lag]).sum();
    let den: f32 = x.iter().map(|v| v * v).sum();
    num / den.max(1e-12)
}

/// The lag in `lo..hi` with the highest autocorrelation, as a frequency.
fn pitch_of(x: &[f32], lo_hz: f32, hi_hz: f32) -> f32 {
    let (lo, hi) = ((FS / hi_hz) as usize, (FS / lo_hz) as usize);
    let best = (lo..hi).max_by(|&a, &b| autocorr(x, a).total_cmp(&autocorr(x, b))).unwrap();
    // Parabolic refinement.
    let (a, b, c) = (autocorr(x, best - 1), autocorr(x, best), autocorr(x, best + 1));
    let delta = 0.5 * (a - c) / (a - 2.0 * b + c);
    FS / (best as f32 + delta)
}

fn run(settings: &Settings, input: &[f32]) -> (Vec<f32>, usize) {
    let mut chassis = Chassis::new();
    chassis.configure(FS, 1);
    let out = input.iter().map(|&x| chassis.process([x, 0.0], settings)[0]).collect();
    (out, chassis.latency())
}

#[test]
fn crossover_sums_flat() {
    for freq in [60.0, 200.0, 250.0, 700.0, 2800.0, 5000.0, 12000.0] {
        let mut x = Crossover::new();
        x.set(250.0, 2800.0, FS);
        let input = sine(freq, 0.5, 48000);
        let sum: Vec<f32> = input
            .iter()
            .map(|&v| {
                let b = x.split(0, v);
                b[0] + b[1] + b[2]
            })
            .collect();
        let ratio = lin_to_db(rms(&sum[24000..]) / rms(&input[24000..]));
        assert!(ratio.abs() < 0.1, "{freq} Hz: {ratio:.3} dB");
    }
}

#[test]
fn crossover_bands_split_where_asked() {
    let mut x = Crossover::new();
    x.set(250.0, 2800.0, FS);
    let measure = |freq: f32, band: usize| {
        let mut x = Crossover::new();
        x.set(250.0, 2800.0, FS);
        let out: Vec<f32> = sine(freq, 0.5, 24000).iter().map(|&v| x.split(0, v)[band]).collect();
        rms(&out[12000..]) / rms(&sine(freq, 0.5, 12000))
    };
    assert!(measure(60.0, 0) > 0.95 && measure(60.0, 1) < 0.02);
    assert!(measure(900.0, 1) > 0.95 && measure(900.0, 0) < 0.02 && measure(900.0, 2) < 0.02);
    assert!(measure(9000.0, 2) > 0.95 && measure(9000.0, 1) < 0.02);
    let _ = &mut x;
}

#[test]
fn mix_zero_is_the_input_delayed_by_the_reported_latency() {
    let s = Settings { mix: 0.0, ..Settings::default() };
    let input = noise(0.3, 20000, 7);
    let (out, latency) = run(&s, &input);
    assert!(latency > 0);
    for i in 0..input.len() - latency {
        assert_eq!(out[i + latency], input[i], "sample {i}");
    }
}

#[test]
fn neutral_chain_keeps_the_level_of_noise() {
    let input = noise(0.2, 96000, 3);
    let (out, latency) = run(&Settings::neutral(), &input);
    let tail = 48000;
    let ratio = lin_to_db(rms(&out[latency + tail..]) / rms(&input[tail..out.len() - latency]));
    assert!(ratio.abs() < 0.5, "{ratio:.2} dB");
}

#[test]
fn neutral_chain_keeps_the_level_of_a_voice() {
    let input = voice(150.0, 0.4, 96000);
    let (out, latency) = run(&Settings::neutral(), &input);
    let tail = 48000;
    let ratio = lin_to_db(rms(&out[latency + tail..]) / rms(&input[tail..out.len() - latency]));
    assert!(ratio.abs() < 1.5, "{ratio:.2} dB");
}

#[test]
fn pitch_knob_lowers_the_voice() {
    let input = voice(150.0, 0.4, 96000);
    for semitones in [-4.0f32, -2.0, 3.0] {
        let s = Settings { pitch: semitones, ..Settings::neutral() };
        let (out, _) = run(&s, &input);
        let found = pitch_of(&out[60000..], 70.0, 300.0);
        let wanted = 150.0 * 2f32.powf(semitones / 12.0);
        assert!((found / wanted - 1.0).abs() < 0.03, "{semitones} st: {found:.1} Hz, wanted {wanted:.1}");
    }
}

#[test]
fn formant_shift_moves_the_envelope_and_not_the_pitch() {
    let input = voice(150.0, 0.4, 96000);
    let s = Settings { formant: 5.0, ..Settings::neutral() };
    let (out, _) = run(&s, &input);
    let found = pitch_of(&out[60000..], 70.0, 300.0);
    assert!((found / 150.0 - 1.0).abs() < 0.02, "{found:.1} Hz");
    // The spectral tilt changes: more energy above 1 kHz than before.
    let (plain, _) = run(&Settings::neutral(), &input);
    let high = |x: &[f32]| (8..20).map(|k| tone(&x[60000..], 150.0 * k as f32).powi(2)).sum::<f32>();
    assert!(high(&out) > 1.3 * high(&plain), "{} vs {}", high(&out), high(&plain));
}

#[test]
fn sub_adds_an_octave_down() {
    let input = sine(140.0, 0.3, 48000);
    let with = |sub: f32| run(&Settings { sub, ..Settings::neutral() }, &input).0;
    let (off, on) = (with(0.0), with(1.0));
    // An octave divider can slip a cycle, which turns the sub's phase over mid-note; its energy
    // still sits at half the pitch.
    let (a, b) = (band_rms(&off[24000..], 40.0, 100.0), band_rms(&on[24000..], 40.0, 100.0));
    assert!(b > 0.05 && b > 8.0 * a, "{a} vs {b}");
}

#[test]
fn sub_is_silent_for_silence_and_follows_the_envelope() {
    let mut sub = Sub::new(FS);
    assert!((0..4800).all(|_| sub.process(0.0) == 0.0));
    let loud: Vec<f32> = sine(120.0, 0.4, 24000).iter().map(|&x| sub.process(x)).collect();
    let quiet: Vec<f32> = sine(120.0, 0.004, 24000).iter().map(|&x| sub.process(x)).collect();
    assert!(rms(&loud[12000..]) > 30.0 * rms(&quiet[12000..]));
}

#[test]
fn ring_mod_makes_sidebands_and_drops_the_carrier() {
    let input = sine(600.0, 0.3, 48000);
    let s = Settings { ring: 1.0, ring_hz: 100.0, ..Settings::neutral() };
    let (out, _) = run(&s, &input);
    let x = &out[24000..];
    assert!(tone(x, 500.0) > 0.08 && tone(x, 700.0) > 0.08, "{} {}", tone(x, 500.0), tone(x, 700.0));
    assert!(tone(x, 600.0) < 0.02, "{}", tone(x, 600.0));
}

#[test]
fn frequency_shifter_moves_a_sine_up_without_an_image() {
    let coefs = hilbert_coefs(FS);
    let mut shifter = FreqShifter::default();
    let mut phase = 0.0f32;
    let out: Vec<f32> = sine(1000.0, 0.5, 48000)
        .iter()
        .map(|&x| {
            let (s, c) = phase.sin_cos();
            phase = (phase + TAU * 10.0 / FS) % TAU;
            shifter.process(x, &coefs, s, c)
        })
        .collect();
    let (up, down) = (tone(&out[24000..], 1010.0), tone(&out[24000..], 990.0));
    assert!(up > 0.4, "{up}");
    assert!(up > 20.0 * down, "{up} vs {down}");
}

#[test]
fn vocoder_wears_the_voices_envelope_and_the_carriers_pitch() {
    // White noise as the voice: no pitch of its own.
    let input = noise(0.3, 96000, 11);
    let s = Settings { vocoder: 1.0, carrier: Carrier::Drone, note: 43.0, wave: 0.0, ..Settings::neutral() };
    let (out, latency) = run(&s, &input);
    let hz = crate::dsp::midi_to_hz(43.0);
    let lag = (FS / hz).round() as usize;
    // Judged in the vocoder's own range, where the rest of the spectrum can't dilute it.
    let x = &out[48000..];
    let (r_out, r_in) = (autocorr(&bandpassed(x, 1000.0, 0.5), lag), autocorr(&bandpassed(&input[48000..], 1000.0, 0.5), lag));
    assert!(r_out > 0.25 && r_in.abs() < 0.05, "{r_out} vs {r_in}");
    // Level: the vocoder is neither much louder nor much quieter than what it speaks.
    let ratio = lin_to_db(rms(&bandpassed(x, 1000.0, 0.5)) / rms(&bandpassed(&input[48000 - latency..96000 - latency], 1000.0, 0.5)));
    assert!(ratio.abs() < 4.0, "{ratio:.1} dB");
}

#[test]
fn vocoder_follows_the_voice_on_and_off() {
    // 100 ms of noise, 100 ms of silence, over and over.
    let gate: Vec<f32> = (0..96000).map(|i| if (i / 4800) % 2 == 0 { 1.0 } else { 0.0 }).collect();
    let input: Vec<f32> = noise(0.3, 96000, 5).iter().zip(&gate).map(|(n, g)| n * g).collect();
    let s = Settings { vocoder: 1.0, carrier: Carrier::Drone, note: 48.0, ..Settings::neutral() };
    let (out, latency) = run(&s, &input);
    // The on-part of the 10th burst (aligned for latency) against the off-part after it.
    let start = 7 * 9600 + latency;
    let on = rms(&out[start + 1200..start + 4000]);
    let off = rms(&out[start + 4800 + 2400..start + 9600 - 400]);
    assert!(on > 0.02 && lin_to_db(on / off.max(1e-9)) > 20.0, "{on} vs {off}");
}

#[test]
fn comb_rings_at_its_pitch() {
    let input = noise(0.2, 96000, 9);
    let lag = (FS / 240.0).round() as usize;
    let measure = |comb: f32| {
        // Wide bands, so most of the noise is in the band the comb sits in.
        let s = Settings { comb, comb_hz: 240.0, low_hz: 80.0, high_hz: 8000.0, ..Settings::neutral() };
        autocorr(&run(&s, &input).0[48000..], lag)
    };
    assert!(measure(1.0) > measure(0.0) + 0.1, "{} vs {}", measure(1.0), measure(0.0));
}

#[test]
fn saturator_adds_harmonics_and_no_dc() {
    let mut sat = Saturator::new(FS);
    let out: Vec<f32> = sine(300.0, 0.5, 48000).iter().map(|&x| sat.process(x, 1.0)).collect();
    let x = &out[24000..];
    assert!(tone(x, 900.0) > 0.02, "{}", tone(x, 900.0));
    let mean = x.iter().sum::<f32>() / x.len() as f32;
    assert!(mean.abs() < 0.01, "{mean}");
    let mut sat = Saturator::new(FS);
    let same: Vec<f32> = sine(300.0, 0.5, 4800).iter().map(|&x| sat.process(x, 0.0)).collect();
    assert_eq!(same, sine(300.0, 0.5, 4800));
}

#[test]
fn squash_flattens_dynamics() {
    let mut squash = Squash::new(FS);
    let mut level = |amp: f32| {
        let out: Vec<f32> = sine(100.0, amp, 24000).iter().map(|&x| squash.process([x, 0.0], 1, 1.0)[0]).collect();
        rms(&out[12000..])
    };
    let (loud, quiet) = (level(0.5), level(0.05));
    // 20 dB apart going in; much closer coming out.
    assert!(lin_to_db(loud / quiet) < 12.0, "{:.1} dB", lin_to_db(loud / quiet));
    let mut squash = Squash::new(FS);
    assert_eq!(squash.process([0.7, 0.0], 1, 0.0), [0.7, 0.0]);
}

#[test]
fn glue_takes_a_few_db_off_loud_signals_and_nothing_off_quiet_ones() {
    let reduction = |amp: f32, amount: f32| {
        let mut glue = Glue::new(FS);
        for x in sine(200.0, amp, 96000) {
            glue.process([x, 0.0], 1, amount);
        }
        glue.reduction()
    };
    let loud = reduction(0.7, 0.4);
    assert!((1.5..8.0).contains(&loud), "{loud:.1} dB");
    assert!(reduction(0.02, 0.4) < 0.1);
    assert!(reduction(0.7, 0.0) < 0.01);
}

#[test]
fn notches_cut_the_harsh_frequencies_only() {
    let measure = |freq: f32, amount: f32| {
        let mut n = Notches::new(FS);
        n.set(amount);
        let out: Vec<f32> = sine(freq, 0.5, 24000).iter().map(|&x| n.process(0, x)).collect();
        lin_to_db(rms(&out[12000..]) / rms(&sine(freq, 0.5, 12000)))
    };
    // The neighbouring cuts' skirts add a little to the 10 dB at the centre.
    assert!((-13.0..-9.5).contains(&measure(4000.0, 1.0)), "{}", measure(4000.0, 1.0));
    assert!(measure(1000.0, 1.0).abs() < 1.0, "{}", measure(1000.0, 1.0));
    assert!(measure(4000.0, 0.0).abs() < 0.01);
}

#[test]
fn body_eq_boosts_weight_and_cuts_box_only_while_it_is_there() {
    let measure = |freq: f32, amp: f32, boxy: f32, weight: f32| {
        let mut eq = BodyEq::new(FS);
        let out: Vec<f32> = sine(freq, amp, 48000)
            .iter()
            .enumerate()
            .map(|(i, &x)| {
                if i % 32 == 0 {
                    eq.update(boxy, weight);
                }
                eq.detect(x);
                eq.process(0, x)
            })
            .collect();
        lin_to_db(rms(&out[24000..]) / rms(&sine(freq, amp, 24000)))
    };
    assert!(measure(60.0, 0.2, 0.0, 1.0) > 5.0);
    assert!(measure(400.0, 0.2, 1.0, 0.0) < -6.0);
    assert!(measure(400.0, 0.001, 1.0, 0.0) > -1.0);
    assert!(measure(400.0, 0.2, 0.0, 0.0).abs() < 0.01);
}

#[test]
fn shine_shifts_the_top_up() {
    let mut shine = Shine::new(FS);
    let ratio = 2f32.powf(3.0 / 12.0);
    let out: Vec<f32> = sine(4000.0, 0.5, 48000).iter().map(|&x| shine.process(x, ratio)).collect();
    let x = &out[24000..];
    assert!(tone(x, 4000.0 * ratio) > 3.0 * tone(x, 4000.0), "{} vs {}", tone(x, 4000.0 * ratio), tone(x, 4000.0));
}

#[test]
fn cavity_echoes_at_the_set_time() {
    let mut cavity = Cavity::new(FS);
    let mut out = Vec::new();
    for i in 0..2000 {
        out.push(cavity.process([if i == 0 { 1.0 } else { 0.0 }, 0.0], 1, 10.0, 1.0)[0]);
    }
    let echo = (1..2000).max_by(|&a, &b| out[a].abs().total_cmp(&out[b].abs())).unwrap();
    assert!((echo as f32 - 480.0).abs() < 4.0, "{echo}");
}

#[test]
fn plate_rings_and_dies_away() {
    let mut plate = Plate::new(FS);
    let mut out = Vec::new();
    for i in 0..60000 {
        out.push(plate.process(if i == 0 { 1.0 } else { 0.0 }, 0.4)[0]);
    }
    let early = rms(&out[1000..6000]);
    let late = rms(&out[24000..30000]);
    assert!(early > 0.002, "{early}");
    assert!(lin_to_db(late / early) < -40.0, "{:.1} dB", lin_to_db(late / early));
    assert!(out.iter().all(|v| v.is_finite()));
}

#[test]
fn limiter_never_passes_the_ceiling() {
    let ceiling = db_to_lin(-3.0);
    let mut limiter = Limiter::new(FS);
    let hot: Vec<f32> = noise(1.5, 48000, 2).iter().zip(sine(220.0, 1.2, 48000)).map(|(n, s)| n + s).collect();
    let out: Vec<f32> = hot.iter().map(|&x| limiter.process([x, x], 2, ceiling)[0]).collect();
    assert!(peak(&out) <= ceiling * 1.0001, "{}", peak(&out));
    assert!(rms(&out) > 0.2, "{}", rms(&out));
}

#[test]
fn limiter_sees_intersample_peaks() {
    // A quarter-rate sine at 45 degrees: every sample is 0.707 and the true peak is 1.0.
    let input: Vec<f32> = (0..9600).map(|i| (TAU * 0.25 * i as f32 + std::f32::consts::FRAC_PI_4).sin()).collect();
    assert!((peak(&input) - 0.7071).abs() < 0.001);
    let ceiling = 0.708;
    let mut limiter = Limiter::new(FS);
    let out: Vec<f32> = input.iter().map(|&x| limiter.process([x, 0.0], 1, ceiling)[0]).collect();
    // Held to the ceiling in the true-peak sense: the samples come out near 0.5, not 0.707.
    assert!(peak(&out[4800..]) < 0.52, "{}", peak(&out[4800..]));
}

#[test]
fn limiter_leaves_quiet_signals_alone_and_reports_its_delay() {
    let mut limiter = Limiter::new(FS);
    let delay = limiter.latency();
    let input = sine(300.0, 0.3, 9600);
    let out: Vec<f32> = input.iter().map(|&x| limiter.process([x, 0.0], 1, 0.9)[0]).collect();
    for i in 0..input.len() - delay {
        assert!((out[i + delay] - input[i]).abs() < 1e-6, "sample {i}");
    }
}

#[test]
fn gate_cuts_the_room_and_opens_before_the_word() {
    let (quiet, loud) = (9600, 14400);
    let mut input = noise(0.003, quiet, 4);
    input.extend(voice(150.0, 0.3, loud));
    let s = Settings { gate_db: -40.0, ..Settings::neutral() };
    let (out, latency) = run(&s, &input);
    let before = rms(&out[latency + 4800..latency + quiet - 100]);
    assert!(lin_to_db(before / rms(&input[4800..quiet - 100])) < -30.0, "{before}");
    // The first 20 ms of the word survives (the gate opened `latency` samples ago).
    let onset = rms(&out[latency + quiet + 200..latency + quiet + 960]);
    let word = rms(&input[quiet + 200..quiet + 960]);
    assert!(onset > 0.6 * word, "{onset} vs {word}");
}

#[test]
fn defaults_stay_under_the_ceiling_and_say_something() {
    let s = Settings::default();
    let ceiling = db_to_lin(s.ceiling_db);
    let input = voice(140.0, 0.5, 96000);
    let (out, _) = run(&s, &input);
    assert!(peak(&out) <= ceiling * 1.0001, "{}", peak(&out));
    assert!(rms(&out[48000..]) > 0.05, "{}", rms(&out[48000..]));
}

#[test]
fn output_stays_finite_at_the_extremes() {
    let mut seed = 12345u32;
    let mut rand = move || {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        (seed >> 8) as f32 / (1u32 << 24) as f32
    };
    let mut input = noise(0.8, 30000, 1);
    input.extend(std::iter::repeat(0.0).take(10000));
    input.extend(voice(110.0, 0.9, 20000));
    input.extend((0..5000).map(|i| if i % 700 == 0 { 1.0 } else { 0.0 }));
    let input_peak = peak(&input);
    for trial in 0..12 {
        let mut pick = |lo: f32, hi: f32| if trial % 3 == 0 { if rand() < 0.5 { lo } else { hi } } else { lo + (hi - lo) * rand() };
        let s = Settings {
            pitch: pick(-8.0, 8.0),
            formant: pick(-12.0, 12.0),
            gate_db: pick(-80.0, -20.0),
            boxy: pick(0.0, 1.0),
            weight: pick(0.0, 1.0),
            low_hz: pick(80.0, 600.0),
            high_hz: pick(1200.0, 8000.0),
            sub: pick(0.0, 1.0),
            squash: pick(0.0, 1.0),
            chest_db: pick(-24.0, 12.0),
            ring: pick(0.0, 1.0),
            ring_hz: pick(15.0, 2000.0),
            vocoder: pick(0.0, 1.0),
            carrier: if pick(0.0, 1.0) < 0.5 { Carrier::Track } else { Carrier::Drone },
            note: pick(24.0, 60.0).round(),
            wave: pick(0.0, 1.0),
            shift_hz: pick(-30.0, 30.0),
            comb: pick(0.0, 1.0),
            comb_hz: pick(60.0, 1200.0),
            grit: pick(0.0, 1.0),
            body_db: pick(-24.0, 12.0),
            cavity: pick(0.0, 1.0),
            cavity_ms: pick(4.0, 40.0),
            shine: pick(0.0, 1.0),
            shine_st: pick(0.0, 12.0),
            plate: pick(0.0, 1.0),
            decay_s: pick(0.1, 1.5),
            shell_db: pick(-24.0, 12.0),
            glue: pick(0.0, 1.0),
            tame: pick(0.0, 1.0),
            push_db: pick(0.0, 12.0),
            ceiling_db: pick(-12.0, 0.0),
            mix: pick(0.0, 1.0),
        };
        for channels in [1usize, 2] {
            let mut chassis = Chassis::new();
            chassis.configure(FS, channels);
            let ceiling = db_to_lin(s.ceiling_db);
            for (i, &x) in input.iter().enumerate() {
                let out = chassis.process([x, x * 0.7], &s);
                assert!(out.iter().all(|v| v.is_finite()), "trial {trial}, {channels} ch, sample {i}: {s:?}");
                // The output is a blend of the limited signal and the delayed input, so it never
                // exceeds the louder of the ceiling and the input's own peak.
                let bound = ceiling.max(input_peak) * 1.001 + 1e-6;
                assert!(out.iter().all(|v| v.abs() <= bound), "trial {trial}, sample {i}: {out:?}");
            }
        }
    }
}

#[test]
fn stereo_runs_both_channels() {
    let input = voice(140.0, 0.4, 48000);
    let mut chassis = Chassis::new();
    chassis.configure(FS, 2);
    let s = Settings::default();
    let out: Vec<[f32; 2]> = input.iter().map(|&x| chassis.process([x, 0.5 * x], &s)).collect();
    let (l, r): (Vec<f32>, Vec<f32>) = out[24000..].iter().map(|v| (v[0], v[1])).unzip();
    assert!(rms(&l) > 0.02 && rms(&r) > 0.02, "{} {}", rms(&l), rms(&r));
}

#[test]
fn meter_publishes_something_sensible() {
    let mut chassis = Chassis::new();
    chassis.configure(FS, 1);
    let s = Settings::default();
    for x in voice(140.0, 0.5, 24000) {
        chassis.process([x, 0.0], &s);
    }
    chassis.publish();
    let m = chassis.meter.clone();
    let loudest = m.output.iter().map(|b| b.load(std::sync::atomic::Ordering::Relaxed)).fold(0.0, f32::max);
    assert!(loudest > 0.01, "{loudest}");
    assert!(m.level.load(std::sync::atomic::Ordering::Relaxed) > 0.05);
    let pitch = m.pitch_hz.load(std::sync::atomic::Ordering::Relaxed);
    assert!((pitch / 140.0 - 1.0).abs() < 0.03, "{pitch}");
    assert!(m.latency.load(std::sync::atomic::Ordering::Relaxed) > 1000.0);
}

#[test]
fn plate_return_is_about_as_loud_as_what_feeds_it() {
    let mut plate = Plate::new(FS);
    let input = noise(0.3, 48000, 8);
    let out: Vec<f32> = input.iter().map(|&x| plate.process(x, 0.4)[0]).collect();
    let ratio = lin_to_db(rms(&out[24000..]) / rms(&input[24000..]));
    assert!((-9.0..3.0).contains(&ratio), "{ratio:.1} dB");
}
