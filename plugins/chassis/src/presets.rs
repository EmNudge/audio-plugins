//! Factory presets. Each lists only what it changes from the defaults (Pitch -3 st, Formant -1 st,
//! Gate -55 dB, Box 50%, Weight 40%, splits 250 Hz and 2.8 kHz, Sub 35%, Squash 60%, Ring 20% at
//! 45 Hz, Vocoder 25% on a tracking carrier, Shift +8 Hz, Comb 30% at 180 Hz, Grit 25%, Cavity 35%
//! at 14 ms, Shine 40% at +3 st, Plate 30% at 0.4 s, Glue 40%, Tame 40%, Push +3 dB, Ceiling
//! -1 dB, Mix 100%).

use nih_plug::prelude::Vst3Plugin;
use preset_kit::{Bank, Preset, V};

use crate::ChassisPlugin;

pub static BANK: Bank = Bank {
    class_id: <ChassisPlugin as Vst3Plugin>::VST3_CLASS_ID,
    version: env!("CARGO_PKG_VERSION"),
    keep: &[],
    presets: &[
        // The recipe: a deeper voice with a chest, a little ring, a little vocoder, a metal shell.
        Preset { name: "Init", set: &[] },
        // Deep, wide and heavy: everything pushed toward the chest.
        Preset {
            name: "Mech Overlord",
            set: &[
                ("pitch", V::F(-4.0)),
                ("formant", V::F(-3.0)),
                ("sub", V::F(0.6)),
                ("squash", V::F(0.8)),
                ("ring", V::F(0.3)),
                ("ring_hz", V::F(40.0)),
                ("vocoder", V::F(0.35)),
                ("shift", V::F(6.0)),
                ("comb", V::F(0.4)),
                ("comb_hz", V::F(120.0)),
                ("grit", V::F(0.4)),
                ("cavity", V::F(0.4)),
                ("shine", V::F(0.35)),
                ("plate", V::F(0.35)),
                ("glue", V::F(0.5)),
                ("push", V::F(4.0)),
            ],
        },
        // Small, bright and clangy: pitched up, formants up, no weight at all.
        Preset {
            name: "Tin Can",
            set: &[
                ("pitch", V::F(2.0)),
                ("formant", V::F(4.0)),
                ("low", V::F(450.0)),
                ("high", V::F(2000.0)),
                ("weight", V::F(0.0)),
                ("sub", V::F(0.0)),
                ("ring", V::F(0.5)),
                ("ring_hz", V::F(120.0)),
                ("vocoder", V::F(0.2)),
                ("comb", V::F(0.5)),
                ("comb_hz", V::F(400.0)),
                ("grit", V::F(0.3)),
                ("cavity", V::F(0.5)),
                ("cavity_ms", V::F(8.0)),
                ("shine", V::F(0.7)),
                ("shine_st", V::F(5.0)),
                ("plate", V::F(0.4)),
                ("decay", V::F(0.3)),
            ],
        },
        // A full-strength 30 Hz ring modulator and little else.
        Preset {
            name: "Exterminate",
            set: &[
                ("pitch", V::F(-1.0)),
                ("formant", V::F(0.0)),
                ("sub", V::F(0.0)),
                ("squash", V::F(0.3)),
                ("ring", V::F(1.0)),
                ("ring_hz", V::F(30.0)),
                ("vocoder", V::F(0.0)),
                ("shift", V::F(0.0)),
                ("comb", V::F(0.0)),
                ("grit", V::F(0.5)),
                ("cavity", V::F(0.2)),
                ("shine", V::F(0.2)),
                ("plate", V::F(0.2)),
            ],
        },
        // The vocoder on a fixed low note: a choir of one, singing C2 whatever you say.
        Preset {
            name: "Drone Choir",
            set: &[
                ("pitch", V::F(0.0)),
                ("formant", V::F(0.0)),
                ("sub", V::F(0.2)),
                ("ring", V::F(0.0)),
                ("vocoder", V::F(1.0)),
                ("carrier", V::Name("Drone")),
                ("note", V::F(36.0)),
                ("wave", V::F(0.5)),
                ("shift", V::F(0.0)),
                ("comb", V::F(0.0)),
                ("cavity", V::F(0.3)),
                ("shine", V::F(0.5)),
                ("plate", V::F(0.5)),
                ("decay", V::F(1.0)),
            ],
        },
        // The vocoder following your own pitch, on a bright square wave.
        Preset {
            name: "Talking Synth",
            set: &[
                ("pitch", V::F(0.0)),
                ("formant", V::F(0.0)),
                ("ring", V::F(0.0)),
                ("vocoder", V::F(0.9)),
                ("wave", V::F(0.7)),
                ("shift", V::F(4.0)),
                ("comb", V::F(0.2)),
                ("grit", V::F(0.4)),
                ("plate", V::F(0.2)),
            ],
        },
        // All chest: a sub-octave and a hard squash under a lowered voice.
        Preset {
            name: "Reactor Core",
            set: &[
                ("pitch", V::F(-6.0)),
                ("formant", V::F(-4.0)),
                ("weight", V::F(0.8)),
                ("sub", V::F(1.0)),
                ("squash", V::F(1.0)),
                ("chest", V::F(3.0)),
                ("ring", V::F(0.1)),
                ("vocoder", V::F(0.15)),
                ("shift", V::F(0.0)),
                ("comb", V::F(0.1)),
                ("shine", V::F(0.2)),
                ("plate", V::F(0.15)),
            ],
        },
        // Nearly human: a hint of each part, so the voice is just slightly wrong.
        Preset {
            name: "Android",
            set: &[
                ("pitch", V::F(-1.0)),
                ("ring", V::F(0.08)),
                ("ring_hz", V::F(60.0)),
                ("vocoder", V::F(0.12)),
                ("shift", V::F(5.0)),
                ("comb", V::F(0.15)),
                ("grit", V::F(0.1)),
                ("cavity", V::F(0.2)),
                ("shine", V::F(0.25)),
                ("plate", V::F(0.15)),
                ("glue", V::F(0.3)),
                ("tame", V::F(0.3)),
                ("push", V::F(2.0)),
            ],
        },
        // A voice in a very large, very empty metal room.
        Preset {
            name: "Server Hall",
            set: &[
                ("cavity", V::F(0.5)),
                ("cavity_ms", V::F(22.0)),
                ("shine", V::F(0.6)),
                ("plate", V::F(0.8)),
                ("decay", V::F(1.2)),
                ("shell", V::F(0.0)),
                ("vocoder", V::F(0.3)),
            ],
        },
        // The chest and the shell turned down: only the middle is left, ringing and gritty.
        Preset {
            name: "Intercom Droid",
            set: &[
                ("low", V::F(600.0)),
                ("high", V::F(1200.0)),
                ("chest", V::F(-12.0)),
                ("shell", V::F(-12.0)),
                ("body", V::F(3.0)),
                ("sub", V::F(0.0)),
                ("ring", V::F(0.3)),
                ("ring_hz", V::F(90.0)),
                ("vocoder", V::F(0.3)),
                ("grit", V::F(0.6)),
                ("comb", V::F(0.35)),
                ("comb_hz", V::F(300.0)),
            ],
        },
        // With no voice to track, the vocoder's carrier is noise: a whisper with a robot's edges.
        Preset {
            name: "Whisper Bot",
            set: &[
                ("gate", V::F(-45.0)),
                ("pitch", V::F(0.0)),
                ("formant", V::F(0.0)),
                ("sub", V::F(0.1)),
                ("ring", V::F(0.0)),
                ("vocoder", V::F(1.0)),
                ("wave", V::F(0.0)),
                ("shift", V::F(0.0)),
                ("comb", V::F(0.0)),
                ("cavity", V::F(0.3)),
                ("plate", V::F(0.4)),
                ("decay", V::F(0.8)),
            ],
        },
        // Pushed until it breaks: grit, a fast shifter, a bright ring and the notches off.
        Preset {
            name: "Blown Speaker",
            set: &[
                ("ring", V::F(0.4)),
                ("ring_hz", V::F(200.0)),
                ("shift", V::F(20.0)),
                ("comb", V::F(0.7)),
                ("comb_hz", V::F(300.0)),
                ("grit", V::F(1.0)),
                ("tame", V::F(0.0)),
                ("push", V::F(6.0)),
            ],
        },
        // As low and as large as the shifter goes, with a long slap off the walls.
        Preset {
            name: "Colossus",
            set: &[
                ("pitch", V::F(-8.0)),
                ("formant", V::F(-8.0)),
                ("sub", V::F(0.8)),
                ("squash", V::F(0.7)),
                ("shine", V::F(0.0)),
                ("cavity", V::F(0.5)),
                ("cavity_ms", V::F(30.0)),
                ("plate", V::F(0.5)),
                ("decay", V::F(0.9)),
            ],
        },
        // Up and bright, with the weight taken out and a lot of glitter on top.
        Preset {
            name: "Tiny Robot",
            set: &[
                ("pitch", V::F(5.0)),
                ("formant", V::F(6.0)),
                ("weight", V::F(0.0)),
                ("sub", V::F(0.0)),
                ("chest", V::F(-6.0)),
                ("ring", V::F(0.15)),
                ("ring_hz", V::F(300.0)),
                ("comb", V::F(0.25)),
                ("comb_hz", V::F(600.0)),
                ("cavity", V::F(0.4)),
                ("cavity_ms", V::F(6.0)),
                ("shine", V::F(0.8)),
                ("shine_st", V::F(7.0)),
                ("plate", V::F(0.35)),
            ],
        },
        // Only the chest and the cleanup: weight, a sub, a squash, and no robot yet.
        Preset {
            name: "Just Weight",
            set: &[
                ("pitch", V::F(-2.0)),
                ("boxy", V::F(0.7)),
                ("weight", V::F(0.7)),
                ("sub", V::F(0.3)),
                ("squash", V::F(0.5)),
                ("ring", V::F(0.0)),
                ("vocoder", V::F(0.0)),
                ("shift", V::F(0.0)),
                ("comb", V::F(0.0)),
                ("grit", V::F(0.15)),
                ("cavity", V::F(0.0)),
                ("shine", V::F(0.0)),
                ("plate", V::F(0.0)),
            ],
        },
    ],
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ChassisParams;

    #[test]
    fn bank_is_valid() {
        BANK.validate(&ChassisParams::default()).unwrap();
    }

    #[test]
    fn init_matches_the_defaults() {
        assert_eq!(BANK.current(&ChassisParams::default()), Some(0));
    }

    #[test]
    fn state_names_every_parameter() {
        use nih_plug::wrapper::state::ParamValue;
        let state = BANK.state(1, &ChassisParams::default());
        assert_eq!(state.params.len(), 33);
        assert!(matches!(state.params["carrier"], ParamValue::I32(0)));
        let drone = BANK.state(4, &ChassisParams::default());
        assert!(matches!(drone.params["carrier"], ParamValue::I32(1)));
    }

    #[test]
    fn every_preset_stays_under_its_ceiling() {
        use crate::dsp::{Carrier, Chassis, Settings};
        let params = ChassisParams::default();
        let input: Vec<f32> = (0..48000)
            .map(|i| {
                let t = i as f32 / 48000.0;
                0.5 * (1..=16).map(|k| (std::f32::consts::TAU * 140.0 * k as f32 * t).sin() / k as f32).sum::<f32>()
            })
            .collect();
        for (n, preset) in BANK.presets.iter().enumerate() {
            let state = BANK.state(n, &params);
            let get = |id: &str| match state.params[id] {
                nih_plug::wrapper::state::ParamValue::F32(v) => v,
                _ => unreachable!(),
            };
            let mut s = Settings::default();
            s.pitch = get("pitch");
            s.formant = get("formant");
            s.gate_db = get("gate");
            s.low_hz = get("low");
            s.high_hz = get("high");
            s.sub = get("sub");
            s.squash = get("squash");
            s.chest_db = get("chest");
            s.ring = get("ring");
            s.ring_hz = get("ring_hz");
            s.vocoder = get("vocoder");
            s.carrier = if matches!(state.params["carrier"], nih_plug::wrapper::state::ParamValue::I32(1)) { Carrier::Drone } else { Carrier::Track };
            s.note = get("note");
            s.wave = get("wave");
            s.shift_hz = get("shift");
            s.comb = get("comb");
            s.comb_hz = get("comb_hz");
            s.grit = get("grit");
            s.body_db = get("body");
            s.cavity = get("cavity");
            s.cavity_ms = get("cavity_ms");
            s.shine = get("shine");
            s.shine_st = get("shine_st");
            s.plate = get("plate");
            s.decay_s = get("decay");
            s.shell_db = get("shell");
            s.glue = get("glue");
            s.tame = get("tame");
            s.push_db = get("push");
            s.ceiling_db = get("ceiling");
            s.mix = get("mix");
            let mut chassis = Chassis::new();
            chassis.configure(48000.0, 1);
            let ceiling = crate::filters::db_to_lin(s.ceiling_db);
            let out: Vec<f32> = input.iter().map(|&x| chassis.process([x, 0.0], &s)[0]).collect();
            let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            let rms = (out[24000..].iter().map(|v| v * v).sum::<f32>() / 24000.0).sqrt();
            assert!(peak <= ceiling * 1.001, "{}: peak {peak}", preset.name);
            assert!(out.iter().all(|v| v.is_finite()), "{}", preset.name);
            assert!(rms > 0.02, "{}: rms {rms}", preset.name);
        }
    }
}
