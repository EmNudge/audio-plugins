use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, egui, EguiState};
use std::f32::consts::{LN_2, TAU};
use std::sync::atomic::Ordering;
use std::sync::Arc;

mod ui;

const MAX_DELAY_MS: f32 = 25.0;
const MAX_CHANNELS: usize = 2;

/// A vibrato is a pitch wobble. Sweeping a delay line's length with a sine LFO changes the pitch of
/// the delayed signal by `-d(delay)/dt`, so `depth` is specified in cents and converted to a delay
/// excursion for the current rate.
struct FakeVibrato {
    params: Arc<VibratoParams>,
    sample_rate: f32,
    lines: [Vec<f32>; MAX_CHANNELS],
    write_pos: usize,
    phase: f32,
    /// Mirror of `phase` for the editor's playhead.
    shared_phase: Arc<AtomicF32>,
}

#[derive(Params)]
struct VibratoParams {
    #[persist = "editor-state"]
    editor_state: Arc<EguiState>,

    #[id = "rate"]
    rate: FloatParam,
    #[id = "depth"]
    depth: FloatParam,
    #[id = "mix"]
    mix: FloatParam,
}

impl Default for VibratoParams {
    fn default() -> Self {
        Self {
            editor_state: EguiState::from_size(520, 380),
            rate: FloatParam::new(
                "Rate",
                5.5,
                FloatRange::Skewed { min: 1.0, max: 10.0, factor: FloatRange::skew_factor(-0.5) },
            )
            .with_smoother(SmoothingStyle::Linear(50.0))
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            depth: FloatParam::new("Depth", 30.0, FloatRange::Linear { min: 0.0, max: 100.0 })
                .with_smoother(SmoothingStyle::Linear(50.0))
                .with_unit(" cents")
                .with_value_to_string(formatters::v2s_f32_rounded(1)),
            mix: FloatParam::new("Mix", 1.0, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(50.0))
                .with_unit(" %")
                .with_value_to_string(formatters::v2s_f32_percentage(0))
                .with_string_to_value(formatters::s2v_f32_percentage()),
        }
    }
}

impl Default for FakeVibrato {
    fn default() -> Self {
        Self {
            params: Arc::new(VibratoParams::default()),
            sample_rate: 44100.0,
            lines: [Vec::new(), Vec::new()],
            write_pos: 0,
            phase: 0.0,
            shared_phase: Arc::new(AtomicF32::new(0.0)),
        }
    }
}

impl Plugin for FakeVibrato {
    const NAME: &'static str = "Fake Vibrato";
    const VENDOR: &'static str = "calvin";
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
        let phase = self.shared_phase.clone();
        create_egui_editor(
            self.params.editor_state.clone(),
            (),
            |_, _| {},
            move |ctx, setter, _state| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE.fill(ui::background()).inner_margin(14.0))
                    .show(ctx, |ui| ui::draw(ui, &params, setter, &phase));
            },
        )
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        self.sample_rate = buffer_config.sample_rate;
        let len = (MAX_DELAY_MS / 1000.0 * self.sample_rate).ceil() as usize + 4;
        for line in &mut self.lines {
            line.clear();
            line.resize(len, 0.0);
        }
        self.write_pos = 0;
        true
    }

    fn reset(&mut self) {
        for line in &mut self.lines {
            line.fill(0.0);
        }
        self.phase = 0.0;
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let len = self.lines[0].len();
        for mut frame in buffer.iter_samples() {
            let rate = self.params.rate.smoothed.next();
            let cents = self.params.depth.smoothed.next();
            let mix = self.params.mix.smoothed.next();

            // Peak pitch ratio deviation is D * 2*pi*rate, so solve for the delay excursion D.
            let excursion = (cents * LN_2 / 1200.0) / (TAU * rate) * self.sample_rate;
            let delay = excursion * (1.0 + (self.phase * TAU).sin());
            self.phase = (self.phase + rate / self.sample_rate).fract();

            let read = self.write_pos as f32 - delay;
            let read = if read < 0.0 { read + len as f32 } else { read };
            let i0 = read as usize % len;
            let i1 = (i0 + 1) % len;
            let frac = read.fract();

            for (ch, sample) in frame.iter_mut().enumerate() {
                let line = &mut self.lines[ch];
                line[self.write_pos] = *sample;
                let wet = line[i0] + (line[i1] - line[i0]) * frac;
                *sample = *sample * (1.0 - mix) + wet * mix;
            }
            self.write_pos = (self.write_pos + 1) % len;
        }
        self.shared_phase.store(self.phase, Ordering::Relaxed);
        ProcessStatus::Normal
    }
}

impl Vst3Plugin for FakeVibrato {
    const VST3_CLASS_ID: [u8; 16] = *b"CalvinFakeVibrat";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Modulation];
}

nih_export_vst3!(FakeVibrato);
