//! The shell: a thin cylinder holding the heads, with the properties of what it is made of.
//!
//! Its vibration modes have `n` waves around the circumference and `m` half-waves along its
//! length. For a thin circular cylindrical shell of radius `a`, length `L`, thickness `h`, Young's
//! modulus `E`, density `rho` and Poisson's ratio `nu`, the Donnell-Mushtari equations give, with
//! `lambda = beta_m a / L` (`beta_m` the free-free beam wavenumbers 0, 4.730, 7.853 ... since a
//! drum shell has free ends),
//!
//! ```text
//! omega^2 = E / (rho (1 - nu^2) a^2) * [ (1 - nu^2) lambda^4 / (n^2 + lambda^2)^2
//!                                        + h^2 / (12 a^2) * (n^2 + lambda^2)^2 ]
//! ```
//!
//! the first term being the stretching of the shell and the second its bending. Nothing in it is a
//! typed-in ratio: a longer shell, a thicker one, a wider one or a stiffer material moves every
//! mode. Damping is the material's loss factor `eta`: a mode at `omega` loses amplitude at
//! `eta omega / 2` a second, so wood (`eta` about 0.015) dies quickly and steel (about 0.001) rings.
//!
//! The shell is coupled to the head both ways, through the rim. Each rim cell of the head is tied
//! by a spring to the shell's displacement at that angle, so the head's motion drives the shell
//! and the shell's motion pushes back on the head: the potential is
//! `V = 1/2 k sum_i (u_i - w_i)^2` and both forces follow from it.

use crate::membrane::{NN, RIM_MAX};
use std::f32::consts::TAU;

/// Circumferential wave numbers n = 2 to 6, and axial orders m = 0, 1, 2.
pub const NS: usize = 5;
pub const MS: usize = 3;
pub const SHELL_MODES: usize = NS * MS;
const BETA: [f32; MS] = [0.0, 4.730, 7.853];
/// The mass per area of a drumhead, kg per square metre: a 7 mil film.
pub const HEAD_AREAL_MASS: f32 = 0.2;

/// What a shell is made of.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Material {
    /// Young's modulus, Pa; density, kg per cubic metre; Poisson's ratio; loss factor.
    pub e: f32,
    pub rho: f32,
    pub nu: f32,
    pub loss: f32,
}

/// Plywood, as in a maple drum shell.
pub const WOOD: Material = Material { e: 10.0e9, rho: 700.0, nu: 0.3, loss: 0.015 };
/// Steel, as in a snare drum.
pub const STEEL: Material = Material { e: 200.0e9, rho: 7800.0, nu: 0.3, loss: 0.001 };
/// Fibreglass or acrylic.
pub const COMPOSITE: Material = Material { e: 20.0e9, rho: 1800.0, nu: 0.3, loss: 0.006 };

/// The size of a shell, in metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    pub radius: f32,
    pub length: f32,
    pub thickness: f32,
}

/// Natural frequencies in Hz of the (n, m) modes, ordered by n then m.
pub fn frequencies(g: Geometry, mat: Material) -> [f32; SHELL_MODES] {
    let speed2 = mat.e / (mat.rho * (1.0 - mat.nu * mat.nu));
    let bend = g.thickness * g.thickness / (12.0 * g.radius * g.radius);
    std::array::from_fn(|k| {
        let (n, m) = ((k / MS + 2) as f32, k % MS);
        let lambda = BETA[m] * g.radius / g.length;
        let d = n * n + lambda * lambda;
        let omega2 = speed2 / (g.radius * g.radius) * ((1.0 - mat.nu * mat.nu) * lambda.powi(4) / (d * d) + bend * d * d);
        omega2.max(0.0).sqrt() / TAU
    })
}

pub struct Shell {
    /// Displacement and velocity of each mode's cosine and sine halves, in the head's units.
    q: [[f32; 2]; SHELL_MODES],
    v: [[f32; 2]; SHELL_MODES],
    w2: [f32; SHELL_MODES],
    sigma: [f32; SHELL_MODES],
    /// Modal mass relative to the head's, and the spring tying the rim to the shell.
    mass: f32,
    kappa: f32,
    cells: f32,
    cos: [[f32; NS]; RIM_MAX],
    sin: [[f32; NS]; RIM_MAX],
    rim: [usize; RIM_MAX],
    len: usize,
    active: bool,
}

impl Shell {
    /// `rim` and `theta` are the head's rim cells and their angles.
    pub fn new(rim: &[usize], theta: &[f32], cells: f32) -> Self {
        let mut s = Shell {
            q: [[0.0; 2]; SHELL_MODES],
            v: [[0.0; 2]; SHELL_MODES],
            w2: [0.0; SHELL_MODES],
            sigma: [0.0; SHELL_MODES],
            mass: 10.0,
            kappa: 0.0,
            cells,
            cos: [[0.0; NS]; RIM_MAX],
            sin: [[0.0; NS]; RIM_MAX],
            rim: [0; RIM_MAX],
            len: rim.len().min(RIM_MAX),
            active: false,
        };
        for j in 0..s.len {
            s.rim[j] = rim[j];
            for n in 0..NS {
                let a = (n + 2) as f32 * theta[j];
                s.cos[j][n] = a.cos();
                s.sin[j][n] = a.sin();
            }
        }
        s
    }

    /// `omega` is the head's fundamental in radians per sample; `rim_stiffness` how many times the
    /// head's own stiffness the rim is tied to the shell with.
    pub fn tune(&mut self, fs: f32, g: Geometry, mat: Material, omega: f32, rim_stiffness: f32) {
        let f = frequencies(g, mat);
        for k in 0..SHELL_MODES {
            let w = TAU * f[k] / fs;
            // Modes too high to integrate stably are left out.
            self.w2[k] = if w < 0.9 { w * w } else { 0.0 };
            self.sigma[k] = if w < 0.9 { 0.5 * mat.loss * w } else { 1.0 };
        }
        // The shell weighs about this many heads, and each mode takes half of a share of it.
        let shell_mass = mat.rho * TAU * g.radius * g.length * g.thickness;
        let head_mass = HEAD_AREAL_MASS * std::f32::consts::PI * g.radius * g.radius;
        self.mass = (shell_mass / head_mass / 2.0).max(0.05);
        self.kappa = rim_stiffness * omega * omega;
    }

    pub fn active(&self) -> bool {
        self.active
    }

    /// No shell: nothing holds the rim.
    pub fn disable(&mut self) {
        self.kappa = 0.0;
        self.clear();
    }

    pub fn kappa(&self) -> f32 {
        self.kappa
    }

    pub fn energy(&self) -> f32 {
        let mut e = 0.0;
        for k in 0..SHELL_MODES {
            for c in 0..2 {
                e += self.v[k][c] * self.v[k][c] + self.w2[k] * self.q[k][c] * self.q[k][c];
            }
        }
        e
    }

    pub fn clear(&mut self) {
        self.q = [[0.0; 2]; SHELL_MODES];
        self.v = [[0.0; 2]; SHELL_MODES];
        self.active = false;
    }

    /// The amplitude of each mode, for the display.
    pub fn amplitude(&self, k: usize) -> f32 {
        (self.v[k][0] * self.v[k][0] + self.v[k][1] * self.v[k][1]).sqrt()
    }

    /// Advance one sample. `u` is the head's displacement; the force on each rim cell is written to
    /// `edge`. Returns the shell's velocity at the point where the microphone is (angle 0).
    pub fn step(&mut self, u: &[f32; NN], edge: &mut [f32; NN]) -> f32 {
        if !self.active && self.kappa > 0.0 {
            // Woken by the head moving at the rim.
            let mut sum = 0.0;
            for j in 0..self.len {
                sum += u[self.rim[j]].abs();
            }
            self.active = sum > 1e-7;
        }
        if !self.active {
            return 0.0;
        }
        // The shell's displacement around the rim, from its modes.
        let mut a = [0.0f32; NS];
        let mut b = [0.0f32; NS];
        for k in 0..SHELL_MODES {
            a[k / MS] += self.q[k][0];
            b[k / MS] += self.q[k][1];
        }
        let mut gc = [0.0f32; NS];
        let mut gs = [0.0f32; NS];
        for j in 0..self.len {
            let mut w = 0.0;
            for n in 0..NS {
                w += a[n] * self.cos[j][n] + b[n] * self.sin[j][n];
            }
            let gap = u[self.rim[j]] - w;
            edge[self.rim[j]] = self.kappa * w;
            for n in 0..NS {
                gc[n] += gap * self.cos[j][n];
                gs[n] += gap * self.sin[j][n];
            }
        }
        let drive = self.kappa / (self.cells * self.mass);
        let mut out = 0.0;
        for k in 0..SHELL_MODES {
            let n = k / MS;
            let (fc, fs_) = (drive * gc[n], drive * gs[n]);
            self.v[k][0] += -self.w2[k] * self.q[k][0] - 2.0 * self.sigma[k] * self.v[k][0] + fc;
            self.v[k][1] += -self.w2[k] * self.q[k][1] - 2.0 * self.sigma[k] * self.v[k][1] + fs_;
            self.q[k][0] += self.v[k][0];
            self.q[k][1] += self.v[k][1];
            out += self.v[k][0];
        }
        if self.energy() < 1e-22 && u.iter().all(|x| x.abs() < 1e-9) {
            self.clear();
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drum_shell() -> Geometry {
        Geometry { radius: 0.18, length: 0.14, thickness: 0.007 }
    }

    #[test]
    fn the_modes_scale_the_way_shell_theory_says() {
        let base = frequencies(drum_shell(), WOOD);
        // Bending dominates the low circumferential modes at this size: they go as the thickness
        // and as the square root of the stiffness over the density.
        let thick = frequencies(Geometry { thickness: 0.014, ..drum_shell() }, WOOD);
        assert!((thick[0] / base[0] - 2.0).abs() < 0.15, "twice as thick: {}", thick[0] / base[0]);
        let stiff = frequencies(drum_shell(), Material { e: 40.0e9, ..WOOD });
        assert!((stiff[0] / base[0] - 2.0).abs() < 0.1, "four times stiffer: {}", stiff[0] / base[0]);
        let heavy = frequencies(drum_shell(), Material { rho: 2800.0, ..WOOD });
        assert!((heavy[0] / base[0] - 0.5).abs() < 0.05, "four times denser: {}", heavy[0] / base[0]);
        // And as one over the radius squared.
        let wide = frequencies(Geometry { radius: 0.36, ..drum_shell() }, WOOD);
        assert!(wide[0] < base[0] * 0.4, "twice as wide: {}", wide[0] / base[0]);
        // More waves round the circumference means a higher mode.
        assert!(base.iter().step_by(MS).zip(base.iter().step_by(MS).skip(1)).all(|(a, b)| b > a));
    }

    #[test]
    fn a_longer_shell_has_lower_axial_modes_and_steel_is_higher_than_wood() {
        // With axial order 1 or 2 the length matters; with order 0 (a ring) it does not.
        let short = frequencies(Geometry { length: 0.10, ..drum_shell() }, WOOD);
        let long = frequencies(Geometry { length: 0.40, ..drum_shell() }, WOOD);
        assert!((short[0] - long[0]).abs() < 1.0, "the ring does not care about the length");
        assert!(long[1] < short[1], "a long shell's axial mode is lower: {} vs {}", long[1], short[1]);
        // Steel is stiffer and denser: its speed of sound is higher, so the modes are too.
        assert!(frequencies(Geometry { thickness: 0.001, ..drum_shell() }, STEEL)[0] > frequencies(Geometry { thickness: 0.001, ..drum_shell() }, WOOD)[0]);
    }

    /// Strike a head tied to a shell and see how the shell takes up the energy.
    fn run(mat: Material, seconds: f32) -> (Vec<f32>, Vec<f32>) {
        run_with(mat, seconds, 1.5)
    }

    fn run_with(mat: Material, seconds: f32, head_decay: f32) -> (Vec<f32>, Vec<f32>) {
        use crate::membrane::Membrane;
        let fs = 44100.0;
        let mut m = Membrane::default();
        m.tune(fs, 120.0, head_decay, 0.0, 0.0);
        let (rim, theta) = m.rim();
        let (rim, theta) = (rim.to_vec(), theta.to_vec());
        let mut shell = Shell::new(&rim, &theta, m.cells());
        shell.tune(fs, drum_shell(), mat, m.omega(), 1.0);
        m.set_edge_stiffness(shell.kappa());
        m.strike(0.5, 0.2, 0.9, 1.0);
        let (mut head, mut sh) = (Vec::new(), Vec::new());
        for _ in 0..(seconds * fs) as usize {
            let mut edge = [0.0f32; NN];
            let out = shell.step(m.surface(), &mut edge);
            *m.edge_mut() = edge;
            m.step();
            let (a, b) = m.pickups();
            head.push(a + b);
            sh.push(out);
        }
        (head, sh)
    }

    #[test]
    fn the_head_drives_the_shell_and_the_shell_rings_by_its_material() {
        let rms = |x: &[f32]| (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt();
        // A head that is damped quickly, so what is left late is the shell ringing on its own.
        let (_, wood) = run_with(WOOD, 1.0, 0.03);
        let (_, steel) = run_with(Material { loss: STEEL.loss, ..WOOD }, 1.0, 0.03);
        assert!(wood.iter().all(|v| v.is_finite()));
        // It is driven by the head...
        assert!(rms(&wood[..4000]) > 0.0, "the shell is silent");
        // ...and a low-loss shell keeps ringing after a lossy one has gone.
        let late = |x: &[f32]| rms(&x[(0.4 * 44100.0) as usize..]) / rms(&x[..8000]).max(1e-12);
        assert!(late(&steel) > late(&wood) * 3.0, "low loss {} lossy {}", late(&steel), late(&wood));
    }

    #[test]
    fn the_shell_pushes_back_on_the_head() {
        // The head's energy leaks into the shell, so a head tied to one dies sooner than a head on
        // a rigid rim.
        let rms = |x: &[f32]| (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt();
        let (tied, _) = run(WOOD, 1.0);
        let free = {
            use crate::membrane::Membrane;
            let mut m = Membrane::default();
            m.tune(44100.0, 120.0, 1.5, 0.0, 0.0);
            m.strike(0.5, 0.2, 0.9, 1.0);
            (0..44100).map(|_| { m.step(); let (a, b) = m.pickups(); a + b }).collect::<Vec<f32>>()
        };
        let ratio = |x: &[f32]| rms(&x[(0.5 * 44100.0) as usize..]) / rms(&x[..8000]);
        assert!(ratio(&tied) < ratio(&free), "tied {} free {}", ratio(&tied), ratio(&free));
    }
}
