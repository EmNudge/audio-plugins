//! Factory presets. Each lists only what it changes from the defaults (Input 0 dB, Output 0 dB,
//! Depth 100%, Upward 100%, Downward 100%, Time 50%, Floor -42 dB, Ceiling -18 dB, band gains 0 dB,
//! Mix 100%, crossovers 120 Hz and 2.5 kHz).

use nih_plug::prelude::Vst3Plugin;
use preset_kit::{Bank, Preset, V};

use crate::Vice;

pub static BANK: Bank = Bank {
    class_id: <Vice as Vst3Plugin>::VST3_CLASS_ID,
    version: env!("CARGO_PKG_VERSION"),
    keep: &[],
    presets: &[
        Preset { name: "Init", set: &[] },
        // The familiar smile: everything squeezed, lows and highs pushed forward, mids tucked back.
        Preset {
            name: "Classic Smile",
            set: &[("g_low", V::F(4.5)), ("g_mid", V::F(-1.5)), ("g_high", V::F(5.0)), ("ceil", V::F(-20.0)), ("floor", V::F(-40.0))],
        },
        // Mostly upward: brings up the tails and room without touching the peaks.
        Preset {
            name: "Gentle Lift",
            set: &[("depth", V::F(0.5)), ("down", V::F(0.25)), ("time", V::F(0.7)), ("mix", V::F(0.85))],
        },
        // Mostly downward, slow: levels a mix bus without sounding squashed.
        Preset {
            name: "Bus Glue",
            set: &[("depth", V::F(0.45)), ("up", V::F(0.15)), ("ceil", V::F(-14.0)), ("time", V::F(0.65))],
        },
        // Fast and hard: bring up the ghost notes and the cymbal wash, flatten the hits.
        Preset {
            name: "Drum Crush",
            set: &[("time", V::F(0.12)), ("g_low", V::F(3.0)), ("g_high", V::F(4.0)), ("ceil", V::F(-24.0)), ("floor", V::F(-36.0)), ("mix", V::F(0.7))],
        },
        // A touch of density and a lot of air for a vocal.
        Preset {
            name: "Vocal Air",
            set: &[("depth", V::F(0.5)), ("up", V::F(0.8)), ("down", V::F(0.4)), ("g_high", V::F(4.0)), ("x_high", V::F(5000.0)), ("time", V::F(0.55)), ("ceil", V::F(-22.0))],
        },
        // The lows breathe with the kick: a low crossover well up and a long release.
        Preset {
            name: "Sub Pump",
            set: &[("x_low", V::F(200.0)), ("g_low", V::F(6.0)), ("time", V::F(0.85)), ("depth", V::F(0.8)), ("g_high", V::F(-2.0))],
        },
        // Heavy squash blended under the dry signal for weight without losing the transients.
        Preset {
            name: "Parallel Squash",
            set: &[("mix", V::F(0.35)), ("ceil", V::F(-30.0)), ("floor", V::F(-50.0)), ("time", V::F(0.3)), ("g_low", V::F(3.0)), ("g_high", V::F(3.0))],
        },
        // Upward only with a high floor: everything quiet crawls up toward the middle.
        Preset {
            name: "Everything Up",
            set: &[("down", V::F(0.0)), ("floor", V::F(-28.0)), ("ceil", V::F(-12.0)), ("time", V::F(0.8)), ("out", V::F(-3.0))],
        },
        // Barely there: a few dB of density and a hint of lift, safe to leave on a master.
        Preset {
            name: "Silk Glue",
            set: &[("depth", V::F(0.2)), ("up", V::F(0.5)), ("down", V::F(0.5)), ("time", V::F(0.75)), ("ceil", V::F(-10.0)), ("floor", V::F(-48.0)), ("mix", V::F(0.9))],
        },
        // Vocal that stays in your face: quick-ish, mids forward, a little air, rumble left alone.
        Preset {
            name: "Lead Vox",
            set: &[("in", V::F(3.0)), ("depth", V::F(0.65)), ("down", V::F(0.7)), ("time", V::F(0.35)), ("g_low", V::F(-3.0)), ("g_mid", V::F(2.0)), ("g_high", V::F(3.0)), ("x_low", V::F(160.0)), ("x_high", V::F(4000.0)), ("ceil", V::F(-20.0))],
        },
        // Spoken word: pulls up the quiet syllables and breaths, tames sibilance, low end trimmed.
        Preset {
            name: "Clear Speech",
            set: &[("depth", V::F(0.55)), ("up", V::F(1.0)), ("down", V::F(0.5)), ("time", V::F(0.5)), ("floor", V::F(-50.0)), ("ceil", V::F(-24.0)), ("g_low", V::F(-6.0)), ("g_high", V::F(-2.0)), ("x_low", V::F(120.0)), ("x_high", V::F(6000.0)), ("mix", V::F(0.9))],
        },
        // Whole kit through one squeeze: punchy low mids, snap on top, medium release.
        Preset {
            name: "Drum Bus",
            set: &[("depth", V::F(0.7)), ("up", V::F(0.6)), ("time", V::F(0.25)), ("g_low", V::F(2.0)), ("g_high", V::F(2.5)), ("x_low", V::F(100.0)), ("x_high", V::F(3500.0)), ("ceil", V::F(-18.0)), ("mix", V::F(0.8))],
        },
        // Downward only on the lows: tightens a boomy bass and leaves everything above alone.
        Preset {
            name: "Low Lock",
            set: &[("up", V::F(0.0)), ("down", V::F(1.0)), ("time", V::F(0.4)), ("g_low", V::F(-2.0)), ("g_mid", V::F(0.0)), ("g_high", V::F(0.0)), ("x_low", V::F(250.0)), ("x_high", V::F(1000.0)), ("ceil", V::F(-26.0)), ("depth", V::F(0.8))],
        },
        // Guitars that hold their sustain: slow upward, strum bite on the highs, mud cut.
        Preset {
            name: "Strum Sustain",
            set: &[("depth", V::F(0.6)), ("up", V::F(0.9)), ("down", V::F(0.35)), ("time", V::F(0.6)), ("g_low", V::F(-3.0)), ("g_mid", V::F(1.0)), ("g_high", V::F(2.0)), ("x_low", V::F(140.0)), ("x_high", V::F(3000.0)), ("floor", V::F(-46.0))],
        },
        // Synth lead made huge: very fast, input driven, bright top and a pushed mid.
        Preset {
            name: "Lead Laser",
            set: &[("in", V::F(6.0)), ("depth", V::F(0.9)), ("time", V::F(0.05)), ("g_mid", V::F(3.0)), ("g_high", V::F(6.0)), ("x_low", V::F(300.0)), ("x_high", V::F(8000.0)), ("ceil", V::F(-26.0)), ("out", V::F(-4.0))],
        },
        // Pads bloom: the slowest release, upward only, all quiet detail rising and spreading.
        Preset {
            name: "Pad Bloom",
            set: &[("depth", V::F(0.75)), ("down", V::F(0.0)), ("time", V::F(1.0)), ("floor", V::F(-56.0)), ("ceil", V::F(-30.0)), ("g_low", V::F(-2.0)), ("g_high", V::F(3.0)), ("mix", V::F(0.8)), ("out", V::F(-2.0))],
        },
        // Only the highs get lifted: lows and mids pass untouched, air and fizz rise out of the noise floor.
        Preset {
            name: "Air Only",
            set: &[("depth", V::F(0.8)), ("down", V::F(0.0)), ("g_low", V::F(0.0)), ("g_mid", V::F(0.0)), ("g_high", V::F(6.0)), ("x_low", V::F(400.0)), ("x_high", V::F(10000.0)), ("floor", V::F(-52.0)), ("time", V::F(0.6)), ("mix", V::F(0.7))],
        },
        // Scooped mids with the lows and highs shoved forward and a hard squeeze: the extreme smile.
        Preset {
            name: "Mid Scoop",
            set: &[("g_low", V::F(8.0)), ("g_mid", V::F(-10.0)), ("g_high", V::F(8.0)), ("x_low", V::F(80.0)), ("x_high", V::F(5000.0)), ("depth", V::F(1.0)), ("time", V::F(0.3)), ("ceil", V::F(-22.0)), ("out", V::F(-3.0))],
        },
        // Loud and bright for the drop: fast, deep, huge on both ends, pulled back at the output.
        Preset {
            name: "Festival Master",
            set: &[("in", V::F(4.0)), ("depth", V::F(0.85)), ("time", V::F(0.2)), ("g_low", V::F(3.0)), ("g_high", V::F(4.0)), ("x_low", V::F(90.0)), ("x_high", V::F(7000.0)), ("ceil", V::F(-20.0)), ("floor", V::F(-38.0)), ("mix", V::F(0.8)), ("out", V::F(-3.5))],
        },
        // Dusty cassette: narrow band of grit, dark top, noise floor dragged up and out.
        Preset {
            name: "Tape Dust",
            set: &[("in", V::F(8.0)), ("depth", V::F(0.7)), ("time", V::F(0.45)), ("g_low", V::F(2.0)), ("g_mid", V::F(1.0)), ("g_high", V::F(-9.0)), ("x_low", V::F(60.0)), ("x_high", V::F(2000.0)), ("floor", V::F(-60.0)), ("ceil", V::F(-34.0)), ("out", V::F(-6.0))],
        },
        // Hair-trigger everything: shortest time, floor above ceiling, input slammed and output pulled right down.
        Preset {
            name: "Hyperdrive",
            set: &[("in", V::F(12.0)), ("time", V::F(0.0)), ("floor", V::F(-24.0)), ("ceil", V::F(-40.0)), ("g_low", V::F(6.0)), ("g_high", V::F(6.0)), ("out", V::F(-10.0))],
        },
        // Quiet input, ceiling at the top and floor at the bottom: barely any compression, only a mid dip and a long release.
        Preset {
            name: "Wide Open",
            set: &[("in", V::F(-6.0)), ("depth", V::F(0.3)), ("floor", V::F(-60.0)), ("ceil", V::F(0.0)), ("time", V::F(0.95)), ("g_mid", V::F(-4.0)), ("x_low", V::F(40.0)), ("x_high", V::F(1000.0)), ("out", V::F(6.0))],
        },
        // Breathing nightmare: a deep, slow, heavily lifted wash with the dry nearly gone.
        Preset {
            name: "Ghost Choir",
            set: &[("in", V::F(-4.0)), ("up", V::F(1.0)), ("down", V::F(0.0)), ("time", V::F(0.95)), ("floor", V::F(-30.0)), ("ceil", V::F(-10.0)), ("g_high", V::F(-12.0)), ("g_low", V::F(12.0)), ("mix", V::F(0.5)), ("out", V::F(4.0))],
        },
    ],
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ViceParams;

    #[test]
    fn bank_is_valid() {
        BANK.validate(&ViceParams::default()).unwrap();
    }

    #[test]
    fn init_matches_the_defaults() {
        assert_eq!(BANK.current(&ViceParams::default()), Some(0));
    }

    #[test]
    fn state_names_every_parameter() {
        use nih_plug::wrapper::state::ParamValue;
        let state = BANK.state(1, &ViceParams::default());
        assert_eq!(state.params.len(), 14);
        assert!(matches!(state.params["g_low"], ParamValue::F32(g) if (g - 4.5).abs() < 1e-3));
    }
}
