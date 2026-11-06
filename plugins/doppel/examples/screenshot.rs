//! Renders the editor to a PNG: `cargo run -p doppel --example screenshot --release [-- out.png]`.

use doppel::dsp::{Doubler, Settings};
use doppel::{ui, DoppelParams};
use std::f32::consts::TAU;
use std::sync::Arc;

const FS: f32 = 48000.0;

fn main() {
    let params = Arc::new(DoppelParams::default());
    let mut doubler = Doubler::new();
    doubler.configure(FS, 2);
    let settings = Settings::default();
    let meter = doubler.meter.clone();

    // A wobbling, syllabic "vocal": a gliding harmonic tone with a phrase envelope.
    let mut t = 0usize;
    let mut state = ui::UiState::default();
    ui_preview::Shot {
        size: (720, 500),
        frames: 90,
        scale: 1.0,
        margin: 16.0,
        background: ui::background(),
        feed: Box::new(move |_| {
            for _ in 0..2048 {
                let time = t as f32 / FS;
                let f0 = 220.0 * (1.0 + 0.12 * (TAU * 0.6 * time).sin());
                let env = (0.5 + 0.5 * (TAU * 2.3 * time).sin()).powf(1.5);
                let x: f32 = (1..=6).map(|h| (TAU * f0 * h as f32 * time).sin() / h as f32).sum();
                let x = x * 0.25 * env;
                doubler.process([x, x], &settings);
                t += 1;
            }
        }),
        draw: Box::new(move |u, setter| ui::draw(u, &params, setter, &meter, &mut state)),
    }
    .save(ui_preview::output_path("doppel"));
}
