use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, egui, EguiState};
use std::sync::Arc;

pub mod dsp;
pub mod presets;
pub mod ui;

use dsp::{Settings, Squeeze, MAX_CHANNELS};

/// Three-band upward and downward compression. See `dsp.rs`.
struct Vice {
    params: Arc<ViceParams>,
    squeeze: Squeeze,
}

#[derive(Params)]
pub struct ViceParams {
    #[persist = "editor-state"]
    editor_state: Arc<EguiState>,

    #[id = "in"]
    input: FloatParam,
    #[id = "out"]
    output: FloatParam,
    #[id = "depth"]
    depth: FloatParam,
    #[id = "up"]
    upward: FloatParam,
    #[id = "down"]
    downward: FloatParam,
    #[id = "time"]
    time: FloatParam,
    #[id = "floor"]
    floor: FloatParam,
    #[id = "ceil"]
    ceil: FloatParam,
    #[id = "g_low"]
    gain_low: FloatParam,
    #[id = "g_mid"]
    gain_mid: FloatParam,
    #[id = "g_high"]
    gain_high: FloatParam,
    #[id = "mix"]
    mix: FloatParam,
    #[id = "x_low"]
    cross_low: FloatParam,
    #[id = "x_high"]
    cross_high: FloatParam,
}

fn percent(name: &str, default: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
        .with_smoother(SmoothingStyle::Linear(30.0))
        .with_unit(" %")
        .with_value_to_string(formatters::v2s_f32_percentage(0))
        .with_string_to_value(formatters::s2v_f32_percentage())
}

fn decibels(name: &str, default: f32, min: f32, max: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min, max })
        .with_smoother(SmoothingStyle::Linear(30.0))
        .with_unit(" dB")
        .with_value_to_string(formatters::v2s_f32_rounded(1))
}

fn crossover(name: &str, default: f32, min: f32, max: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Skewed { min, max, factor: FloatRange::skew_factor(-1.0) })
        .with_smoother(SmoothingStyle::Logarithmic(50.0))
        .with_unit(" Hz")
        .with_value_to_string(formatters::v2s_f32_hz_then_khz(0))
        .with_string_to_value(formatters::s2v_f32_hz_then_khz())
}

impl Default for ViceParams {
    fn default() -> Self {
        let d = Settings::default();
        Self {
            editor_state: EguiState::from_size(760, 540),
            input: decibels("Input", d.in_db, -12.0, 24.0),
            output: decibels("Output", d.out_db, -24.0, 12.0),
            depth: percent("Depth", d.depth),
            upward: percent("Upward", d.upward),
            downward: percent("Downward", d.downward),
            time: percent("Time", d.time),
            floor: decibels("Floor", d.floor_db, -60.0, -10.0),
            ceil: decibels("Ceiling", d.ceil_db, -40.0, 0.0),
            gain_low: decibels("Low gain", 0.0, -12.0, 12.0),
            gain_mid: decibels("Mid gain", 0.0, -12.0, 12.0),
            gain_high: decibels("High gain", 0.0, -12.0, 12.0),
            mix: percent("Mix", d.mix),
            cross_low: crossover("Low crossover", d.low_hz, 40.0, 400.0),
            cross_high: crossover("High crossover", d.high_hz, 1000.0, 10000.0),
        }
    }
}

impl Default for Vice {
    fn default() -> Self {
        Self { params: Arc::new(ViceParams::default()), squeeze: Squeeze::new() }
    }
}

impl Plugin for Vice {
    const NAME: &'static str = "Vice";
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
        let meter = self.squeeze.meter.clone();
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
        self.squeeze.configure(buffer_config.sample_rate, channels);
        true
    }

    fn reset(&mut self) {
        self.squeeze.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let p = &self.params;
        let frames = buffer.samples();
        for mut frame in buffer.iter_samples() {
            let settings = Settings {
                in_db: p.input.smoothed.next(),
                out_db: p.output.smoothed.next(),
                depth: p.depth.smoothed.next(),
                upward: p.upward.smoothed.next(),
                downward: p.downward.smoothed.next(),
                time: p.time.smoothed.next(),
                floor_db: p.floor.smoothed.next(),
                ceil_db: p.ceil.smoothed.next(),
                band_db: [p.gain_low.smoothed.next(), p.gain_mid.smoothed.next(), p.gain_high.smoothed.next()],
                mix: p.mix.smoothed.next(),
                low_hz: p.cross_low.smoothed.next(),
                high_hz: p.cross_high.smoothed.next(),
            };
            let mut x = [0.0; MAX_CHANNELS];
            for (ch, sample) in frame.iter_mut().enumerate().take(MAX_CHANNELS) {
                x[ch] = *sample;
            }
            let out = self.squeeze.process(x, &settings);
            for (ch, sample) in frame.iter_mut().enumerate().take(MAX_CHANNELS) {
                *sample = out[ch];
            }
        }
        self.squeeze.publish(frames);
        ProcessStatus::Normal
    }
}

impl Vst3Plugin for Vice {
    const VST3_CLASS_ID: [u8; 16] = *b"CalvinViceSqueez";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Fx, Vst3SubCategory::Dynamics];
}

nih_export_vst3!(Vice);
