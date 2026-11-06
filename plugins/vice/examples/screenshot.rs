//! Renders the editor to a PNG: `cargo run -p vice --example screenshot --release [-- out.png]`.

use std::f32::consts::TAU;
use std::sync::Arc;
use vice::dsp::{Settings, Squeeze};
use vice::{ui, ViceParams};

const FS: f32 = 48000.0;
const BLOCK: usize = 512;

fn main() {
    let params = Arc::new(ViceParams::default());
    let mut sq = Squeeze::new();
    sq.configure(FS, 2);
    let settings = Settings::default();
    let meter = sq.meter.clone();

    let mut seed = 0x1234_5678u32;
    let mut t = 0usize;
    let mut state = ui::UiState::default();
    ui_preview::Shot {
        size: (760, 540),
        frames: 90,
        scale: 1.0,
        margin: 16.0,
        background: ui::background(),
        feed: Box::new(move |_| {
            for _ in 0..4 {
                for _ in 0..BLOCK {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    let noise = seed as f32 / u32::MAX as f32 - 0.5;
                    let time = t as f32 / FS;
                    // Kick-like pulsing bass, steady chordal mids, hi-hat bursts on the off-beat.
                    let beat = (time * 2.0).fract();
                    let bass = 0.4 * (-beat * 6.0).exp() * (TAU * 55.0 * time).sin();
                    let mid = 0.025 * ((TAU * 440.0 * time).sin() + 0.7 * (TAU * 660.0 * time).sin());
                    let hat_env = (-((time * 4.0 + 0.5).fract()) * 18.0).exp();
                    let hat = 0.12 * hat_env * noise;
                    let x = bass + mid + hat;
                    sq.process([x, x], &settings);
                    t += 1;
                }
                sq.publish(BLOCK);
            }
        }),
        draw: Box::new(move |u, setter| ui::draw(u, &params, setter, &meter, &mut state)),
    }
    .save(ui_preview::output_path("vice"));
}
