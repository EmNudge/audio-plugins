//! Factory presets. Each lists only what it changes from the defaults (Tube, 12 dB drive, 30% pentode and triode, flat).

use nih_plug::prelude::Vst3Plugin;
use preset_kit::{Bank, Preset, V};

use crate::Kiln;

pub static BANK: Bank = Bank {
    class_id: <Kiln as Vst3Plugin>::VST3_CLASS_ID,
    version: env!("CARGO_PKG_VERSION"),
    keep: &[],
    presets: &[
        Preset { name: "Init", set: &[] },
        // Gentle even-harmonic warmth, a touch softer on top.
        Preset {
            name: "Warm Bus",
            set: &[("drive", V::F(8.0)), ("low", V::F(2.0)), ("high", V::F(-4.0)), ("dynamics", V::F(-0.2)), ("tone", V::F(-0.15))],
        },
        // Tape saturates the highs first; negative dynamics lets loud hits squash into the glue.
        Preset {
            name: "Tape Glue",
            set: &[
                ("style", V::Name("Tape")),
                ("drive", V::F(10.0)),
                ("mid", V::F(2.0)),
                ("high", V::F(4.0)),
                ("dynamics", V::F(-0.35)),
                ("tone", V::F(-0.1)),
                ("mix", V::F(0.8)),
            ],
        },
        // Only the lows are pushed; the top band sits at -10 dB of effective drive, so it stays clean.
        Preset {
            name: "Iron Low End",
            set: &[
                ("style", V::Name("Iron")),
                ("drive", V::F(14.0)),
                ("low", V::F(8.0)),
                ("mid", V::F(-6.0)),
                ("high", V::F(-24.0)),
                ("xlow", V::F(180.0)),
                ("xhigh", V::F(1800.0)),
                ("tone", V::F(0.1)),
            ],
        },
        // Hard diode clipping blended in underneath the clean signal.
        Preset {
            name: "Parallel Crunch",
            set: &[
                ("style", V::Name("Diode")),
                ("drive", V::F(28.0)),
                ("low", V::F(-6.0)),
                ("mid", V::F(2.0)),
                ("asym", V::F(0.2)),
                ("dynamics", V::F(0.3)),
                ("tone", V::F(0.2)),
                ("mix", V::F(0.4)),
                ("output", V::Db(-2.0)),
            ],
        },
        // Only the top band does anything: a bright, soft shimmer.
        Preset {
            name: "Clean Air",
            set: &[
                ("style", V::Name("Tape")),
                ("drive", V::F(6.0)),
                ("low", V::F(-24.0)),
                ("mid", V::F(-24.0)),
                ("high", V::F(8.0)),
                ("xhigh", V::F(6000.0)),
                ("tone", V::F(0.3)),
                ("mix", V::F(0.5)),
            ],
        },
        // Both tubes leaned on together, iron on both ends, a hot 12AX7 sizzling above 3 kHz.
        Preset {
            name: "Tube Console",
            set: &[
                ("drive", V::F(6.0)),
                ("pentode", V::F(0.45)),
                ("triode", V::F(0.55)),
                ("density", V::F(0.35)),
                ("xfmr", V::F(0.6)),
                ("sat", V::F(0.4)),
                ("satfreq", V::F(3000.0)),
                ("air", V::F(0.35)),
                ("calib", V::Name("Bright")),
                ("mix", V::F(0.7)),
            ],
        },
        // The parallel path alone, no band drive: harmonics only above 5 kHz.
        Preset {
            name: "Valve Sparkle",
            set: &[
                ("drive", V::F(0.0)),
                ("pentode", V::F(0.0)),
                ("triode", V::F(0.0)),
                ("sat", V::F(0.8)),
                ("voice", V::Name("Hot")),
                ("satfreq", V::F(5000.0)),
                ("air", V::F(0.5)),
            ],
        },
        // Transformer thickening on the lows, the tubes barely warm.
        Preset {
            name: "Iron Heart",
            set: &[("drive", V::F(4.0)), ("xfmr", V::F(1.0)), ("pentode", V::F(0.1)), ("triode", V::F(0.2)), ("calib", V::Name("Dark"))],
        },
        // Vocals: lows left clean, a bright calibration and a warm 12AX7 putting presence above 2.5 kHz.
        Preset {
            name: "Vocal Presence",
            set: &[
                ("drive", V::F(8.0)),
                ("low", V::F(-6.0)),
                ("pentode", V::F(0.4)),
                ("triode", V::F(0.3)),
                ("sat", V::F(0.35)),
                ("satfreq", V::F(2500.0)),
                ("air", V::F(0.3)),
                ("calib", V::Name("Bright")),
                ("mix", V::F(0.6)),
            ],
        },
        // Iron on the lows, negative dynamics so hits squash, triode warmth on the body.
        Preset {
            name: "Drum Bus Thump",
            set: &[
                ("style", V::Name("Iron")),
                ("drive", V::F(12.0)),
                ("low", V::F(4.0)),
                ("triode", V::F(0.5)),
                ("density", V::F(0.2)),
                ("xfmr", V::F(0.7)),
                ("dynamics", V::F(-0.3)),
                ("mix", V::F(0.75)),
            ],
        },
        // Weight in the lows and nothing above 1.2 kHz is touched.
        Preset {
            name: "Bass Weight",
            set: &[
                ("style", V::Name("Iron")),
                ("drive", V::F(10.0)),
                ("low", V::F(6.0)),
                ("mid", V::F(-3.0)),
                ("high", V::F(-18.0)),
                ("xlow", V::F(120.0)),
                ("xhigh", V::F(1200.0)),
                ("pentode", V::F(0.2)),
                ("triode", V::F(0.5)),
                ("xfmr", V::F(0.9)),
                ("calib", V::Name("Dark")),
            ],
        },
        // Pentode pushed hard for the odd harmonics, mids forward, highs tamed.
        Preset {
            name: "Guitar Crunch",
            set: &[
                ("drive", V::F(20.0)),
                ("mid", V::F(3.0)),
                ("high", V::F(-4.0)),
                ("asym", V::F(0.2)),
                ("tone", V::F(0.1)),
                ("pentode", V::F(0.8)),
                ("triode", V::F(0.4)),
                ("density", V::F(0.3)),
                ("output", V::Db(-3.0)),
            ],
        },
        // Barely-there saturation for a whole mix: transformers, a little tape squash, a breath of air.
        Preset {
            name: "Mix Bus Glue",
            set: &[
                ("style", V::Name("Tape")),
                ("drive", V::F(4.0)),
                ("dynamics", V::F(-0.4)),
                ("pentode", V::F(0.15)),
                ("triode", V::F(0.25)),
                ("xfmr", V::F(0.4)),
                ("air", V::F(0.15)),
                ("mix", V::F(0.6)),
            ],
        },
        // Only the 12AX7 path works, from 6 kHz up, with a warm voicing and a lot of air.
        Preset {
            name: "Silk Highs",
            set: &[
                ("drive", V::F(0.0)),
                ("pentode", V::F(0.0)),
                ("triode", V::F(0.0)),
                ("sat", V::F(0.5)),
                ("satfreq", V::F(6000.0)),
                ("air", V::F(0.6)),
            ],
        },
        // Everything turned up: both tubes, density, a hot 12AX7 growling from 800 Hz.
        Preset {
            name: "Fuzz Box",
            set: &[
                ("style", V::Name("Diode")),
                ("drive", V::F(34.0)),
                ("asym", V::F(0.5)),
                ("tone", V::F(-0.1)),
                ("pentode", V::F(1.0)),
                ("triode", V::F(0.8)),
                ("density", V::F(0.6)),
                ("sat", V::F(0.6)),
                ("voice", V::Name("Hot")),
                ("satfreq", V::F(800.0)),
                ("output", V::Db(-8.0)),
            ],
        },
        // Band-limited and dark, the way a small speaker sounds: mids pushed, top and bottom cut.
        Preset {
            name: "Lo-Fi Radio",
            set: &[
                ("style", V::Name("Diode")),
                ("drive", V::F(22.0)),
                ("low", V::F(-12.0)),
                ("mid", V::F(6.0)),
                ("high", V::F(-18.0)),
                ("xlow", V::F(300.0)),
                ("xhigh", V::F(3000.0)),
                ("tone", V::F(-0.3)),
                ("pentode", V::F(0.6)),
                ("calib", V::Name("Dark")),
                ("output", V::Db(-4.0)),
            ],
        },
        // Fattens synths: density on both tubes, a warm 12AX7 at 1 kHz for upper harmonics.
        Preset {
            name: "Synth Fatten",
            set: &[
                ("drive", V::F(10.0)),
                ("pentode", V::F(0.5)),
                ("triode", V::F(0.6)),
                ("density", V::F(0.5)),
                ("xfmr", V::F(0.3)),
                ("sat", V::F(0.3)),
                ("satfreq", V::F(1000.0)),
            ],
        },
        // Mastering-level touch: a whisper of everything, half wet.
        Preset {
            name: "Master Polish",
            set: &[
                ("style", V::Name("Tape")),
                ("drive", V::F(2.0)),
                ("dynamics", V::F(-0.2)),
                ("pentode", V::F(0.1)),
                ("triode", V::F(0.15)),
                ("xfmr", V::F(0.25)),
                ("air", V::F(0.3)),
                ("calib", V::Name("Bright")),
                ("mix", V::F(0.5)),
            ],
        },
        // Warm and dull on purpose: dark calibration, tilted down, iron under it.
        Preset {
            name: "Dark Room",
            set: &[
                ("drive", V::F(8.0)),
                ("tone", V::F(-0.4)),
                ("triode", V::F(0.4)),
                ("xfmr", V::F(0.5)),
                ("calib", V::Name("Dark")),
            ],
        },
        // The full chain driven into a console: both tubes, density, transformers at full.
        Preset {
            name: "Overdriven Console",
            set: &[
                ("drive", V::F(18.0)),
                ("asym", V::F(0.1)),
                ("pentode", V::F(0.7)),
                ("triode", V::F(0.7)),
                ("density", V::F(0.5)),
                ("xfmr", V::F(1.0)),
                ("output", V::Db(-5.0)),
            ],
        },
        // Warm harmonics from 4 kHz up on a clean signal, no band drive.
        Preset {
            name: "Harmonic Exciter",
            set: &[
                ("drive", V::F(0.0)),
                ("pentode", V::F(0.0)),
                ("triode", V::F(0.0)),
                ("sat", V::F(0.7)),
                ("satfreq", V::F(4000.0)),
                ("air", V::F(0.4)),
            ],
        },
        Preset {
            name: "Furnace",
            set: &[
                ("style", V::Name("Diode")),
                ("drive", V::F(36.0)),
                ("asym", V::F(0.6)),
                ("dynamics", V::F(-0.5)),
                ("tone", V::F(-0.2)),
                ("output", V::Db(-6.0)),
            ],
        },
    ],
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::KilnParams;

    #[test]
    fn bank_is_valid() {
        BANK.validate(&KilnParams::default()).unwrap();
    }

    #[test]
    fn init_matches_the_defaults_and_others_do_not() {
        let params = KilnParams::default();
        assert_eq!(BANK.current(&params), Some(0));
    }

    #[test]
    fn state_names_every_parameter_with_the_right_kind() {
        use nih_plug::wrapper::state::ParamValue;
        let state = BANK.state(2, &KilnParams::default());
        assert_eq!(state.params.len(), 22);
        assert!(matches!(state.params["style"], ParamValue::I32(1)));
        assert!(matches!(state.params["match"], ParamValue::Bool(true)));
        assert!(matches!(state.params["mix"], ParamValue::F32(m) if (m - 0.8).abs() < 1e-4));
    }
}
