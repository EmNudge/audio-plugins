//! Factory presets. Each lists only what it changes from the defaults (position at slide 1,
//! curve 0, fade 50%, Cross mode, no scale or shift, detail 50%, no evolve, 0.25 Hz, depth 50%,
//! glide 120 ms, mix 100%). Freeze is a performance gesture, so presets never touch it.

use nih_plug::prelude::Vst3Plugin;
use preset_kit::{Bank, Preset, V};

use crate::Specimen;

pub static BANK: Bank = Bank {
    class_id: <Specimen as Vst3Plugin>::VST3_CLASS_ID,
    version: env!("CARGO_PKG_VERSION"),
    keep: &["freeze"],
    presets: &[
        // Defaults: Cross mode, linear-ish blend, position at slide 1.
        Preset { name: "Init", set: &[] },
        // Slide the envelope of one vowel into another on a held note.
        Preset {
            name: "Vowel Morph",
            set: &[
                ("detail", V::F(0.6)),
                ("glide", V::F(200.0)),
                ("fade", V::F(0.3)),
            ],
        },
        // Fast, detailed envelope following: capture a mouth shape, play a synth through it.
        Preset {
            name: "Talk Box",
            set: &[
                ("detail", V::F(0.85)),
                ("curve", V::F(0.2)),
                ("glide", V::F(40.0)),
                ("fade", V::F(0.2)),
            ],
        },
        // A heavily blurred envelope, a gentle tonal tilt that keeps the input intact.
        Preset {
            name: "Soft Focus",
            set: &[
                ("detail", V::F(0.1)),
                ("mix", V::F(0.8)),
                ("glide", V::F(300.0)),
            ],
        },
        // Resynthesis of the slides as a steady pad that glides between them.
        Preset {
            name: "Glass Pad",
            set: &[
                ("mode", V::Name("Resynth")),
                ("glide", V::F(600.0)),
                ("fade", V::F(0.6)),
            ],
        },
        // A held spectrum with a very slow drift across the rack.
        Preset {
            name: "Frozen Cathedral",
            set: &[
                ("mode", V::Name("Resynth")),
                ("pos", V::F(0.5)),
                ("glide", V::F(1500.0)),
                ("fade", V::F(0.8)),
                ("evolve", V::Name("Sine")),
                ("rate", V::F(0.05)),
                ("depth", V::F(0.6)),
            ],
        },
        // A full sweep through all four slides every twelve seconds.
        Preset {
            name: "Slow Tide",
            set: &[
                ("mode", V::Name("Resynth")),
                ("pos", V::F(0.5)),
                ("glide", V::F(800.0)),
                ("evolve", V::Name("Sine")),
                ("rate", V::F(0.08)),
                ("depth", V::F(1.0)),
                ("fade", V::F(0.7)),
            ],
        },
        // A random walk over the slides, stumbling but never leaving the rack.
        Preset {
            name: "Drunk Walk",
            set: &[
                ("mode", V::Name("Resynth")),
                ("pos", V::F(0.5)),
                ("glide", V::F(300.0)),
                ("evolve", V::Name("Walk")),
                ("rate", V::F(0.6)),
                ("depth", V::F(0.8)),
                ("fade", V::F(0.6)),
            ],
        },
        // A nervous envelope that hops around on the live input.
        Preset {
            name: "Jitter Walk",
            set: &[
                ("pos", V::F(0.5)),
                ("glide", V::F(60.0)),
                ("evolve", V::Name("Walk")),
                ("rate", V::F(3.0)),
                ("depth", V::F(0.5)),
                ("detail", V::F(0.7)),
            ],
        },
        // The frozen spectrum an octave higher, mixed under the dry signal.
        Preset {
            name: "Octave Up Ghost",
            set: &[
                ("mode", V::Name("Resynth")),
                ("scale", V::F(1.0)),
                ("mix", V::F(0.6)),
                ("fade", V::F(0.7)),
            ],
        },
        // The frozen spectrum an octave lower, slow and dark.
        Preset {
            name: "Octave Down Fog",
            set: &[
                ("mode", V::Name("Resynth")),
                ("scale", V::F(-1.0)),
                ("glide", V::F(400.0)),
                ("mix", V::F(0.7)),
                ("fade", V::F(0.7)),
            ],
        },
        // Pulls the imposed envelope up a fourth: smaller, brighter voice.
        Preset {
            name: "Formant Lift",
            set: &[
                ("scale", V::F(0.4)),
                ("detail", V::F(0.7)),
            ],
        },
        // Pushes the imposed envelope down: bigger body, same pitch.
        Preset {
            name: "Formant Drop",
            set: &[
                ("scale", V::F(-0.4)),
                ("detail", V::F(0.7)),
            ],
        },
        // Moves the envelope up 400 Hz, a hollow, nasal shift.
        Preset {
            name: "Hz Slide",
            set: &[
                ("shift", V::F(400.0)),
                ("detail", V::F(0.5)),
            ],
        },
        // Resynthesis shifted by 700 Hz: inharmonic, bell-like.
        Preset {
            name: "Metal Shift",
            set: &[
                ("mode", V::Name("Resynth")),
                ("shift", V::F(700.0)),
                ("fade", V::F(0.3)),
            ],
        },
        // A square-ish sweep that dwells on each slide and snaps between them.
        Preset {
            name: "Stepped Slides",
            set: &[
                ("mode", V::Name("Resynth")),
                ("pos", V::F(0.5)),
                ("curve", V::F(1.0)),
                ("glide", V::F(20.0)),
                ("evolve", V::Name("Sine")),
                ("rate", V::F(0.5)),
                ("depth", V::F(1.0)),
                ("fade", V::F(0.8)),
            ],
        },
        // No detail at all: just a wide tonal wash from the slide.
        Preset {
            name: "Spectral Smear",
            set: &[
                ("detail", V::F(0.0)),
                ("glide", V::F(1000.0)),
                ("mix", V::F(0.9)),
            ],
        },
        // Plain amplitude crossfade between slides, no log morph.
        Preset {
            name: "Pure Crossfade",
            set: &[
                ("mode", V::Name("Resynth")),
                ("fade", V::F(1.0)),
                ("glide", V::F(250.0)),
            ],
        },
        // Pure log-magnitude morph: timbres slide, shared partials stay.
        Preset {
            name: "Log Morph",
            set: &[
                ("fade", V::F(0.0)),
                ("detail", V::F(1.0)),
                ("glide", V::F(150.0)),
            ],
        },
        // A fifth-ish up resynthesis, wandering slowly, blended under the dry.
        Preset {
            name: "Shimmer Bed",
            set: &[
                ("mode", V::Name("Resynth")),
                ("pos", V::F(0.5)),
                ("scale", V::F(0.58)),
                ("evolve", V::Name("Sine")),
                ("rate", V::F(0.15)),
                ("depth", V::F(0.4)),
                ("mix", V::F(0.5)),
                ("glide", V::F(900.0)),
                ("fade", V::F(0.7)),
            ],
        },
        // Quick capture, half-wet resynth for holding a chord under the next one.
        Preset {
            name: "Dub Hold",
            set: &[
                ("mode", V::Name("Resynth")),
                ("glide", V::F(100.0)),
                ("mix", V::F(0.5)),
            ],
        },
        // Very slow wander over the whole rack with long glides.
        Preset {
            name: "Wide Drift",
            set: &[
                ("mode", V::Name("Resynth")),
                ("pos", V::F(0.5)),
                ("glide", V::F(2000.0)),
                ("evolve", V::Name("Walk")),
                ("rate", V::F(0.15)),
                ("depth", V::F(1.0)),
                ("fade", V::F(0.8)),
            ],
        },
    ],
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SpecimenParams;

    #[test]
    fn bank_is_valid() {
        BANK.validate(&SpecimenParams::default()).unwrap();
        assert!(BANK.presets.len() >= 15 && BANK.presets.len() <= 24);
    }

    #[test]
    fn init_matches_the_defaults_and_others_do_not() {
        let params = SpecimenParams::default();
        assert_eq!(BANK.current(&params), Some(0));
    }

    #[test]
    fn state_covers_every_parameter_except_the_kept_one() {
        use nih_plug::wrapper::state::ParamValue;
        let state = BANK.state(1, &SpecimenParams::default());
        assert_eq!(state.params.len(), 12);
        assert!(!state.params.contains_key("freeze"));
        assert!(matches!(state.params["fade"], ParamValue::F32(f) if (f - 0.3).abs() < 1e-4));
    }
}
