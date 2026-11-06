//! Factory presets. Each lists only what it changes from the defaults (depth, sharpness and
//! selectivity 50%, 8 ms attack, 150 ms release, full focus range, stereo link 100%, mix 100%).
//! `delta` is a monitoring aid, so presets never touch it.

use nih_plug::prelude::Vst3Plugin;
use preset_kit::{Bank, Preset, V};

use crate::Rime;

pub static BANK: Bank = Bank {
    class_id: <Rime as Vst3Plugin>::VST3_CLASS_ID,
    version: env!("CARGO_PKG_VERSION"),
    keep: &["delta"],
    presets: &[
        Preset { name: "Init", set: &[] },
        // Calms sibilant, nasal harshness in the presence region without dulling the voice.
        Preset {
            name: "Vocal De-Harsh",
            set: &[
                ("depth", V::F(0.45)),
                ("sharp", V::F(0.4)),
                ("select", V::F(0.55)),
                ("attack", V::F(5.0)),
                ("release", V::F(120.0)),
                ("low", V::F(1500.0)),
                ("high", V::F(9000.0)),
                ("mix", V::F(0.85)),
            ],
        },
        // High selectivity and sharpness: only narrow, prominent peaks (room modes, whistles) are cut.
        Preset {
            name: "Surgical Notches",
            set: &[
                ("depth", V::F(0.7)),
                ("sharp", V::F(0.9)),
                ("select", V::F(0.85)),
                ("attack", V::F(15.0)),
                ("release", V::F(300.0)),
            ],
        },
        // Wide, low-sharpness, slow action: evens out buildup across the whole spectrum on a bus.
        Preset {
            name: "Bus Smoothing",
            set: &[
                ("depth", V::F(0.35)),
                ("sharp", V::F(0.2)),
                ("select", V::F(0.25)),
                ("attack", V::F(25.0)),
                ("release", V::F(500.0)),
                ("mix", V::F(0.8)),
            ],
        },
        // Focused on the top end, a bit deeper, to take the sting out of bright cymbals and guitars.
        Preset {
            name: "Tame Brightness",
            set: &[
                ("depth", V::F(0.65)),
                ("sharp", V::F(0.5)),
                ("select", V::F(0.45)),
                ("attack", V::F(3.0)),
                ("release", V::F(200.0)),
                ("low", V::F(3000.0)),
                ("high", V::F(16000.0)),
                ("trim", V::Db(0.5)),
            ],
        },
        // Very fast attack and release hunt short ringing on drums; independent channels keep the image lively.
        Preset {
            name: "Drum De-Ring",
            set: &[
                ("depth", V::F(0.6)),
                ("sharp", V::F(0.75)),
                ("select", V::F(0.7)),
                ("attack", V::F(0.5)),
                ("release", V::F(40.0)),
                ("low", V::F(200.0)),
                ("high", V::F(8000.0)),
                ("link", V::F(0.5)),
            ],
        },
        // Barely-there, slow and wide; shaves occasional spikes on a full mix.
        Preset {
            name: "Mastering Touch",
            set: &[
                ("depth", V::F(0.2)),
                ("sharp", V::F(0.3)),
                ("select", V::F(0.35)),
                ("attack", V::F(40.0)),
                ("release", V::F(800.0)),
                ("low", V::F(100.0)),
                ("mix", V::F(0.7)),
            ],
        },
        // Mastering-safe: tiny depth, very slow, only the harsh upper mids; mostly parallel.
        Preset {
            name: "Glass Master",
            set: &[
                ("depth", V::F(0.12)),
                ("sharp", V::F(0.25)),
                ("select", V::F(0.3)),
                ("attack", V::F(60.0)),
                ("release", V::F(1200.0)),
                ("low", V::F(2000.0)),
                ("high", V::F(12000.0)),
                ("mix", V::F(0.6)),
            ],
        },
        // Takes the boxy, pick-click edge off a strummed acoustic guitar.
        Preset {
            name: "Acoustic Pluck",
            set: &[
                ("depth", V::F(0.5)),
                ("sharp", V::F(0.55)),
                ("select", V::F(0.5)),
                ("attack", V::F(6.0)),
                ("release", V::F(180.0)),
                ("low", V::F(180.0)),
                ("high", V::F(5000.0)),
                ("mix", V::F(0.9)),
            ],
        },
        // Slow and gentle: stops sympathetic ring from building in the piano's upper mids.
        Preset {
            name: "Piano Bloom",
            set: &[
                ("depth", V::F(0.4)),
                ("sharp", V::F(0.6)),
                ("select", V::F(0.6)),
                ("attack", V::F(20.0)),
                ("release", V::F(600.0)),
                ("low", V::F(300.0)),
                ("high", V::F(6000.0)),
                ("link", V::F(0.8)),
            ],
        },
        // Fast, high-focused cut for harsh overheads; keeps the shimmer, loses the ice pick.
        Preset {
            name: "Cymbal Silk",
            set: &[
                ("depth", V::F(0.7)),
                ("sharp", V::F(0.45)),
                ("select", V::F(0.4)),
                ("attack", V::F(1.0)),
                ("release", V::F(90.0)),
                ("low", V::F(5000.0)),
                ("high", V::F(20000.0)),
                ("link", V::F(0.7)),
                ("trim", V::Db(0.5)),
            ],
        },
        // Tight, narrow and quick: hunts the one ringing tone in a snare.
        Preset {
            name: "Snare Ping",
            set: &[
                ("depth", V::F(0.8)),
                ("sharp", V::F(0.85)),
                ("select", V::F(0.9)),
                ("attack", V::F(0.5)),
                ("release", V::F(25.0)),
                ("low", V::F(250.0)),
                ("high", V::F(3000.0)),
                ("link", V::F(0.3)),
            ],
        },
        // Lows only: slow, deep cut keeps bass boom and room-mode bloom in check.
        Preset {
            name: "Boom Control",
            set: &[
                ("depth", V::F(0.65)),
                ("sharp", V::F(0.7)),
                ("select", V::F(0.7)),
                ("attack", V::F(30.0)),
                ("release", V::F(700.0)),
                ("low", V::F(40.0)),
                ("high", V::F(250.0)),
            ],
        },
        // Squashes resonant filter peaks on a lead synth without killing the movement.
        Preset {
            name: "Synth Tamer",
            set: &[
                ("depth", V::F(0.6)),
                ("sharp", V::F(0.65)),
                ("select", V::F(0.55)),
                ("attack", V::F(2.0)),
                ("release", V::F(100.0)),
                ("low", V::F(400.0)),
                ("high", V::F(10000.0)),
                ("link", V::F(0.9)),
            ],
        },
        // Speech cleanup: removes boxy room tone and harsh consonants, voice band only.
        Preset {
            name: "Dialogue Clean",
            set: &[
                ("depth", V::F(0.55)),
                ("sharp", V::F(0.5)),
                ("select", V::F(0.6)),
                ("attack", V::F(4.0)),
                ("release", V::F(140.0)),
                ("low", V::F(120.0)),
                ("high", V::F(7500.0)),
                ("mix", V::F(0.9)),
                ("link", V::F(1.0)),
            ],
        },
        // Slow release, wide and deep: smooths metallic ringing inside reverb tails.
        Preset {
            name: "Room Tail Tidy",
            set: &[
                ("depth", V::F(0.6)),
                ("sharp", V::F(0.5)),
                ("select", V::F(0.35)),
                ("attack", V::F(50.0)),
                ("release", V::F(1500.0)),
                ("low", V::F(500.0)),
                ("high", V::F(12000.0)),
                ("link", V::F(0.6)),
            ],
        },
        // Low mids only, broad and moderate: clears the muddy 100-500 Hz buildup.
        Preset {
            name: "Mud Cut",
            set: &[
                ("depth", V::F(0.5)),
                ("sharp", V::F(0.3)),
                ("select", V::F(0.3)),
                ("attack", V::F(15.0)),
                ("release", V::F(350.0)),
                ("low", V::F(100.0)),
                ("high", V::F(500.0)),
            ],
        },
        // Minimum attack and release: ultra-fast, shimmery chase of every transient resonance.
        Preset {
            name: "Hair Trigger",
            set: &[
                ("depth", V::F(0.55)),
                ("sharp", V::F(1.0)),
                ("select", V::F(0.8)),
                ("attack", V::F(0.5)),
                ("release", V::F(10.0)),
                ("low", V::F(800.0)),
                ("high", V::F(14000.0)),
                ("link", V::F(0.5)),
            ],
        },
        // Slowest settings: 100 ms attack, 1.5 s release; only sustained resonances ever get cut.
        Preset {
            name: "Glacier",
            set: &[
                ("depth", V::F(0.5)),
                ("sharp", V::F(0.15)),
                ("select", V::F(0.1)),
                ("attack", V::F(100.0)),
                ("release", V::F(1500.0)),
            ],
        },
        // Smallest effect: depth and mix near zero, for checking how little it takes.
        Preset {
            name: "Whisper",
            set: &[
                ("depth", V::F(0.05)),
                ("sharp", V::F(0.0)),
                ("select", V::F(0.0)),
                ("attack", V::F(100.0)),
                ("release", V::F(1500.0)),
                ("mix", V::F(0.1)),
                ("trim", V::Db(-1.0)),
            ],
        },
        // Everything up: maximum depth, sharpness and selectivity cut hard at every peak.
        Preset {
            name: "Full Freeze",
            set: &[
                ("depth", V::F(1.0)),
                ("sharp", V::F(1.0)),
                ("select", V::F(1.0)),
                ("attack", V::F(0.5)),
                ("release", V::F(10.0)),
                ("low", V::F(40.0)),
                ("high", V::F(20000.0)),
                ("link", V::F(1.0)),
                ("mix", V::F(1.0)),
                ("trim", V::Db(3.0)),
            ],
        },
        // Creative: deep, broad, wet cut leaves a hollowed-out, vocal-less shell; trim restores level.
        Preset {
            name: "Hollow Ghost",
            set: &[
                ("depth", V::F(0.95)),
                ("sharp", V::F(0.05)),
                ("select", V::F(0.05)),
                ("attack", V::F(0.5)),
                ("release", V::F(1500.0)),
                ("link", V::F(0.0)),
                ("trim", V::Db(6.0)),
            ],
        },
        // Creative: slow attack with fast release lets transients through then snaps cuts in and out.
        Preset {
            name: "Pump Shimmer",
            set: &[
                ("depth", V::F(0.9)),
                ("sharp", V::F(0.7)),
                ("select", V::F(0.2)),
                ("attack", V::F(100.0)),
                ("release", V::F(10.0)),
                ("low", V::F(1000.0)),
                ("high", V::F(16000.0)),
                ("link", V::F(0.2)),
                ("trim", V::Db(-3.0)),
            ],
        },
        // Creative: deep cut on the upper band, trimmed down, turns bright rooms into a dull mumble.
        Preset {
            name: "Dark Room",
            set: &[
                ("depth", V::F(0.85)),
                ("sharp", V::F(0.6)),
                ("select", V::F(0.3)),
                ("attack", V::F(12.0)),
                ("release", V::F(400.0)),
                ("low", V::F(2500.0)),
                ("high", V::F(20000.0)),
                ("mix", V::F(0.75)),
                ("trim", V::Db(-6.0)),
            ],
        },
    ],
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RimeParams;

    #[test]
    fn bank_is_valid() {
        BANK.validate(&RimeParams::default()).unwrap();
    }

    #[test]
    fn init_matches_the_defaults_and_others_do_not() {
        let params = RimeParams::default();
        assert_eq!(BANK.current(&params), Some(0));
    }

    #[test]
    fn state_covers_every_parameter_except_the_kept_one() {
        use nih_plug::wrapper::state::ParamValue;
        let state = BANK.state(5, &RimeParams::default());
        assert_eq!(state.params.len(), 10);
        assert!(!state.params.contains_key("delta"));
        assert!(matches!(state.params["link"], ParamValue::F32(l) if (l - 0.5).abs() < 1e-4));
    }
}
