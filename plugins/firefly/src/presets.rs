//! Factory presets, from gentle levelling through per-source starting points to extremes. Each lists only what it changes from the defaults (Peak Reduction 50%, Gain
//! 0 dB, Mode Compress, Emphasis 35%, Tube 30%, Link 100%, Mix 100%).

use nih_plug::prelude::Vst3Plugin;
use preset_kit::{Bank, Preset, V};

use crate::Firefly;

pub static BANK: Bank = Bank {
    class_id: <Firefly as Vst3Plugin>::VST3_CLASS_ID,
    version: env!("CARGO_PKG_VERSION"),
    keep: &[],
    presets: &[
        Preset { name: "Init", set: &[] },
        // The classic: a few dB of slow levelling and a little warmth on a sung vocal.
        Preset { name: "Vocal Leveller", set: &[("pr", V::F(0.45)), ("gain", V::F(3.0)), ("tube", V::F(0.35))] },
        // Firm and fast to catch a belted chorus; the tail stays short because the cell is not worked long.
        Preset { name: "Belt It", set: &[("pr", V::F(0.65)), ("mode", V::Name("Limit")), ("gain", V::F(4.0)), ("emph", V::F(0.5))] },
        // Fat low-end control: no tilt so the bass drives the lamp as hard as anything.
        Preset { name: "Bass Guitar", set: &[("pr", V::F(0.55)), ("emph", V::F(0.0)), ("gain", V::F(3.0)), ("tube", V::F(0.5))] },
        // Barely there: a dB or two of glue across two channels.
        Preset { name: "Mix Bus Glue", set: &[("pr", V::F(0.25)), ("emph", V::F(0.2)), ("tube", V::F(0.15)), ("gain", V::F(1.0))] },
        // Slam it, then blend the squash under the dry signal.
        Preset { name: "Parallel Squash", set: &[("pr", V::F(0.9)), ("mode", V::Name("Limit")), ("gain", V::F(8.0)), ("tube", V::F(0.6)), ("mix", V::F(0.4))] },
        // The tilt does the de-essing: highs drive the lamp so sibilance gets pulled down first.
        Preset { name: "Tame the Top", set: &[("pr", V::F(0.5)), ("emph", V::F(1.0)), ("gain", V::F(2.0)), ("tube", V::F(0.2))] },
        // Mostly the tube: almost no reduction, a lot of colour.
        Preset { name: "Tube Warmth", set: &[("pr", V::F(0.1)), ("tube", V::F(0.85)), ("gain", V::F(6.0)), ("emph", V::F(0.0))] },
        // Independent channels: for wide sources that should not duck each other.
        Preset { name: "Wide Stereo", set: &[("pr", V::F(0.4)), ("link", V::F(0.3)), ("gain", V::F(2.0))] },
        // Barely touches the peaks: one slow dB of evening-out, nothing you can point at.
        Preset { name: "Whisper Level", set: &[("pr", V::F(0.15)), ("gain", V::F(1.0)), ("tube", V::F(0.1)), ("emph", V::F(0.25))] },
        // Backing vocals: even, soft and a little dark so they sit behind the lead.
        Preset { name: "Backing Cloud", set: &[("pr", V::F(0.55)), ("gain", V::F(4.0)), ("emph", V::F(0.7)), ("tube", V::F(0.25)), ("link", V::F(0.6))] },
        // Strummed steel strings: moderate squeeze, a touch of top weighting to calm pick attack.
        Preset { name: "Acoustic Strum", set: &[("pr", V::F(0.4)), ("gain", V::F(2.5)), ("emph", V::F(0.6)), ("tube", V::F(0.2))] },
        // Electric guitar: sustain and bloom from the tube, heavy drive into the cell.
        Preset { name: "Electric Sustain", set: &[("pr", V::F(0.7)), ("gain", V::F(5.0)), ("emph", V::F(0.15)), ("tube", V::F(0.65)), ("link", V::F(0.8))] },
        // Drum bus: let the transient through (low weighting), then hit the sustain hard-ish.
        Preset { name: "Drum Bus Pump", set: &[("pr", V::F(0.75)), ("mode", V::Name("Limit")), ("gain", V::F(5.0)), ("emph", V::F(0.1)), ("tube", V::F(0.4)), ("mix", V::F(0.8))] },
        // Room mics: crushed hard and tucked in underneath, so the space blooms between hits.
        Preset { name: "Room Bloom", set: &[("pr", V::F(0.95)), ("mode", V::Name("Limit")), ("gain", V::F(12.0)), ("emph", V::F(0.3)), ("tube", V::F(0.5)), ("mix", V::F(0.5))] },
        // Piano: a gentle hold on the loud notes and a clean tail, no colour to dull the hammers.
        Preset { name: "Grand Piano", set: &[("pr", V::F(0.3)), ("gain", V::F(1.5)), ("emph", V::F(0.4)), ("tube", V::F(0.1)), ("link", V::F(0.7))] },
        // Synth pad: slow swell smoothing with a hint of drive to thicken the sustain.
        Preset { name: "Pad Cushion", set: &[("pr", V::F(0.35)), ("gain", V::F(0.0)), ("emph", V::F(0.5)), ("tube", V::F(0.45)), ("link", V::F(0.5))] },
        // Podcast and dialogue: firm, bright-weighted levelling with a lot of make-up for loudness.
        Preset { name: "Talk Radio", set: &[("pr", V::F(0.6)), ("gain", V::F(8.0)), ("emph", V::F(0.8)), ("tube", V::F(0.3)), ("mode", V::Name("Limit"))] },
        // Sidechain sees mostly lows: a kick or bass line pumps the whole thing and the highs stay free.
        Preset { name: "Low End Ducker", set: &[("pr", V::F(0.7)), ("emph", V::F(0.0)), ("gain", V::F(3.0)), ("tube", V::F(0.2)), ("link", V::F(1.0))] },
        // Sidechain sees only the air: de-esser behaviour with the tilt wide open and a quick limiter.
        Preset { name: "Sibilance Hunter", set: &[("pr", V::F(0.7)), ("emph", V::F(1.0)), ("mode", V::Name("Limit")), ("gain", V::F(1.0)), ("tube", V::F(0.0))] },
        // Everything on the dry side: just a ghost of the compressed copy added behind the original.
        Preset { name: "Ghost Layer", set: &[("pr", V::F(1.0)), ("gain", V::F(24.0)), ("tube", V::F(0.7)), ("mix", V::F(0.15)), ("emph", V::F(0.5))] },
        // Turned down on purpose: the cell works hard, but the output is pulled well under unity.
        Preset { name: "Dark Room", set: &[("pr", V::F(0.6)), ("gain", V::F(-12.0)), ("emph", V::F(0.9)), ("tube", V::F(0.0)), ("link", V::F(0.4))] },
        // Everything at the stops: full squash, full tube, huge make-up. Turn the monitors down.
        Preset { name: "Meltdown", set: &[("pr", V::F(1.0)), ("mode", V::Name("Limit")), ("gain", V::F(20.0)), ("emph", V::F(1.0)), ("tube", V::F(1.0)), ("link", V::F(0.0)), ("mix", V::F(0.7))] },
        // Tube as a fuzz box: no compression, maximum drive, leaning on make-up gain.
        Preset { name: "Glow Fuzz", set: &[("pr", V::F(0.0)), ("tube", V::F(1.0)), ("gain", V::F(12.0)), ("emph", V::F(0.5))] },
    ],
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FireflyParams;

    #[test]
    fn bank_is_valid() {
        BANK.validate(&FireflyParams::default()).unwrap();
    }

    #[test]
    fn init_matches_the_defaults() {
        assert_eq!(BANK.current(&FireflyParams::default()), Some(0));
    }

    #[test]
    fn state_names_every_parameter() {
        let state = BANK.state(1, &FireflyParams::default());
        assert_eq!(state.params.len(), 7);
    }
}
