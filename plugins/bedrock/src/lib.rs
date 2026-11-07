use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, egui, EguiState};
use std::sync::Arc;

pub mod dsp;
pub mod filters;
pub mod presets;
pub mod ui;

use dsp::{Bedrock, Settings, MAX_FLOOR_HZ, MIN_FLOOR_HZ};

/// A low-end exciter: see `dsp.rs`. It adds no latency: the engines are filters and shapers, and
/// the dry signal goes straight through with their output summed on top.
pub struct BedrockPlugin {
    params: Arc<BedrockParams>,
    bedrock: Bedrock,
}

#[derive(Params)]
pub struct BedrockParams {
    #[persist = "editor-state"]
    pub editor_state: Arc<EguiState>,

    /// Harmonics of the bass.
    #[id = "body"]
    pub body: FloatParam,
    /// A tracked octave below it.
    #[id = "sub"]
    pub sub: FloatParam,
    /// Nothing is added to the sub below this.
    #[id = "floor"]
    pub floor: FloatParam,
    #[id = "trim"]
    pub trim: FloatParam,
}

impl Default for BedrockParams {
    fn default() -> Self {
        let defaults = Settings::default();
        let percent = |name: &'static str, default: f32| {
            FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(50.0))
                .with_value_to_string(formatters::v2s_f32_percentage(0))
                .with_string_to_value(formatters::s2v_f32_percentage())
        };
        Self {
            editor_state: EguiState::from_size(560, 420),
            body: percent("Body", defaults.body),
            sub: percent("Sub", defaults.sub),
            floor: FloatParam::new("Floor", defaults.floor_hz, FloatRange::Linear { min: MIN_FLOOR_HZ, max: MAX_FLOOR_HZ })
                .with_smoother(SmoothingStyle::Linear(50.0))
                .with_unit(" Hz")
                .with_value_to_string(formatters::v2s_f32_rounded(0)),
            trim: FloatParam::new(
                "Trim",
                util::db_to_gain(0.0),
                FloatRange::Skewed {
                    min: util::db_to_gain(-12.0),
                    max: util::db_to_gain(12.0),
                    factor: FloatRange::gain_skew_factor(-12.0, 12.0),
                },
            )
            .with_smoother(SmoothingStyle::Logarithmic(50.0))
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_gain_to_db(1))
            .with_string_to_value(formatters::s2v_f32_gain_to_db()),
        }
    }
}

impl Default for BedrockPlugin {
    fn default() -> Self {
        Self { params: Arc::new(BedrockParams::default()), bedrock: Bedrock::new() }
    }
}

impl Plugin for BedrockPlugin {
    const NAME: &'static str = "Bedrock Bass Exciter";
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
        let meter = self.bedrock.meter.clone();
        create_egui_editor(
            self.params.editor_state.clone(),
            ui::UiState::default(),
            |_, _| {},
            move |ctx, setter, state| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE.fill(ui::background()).inner_margin(14.0))
                    .show(ctx, |ui| ui::draw(ui, &params, setter, &meter, state));
            },
        )
    }

    fn initialize(
        &mut self,
        audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        let channels = audio_io_layout.main_input_channels.map_or(2, |c| c.get() as usize);
        self.bedrock.configure(buffer_config.sample_rate, channels);
        true
    }

    fn reset(&mut self) {
        self.bedrock.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        for mut frame in buffer.iter_samples() {
            let settings = Settings {
                body: self.params.body.smoothed.next(),
                sub: self.params.sub.smoothed.next(),
                floor_hz: self.params.floor.smoothed.next(),
            };
            let trim = self.params.trim.smoothed.next();
            let mut x = [0.0; 2];
            for (ch, sample) in frame.iter_mut().enumerate().take(2) {
                x[ch] = *sample;
            }
            let out = self.bedrock.process(x, &settings);
            for (ch, sample) in frame.iter_mut().enumerate().take(2) {
                *sample = out[ch] * trim;
            }
        }
        self.bedrock.publish();
        ProcessStatus::Normal
    }
}

impl Vst3Plugin for BedrockPlugin {
    const VST3_CLASS_ID: [u8; 16] = *b"ClowdBedrockBas_";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Fx, Vst3SubCategory::Filter];
}

nih_export_vst3!(BedrockPlugin);
