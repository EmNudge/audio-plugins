//! Factory presets: the character of the correction. Each lists only what it changes from the
//! defaults (Quality, Low range, 20 ms retune, 35% humanize, 100% amount, no formant or detune).
//! They cover vocals, instruments, live use, robot tuning, formant and detune tricks, and the
//! extremes of every control.

use nih_plug::prelude::Vst3Plugin;
use preset_kit::{Bank, Preset, V};

use crate::Detent;

/// `keep` is everything that says which notes are allowed or where they come from: the key, the
/// scale, the twelve per-note flips and the target (scale vs held MIDI notes). That is the song,
/// not the sound; a preset that reset the key to C would put a singer in E minor out of tune.
/// `range` is not kept: it is a property of the voice, but presets leave it at the default, so
/// loading one resets it. That is fine, since it only moves the tracker's floor.
pub static BANK: Bank = Bank {
    class_id: <Detent as Vst3Plugin>::VST3_CLASS_ID,
    version: env!("CARGO_PKG_VERSION"),
    keep: &[
        "key", "scale", "target", "flip_1", "flip_2", "flip_3", "flip_4", "flip_5", "flip_6", "flip_7", "flip_8",
        "flip_9", "flip_10", "flip_11", "flip_12",
    ],
    presets: &[
        Preset { name: "Init", set: &[] },
        // Slow, partial correction that leaves vibrato and small expressive bends alone.
        Preset {
            name: "Natural",
            set: &[("retune", V::F(80.0)), ("humanize", V::F(0.7)), ("amount", V::F(0.8))],
        },
        // Instant snap, no vibrato kept: the stepped, robotic T-Pain sound.
        Preset {
            name: "Hard Tune",
            set: &[("retune", V::F(0.0)), ("humanize", V::F(0.0)), ("amount", V::F(1.0))],
        },
        // Only pulls the long-term drift back; vibrato stays and half of each error is left in.
        Preset {
            name: "Drift Keeper",
            set: &[("retune", V::F(200.0)), ("humanize", V::F(0.9)), ("amount", V::F(0.5))],
        },
        // The low-latency engine for monitoring while tracking; fast but not robotic.
        Preset {
            name: "Live Monitor",
            set: &[("mode", V::Name("Live")), ("retune", V::F(15.0)), ("humanize", V::F(0.4))],
        },
        // Formant up with a quick retune: a brighter, smaller voice at the same pitch.
        Preset {
            name: "Gender Up",
            set: &[("formant", V::F(3.5)), ("retune", V::F(40.0)), ("humanize", V::F(0.5))],
        },
        // Formant down for a bigger, darker voice at the same pitch.
        Preset {
            name: "Gender Down",
            set: &[("formant", V::F(-3.5)), ("retune", V::F(40.0)), ("humanize", V::F(0.5))],
        },
        // Tuned, then pushed 18 cents sharp with the vibrato intact. Detune moves the whole output,
        // so it is meant for a duplicated track blended under the lead, not for the lead itself.
        Preset {
            name: "Wide Double",
            set: &[("detune", V::F(18.0)), ("retune", V::F(60.0)), ("humanize", V::F(0.8)), ("formant", V::F(0.5))],
        },
        // Lead vocal polish: quick but not instant, most of the vibrato kept, a touch of formant lift for air.
        Preset {
            name: "Glass Lead",
            set: &[("retune", V::F(28.0)), ("humanize", V::F(0.55)), ("amount", V::F(0.9)), ("formant", V::F(0.4))],
        },
        // Barely-there safety net: only the worst misses get nudged, so nobody hears it working.
        Preset {
            name: "Invisible",
            set: &[("retune", V::F(120.0)), ("humanize", V::F(0.8)), ("amount", V::F(0.3))],
        },
        // Backing vocals: medium snap that tightens a stack of takes so they lock without going stiff.
        Preset {
            name: "Choir Glue",
            set: &[("retune", V::F(45.0)), ("humanize", V::F(0.45)), ("amount", V::F(0.85)), ("detune", V::F(-6.0))],
        },
        // Spoken and rapped lines: very slow, shallow correction on a high tracker floor so speech stays speech.
        Preset {
            name: "Mumble Rap",
            set: &[("range", V::Name("Mid 110 Hz")), ("retune", V::F(400.0)), ("humanize", V::F(1.0)), ("amount", V::F(0.25))],
        },
        // Spoken word pushed into near-sung steps: fast retune on the mid range, no humanize.
        Preset {
            name: "Auto Chant",
            set: &[("range", V::Name("Mid 110 Hz")), ("retune", V::F(6.0)), ("humanize", V::F(0.05)), ("amount", V::F(1.0))],
        },
        // Bass guitar and low synths: the Bass floor follows down to 55 Hz, with a gentle pull.
        Preset {
            name: "Bass Anchor",
            set: &[("range", V::Name("Bass 55 Hz")), ("retune", V::F(90.0)), ("humanize", V::F(0.6)), ("amount", V::F(0.7))],
        },
        // Violin, flute and other high instruments: High range, quick snap, slides allowed through.
        Preset {
            name: "Fiddle Fix",
            set: &[("range", V::Name("High 170 Hz")), ("retune", V::F(35.0)), ("humanize", V::F(0.65)), ("amount", V::F(0.9))],
        },
        // Monophonic guitar lines and leads: mid range, firm correction that keeps bends alive.
        Preset {
            name: "Guitar Lock",
            set: &[("range", V::Name("Mid 110 Hz")), ("retune", V::F(18.0)), ("humanize", V::F(0.5)), ("amount", V::F(1.0))],
        },
        // Live engine with a slow, shallow pull for stage monitors: minimal latency, nothing audible.
        Preset {
            name: "Stage Safe",
            set: &[("mode", V::Name("Live")), ("range", V::Name("Mid 110 Hz")), ("retune", V::F(70.0)), ("humanize", V::F(0.7)), ("amount", V::F(0.6))],
        },
        // The T-Pain sound at low latency: Live engine, instant snap, zero humanize.
        Preset {
            name: "Live Robot",
            set: &[("mode", V::Name("Live")), ("retune", V::F(0.0)), ("humanize", V::F(0.0)), ("amount", V::F(1.0))],
        },
        // Classic hard-tune with a brighter voice: instant snap and a small formant lift.
        Preset {
            name: "Cher Step",
            set: &[("retune", V::F(2.0)), ("humanize", V::F(0.0)), ("formant", V::F(1.5)), ("amount", V::F(1.0))],
        },
        // Human variant: slow retune, lots of humanize, half the error left in. Feels untuned.
        Preset {
            name: "Loose Hands",
            set: &[("retune", V::F(160.0)), ("humanize", V::F(0.95)), ("amount", V::F(0.45))],
        },
        // Formant maxed up, tuned fast: a cartoon voice that still sits in the key.
        Preset {
            name: "Chipmunk",
            set: &[("formant", V::F(6.0)), ("retune", V::F(10.0)), ("humanize", V::F(0.2))],
        },
        // Formant maxed down on the Bass floor: a huge, slow voice from the floor of the speaker.
        Preset {
            name: "Giant",
            set: &[("range", V::Name("Bass 55 Hz")), ("formant", V::F(-6.0)), ("retune", V::F(50.0)), ("humanize", V::F(0.5))],
        },
        // Detune at its lowest edge: the whole output sits 50 cents under, a queasy double for blending.
        Preset {
            name: "Flat Sharp",
            set: &[("detune", V::F(-50.0)), ("retune", V::F(60.0)), ("humanize", V::F(0.8)), ("amount", V::F(0.9))],
        },
        // Detune at the top edge, 50 cents sharp: a deliberately out-of-tune layer, quarter-tone wrong.
        Preset {
            name: "Quarter Sour",
            set: &[("detune", V::F(50.0)), ("retune", V::F(30.0)), ("humanize", V::F(0.4)), ("formant", V::F(-1.0))],
        },
        // Slowest retune at full amount: the pitch drifts to the target over nearly half a second.
        Preset {
            name: "Glacier",
            set: &[("retune", V::F(400.0)), ("humanize", V::F(0.0)), ("amount", V::F(1.0))],
        },
        // Dual-voice trick: bright formant and a few cents up for a twin that floats above the lead.
        Preset {
            name: "Shimmer Pair",
            set: &[("detune", V::F(9.0)), ("formant", V::F(2.5)), ("retune", V::F(55.0)), ("humanize", V::F(0.6)), ("amount", V::F(0.75))],
        },
        // Wild one: instant snap, formant down, detune flat, Live engine. Nothing human is left.
        Preset {
            name: "Alien",
            set: &[("mode", V::Name("Live")), ("retune", V::F(0.0)), ("humanize", V::F(0.0)), ("formant", V::F(-5.0)), ("detune", V::F(-30.0))],
        },
        // Creative: fast retune with full humanize and a little amount. Vibrato wobbles through a stepped floor.
        Preset {
            name: "Warble",
            set: &[("retune", V::F(8.0)), ("humanize", V::F(1.0)), ("amount", V::F(0.55)), ("formant", V::F(2.0)), ("detune", V::F(14.0))],
        },
    ],
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DetentParams;

    #[test]
    fn bank_is_valid() {
        BANK.validate(&DetentParams::default()).unwrap();
    }

    #[test]
    fn init_matches_the_defaults() {
        assert_eq!(BANK.current(&DetentParams::default()), Some(0));
    }

    #[test]
    fn presets_leave_the_song_alone() {
        use nih_plug::wrapper::state::ParamValue;
        let state = BANK.state(2, &DetentParams::default());
        assert!(!state.params.contains_key("key"));
        assert!(!state.params.contains_key("flip_1"));
        assert!(matches!(state.params["retune"], ParamValue::F32(r) if r.abs() < 1e-4));
        assert!(matches!(state.params["mode"], ParamValue::I32(0)));
        let live = BANK.state(4, &DetentParams::default());
        assert!(matches!(live.params["mode"], ParamValue::I32(1)));
    }
}
