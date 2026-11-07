//! Factory presets. Each lists only what it changes from the defaults (Free, 440 Hz, Diode,
//! Sine, Drive 40%, Bias 0, Sideband both, Follow 0, Wobble 0 at 1.5 Hz, Spread 0, Tone off,
//! Mix 100%, Ratio 1x, 4x oversampling).

use nih_plug::prelude::Vst3Plugin;
use preset_kit::{Bank, Preset, V};

use crate::Sunder;

pub static BANK: Bank = Bank {
    class_id: <Sunder as Vst3Plugin>::VST3_CLASS_ID,
    version: env!("CARGO_PKG_VERSION"),
    keep: &[],
    presets: &[
        Preset { name: "Init", set: &[] },
        // The sound of a Dalek: a voice multiplied by a low sine, so it loses its pitch and keeps its words.
        Preset { name: "Dalek", set: &[("circuit", V::Name("Ideal")), ("freq", V::F(30.0)), ("tone", V::F(6000.0))] },
        Preset { name: "Robot", set: &[("circuit", V::Name("Ideal")), ("freq", V::F(110.0)), ("mix", V::F(0.85))] },
        // A diode ring at a few hundred hertz: clangy, a little rough, never in tune.
        Preset { name: "Tin Can", set: &[("freq", V::F(280.0)), ("drive", V::F(0.3)), ("tone", V::F(7000.0))] },
        // Sums and differences of a high carrier: inharmonic metal, struck.
        Preset { name: "Bell", set: &[("freq", V::F(1480.0)), ("drive", V::F(0.5)), ("mix", V::F(0.7))] },
        Preset { name: "Gong", set: &[("freq", V::F(212.0)), ("circuit", V::Name("Ideal")), ("wave", V::Name("Triangle")), ("mix", V::F(0.6))] },
        // Full bias is amplitude modulation: the ring modulator's slow cousin.
        Preset { name: "Tremolo", set: &[("circuit", V::Name("Ideal")), ("freq", V::F(5.0)), ("bias", V::F(1.0))] },
        Preset { name: "Square Chop", set: &[("circuit", V::Name("Ideal")), ("wave", V::Name("Square")), ("freq", V::F(8.0)), ("bias", V::F(0.6))] },
        Preset { name: "Helicopter", set: &[("circuit", V::Name("Ideal")), ("wave", V::Name("Saw")), ("freq", V::F(14.0)), ("bias", V::F(0.8))] },
        // Starved diodes: the carrier barely opens them, so the sound crackles in and out.
        Preset { name: "Starved Diodes", set: &[("freq", V::F(520.0)), ("drive", V::F(0.0)), ("mix", V::F(0.8))] },
        // Hammered diodes: a multiplication by a square wave, all odd harmonics.
        Preset { name: "Hard Driven", set: &[("freq", V::F(330.0)), ("drive", V::F(1.0)), ("tone", V::F(5000.0))] },
        Preset { name: "Moog Mod", set: &[("freq", V::F(660.0)), ("drive", V::F(0.45)), ("wobble", V::F(0.1)), ("rate", V::F(4.5)), ("mix", V::F(0.75))] },
        // Single sideband: every partial moves by the same number of hertz, so harmony turns to inharmony.
        Preset { name: "Shift Up", set: &[("circuit", V::Name("Ideal")), ("freq", V::F(60.0)), ("shift", V::F(1.0)), ("mix", V::F(0.5))] },
        Preset { name: "Shift Down", set: &[("circuit", V::Name("Ideal")), ("freq", V::F(60.0)), ("shift", V::F(-1.0)), ("mix", V::F(0.5))] },
        // A few hertz of shift against the dry signal beats like a phaser that never repeats.
        Preset { name: "Beating", set: &[("circuit", V::Name("Ideal")), ("freq", V::F(1.4)), ("shift", V::F(1.0)), ("mix", V::F(0.5))] },
        // Opposite shifts left and right: a stereo smear with no delay.
        Preset { name: "Spread Shimmer", set: &[("circuit", V::Name("Ideal")), ("freq", V::F(9.0)), ("shift", V::F(1.0)), ("spread", V::F(1.0)), ("mix", V::F(0.5))] },
        Preset { name: "Metal Up", set: &[("circuit", V::Name("Ideal")), ("freq", V::F(437.0)), ("shift", V::F(1.0)), ("mix", V::F(0.6))] },
        // The carrier is the input's own pitch: an octave up where the difference sideband vanishes.
        Preset { name: "Octaver", set: &[("source", V::Name("Track")), ("circuit", V::Name("Ideal")), ("mix", V::F(0.7))] },
        Preset { name: "Fifth Grit", set: &[("source", V::Name("Track")), ("ratio", V::F(1.5)), ("drive", V::F(0.5))] },
        Preset { name: "Sub Ring", set: &[("source", V::Name("Track")), ("circuit", V::Name("Ideal")), ("ratio", V::F(0.5)), ("mix", V::F(0.6))] },
        // Play it: the last MIDI note is the carrier.
        Preset { name: "Keyed", set: &[("source", V::Name("MIDI")), ("drive", V::F(0.5))] },
        Preset { name: "Keyed Octave", set: &[("source", V::Name("MIDI")), ("ratio", V::F(2.0)), ("circuit", V::Name("Ideal")), ("mix", V::F(0.7))] },
        // Louder input drags the carrier up two octaves, so every hit pitches itself.
        Preset { name: "Rising Hits", set: &[("freq", V::F(120.0)), ("follow", V::F(0.9)), ("circuit", V::Name("Ideal"))] },
        Preset { name: "Dive Bomb", set: &[("freq", V::F(900.0)), ("follow", V::F(-0.9)), ("drive", V::F(0.6))] },
        Preset { name: "Siren", set: &[("freq", V::F(760.0)), ("wobble", V::F(0.7)), ("rate", V::F(3.5)), ("mix", V::F(0.8))] },
        Preset { name: "Warble", set: &[("freq", V::F(190.0)), ("wobble", V::F(0.25)), ("rate", V::F(6.0)), ("circuit", V::Name("Ideal")), ("tone", V::F(6500.0))] },
        // The sidechain input is the carrier: a voice through a synth, or drums through a bass.
        Preset { name: "Cross Mod", set: &[("source", V::Name("Sidechain")), ("circuit", V::Name("Ideal"))] },
        Preset { name: "Cross Diodes", set: &[("source", V::Name("Sidechain")), ("drive", V::F(0.6)), ("mix", V::F(0.8))] },
    ],
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SunderParams;

    #[test]
    fn bank_is_valid() {
        BANK.validate(&SunderParams::default()).unwrap();
    }

    #[test]
    fn init_matches_the_defaults_and_others_do_not() {
        let params = SunderParams::default();
        assert_eq!(BANK.current(&params), Some(0));
    }

    #[test]
    fn state_names_every_parameter_with_the_right_kind() {
        use nih_plug::wrapper::state::ParamValue;
        let state = BANK.state(1, &SunderParams::default());
        assert_eq!(state.params.len(), 15);
        assert!(matches!(state.params["circuit"], ParamValue::I32(1)));
        assert!(matches!(state.params["freq"], ParamValue::F32(f) if (f - 30.0).abs() < 1e-3));
    }
}
