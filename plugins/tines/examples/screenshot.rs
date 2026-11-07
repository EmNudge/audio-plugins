//! Renders the editor to a PNG: `cargo run -p tines --example screenshot --release [-- out.png]`.

use std::f32::consts::TAU;
use std::sync::Arc;
use tines::dsp::{Resonator, Settings};
use tines::{ui, TinesParams};

const FS: f32 = 48000.0;

fn main() {
    let params = Arc::new(TinesParams::default());
    let mut fx = Resonator::new();
    fx.configure(FS, 2);
    // The defaults, which are what the knobs show (parameter edits are ignored here).
    let settings = Settings::default();
    let meter = fx.meter.clone();

    // A voiced vowel on a gliding fundamental, breathing in and out so the tines light and fade.
    let mut t = 0usize;
    let mut phases = [0.0_f32; 24];
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
                let f0 = 110.0 * (1.0 + 0.02 * (TAU * 0.7 * time).sin());
                let env = (0.5 + 0.5 * (TAU * 1.1 * time).sin()).powi(2);
                let mut x = 0.0;
                for (h, phase) in phases.iter_mut().enumerate() {
                    *phase = (*phase + TAU * f0 * (h + 1) as f32 / FS) % TAU;
                    let f = f0 * (h + 1) as f32;
                    let gain = (-((f - 600.0) / 300.0).powi(2)).exp() + 0.6 * (-((f - 1500.0) / 500.0).powi(2)).exp() + 0.04;
                    x += gain * phase.sin() / (h + 1) as f32 * 3.0;
                }
                let x = x * 0.3 * env;
                fx.process([x, x], &settings);
                t += 1;
            }
        }),
        draw: Box::new(move |u, setter| ui::draw(u, &params, setter, &meter, &mut state)),
    }
    .save(ui_preview::output_path("tines"));
}
