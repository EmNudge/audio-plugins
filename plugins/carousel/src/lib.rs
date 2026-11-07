use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, egui, EguiState};
use std::sync::Arc;

pub mod dsp;
pub mod presets;
pub mod ui;

use dsp::{synced_rate_hz, Chorus, Division, Mode, Settings, Shape, MAX_CHANNELS, MAX_DELAY_MS, MIN_DELAY_MS};

/// A multi-voice chorus with analog voicing. See `dsp.rs`.
///
/// It reports no latency: only the wet voices are delayed, and the dry signal is deliberately left
/// alone because the interference between the two is the effect.
struct Carousel {
    params: Arc<CarouselParams>,
    chorus: Chorus,
}

#[derive(Params)]
pub struct CarouselParams {
    #[persist = "editor-state"]
    editor_state: Arc<EguiState>,

    #[id = "mode"]
    mode: EnumParam<Mode>,
    #[id = "rate"]
    rate: FloatParam,
    #[id = "sync"]
    sync: EnumParam<Division>,
    #[id = "depth"]
    depth: FloatParam,
    #[id = "delay"]
    delay: FloatParam,
    #[id = "voices"]
    voices: IntParam,
    #[id = "shape"]
    shape: EnumParam<Shape>,
    #[id = "spread"]
    spread: FloatParam,
    #[id = "analog"]
    analog: FloatParam,
    #[id = "mix"]
    mix: FloatParam,
}

fn percent(name: &str, default: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
        .with_smoother(SmoothingStyle::Linear(50.0))
        .with_unit(" %")
        .with_value_to_string(formatters::v2s_f32_percentage(0))
        .with_string_to_value(formatters::s2v_f32_percentage())
}

impl Default for CarouselParams {
    fn default() -> Self {
        let defaults = Settings::default();
        Self {
            editor_state: EguiState::from_size(800, 580),
            mode: EnumParam::new("Mode", defaults.mode),
            rate: FloatParam::new(
                "Rate",
                defaults.rate_hz,
                FloatRange::Skewed { min: 0.05, max: 10.0, factor: FloatRange::skew_factor(-2.0) },
            )
            .with_smoother(SmoothingStyle::Linear(80.0))
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            sync: EnumParam::new("Sync", Division::Off),
            depth: percent("Depth", defaults.depth),
            // Moving the delay bends the pitch while it moves, so it is smoothed.
            delay: FloatParam::new(
                "Delay",
                defaults.delay_ms,
                FloatRange::Skewed { min: MIN_DELAY_MS, max: MAX_DELAY_MS, factor: FloatRange::skew_factor(-1.0) },
            )
            .with_smoother(SmoothingStyle::Linear(80.0))
            .with_unit(" ms")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),
            voices: IntParam::new("Voices", defaults.voices as i32, IntRange::Linear { min: 1, max: 3 }),
            shape: EnumParam::new("Shape", defaults.shape),
            spread: percent("Spread", defaults.spread),
            analog: percent("Analog", defaults.analog),
            mix: percent("Mix", defaults.mix),
        }
    }
}

impl CarouselParams {
    /// The current values with no smoothing and no tempo sync, for the editor.
    fn settings(&self) -> Settings {
        Settings {
            mode: self.mode.value(),
            rate_hz: self.rate.value(),
            depth: self.depth.value(),
            delay_ms: self.delay.value(),
            voices: self.voices.value() as usize,
            shape: self.shape.value(),
            spread: self.spread.value(),
            analog: self.analog.value(),
            mix: self.mix.value(),
        }
    }
}

impl Default for Carousel {
    fn default() -> Self {
        Self { params: Arc::new(CarouselParams::default()), chorus: Chorus::new() }
    }
}

impl Plugin for Carousel {
    const NAME: &'static str = "Carousel Chorus";
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
        let meter = self.chorus.meter.clone();
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
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        let channels = audio_io_layout.main_input_channels.map_or(2, |c| c.get() as usize);
        self.chorus.configure(buffer_config.sample_rate, channels);
        true
    }

    fn reset(&mut self) {
        self.chorus.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let p = &self.params;
        let tempo = context.transport().tempo;
        let division = p.sync.value();
        for mut frame in buffer.iter_samples() {
            let mode = p.mode.value();
            let mut rate_hz = p.rate.smoothed.next();
            if mode.free_rate() {
                if let Some(synced) = tempo.and_then(|bpm| synced_rate_hz(bpm, division)) {
                    rate_hz = synced;
                }
            }
            let settings = Settings {
                mode,
                rate_hz,
                depth: p.depth.smoothed.next(),
                delay_ms: p.delay.smoothed.next(),
                voices: p.voices.value() as usize,
                shape: p.shape.value(),
                spread: p.spread.smoothed.next(),
                analog: p.analog.smoothed.next(),
                mix: p.mix.smoothed.next(),
            };
            let mut x = [0.0; MAX_CHANNELS];
            for (ch, sample) in frame.iter_mut().enumerate().take(MAX_CHANNELS) {
                x[ch] = *sample;
            }
            let out = self.chorus.process(x, &settings);
            for (ch, sample) in frame.iter_mut().enumerate().take(MAX_CHANNELS) {
                *sample = out[ch];
            }
        }
        ProcessStatus::Normal
    }
}

impl Vst3Plugin for Carousel {
    const VST3_CLASS_ID: [u8; 16] = *b"ClowdCarouselCh_";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Modulation];
}

nih_export_vst3!(Carousel);
