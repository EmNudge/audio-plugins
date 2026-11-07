use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, egui, EguiState};
use std::sync::Arc;

pub mod dsp;
pub mod presets;
pub mod ui;

use dsp::{Resynth, Settings, LATENCY, MAX_CHANNELS, MAX_PARTIALS};

pub struct Abacus {
    params: Arc<AbacusParams>,
    resynth: Resynth,
}

#[derive(Params)]
pub struct AbacusParams {
    #[persist = "editor-state"]
    pub editor_state: Arc<EguiState>,

    #[id = "partials"]
    pub partials: IntParam,
    #[id = "ffreeze"]
    pub freeze_freq: BoolParam,
    #[id = "afreeze"]
    pub freeze_amp: BoolParam,
    #[id = "res"]
    pub resolution: FloatParam,
    #[id = "glide"]
    pub glide: FloatParam,
    #[id = "pitch"]
    pub pitch: FloatParam,
    #[id = "stretch"]
    pub stretch: FloatParam,
    #[id = "noise"]
    pub noise: FloatParam,
    #[id = "spread"]
    pub spread: FloatParam,
    #[id = "mix"]
    pub mix: FloatParam,
}

impl Default for AbacusParams {
    fn default() -> Self {
        let percent = |name: &'static str, default: f32| {
            FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_value_to_string(formatters::v2s_f32_percentage(0))
                .with_string_to_value(formatters::s2v_f32_percentage())
        };
        Self {
            editor_state: EguiState::from_size(820, 600),
            partials: IntParam::new("Partials", 16, IntRange::Linear { min: 1, max: MAX_PARTIALS as i32 }),
            freeze_freq: BoolParam::new("Freeze Freq", false),
            freeze_amp: BoolParam::new("Freeze Amp", false),
            resolution: percent("Resolution", 1.0),
            glide: FloatParam::new("Glide", 80.0, FloatRange::Skewed { min: 0.0, max: 2000.0, factor: FloatRange::skew_factor(-2.0) })
                .with_unit(" ms")
                .with_value_to_string(formatters::v2s_f32_rounded(0)),
            pitch: FloatParam::new("Pitch", 0.0, FloatRange::Linear { min: -24.0, max: 24.0 })
                .with_unit(" st")
                .with_value_to_string(formatters::v2s_f32_rounded(2)),
            stretch: FloatParam::new("Stretch", 0.0, FloatRange::Linear { min: -1.0, max: 1.0 })
                .with_value_to_string(formatters::v2s_f32_rounded(2)),
            noise: percent("Noise", 0.0),
            spread: percent("Spread", 0.0),
            mix: percent("Mix", 1.0).with_smoother(SmoothingStyle::Linear(30.0)),
        }
    }
}

impl AbacusParams {
    fn settings(&self) -> Settings {
        Settings {
            partials: self.partials.value() as usize,
            freeze_freq: self.freeze_freq.value(),
            freeze_amp: self.freeze_amp.value(),
            resolution: self.resolution.value(),
            glide_ms: self.glide.value(),
            pitch_semitones: self.pitch.value(),
            stretch: self.stretch.value(),
            noise: self.noise.value(),
            spread: self.spread.value(),
        }
    }
}

impl Default for Abacus {
    fn default() -> Self {
        Self { params: Arc::new(AbacusParams::default()), resynth: Resynth::new() }
    }
}

impl Plugin for Abacus {
    const NAME: &'static str = "Abacus Sine Resynth";
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
        let meter = self.resynth.meter.clone();
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
        self.resynth.configure(buffer_config.sample_rate, channels);
        context.set_latency_samples(LATENCY as u32);
        true
    }

    fn reset(&mut self) {
        self.resynth.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        self.resynth.settings = self.params.settings();

        for mut frame in buffer.iter_samples() {
            let mix = self.params.mix.smoothed.next();
            let mut x = [0.0; MAX_CHANNELS];
            for (ch, sample) in frame.iter_mut().enumerate().take(MAX_CHANNELS) {
                x[ch] = *sample;
            }
            let (wet, dry) = self.resynth.process(x);
            for (ch, sample) in frame.iter_mut().enumerate().take(MAX_CHANNELS) {
                *sample = dry[ch] + (wet[ch] - dry[ch]) * mix;
            }
        }
        ProcessStatus::Normal
    }
}

impl Vst3Plugin for Abacus {
    const VST3_CLASS_ID: [u8; 16] = *b"ClowdAbacusSines";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Fx, Vst3SubCategory::Synth];
}

nih_export_vst3!(Abacus);
