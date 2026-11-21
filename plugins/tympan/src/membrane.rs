//! A drumhead: the 2D wave equation on a square grid, clamped at a circular rim.
//!
//! ```text
//! u_tt = c^2 (u_xx + u_yy) - 2 s0 u_t + 2 s1 (u_xx + u_yy)_t
//! ```
//!
//! solved by finite differences, one step per audio sample. `s0` damps every mode alike and `s1`
//! damps the high ones faster, which is why a real head loses its overtones first. Tension
//! modulation (a head stiffens as it swings further) raises the wave speed with the energy in the
//! skin, so a hard hit starts sharp and settles: the pitch drop of a kick.
//!
//! The grid is only 25 x 25, so it is cheap enough to run for four heads, and it is also what the
//! editor draws.

/// Cells along one side of the grid.
pub const N: usize = 25;
pub const NN: usize = N * N;
const CENTRE: f32 = 12.0;
/// Radius of the rim in cells. Cells further out than this are clamped to zero.
pub const RADIUS: f32 = 11.5;
/// First zero of the Bessel function J0: the fundamental of a circular membrane is this times c / (2 pi a).
const J01: f32 = 2.404_826;
/// Mean of the fundamental's mode shape squared over the disc, J1(j01)^2.
const MODE_SQUARE: f32 = 0.2695;
/// Where the "microphones" sit, in cells: off-centre, and not on a nodal ring of the low modes.
const PICKUPS: [(usize, usize); 2] = [(9, 14), (17, 8)];
/// How much of what is heard is the volume velocity, as against the skin at a point.
const MONOPOLE: f32 = 0.7;
/// The stair-stepped rim is a touch larger than its radius says; this brings the pitch back in line.
const PITCH_TRIM: f32 = 1.0;

pub struct Membrane {
    bufs: [[f32; NN]; 3],
    phase: usize,
    mask: [f32; NN],
    cells: f32,
    r_eff: f32,
    omega: f32,
    lambda2: f32,
    d0: f32,
    b: f32,
    bend: f32,
    energy: f32,
    quiet: u32,
    active: bool,
    /// What the editor draws: how far each cell is swinging, and how fast, as slowly-falling peaks.
    /// The ratio of the two says whether a cell is moving at the fundamental or at an overtone.
    env_disp: [f32; NN],
    env_vel: [f32; NN],
    tick: u32,
    env_release: f32,
    /// A mallet pressing on the head: its footprint (weights summing to 1) and the force on it,
    /// as an acceleration per unit of footprint weight.
    footprint: [f32; NN],
    force: f32,
    /// A pressure acting on every cell alike: the air inside the drum.
    uniform: f32,
    /// Mean displacement over the head after the last step: the volume the head has swept.
    mean: f32,
    /// The mean displacement one step earlier, for the volume velocity.
    prev_mean: f32,
    /// The cells at the rim, which the shell holds: where they are, and an elastic coupling to it.
    rim: [f32; NN],
    rim_cells: [usize; RIM_MAX],
    rim_theta: [f32; RIM_MAX],
    rim_len: usize,
    /// Force from the shell on each rim cell, and the spring that ties the rim to it.
    edge: [f32; NN],
    edge_k: f32,
}

/// Most cells there can be on the rim of the grid.
pub const RIM_MAX: usize = 96;

/// Position of a cell in the unit disc, for drawing and for strike points.
pub fn cell_pos(index: usize) -> (f32, f32) {
    (((index % N) as f32 - CENTRE) / RADIUS, ((index / N) as f32 - CENTRE) / RADIUS)
}

pub fn in_head(index: usize) -> bool {
    let (x, y) = cell_pos(index);
    x * x + y * y <= 1.0
}

impl Default for Membrane {
    fn default() -> Self {
        let mut mask = [0.0; NN];
        for (i, m) in mask.iter_mut().enumerate() {
            *m = in_head(i) as u8 as f32;
        }
        let cells: f32 = mask.iter().sum();
        let mut m = Self {
            bufs: [[0.0; NN]; 3],
            phase: 0,
            mask,
            cells,
            r_eff: (cells / std::f32::consts::PI).sqrt(),
            omega: 0.01,
            lambda2: 0.01,
            d0: 1e-4,
            b: 0.0,
            bend: 0.0,
            energy: 0.0,
            quiet: 0,
            active: false,
            env_disp: [0.0; NN],
            env_vel: [0.0; NN],
            tick: 0,
            env_release: 0.99,
            footprint: [0.0; NN],
            force: 0.0,
            uniform: 0.0,
            mean: 0.0,
            prev_mean: 0.0,
            rim: [0.0; NN],
            rim_cells: [0; RIM_MAX],
            rim_theta: [0.0; RIM_MAX],
            rim_len: 0,
            edge: [0.0; NN],
            edge_k: 0.0,
        };
        // A rim cell is one inside the head with a neighbour outside it.
        for i in 0..NN {
            let inside = |j: usize| m.mask[j] > 0.0;
            if m.mask[i] > 0.0 && !(inside(i - 1) && inside(i + 1) && inside(i - N) && inside(i + N)) && m.rim_len < RIM_MAX {
                m.rim[i] = 1.0;
                m.rim_cells[m.rim_len] = i;
                let (x, y) = cell_pos(i);
                m.rim_theta[m.rim_len] = y.atan2(x);
                m.rim_len += 1;
            }
        }
        m.tune(48000.0, 100.0, 0.5, 0.1, 0.0);
        m
    }
}

impl Membrane {
    /// Set the head's resting pitch in Hz, its decay time (the fundamental's amplitude falls by
    /// 1/e in this many seconds), how much faster the overtones die (`tilt`, 0 to 1), and the
    /// strength of the tension modulation (`bend`, 0 to 1).
    pub fn tune(&mut self, fs: f32, hz: f32, decay: f32, tilt: f32, bend: f32) {
        self.env_release = (-4.0 / (0.06 * fs)).exp();
        self.omega = std::f32::consts::TAU * hz.clamp(10.0, fs * 0.2) / fs;
        let lambda = self.omega * self.r_eff / J01 * PITCH_TRIM;
        self.lambda2 = (lambda * lambda).min(0.4);
        self.d0 = 1.0 / (decay.max(0.005) * fs);
        let scale = self.r_eff / J01;
        self.b = (self.d0 * tilt * scale * scale).min(0.02);
        self.bend = bend;
    }

    pub fn active(&self) -> bool {
        self.active
    }

    /// The rim cells and their angles around the head.
    pub fn rim(&self) -> (&[usize], &[f32]) {
        (&self.rim_cells[..self.rim_len], &self.rim_theta[..self.rim_len])
    }

    /// The force the shell puts on each rim cell this sample, and the stiffness of the spring that
    /// ties the rim to it.
    pub fn edge_mut(&mut self) -> &mut [f32; NN] {
        &mut self.edge
    }

    /// Hand the head's displacement and the edge forces to `f` together, so a shell can read the
    /// one and write the other.
    pub fn with_edge<R>(&mut self, f: impl FnOnce(&[f32; NN], &mut [f32; NN]) -> R) -> R {
        let cur = (3 - self.phase) % 3;
        f(&self.bufs[cur], &mut self.edge)
    }

    pub fn set_edge_stiffness(&mut self, k: f32) {
        self.edge_k = k;
    }

    /// Wake the head so it is simulated, as when a mallet is pressing on it.
    pub fn wake(&mut self) {
        self.active = true;
        self.quiet = 0;
    }

    /// The fundamental, in radians per sample.
    pub fn omega(&self) -> f32 {
        self.omega
    }

    pub fn cells(&self) -> f32 {
        self.cells
    }

    pub fn mean(&self) -> f32 {
        self.mean
    }

    /// Spread a mallet's contact over a raised-cosine patch centred at `(x, y)` in the unit disc,
    /// `reach` cells wide. Returns the sum of the squared weights, which says how much of the
    /// head's mass the patch represents.
    pub fn set_footprint(&mut self, x: f32, y: f32, reach: f32) -> f32 {
        let r = (x * x + y * y).sqrt();
        let k = if r > 0.95 { 0.95 / r } else { 1.0 };
        let (px, py) = (CENTRE + x * k * RADIUS, CENTRE + y * k * RADIUS);
        let mut total = 0.0;
        for i in 0..NN {
            let (dx, dy) = ((i % N) as f32 - px, (i / N) as f32 - py);
            let d = (dx * dx + dy * dy).sqrt();
            self.footprint[i] = if d < reach && self.mask[i] > 0.0 { 0.5 + 0.5 * (std::f32::consts::PI * d / reach).cos() } else { 0.0 };
            total += self.footprint[i];
        }
        let mut squares = 0.0;
        for w in self.footprint.iter_mut() {
            *w /= total.max(1e-9);
            squares += *w * *w;
        }
        squares
    }

    /// Displacement and velocity (per sample) of the head under the mallet.
    pub fn probe(&self) -> (f32, f32) {
        let (cur, old) = (&self.bufs[self.newest()], &self.bufs[(self.newest() + 1) % 3]);
        let (mut u, mut v) = (0.0, 0.0);
        for i in 0..NN {
            let w = self.footprint[i];
            if w > 0.0 {
                u += w * cur[i];
                v += w * (cur[i] - old[i]);
            }
        }
        (u, v)
    }

    /// Push on the footprint (`force`) and on the whole head (`uniform`) for the next step.
    pub fn drive(&mut self, force: f32, uniform: f32) {
        self.force = force;
        self.uniform = uniform;
    }

    /// Energy in the skin, 1 for a centre hit at full velocity, falling as it rings out.
    pub fn energy(&self) -> f32 {
        self.energy
    }

    /// Buffer holding the newest state. Each step writes the new state over the oldest buffer, so
    /// the newest walks 0, 2, 1, 0, ... as `phase` counts up.
    fn newest(&self) -> usize {
        (3 - self.phase) % 3
    }

    /// Peak displacement and peak velocity (in units of the fundamental) of each cell.
    pub fn envelopes(&self) -> (&[f32; NN], &[f32; NN]) {
        (&self.env_disp, &self.env_vel)
    }

    /// The grid as it stands now.
    pub fn surface(&self) -> &[f32; NN] {
        &self.bufs[self.newest()]
    }

    /// Hit the head at `(x, y)` in the unit disc. A hard mallet touches a small patch and so
    /// excites the overtones; a soft one spreads out and mostly sets the fundamental going.
    pub fn strike(&mut self, x: f32, y: f32, hard: f32, velocity: f32) {
        // Keep the strike inside the rim.
        let r = (x * x + y * y).sqrt();
        let k = if r > 0.95 { 0.95 / r } else { 1.0 };
        let (px, py) = (CENTRE + x * k * RADIUS, CENTRE + y * k * RADIUS);
        // Hit harder and the contact gets harder too.
        let hard = (hard + 0.3 * (velocity - 0.5)).clamp(0.0, 1.0);
        let reach = 6.0 - 4.7 * hard;
        let mut weights = [0.0f32; NN];
        let mut total = 0.0;
        for (i, w) in weights.iter_mut().enumerate() {
            let (dx, dy) = ((i % N) as f32 - px, (i / N) as f32 - py);
            let d = (dx * dx + dy * dy).sqrt();
            if d < reach && self.mask[i] > 0.0 {
                *w = 0.5 + 0.5 * (std::f32::consts::PI * d / reach).cos();
                total += *w;
            }
        }
        if total <= 0.0 {
            return;
        }
        // An impulse that sets the fundamental swinging at amplitude `velocity`.
        let impulse = velocity * self.omega * self.cells * MODE_SQUARE / total;
        let old = (self.newest() + 1) % 3;
        for (i, w) in weights.iter().enumerate() {
            self.bufs[old][i] -= impulse * w;
        }
        self.active = true;
        self.quiet = 0;
    }

    /// One step of the simulation.
    pub fn step(&mut self) {
        if !self.active {
            return;
        }
        let [b0, b1, b2] = &mut self.bufs;
        let (cur, old, new): (&[f32; NN], &[f32; NN], &mut [f32; NN]) = match self.phase {
            0 => (&*b0, &*b1, b2),
            1 => (&*b2, &*b0, b1),
            _ => (&*b1, &*b2, b0),
        };
        let lam2 = (self.lambda2 * (1.0 + 3.0 * self.bend * self.energy)).min(0.4);
        let (d0, b) = (self.d0, self.b);
        let a = 1.0 / (1.0 + d0);
        let (kc, ko) = (lam2 + 2.0 * b, 2.0 * b);
        let keep = 1.0 - d0;
        let (force, uniform, edge_k) = (self.force, self.uniform, self.edge_k);
        let mut sum = 0.0f32;
        let mut total = 0.0f32;
        for y in 1..N - 1 {
            let row = y * N;
            for x in 1..N - 1 {
                let i = row + x;
                let u = cur[i];
                let o = old[i];
                let lu = cur[i - 1] + cur[i + 1] + cur[i - N] + cur[i + N] - 4.0 * u;
                let lo = old[i - 1] + old[i + 1] + old[i - N] + old[i + N] - 4.0 * o;
                let v = a * (2.0 * u - keep * o + kc * lu - ko * lo + force * self.footprint[i] + uniform + self.edge[i] - edge_k * self.rim[i] * u) * self.mask[i];
                new[i] = v;
                sum += v * v;
                total += v;
            }
        }
        // Every fourth step is plenty to catch the peaks, and keeps the drawing cheap.
        self.tick = self.tick.wrapping_add(1);
        if self.tick & 3 == 0 {
            // Measure against the pitch the head is ringing at now, bend included.
            let inv = 1.0 / (self.omega * (lam2 / self.lambda2).sqrt());
            for i in 0..NN {
                self.env_disp[i] = new[i].abs().max(self.env_disp[i] * self.env_release);
                self.env_vel[i] = ((new[i] - cur[i]).abs() * inv).max(self.env_vel[i] * self.env_release);
            }
        }
        self.phase = (self.phase + 1) % 3;
        self.energy = sum / (self.cells * MODE_SQUARE);
        self.prev_mean = self.mean;
        self.mean = total / self.cells;
        // Ringing out: stop spending cycles on a head nobody can hear.
        if sum < 1e-9 {
            self.quiet += 1;
            if self.quiet > 4096 {
                self.bufs = [[0.0; NN]; 3];
                self.env_disp = [0.0; NN];
                self.env_vel = [0.0; NN];
                self.energy = 0.0;
                self.mean = 0.0;
                self.prev_mean = 0.0;
                self.active = false;
            }
        } else {
            self.quiet = 0;
        }
    }

    /// What the two microphones hear, scaled so the loudness does not depend on the pitch.
    ///
    /// A head radiates mostly by the volume it sweeps, which is the mean of its velocity: only the
    /// symmetric modes do that, while a mode with a node line across it (the (1, 1) mode, say)
    /// moves one side up as the other goes down and radiates poorly. So most of what is heard is
    /// the volume velocity, and the rest is the skin's velocity at each microphone's point, which
    /// is what gives the two their different views of the other modes.
    pub fn pickups(&self) -> (f32, f32) {
        if !self.active {
            return (0.0, 0.0);
        }
        let cur = &self.bufs[self.newest()];
        let old = &self.bufs[(self.newest() + 1) % 3];
        let read = |(x, y): (usize, usize)| (cur[y * N + x] - old[y * N + x]) / self.omega;
        // The mean displacement of the fundamental is 0.4318 of its peak.
        let volume = (self.mean - self.prev_mean) / self.omega / 0.4318;
        let (a, b) = (read(PICKUPS[0]), read(PICKUPS[1]));
        (MONOPOLE * volume + (1.0 - MONOPOLE) * a, MONOPOLE * volume + (1.0 - MONOPOLE) * b)
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub const FS: f32 = 48000.0;

    /// Strike a head and render `seconds` of the two pickups summed.
    pub fn render(m: &mut Membrane, seconds: f32) -> Vec<f32> {
        (0..(seconds * FS) as usize)
            .map(|_| {
                m.step();
                let (a, b) = m.pickups();
                a + b
            })
            .collect()
    }

    /// Windowed single-frequency magnitude.
    pub fn tone_at(x: &[f32], hz: f32) -> f32 {
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (n, v) in x.iter().enumerate() {
            let w = 0.5 - 0.5 * (std::f64::consts::TAU * n as f64 / x.len() as f64).cos();
            let ph = std::f64::consts::TAU * hz as f64 * n as f64 / FS as f64;
            re += *v as f64 * w * ph.cos();
            im += *v as f64 * w * ph.sin();
        }
        ((re * re + im * im).sqrt() / x.len() as f64) as f32
    }

    /// The loudest frequency between `lo` and `hi`.
    pub fn peak_hz(x: &[f32], lo: f32, hi: f32) -> f32 {
        let mut best = (0.0, lo);
        let mut f = lo;
        while f < hi {
            let m = tone_at(x, f);
            if m > best.0 {
                best = (m, f);
            }
            f += 0.5;
        }
        best.1
    }

    fn head(hz: f32, decay: f32, tilt: f32, bend: f32) -> Membrane {
        let mut m = Membrane::default();
        m.tune(FS, hz, decay, tilt, bend);
        m
    }

    #[test]
    fn an_unstruck_head_is_silent_and_costs_nothing() {
        let mut m = head(100.0, 0.5, 0.1, 0.0);
        assert!(render(&mut m, 0.1).iter().all(|v| *v == 0.0));
        assert!(!m.active());
    }

    #[test]
    fn a_centre_hit_rings_at_the_tuned_pitch() {
        for hz in [60.0, 110.0, 200.0, 400.0] {
            let mut m = head(hz, 2.0, 0.0, 0.0);
            m.strike(0.0, 0.0, 0.0, 1.0);
            let x = render(&mut m, 1.0);
            let f = peak_hz(&x, hz * 0.6, hz * 1.6);
            assert!((f - hz).abs() / hz < 0.04, "tuned to {hz} Hz but rings at {f} Hz");
        }
    }

    #[test]
    fn the_overtones_are_the_bessel_zeros_not_harmonics() {
        // Centre strike: only the rotationally symmetric modes (0,n) sound, the second at 2.295x.
        let mut m = head(100.0, 2.0, 0.0, 0.0);
        m.strike(0.0, 0.0, 1.0, 1.0);
        let x = render(&mut m, 1.0);
        let second = peak_hz(&x, 200.0, 260.0);
        assert!((second / 100.0 - 2.295).abs() < 0.12, "second mode at {second} Hz");
        assert!(tone_at(&x, 200.0) < tone_at(&x, second) * 0.6, "no octave: the head is not a string");
    }

    #[test]
    fn where_you_hit_picks_the_modes() {
        // The (1,1) mode, at 1.594x, has a node at the centre and a belly near the rim.
        let level = |x: f32, y: f32| {
            let mut m = head(100.0, 2.0, 0.0, 0.0);
            m.strike(x, y, 0.8, 1.0);
            let s = render(&mut m, 1.0);
            tone_at(&s, 159.4) / tone_at(&s, 100.0)
        };
        let (centre, rim) = (level(0.0, 0.0), level(0.7, 0.0));
        assert!(rim > centre * 5.0, "rim {rim} vs centre {centre}");
    }

    #[test]
    fn a_hard_mallet_is_brighter_than_a_soft_one() {
        let bright = |hard: f32| {
            let mut m = head(100.0, 1.0, 0.0, 0.0);
            m.strike(0.3, 0.1, hard, 0.7);
            let x = render(&mut m, 0.3);
            // The energy in the sample-to-sample difference over the energy in the signal.
            let d: f32 = x.windows(2).map(|w| (w[1] - w[0]).powi(2)).sum();
            let e: f32 = x.iter().map(|v| v * v).sum();
            (d / e).sqrt()
        };
        assert!(bright(1.0) > bright(0.0) * 1.15, "{} vs {}", bright(1.0), bright(0.0));
    }

    #[test]
    fn decay_time_sets_how_fast_it_dies() {
        let mut m = head(120.0, 0.3, 0.0, 0.0);
        m.strike(0.0, 0.0, 0.5, 1.0);
        let x = render(&mut m, 0.6);
        let rms = |from: f32, to: f32| {
            let s = &x[(from * FS) as usize..(to * FS) as usize];
            (s.iter().map(|v| v * v).sum::<f32>() / s.len() as f32).sqrt()
        };
        // One decay time apart the amplitude falls by 1/e.
        let ratio = rms(0.35, 0.40) / rms(0.05, 0.10);
        assert!((ratio - (-1.0f32).exp()).abs() < 0.12, "ratio {ratio}");
    }

    #[test]
    fn more_skin_loss_kills_the_overtones_sooner() {
        let overtone_share = |tilt: f32| {
            let mut m = head(100.0, 1.0, tilt, 0.0);
            m.strike(0.0, 0.0, 1.0, 1.0);
            let x = render(&mut m, 0.6);
            let late = &x[(0.4 * FS) as usize..];
            tone_at(late, 229.5) / tone_at(late, 100.0)
        };
        assert!(overtone_share(1.0) < overtone_share(0.0) * 0.5);
    }

    #[test]
    fn tension_modulation_starts_sharp_and_settles() {
        // And with the bend off the pitch holds.
        let mut flat = head(80.0, 1.5, 0.0, 0.0);
        flat.strike(0.0, 0.0, 0.0, 1.0);
        let y = render(&mut flat, 0.5);
        let f_early = peak_hz(&y[..(0.2 * FS) as usize], 50.0, 140.0);
        let f_late = peak_hz(&y[(0.3 * FS) as usize..], 50.0, 140.0);
        assert!((f_early - f_late).abs() < 4.0);
        let z = {
            let mut m = head(80.0, 1.5, 0.0, 1.0);
            m.strike(0.0, 0.0, 0.0, 1.0);
            render(&mut m, 0.5)
        };
        let g_early = peak_hz(&z[..(0.1 * FS) as usize], 50.0, 260.0);
        let g_late = peak_hz(&z[(0.35 * FS) as usize..], 50.0, 260.0);
        assert!(g_early > g_late * 1.1, "{g_early} then {g_late}");
    }

    #[test]
    fn harder_hits_are_louder_and_it_stays_finite_everywhere() {
        let peak = |v: f32| {
            let mut m = head(100.0, 0.5, 0.1, 0.0);
            m.strike(0.2, 0.2, 0.5, v);
            render(&mut m, 0.2).iter().fold(0.0f32, |p, s| p.max(s.abs()))
        };
        assert!(peak(1.0) > peak(0.5) * 1.6);
        for (hz, decay, tilt, bend, hard) in [(30.0, 4.0, 1.0, 1.0, 1.0), (600.0, 0.02, 1.0, 1.0, 1.0), (600.0, 4.0, 1.0, 1.0, 0.0), (30.0, 0.02, 0.0, 0.0, 0.5)] {
            let mut m = head(hz, decay, tilt, bend);
            for k in 0..4 {
                m.strike(0.9 * (k as f32 - 1.5) / 1.5, 0.3, hard, 1.0);
            }
            let x = render(&mut m, 0.5);
            assert!(x.iter().all(|v| v.is_finite() && v.abs() < 50.0), "unstable at {hz} Hz decay {decay} tilt {tilt} bend {bend}");
        }
    }

    #[test]
    fn it_goes_quiet_for_good_and_a_new_hit_wakes_it() {
        let mut m = head(200.0, 0.02, 0.0, 0.0);
        m.strike(0.0, 0.0, 0.5, 1.0);
        render(&mut m, 1.0);
        assert!(!m.active() && m.surface().iter().all(|v| *v == 0.0));
        m.strike(0.0, 0.0, 0.5, 1.0);
        assert!(m.active());
    }

    #[test]
    fn the_envelopes_show_where_and_at_what_pitch_the_skin_moves() {
        // A soft centre hit swings the middle at the fundamental and barely moves the rim.
        let mut m = head(100.0, 2.0, 0.0, 0.0);
        m.strike(0.0, 0.0, 0.0, 1.0);
        render(&mut m, 0.3);
        let (disp, vel) = m.envelopes();
        let (mid, rim) = (12 * N + 12, 12 * N + 22);
        assert!(disp[mid] > disp[rim] * 3.0, "{} vs {}", disp[mid], disp[rim]);
        assert!(vel[mid] > 0.0);
        // The harder the mallet, the more of the motion is overtones: velocity outgrows displacement.
        let overtone_ratio = |hard: f32| {
            let mut h = head(100.0, 2.0, 0.0, 0.0);
            h.strike(0.3, 0.0, hard, 1.0);
            render(&mut h, 0.05);
            let (disp, vel) = h.envelopes();
            let near = 12 * N + 16;
            vel[near] / disp[near]
        };
        assert!(overtone_ratio(1.0) > overtone_ratio(0.0) * 1.2, "{} vs {}", overtone_ratio(1.0), overtone_ratio(0.0));
    }

    #[test]
    fn the_rim_is_clamped() {
        let mut m = head(100.0, 1.0, 0.0, 0.0);
        m.strike(0.9, 0.0, 0.5, 1.0);
        render(&mut m, 0.1);
        for (i, v) in m.surface().iter().enumerate() {
            if !in_head(i) {
                assert_eq!(*v, 0.0);
            }
        }
    }
}
