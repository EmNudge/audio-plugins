use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, egui, EguiState};
use std::sync::Arc;

pub mod dsp;
pub mod presets;
pub mod ui;

use dsp::{Analyzer, Chain, LATENCY};

/// Corner frequencies of the two excitation bands.
pub const MID_HIGH_HZ: f32 = 3500.0;
pub const HIGH_HZ: f32 = 9000.0;
const MID_HIGH_GAIN: f32 = 1.0;
const HIGH_GAIN: f32 = 1.5;

pub struct AuroraAir {
    params: Arc<AirParams>,
    /// `[channel][band]`: band 0 is Mid High, band 1 is High.
    chains: [[Chain; 2]; 2],
    dry_delay: [[f32; 32]; 2],
    delay_pos: usize,
    analyzer: Analyzer,
}

#[derive(Params)]
pub struct AirParams {
    #[persist = "editor-state"]
    pub editor_state: Arc<EguiState>,

    #[id = "midhigh"]
    pub mid_high: FloatParam,
    #[id = "high"]
    pub high: FloatParam,
    #[id = "trim"]
    pub trim: FloatParam,
}

impl Default for AirParams {
    fn default() -> Self {
        let percent = |name: &'static str, default: f32| {
            FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(50.0))
                .with_value_to_string(formatters::v2s_f32_percentage(0))
                .with_string_to_value(formatters::s2v_f32_percentage())
        };
        Self {
            editor_state: EguiState::from_size(560, 400),
            mid_high: percent("Mid High", 0.35),
            high: percent("High", 0.5),
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

fn make_chains(fs: f32) -> [[Chain; 2]; 2] {
    let pair = || [Chain::new(fs, MID_HIGH_HZ), Chain::new(fs, HIGH_HZ)];
    [pair(), pair()]
}

impl Default for AuroraAir {
    fn default() -> Self {
        Self {
            params: Arc::new(AirParams::default()),
            chains: make_chains(44100.0),
            dry_delay: [[0.0; 32]; 2],
            delay_pos: 0,
            analyzer: Analyzer::new(),
        }
    }
}

impl Plugin for AuroraAir {
    const NAME: &'static str = "Aurora Air Exciter";
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
        let spectrum = self.analyzer.spectrum.clone();
        create_egui_editor(
            self.params.editor_state.clone(),
            ui::UiState::default(),
            |_, _| {},
            move |ctx, setter, state| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE.fill(ui::background()).inner_margin(14.0))
                    .show(ctx, |ui| ui::draw(ui, &params, setter, &spectrum, state));
            },
        )
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        context: &mut impl InitContext<Self>,
    ) -> bool {
        self.chains = make_chains(buffer_config.sample_rate);
        self.analyzer.configure(buffer_config.sample_rate);
        context.set_latency_samples(LATENCY as u32);
        true
    }

    fn reset(&mut self) {
        self.dry_delay = [[0.0; 32]; 2];
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        for mut frame in buffer.iter_samples() {
            let mid_high = self.params.mid_high.smoothed.next() * MID_HIGH_GAIN;
            let high = self.params.high.smoothed.next() * HIGH_GAIN;
            let trim = self.params.trim.smoothed.next();

            let channels = frame.len();
            let (mut dry_sum, mut wet_sum) = (0.0, 0.0);
            for (ch, sample) in frame.iter_mut().enumerate() {
                let x = *sample;
                // Run both chains even at zero amount so their filter state stays warm.
                let wet = self.chains[ch][0].process(x) * mid_high + self.chains[ch][1].process(x) * high;

                self.dry_delay[ch][self.delay_pos] = x;
                let dry = self.dry_delay[ch][(self.delay_pos + 32 - LATENCY) & 31];

                *sample = (dry + wet) * trim;
                dry_sum += dry;
                wet_sum += wet;
            }
            self.delay_pos = (self.delay_pos + 1) & 31;
            self.analyzer.push(dry_sum / channels as f32, wet_sum / channels as f32);
        }
        ProcessStatus::Normal
    }
}

impl Vst3Plugin for AuroraAir {
    const VST3_CLASS_ID: [u8; 16] = *b"ClowdAuroraAirF_";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Fx, Vst3SubCategory::Filter];
}

nih_export_vst3!(AuroraAir);
