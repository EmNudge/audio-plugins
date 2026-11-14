//! A small stereo room, so the drums sit in a space instead of in a vacuum. Freeverb's layout
//! (parallel damped combs into series allpasses) with the delays shortened to a room rather than a
//! hall. Buffers are allocated once, when the sample rate is set.

const COMBS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASSES: [usize; 4] = [556, 441, 341, 225];
const STEREO_SPREAD: usize = 23;
/// Delay lengths are tuned at 44.1 kHz; this makes the room small.
const ROOM_SCALE: f32 = 0.62;

struct Comb {
    buf: Vec<f32>,
    pos: usize,
    store: f32,
}

impl Comb {
    fn new(len: usize) -> Self {
        Comb { buf: vec![0.0; len.max(2)], pos: 0, store: 0.0 }
    }

    fn process(&mut self, x: f32, feedback: f32, damp: f32) -> f32 {
        let out = self.buf[self.pos];
        self.store = out * (1.0 - damp) + self.store * damp;
        self.buf[self.pos] = x + self.store * feedback;
        self.pos += 1;
        if self.pos == self.buf.len() {
            self.pos = 0;
        }
        out
    }
}

struct Allpass {
    buf: Vec<f32>,
    pos: usize,
}

impl Allpass {
    fn new(len: usize) -> Self {
        Allpass { buf: vec![0.0; len.max(2)], pos: 0 }
    }

    fn process(&mut self, x: f32) -> f32 {
        let delayed = self.buf[self.pos];
        let out = delayed - x;
        self.buf[self.pos] = x + delayed * 0.5;
        self.pos += 1;
        if self.pos == self.buf.len() {
            self.pos = 0;
        }
        out
    }
}

pub struct Reverb {
    combs: [Vec<Comb>; 2],
    allpasses: [Vec<Allpass>; 2],
    feedback: f32,
    damp: f32,
}

impl Default for Reverb {
    fn default() -> Self {
        Self::new(44100.0)
    }
}

impl Reverb {
    pub fn new(sample_rate: f32) -> Self {
        let scale = sample_rate / 44100.0 * ROOM_SCALE;
        let sized = |n: usize, extra: usize| (((n + extra) as f32 * scale) as usize).max(2);
        Reverb {
            combs: std::array::from_fn(|ch| COMBS.iter().map(|&n| Comb::new(sized(n, ch * STEREO_SPREAD))).collect()),
            allpasses: std::array::from_fn(|ch| ALLPASSES.iter().map(|&n| Allpass::new(sized(n, ch * STEREO_SPREAD))).collect()),
            feedback: 0.8,
            damp: 0.4,
        }
    }

    pub fn clear(&mut self) {
        for ch in 0..2 {
            for c in &mut self.combs[ch] {
                c.buf.fill(0.0);
                c.store = 0.0;
            }
            for a in &mut self.allpasses[ch] {
                a.buf.fill(0.0);
            }
        }
    }

    /// `x` is a mono send; returns the wet left and right.
    pub fn process(&mut self, x: f32) -> (f32, f32) {
        let input = x * 0.15;
        let mut out = [0.0f32; 2];
        for (ch, out) in out.iter_mut().enumerate() {
            let mut sum = 0.0;
            for c in &mut self.combs[ch] {
                sum += c.process(input, self.feedback, self.damp);
            }
            for a in &mut self.allpasses[ch] {
                sum = a.process(sum);
            }
            *out = sum;
        }
        (out[0], out[1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_impulse_rings_and_dies_away() {
        let mut r = Reverb::new(48000.0);
        let mut energy = Vec::new();
        let mut peak = 0.0f32;
        for i in 0..96000 {
            let (l, rr) = r.process(if i == 0 { 1.0 } else { 0.0 });
            assert!(l.is_finite() && rr.is_finite());
            peak = peak.max(l.abs()).max(rr.abs());
            energy.push(l * l + rr * rr);
        }
        assert!(peak > 0.001, "silent: {peak}");
        let early: f32 = energy[..24000].iter().sum();
        let late: f32 = energy[72000..].iter().sum();
        assert!(early > 20.0 * late, "tail does not decay: {early} vs {late}");
    }

    #[test]
    fn the_two_channels_differ() {
        let mut r = Reverb::new(44100.0);
        let mut diff = 0.0;
        for i in 0..8000 {
            let (l, rr) = r.process(if i == 0 { 1.0 } else { 0.0 });
            diff += (l - rr).abs();
        }
        assert!(diff > 0.01, "mono room");
    }

    #[test]
    fn it_is_stable_under_constant_input_and_clears() {
        let mut r = Reverb::new(44100.0);
        for _ in 0..200000 {
            let (l, rr) = r.process(1.0);
            assert!(l.abs() < 50.0 && rr.abs() < 50.0);
        }
        r.clear();
        assert_eq!(r.process(0.0), (0.0, 0.0));
    }
}
