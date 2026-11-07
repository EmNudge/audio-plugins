//! Chassis: a robot-voice rig in one plugin.
//!
//! The signal runs through the chain a producer would build with a pile of sends:
//!
//! ```text
//! in -> pitch + formant shift (PSOLA) -> gate -> body EQ -> 3-way split
//!        chest (low):  sub-octave -> squash compressor
//!        body  (mid):  ring mod / vocoder -> frequency shifter -> comb -> saturation
//!        shell (high): cavity delay -> granular shine -> plate
//!      -> sum -> glue compressor -> notches -> push -> true-peak limiter -> mix
//! ```
//!
//! The gate looks at the input but acts on the shifter's output, and the vocoder's carrier pitch
//! is delayed the same way, so both line up with the audio they belong to: the shifter's latency
//! is the gate's lookahead. The total latency (shifter plus limiter) is reported, and the dry path
//! of the Mix knob is delayed to match.

use nih_plug::prelude::{AtomicF32, Enum};
use std::f32::consts::TAU;
use std::sync::atomic::Ordering;
use std::sync::Arc;

pub use crate::filters::MAX_CHANNELS;
use crate::dynamics::{BodyEq, Gate, Glue, Limiter, Notches, Squash, CONTROL};
use crate::filters::{db_to_lin, Crossover};
use crate::fx::{hilbert_coefs, Comb, Cavity, FreqShifter, Plate, Saturator, Shine, Spectrum, Sub, Vocoder, BINS};
use crate::pitch::PitchTracker;
use crate::psola::Shifter;

/// The lowest voice pitch the tracker and shifter are built for, in Hz.
const F_MIN: f32 = 70.0;

pub const MIN_LOW_HZ: f32 = 80.0;
pub const MAX_LOW_HZ: f32 = 600.0;
pub const MIN_HIGH_HZ: f32 = 1200.0;
pub const MAX_HIGH_HZ: f32 = 8000.0;
/// The split points never come closer than this ratio.
pub const MIN_SPLIT_RATIO: f32 = 1.8;

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Carrier {
    /// The vocoder's oscillators follow the voice's own (shifted) pitch.
    #[name = "Track"]
    Track,
    /// A fixed note.
    #[name = "Drone"]
    Drone,
}

pub fn midi_to_hz(note: f32) -> f32 {
    440.0 * 2f32.powf((note - 69.0) / 12.0)
}

/// What the audio thread publishes for the editor.
pub struct Meter {
    /// Level of each spectrum bin going in and coming out, linear.
    pub input: [AtomicF32; BINS],
    pub output: [AtomicF32; BINS],
    /// Output peak since the last block, linear.
    pub level: AtomicF32,
    /// The gate's gain at the output, 0 to 1.
    pub gate: AtomicF32,
    /// Gain reduction in dB (positive) of the chest, glue and limiter stages.
    pub squash: AtomicF32,
    pub glue: AtomicF32,
    pub limiter: AtomicF32,
    /// The voice's pitch in Hz, 0 while unvoiced, and the vocoder carrier's pitch.
    pub pitch_hz: AtomicF32,
    pub carrier_hz: AtomicF32,
    /// Reported latency in samples, and the sample rate it is measured in.
    pub latency: AtomicF32,
    pub sample_rate: AtomicF32,
}

impl Meter {
    fn new() -> Self {
        Self {
            input: std::array::from_fn(|_| AtomicF32::new(0.0)),
            output: std::array::from_fn(|_| AtomicF32::new(0.0)),
            level: AtomicF32::new(0.0),
            gate: AtomicF32::new(1.0),
            squash: AtomicF32::new(0.0),
            glue: AtomicF32::new(0.0),
            limiter: AtomicF32::new(0.0),
            pitch_hz: AtomicF32::new(0.0),
            carrier_hz: AtomicF32::new(0.0),
            latency: AtomicF32::new(0.0),
            sample_rate: AtomicF32::new(48000.0),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Settings {
    /// Pitch and formant shift in semitones.
    pub pitch: f32,
    pub formant: f32,
    pub gate_db: f32,
    pub boxy: f32,
    pub weight: f32,
    pub low_hz: f32,
    pub high_hz: f32,
    pub sub: f32,
    pub squash: f32,
    pub chest_db: f32,
    pub ring: f32,
    pub ring_hz: f32,
    pub vocoder: f32,
    pub carrier: Carrier,
    /// The drone's MIDI note.
    pub note: f32,
    /// Carrier shape, saw (0) to square (1).
    pub wave: f32,
    pub shift_hz: f32,
    pub comb: f32,
    pub comb_hz: f32,
    pub grit: f32,
    pub body_db: f32,
    pub cavity: f32,
    pub cavity_ms: f32,
    pub shine: f32,
    pub shine_st: f32,
    pub plate: f32,
    pub decay_s: f32,
    pub shell_db: f32,
    pub glue: f32,
    pub tame: f32,
    pub push_db: f32,
    pub ceiling_db: f32,
    pub mix: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            pitch: -3.0,
            formant: -1.0,
            gate_db: -55.0,
            boxy: 0.5,
            weight: 0.4,
            low_hz: 250.0,
            high_hz: 2800.0,
            sub: 0.35,
            squash: 0.6,
            chest_db: 0.0,
            ring: 0.2,
            ring_hz: 45.0,
            vocoder: 0.25,
            carrier: Carrier::Track,
            note: 36.0,
            wave: 0.3,
            shift_hz: 8.0,
            comb: 0.3,
            comb_hz: 180.0,
            grit: 0.25,
            body_db: 0.0,
            cavity: 0.35,
            cavity_ms: 14.0,
            shine: 0.4,
            shine_st: 3.0,
            plate: 0.3,
            decay_s: 0.4,
            shell_db: -3.0,
            glue: 0.4,
            tame: 0.4,
            push_db: 3.0,
            ceiling_db: -1.0,
            mix: 1.0,
        }
    }
}

impl Settings {
    /// Every effect off and the chain as close to transparent as it goes (the shifter and the
    /// crossover still run, at no shift and with a flat sum).
    pub fn neutral() -> Self {
        Self {
            pitch: 0.0,
            formant: 0.0,
            gate_db: -80.0,
            boxy: 0.0,
            weight: 0.0,
            sub: 0.0,
            squash: 0.0,
            ring: 0.0,
            vocoder: 0.0,
            shift_hz: 0.0,
            comb: 0.0,
            grit: 0.0,
            cavity: 0.0,
            shine: 0.0,
            plate: 0.0,
            glue: 0.0,
            tame: 0.0,
            push_db: 0.0,
            ceiling_db: 0.0,
            shell_db: 0.0,
            ..Self::default()
        }
    }

    /// The two split points, kept in range and apart.
    pub fn splits(&self) -> (f32, f32) {
        let low = self.low_hz.clamp(MIN_LOW_HZ, MAX_LOW_HZ);
        let high = self.high_hz.clamp(MIN_HIGH_HZ, MAX_HIGH_HZ).max(low * MIN_SPLIT_RATIO);
        (low, high)
    }
}

/// Values worked out from the settings every `CONTROL` samples.
#[derive(Default)]
struct Derived {
    chest: f32,
    body: f32,
    shell: f32,
    ring_inc: f32,
    shift_inc: f32,
    shift_mix: f32,
    comb_delay: f32,
    comb_feedback: f32,
    shine_ratio: f32,
    push: f32,
    ceiling: f32,
}

pub struct Chassis {
    pub meter: Arc<Meter>,
    fs: f32,
    channels: usize,
    tick: u32,
    d: Derived,

    tracker: PitchTracker,
    shifter: Shifter,
    f0: f32,
    voiced: bool,
    gate: Gate,
    /// The gate gain and the carrier's controls, written at the input and read `latency` later.
    delayed_gate: Vec<f32>,
    delayed_f0: Vec<f32>,
    delayed_voiced: Vec<f32>,
    delay_pos: usize,

    eq: BodyEq,
    crossover: Crossover,

    sub: Sub,
    squash: Squash,

    ring_phase: f32,
    vocoder: Vocoder,
    hilbert: [f32; 12],
    shift_phase: f32,
    shifters: [FreqShifter; MAX_CHANNELS],
    combs: [Comb; MAX_CHANNELS],
    saturators: [Saturator; MAX_CHANNELS],

    cavity: Cavity,
    shines: [Shine; MAX_CHANNELS],
    plate: Plate,

    glue: Glue,
    notches: Notches,
    limiter: Limiter,

    dry: [Vec<f32>; MAX_CHANNELS],
    dry_pos: usize,
    spectrum_in: Spectrum,
    spectrum_out: Spectrum,
    block_peak: f32,
    last_gate: f32,
    last_carrier: f32,
}

impl Chassis {
    pub fn new() -> Self {
        Self::build(48000.0, 2, Arc::new(Meter::new()))
    }

    pub fn configure(&mut self, fs: f32, channels: usize) {
        *self = Self::build(fs, channels, self.meter.clone());
        self.meter.sample_rate.store(fs, Ordering::Relaxed);
        self.meter.latency.store(self.latency() as f32, Ordering::Relaxed);
    }

    fn build(fs: f32, channels: usize, meter: Arc<Meter>) -> Self {
        let shifter = Shifter::new(fs, F_MIN, false);
        let limiter = Limiter::new(fs);
        let shifter_latency = shifter.latency();
        let total = shifter_latency + limiter.latency();
        Self {
            meter,
            fs,
            channels: channels.clamp(1, MAX_CHANNELS),
            tick: 0,
            d: Derived::default(),
            tracker: PitchTracker::new(fs, F_MIN),
            shifter,
            f0: 110.0,
            voiced: false,
            gate: Gate::new(fs),
            delayed_gate: vec![1.0; shifter_latency + 1],
            delayed_f0: vec![110.0; shifter_latency + 1],
            delayed_voiced: vec![0.0; shifter_latency + 1],
            delay_pos: 0,
            eq: BodyEq::new(fs),
            crossover: Crossover::new(),
            sub: Sub::new(fs),
            squash: Squash::new(fs),
            ring_phase: 0.0,
            vocoder: Vocoder::new(fs),
            hilbert: hilbert_coefs(fs),
            shift_phase: 0.0,
            shifters: [FreqShifter::default(); MAX_CHANNELS],
            combs: [Comb::new(fs), Comb::new(fs)],
            saturators: [Saturator::new(fs), Saturator::new(fs)],
            cavity: Cavity::new(fs),
            shines: [Shine::new(fs), Shine::new(fs)],
            plate: Plate::new(fs),
            glue: Glue::new(fs),
            notches: Notches::new(fs),
            limiter,
            dry: [vec![0.0; total + 1], vec![0.0; total + 1]],
            dry_pos: 0,
            spectrum_in: Spectrum::new(fs),
            spectrum_out: Spectrum::new(fs),
            block_peak: 0.0,
            last_gate: 1.0,
            last_carrier: 0.0,
        }
    }

    pub fn reset(&mut self) {
        self.tick = 0;
        self.tracker.reset();
        self.shifter.reset();
        self.f0 = 110.0;
        self.voiced = false;
        self.gate.reset();
        self.delayed_gate.fill(1.0);
        self.delayed_f0.fill(110.0);
        self.delayed_voiced.fill(0.0);
        self.eq.reset();
        self.crossover.clear();
        self.sub.reset();
        self.squash.reset();
        self.vocoder.reset();
        self.shifters = [FreqShifter::default(); MAX_CHANNELS];
        for c in &mut self.combs {
            c.clear();
        }
        for s in &mut self.saturators {
            s.reset();
        }
        self.cavity.reset();
        for s in &mut self.shines {
            s.reset();
        }
        self.plate.reset();
        self.glue.reset();
        self.notches.reset();
        self.limiter.reset();
        for d in &mut self.dry {
            d.fill(0.0);
        }
        self.spectrum_in.reset();
        self.spectrum_out.reset();
    }

    /// Latency in samples: the shifter's grains plus the limiter's lookahead.
    pub fn latency(&self) -> usize {
        self.shifter.latency() + self.limiter.latency()
    }

    /// How long the plate and the cavity ring on after the input stops.
    pub fn tail_samples(&self, s: &Settings) -> u32 {
        self.latency() as u32 + self.plate.tail(s.decay_s)
    }

    fn update(&mut self, s: &Settings) {
        let (low, high) = s.splits();
        self.crossover.set(low, high, self.fs);
        self.eq.update(s.boxy, s.weight);
        self.notches.set(s.tame);
        self.d = Derived {
            chest: db_to_lin(s.chest_db),
            body: db_to_lin(s.body_db),
            shell: db_to_lin(s.shell_db),
            ring_inc: TAU * s.ring_hz / self.fs,
            shift_inc: TAU * s.shift_hz / self.fs,
            shift_mix: 0.5 * (s.shift_hz.abs() / 4.0).min(1.0),
            comb_delay: self.fs / s.comb_hz.max(20.0),
            comb_feedback: 0.78 * s.comb,
            shine_ratio: 2f32.powf(s.shine_st / 12.0),
            push: db_to_lin(s.push_db),
            ceiling: db_to_lin(s.ceiling_db),
        };
    }

    pub fn process(&mut self, x: [f32; MAX_CHANNELS], s: &Settings) -> [f32; MAX_CHANNELS] {
        if self.tick == 0 {
            self.update(s);
        }
        self.tick = (self.tick + 1) % CONTROL;
        let ch = self.channels;
        let d = &self.d;

        let mono = if ch == 2 { 0.5 * (x[0] + x[1]) } else { x[0] };
        self.spectrum_in.push(mono);

        // Analysis on the way in.
        if let Some(frame) = self.tracker.push(mono) {
            if frame.voiced {
                self.f0 = frame.f0;
            }
            self.voiced = frame.voiced;
            self.shifter.set_control(frame.period, frame.voiced, s.pitch, s.formant);
        }
        let level = x[..ch].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        let gate = self.gate.process(level, s.gate_db);

        // Park the controls and read what was parked `latency` samples ago, to meet the shifted audio.
        let len = self.delayed_gate.len();
        self.delayed_gate[self.delay_pos] = gate;
        self.delayed_f0[self.delay_pos] = self.f0;
        self.delayed_voiced[self.delay_pos] = self.voiced as u8 as f32;
        self.delay_pos = (self.delay_pos + 1) % len;
        let gate = self.delayed_gate[self.delay_pos];
        let f0 = self.delayed_f0[self.delay_pos];
        let voiced = self.delayed_voiced[self.delay_pos] > 0.5;

        let shifted = self.shifter.process(x, ch);
        let mut v = [shifted[0] * gate, shifted[1] * gate];

        // Pre-processing EQ.
        let v_mono = if ch == 2 { 0.5 * (v[0] + v[1]) } else { v[0] };
        self.eq.detect(v_mono);
        for c in 0..ch {
            v[c] = self.eq.process(c, v[c]);
        }

        let mut bands = [[0.0f32; MAX_CHANNELS]; 3];
        for c in 0..ch {
            let b = self.crossover.split(c, v[c]);
            for (band, value) in bands.iter_mut().zip(b) {
                band[c] = value;
            }
        }

        // Chest: sub-octave, then squash.
        let mut chest = bands[0];
        let low_mono = if ch == 2 { 0.5 * (chest[0] + chest[1]) } else { chest[0] };
        let sub = self.sub.process(low_mono) * s.sub;
        for c in 0..ch {
            chest[c] += sub;
        }
        let mut chest = self.squash.process(chest, ch, s.squash);
        for c in 0..ch {
            chest[c] *= d.chest;
        }

        // Body: ring mod and vocoder, frequency shifter, comb, saturation.
        let clean = bands[1];
        let mut body = clean;
        if s.ring > 1e-3 {
            let carrier = self.ring_phase.sin();
            for c in 0..ch {
                body[c] += s.ring * (1.4 * body[c] * carrier - body[c]);
            }
        }
        self.ring_phase = (self.ring_phase + d.ring_inc) % TAU;
        let carrier_hz = match s.carrier {
            Carrier::Track => f0 * 2f32.powf(s.pitch / 12.0),
            Carrier::Drone => midi_to_hz(s.note),
        };
        if s.vocoder > 1e-3 {
            let voc = self.vocoder.process(clean, ch, carrier_hz, voiced, s.carrier == Carrier::Drone, s.wave);
            for c in 0..ch {
                body[c] += s.vocoder * (voc[c] - body[c]);
            }
        }
        if d.shift_mix > 0.0 {
            let (sin, cos) = self.shift_phase.sin_cos();
            for c in 0..ch {
                let shifted = self.shifters[c].process(body[c], &self.hilbert, sin, cos);
                body[c] += d.shift_mix * (shifted - body[c]);
            }
        }
        self.shift_phase = (self.shift_phase + d.shift_inc).rem_euclid(TAU);
        for c in 0..ch {
            if s.comb > 1e-3 {
                let y = self.combs[c].process(body[c], d.comb_delay, d.comb_feedback);
                body[c] += s.comb * (y * (1.0 - 0.5 * d.comb_feedback) - body[c]);
            }
            body[c] = self.saturators[c].process(body[c], s.grit) * d.body;
        }

        // Shell: cavity, shine, plate.
        let mut shell = self.cavity.process(bands[2], ch, s.cavity_ms, s.cavity);
        if s.shine > 1e-3 {
            for c in 0..ch {
                shell[c] += s.shine * self.shines[c].process(shell[c], d.shine_ratio);
            }
        }
        let send = if ch == 2 { 0.5 * (shell[0] + shell[1]) } else { shell[0] };
        let ring = self.plate.process(send, s.decay_s);
        let ring = if ch == 2 { ring } else { [0.5 * (ring[0] + ring[1]); 2] };
        for c in 0..ch {
            shell[c] = (shell[c] + s.plate * 1.2 * ring[c]) * d.shell;
        }

        // Master.
        let mut sum = [chest[0] + body[0] + shell[0], chest[1] + body[1] + shell[1]];
        sum = self.glue.process(sum, ch, s.glue);
        for c in 0..ch {
            sum[c] = self.notches.process(c, sum[c]) * d.push;
        }
        let wet = self.limiter.process(sum, ch, d.ceiling);

        // Mix against the input, delayed to match.
        let mut out = [0.0; MAX_CHANNELS];
        let dry_len = self.dry[0].len();
        for c in 0..ch {
            self.dry[c][self.dry_pos] = x[c];
            let dry = self.dry[c][(self.dry_pos + 1) % dry_len];
            out[c] = dry + (wet[c] - dry) * s.mix;
        }
        self.dry_pos = (self.dry_pos + 1) % dry_len;

        let out_mono = if ch == 2 { 0.5 * (out[0] + out[1]) } else { out[0] };
        self.spectrum_out.push(out_mono);
        self.block_peak = out[..ch].iter().fold(self.block_peak, |m, v| m.max(v.abs()));
        self.last_gate = gate;
        self.last_carrier = self.vocoder.carrier_hz();
        out
    }

    /// Hand the editor what happened since the last call.
    pub fn publish(&mut self) {
        let m = &self.meter;
        self.spectrum_in.publish(&m.input);
        self.spectrum_out.publish(&m.output);
        m.level.store(self.block_peak, Ordering::Relaxed);
        self.block_peak = 0.0;
        m.gate.store(self.last_gate, Ordering::Relaxed);
        m.squash.store(self.squash.reduction(), Ordering::Relaxed);
        m.glue.store(self.glue.reduction(), Ordering::Relaxed);
        m.limiter.store(self.limiter.reduction(), Ordering::Relaxed);
        m.pitch_hz.store(if self.voiced { self.f0 } else { 0.0 }, Ordering::Relaxed);
        m.carrier_hz.store(self.last_carrier, Ordering::Relaxed);
    }
}
