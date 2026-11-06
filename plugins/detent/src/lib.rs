use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, egui, EguiState};
use std::sync::atomic::{AtomicBool, AtomicU32};
use std::sync::Arc;

mod control;
mod engine;
mod pitch;
mod psola;
mod scale;
#[cfg(test)]
mod tests;
mod ui;

use control::Settings;
use engine::Engine;
use scale::Scale;

#[derive(Enum, Debug, PartialEq, Clone, Copy)]
pub enum Range {
    #[name = "Bass 55 Hz"]
    Bass,
    #[name = "Low 75 Hz"]
    Low,
    #[name = "Mid 110 Hz"]
    Mid,
    #[name = "High 170 Hz"]
    High,
}

impl Range {
    /// The lowest pitch the tracker will follow. Lower floors need a longer analysis window,
    /// which is where the latency comes from.
    fn f_min(self) -> f32 {
        match self {
            Range::Bass => 55.0,
            Range::Low => 75.0,
            Range::Mid => 110.0,
            Range::High => 170.0,
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

#[derive(Enum, Debug, PartialEq, Clone, Copy)]
pub enum Mode {
    #[name = "Quality"]
    Quality,
    #[name = "Live"]
    Live,
}

#[derive(Enum, Debug, PartialEq, Clone, Copy)]
pub enum Target {
    #[name = "Scale"]
    Scale,
    #[name = "MIDI"]
    Midi,
}

#[derive(Enum, Debug, PartialEq, Clone, Copy)]
pub enum Key {
    #[name = "C"]
    C,
    #[name = "C#"]
    Cs,
    #[name = "D"]
    D,
    #[name = "D#"]
    Ds,
    #[name = "E"]
    E,
    #[name = "F"]
    F,
    #[name = "F#"]
    Fs,
    #[name = "G"]
    G,
    #[name = "G#"]
    Gs,
    #[name = "A"]
    A,
    #[name = "A#"]
    As,
    #[name = "B"]
    B,
}

#[derive(Params)]
pub struct NoteFlip {
    /// Inverts whether this pitch class is allowed, relative to the chosen key and scale.
    #[id = "flip"]
    pub flip: BoolParam,
}

#[derive(Params)]
pub struct DetentParams {
    #[persist = "editor-state"]
    pub editor_state: Arc<EguiState>,

    #[id = "key"]
    pub key: EnumParam<Key>,
    #[id = "scale"]
    pub scale: EnumParam<Scale>,
    #[id = "range"]
    pub range: EnumParam<Range>,
    #[id = "mode"]
    pub mode: EnumParam<Mode>,
    #[id = "target"]
    pub target: EnumParam<Target>,
    #[id = "formant"]
    pub formant: FloatParam,
    #[id = "retune"]
    pub retune: FloatParam,
    #[id = "humanize"]
    pub humanize: FloatParam,
    #[id = "amount"]
    pub amount: FloatParam,
    #[id = "detune"]
    pub detune: FloatParam,
    #[nested(array, group = "Note flips")]
    pub notes: [NoteFlip; 12],
}

impl Default for DetentParams {
    fn default() -> Self {
        let percent = |name: &'static str, default: f32| {
            FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_value_to_string(formatters::v2s_f32_percentage(0))
                .with_string_to_value(formatters::s2v_f32_percentage())
        };
        Self {
            editor_state: EguiState::from_size(700, 600),
            key: EnumParam::new("Key", Key::C),
            scale: EnumParam::new("Scale", Scale::Chromatic),
            range: EnumParam::new("Range", Range::Low),
            mode: EnumParam::new("Mode", Mode::Quality),
            target: EnumParam::new("Target", Target::Scale),
            formant: FloatParam::new("Formant", 0.0, FloatRange::Linear { min: -6.0, max: 6.0 })
                .with_unit(" st")
                .with_value_to_string(formatters::v2s_f32_rounded(1)),
            retune: FloatParam::new(
                "Retune",
                20.0,
                FloatRange::Skewed { min: 0.0, max: 400.0, factor: FloatRange::skew_factor(-1.5) },
            )
            .with_unit(" ms")
            .with_value_to_string(formatters::v2s_f32_rounded(0)),
            humanize: percent("Humanize", 0.35),
            amount: percent("Amount", 1.0),
            detune: FloatParam::new("Detune", 0.0, FloatRange::Linear { min: -50.0, max: 50.0 })
                .with_unit(" cents")
                .with_value_to_string(formatters::v2s_f32_rounded(0)),
            notes: std::array::from_fn(|i| NoteFlip {
                flip: BoolParam::new(format!("Flip {}", scale::NOTE_NAMES[i]), false),
            }),
        }
    }
}

/// Values the audio thread publishes for the editor's dial and pitch tape.
#[derive(Default)]
pub struct Telemetry {
    pub voiced: AtomicBool,
    pub input_midi: AtomicF32,
    pub target_midi: AtomicF32,
    pub output_midi: AtomicF32,
    pub latency_samples: AtomicF32,
    pub sample_rate: AtomicF32,
    /// Pitch classes of the MIDI notes currently held, as a 12-bit mask.
    pub held_mask: AtomicU32,
}

struct Detent {
    params: Arc<DetentParams>,
    /// One engine per range, all allocated up front so switching never allocates.
    engines: Vec<Engine>,
    active: usize,
    held: [bool; 128],
    telemetry: Arc<Telemetry>,
}

impl Default for Detent {
    fn default() -> Self {
        Self {
            params: Arc::new(DetentParams::default()),
            engines: Vec::new(),
            active: Range::Low.index(),
            held: [false; 128],
            telemetry: Arc::new(Telemetry::default()),
        }
    }
}

const RANGES: [Range; 4] = [Range::Bass, Range::Low, Range::Mid, Range::High];

fn engine_index(mode: Mode, range: Range) -> usize {
    (mode == Mode::Live) as usize * RANGES.len() + range.index()
}

impl Detent {
    fn settings(&self) -> Settings {
        let flips: [bool; 12] = std::array::from_fn(|i| self.params.notes[i].flip.value());
        let mask = match self.params.target.value() {
            Target::Scale => scale::mask(self.params.key.value().to_index(), self.params.scale.value(), flips),
            Target::Midi => scale::midi_mask(&self.held),
        };
        Settings {
            mask,
            formant_semitones: self.params.formant.value(),
            retune_ms: self.params.retune.value(),
            humanize: self.params.humanize.value(),
            amount: self.params.amount.value(),
            detune_cents: self.params.detune.value(),
        }
    }

    fn publish_latency(&self) {
        if let Some(engine) = self.engines.get(self.active) {
            self.telemetry.latency_samples.store(engine.latency() as f32, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

impl Plugin for Detent {
    const NAME: &'static str = "Detent";
    const MIDI_INPUT: MidiConfig = MidiConfig::Basic;
    const VENDOR: &'static str = "calvin";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(2),
            main_output_channels: NonZeroU32::new(2),
            ..AudioIOLayout::const_default()
        },
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(1),
            main_output_channels: NonZeroU32::new(1),
            ..AudioIOLayout::const_default()
        },
    ];

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        let params = self.params.clone();
        let telemetry = self.telemetry.clone();
        create_egui_editor(
            self.params.editor_state.clone(),
            ui::UiState::default(),
            |_, _| {},
            move |ctx, setter, state| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE.fill(ui::background()).inner_margin(14.0))
                    .show(ctx, |ui| ui::draw(ui, &params, setter, &telemetry, state));
            },
        )
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        context: &mut impl InitContext<Self>,
    ) -> bool {
        let fs = buffer_config.sample_rate;
        self.engines = [false, true]
            .iter()
            .flat_map(|&live| RANGES.iter().map(move |r| Engine::new(fs, r.f_min(), live)))
            .collect();
        self.active = engine_index(self.params.mode.value(), self.params.range.value());
        self.telemetry.sample_rate.store(fs, std::sync::atomic::Ordering::Relaxed);
        self.publish_latency();
        context.set_latency_samples(self.engines[self.active].latency() as u32);
        true
    }

    fn reset(&mut self) {
        for engine in &mut self.engines {
            engine.reset();
        }
        self.held = [false; 128];
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        use std::sync::atomic::Ordering::Relaxed;

        while let Some(event) = context.next_event() {
            match event {
                NoteEvent::NoteOn { note, .. } => self.held[note as usize & 127] = true,
                NoteEvent::NoteOff { note, .. } => self.held[note as usize & 127] = false,
                _ => {}
            }
        }

        let range = engine_index(self.params.mode.value(), self.params.range.value());
        if range != self.active {
            self.active = range;
            self.engines[range].reset();
            context.set_latency_samples(self.engines[range].latency() as u32);
            self.publish_latency();
        }

        let settings = self.settings();
        let engine = &mut self.engines[self.active];
        for mut frame in buffer.iter_samples() {
            let channels = frame.len().min(2);
            let mut x = [0.0; 2];
            for (c, sample) in frame.iter_mut().enumerate().take(2) {
                x[c] = *sample;
            }
            let y = engine.process(x, channels, &settings);
            for (c, sample) in frame.iter_mut().enumerate().take(2) {
                *sample = y[c];
            }
        }

        let last = engine.last;
        self.telemetry.held_mask.store(scale::midi_mask(&self.held) as u32, Relaxed);
        self.telemetry.voiced.store(last.voiced, Relaxed);
        self.telemetry.input_midi.store(last.input_midi, Relaxed);
        self.telemetry.target_midi.store(last.target_midi, Relaxed);
        self.telemetry.output_midi.store(last.output_midi, Relaxed);
        ProcessStatus::Normal
    }
}

impl Vst3Plugin for Detent {
    const VST3_CLASS_ID: [u8; 16] = *b"CalvinDetentTune";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Fx, Vst3SubCategory::PitchShift];
}

nih_export_vst3!(Detent);
