//! Renders the editor to a PNG: `cargo run -p carousel --example screenshot --release [-- out.png]`.

use carousel::dsp::{Chorus, Settings};
use carousel::{ui, CarouselParams};
use std::f32::consts::TAU;
use std::sync::Arc;

const FS: f32 = 48000.0;

fn main() {
    let params = Arc::new(CarouselParams::default());
    let mut chorus = Chorus::new();
    chorus.configure(FS, 2);
    let settings = Settings::default();
    let meter = chorus.meter.clone();

    // A slowly pulsing pad: a detuned chord of harmonic tones.
    let mut t = 0usize;
    let mut state = ui::UiState::default();
    ui_preview::Shot {
        size: (800, 580),
        frames: 90,
        scale: 1.0,
        margin: 16.0,
        background: ui::background(),
        feed: Box::new(move |_| {
            for _ in 0..2048 {
                let time = t as f32 / FS;
                let env = 0.55 + 0.45 * (TAU * 0.9 * time).sin();
                let x: f32 = [110.0f32, 164.8, 220.5, 277.3]
                    .iter()
                    .map(|f| (1..=4).map(|h| (TAU * f * h as f32 * time).sin() / (h * h) as f32).sum::<f32>())
                    .sum();
                let x = x * 0.2 * env;
                chorus.process([x, x], &settings);
                t += 1;
            }
        }),
        draw: Box::new(move |u, setter| ui::draw(u, &params, setter, &meter, &mut state)),
    }
    .save(ui_preview::output_path("carousel"));
}
