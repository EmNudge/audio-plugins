use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, EguiState};
use std::sync::atomic::Ordering;
use std::sync::Arc;

pub mod dsp;
pub mod presets;
pub mod ui;

use dsp::{Calibration, Channel, Scope, Settings, Style, Voice, LATENCY, SCOPE_N, SCOPE_STRIDE};

/// Range of the per-band trim, in dB. A band's drive is the master drive plus its trim.
pub const TRIM_MIN: f32 = -24.0;
pub const TRIM_MAX: f32 = 12.0;
/// Gain of a tube stage at full amount, and the most density can push it to.
pub const TUBE_GAIN: f32 = 5.0;
const TUBE_GAIN_MAX: f32 = 8.0;
const DRIVE_MIN: f32 = -24.0;
const DRIVE_MAX: f32 = 48.0;
const DELAY_LEN: usize = 64;

struct Kiln {
    params: Arc<KilnParams>,
    fs: f32,
    channels: [Channel; 2],
    dry_delay: [[f32; DELAY_LEN]; 2],
    delay_pos: usize,
    /// Last crossover and tone values pushed into the channels. NaN forces a refresh.
    crossover: (f32, f32),
    tone: f32,
    air: f32,
    sat_freq: f32,
    calibration: Option<Calibration>,
    /// Slow power followers for gain matching, and the gain they produce.
    power_in: f32,
    power_out: f32,
    match_gain: f32,
    scope: Arc<Scope>,
    scope_pos: usize,
    scope_tick: usize,
}

#[derive(Params)]
pub struct KilnParams {
    #[persist = "editor-state"]
    pub editor_state: Arc<EguiState>,

    #[id = "style"]
    pub style: EnumParam<Style>,
    #[id = "drive"]
    pub drive: FloatParam,
    #[id = "low"]
    pub low: FloatParam,
    #[id = "mid"]
    pub mid: FloatParam,
    #[id = "high"]
    pub high: FloatParam,
    #[id = "xlow"]
    pub xover_low: FloatParam,
    #[id = "xhigh"]
    pub xover_high: FloatParam,
    #[id = "asym"]
    pub asym: FloatParam,
    #[id = "dynamics"]
    pub dynamics: FloatParam,
    #[id = "tone"]
    pub tone: FloatParam,
    #[id = "pentode"]
    pub pentode: FloatParam,
    #[id = "triode"]
    pub triode: FloatParam,
    #[id = "density"]
    pub density: FloatParam,
    #[id = "xfmr"]
    pub xfmr: FloatParam,
    #[id = "calib"]
    pub calibration: EnumParam<Calibration>,
    #[id = "air"]
    pub air: FloatParam,
    #[id = "sat"]
    pub sat: FloatParam,
    #[id = "voice"]
    pub voice: EnumParam<Voice>,
    #[id = "satfreq"]
    pub sat_freq: FloatParam,
    #[id = "mix"]
    pub mix: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
    #[id = "match"]
    pub gain_match: BoolParam,
}

impl KilnParams {
    pub fn trim(&self, band: usize) -> &FloatParam {
        [&self.low, &self.mid, &self.high][band]
    }

    /// The drive a band actually runs at, in dB.
    pub fn drive_db(&self, band: usize) -> f32 {
        (self.drive.value() + self.trim(band).value()).clamp(DRIVE_MIN, DRIVE_MAX)
    }
}

/// The gain a tube stage runs at: its own amount, pushed together with the other by density.
pub fn tube_gain(amount: f32, density: f32) -> f32 {
    (TUBE_GAIN * amount * (1.0 + density)).min(TUBE_GAIN_MAX)
}

impl Default for KilnParams {
    fn default() -> Self {
        let db = |name: &'static str, default: f32, min: f32, max: f32| {
            FloatParam::new(name, default, FloatRange::Linear { min, max })
                .with_smoother(SmoothingStyle::Linear(30.0))
                .with_unit(" dB")
                .with_step_size(0.1)
        };
        let signed = |name: &'static str| {
            FloatParam::new(name, 0.0, FloatRange::Linear { min: -1.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(40.0))
                .with_value_to_string(formatters::v2s_f32_percentage(0))
                .with_string_to_value(formatters::s2v_f32_percentage())
        };
        let percent = |name: &'static str, default: f32| {
            FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(40.0))
                .with_value_to_string(formatters::v2s_f32_percentage(0))
                .with_string_to_value(formatters::s2v_f32_percentage())
        };
        let hz = |name: &'static str, default: f32, min: f32, max: f32| {
            FloatParam::new(name, default, FloatRange::Skewed { min, max, factor: FloatRange::skew_factor(-1.0) })
                .with_unit(" Hz")
                .with_value_to_string(formatters::v2s_f32_rounded(0))
        };
        Self {
            editor_state: EguiState::from_size(640, 660),
            style: EnumParam::new("Style", Style::Tube),
            drive: db("Drive", 12.0, 0.0, 36.0),
            low: db("Low", 0.0, TRIM_MIN, TRIM_MAX),
            mid: db("Mid", 0.0, TRIM_MIN, TRIM_MAX),
            high: db("High", 0.0, TRIM_MIN, TRIM_MAX),
            xover_low: hz("Low Crossover", 150.0, 40.0, 600.0),
            xover_high: hz("High Crossover", 2500.0, 800.0, 10000.0),
            asym: signed("Asym"),
            dynamics: signed("Dynamics"),
            tone: signed("Tone"),
            pentode: percent("Pentode", 0.3),
            triode: percent("Triode", 0.3),
            density: percent("Density", 0.0),
            xfmr: percent("Transformer", 0.0),
            calibration: EnumParam::new("Calibration", Calibration::Normal),
            air: percent("Air", 0.0),
            sat: percent("Saturation", 0.0),
            voice: EnumParam::new("Saturation Voice", Voice::Warm),
            sat_freq: hz("Saturation Frequency", 1500.0, 200.0, 10000.0),
            mix: FloatParam::new("Mix", 1.0, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(40.0))
                .with_value_to_string(formatters::v2s_f32_percentage(0))
                .with_string_to_value(formatters::s2v_f32_percentage()),
            output: FloatParam::new(
                "Output",
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
            gain_match: BoolParam::new("Gain Match", true),
        }
    }
}

fn make_channels(fs: f32) -> [Channel; 2] {
    [Channel::new(fs), Channel::new(fs)]
}

impl Default for Kiln {
    fn default() -> Self {
        Self {
            params: Arc::new(KilnParams::default()),
            fs: 44100.0,
            channels: make_channels(44100.0),
            dry_delay: [[0.0; DELAY_LEN]; 2],
            delay_pos: 0,
            crossover: (f32::NAN, f32::NAN),
            tone: f32::NAN,
            air: f32::NAN,
            sat_freq: f32::NAN,
            calibration: None,
            power_in: 0.0,
            power_out: 0.0,
            match_gain: 1.0,
            scope: Arc::new(Scope::default()),
            scope_pos: 0,
            scope_tick: 0,
        }
    }
}

impl Plugin for Kiln {
    const NAME: &'static str = "Kiln Saturator";
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
        let scope = self.scope.clone();
        create_egui_editor(
            self.params.editor_state.clone(),
            ui::UiState::default(),
            |_, _| {},
            move |ctx, setter, state| {
                nih_plug_egui::egui::CentralPanel::default()
                    .frame(nih_plug_egui::egui::Frame::NONE.fill(ui::background()).inner_margin(14.0))
                    .show(ctx, |ui| ui::draw(ui, &params, setter, &scope, state));
            },
        )
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        context: &mut impl InitContext<Self>,
    ) -> bool {
        self.fs = buffer_config.sample_rate;
        self.channels = make_channels(self.fs);
        self.reset();
        context.set_latency_samples(LATENCY as u32);
        true
    }

    fn reset(&mut self) {
        self.channels = make_channels(self.fs);
        self.dry_delay = [[0.0; DELAY_LEN]; 2];
        self.crossover = (f32::NAN, f32::NAN);
        self.tone = f32::NAN;
        self.air = f32::NAN;
        self.sat_freq = f32::NAN;
        self.calibration = None;
        self.power_in = 0.0;
        self.power_out = 0.0;
        self.match_gain = 1.0;
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let p = self.params.clone();
        let crossover = (p.xover_low.value(), p.xover_high.value());
        if crossover != self.crossover {
            self.crossover = crossover;
            for ch in &mut self.channels {
                ch.set_crossover(crossover.0, crossover.1);
            }
        }
        let sat_freq = p.sat_freq.value();
        if sat_freq != self.sat_freq {
            self.sat_freq = sat_freq;
            for ch in &mut self.channels {
                ch.set_sat_freq(sat_freq);
            }
        }
        let calibration = p.calibration.value();
        if self.calibration != Some(calibration) {
            self.calibration = Some(calibration);
            for ch in &mut self.channels {
                ch.set_calibration(calibration);
            }
        }
        let power_coef = 1.0 - (-1.0 / (0.4 * self.fs)).exp();
        let mut peaks = [0.0f32; 3];

        for mut frame in buffer.iter_samples() {
            let drive = p.drive.smoothed.next();
            let gains = [p.low.smoothed.next(), p.mid.smoothed.next(), p.high.smoothed.next()]
                .map(|trim| 10.0f32.powf((drive + trim).clamp(DRIVE_MIN, DRIVE_MAX) / 20.0));
            let density = p.density.smoothed.next();
            let set = Settings {
                style: p.style.value(),
                asym: p.asym.smoothed.next(),
                dynamics: p.dynamics.smoothed.next(),
                tubes: [
                    tube_gain(p.pentode.smoothed.next(), density),
                    tube_gain(p.triode.smoothed.next(), density),
                ],
                sat: p.sat.smoothed.next(),
                voice: p.voice.value(),
                xfmr: p.xfmr.smoothed.next(),
            };
            let air = p.air.smoothed.next();
            if air != self.air {
                self.air = air;
                for ch in &mut self.channels {
                    ch.set_air(air);
                }
            }
            let tone = p.tone.smoothed.next();
            if tone != self.tone {
                self.tone = tone;
                for ch in &mut self.channels {
                    ch.set_tone(tone);
                }
            }
            let mix = p.mix.smoothed.next();
            let output = p.output.smoothed.next();

            let channels = frame.len();
            let (mut dry, mut wet) = ([0.0f32; 2], [0.0f32; 2]);
            for (ch, sample) in frame.iter_mut().enumerate() {
                let x = *sample;
                let rendered = self.channels[ch].process(x, gains, set);
                self.dry_delay[ch][self.delay_pos] = x;
                dry[ch] = self.dry_delay[ch][(self.delay_pos + DELAY_LEN - LATENCY) % DELAY_LEN];
                wet[ch] = rendered.wet;

                if ch == 0 {
                    for b in 0..3 {
                        peaks[b] = peaks[b].max(rendered.output[b].abs());
                    }
                    if self.scope_tick == 0 {
                        for b in 0..3 {
                            self.scope.input[b][self.scope_pos].store(rendered.input[b], Ordering::Relaxed);
                            self.scope.output[b][self.scope_pos].store(rendered.output[b], Ordering::Relaxed);
                        }
                        self.scope_pos = (self.scope_pos + 1) % SCOPE_N;
                    }
                    self.scope_tick = (self.scope_tick + 1) % SCOPE_STRIDE;
                }
            }
            self.delay_pos = (self.delay_pos + 1) % DELAY_LEN;

            // Gain match: ride the wet signal to the loudness of the dry one, slowly enough
            // that it follows the song and not the waveform.
            let power = |v: &[f32; 2]| v[..channels].iter().map(|s| s * s).sum::<f32>() / channels as f32;
            self.power_in += power_coef * (power(&dry) - self.power_in);
            self.power_out += power_coef * (power(&wet) - self.power_out);
            let target = if p.gain_match.value() && self.power_in > 1e-8 && self.power_out > 1e-8 {
                (self.power_in / self.power_out).sqrt().clamp(0.1, 4.0)
            } else {
                1.0
            };
            self.match_gain += 0.0005 * (target - self.match_gain);

            for (ch, sample) in frame.iter_mut().enumerate() {
                *sample = (dry[ch] * (1.0 - mix) + wet[ch] * self.match_gain * mix) * output;
            }
        }

        for b in 0..3 {
            self.scope.level[b].store(peaks[b], Ordering::Relaxed);
        }
        ProcessStatus::Normal
    }
}

impl Vst3Plugin for Kiln {
    const VST3_CLASS_ID: [u8; 16] = *b"ClowdKilnSatur1_";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Fx, Vst3SubCategory::Distortion];
}

nih_export_vst3!(Kiln);
