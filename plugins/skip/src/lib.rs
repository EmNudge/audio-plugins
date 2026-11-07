use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, egui, EguiState};
use std::sync::Arc;

pub mod dsp;
pub mod presets;
pub mod ui;

use dsp::{Division, Echo, Settings, MAX_CHANNELS, MAX_TIME_MS, MIN_TIME_MS};

/// Parameters are read in blocks of this many samples; the smoothers step across each block.
const BLOCK: usize = 32;

/// A character delay: filters, saturation, diffusion and pitch shifting in the feedback loop,
/// with wow, ducking and ping-pong. See `dsp.rs`.
struct Skip {
    params: Arc<SkipParams>,
    echo: Echo,
}

#[derive(Params)]
pub struct SkipParams {
    #[persist = "editor-state"]
    editor_state: Arc<EguiState>,

    #[id = "time"]
    pub time: FloatParam,
    #[id = "sync"]
    pub sync: BoolParam,
    #[id = "division"]
    pub division: EnumParam<Division>,
    #[id = "feedback"]
    pub feedback: FloatParam,
    #[id = "pingpong"]
    pub pingpong: FloatParam,
    #[id = "hp"]
    pub hp: FloatParam,
    #[id = "lp"]
    pub lp: FloatParam,
    #[id = "drive"]
    pub drive: FloatParam,
    #[id = "diffusion"]
    pub diffusion: FloatParam,
    #[id = "pitch"]
    pub pitch: FloatParam,
    #[id = "mod_depth"]
    pub mod_depth: FloatParam,
    #[id = "mod_rate"]
    pub mod_rate: FloatParam,
    #[id = "duck"]
    pub duck: FloatParam,
    #[id = "mix"]
    pub mix: FloatParam,
}

fn percent(name: &str, default: f32, max: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max })
        .with_smoother(SmoothingStyle::Linear(40.0))
        .with_unit(" %")
        .with_value_to_string(formatters::v2s_f32_percentage(0))
        .with_string_to_value(formatters::s2v_f32_percentage())
}

fn cutoff(name: &str, default: f32, min: f32, max: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Skewed { min, max, factor: FloatRange::skew_factor(-2.0) })
        .with_smoother(SmoothingStyle::Logarithmic(40.0))
        .with_value_to_string(formatters::v2s_f32_hz_then_khz(0))
        .with_string_to_value(formatters::s2v_f32_hz_then_khz())
}

impl Default for SkipParams {
    fn default() -> Self {
        Self {
            editor_state: EguiState::from_size(760, 560),
            // The delay slides to its target inside the DSP, so no smoother here.
            time: FloatParam::new(
                "Time",
                350.0,
                FloatRange::Skewed { min: MIN_TIME_MS, max: MAX_TIME_MS, factor: FloatRange::skew_factor(-2.5) },
            )
            .with_unit(" ms")
            .with_value_to_string(formatters::v2s_f32_rounded(0)),
            sync: BoolParam::new("Sync", false),
            division: EnumParam::new("Division", Division::DottedEighth),
            feedback: percent("Feedback", 0.5, 1.1),
            pingpong: percent("Ping-pong", 0.0, 1.0),
            hp: cutoff("Low cut", 120.0, 20.0, 2000.0),
            lp: cutoff("High cut", 6000.0, 1000.0, 20000.0),
            drive: percent("Drive", 0.2, 1.0),
            diffusion: percent("Diffusion", 0.0, 1.0),
            pitch: FloatParam::new("Pitch", 0.0, FloatRange::Linear { min: -12.0, max: 12.0 })
                .with_smoother(SmoothingStyle::Linear(60.0))
                .with_unit(" st")
                .with_value_to_string(formatters::v2s_f32_rounded(1)),
            mod_depth: percent("Wow", 0.2, 1.0),
            mod_rate: FloatParam::new(
                "Rate",
                0.6,
                FloatRange::Skewed { min: 0.1, max: 5.0, factor: FloatRange::skew_factor(-1.0) },
            )
            .with_smoother(SmoothingStyle::Linear(80.0))
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            duck: percent("Duck", 0.0, 1.0),
            mix: percent("Mix", 0.35, 1.0),
        }
    }
}

impl SkipParams {
    /// The delay time in effect: the note length at the host tempo when synced.
    pub fn time_ms(&self, tempo: Option<f64>) -> f32 {
        if self.sync.value() {
            self.division.value().time_ms(tempo.unwrap_or(120.0) as f32)
        } else {
            self.time.value()
        }
    }

    fn settings(&self, tempo: Option<f64>, steps: u32) -> Settings {
        Settings {
            time_ms: self.time_ms(tempo),
            feedback: self.feedback.smoothed.next_step(steps),
            pingpong: self.pingpong.smoothed.next_step(steps),
            hp_hz: self.hp.smoothed.next_step(steps),
            lp_hz: self.lp.smoothed.next_step(steps),
            drive: self.drive.smoothed.next_step(steps),
            diffusion: self.diffusion.smoothed.next_step(steps),
            pitch: self.pitch.smoothed.next_step(steps),
            mod_depth: self.mod_depth.smoothed.next_step(steps),
            mod_rate_hz: self.mod_rate.smoothed.next_step(steps),
            duck: self.duck.smoothed.next_step(steps),
            mix: self.mix.smoothed.next_step(steps),
        }
    }
}

impl Default for Skip {
    fn default() -> Self {
        Self { params: Arc::new(SkipParams::default()), echo: Echo::new() }
    }
}

impl Plugin for Skip {
    const NAME: &'static str = "Skip Delay";
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
        let meter = self.echo.meter.clone();
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
        self.echo.configure(buffer_config.sample_rate, channels);
        true
    }

    fn reset(&mut self) {
        self.echo.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let tempo = context.transport().tempo;
        let mut settings = self.params.settings(tempo, 0);
        for (_, mut block) in buffer.iter_blocks(BLOCK) {
            settings = self.params.settings(tempo, block.samples() as u32);
            for mut frame in block.iter_samples() {
                let mut x = [0.0; MAX_CHANNELS];
                for (ch, sample) in frame.iter_mut().enumerate().take(MAX_CHANNELS) {
                    x[ch] = *sample;
                }
                let out = self.echo.process(x, &settings);
                for (ch, sample) in frame.iter_mut().enumerate().take(MAX_CHANNELS) {
                    *sample = out[ch];
                }
            }
        }
        ProcessStatus::Tail(self.echo.tail_samples(&settings))
    }
}

impl Vst3Plugin for Skip {
    const VST3_CLASS_ID: [u8; 16] = *b"ClowdSkipStones_";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Fx, Vst3SubCategory::Delay];
}

nih_export_vst3!(Skip);
