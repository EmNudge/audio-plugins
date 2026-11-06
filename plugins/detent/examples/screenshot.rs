//! Renders the editor to a PNG: `cargo run -p detent --example screenshot --release [-- out.png]`.

use detent::control::{Settings};
use detent::engine::Engine;
use detent::scale::{self, Scale};
use detent::{ui, DetentParams, Telemetry};
use nih_plug::prelude::EnumParam;
use std::f32::consts::TAU;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;

const FS: f32 = 48000.0;
/// Audio per editor frame (60 fps).
const BLOCK: usize = 800;

/// A sung line through C major: (note, seconds), with a slide into each note.
const MELODY: [(f32, f32); 7] = [(57.0, 0.9), (60.0, 0.8), (64.0, 0.9), (62.0, 0.7), (67.0, 1.0), (65.0, 0.8), (64.0, 1.2)];

fn main() {
    let mut params = DetentParams::default();
    params.scale = EnumParam::new("Scale", Scale::Major);
    let params = Arc::new(params);
    let telemetry = Arc::new(Telemetry::default());
    telemetry.sample_rate.store(FS, Relaxed);

    let mut engine = Engine::new(FS, 75.0, false);
    telemetry.latency_samples.store(engine.latency() as f32, Relaxed);
    let mask = scale::mask(0, Scale::Major, [false; 12]);
    let settings = Settings { mask, retune_ms: 20.0, humanize: 0.35, amount: 1.0, detune_cents: 0.0, formant_semitones: 0.0 };

    let total: f32 = MELODY.iter().map(|m| m.1).sum();
    let mut t = 0usize;
    let mut phase = 0.0f32;
    let tele = telemetry.clone();
    let mut state = ui::UiState::default();
    let p = params.clone();
    ui_preview::Shot {
        size: (700, 600),
        frames: 330,
        scale: 1.0,
        margin: 14.0,
        background: ui::background(),
        feed: Box::new(move |_| {
            for _ in 0..BLOCK {
                let time = t as f32 / FS;
                // Find the current note and the previous one for a short glide.
                let (mut acc, mut idx) = (0.0, 0);
                while idx + 1 < MELODY.len() && time % total >= acc + MELODY[idx].1 {
                    acc += MELODY[idx].1;
                    idx += 1;
                }
                let local = time % total - acc;
                let prev = if idx == 0 { MELODY[0].0 } else { MELODY[idx - 1].0 };
                let glide = (local / 0.12).min(1.0);
                let glide = glide * glide * (3.0 - 2.0 * glide);
                // Sung pitch: off-key drift, vibrato and a little wobble, in semitones.
                let mut m = prev + (MELODY[idx].0 - prev) * glide;
                m += 0.35 * (TAU * 0.6 * time).sin() + 0.25 * (TAU * 5.3 * time).sin() * (local / 0.4).min(1.0);
                let f0 = 440.0 * 2f32.powf((m - 69.0) / 12.0);
                phase = (phase + f0 / FS).fract();
                let mut x = 0.0;
                for h in 1..=8 {
                    x += (TAU * phase * h as f32).sin() / h as f32;
                }
                engine.process([0.25 * x, 0.0], 1, &settings);
                t += 1;
            }
            let last = engine.last;
            tele.held_mask.store(0, Relaxed);
            tele.voiced.store(last.voiced, Relaxed);
            tele.input_midi.store(last.input_midi, Relaxed);
            tele.target_midi.store(last.target_midi, Relaxed);
            tele.output_midi.store(last.output_midi, Relaxed);
        }),
        draw: Box::new(move |u, setter| ui::draw(u, &p, setter, &telemetry, &mut state)),
    }
    .save(ui_preview::output_path("detent"));
}
