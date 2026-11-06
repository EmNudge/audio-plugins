//! Tracker, note logic and shifter wired together. Shared by the plugin and the tests.

use crate::control::{Controller, Output, Settings};
use crate::pitch::PitchTracker;
use crate::psola::Shifter;

pub struct Engine {
    tracker: PitchTracker,
    controller: Controller,
    shifter: Shifter,
    hop_seconds: f32,
    pub last: Output,
}

impl Engine {
    pub fn new(fs: f32, f_min: f32, live: bool) -> Self {
        let tracker = PitchTracker::new(fs, f_min);
        Self {
            hop_seconds: tracker.hop_seconds(),
            tracker,
            controller: Controller::default(),
            shifter: Shifter::new(fs, f_min, live),
            last: Output::default(),
        }
    }

    pub fn reset(&mut self) {
        self.tracker.reset();
        self.controller = Controller::default();
        self.shifter.reset();
        self.last = Output::default();
    }

    pub fn latency(&self) -> usize {
        self.shifter.latency()
    }

    pub fn process(&mut self, x: [f32; 2], channels: usize, settings: &Settings) -> [f32; 2] {
        let mono = if channels == 2 { 0.5 * (x[0] + x[1]) } else { x[0] };
        if let Some(frame) = self.tracker.push(mono) {
            let out = self.controller.update(self.hop_seconds, frame.f0, frame.voiced, settings);
            self.last = out;
            self.shifter.set_control(frame.period, out.voiced, out.shift, settings.formant_semitones);
        }
        self.shifter.process(x, channels)
    }
}
