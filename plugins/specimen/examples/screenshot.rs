//! Renders the editor to a PNG: `cargo run -p specimen --example screenshot --release [-- out.png]`.

use specimen::dsp::{Engine, Evolve, Settings};
use specimen::{ui, SpecimenParams};
use std::f32::consts::TAU;
use std::sync::Arc;

const FS: f32 = 48000.0;
const FRAME: usize = 1024;

/// A voice-like tone: harmonics of `f0` shaped by two formants, plus a little noise.
fn voice(t: f32, f0: f32, formants: [(f32, f32); 2], seed: &mut u32) -> f32 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 17;
    *seed ^= *seed << 5;
    let noise = (*seed as f32 / u32::MAX as f32 - 0.5) * 0.01;
    let mut x = 0.0;
    for h in 1..80 {
        let f = f0 * h as f32;
        if f > 12000.0 {
            break;
        }
        let env: f32 = formants.iter().map(|(c, w)| (-((f - c) / w).powi(2)).exp()).sum::<f32>() + 0.02;
        x += env * (TAU * f * t).sin() / h as f32 * 2.5;
    }
    x * 0.3 + noise
}

fn main() {
    let params = Arc::new(SpecimenParams::default());
    let mut engine = Engine::new();
    engine.configure(FS, 1);
    let meter = engine.meter.clone();
    let ui_meter = meter.clone();
    engine.settings = Settings { position: 0.5, resynth: true, glide_ms: 80.0, ..Settings::default() };

    // Four vowels, each captured into its own slide, then the position drifts through them.
    let vowels = [
        [(700.0, 120.0), (1200.0, 150.0)],
        [(300.0, 80.0), (2300.0, 250.0)],
        [(450.0, 100.0), (800.0, 120.0)],
        [(350.0, 90.0), (3000.0, 300.0)],
    ];
    let mut seed = 0x1234_5678u32;
    let mut t = 0usize;
    let mut frame = 0usize;
    let mut state = ui::UiState::default();
    ui_preview::Shot {
        size: (780, 560),
        frames: 150,
        scale: 1.0,
        margin: 16.0,
        background: ui::background(),
        feed: Box::new(move |_| {
            let vowel = (frame / 12).min(3);
            if frame % 12 == 4 && frame / 12 < 4 {
                meter.request_capture(frame / 12);
            }
            if frame >= 52 {
                engine.settings.evolve = Evolve::Sine;
                engine.settings.rate_hz = 0.5;
                engine.settings.depth = 0.9;
            }
            for _ in 0..FRAME {
                let x = voice(t as f32 / FS, 110.0 + 25.0 * vowel as f32, vowels[vowel], &mut seed);
                engine.process([x, 0.0], 1.0);
                t += 1;
            }
            frame += 1;
        }),
        draw: Box::new(move |u, setter| ui::draw(u, &params, setter, &ui_meter, &mut state)),
    }
    .save(ui_preview::output_path("specimen"));
}
