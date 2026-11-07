//! Renders the editor to a PNG: `cargo run -p loom --example screenshot --release [-- out.png]`.

use loom::dsp::{HeadSettings, Loom, Settings, HEADS};
use loom::{presets, ui, LoomParams};
use std::f32::consts::TAU;
use std::sync::Arc;

const FS: f32 = 48000.0;

fn main() {
    // Load a preset into the params so the heads on the cloth match the ones in the DSP.
    let preset = presets::BANK.presets.iter().find(|p| p.name == std::env::var("LOOM_PRESET").as_deref().unwrap_or("Whisper Heads")).unwrap();
    let get = |id: &str| {
        preset.set.iter().find(|(k, _)| *k == id).and_then(|(_, v)| match v {
            preset_kit::V::F(x) => Some(*x),
            _ => None,
        })
    };
    let params = Arc::new(LoomParams::with_values(&get));
    let mut settings = Settings { smooth_ms: 0.0, feedback: params.feedback.value(), ..Settings::default() };
    for h in 0..HEADS {
        settings.heads[h] = HeadSettings {
            delay_ms: params.heads[h].delay.value(),
            shift: params.heads[h].shift.value(),
            slide: params.heads[h].slide.value(),
            centre_hz: params.heads[h].centre.value(),
            width_oct: params.heads[h].width.value(),
            level_db: params.heads[h].level.value(),
            pan: params.heads[h].pan.value(),
        };
    }
    // One head gliding, so the slats show a live slant.
    settings.heads[2].slide = 5.0;

    let mut dsp = Loom::new();
    dsp.configure(FS, 2);
    dsp.settings = settings;
    let meter = dsp.meter.clone();

    // A plucked phrase over a pad: a note every 450 ms, each with a few harmonics and a decay.
    let notes = [196.0f32, 247.0, 294.0, 392.0, 330.0, 220.0, 262.0, 349.0];
    let mut t = 0usize;
    let mut seed = 0x1234_5678u32;
    let mut state = ui::UiState::default();
    ui_preview::Shot {
        size: (920, 660),
        frames: 160,
        scale: 1.0,
        margin: 16.0,
        background: ui::background(),
        feed: Box::new(move |_| {
            for _ in 0..2048 {
                let time = t as f32 / FS;
                let step = (time / 0.45) as usize;
                let age = time - step as f32 * 0.45;
                let f = notes[step % notes.len()] * if step % 5 == 4 { 2.0 } else { 1.0 };
                let env = (-age * 4.0).exp();
                let mut x = 0.0;
                for h in 1..=6 {
                    x += (TAU * f * h as f32 * time).sin() / (h * h) as f32 * 1.4;
                }
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                let hiss = (seed as f32 / u32::MAX as f32 - 0.5) * 0.02;
                let pad = 0.05 * (TAU * 110.0 * time).sin();
                let v = x * env * 0.5 + pad + hiss;
                dsp.process([v, v * 0.9]);
                t += 1;
            }
        }),
        draw: Box::new(move |u, setter| ui::draw(u, &params, setter, &meter, &mut state)),
    }
    .save(ui_preview::output_path("loom"));
}
