//! The electro kit: 808-style synthesis, one voice per drum.

use crate::pattern::*;
use std::f32::consts::TAU;

/// The six square-wave ratios of the classic 808 metallic tone.
const METAL: [f32; 6] = [205.3, 296.9, 332.0, 395.5, 514.1, 546.1];

#[derive(Clone, Copy, Default)]
pub struct Voice {
    pub on: bool,
    age: f32,
    pub vel: f32,
    phase: [f32; 6],
    lp: f32,
    lp2: f32,
}

impl Voice {
    pub fn start(vel: f32) -> Self {
        Voice { on: true, vel, ..Voice::default() }
    }

    /// One sample, and whether the voice is still sounding.
    pub fn next(&mut self, lane: usize, dt: f32, noise: f32) -> f32 {
        let t = self.age;
        let (s, life) = voice(lane, self, t, dt, noise);
        self.age += dt;
        if self.age > life {
            self.on = false;
        }
        s * self.vel
    }
}

fn metallic(v: &mut Voice, dt: f32, scale: f32) -> f32 {
    let mut sum = 0.0;
    for (ph, f) in v.phase.iter_mut().zip(METAL) {
        *ph = (*ph + f * scale * dt).fract();
        sum += if *ph < 0.5 { 1.0 } else { -1.0 };
    }
    sum / 6.0
}

/// One sample of a lane's voice and how long (in seconds) it lasts.
fn voice(lane: usize, v: &mut Voice, t: f32, dt: f32, noise: f32) -> (f32, f32) {
    match lane {
        LANE_KICK => {
            let f = 46.0 + 130.0 * (-t / 0.03).exp();
            v.phase[0] = (v.phase[0] + f * dt).fract();
            let body = (v.phase[0] * TAU).sin() * (-t / 0.32).exp();
            let click = noise * (-t / 0.0015).exp() * 0.35;
            (((body + click) * 1.7).tanh() * 0.9, 0.8)
        }
        LANE_SNARE => {
            let f = 185.0 * (1.0 + 0.5 * (-t / 0.02).exp());
            v.phase[0] = (v.phase[0] + f * dt).fract();
            v.lp += 0.35 * (noise - v.lp);
            let hiss = noise - v.lp;
            let s = (v.phase[0] * TAU).sin() * (-t / 0.09).exp() * 0.5 + hiss * (-t / 0.13).exp() * 0.7;
            (s, 0.6)
        }
        LANE_RIM => {
            v.phase[0] = (v.phase[0] + 1750.0 * dt).fract();
            v.phase[1] = (v.phase[1] + 480.0 * dt).fract();
            let s = (v.phase[0] * TAU).sin() * 0.45 + (v.phase[1] * TAU).sin() * 0.45;
            (s * (-t / 0.012).exp(), 0.12)
        }
        LANE_CLAP => {
            // Three quick bursts then a tail, through a band around 1.3 kHz.
            let mut env = 0.0;
            for start in [0.0f32, 0.011, 0.022] {
                if t >= start {
                    env += (-(t - start) / 0.004).exp();
                }
            }
            if t > 0.033 {
                env += (-(t - 0.033) / 0.09).exp() * 0.8;
            }
            v.lp += 0.45 * (noise - v.lp);
            v.lp2 += 0.05 * (v.lp - v.lp2);
            (((v.lp - v.lp2) * 2.6) * env * 0.45, 0.5)
        }
        LANE_HAT | LANE_OPEN => {
            let decay = if lane == LANE_HAT { 0.03 } else { 0.28 };
            let x = metallic(v, dt, 1.0) * 0.7 + noise * 0.3;
            v.lp += 0.75 * (x - v.lp);
            ((x - v.lp) * (-t / decay).exp() * 0.8, decay * 8.0)
        }
        LANE_LTOM | LANE_HTOM => {
            let base = if lane == LANE_LTOM { 98.0 } else { 150.0 };
            let f = base * (1.0 + 0.7 * (-t / 0.05).exp());
            v.phase[0] = (v.phase[0] + f * dt).fract();
            ((v.phase[0] * TAU).sin() * (-t / 0.22).exp() * 0.9, 1.0)
        }
        LANE_CRASH => {
            let x = metallic(v, dt, 1.18) * 0.5 + noise * 0.5;
            v.lp += 0.6 * (x - v.lp);
            ((x - v.lp) * (-t / 0.9).exp() * (1.0 - (-t / 0.002).exp()).max(0.2) * 0.7, 3.0)
        }
        LANE_RIDE => {
            let x = metallic(v, dt, 1.45) * 0.45 + noise * 0.1;
            v.phase[5] = (v.phase[5] + 3150.0 * dt).fract();
            v.lp += 0.7 * (x - v.lp);
            let bell = (v.phase[5] * TAU).sin() * (-t / 0.35).exp() * 0.25;
            ((x - v.lp) * (-t / 0.45).exp() * 0.7 + bell, 2.0)
        }
        _ => {
            // Cowbell: two detuned squares through a band-pass.
            v.phase[0] = (v.phase[0] + 587.0 * dt).fract();
            v.phase[1] = (v.phase[1] + 845.0 * dt).fract();
            let x = (if v.phase[0] < 0.5 { 1.0 } else { -1.0 }) + (if v.phase[1] < 0.5 { 1.0 } else { -1.0 });
            v.lp += 0.2 * (x - v.lp);
            v.lp2 += 0.03 * (v.lp - v.lp2);
            ((v.lp - v.lp2) * (-t / 0.1).exp() * 0.4, 0.8)
        }
    }
}

