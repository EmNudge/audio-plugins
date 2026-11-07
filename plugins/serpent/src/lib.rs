use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, egui, EguiState};
use std::sync::Arc;

pub mod dsp;
pub mod presets;
pub mod ui;

use dsp::{Analyzer, Detect, Engine, Listen, Settings, Shape, Telemetry, MAX_LOOKAHEAD_MS};

pub struct Serpent {
    params: Arc<SerpentParams>,
    engine: Engine,
    analyzer: Analyzer,
    telemetry: Arc<Telemetry>,
    sample_rate: f32,
    latency: usize,
}

#[derive(Params)]
pub struct SerpentParams {
    #[persist = "editor-state"]
    pub editor_state: Arc<EguiState>,

    #[id = "thresh"]
    pub threshold: FloatParam,
    #[id = "range"]
    pub range: FloatParam,
    #[id = "freq"]
    pub freq: FloatParam,
    #[id = "width"]
    pub width: FloatParam,
    #[id = "follow"]
    pub follow: FloatParam,
    #[id = "attack"]
    pub attack: FloatParam,
    #[id = "release"]
    pub release: FloatParam,
    #[id = "look"]
    pub lookahead: FloatParam,
    #[id = "shape"]
    pub shape: EnumParam<Shape>,
    #[id = "detect"]
    pub detect: EnumParam<Detect>,
    #[id = "listen"]
    pub listen: EnumParam<Listen>,
}

impl Default for SerpentParams {
    fn default() -> Self {
        Self {
            editor_state: EguiState::from_size(760, 640),
            threshold: FloatParam::new("Threshold", -16.0, FloatRange::Linear { min: -48.0, max: 0.0 })
                .with_unit(" dB")
                .with_step_size(0.1),
            range: FloatParam::new("Range", 9.0, FloatRange::Linear { min: 0.0, max: 24.0 })
                .with_unit(" dB")
                .with_step_size(0.1),
            freq: FloatParam::new(
                "Frequency",
                7000.0,
                FloatRange::Skewed { min: 2000.0, max: 14000.0, factor: FloatRange::skew_factor(-1.0) },
            )
            .with_smoother(SmoothingStyle::Logarithmic(40.0))
            .with_value_to_string(formatters::v2s_f32_hz_then_khz(1))
            .with_string_to_value(formatters::s2v_f32_hz_then_khz()),
            width: FloatParam::new("Width", 1.2, FloatRange::Linear { min: 0.3, max: 3.0 })
                .with_unit(" oct")
                .with_step_size(0.01)
                .with_value_to_string(formatters::v2s_f32_rounded(2)),
            follow: FloatParam::new("Follow", 0.5, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_value_to_string(formatters::v2s_f32_percentage(0))
                .with_string_to_value(formatters::s2v_f32_percentage()),
            attack: FloatParam::new(
                "Attack",
                0.5,
                FloatRange::Skewed { min: 0.05, max: 20.0, factor: FloatRange::skew_factor(-2.0) },
            )
            .with_unit(" ms")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            release: FloatParam::new(
                "Release",
                80.0,
                FloatRange::Skewed { min: 10.0, max: 500.0, factor: FloatRange::skew_factor(-1.5) },
            )
            .with_unit(" ms")
            .with_value_to_string(formatters::v2s_f32_rounded(0)),
            lookahead: FloatParam::new("Lookahead", 2.0, FloatRange::Linear { min: 0.0, max: MAX_LOOKAHEAD_MS })
                .with_unit(" ms")
                .with_value_to_string(formatters::v2s_f32_rounded(1)),
            shape: EnumParam::new("Shape", Shape::Bell),
            detect: EnumParam::new("Detect", Detect::Relative),
            listen: EnumParam::new("Listen", Listen::Off),
        }
    }
}

impl Default for Serpent {
    fn default() -> Self {
        let telemetry = Arc::new(Telemetry::default());
        Self {
            params: Arc::new(SerpentParams::default()),
            engine: Engine::new(44100.0),
            analyzer: Analyzer::new(telemetry.clone()),
            telemetry,
            sample_rate: 44100.0,
            latency: 0,
        }
    }
}

impl Serpent {
    fn lookahead_samples(&self) -> usize {
        (self.params.lookahead.value() * 0.001 * self.sample_rate).round() as usize
    }
}

impl Plugin for Serpent {
    const NAME: &'static str = "Serpent De-esser";
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
        self.sample_rate = buffer_config.sample_rate;
        self.engine = Engine::new(self.sample_rate);
        self.analyzer.configure(self.sample_rate);
        self.latency = self.lookahead_samples();
        context.set_latency_samples(self.latency as u32);
        self.telemetry.sample_rate.store(self.sample_rate, std::sync::atomic::Ordering::Relaxed);
        self.telemetry.latency_samples.store(self.latency as f32, std::sync::atomic::Ordering::Relaxed);
        true
    }

    fn reset(&mut self) {
        self.engine.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        use std::sync::atomic::Ordering::Relaxed;

        let p = &self.params;
        let mut settings = Settings {
            threshold_db: p.threshold.value(),
            range_db: p.range.value(),
            freq: p.freq.value(),
            width_oct: p.width.value(),
            follow_oct: p.follow.value(),
            attack_ms: p.attack.value(),
            release_ms: p.release.value(),
            lookahead: self.lookahead_samples(),
            shape: p.shape.value(),
            detect: p.detect.value(),
            listen: p.listen.value(),
        };
        if settings.lookahead != self.latency {
            self.latency = settings.lookahead;
            context.set_latency_samples(self.latency as u32);
            self.telemetry.latency_samples.store(self.latency as f32, Relaxed);
        }

        let (mut gr_peak, mut detect_peak) = (0.0f32, -120.0f32);
        for mut frame in buffer.iter_samples() {
            settings.freq = self.params.freq.smoothed.next();
            let channels = frame.len().min(2);
            let mut input = [0.0; 2];
            for (sample, slot) in frame.iter_mut().zip(&mut input) {
                *slot = *sample;
            }
            let out = self.engine.process(input, channels, &settings);
            for (sample, y) in frame.iter_mut().zip(out) {
                *sample = y;
            }

            let mix = |v: [f32; 2]| v[..channels].iter().sum::<f32>() / channels as f32;
            self.analyzer.push(mix(self.engine.dry), mix(self.engine.wet));
            gr_peak = gr_peak.max(self.engine.gr_db);
            detect_peak = detect_peak.max(self.engine.detect_db);
        }

        self.telemetry.gr_db.store(gr_peak, Relaxed);
        self.telemetry.detect_db.store(detect_peak, Relaxed);
        self.telemetry.center_hz.store(self.engine.center_hz, Relaxed);
        ProcessStatus::Normal
    }
}

impl Vst3Plugin for Serpent {
    const VST3_CLASS_ID: [u8; 16] = *b"ClowdSerpentDsr_";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Fx, Vst3SubCategory::Dynamics];
}

nih_export_vst3!(Serpent);
