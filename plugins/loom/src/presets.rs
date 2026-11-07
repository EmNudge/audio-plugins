//! Factory presets. Each lists only what it changes from the defaults: head 1 on at 0 dB with a
//! 400 ms delay, every other head off, full-width windows, no offset, 50% mix, no feedback and a
//! 120 ms smoothing time. Heads are written `(n, delay ms, shift st, slide st/s, centre Hz,
//! width oct, level dB, pan)`; a head that is not listed keeps its default, so presets that do
//! not want head 1 list it with a level of -60.

use nih_plug::prelude::Vst3Plugin;
use preset_kit::{Bank, Preset, V};

use crate::Loom;

const FULL: f32 = 10.0;

macro_rules! preset {
    ($name:expr, mix $mix:expr, fb $fb:expr, smooth $smooth:expr; $(($n:literal, $delay:expr, $shift:expr, $slide:expr, $centre:expr, $width:expr, $level:expr, $pan:expr)),* $(,)?) => {
        Preset {
            name: $name,
            set: &[
                ("mix", V::F($mix)),
                ("fb", V::F($fb)),
                ("smooth", V::F($smooth)),
                $(
                    (concat!("delay_", $n), V::F($delay)),
                    (concat!("shift_", $n), V::F($shift)),
                    (concat!("slide_", $n), V::F($slide)),
                    (concat!("centre_", $n), V::F($centre)),
                    (concat!("width_", $n), V::F($width)),
                    (concat!("level_", $n), V::F($level)),
                    (concat!("pan_", $n), V::F($pan)),
                )*
            ],
        }
    };
}

pub static BANK: Bank = Bank {
    class_id: <Loom as Vst3Plugin>::VST3_CLASS_ID,
    version: env!("CARGO_PKG_VERSION"),
    keep: &[],
    presets: &[
        Preset { name: "Init", set: &[] },
        // One head reading half a second back: the spectral version of a plain echo.
        preset!("Slow Echo", mix 0.5, fb 0.3, smooth 120.0;
            (1, 500.0, 0.0, 0.0, 1000.0, FULL, 0.0, 0.0)),
        // A canon at the octave above and below, with a little feedback to stack the copies.
        preset!("Octave Canon", mix 0.5, fb 0.25, smooth 150.0;
            (1, 250.0, 12.0, 0.0, 1000.0, FULL, -3.0, -0.4),
            (2, 500.0, -12.0, 0.0, 1000.0, FULL, -3.0, 0.4)),
        // Everything the loop sends back climbs an octave, so tails shimmer upward.
        preset!("Shimmer Tail", mix 0.45, fb 0.6, smooth 300.0;
            (1, 600.0, 12.0, 0.0, 1000.0, FULL, -2.0, 0.0)),
        // Two heads gliding in opposite directions, panned apart.
        preset!("Glissando Rain", mix 0.5, fb 0.35, smooth 200.0;
            (1, 700.0, 0.0, 6.0, 1000.0, FULL, -1.0, -0.6),
            (2, 1100.0, 0.0, -5.0, 1000.0, FULL, -1.0, 0.6)),
        // Rising glides in the loop: a barber-pole that never arrives.
        preset!("Barber Pole", mix 0.5, fb 0.7, smooth 150.0;
            (1, 120.0, 0.0, 12.0, 1000.0, FULL, -3.0, -0.3),
            (2, 240.0, 0.0, 12.0, 1000.0, FULL, -3.0, 0.3)),
        // The lows come back quickly, the highs a beat later and a fifth down.
        preset!("Spectral Split", mix 0.55, fb 0.2, smooth 120.0;
            (1, 200.0, 0.0, 0.0, 300.0, 2.0, 0.0, -0.3),
            (2, 900.0, -7.0, 0.0, 3000.0, 3.0, -1.0, 0.4)),
        // Only the bass is heard, a second and a half late and an octave down.
        preset!("Low Ghost", mix 0.6, fb 0.25, smooth 250.0;
            (1, 1500.0, -12.0, 0.0, 120.0, 3.0, 0.0, 0.0)),
        // The air band mirrored a fifth up on the other side of the stereo field.
        preset!("Air Mirror", mix 0.45, fb 0.3, smooth 150.0;
            (1, 350.0, 7.0, 0.0, 6000.0, 3.0, -1.0, 0.8),
            (2, 700.0, 7.0, 0.0, 6000.0, 3.0, -4.0, -0.8)),
        // Three voices stacked on the input a hair behind it: fifth, octave, fourth down.
        preset!("Fifths Choir", mix 0.5, fb 0.0, smooth 100.0;
            (1, 80.0, 7.0, 0.0, 1000.0, FULL, -3.0, -0.5),
            (2, 160.0, 12.0, 0.0, 1000.0, FULL, -5.0, 0.5),
            (3, 240.0, -5.0, 0.0, 1000.0, FULL, -4.0, 0.0)),
        // Six unshifted heads fanned out in time, each quieter: a dense spectral multitap.
        preset!("Fan of Echoes", mix 0.5, fb 0.3, smooth 80.0;
            (1, 100.0, 0.0, 0.0, 1000.0, FULL, -2.0, -0.7),
            (2, 210.0, 0.0, 0.0, 1000.0, FULL, -4.0, 0.7),
            (3, 330.0, 0.0, 0.0, 1000.0, FULL, -6.0, -0.4),
            (4, 450.0, 0.0, 0.0, 1000.0, FULL, -8.0, 0.4),
            (5, 590.0, 0.0, 0.0, 1000.0, FULL, -10.0, -0.1),
            (6, 740.0, 0.0, 0.0, 1000.0, FULL, -12.0, 0.1)),
        // What you played three and a half seconds ago, under what you play now.
        preset!("Past Self", mix 0.35, fb 0.0, smooth 400.0;
            (1, 3500.0, 0.0, 0.0, 1000.0, FULL, 0.0, 0.0)),
        // The mids only, a fourth down and straight away: a formant swap against the dry voice.
        preset!("Formant Swap", mix 0.6, fb 0.0, smooth 100.0;
            (1, 0.0, -5.0, 0.0, 1000.0, 2.5, 0.0, 0.0)),
        // Two heads a few cents either side, a few tens of milliseconds late: a spectral chorus.
        preset!("Detune Cloud", mix 0.5, fb 0.0, smooth 100.0;
            (1, 60.0, 0.2, 0.0, 1000.0, FULL, -1.0, -1.0),
            (2, 90.0, -0.2, 0.0, 1000.0, FULL, -1.0, 1.0)),
        // Fast glides that bounce off plus and minus an octave: dive-bombing echoes.
        preset!("Dopplers", mix 0.5, fb 0.25, smooth 80.0;
            (1, 300.0, 0.0, 24.0, 1000.0, FULL, -2.0, -0.5),
            (2, 450.0, 0.0, -18.0, 1000.0, FULL, -2.0, 0.5)),
        // High feedback and slow smoothing: the input is woven into a held, shifting drone.
        preset!("Drone Fabric", mix 0.6, fb 0.88, smooth 600.0;
            (1, 400.0, 7.0, 0.0, 1000.0, FULL, -6.0, -0.4),
            (2, 800.0, -5.0, 0.0, 1000.0, FULL, -6.0, 0.4)),
        // Eight narrow windows scattered in time, pitch and place: a whisper of the past.
        preset!("Whisper Heads", mix 0.5, fb 0.3, smooth 200.0;
            (1, 150.0, 0.0, 0.0, 300.0, 1.5, -6.0, -0.9),
            (2, 330.0, 5.0, 0.0, 600.0, 1.5, -8.0, 0.8),
            (3, 520.0, 0.0, 0.0, 1200.0, 1.5, -8.0, -0.6),
            (4, 710.0, -7.0, 0.0, 2000.0, 1.5, -9.0, 0.6),
            (5, 960.0, 0.0, 0.0, 3500.0, 1.5, -10.0, -0.3),
            (6, 1200.0, 12.0, 0.0, 5000.0, 1.5, -10.0, 0.3),
            (7, 1450.0, 0.0, 0.0, 7500.0, 1.5, -11.0, -0.1),
            (8, 1700.0, -12.0, 0.0, 9000.0, 1.5, -12.0, 0.1)),
        // Repeats fall an octave each trip round the loop.
        preset!("Down The Well", mix 0.5, fb 0.45, smooth 200.0;
            (1, 500.0, -12.0, 0.0, 1000.0, FULL, -1.0, -0.3),
            (2, 1000.0, -24.0, 0.0, 1000.0, FULL, -4.0, 0.3)),
        // Short, hard-edged taps with no smoothing: a spectral stutter.
        preset!("Stutter Weave", mix 0.5, fb 0.5, smooth 0.0;
            (1, 150.0, 0.0, 0.0, 1000.0, FULL, 0.0, -0.4),
            (2, 300.0, 0.0, 0.0, 1000.0, FULL, -2.0, 0.4),
            (3, 450.0, 0.0, 0.0, 1000.0, FULL, -4.0, 0.0)),
        // Two bands traded across the octaves: the mids go up, the highs come down.
        preset!("Crossed Threads", mix 0.55, fb 0.2, smooth 150.0;
            (1, 400.0, 12.0, 0.0, 800.0, 2.0, 0.0, -0.8),
            (2, 400.0, -12.0, 0.0, 3000.0, 2.0, 0.0, 0.8)),
        // A slow upward drift in a long, soft loop.
        preset!("Rising Fog", mix 0.5, fb 0.7, smooth 500.0;
            (1, 900.0, 0.0, 3.0, 1000.0, FULL, -3.0, 0.0)),
    ],
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LoomParams;

    #[test]
    fn bank_is_valid() {
        BANK.validate(&LoomParams::default()).unwrap();
    }

    #[test]
    fn init_matches_the_defaults_and_others_do_not() {
        let params = LoomParams::default();
        assert_eq!(BANK.current(&params), Some(0));
        assert!(BANK.presets.len() >= 15);
    }
}
