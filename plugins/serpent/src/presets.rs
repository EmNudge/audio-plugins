//! Factory presets. Each lists only what it changes from the defaults (-16 dB threshold, 9 dB
//! range, 7 kHz, 1.2 oct bell, follow 50%, 0.5 ms attack, 80 ms release, 2 ms lookahead, relative).

use nih_plug::prelude::Vst3Plugin;
use preset_kit::{Bank, Preset, V};

use crate::Serpent;

pub static BANK: Bank = Bank {
    class_id: <Serpent as Vst3Plugin>::VST3_CLASS_ID,
    version: env!("CARGO_PKG_VERSION"),
    // Listen is a monitoring aid, not part of the sound.
    keep: &["listen"],
    presets: &[
        Preset { name: "Init", set: &[] },
        // Female lead: sibilance sits high and narrow-ish, so look around 8 kHz and let the
        // detector track the singer's moving "s".
        Preset {
            name: "Female Lead",
            set: &[
                ("thresh", V::F(-18.0)),
                ("range", V::F(8.0)),
                ("freq", V::F(8000.0)),
                ("width", V::F(1.0)),
                ("follow", V::F(0.6)),
                ("attack", V::F(0.3)),
                ("release", V::F(70.0)),
            ],
        },
        // Male vocal: sibilance lives lower and wider, so centre near 5.5 kHz with a broader bell.
        Preset {
            name: "Male Vocal",
            set: &[
                ("thresh", V::F(-17.0)),
                ("range", V::F(9.0)),
                ("freq", V::F(5500.0)),
                ("width", V::F(1.5)),
                ("follow", V::F(0.4)),
                ("release", V::F(90.0)),
            ],
        },
        // Podcast or broadcast: gentle, slow to grab and slow to let go, so it never sounds lispy.
        Preset {
            name: "Podcast Gentle",
            set: &[
                ("thresh", V::F(-22.0)),
                ("range", V::F(5.0)),
                ("freq", V::F(6500.0)),
                ("width", V::F(1.8)),
                ("follow", V::F(0.5)),
                ("attack", V::F(1.5)),
                ("release", V::F(140.0)),
                ("look", V::F(1.0)),
                ("shape", V::Name("Shelf")),
            ],
        },
        // Rap vocal: hot, bright, compressed takes. Deep and fast, absolute threshold so it
        // clamps consistently however loud the verse gets.
        Preset {
            name: "Rap Vocal",
            set: &[
                ("thresh", V::F(-24.0)),
                ("range", V::F(16.0)),
                ("freq", V::F(7500.0)),
                ("width", V::F(1.1)),
                ("follow", V::F(0.7)),
                ("attack", V::F(0.1)),
                ("release", V::F(40.0)),
                ("look", V::F(3.0)),
                ("detect", V::Name("Absolute")),
            ],
        },
        // Drum overheads: tame harsh hats and cymbal splash. High, wide, wideband-ish taste with
        // a slower release so rides do not pump.
        Preset {
            name: "Hat Tamer",
            set: &[
                ("thresh", V::F(-20.0)),
                ("range", V::F(6.0)),
                ("freq", V::F(10000.0)),
                ("width", V::F(2.2)),
                ("follow", V::F(0.1)),
                ("attack", V::F(0.2)),
                ("release", V::F(120.0)),
                ("look", V::F(1.5)),
                ("detect", V::Name("Absolute")),
            ],
        },
        // Bright acoustic guitar: soften pick and string zing in the upper mids with a fixed,
        // fairly wide bell.
        Preset {
            name: "Bright Acoustic",
            set: &[
                ("thresh", V::F(-20.0)),
                ("range", V::F(5.0)),
                ("freq", V::F(4500.0)),
                ("width", V::F(1.6)),
                ("follow", V::F(0.0)),
                ("attack", V::F(1.0)),
                ("release", V::F(100.0)),
            ],
        },
        // Transparent mastering insurance: shallow, wide, slow and high threshold so only genuinely harsh peaks are touched.
        Preset {
            name: "Mastering Safe",
            set: &[
                ("thresh", V::F(-12.0)),
                ("range", V::F(3.0)),
                ("freq", V::F(9000.0)),
                ("width", V::F(2.4)),
                ("follow", V::F(0.5)),
                ("attack", V::F(3.0)),
                ("release", V::F(220.0)),
                ("look", V::F(3.0)),
            ],
        },
        // Stacked backing vocals: their sibilance adds up, so a mid-depth wide bell with a lazy tracker keeps the choir smooth.
        Preset {
            name: "Backing Vocals",
            set: &[
                ("thresh", V::F(-21.0)),
                ("range", V::F(10.0)),
                ("freq", V::F(6800.0)),
                ("width", V::F(1.7)),
                ("follow", V::F(0.5)),
                ("attack", V::F(0.8)),
                ("release", V::F(110.0)),
                ("look", V::F(2.0)),
            ],
        },
        // Glue for a whole vocal group: gentle, wide, absolute so the bus behaves the same whoever is singing.
        Preset {
            name: "Vocal Bus",
            set: &[
                ("thresh", V::F(-19.0)),
                ("range", V::F(6.0)),
                ("freq", V::F(7000.0)),
                ("width", V::F(2.0)),
                ("follow", V::F(0.3)),
                ("attack", V::F(1.0)),
                ("release", V::F(130.0)),
                ("look", V::F(2.0)),
                ("detect", V::Name("Absolute")),
            ],
        },
        // Light touch on a stereo mix: hardly any range, a very wide shelf-like reach and a long lookahead for clean transients.
        Preset {
            name: "Full Mix",
            set: &[
                ("thresh", V::F(-14.0)),
                ("range", V::F(2.5)),
                ("freq", V::F(8500.0)),
                ("width", V::F(2.8)),
                ("follow", V::F(0.2)),
                ("attack", V::F(4.0)),
                ("release", V::F(250.0)),
                ("look", V::F(4.0)),
                ("shape", V::Name("Shelf")),
            ],
        },
        // Classic wideband de-esser: duck the whole signal for a few ms when an "s" hits, fast and firm.
        Preset {
            name: "Wideband Snap",
            set: &[
                ("thresh", V::F(-22.0)),
                ("range", V::F(12.0)),
                ("freq", V::F(7000.0)),
                ("width", V::F(1.2)),
                ("follow", V::F(0.5)),
                ("attack", V::F(0.1)),
                ("release", V::F(30.0)),
                ("look", V::F(3.0)),
                ("shape", V::Name("Wideband")),
            ],
        },
        // Shelf mode keeps the top end smooth: tames everything above 10 kHz without notching the presence band.
        Preset {
            name: "Air Shelf",
            set: &[
                ("thresh", V::F(-18.0)),
                ("range", V::F(7.0)),
                ("freq", V::F(10000.0)),
                ("width", V::F(1.4)),
                ("follow", V::F(0.3)),
                ("attack", V::F(0.5)),
                ("release", V::F(90.0)),
                ("look", V::F(2.0)),
                ("shape", V::Name("Shelf")),
            ],
        },
        // Surgical: a very narrow bell parked on one whistly frequency, deep and tightly tracked.
        Preset {
            name: "Narrow Notch",
            set: &[
                ("thresh", V::F(-26.0)),
                ("range", V::F(14.0)),
                ("freq", V::F(6200.0)),
                ("width", V::F(0.3)),
                ("follow", V::F(0.0)),
                ("attack", V::F(0.2)),
                ("release", V::F(50.0)),
                ("look", V::F(2.5)),
            ],
        },
        // No lookahead at all, for live monitoring: attack does the work and nothing is delayed.
        Preset {
            name: "Zero Latency",
            set: &[
                ("thresh", V::F(-18.0)),
                ("range", V::F(8.0)),
                ("freq", V::F(7000.0)),
                ("width", V::F(1.2)),
                ("follow", V::F(0.5)),
                ("attack", V::F(0.05)),
                ("release", V::F(60.0)),
                ("look", V::F(0.0)),
            ],
        },
        // Longest lookahead with a slow attack: the cut arrives before the "s" and glides in with no click.
        Preset {
            name: "Max Lookahead",
            set: &[
                ("thresh", V::F(-20.0)),
                ("range", V::F(9.0)),
                ("freq", V::F(7000.0)),
                ("width", V::F(1.5)),
                ("follow", V::F(0.5)),
                ("attack", V::F(8.0)),
                ("release", V::F(150.0)),
                ("look", V::F(5.0)),
            ],
        },
        // Tracker off: the bell stays exactly where you put it, for sources with a fixed harsh spot.
        Preset {
            name: "Locked On",
            set: &[
                ("thresh", V::F(-19.0)),
                ("range", V::F(9.0)),
                ("freq", V::F(6000.0)),
                ("width", V::F(1.0)),
                ("follow", V::F(0.0)),
                ("attack", V::F(0.5)),
                ("release", V::F(80.0)),
                ("look", V::F(2.0)),
            ],
        },
        // Tracker all the way up: the bell chases the sibilance anywhere it goes, ideal for unknown or shifting sources.
        Preset {
            name: "Roaming Ear",
            set: &[
                ("thresh", V::F(-20.0)),
                ("range", V::F(10.0)),
                ("freq", V::F(7500.0)),
                ("width", V::F(0.8)),
                ("follow", V::F(1.0)),
                ("attack", V::F(0.3)),
                ("release", V::F(60.0)),
                ("look", V::F(2.0)),
            ],
        },
        // Sensitive relative detection: reacts to the ratio of highs to body, so quiet whispers get treated as much as loud shouts.
        Preset {
            name: "Relative Ear",
            set: &[
                ("thresh", V::F(-30.0)),
                ("range", V::F(7.0)),
                ("freq", V::F(7000.0)),
                ("width", V::F(1.3)),
                ("follow", V::F(0.5)),
                ("attack", V::F(0.5)),
                ("release", V::F(100.0)),
                ("look", V::F(2.0)),
                ("detect", V::Name("Relative")),
            ],
        },
        // Absolute detection with a high threshold: only loud sibilants are caught, everything else passes untouched.
        Preset {
            name: "Hard Ceiling",
            set: &[
                ("thresh", V::F(-8.0)),
                ("range", V::F(12.0)),
                ("freq", V::F(7500.0)),
                ("width", V::F(1.0)),
                ("follow", V::F(0.4)),
                ("attack", V::F(0.2)),
                ("release", V::F(60.0)),
                ("look", V::F(3.0)),
                ("detect", V::Name("Absolute")),
            ],
        },
        // Bottom of the range: dig into the 2 kHz zone for a thick, lispy or nasal take, wide and slow.
        Preset {
            name: "Low Lisp Fix",
            set: &[
                ("thresh", V::F(-24.0)),
                ("range", V::F(11.0)),
                ("freq", V::F(2000.0)),
                ("width", V::F(2.5)),
                ("follow", V::F(0.2)),
                ("attack", V::F(2.0)),
                ("release", V::F(180.0)),
                ("look", V::F(1.0)),
            ],
        },
        // Top of the range: 14 kHz fizz and digital zing, narrow and quick.
        Preset {
            name: "Top Sizzle",
            set: &[
                ("thresh", V::F(-28.0)),
                ("range", V::F(10.0)),
                ("freq", V::F(14000.0)),
                ("width", V::F(0.5)),
                ("follow", V::F(0.2)),
                ("attack", V::F(0.1)),
                ("release", V::F(25.0)),
                ("look", V::F(1.5)),
            ],
        },
        // The smallest useful setting: barely 1 dB of range, a sluggish release and a lot of headroom. Almost a bypass.
        Preset {
            name: "Whisper",
            set: &[
                ("thresh", V::F(-4.0)),
                ("range", V::F(1.0)),
                ("freq", V::F(7000.0)),
                ("width", V::F(3.0)),
                ("follow", V::F(0.5)),
                ("attack", V::F(20.0)),
                ("release", V::F(500.0)),
                ("look", V::F(0.0)),
            ],
        },
        // Wild: everything the plugin has. 24 dB wideband, super low threshold, instant release. Turns speech into a robot lisp.
        Preset {
            name: "Lisp Machine",
            set: &[
                ("thresh", V::F(-48.0)),
                ("range", V::F(24.0)),
                ("freq", V::F(5000.0)),
                ("width", V::F(0.3)),
                ("follow", V::F(1.0)),
                ("attack", V::F(0.05)),
                ("release", V::F(10.0)),
                ("look", V::F(5.0)),
                ("shape", V::Name("Wideband")),
                ("detect", V::Name("Absolute")),
            ],
        },
        // Creative: a very deep, wide shelf that removes tape hiss and cymbal wash whenever it pokes above the music.
        Preset {
            name: "Hiss Eraser",
            set: &[
                ("thresh", V::F(-40.0)),
                ("range", V::F(20.0)),
                ("freq", V::F(12000.0)),
                ("width", V::F(3.0)),
                ("follow", V::F(0.0)),
                ("attack", V::F(5.0)),
                ("release", V::F(400.0)),
                ("look", V::F(4.0)),
                ("shape", V::Name("Shelf")),
                ("detect", V::Name("Absolute")),
            ],
        },
        // Creative: a narrow, deep, fully tracking bell sweeps around like a heat-seeking filter. Hear it with the delta listen mode.
        Preset {
            name: "Sweep Hunter",
            set: &[
                ("thresh", V::F(-32.0)),
                ("range", V::F(18.0)),
                ("freq", V::F(9000.0)),
                ("width", V::F(0.4)),
                ("follow", V::F(1.0)),
                ("attack", V::F(0.05)),
                ("release", V::F(15.0)),
                ("look", V::F(0.5)),
            ],
        },
    ],
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SerpentParams;

    #[test]
    fn bank_is_valid() {
        BANK.validate(&SerpentParams::default()).unwrap();
    }

    #[test]
    fn init_matches_the_defaults_and_others_do_not() {
        let params = SerpentParams::default();
        assert_eq!(BANK.current(&params), Some(0));
    }

    #[test]
    fn state_names_every_parameter_except_the_kept_one() {
        use nih_plug::wrapper::state::ParamValue;
        let state = BANK.state(4, &SerpentParams::default());
        assert_eq!(state.params.len(), 10);
        assert!(!state.params.contains_key("listen"));
        assert!(matches!(state.params["detect"], ParamValue::I32(1)));
        assert!(matches!(state.params["shape"], ParamValue::I32(0)));
        assert!(matches!(state.params["thresh"], ParamValue::F32(t) if (t + 24.0).abs() < 1e-3));
    }
}
