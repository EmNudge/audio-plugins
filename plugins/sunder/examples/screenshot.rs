//! Renders the editor to a PNG: `cargo run -p sunder --example screenshot --release [-- out.png]`.

use std::f32::consts::TAU;
use std::sync::Arc;
use sunder::dsp::{RingMod, Settings};
use sunder::{ui, SunderParams};

const FS: f32 = 48000.0;

fn main() {
    let params = Arc::new(SunderParams::default());
    let mut fx = RingMod::new();
    fx.configure(FS, 2);
    let settings = Settings::default();
    let meter = fx.meter.clone();

    // A voiced vowel: harmonics of a gliding fundamental under a slowly breathing envelope.
    let mut t = 0usize;
    let mut phases = [0.0_f32; 16];
    let mut state = ui::UiState::default();
    ui_preview::Shot {
        size: (920, 660),
        frames: 90,
        scale: 1.0,
        margin: 16.0,
        background: ui::background(),
        feed: Box::new(move |_| {
            for _ in 0..2048 {
                let time = t as f32 / FS;
                let f0 = 196.0 * (1.0 + 0.03 * (TAU * 0.7 * time).sin());
                let env = 0.6 + 0.4 * (TAU * 1.3 * time).sin();
                let mut x = 0.0;
                for (h, phase) in phases.iter_mut().enumerate() {
                    *phase = (*phase + TAU * f0 * (h + 1) as f32 / FS) % TAU;
                    // A formant bump around 700 Hz and another near 1.8 kHz.
                    let f = f0 * (h + 1) as f32;
                    let gain = (-((f - 700.0) / 350.0).powi(2)).exp() + 0.5 * (-((f - 1800.0) / 500.0).powi(2)).exp() + 0.05;
                    x += gain * phase.sin() / (h + 1) as f32 * 3.0;
                }
                let x = x * 0.3 * env;
                fx.process([x, x], [0.0; 2], &settings);
                t += 1;
            }
        }),
        draw: Box::new(move |u, setter| ui::draw(u, &params, setter, &meter, &mut state)),
    }
    .save(ui_preview::output_path("sunder"));
}
