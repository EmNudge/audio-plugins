//! Four drums, each a membrane feeding a modal shell, and a set of snare wires that buzz in
//! sympathy with whatever is ringing.

use crate::drum::{cavity_stiffness, tune_stick, Bank, Cavity, Mallet, STICK_MODES};
use crate::shell::{self, Geometry, Material, Shell, HEAD_AREAL_MASS, SHELL_MODES};
use crate::membrane::{Membrane, NN};
use crate::metal::{Metal, MetalSettings, PlateDesign, MMODES};
use nih_plug::prelude::{AtomicF32, Enum};
use std::sync::atomic::{AtomicU32, Ordering};

pub const HEADS: usize = 4;
pub const MODES: usize = SHELL_MODES;
pub const KICK: usize = 0;
pub const SNARE: usize = 1;
pub const TOM_LO: usize = 2;
pub const TOM_HI: usize = 3;

pub const METALS: usize = 4;
pub const HAT: usize = 0;
pub const OPEN: usize = 1;
pub const CRASH: usize = 2;
pub const RIDE: usize = 3;
pub const METAL_NAMES: [&str; METALS] = ["HAT", "OPEN HAT", "CRASH", "RIDE"];
pub const METAL_NOTES: [u8; METALS] = [42, 46, 49, 51];
/// What makes each drum the instrument it is, beyond the knobs the player turns: the mass of the
/// mallet, the wood of the stick, the second head and the shell. Fitted to recordings
/// (`examples/fit.rs`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeadDesign {
    /// Mass of the mallet relative to the whole head: a stick is light, a bass drum beater heavy.
    pub mallet_mass: f32,
    /// Lowest bending mode of the stick, Hz, and how long it rings, seconds.
    pub stick_hz: f32,
    pub stick_tau: f32,
    /// Pitch of the second head relative to the first, and how long it rings, seconds.
    pub res_ratio: f32,
    pub res_decay: f32,
    /// How hard the player hits, as a multiple of the speed that sets the head swinging at
    /// amplitude 1: a kick is hit far harder than a snare is, which is what makes it a thump and
    /// not a tap, and the harder a head is driven the more it bends in pitch and the more of the
    /// blow's energy is lost in the contact as impact noise.
    pub drive: f32,
}

pub const HEAD_DESIGN: [HeadDesign; HEADS] = [
    // A bass drum: a heavy felt beater and a second head tuned a little below the batter.
    HeadDesign { mallet_mass: 1.5, stick_hz: 180.0, stick_tau: 0.01, res_ratio: 0.9, res_decay: 0.3, drive: 2.5 },
    // A snare drum: a stick, and a snare-side head tuned above the batter.
    HeadDesign { mallet_mass: 0.5, stick_hz: 900.0, stick_tau: 0.01, res_ratio: 1.3, res_decay: 0.12, drive: 3.2 },
    HeadDesign { mallet_mass: 0.6, stick_hz: 450.0, stick_tau: 0.012, res_ratio: 1.1, res_decay: 0.3, drive: 1.6 },
    HeadDesign { mallet_mass: 0.6, stick_hz: 450.0, stick_tau: 0.012, res_ratio: 1.1, res_decay: 0.3, drive: 1.6 },
];

/// How each plate is built (see `metal.rs`). Sizes are typical for each instrument.
pub const METAL_DESIGN: [PlateDesign; METALS] = [
    // Closed hat: two 14 inch plates, the bottom a quarter thicker, pressed together.
    PlateDesign { curvature: 1.4, bell_diameter: 0.0, bell_thickness: 0.0, partner: 1.25, contact_rate: 18.0, decay_rate: 6.0, wash: 3.0, irregularity: 0.10, radiation: 0.0, peakiness: 0.0, seed: 0 },
    // Open hat: the same plates with a gap between them.
    PlateDesign { curvature: 1.4, bell_diameter: 0.0, bell_thickness: 0.0, partner: 1.25, contact_rate: 0.0, decay_rate: 3.5, wash: 0.0, irregularity: 0.10, radiation: 3.5, peakiness: 2.2, seed: 0 },
    // Crash: 17 inches, thin, with a small bell and more bow.
    PlateDesign { curvature: 1.1, bell_diameter: 0.045, bell_thickness: 0.0025, partner: 0.0, contact_rate: 0.0, decay_rate: 1.2, wash: 8.0, irregularity: 0.06, radiation: 0.0, peakiness: 0.0, seed: 2 },
    // Ride: 21 inches, flatter, with a big bell.
    PlateDesign { curvature: 3.0, bell_diameter: 0.075, bell_thickness: 0.003, partner: 0.0, contact_rate: 0.0, decay_rate: 3.2, wash: 2.0, irregularity: 0.04, radiation: 0.0, peakiness: 0.0, seed: 3 },
];
/// Output trim, as for the drums.
const METAL_TRIM: [f32; METALS] = [0.34, 0.33, 0.16, 0.35];
/// Where each plate sits in the stereo field.
const METAL_PAN: [f32; METALS] = [0.25, 0.25, -0.3, 0.35];

/// Which plate a General MIDI note plays: the pedal hat sounds the closed hat.
pub fn note_metal(note: u8) -> Option<usize> {
    match note {
        42 | 44 => Some(HAT),
        46 => Some(OPEN),
        49 | 52 | 55 | 57 => Some(CRASH),
        51 | 53 | 59 => Some(RIDE),
        _ => None,
    }
}

pub const NAMES: [&str; HEADS] = ["KICK", "SNARE", "TOM LO", "TOM HI"];
/// The General MIDI note each drum is shown under.
pub const NOTES: [u8; HEADS] = [36, 38, 45, 48];

/// Which drum a General MIDI note plays.
pub fn note_head(note: u8) -> Option<usize> {
    match note {
        35 | 36 => Some(KICK),
        37..=40 => Some(SNARE),
        41 | 43 | 45 => Some(TOM_LO),
        47 | 48 | 50 => Some(TOM_HI),
        _ => None,
    }
}

/// Output trim per drum. The velocity pickups hear high modes louder than low ones, so the kick
/// needs more than the snare to sit level with it.
const TRIM: [f32; HEADS] = [0.239, 0.569, 0.789, 1.34];
/// The final level of each drum, on top of `TRIM`: `TRIM` is what the drums were fitted with and
/// this brings them to a common loudness. The wires follow the drum's final level.
const OUT: [f32; HEADS] = [2.3, 0.23, 0.474, 0.298];
/// How loud a fully-up shell is next to the head.

/// Everything the editor reads. Written by the audio thread, a block at a time.
pub struct Meters {
    /// Per drum, the displacement envelope grid and then the velocity envelope grid.
    pub grid: Vec<AtomicF32>,
    pub hit: [AtomicF32; HEADS],
    /// Envelope of each shell resonance.
    pub modes: Vec<AtomicF32>,
    pub wires: AtomicF32,
    pub out_peak: AtomicF32,
    /// The editor asks for a hit by bumping a counter, with where and how hard.
    pub audition: [AtomicU32; HEADS],
    pub audition_at: [[AtomicF32; 2]; HEADS],
    pub audition_vel: [AtomicF32; HEADS],
    /// The same for the plates: each mode's amplitude, a flash, and hits asked for by the editor.
    pub metal_modes: Vec<AtomicF32>,
    pub metal_hit: [AtomicF32; METALS],
    pub metal_audition: [AtomicU32; METALS],
    pub metal_audition_vel: [AtomicF32; METALS],
}

impl Default for Meters {
    fn default() -> Self {
        let z = || AtomicF32::new(0.0);
        Self {
            grid: (0..HEADS * 2 * NN).map(|_| z()).collect(),
            hit: std::array::from_fn(|_| z()),
            modes: (0..HEADS * MODES).map(|_| z()).collect(),
            wires: z(),
            out_peak: z(),
            audition: std::array::from_fn(|_| AtomicU32::new(0)),
            audition_at: std::array::from_fn(|_| [z(), z()]),
            audition_vel: std::array::from_fn(|_| z()),
            metal_modes: (0..METALS * MMODES).map(|_| z()).collect(),
            metal_hit: std::array::from_fn(|_| z()),
            metal_audition: std::array::from_fn(|_| AtomicU32::new(0)),
            metal_audition_vel: std::array::from_fn(|_| z()),
        }
    }
}

impl Meters {
    /// `kind` 0 is the displacement envelope, 1 the velocity envelope.
    pub fn head_grid(&self, head: usize, kind: usize) -> &[AtomicF32] {
        let start = (head * 2 + kind) * NN;
        &self.grid[start..start + NN]
    }

    pub fn head_modes(&self, head: usize) -> &[AtomicF32] {
        &self.modes[head * MODES..(head + 1) * MODES]
    }

    pub fn plate_modes(&self, plate: usize) -> &[AtomicF32] {
        &self.metal_modes[plate * MMODES..(plate + 1) * MMODES]
    }

    pub fn request_plate_hit(&self, plate: usize, velocity: f32) {
        self.metal_audition_vel[plate].store(velocity, Ordering::Relaxed);
        self.metal_audition[plate].fetch_add(1, Ordering::Release);
    }

    /// Ask for a hit from the editor.
    pub fn request_hit(&self, head: usize, x: f32, y: f32, velocity: f32) {
        self.audition_at[head][0].store(x, Ordering::Relaxed);
        self.audition_at[head][1].store(y, Ordering::Relaxed);
        self.audition_vel[head].store(velocity, Ordering::Relaxed);
        self.audition[head].fetch_add(1, Ordering::Release);
    }
}

/// What strikes the head. The fitted drum design is for a wooden stick; the others scale it.
#[derive(Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Striker {
    /// A wooden drumstick.
    #[name = "Stick"]
    Stick,
    /// A felt or plastic bass-drum beater: heavy, soft, and it does not ring.
    #[name = "Beater"]
    Beater,
    /// A felt mallet, as on timpani: soft and wide.
    #[name = "Felt"]
    Felt,
    /// A palm and fingers, as on a conga or djembe: soft and wide, with no wood to ring.
    #[name = "Hand"]
    Hand,
}

/// How a striker differs from the fitted wooden stick.
pub struct StrikerSpec {
    /// Mass of the mallet, as a multiple.
    pub mass: f32,
    /// How loudly it rings, as a multiple, and its lowest bending mode, as a multiple.
    pub ring: f32,
    pub hz: f32,
    /// Added to the mallet hardness, and a multiple of the contact patch width.
    pub hard: f32,
    pub reach: f32,
}

impl Striker {
    pub fn spec(self) -> StrikerSpec {
        match self {
            Striker::Stick => StrikerSpec { mass: 1.0, ring: 1.0, hz: 1.0, hard: 0.0, reach: 1.0 },
            Striker::Beater => StrikerSpec { mass: 2.5, ring: 0.4, hz: 0.5, hard: -0.3, reach: 1.6 },
            Striker::Felt => StrikerSpec { mass: 1.5, ring: 0.15, hz: 0.7, hard: -0.5, reach: 2.0 },
            Striker::Hand => StrikerSpec { mass: 1.0, ring: 0.0, hz: 1.0, hard: -0.4, reach: 2.6 },
        }
    }
}

/// What the shell is made of. The fitted design is for wood.
#[derive(Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellKind {
    #[name = "Wood"]
    Wood,
    /// Steel or brass: stiffer, so its modes are higher, and it rings.
    #[name = "Metal"]
    Metal,
    /// Fibreglass or acrylic: in between.
    #[name = "Composite"]
    Composite,
    /// No shell, as on a frame drum.
    #[name = "None"]
    None,
}

impl ShellKind {
    /// The material, or none for a drum with no shell.
    pub fn material(self) -> Option<Material> {
        match self {
            ShellKind::Wood => Some(shell::WOOD),
            ShellKind::Metal => Some(shell::STEEL),
            ShellKind::Composite => Some(shell::COMPOSITE),
            ShellKind::None => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeadSettings {
    pub striker: Striker,
    pub shell_kind: ShellKind,
    /// Resting pitch of the head, Hz.
    pub tune: f32,
    /// Decay time of the fundamental, seconds.
    pub decay: f32,
    pub hard: f32,
    pub bend: f32,
    /// How loudly the stick's own wood rings, 0 to 1.
    pub click: f32,
    /// The shell, in metres: diameter, length (how deep the drum is) and wall thickness.
    pub diameter: f32,
    pub length: f32,
    pub thickness: f32,
    pub level: f32,
    pub pan: f32,
    /// Where the mallet lands, in the unit disc.
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Global {
    /// How much faster the high modes of the skin die, 0 to 1.
    pub skin: f32,
    pub wires: f32,
}

impl Default for HeadSettings {
    fn default() -> Self {
        Self { striker: Striker::Stick, shell_kind: ShellKind::Wood, tune: 100.0, decay: 0.4, hard: 0.5, bend: 0.0, click: 0.0, diameter: 0.35, length: 0.14, thickness: 0.007, level: 1.0, pan: 0.0, x: 0.0, y: 0.0 }
    }
}

impl Default for Global {
    fn default() -> Self {
        Self { skin: 0.25, wires: 0.15 }
    }
}

/// How firmly the rim is tied to the shell, as a multiple of the head's own stiffness.
const RIM_STIFFNESS: f32 = 1.0;
/// How loud the shell is next to the head. A wooden shell moves so little next to a head (it
/// weighs dozens of heads) that at unit gain it would be 77 dB down; radiated sound is taken to be
/// about 20 dB below the head for wood, and steel, heavier, is lower.
const SHELL_OUT: f32 = 700.0;
/// How loud the impact noise is: with a standard blow it is about a third of the head's own level.
const IMPACT_GAIN: f32 = 1.0;
/// How loud the stick's ring is next to the head, with `click` at 1.
const STICK_OUT: f32 = 1.0;
/// How loud the second head is next to the first, with the cavity at full.
const RES_OUT: f32 = 1.0;

struct Head {
    membrane: Membrane,
    mallet: Mallet,
    stick: Bank<STICK_MODES>,
    shell: Shell,
    cavity: Cavity,
    edge_dirty: bool,
    s: HeadSettings,
    g: Global,
    design: HeadDesign,
    /// One over the force of a nominal full-strength blow, so the stick's input does not depend
    /// on how the drum is tuned.
    stick_gain: f32,
    /// Loudness compensation for the hit: a hard, short contact excites many more head modes than
    /// a soft one, so the same blow is much louder; this evens it out. It eases to its new value
    /// so a ringing tail is not jolted.
    norm: f32,
    norm_now: f32,
    /// The whack: noise from the energy the contact loses, its envelope, filter and normalisation.
    impact_norm: f32,
    impact_env: f32,
    impact_decay: f32,
    impact_lp: f32,
    impact_k: f32,
    rng: u32,
    noise_scale: f32,
    /// Whether the editor has seen the head at rest, so it stops being redrawn.
    shown_idle: bool,
    tick: u32,
}

impl Default for Head {
    fn default() -> Self {
        Self {
            membrane: Membrane::default(),
            mallet: Mallet::default(),
            stick: Bank::default(),
            shell: {
                let m = Membrane::default();
                let (rim, theta) = m.rim();
                Shell::new(rim, theta, m.cells())
            },
            cavity: Cavity::default(),
            edge_dirty: false,
            s: HeadSettings::default(),
            g: Global::default(),
            design: HEAD_DESIGN[0],
            stick_gain: 1.0,
            norm: 1.0,
            norm_now: 1.0,
            impact_norm: 0.0,
            impact_env: 0.0,
            impact_decay: 0.99,
            impact_lp: 0.0,
            impact_k: 0.5,
            rng: 0x2545_F491,
            noise_scale: 1.0,
            shown_idle: false,
            tick: 0,
        }
    }
}

/// Snare wires: noise through a band that follows the drums' own loudness.
struct Wires {
    rng: u32,
    hp: f32,
    lp: f32,
    hp_state: f32,
    lp_state: f32,
    env: f32,
    attack: f32,
    release: f32,
    scale: f32,
}

impl Default for Wires {
    fn default() -> Self {
        Self { rng: 0x1234_5678, hp: 0.0, lp: 0.0, hp_state: 0.0, lp_state: 0.0, env: 0.0, attack: 0.0, release: 0.0, scale: 1.0 }
    }
}

impl Wires {
    fn set_sample_rate(&mut self, fs: f32) {
        let pole = |hz: f32| 1.0 - (-std::f32::consts::TAU * hz / fs).exp();
        // Wires are steel coils: they sound from about a kilohertz to the top of the spectrum, and
        // they answer the strike at once and buzz on for a few hundredths of a second, no more.
        self.hp = pole(1000.0);
        self.lp = pole(7000.0);
        self.attack = pole(2000.0);
        self.release = (-1.0 / (0.035 * fs)).exp();
        self.scale = (fs / 44100.0).sqrt();
    }

    /// `drive` is how hard the drums are shaking the wires.
    fn next(&mut self, drive: f32) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        let n = self.rng as f32 / u32::MAX as f32 * 2.0 - 1.0;
        self.lp_state += self.lp * (n - self.lp_state);
        self.hp_state += self.hp * (self.lp_state - self.hp_state);
        let band = self.lp_state - self.hp_state;
        self.env = if drive > self.env { self.env + self.attack * (drive - self.env) } else { self.env * self.release };
        // Linear in how hard the drums are shaking them, so they fade with the snare instead of
        // hissing at full whenever anything rings.
        // Proportional to how hard the head shakes them, saturating only for a violent hit.
        band * (self.env / (self.env + 0.1)) * 1.1 * self.scale
    }
}

pub struct Engine {
    fs: f32,
    heads: [Head; HEADS],
    metals: [Metal; METALS],
    wires: Wires,
    global: Global,
}

impl Default for Engine {
    fn default() -> Self {
        let mut e = Self { fs: 48000.0, heads: std::array::from_fn(|i| Head { design: HEAD_DESIGN[i], ..Head::default() }), metals: std::array::from_fn(|i| Metal::new(METAL_DESIGN[i])), wires: Wires::default(), global: Global::default() };
        e.set_sample_rate(48000.0);
        e
    }
}

impl Engine {
    pub fn set_sample_rate(&mut self, fs: f32) {
        self.fs = fs;
        self.wires.set_sample_rate(fs);
        self.stop_all();
        for h in 0..HEADS {
            let (s, g) = (self.heads[h].s, self.global);
            self.apply(h, s, g);
        }
    }

    /// Silence everything. The heads keep their tuning: a host calls this on every transport start,
    /// and a blank membrane would play untuned until some setting happened to change.
    pub fn stop_all(&mut self) {
        for h in &mut self.heads {
            h.membrane = Membrane::default();
            h.mallet.clear();
            h.stick.clear();
            h.shell.clear();
            h.cavity.clear();
            h.shown_idle = false;
        }
        self.wires.env = 0.0;
        for m in &mut self.metals {
            m.reset();
        }
        for h in 0..HEADS {
            let (s, g) = (self.heads[h].s, self.heads[h].g);
            self.apply(h, s, g);
        }
    }

    fn apply(&mut self, head: usize, s: HeadSettings, g: Global) {
        let h = &mut self.heads[head];
        h.s = s;
        h.g = g;
        let tilt = 0.02 + 1.2 * g.skin * g.skin;
        h.membrane.tune(self.fs, s.tune, s.decay, tilt, s.bend);
        let st = s.striker.spec();
        tune_stick(&mut h.stick, self.fs, h.design.stick_hz * st.hz, h.design.stick_tau);
        match s.shell_kind.material() {
            Some(mat) => {
                // The shell holds the rim and the air in it is a spring between the heads.
                let geom = Geometry { radius: s.diameter * 0.5, length: s.length, thickness: s.thickness };
                h.shell.tune(self.fs, geom, mat, h.membrane.omega(), RIM_STIFFNESS);
                h.membrane.set_edge_stiffness(h.shell.kappa());
                h.cavity.tune(self.fs, h.membrane.omega(), h.design.res_ratio, h.design.res_decay, cavity_stiffness(self.fs, s.length, HEAD_AREAL_MASS));
            }
            None => {
                h.shell.disable();
                h.membrane.set_edge_stiffness(0.0);
                h.cavity.tune(self.fs, h.membrane.omega(), h.design.res_ratio, h.design.res_decay, 0.0);
            }
        }
    }

    /// Update one plate's settings.
    pub fn set_metal(&mut self, plate: usize, s: MetalSettings) {
        if s != self.metals[plate].settings {
            self.metals[plate].tune(self.fs, s);
        }
    }

    pub fn set_metal_design(&mut self, plate: usize, design: PlateDesign) {
        if design != self.metals[plate].design {
            self.metals[plate].set_design(design);
        }
    }

    /// Hit a plate. The closed hat chokes the open one.
    pub fn trigger_metal(&mut self, plate: usize, velocity: f32, bell: bool) {
        if plate == HAT {
            self.metals[OPEN].choke();
        }
        self.metals[plate].strike(velocity.clamp(0.0, 1.0), bell);
    }

    pub fn set_head_design(&mut self, head: usize, d: HeadDesign) {
        if d != self.heads[head].design {
            self.heads[head].design = d;
            let (s, g) = (self.heads[head].s, self.global);
            self.apply(head, s, g);
        }
    }

    pub fn set_global(&mut self, g: Global) {
        self.global = g;
    }

    /// Update one drum's settings. Cheap, but not free: call it once per block.
    pub fn set_head(&mut self, head: usize, s: HeadSettings) {
        let g = self.global;
        if s != self.heads[head].s || g != self.heads[head].g {
            self.apply(head, s, g);
        }
    }

    /// Hit a drum. `at` overrides the position set in the head's settings.
    pub fn trigger(&mut self, head: usize, velocity: f32, at: Option<(f32, f32)>) {
        let fs = self.fs;
        let h = &mut self.heads[head];
        let (x, y) = at.unwrap_or((h.s.x, h.s.y));
        let velocity = velocity.clamp(0.0, 1.0);
        // A harder hit makes the contact harder too.
        let was_idle = !h.membrane.active() && !h.mallet.active;
        let st = h.s.striker.spec();
        let hard = (h.s.hard + st.hard + 0.3 * (velocity - 0.5)).clamp(0.0, 1.0);
        let reach = (6.0 - 4.7 * hard) * st.reach;
        let squares = h.membrane.set_footprint(x, y, reach);
        // The part of the head the mallet lands on weighs about this much of the whole.
        let patch = 1.0 / (h.membrane.cells() * squares.max(1e-9));
        let mass = h.design.mallet_mass * st.mass;
        let reduced = mass * patch / (mass + patch);
        // The speed that sets the fundamental swinging at amplitude 1, as the loudness reference.
        let v_full = crate::drum::full_speed(h.membrane.omega(), reduced);
        let (uc, _) = h.membrane.probe();
        h.mallet.start(fs, hard, velocity * h.design.drive, v_full, mass, patch, uc);
        h.membrane.wake();
        let t = crate::drum::contact_time(hard) * fs;
        h.stick_gain = 1.0 / (reduced * v_full * 1.7 * std::f32::consts::FRAC_PI_2 / t).max(1e-12);
        // The energy the contact loses, against that of a standard blow on a head tuned to 150 Hz
        // (so it follows how hard the stick actually moves, not how the head is tuned); how bright
        // it is follows the hardness of the tip; it lasts a few milliseconds.
        let v_ref = crate::drum::full_speed(std::f32::consts::TAU * 150.0 / fs, reduced);
        // Against a fixed 2 ms, so the noise follows the energy lost and not how long the contact
        // lasts: a soft mallet's long contact is quiet and dull, not louder.
        h.impact_norm = 0.002 * fs / (reduced * v_ref * v_ref).max(1e-18);
        let fc = (1500.0 * (3.0 * hard).exp2()).min(fs * 0.4);
        h.impact_k = 1.0 - (-std::f32::consts::TAU * fc / fs).exp();
        h.impact_decay = (-1.0 / (0.003 * fs)).exp();
        h.noise_scale = (fs / 44100.0).sqrt();
        // The contact's force pulse reaches modes up to about 1 / (2 T); a circular membrane has
        // about 1.45 rho^2 modes below rho times its fundamental, and the output is velocity, so
        // each adds the same: the level grows with the square root of the count.
        let f1 = h.membrane.omega() * fs / std::f32::consts::TAU;
        let rho = (0.5 / crate::drum::contact_time(hard) / f1.max(1.0)).min(13.0);
        h.norm = 1.0 / (1.2 * rho).max(1.0);
        if was_idle {
            h.norm_now = h.norm;
        }
        h.shown_idle = false;
    }

    /// One stereo sample.
    pub fn next(&mut self) -> (f32, f32) {
        let (mut l, mut r) = (0.0f32, 0.0f32);
        let mut drive = 0.0f32;
        for (i, h) in self.heads.iter_mut().enumerate() {
            // The mallet pushes on the head, and the air between the heads pushes on both.
            let mut force = 0.0;
            if h.mallet.active {
                let (uc, vc) = h.membrane.probe();
                force = h.mallet.step(uc, vc);
                h.membrane.wake();
            }
            let coupled = h.cavity.kappa() > 0.0;
            let pressure = if coupled { h.cavity.kappa() * (h.membrane.mean() + h.cavity.mean()) } else { 0.0 };
            h.membrane.drive(force * h.membrane.cells(), -pressure);
            if coupled {
                h.cavity.step(pressure);
                h.tick = h.tick.wrapping_add(1);
                if h.tick & 255 == 0 && h.cavity.energy() < 1e-20 {
                    h.cavity.clear();
                }
            }
            h.membrane.step();
            let (a, b) = h.membrane.pickups();
            let drive_in = force * h.stick_gain;
            let second = if coupled { h.cavity.velocity() / h.membrane.omega() * RES_OUT } else { 0.0 };
            let stick = h.stick.process(drive_in) * h.s.click * h.s.striker.spec().ring * STICK_OUT;
            // The whack of the impact: noise with the energy the contact loses, then gone in a few ms.
            let impact = if h.mallet.active || h.impact_env > 1e-6 {
                let a = (h.mallet.dissipation() * h.impact_norm).sqrt();
                h.impact_env = a.max(h.impact_env * h.impact_decay);
                h.rng ^= h.rng << 13;
                h.rng ^= h.rng >> 17;
                h.rng ^= h.rng << 5;
                let n = h.rng as f32 / u32::MAX as f32 * 2.0 - 1.0;
                h.impact_lp += h.impact_k * (n - h.impact_lp);
                h.impact_lp * h.impact_env * IMPACT_GAIN * h.noise_scale
            } else {
                0.0
            };
            let mono = (a + b) * 0.5 + stick + second + impact;
            // The shell, driven by the head's rim and pushing back on it.
            let shell_velocity = {
                let shell = &mut h.shell;
                h.membrane.with_edge(|u, edge| shell.step(u, edge))
            };
            if h.shell.active() {
                h.edge_dirty = true;
            } else if h.edge_dirty {
                h.membrane.with_edge(|_, edge| edge.fill(0.0));
                h.edge_dirty = false;
            }
            let shell = shell_velocity / h.membrane.omega() * SHELL_OUT;
            // Half the stereo spread between the two microphones.
            let spread = 0.25 * (a - b);
            h.norm_now += (h.norm - h.norm_now) * 0.002;
            let (hl, hr) = ((mono + spread + shell) * h.norm_now, (mono - spread + shell) * h.norm_now);
            let (gl, gr) = pan_gains(h.s.pan);
            l += hl * gl * h.s.level * TRIM[i] * OUT[i];
            r += hr * gr * h.s.level * TRIM[i] * OUT[i];
            // The snare shakes the wires; the others only faintly, through the stand.
            drive += mono.abs() * h.norm_now * TRIM[i] * OUT[i] * h.s.level * if i == SNARE { 1.0 } else { 0.004 };
        }
        for (i, m) in self.metals.iter_mut().enumerate() {
            let v = m.next() * METAL_TRIM[i] * m.settings.level;
            let (gl, gr) = pan_gains(METAL_PAN[i]);
            l += v * gl;
            r += v * gr;
        }
        let wires = self.wires.next(drive) * self.global.wires;
        let (gl, gr) = pan_gains(self.heads[SNARE].s.pan);
        l += wires * gl * self.heads[SNARE].s.level;
        r += wires * gr * self.heads[SNARE].s.level;
        (l, r)
    }

    /// Hand the editor what it draws.
    pub fn publish(&mut self, m: &Meters) {
        for (i, h) in self.heads.iter_mut().enumerate() {
            let idle = !h.membrane.active();
            if !idle || !h.shown_idle {
                let (disp, vel) = h.membrane.envelopes();
                for (slot, v) in m.head_grid(i, 0).iter().zip(disp.iter()) {
                    slot.store(*v, Ordering::Relaxed);
                }
                for (slot, v) in m.head_grid(i, 1).iter().zip(vel.iter()) {
                    slot.store(*v, Ordering::Relaxed);
                }
                h.shown_idle = idle;
            }
            for (k, slot) in m.head_modes(i).iter().enumerate() {
                slot.store(h.shell.amplitude(k) / h.membrane.omega().max(1e-6), Ordering::Relaxed);
            }
        }
        for (i, plate) in self.metals.iter().enumerate() {
            for (slot, v) in m.plate_modes(i).iter().zip(plate.env.iter()) {
                slot.store(*v, Ordering::Relaxed);
            }
        }
        m.wires.store(self.wires.env, Ordering::Relaxed);
    }

    /// Whether anything is still sounding.
    pub fn is_active(&self) -> bool {
        self.heads.iter().any(|h| h.membrane.active() || h.mallet.active || h.shell.active() || h.cavity.energy() > 1e-12) || self.wires.env > 1e-5 || self.metals.iter().any(|m| m.active())
    }

    pub fn energy(&self, head: usize) -> f32 {
        self.heads[head].membrane.energy()
    }
}

/// Balance-style pan: the far side comes down, the near side stays.
fn pan_gains(pan: f32) -> (f32, f32) {
    ((1.0 - pan).min(1.0), (1.0 + pan).min(1.0))
}
