//! The Samples kit: your own WAV files, played back as the drums.
//!
//! Point Pocket at a folder and it files each `.wav` under a drum by its name (`kick`, `snare`,
//! `hat open`, `tom low`, ... in the file or in any folder above it). Drums with several files
//! use them as velocity layers, quietest to loudest. Loading happens on a background thread and is
//! handed to the audio thread without a lock or an allocation.

use crate::pattern::*;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

const MAX_FILES: usize = 800;
const MAX_FILE_BYTES: u64 = 24 * 1024 * 1024;
/// Longest sample kept, in seconds. A drum tail beyond this is not worth the memory.
const MAX_SECONDS: f32 = 6.0;

pub struct Sample {
    pub left: Vec<f32>,
    pub right: Vec<f32>,
    /// Loudness, to order velocity layers.
    pub level: f32,
}

#[derive(Default)]
pub struct SampleKit {
    pub lanes: [Vec<Sample>; LANES],
}

impl SampleKit {
    pub fn count(&self) -> usize {
        self.lanes.iter().map(Vec::len).sum()
    }

    /// Which layer a hit of this velocity plays: quiet layers for soft hits, loud ones for hard.
    pub fn layer(&self, lane: usize, vel: f32) -> Option<usize> {
        let n = self.lanes[lane].len();
        (n > 0).then(|| ((vel.clamp(0.0, 0.999) * n as f32) as usize).min(n - 1))
    }
}

// ----------------------------------------------------------------------------------------------
// WAV

pub struct Wav {
    pub sample_rate: u32,
    pub channels: Vec<Vec<f32>>,
}

fn le16(b: &[u8]) -> Option<u32> {
    Some(u16::from_le_bytes(b.get(..2)?.try_into().ok()?) as u32)
}

fn le32(b: &[u8]) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(..4)?.try_into().ok()?))
}

/// PCM 8, 16, 24 and 32 bit, and 32 and 64 bit float, any number of channels.
pub fn decode_wav(bytes: &[u8]) -> Result<Wav, String> {
    if bytes.get(..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"WAVE") {
        return Err("not a WAV file".into());
    }
    let mut i = 12;
    let (mut format, mut channels, mut rate, mut bits) = (0u32, 0usize, 0u32, 0u32);
    let mut data: Option<&[u8]> = None;
    while i + 8 <= bytes.len() {
        let id = &bytes[i..i + 4];
        let len = le32(&bytes[i + 4..]).ok_or("short chunk")? as usize;
        let body = bytes.get(i + 8..(i + 8 + len).min(bytes.len())).ok_or("short chunk")?;
        match id {
            b"fmt " => {
                format = le16(body).ok_or("short fmt")?;
                channels = le16(body.get(2..).ok_or("short fmt")?).ok_or("short fmt")? as usize;
                rate = le32(body.get(4..).ok_or("short fmt")?).ok_or("short fmt")?;
                bits = le16(body.get(14..).ok_or("short fmt")?).ok_or("short fmt")?;
                // WAVE_FORMAT_EXTENSIBLE keeps the real format in the sub-format GUID.
                if format == 0xfffe {
                    format = le16(body.get(24..).ok_or("short fmt")?).ok_or("short fmt")?;
                }
            }
            b"data" => data = Some(body),
            _ => {}
        }
        i += 8 + len + (len & 1);
    }
    let data = data.ok_or("no audio data")?;
    if channels == 0 || rate == 0 {
        return Err("no format chunk".into());
    }
    let bytes_per = (bits / 8) as usize;
    if bytes_per == 0 {
        return Err("bad bit depth".into());
    }
    let frames = data.len() / (bytes_per * channels);
    let mut out = vec![Vec::with_capacity(frames); channels];
    for f in 0..frames {
        for (c, chan) in out.iter_mut().enumerate() {
            let at = (f * channels + c) * bytes_per;
            let b = &data[at..at + bytes_per];
            let v = match (format, bits) {
                (1, 8) => (b[0] as f32 - 128.0) / 128.0,
                (1, 16) => i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0,
                (1, 24) => (i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8) as f32 / 8_388_608.0,
                (1, 32) => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32 / 2_147_483_648.0,
                (3, 32) => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                (3, 64) => f64::from_le_bytes(b.try_into().map_err(|_| "bad sample")?) as f32,
                _ => return Err(format!("unsupported format {format} at {bits} bits")),
            };
            chan.push(v);
        }
    }
    Ok(Wav { sample_rate: rate, channels: out })
}

/// Linear resampling. Good enough for drum one-shots.
pub fn resample(x: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || x.is_empty() {
        return x.to_vec();
    }
    let ratio = from as f64 / to as f64;
    let n = (x.len() as f64 / ratio).floor() as usize;
    (0..n)
        .map(|i| {
            let p = i as f64 * ratio;
            let k = p as usize;
            let frac = (p - k as f64) as f32;
            x[k] + (x.get(k + 1).copied().unwrap_or(x[k]) - x[k]) * frac
        })
        .collect()
}

fn into_sample(wav: Wav, sr: u32) -> Option<Sample> {
    let first = wav.channels.first()?;
    if first.is_empty() {
        return None;
    }
    let left = resample(first, wav.sample_rate, sr);
    let right = match wav.channels.get(1) {
        Some(r) => resample(r, wav.sample_rate, sr),
        None => left.clone(),
    };
    let keep = (MAX_SECONDS * sr as f32) as usize;
    let (mut left, mut right) = (left, right);
    left.truncate(keep);
    right.truncate(keep);
    // A cut tail would click, so fade the last few milliseconds.
    let fade = (0.01 * sr as f32) as usize;
    let n = left.len().min(right.len());
    for k in 0..fade.min(n) {
        let g = k as f32 / fade as f32;
        left[n - 1 - k] *= g;
        right[n - 1 - k] *= g;
    }
    // Loudness over the first 150 ms, which is where a drum's level is set.
    let head = ((0.15 * sr as f32) as usize).min(n).max(1);
    let level = (left[..head].iter().chain(&right[..head]).map(|s| s * s).sum::<f32>() / (2 * head) as f32).sqrt();
    Some(Sample { left, right, level })
}

// ----------------------------------------------------------------------------------------------
// Naming

fn tokens(text: &str) -> Vec<String> {
    text.to_lowercase().split(|c: char| !c.is_ascii_alphanumeric()).filter(|t| !t.is_empty()).map(str::to_owned).collect()
}

/// Which drum a file is, from its name and the folders above it (nearest first wins).
pub fn classify(path_parts: &[String]) -> Option<usize> {
    // Look at the file name first, then each folder up.
    for part in path_parts.iter().rev() {
        let t = tokens(part);
        let has = |words: &[&str]| t.iter().any(|tok| words.iter().any(|w| tok.contains(w)));
        let is = |words: &[&str]| t.iter().any(|tok| words.contains(&tok.as_str()));
        let open = has(&["open"]) || is(&["oh", "ohat"]);
        let lane = if has(&["kick", "bassdrum", "kik"]) || is(&["bd", "kd"]) {
            Some(LANE_KICK)
        } else if has(&["snare", "snr"]) || is(&["sd"]) {
            Some(LANE_SNARE)
        } else if has(&["rimshot", "sidestick", "rim"]) || is(&["rs", "stick", "side"]) {
            Some(LANE_RIM)
        } else if has(&["clap"]) || is(&["cp", "hc"]) {
            Some(LANE_CLAP)
        } else if has(&["hihat", "hat"]) || is(&["hh", "ch", "oh", "ohat", "chat"]) {
            Some(if open { LANE_OPEN } else { LANE_HAT })
        } else if has(&["tom"]) || is(&["lt", "ht", "mt", "ft"]) {
            let low = has(&["low", "floor"]) || is(&["lo", "lt", "ft", "l"]);
            Some(if low { LANE_LTOM } else { LANE_HTOM })
        } else if has(&["crash", "splash", "china"]) {
            Some(LANE_CRASH)
        } else if has(&["ride"]) {
            Some(LANE_RIDE)
        } else if has(&["perc", "cowbell", "shaker", "tamb", "conga", "bongo", "clave", "woodblock", "block", "bell"]) {
            Some(LANE_PERC)
        } else if has(&["cymbal"]) || is(&["cym"]) {
            Some(LANE_CRASH)
        } else {
            None
        };
        if lane.is_some() {
            return lane;
        }
    }
    None
}

fn collect(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(read) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = read.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name().to_string_lossy().to_lowercase());
    for e in entries {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || out.len() >= MAX_FILES {
            continue;
        }
        let Ok(kind) = e.file_type() else { continue };
        if kind.is_dir() && depth < 4 {
            collect(&e.path(), depth + 1, out);
        } else if kind.is_file() && name.to_lowercase().ends_with(".wav") {
            out.push(e.path());
        }
    }
}

/// Load every `.wav` below `dir`, resampled to `sr`.
pub fn load_folder(dir: &Path, sr: u32) -> Result<SampleKit, String> {
    let mut files = Vec::new();
    collect(dir, 0, &mut files);
    if files.is_empty() {
        return Err("no .wav files in it".into());
    }
    let mut kit = SampleKit::default();
    for path in files {
        if std::fs::metadata(&path).map(|m| m.len() > MAX_FILE_BYTES).unwrap_or(true) {
            continue;
        }
        // The name, and the folders between the chosen one and the file.
        let rel = path.strip_prefix(dir).unwrap_or(&path);
        let parts: Vec<String> = rel.iter().map(|p| p.to_string_lossy().into_owned()).collect();
        let Some(lane) = classify(&parts) else { continue };
        let Ok(bytes) = std::fs::read(&path) else { continue };
        let Ok(wav) = decode_wav(&bytes) else { continue };
        if let Some(sample) = into_sample(wav, sr) {
            kit.lanes[lane].push(sample);
        }
    }
    if kit.count() == 0 {
        return Err("no file names it could tell apart (try kick, snare, hat ...)".into());
    }
    for lane in &mut kit.lanes {
        lane.sort_by(|a, b| a.level.total_cmp(&b.level));
    }
    Ok(kit)
}

// ----------------------------------------------------------------------------------------------
// Handing a kit to the audio thread

/// Shared between the loader, the editor and the audio thread.
pub struct KitShare {
    /// A finished kit waiting for the audio thread. It only ever `try_lock`s.
    pub incoming: Mutex<Option<Arc<SampleKit>>>,
    /// Kits the audio thread has replaced, parked here so they are freed somewhere else.
    pub retired: Mutex<Vec<Arc<SampleKit>>>,
    pub sample_rate: AtomicU32,
    pub status: Mutex<String>,
    /// Counts up whenever a load finishes, so the editor knows to look at `status`.
    pub generation: AtomicU32,
}

impl Default for KitShare {
    fn default() -> Self {
        Self {
            incoming: Mutex::new(None),
            retired: Mutex::new(Vec::with_capacity(16)),
            sample_rate: AtomicU32::new(44100),
            status: Mutex::new(String::new()),
            generation: AtomicU32::new(0),
        }
    }
}

impl KitShare {
    pub fn status(&self) -> String {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }

    fn set_status(&self, text: String) {
        if let Ok(mut s) = self.status.lock() {
            *s = text;
        }
        self.generation.fetch_add(1, Ordering::Relaxed);
    }

    /// Load `dir` in the background and queue the result for the audio thread.
    pub fn spawn_load(self: &Arc<Self>, dir: PathBuf) {
        let share = self.clone();
        self.set_status("LOADING".into());
        std::thread::spawn(move || {
            let sr = share.sample_rate.load(Ordering::Relaxed);
            match load_folder(&dir, sr) {
                Ok(kit) => {
                    let counts: Vec<String> = kit.lanes.iter().zip(LANE_INFO.iter()).filter(|(l, _)| !l.is_empty()).map(|(l, i)| format!("{} {}", l.len(), i.short)).collect();
                    let text = format!("{} SAMPLES: {}", kit.count(), counts.join(" "));
                    if let Ok(mut slot) = share.incoming.lock() {
                        *slot = Some(Arc::new(kit));
                    }
                    // Anything the audio thread has finished with can go now.
                    if let Ok(mut r) = share.retired.lock() {
                        r.clear();
                    }
                    share.set_status(text);
                }
                Err(e) => share.set_status(format!("NO KIT: {}", e.to_uppercase())),
            }
        });
    }
}

// ----------------------------------------------------------------------------------------------
// Playback

#[derive(Clone, Copy, Default)]
pub struct SampleVoice {
    pub on: bool,
    pub layer: usize,
    pub pos: usize,
    pub gain: f32,
}

impl SampleVoice {
    pub fn next(&mut self, kit: &SampleKit, lane: usize) -> (f32, f32) {
        let Some(s) = kit.lanes[lane].get(self.layer) else {
            self.on = false;
            return (0.0, 0.0);
        };
        if self.pos >= s.left.len().min(s.right.len()) {
            self.on = false;
            return (0.0, 0.0);
        }
        let out = (s.left[self.pos] * self.gain, s.right[self.pos] * self.gain);
        self.pos += 1;
        out
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A WAV file in memory: `bits` is 8, 16, 24 or 32 (integer) or -32 for float.
    pub fn make_wav(channels: usize, rate: u32, bits: i32, frames: &[Vec<f32>]) -> Vec<u8> {
        let (format, depth) = if bits < 0 { (3u16, 32u16) } else { (1u16, bits as u16) };
        let mut data = Vec::new();
        for f in 0..frames[0].len() {
            for ch in frames.iter().take(channels) {
                let v = ch[f];
                match (format, depth) {
                    (1, 8) => data.push((v * 127.0 + 128.0).round() as u8),
                    (1, 16) => data.extend(((v * 32767.0).round() as i16).to_le_bytes()),
                    (1, 24) => data.extend(&((v * 8_388_607.0).round() as i32).to_le_bytes()[..3]),
                    (1, 32) => data.extend(((v as f64 * 2_147_483_647.0).round() as i32).to_le_bytes()),
                    _ => data.extend(v.to_le_bytes()),
                }
            }
        }
        let mut fmt = Vec::new();
        fmt.extend(format.to_le_bytes());
        fmt.extend((channels as u16).to_le_bytes());
        fmt.extend(rate.to_le_bytes());
        fmt.extend((rate * channels as u32 * depth as u32 / 8).to_le_bytes());
        fmt.extend((channels as u16 * depth / 8).to_le_bytes());
        fmt.extend(depth.to_le_bytes());
        let mut body = b"WAVE".to_vec();
        for (id, chunk) in [(&b"fmt "[..], &fmt), (&b"LIST"[..], &vec![1, 2, 3]), (&b"data"[..], &data)] {
            body.extend(id);
            body.extend((chunk.len() as u32).to_le_bytes());
            body.extend(chunk.iter());
            if chunk.len() % 2 == 1 {
                body.push(0);
            }
        }
        let mut file = b"RIFF".to_vec();
        file.extend((body.len() as u32).to_le_bytes());
        file.extend(body);
        file
    }

    fn ramp(n: usize, scale: f32) -> Vec<f32> {
        (0..n).map(|i| ((i as f32 * 0.37).sin() * scale * (1.0 - i as f32 / n as f32)).clamp(-1.0, 1.0)).collect()
    }

    #[test]
    fn every_bit_depth_decodes_to_the_same_audio() {
        let x = ramp(400, 0.8);
        for (bits, tol) in [(8, 0.02), (16, 1e-3), (24, 1e-5), (32, 1e-6), (-32, 1e-7)] {
            let wav = decode_wav(&make_wav(1, 44100, bits, &[x.clone()])).unwrap_or_else(|e| panic!("{bits}: {e}"));
            assert_eq!((wav.sample_rate, wav.channels.len(), wav.channels[0].len()), (44100, 1, 400), "{bits}");
            for (a, b) in x.iter().zip(&wav.channels[0]) {
                assert!((a - b).abs() < tol, "{bits}-bit: {a} vs {b}");
            }
        }
    }

    #[test]
    fn stereo_keeps_its_channels_apart() {
        let (l, r) = (ramp(100, 0.5), ramp(100, -0.25));
        let wav = decode_wav(&make_wav(2, 48000, 16, &[l.clone(), r.clone()])).unwrap();
        assert_eq!(wav.channels.len(), 2);
        assert!((wav.channels[0][10] - l[10]).abs() < 1e-3 && (wav.channels[1][10] - r[10]).abs() < 1e-3);
    }

    #[test]
    fn bad_files_are_errors_not_panics() {
        assert!(decode_wav(b"nope").is_err());
        assert!(decode_wav(b"RIFF\0\0\0\0WAVE").is_err());
        let good = make_wav(1, 44100, 16, &[ramp(50, 0.5)]);
        for cut in [13usize, 20, 30, 44, good.len() - 5] {
            let _ = decode_wav(&good[..cut]);
        }
        let mut seed = 3u32;
        for _ in 0..300 {
            let mut f = good.clone();
            for b in f.iter_mut() {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                if seed % 17 == 0 {
                    *b = seed as u8;
                }
            }
            let _ = decode_wav(&f);
        }
    }

    #[test]
    fn resampling_keeps_pitch_and_length() {
        let sr_from = 22050u32;
        let x: Vec<f32> = (0..2205).map(|i| (std::f32::consts::TAU * 440.0 * i as f32 / sr_from as f32).sin()).collect();
        let y = resample(&x, sr_from, 44100);
        assert!((y.len() as i32 - 4410).abs() <= 1);
        // Still a 440 Hz sine: compare against the ideal one.
        for (i, v) in y.iter().enumerate().skip(10).take(2000) {
            let ideal = (std::f32::consts::TAU * 440.0 * i as f32 / 44100.0).sin();
            assert!((v - ideal).abs() < 0.05, "sample {i}");
        }
        assert_eq!(resample(&x, 44100, 44100), x);
    }

    #[test]
    fn file_names_say_which_drum_it_is() {
        let c = |p: &[&str]| classify(&p.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        for (parts, lane) in [
            (vec!["Kick 01.wav"], LANE_KICK),
            (vec!["BD_hard.wav"], LANE_KICK),
            (vec!["Snare Tight.wav"], LANE_SNARE),
            (vec!["sd-02.wav"], LANE_SNARE),
            (vec!["HiHat Closed.wav"], LANE_HAT),
            (vec!["hh open 3.wav"], LANE_OPEN),
            (vec!["OH.wav"], LANE_OPEN),
            (vec!["CH_1.wav"], LANE_HAT),
            (vec!["Tom Low.wav"], LANE_LTOM),
            (vec!["floor tom.wav"], LANE_LTOM),
            (vec!["Tom High.wav"], LANE_HTOM),
            (vec!["Tom 1.wav"], LANE_HTOM),
            (vec!["Crash.wav"], LANE_CRASH),
            (vec!["Ride Bell.wav"], LANE_RIDE),
            (vec!["Clap.wav"], LANE_CLAP),
            (vec!["Rimshot.wav"], LANE_RIM),
            (vec!["Cowbell.wav"], LANE_PERC),
            // The folder can say it when the file does not.
            (vec!["Snares", "01.wav"], LANE_SNARE),
            (vec!["Kit A", "Hats Open", "a.wav"], LANE_OPEN),
            // The nearest name wins.
            (vec!["Kicks", "snare dub.wav"], LANE_SNARE),
        ] {
            assert_eq!(c(&parts.iter().map(|s| &**s).collect::<Vec<_>>()), Some(lane), "{parts:?}");
        }
        assert_eq!(c(&["loop 120 bpm.wav"]), None);
    }

    #[test]
    fn layers_follow_velocity() {
        let mut kit = SampleKit::default();
        for lvl in [0.1, 0.4, 0.9] {
            kit.lanes[LANE_SNARE].push(Sample { left: vec![lvl], right: vec![lvl], level: lvl });
        }
        assert_eq!(kit.layer(LANE_SNARE, 0.05), Some(0));
        assert_eq!(kit.layer(LANE_SNARE, 0.5), Some(1));
        assert_eq!(kit.layer(LANE_SNARE, 1.0), Some(2));
        assert_eq!(kit.layer(LANE_KICK, 0.5), None);
    }

    #[test]
    fn a_folder_loads_resamples_orders_layers_and_reports() {
        let dir = std::env::temp_dir().join(format!("pocket-kit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Snares")).unwrap();
        std::fs::create_dir_all(dir.join(".hidden")).unwrap();
        let write = |path: PathBuf, rate: u32, bits: i32, amp: f32| std::fs::write(path, make_wav(1, rate, bits, &[ramp(2000, amp)])).unwrap();
        write(dir.join("kick.wav"), 22050, 16, 0.9);
        write(dir.join("Snares").join("a loud.wav"), 44100, 24, 0.9);
        write(dir.join("Snares").join("b soft.wav"), 44100, 16, 0.2);
        write(dir.join("hat open.wav"), 48000, -32, 0.5);
        write(dir.join("loop.wav"), 44100, 16, 0.5);
        write(dir.join(".hidden").join("kick.wav"), 44100, 16, 0.5);
        std::fs::write(dir.join("snare.txt"), b"x").unwrap();
        let kit = load_folder(&dir, 44100).unwrap();
        assert_eq!(kit.lanes[LANE_KICK].len(), 1);
        assert!((kit.lanes[LANE_KICK][0].left.len() as i32 - 4000).abs() < 20, "resampled up from 22.05 kHz: {}", kit.lanes[LANE_KICK][0].left.len());
        assert_eq!(kit.lanes[LANE_SNARE].len(), 2);
        assert!(kit.lanes[LANE_SNARE][0].level < kit.lanes[LANE_SNARE][1].level, "quietest layer first");
        assert_eq!(kit.lanes[LANE_OPEN].len(), 1);
        assert_eq!(kit.count(), 4, "the loop, the hidden folder and the text file are ignored");
        // A folder with nothing usable says so.
        let empty = dir.join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        assert!(load_folder(&empty, 44100).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_background_loader_hands_a_kit_over_and_reports() {
        let dir = std::env::temp_dir().join(format!("pocket-kit-bg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("kick.wav"), make_wav(1, 44100, 16, &[ramp(500, 0.8)])).unwrap();
        let share = Arc::new(KitShare::default());
        share.spawn_load(dir.clone());
        for _ in 0..200 {
            if share.incoming.lock().unwrap().is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let kit = share.incoming.lock().unwrap().take().expect("kit arrived");
        assert_eq!(kit.count(), 1);
        assert!(share.status().contains("1 SAMPLES"), "{}", share.status());
        share.spawn_load(dir.join("missing"));
        for _ in 0..200 {
            if share.status().starts_with("NO KIT") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(share.status().starts_with("NO KIT"), "{}", share.status());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_sample_voice_plays_through_and_stops() {
        let mut kit = SampleKit::default();
        kit.lanes[LANE_KICK].push(Sample { left: vec![1.0, 0.5, 0.25], right: vec![0.5, 0.25, 0.125], level: 1.0 });
        let mut v = SampleVoice { on: true, layer: 0, pos: 0, gain: 2.0 };
        assert_eq!(v.next(&kit, LANE_KICK), (2.0, 1.0));
        assert_eq!(v.next(&kit, LANE_KICK), (1.0, 0.5));
        assert_eq!(v.next(&kit, LANE_KICK), (0.5, 0.25));
        assert!(v.on);
        assert_eq!(v.next(&kit, LANE_KICK), (0.0, 0.0));
        assert!(!v.on);
    }
}
