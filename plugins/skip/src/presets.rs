//! Factory presets. Each lists only what it changes from the defaults (350 ms, free time, 1/8.,
//! Feedback 50%, Ping-pong 0, Low cut 120 Hz, High cut 6 kHz, Drive 20%, Diffusion 0, Pitch 0,
//! Wow 20%, Rate 0.6 Hz, Duck 0, Mix 35%).

use nih_plug::prelude::Vst3Plugin;
use preset_kit::{Bank, Preset, V};

use crate::Skip;

pub static BANK: Bank = Bank {
    class_id: <Skip as Vst3Plugin>::VST3_CLASS_ID,
    version: env!("CARGO_PKG_VERSION"),
    keep: &[],
    presets: &[
        Preset { name: "Init", set: &[] },
        // One quick bounce, a little grit: the rockabilly echo.
        Preset {
            name: "Slapback",
            set: &[("time", V::F(115.0)), ("feedback", V::F(0.12)), ("hp", V::F(180.0)), ("lp", V::F(5000.0)), ("drive", V::F(0.4)), ("mod_depth", V::F(0.1)), ("mix", V::F(0.28))],
        },
        // Two tiny delays and a drift of wow: a second take that never quite lines up.
        Preset {
            name: "Doubler",
            set: &[("time", V::F(22.0)), ("feedback", V::F(0.0)), ("pingpong", V::F(0.6)), ("hp", V::F(100.0)), ("lp", V::F(12000.0)), ("drive", V::F(0.05)), ("mod_depth", V::F(0.3)), ("mod_rate", V::F(0.8)), ("mix", V::F(0.5))],
        },
        // Dark, wobbly, saturating: every repeat is a worse photocopy of the last.
        Preset {
            name: "Worn Tape",
            set: &[("time", V::F(380.0)), ("feedback", V::F(0.58)), ("hp", V::F(160.0)), ("lp", V::F(3600.0)), ("drive", V::F(0.55)), ("mod_depth", V::F(0.45)), ("mod_rate", V::F(0.45)), ("mix", V::F(0.35))],
        },
        // Murky analog chip echo: the top end goes first and the clock whines underneath.
        Preset {
            name: "Bucket Brigade",
            set: &[("time", V::F(180.0)), ("feedback", V::F(0.55)), ("hp", V::F(200.0)), ("lp", V::F(2000.0)), ("drive", V::F(0.3)), ("mod_depth", V::F(0.35)), ("mod_rate", V::F(0.9)), ("mix", V::F(0.33))],
        },
        // Nothing added, nothing taken: a clean digital repeat for when the part is the star.
        Preset {
            name: "Clean Slate",
            set: &[("time", V::F(333.0)), ("feedback", V::F(0.35)), ("hp", V::F(40.0)), ("lp", V::F(20000.0)), ("drive", V::F(0.0)), ("mod_depth", V::F(0.0)), ("mix", V::F(0.3))],
        },
        // Long, dark repeats that you ride the feedback on.
        Preset {
            name: "Dub Skank",
            set: &[("time", V::F(520.0)), ("feedback", V::F(0.78)), ("hp", V::F(300.0)), ("lp", V::F(2400.0)), ("drive", V::F(0.65)), ("mod_depth", V::F(0.3)), ("mod_rate", V::F(0.3)), ("mix", V::F(0.42))],
        },
        // Free-running ping-pong: repeats bounce hard left and right.
        Preset {
            name: "Ping Pong",
            set: &[("time", V::F(300.0)), ("feedback", V::F(0.5)), ("pingpong", V::F(1.0)), ("hp", V::F(250.0)), ("lp", V::F(7500.0)), ("drive", V::F(0.1)), ("mod_depth", V::F(0.1)), ("mix", V::F(0.32))],
        },
        // Eighth notes batted between the ears, locked to the grid.
        Preset {
            name: "Table Tennis",
            set: &[("sync", V::B(true)), ("division", V::Name("1/8")), ("feedback", V::F(0.55)), ("pingpong", V::F(0.75)), ("hp", V::F(200.0)), ("lp", V::F(9000.0)), ("drive", V::F(0.1)), ("mod_depth", V::F(0.05)), ("mix", V::F(0.3))],
        },
        // One big half-note throw on the last word, ducked so it only blooms after.
        Preset {
            name: "Vocal Throw",
            set: &[("sync", V::B(true)), ("division", V::Name("1/2")), ("feedback", V::F(0.2)), ("hp", V::F(500.0)), ("lp", V::F(9000.0)), ("drive", V::F(0.0)), ("diffusion", V::F(0.35)), ("mod_depth", V::F(0.1)), ("duck", V::F(1.0)), ("mix", V::F(0.2))],
        },
        // Stays out of a lead vocal's way and swells back in the gaps.
        Preset {
            name: "Vocal Ducker",
            set: &[("time", V::F(360.0)), ("feedback", V::F(0.42)), ("hp", V::F(350.0)), ("lp", V::F(6500.0)), ("drive", V::F(0.15)), ("diffusion", V::F(0.2)), ("duck", V::F(0.85)), ("mix", V::F(0.4))],
        },
        // Warbling, tremolo-ish repeats that shimmer like a spring tank on a surf riff.
        Preset {
            name: "Spring Surf",
            set: &[("time", V::F(440.0)), ("feedback", V::F(0.5)), ("hp", V::F(250.0)), ("lp", V::F(5000.0)), ("drive", V::F(0.35)), ("mod_depth", V::F(0.6)), ("mod_rate", V::F(4.5)), ("mix", V::F(0.35))],
        },
        // Triplet toss with a bright, smeared tail: the snare gets a little room to fly.
        Preset {
            name: "Snare Toss",
            set: &[("sync", V::B(true)), ("division", V::Name("1/8T")), ("feedback", V::F(0.3)), ("pingpong", V::F(0.4)), ("hp", V::F(1800.0)), ("lp", V::F(9000.0)), ("drive", V::F(0.2)), ("diffusion", V::F(0.5)), ("mod_depth", V::F(0.0)), ("mix", V::F(0.3))],
        },
        // Dotted-eighth bounce that leans on the synth line and gets out of its way.
        Preset {
            name: "Lead Sled",
            set: &[("sync", V::B(true)), ("division", V::Name("1/8.")), ("feedback", V::F(0.45)), ("pingpong", V::F(0.35)), ("hp", V::F(150.0)), ("lp", V::F(14000.0)), ("drive", V::F(0.15)), ("mod_depth", V::F(0.15)), ("duck", V::F(0.5)), ("mix", V::F(0.3))],
        },
        // Almost four seconds of slow, smeared drift: hold a chord and let it breathe.
        Preset {
            name: "Pad Drift",
            set: &[("time", V::F(900.0)), ("feedback", V::F(0.7)), ("hp", V::F(100.0)), ("lp", V::F(7000.0)), ("drive", V::F(0.0)), ("diffusion", V::F(0.8)), ("mod_depth", V::F(0.5)), ("mod_rate", V::F(0.15)), ("mix", V::F(0.5))],
        },
        // Sixteenths that crush into a dirty loop, for chopping up a break.
        Preset {
            name: "Crate Dig",
            set: &[("sync", V::B(true)), ("division", V::Name("1/16")), ("feedback", V::F(0.8)), ("hp", V::F(1500.0)), ("lp", V::F(3000.0)), ("drive", V::F(0.8)), ("mod_depth", V::F(0.0)), ("mix", V::F(0.45))],
        },
        // Each bounce lands an octave higher and a little cloudier.
        Preset {
            name: "Skip Up",
            set: &[("time", V::F(470.0)), ("feedback", V::F(0.62)), ("hp", V::F(300.0)), ("lp", V::F(10000.0)), ("drive", V::F(0.1)), ("diffusion", V::F(0.6)), ("pitch", V::F(12.0)), ("mod_depth", V::F(0.25)), ("mix", V::F(0.38))],
        },
        // A fourth up per pass: a gentle, hymn-like climb.
        Preset {
            name: "Fourth Floor",
            set: &[("time", V::F(560.0)), ("feedback", V::F(0.6)), ("hp", V::F(250.0)), ("lp", V::F(11000.0)), ("drive", V::F(0.05)), ("diffusion", V::F(0.5)), ("pitch", V::F(5.0)), ("mod_depth", V::F(0.2)), ("mod_rate", V::F(0.35)), ("mix", V::F(0.4))],
        },
        // Every repeat drops a fourth into the dark.
        Preset {
            name: "Sinking Stone",
            set: &[("time", V::F(430.0)), ("feedback", V::F(0.7)), ("hp", V::F(90.0)), ("lp", V::F(3200.0)), ("drive", V::F(0.4)), ("diffusion", V::F(0.3)), ("pitch", V::F(-5.0)), ("mix", V::F(0.4))],
        },
        // A fifth up per pass and fully smeared: a pad built from a single pluck.
        Preset {
            name: "Cloud Chamber",
            set: &[("time", V::F(700.0)), ("feedback", V::F(0.82)), ("lp", V::F(8000.0)), ("drive", V::F(0.2)), ("diffusion", V::F(1.0)), ("pitch", V::F(7.0)), ("mod_depth", V::F(0.5)), ("mod_rate", V::F(0.2)), ("mix", V::F(0.45))],
        },
        // Slow, deep pitch drift that makes the repeats feel like they are underwater.
        Preset {
            name: "Seasick",
            set: &[("time", V::F(450.0)), ("feedback", V::F(0.6)), ("hp", V::F(140.0)), ("lp", V::F(4000.0)), ("drive", V::F(0.3)), ("mod_depth", V::F(1.0)), ("mod_rate", V::F(0.1)), ("mix", V::F(0.5))],
        },
        // All wet and fast: a vibrato box with the dry signal removed.
        Preset {
            name: "Shimmy",
            set: &[("time", V::F(14.0)), ("feedback", V::F(0.1)), ("hp", V::F(60.0)), ("lp", V::F(14000.0)), ("drive", V::F(0.0)), ("mod_depth", V::F(1.0)), ("mod_rate", V::F(5.0)), ("mix", V::F(1.0))],
        },
        // A couple of milliseconds with feedback: the delay turns into a pitched comb.
        Preset {
            name: "Comb Filter",
            set: &[("time", V::F(3.0)), ("feedback", V::F(0.85)), ("hp", V::F(20.0)), ("lp", V::F(20000.0)), ("drive", V::F(0.3)), ("mod_depth", V::F(0.0)), ("mix", V::F(0.5))],
        },
        // So short the repeats fuse into a rolling flutter, like a stone's last hops.
        Preset {
            name: "Last Hops",
            set: &[("time", V::F(62.0)), ("feedback", V::F(0.88)), ("hp", V::F(200.0)), ("lp", V::F(8000.0)), ("drive", V::F(0.3)), ("mod_depth", V::F(0.15)), ("mix", V::F(0.35))],
        },
        // Cheap radio in a tin can: narrow band, crushed and sagging.
        Preset {
            name: "Pocket Radio",
            set: &[("time", V::F(260.0)), ("feedback", V::F(0.5)), ("hp", V::F(900.0)), ("lp", V::F(1400.0)), ("drive", V::F(0.9)), ("mod_depth", V::F(0.7)), ("mod_rate", V::F(2.0)), ("mix", V::F(0.4))],
        },
        // Nearly four seconds between hops: one distant echo, barely there.
        Preset {
            name: "Long Haul",
            set: &[("time", V::F(3600.0)), ("feedback", V::F(0.45)), ("hp", V::F(120.0)), ("lp", V::F(9000.0)), ("drive", V::F(0.1)), ("diffusion", V::F(0.7)), ("mod_depth", V::F(0.1)), ("mix", V::F(0.18))],
        },
        // Loop gain above 1: only the saturator holds it back, and it sings.
        Preset {
            name: "Runaway",
            set: &[("time", V::F(240.0)), ("feedback", V::F(1.06)), ("hp", V::F(220.0)), ("lp", V::F(4500.0)), ("drive", V::F(0.8)), ("mod_depth", V::F(0.35)), ("mix", V::F(0.4))],
        },
        // Self-oscillating drone an octave down per pass, smeared into a dark mass; duck it to play it.
        Preset {
            name: "Black Hole",
            set: &[("time", V::F(1500.0)), ("feedback", V::F(1.1)), ("hp", V::F(30.0)), ("lp", V::F(1000.0)), ("drive", V::F(0.5)), ("diffusion", V::F(1.0)), ("pitch", V::F(-12.0)), ("mod_depth", V::F(0.4)), ("mod_rate", V::F(0.12)), ("duck", V::F(0.4)), ("mix", V::F(0.5))],
        },
    ],
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SkipParams;

    #[test]
    fn bank_is_valid() {
        BANK.validate(&SkipParams::default()).unwrap();
    }

    #[test]
    fn init_matches_the_defaults() {
        assert_eq!(BANK.current(&SkipParams::default()), Some(0));
    }

    #[test]
    fn state_names_every_parameter() {
        use nih_plug::wrapper::state::ParamValue;
        let state = BANK.state(1, &SkipParams::default());
        assert_eq!(state.params.len(), 14);
        assert!(matches!(state.params["feedback"], ParamValue::F32(_)));
    }
}
