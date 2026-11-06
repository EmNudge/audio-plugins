//! Renders the editor to a PNG: `cargo run -p vault --example screenshot --release [-- out.png]`.

use std::f32::consts::TAU;
use std::sync::Arc;
use vault::dsp::{Reverb, Settings, Space};
use vault::{ui, VaultParams};

const FS: f32 = 48000.0;

fn main() {
    let params = Arc::new(VaultParams::default());
    let mut reverb = Reverb::new();
    reverb.configure(FS, 2);
    let settings = Settings { space: Space::Hall, ..Settings::default() };
    reverb.set(&settings);
    let meter = reverb.meter.clone();

    // Plucked chords every 1.2 s, so the vault shows fresh input over a decaying tail.
    let chords: [[f32; 3]; 3] = [[220.0, 277.2, 329.6], [196.0, 246.9, 293.7], [174.6, 220.0, 261.6]];
    let mut t = 0usize;
    let mut state = ui::UiState::default();
    ui_preview::Shot {
        size: (760, 620),
        frames: 120,
        scale: 1.0,
        margin: 16.0,
        background: ui::background(),
        feed: Box::new(move |_| {
            for _ in 0..2048 {
                let time = t as f32 / FS;
                let n = (time / 1.2) as usize;
                let age = time - n as f32 * 1.2;
                let env = (-age * 5.0).exp() * (1.0 - (-age * 400.0).exp());
                let x: f32 = chords[n % 3]
                    .iter()
                    .map(|f| (TAU * f * time).sin() + 0.3 * (TAU * 2.0 * f * time).sin())
                    .sum::<f32>()
                    * 0.12
                    * env;
                reverb.process([x, x]);
                t += 1;
            }
        }),
        draw: Box::new(move |u, setter| ui::draw(u, &params, setter, &meter, &mut state)),
    }
    .save(ui_preview::output_path("vault"));
}
