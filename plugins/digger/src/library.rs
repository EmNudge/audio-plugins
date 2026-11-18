//! The sample library: a root folder whose sub-folders are categories (`kick`, `snare`, ...), every
//! WAV in them decoded into memory so that swapping a sample while the groove plays is instant.
//!
//! Loading happens on a background thread and is handed to the audio thread without a lock or an
//! allocation, the same way Pocket hands over its sample kit.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

const MAX_FILES: usize = 4000;
const MAX_FILE_BYTES: u64 = 24 * 1024 * 1024;
/// Longest sample kept, in seconds. A drum tail beyond this is not worth the memory.
const MAX_SECONDS: f32 = 8.0;
/// Bars in the waveform thumbnail each sample carries for the editor.
pub const PEAKS: usize = 40;

pub struct Sample {
    pub name: String,
    pub left: Vec<f32>,
    /// `None` for a mono file, so it is not stored twice.
    pub right: Option<Vec<f32>>,
    /// Loudest absolute value in each of `PEAKS` slices, scaled so the biggest is 1.
    pub peaks: Vec<f32>,
    /// Seconds until the envelope has fallen 30 dB below its peak.
    pub decay: f32,
    /// Zero-crossing rate in hertz over the attack: a cheap stand-in for brightness.
    pub bright: f32,
    /// Bit `n` is set when the library's tag number `n` applies. Atomic so the editor can retag
    /// while the audio thread reads it.
    pub tags: AtomicU64,
    /// The subset of `tags` the user added, which is what gets saved.
    pub user: AtomicU64,
}

impl Sample {
    /// A sample with no analysis or tags, for tests and generated libraries.
    pub fn plain(name: impl Into<String>, left: Vec<f32>, right: Option<Vec<f32>>, peaks: Vec<f32>) -> Self {
        Self { name: name.into(), left, right, peaks, decay: 0.0, bright: 0.0, tags: AtomicU64::new(0), user: AtomicU64::new(0) }
    }

    pub fn len(&self) -> usize {
        self.left.len()
    }

    pub fn is_empty(&self) -> bool {
        self.left.is_empty()
    }

    pub fn right(&self) -> &[f32] {
        self.right.as_deref().unwrap_or(&self.left)
    }
}

pub struct Category {
    pub name: String,
    pub samples: Vec<Sample>,
}

#[derive(Default)]
pub struct Library {
    pub root: String,
    pub sample_rate: u32,
    pub cats: Vec<Category>,
    /// The tag vocabulary: tag `n` is `tag_names[n]`. Only the editor and the loader touch it.
    pub tag_names: std::sync::Mutex<Vec<String>>,
}

impl Library {
    pub fn new(root: String, sample_rate: u32, cats: Vec<Category>) -> Self {
        Self { root, sample_rate, cats, tag_names: Default::default() }
    }

    pub fn count(&self) -> usize {
        self.cats.iter().map(|c| c.samples.len()).sum()
    }

    pub fn sample(&self, cat: usize, idx: usize) -> Option<&Sample> {
        self.cats.get(cat)?.samples.get(idx)
    }

    pub fn find_cat(&self, name: &str) -> Option<usize> {
        self.cats.iter().position(|c| c.name.eq_ignore_ascii_case(name))
    }

    /// The category a drum slot should start on: an exact name match on one of its hints, then a
    /// partial one, then whatever is in that position.
    pub fn default_cat(&self, hints: &[&str], fallback: usize) -> usize {
        for h in hints {
            if let Some(i) = self.find_cat(h) {
                return i;
            }
        }
        for h in hints {
            if let Some(i) = self.cats.iter().position(|c| c.name.to_lowercase().contains(h)) {
                return i;
            }
        }
        fallback % self.cats.len().max(1)
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

fn into_sample(name: String, wav: Wav, sr: u32) -> Option<Sample> {
    let first = wav.channels.first()?;
    if first.is_empty() {
        return None;
    }
    let keep = (MAX_SECONDS * sr as f32) as usize;
    let mut left = resample(first, wav.sample_rate, sr);
    left.truncate(keep);
    // A second channel that is the first one again is stored once.
    let mut right = wav.channels.get(1).filter(|r| *r != first).map(|r| resample(r, wav.sample_rate, sr));
    if let Some(r) = right.as_mut() {
        r.truncate(keep);
        r.resize(left.len(), 0.0);
    }
    // A cut tail would click, so fade the last few milliseconds.
    let fade = (0.01 * sr as f32) as usize;
    let n = left.len();
    for k in 0..fade.min(n) {
        let g = k as f32 / fade as f32;
        left[n - 1 - k] *= g;
        if let Some(r) = right.as_mut() {
            r[n - 1 - k] *= g;
        }
    }
    let peaks = peaks_of(&left, right.as_deref());
    let (decay, bright) = crate::tags::measure(&left, sr);
    Some(Sample { name, left, right, peaks, decay, bright, tags: AtomicU64::new(0), user: AtomicU64::new(0) })
}

fn peaks_of(left: &[f32], right: Option<&[f32]>) -> Vec<f32> {
    let n = left.len().max(1);
    let mut peaks: Vec<f32> = (0..PEAKS)
        .map(|b| {
            let (a, z) = (b * n / PEAKS, ((b + 1) * n / PEAKS).max(b * n / PEAKS + 1).min(n));
            let mut m = 0.0f32;
            for k in a..z {
                m = m.max(left[k].abs());
                if let Some(r) = right {
                    m = m.max(r[k].abs());
                }
            }
            m
        })
        .collect();
    let top = peaks.iter().cloned().fold(0.0, f32::max);
    if top > 1e-6 {
        for p in peaks.iter_mut() {
            *p /= top;
        }
    }
    peaks
}

// ----------------------------------------------------------------------------------------------
// Folders

fn collect(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() && depth < 4 {
            collect(&p, depth + 1, out);
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("wav")) {
            out.push(p);
        }
    }
}

fn sorted_dirs(dir: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|r| r.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect())
        .unwrap_or_default();
    dirs.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    dirs
}

fn load_category(name: String, dir: &Path, sr: u32, budget: &mut usize, loose: bool) -> Option<Category> {
    let mut files = Vec::new();
    if loose {
        // Only the WAVs sitting directly in the folder.
        if let Ok(entries) = std::fs::read_dir(dir) {
            files.extend(entries.flatten().map(|e| e.path()).filter(|p| p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("wav"))));
        }
    } else {
        collect(dir, 0, &mut files);
    }
    files.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    let mut samples = Vec::new();
    for path in files {
        if *budget == 0 {
            break;
        }
        if std::fs::metadata(&path).map(|m| m.len() > MAX_FILE_BYTES).unwrap_or(true) {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else { continue };
        let Ok(wav) = decode_wav(&bytes) else { continue };
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        if let Some(s) = into_sample(stem, wav, sr) {
            *budget -= 1;
            samples.push(s);
        }
    }
    (!samples.is_empty()).then_some(Category { name, samples })
}

/// Every sub-folder of `root` holding WAVs becomes a category. WAVs loose in `root` form one more,
/// named after the folder. A folder with no sub-folders is a single category.
pub fn load_library(root: &Path, sr: u32) -> Result<Library, String> {
    if !root.is_dir() {
        return Err("not a folder".into());
    }
    let mut budget = MAX_FILES;
    let mut cats = Vec::new();
    for dir in sorted_dirs(root) {
        let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if name.starts_with('.') {
            continue;
        }
        if let Some(c) = load_category(name, &dir, sr, &mut budget, false) {
            cats.push(c);
        }
    }
    let own = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "samples".into());
    if let Some(c) = load_category(own, root, sr, &mut budget, true) {
        cats.push(c);
    }
    if cats.is_empty() {
        return Err("no WAV files".into());
    }
    Ok(Library::new(root.to_string_lossy().into_owned(), sr, cats))
}

// ----------------------------------------------------------------------------------------------
// Handing a library to the audio thread

/// Shared between the loader, the editor and the audio thread.
pub struct Share {
    /// A finished library waiting for the audio thread. It only ever `try_lock`s.
    pub incoming: Mutex<Option<Arc<Library>>>,
    /// Libraries the audio thread has replaced, parked here so they are freed somewhere else.
    pub retired: Mutex<Vec<Arc<Library>>>,
    /// The editor's copy of the newest library.
    pub current: Mutex<Option<Arc<Library>>>,
    pub sample_rate: AtomicU32,
    pub status: Mutex<String>,
    /// Counts up whenever a load finishes, so the editor knows to look again.
    pub generation: AtomicU32,
}

impl Default for Share {
    fn default() -> Self {
        Self {
            incoming: Mutex::new(None),
            retired: Mutex::new(Vec::with_capacity(16)),
            current: Mutex::new(None),
            sample_rate: AtomicU32::new(44100),
            status: Mutex::new(String::new()),
            generation: AtomicU32::new(0),
        }
    }
}

impl Share {
    pub fn status(&self) -> String {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn set_status(&self, text: String) {
        if let Ok(mut s) = self.status.lock() {
            *s = text;
        }
        self.generation.fetch_add(1, Ordering::Relaxed);
    }

    pub fn library(&self) -> Option<Arc<Library>> {
        self.current.lock().ok().and_then(|c| c.clone())
    }

    /// Queue a finished library for the audio thread and for the editor.
    pub fn publish(&self, lib: Library, text: String) {
        let lib = Arc::new(lib);
        if let Ok(mut slot) = self.incoming.lock() {
            *slot = Some(lib.clone());
        }
        if let Ok(mut cur) = self.current.lock() {
            *cur = Some(lib);
        }
        // Anything the audio thread has finished with can go now.
        if let Ok(mut r) = self.retired.lock() {
            r.clear();
        }
        self.set_status(text);
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A WAV file in memory, 16-bit PCM.
    pub fn make_wav(channels: usize, rate: u32, frames: &[Vec<f32>]) -> Vec<u8> {
        let n = frames[0].len();
        let mut data = Vec::new();
        for i in 0..n {
            for c in 0..channels {
                data.extend_from_slice(&((frames[c][i] * 32767.0) as i16).to_le_bytes());
            }
        }
        let mut out = b"RIFF".to_vec();
        out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&(channels as u16).to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * channels as u32 * 2).to_le_bytes());
        out.extend_from_slice(&((channels * 2) as u16).to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&data);
        out
    }

    pub fn tone(n: usize, amp: f32) -> Vec<f32> {
        (0..n).map(|i| amp * (i as f32 * 0.05).sin() * (1.0 - i as f32 / n as f32)).collect()
    }

    /// A scratch folder that cleans up after itself.
    pub struct Scratch(pub PathBuf);

    impl Scratch {
        pub fn new(tag: &str) -> Self {
            let p = std::env::temp_dir().join(format!("digger-test-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Scratch(p)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn every_bit_depth_round_trips_and_bad_files_are_errors() {
        let wav = make_wav(1, 44100, &[tone(500, 0.5)]);
        let d = decode_wav(&wav).unwrap();
        assert_eq!((d.sample_rate, d.channels.len(), d.channels[0].len()), (44100, 1, 500));
        assert!(decode_wav(b"nope").is_err());
        assert!(decode_wav(&wav[..30]).is_err());
        assert!(decode_wav(&[]).is_err());
    }

    #[test]
    fn a_library_is_one_category_per_folder_sorted_and_resampled() {
        let s = Scratch::new("lib");
        for (cat, names) in [("snare", vec!["b", "a"]), ("kick", vec!["z"])] {
            std::fs::create_dir_all(s.0.join(cat)).unwrap();
            for n in names {
                std::fs::write(s.0.join(cat).join(format!("{n}.wav")), make_wav(1, 22050, &[tone(2205, 0.8)])).unwrap();
            }
        }
        std::fs::write(s.0.join("notes.txt"), "not audio").unwrap();
        let lib = load_library(&s.0, 44100).unwrap();
        assert_eq!(lib.cats.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["kick", "snare"]);
        assert_eq!(lib.cats[1].samples.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        // 0.1 s at 22.05 kHz is 0.1 s at 44.1 kHz.
        assert!((lib.cats[0].samples[0].len() as i32 - 4410).abs() <= 2);
        assert_eq!(lib.count(), 3);
        assert!(lib.cats[0].samples[0].right.is_none(), "mono files are stored once");
        let p = &lib.cats[0].samples[0].peaks;
        assert_eq!(p.len(), PEAKS);
        assert!((p.iter().cloned().fold(0.0, f32::max) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn a_folder_of_wavs_is_a_single_category_and_an_empty_one_is_an_error() {
        let s = Scratch::new("flat");
        assert!(load_library(&s.0, 44100).is_err());
        std::fs::write(s.0.join("one.wav"), make_wav(2, 44100, &[tone(300, 0.5), tone(300, 0.2)])).unwrap();
        let lib = load_library(&s.0, 44100).unwrap();
        assert_eq!(lib.cats.len(), 1);
        assert!(lib.cats[0].samples[0].right.is_some(), "a real stereo file keeps both channels");
        assert!(load_library(&s.0.join("missing"), 44100).is_err());
    }

    #[test]
    fn slots_find_their_category_by_name() {
        let cat = |n: &str| Category { name: n.into(), samples: vec![] };
        let lib = Library::new(String::new(), 44100, vec![cat("hat"), cat("hat-open"), cat("Kick")]);
        assert_eq!(lib.default_cat(&["kick"], 0), 2);
        assert_eq!(lib.default_cat(&["hat"], 0), 0, "exact beats partial");
        assert_eq!(lib.default_cat(&["hat-open"], 0), 1);
        assert_eq!(lib.default_cat(&["cymbal"], 4), 1, "no match falls back to a position");
    }
}
