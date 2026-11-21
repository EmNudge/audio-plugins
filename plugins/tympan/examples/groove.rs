//! Renders a simple groove (kick, snare, closed hats) through the engine to a WAV, as MIDI would
//! play it: `cargo run -p tympan --release --example groove -- <out.wav> [bpm]`.

use tympan::analysis::write_wav;
use nih_plug::prelude::Param;
use tympan::engine::Engine;
use tympan::{soft_limit, TympanParams};

fn main() {
    let mut args = std::env::args().skip(1);
    let out = std::path::PathBuf::from(args.next().expect("usage: groove <out.wav> [bpm]"));
    let bpm: f32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(100.0);
    let sr = 44100.0;
    let params = TympanParams::default();
    let mut e = Engine::default();
    e.set_sample_rate(sr);
    e.set_global(params.global());
    for h in 0..4 {
        e.set_head(h, params.heads[h].settings());
        e.set_metal(h, params.plates[h].settings());
    }
    let gain = 10f32.powf(params.level.default_plain_value() / 20.0);
    let step = (60.0 / bpm / 2.0 * sr) as usize;
    let mut x = Vec::new();
    for bar_step in 0..16 {
        // Eighth notes: kick on 1 and 3, snare on 2 and 4, hats on every eighth.
        if bar_step % 4 == 0 { e.trigger(0, 0.85, None); }
        if bar_step % 4 == 2 { e.trigger(1, 0.75, None); }
        e.trigger_metal(0, if bar_step % 2 == 0 { 0.8 } else { 0.55 }, false);
        for _ in 0..step {
            let (l, r) = e.next();
            x.push(soft_limit((l + r) * 0.5 * gain));
        }
    }
    write_wav(&out, &x, sr).unwrap();
}
