//! Renders the editor to a PNG: `cargo run -p rime --example screenshot --release [-- out.png]`.

use rime::dsp::{Settings, Suppressor};
use rime::{ui, RimeParams};
use std::f32::consts::TAU;
use std::sync::Arc;

const FS: f32 = 48000.0;

fn main() {
    let params = Arc::new(RimeParams::default());
    let mut sup = Suppressor::new();
    sup.configure(FS, 1);
    sup.settings = Settings { depth: 0.75, sharpness: 0.6, ..Settings::default() };
    let meter = sup.meter.clone();

    // Quiet noise with four ringing resonances on top, like a boxy vocal.
    let mut seed = 0x1234_5678u32;
    let mut t = 0usize;
    let mut state = ui::UiState::default();
    ui_preview::Shot {
        size: (780, 560),
        frames: 90,
        scale: 1.0,
        margin: 16.0,
        background: ui::background(),
        feed: Box::new(move |_| {
            for _ in 0..2048 {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                let noise = (seed as f32 / u32::MAX as f32 - 0.5) * 0.04;
                let time = t as f32 / FS;
                let ring: f32 = [(420.0, 0.10), (1250.0, 0.13), (3100.0, 0.09), (6900.0, 0.06)]
                    .iter()
                    .map(|(f, a)| a * (TAU * f * time).sin())
                    .sum();
                sup.process([noise + ring, 0.0]);
                t += 1;
            }
        }),
        draw: Box::new(move |u, setter| ui::draw(u, &params, setter, &meter, &mut state)),
    }
    .save(ui_preview::output_path("rime"));
}
