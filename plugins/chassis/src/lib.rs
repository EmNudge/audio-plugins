use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, egui, EguiState};
use std::sync::Arc;

pub mod dsp;
pub mod dynamics;
pub mod filters;
pub mod fx;
pub mod pitch;
pub mod presets;
pub mod psola;
pub mod ui;

#[cfg(test)]
mod tests;

use dsp::{Carrier, Chassis, Settings, MAX_CHANNELS, MAX_HIGH_HZ, MAX_LOW_HZ, MIN_HIGH_HZ, MIN_LOW_HZ};

/// A robot-voice rig: see `dsp.rs`.
///
/// The latency it reports is the pitch shifter's grains plus the limiter's lookahead, and the dry
/// path of the Mix knob is delayed to match.
struct ChassisPlugin {
    params: Arc<ChassisParams>,
    chassis: Chassis,
}

#[derive(Params)]
pub struct ChassisParams {
    #[persist = "editor-state"]
    editor_state: Arc<EguiState>,

    #[id = "pitch"]
    pitch: FloatParam,
    #[id = "formant"]
    formant: FloatParam,
    #[id = "gate"]
    gate: FloatParam,
    #[id = "boxy"]
    boxy: FloatParam,
    #[id = "weight"]
    weight: FloatParam,

    #[id = "low"]
    low: FloatParam,
    #[id = "high"]
    high: FloatParam,

    #[id = "sub"]
    sub: FloatParam,
    #[id = "squash"]
    squash: FloatParam,
    #[id = "chest"]
    chest: FloatParam,

    #[id = "ring"]
    ring: FloatParam,
    #[id = "ring_hz"]
    ring_hz: FloatParam,
    #[id = "vocoder"]
    vocoder: FloatParam,
    #[id = "carrier"]
    carrier: EnumParam<Carrier>,
    #[id = "note"]
    note: FloatParam,
    #[id = "wave"]
    wave: FloatParam,
    #[id = "shift"]
    shift: FloatParam,
    #[id = "comb"]
    comb: FloatParam,
    #[id = "comb_hz"]
    comb_hz: FloatParam,
    #[id = "grit"]
    grit: FloatParam,
    #[id = "body"]
    body: FloatParam,

    #[id = "cavity"]
    cavity: FloatParam,
    #[id = "cavity_ms"]
    cavity_ms: FloatParam,
    #[id = "shine"]
    shine: FloatParam,
    #[id = "shine_st"]
    shine_st: FloatParam,
    #[id = "plate"]
    plate: FloatParam,
    #[id = "decay"]
    decay: FloatParam,
    #[id = "shell"]
    shell: FloatParam,

    #[id = "glue"]
    glue: FloatParam,
    #[id = "tame"]
    tame: FloatParam,
    #[id = "push"]
    push: FloatParam,
    #[id = "ceiling"]
    ceiling: FloatParam,
    #[id = "mix"]
    mix: FloatParam,
}

const SMOOTH_MS: f32 = 40.0;

fn percent(name: &str, default: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
        .with_smoother(SmoothingStyle::Linear(SMOOTH_MS))
        .with_unit(" %")
        .with_value_to_string(formatters::v2s_f32_percentage(0))
        .with_string_to_value(formatters::s2v_f32_percentage())
}

fn semitones(name: &str, default: f32, min: f32, max: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min, max })
        .with_smoother(SmoothingStyle::Linear(SMOOTH_MS))
        .with_unit(" st")
        .with_value_to_string(Arc::new(|v| format!("{v:+.1}")))
}

fn db(name: &str, default: f32, min: f32, max: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min, max })
        .with_smoother(SmoothingStyle::Linear(SMOOTH_MS))
        .with_unit(" dB")
        .with_value_to_string(formatters::v2s_f32_rounded(1))
}

fn hz(name: &str, default: f32, min: f32, max: f32, skew: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Skewed { min, max, factor: FloatRange::skew_factor(skew) })
        .with_smoother(SmoothingStyle::Linear(SMOOTH_MS))
        .with_unit(" Hz")
        .with_value_to_string(formatters::v2s_f32_rounded(0))
}

fn ms(name: &str, default: f32, min: f32, max: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min, max })
        .with_smoother(SmoothingStyle::Linear(SMOOTH_MS))
        .with_unit(" ms")
        .with_value_to_string(formatters::v2s_f32_rounded(1))
}

const NOTES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

pub fn note_name(midi: f32) -> String {
    let n = midi.round() as i32;
    format!("{}{}", NOTES[n.rem_euclid(12) as usize], n.div_euclid(12) - 1)
}

impl Default for ChassisParams {
    fn default() -> Self {
        let d = Settings::default();
        Self {
            editor_state: EguiState::from_size(1000, 640),

            pitch: semitones("Pitch", d.pitch, -8.0, 8.0),
            formant: semitones("Formant", d.formant, -12.0, 12.0),
            gate: FloatParam::new("Gate", d.gate_db, FloatRange::Linear { min: -80.0, max: -20.0 })
                .with_smoother(SmoothingStyle::Linear(SMOOTH_MS))
                .with_value_to_string(Arc::new(|v| if v <= -79.0 { "OFF".to_owned() } else { format!("{v:.0} dB") })),
            boxy: percent("Box", d.boxy),
            weight: percent("Weight", d.weight),

            low: hz("Low split", d.low_hz, MIN_LOW_HZ, MAX_LOW_HZ, -1.5),
            high: hz("High split", d.high_hz, MIN_HIGH_HZ, MAX_HIGH_HZ, -1.5),

            sub: percent("Sub", d.sub),
            squash: percent("Squash", d.squash),
            chest: db("Chest", d.chest_db, -24.0, 12.0),

            ring: percent("Ring", d.ring),
            ring_hz: hz("Ring Hz", d.ring_hz, 15.0, 2000.0, -2.5),
            vocoder: percent("Vocoder", d.vocoder),
            carrier: EnumParam::new("Carrier", d.carrier),
            note: FloatParam::new("Note", d.note, FloatRange::Linear { min: 24.0, max: 60.0 })
                .with_step_size(1.0)
                .with_smoother(SmoothingStyle::Linear(SMOOTH_MS))
                .with_value_to_string(Arc::new(|v| note_name(v))),
            wave: FloatParam::new("Wave", d.wave, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(SMOOTH_MS))
                .with_value_to_string(Arc::new(|v| {
                    if v < 0.02 {
                        "saw".to_owned()
                    } else if v > 0.98 {
                        "square".to_owned()
                    } else {
                        format!("{:.0}% sq", v * 100.0)
                    }
                })),
            shift: FloatParam::new("Shift", d.shift_hz, FloatRange::Linear { min: -30.0, max: 30.0 })
                .with_smoother(SmoothingStyle::Linear(SMOOTH_MS))
                .with_unit(" Hz")
                .with_value_to_string(Arc::new(|v| format!("{v:+.1}"))),
            comb: percent("Comb", d.comb),
            comb_hz: hz("Comb Hz", d.comb_hz, 60.0, 1200.0, -1.5),
            grit: percent("Grit", d.grit),
            body: db("Body", d.body_db, -24.0, 12.0),

            cavity: percent("Cavity", d.cavity),
            cavity_ms: ms("Cavity ms", d.cavity_ms, 4.0, 40.0),
            shine: percent("Shine", d.shine),
            shine_st: semitones("Shine st", d.shine_st, 0.0, 12.0),
            plate: percent("Plate", d.plate),
            decay: FloatParam::new("Decay", d.decay_s, FloatRange::Skewed { min: 0.1, max: 1.5, factor: FloatRange::skew_factor(-1.0) })
                .with_smoother(SmoothingStyle::Linear(SMOOTH_MS))
                .with_unit(" s")
                .with_value_to_string(formatters::v2s_f32_rounded(2)),
            shell: db("Shell", d.shell_db, -24.0, 12.0),

            glue: percent("Glue", d.glue),
            tame: percent("Tame", d.tame),
            push: db("Push", d.push_db, 0.0, 12.0),
            ceiling: db("Ceiling", d.ceiling_db, -12.0, 0.0),
            mix: percent("Mix", d.mix),
        }
    }
}

impl ChassisParams {
    /// Every setting, read through `f` (the plain value for the editor, the next smoothed one in
    /// the audio thread).
    fn read(&self, f: impl Fn(&FloatParam) -> f32) -> Settings {
        Settings {
            pitch: f(&self.pitch),
            formant: f(&self.formant),
            gate_db: f(&self.gate),
            boxy: f(&self.boxy),
            weight: f(&self.weight),
            low_hz: f(&self.low),
            high_hz: f(&self.high),
            sub: f(&self.sub),
            squash: f(&self.squash),
            chest_db: f(&self.chest),
            ring: f(&self.ring),
            ring_hz: f(&self.ring_hz),
            vocoder: f(&self.vocoder),
            carrier: self.carrier.value(),
            note: f(&self.note),
            wave: f(&self.wave),
            shift_hz: f(&self.shift),
            comb: f(&self.comb),
            comb_hz: f(&self.comb_hz),
            grit: f(&self.grit),
            body_db: f(&self.body),
            cavity: f(&self.cavity),
            cavity_ms: f(&self.cavity_ms),
            shine: f(&self.shine),
            shine_st: f(&self.shine_st),
            plate: f(&self.plate),
            decay_s: f(&self.decay),
            shell_db: f(&self.shell),
            glue: f(&self.glue),
            tame: f(&self.tame),
            push_db: f(&self.push),
            ceiling_db: f(&self.ceiling),
            mix: f(&self.mix),
        }
    }

    /// The current values with no smoothing, for the editor.
    pub fn settings(&self) -> Settings {
        self.read(|p| p.value())
    }
}

impl Default for ChassisPlugin {
    fn default() -> Self {
        Self { params: Arc::new(ChassisParams::default()), chassis: Chassis::new() }
    }
}

impl Plugin for ChassisPlugin {
    const NAME: &'static str = "Chassis Robot Voice";
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
        let meter = self.chassis.meter.clone();
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
        self.chassis.configure(buffer_config.sample_rate, channels);
        context.set_latency_samples(self.chassis.latency() as u32);
        true
    }

    fn reset(&mut self) {
        self.chassis.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let mut settings = self.params.read(|p| p.value());
        for mut frame in buffer.iter_samples() {
            settings = self.params.read(|p| p.smoothed.next());
            let mut x = [0.0; MAX_CHANNELS];
            for (ch, sample) in frame.iter_mut().enumerate().take(MAX_CHANNELS) {
                x[ch] = *sample;
            }
            let out = self.chassis.process(x, &settings);
            for (ch, sample) in frame.iter_mut().enumerate().take(MAX_CHANNELS) {
                *sample = out[ch];
            }
        }
        self.chassis.publish();
        ProcessStatus::Tail(self.chassis.tail_samples(&settings))
    }
}

impl Vst3Plugin for ChassisPlugin {
    const VST3_CLASS_ID: [u8; 16] = *b"ClowdChassisRob_";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Fx, Vst3SubCategory::Distortion];
}

nih_export_vst3!(ChassisPlugin);
