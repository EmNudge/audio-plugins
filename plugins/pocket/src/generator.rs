//! The groove generator: a style's template, rolled with a seed.
//!
//! A template is one 16-step string per lane. Letters are certain hits (same letters as the
//! library: `X x o r R`). A digit 1-9 is an optional hit that fires with that many tenths of
//! probability, scaled by Complexity, so Complexity 0 is the bare skeleton of the style and 1 is
//! the same style busy with ghost notes and sixteenths. The same seed always rolls the same groove.

use crate::grooves::{cell_of, VEL_ACCENT};
use crate::pattern::*;
use nih_plug::prelude::Enum;

#[derive(Enum, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Style {
    Rock,
    Pop,
    Funk,
    Disco,
    House,
    Techno,
    #[name = "Boom Bap"]
    BoomBap,
    Trap,
    #[name = "Drum & Bass"]
    DnB,
    Dembow,
    Shuffle,
    Breakbeat,
}

pub const STYLES: [Style; 12] = [
    Style::Rock,
    Style::Pop,
    Style::Funk,
    Style::Disco,
    Style::House,
    Style::Techno,
    Style::BoomBap,
    Style::Trap,
    Style::DnB,
    Style::Dembow,
    Style::Shuffle,
    Style::Breakbeat,
];

pub struct Template {
    pub name: &'static str,
    pub bpm: u16,
    pub swing: f32,
    pub eighths: bool,
    /// Kick, snare, rim, clap, hat, open hat, low tom, high tom, crash, ride, perc.
    pub lanes: [&'static str; LANES],
}

impl Style {
    pub fn template(self) -> &'static Template {
        match self {
            Style::Rock => &ROCK,
            Style::Pop => &POP,
            Style::Funk => &FUNK,
            Style::Disco => &DISCO,
            Style::House => &HOUSE,
            Style::Techno => &TECHNO,
            Style::BoomBap => &BOOM_BAP,
            Style::Trap => &TRAP,
            Style::DnB => &DNB,
            Style::Dembow => &DEMBOW,
            Style::Shuffle => &SHUFFLE,
            Style::Breakbeat => &BREAKBEAT,
        }
    }

    pub fn index(self) -> usize {
        STYLES.iter().position(|s| *s == self).unwrap_or(0)
    }
}

#[rustfmt::skip]
const ROCK: Template = Template { name: "Rock", bpm: 110, swing: 0.5, eighths: false, lanes: [
    "X... ..3. x.3. 2..3", "..2. X... ..2. X.3.", "", "", "X5x5 x5x5 x5x5 x5x5", "...2 .... .... ..2.", "", "", "2... .... .... ....", "", ""] };
#[rustfmt::skip]
const POP: Template = Template { name: "Pop", bpm: 118, swing: 0.5, eighths: false, lanes: [
    "X... ..2. x..x ..2.", ".... X... .... X..2", "", ".... 3... .... 3...", "x5x5 x5x5 x5x5 x5.5", ".... .... .... ..2.", "", "", "2... .... .... ....", "", ""] };
#[rustfmt::skip]
const FUNK: Template = Template { name: "Funk", bpm: 100, swing: 0.54, eighths: false, lanes: [
    "X..2 ..x. .x2. ..2.", "..2. X2.2 .2.. X.2.", "", "", "xoxo xoxo xoxo xoxo", "", "", "", "", "", ""] };
#[rustfmt::skip]
const DISCO: Template = Template { name: "Disco", bpm: 118, swing: 0.5, eighths: false, lanes: [
    "X... X... X... X...", ".... X... .... X...", "", ".... 5... .... 5...", "x.x5 x.x5 x.x5 x.x5", "..x. ..x. ..x. ..x.", "", "", "2... .... .... ....", "", ""] };
#[rustfmt::skip]
const HOUSE: Template = Template { name: "House", bpm: 124, swing: 0.5, eighths: false, lanes: [
    "X... X... X... X...", "", "...3 .... ...3 ....", ".... X... .... X...", ".5.5 .5.5 .5.5 .5.5", "..x. ..x. ..x. ..x.", "", "", "", "", "..3. .3.. ..3. 3..."] };
#[rustfmt::skip]
const TECHNO: Template = Template { name: "Techno", bpm: 132, swing: 0.5, eighths: false, lanes: [
    "X... X... X... X...", "", "..3. .... ...3 ....", ".... x... .... x...", "o5o5 o5o5 o5o5 o5o5", "..x. ..x. ..x. ..x.", "", "", "3... .... .... ....", "", "...3 ..3. .3.. ...3"] };
#[rustfmt::skip]
const BOOM_BAP: Template = Template { name: "Boom Bap", bpm: 90, swing: 0.56, eighths: false, lanes: [
    "X... ..x. .x2. ..3.", ".... X..3 .3.. X.3.", "", "", "x.x5 x.x. x.x5 x.x.", ".... .... .... ..2.", "", "", "", "", ""] };
#[rustfmt::skip]
const TRAP: Template = Template { name: "Trap", bpm: 140, swing: 0.5, eighths: false, lanes: [
    "X..2 ..x. 3.2. .x..", ".... .... X... ....", "", ".... .... X... ....", "x5x5 x5xr x5x5 R5xr", "", "", "", "", "", ""] };
#[rustfmt::skip]
const DNB: Template = Template { name: "Drum & Bass", bpm: 174, swing: 0.5, eighths: false, lanes: [
    "X... .... ..x. ..2.", "..2. X..2 .2.. X.2.", "", "", "x5x. x.x5 x5x. x.x5", "", "", "", "2... .... .... ....", "", ""] };
#[rustfmt::skip]
const DEMBOW: Template = Template { name: "Dembow", bpm: 96, swing: 0.5, eighths: false, lanes: [
    "X... X... X... X...", "...x ..x. ...x ..x.", "", "", "x.x. x.x. x.x. x.x.", ".... .... .... ..2.", "", "", "", "", "..5. ..5. ..5. ..5."] };
#[rustfmt::skip]
const SHUFFLE: Template = Template { name: "Shuffle", bpm: 100, swing: 0.667, eighths: true, lanes: [
    "X... .... X... ..2.", ".... X.3. .... X.3.", "", "", "", "", "", "", "", "x.x. x.x. x.x. x.x.", ""] };
#[rustfmt::skip]
const BREAKBEAT: Template = Template { name: "Breakbeat", bpm: 130, swing: 0.52, eighths: false, lanes: [
    "X.2. ..x. .x2. ..2.", ".... X..3 .2.. X.2.", "", "", "x5x5 x5x5 x5x5 x5x5", "...2 ..2. .... ....", "", "", "2... .... .... ....", "", ""] };

#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub style: Style,
    /// 0..1: how many of the optional notes fire.
    pub complexity: f32,
    /// 0..1: how far apart loud and quiet hits are. 0.5 is the template's own.
    pub dynamics: f32,
    /// 0..1: 0 no fill, up to half a fill of a beat, past that two beats.
    pub fill: f32,
    pub seed: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self { style: Style::Rock, complexity: 0.4, dynamics: 0.5, fill: 0.0, seed: 1 }
    }
}

/// A small xorshift generator; plugin code never needs anything stronger.
struct Rng(u32);

impl Rng {
    fn new(seed: u32, salt: u32) -> Self {
        let mut r = Rng(seed.wrapping_mul(0x9E37_79B1) ^ salt.wrapping_mul(0x85EB_CA77) ^ 0x1234_5678);
        for _ in 0..4 {
            r.next();
        }
        r
    }

    fn next(&mut self) -> u32 {
        let mut x = self.0.max(1);
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    fn unit(&mut self) -> f32 {
        (self.next() >> 8) as f32 / (1u32 << 24) as f32
    }

    fn below(&mut self, n: u32) -> u32 {
        self.next() % n.max(1)
    }
}

/// Scale a velocity's distance from a normal hit by the dynamics control.
fn shape(vel: f32, dynamics: f32, rng: &mut Rng) -> u8 {
    let spread = 0.35 + 1.3 * dynamics;
    let v = 100.0 + (vel - 100.0) * spread + (rng.unit() - 0.5) * 12.0 * dynamics;
    v.round().clamp(1.0, 127.0) as u8
}

/// Roll the style into `store`, leaving locked lanes as they are.
pub fn generate(store: &PatternStore, s: &Settings) {
    let t = s.style.template();
    let steps = store.steps();
    let mut rng = Rng::new(s.seed, s.style.index() as u32 + 1);
    let chance_scale = (s.complexity * 2.0).clamp(0.0, 2.0);

    for lane in 0..LANES {
        if store.locked(lane) {
            continue;
        }
        store.clear_lane(lane);
        let text: Vec<char> = t.lanes[lane].chars().filter(|c| !c.is_whitespace()).collect();
        if text.is_empty() {
            continue;
        }
        for step in 0..steps {
            let ch = text[step % 16];
            // The crash is a landmark: once, at the top of the loop.
            if lane == LANE_CRASH && step != 0 {
                continue;
            }
            let cell = if let Some(d) = ch.to_digit(10) {
                let p = (d as f32 / 10.0 * chance_scale).min(0.95);
                if rng.unit() < p {
                    // Optional notes are the quiet ones.
                    let loud = if matches!(lane, LANE_HAT | LANE_OPEN) { 78.0 } else { 62.0 };
                    Some(Cell::hit(shape(loud + rng.unit() * 24.0, s.dynamics, &mut rng)))
                } else {
                    None
                }
            } else {
                cell_of(ch).filter(|c| c.is_on()).map(|c| {
                    let v = shape(c.vel() as f32, s.dynamics, &mut rng).max(if c.vel() >= VEL_ACCENT { 100 } else { 1 });
                    c.with_vel(v)
                })
            };
            if let Some(c) = cell {
                store.set(lane, step, c);
            }
        }
    }

    // An open hat chokes the closed one under it.
    if !store.locked(LANE_HAT) || !store.locked(LANE_OPEN) {
        for step in 0..steps {
            if store.get(LANE_OPEN, step).is_on() && !store.locked(LANE_HAT) {
                store.set(LANE_HAT, step, Cell::default());
            }
        }
    }

    if s.fill > 0.02 {
        add_fill(store, s, &mut rng);
    }
    store.set_name(&format!("{} #{}", t.name, s.seed));
}

/// Replace the end of the last bar with a fill.
fn add_fill(store: &PatternStore, s: &Settings, rng: &mut Rng) {
    let steps = store.steps();
    let len = if s.fill > 0.55 { 8 } else { 4 };
    let start = steps - len;
    // What the fill replaces: the drums it plays and the timekeeping it interrupts.
    let owned = [LANE_KICK, LANE_SNARE, LANE_RIM, LANE_CLAP, LANE_HAT, LANE_OPEN, LANE_LTOM, LANE_HTOM, LANE_RIDE];
    for &lane in &owned {
        if !store.locked(lane) {
            for step in start..steps {
                store.set(lane, step, Cell::default());
            }
        }
    }
    let density = 0.55 + 0.45 * s.fill;
    let kind = rng.below(4);
    let put = |lane: usize, step: usize, vel: f32, rng: &mut Rng| {
        if !store.locked(lane) {
            store.set(lane, step, Cell::hit(shape(vel, s.dynamics, rng)));
        }
    };
    for i in 0..len {
        let step = start + i;
        let last = i == len - 1;
        // The final step always sounds so the fill lands; the rest thin out with the fill amount.
        if !last && i > 0 && rng.unit() > density {
            continue;
        }
        let rise = 62.0 + 58.0 * i as f32 / (len - 1) as f32;
        match kind {
            // A snare roll that builds.
            0 => put(LANE_SNARE, step, rise, rng),
            // Toms falling from high to low.
            1 => put(if i < len / 2 { LANE_HTOM } else { LANE_LTOM }, step, rise.max(85.0), rng),
            // Snare against toms.
            2 => {
                if i % 2 == 0 {
                    put(LANE_SNARE, step, rise, rng)
                } else {
                    put(if i % 4 == 1 { LANE_HTOM } else { LANE_LTOM }, step, rise.max(85.0), rng)
                }
            }
            // Kick and snare trading.
            _ => {
                if i % 2 == 0 {
                    put(LANE_KICK, step, 105.0, rng)
                } else {
                    put(LANE_SNARE, step, rise, rng)
                }
            }
        }
    }
    // The crash lands on the next time round.
    if s.fill > 0.5 && !store.locked(LANE_CRASH) {
        store.set(LANE_CRASH, 0, Cell::hit(112));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rolled(s: Settings) -> PatternStore {
        let store = PatternStore::default();
        generate(&store, &s);
        store
    }

    fn snapshot(store: &PatternStore) -> Vec<u32> {
        (0..LANES).flat_map(|l| (0..MAX_STEPS).map(move |s| (l, s))).map(|(l, s)| store.get(l, s).0).collect()
    }

    #[test]
    fn templates_are_sixteen_steps_per_lane() {
        for style in STYLES {
            let t = style.template();
            for (lane, text) in t.lanes.iter().enumerate() {
                let chars: Vec<_> = text.chars().filter(|c| !c.is_whitespace()).collect();
                assert!(chars.is_empty() || chars.len() == 16, "{} lane {lane} has {} steps", t.name, chars.len());
                for c in chars {
                    assert!(c == '.' || c.is_ascii_digit() || cell_of(c).is_some(), "{}: bad char {c:?}", t.name);
                }
            }
            assert!(t.lanes[LANE_KICK].len() > 3 || t.lanes[LANE_SNARE].len() > 3);
        }
    }

    #[test]
    fn same_seed_same_groove_different_seed_differs() {
        let s = Settings { style: Style::Funk, complexity: 0.7, ..Settings::default() };
        assert_eq!(snapshot(&rolled(s)), snapshot(&rolled(s)));
        let other = Settings { seed: 2, ..s };
        assert_ne!(snapshot(&rolled(s)), snapshot(&rolled(other)));
    }

    #[test]
    fn complexity_adds_notes_and_zero_is_the_skeleton() {
        for style in STYLES {
            let count = |c: f32| rolled(Settings { style, complexity: c, ..Settings::default() }).hit_count();
            let (lo, hi) = (count(0.0), count(1.0));
            assert!(hi >= lo, "{style:?}: {hi} < {lo}");
            assert!(lo > 0, "{style:?} skeleton is empty");
        }
        // Zero complexity has no randomness in which notes sound.
        let a = Settings { style: Style::Rock, complexity: 0.0, seed: 5, ..Settings::default() };
        let b = Settings { seed: 99, ..a };
        let hits = |s| {
            let st = rolled(s);
            (0..LANES).map(|l| (0..16).filter(|&x| st.get(l, x).is_on()).count()).collect::<Vec<_>>()
        };
        assert_eq!(hits(a), hits(b));
    }

    #[test]
    fn backbeat_styles_put_the_snare_on_two_and_four() {
        for style in [Style::Rock, Style::Pop, Style::Disco, Style::BoomBap] {
            let st = rolled(Settings { style, complexity: 0.0, ..Settings::default() });
            assert!(st.get(LANE_SNARE, 4).is_on() && st.get(LANE_SNARE, 12).is_on(), "{style:?}");
            assert!(st.get(LANE_KICK, 0).is_on(), "{style:?}");
        }
    }

    #[test]
    fn four_on_the_floor_styles_kick_every_beat() {
        for style in [Style::House, Style::Techno, Style::Disco] {
            let st = rolled(Settings { style, ..Settings::default() });
            assert!((0..4).all(|b| st.get(LANE_KICK, b * 4).vel() > 0), "{style:?}");
        }
    }

    #[test]
    fn two_bars_fill_both_and_the_second_differs_with_complexity() {
        let store = PatternStore::default();
        store.set_bars(2);
        generate(&store, &Settings { style: Style::Funk, complexity: 1.0, ..Settings::default() });
        let bar = |b: usize| (0..LANES).flat_map(|l| (0..16).map(move |s| (l, s))).map(|(l, s)| store.get(l, b * 16 + s).0).collect::<Vec<_>>();
        assert!(bar(1).iter().any(|c| *c != 0));
        assert_ne!(bar(0), bar(1));
    }

    #[test]
    fn locked_lanes_survive_a_roll() {
        let store = PatternStore::default();
        store.set(LANE_KICK, 3, Cell::hit(77));
        store.toggle_lock(LANE_KICK);
        generate(&store, &Settings { fill: 1.0, ..Settings::default() });
        assert_eq!(store.get(LANE_KICK, 3).vel(), 77);
        assert_eq!(store.get(LANE_KICK, 0).vel(), 0, "a locked lane gets nothing new");
    }

    #[test]
    fn fills_replace_the_end_of_the_bar_and_land() {
        let plain = rolled(Settings { style: Style::Rock, complexity: 0.0, fill: 0.0, ..Settings::default() });
        let filled = rolled(Settings { style: Style::Rock, complexity: 0.0, fill: 0.4, ..Settings::default() });
        // The first three beats are unchanged, the last beat is not.
        for lane in 0..LANES {
            for step in 0..12 {
                if lane != LANE_CRASH {
                    assert_eq!(plain.get(lane, step).is_on(), filled.get(lane, step).is_on(), "lane {lane} step {step}");
                }
            }
        }
        assert!(filled.get(LANE_SNARE, 15).is_on() || filled.get(LANE_KICK, 15).is_on() || filled.get(LANE_HTOM, 15).is_on() || filled.get(LANE_LTOM, 15).is_on());
        assert!((12..16).all(|s| !filled.get(LANE_HAT, s).is_on()), "hats stop for the fill");
        let big = rolled(Settings { fill: 1.0, ..Settings::default() });
        assert!(big.get(LANE_CRASH, 0).is_on());
    }

    #[test]
    fn open_hats_choke_closed_ones() {
        let st = rolled(Settings { style: Style::Disco, complexity: 1.0, ..Settings::default() });
        for s in 0..16 {
            assert!(!(st.get(LANE_OPEN, s).is_on() && st.get(LANE_HAT, s).is_on()), "step {s}");
        }
    }

    #[test]
    fn dynamics_widens_the_velocity_range() {
        let spread = |dynamics: f32| {
            let st = rolled(Settings { style: Style::Funk, complexity: 1.0, dynamics, ..Settings::default() });
            let v: Vec<u8> = (0..LANES).flat_map(|l| (0..16).map(move |s| (l, s))).map(|(l, s)| st.get(l, s).vel()).filter(|v| *v > 0).collect();
            *v.iter().max().unwrap() as i32 - *v.iter().min().unwrap() as i32
        };
        assert!(spread(1.0) > spread(0.0));
    }

    #[test]
    fn velocities_stay_in_midi_range() {
        for style in STYLES {
            for seed in 0..20 {
                let st = rolled(Settings { style, complexity: 1.0, dynamics: 1.0, fill: 1.0, seed });
                for l in 0..LANES {
                    for s in 0..16 {
                        assert!(st.get(l, s).vel() <= 127);
                    }
                }
            }
        }
    }
}
