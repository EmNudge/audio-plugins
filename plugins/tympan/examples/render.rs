//! Renders the default kit to WAV files, one per voice, with no host: the plugin's own output.
//!
//! ```text
//! cargo run -p tympan --release --example render -- <out-dir> [sample rate]
//! ```

use tympan::analysis::write_wav;
use tympan::engine::{Engine, Global};
use tympan::TympanParams;

fn main() {
    let mut args = std::env::args().skip(1);
    let out = std::path::PathBuf::from(args.next().expect("usage: render <out-dir> [sample rate]"));
    let sr: f32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(44100.0);
    let params = TympanParams::default();
    let names = ["kick", "snare", "tom-lo", "tom-hi"];
    for (h, name) in names.iter().enumerate() {
        let mut e = Engine::default();
        e.set_sample_rate(sr);
        e.set_global(params.global());
        e.set_head(h, params.heads[h].settings());
        e.trigger(h, 0.9, None);
        let x: Vec<f32> = (0..(2.0 * sr) as usize).map(|_| { let (l, r) = e.next(); (l + r) * 0.5 }).collect();
        write_wav(&out.join(format!("{name}.wav")), &x, sr).unwrap();
    }
    for (m, name) in ["hat", "open-hat", "crash", "ride"].iter().enumerate() {
        let mut e = Engine::default();
        e.set_sample_rate(sr);
        e.set_metal(m, params.plates[m].settings());
        e.trigger_metal(m, 0.9, false);
        let x: Vec<f32> = (0..(3.0 * sr) as usize).map(|_| { let (l, r) = e.next(); (l + r) * 0.5 }).collect();
        write_wav(&out.join(format!("{name}.wav")), &x, sr).unwrap();
    }
}
