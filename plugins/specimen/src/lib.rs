use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, egui, EguiState};
use std::sync::Arc;

pub mod dsp;
pub mod presets;
pub mod ui;

#[cfg(test)]
mod tests;

use dsp::{Engine, Evolve, Meter, Settings, MAX_CHANNELS};

pub struct Specimen {
    params: Arc<SpecimenParams>,
    engine: Engine,
}

#[derive(Enum, PartialEq, Eq, Clone, Copy)]
pub enum Mode {
    #[name = "Cross"]
    Cross,
    #[name = "Resynth"]
    Resynth,
}

#[derive(Enum, PartialEq, Eq, Clone, Copy)]
pub enum EvolveMode {
    #[name = "Off"]
    Off,
    #[name = "Sine"]
    Sine,
    #[name = "Walk"]
    Walk,
}

#[derive(Params)]
pub struct SpecimenParams {
    #[persist = "editor-state"]
    pub editor_state: Arc<EguiState>,

    #[id = "pos"]
    pub position: FloatParam,
    #[id = "curve"]
    pub curve: FloatParam,
    #[id = "fade"]
    pub fade: FloatParam,
    #[id = "mode"]
    pub mode: EnumParam<Mode>,
    #[id = "scale"]
    pub scale: FloatParam,
    #[id = "shift"]
    pub shift: FloatParam,
    #[id = "detail"]
    pub detail: FloatParam,
    #[id = "evolve"]
    pub evolve: EnumParam<EvolveMode>,
    #[id = "rate"]
    pub rate: FloatParam,
    #[id = "depth"]
    pub depth: FloatParam,
    #[id = "glide"]
    pub glide: FloatParam,
    #[id = "freeze"]
    pub freeze: BoolParam,
    #[id = "mix"]
    pub mix: FloatParam,
}

impl Default for SpecimenParams {
    fn default() -> Self {
        let percent = |name: &'static str, default: f32| {
            FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_value_to_string(formatters::v2s_f32_percentage(0))
                .with_string_to_value(formatters::s2v_f32_percentage())
        };
        Self {
            editor_state: EguiState::from_size(780, 560),
            position: FloatParam::new("Position", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_value_to_string(Arc::new(|v| format!("{:.2}", v * 3.0 + 1.0))),
            curve: percent("Curve", 0.0),
            fade: percent("Fade", 0.5),
            mode: EnumParam::new("Mode", Mode::Cross),
            scale: FloatParam::new("Scale", 0.0, FloatRange::Linear { min: -1.0, max: 1.0 })
                .with_value_to_string(Arc::new(|v| format!("x{:.2}", 2f32.powf(v)))),
            shift: FloatParam::new("Shift", 0.0, FloatRange::Linear { min: -1500.0, max: 1500.0 })
                .with_unit(" Hz")
                .with_value_to_string(formatters::v2s_f32_rounded(0)),
            detail: percent("Detail", 0.5),
            evolve: EnumParam::new("Evolve", EvolveMode::Off),
            rate: FloatParam::new(
                "Rate",
                0.25,
                FloatRange::Skewed { min: 0.02, max: 4.0, factor: FloatRange::skew_factor(-2.0) },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            depth: percent("Depth", 0.5),
            glide: FloatParam::new(
                "Glide",
                120.0,
                FloatRange::Skewed { min: 5.0, max: 4000.0, factor: FloatRange::skew_factor(-2.0) },
            )
            .with_unit(" ms")
            .with_value_to_string(formatters::v2s_f32_rounded(0)),
            freeze: BoolParam::new("Freeze", false),
            mix: percent("Mix", 1.0).with_smoother(SmoothingStyle::Linear(30.0)),
        }
    }
}

impl SpecimenParams {
    fn settings(&self) -> Settings {
        Settings {
            position: self.position.value(),
            curve: self.curve.value(),
            blend: self.fade.value(),
            resynth: self.mode.value() == Mode::Resynth,
            scale_oct: self.scale.value(),
            shift_hz: self.shift.value(),
            detail: self.detail.value(),
            evolve: match self.evolve.value() {
                EvolveMode::Off => Evolve::Off,
                EvolveMode::Sine => Evolve::Sine,
                EvolveMode::Walk => Evolve::Walk,
            },
            rate_hz: self.rate.value(),
            depth: self.depth.value(),
            glide_ms: self.glide.value(),
            freeze: self.freeze.value(),
        }
    }
}

impl Default for Specimen {
    fn default() -> Self {
        Self { params: Arc::new(SpecimenParams::default()), engine: Engine::new() }
    }
}

impl Plugin for Specimen {
    const NAME: &'static str = "Specimen Spectral Freeze";
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
        let meter: Arc<Meter> = self.engine.meter.clone();
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
        // Keep captured slides if the format has not changed (hosts re-initialize often).
        let fft = dsp::fft_len(buffer_config.sample_rate);
        if self.engine.fft_len() != fft || self.engine.sample_rate() != buffer_config.sample_rate || self.engine.channels() != channels {
            self.engine.configure(buffer_config.sample_rate, channels);
        }
        context.set_latency_samples(self.engine.latency() as u32);
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
            let mut x = [0.0; MAX_CHANNELS];
            for (ch, sample) in frame.iter_mut().enumerate().take(MAX_CHANNELS) {
                x[ch] = *sample;
            }
            let y = self.engine.process(x, mix);
            for (ch, sample) in frame.iter_mut().enumerate().take(MAX_CHANNELS) {
                *sample = y[ch];
            }
        }
        ProcessStatus::Normal
    }
}

impl Vst3Plugin for Specimen {
    const VST3_CLASS_ID: [u8; 16] = *b"ClowdSpecimenFrz";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Fx, Vst3SubCategory::Filter];
}

nih_export_vst3!(Specimen);
