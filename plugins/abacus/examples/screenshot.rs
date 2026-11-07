//! Renders the editor to a PNG: `cargo run -p abacus --example screenshot --release [-- out.png]`.

use abacus::dsp::{Resynth, Settings};
use abacus::{ui, AbacusParams};
use nih_plug::prelude::*;
use std::f32::consts::TAU;
use std::sync::Arc;

const FS: f32 = 48000.0;

fn main() {
    // The preview ignores edits, so the state to show is set in the defaults.
    let params = Arc::new(AbacusParams {
        partials: IntParam::new("Partials", 24, IntRange::Linear { min: 1, max: 64 }),
        freeze_freq: BoolParam::new("Freeze Freq", true),
        freeze_amp: BoolParam::new("Freeze Amp", true),
        pitch: FloatParam::new("Pitch", 3.0, FloatRange::Linear { min: -24.0, max: 24.0 }).with_unit(" st").with_value_to_string(formatters::v2s_f32_rounded(2)),
        noise: FloatParam::new("Noise", 0.35, FloatRange::Linear { min: 0.0, max: 1.0 }).with_value_to_string(formatters::v2s_f32_percentage(0)),
        ..AbacusParams::default()
    });
    let mut r = Resynth::new();
    r.configure(FS, 1);
    r.settings = Settings { partials: 24, noise: 0.35, pitch_semitones: 3.0, glide_ms: 120.0, ..Settings::default() };
    let meter = r.meter.clone();

    // A sung vowel: a wandering fundamental with a harmonic series that rolls off, plus breath.
    let (mut phase, mut seed, mut t) = (0.0f32, 0x1234_5678u32, 0usize);
    let mut state = ui::UiState::default();
    ui_preview::Shot {
        size: (820, 600),
        frames: 140,
        scale: 1.0,
        margin: 16.0,
        background: ui::background(),
        feed: Box::new(move |frame| {
            // Latch both freezes near the end, as the display says.
            r.settings.freeze_freq = frame >= 118;
            r.settings.freeze_amp = frame >= 118;
            for _ in 0..1600 {
                let time = t as f32 / FS;
                let f0 = 150.0 * (1.0 + 0.06 * (TAU * 0.35 * time).sin()) * (1.0 + 0.5 * (time * 0.2).min(1.0) * 0.1);
                phase = (phase + f0 / FS).fract();
                let mut x = 0.0;
                for h in 1..=14 {
                    // Two broad formants shape the harmonics.
                    let f = f0 * h as f32;
                    let gain = 0.6 / h as f32 + 0.25 * (-((f - 900.0) / 250.0).powi(2)).exp() + 0.15 * (-((f - 2600.0) / 400.0).powi(2)).exp();
                    x += 0.12 * gain * (TAU * phase * h as f32).sin();
                }
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                x += (seed as f32 / u32::MAX as f32 - 0.5) * 0.01;
                r.process([x, 0.0]);
                t += 1;
            }
        }),
        draw: Box::new(move |u, setter| ui::draw(u, &params, setter, &meter, &mut state)),
    }
    .save(ui_preview::output_path("abacus"));
}
