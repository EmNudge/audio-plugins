use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, egui, EguiState};
use std::sync::Arc;

pub mod dsp;
pub mod presets;
pub mod ui;

use dsp::{HeadSettings, Loom as Engine, Settings, FULL_WIDTH, HEADS, LATENCY, MAX_DELAY_MS, MAX_FEEDBACK, MAX_SHIFT, MIN_WIDTH, OFF_DB};

pub struct Loom {
    params: Arc<LoomParams>,
    engine: Engine,
}

#[derive(Params)]
pub struct HeadParams {
    #[id = "delay"]
    pub delay: FloatParam,
    #[id = "shift"]
    pub shift: FloatParam,
    #[id = "slide"]
    pub slide: FloatParam,
    #[id = "centre"]
    pub centre: FloatParam,
    #[id = "width"]
    pub width: FloatParam,
    #[id = "level"]
    pub level: FloatParam,
    #[id = "pan"]
    pub pan: FloatParam,
}

impl HeadParams {
    fn new(index: usize, get: &dyn Fn(&str) -> Option<f32>) -> Self {
        let n = index + 1;
        let v = |id: &str, default: f32| get(&format!("{id}_{n}")).unwrap_or(default);
        Self {
            // Later heads start further back so switching one on is audible straight away.
            delay: FloatParam::new(
                "Delay",
                v("delay", 400.0 * n as f32),
                FloatRange::Skewed { min: 0.0, max: MAX_DELAY_MS, factor: FloatRange::skew_factor(-1.0) },
            )
            .with_unit(" ms")
            .with_value_to_string(formatters::v2s_f32_rounded(0)),
            shift: FloatParam::new("Shift", v("shift", 0.0), FloatRange::Linear { min: -MAX_SHIFT, max: MAX_SHIFT })
                .with_unit(" st")
                .with_value_to_string(Arc::new(|v| format!("{v:+.1}"))),
            slide: FloatParam::new("Slide", v("slide", 0.0), FloatRange::Linear { min: -24.0, max: 24.0 })
                .with_unit(" st/s")
                .with_value_to_string(Arc::new(|v| format!("{v:+.1}"))),
            centre: FloatParam::new(
                "Centre",
                v("centre", 1000.0),
                FloatRange::Skewed { min: 40.0, max: 12000.0, factor: FloatRange::skew_factor(-2.0) },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_rounded(0)),
            width: FloatParam::new("Width", v("width", FULL_WIDTH), FloatRange::Linear { min: MIN_WIDTH, max: FULL_WIDTH })
                .with_value_to_string(Arc::new(|v| if v >= FULL_WIDTH - 0.01 { "full".into() } else { format!("{v:.2} oct") }))
                .with_string_to_value(Arc::new(|s| if s.trim().starts_with("full") { Some(FULL_WIDTH) } else { s.trim().trim_end_matches("oct").trim().parse().ok() })),
            level: FloatParam::new(
                "Level",
                v("level", if index == 0 { 0.0 } else { OFF_DB }),
                FloatRange::Linear { min: OFF_DB, max: 6.0 },
            )
            .with_value_to_string(Arc::new(|v| if v <= OFF_DB + 0.05 { "off".into() } else { format!("{v:.1} dB") }))
            .with_string_to_value(Arc::new(|s| if s.trim() == "off" { Some(OFF_DB) } else { s.trim().trim_end_matches("dB").trim().parse().ok() })),
            pan: FloatParam::new("Pan", v("pan", 0.0), FloatRange::Linear { min: -1.0, max: 1.0 })
                .with_value_to_string(Arc::new(|v| if v.abs() < 0.005 { "C".into() } else if v < 0.0 { format!("L{:.0}", -v * 100.0) } else { format!("R{:.0}", v * 100.0) })),
        }
    }

    pub fn settings(&self) -> HeadSettings {
        HeadSettings {
            delay_ms: self.delay.value(),
            shift: self.shift.value(),
            slide: self.slide.value(),
            centre_hz: self.centre.value(),
            width_oct: self.width.value(),
            level_db: self.level.value(),
            pan: self.pan.value(),
        }
    }

    pub fn is_on(&self) -> bool {
        self.level.value() > OFF_DB + 0.05
    }
}

#[derive(Params)]
pub struct LoomParams {
    #[persist = "editor-state"]
    pub editor_state: Arc<EguiState>,

    #[nested(array, group = "Head")]
    pub heads: [HeadParams; HEADS],

    #[id = "mix"]
    pub mix: FloatParam,
    #[id = "fb"]
    pub feedback: FloatParam,
    #[id = "smooth"]
    pub smooth: FloatParam,
}

impl Default for LoomParams {
    fn default() -> Self {
        Self::with_values(&|_| None)
    }
}

impl LoomParams {
    /// Parameters whose defaults come from `get(id)` where it has a value, e.g. a preset's table.
    /// Used by the screenshot example; hosts only ever see `Default`.
    pub fn with_values(get: &dyn Fn(&str) -> Option<f32>) -> Self {
        let v = |id: &str, default: f32| get(id).unwrap_or(default);
        Self {
            editor_state: EguiState::from_size(920, 660),
            heads: std::array::from_fn(|i| HeadParams::new(i, get)),
            mix: FloatParam::new("Mix", v("mix", 0.5), FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(30.0))
                .with_value_to_string(formatters::v2s_f32_percentage(0))
                .with_string_to_value(formatters::s2v_f32_percentage()),
            feedback: FloatParam::new("Feedback", v("fb", 0.0), FloatRange::Linear { min: 0.0, max: MAX_FEEDBACK })
                .with_value_to_string(formatters::v2s_f32_percentage(0))
                .with_string_to_value(formatters::s2v_f32_percentage()),
            smooth: FloatParam::new("Smooth", v("smooth", 120.0), FloatRange::Skewed { min: 0.0, max: 2000.0, factor: FloatRange::skew_factor(-2.0) })
                .with_unit(" ms")
                .with_value_to_string(formatters::v2s_f32_rounded(0)),
        }
    }
}

impl LoomParams {
    fn settings(&self) -> Settings {
        Settings {
            heads: std::array::from_fn(|i| self.heads[i].settings()),
            feedback: self.feedback.value(),
            smooth_ms: self.smooth.value(),
        }
    }
}

impl Default for Loom {
    fn default() -> Self {
        Self { params: Arc::new(LoomParams::default()), engine: Engine::new() }
    }
}

impl Plugin for Loom {
    const NAME: &'static str = "Loom Spectral Playheads";
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
        let meter = self.engine.meter.clone();
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
        self.engine.configure(buffer_config.sample_rate, channels);
        context.set_latency_samples(LATENCY as u32);
        true
    }

    fn reset(&mut self) {
        self.engine.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        self.engine.settings = self.params.settings();

        for mut frame in buffer.iter_samples() {
            let mix = self.params.mix.smoothed.next();
            let mut x = [0.0; dsp::MAX_CHANNELS];
            for (ch, sample) in frame.iter_mut().enumerate().take(dsp::MAX_CHANNELS) {
                x[ch] = *sample;
            }
            let (wet, dry) = self.engine.process(x);
            for (ch, sample) in frame.iter_mut().enumerate().take(dsp::MAX_CHANNELS) {
                *sample = dsp::mix(wet[ch], dry[ch], mix);
            }
        }
        ProcessStatus::Normal
    }
}

impl Vst3Plugin for Loom {
    const VST3_CLASS_ID: [u8; 16] = *b"ClowdLoomPlayhds";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Fx, Vst3SubCategory::Delay];
}

nih_export_vst3!(Loom);
