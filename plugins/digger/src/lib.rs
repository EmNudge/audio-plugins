use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, egui, EguiState};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

pub mod engine;
pub mod library;
pub mod state;
pub mod tags;
pub mod ui;

use engine::{Engine, SLOTS};
use library::{Library, Share};
use state::{note_slot, Nav, Shared, SLOT_INFO};

pub struct Digger {
    params: Arc<DiggerParams>,
    engine: Engine,
    lib: Option<Arc<Library>>,
    /// A replaced library waiting for somewhere it can be freed off the audio thread.
    parked: Option<Arc<Library>>,
    seen_audition: [u32; SLOTS],
    seen_browse: [u32; SLOTS],
    rng: u32,
}

#[derive(Params)]
pub struct SlotParams {
    #[id = "level"]
    pub level: FloatParam,
    #[id = "tune"]
    pub tune: FloatParam,
    #[id = "pan"]
    pub pan: FloatParam,
}

impl SlotParams {
    fn new(slot: usize) -> Self {
        let name = SLOT_INFO[slot].name;
        let pan = match slot {
            3 => -0.25,
            4 => 0.25,
            5 => 0.2,
            6 => -0.2,
            7 => 0.3,
            _ => 0.0,
        };
        Self {
            level: FloatParam::new(format!("{name} Level"), 0.0, FloatRange::Linear { min: -30.0, max: 6.0 })
                .with_unit(" dB")
                .with_value_to_string(formatters::v2s_f32_rounded(1)),
            tune: FloatParam::new(format!("{name} Tune"), 0.0, FloatRange::Linear { min: -12.0, max: 12.0 })
                .with_unit(" st")
                .with_value_to_string(Arc::new(|v| format!("{v:+.1}"))),
            pan: FloatParam::new(format!("{name} Pan"), pan, FloatRange::Linear { min: -1.0, max: 1.0 })
                .with_value_to_string(Arc::new(|v| if v.abs() < 0.005 { "C".into() } else if v < 0.0 { format!("L{:.0}", -v * 100.0) } else { format!("R{:.0}", v * 100.0) })),
        }
    }
}

#[derive(Params)]
pub struct DiggerParams {
    #[persist = "editor-state"]
    pub editor_state: Arc<EguiState>,
    /// Folder and per-slot selection, saved with the project by name.
    #[persist = "selection"]
    pub shared: Shared,

    #[nested(array, group = "Slot")]
    pub slots: [SlotParams; SLOTS],

    /// Sound a sample as you browse to it, when the transport is stopped.
    #[id = "browse"]
    pub audition_browse: BoolParam,
    #[id = "level"]
    pub level: FloatParam,
}

impl Default for DiggerParams {
    fn default() -> Self {
        Self {
            editor_state: EguiState::from_size(1040, 780),
            shared: Shared::default(),
            slots: std::array::from_fn(SlotParams::new),
            audition_browse: BoolParam::new("Audition On Browse", true),
            level: FloatParam::new("Level", -3.0, FloatRange::Linear { min: -30.0, max: 6.0 })
                .with_smoother(SmoothingStyle::Linear(20.0))
                .with_unit(" dB")
                .with_value_to_string(formatters::v2s_f32_rounded(1)),
        }
    }
}

/// Load `dir` in the background and queue the result for the audio thread and the editor.
/// `keep` holds on to the current selections; a newly chosen folder starts afresh.
pub fn spawn_load(params: &Arc<DiggerParams>, dir: PathBuf, keep: bool) {
    let params = params.clone();
    params.shared.lib.set_status("LOADING".into());
    std::thread::spawn(move || {
        let shared = &params.shared;
        let sr = shared.lib.sample_rate.load(Ordering::Relaxed);
        match library::load_library(&dir, sr) {
            Ok(lib) => {
                // Tags from names and audio, then what the user added and the kits they saved.
                tags::auto_tag(&lib);
                let side = tags::load_sidecar(&dir);
                tags::apply_sidecar(&lib, &side);
                if let Ok(mut kits) = shared.kits.lock() {
                    *kits = side.kits;
                }
                shared.kit.store(state::NO_KIT, Ordering::Relaxed);
                shared.filter.store(0, Ordering::Relaxed);
                shared.adopt(&lib, keep);
                let text = format!("{} SAMPLES IN {} CATEGORIES", lib.count(), lib.cats.len());
                shared.lib.publish(lib, text);
            }
            Err(e) => shared.lib.set_status(format!("NO LIBRARY: {}", e.to_uppercase())),
        }
    });
}

/// Park a library the audio thread has finished with, without freeing it here.
fn retire(share: &Share, parked: &mut Option<Arc<Library>>, old: Arc<Library>) {
    let leftover = match share.retired.try_lock() {
        Ok(mut list) if list.len() < list.capacity() => {
            list.push(old);
            None
        }
        _ => Some(old),
    };
    match (parked.is_none(), leftover) {
        (true, leftover) => *parked = leftover,
        // Nowhere to put it: leaking a library once in a long while beats freeing it here.
        (false, Some(extra)) => std::mem::forget(extra),
        (false, None) => {}
    }
}

/// Sound whatever a slot has selected. The closed hat chokes the open one.
pub fn fire(engine: &mut Engine, lib: &Library, params: &DiggerParams, slot: usize, velocity: f32) {
    let shared = &params.shared;
    let (cat, idx) = shared.selection(slot);
    if lib.sample(cat, idx).is_none() {
        return;
    }
    if slot == 3 {
        engine.choke(4);
    }
    let p = &params.slots[slot];
    engine.trigger(slot, cat, idx, velocity, p.tune.value(), util::db_to_gain(p.level.value()), p.pan.value());
    shared.hit[slot].fetch_max(velocity, Ordering::Relaxed);
}

impl Default for Digger {
    fn default() -> Self {
        Self {
            params: Arc::new(DiggerParams::default()),
            engine: Engine::default(),
            lib: None,
            parked: None,
            seen_audition: [0; SLOTS],
            seen_browse: [0; SLOTS],
            rng: 0x9e3779b9,
        }
    }
}

impl Digger {
    fn random(&mut self) -> u32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng
    }
}

/// Notes for one block, in the order they arrive.
struct Hits {
    items: [(u32, u8, f32); 256],
    len: usize,
}

impl Hits {
    fn push(&mut self, timing: u32, slot: usize, velocity: f32) {
        if self.len < self.items.len() {
            self.items[self.len] = (timing, slot as u8, velocity);
            self.len += 1;
        }
    }
}

impl Plugin for Digger {
    const NAME: &'static str = "Digger Drum Sampler";
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
        create_egui_editor(
            self.params.editor_state.clone(),
            ui::UiState::default(),
            |_, _| {},
            move |ctx, setter, state| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE.fill(ui::background()).inner_margin(14.0))
                    .show(ctx, |ui| ui::draw(ui, &params, setter, state));
            },
        )
    }

    fn initialize(&mut self, _layout: &AudioIOLayout, config: &BufferConfig, _context: &mut impl InitContext<Self>) -> bool {
        self.engine.set_sample_rate(config.sample_rate);
        let shared = &self.params.shared;
        shared.lib.sample_rate.store(config.sample_rate as u32, Ordering::Relaxed);
        // The folder this project used, else the last one used anywhere. Resampled for this host.
        let mut folder = shared.root.lock().map(|f| f.clone()).unwrap_or_default();
        if folder.is_empty() {
            folder = state::remembered_folder();
            shared.set_root(&folder);
        }
        if !folder.is_empty() {
            spawn_load(&self.params, folder.into(), true);
        }
        true
    }

    fn reset(&mut self) {
        self.engine.stop_all();
    }

    fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, context: &mut impl ProcessContext<Self>) -> ProcessStatus {
        let len = buffer.samples();
        let params = self.params.clone();
        let shared = &params.shared;
        let playing = context.transport().playing;
        shared.playing.store(playing, Ordering::Relaxed);

        // A finished library, if one has arrived. Voices point into the old one, so they stop.
        let arrived = shared.lib.incoming.try_lock().ok().and_then(|mut slot| slot.take());
        if let Some(new) = arrived {
            self.engine.stop_all();
            if let Some(old) = self.lib.replace(new) {
                retire(&shared.lib, &mut self.parked, old);
            }
        }
        if let Some(old) = self.parked.take() {
            retire(&shared.lib, &mut self.parked, old);
        }
        let Some(lib) = self.lib.clone() else {
            while context.next_event().is_some() {}
            for mut frame in buffer.iter_samples() {
                for s in frame.iter_mut() {
                    *s = 0.0;
                }
            }
            return ProcessStatus::KeepAlive;
        };

        let mut hits = Hits { items: [(0, 0, 0.0); 256], len: 0 };
        while let Some(event) = context.next_event() {
            if let NoteEvent::NoteOn { timing, note, velocity, .. } = event {
                let timing = timing.min(len.saturating_sub(1) as u32);
                if let Some(nav) = Nav::from_note(note) {
                    let seed = self.random();
                    shared.navigate(nav, seed, &lib);
                } else if let Some(slot) = note_slot(note) {
                    hits.push(timing, slot, velocity);
                }
            }
        }

        // Browsing and auditioning, from the editor or from the navigation keys. Browsing only
        // speaks when the transport is stopped, so it never lands on top of the groove.
        let browse_on = params.audition_browse.value();
        for slot in 0..SLOTS {
            let a = shared.audition[slot].load(Ordering::Relaxed);
            if a != self.seen_audition[slot] {
                self.seen_audition[slot] = a;
                hits.push(0, slot, 0.9);
            }
            let b = shared.browse[slot].load(Ordering::Relaxed);
            if b != self.seen_browse[slot] {
                self.seen_browse[slot] = b;
                if browse_on && !playing {
                    hits.push(0, slot, 0.9);
                }
            }
        }

        let mut next = 0;
        let mut peak = 0.0f32;
        for (i, mut frame) in buffer.iter_samples().enumerate() {
            while next < hits.len && hits.items[next].0 as usize <= i {
                let (_, slot, vel) = hits.items[next];
                fire(&mut self.engine, &lib, &params, slot as usize, vel);
                next += 1;
            }
            let gain = util::db_to_gain(params.level.smoothed.next());
            let (l, r) = self.engine.next(&lib);
            let n = frame.len();
            for (ch, sample) in frame.iter_mut().enumerate() {
                let v = if n == 1 { (l + r) * 0.5 } else if ch == 0 { l } else { r } * gain;
                *sample = v;
                peak = peak.max(v.abs());
            }
        }
        for slot in 0..SLOTS {
            shared.pos[slot].store(self.engine.playhead(&lib, slot), Ordering::Relaxed);
        }
        shared.out_peak.fetch_max(peak, Ordering::Relaxed);
        ProcessStatus::KeepAlive
    }
}

impl Vst3Plugin for Digger {
    const VST3_CLASS_ID: [u8; 16] = *b"ClowdDiggerSmplr";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Instrument, Vst3SubCategory::Sampler, Vst3SubCategory::Drum];
}

nih_export_vst3!(Digger);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::tests::{make_wav, tone, Scratch};

    fn folder(tag: &str) -> Scratch {
        let s = Scratch::new(tag);
        for (cat, n) in [("kick", 3), ("snare", 2), ("hat", 2), ("hat-open", 1)] {
            std::fs::create_dir_all(s.0.join(cat)).unwrap();
            for k in 0..n {
                std::fs::write(s.0.join(cat).join(format!("{cat}{k}.wav")), make_wav(1, 44100, &[tone(2000 + k * 100, 0.8)])).unwrap();
            }
        }
        s
    }

    fn wait_for_library(params: &Arc<DiggerParams>) -> Arc<Library> {
        for _ in 0..200 {
            if let Some(l) = params.shared.lib.library() {
                return l;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("the library never arrived");
    }

    #[test]
    fn the_background_loader_fills_the_slots_and_hands_the_library_over() {
        let dir = folder("loader");
        let params = Arc::new(DiggerParams::default());
        params.shared.lib.sample_rate.store(48000, Ordering::Relaxed);
        spawn_load(&params, dir.0.clone(), false);
        let lib = wait_for_library(&params);
        assert_eq!(lib.count(), 8);
        // Each slot lands on the category named for it.
        let name = |s: usize| lib.cats[params.shared.selection(s).0].name.clone();
        assert_eq!((name(0), name(1), name(3), name(4)), ("kick".into(), "snare".into(), "hat".into(), "hat-open".into()));
        assert!(params.shared.lib.incoming.lock().unwrap().is_some(), "queued for the audio thread");
        assert!(params.shared.lib.status().contains("8 SAMPLES"));
    }

    #[test]
    fn a_hit_sounds_the_selected_sample_and_swapping_changes_what_plays() {
        let dir = folder("hit");
        let params = Arc::new(DiggerParams::default());
        spawn_load(&params, dir.0.clone(), false);
        let lib = wait_for_library(&params);
        let mut engine = Engine::default();

        let render = |engine: &mut Engine| -> Vec<f32> { (0..4000).map(|_| engine.next(&lib).0).collect() };
        fire(&mut engine, &lib, &params, 0, 1.0);
        let first = render(&mut engine);
        assert!(first.iter().any(|v| v.abs() > 0.1) && first.iter().all(|v| v.is_finite()));
        assert!(params.shared.hit[0].load(Ordering::Relaxed) > 0.99);

        params.shared.step_sample(0, 1, &lib);
        fire(&mut engine, &lib, &params, 0, 1.0);
        let second = render(&mut engine);
        assert_ne!(first.iter().filter(|v| **v != 0.0).count(), second.iter().filter(|v| **v != 0.0).count(), "the next kick is a different length");
    }

    #[test]
    fn the_closed_hat_chokes_the_open_hat_and_an_empty_slot_is_silent() {
        let dir = folder("choke");
        let params = Arc::new(DiggerParams::default());
        spawn_load(&params, dir.0.clone(), false);
        let lib = wait_for_library(&params);
        let mut engine = Engine::default();
        engine.set_sample_rate(44100.0);
        fire(&mut engine, &lib, &params, 4, 1.0);
        for _ in 0..100 {
            engine.next(&lib);
        }
        assert_eq!(engine.active(), 1);
        fire(&mut engine, &lib, &params, 3, 1.0);
        for _ in 0..600 {
            engine.next(&lib);
        }
        assert_eq!(engine.active(), 1, "only the closed hat is left ringing");

        let mut quiet = Engine::default();
        params.shared.cat[2].store(99, Ordering::Relaxed);
        fire(&mut quiet, &lib, &params, 2, 1.0);
        assert_eq!(quiet.active(), 0);
    }

    #[test]
    fn per_slot_settings_reach_the_voice() {
        let dir = folder("settings");
        let params = Arc::new(DiggerParams::default());
        spawn_load(&params, dir.0.clone(), false);
        let lib = wait_for_library(&params);
        let peak = |params: &DiggerParams| {
            let mut e = Engine::default();
            fire(&mut e, &lib, params, 1, 1.0);
            (0..2000).map(|_| e.next(&lib)).fold((0.0f32, 0.0f32), |a, (l, r)| (a.0.max(l.abs()), a.1.max(r.abs())))
        };
        let (l0, r0) = peak(&params);
        assert!((l0 - r0).abs() < 1e-6, "centred by default");
        // The same folder under settings of -6 dB and hard right.
        let mut moved = DiggerParams::default();
        moved.slots[1].level = FloatParam::new("Level", -6.0, FloatRange::Linear { min: -30.0, max: 6.0 });
        moved.slots[1].pan = FloatParam::new("Pan", 1.0, FloatRange::Linear { min: -1.0, max: 1.0 });
        moved.shared.cat[1].store(params.shared.cat[1].load(Ordering::Relaxed), Ordering::Relaxed);
        let (l1, r1) = peak(&moved);
        assert!(l1 < 1e-6, "hard right");
        assert!((r1 / r0 - util::db_to_gain(-6.0)).abs() < 0.01, "level in dB");
    }
}
