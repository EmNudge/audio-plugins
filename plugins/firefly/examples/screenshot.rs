//! Renders the editor to a PNG: `cargo run -p firefly --example screenshot --release [-- out.png]`.

use firefly::dsp::{Compressor, Settings};
use firefly::{ui, FireflyParams};
use std::f32::consts::TAU;
use std::sync::Arc;

const FS: f32 = 48000.0;
const FRAME: usize = 4800; // kittest steps 0.25 s per frame but egui caps dt at 0.1 s

fn main() {
    let params = Arc::new(FireflyParams::default());
    let mut comp = Compressor::new();
    comp.configure(FS, 1);
    let settings = Settings::default();
    let meter = comp.meter.clone();

    // A four-on-the-floor bass-and-chord pulse with a louder accent every bar, so the photocell's fast
    // and slow memory both show.
    let mut t = 0usize;
    let mut state = ui::UiState::default();
    ui_preview::Shot {
        size: (760, 520),
        frames: 90,
        scale: 1.0,
        margin: 16.0,
        background: ui::background(),
        feed: Box::new(move |_| {
            for _ in 0..FRAME {
                let time = t as f32 / FS;
                let beat = time * 1.5;
                let phase = beat.fract();
                let bar = (beat as usize) % 4;
                let accent = if bar % 2 == 0 { 1.0 } else { 0.6 };
                let env = (-phase * 5.0).exp() * (1.0 - (-phase * 400.0).exp());
                let tone = (TAU * 55.0 * time).sin() + 0.5 * (TAU * 110.0 * time).sin() + 0.25 * (TAU * 1320.0 * time).sin();
                let x = 0.9 * accent * env * tone;
                comp.process([x, 0.0], &settings);
                t += 1;
            }
            comp.publish(FRAME);
        }),
        draw: Box::new(move |u, setter| ui::draw(u, &params, setter, &meter, &mut state)),
    }
    .save(ui_preview::output_path("firefly"));
}
