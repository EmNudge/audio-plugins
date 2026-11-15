//! Renders the editor to a PNG: `cargo run -p digger --example screenshot --release [-- out.png]`.
//!
//! With `DIGGER_ROOT=/path/to/folder` it loads real samples; without, it synthesises a small
//! library so the picture can be made anywhere. A groove is played through the real engine so the
//! flashes and playheads come from the same atomics the plugin uses.

use digger::engine::{Engine, SLOTS};
use digger::library::{Category, Library, Sample, PEAKS};
use digger::{fire, ui, DiggerParams};
use std::sync::atomic::Ordering;
use std::sync::Arc;

const FS: u32 = 48000;

/// Deterministic noise.
fn noise(i: usize, salt: u32) -> f32 {
    let mut x = (i as u32).wrapping_mul(2654435761) ^ salt.wrapping_mul(40503);
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    x as f32 / u32::MAX as f32 * 2.0 - 1.0
}

fn make(name: String, secs: f32, f: impl Fn(f32, usize) -> f32) -> Sample {
    let n = (secs * FS as f32) as usize;
    let left: Vec<f32> = (0..n).map(|i| f(i as f32 / FS as f32, i)).collect();
    let top = left.iter().cloned().fold(0.0, f32::max).max(1e-6);
    let peaks = (0..PEAKS)
        .map(|b| left[b * n / PEAKS..((b + 1) * n / PEAKS).max(b * n / PEAKS + 1)].iter().fold(0.0f32, |m, v| m.max(v.abs())) / top)
        .collect();
    let mut sample = Sample::plain(name, left, None, peaks);
    (sample.decay, sample.bright) = digger::tags::measure(&sample.left, FS);
    sample
}

fn synthetic() -> Library {
    let kinds: [(&str, usize, fn(usize, usize) -> Sample); 8] = [
        ("kick", 14, |k, _| {
            let (pitch, decay) = (42.0 + k as f32 * 3.0, 6.0 + k as f32 * 0.7);
            make(format!("kick-{:02}", k + 1), 0.5, move |t, _| (t * (pitch + 140.0 * (-t * 40.0).exp()) * std::f32::consts::TAU).sin() * (-t * decay).exp())
        }),
        ("snare", 12, |k, _| make(format!("snare-{:02}", k + 1), 0.35, move |t, i| (noise(i, k as u32) * 0.7 + (t * 190.0 * std::f32::consts::TAU).sin() * 0.4) * (-t * (14.0 + k as f32)).exp())),
        ("clap", 6, |k, _| make(format!("clap-{:02}", k + 1), 0.3, move |t, i| noise(i, 9 + k as u32) * (-t * 18.0).exp() * (1.0 + 0.8 * (t * 90.0).sin().signum()) * 0.5)),
        ("hat", 9, |k, _| make(format!("hat-{:02}", k + 1), 0.12, move |t, i| noise(i, 3 + k as u32) * (-t * (30.0 + k as f32 * 4.0)).exp())),
        ("hat-open", 5, |k, _| make(format!("hat-open-{:02}", k + 1), 0.5, move |t, i| noise(i, 5 + k as u32) * (-t * 7.0).exp())),
        ("tom", 5, |k, _| make(format!("tom-{:02}", k + 1), 0.4, move |t, _| (t * (90.0 + 25.0 * k as f32) * std::f32::consts::TAU).sin() * (-t * 9.0).exp())),
        ("perc", 8, |k, _| make(format!("perc-{:02}", k + 1), 0.25, move |t, _| (t * (700.0 + 90.0 * k as f32) * std::f32::consts::TAU).sin() * (-t * 22.0).exp())),
        ("cymbal", 4, |k, _| make(format!("cymbal-{:02}", k + 1), 0.9, move |t, i| noise(i, 17 + k as u32) * (-t * 4.0).exp())),
    ];
    let cats = kinds.iter().map(|(name, count, f)| Category { name: name.to_string(), samples: (0..*count).map(|k| f(k, 0)).collect() }).collect();
    Library::new("/Users/you/sounds-sorted".into(), FS, cats)
}

fn main() {
    let params = Arc::new(DiggerParams::default());
    let shared = &params.shared;
    shared.lib.sample_rate.store(FS, Ordering::Relaxed);
    let lib = match std::env::var("DIGGER_ROOT") {
        Ok(root) => {
            shared.set_root(&root);
            digger::library::load_library(std::path::Path::new(&root), FS).expect("could not load DIGGER_ROOT")
        }
        Err(_) => synthetic(),
    };
    digger::tags::auto_tag(&lib);
    *shared.root.lock().unwrap() = lib.root.clone();
    shared.adopt(&lib, false);
    // Pick up a mid-crate position so the sleeves fan either side.
    let focus: usize = std::env::var("DIGGER_FOCUS").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
    shared.set_focus(focus);
    for (slot, idx) in [(0, 4), (1, 5), (2, 2), (3, 3), (4, 1), (5, 2), (6, 4), (7, 1)] {
        shared.set_sample(slot, idx, &lib);
    }
    let text = format!("{} SAMPLES IN {} CATEGORIES", lib.count(), lib.cats.len());
    let lib = Arc::new(lib);
    *shared.lib.current.lock().unwrap() = Some(lib.clone());
    // A few saved kits: today's picks, then two rolls. DIGGER_TAG=808 rolls from that tag.
    let tag = std::env::var("DIGGER_TAG").unwrap_or_else(|_| "808".into());
    if let Some(bit) = lib.tag_bit(&tag, false) {
        shared.toggle_filter(bit);
    }
    lib.toggle_user_tag(shared.selection(focus).0, shared.selection(focus).1, "favourite");
    let mut kits = vec![shared.kit_from_selection("DUSTY", &lib)];
    for (name, seed) in [("PUNCH", 11u32), ("AIRY", 23)] {
        shared.roll_kit(seed.wrapping_mul(2654435761), &lib);
        kits.push(shared.kit_from_selection(name, &lib));
    }
    *shared.kits.lock().unwrap() = kits;
    shared.apply_kit(1, &lib);
    shared.hit.iter().for_each(|h| h.store(0.0, Ordering::Relaxed));
    shared.lib.set_status(text);

    // Eight steps of a groove, one per frame-ish: kick on 1 and 3, snare and clap on 2 and 4.
    let mut engine = Engine::default();
    engine.set_sample_rate(FS as f32);
    let pattern: [&[usize]; 8] = [&[0, 3], &[3], &[3, 2], &[3, 6], &[0, 1, 3], &[3], &[5, 3], &[3, 4]];
    let (feed_params, feed_lib) = (params.clone(), lib.clone());
    let mut step = 0;
    let mut peak_seen = 0.0f32;
    let draw_params = params.clone();
    let mut state = ui::UiState::default();
    ui_preview::Shot {
        size: (1040, 780),
        frames: 150,
        scale: 1.0,
        margin: 14.0,
        background: ui::background(),
        feed: Box::new(move |frame| {
            // One frame is about 16 ms; a step every 9 frames is a 16th at ~104 bpm.
            if frame % 9 == 0 {
                for &slot in pattern[step % 8] {
                    fire(&mut engine, &feed_lib, &feed_params, slot, 0.6 + 0.35 * ((step + slot) % 3) as f32 / 2.0);
                }
                step += 1;
            }
            for _ in 0..768 {
                let (l, r) = engine.next(&feed_lib);
                peak_seen = peak_seen.max(l.abs().max(r.abs()));
            }
            for s in 0..SLOTS {
                feed_params.shared.pos[s].store(engine.playhead(&feed_lib, s), Ordering::Relaxed);
            }
            feed_params.shared.out_peak.fetch_max(peak_seen, Ordering::Relaxed);
            peak_seen = 0.0;
        }),
        draw: Box::new(move |u, setter| ui::draw(u, &draw_params, setter, &mut state)),
    }
    .save(ui_preview::output_path("digger"));
}
