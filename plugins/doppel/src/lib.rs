use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, egui, EguiState};
use std::sync::Arc;

pub mod dsp;
pub mod presets;
pub mod ui;

use dsp::{Doubler, Settings, MAX_CHANNELS, MAX_TIME_MS, MIN_TIME_MS};

/// Below this the level knob means "off" (the bottom of its range is -60 dB).
const LEVEL_OFF_DB: f32 = -59.5;

/// Automatic double tracking: a copy of the input that lags it and keeps drifting, like a second
/// take on a tape machine. See `dsp.rs`.
struct Doppel {
    params: Arc<DoppelParams>,
    doubler: Doubler,
}

#[derive(Params)]
pub struct DoppelParams {
    #[persist = "editor-state"]
    editor_state: Arc<EguiState>,

    #[id = "time"]
    time: FloatParam,
    #[id = "drift"]
    drift: FloatParam,
    #[id = "rate"]
    rate: FloatParam,
    #[id = "tape"]
    tape: FloatParam,
    #[id = "width"]
    width: FloatParam,
    #[id = "level"]
    level: FloatParam,
}

fn percent(name: &str, default: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
        .with_smoother(SmoothingStyle::Linear(50.0))
        .with_unit(" %")
        .with_value_to_string(formatters::v2s_f32_percentage(0))
        .with_string_to_value(formatters::s2v_f32_percentage())
}

impl Default for DoppelParams {
    fn default() -> Self {
        Self {
            editor_state: EguiState::from_size(720, 500),
            // Moving the time bends the pitch of the copy, as turning a tape machine's speed does,
            // so it is smoothed slowly.
            time: FloatParam::new("Time", 45.0, FloatRange::Linear { min: MIN_TIME_MS, max: MAX_TIME_MS })
                .with_smoother(SmoothingStyle::Linear(120.0))
                .with_unit(" ms")
                .with_value_to_string(formatters::v2s_f32_rounded(1)),
            drift: percent("Drift", 0.35),
            rate: FloatParam::new(
                "Rate",
                0.7,
                FloatRange::Skewed { min: 0.1, max: 4.0, factor: FloatRange::skew_factor(-1.0) },
            )
            .with_smoother(SmoothingStyle::Linear(80.0))
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            tape: percent("Tape", 0.4),
            width: percent("Width", 0.5),
            level: FloatParam::new(
                "Double",
                util::db_to_gain(-3.0),
                FloatRange::Skewed {
                    min: util::db_to_gain(-60.0),
                    max: util::db_to_gain(6.0),
                    factor: FloatRange::gain_skew_factor(-60.0, 6.0),
                },
            )
            .with_smoother(SmoothingStyle::Logarithmic(50.0))
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_gain_to_db(1))
            .with_string_to_value(formatters::s2v_f32_gain_to_db()),
        }
    }
}

impl Default for Doppel {
    fn default() -> Self {
        Self { params: Arc::new(DoppelParams::default()), doubler: Doubler::new() }
    }
}

impl Plugin for Doppel {
    const NAME: &'static str = "Doppel Doubler";
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
        let meter = self.doubler.meter.clone();
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
        self.doubler.configure(buffer_config.sample_rate, channels);
        true
    }

    fn reset(&mut self) {
        self.doubler.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let p = &self.params;
        let off = util::db_to_gain(LEVEL_OFF_DB);
        for mut frame in buffer.iter_samples() {
            let level = p.level.smoothed.next();
            let settings = Settings {
                time_ms: p.time.smoothed.next(),
                drift: p.drift.smoothed.next(),
                rate_hz: p.rate.smoothed.next(),
                tape: p.tape.smoothed.next(),
                width: p.width.smoothed.next(),
                level: if level < off { 0.0 } else { level },
            };
            let mut x = [0.0; MAX_CHANNELS];
            for (ch, sample) in frame.iter_mut().enumerate().take(MAX_CHANNELS) {
                x[ch] = *sample;
            }
            let out = self.doubler.process(x, &settings);
            for (ch, sample) in frame.iter_mut().enumerate().take(MAX_CHANNELS) {
                *sample = out[ch];
            }
        }
        ProcessStatus::Normal
    }
}

impl Vst3Plugin for Doppel {
    const VST3_CLASS_ID: [u8; 16] = *b"ClowdDoppelTwin_";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Modulation];
}

nih_export_vst3!(Doppel);
