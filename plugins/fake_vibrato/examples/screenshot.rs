//! Renders the editor to a PNG: `cargo run -p fake_vibrato --example screenshot --release`.

use fake_vibrato::{ui, VibratoParams};
use nih_plug::prelude::AtomicF32;
use std::sync::atomic::Ordering;
use std::sync::Arc;

fn main() {
    let params = Arc::new(VibratoParams::default());
    let phase = Arc::new(AtomicF32::new(0.0));
    let (feed_phase, draw_phase) = (phase.clone(), phase);
    let rate = 5.5_f32;
    ui_preview::Shot {
        size: (520, 380),
        frames: 90,
        scale: 1.0,
        margin: 14.0,
        background: ui::background(),
        // Advance the LFO phase the way the DSP does, one 60 fps frame at a time.
        feed: Box::new(move |_| {
            let p = (feed_phase.load(Ordering::Relaxed) + rate / 60.0).fract();
            feed_phase.store(p, Ordering::Relaxed);
        }),
        draw: Box::new(move |u, setter| ui::draw(u, &params, setter, &draw_phase)),
    }
    .save(ui_preview::output_path("fake_vibrato"));
}
