//! Renders the editor to a PNG: `cargo run -p chassis --example screenshot --release [-- out.png]`.

use chassis::dsp::{Chassis, Settings};
use chassis::{ui, ChassisParams};
use std::f32::consts::TAU;
use std::sync::Arc;

const FS: f32 = 48000.0;

/// A sung vowel: a harmonic series shaped by three formants, with vibrato and a slow swell.
struct Voice {
    phase: f32,
    t: f32,
}

impl Voice {
    fn next(&mut self) -> f32 {
        let f0 = 138.0 * (1.0 + 0.025 * (TAU * 5.4 * self.t).sin()) * (1.0 + 0.06 * (TAU * 0.35 * self.t).sin());
        self.phase = (self.phase + f0 / FS).fract();
        // The vowel drifts between "ah" and "ee".
        let blend = 0.5 + 0.5 * (TAU * 0.5 * self.t).sin();
        let formants = [(700.0 + (300.0 - 700.0) * blend, 2.0), (1200.0 + (2300.0 - 1200.0) * blend, 1.6), (2600.0 + (3000.0 - 2600.0) * blend, 1.0)];
        let mut x = 0.0;
        for k in 1..=40 {
            let f = k as f32 * f0;
            let shape: f32 = 0.35 + formants.iter().map(|(c, g)| g * (-((f - c) / (0.18 * c + 80.0)).powi(2)).exp()).sum::<f32>();
            x += shape * (TAU * self.phase * k as f32).sin() / (k as f32).powf(0.8);
        }
        self.t += 1.0 / FS;
        x * 0.07 * (0.65 + 0.35 * (TAU * 1.3 * self.t).sin())
    }
}

fn main() {
    let params = Arc::new(ChassisParams::default());
    let mut chassis = Chassis::new();
    chassis.configure(FS, 2);
    let settings = Settings::default();
    let meter = chassis.meter.clone();

    let mut voice = Voice { phase: 0.0, t: 0.0 };
    let mut state = ui::UiState::default();
    ui_preview::Shot {
        size: (1000, 640),
        frames: 120,
        scale: 1.0,
        margin: 16.0,
        background: ui::background(),
        feed: Box::new(move |_| {
            for _ in 0..2048 {
                let x = voice.next();
                chassis.process([x, x], &settings);
            }
            chassis.publish();
        }),
        draw: Box::new(move |u, setter| ui::draw(u, &params, setter, &meter, &mut state)),
    }
    .save(ui_preview::output_path("chassis"));
}
