use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, egui, EguiState};
use std::sync::Arc;

pub mod dsp;
mod pitch;
pub mod presets;
pub mod ui;

use dsp::{Chord, Model, Resonator, Settings, Source, MAX_CHANNELS};

/// A tuned resonator bank. See `dsp.rs`.
///
/// No latency: the dry path is untouched, and the wet path is a bank of causal filters. The keys
/// held in MIDI mode latch when released.
struct Tines {
    params: Arc<TinesParams>,
    fx: Resonator,
}

#[derive(Params)]
pub struct TinesParams {
    #[persist = "editor-state"]
    editor_state: Arc<EguiState>,

    #[id = "source"]
    source: EnumParam<Source>,
    #[id = "chord"]
    chord: EnumParam<Chord>,
    #[id = "model"]
    model: EnumParam<Model>,
    #[id = "note"]
    note: FloatParam,
    #[id = "octave"]
    octave: IntParam,
    #[id = "fine"]
    fine: FloatParam,
    #[id = "decay"]
    decay: FloatParam,
    #[id = "damping"]
    damping: FloatParam,
    #[id = "tilt"]
    tilt: FloatParam,
    #[id = "partials"]
    partials: IntParam,
    #[id = "stiffness"]
    stiffness: FloatParam,
    #[id = "detune"]
    detune: FloatParam,
    #[id = "drift"]
    drift: FloatParam,
    #[id = "feedback"]
    feedback: FloatParam,
    #[id = "pluck"]
    pluck: FloatParam,
    #[id = "tone"]
    tone: FloatParam,
    #[id = "level"]
    level: FloatParam,
    #[id = "mix"]
    mix: FloatParam,
    #[id = "freeze"]
    freeze: BoolParam,
}

fn percent(name: &str, default: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
        .with_smoother(SmoothingStyle::Linear(40.0))
        .with_unit(" %")
        .with_value_to_string(formatters::v2s_f32_percentage(0))
        .with_string_to_value(formatters::s2v_f32_percentage())
}

const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

/// "A2" for MIDI note 45 (the convention where middle C is C4).
pub fn note_name(note: f32) -> String {
    let n = note.round() as i32;
    format!("{}{}", NAMES[n.rem_euclid(12) as usize], n.div_euclid(12) - 1)
}

impl Default for TinesParams {
    fn default() -> Self {
        let d = Settings::default();
        Self {
            editor_state: EguiState::from_size(920, 660),
            source: EnumParam::new("Source", d.source),
            chord: EnumParam::new("Chord", d.chord),
            model: EnumParam::new("Model", d.model),
            // Not smoothed: the bank glides between notes itself.
            note: FloatParam::new("Note", d.note, FloatRange::Linear { min: 12.0, max: 96.0 })
                .with_value_to_string(Arc::new(|v| note_name(v))),
            octave: IntParam::new("Octave", d.octave, IntRange::Linear { min: -3, max: 3 })
                .with_value_to_string(Arc::new(|v| format!("{v:+}"))),
            fine: FloatParam::new("Fine", d.fine_cents, FloatRange::Linear { min: -100.0, max: 100.0 })
                .with_unit(" ct")
                .with_value_to_string(formatters::v2s_f32_rounded(0)),
            decay: FloatParam::new(
                "Decay",
                d.decay_s,
                FloatRange::Skewed { min: 0.05, max: 20.0, factor: FloatRange::skew_factor(-2.0) },
            )
            .with_smoother(SmoothingStyle::Linear(30.0))
            .with_value_to_string(Arc::new(|v| if v < 1.0 { format!("{:.0} ms", v * 1000.0) } else { format!("{v:.1} s") })),
            damping: percent("Damping", d.damping),
            tilt: FloatParam::new("Tilt", d.tilt, FloatRange::Linear { min: -1.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(40.0))
                .with_value_to_string(Arc::new(|v| format!("{:+.0}", v * 100.0))),
            partials: IntParam::new("Partials", d.partials as i32, IntRange::Linear { min: 1, max: 16 }),
            stiffness: percent("Stiffness", d.stiffness),
            detune: FloatParam::new("Detune", d.detune, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(40.0))
                .with_value_to_string(Arc::new(|v| format!("{:.0} ct", v * 40.0))),
            drift: percent("Drift", d.drift),
            feedback: percent("Feedback", d.feedback),
            pluck: percent("Pluck", d.pluck),
            tone: FloatParam::new(
                "Tone",
                d.tone_hz,
                FloatRange::Skewed { min: 500.0, max: 20000.0, factor: FloatRange::skew_factor(-1.5) },
            )
            .with_smoother(SmoothingStyle::Linear(40.0))
            .with_value_to_string(Arc::new(|v| if v >= 19900.0 { "off".to_string() } else { format!("{v:.0} Hz") })),
            level: FloatParam::new("Level", d.level_db, FloatRange::Linear { min: -24.0, max: 12.0 })
                .with_smoother(SmoothingStyle::Linear(40.0))
                .with_unit(" dB")
                .with_value_to_string(formatters::v2s_f32_rounded(1)),
            mix: percent("Mix", d.mix),
            freeze: BoolParam::new("Freeze", d.freeze),
        }
    }
}

impl TinesParams {
    /// The current values with no smoothing, for the editor.
    fn settings(&self) -> Settings {
        Settings {
            source: self.source.value(),
            chord: self.chord.value(),
            model: self.model.value(),
            note: self.note.value(),
            octave: self.octave.value(),
            fine_cents: self.fine.value(),
            decay_s: self.decay.value(),
            damping: self.damping.value(),
            tilt: self.tilt.value(),
            partials: self.partials.value() as usize,
            stiffness: self.stiffness.value(),
            detune: self.detune.value(),
            drift: self.drift.value(),
            feedback: self.feedback.value(),
            pluck: self.pluck.value(),
            tone_hz: self.tone.value(),
            level_db: self.level.value(),
            mix: self.mix.value(),
            freeze: self.freeze.value(),
        }
    }
}

impl Default for Tines {
    fn default() -> Self {
        Self { params: Arc::new(TinesParams::default()), fx: Resonator::new() }
    }
}

impl Plugin for Tines {
    const NAME: &'static str = "Tines Resonator";
    const VENDOR: &'static str = "Clowd";
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

    const MIDI_INPUT: MidiConfig = MidiConfig::Basic;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        let params = self.params.clone();
        let meter = self.fx.meter.clone();
        create_egui_editor(
            self.params.editor_state.clone(),
            ui::UiState::default(),
            |_, _| {},
            move |ctx, setter, state| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE.fill(ui::background()).inner_margin(16.0))
                    .show(ctx, |ui| ui::draw(ui, &params, setter, &meter, state));
            },
        )
    }

    fn initialize(
        &mut self,
        audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        context: &mut impl InitContext<Self>,
    ) -> bool {
        let channels = audio_io_layout.main_input_channels.map_or(2, |c| c.get() as usize);
        self.fx.configure(buffer_config.sample_rate, channels);
        context.set_latency_samples(0);
        true
    }

    fn reset(&mut self) {
        self.fx.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let p = &self.params;
        let source = p.source.value();
        let chord = p.chord.value();
        let model = p.model.value();
        let octave = p.octave.value();
        let partials = p.partials.value() as usize;
        let freeze = p.freeze.value();
        let note = p.note.value();
        let fine = p.fine.value();

        let mut next_event = context.next_event();
        for (i, mut frame) in buffer.iter_samples().enumerate() {
            while let Some(event) = next_event {
                if event.timing() > i as u32 {
                    break;
                }
                match event {
                    NoteEvent::NoteOn { note, velocity, .. } if velocity > 0.0 => self.fx.note_on(note),
                    NoteEvent::NoteOn { note, .. } | NoteEvent::NoteOff { note, .. } => self.fx.note_off(note),
                    _ => {}
                }
                next_event = context.next_event();
            }

            let settings = Settings {
                source,
                chord,
                model,
                note,
                octave,
                fine_cents: fine,
                decay_s: p.decay.smoothed.next(),
                damping: p.damping.smoothed.next(),
                tilt: p.tilt.smoothed.next(),
                partials,
                stiffness: p.stiffness.smoothed.next(),
                detune: p.detune.smoothed.next(),
                drift: p.drift.smoothed.next(),
                feedback: p.feedback.smoothed.next(),
                pluck: p.pluck.smoothed.next(),
                tone_hz: p.tone.smoothed.next(),
                level_db: p.level.smoothed.next(),
                mix: p.mix.smoothed.next(),
                freeze,
            };
            let mut x = [0.0; MAX_CHANNELS];
            for (ch, sample) in frame.iter_mut().enumerate().take(MAX_CHANNELS) {
                x[ch] = *sample;
            }
            let out = self.fx.process(x, &settings);
            for (ch, sample) in frame.iter_mut().enumerate().take(MAX_CHANNELS) {
                *sample = out[ch];
            }
        }
        ProcessStatus::Normal
    }
}

impl Vst3Plugin for Tines {
    const VST3_CLASS_ID: [u8; 16] = *b"ClowdTinesReson_";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Fx, Vst3SubCategory::Filter];
}

nih_export_vst3!(Tines);
