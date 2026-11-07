//! Renders the editor to a PNG: `cargo run -p smolder --example screenshot --release [-- out.png]`.

use nih_plug::prelude::*;
use smolder::dsp::{Bands, Model, ModSource};
use smolder::{ui, BandParams, SmolderParams};
use std::f32::consts::TAU;
use std::sync::atomic::Ordering;
use std::sync::Arc;

fn main() {
    // The parameters can't be set from outside nih-plug, so the shot swaps in ones whose defaults
    // show the 3-band and modulation sections.
    let mut params = SmolderParams::default();
    params.bands = EnumParam::new("Bands", Bands::Multi);
    params.mod_source = EnumParam::new("Modulation", ModSource::Lfo);
    params.band[0] = BandParams::new(Model::Tape);
    params.band[2] = BandParams::new(Model::Crunch);
    let params = Arc::new(params);

    let live = Arc::new(smolder::Live::default());
    live.bias.store(0.2, Ordering::Relaxed);
    let (feed_live, draw_live) = (live.clone(), live);
    let mut state = ui::UiState::default();
    ui_preview::Shot {
        size: (760, 830),
        frames: 12,
        scale: 1.0,
        margin: 14.0,
        background: ui::background(),
        // Stands in for the LFO pushing the drive around.
        feed: Box::new(move |frame| {
            feed_live.drive_db.store(16.0 + 8.0 * (TAU * frame as f32 / 40.0).sin(), Ordering::Relaxed);
        }),
        draw: Box::new(move |u, setter| ui::draw(u, &params, setter, &draw_live, &mut state)),
    }
    .save(ui_preview::output_path("smolder"));
}
