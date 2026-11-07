//! Factory presets. Each lists only what it changes from the defaults (Tube, 12 dB drive, single band, 2x oversampling).

use nih_plug::prelude::Vst3Plugin;
use preset_kit::{Bank, Preset, V};

use crate::Smolder;

const MULTI: (&str, V) = ("bands", V::Name("3-Band"));
const X4: (&str, V) = ("oversamp", V::Name("4x"));

pub static BANK: Bank = Bank {
    class_id: <Smolder as Vst3Plugin>::VST3_CLASS_ID,
    version: env!("CARGO_PKG_VERSION"),
    keep: &[],
    presets: &[
        Preset { name: "Init", set: &[] },
        Preset {
            name: "Warm Tube",
            set: &[("model", V::Name("Tube")), ("drive", V::F(8.0)), ("bias", V::F(0.25)), ("tone", V::F(-0.2))],
        },
        Preset {
            name: "Tape Glue",
            set: &[
                ("model", V::Name("Tape")),
                ("drive", V::F(6.0)),
                ("tone", V::F(-0.3)),
                ("lowcut", V::F(30.0)),
                ("mix", V::F(0.7)),
            ],
        },
        Preset {
            name: "Diode Bite",
            set: &[("model", V::Name("Diode")), ("drive", V::F(18.0)), ("lowcut", V::F(120.0)), ("tone", V::F(0.2))],
        },
        Preset {
            name: "Fuzz Box",
            set: &[
                ("model", V::Name("Crunch")),
                ("drive", V::F(30.0)),
                ("bias", V::F(0.3)),
                ("lowcut", V::F(80.0)),
                ("tone", V::F(-0.3)),
                X4,
            ],
        },
        Preset {
            name: "Wavefolder Lead",
            set: &[("model", V::Name("Fold")), ("drive", V::F(22.0)), ("bias", V::F(0.15)), X4],
        },
        Preset {
            name: "Bit Rot",
            set: &[("model", V::Name("Crush")), ("drive", V::F(6.0)), ("tone", V::F(0.3)), ("oversamp", V::Name("Off"))],
        },
        Preset {
            name: "Even Rectifier",
            set: &[("model", V::Name("Rectify")), ("drive", V::F(16.0)), ("bias", V::F(0.3)), X4],
        },
        Preset {
            name: "Drum Crush",
            set: &[("model", V::Name("Clip")), ("drive", V::F(14.0)), ("lowcut", V::F(90.0)), ("mix", V::F(0.5))],
        },
        Preset {
            name: "Bass Warmth",
            set: &[
                MULTI,
                ("drive", V::F(8.0)),
                ("xover1", V::F(180.0)),
                ("xover2", V::F(2500.0)),
                ("model_1", V::Name("Tape")),
                ("drive_1", V::F(6.0)),
                ("model_2", V::Name("Tube")),
                ("model_3", V::Name("Tube")),
                ("drive_3", V::F(-12.0)),
                ("level_3", V::Db(-3.0)),
            ],
        },
        Preset {
            name: "Vocal Air",
            set: &[
                MULTI,
                ("drive", V::F(10.0)),
                ("xover1", V::F(250.0)),
                ("xover2", V::F(3500.0)),
                ("model_1", V::Name("Tube")),
                ("drive_1", V::F(-18.0)),
                ("level_1", V::Db(-3.0)),
                ("model_2", V::Name("Tube")),
                ("model_3", V::Name("Tape")),
                ("drive_3", V::F(6.0)),
                ("mix", V::F(0.6)),
            ],
        },
        Preset {
            name: "Master Heat",
            set: &[
                MULTI,
                ("drive", V::F(4.0)),
                ("xover1", V::F(150.0)),
                ("xover2", V::F(5000.0)),
                ("model_1", V::Name("Tape")),
                ("model_2", V::Name("Tape")),
                ("model_3", V::Name("Tape")),
                ("drive_3", V::F(-6.0)),
                ("mix", V::F(0.5)),
            ],
        },
        Preset {
            name: "Wobble Drive",
            set: &[
                ("model", V::Name("Tube")),
                ("drive", V::F(14.0)),
                ("modsrc", V::Name("LFO")),
                ("modtgt", V::Name("Drive")),
                ("modamt", V::F(0.5)),
                ("modrate", V::F(2.0)),
            ],
        },
        Preset {
            name: "Envelope Grit",
            set: &[
                ("model", V::Name("Crunch")),
                ("drive", V::F(10.0)),
                ("modsrc", V::Name("Envelope")),
                ("modtgt", V::Name("Drive")),
                ("modamt", V::F(0.7)),
            ],
        },
        Preset {
            name: "Slow Fold",
            set: &[
                ("model", V::Name("Fold")),
                ("drive", V::F(18.0)),
                ("modsrc", V::Name("LFO")),
                ("modtgt", V::Name("Bias")),
                ("modamt", V::F(0.6)),
                ("modrate", V::F(0.4)),
                ("modshape", V::Name("Triangle")),
                X4,
            ],
        },
    ],
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SmolderParams;

    #[test]
    fn bank_is_valid() {
        BANK.validate(&SmolderParams::default()).unwrap();
    }

    #[test]
    fn init_matches_the_defaults() {
        assert_eq!(BANK.current(&SmolderParams::default()), Some(0));
    }
}
