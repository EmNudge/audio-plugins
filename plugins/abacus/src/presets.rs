//! Factory presets. Each lists only what it changes from the defaults (16 partials, no freeze,
//! full resolution, 80 ms glide, no transpose or stretch, no noise, no spread, mix 100%).

use nih_plug::prelude::Vst3Plugin;
use preset_kit::{Bank, Preset, V};

use crate::Abacus;

pub static BANK: Bank = Bank {
    class_id: <Abacus as Vst3Plugin>::VST3_CLASS_ID,
    version: env!("CARGO_PKG_VERSION"),
    keep: &[],
    presets: &[
        Preset { name: "Init", set: &[] },
        // The grinder drone: whatever is playing when you flip the switch, held for ever.
        Preset { name: "Grinder Drone", set: &[("partials", V::F(24.0)), ("afreeze", V::B(true)), ("noise", V::F(0.1)), ("spread", V::F(0.2))] },
        Preset { name: "Frozen Pad", set: &[("partials", V::F(40.0)), ("afreeze", V::B(true)), ("spread", V::F(0.45)), ("res", V::F(0.7)), ("mix", V::F(0.85))] },
        Preset { name: "Pinned Pitch", set: &[("partials", V::F(20.0)), ("ffreeze", V::B(true)), ("glide", V::F(0.0))] },
        Preset { name: "Stone Voice", set: &[("partials", V::F(12.0)), ("ffreeze", V::B(true)), ("afreeze", V::B(true)), ("noise", V::F(0.2))] },
        // One sine at a time: monophonic pitch follower.
        Preset { name: "Sine Follower", set: &[("partials", V::F(1.0)), ("glide", V::F(30.0))] },
        Preset { name: "Whistle", set: &[("partials", V::F(2.0)), ("glide", V::F(120.0)), ("noise", V::F(0.15))] },
        Preset { name: "Three Voices", set: &[("partials", V::F(3.0)), ("spread", V::F(0.3)), ("glide", V::F(60.0))] },
        // A few partials of a voice, nothing else: the skeleton of the sound.
        Preset { name: "Skeleton", set: &[("partials", V::F(8.0)), ("res", V::F(1.0)), ("glide", V::F(20.0))] },
        Preset { name: "Choir of Sines", set: &[("partials", V::F(32.0)), ("spread", V::F(0.6)), ("glide", V::F(150.0)), ("mix", V::F(0.9))] },
        // Resolution down: wide windows merge neighbours, so the partials wander and smear.
        Preset { name: "Smeared Glass", set: &[("partials", V::F(16.0)), ("res", V::F(0.2)), ("glide", V::F(250.0)), ("spread", V::F(0.25))] },
        Preset { name: "Lo-Fi Partials", set: &[("partials", V::F(10.0)), ("res", V::F(0.0)), ("glide", V::F(0.0)), ("noise", V::F(0.3))] },
        Preset { name: "Underwater", set: &[("partials", V::F(24.0)), ("res", V::F(0.1)), ("glide", V::F(600.0)), ("spread", V::F(0.5)), ("noise", V::F(0.2))] },
        // The residual brings the breath and consonants back.
        Preset { name: "Voice And Breath", set: &[("partials", V::F(28.0)), ("noise", V::F(0.8)), ("glide", V::F(40.0))] },
        Preset { name: "Whisper Sines", set: &[("partials", V::F(6.0)), ("noise", V::F(1.0)), ("spread", V::F(0.15))] },
        // Transposing the partial set, not the audio: no grains, no formant ripple.
        Preset { name: "Octave Up", set: &[("partials", V::F(32.0)), ("pitch", V::F(12.0)), ("noise", V::F(0.1))] },
        Preset { name: "Octave Down", set: &[("partials", V::F(32.0)), ("pitch", V::F(-12.0))] },
        Preset { name: "Fifth Layer", set: &[("partials", V::F(24.0)), ("pitch", V::F(7.0)), ("mix", V::F(0.5))] },
        // Stretch pulls overtones out of the harmonic series: bells, then gongs.
        Preset { name: "Bell Stretch", set: &[("partials", V::F(20.0)), ("stretch", V::F(0.7)), ("glide", V::F(40.0)), ("mix", V::F(0.8))] },
        Preset { name: "Piano Inharmony", set: &[("partials", V::F(30.0)), ("stretch", V::F(0.25)), ("glide", V::F(10.0))] },
        Preset { name: "Squeezed", set: &[("partials", V::F(24.0)), ("stretch", V::F(-0.7)), ("noise", V::F(0.15))] },
        Preset { name: "Frozen Bell", set: &[("partials", V::F(16.0)), ("afreeze", V::B(true)), ("stretch", V::F(0.6)), ("pitch", V::F(5.0)), ("spread", V::F(0.15))] },
        Preset { name: "Chorus Cloud", set: &[("partials", V::F(48.0)), ("spread", V::F(1.0)), ("res", V::F(0.8)), ("mix", V::F(0.7))] },
        Preset { name: "Robot Choir", set: &[("partials", V::F(14.0)), ("ffreeze", V::B(true)), ("glide", V::F(0.0)), ("spread", V::F(0.5)), ("pitch", V::F(-5.0))] },
    ],
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AbacusParams;

    #[test]
    fn bank_is_valid() {
        BANK.validate(&AbacusParams::default()).unwrap();
    }

    #[test]
    fn init_matches_the_defaults() {
        assert_eq!(BANK.current(&AbacusParams::default()), Some(0));
    }
}
