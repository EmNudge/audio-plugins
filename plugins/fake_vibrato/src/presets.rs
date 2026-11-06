//! Factory presets. Each lists only what it changes from the defaults (5.5 Hz, 30 cents, 100% mix).

use nih_plug::prelude::Vst3Plugin;
use preset_kit::{Bank, Preset, V};

use crate::FakeVibrato;

pub static BANK: Bank = Bank {
    class_id: <FakeVibrato as Vst3Plugin>::VST3_CLASS_ID,
    version: env!("CARGO_PKG_VERSION"),
    keep: &[],
    presets: &[
        Preset { name: "Init", set: &[] },
        // Slow, shallow drift like a worn tape transport.
        Preset { name: "Tape Wobble", set: &[("rate", V::F(1.4)), ("depth", V::F(14.0)), ("mix", V::F(1.0))] },
        // A hand-vibrato on a guitar or voice: moderate rate, about a sixth of a semitone.
        Preset { name: "Guitar Vibrato", set: &[("rate", V::F(5.0)), ("depth", V::F(22.0))] },
        // Slow and very deep, so notes audibly sag and bend.
        Preset { name: "Seasick", set: &[("rate", V::F(1.8)), ("depth", V::F(85.0))] },
        // Fast and fairly deep, like a rotary speaker's horn.
        Preset { name: "Leslie Shimmer", set: &[("rate", V::F(7.8)), ("depth", V::F(40.0)), ("mix", V::F(0.85))] },
        // Half dry, half modulated: the beating between them thickens the sound like a chorus.
        Preset { name: "Thick Chorus", set: &[("rate", V::F(2.2)), ("depth", V::F(18.0)), ("mix", V::F(0.5))] },
        // Fast, very shallow flutter for a nervous, shaky voice.
        Preset { name: "Nervous Flutter", set: &[("rate", V::F(9.0)), ("depth", V::F(10.0))] },
        // Barely there: a slow, tiny wander that keeps a sustained note from sounding static.
        Preset { name: "Breathing Room", set: &[("rate", V::F(4.4)), ("depth", V::F(6.0)), ("mix", V::F(1.0))] },
        // A trained singer's vibrato: about 5.5 Hz, a touch under a quarter tone.
        Preset { name: "Choir Loft", set: &[("rate", V::F(5.6)), ("depth", V::F(28.0))] },
        // Slow, wide, and a little loose, like a country steel player leaning on the bar.
        Preset { name: "Lazy Pedal Steel", set: &[("rate", V::F(3.6)), ("depth", V::F(48.0)), ("mix", V::F(0.9))] },
        // Gentle organ-style scanner vibrato: fast, shallow, and perfectly steady.
        Preset { name: "Chapel Organ", set: &[("rate", V::F(6.8)), ("depth", V::F(16.0))] },
        // Very slow, very shallow pitch sway like a warped record on a cheap turntable.
        Preset { name: "Warped Vinyl", set: &[("rate", V::F(1.0)), ("depth", V::F(9.0)), ("mix", V::F(1.0))] },
        // Mid-slow wow with some dry signal, the sort of drift from a tired cassette deck.
        Preset { name: "Cassette Wow", set: &[("rate", V::F(2.6)), ("depth", V::F(20.0)), ("mix", V::F(0.8))] },
        // Slow, huge, and unsteady: the whole song sounds like it is leaning over at the bar.
        Preset { name: "Last Call", set: &[("rate", V::F(1.2)), ("depth", V::F(100.0)), ("mix", V::F(1.0))] },
        // Low mix and tiny depth: a faint detuned double that adds width without wobble.
        Preset { name: "Ghost Double", set: &[("rate", V::F(3.2)), ("depth", V::F(12.0)), ("mix", V::F(0.3))] },
        // Fast and light at low mix, a glassy shimmer that sparkles on top of the dry sound.
        Preset { name: "Glass Shimmer", set: &[("rate", V::F(8.4)), ("depth", V::F(24.0)), ("mix", V::F(0.4))] },
        // Fast and deep enough to sound like a pulsing tremolo, though it is only pitch.
        Preset { name: "Hummingbird", set: &[("rate", V::F(10.0)), ("depth", V::F(30.0)), ("mix", V::F(1.0))] },
        // Top speed, top depth: pitch smears into a buzzing, metallic warble.
        Preset { name: "Wasp Nest", set: &[("rate", V::F(10.0)), ("depth", V::F(100.0)), ("mix", V::F(1.0))] },
        // Fast, deep vibrato blended with dry: a rubbery, cartoonish blur around each note.
        Preset { name: "Jelly Legs", set: &[("rate", V::F(7.2)), ("depth", V::F(70.0)), ("mix", V::F(0.6))] },
        // Slowest rate and a wide bend: each note takes a long, lazy lap around its pitch.
        Preset { name: "Slow Orbit", set: &[("rate", V::F(1.0)), ("depth", V::F(60.0)), ("mix", V::F(0.5))] },
    ],
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::VibratoParams;

    #[test]
    fn bank_is_valid() {
        BANK.validate(&VibratoParams::default()).unwrap();
    }

    #[test]
    fn init_matches_the_defaults_and_others_do_not() {
        let params = VibratoParams::default();
        assert_eq!(BANK.current(&params), Some(0));
    }

    #[test]
    fn state_names_every_parameter_with_the_right_kind() {
        use nih_plug::wrapper::state::ParamValue;
        let state = BANK.state(5, &VibratoParams::default());
        assert_eq!(state.params.len(), 3);
        assert!(matches!(state.params["mix"], ParamValue::F32(m) if (m - 0.5).abs() < 1e-4));
        assert!(matches!(state.params["rate"], ParamValue::F32(r) if (r - 2.2).abs() < 1e-3));
    }
}
