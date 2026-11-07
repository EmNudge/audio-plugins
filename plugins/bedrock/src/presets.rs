//! Factory presets. Each lists only what it changes from the defaults (Body 50%, Sub 35%, Floor
//! 28 Hz, Trim 0 dB). Together they sweep every control: Body and Sub from 0 to 100%, Floor from
//! 20 to 50 Hz, Trim from 0 to -6 dB (what is added is summed on top, so more of it wants a lower trim).
//! Body makes harmonics of the bass and works on anything; Sub follows one note and steps aside under chords.

use nih_plug::prelude::Vst3Plugin;
use preset_kit::{Bank, Preset, V};

use crate::BedrockPlugin;

pub static BANK: Bank = Bank {
    class_id: <BedrockPlugin as Vst3Plugin>::VST3_CLASS_ID,
    version: env!("CARGO_PKG_VERSION"),
    keep: &[],
    presets: &[
        Preset { name: "Init", set: &[] },
        // The first thing to try on a full mix: weight you feel more than hear, no sub to speak of.
        Preset { name: "Master Weight", set: &[("body", V::F(0.2)), ("sub", V::F(0.15)), ("floor", V::F(30.0)), ("trim", V::Db(-0.5))] },
        // A kick that hits the chest: both engines, with the sub held above where a club system stops caring.
        Preset { name: "Kick Weight", set: &[("body", V::F(0.6)), ("sub", V::F(0.6)), ("floor", V::F(32.0)), ("trim", V::Db(-1.5))] },
        // A sustained sine at 40 to 60 Hz is the easiest thing for the sub to lock on: mostly octave, a little body.
        Preset { name: "808 Foundation", set: &[("body", V::F(0.35)), ("sub", V::F(0.8)), ("floor", V::F(30.0)), ("trim", V::Db(-2.0))] },
        // Fatten a DI bass: second and third harmonics to carry the note, a touch of sub under it.
        Preset { name: "Bass Guitar Body", set: &[("body", V::F(0.7)), ("sub", V::F(0.15)), ("floor", V::F(34.0)), ("trim", V::Db(-1.0))] },
        // Bass you can hear on a phone: harmonics only, nothing the speaker cannot move.
        Preset { name: "Small Speaker Bass", set: &[("body", V::F(1.0)), ("sub", V::F(0.0)), ("floor", V::F(40.0)), ("trim", V::Db(-3.0))] },
        // A little chest under a voice: the band stops at 130 Hz, so this reaches the low end of a baritone and no further.
        Preset { name: "Vocal Chest", set: &[("body", V::F(0.35)), ("sub", V::F(0.1)), ("floor", V::F(45.0))] },
        // Only the octave below, for a low end that is already thick.
        Preset { name: "Sub Only", set: &[("body", V::F(0.0)), ("sub", V::F(0.8)), ("floor", V::F(24.0)), ("trim", V::Db(-1.5))] },
        // Only the harmonics: the bass reads louder without getting any lower.
        Preset { name: "Harmonics Only", set: &[("body", V::F(0.8)), ("sub", V::F(0.0)), ("trim", V::Db(-2.0))] },
        // Both engines open for a system that can take it, with the floor low and the trim pulled down to match.
        Preset { name: "Club System", set: &[("body", V::F(0.45)), ("sub", V::F(0.75)), ("floor", V::F(26.0)), ("trim", V::Db(-2.5))] },
        // Thin laptop speakers and earbuds: all harmonics, and the floor far up.
        Preset { name: "Laptop Rescue", set: &[("body", V::F(0.9)), ("sub", V::F(0.0)), ("floor", V::F(50.0)), ("trim", V::Db(-2.0))] },
        // A synth bass that sounds thin in the mix: both, and a lot of each.
        Preset { name: "Synth Bass Thick", set: &[("body", V::F(0.75)), ("sub", V::F(0.55)), ("floor", V::F(30.0)), ("trim", V::Db(-2.5))] },
        // Tight and controlled: little of each, and nothing added where it would only muddy the mix.
        Preset { name: "Tight Low", set: &[("body", V::F(0.3)), ("sub", V::F(0.25)), ("floor", V::F(42.0))] },
        // The default balance with the floor at 40 Hz, for material headed to small PA systems or vinyl.
        Preset { name: "Floor 40 Safe", set: &[("body", V::F(0.5)), ("sub", V::F(0.4)), ("floor", V::F(40.0))] },
        // Everything open and the floor at the bottom: heavy, and level pulled well back.
        Preset { name: "Heavy", set: &[("body", V::F(1.0)), ("sub", V::F(1.0)), ("floor", V::F(24.0)), ("trim", V::Db(-6.0))] },
        // A trace of each, for a final bus where you want to feel it and not hear it.
        Preset { name: "Whisper Warmth", set: &[("body", V::F(0.12)), ("sub", V::F(0.08)), ("floor", V::F(34.0))] },
        // Low impacts and drones for picture: a lot of octave, the floor as low as it goes.
        Preset { name: "Cinema Rumble", set: &[("body", V::F(0.25)), ("sub", V::F(1.0)), ("floor", V::F(20.0)), ("trim", V::Db(-3.0))] },
    ],
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BedrockParams;

    #[test]
    fn bank_is_valid() {
        BANK.validate(&BedrockParams::default()).unwrap();
    }

    #[test]
    fn init_matches_the_defaults() {
        assert_eq!(BANK.current(&BedrockParams::default()), Some(0));
    }

    #[test]
    fn state_names_every_parameter() {
        use nih_plug::wrapper::state::ParamValue;
        let state = BANK.state(1, &BedrockParams::default());
        assert_eq!(state.params.len(), 4);
        assert!(matches!(state.params["floor"], ParamValue::F32(f) if (f - 30.0).abs() < 0.01));
    }
}
