//! Renders the editor to a PNG: `cargo run -p pocket --example screenshot --release [-- out.png]`.

use pocket::engine::{EventBuf, Sequencer, Transport};
use pocket::{generator, grooves, ui, PocketParams};
use std::sync::Arc;

const FS: f64 = 48000.0;

fn main() {
    let params = Arc::new(PocketParams::default());
    let name = std::env::var("POCKET_GROOVE").unwrap_or_else(|_| "Drummer's Ghosts".into());
    let groove = grooves::library().iter().find(|g| g.name == name).expect("no such groove");
    groove.load(&params.pattern, false);
    if std::env::var("POCKET_BARS").as_deref() == Ok("2") {
        params.pattern.set_bars(2);
    }
    // A little hand-editing on top, so the board shows a nudge, a roll and a lock.
    params.pattern.toggle_lock(pocket::pattern::LANE_KICK);
    params.pattern.toggle_mute(pocket::pattern::LANE_RIDE);
    let _ = generator::STYLES;

    let mut seq = Sequencer::default();
    let meter = seq.meter.clone();
    meter.audition.store(true, std::sync::atomic::Ordering::Relaxed);
    let mut buf = EventBuf::default();
    let feel = pocket::pattern::Feel { swing: groove.swing, eighths: groove.eighths, humanize: 0.0 };
    let store_params = params.clone();
    let transport = Transport { host: None, audition: true, tempo: groove.bpm as f64 };

    let mut state = ui::UiState::default();
    if let Ok(path) = std::env::var("POCKET_PATH") {
        state.browse(&path.split('/').collect::<Vec<_>>());
    }
    let draw_params = params.clone();
    ui_preview::Shot {
        size: (1080, 748),
        frames: 150,
        scale: 1.0,
        margin: 14.0,
        background: ui::background(),
        feed: Box::new(move |_| {
            // One frame is about 16 ms.
            seq.process(&store_params.pattern, feel, transport, FS, 768, &mut buf);
        }),
        draw: Box::new(move |u, setter| ui::draw(u, &draw_params, setter, &meter, &mut state)),
    }
    .save(ui_preview::output_path("pocket"));
}
