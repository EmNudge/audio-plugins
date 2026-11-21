use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, egui, EguiState};
use std::sync::atomic::Ordering;
use std::sync::Arc;

pub mod analysis;
pub mod drum;
pub mod engine;
pub mod membrane;
pub mod metal;
pub mod presets;
pub mod shell;
pub mod ui;

use engine::{note_head, note_metal, Engine, Global, HeadSettings, Meters, ShellKind, Striker, HEADS, METALS};
use metal::MetalSettings;

pub struct Tympan {
    params: Arc<TympanParams>,
    engine: Engine,
    meters: Arc<Meters>,
    seen_audition: [u32; HEADS],
    seen_plate_audition: [u32; METALS],
}

/// One drum.
#[derive(Params)]
pub struct HeadParams {
    #[id = "striker"]
    pub striker: EnumParam<Striker>,
    #[id = "shellkind"]
    pub shell_kind: EnumParam<ShellKind>,
    #[id = "tune"]
    pub tune: FloatParam,
    #[id = "decay"]
    pub decay: FloatParam,
    #[id = "hard"]
    pub hard: FloatParam,
    #[id = "bend"]
    pub bend: FloatParam,
    /// The shell: diameter and length in centimetres, wall thickness in millimetres.
    #[id = "diameter"]
    pub diameter: FloatParam,
    #[id = "length"]
    pub length: FloatParam,
    #[id = "thickness"]
    pub thickness: FloatParam,
    #[id = "click"]
    pub click: FloatParam,
    #[id = "level"]
    pub level: FloatParam,
    #[id = "pan"]
    pub pan: FloatParam,
    /// Where the mallet lands, in the unit disc.
    #[id = "x"]
    pub x: FloatParam,
    #[id = "y"]
    pub y: FloatParam,
}

/// One hat or cymbal.
#[derive(Params)]
pub struct MetalParams {
    /// The plate's diameter in centimetres and thickness in millimetres.
    #[id = "mdiameter"]
    pub diameter: FloatParam,
    #[id = "mthickness"]
    pub thickness: FloatParam,
    #[id = "mtone"]
    pub tone: FloatParam,
    #[id = "mlevel"]
    pub level: FloatParam,
}

/// A plate's starting point: diameter in centimetres, thickness in millimetres, stick hardness.
/// 14 inch hats, a 17 inch crash and a 21 inch ride.
pub(crate) const PLATES: [(f32, f32, f32); METALS] = [(36.0, 0.8, 0.7), (36.0, 0.8, 0.65), (43.0, 0.8, 0.55), (53.0, 1.3, 0.45)];

impl MetalParams {
    fn new(diameter: f32, thickness: f32, tone: f32) -> Self {
        Self {
            diameter: FloatParam::new("Diameter", diameter, FloatRange::Linear { min: 20.0, max: 70.0 }).with_unit(" cm").with_value_to_string(formatters::v2s_f32_rounded(0)),
            thickness: FloatParam::new("Thickness", thickness, FloatRange::Linear { min: 0.3, max: 3.0 }).with_unit(" mm").with_value_to_string(formatters::v2s_f32_rounded(2)),
            tone: FloatParam::new("Stick", tone, FloatRange::Linear { min: 0.0, max: 1.0 }).with_value_to_string(formatters::v2s_f32_percentage(0)),
            level: FloatParam::new("Level", 0.0, FloatRange::Linear { min: -30.0, max: 6.0 })
                .with_unit(" dB")
                .with_value_to_string(formatters::v2s_f32_rounded(1)),
        }
    }

    pub fn settings(&self) -> MetalSettings {
        MetalSettings { diameter: self.diameter.value() * 0.01, thickness: self.thickness.value() * 0.001, tone: self.tone.value(), level: util::db_to_gain(self.level.value()) }
    }
}

/// A drum's starting point: pitch in Hz, decay in seconds, mallet hardness, bend, shell, pan, strike x and y.
pub(crate) struct Voicing {
    pub(crate) tune: f32,
    pub(crate) decay: f32,
    pub(crate) hard: f32,
    pub(crate) bend: f32,
    pub(crate) diameter: f32,
    pub(crate) length: f32,
    pub(crate) thickness: f32,
    pub(crate) click: f32,
    pub(crate) pan: f32,
    pub(crate) at: (f32, f32),
}

pub(crate) const VOICINGS: [Voicing; HEADS] = [
    // A 22 x 16 inch bass drum, 14 x 5.5 inch snare, 16 x 16 inch floor tom and 12 x 8 inch rack
    // tom, in centimetres, with 7 mm maple shells.
    Voicing { tune: 54.0, decay: 0.22, hard: 0.35, bend: 0.5, diameter: 56.0, length: 41.0, thickness: 7.0, click: 0.3, pan: 0.0, at: (0.0, 0.0) },
    Voicing { tune: 190.0, decay: 0.1, hard: 0.9, bend: 0.15, diameter: 36.0, length: 14.0, thickness: 7.0, click: 0.6, pan: -0.1, at: (0.3, -0.3) },
    Voicing { tune: 105.0, decay: 0.28, hard: 0.55, bend: 0.3, diameter: 41.0, length: 41.0, thickness: 7.0, click: 0.4, pan: -0.35, at: (0.3, 0.0) },
    Voicing { tune: 165.0, decay: 0.24, hard: 0.55, bend: 0.3, diameter: 30.0, length: 20.0, thickness: 7.0, click: 0.4, pan: 0.35, at: (-0.3, 0.1) },
];

fn pan_text() -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(|v| if v.abs() < 0.005 { "C".into() } else if v < 0.0 { format!("L{:.0}", -v * 100.0) } else { format!("R{:.0}", v * 100.0) })
}

impl HeadParams {
    fn new(v: &Voicing) -> Self {
        Self {
            striker: EnumParam::new("Striker", Striker::Stick),
            shell_kind: EnumParam::new("Shell Material", ShellKind::Wood),
            tune: FloatParam::new("Tune", v.tune, FloatRange::Skewed { min: 30.0, max: 600.0, factor: FloatRange::skew_factor(-1.2) })
                .with_unit(" Hz")
                .with_value_to_string(formatters::v2s_f32_rounded(0)),
            decay: FloatParam::new("Decay", v.decay, FloatRange::Skewed { min: 0.02, max: 3.0, factor: FloatRange::skew_factor(-2.0) })
                .with_unit(" s")
                .with_value_to_string(formatters::v2s_f32_rounded(2)),
            hard: FloatParam::new("Mallet", v.hard, FloatRange::Linear { min: 0.0, max: 1.0 }).with_value_to_string(formatters::v2s_f32_percentage(0)),
            bend: FloatParam::new("Bend", v.bend, FloatRange::Linear { min: 0.0, max: 1.0 }).with_value_to_string(formatters::v2s_f32_percentage(0)),
            click: FloatParam::new("Stick Ring", v.click, FloatRange::Linear { min: 0.0, max: 1.0 }).with_value_to_string(formatters::v2s_f32_percentage(0)),
            diameter: FloatParam::new("Diameter", v.diameter, FloatRange::Linear { min: 15.0, max: 80.0 }).with_unit(" cm").with_value_to_string(formatters::v2s_f32_rounded(0)),
            length: FloatParam::new("Length", v.length, FloatRange::Linear { min: 3.0, max: 70.0 }).with_unit(" cm").with_value_to_string(formatters::v2s_f32_rounded(0)),
            thickness: FloatParam::new("Thickness", v.thickness, FloatRange::Linear { min: 0.5, max: 14.0 }).with_unit(" mm").with_value_to_string(formatters::v2s_f32_rounded(1)),
            level: FloatParam::new("Level", 0.0, FloatRange::Linear { min: -30.0, max: 6.0 })
                .with_unit(" dB")
                .with_value_to_string(formatters::v2s_f32_rounded(1)),
            pan: FloatParam::new("Pan", v.pan, FloatRange::Linear { min: -1.0, max: 1.0 }).with_value_to_string(pan_text()),
            x: FloatParam::new("Strike X", v.at.0, FloatRange::Linear { min: -1.0, max: 1.0 }).with_value_to_string(formatters::v2s_f32_rounded(2)),
            y: FloatParam::new("Strike Y", v.at.1, FloatRange::Linear { min: -1.0, max: 1.0 }).with_value_to_string(formatters::v2s_f32_rounded(2)),
        }
    }

    pub fn settings(&self) -> HeadSettings {
        HeadSettings {
            striker: self.striker.value(),
            shell_kind: self.shell_kind.value(),
            tune: self.tune.value(),
            decay: self.decay.value(),
            hard: self.hard.value(),
            bend: self.bend.value(),
            diameter: self.diameter.value() * 0.01,
            length: self.length.value() * 0.01,
            thickness: self.thickness.value() * 0.001,
            click: self.click.value(),
            level: util::db_to_gain(self.level.value()),
            pan: self.pan.value(),
            x: self.x.value(),
            y: self.y.value(),
        }
    }
}

#[derive(Params)]
pub struct TympanParams {
    #[persist = "editor-state"]
    pub editor_state: Arc<EguiState>,

    #[nested(array, group = "Drum")]
    pub heads: [HeadParams; HEADS],
    #[nested(array, group = "Plate")]
    pub plates: [MetalParams; METALS],

    /// How much faster the skin's overtones die than its fundamental.
    #[id = "skin"]
    pub skin: FloatParam,
    #[id = "wires"]
    pub wires: FloatParam,
    #[id = "level"]
    pub level: FloatParam,
}

impl Default for TympanParams {
    fn default() -> Self {
        Self {
            editor_state: EguiState::from_size(1000, 818),
            heads: std::array::from_fn(|i| HeadParams::new(&VOICINGS[i])),
            plates: std::array::from_fn(|i| MetalParams::new(PLATES[i].0, PLATES[i].1, PLATES[i].2)),
            skin: FloatParam::new("Skin Damping", Global::default().skin, FloatRange::Linear { min: 0.0, max: 1.0 }).with_value_to_string(formatters::v2s_f32_percentage(0)),
            wires: FloatParam::new("Snare Wires", Global::default().wires, FloatRange::Linear { min: 0.0, max: 1.0 }).with_value_to_string(formatters::v2s_f32_percentage(0)),
            level: FloatParam::new("Level", -10.0, FloatRange::Linear { min: -30.0, max: 6.0 })
                .with_smoother(SmoothingStyle::Linear(20.0))
                .with_unit(" dB")
                .with_value_to_string(formatters::v2s_f32_rounded(1)),
        }
    }
}

impl TympanParams {
    pub fn global(&self) -> Global {
        Global { skin: self.skin.value(), wires: self.wires.value() }
    }
}

impl Default for Tympan {
    fn default() -> Self {
        Self { params: Arc::new(TympanParams::default()), engine: Engine::default(), meters: Arc::new(Meters::default()), seen_audition: [0; HEADS], seen_plate_audition: [0; METALS] }
    }
}

/// A limiter that leaves the signal alone below 0.75 and bends smoothly to a ceiling of 1 above
/// it. The old `tanh` bent everything from the start, so a groove, where voices land together and
/// sum past 1, was distorted by 15 to 24 dB while a single hit was clean; that was why MIDI
/// playing sounded more tonal and worse than clicking in the editor.
pub fn soft_limit(x: f32) -> f32 {
    const KNEE: f32 = 0.75;
    let a = x.abs();
    if a <= KNEE {
        x
    } else {
        x.signum() * (KNEE + (1.0 - KNEE) * ((a - KNEE) / (1.0 - KNEE)).tanh())
    }
}

/// Hits found in one block, in the order they arrive.
struct Hits {
    items: [(u32, u8, f32, Option<(f32, f32)>); 128],
    len: usize,
}

impl Hits {
    fn push(&mut self, timing: u32, head: usize, velocity: f32, at: Option<(f32, f32)>) {
        if self.len < self.items.len() {
            self.items[self.len] = (timing, head as u8, velocity, at);
            self.len += 1;
        }
    }
}

impl Plugin for Tympan {
    const NAME: &'static str = "Tympan Drum Kit";
    const VENDOR: &'static str = "Clowd";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
        AudioIOLayout { main_output_channels: NonZeroU32::new(2), ..AudioIOLayout::const_default() },
        AudioIOLayout { main_output_channels: NonZeroU32::new(1), ..AudioIOLayout::const_default() },
    ];

    const MIDI_INPUT: MidiConfig = MidiConfig::Basic;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        let params = self.params.clone();
        let meters = self.meters.clone();
        create_egui_editor(
            self.params.editor_state.clone(),
            ui::UiState::default(),
            |_, _| {},
            move |ctx, setter, state| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE.fill(ui::background()).inner_margin(14.0))
                    .show(ctx, |ui| ui::draw(ui, &params, &meters, setter, state));
            },
        )
    }

    fn initialize(&mut self, _layout: &AudioIOLayout, config: &BufferConfig, _context: &mut impl InitContext<Self>) -> bool {
        self.engine.set_sample_rate(config.sample_rate);
        true
    }

    fn reset(&mut self) {
        self.engine.stop_all();
    }

    fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, context: &mut impl ProcessContext<Self>) -> ProcessStatus {
        let len = buffer.samples();
        let params = self.params.clone();
        self.engine.set_global(params.global());
        for h in 0..HEADS {
            self.engine.set_head(h, params.heads[h].settings());
        }
        for m in 0..METALS {
            self.engine.set_metal(m, params.plates[m].settings());
        }

        let mut hits = Hits { items: [(0, 0, 0.0, None); 128], len: 0 };
        // Hits asked for from the editor land where the pointer was. They go first, at the start of
        // the block, so the hits stay in time order for the loop below.
        for h in 0..HEADS {
            let a = self.meters.audition[h].load(Ordering::Acquire);
            if a != self.seen_audition[h] {
                self.seen_audition[h] = a;
                let at = (self.meters.audition_at[h][0].load(Ordering::Relaxed), self.meters.audition_at[h][1].load(Ordering::Relaxed));
                hits.push(0, h, self.meters.audition_vel[h].load(Ordering::Relaxed), Some(at));
            }
        }

        for m in 0..METALS {
            let a = self.meters.metal_audition[m].load(Ordering::Acquire);
            if a != self.seen_plate_audition[m] {
                self.seen_plate_audition[m] = a;
                hits.push(0, HEADS + m, self.meters.metal_audition_vel[m].load(Ordering::Relaxed), None);
            }
        }

        while let Some(event) = context.next_event() {
            if let NoteEvent::NoteOn { timing, note, velocity, .. } = event {
                let timing = timing.min(len.saturating_sub(1) as u32);
                if let Some(head) = note_head(note) {
                    hits.push(timing, head, velocity, None);
                } else if let Some(plate) = note_metal(note) {
                    // Plates follow the drums in the hit list. The ride bell, note 53, is the
                    // same plate struck on its bell, which the position carries.
                    hits.push(timing, HEADS + plate, velocity, if note == 53 { Some((1.0, 0.0)) } else { None });
                }
            }
        }
        let mut next = 0;
        let mut peak = 0.0f32;
        for (i, mut frame) in buffer.iter_samples().enumerate() {
            while next < hits.len && hits.items[next].0 as usize <= i {
                let (_, head, velocity, at) = hits.items[next];
                if (head as usize) < HEADS {
                    self.engine.trigger(head as usize, velocity, at);
                    self.meters.hit[head as usize].fetch_max(velocity, Ordering::Relaxed);
                } else {
                    let plate = head as usize - HEADS;
                    self.engine.trigger_metal(plate, velocity, at.is_some());
                    self.meters.metal_hit[plate].fetch_max(velocity, Ordering::Relaxed);
                }
                next += 1;
            }
            let gain = util::db_to_gain(params.level.smoothed.next());
            let (l, r) = self.engine.next();
            let n = frame.len();
            for (ch, sample) in frame.iter_mut().enumerate() {
                let v = if n == 1 { (l + r) * 0.5 } else if ch == 0 { l } else { r };
                // A ceiling for the rare pile-up; everything below it passes untouched.
                let v = soft_limit(v * gain);
                *sample = v;
                peak = peak.max(v.abs());
            }
        }
        self.engine.publish(&self.meters);
        self.meters.out_peak.fetch_max(peak, Ordering::Relaxed);
        if self.engine.is_active() {
            ProcessStatus::KeepAlive
        } else {
            ProcessStatus::Normal
        }
    }
}

impl Vst3Plugin for Tympan {
    const VST3_CLASS_ID: [u8; 16] = *b"ClowdTympanDrums";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Instrument, Vst3SubCategory::Synth, Vst3SubCategory::Drum];
}

nih_export_vst3!(Tympan);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::membrane::tests::{peak_hz, tone_at, FS};

    fn engine() -> Engine {
        let mut e = Engine::default();
        e.set_sample_rate(FS);
        // Most tests are about the instrument, so they hear it dry.
        e.set_global(Global::default());
        e
    }


    fn render(e: &mut Engine, seconds: f32) -> (Vec<f32>, Vec<f32>) {
        (0..(seconds * FS) as usize).map(|_| e.next()).unzip()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    #[test]
    fn general_midi_notes_land_on_the_right_drum() {
        assert_eq!([35, 36, 38, 40, 41, 43, 45, 47, 48, 50].map(note_head), [0, 0, 1, 1, 2, 2, 2, 3, 3, 3].map(Some));
        // Hats and cymbals are not this plugin's business.
        assert_eq!([42, 46, 49, 51, 60].map(note_head), [None; 5]);
    }

    #[test]
    fn every_drum_hits_at_a_similar_healthy_level() {
        let peaks: Vec<f32> = (0..HEADS)
            .map(|h| {
                let mut e = engine();
                for k in 0..HEADS {
                    e.set_head(k, TympanParams::default().heads[k].settings());
                }
                e.trigger(h, 1.0, None);
                let (l, _) = render(&mut e, 0.5);
                l.iter().fold(0.0f32, |p, v| p.max(v.abs()))
            })
            .collect();
        assert!(peaks.iter().all(|p| (0.25..1.0).contains(p)), "peaks per drum: {peaks:?}");
    }

    #[test]
    fn only_the_snare_has_a_hiss_and_it_is_not_louder_than_the_drum() {
        // Energy in the second difference over total energy: a crude measure of the top end.
        let top = |wires: f32, h: usize| {
            let mut e = engine();
            e.set_global(Global { wires, ..Default::default() });
            for k in 0..HEADS {
                e.set_head(k, TympanParams::default().heads[k].settings());
            }
            e.trigger(h, 1.0, None);
            let l = render(&mut e, 0.6).0;
            let hf: Vec<f32> = l.windows(3).map(|w| w[2] - 2.0 * w[1] + w[0]).collect();
            ((hf.iter().map(|v| v * v).sum::<f32>() / l.iter().map(|v| v * v).sum::<f32>()).sqrt(), rms(&l[(0.4 * FS) as usize..]))
        };
        // Wires at their default leave the other three drums as clean as with wires off.
        for h in [0, 2, 3] {
            let (on, _) = top(0.4, h);
            let (off, _) = top(0.0, h);
            assert!(on < off + 0.05, "drum {h}: top end {off} -> {on}");
        }
        // The snare gets its buzz, but it stays a part of the drum rather than a hiss over it.
        let (on, tail_on) = top(0.4, 1);
        let (off, tail_off) = top(0.0, 1);
        assert!(on > off + 0.05 && on < 4.0, "snare top end {off} -> {on}");
        assert!(tail_on < tail_off * 3.0 + 0.01, "the wires must not outlast the drum: tail {tail_off} -> {tail_on}");
    }

    fn plate(e: &mut Engine, which: usize, s: MetalSettings, vel: f32, seconds: f32) -> Vec<f32> {
        e.set_metal(which, s);
        e.trigger_metal(which, vel, false);
        render(e, seconds).0
    }

    /// Top-end share of a signal, by the second difference.
    fn brightness(x: &[f32]) -> f32 {
        let hf: Vec<f32> = x.windows(3).map(|w| w[2] - 2.0 * w[1] + w[0]).collect();
        (hf.iter().map(|v| v * v).sum::<f32>() / x.iter().map(|v| v * v).sum::<f32>()).sqrt()
    }

    #[test]
    fn hats_and_cymbals_have_their_general_midi_notes() {
        assert_eq!([42, 44, 46, 49, 51, 52, 53, 55, 57, 59].map(note_metal), [0, 0, 1, 2, 3, 2, 3, 2, 2, 3].map(Some));
        // Nothing is claimed by both a drum and a plate, and Pocket's own notes all land somewhere
        // except the perc lane (56).
        for n in 0..128u8 {
            assert!(!(note_head(n).is_some() && note_metal(n).is_some()), "note {n}");
        }
        for pocket in [36, 38, 37, 39, 42, 46, 45, 50, 49, 51] {
            assert!(note_head(pocket).is_some() || note_metal(pocket).is_some(), "note {pocket} is silent");
        }
    }

    #[test]
    fn the_striker_changes_the_attack_the_way_its_material_would() {
        let hit = |striker: Striker, hard: f32| {
            let mut e = engine();
            e.set_global(Global { wires: 0.0, ..Default::default() });
                        e.set_head(2, HeadSettings { striker, hard, tune: 120.0, decay: 0.5, shell_kind: ShellKind::None, click: 1.0, ..Default::default() });
            e.trigger(2, 0.9, None);
            render(&mut e, 0.4).0
        };
        let (stick, beater, felt, hand) = (hit(Striker::Stick, 0.6), hit(Striker::Beater, 0.6), hit(Striker::Felt, 0.6), hit(Striker::Hand, 0.6));
        // The softer the striker, the less top end the same hit has.
        // The energy between 1 and 8 kHz in the first 100 ms: the crack of the strike.
        let b = |x: &[f32]| {
            let p = analysis::power_spectrum(x, 0, 4800, 8192);
            let bin = |hz: f32| (hz * 8192.0 / FS) as usize;
            p[bin(1000.0)..bin(8000.0)].iter().sum::<f32>()
        };
        assert!(b(&stick) > b(&beater) * 1.1, "stick {} beater {}", b(&stick), b(&beater));
        assert!(b(&stick) > b(&felt) * 1.1, "stick {} felt {}", b(&stick), b(&felt));
        assert!(b(&stick) > b(&hand) * 1.1, "stick {} hand {}", b(&stick), b(&hand));
        // And every one of them still makes a sound.
        assert!([&stick, &beater, &felt, &hand].iter().all(|x| rms(&x[..4800]) > 0.005));
    }

    #[test]
    fn a_metal_shell_rings_on_and_no_shell_does_not() {
        // How much of the early shell sound is left late: a longer ring keeps more of it.
        let keeps = |kind: ShellKind| {
            let mut e = engine();
            e.set_global(Global { wires: 0.0, ..Default::default() });
            // A steel shell is thin; 7 mm of steel would put its modes far above what is sounded.
            let thickness = if kind == ShellKind::Metal { 0.0012 } else { 0.007 };
            e.set_head(2, HeadSettings { shell_kind: kind, thickness, tune: 100.0, decay: 0.03, ..Default::default() });
            e.trigger(2, 1.0, None);
            let l = render(&mut e, 1.2).0;
            let at = |a: f32, b: f32| rms(&l[(a * FS) as usize..(b * FS) as usize]);
            (at(0.7, 1.1) / at(0.1, 0.3).max(1e-9), at(0.1, 0.3))
        };
        let ((wood, wood_level), (metal, _), (_, none_level)) = (keeps(ShellKind::Wood), keeps(ShellKind::Metal), keeps(ShellKind::None));
        assert!(metal > wood * 2.0, "metal keeps {metal} wood {wood}");
        assert!(none_level < wood_level * 0.2, "no shell {none_level} wood {wood_level}");
    }

    /// How line-like a stretch of sound is: the loudest 1 percent of its spectrum over the typical
    /// level, between 1 and 10 kHz. White noise gives about 6; a few steady partials give
    /// hundreds, and are heard as a pitch.
    fn lines(x: &[f32], from: f32, len: f32) -> f32 {
        let p = analysis::power_spectrum(x, (from * FS) as usize, (len * FS) as usize, 1 << 16);
        let bin = |hz: f32| (hz * (1 << 16) as f32 / FS) as usize;
        let mut band: Vec<f32> = p[bin(1000.0)..bin(10000.0)].to_vec();
        band.sort_by(|a, b| a.total_cmp(b));
        band[band.len() * 99 / 100] / band[band.len() / 2].max(1e-30)
    }

    fn default_plate(which: usize) -> MetalSettings {
        TympanParams::default().plates[which].settings()
    }

    fn rendered_plate(which: usize, s: MetalSettings, bell: bool, seconds: f32) -> Vec<f32> {
        let mut e = engine();
        e.set_metal(which, s);
        e.trigger_metal(which, 0.9, bell);
        render(&mut e, seconds).0
    }

    #[test]
    fn a_plates_modes_follow_its_geometry() {
        let hz = |s: MetalSettings| metal::mode_frequencies(&engine::METAL_DESIGN[2], &s, FS);
        let base = default_plate(2);
        let (b, thick, wide) = (hz(base), hz(MetalSettings { thickness: base.thickness * 2.0, ..base }), hz(MetalSettings { diameter: base.diameter * 1.5, ..base }));
        // Plenty of modes, the way a real cymbal has hundreds (Rossing and Wilbur counted about
        // 300 in a 16 inch crash).
        assert!(b.len() > 300, "{} modes", b.len());
        // Thicker is higher and wider is lower, by the plate laws: a mode a good way above the
        // bow's floor goes as thickness / diameter^2.
        let at = |v: &[f32], i: usize| v[i];
        let i = 120;
        assert!(at(&thick, i) > at(&b, i) * 1.5, "twice as thick {} vs {}", at(&thick, i), at(&b, i));
        assert!(at(&wide, i) < at(&b, i) * 0.85, "wider {} vs {}", at(&wide, i), at(&b, i));
    }

    #[test]
    fn the_modes_cluster_at_the_bottom_and_merge_into_a_band_above() {
        // Few, well separated modes at the bottom are heard as tones; higher up the clusters
        // overlap and the spacing between neighbours is small next to the frequency.
        let f = metal::mode_frequencies(&engine::METAL_DESIGN[3], &default_plate(3), FS);
        let gap = |lo: f32, hi: f32| {
            let band: Vec<f32> = f.iter().cloned().filter(|x| (lo..hi).contains(x)).collect();
            let sum: f32 = band.windows(2).map(|w| w[1] - w[0]).sum();
            sum / (band.len().max(2) - 1) as f32 / ((lo + hi) / 2.0)
        };
        assert!(gap(200.0, 800.0) > gap(4000.0, 8000.0) * 5.0, "{} vs {}", gap(200.0, 800.0), gap(4000.0, 8000.0));
    }

    #[test]
    fn a_ride_is_tuned_and_a_hat_is_noise_and_a_closed_hat_is_short() {
        let hat = rendered_plate(0, default_plate(0), false, 1.5);
        let open = rendered_plate(1, default_plate(1), false, 2.0);
        let ride = rendered_plate(3, default_plate(3), false, 3.0);
        // The recorded rides are strongly line-like (hundreds and more on this measure, against 6
        // for white noise) and the recorded hats moderately (about 35).
        let (hat_l, ride_l) = (lines(&hat, 0.01, 0.06), lines(&ride, 0.3, 0.5));
        assert!(ride_l > hat_l * 3.0, "ride {ride_l} hat {hat_l}");
        assert!(hat_l < 80.0, "the hat is not a chord: {hat_l}");
        // Pressed together the hat dies in a tenth of a second or so; free, it rings.
        let tail = |x: &[f32]| rms(&x[(0.3 * FS) as usize..(0.5 * FS) as usize]) / rms(&x[..4800]);
        assert!(tail(&hat) < 0.02, "closed hat tail {}", tail(&hat));
        assert!(tail(&open) > 0.03, "open hat tail {}", tail(&open));
    }

    /// An open hat should sound like metal, not like noise: in the recording the spectrum between 3
    /// and 12 kHz has peaks 15 dB above its valleys (at 100 Hz resolution), it is flat across the
    /// band, and the top does not pile up with energy pushed up by the wash.
    #[test]
    fn the_open_hat_is_metal_not_noise() {
        let mut e = engine();
        e.set_metal(1, default_plate(1));
        e.trigger_metal(1, 0.9, false);
        let x = render(&mut e, 0.6).0;
        let n = 1 << 16;
        let p = analysis::power_spectrum(&x, (0.05 * FS) as usize, (0.35 * FS) as usize, n);
        let bin = |hz: f32| (hz * n as f32 / FS) as usize;
        let k = (100.0 * n as f32 / FS) as usize;
        let smooth: Vec<f32> = p[bin(3000.0)..bin(12000.0)].chunks(k).map(|c| c.iter().sum::<f32>() / c.len() as f32).collect();
        let mut sorted = smooth.clone();
        sorted.sort_by(|a, b| a.total_cmp(b));
        let spread = 10.0 * (sorted[sorted.len() * 95 / 100] / sorted[sorted.len() * 5 / 100]).log10();
        assert!(spread > 10.0, "peak to valley {spread:.1} dB: a flat spectrum is noise");
        let band = |a: f32, b: f32| p[bin(a)..bin(b)].iter().sum::<f32>() / (bin(b) - bin(a)) as f32;
        let top = band(13000.0, 14000.0);
        let body = band(3000.0, 12000.0);
        assert!(top < body * 2.0, "energy piles up at 13 to 14 kHz: {top} against {body}");
    }

    #[test]
    fn the_bell_rings_the_ping() {
        let top = |x: &[f32]| {
            let p = analysis::power_spectrum(x, (0.05 * FS) as usize, (0.3 * FS) as usize, 1 << 16);
            let bin = |hz: f32| (hz * (1 << 16) as f32 / FS) as usize;
            p[bin(2500.0)..bin(6000.0)].iter().sum::<f32>() / p[bin(150.0)..bin(10000.0)].iter().sum::<f32>()
        };
        // Without the wash, which would spread the bell's energy over the spectrum.
        let render_ride = |bell: bool| {
            let mut e = engine();
            e.set_metal_design(3, metal::PlateDesign { wash: 0.0, ..engine::METAL_DESIGN[3] });
            e.set_metal(3, default_plate(3));
            e.trigger_metal(3, 0.9, bell);
            render(&mut e, 1.0).0
        };
        let (on_bow, on_bell) = (render_ride(false), render_ride(true));
        assert!(top(&on_bell) > top(&on_bow) * 1.3, "bell {} bow {}", top(&on_bell), top(&on_bow));
    }

    #[test]
    fn a_thicker_plate_has_fewer_modes_and_a_bigger_one_has_more() {
        // The density of a plate's modes is its area over twice its bending-wave constant, so a
        // thick plate is sparser (and rings more like a bell) and a wide one denser.
        let count = |s: MetalSettings| metal::mode_frequencies(&engine::METAL_DESIGN[2], &s, FS).len();
        let base = default_plate(2);
        assert!(count(MetalSettings { thickness: base.thickness * 2.0, ..base }) < count(base) * 8 / 10);
        assert!(count(MetalSettings { diameter: base.diameter * 1.3, ..base }) > count(base) * 12 / 10);
    }

    #[test]
    fn the_closed_hat_chokes_the_open_hat() {
        let tail_after = |choke: bool| {
            let mut e = engine();
            e.set_metal(0, MetalSettings { level: 0.0, ..default_plate(0) });
            e.set_metal(1, default_plate(1));
            e.trigger_metal(1, 1.0, false);
            render(&mut e, 0.1);
            if choke {
                e.trigger_metal(0, 1.0, false);
            }
            let x = render(&mut e, 0.3).0;
            rms(&x[(0.15 * FS) as usize..])
        };
        assert!(tail_after(true) < tail_after(false) * 0.1, "{} vs {}", tail_after(true), tail_after(false));
    }

    #[test]
    fn a_harder_stick_is_brighter() {
        let bright = |tone: f32| analysis::features(&rendered_plate(3, MetalSettings { tone, ..default_plate(3) }, false, 0.4), FS).centroid;
        assert!(bright(1.0) > bright(0.0) * 1.1, "{} vs {}", bright(1.0), bright(0.0));
    }

    #[test]
    fn plates_hit_at_a_healthy_level_and_stay_finite() {
        for which in 0..METALS {
            let l = rendered_plate(which, default_plate(which), false, 0.5);
            let peak = l.iter().fold(0.0f32, |p, v| p.max(v.abs()));
            let early = rms(&l[..4800]);
            assert!(l.iter().all(|v| v.is_finite()) && peak < 1.0 && (0.04..0.3).contains(&early), "plate {which}: peak {peak} early rms {early}");
        }
        let mut e = engine();
        for k in 0..METALS {
            e.set_metal(k, MetalSettings { diameter: 0.7, thickness: 0.003, tone: 1.0, level: 2.0 });
        }
        for k in 0..40 {
            e.trigger_metal(k % METALS, 1.0, k % 3 == 0);
            let (l, r) = render(&mut e, 0.05);
            assert!(l.iter().chain(&r).all(|v| v.is_finite() && v.abs() < 50.0));
        }
    }

    #[test]
    fn plates_rest_when_they_have_rung_out() {
        let mut e = engine();
        e.set_metal(0, default_plate(0));
        e.trigger_metal(0, 1.0, false);
        render(&mut e, 2.0);
        assert!(!e.is_active());
    }

    /// The crash's wash: the plate's nonlinearity moves energy up the spectrum and blurs its
    /// discrete lines into a dense wash, more for a hard hit than a soft one, and the total energy
    /// never grows.
    #[test]
    fn the_wash_blurs_the_lines_with_the_strength_of_the_hit_and_conserves_energy() {
        let run = |wash: f32, vel: f32| {
            let mut e = engine();
            e.set_metal_design(2, metal::PlateDesign { wash, ..engine::METAL_DESIGN[2] });
            e.set_metal(2, default_plate(2));
            e.trigger_metal(2, vel, false);
            let x = render(&mut e, 3.0).0;
            (lines(&x, 0.6, 0.5), x)
        };
        let (dry, dry_x) = run(0.0, 0.9);
        let (washed, washed_x) = run(8.0, 0.9);
        assert!(washed < dry * 0.6, "lines {washed} with the wash, {dry} without");
        // A tap is hardly touched by it: the nonlinearity needs a real swing.
        let (tap_dry, _) = run(0.0, 0.03);
        let (tap_washed, _) = run(8.0, 0.03);
        assert!(tap_washed / tap_dry > washed / dry * 1.2, "a tap is blurred less: x{} against x{}", tap_washed / tap_dry, washed / dry);
        assert!(washed_x.iter().all(|v| v.is_finite()));
        // Energy only moves: the washed crash is not louder overall than the dry one.
        let energy = |x: &[f32]| x.iter().map(|v| v * v).sum::<f32>();
        assert!(energy(&washed_x) < energy(&dry_x) * 1.5);
    }

    #[test]
    fn the_limiter_is_transparent_until_it_is_needed() {
        // Linear below the knee, monotonic, continuous, and never above 1.
        for i in -75..=75 {
            let x = i as f32 / 100.0;
            assert_eq!(soft_limit(x), x);
        }
        let mut last = 0.0;
        for i in 0..400 {
            let y = soft_limit(i as f32 / 100.0);
            assert!(y >= last && y <= 1.0, "{y}");
            assert!((y - last).abs() < 0.02, "a jump at {i}");
            last = y;
        }
        assert_eq!(soft_limit(-0.5), -0.5);
        assert!(soft_limit(-3.0) >= -1.0 && soft_limit(-3.0) < -0.99);
    }

    /// A groove lands several voices together. At the default master level they must sum to well
    /// under the limiter's knee, or every groove is distorted and a single hit never is.
    #[test]
    fn a_groove_has_headroom_at_the_default_level() {
        let mut e = Engine::default();
        e.set_sample_rate(FS);
        e.set_global(TympanParams::default().global());
        for h in 0..HEADS {
            e.set_head(h, TympanParams::default().heads[h].settings());
        }
        for m in 0..METALS {
            e.set_metal(m, TympanParams::default().plates[m].settings());
        }
        let gain = util::db_to_gain(TympanParams::default().level.default_plain_value());
        for (name, drums, plates) in [("kick+snare+hat", vec![0, 1], vec![0]), ("kick+snare+crash", vec![0, 1], vec![2]), ("kick+snare+tom+ride", vec![0, 1, 2], vec![3])] {
            e.stop_all();
            for d in &drums {
                e.trigger(*d, 0.8, None);
            }
            for p in &plates {
                e.trigger_metal(*p, 0.8, false);
            }
            let peak = (0..(0.5 * FS) as usize).map(|_| { let (l, r) = e.next(); ((l + r) * 0.5).abs() }).fold(0.0f32, f32::max) * gain;
            assert!(peak < 0.75, "{name} peaks at {peak}, into the limiter");
        }
    }

    /// A drum is its body as much as its attack: the shell, the air and the second head keep
    /// ringing after the stick has gone. Levels are in dB, RMS, of a velocity 0.8 hit.
    #[test]
    fn the_body_keeps_ringing_after_the_hit() {
        for (h, window, floor) in [(0usize, (0.15, 0.3), -33.0f32), (1, (0.12, 0.25), -45.0), (2, (0.15, 0.3), -34.0), (3, (0.15, 0.3), -29.0)] {
            let mut e = Engine::default();
            e.set_sample_rate(FS);
            e.set_global(TympanParams::default().global());
            e.set_head(h, TympanParams::default().heads[h].settings());
            e.trigger(h, 0.8, None);
            let x: Vec<f32> = (0..(0.5 * FS) as usize).map(|_| { let (l, r) = e.next(); (l + r) * 0.5 }).collect();
            let db = 20.0 * rms(&x[(window.0 * FS) as usize..(window.1 * FS) as usize]).log10();
            assert!(db > floor, "drum {h} has died to {db:.1} dB by {} ms", window.0 * 1000.0);
        }
    }

    /// A host resets the plug-in on every transport start. The drums must sound the same after it:
    /// a reset once left them untuned, so MIDI after pressing play was tonal and wrong while a click
    /// in the editor (which changes a setting and so retunes) was fine.
    #[test]
    fn a_reset_does_not_change_the_sound() {
        for h in 0..HEADS {
            let hit = |reset: bool| {
                let mut e = Engine::default();
                e.set_sample_rate(FS);
                e.set_global(TympanParams::default().global());
                e.set_head(h, TympanParams::default().heads[h].settings());
                if reset {
                    e.stop_all();
                    // The host sends the same settings again; nothing has changed.
                    e.set_head(h, TympanParams::default().heads[h].settings());
                }
                e.trigger(h, 0.8, None);
                render(&mut e, 0.3).0
            };
            let (a, b) = (hit(false), hit(true));
            let err: f32 = a.iter().zip(&b).map(|(x, y)| (x - y).powi(2)).sum();
            let sig: f32 = a.iter().map(|x| x * x).sum();
            assert!(err < sig * 1e-6, "drum {h} differs after a reset: {}", err / sig);
        }
    }

    #[test]
    fn silence_in_silence_out() {
        let mut e = engine();
        let (l, r) = render(&mut e, 0.5);
        assert!(l.iter().chain(&r).all(|v| *v == 0.0));
        assert!(!e.is_active());
    }

    #[test]
    fn each_drum_sounds_at_its_own_pitch() {
        for (head, hz) in [(0, 58.0), (1, 190.0), (2, 105.0), (3, 165.0)] {
            let mut e = engine();
            // A soft, centred strike, so the fundamental dominates; no shell or wires.
            // A mallet that is on the head for less than half a period, or the fundamental is not set
            // going: a slow push is a thud.
            let s = HeadSettings { tune: hz, decay: 1.5, hard: 0.6, bend: 0.0, shell_kind: ShellKind::None, x: 0.0, y: 0.0, ..Default::default() };
                        e.set_global(Global { wires: 0.0, ..Default::default() });
            e.set_head(head, s);
            e.trigger(head, 1.0, None);
            let (l, r) = render(&mut e, 1.0);
            let mono: Vec<f32> = l.iter().zip(&r).map(|(a, b)| (a + b) * 0.5).collect();
            let f = peak_hz(&mono, hz * 0.6, hz * 1.5);
            assert!((f - hz).abs() / hz < 0.05, "drum {head} should ring at {hz} Hz, got {f}");
            for other in (0..HEADS).filter(|o| *o != head) {
                assert_eq!(e.energy(other), 0.0, "hitting drum {head} must not move drum {other}");
            }
        }
    }

    #[test]
    fn the_wires_buzz_with_the_snare_and_stay_shut_without_it() {
        let wire_band = |amount: f32, head: usize| {
            let mut e = engine();
            e.set_global(Global { wires: amount, ..Default::default() });
            e.set_head(head, HeadSettings { tune: 190.0, decay: 0.3, shell_kind: ShellKind::None, ..Default::default() });
            e.trigger(head, 1.0, None);
            let (l, _) = render(&mut e, 0.3);
            // Energy above 3 kHz is the wires; the head has none.
            let hf: Vec<f32> = l.windows(2).map(|w| w[1] - w[0]).collect();
            rms(&hf)
        };
        // The snare's own dense high modes are most of its top end, so the wires add to it.
        assert!(wire_band(1.0, 1) > wire_band(0.0, 1) * 1.15, "{} vs {}", wire_band(1.0, 1), wire_band(0.0, 1));
        // A tom hit rattles the wires, but only a little.
        let tom_rattle = wire_band(1.0, 2) - wire_band(0.0, 2);
        assert!(tom_rattle < (wire_band(1.0, 1) - wire_band(0.0, 1)) * 0.6, "tom rattle {tom_rattle}");
    }

    #[test]
    fn velocity_and_level_scale_the_loudness() {
        let loud = |v: f32, level: f32| {
            let mut e = engine();
            e.set_head(1, HeadSettings { level, ..Default::default() });
            e.trigger(1, v, None);
            rms(&render(&mut e, 0.2).0)
        };
        assert!(loud(1.0, 1.0) > loud(0.4, 1.0) * 1.8);
        assert!((loud(1.0, 0.5) / loud(1.0, 1.0) - 0.5).abs() < 0.08);
    }

    #[test]
    fn a_pan_moves_the_kit_without_losing_the_near_side() {
        let side = |pan: f32| {
            let mut e = engine();
            e.set_head(0, HeadSettings { pan, shell_kind: ShellKind::None, ..Default::default() });
            e.set_global(Global { wires: 0.0, ..Default::default() });
            e.trigger(0, 1.0, None);
            let (l, r) = render(&mut e, 0.3);
            (rms(&l), rms(&r))
        };
        let (l, r) = side(-1.0);
        assert!(l > r * 20.0, "hard left: {l} {r}");
        let (l, r) = side(1.0);
        assert!(r > l * 20.0);
    }

    #[test]
    fn it_stays_finite_under_a_flurry_of_extreme_hits() {
        let mut e = engine();
        e.set_global(Global { skin: 1.0, wires: 1.0 });
        for h in 0..HEADS {
            e.set_head(h, HeadSettings { tune: [30.0, 600.0, 45.0, 400.0][h], decay: 3.0, hard: 1.0, bend: 1.0, ..Default::default() });
        }
        let mut peak = 0.0f32;
        for k in 0..40 {
            e.trigger(k % HEADS, 1.0, Some((((k * 7) % 11) as f32 / 6.0 - 0.9, 0.3)));
            let (l, r) = render(&mut e, 0.05);
            assert!(l.iter().chain(&r).all(|v| v.is_finite()));
            peak = l.iter().chain(&r).fold(peak, |p, v| p.max(v.abs()));
        }
        assert!(peak < 200.0, "peak {peak}");
    }

    #[test]
    fn the_editor_sees_the_head_move_and_then_rest() {
        let mut e = engine();
        let meters = Meters::default();
        e.set_head(0, HeadSettings { decay: 0.05, ..Default::default() });
        e.trigger(0, 1.0, None);
        for _ in 0..200 {
            e.next();
        }
        e.publish(&meters);
        let seen: f32 = meters.head_grid(0, 1).iter().map(|v| v.load(Ordering::Relaxed).abs()).sum();
        assert!(seen > 0.1, "the grid is published");
        // The shell goes on ringing, and pushing on the head, after the head itself is damped.
        render(&mut e, 5.0);
        e.publish(&meters);
        e.publish(&meters);
        let after: f32 = meters.head_grid(0, 1).iter().map(|v| v.load(Ordering::Relaxed).abs()).sum();
        assert_eq!(after, 0.0, "and published at rest");
    }

    #[test]
    fn a_hit_from_the_editor_lands_where_it_was_asked() {
        let mut e = engine();
        e.set_head(0, HeadSettings { tune: 100.0, decay: 2.0, hard: 0.8, shell_kind: ShellKind::None, x: 0.0, y: 0.0, ..Default::default() });
        // At the rim the (1,1) mode at 159 Hz is strong; at the centre it is not.
        let strike = |at: (f32, f32)| {
            let mut e = engine();
                        e.set_head(0, HeadSettings { tune: 100.0, decay: 2.0, hard: 0.8, shell_kind: ShellKind::None, ..Default::default() });
            e.trigger(0, 1.0, Some(at));
            let l = render(&mut e, 1.0).0;
            tone_at(&l, 159.4) / tone_at(&l, 100.0)
        };
        assert!(strike((0.7, 0.0)) > strike((0.0, 0.0)) * 1.4, "rim {} centre {}", strike((0.7, 0.0)), strike((0.0, 0.0)));
        let _ = &mut e;
    }

    #[test]
    fn the_factory_presets_are_valid() {
        presets::BANK.validate(&TympanParams::default()).unwrap();
    }
}
