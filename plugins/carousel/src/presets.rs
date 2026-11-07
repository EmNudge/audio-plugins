//! Factory presets. Each lists only what it changes from the defaults (Free, Rate 0.6 Hz, Depth
//! 40%, Delay 10 ms, 3 voices, Sine, Spread 100%, Analog 40%, Mix 50%, Sync off).

use nih_plug::prelude::Vst3Plugin;
use preset_kit::{Bank, Preset, V};

use crate::Carousel;

pub static BANK: Bank = Bank {
    class_id: <Carousel as Vst3Plugin>::VST3_CLASS_ID,
    version: env!("CARGO_PKG_VERSION"),
    keep: &[],
    presets: &[
        Preset { name: "Init", set: &[] },
        // The three buttons of the box that thickened a generation of synth pads.
        Preset { name: "Juno I", set: &[("mode", V::Name("Juno I")), ("analog", V::F(0.7))] },
        Preset { name: "Juno II", set: &[("mode", V::Name("Juno II")), ("analog", V::F(0.7))] },
        // Fast and shallow: a shimmer more than a chorus.
        Preset { name: "Juno I+II", set: &[("mode", V::Name("Juno I+II")), ("analog", V::F(0.7))] },
        // Both buttons in, wetter and dirtier, for a pad that should fill the room.
        Preset { name: "Juno Pad", set: &[("mode", V::Name("Juno I+II")), ("analog", V::F(1.0)), ("mix", V::F(0.65))] },
        // Width you cannot point at: for a vocal or a lead that has to stay in front.
        Preset { name: "Dimension Subtle", set: &[("mode", V::Name("Dimension")), ("depth", V::F(0.2)), ("mix", V::F(0.35)), ("analog", V::F(0.3))] },
        Preset { name: "Dimension Wide", set: &[("mode", V::Name("Dimension")), ("depth", V::F(0.6)), ("mix", V::F(0.5))] },
        // The deepest setting: slow, obvious, hollow.
        Preset { name: "Dimension Deep", set: &[("mode", V::Name("Dimension")), ("depth", V::F(1.0)), ("mix", V::F(0.55)), ("analog", V::F(0.6))] },
        // Three voices a third of a cycle apart, slow and soft.
        Preset { name: "Solina Strings", set: &[("mode", V::Name("Ensemble")), ("delay", V::F(6.0)), ("depth", V::F(0.45)), ("rate", V::F(0.55)), ("mix", V::F(0.6)), ("analog", V::F(0.5))] },
        Preset { name: "Wide Choir", set: &[("mode", V::Name("Ensemble")), ("delay", V::F(14.0)), ("depth", V::F(0.6)), ("rate", V::F(0.35)), ("mix", V::F(0.6)), ("analog", V::F(0.3))] },
        Preset { name: "Triple Lush", set: &[("delay", V::F(12.0)), ("depth", V::F(0.45)), ("rate", V::F(0.45)), ("mix", V::F(0.55))] },
        // One voice, centred, a little dirty: the pedal on the floor.
        Preset {
            name: "Stomp Box",
            set: &[("voices", V::F(1.0)), ("delay", V::F(10.0)), ("depth", V::F(0.4)), ("rate", V::F(0.9)), ("spread", V::F(0.0)), ("analog", V::F(0.8))],
        },
        // All wet and one voice: nothing but the moving delay, which is vibrato.
        Preset {
            name: "Tape Vibrato",
            set: &[("voices", V::F(1.0)), ("delay", V::F(6.0)), ("depth", V::F(0.5)), ("rate", V::F(5.5)), ("mix", V::F(1.0)), ("analog", V::F(0.5))],
        },
        // Each voice wanders to a new place every cycle.
        Preset { name: "Slow Drift", set: &[("shape", V::Name("Random")), ("rate", V::F(0.25)), ("depth", V::F(0.6)), ("delay", V::F(16.0))] },
        // A short, dark pair that fattens a bass without smearing the attack.
        Preset {
            name: "Bass Thickener",
            set: &[("voices", V::F(2.0)), ("delay", V::F(4.0)), ("depth", V::F(0.2)), ("rate", V::F(0.3)), ("mix", V::F(0.3)), ("analog", V::F(0.7)), ("spread", V::F(0.6))],
        },
        // The sweep locks to the tempo: one trip round the carousel per beat.
        Preset { name: "Tempo Swirl", set: &[("sync", V::Name("1/4")), ("shape", V::Name("Triangle")), ("depth", V::F(0.7)), ("delay", V::F(14.0)), ("mix", V::F(0.55))] },
        Preset { name: "Seasick", set: &[("voices", V::F(2.0)), ("rate", V::F(3.0)), ("depth", V::F(1.0)), ("delay", V::F(20.0)), ("mix", V::F(0.6))] },
        // Very short delays, fast and clean: a glassy shimmer on top of a sound.
        Preset {
            name: "Glass Shimmer",
            set: &[("voices", V::F(2.0)), ("delay", V::F(2.0)), ("depth", V::F(0.9)), ("rate", V::F(6.0)), ("mix", V::F(0.4)), ("analog", V::F(0.1))],
        },
        // A cassette that has been in a hot car.
        Preset {
            name: "Broken Cassette",
            set: &[("voices", V::F(2.0)), ("shape", V::Name("Random")), ("rate", V::F(2.4)), ("depth", V::F(0.8)), ("delay", V::F(11.0)), ("analog", V::F(1.0))],
        },
        // Voices stacked in the centre: the same movement with nothing to fall apart in mono.
        Preset {
            name: "Mono Safe",
            set: &[("voices", V::F(2.0)), ("delay", V::F(7.0)), ("depth", V::F(0.3)), ("rate", V::F(0.7)), ("spread", V::F(0.0)), ("mix", V::F(0.4))],
        },
    ],
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CarouselParams;

    #[test]
    fn bank_is_valid() {
        BANK.validate(&CarouselParams::default()).unwrap();
    }

    #[test]
    fn init_matches_the_defaults() {
        assert_eq!(BANK.current(&CarouselParams::default()), Some(0));
    }

    #[test]
    fn state_names_every_parameter() {
        use nih_plug::wrapper::state::ParamValue;
        let state = BANK.state(1, &CarouselParams::default());
        assert_eq!(state.params.len(), 10);
        assert!(matches!(state.params["mode"], ParamValue::I32(1)));
    }
}
