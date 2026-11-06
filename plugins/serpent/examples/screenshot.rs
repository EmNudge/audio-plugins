//! Renders the editor to a PNG: `cargo run -p serpent --example screenshot --release [-- out.png]`.

use serpent::dsp::{Analyzer, Engine, Settings, Telemetry};
use serpent::{ui, SerpentParams};
use std::f32::consts::TAU;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;

const FS: f32 = 48000.0;
/// Audio per editor frame. The harness runs frames with a large dt, so the waterfall advances several
/// columns per frame; event timings below are compressed to match.
const PER_FRAME: usize = 800;

fn main() {
    let params = Arc::new(SerpentParams::default());
    let telemetry = Arc::new(Telemetry::default());
    let mut engine = Engine::new(FS);
    let mut analyzer = Analyzer::new(telemetry.clone());
    analyzer.configure(FS);
    telemetry.sample_rate.store(FS, Relaxed);
    let lookahead = (params.lookahead.value() * 0.001 * FS).round() as usize;
    telemetry.latency_samples.store(lookahead as f32, Relaxed);

    // The editor shows the parameters' defaults, so run the DSP with the same values.
    let settings = Settings {
        threshold_db: params.threshold.value(),
        range_db: params.range.value(),
        freq: params.freq.value(),
        width_oct: params.width.value(),
        follow_oct: params.follow.value(),
        attack_ms: params.attack.value(),
        release_ms: params.release.value(),
        lookahead,
        shape: params.shape.value(),
        detect: params.detect.value(),
        listen: params.listen.value(),
    };

    let mut seed = 0x9e37_79b9u32;
    let mut t = 0usize;
    let (mut lp1, mut lp2) = (0.0f32, 0.0f32);
    let tel = telemetry.clone();
    let mut state = ui::UiState::default();
    ui_preview::Shot {
        size: (760, 640),
        frames: 300,
        scale: 1.0,
        margin: 14.0,
        background: ui::background(),
        feed: Box::new(move |_| {
            let (mut gr_peak, mut detect_peak) = (0.0f32, -120.0f32);
            for _ in 0..PER_FRAME {
                let time = t as f32 / FS;
                // Voiced syllables with a slowly gliding pitch and a few harmonics.
                let syl = (0.5 - 0.5 * (TAU * 12.0 * time).cos()).powf(0.6);
                let f0 = 140.0 + 25.0 * (TAU * 0.4 * time).sin();
                let voiced: f32 =
                    (1..=6).map(|h| (0.30 / h as f32) * (TAU * f0 * h as f32 * time).sin()).sum::<f32>() * syl;
                // Sibilant bursts every 0.2 s lasting 0.05 s: noise band-limited around a centre
                // that drifts from one "s" to the next.
                let phase = time % 0.2;
                let env = if phase < 0.05 { (std::f32::consts::PI * phase / 0.05).sin().powf(0.5) } else { 0.0 };
                let burst_idx = (time / 0.2) as usize;
                let centre = [6200.0, 7600.0, 6800.0, 8400.0][burst_idx % 4];
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                let white = seed as f32 / u32::MAX as f32 - 0.5;
                // Crude band-pass: white minus two one-pole low-passes tuned around the centre.
                let a = 1.0 - (-TAU * centre * 0.6 / FS).exp();
                let b = 1.0 - (-TAU * centre * 1.6 / FS).exp();
                lp1 += a * (white - lp1);
                lp2 += b * (white - lp2);
                let hiss = (lp2 - lp1) * 3.2 * env;
                let x = voiced * 0.5 + hiss + white * 0.002;

                let out = engine.process([x, x], 1, &settings);
                analyzer.push(engine.dry[0], engine.wet[0]);
                let _ = out;
                gr_peak = gr_peak.max(engine.gr_db);
                detect_peak = detect_peak.max(engine.detect_db);
                t += 1;
            }
            tel.gr_db.store(gr_peak, Relaxed);
            tel.detect_db.store(detect_peak, Relaxed);
            tel.center_hz.store(engine.center_hz, Relaxed);
        }),
        draw: Box::new(move |u, setter| ui::draw(u, &params, setter, &telemetry, &mut state)),
    }
    .save(ui_preview::output_path("serpent"));
}
