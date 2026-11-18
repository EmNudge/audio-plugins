use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, egui, EguiState};
use std::sync::atomic::Ordering;
use std::sync::Arc;

pub mod browser;
pub mod drag;
pub mod electro;
pub mod engine;
pub mod generator;
pub mod grooves;
pub mod import;
pub mod layered;
pub mod midi;
pub mod pattern;
pub mod reverb;
pub mod samples;
pub mod synth;
pub mod ui;

use engine::{Event, EventBuf, Sequencer, Transport};
use generator::Style;
use pattern::{Feel, NoteMap, PatternStore};
use samples::{KitShare, SampleKit};
use synth::KitKind;

/// Keys in the top octave that run the sequencer, as numbered from C7: play/stop, record, roll a
/// new groove, a fill, undo the last take. They are kept out of the drum and thru streams.
pub const CONTROL_BASE: u8 = 96;
pub const CONTROL_KEYS: u8 = 12;
/// One key per drum from C6 up, each toggling that drum's mute.
pub const MUTE_BASE: u8 = 84;

/// Work started from the audio thread that must not run on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Task {
    /// Remember the pattern so a take can be undone.
    Snapshot,
    Undo,
    /// Roll the generator with a fresh seed.
    Roll,
    /// Put a random fill on the end.
    Fill,
}

/// Outgoing notes for a block, sorted before they are sent so the host sees them in time order.
struct OutNotes {
    items: [(u32, bool, u8, u8, f32); 640],
    len: usize,
}

impl Default for OutNotes {
    fn default() -> Self {
        Self { items: [(0, false, 0, 0, 0.0); 640], len: 0 }
    }
}

impl OutNotes {
    fn push(&mut self, timing: u32, on: bool, channel: u8, note: u8, velocity: f32) {
        if self.len < self.items.len() {
            self.items[self.len] = (timing, on, channel, note, velocity);
            self.len += 1;
        }
    }

    fn sort(&mut self) {
        self.items[..self.len].sort_unstable_by(|a, b| (a.0, a.1, a.3).cmp(&(b.0, b.1, b.3)));
    }
}

pub struct Pocket {
    params: Arc<PocketParams>,
    seq: Sequencer,
    kit: synth::Kit,
    events: EventBuf,
    played: EventBuf,
    out: OutNotes,
    /// The host gave an audio input, so the buffer arrives holding the previous plugin's sound.
    has_input: bool,
    sample_rate: f64,
    last_tempo: f64,
    /// A replaced sample kit waiting for somewhere it can be freed off the audio thread.
    parked: Option<Arc<SampleKit>>,
}

/// Park a sample kit the audio thread has finished with, without freeing it here.
fn retire(share: &KitShare, parked: &mut Option<Arc<SampleKit>>, old: Arc<SampleKit>) {
    let leftover = match share.retired.try_lock() {
        Ok(mut list) if list.len() < list.capacity() => {
            list.push(old);
            None
        }
        _ => Some(old),
    };
    match (parked.is_none(), leftover) {
        (true, leftover) => *parked = leftover,
        // Nowhere to put it: leaking a kit once in a long while beats freeing on the audio thread.
        (false, Some(extra)) => std::mem::forget(extra),
        (false, None) => {}
    }
}

#[derive(Params)]
pub struct PocketParams {
    #[persist = "editor-state"]
    pub editor_state: Arc<EguiState>,
    #[persist = "pattern"]
    pub pattern: PatternStore,
    /// Folders of MIDI files the browser lists under MY MIDI.
    #[persist = "folders"]
    pub folders: std::sync::Mutex<Vec<String>>,

    #[id = "style"]
    pub style: EnumParam<Style>,
    #[id = "complex"]
    pub complexity: FloatParam,
    #[id = "dynamics"]
    pub dynamics: FloatParam,
    #[id = "fill"]
    pub fill: FloatParam,
    #[id = "seed"]
    pub seed: IntParam,

    #[id = "swing"]
    pub swing: FloatParam,
    #[id = "swing8"]
    pub swing_eighths: BoolParam,
    #[id = "human"]
    pub humanize: FloatParam,

    #[id = "map"]
    pub map: EnumParam<NoteMap>,
    #[id = "kit"]
    pub kit: EnumParam<KitKind>,
    #[id = "room"]
    pub room: FloatParam,
    #[id = "tune"]
    pub tune: FloatParam,
    #[id = "decay"]
    pub decay: FloatParam,
    /// The folder the My Samples kit loads from.
    #[persist = "kit-folder"]
    pub kit_folder: std::sync::Mutex<String>,
    #[id = "sound"]
    pub sound: BoolParam,
    /// Pass notes played into Pocket on to whatever follows it.
    #[id = "thru"]
    pub thru: BoolParam,
    #[id = "level"]
    pub level: FloatParam,
}

impl Default for PocketParams {
    fn default() -> Self {
        let pattern = PatternStore::default();
        if let Some(g) = grooves::library().iter().find(|g| g.name == "Backbeat") {
            g.load(&pattern, false);
        }
        let unit = |name: &'static str, default: f32| {
            FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_unit("%")
                .with_value_to_string(formatters::v2s_f32_percentage(0))
                .with_string_to_value(formatters::s2v_f32_percentage())
        };
        Self {
            editor_state: EguiState::from_size(1080, 748),
            pattern,
            folders: Default::default(),
            style: EnumParam::new("Style", Style::Rock),
            complexity: unit("Complexity", 0.4),
            dynamics: unit("Dynamics", 0.5),
            fill: unit("Fill", 0.0),
            seed: IntParam::new("Seed", 1, IntRange::Linear { min: 1, max: 9999 }),
            swing: FloatParam::new("Swing", 0.5, FloatRange::Linear { min: 0.5, max: 0.75 })
                .with_value_to_string(Arc::new(|v| format!("{:.0}%", v * 100.0)))
                .with_string_to_value(Arc::new(|s| s.trim().trim_end_matches('%').trim().parse::<f32>().ok().map(|v| v / 100.0))),
            swing_eighths: BoolParam::new("Swing Eighths", false),
            humanize: unit("Humanize", 0.0),
            map: EnumParam::new("Note Map", NoteMap::Gm),
            kit: EnumParam::new("Kit", KitKind::Studio),
            room: unit("Room", 0.3),
            tune: FloatParam::new("Tune", 0.0, FloatRange::Linear { min: -12.0, max: 12.0 })
                .with_unit(" st")
                .with_value_to_string(Arc::new(|v| format!("{v:+.1}"))),
            decay: FloatParam::new("Decay", 1.0, FloatRange::Skewed { min: 0.3, max: 3.0, factor: FloatRange::skew_factor(-1.0) })
                .with_value_to_string(Arc::new(|v| format!("{:.0}%", v * 100.0))),
            kit_folder: Default::default(),
            sound: BoolParam::new("Audition Sound", true),
            thru: BoolParam::new("MIDI Thru", true),
            level: FloatParam::new("Level", -6.0, FloatRange::Linear { min: -30.0, max: 6.0 })
                .with_smoother(SmoothingStyle::Linear(20.0))
                .with_unit(" dB")
                .with_value_to_string(formatters::v2s_f32_rounded(1)),
        }
    }
}

impl PocketParams {
    pub fn feel(&self) -> Feel {
        Feel { swing: self.swing.value(), eighths: self.swing_eighths.value(), humanize: self.humanize.value() }
    }

    pub fn generator(&self) -> generator::Settings {
        generator::Settings {
            style: self.style.value(),
            complexity: self.complexity.value(),
            dynamics: self.dynamics.value(),
            fill: self.fill.value(),
            seed: self.seed.value() as u32,
        }
    }
}

impl Default for Pocket {
    fn default() -> Self {
        Self {
            params: Arc::new(PocketParams::default()),
            seq: Sequencer::default(),
            kit: synth::Kit::default(),
            events: EventBuf::default(),
            played: EventBuf::default(),
            out: OutNotes::default(),
            has_input: false,
            sample_rate: 44100.0,
            last_tempo: 120.0,
            parked: None,
        }
    }
}

impl Plugin for Pocket {
    const NAME: &'static str = "Pocket Groove Sequencer";
    const VENDOR: &'static str = "Clowd";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    // With an audio input first, so a host that offers it lets whatever is before Pocket in the
    // chain keep sounding; the kit is added on top. Output-only layouts follow for hosts that
    // load it as a plain instrument.
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
        AudioIOLayout { main_input_channels: NonZeroU32::new(2), main_output_channels: NonZeroU32::new(2), ..AudioIOLayout::const_default() },
        AudioIOLayout { main_input_channels: NonZeroU32::new(1), main_output_channels: NonZeroU32::new(1), ..AudioIOLayout::const_default() },
        AudioIOLayout { main_output_channels: NonZeroU32::new(2), ..AudioIOLayout::const_default() },
        AudioIOLayout { main_output_channels: NonZeroU32::new(1), ..AudioIOLayout::const_default() },
    ];

    const MIDI_INPUT: MidiConfig = MidiConfig::Basic;
    const MIDI_OUTPUT: MidiConfig = MidiConfig::Basic;

    type SysExMessage = ();
    type BackgroundTask = Task;

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn task_executor(&mut self) -> TaskExecutor<Self> {
        let params = self.params.clone();
        let meter = self.seq.meter.clone();
        Box::new(move |task| match task {
            Task::Snapshot => {
                if let Ok(mut undo) = meter.undo.lock() {
                    *undo = Some(params.pattern.save());
                }
            }
            Task::Undo => {
                let saved = meter.undo.lock().ok().and_then(|mut u| u.take());
                if let Some(saved) = saved {
                    params.pattern.load(&saved);
                }
            }
            Task::Roll => {
                let bump = meter.seed_bump.fetch_add(1, Ordering::Relaxed) + 1;
                let mut g = params.generator();
                g.seed = (g.seed + bump.wrapping_mul(7919)) % 9999 + 1;
                generator::generate(&params.pattern, &g);
            }
            Task::Fill => {
                let bump = meter.seed_bump.fetch_add(1, Ordering::Relaxed) + 1;
                let fills: Vec<_> = grooves::library().iter().filter(|g| g.kind == grooves::Kind::Fill).collect();
                if !fills.is_empty() {
                    let pick = (pattern::hash01(bump, 31, 7) * fills.len() as f32) as usize % fills.len();
                    fills[pick].load(&params.pattern, true);
                }
            }
        })
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        let params = self.params.clone();
        let meter = self.seq.meter.clone();
        create_egui_editor(
            self.params.editor_state.clone(),
            ui::UiState::default(),
            |_, _| {},
            move |ctx, setter, state| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE.fill(ui::background()).inner_margin(14.0))
                    .show(ctx, |ui| ui::draw(ui, &params, setter, &meter, state));
            },
        )
    }

    fn initialize(&mut self, layout: &AudioIOLayout, config: &BufferConfig, _context: &mut impl InitContext<Self>) -> bool {
        self.sample_rate = config.sample_rate as f64;
        self.has_input = layout.main_input_channels.is_some();
        self.kit.set_sample_rate(config.sample_rate);
        let share = &self.seq.meter.samples;
        share.sample_rate.store(config.sample_rate as u32, Ordering::Relaxed);
        // Reload the folder the last session used, resampled for this host.
        let folder = self.params.kit_folder.lock().map(|f| f.clone()).unwrap_or_default();
        if !folder.is_empty() {
            share.spawn_load(folder.into());
        }
        true
    }

    fn reset(&mut self) {
        self.seq.reset();
        self.kit.reset();
    }

    fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, context: &mut impl ProcessContext<Self>) -> ProcessStatus {
        let len = buffer.samples();
        let transport = context.transport();
        if let Some(t) = transport.tempo {
            self.last_tempo = t;
        }
        let tr = Transport {
            host: if transport.playing { transport.pos_beats() } else { None },
            audition: self.seq.meter.audition.load(Ordering::Relaxed),
            tempo: self.last_tempo,
        };
        let params = self.params.clone();
        self.seq.process(&params.pattern, params.feel(), tr, self.sample_rate, len, &mut self.events);

        let map = params.map.value();
        let feel = params.feel();
        let thru = params.thru.value();
        let meter = self.seq.meter.clone();
        self.out.len = 0;
        for e in self.events.as_slice() {
            self.out.push(e.timing, e.on, 9, map.note(e.lane as usize), e.vel as f32 / 127.0);
        }

        // Notes played into the plugin: they sound the kit, can be recorded into the pattern, can
        // run the sequencer from the control keys, and pass on to whatever follows.
        self.played.clear();
        while let Some(event) = context.next_event() {
            match event {
                NoteEvent::NoteOn { timing, note, velocity, channel, .. } => {
                    let timing = timing.min(len.saturating_sub(1) as u32);
                    if (CONTROL_BASE..CONTROL_BASE + CONTROL_KEYS).contains(&note) {
                        match note - CONTROL_BASE {
                            0 => {
                                meter.audition.fetch_xor(true, Ordering::Relaxed);
                            }
                            1 => {
                                if !meter.record.fetch_xor(true, Ordering::Relaxed) {
                                    context.execute_background(Task::Snapshot);
                                }
                            }
                            2 => {
                                context.execute_background(Task::Roll);
                            }
                            3 => {
                                context.execute_background(Task::Fill);
                            }
                            4 => {
                                context.execute_background(Task::Undo);
                            }
                            _ => {}
                        }
                    } else if (MUTE_BASE..MUTE_BASE + pattern::LANES as u8).contains(&note) {
                        params.pattern.toggle_mute((note - MUTE_BASE) as usize);
                    } else {
                        if let Some(lane) = map.lane_of(note) {
                            let vel = (velocity * 127.0).round().clamp(1.0, 127.0) as u8;
                            self.played.push(Event { timing, on: true, lane: lane as u8, vel });
                            if meter.record.load(Ordering::Relaxed) {
                                self.seq.record(&params.pattern, feel, lane, vel, timing);
                            }
                        }
                        if thru {
                            self.out.push(timing, true, channel, note, velocity);
                        }
                    }
                }
                NoteEvent::NoteOff { timing, note, velocity, channel, .. } => {
                    let control = (CONTROL_BASE..CONTROL_BASE + CONTROL_KEYS).contains(&note) || (MUTE_BASE..MUTE_BASE + pattern::LANES as u8).contains(&note);
                    if thru && !control {
                        self.out.push(timing.min(len.saturating_sub(1) as u32), false, channel, note, velocity);
                    }
                }
                _ => {}
            }
        }
        self.out.sort();
        for &(timing, on, channel, note, velocity) in &self.out.items[..self.out.len] {
            context.send_event(if on {
                NoteEvent::NoteOn { timing, voice_id: None, channel, note, velocity }
            } else {
                NoteEvent::NoteOff { timing, voice_id: None, channel, note, velocity }
            });
        }

        // The kit: its settings, and a finished sample kit if one has arrived.
        self.kit.set_kind(params.kit.value());
        self.kit.set_voice(params.tune.value(), params.decay.value(), params.room.value());
        let share = &self.seq.meter.samples;
        if let Some(old) = self.parked.take() {
            retire(share, &mut self.parked, old);
        }
        let arrived = share.incoming.try_lock().ok().and_then(|mut slot| slot.take());
        if let Some(new) = arrived {
            if let Some(old) = self.kit.set_samples(Some(new)) {
                retire(share, &mut self.parked, old);
            }
        }

        let sound = params.sound.value();
        let (mut ei, mut pi) = (0, 0);
        let (events, played) = (self.events.as_slice(), self.played.as_slice());
        for (i, mut frame) in buffer.iter_samples().enumerate() {
            while ei < events.len() && events[ei].timing as usize <= i {
                if events[ei].on && sound {
                    self.kit.trigger(events[ei].lane as usize, events[ei].vel);
                }
                ei += 1;
            }
            while pi < played.len() && played[pi].timing as usize <= i {
                if sound {
                    self.kit.trigger(played[pi].lane as usize, played[pi].vel);
                }
                pi += 1;
            }
            let level = util::db_to_gain(params.level.smoothed.next());
            let (l, r) = if sound { self.kit.next() } else { (0.0, 0.0) };
            let n = frame.len();
            for (ch, sample) in frame.iter_mut().enumerate() {
                let kit = if n == 1 { (l + r) * 0.5 } else if ch == 0 { l } else { r } * level;
                // Add to what came in, or, with no input, write it.
                *sample = if self.has_input { *sample + kit } else { kit };
            }
        }
        ProcessStatus::KeepAlive
    }
}

impl Vst3Plugin for Pocket {
    const VST3_CLASS_ID: [u8; 16] = *b"ClowdPocketSeqnc";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Instrument, Vst3SubCategory::Drum];
}

nih_export_vst3!(Pocket);

#[cfg(test)]
mod tests {
    use super::*;
    use pattern::*;

    fn snapshot(store: &PatternStore) -> Vec<u32> {
        (0..LANES).flat_map(|l| (0..MAX_STEPS).map(move |s| (l, s))).map(|(l, s)| store.get(l, s).0).collect()
    }

    #[test]
    fn the_control_keys_sit_clear_of_every_drum_note() {
        for map in [NoteMap::Gm, NoteMap::Pads] {
            for lane in 0..LANES {
                let n = map.note(lane);
                assert!(n < MUTE_BASE, "{map:?} lane {lane} note {n} collides with the control keys");
            }
        }
        assert!(MUTE_BASE + LANES as u8 <= CONTROL_BASE, "mute keys run into the control keys");
        assert!(CONTROL_BASE + CONTROL_KEYS <= 128);
    }

    #[test]
    fn background_tasks_roll_fill_snapshot_and_undo() {
        let mut plugin = Pocket::default();
        let run = plugin.task_executor();
        let store = &plugin.params.pattern;
        let start = snapshot(store);

        run(Task::Snapshot);
        run(Task::Roll);
        let rolled = snapshot(store);
        assert_ne!(rolled, start, "a roll changes the pattern");
        run(Task::Roll);
        assert_ne!(snapshot(store), rolled, "each roll is different");

        run(Task::Fill);
        assert!(store.hit_count() > 0);

        run(Task::Undo);
        assert_eq!(snapshot(store), start, "undo restores the pattern from the snapshot");
        // A second undo has nothing to restore and changes nothing.
        run(Task::Roll);
        let after = snapshot(store);
        run(Task::Undo);
        assert_eq!(snapshot(store), after);
    }

    #[test]
    fn rolling_leaves_locked_drums_alone() {
        let mut plugin = Pocket::default();
        let run = plugin.task_executor();
        let store = &plugin.params.pattern;
        let kick: Vec<u32> = (0..16).map(|s| store.get(LANE_KICK, s).0).collect();
        store.toggle_lock(LANE_KICK);
        for _ in 0..5 {
            run(Task::Roll);
            run(Task::Fill);
        }
        assert_eq!((0..16).map(|s| store.get(LANE_KICK, s).0).collect::<Vec<_>>(), kick);
    }

    #[test]
    fn outgoing_notes_are_sorted_with_offs_first() {
        let mut out = OutNotes::default();
        out.push(100, true, 9, 36, 1.0);
        out.push(100, false, 9, 36, 0.0);
        out.push(5, true, 0, 60, 0.5);
        out.sort();
        let order: Vec<_> = out.items[..out.len].iter().map(|i| (i.0, i.1)).collect();
        assert_eq!(order, vec![(5, true), (100, false), (100, true)]);
    }
}
