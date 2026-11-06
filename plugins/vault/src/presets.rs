//! Factory presets. Each lists only what it changes from the defaults (Hall, Size 50%, Pre-delay
//! 12 ms, Decay 2.5 s, Low x1, High x0.7, crossovers 300 Hz and 4 kHz, Diffusion 70%, Modulation 30%,
//! Rate 0.6 Hz, Shift 0, Early 50%, Width 100%, Duck 0, cuts off, Mix 30%).

use nih_plug::prelude::Vst3Plugin;
use preset_kit::{Bank, Preset, V};

use crate::Vault;

pub static BANK: Bank = Bank {
    class_id: <Vault as Vst3Plugin>::VST3_CLASS_ID,
    version: env!("CARGO_PKG_VERSION"),
    keep: &[],
    presets: &[
        Preset { name: "Init", set: &[] },
        // A small wooden room: short, early-heavy, dark at the top.
        Preset {
            name: "Wood Room",
            set: &[("space", V::Name("Room")), ("size", V::F(0.3)), ("predelay", V::F(4.0)), ("decay", V::F(0.9)), ("high_mult", V::F(0.5)), ("early", V::F(0.8)), ("diffusion", V::F(0.6)), ("mix", V::F(0.22))],
        },
        // Stone walls and a high ceiling: a slow bloom with warm lows.
        Preset {
            name: "Stone Chapel",
            set: &[("size", V::F(0.65)), ("predelay", V::F(20.0)), ("decay", V::F(3.8)), ("low_mult", V::F(1.25)), ("high_mult", V::F(0.5)), ("early", V::F(0.45)), ("mix", V::F(0.28))],
        },
        Preset {
            name: "Cathedral",
            set: &[("size", V::F(1.0)), ("predelay", V::F(35.0)), ("decay", V::F(8.0)), ("low_mult", V::F(1.3)), ("high_mult", V::F(0.4)), ("xo_high", V::F(3000.0)), ("early", V::F(0.35)), ("mod_depth", V::F(0.4)), ("mix", V::F(0.32))],
        },
        // Dense, bright, no early reflections: the sound of a steel sheet.
        Preset {
            name: "Bright Plate",
            set: &[("space", V::Name("Plate")), ("size", V::F(0.5)), ("decay", V::F(1.8)), ("low_mult", V::F(0.8)), ("high_mult", V::F(1.1)), ("diffusion", V::F(0.95)), ("mod_depth", V::F(0.45)), ("mix", V::F(0.25))],
        },
        // Out of the way of a lead vocal: low cut, a pre-delay and ducking under the singing.
        Preset {
            name: "Vocal Plate",
            set: &[("space", V::Name("Plate")), ("predelay", V::F(24.0)), ("decay", V::F(1.5)), ("high_mult", V::F(0.8)), ("duck", V::F(0.55)), ("low_cut", V::F(200.0)), ("high_cut", V::F(9000.0)), ("mix", V::F(0.22))],
        },
        Preset {
            name: "Brick Chamber",
            set: &[("space", V::Name("Chamber")), ("size", V::F(0.55)), ("predelay", V::F(8.0)), ("decay", V::F(2.2)), ("low_mult", V::F(1.1)), ("early", V::F(0.8)), ("mod_depth", V::F(0.2)), ("mix", V::F(0.3))],
        },
        // A huge, slow, drifting.
        Preset {
            name: "Abyss",
            set: &[("space", V::Name("Abyss")), ("size", V::F(0.8)), ("predelay", V::F(40.0)), ("decay", V::F(18.0)), ("high_mult", V::F(0.35)), ("xo_high", V::F(2500.0)), ("early", V::F(0.0)), ("mod_depth", V::F(0.55)), ("mod_rate", V::F(0.25)), ("mix", V::F(0.4))],
        },
        // Every pass lifts the tail by a few Hz, so it spirals upward and thins into glass.
        Preset {
            name: "Spiral Up",
            set: &[("size", V::F(0.7)), ("predelay", V::F(25.0)), ("decay", V::F(9.0)), ("high_mult", V::F(0.85)), ("shift", V::F(9.0)), ("mod_depth", V::F(0.4)), ("early", V::F(0.2)), ("mix", V::F(0.4))],
        },
        Preset {
            name: "Spiral Down",
            set: &[("space", V::Name("Abyss")), ("size", V::F(0.55)), ("decay", V::F(7.0)), ("low_mult", V::F(1.2)), ("high_mult", V::F(0.6)), ("shift", V::F(-14.0)), ("early", V::F(0.0)), ("mix", V::F(0.4))],
        },
        // A big hall that gets out of the way while you play and blooms in the gaps.
        Preset {
            name: "Ducked Hall",
            set: &[("size", V::F(0.75)), ("predelay", V::F(30.0)), ("decay", V::F(4.5)), ("duck", V::F(0.8)), ("early", V::F(0.3)), ("mix", V::F(0.38))],
        },
        // Nearly all low end: a rumbling cave that swallows the highs.
        Preset {
            name: "Dark Cave",
            set: &[("space", V::Name("Abyss")), ("size", V::F(0.6)), ("decay", V::F(12.0)), ("low_mult", V::F(1.6)), ("high_mult", V::F(0.15)), ("xo_low", V::F(500.0)), ("xo_high", V::F(1800.0)), ("early", V::F(0.0)), ("mod_depth", V::F(0.5)), ("mix", V::F(0.35))],
        },
        // Tight and dry enough for a kit: short, early-heavy, ducked under the hits.
        Preset {
            name: "Drum Room",
            set: &[("space", V::Name("Room")), ("size", V::F(0.4)), ("predelay", V::F(2.0)), ("decay", V::F(0.7)), ("high_mult", V::F(0.6)), ("early", V::F(0.9)), ("duck", V::F(0.35)), ("low_cut", V::F(120.0)), ("mix", V::F(0.25))],
        },
        // Short and bright with a little pre-delay, so the snare crack stays up front.
        Preset {
            name: "Snare Plate",
            set: &[("space", V::Name("Plate")), ("size", V::F(0.35)), ("predelay", V::F(8.0)), ("decay", V::F(1.1)), ("high_mult", V::F(1.2)), ("diffusion", V::F(0.9)), ("low_cut", V::F(250.0)), ("mix", V::F(0.28))],
        },
        Preset {
            name: "Tiny Booth",
            set: &[("space", V::Name("Room")), ("size", V::F(0.0)), ("predelay", V::F(1.0)), ("decay", V::F(0.3)), ("high_mult", V::F(0.5)), ("early", V::F(0.7)), ("diffusion", V::F(0.5)), ("mod_depth", V::F(0.0)), ("mix", V::F(0.18))],
        },
        // Air around a vocal: bright, mostly out of the lows, with a gentle bloom.
        Preset {
            name: "Vocal Air",
            set: &[("size", V::F(0.55)), ("predelay", V::F(28.0)), ("decay", V::F(2.2)), ("low_mult", V::F(0.6)), ("high_mult", V::F(1.1)), ("xo_low", V::F(500.0)), ("low_cut", V::F(300.0)), ("early", V::F(0.25)), ("duck", V::F(0.3)), ("mix", V::F(0.24))],
        },
        // A big hall that is all low end and no top, for a heavy dub throw.
        Preset {
            name: "Dub Chamber",
            set: &[("space", V::Name("Chamber")), ("size", V::F(0.7)), ("predelay", V::F(45.0)), ("decay", V::F(4.5)), ("low_mult", V::F(0.7)), ("high_mult", V::F(0.5)), ("low_cut", V::F(350.0)), ("high_cut", V::F(4500.0)), ("mod_depth", V::F(0.5)), ("mod_rate", V::F(0.4)), ("mix", V::F(0.4))],
        },
        // A sheet of steel in a tank: huge for a plate, thin and rippling.
        Preset {
            name: "Metal Tank",
            set: &[("space", V::Name("Plate")), ("size", V::F(0.9)), ("decay", V::F(3.5)), ("low_mult", V::F(0.5)), ("high_mult", V::F(1.3)), ("diffusion", V::F(0.3)), ("mod_depth", V::F(0.8)), ("mod_rate", V::F(1.5)), ("mix", V::F(0.3))],
        },
        // The tail arrives late and grows: a long pre-delay and a slow, dark bloom.
        Preset {
            name: "Slow Swell",
            set: &[("space", V::Name("Abyss")), ("size", V::F(0.6)), ("predelay", V::F(180.0)), ("decay", V::F(10.0)), ("high_mult", V::F(0.45)), ("diffusion", V::F(1.0)), ("early", V::F(0.0)), ("mod_depth", V::F(0.4)), ("mix", V::F(0.45))],
        },
        // Set up to freeze: long, dark and lush, so a held chord makes a pad.
        Preset {
            name: "Frozen Pad",
            set: &[("space", V::Name("Abyss")), ("size", V::F(0.75)), ("predelay", V::F(20.0)), ("decay", V::F(25.0)), ("low_mult", V::F(1.1)), ("high_mult", V::F(0.4)), ("early", V::F(0.0)), ("mod_depth", V::F(0.6)), ("mod_rate", V::F(0.3)), ("mix", V::F(0.5))],
        },
        // A touch of upward shift: shimmer without the obvious pitch.
        Preset {
            name: "Subtle Shimmer",
            set: &[("size", V::F(0.65)), ("predelay", V::F(22.0)), ("decay", V::F(5.0)), ("high_mult", V::F(0.9)), ("shift", V::F(3.0)), ("mod_depth", V::F(0.35)), ("early", V::F(0.2)), ("mix", V::F(0.35))],
        },
        // A fast frequency shift per pass: the tail twists like a barber pole.
        Preset {
            name: "Barber Pole",
            set: &[("space", V::Name("Abyss")), ("size", V::F(0.5)), ("decay", V::F(6.0)), ("high_mult", V::F(0.7)), ("shift", V::F(60.0)), ("mod_depth", V::F(0.3)), ("early", V::F(0.0)), ("mix", V::F(0.35))],
        },
    ],
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::VaultParams;

    #[test]
    fn bank_is_valid() {
        BANK.validate(&VaultParams::default()).unwrap();
    }

    #[test]
    fn init_matches_the_defaults() {
        assert_eq!(BANK.current(&VaultParams::default()), Some(0));
    }

    #[test]
    fn state_names_every_parameter() {
        use nih_plug::wrapper::state::ParamValue;
        let state = BANK.state(1, &VaultParams::default());
        assert_eq!(state.params.len(), 19);
        assert!(matches!(state.params["decay"], ParamValue::F32(t) if (t - 0.9).abs() < 1e-3));
    }
}
