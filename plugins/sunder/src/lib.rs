use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, egui, EguiState};
use std::sync::Arc;

pub mod dsp;
mod pitch;
pub mod presets;
pub mod ui;

use dsp::{Circuit, Oversampling, RingMod, Settings, Source, Wave, MAX_CHANNELS};

/// A ring modulator and frequency shifter. See `dsp.rs`.
///
/// Latency is the oversampler's, and the dry path is delayed to match, so Mix sweeps without
/// combing. The note that sets the carrier in MIDI mode is the last one played.
struct Sunder {
    params: Arc<SunderParams>,
    fx: RingMod,
    latency: u32,
}

#[derive(Params)]
pub struct SunderParams {
    #[persist = "editor-state"]
    editor_state: Arc<EguiState>,

    #[id = "source"]
    source: EnumParam<Source>,
    #[id = "circuit"]
    circuit: EnumParam<Circuit>,
    #[id = "wave"]
    wave: EnumParam<Wave>,
    #[id = "os"]
    os: EnumParam<Oversampling>,
    #[id = "freq"]
    freq: FloatParam,
    #[id = "ratio"]
    ratio: FloatParam,
    #[id = "drive"]
    drive: FloatParam,
    #[id = "bias"]
    bias: FloatParam,
    #[id = "shift"]
    shift: FloatParam,
    #[id = "follow"]
    follow: FloatParam,
    #[id = "wobble"]
    wobble: FloatParam,
    #[id = "rate"]
    rate: FloatParam,
    #[id = "spread"]
    spread: FloatParam,
    #[id = "tone"]
    tone: FloatParam,
    #[id = "mix"]
    mix: FloatParam,
}

fn percent(name: &str, default: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
        .with_smoother(SmoothingStyle::Linear(40.0))
        .with_unit(" %")
        .with_value_to_string(formatters::v2s_f32_percentage(0))
        .with_string_to_value(formatters::s2v_f32_percentage())
}

impl Default for SunderParams {
    fn default() -> Self {
        let d = Settings::default();
        Self {
            editor_state: EguiState::from_size(920, 660),
            source: EnumParam::new("Source", d.source),
            circuit: EnumParam::new("Circuit", d.circuit),
            wave: EnumParam::new("Wave", d.wave),
            os: EnumParam::new("Oversampling", d.oversampling),
            // Moving the carrier sweeps the sidebands, so it glides.
            freq: FloatParam::new(
                "Freq",
                d.freq_hz,
                FloatRange::Skewed { min: 0.5, max: 8000.0, factor: FloatRange::skew_factor(-2.0) },
            )
            .with_smoother(SmoothingStyle::Linear(30.0))
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),
            ratio: FloatParam::new(
                "Ratio",
                d.ratio,
                FloatRange::Skewed { min: 0.25, max: 8.0, factor: FloatRange::skew_factor(-1.0) },
            )
            .with_smoother(SmoothingStyle::Linear(30.0))
            .with_unit("x")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            drive: percent("Drive", d.drive),
            bias: percent("Bias", d.bias),
            shift: FloatParam::new("Sideband", d.shift, FloatRange::Linear { min: -1.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(40.0))
                .with_value_to_string(Arc::new(|v| match v {
                    v if v.abs() < 0.005 => "both".to_string(),
                    v if v > 0.0 => format!("up {:.0}", v * 100.0),
                    v => format!("dn {:.0}", -v * 100.0),
                })),
            follow: FloatParam::new("Follow", d.follow, FloatRange::Linear { min: -1.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(40.0))
                .with_unit(" %")
                .with_value_to_string(Arc::new(|v| format!("{:+.0}", v * 100.0))),
            wobble: percent("Wobble", d.wobble),
            rate: FloatParam::new(
                "Wobble rate",
                d.wobble_hz,
                FloatRange::Skewed { min: 0.05, max: 20.0, factor: FloatRange::skew_factor(-2.0) },
            )
            .with_smoother(SmoothingStyle::Linear(40.0))
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            spread: percent("Spread", d.spread),
            tone: FloatParam::new(
                "Tone",
                d.tone_hz,
                FloatRange::Skewed { min: 400.0, max: 20000.0, factor: FloatRange::skew_factor(-1.5) },
            )
            .with_smoother(SmoothingStyle::Linear(40.0))
            .with_value_to_string(Arc::new(|v| if v >= 19900.0 { "off".to_string() } else { format!("{v:.0} Hz") })),
            mix: percent("Mix", d.mix),
        }
    }
}

impl SunderParams {
    /// The current values with no smoothing, for the editor.
    fn settings(&self) -> Settings {
        Settings {
            source: self.source.value(),
            circuit: self.circuit.value(),
            wave: self.wave.value(),
            oversampling: self.os.value(),
            freq_hz: self.freq.value(),
            ratio: self.ratio.value(),
            drive: self.drive.value(),
            bias: self.bias.value(),
            shift: self.shift.value(),
            follow: self.follow.value(),
            wobble: self.wobble.value(),
            wobble_hz: self.rate.value(),
            spread: self.spread.value(),
            tone_hz: self.tone.value(),
            mix: self.mix.value(),
        }
    }
}

impl Default for Sunder {
    fn default() -> Self {
        Self { params: Arc::new(SunderParams::default()), fx: RingMod::new(), latency: 0 }
    }
}

impl Plugin for Sunder {
    const NAME: &'static str = "Sunder Ring Mod";
    const VENDOR: &'static str = "Clowd";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(2),
            main_output_channels: NonZeroU32::new(2),
            aux_input_ports: &[new_nonzero_u32(2)],
            names: PortNames { aux_inputs: &["Carrier"], ..PortNames::const_default() },
            ..AudioIOLayout::const_default()
        },
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(1),
            main_output_channels: NonZeroU32::new(1),
            aux_input_ports: &[new_nonzero_u32(1)],
            names: PortNames { aux_inputs: &["Carrier"], ..PortNames::const_default() },
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
        self.fx.set_oversampling(self.params.os.value().stages());
        self.latency = self.fx.latency();
        context.set_latency_samples(self.latency);
        true
    }

    fn reset(&mut self) {
        self.fx.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let p = &self.params;
        let stages = p.os.value().stages();
        if stages != self.fx.stages() {
            self.fx.set_oversampling(stages);
        }
        if self.fx.latency() != self.latency {
            self.latency = self.fx.latency();
            context.set_latency_samples(self.latency);
        }

        let mut next_event = context.next_event();
        for (i, mut frame) in buffer.iter_samples().enumerate() {
            while let Some(event) = next_event {
                if event.timing() > i as u32 {
                    break;
                }
                if let NoteEvent::NoteOn { note, .. } = event {
                    self.fx.note_on(note);
                }
                next_event = context.next_event();
            }

            let settings = Settings {
                source: p.source.value(),
                circuit: p.circuit.value(),
                wave: p.wave.value(),
                oversampling: p.os.value(),
                freq_hz: p.freq.smoothed.next(),
                ratio: p.ratio.smoothed.next(),
                drive: p.drive.smoothed.next(),
                bias: p.bias.smoothed.next(),
                shift: p.shift.smoothed.next(),
                follow: p.follow.smoothed.next(),
                wobble: p.wobble.smoothed.next(),
                wobble_hz: p.rate.smoothed.next(),
                spread: p.spread.smoothed.next(),
                tone_hz: p.tone.smoothed.next(),
                mix: p.mix.smoothed.next(),
            };
            let mut x = [0.0; MAX_CHANNELS];
            let mut side = [0.0; MAX_CHANNELS];
            for (ch, sample) in frame.iter_mut().enumerate().take(MAX_CHANNELS) {
                x[ch] = *sample;
                side[ch] = aux
                    .inputs
                    .first()
                    .and_then(|b| b.as_slice_immutable().get(ch))
                    .and_then(|s| s.get(i))
                    .copied()
                    .unwrap_or(0.0);
            }
            let out = self.fx.process(x, side, &settings);
            for (ch, sample) in frame.iter_mut().enumerate().take(MAX_CHANNELS) {
                *sample = out[ch];
            }
        }
        ProcessStatus::Normal
    }
}

impl Vst3Plugin for Sunder {
    const VST3_CLASS_ID: [u8; 16] = *b"ClowdSunderRing_";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Modulation];
}

nih_export_vst3!(Sunder);
