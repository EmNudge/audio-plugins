//! Renders the editor to a PNG: `cargo run -p tympan --example screenshot --release [-- out.png]`.
//!
//! A groove is played through the real engine, so the heads, shell ticks and flashes come from the
//! same atomics the plugin writes.

use tympan::engine::{Engine, Global, Meters};
use tympan::{ui, TympanParams};
use std::sync::{Arc, Mutex};

const FS: f32 = 48000.0;

fn main() {
    let params = Arc::new(TympanParams::default());
    let meters = Arc::new(Meters::default());
    let engine = Arc::new(Mutex::new(Engine::default()));
    {
        let mut e = engine.lock().unwrap();
        e.set_sample_rate(FS);
        e.set_global(params.global());
        for h in 0..4 {
            e.set_head(h, params.heads[h].settings());
            e.set_metal(h, params.plates[h].settings());
        }
    }
    let _ = Global::default();
    let mut state = ui::UiState::default();
    let (feed_engine, feed_meters) = (engine.clone(), meters.clone());
    let draw_params = params.clone();
    let draw_meters = meters.clone();
    // One frame is about 16 ms: 768 samples. Hits land on a pattern of kick, snare and toms.
    let pattern: [(usize, usize, f32); 8] = [(0, 0, 1.0), (14, 3, 0.7), (26, 1, 0.95), (34, 0, 0.9), (44, 2, 0.8), (52, 3, 0.85), (60, 1, 1.0), (66, 0, 1.0)];
    // Plates: (frame, plate, velocity).
    let plates: [(usize, usize, f32); 6] = [(2, 2, 0.9), (30, 0, 0.8), (46, 0, 0.7), (50, 1, 0.9), (58, 3, 0.8), (64, 0, 0.9)];
    ui_preview::Shot {
        size: (1000, 818),
        frames: 70,
        scale: 1.0,
        margin: 14.0,
        background: ui::background(),
        feed: Box::new(move |frame| {
            let mut e = feed_engine.lock().unwrap();
            for (at, head, vel) in pattern {
                if at == frame {
                    e.trigger(head, vel, None);
                    feed_meters.hit[head].fetch_max(vel, std::sync::atomic::Ordering::Relaxed);
                }
            }
            for (at, plate, vel) in plates {
                if at == frame {
                    e.trigger_metal(plate, vel, false);
                    feed_meters.metal_hit[plate].fetch_max(vel, std::sync::atomic::Ordering::Relaxed);
                }
            }
            let mut peak = 0.0f32;
            for _ in 0..768 {
                let (l, r) = e.next();
                peak = peak.max(l.abs()).max(r.abs());
            }
            e.publish(&feed_meters);
            feed_meters.out_peak.fetch_max(peak.tanh(), std::sync::atomic::Ordering::Relaxed);
        }),
        draw: Box::new(move |u, setter| ui::draw(u, &draw_params, &draw_meters, setter, &mut state)),
    }
    .save(ui_preview::output_path("tympan"));
}
