//! Factory presets. Each lists only what it changes from the defaults (Mid High 35%, High 50%, Trim 0 dB).
//! Together they sweep every control: Mid High and High from 0 to 100%, Trim from -12 to +12 dB.
//! Mid High acts from about 3.5 kHz (presence, bite); High from about 9 kHz (air, sparkle).

use nih_plug::prelude::Vst3Plugin;
use preset_kit::{Bank, Preset, V};

use crate::AuroraAir;

pub static BANK: Bank = Bank {
    class_id: <AuroraAir as Vst3Plugin>::VST3_CLASS_ID,
    version: env!("CARGO_PKG_VERSION"),
    keep: &[],
    presets: &[
        Preset { name: "Init", set: &[] },
        // Barely-there polish for a full mix: a little of both bands, nothing you can point at.
        Preset { name: "Master Sheen", set: &[("midhigh", V::F(0.15)), ("high", V::F(0.3))] },
        // Lifts breath and sibilant sparkle on a vocal without pushing the harsh upper mids.
        Preset { name: "Vocal Air", set: &[("midhigh", V::F(0.25)), ("high", V::F(0.75)), ("trim", V::Db(-0.5))] },
        // Heavy excitation for dull, lo-fi or over-filtered samples; trimmed back to offset the added level.
        Preset { name: "Dull Revival", set: &[("midhigh", V::F(0.65)), ("high", V::F(0.85)), ("trim", V::Db(-2.5))] },
        // Mostly the 3.5 kHz band: adds bite to guitars, snares and synths with little top-end fizz.
        Preset { name: "Presence Bite", set: &[("midhigh", V::F(0.7)), ("high", V::F(0.15)), ("trim", V::Db(-1.5))] },
        // Only the 9 kHz band: a glassy shimmer that leaves the midrange alone.
        Preset { name: "Top Shimmer", set: &[("midhigh", V::F(0.0)), ("high", V::F(0.6))] },
        // Both bands wide open for an aggressive, fizzy exciter; trim pulls the level back down.
        Preset { name: "Exciter Max", set: &[("midhigh", V::F(1.0)), ("high", V::F(1.0)), ("trim", V::Db(-5.0))] },
        // Mastering-safe: a whisper of air only, for a final bus where you want to feel it, not hear it.
        Preset { name: "Glass Ceiling", set: &[("midhigh", V::F(0.05)), ("high", V::F(0.2)), ("trim", V::Db(-0.2))] },
        // Gentle lift for a dull acoustic guitar: some pick attack, a little string shimmer.
        Preset { name: "Steel Strings", set: &[("midhigh", V::F(0.4)), ("high", V::F(0.45)), ("trim", V::Db(-1.0))] },
        // Cymbal and room sparkle for drum overheads, with almost no midrange push so snare bleed stays tame.
        Preset { name: "Overhead Fizz", set: &[("midhigh", V::F(0.2)), ("high", V::F(0.8)), ("trim", V::Db(-1.0))] },
        // Synth pads and leads: wide top-end bloom with a touch of bite so they cut through.
        Preset { name: "Synth Halo", set: &[("midhigh", V::F(0.5)), ("high", V::F(0.7)), ("trim", V::Db(-1.5))] },
        // Bass-safe: nothing in the mids, a trace of top so the harmonics read on small speakers.
        Preset { name: "Low End Safe", set: &[("midhigh", V::F(0.0)), ("high", V::F(0.12))] },
        // Intelligibility for dialogue and podcasts: presence band forward, air kept low to avoid hiss and sibilance.
        Preset { name: "Talk Radio", set: &[("midhigh", V::F(0.55)), ("high", V::F(0.2)), ("trim", V::Db(-1.0))] },
        // Cheap-cassette and phone-speaker rescue: strong presence, moderate air, level pulled down to compensate.
        Preset { name: "Tape Rescue", set: &[("midhigh", V::F(0.8)), ("high", V::F(0.5)), ("trim", V::Db(-3.0))] },
        // Both bands low with a small level boost: a quiet make-up boost to bring a bright mix up in level.
        Preset { name: "Quiet Lift", set: &[("midhigh", V::F(0.1)), ("high", V::F(0.1)), ("trim", V::Db(3.0))] },
        // Mid-way on both bands: a balanced, all-purpose enhancer.
        Preset { name: "Half Light", set: &[("midhigh", V::F(0.5)), ("high", V::F(0.5))] },
        // Mid High only at the top of its range, High off: pure 3.5 kHz grit.
        Preset { name: "Grit Only", set: &[("midhigh", V::F(1.0)), ("high", V::F(0.0)), ("trim", V::Db(-4.0))] },
        // High only at full: all sparkle, no mids, for sizzling hats and breathy tops.
        Preset { name: "Pure Sparkle", set: &[("midhigh", V::F(0.0)), ("high", V::F(1.0)), ("trim", V::Db(-3.0))] },
        // Everything off but the trim at its quietest: a clean way to drop a track 12 dB.
        Preset { name: "Trim Floor", set: &[("midhigh", V::F(0.0)), ("high", V::F(0.0)), ("trim", V::Db(-12.0))] },
        // Everything off but the trim at its loudest: a clean 12 dB gain stage.
        Preset { name: "Trim Ceiling", set: &[("midhigh", V::F(0.0)), ("high", V::F(0.0)), ("trim", V::Db(12.0))] },
        // Wild: both bands maxed and the level pushed up for a blown-out, harsh, in-your-face wall.
        Preset { name: "Solar Flare", set: &[("midhigh", V::F(0.9)), ("high", V::F(0.95)), ("trim", V::Db(2.0))] },
        // Creative: heavy mids with a sliver of air, for a nasal, megaphone-style telephone tone.
        Preset { name: "Megaphone", set: &[("midhigh", V::F(0.95)), ("high", V::F(0.05)), ("trim", V::Db(-2.0))] },
        // Creative: bright, hissy ghost of the original, good for parallel sends on a vocal or drum bus.
        Preset { name: "Ghost Send", set: &[("midhigh", V::F(0.3)), ("high", V::F(1.0)), ("trim", V::Db(-6.0))] },
    ],
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AirParams;

    #[test]
    fn bank_is_valid() {
        BANK.validate(&AirParams::default()).unwrap();
    }

    #[test]
    fn init_matches_the_defaults_and_others_do_not() {
        let params = AirParams::default();
        assert_eq!(BANK.current(&params), Some(0));
    }

    #[test]
    fn state_names_every_parameter_with_the_right_kind() {
        use nih_plug::wrapper::state::ParamValue;
        let state = BANK.state(2, &AirParams::default());
        assert_eq!(state.params.len(), 3);
        assert!(matches!(state.params["midhigh"], ParamValue::F32(m) if (m - 0.25).abs() < 1e-4));
        assert!(matches!(state.params["high"], ParamValue::F32(m) if (m - 0.75).abs() < 1e-4));
        assert!(matches!(state.params["trim"], ParamValue::F32(t) if (t - 0.944).abs() < 1e-3));
    }
}
