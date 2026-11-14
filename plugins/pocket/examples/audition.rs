//! Renders a groove through a kit to a WAV file, to hear the built-in drums without a DAW:
//! `POCKET_GROOVE="Boom Bap" POCKET_KIT=studio cargo run -p pocket --example audition --release -- out.wav`
//! Kits: studio, rock, jazz, tight, electro, lofi.

use pocket::engine::{EventBuf, Sequencer, Transport};
use pocket::pattern::{Feel, PatternStore};
use pocket::synth::{Kit, KitKind};
use pocket::{generator, grooves};

const FS: f64 = 44100.0;

fn main() {
    let name = std::env::var("POCKET_GROOVE").unwrap_or_else(|_| "Boom Bap".into());
    let kit_name = std::env::var("POCKET_KIT").unwrap_or_else(|_| "studio".into());
    let room: f32 = std::env::var("POCKET_ROOM").ok().and_then(|s| s.parse().ok()).unwrap_or(0.3);
    let kind = match kit_name.as_str() {
        "rock" => KitKind::Rock,
        "jazz" => KitKind::Jazz,
        "tight" => KitKind::Tight,
        "electro" => KitKind::Electro,
        "lofi" => KitKind::LoFi,
        _ => KitKind::Studio,
    };
    let store = PatternStore::default();
    let groove = grooves::library().iter().find(|g| g.name == name).expect("no such groove");
    groove.load(&store, false);
    let _ = generator::STYLES;
    let feel = Feel { swing: groove.swing, eighths: groove.eighths, humanize: 0.25 };
    let tempo = groove.bpm as f64;

    let mut seq = Sequencer::default();
    let mut kit = Kit::new(FS as f32);
    kit.set_kind(kind);
    kit.set_voice(0.0, 1.0, room);
    let mut buf = EventBuf::default();
    let transport = Transport { host: None, audition: true, tempo };
    let bar_seconds = 4.0 * 60.0 / tempo;
    let total = ((bar_seconds * store.bars() as f64 * 4.0 + 2.0) * FS) as usize;
    let mut out: Vec<i16> = Vec::with_capacity(total * 2);
    let block = 256;
    let mut done = 0;
    while done < total {
        seq.process(&store, feel, transport, FS, block, &mut buf);
        let mut next = 0;
        let events = buf.as_slice().to_vec();
        for i in 0..block {
            while next < events.len() && events[next].timing as usize <= i {
                if events[next].on {
                    kit.trigger(events[next].lane as usize, events[next].vel);
                }
                next += 1;
            }
            let (l, r) = kit.next();
            out.push((l.clamp(-1.0, 1.0) * 32767.0) as i16);
            out.push((r.clamp(-1.0, 1.0) * 32767.0) as i16);
        }
        done += block;
    }
    let mut wav = b"RIFF".to_vec();
    wav.extend((36 + out.len() as u32 * 2).to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend([1, 0, 2, 0]);
    wav.extend((FS as u32).to_le_bytes());
    wav.extend((FS as u32 * 4).to_le_bytes());
    wav.extend([4, 0, 16, 0]);
    wav.extend(b"data");
    wav.extend((out.len() as u32 * 2).to_le_bytes());
    for s in &out {
        wav.extend(s.to_le_bytes());
    }
    let path = std::env::args().nth(1).unwrap_or_else(|| "audition.wav".into());
    std::fs::write(&path, wav).expect("write");
    println!("{path}: {name} on the {kit_name} kit, {:.1} s", out.len() as f64 / 2.0 / FS);
}
