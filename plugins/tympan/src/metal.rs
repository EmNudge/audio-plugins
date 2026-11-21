//! Hats and cymbals: thin bronze plates, with the modes the geometry gives them.
//!
//! A cymbal is a circular plate of bronze (B20: 80% copper, 20% tin), 0.5 to 1.5 mm thick, with a
//! shallow bow (the plate is a very flat dome, not flat) and, on most, a thicker raised bell.
//! Hi-hats are two such plates, the bottom a little thicker than the top. What each is for decides
//! its construction: a ride is large and thick-ish with a big bell, for a clear "ping" and a wash
//! that stays out of the way; a crash is thinner and bell-less for a fast, dense wash; hats are
//! small and thin so they are all attack, and in the closed position the two plates press
//! together, which damps both.
//!
//! The modes of a free circular plate (Rossing and Wilbur measured over 100 in a crash and about
//! 300 in a 16 inch cymbal) have `n` nodal diameters and `s` nodal circles, and their frequencies
//! follow Chladni's law, `f` proportional to `(n + 2 s)^2`, with the constant from the plate:
//! `pi kappa / (8 a^2)`, where `kappa = h c_p / sqrt(12)` is how quickly bending waves travel,
//! `h` the thickness, `a` the radius and `c_p` the plate speed of sound. Many `(n, s)` share each
//! `n + 2 s`, so the modes come in clusters: few and tonal at the bottom (a ride's low partials),
//! spreading into a dense band higher up where the clusters, split by the irregularities of
//! hammering, overlap into noise. The bow adds a floor: curvature `R` raises every mode to
//! `sqrt(f^2 + f_ring^2)` with `f_ring = c_p / (2 pi R)`. The bell is a smaller, stiffer plate of
//! its own, whose few high modes are the ping. A mode at `omega` loses amplitude at
//! `eta omega / 2`, with a loss factor `eta` that grows with frequency.
//!
//! A struck plate also stiffens while it swings hard, so its modes start about 1% sharp and settle:
//! the plate's version of a head's tension modulation, and where a crash's slow wobble comes from.
//! The nonlinear exchange of energy between modes that builds a crash's top end (the von Karman
//! equations) is not simulated.

use crate::shell::Material;
use std::f32::consts::{PI, TAU};

/// Display bins for the editor: modes grouped by frequency, 100 Hz to 20 kHz.
pub const MMODES: usize = 96;
/// The most modes a plate can have.
const MAX_MODES: usize = 2600;

/// B20 bronze.
pub const BRONZE: Material = Material { e: 1.1e11, rho: 8800.0, nu: 0.33, loss: 0.00002 };
/// Below this a plate radiates poorly, so its modes are weighted down.
const RADIATE_CORNER: f32 = 350.0;
/// How many display bands (each about 1/12 octave) the cascade moves energy up in one step, and
/// how fast at most, in 1/s.
const CASCADE_STEP: usize = 6;
const CASCADE_RATE: f32 = 150.0;
/// How much a plate's modes are raised, as a fraction, while it carries all its starting energy.
const STIFFENING: f32 = 0.01;

pub fn hash(a: u32, b: u32) -> f32 {
    let mut h = a.wrapping_mul(0x9E37_79B1) ^ b.wrapping_mul(0x85EB_CA77);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^= h >> 15;
    (h >> 8) as f32 / (1u32 << 24) as f32
}

/// What a cymbal is built like, beyond the diameter and thickness the player chooses.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlateDesign {
    /// Radius of curvature of the bow, metres: large is nearly flat.
    pub curvature: f32,
    /// The bell's diameter and wall thickness, metres. 0 for none.
    pub bell_diameter: f32,
    pub bell_thickness: f32,
    /// For hi-hats: the bottom plate's thickness as a multiple of the top's. 0 for a single plate.
    pub partner: f32,
    /// Decay rate in 1/s added by the plates pressing together (a closed hat), 0 for a free plate.
    /// Friction takes energy at the same rate from every mode, unlike material loss, which grows
    /// with frequency.
    pub contact_rate: f32,
    /// How fast every mode loses energy to the air, in 1/s: a thin plate radiates and drags the air
    /// along at a rate that does not depend on which mode it is, which is why a hat's top end rings
    /// as long as its lows. Larger and thinner plates lose it faster per unit of energy; a ride
    /// (large, thick) keeps it longest of the dishes and a crash longer still.
    pub decay_rate: f32,
    /// Strength of the wash. A crash gets its dense, sustained top end from the plate's nonlinearity
    /// (its stretching as it deflects: the von Karman equations), which passes energy from the loud
    /// low modes up to higher and higher ones, a step at a time: wave turbulence, with a cascade of
    /// energy towards high frequencies (Duering, Josserand and Rica 2006). Treated statistically,
    /// each band of modes is stirred, with random phases, by the energy in the band just below it,
    /// in proportion to that energy squared: a soft tap stays clean and a hard crash washes out.
    /// 0 is a linear plate.
    pub wash: f32,
    /// How far the irregularities of hammering move each mode, as a fraction of its frequency.
    pub irregularity: f32,
    /// How much a mode's decay follows how strongly it is driven. A mode that a blow drives hard
    /// also radiates hard, so it loses its energy to the air faster, and a weakly driven one rings
    /// on: the sound drops quickly and then keeps a thin shimmer. 0 gives every mode the same rate.
    pub radiation: f32,
    /// How unevenly a blow drives the modes. 0 drives them within a few dB of each other (a
    /// smooth, dense wash); above 0 each mode gets a random shape at the strike point raised to this
    /// power, so a few stand out and the spectrum has the peaks that make it sound like metal.
    pub peakiness: f32,
    pub seed: u32,
}

/// What the player sets: size, and how hard the stick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MetalSettings {
    /// Diameter and thickness of the plate, metres.
    pub diameter: f32,
    pub thickness: f32,
    /// Stick hardness: how much of the top end a hit excites, 0 to 1.
    pub tone: f32,
    pub level: f32,
}

impl Default for MetalSettings {
    fn default() -> Self {
        Self { diameter: 0.4, thickness: 0.0008, tone: 0.5, level: 1.0 }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Part {
    Plate,
    Bell,
}

struct Mode {
    hz: f32,
    part: Part,
}

/// Frequencies of the modes of a plate: its cluster structure is the point, so tests look at it.
pub fn mode_frequencies(design: &PlateDesign, s: &MetalSettings, fs: f32) -> Vec<f32> {
    build(design, s, fs).into_iter().map(|m| m.hz).collect()
}

fn build(d: &PlateDesign, s: &MetalSettings, fs: f32) -> Vec<Mode> {
    let mat = BRONZE;
    let cp = (mat.e / (mat.rho * (1.0 - mat.nu * mat.nu))).sqrt();
    let fmax = (fs * 0.45).min(20000.0);
    let mut modes = Vec::new();
    // The plate itself, and the second plate of a hi-hat pair; then the bell.
    let mut layers: Vec<(f32, f32, f32, Part, u32)> = vec![(s.diameter * 0.5, s.thickness, d.curvature, Part::Plate, d.seed)];
    if d.partner > 0.0 {
        layers.push((s.diameter * 0.5, s.thickness * d.partner, d.curvature, Part::Plate, d.seed + 101));
    }
    if d.bell_diameter > 0.0 {
        let a = d.bell_diameter * 0.5;
        // A shallow dome: radius of curvature several times its own radius.
        layers.push((a, d.bell_thickness, a * 6.0, Part::Bell, d.seed + 202));
    }
    for (a, h, curvature, part, seed) in layers {
        let kappa = h * cp / 12f32.sqrt();
        let c0 = PI * kappa / (8.0 * a * a);
        let ring = cp / (TAU * curvature.max(0.01));
        for m in 2u32..=80 {
            let plate = c0 * (m * m) as f32;
            // A plate's lowest-order modes can sit under the bow's floor; the break is on the plate.
            if plate > fmax * 1.2 {
                break;
            }
            for sc in 0..=m / 2 {
                let n = m - 2 * sc;
                // The bow stiffens the axisymmetric modes most (stretching the curved shell); modes
                // with nodal diameters can bend instead and stay lower.
                let floor = ring / (1.0 + 0.5 * (n * n) as f32).sqrt();
                let f0 = (plate * plate + floor * floor).sqrt();
                let pair = if n == 0 { 1 } else { 2 };
                for o in 0..pair {
                    let g = hash(seed.wrapping_mul(7919) + m * 131 + sc, o + 1);
                    // Hammering scatters the modes; the two orientations of a mode split slightly.
                    let mut f = f0 * (1.0 + d.irregularity * (2.0 * g - 1.0));
                    if o == 1 {
                        f *= 1.0 + 0.002 + 0.01 * hash(seed + m * 17 + sc, 5);
                    }
                    if f < fmax && modes.len() < MAX_MODES {
                        modes.push(Mode { hz: f, part });
                    }
                }
            }
        }
    }
    modes.sort_by(|a, b| a.hz.total_cmp(&b.hz));
    modes
}

/// How strongly a mode is driven by a blow at a point. That is the mode's shape there, and on a
/// plate with hundreds of modes the shapes behave like random numbers: modes with a nodal line
/// near the strike are nearly silent and others ring hard, so the spectrum is peaked, not flat.
/// A Gaussian magnitude (the sum of a few uniforms stands in for it) has that spread.
fn shape_at_strike(key: u32, peakiness: f32) -> f32 {
    if peakiness <= 0.0 {
        return 0.4 + hash(key, 7);
    }
    let g: f32 = (0..4).map(|i| hash(key, 7 + i) - 0.5).sum::<f32>() * 1.7;
    g.abs().powf(peakiness) + 0.02
}

pub struct Metal {
    modes: Vec<Mode>,
    x: Vec<f32>,
    y: Vec<f32>,
    c: Vec<f32>,
    s: Vec<f32>,
    /// Each mode's angle per sample and decay per sample at rest, for the stiffening glide.
    theta: Vec<f32>,
    decay: Vec<f32>,
    /// Which display bin each mode is in, and the modes still ringing loudly enough to compute.
    bin: Vec<u8>,
    live: Vec<u16>,
    energy0: f32,
    /// The size of the strike, which the cascade measures energy against.
    reference: f32,
    rng: u32,
    /// What the editor draws: how loud each band of modes is.
    pub env: [f32; MMODES],
    fs: f32,
    pub design: PlateDesign,
    pub settings: MetalSettings,
    choking: bool,
    active: bool,
    tick: u32,
}

impl Metal {
    pub fn new(design: PlateDesign) -> Self {
        let mut m = Self {
            modes: Vec::new(),
            x: Vec::new(),
            y: Vec::new(),
            c: Vec::new(),
            s: Vec::new(),
            theta: Vec::new(),
            decay: Vec::new(),
            bin: Vec::new(),
            live: Vec::new(),
            energy0: 1.0,
            reference: 1.0,
            rng: 0x9E37_79B9,
            env: [0.0; MMODES],
            fs: 48000.0,
            design,
            settings: MetalSettings::default(),
            choking: false,
            active: false,
            tick: 0,
        };
        m.tune(48000.0, MetalSettings::default());
        m
    }

    pub fn active(&self) -> bool {
        self.active
    }

    pub fn set_design(&mut self, design: PlateDesign) {
        self.design = design;
        self.tune(self.fs, self.settings);
    }

    pub fn tune(&mut self, fs: f32, s: MetalSettings) {
        self.fs = fs;
        self.settings = s;
        self.modes = build(&self.design, &s, fs);
        let n = self.modes.len();
        self.x = vec![0.0; n];
        self.y = vec![0.0; n];
        self.c = vec![0.0; n];
        self.s = vec![0.0; n];
        self.theta = vec![0.0; n];
        self.decay = vec![0.0; n];
        self.bin = vec![0; n];
        self.live = Vec::with_capacity(n);
        let eta0 = BRONZE.loss;
        for (k, m) in self.modes.iter().enumerate() {
            let theta = TAU * m.hz / fs;
            // A rate that is the same for every mode (the air, and for a closed hat the plates rubbing
            // together) plus the small material loss, which grows with frequency.
            let eta = eta0 * (1.0 + m.hz / 8000.0);
            let coupling = shape_at_strike(k as u32 + 977 * self.design.seed, 1.0).min(1.0);
            let radiation = 1.0 + self.design.radiation * (coupling - 0.6);
            let rate = self.design.decay_rate * radiation.max(0.6) + self.design.contact_rate + 0.5 * eta * TAU * m.hz;
            let tau = (1.0 / rate.max(1e-6)).max(0.003);
            self.theta[k] = theta;
            self.decay[k] = (-1.0 / (tau * fs)).exp();
            self.c[k] = self.decay[k] * theta.cos();
            self.s[k] = self.decay[k] * theta.sin();
            self.bin[k] = (((m.hz / 100.0).max(1.0).ln() / 200f32.ln()) * MMODES as f32).clamp(0.0, MMODES as f32 - 1.0) as u8;
        }
        self.active = false;
    }

    /// Hit the plate with a stick: on the bow, or on the bell. A hard stick, or a hard hit, puts
    /// more energy into the high modes.
    pub fn strike(&mut self, velocity: f32, bell: bool) {
        let tone = (self.settings.tone + 0.3 * (velocity - 0.5)).clamp(0.0, 1.0);
        let tilt = 1.1 - 1.5 * tone;
        let n = self.modes.len();
        let mut amps = vec![0.0f32; n];
        let mut power = 0.0;
        for (k, m) in self.modes.iter().enumerate() {
            let radiates = m.hz * m.hz / (m.hz * m.hz + RADIATE_CORNER * RADIATE_CORNER);
            // On the bell the bell's modes are driven hard and the plate's lightly, and the other
            // way round on the bow.
            let place = match (m.part, bell) {
                (Part::Bell, true) => 8.0,
                (Part::Plate, true) => 0.08,
                (Part::Bell, false) => 0.1,
                (Part::Plate, false) => 1.0,
            };
            let a = (m.hz / 1000.0).max(0.05).powf(-tilt) * radiates * place * shape_at_strike(k as u32 + 977 * self.design.seed, self.design.peakiness);
            amps[k] = a;
            power += a * a;
        }
        // So the stick's hardness changes the colour, not the loudness.
        let norm = velocity / power.sqrt().max(1e-6);
        for (k, a) in amps.iter().enumerate() {
            let phase = hash(k as u32 + 977 * self.design.seed, 11) * TAU;
            self.x[k] += a * norm * phase.sin();
            self.y[k] += a * norm * phase.cos();
        }
        self.reference = velocity.max(1e-3);
        self.energy0 = (0..n).map(|k| (self.x[k] * self.x[k] + self.y[k] * self.y[k]).sqrt()).sum::<f32>().max(1e-9);
        self.live.clear();
        self.live.extend((0..n as u16).filter(|k| amps[*k as usize] > 0.0));
        self.choking = false;
        self.active = true;
    }

    /// Grab the plate: it dies in a few milliseconds.
    pub fn choke(&mut self) {
        self.choking = true;
    }

    pub fn next(&mut self) -> f32 {
        if !self.active {
            return 0.0;
        }
        let grab = if self.choking { 0.996 } else { 1.0 };
        let mut out = 0.0;
        for &k in &self.live {
            let k = k as usize;
            let (x, y) = (self.x[k], self.y[k]);
            let nx = (x * self.c[k] - y * self.s[k]) * grab;
            let ny = (x * self.s[k] + y * self.c[k]) * grab;
            self.x[k] = nx;
            self.y[k] = ny;
            out += ny;
        }
        self.tick = self.tick.wrapping_add(1);
        if self.tick & 127 == 0 {
            self.refresh();
        }
        out
    }

    /// Every so often: drop the modes that have died away, redraw, and let the stiffening settle.
    fn refresh(&mut self) {
        self.env = [0.0; MMODES];
        let mut total = 0.0;
        let mut kept = 0;
        for i in 0..self.live.len() {
            let k = self.live[i] as usize;
            let amp = (self.x[k] * self.x[k] + self.y[k] * self.y[k]).sqrt();
            total += amp;
            let b = self.bin[k] as usize;
            self.env[b] = self.env[b].max(amp);
            if amp > 2e-5 {
                self.live[kept] = k as u16;
                kept += 1;
            } else {
                self.x[k] = 0.0;
                self.y[k] = 0.0;
            }
        }
        self.live.truncate(kept);
        self.cascade();
        if self.live.is_empty() {
            self.active = false;
            return;
        }
        // Stiffened while it swings hard: sharp at first, settling as the energy goes.
        let sharp = 1.0 + STIFFENING * (total / self.energy0).min(1.5);
        for i in 0..self.live.len() {
            let k = self.live[i] as usize;
            let a = self.theta[k] * sharp;
            self.c[k] = self.decay[k] * a.cos();
            self.s[k] = self.decay[k] * a.sin();
        }
    }

    /// The energy cascade, as a transfer that conserves energy: every refresh, a fraction of each
    /// band's energy moves to the band `CASCADE_STEP` above it, in proportion to how much of the
    /// blow's energy the band still holds, against a full-strength blow (so a soft tap cascades little and a hard crash a lot),
    /// with random phases. What the upper band gains the lower band loses.
    fn cascade(&mut self) {
        if self.design.wash <= 0.0 || self.live.is_empty() {
            return;
        }
        let mut energy = [0.0f32; MMODES];
        let mut count = [0u32; MMODES];
        for &k in &self.live {
            let k = k as usize;
            energy[self.bin[k] as usize] += self.x[k] * self.x[k] + self.y[k] * self.y[k];
            count[self.bin[k] as usize] += 1;
        }
        let dt = 128.0 / self.fs;
        let mut keep = [1.0f32; MMODES];
        let mut gain = [0.0f32; MMODES];
        // From the top down, so a band passes on what it had, not what it has just been given.
        for b in (0..MMODES - CASCADE_STEP).rev() {
            // Against a full-strength blow, not this one: the nonlinearity depends on how far the plate
            // actually swings.
            let share = energy[b].sqrt().min(1.0);
            let moved = (self.design.wash * CASCADE_RATE * share * dt).min(0.5) * energy[b];
            if moved > 0.0 && count[b + CASCADE_STEP] > 0 {
                keep[b] = (1.0 - moved / energy[b].max(1e-30)).sqrt();
                gain[b + CASCADE_STEP] += moved;
            }
        }
        for i in 0..self.live.len() {
            let k = self.live[i] as usize;
            let b = self.bin[k] as usize;
            self.x[k] *= keep[b];
            self.y[k] *= keep[b];
            if gain[b] > 0.0 {
                self.rng ^= self.rng << 13;
                self.rng ^= self.rng >> 17;
                self.rng ^= self.rng << 5;
                let phase = self.rng as f32 / u32::MAX as f32 * TAU;
                let amp = (gain[b] / count[b] as f32).sqrt();
                self.x[k] += amp * phase.cos();
                self.y[k] += amp * phase.sin();
            }
        }
    }

    pub fn reset(&mut self) {
        let (fs, s, design) = (self.fs, self.settings, self.design);
        *self = Metal::new(design);
        self.tune(fs, s);
    }
}
