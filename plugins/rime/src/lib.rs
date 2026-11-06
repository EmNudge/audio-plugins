use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, egui, EguiState};
use std::sync::Arc;

pub mod dsp;
pub mod presets;
pub mod ui;

use dsp::{Settings, Suppressor, F_MAX, F_MIN, LATENCY};

pub struct Rime {
    params: Arc<RimeParams>,
    suppressor: Suppressor,
}

#[derive(Params)]
pub struct RimeParams {
    #[persist = "editor-state"]
    pub editor_state: Arc<EguiState>,

    #[id = "depth"]
    pub depth: FloatParam,
    #[id = "sharp"]
    pub sharpness: FloatParam,
    #[id = "select"]
    pub selectivity: FloatParam,
    #[id = "attack"]
    pub attack: FloatParam,
    #[id = "release"]
    pub release: FloatParam,
    #[id = "low"]
    pub low: FloatParam,
    #[id = "high"]
    pub high: FloatParam,
    #[id = "link"]
    pub link: FloatParam,
    #[id = "mix"]
    pub mix: FloatParam,
    #[id = "trim"]
    pub trim: FloatParam,
    #[id = "delta"]
    pub delta: BoolParam,
}

impl Default for RimeParams {
    fn default() -> Self {
        let percent = |name: &'static str, default: f32| {
            FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_value_to_string(formatters::v2s_f32_percentage(0))
                .with_string_to_value(formatters::s2v_f32_percentage())
        };
        let milliseconds = |name: &'static str, default: f32, min: f32, max: f32| {
            FloatParam::new(name, default, FloatRange::Skewed { min, max, factor: FloatRange::skew_factor(-2.0) })
                .with_unit(" ms")
                .with_value_to_string(formatters::v2s_f32_rounded(1))
        };
        let hertz = |name: &'static str, default: f32| {
            FloatParam::new(name, default, FloatRange::Skewed { min: F_MIN, max: F_MAX, factor: FloatRange::skew_factor(-2.0) })
                .with_unit(" Hz")
                .with_value_to_string(formatters::v2s_f32_rounded(0))
        };
        Self {
            editor_state: EguiState::from_size(780, 560),
            depth: percent("Depth", 0.5),
            sharpness: percent("Sharpness", 0.5),
            selectivity: percent("Selectivity", 0.5),
            attack: milliseconds("Attack", 8.0, 0.5, 100.0),
            release: milliseconds("Release", 150.0, 10.0, 1500.0),
            low: hertz("Focus Low", F_MIN),
            high: hertz("Focus High", F_MAX),
            link: percent("Stereo Link", 1.0),
            mix: percent("Mix", 1.0).with_smoother(SmoothingStyle::Linear(30.0)),
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
            delta: BoolParam::new("Delta", false),
        }
    }
}

impl RimeParams {
    fn settings(&self) -> Settings {
        // The two focus handles can cross; whichever is lower is the low edge.
        let (a, b) = (self.low.value(), self.high.value());
        Settings {
            depth: self.depth.value(),
            sharpness: self.sharpness.value(),
            selectivity: self.selectivity.value(),
            attack_ms: self.attack.value(),
            release_ms: self.release.value(),
            low_hz: a.min(b),
            high_hz: a.max(b),
            link: self.link.value(),
        }
    }
}

impl Default for Rime {
    fn default() -> Self {
        Self { params: Arc::new(RimeParams::default()), suppressor: Suppressor::new() }
    }
}

impl Plugin for Rime {
    const NAME: &'static str = "Rime";
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
        let meter = self.suppressor.meter.clone();
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
        self.suppressor.configure(buffer_config.sample_rate, channels);
        context.set_latency_samples(LATENCY as u32);
        true
    }

    fn reset(&mut self) {
        self.suppressor.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        self.suppressor.settings = self.params.settings();
        let delta = self.params.delta.value();

        for mut frame in buffer.iter_samples() {
            let mix = self.params.mix.smoothed.next();
            let trim = self.params.trim.smoothed.next();

            let mut x = [0.0; dsp::MAX_CHANNELS];
            for (ch, sample) in frame.iter_mut().enumerate().take(dsp::MAX_CHANNELS) {
                x[ch] = *sample;
            }
            let (wet, dry) = self.suppressor.process(x);
            for (ch, sample) in frame.iter_mut().enumerate().take(dsp::MAX_CHANNELS) {
                // Delta is what the suppressor took out.
                let out = if delta { dry[ch] - wet[ch] } else { dry[ch] + (wet[ch] - dry[ch]) * mix };
                *sample = out * trim;
            }
        }
        ProcessStatus::Normal
    }
}

impl Vst3Plugin for Rime {
    const VST3_CLASS_ID: [u8; 16] = *b"CalvinRimeResSup";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Fx, Vst3SubCategory::Filter];
}

nih_export_vst3!(Rime);
