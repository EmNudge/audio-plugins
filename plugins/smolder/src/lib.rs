use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, egui, EguiState};
use std::sync::atomic::Ordering;
use std::sync::Arc;

pub mod dsp;
pub mod presets;
pub mod ui;

use dsp::{
    Bands, Crossover, Envelope, Filters, Lfo, LfoShape, Model, ModSource, ModTarget, Oversampler, Oversampling,
    LATENCY,
};

pub const BAND_COUNT: usize = 3;
/// How far a full-strength modulation moves the drive, in dB.
const MOD_DRIVE_DB: f32 = 24.0;
const MAX_DRIVE_DB: f32 = 48.0;

struct Smolder {
    params: Arc<SmolderParams>,
    live: Arc<Live>,
    /// `[channel][band]`; single-band mode only uses band 0.
    os: [[Oversampler; BAND_COUNT]; 2],
    crossover: [Crossover; 2],
    crossover_freqs: (f32, f32),
    filters: [Filters; 2],
    lfo: Lfo,
    envelope: Envelope,
    dry_delay: [[f32; 64]; 2],
    delay_pos: usize,
    sample_rate: f32,
    stages: usize,
    bands: Bands,
}

/// What the audio thread is doing right now, for the editor to draw.
#[derive(Default)]
pub struct Live {
    pub drive_db: AtomicF32,
    pub bias: AtomicF32,
    pub tone: AtomicF32,
}

#[derive(Params)]
pub struct BandParams {
    #[id = "model"]
    pub model: EnumParam<Model>,
    /// Added to the main drive.
    #[id = "drive"]
    pub drive: FloatParam,
    #[id = "level"]
    pub level: FloatParam,
}

impl BandParams {
    pub fn new(model: Model) -> Self {
        Self {
            model: EnumParam::new("Model", model),
            drive: FloatParam::new("Drive", 0.0, FloatRange::Linear { min: -24.0, max: 24.0 })
                .with_smoother(SmoothingStyle::Linear(30.0))
                .with_unit(" dB")
                .with_value_to_string(formatters::v2s_f32_rounded(1)),
            level: gain_param("Level", -24.0, 12.0),
        }
    }
}

#[derive(Params)]
pub struct SmolderParams {
    #[persist = "editor-state"]
    pub editor_state: Arc<EguiState>,

    #[id = "model"]
    pub model: EnumParam<Model>,
    #[id = "drive"]
    pub drive: FloatParam,
    #[id = "bias"]
    pub bias: FloatParam,
    #[id = "lowcut"]
    pub low_cut: FloatParam,
    #[id = "tone"]
    pub tone: FloatParam,
    #[id = "oversamp"]
    pub oversampling: EnumParam<Oversampling>,
    #[id = "mix"]
    pub mix: FloatParam,
    #[id = "output"]
    pub output: FloatParam,

    #[id = "bands"]
    pub bands: EnumParam<Bands>,
    #[id = "xover1"]
    pub crossover_low: FloatParam,
    #[id = "xover2"]
    pub crossover_high: FloatParam,
    #[nested(array, group = "Band")]
    pub band: [BandParams; BAND_COUNT],

    #[id = "modsrc"]
    pub mod_source: EnumParam<ModSource>,
    #[id = "modtgt"]
    pub mod_target: EnumParam<ModTarget>,
    #[id = "modshape"]
    pub mod_shape: EnumParam<LfoShape>,
    #[id = "modamt"]
    pub mod_amount: FloatParam,
    #[id = "modrate"]
    pub mod_rate: FloatParam,
}

fn percent(name: &'static str, default: f32, min: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min, max: 1.0 })
        .with_smoother(SmoothingStyle::Linear(30.0))
        .with_value_to_string(formatters::v2s_f32_percentage(0))
        .with_string_to_value(formatters::s2v_f32_percentage())
}

fn gain_param(name: &'static str, min_db: f32, max_db: f32) -> FloatParam {
    FloatParam::new(
        name,
        util::db_to_gain(0.0),
        FloatRange::Skewed {
            min: util::db_to_gain(min_db),
            max: util::db_to_gain(max_db),
            factor: FloatRange::gain_skew_factor(min_db, max_db),
        },
    )
    .with_smoother(SmoothingStyle::Logarithmic(30.0))
    .with_unit(" dB")
    .with_value_to_string(formatters::v2s_f32_gain_to_db(1))
    .with_string_to_value(formatters::s2v_f32_gain_to_db())
}

fn hz_param(name: &'static str, default: f32, min: f32, max: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Skewed { min, max, factor: FloatRange::skew_factor(-2.0) })
        .with_value_to_string(formatters::v2s_f32_hz_then_khz(0))
        .with_string_to_value(formatters::s2v_f32_hz_then_khz())
}

impl Default for SmolderParams {
    fn default() -> Self {
        Self {
            editor_state: EguiState::from_size(760, 780),
            model: EnumParam::new("Model", Model::Tube),
            drive: FloatParam::new("Drive", 12.0, FloatRange::Linear { min: 0.0, max: 40.0 })
                .with_smoother(SmoothingStyle::Linear(30.0))
                .with_unit(" dB")
                .with_value_to_string(formatters::v2s_f32_rounded(1)),
            bias: percent("Bias", 0.0, -1.0),
            low_cut: hz_param("Low Cut", 20.0, 20.0, 1000.0).with_smoother(SmoothingStyle::Logarithmic(30.0)),
            tone: percent("Tone", 0.0, -1.0),
            oversampling: EnumParam::new("Oversampling", Oversampling::X2),
            mix: percent("Mix", 1.0, 0.0),
            output: gain_param("Output", -24.0, 12.0),

            bands: EnumParam::new("Bands", Bands::Single),
            crossover_low: hz_param("Crossover 1", 200.0, 40.0, 2000.0),
            crossover_high: hz_param("Crossover 2", 2500.0, 500.0, 12000.0),
            band: std::array::from_fn(|_| BandParams::new(Model::Tube)),

            mod_source: EnumParam::new("Modulation", ModSource::Off),
            mod_target: EnumParam::new("Target", ModTarget::Drive),
            mod_shape: EnumParam::new("LFO Shape", LfoShape::Sine),
            mod_amount: percent("Mod Amount", 0.5, -1.0),
            mod_rate: FloatParam::new(
                "LFO Rate",
                1.0,
                FloatRange::Skewed { min: 0.05, max: 20.0, factor: FloatRange::skew_factor(-2.0) },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
        }
    }
}

const FREQS: (f32, f32) = (200.0, 2500.0);

impl Default for Smolder {
    fn default() -> Self {
        Self {
            params: Arc::new(SmolderParams::default()),
            live: Arc::new(Live::default()),
            os: std::array::from_fn(|_| std::array::from_fn(|_| Oversampler::new())),
            crossover: [Crossover::new(44100.0, FREQS.0, FREQS.1), Crossover::new(44100.0, FREQS.0, FREQS.1)],
            crossover_freqs: FREQS,
            filters: Default::default(),
            lfo: Lfo::default(),
            envelope: Envelope::default(),
            dry_delay: [[0.0; 64]; 2],
            delay_pos: 0,
            sample_rate: 44100.0,
            stages: 0,
            bands: Bands::Single,
        }
    }
}

impl Smolder {
    fn clear_oversamplers(&mut self) {
        for o in self.os.iter_mut().flatten() {
            o.clear();
            o.set_stages(self.stages);
        }
    }
}

impl Plugin for Smolder {
    const NAME: &'static str = "Smolder Distortion";
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
        let live = self.live.clone();
        create_egui_editor(
            self.params.editor_state.clone(),
            ui::UiState::default(),
            |_, _| {},
            move |ctx, setter, state| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE.fill(ui::background()).inner_margin(14.0))
                    .show(ctx, |ui| ui::draw(ui, &params, setter, &live, state));
            },
        )
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        context: &mut impl InitContext<Self>,
    ) -> bool {
        self.sample_rate = buffer_config.sample_rate;
        self.stages = self.params.oversampling.value().stages();
        self.crossover_freqs = FREQS;
        self.crossover = std::array::from_fn(|_| Crossover::new(self.sample_rate, FREQS.0, FREQS.1));
        context.set_latency_samples(LATENCY[self.stages] as u32);
        true
    }

    fn reset(&mut self) {
        self.clear_oversamplers();
        self.crossover = std::array::from_fn(|_| {
            Crossover::new(self.sample_rate, self.crossover_freqs.0, self.crossover_freqs.1)
        });
        self.filters = Default::default();
        self.dry_delay = [[0.0; 64]; 2];
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let p = self.params.clone();
        let stages = p.oversampling.value().stages();
        if stages != self.stages {
            self.stages = stages;
            self.os.iter_mut().flatten().for_each(|o| o.set_stages(stages));
            context.set_latency_samples(LATENCY[stages] as u32);
        }
        let bands = p.bands.value();
        if bands != self.bands {
            self.bands = bands;
            self.clear_oversamplers();
        }
        let freqs = (p.crossover_low.value(), p.crossover_high.value().max(p.crossover_low.value() * 1.25));
        if freqs != self.crossover_freqs {
            self.crossover_freqs = freqs;
            for xo in &mut self.crossover {
                xo.retune(self.sample_rate, freqs.0, freqs.1);
            }
        }

        let latency = LATENCY[stages];
        let model = p.model.value();
        let band_models: [Model; BAND_COUNT] = std::array::from_fn(|i| p.band[i].model.value());
        let (source, target, shape) = (p.mod_source.value(), p.mod_target.value(), p.mod_shape.value());
        let rate = p.mod_rate.value();
        let fs = self.sample_rate;
        let tone_a = dsp::tone_coeff(fs);

        for mut frame in buffer.iter_samples() {
            let peak = frame.iter_mut().map(|s| s.abs()).fold(0.0, f32::max);
            let modulation = p.mod_amount.smoothed.next()
                * match source {
                    ModSource::Off => 0.0,
                    ModSource::Lfo => self.lfo.next(fs, rate, shape),
                    ModSource::Envelope => self.envelope.next(fs, peak),
                };
            let drive_db = (p.drive.smoothed.next()
                + if target == ModTarget::Drive { modulation * MOD_DRIVE_DB } else { 0.0 })
            .clamp(0.0, MAX_DRIVE_DB);
            let bias = (p.bias.smoothed.next() + if target == ModTarget::Bias { modulation } else { 0.0 })
                .clamp(-1.0, 1.0);
            let tone = (p.tone.smoothed.next() + if target == ModTarget::Tone { modulation } else { 0.0 })
                .clamp(-1.0, 1.0);
            let low_cut_a = dsp::one_pole(fs, p.low_cut.smoothed.next());
            let mix = p.mix.smoothed.next();
            let output = p.output.smoothed.next();

            let drive = util::db_to_gain(drive_db);
            // Per band: the gain applied before the shaper, and the one after it. The second
            // undoes part of the first so loud drive settings stay near the dry level.
            let band_gains: [(f32, f32); BAND_COUNT] = std::array::from_fn(|i| {
                let band_drive = util::db_to_gain((drive_db + p.band[i].drive.smoothed.next()).clamp(-24.0, MAX_DRIVE_DB));
                (band_drive, p.band[i].level.smoothed.next() / band_drive.sqrt())
            });

            for (ch, sample) in frame.iter_mut().enumerate() {
                let x = *sample;
                self.dry_delay[ch][self.delay_pos] = x;
                let dry = self.dry_delay[ch][(self.delay_pos + 64 - latency) & 63];

                let filters = &mut self.filters[ch];
                let pre = filters.low_cut(x, low_cut_a);
                let shaped = match bands {
                    Bands::Single => {
                        self.os[ch][0].process(pre, &mut |v| dsp::shape(model, v * drive, bias)) / drive.sqrt()
                    }
                    Bands::Multi => {
                        let parts = self.crossover[ch].split(pre);
                        let mut sum = 0.0;
                        for b in 0..BAND_COUNT {
                            let (gain_in, gain_out) = band_gains[b];
                            let m = band_models[b];
                            sum += self.os[ch][b].process(parts[b], &mut |v| dsp::shape(m, v * gain_in, bias)) * gain_out;
                        }
                        sum
                    }
                };
                let centered = filters.dc_block(shaped, fs);
                let wet = filters.tilt(centered, tone_a, tone) * output;

                *sample = dry + (wet - dry) * mix;
            }
            self.delay_pos = (self.delay_pos + 1) & 63;

            self.live.drive_db.store(drive_db, Ordering::Relaxed);
            self.live.bias.store(bias, Ordering::Relaxed);
            self.live.tone.store(tone, Ordering::Relaxed);
        }
        ProcessStatus::Normal
    }
}

impl Vst3Plugin for Smolder {
    const VST3_CLASS_ID: [u8; 16] = *b"ClowdSmolderFx!_";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Fx, Vst3SubCategory::Distortion];
}

nih_export_vst3!(Smolder);
