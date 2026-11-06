//! Renders the editor to a PNG: `cargo run -p skip --example screenshot --release [-- out.png]`.

use skip::dsp::{Echo, Settings};
use skip::{ui, SkipParams};
use std::f32::consts::TAU;
use std::sync::Arc;

const FS: f32 = 48000.0;

fn main() {
    let params = Arc::new(SkipParams::default());
    let mut echo = Echo::new();
    echo.configure(FS, 2);
    // Close to the Skip Up preset, so the pitch labels and pink wear show.
    let settings = Settings { time_ms: 350.0, feedback: 0.7, pitch: 5.0, drive: 0.3, diffusion: 0.4, mix: 0.45, ..Settings::default() };
    let meter = echo.meter.clone();

    // Plucked notes every 0.9 s: a decaying harmonic tone, so the echoes are visible in the history.
    let mut t = 0usize;
    let mut state = ui::UiState::default();
    ui_preview::Shot {
        size: (760, 560),
        frames: 240,
        scale: 1.0,
        margin: 16.0,
        background: ui::background(),
        feed: Box::new(move |_| {
            for _ in 0..1024 {
                let time = t as f32 / FS;
                let since = time % 0.9;
                let f0 = [220.0, 277.0, 330.0][(time / 0.9) as usize % 3];
                let env = (-since * 9.0).exp();
                let x: f32 = (1..=5).map(|h| (TAU * f0 * h as f32 * time).sin() / h as f32).sum();
                let x = x * 0.3 * env;
                echo.process([x, x], &settings);
                t += 1;
            }
        }),
        draw: Box::new(move |u, setter| ui::draw(u, &params, setter, &meter, &mut state)),
    }
    .save(ui_preview::output_path("skip"));
}
