//! Factory presets. Each lists only what it changes from the defaults (Note source, Single chord,
//! String model, A2 (45), Octave 0, Fine 0, Decay 1.2 s, Damping 35%, Tilt 0, 8 partials,
//! Stiffness 0, Detune 20%, Drift 15%, Feedback 0, Pluck 0, Tone off, Level 0 dB, Mix 50%).

use nih_plug::prelude::Vst3Plugin;
use preset_kit::{Bank, Preset, V};

use crate::Tines;

pub static BANK: Bank = Bank {
    class_id: <Tines as Vst3Plugin>::VST3_CLASS_ID,
    version: env!("CARGO_PKG_VERSION"),
    keep: &[],
    presets: &[
        Preset { name: "Init", set: &[] },
        // A deep voice with a metal body: a low power chord of strings, the upper modes plucked
        // at every consonant so words keep their edges.
        Preset {
            name: "Hayden",
            set: &[("note", V::F(36.0)), ("chord", V::Name("Power")), ("decay", V::F(0.7)), ("damping", V::F(0.4)), ("partials", V::F(12.0)), ("detune", V::F(0.35)), ("pluck", V::F(0.2)), ("mix", V::F(0.4))],
        },
        // The same, tuned to the voice itself so it follows every inflection.
        Preset {
            name: "Hayden Tracking",
            set: &[("source", V::Name("Track")), ("chord", V::Name("Octave")), ("octave", V::F(-1.0)), ("decay", V::F(0.5)), ("damping", V::F(0.5)), ("tilt", V::F(0.1)), ("partials", V::F(10.0)), ("pluck", V::F(0.3)), ("mix", V::F(0.4))],
        },
        // Odd harmonics only, a long tail: a voice in a very large tube.
        Preset {
            name: "Hayden Cathedral",
            set: &[("model", V::Name("Pipe")), ("note", V::F(33.0)), ("chord", V::Name("Power")), ("decay", V::F(3.5)), ("damping", V::F(0.3)), ("partials", V::F(14.0)), ("detune", V::F(0.5)), ("drift", V::F(0.3)), ("mix", V::F(0.35))],
        },
        Preset { name: "Dark Room", set: &[("note", V::F(24.0)), ("tilt", V::F(-0.7)), ("decay", V::F(1.5)), ("mix", V::F(0.45))] },
        // Short resonators are a comb filter with a pitch: robot speech.
        Preset { name: "Robot Tube", set: &[("model", V::Name("Pipe")), ("note", V::F(40.0)), ("decay", V::F(0.15)), ("damping", V::F(0.2)), ("partials", V::F(16.0)), ("mix", V::F(0.6))] },
        Preset { name: "Comb Flange", set: &[("note", V::F(60.0)), ("partials", V::F(16.0)), ("decay", V::F(0.08)), ("damping", V::F(0.0)), ("detune", V::F(0.0)), ("drift", V::F(0.0))] },
        Preset { name: "Tin Pipe", set: &[("model", V::Name("Pipe")), ("note", V::F(62.0)), ("decay", V::F(0.4)), ("partials", V::F(12.0)), ("tilt", V::F(0.3))] },
        // Play it: keys select the notes, and they stay when you let go.
        Preset { name: "Marimba", set: &[("source", V::Name("MIDI")), ("model", V::Name("Bar")), ("decay", V::F(0.8)), ("damping", V::F(0.5)), ("partials", V::F(4.0)), ("pluck", V::F(0.4))] },
        Preset { name: "Vibes", set: &[("source", V::Name("MIDI")), ("model", V::Name("Bar")), ("decay", V::F(3.0)), ("damping", V::F(0.2)), ("partials", V::F(5.0)), ("tilt", V::F(0.2)), ("detune", V::F(0.5))] },
        Preset { name: "Piano Strings", set: &[("source", V::Name("MIDI")), ("stiffness", V::F(0.35)), ("decay", V::F(2.0)), ("damping", V::F(0.5)), ("partials", V::F(12.0)), ("detune", V::F(0.15))] },
        Preset { name: "Church Bell", set: &[("model", V::Name("Bell")), ("note", V::F(48.0)), ("decay", V::F(6.0)), ("damping", V::F(0.3)), ("partials", V::F(12.0)), ("pluck", V::F(0.3))] },
        Preset { name: "Glass", set: &[("model", V::Name("Bell")), ("note", V::F(84.0)), ("decay", V::F(4.0)), ("damping", V::F(0.1)), ("partials", V::F(8.0)), ("tilt", V::F(0.4)), ("detune", V::F(0.6))] },
        Preset { name: "Gamelan", set: &[("model", V::Name("Bell")), ("chord", V::Name("Octave")), ("note", V::F(55.0)), ("decay", V::F(2.5)), ("damping", V::F(0.4)), ("partials", V::F(10.0)), ("detune", V::F(0.7)), ("drift", V::F(0.4))] },
        Preset { name: "Tom Skin", set: &[("model", V::Name("Drum")), ("note", V::F(38.0)), ("decay", V::F(0.5)), ("damping", V::F(0.5)), ("pluck", V::F(0.3))] },
        Preset { name: "Snare Skin", set: &[("model", V::Name("Drum")), ("note", V::F(50.0)), ("decay", V::F(0.25)), ("damping", V::F(0.4)), ("partials", V::F(12.0)), ("pluck", V::F(1.0)), ("tilt", V::F(0.4))] },
        Preset { name: "Flat Metal", set: &[("model", V::Name("Drum")), ("note", V::F(57.0)), ("stiffness", V::F(0.5)), ("decay", V::F(1.8)), ("damping", V::F(0.2)), ("partials", V::F(14.0)), ("tilt", V::F(0.5))] },
        // Chords that ring under whatever you feed them.
        Preset { name: "Sympathetic Drone", set: &[("chord", V::Name("Maj7")), ("note", V::F(41.0)), ("decay", V::F(6.0)), ("damping", V::F(0.3)), ("partials", V::F(10.0)), ("detune", V::F(0.6)), ("drift", V::F(0.5)), ("mix", V::F(0.35))] },
        Preset { name: "Minor Choir", set: &[("chord", V::Name("Minor")), ("model", V::Name("Pipe")), ("note", V::F(48.0)), ("decay", V::F(4.0)), ("partials", V::F(10.0)), ("detune", V::F(0.5)), ("drift", V::F(0.4)), ("mix", V::F(0.4))] },
        Preset { name: "Sus Haze", set: &[("chord", V::Name("Sus4")), ("note", V::F(50.0)), ("decay", V::F(8.0)), ("damping", V::F(0.6)), ("partials", V::F(8.0)), ("tone", V::F(5000.0)), ("mix", V::F(0.35))] },
        // Hit it, then it holds: the input closes and the tail stretches to forty seconds.
        Preset { name: "Frozen Chord", set: &[("chord", V::Name("Major")), ("note", V::F(48.0)), ("decay", V::F(20.0)), ("partials", V::F(8.0)), ("freeze", V::B(true)), ("mix", V::F(0.6))] },
        // Feedback closes the loop: the bank sings on its own and the input plays it.
        Preset { name: "Self-Oscillator", set: &[("chord", V::Name("Power")), ("note", V::F(43.0)), ("feedback", V::F(0.8)), ("decay", V::F(4.0)), ("damping", V::F(0.5)), ("mix", V::F(0.5))] },
        Preset { name: "Singing Wire", set: &[("source", V::Name("Track")), ("feedback", V::F(0.55)), ("decay", V::F(1.5)), ("partials", V::F(6.0)), ("pluck", V::F(0.2)), ("mix", V::F(0.5))] },
    ],
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TinesParams;

    #[test]
    fn bank_is_valid() {
        BANK.validate(&TinesParams::default()).unwrap();
    }

    #[test]
    fn init_matches_the_defaults_and_others_do_not() {
        let params = TinesParams::default();
        assert_eq!(BANK.current(&params), Some(0));
    }

    #[test]
    fn state_names_every_parameter_with_the_right_kind() {
        use nih_plug::wrapper::state::ParamValue;
        let state = BANK.state(1, &TinesParams::default());
        assert_eq!(state.params.len(), 19);
        assert!(matches!(state.params["chord"], ParamValue::I32(2)));
        assert!(matches!(state.params["note"], ParamValue::F32(f) if (f - 36.0).abs() < 1e-3));
    }
}
