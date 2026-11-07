use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, egui, EguiState};
use std::sync::Arc;

pub mod dsp;
pub mod presets;
pub mod ui;
#[cfg(test)]
mod preview;

use dsp::{Reverb, Settings, Space, MAX_CHANNELS, MAX_DECAY_S, MIN_DECAY_S};

/// Parameters are read in blocks of this many samples; the smoothers step across each block.
const BLOCK: usize = 32;

/// An algorithmic reverb: early reflections and a modulated 8-line network with a three-band decay
/// filter and an optional frequency shifter in the loop. See `dsp.rs`.
struct Vault {
    params: Arc<VaultParams>,
    reverb: Reverb,
}

#[derive(Params)]
pub struct VaultParams {
    #[persist = "editor-state"]
    editor_state: Arc<EguiState>,

    #[id = "space"]
    pub space: EnumParam<Space>,
    #[id = "size"]
    pub size: FloatParam,
    #[id = "predelay"]
    pub predelay: FloatParam,
    #[id = "decay"]
    pub decay: FloatParam,
    #[id = "low_mult"]
    pub low_mult: FloatParam,
    #[id = "high_mult"]
    pub high_mult: FloatParam,
    #[id = "xo_low"]
    pub xo_low: FloatParam,
    #[id = "xo_high"]
    pub xo_high: FloatParam,
    #[id = "diffusion"]
    pub diffusion: FloatParam,
    #[id = "mod_depth"]
    pub mod_depth: FloatParam,
    #[id = "mod_rate"]
    pub mod_rate: FloatParam,
    #[id = "shift"]
    pub shift: FloatParam,
    #[id = "early"]
    pub early: FloatParam,
    #[id = "width"]
    pub width: FloatParam,
    #[id = "duck"]
    pub duck: FloatParam,
    #[id = "low_cut"]
    pub low_cut: FloatParam,
    #[id = "high_cut"]
    pub high_cut: FloatParam,
    #[id = "mix"]
    pub mix: FloatParam,
    #[id = "freeze"]
    pub freeze: BoolParam,
}

fn percent(name: &str, default: f32, max: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max })
        .with_smoother(SmoothingStyle::Linear(50.0))
        .with_unit(" %")
        .with_value_to_string(formatters::v2s_f32_percentage(0))
        .with_string_to_value(formatters::s2v_f32_percentage())
}

/// A frequency in Hz, shown as `1.5k` above a thousand.
fn hertz(name: &str, default: f32, min: f32, max: f32, skew: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Skewed { min, max, factor: FloatRange::skew_factor(skew) })
        .with_smoother(SmoothingStyle::Logarithmic(50.0))
        .with_value_to_string(Arc::new(|v| if v >= 1000.0 { format!("{:.1}k Hz", v / 1000.0) } else { format!("{v:.0} Hz") }))
}

fn multiplier(name: &str, default: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 0.1, max: 2.0 })
        .with_smoother(SmoothingStyle::Linear(50.0))
        .with_value_to_string(Arc::new(|v| format!("x{v:.2}")))
}

impl Default for VaultParams {
    fn default() -> Self {
        Self::new(&Settings::default())
    }
}

impl VaultParams {
    /// Parameters starting at the given settings.
    fn new(d: &Settings) -> Self {
        Self {
            editor_state: EguiState::from_size(760, 620),
            space: EnumParam::new("Space", d.space),
            size: percent("Size", d.size, 1.0),
            predelay: FloatParam::new(
                "Pre-delay",
                d.predelay_ms,
                FloatRange::Skewed { min: 0.0, max: 300.0, factor: FloatRange::skew_factor(-2.0) },
            )
            .with_smoother(SmoothingStyle::Linear(100.0))
            .with_unit(" ms")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),
            decay: FloatParam::new(
                "Decay",
                d.decay_s,
                FloatRange::Skewed { min: MIN_DECAY_S, max: MAX_DECAY_S, factor: FloatRange::skew_factor(-1.8) },
            )
            .with_smoother(SmoothingStyle::Logarithmic(60.0))
            .with_unit(" s")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            low_mult: multiplier("Low decay", d.low_mult),
            high_mult: multiplier("High decay", d.high_mult),
            xo_low: hertz("Low crossover", d.xo_low, 100.0, 1000.0, -1.0),
            xo_high: hertz("High crossover", d.xo_high, 1500.0, 12000.0, -1.0),
            diffusion: percent("Diffusion", d.diffusion, 1.0),
            mod_depth: percent("Modulation", d.mod_depth, 1.0),
            mod_rate: FloatParam::new(
                "Mod rate",
                d.mod_rate,
                FloatRange::Skewed { min: 0.05, max: 5.0, factor: FloatRange::skew_factor(-1.5) },
            )
            .with_smoother(SmoothingStyle::Linear(50.0))
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            shift: FloatParam::new(
                "Shift",
                d.shift_hz,
                FloatRange::SymmetricalSkewed { min: -500.0, max: 500.0, factor: FloatRange::skew_factor(-2.0), center: 0.0 },
            )
            .with_smoother(SmoothingStyle::Linear(80.0))
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),
            early: percent("Early", d.early, 1.0),
            width: percent("Width", d.width, 1.5),
            duck: percent("Duck", d.duck, 1.0),
            low_cut: hertz("Low cut", d.low_cut, 20.0, 1000.0, -2.0).with_value_to_string(Arc::new(|v| {
                if v <= 21.0 { "off".to_owned() } else { format!("{v:.0} Hz") }
            })),
            high_cut: hertz("High cut", d.high_cut, 1000.0, 20000.0, -1.0).with_value_to_string(Arc::new(|v| {
                if v >= 19500.0 { "off".to_owned() } else { format!("{:.1}k Hz", v / 1000.0) }
            })),
            mix: percent("Mix", d.mix, 1.0),
            freeze: BoolParam::new("Freeze", d.freeze),
        }
    }
}

impl Default for Vault {
    fn default() -> Self {
        Self { params: Arc::new(VaultParams::default()), reverb: Reverb::new() }
    }
}

impl VaultParams {
    /// The settings at the end of a block of `n` samples, stepping every smoother across it.
    fn settings(&self, n: u32) -> Settings {
        Settings {
            space: self.space.value(),
            size: self.size.smoothed.next_step(n),
            predelay_ms: self.predelay.smoothed.next_step(n),
            decay_s: self.decay.smoothed.next_step(n),
            low_mult: self.low_mult.smoothed.next_step(n),
            high_mult: self.high_mult.smoothed.next_step(n),
            xo_low: self.xo_low.smoothed.next_step(n),
            xo_high: self.xo_high.smoothed.next_step(n),
            diffusion: self.diffusion.smoothed.next_step(n),
            mod_depth: self.mod_depth.smoothed.next_step(n),
            mod_rate: self.mod_rate.smoothed.next_step(n),
            shift_hz: self.shift.smoothed.next_step(n),
            early: self.early.smoothed.next_step(n),
            width: self.width.smoothed.next_step(n),
            duck: self.duck.smoothed.next_step(n),
            low_cut: self.low_cut.smoothed.next_step(n),
            high_cut: self.high_cut.smoothed.next_step(n),
            mix: self.mix.smoothed.next_step(n),
            freeze: self.freeze.value(),
        }
    }
}

impl Plugin for Vault {
    const NAME: &'static str = "Vault Reverb";
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
        let meter = self.reverb.meter.clone();
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
        self.reverb.configure(buffer_config.sample_rate, channels);
        true
    }

    fn reset(&mut self) {
        self.reverb.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let mut settings = self.params.settings(0);
        for (_, mut block) in buffer.iter_blocks(BLOCK) {
            settings = self.params.settings(block.samples() as u32);
            self.reverb.set(&settings);
            for mut frame in block.iter_samples() {
                let mut x = [0.0; MAX_CHANNELS];
                for (ch, sample) in frame.iter_mut().enumerate().take(MAX_CHANNELS) {
                    x[ch] = *sample;
                }
                let out = self.reverb.process(x);
                for (ch, sample) in frame.iter_mut().enumerate().take(MAX_CHANNELS) {
                    *sample = out[ch];
                }
            }
        }
        if settings.freeze {
            ProcessStatus::KeepAlive
        } else {
            ProcessStatus::Tail(self.reverb.tail_samples(&settings))
        }
    }
}

impl Vst3Plugin for Vault {
    const VST3_CLASS_ID: [u8; 16] = *b"ClowdVaultRever_";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Fx, Vst3SubCategory::Reverb];
}

nih_export_vst3!(Vault);
