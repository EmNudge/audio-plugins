use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, egui, EguiState};
use std::sync::Arc;

pub mod dsp;
pub mod presets;
pub mod ui;

use dsp::{Compressor, Mode, Settings, LATENCY, MAX_CHANNELS};

/// Opto compressor: a lamp, a photocell with a memory, and a tube. See `dsp.rs`.
struct Firefly {
    params: Arc<FireflyParams>,
    compressor: Compressor,
}

#[derive(Params)]
pub struct FireflyParams {
    #[persist = "editor-state"]
    editor_state: Arc<EguiState>,

    #[id = "pr"]
    peak_reduction: FloatParam,
    #[id = "gain"]
    gain: FloatParam,
    #[id = "mode"]
    mode: EnumParam<Mode>,
    #[id = "emph"]
    emphasis: FloatParam,
    #[id = "tube"]
    tube: FloatParam,
    #[id = "link"]
    link: FloatParam,
    #[id = "mix"]
    mix: FloatParam,
}

fn percent(name: &str, default: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
        .with_smoother(SmoothingStyle::Linear(30.0))
        .with_unit(" %")
        .with_value_to_string(formatters::v2s_f32_percentage(0))
        .with_string_to_value(formatters::s2v_f32_percentage())
}

impl Default for FireflyParams {
    fn default() -> Self {
        let d = Settings::default();
        Self {
            editor_state: EguiState::from_size(760, 520),
            peak_reduction: percent("Peak Reduction", d.peak_reduction),
            gain: FloatParam::new("Gain", d.gain_db, FloatRange::Linear { min: -12.0, max: 24.0 })
                .with_smoother(SmoothingStyle::Linear(30.0))
                .with_unit(" dB")
                .with_value_to_string(formatters::v2s_f32_rounded(1)),
            mode: EnumParam::new("Mode", d.mode),
            emphasis: percent("Emphasis", d.emphasis),
            tube: percent("Tube", d.tube),
            link: percent("Link", d.link),
            mix: percent("Mix", d.mix),
        }
    }
}

impl Default for Firefly {
    fn default() -> Self {
        Self { params: Arc::new(FireflyParams::default()), compressor: Compressor::new() }
    }
}

impl Plugin for Firefly {
    const NAME: &'static str = "Firefly";
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

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        let params = self.params.clone();
        let meter = self.compressor.meter.clone();
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
        self.compressor.configure(buffer_config.sample_rate, channels);
        context.set_latency_samples(LATENCY as u32);
        true
    }

    fn reset(&mut self) {
        self.compressor.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let p = &self.params;
        let frames = buffer.samples();
        let mode = p.mode.value();
        for mut frame in buffer.iter_samples() {
            let settings = Settings {
                peak_reduction: p.peak_reduction.smoothed.next(),
                gain_db: p.gain.smoothed.next(),
                mode,
                emphasis: p.emphasis.smoothed.next(),
                tube: p.tube.smoothed.next(),
                link: p.link.smoothed.next(),
                mix: p.mix.smoothed.next(),
            };
            let mut x = [0.0; MAX_CHANNELS];
            for (ch, sample) in frame.iter_mut().enumerate().take(MAX_CHANNELS) {
                x[ch] = *sample;
            }
            let out = self.compressor.process(x, &settings);
            for (ch, sample) in frame.iter_mut().enumerate().take(MAX_CHANNELS) {
                *sample = out[ch];
            }
        }
        self.compressor.publish(frames);
        ProcessStatus::Normal
    }
}

impl Vst3Plugin for Firefly {
    const VST3_CLASS_ID: [u8; 16] = *b"CalvinFireflyOpt";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Fx, Vst3SubCategory::Dynamics];
}

nih_export_vst3!(Firefly);
