//! Scales as 12-bit pitch-class masks, and snapping to the nearest allowed note.

use nih_plug::prelude::Enum;

#[derive(Enum, Debug, PartialEq, Clone, Copy)]
pub enum Scale {
    #[name = "Chromatic"]
    Chromatic,
    #[name = "Major"]
    Major,
    #[name = "Minor"]
    Minor,
    #[name = "Harmonic Minor"]
    HarmonicMinor,
    #[name = "Major Pentatonic"]
    MajorPentatonic,
    #[name = "Minor Pentatonic"]
    MinorPentatonic,
    #[name = "Blues"]
    Blues,
    #[name = "Dorian"]
    Dorian,
    #[name = "Mixolydian"]
    Mixolydian,
}

impl Scale {
    fn intervals(self) -> &'static [u32] {
        match self {
            Scale::Chromatic => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
            Scale::Major => &[0, 2, 4, 5, 7, 9, 11],
            Scale::Minor => &[0, 2, 3, 5, 7, 8, 10],
            Scale::HarmonicMinor => &[0, 2, 3, 5, 7, 8, 11],
            Scale::MajorPentatonic => &[0, 2, 4, 7, 9],
            Scale::MinorPentatonic => &[0, 3, 5, 7, 10],
            Scale::Blues => &[0, 3, 5, 6, 7, 10],
            Scale::Dorian => &[0, 2, 3, 5, 7, 9, 10],
            Scale::Mixolydian => &[0, 2, 4, 5, 7, 9, 10],
        }
    }
}

pub const NOTE_NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

/// Bit `i` is set if pitch class `i` (0 = C) is allowed. `flips` inverts individual notes.
pub fn mask(key: usize, scale: Scale, flips: [bool; 12]) -> u16 {
    let mut mask = 0u16;
    for interval in scale.intervals() {
        mask |= 1 << ((key as u32 + interval) % 12);
    }
    for (pc, flip) in flips.iter().enumerate() {
        if *flip {
            mask ^= 1 << pc;
        }
    }
    mask
}

/// The nearest allowed MIDI note to `midi`. With an empty mask the pitch is left alone.
pub fn nearest_allowed(midi: f32, mask: u16) -> f32 {
    if mask == 0 {
        return midi;
    }
    let base = midi.floor() as i32;
    let mut best = midi;
    let mut best_dist = f32::MAX;
    for n in base - 12..=base + 13 {
        if mask & (1 << n.rem_euclid(12)) != 0 {
            let dist = (midi - n as f32).abs();
            if dist < best_dist {
                best_dist = dist;
                best = n as f32;
            }
        }
    }
    best
}

pub fn is_allowed(midi: f32, mask: u16) -> bool {
    mask & (1 << (midi.round() as i32).rem_euclid(12)) != 0
}

/// Pitch classes of the MIDI notes currently held down.
pub fn midi_mask(held: &[bool; 128]) -> u16 {
    held.iter().enumerate().fold(0u16, |mask, (note, on)| if *on { mask | 1 << (note % 12) } else { mask })
}

/// Adds one pitch reading to a pitch-class histogram. The reading is split between its two
/// neighbouring semitones, so a slightly out-of-tune note still counts toward the right one.
pub fn add_to_histogram(histogram: &mut [f32; 12], midi: f32) {
    let lower = midi.floor();
    let frac = midi - lower;
    histogram[(lower as i32).rem_euclid(12) as usize] += 1.0 - frac;
    histogram[(lower as i32 + 1).rem_euclid(12) as usize] += frac;
}

/// Krumhansl-Kessler key profiles: how strongly each scale degree is felt in a major or minor key.
const MAJOR_PROFILE: [f32; 12] = [6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88];
const MINOR_PROFILE: [f32; 12] = [6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17];

fn correlation(a: &[f32; 12], b: &[f32; 12]) -> f32 {
    let (ma, mb) = (a.iter().sum::<f32>() / 12.0, b.iter().sum::<f32>() / 12.0);
    let (mut num, mut da, mut db) = (0.0, 0.0, 0.0);
    for i in 0..12 {
        num += (a[i] - ma) * (b[i] - mb);
        da += (a[i] - ma).powi(2);
        db += (b[i] - mb).powi(2);
    }
    num / (da * db).sqrt().max(1e-9)
}

/// Best-fitting major or minor key for a pitch-class histogram (index 0 = C), with its correlation.
/// Returns `None` when the histogram is empty.
pub fn detect_key(histogram: &[f32; 12]) -> Option<(usize, Scale, f32)> {
    if histogram.iter().sum::<f32>() <= 0.0 {
        return None;
    }
    let mut best: Option<(usize, Scale, f32)> = None;
    for key in 0..12 {
        let rotated: [f32; 12] = std::array::from_fn(|pc| histogram[(pc + key) % 12]);
        for (profile, scale) in [(&MAJOR_PROFILE, Scale::Major), (&MINOR_PROFILE, Scale::Minor)] {
            let score = correlation(&rotated, profile);
            if best.map_or(true, |b| score > b.2) {
                best = Some((key, scale, score));
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn histogram(notes: &[(usize, f32)]) -> [f32; 12] {
        let mut h = [0.0; 12];
        for (pc, weight) in notes {
            h[*pc] += weight;
        }
        h
    }

    #[test]
    fn finds_c_major() {
        // A melody that leans on C, E and G, with passing tones.
        let h = histogram(&[(0, 9.0), (2, 4.0), (4, 6.0), (5, 3.0), (7, 7.0), (9, 3.0), (11, 2.0)]);
        let (key, scale, _) = detect_key(&h).unwrap();
        assert_eq!((key, scale), (0, Scale::Major));
    }

    #[test]
    fn finds_a_minor_and_a_transposed_key() {
        let a_minor = histogram(&[(9, 9.0), (11, 3.0), (0, 6.0), (2, 3.0), (4, 7.0), (5, 3.0), (7, 3.0)]);
        let (key, scale, _) = detect_key(&a_minor).unwrap();
        assert_eq!((key, scale), (9, Scale::Minor));

        // The same major melody moved up three semitones is E-flat major.
        let c_major = [(0, 9.0), (2, 4.0), (4, 6.0), (5, 3.0), (7, 7.0), (9, 3.0), (11, 2.0)];
        let shifted: Vec<(usize, f32)> = c_major.iter().map(|(pc, w)| ((pc + 3) % 12, *w)).collect();
        let (key, scale, _) = detect_key(&histogram(&shifted)).unwrap();
        assert_eq!((key, scale), (3, Scale::Major));
    }

    #[test]
    fn empty_histogram_has_no_key() {
        assert!(detect_key(&[0.0; 12]).is_none());
    }
}
