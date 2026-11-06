//! Factory presets. Each lists only what it changes from the defaults (Time 45 ms, Drift 35%,
//! Rate 0.7 Hz, Tape 40%, Width 50%, Double -3 dB).

use nih_plug::prelude::Vst3Plugin;
use preset_kit::{Bank, Preset, V};

use crate::Doppel;

pub static BANK: Bank = Bank {
    class_id: <Doppel as Vst3Plugin>::VST3_CLASS_ID,
    version: env!("CARGO_PKG_VERSION"),
    keep: &[],
    presets: &[
        Preset { name: "Init", set: &[] },
        // The 1966 sound: a copy a little over 40 ms behind, wandering gently, with plenty of tape.
        Preset {
            name: "Abbey Road",
            set: &[("time", V::F(42.0)), ("drift", V::F(0.4)), ("rate", V::F(0.6)), ("tape", V::F(0.55)), ("width", V::F(0.3))],
        },
        // Close enough to fuse with the lead: thickens a vocal without sounding like an echo.
        Preset {
            name: "Tight Double",
            set: &[("time", V::F(18.0)), ("drift", V::F(0.2)), ("rate", V::F(1.2)), ("tape", V::F(0.25)), ("width", V::F(0.4)), ("level", V::Db(-4.0))],
        },
        // Barely there: fills out a lead or a guitar without being heard as a second part.
        Preset {
            name: "Subtle Thickener",
            set: &[("time", V::F(25.0)), ("drift", V::F(0.3)), ("rate", V::F(0.8)), ("tape", V::F(0.2)), ("width", V::F(0.6)), ("level", V::Db(-9.0))],
        },
        // A long, dark copy spread across the stereo field: a slapback you can hear in the width.
        Preset {
            name: "Slapback Wide",
            set: &[("time", V::F(95.0)), ("drift", V::F(0.15)), ("rate", V::F(0.5)), ("tape", V::F(0.6)), ("width", V::F(0.8)), ("level", V::Db(-6.0))],
        },
        // Two independent wandering copies, fast and wide: a small choir from one voice.
        Preset {
            name: "Stereo Choir",
            set: &[("time", V::F(30.0)), ("drift", V::F(0.7)), ("rate", V::F(1.4)), ("tape", V::F(0.3)), ("width", V::F(1.0)), ("level", V::Db(-2.0))],
        },
        // A machine with a failing capstan: slow, deep wander and a lot of tape.
        Preset {
            name: "Drunk Tape",
            set: &[("time", V::F(60.0)), ("drift", V::F(1.0)), ("rate", V::F(0.25)), ("tape", V::F(0.9)), ("width", V::F(0.7)), ("level", V::Db(-4.0))],
        },
        // Fast, shallow warble and a dull, saturated copy.
        Preset {
            name: "Worn Cassette",
            set: &[("time", V::F(70.0)), ("drift", V::F(0.8)), ("rate", V::F(3.0)), ("tape", V::F(1.0)), ("width", V::F(0.3)), ("level", V::Db(-5.0))],
        },
        // A hair behind the lead, almost no wander: just enough to blur the edges of a vocal.
        Preset {
            name: "Vocal Glue",
            set: &[("time", V::F(14.0)), ("drift", V::F(0.15)), ("rate", V::F(0.9)), ("tape", V::F(0.15)), ("width", V::F(0.35)), ("level", V::Db(-7.0))],
        },
        // Lead vocal: a clearly human second take, slightly late and slightly different each pass.
        Preset {
            name: "Lead Take Two",
            set: &[("time", V::F(32.0)), ("drift", V::F(0.5)), ("rate", V::F(0.9)), ("tape", V::F(0.3)), ("width", V::F(0.55)), ("level", V::Db(-5.0))],
        },
        // Backing vocals: loose, wide and a bit louder so the stack reads as several singers.
        Preset {
            name: "Backing Crowd",
            set: &[("time", V::F(38.0)), ("drift", V::F(0.65)), ("rate", V::F(1.1)), ("tape", V::F(0.35)), ("width", V::F(0.9)), ("level", V::Db(0.0))],
        },
        // Spoken word: short, quiet and dark enough to add body without smearing consonants.
        Preset {
            name: "Narrator",
            set: &[("time", V::F(10.0)), ("drift", V::F(0.1)), ("rate", V::F(0.6)), ("tape", V::F(0.5)), ("width", V::F(0.2)), ("level", V::Db(-10.0))],
        },
        // The classic guitar trick: a second pass panned out wide, with a touch of tape grit.
        Preset {
            name: "Guitar Pair",
            set: &[("time", V::F(22.0)), ("drift", V::F(0.35)), ("rate", V::F(0.7)), ("tape", V::F(0.45)), ("width", V::F(0.85)), ("level", V::Db(-3.0))],
        },
        // Synth pads: slow, deep drift so a static chord seems to breathe, with clean repeats.
        Preset {
            name: "Pad Breath",
            set: &[("time", V::F(50.0)), ("drift", V::F(0.8)), ("rate", V::F(0.15)), ("tape", V::F(0.1)), ("width", V::F(1.0)), ("level", V::Db(-3.0))],
        },
        // Drums-safe: short, quiet, stable and mono-friendly, so transients stay in one place.
        Preset {
            name: "Drums Safe",
            set: &[("time", V::F(8.0)), ("drift", V::F(0.0)), ("rate", V::F(0.5)), ("tape", V::F(0.2)), ("width", V::F(0.0)), ("level", V::Db(-12.0))],
        },
        // A fast, shallow wobble that turns the doubler into something close to a chorus.
        Preset {
            name: "Chorus Ghost",
            set: &[("time", V::F(12.0)), ("drift", V::F(0.6)), ("rate", V::F(2.2)), ("tape", V::F(0.0)), ("width", V::F(0.9)), ("level", V::Db(-4.0))],
        },
        // Rockabilly echo: one dark repeat well behind the dry, steady as a metronome.
        Preset {
            name: "Rockabilly",
            set: &[("time", V::F(120.0)), ("drift", V::F(0.0)), ("rate", V::F(0.3)), ("tape", V::F(0.7)), ("width", V::F(0.2)), ("level", V::Db(-5.0))],
        },
        // Tape that has been left in a hot car: slow lurching pitch and heavy saturation.
        Preset {
            name: "Warped Reel",
            set: &[("time", V::F(80.0)), ("drift", V::F(1.0)), ("rate", V::F(0.1)), ("tape", V::F(0.75)), ("width", V::F(0.5)), ("level", V::Db(-3.0))],
        },
        // Lo-fi bedroom: murky copy, nervous wobble, kept low under the dry.
        Preset {
            name: "Bedroom Lo-Fi",
            set: &[("time", V::F(55.0)), ("drift", V::F(0.9)), ("rate", V::F(4.0)), ("tape", V::F(0.95)), ("width", V::F(0.4)), ("level", V::Db(-8.0))],
        },
        // The shortest delay, no movement, no tape: a clean comb filter for metallic thickening.
        Preset {
            name: "Comb Lock",
            set: &[("time", V::F(5.0)), ("drift", V::F(0.0)), ("rate", V::F(0.1)), ("tape", V::F(0.0)), ("width", V::F(0.0)), ("level", V::Db(-6.0))],
        },
        // Everything wide open: fast, deep, saturated and hotter than the dry. Not subtle.
        Preset {
            name: "Hall Of Mirrors",
            set: &[("time", V::F(100.0)), ("drift", V::F(1.0)), ("rate", V::F(4.0)), ("tape", V::F(1.0)), ("width", V::F(1.0)), ("level", V::Db(6.0))],
        },
        // The copy sits almost 60 dB down: a ghost you feel more than hear, a check that nothing is added.
        Preset {
            name: "Whisper",
            set: &[("time", V::F(40.0)), ("drift", V::F(0.3)), ("rate", V::F(0.7)), ("tape", V::F(0.0)), ("width", V::F(0.5)), ("level", V::Db(-40.0))],
        },
        // Seasick vibrato from a short delay: deep drift at a fast rate with no tape colour.
        Preset {
            name: "Seasick",
            set: &[("time", V::F(20.0)), ("drift", V::F(1.0)), ("rate", V::F(3.2)), ("tape", V::F(0.05)), ("width", V::F(0.6)), ("level", V::Db(0.0))],
        },
        // A second voice that is clearly a different machine: long, wide, slow and loud.
        Preset {
            name: "Other Room",
            set: &[("time", V::F(110.0)), ("drift", V::F(0.55)), ("rate", V::F(0.35)), ("tape", V::F(0.5)), ("width", V::F(1.0)), ("level", V::Db(-1.0))],
        },
    ],
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DoppelParams;

    #[test]
    fn bank_is_valid() {
        BANK.validate(&DoppelParams::default()).unwrap();
    }

    #[test]
    fn init_matches_the_defaults() {
        assert_eq!(BANK.current(&DoppelParams::default()), Some(0));
    }

    #[test]
    fn state_names_every_parameter() {
        use nih_plug::wrapper::state::ParamValue;
        let state = BANK.state(1, &DoppelParams::default());
        assert_eq!(state.params.len(), 6);
        assert!(matches!(state.params["time"], ParamValue::F32(t) if (t - 42.0).abs() < 1e-3));
    }
}
