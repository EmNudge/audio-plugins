//! Note logic: turns the tracked pitch into a pitch-shift amount.
//!
//! Pitch is split into a slow part (the note being sung) and a fast part (vibrato). The slow part
//! picks the target note with hysteresis. `humanize` decides how much of the fast part survives,
//! and `retune_ms` how quickly the correction is allowed to move.

use crate::scale::{is_allowed, nearest_allowed};

const SLOW_TAU: f32 = 0.12;
const MAX_SHIFT: f32 = 6.0;
/// A pitch this far (semitones) from the slow average, for three frames, is a new note.
const NEW_NOTE: f32 = 1.0;
/// The slow pitch must be this much closer to another note before the target switches.
const HYSTERESIS: f32 = 0.2;

pub struct Settings {
    pub mask: u16,
    pub retune_ms: f32,
    pub humanize: f32,
    pub amount: f32,
    pub detune_cents: f32,
    pub formant_semitones: f32,
}

#[derive(Clone, Copy, Default)]
pub struct Output {
    pub voiced: bool,
    /// Correction in semitones.
    pub shift: f32,
    /// Detected, target and corrected pitch as MIDI note numbers. Only valid when `voiced`.
    pub input_midi: f32,
    pub target_midi: f32,
    pub output_midi: f32,
}

#[derive(Default)]
pub struct Controller {
    history: [f32; 3],
    history_len: usize,
    has_note: bool,
    m_slow: f32,
    target: f32,
    jump: u32,
    shift: f32,
}

pub fn hz_to_midi(f0: f32, detune_cents: f32) -> f32 {
    69.0 + 12.0 * (f0 / (440.0 * 2f32.powf(detune_cents / 1200.0))).log2()
}

impl Controller {
    pub fn update(&mut self, dt: f32, f0: f32, voiced: bool, s: &Settings) -> Output {
        if !voiced {
            self.has_note = false;
            self.history_len = 0;
            return Output { voiced: false, shift: self.shift, ..Output::default() };
        }

        // Median of three removes single-frame glitches without adding much lag.
        let raw = hz_to_midi(f0, s.detune_cents);
        if self.history_len < 3 {
            self.history[self.history_len] = raw;
            self.history_len += 1;
        } else {
            self.history = [self.history[1], self.history[2], raw];
        }
        let mut sorted = self.history;
        sorted[..self.history_len].sort_by(|a, b| a.total_cmp(b));
        let m = sorted[self.history_len / 2];

        if !self.has_note {
            self.has_note = true;
            self.m_slow = m;
            self.target = nearest_allowed(m, s.mask);
            self.shift = 0.0;
            self.jump = 0;
        } else {
            let d = m - self.m_slow;
            self.jump = if d.abs() > NEW_NOTE { self.jump + 1 } else { 0 };
            if self.jump >= 3 {
                self.m_slow = m;
                self.target = nearest_allowed(m, s.mask);
                self.jump = 0;
            } else {
                self.m_slow += (1.0 - (-dt / SLOW_TAU).exp()) * d;
            }
        }

        let candidate = nearest_allowed(self.m_slow, s.mask);
        let stale = s.mask != 0 && !is_allowed(self.target, s.mask);
        if candidate != self.target
            && (stale || (self.m_slow - candidate).abs() + HYSTERESIS < (self.m_slow - self.target).abs())
        {
            self.target = candidate;
        }

        let reference = m + s.humanize * (self.m_slow - m);
        // With no notes to snap to there is nothing to correct.
        let wanted = if s.mask == 0 {
            0.0
        } else {
            ((self.target - reference) * s.amount).clamp(-MAX_SHIFT, MAX_SHIFT)
        };
        let tau = s.retune_ms / 1000.0;
        self.shift = if tau < 0.002 {
            wanted
        } else {
            self.shift + (1.0 - (-dt / tau).exp()) * (wanted - self.shift)
        };

        Output {
            voiced: true,
            shift: self.shift,
            input_midi: raw,
            target_midi: self.target,
            output_midi: raw + self.shift,
        }
    }
}
