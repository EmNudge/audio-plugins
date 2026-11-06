//! Renders the editor to a PNG: `cargo run -p aurora_air --example screenshot --release [-- out.png]`.

use aurora_air::dsp::{Analyzer, Chain};
use aurora_air::{ui, AirParams, HIGH_HZ, MID_HIGH_HZ};
use std::f32::consts::TAU;
use std::sync::Arc;

const FS: f32 = 48000.0;

fn main() {
    let params = Arc::new(AirParams::default());
    let mut analyzer = Analyzer::new();
    analyzer.configure(FS);
    let spectrum = analyzer.spectrum.clone();
    let mut chains = [Chain::new(FS, MID_HIGH_HZ), Chain::new(FS, HIGH_HZ)];
    let (mid_high, high) = (params.mid_high.value(), params.high.value() * 1.5);

    // A dull, band-limited mix: harmonic stack with a steep rolloff plus a little hiss.
    let mut seed = 0x9e37_79b9u32;
    let mut t = 0usize;
    let mut state = ui::UiState::default();
    ui_preview::Shot {
        size: (560, 400),
        frames: 90,
        scale: 1.0,
        margin: 14.0,
        background: ui::background(),
        feed: Box::new(move |_| {
            for _ in 0..2048 {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                let hiss = (seed as f32 / u32::MAX as f32 - 0.5) * 0.02;
                let time = t as f32 / FS;
                let swell = 0.7 + 0.3 * (TAU * 0.5 * time).sin();
                let tone: f32 = (1..=40)
                    .map(|k| {
                        let f = 220.0 * k as f32;
                        0.35 / (k as f32).powf(1.3) * (TAU * f * time).sin()
                    })
                    .sum();
                let x = tone * swell + hiss;
                let wet = chains[0].process(x) * mid_high + chains[1].process(x) * high;
                analyzer.push(x, wet);
                t += 1;
            }
        }),
        draw: Box::new(move |u, setter| ui::draw(u, &params, setter, &spectrum, &mut state)),
    }
    .save(ui_preview::output_path("aurora_air"));
}
