//! The parts of a drum around its head: the stick that strikes it, the stick's own wood, the
//! second head and the air between the two, and the shell. Each follows what the acoustics
//! literature says about it.
//!
//! * **The mallet** collides with the head through a Hunt-Crossley contact force,
//!   `f = k x^1.5 (1 + 1.5 (1 - e) x' / v0)`, a Hertzian spring with hysteresis (Avanzini and
//!   Rocchesso, "Modeling collision sounds: non-linear contact force", DAFx 2001). The contact
//!   time is not set but falls out of the stiffness: a few milliseconds for a stick on a tom
//!   (Wagner 2006; Dahl), shorter for a harder or faster hit, which is how a harder hit gets
//!   brighter.
//! * **The stick** is a free-free beam: its bending modes sit at 1, 2.76, 5.40, 8.93 ... times the
//!   lowest (Euler-Bernoulli), and ring a few milliseconds after the impact. That is the "tick" of
//!   wood on a head.
//! * **The second head** is coupled to the first through the air between them (Rossing; Zhao
//!   1990): a sealed cavity is a spring that pushes back on the sum of the volumes the two heads
//!   sweep. That couples the symmetric modes and splits each into a pair, a lower one with the
//!   heads moving together and a higher one where they breathe in and out.
//! * **The shell** is a cylinder. Its ring modes go as `n (n^2 - 1) / sqrt(n^2 + 1)` (the
//!   inextensional bending modes of a thin cylindrical shell), and its damping depends on the
//!   material: wood loses its high modes quickly, metal rings.

use std::f32::consts::TAU;

/// Frequencies of a free-free beam's bending modes relative to the lowest.
pub const BEAM_RATIO: [f32; 6] = [1.0, 2.757, 5.404, 8.933, 13.34, 18.64];
pub const STICK_MODES: usize = 6;
const STICK_WEIGHT: [f32; STICK_MODES] = [1.0, 0.7, 0.5, 0.35, 0.25, 0.2];

pub const SHELL_MODES: usize = 8;
const SHELL_WEIGHT: [f32; SHELL_MODES] = [1.0, 0.8, 0.7, 0.55, 0.45, 0.35, 0.3, 0.25];

/// Ring modes of a thin cylindrical shell, n = 2 to 9, relative to the first.
pub fn shell_ratios() -> [f32; SHELL_MODES] {
    let f = |n: f32| n * (n * n - 1.0) / (n * n + 1.0).sqrt();
    std::array::from_fn(|k| f(k as f32 + 2.0) / f(2.0))
}

/// A bank of two-pole resonators, each normalised to unit gain at its own centre.
pub struct Bank<const N: usize> {
    y1: [f32; N],
    y2: [f32; N],
    a1: [f32; N],
    a2: [f32; N],
    g: [f32; N],
    pub env: [f32; N],
    release: f32,
}

impl<const N: usize> Default for Bank<N> {
    fn default() -> Self {
        Self { y1: [0.0; N], y2: [0.0; N], a1: [0.0; N], a2: [0.0; N], g: [0.0; N], env: [0.0; N], release: 0.999 }
    }
}

impl<const N: usize> Bank<N> {
    /// Centre frequencies in Hz, decay times in seconds and weights.
    pub fn tune(&mut self, fs: f32, hz: &[f32; N], tau: &[f32; N], weight: &[f32; N]) {
        for k in 0..N {
            let theta = TAU * hz[k].min(fs * 0.45) / fs;
            let live = hz[k] < fs * 0.45;
            let r = (-1.0 / (tau[k].max(0.002) * fs)).exp();
            self.a1[k] = 2.0 * r * theta.cos();
            self.a2[k] = -r * r;
            self.g[k] = if live { (1.0 - r) * (1.0 + r * r - 2.0 * r * (2.0 * theta).cos()).max(1e-12).sqrt() * weight[k] } else { 0.0 };
        }
        self.release = (-1.0 / (0.03 * fs)).exp();
    }

    pub fn process(&mut self, x: f32) -> f32 {
        let mut out = 0.0;
        for k in 0..N {
            let y = self.g[k] * x + self.a1[k] * self.y1[k] + self.a2[k] * self.y2[k];
            self.y2[k] = self.y1[k];
            self.y1[k] = y;
            self.env[k] = y.abs().max(self.env[k] * self.release);
            out += y;
        }
        out
    }

    pub fn clear(&mut self) {
        self.y1 = [0.0; N];
        self.y2 = [0.0; N];
        self.env = [0.0; N];
    }
}

/// The shell: cylindrical-shell ring modes with material damping. `base_hz` is the lowest ring
/// mode, `ring` how long it sounds and `gamma` how much faster the higher ones die (high for
/// wood, low for metal).
pub fn tune_shell(bank: &mut Bank<SHELL_MODES>, fs: f32, base_hz: f32, ring: f32, gamma: f32) {
    let ratios = shell_ratios();
    let hz: [f32; SHELL_MODES] = std::array::from_fn(|k| base_hz * ratios[k]);
    let tau: [f32; SHELL_MODES] = std::array::from_fn(|k| ring * ratios[k].powf(-gamma));
    bank.tune(fs, &hz, &tau, &SHELL_WEIGHT);
}

/// The stick's bending modes, `hz` for the lowest and `tau` how long it rings.
pub fn tune_stick(bank: &mut Bank<STICK_MODES>, fs: f32, hz: f32, tau: f32) {
    let f: [f32; STICK_MODES] = std::array::from_fn(|k| hz * BEAM_RATIO[k]);
    let t: [f32; STICK_MODES] = std::array::from_fn(|k| tau / (1.0 + 0.4 * k as f32));
    bank.tune(fs, &f, &t, &STICK_WEIGHT);
}

/// A mallet in contact with the head. Positions are displacements in the head's own units, time
/// is in samples, and mass is relative to the whole head's.
#[derive(Default)]
pub struct Mallet {
    pub active: bool,
    pos: f32,
    vel: f32,
    /// Contact stiffness, mass ratio, and the hysteresis term.
    k: f32,
    mass: f32,
    lam: f32,
    age: u32,
    /// How many samples the force was above zero, for the tests.
    pub contact_samples: u32,
    /// Power lost in the contact on the last step: the Hunt-Crossley damping force times the speed
    /// of the compression. Collisions lose energy, and what is lost does not vanish: it goes into
    /// vibrations far above what the head's grid holds, which is the broadband whack of an impact.
    dissipation: f32,
}

/// Contact time of a Hertzian impact is `2.943 (m^2 / (k^2 v))^(1/5)`.
const HERTZ: f32 = 2.943;
/// Coefficient of restitution of a stick or beater on a head.
const RESTITUTION: f32 = 0.6;

/// How a mallet of hardness `hard` (0 soft to 1 hard) behaves when it hits at full velocity: the
/// contact time in seconds. A felt beater is around 8 ms, a hard stick tip under a millisecond.
pub fn contact_time(hard: f32) -> f32 {
    0.008 * (0.0007 / 0.008f32).powf(hard.clamp(0.0, 1.0))
}

/// The speed, in head units per sample, at which a mallet of reduced mass `reduced` must arrive to
/// set a head with fundamental `omega` (radians per sample) swinging at amplitude 1. It is the
/// loudness reference: a hit of velocity 1 is this fast.
pub fn full_speed(omega: f32, reduced: f32) -> f32 {
    // The momentum handed over is about (1 + e) times the mallet's, and a head swinging at
    // amplitude a carries a * omega * 0.2695 of it (0.2695 is the mean square of the mode shape).
    omega * 0.2695 / ((1.0 + RESTITUTION) * reduced)
}

impl Mallet {
    /// Throw the mallet at the head. `v_full` is the speed that sets the head swinging with
    /// amplitude 1; `mass` the mallet's mass relative to the head patch it lands on.
    pub fn start(&mut self, fs: f32, hard: f32, velocity: f32, v_full: f32, mass: f32, patch_mass: f32, start_pos: f32) {
        let reduced = mass * patch_mass / (mass + patch_mass);
        let t_steps = contact_time(hard) * fs;
        // The stiffness that gives the wanted contact time at full velocity.
        self.k = HERTZ.powf(2.5) * reduced / (t_steps.powf(2.5) * v_full.sqrt());
        let v0 = (velocity * v_full).max(1e-9);
        self.lam = 1.5 * (1.0 - RESTITUTION) / v0;
        self.mass = mass;
        self.pos = start_pos;
        self.vel = v0;
        self.age = 0;
        self.contact_samples = 0;
        self.active = true;
    }

    /// One step against a head that is at displacement `uc` moving at `vc` under the mallet.
    /// Returns the force on the head, in head masses per sample squared.
    pub fn step(&mut self, uc: f32, vc: f32) -> f32 {
        let x = self.pos - uc;
        let xd = self.vel - vc;
        let f = if x > 0.0 { (self.k * x * x.sqrt() * (1.0 + self.lam * xd)).max(0.0) } else { 0.0 };
        self.dissipation = if x > 0.0 { self.k * x * x.sqrt() * self.lam * xd * xd } else { 0.0 };
        if f > 0.0 {
            self.contact_samples += 1;
        }
        self.vel -= f / self.mass;
        self.pos += self.vel;
        self.age += 1;
        // Once it has left the head, or after long enough, the hand takes it away.
        if (x <= 0.0 && xd <= 0.0 && self.age > 2) || self.age > 6000 {
            self.active = false;
        }
        f
    }

    pub fn dissipation(&self) -> f32 {
        self.dissipation.max(0.0)
    }

    pub fn clear(&mut self) {
        self.active = false;
    }
}

/// The second head and the air between the heads: a few axisymmetric modes of a circular
/// membrane, and the spring of the sealed cavity that links them to the first head.
///
/// Only the axisymmetric modes (0, n) sweep any volume, so only they feel the cavity pressure.
pub struct Cavity {
    q: [f32; 4],
    v: [f32; 4],
    w2: [f32; 4],
    sigma: [f32; 4],
    kappa: f32,
}

/// Frequencies of the (0, n) modes relative to the fundamental: Bessel zeros of J0.
const RES_RATIO: [f32; 4] = [1.0, 2.295, 3.598, 4.903];
/// Mean over the disc of each mode shape, 2 J1(j) / j.
const RES_MEAN: [f32; 4] = [0.4318, -0.1233, 0.0627, -0.0394];
/// Mean of each shape squared, J1(j)^2: the modal mass.
const RES_MASS: [f32; 4] = [0.2695, 0.1158, 0.0737, 0.0541];
/// Bulk modulus of air, rho c^2 at room temperature, in Pa.
const AIR_STIFFNESS: f32 = 1.41e5;
/// The fraction of that spring that actually acts. A sealed, rigid drum would have all of it, and
/// then the breathing mode of a deep drum sits several times above the fundamental and swallows
/// it. Real drums have vent holes and leaks, and the shell and hoop flex, so most of the pressure
/// is relieved. This one number stands for all of that; it is a calibration, not a measurement, and
/// is set so the breathing mode of a typical drum sits 20 to 30 percent above its fundamental.
const AIR_ACTING: f32 = 0.02;

/// How hard the air between the heads pushes back, as an acceleration of the head per unit of
/// mean displacement, per sample squared. A head that sweeps a volume `A u` compresses the air in
/// a drum of volume `A L` by the fraction `u / L`, so the pressure is `rho c^2 u / L` and the
/// head, of areal mass `sigma`, is pushed with `rho c^2 / (sigma L)` per unit `u`. A shorter drum
/// is a stiffer spring.
pub fn cavity_stiffness(fs: f32, length: f32, head_areal_mass: f32) -> f32 {
    AIR_ACTING * AIR_STIFFNESS / (head_areal_mass * length.max(0.01)) / (fs * fs)
}

impl Default for Cavity {
    fn default() -> Self {
        Self { q: [0.0; 4], v: [0.0; 4], w2: [0.0; 4], sigma: [0.0; 4], kappa: 0.0 }
    }
}

impl Cavity {
    /// `omega` is the first head's fundamental in radians per sample, `ratio` the second head's
    /// pitch relative to it, `decay` how long the second head rings, and `stiffness` the air
    /// spring from [`cavity_stiffness`] (0 for no cavity).
    pub fn tune(&mut self, fs: f32, omega: f32, ratio: f32, decay: f32, stiffness: f32) {
        for k in 0..4 {
            let w = omega * ratio * RES_RATIO[k];
            self.w2[k] = w * w;
            self.sigma[k] = 1.0 / (decay.max(0.005) * fs * (1.0 + 0.3 * k as f32));
        }
        self.kappa = stiffness;
    }

    pub fn kappa(&self) -> f32 {
        self.kappa
    }

    /// Volume swept by this head: mean displacement.
    pub fn mean(&self) -> f32 {
        (0..4).map(|k| RES_MEAN[k] * self.q[k]).sum()
    }

    /// Velocity at the centre of the head, summed over the modes.
    pub fn velocity(&self) -> f32 {
        self.v.iter().sum()
    }

    /// Advance one sample under cavity pressure `p`.
    pub fn step(&mut self, p: f32) {
        for k in 0..4 {
            let a = -self.w2[k] * self.q[k] - 2.0 * self.sigma[k] * self.v[k] - p * RES_MEAN[k] / RES_MASS[k];
            self.v[k] += a;
            self.q[k] += self.v[k];
        }
    }

    pub fn energy(&self) -> f32 {
        self.v.iter().map(|v| v * v).sum::<f32>() + self.q.iter().zip(&self.w2).map(|(q, w2)| q * q * w2).sum::<f32>()
    }

    pub fn clear(&mut self) {
        self.q = [0.0; 4];
        self.v = [0.0; 4];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stick_is_a_beam_and_the_shell_a_cylinder() {
        // Free-free beam: 4.730^2, 7.853^2, 10.996^2 ... over the first.
        let beam = [4.730f32, 7.853, 10.996, 14.137].map(|b| b * b / (4.730 * 4.730));
        for (got, want) in BEAM_RATIO.iter().skip(1).zip(beam.iter().skip(1)) {
            assert!((got / want - 1.0).abs() < 0.02, "beam mode {got} vs {want}");
        }
        let shell = shell_ratios();
        assert_eq!(shell[0], 1.0);
        // n = 3 over n = 2: (3*8/sqrt(10)) / (2*3/sqrt(5)) = 2.83.
        assert!((shell[1] - 2.83).abs() < 0.01, "{}", shell[1]);
        assert!(shell.windows(2).all(|w| w[1] > w[0] * 1.2), "the ring modes spread out");
    }

    /// Run a mallet against a head that does not move, and see how long the contact lasts.
    fn contact_ms(hard: f32, velocity: f32) -> f32 {
        let fs = 44100.0;
        let mut m = Mallet::default();
        // Against a wall that does not give, so the whole mallet mass is what bounces.
        let v_full = TAU * 100.0 / fs * 0.2695 / 1.7 / 0.1;
        m.start(fs, hard, velocity, v_full, 0.1, 1e6, 0.0);
        let mut n = 0;
        while m.active && n < 20000 {
            m.step(0.0, 0.0);
            n += 1;
        }
        m.contact_samples as f32 / fs * 1000.0
    }

    #[test]
    fn the_contact_time_is_set_by_hardness_and_lands_where_sticks_do() {
        let (soft, medium, hard) = (contact_ms(0.0, 1.0), contact_ms(0.5, 1.0), contact_ms(1.0, 1.0));
        // A felt beater is several milliseconds, a stick on a tom 3 to 8, a hard tip under 1 to 2.
        assert!((5.0..11.0).contains(&soft), "soft {soft} ms");
        assert!((1.0..4.0).contains(&medium), "medium {medium} ms");
        assert!((0.3..1.5).contains(&hard), "hard {hard} ms");
        assert!(soft > medium && medium > hard);
    }

    #[test]
    fn a_harder_blow_is_shorter_by_the_hertz_law() {
        // The contact time goes as velocity^(-1/5).
        let slow = contact_ms(0.5, 0.25);
        let fast = contact_ms(0.5, 1.0);
        let expect = 4.0f32.powf(0.2);
        assert!((slow / fast / expect - 1.0).abs() < 0.2, "ratio {} vs {expect}", slow / fast);
    }

    #[test]
    fn the_mallet_bounces_off_and_loses_energy() {
        let fs = 44100.0;
        let mut m = Mallet::default();
        m.start(fs, 0.5, 1.0, 0.003, 0.1, 0.1, 0.0);
        let v0 = m.vel;
        while m.active {
            m.step(0.0, 0.0);
        }
        // It leaves going the other way, slower than it came: restitution below 1.
        let out = -m.vel;
        assert!(out > 0.2 * v0 && out < 0.95 * v0, "in {v0} out {out}");
    }

    /// A head tied to a second by the air between them, struck, and the first head's spectrum.
    fn coupled_spectrum(length: f32) -> Vec<f32> {
        use crate::membrane::{tests::FS, Membrane};
        // The spectrum helpers analyse at this rate.
        let fs = FS;
        let mut m = Membrane::default();
        m.tune(fs, 100.0, 3.0, 0.0, 0.0);
        let mut c = Cavity::default();
        c.tune(fs, m.omega(), 1.0, 3.0, cavity_stiffness(fs, length, 0.2));
        m.strike(0.0, 0.0, 0.0, 1.0);
        let mut x = Vec::new();
        for _ in 0..(1.0 * fs) as usize {
            let p = c.kappa() * (m.mean() + c.mean());
            m.drive(0.0, -p);
            c.step(p);
            m.step();
            let (a, b) = m.pickups();
            x.push(a + b);
        }
        x
    }

    #[test]
    fn the_air_between_the_heads_splits_the_fundamental_by_the_formula() {
        use crate::membrane::tests::peak_hz;
        // The heads moving together leave the volume alone and stay at 100 Hz; the pair that
        // breathes compresses the air and is raised by sqrt(1 + 2 Omega^2 / omega^2), where
        // Omega^2 = kappa g^2 / M for the (0, 1) mode.
        let length = 0.15;
        let x = coupled_spectrum(length);
        let omega = std::f32::consts::TAU * 100.0 / crate::membrane::tests::FS;
        let kappa = cavity_stiffness(crate::membrane::tests::FS, length, 0.2);
        let big_omega2 = kappa * RES_MEAN[0] * RES_MEAN[0] / RES_MASS[0];
        let expect = 100.0 * (1.0 + 2.0 * big_omega2 / (omega * omega)).sqrt();
        let (lower, upper) = (peak_hz(&x, 70.0, 111.0), peak_hz(&x, 112.0, 200.0));
        assert!((lower - 100.0).abs() < 6.0, "heads together at {lower}");
        assert!((upper / expect - 1.0).abs() < 0.08, "heads breathing at {upper}, expected {expect}");
    }

    #[test]
    fn a_shorter_drum_has_a_stiffer_air_spring_and_a_higher_breathing_mode() {
        use crate::membrane::tests::peak_hz;
        let upper = |length: f32| peak_hz(&coupled_spectrum(length), 104.0, 200.0);
        assert!(upper(0.08) > upper(0.4) * 1.1, "{} vs {}", upper(0.08), upper(0.4));
    }
}
