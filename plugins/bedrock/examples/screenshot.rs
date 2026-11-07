//! Renders the editor to a PNG: `cargo run -p bedrock --example screenshot --release [-- out.png]`.

use bedrock::dsp::{Bedrock, Settings};
use bedrock::{ui, BedrockParams};
use std::f32::consts::TAU;
use std::sync::Arc;

const FS: f32 = 48000.0;

fn main() {
    let params = Arc::new(BedrockParams::default());
    let mut bedrock = Bedrock::new();
    bedrock.configure(FS, 2);
    let defaults = Settings::default();
    let settings = Settings { body: params.body.value(), sub: params.sub.value(), floor_hz: defaults.floor_hz };
    let meter = bedrock.meter.clone();

    // A bass line: a saw-ish note every 1.3 s, each plucked and left to decay, with a soft kick on the beat.
    let notes = [55.0f32, 65.4, 82.4, 73.4];
    let mut t = 0usize;
    let mut state = ui::UiState::default();
    ui_preview::Shot {
        size: (560, 420),
        frames: 125,
        scale: 1.0,
        margin: 14.0,
        background: ui::background(),
        feed: Box::new(move |_| {
            for _ in 0..2048 {
                let time = t as f32 / FS;
                let (index, into) = ((time / 1.3) as usize, time % 1.3);
                let f = notes[index % notes.len()];
                let pluck = (0.45 + 0.55 * (-into * 1.4).exp()).min(1.0);
                let bass: f32 = (1..=14).map(|k| (TAU * f * k as f32 * time).sin() / (k as f32).powf(1.1)).sum::<f32>() * 0.14 * pluck;
                let beat = time % 0.65;
                let kick = 0.11 * (-beat * 14.0).exp() * (TAU * (48.0 + 90.0 * (-beat * 30.0).exp()) * beat).sin();
                let x = bass + kick;
                bedrock.process([x, x], &settings);
                t += 1;
            }
            bedrock.publish();
        }),
        draw: Box::new(move |u, setter| ui::draw(u, &params, setter, &meter, &mut state)),
    }
    .save(ui_preview::output_path("bedrock"));
}
