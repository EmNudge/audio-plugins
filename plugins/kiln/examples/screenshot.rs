//! Renders the editor to a PNG: `cargo run -p kiln --example screenshot --release [-- out.png]`.

use kiln::dsp::{Channel, Scope, Settings, SCOPE_N, SCOPE_STRIDE};
use kiln::{tube_gain, ui, KilnParams};
use std::f32::consts::TAU;
use std::sync::atomic::Ordering;
use std::sync::Arc;

const FS: f32 = 48000.0;

fn main() {
    let params = Arc::new(KilnParams::default());
    let scope = Arc::new(Scope::default());
    let mut ch = Channel::new(FS);
    ch.set_crossover(params.xover_low.value(), params.xover_high.value());

    let density = params.density.value();
    let set = Settings {
        style: params.style.value(),
        asym: params.asym.value(),
        dynamics: params.dynamics.value(),
        tubes: [tube_gain(params.pentode.value(), density), tube_gain(params.triode.value(), density)],
        sat: params.sat.value(),
        voice: params.voice.value(),
        xfmr: params.xfmr.value(),
    };
    let gains = [0, 1, 2].map(|b| 10.0f32.powf(params.drive_db(b) / 20.0));

    // A bass note, a chord and some top sparkle, so all three bands move.
    let partials: [(f32, f32); 7] =
        [(55.0, 0.30), (110.0, 0.12), (220.0, 0.12), (277.2, 0.08), (329.6, 0.08), (1318.5, 0.05), (4200.0, 0.03)];
    let (mut t, mut pos, mut tick) = (0usize, 0usize, 0usize);
    let mut state = ui::UiState::default();
    let feed_scope = scope.clone();
    ui_preview::Shot {
        size: (640, 660),
        frames: 90,
        scale: 1.0,
        margin: 14.0,
        background: ui::background(),
        feed: Box::new(move |_| {
            let mut peaks = [0.0f32; 3];
            for _ in 0..2048 {
                let time = t as f32 / FS;
                let env = 1.6 + 1.0 * (TAU * 2.0 * time).sin();
                let x: f32 = env * partials.iter().map(|(f, a)| a * (TAU * f * time).sin()).sum::<f32>();
                let r = ch.process(x, gains, set);
                for b in 0..3 {
                    peaks[b] = peaks[b].max(r.output[b].abs());
                }
                if tick == 0 {
                    for b in 0..3 {
                        feed_scope.input[b][pos].store(r.input[b], Ordering::Relaxed);
                        feed_scope.output[b][pos].store(r.output[b], Ordering::Relaxed);
                    }
                    pos = (pos + 1) % SCOPE_N;
                }
                tick = (tick + 1) % SCOPE_STRIDE;
                t += 1;
            }
            for b in 0..3 {
                feed_scope.level[b].store(peaks[b], Ordering::Relaxed);
            }
        }),
        draw: Box::new(move |u, setter| ui::draw(u, &params, setter, &scope, &mut state)),
    }
    .save(ui_preview::output_path("kiln"));
}
