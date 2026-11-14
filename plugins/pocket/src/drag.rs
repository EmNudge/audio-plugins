//! Dragging a MIDI file out of the editor and into the host.
//!
//! On macOS this starts a native drag session on the host window, the same thing a Finder file
//! drag does, so DAWs that take dropped `.mid` files (REAPER, Ableton, Logic, Bitwig, FL Studio,
//! Cubase) treat it as one. The cursor carries a little picture of the clip.

use crate::pattern::*;
use std::path::Path;

pub const CHIP_W: usize = 176;
pub const CHIP_H: usize = 72;

/// RGB for each family, matching the editor's palette.
fn family_rgb(f: Family) -> [u8; 3] {
    match f {
        Family::Kick => [255, 85, 61],
        Family::Snare => [255, 207, 31],
        Family::Cymbal => [46, 226, 166],
        Family::Tom => [58, 160, 255],
    }
}

/// The clip as a picture: a dark card with a dot for every hit, one row per drum.
pub fn chip_pixels(store: &PatternStore) -> Vec<u8> {
    let mut px = vec![0u8; CHIP_W * CHIP_H * 4];
    let put = |px: &mut Vec<u8>, x: usize, y: usize, rgb: [u8; 3], a: u8| {
        if x < CHIP_W && y < CHIP_H {
            let i = (y * CHIP_W + x) * 4;
            px[i..i + 4].copy_from_slice(&[rgb[0], rgb[1], rgb[2], a]);
        }
    };
    // A rounded card.
    let r = 9.0f32;
    for y in 0..CHIP_H {
        for x in 0..CHIP_W {
            let (xc, yc) = (x as f32 + 0.5, y as f32 + 0.5);
            let dx = (r - xc).max(xc - (CHIP_W as f32 - r)).max(0.0);
            let dy = (r - yc).max(yc - (CHIP_H as f32 - r)).max(0.0);
            let inside = (dx * dx + dy * dy).sqrt() <= r;
            if inside {
                put(&mut px, x, y, [20, 22, 28], 235);
            }
        }
    }
    let steps = store.steps();
    let (left, top) = (10usize, 9usize);
    let cell_w = (CHIP_W - 2 * left) as f32 / steps as f32;
    let row_h = (CHIP_H - 2 * top) as f32 / LANES as f32;
    for lane in 0..LANES {
        let rgb = family_rgb(LANE_INFO[lane].family);
        for s in 0..steps {
            let cell = store.get(lane, s);
            if !cell.is_on() {
                continue;
            }
            let a = (110.0 + cell.vel() as f32 / 127.0 * 145.0) as u8;
            let cx = left as f32 + (s as f32 + 0.5) * cell_w;
            let cy = top as f32 + (lane as f32 + 0.5) * row_h;
            let rad = (row_h * 0.42).max(1.4);
            for y in (cy - rad).floor() as usize..=(cy + rad).ceil() as usize {
                for x in (cx - rad).floor() as usize..=(cx + rad).ceil() as usize {
                    let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
                    if d <= rad {
                        put(&mut px, x, y, rgb, a);
                    }
                }
            }
        }
    }
    px
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 == 1 { crc >> 1 ^ 0xedb8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    b << 16 | a
}

/// A PNG with no compression (stored deflate blocks). The pictures are tiny.
pub fn png(width: usize, height: usize, rgba: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity((width * 4 + 1) * height);
    for row in rgba.chunks(width * 4) {
        raw.push(0);
        raw.extend(row);
    }
    let mut z = vec![0x78, 0x01];
    let mut chunks = raw.chunks(65535).peekable();
    while let Some(block) = chunks.next() {
        z.push(chunks.peek().is_none() as u8);
        z.extend((block.len() as u16).to_le_bytes());
        z.extend((!(block.len() as u16)).to_le_bytes());
        z.extend(block);
    }
    z.extend(adler32(&raw).to_be_bytes());

    let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let mut chunk = |kind: &[u8; 4], data: &[u8]| {
        out.extend((data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend(data);
        out.extend(&body);
        out.extend(crc32(&body).to_be_bytes());
    };
    let mut header = Vec::new();
    header.extend((width as u32).to_be_bytes());
    header.extend((height as u32).to_be_bytes());
    header.extend([8, 6, 0, 0, 0]);
    chunk(b"IHDR", &header);
    chunk(b"IDAT", &z);
    chunk(b"IEND", &[]);
    out
}

/// Start dragging `file` out of the editor. Returns whether a drag began; if it did not, the
/// caller should offer the saved file instead.
pub fn start(file: &Path, store: &PatternStore) -> bool {
    let image = png(CHIP_W, CHIP_H, &chip_pixels(store));
    platform::start(file, image)
}

#[cfg(target_os = "macos")]
mod platform {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    use raw_window_handle::{AppKitWindowHandle, HandleError, HasWindowHandle, RawWindowHandle, WindowHandle};
    use std::ffi::c_void;
    use std::path::Path;
    use std::ptr::NonNull;

    struct HostView(NonNull<c_void>);

    impl HasWindowHandle for HostView {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            // SAFETY: the view is the content view of the key window, alive for this call.
            Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::AppKit(AppKitWindowHandle::new(self.0))) })
        }
    }

    pub fn start(file: &Path, image: Vec<u8>) -> bool {
        // The editor runs on the host's UI thread, which is the main thread; anything else cannot
        // start a drag.
        let Some(mtm) = MainThreadMarker::new() else { return false };
        let app = NSApplication::sharedApplication(mtm);
        // The window the mouse is in is the key window while a button is down in it.
        let Some(window) = app.keyWindow().or_else(|| app.mainWindow()) else { return false };
        let Some(view) = window.contentView() else { return false };
        let handle = HostView(NonNull::from(&*view).cast());
        drag::start_drag(
            &handle,
            drag::DragItem::Files(vec![file.to_path_buf()]),
            drag::Image::Raw(image),
            |_, _| {},
            drag::Options::default(),
        )
        .is_ok()
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use std::path::Path;

    pub fn start(_file: &Path, _image: Vec<u8>) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunks(png: &[u8]) -> Vec<(String, Vec<u8>)> {
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
        let mut i = 8;
        let mut out = Vec::new();
        while i < png.len() {
            let len = u32::from_be_bytes(png[i..i + 4].try_into().unwrap()) as usize;
            let body = &png[i + 4..i + 8 + len];
            let crc = u32::from_be_bytes(png[i + 8 + len..i + 12 + len].try_into().unwrap());
            assert_eq!(crc, crc32(body), "chunk CRC");
            out.push((String::from_utf8_lossy(&body[..4]).into_owned(), body[4..].to_vec()));
            i += 12 + len;
        }
        out
    }

    #[test]
    fn crc_and_adler_match_known_values() {
        assert_eq!(crc32(b"IEND"), 0xae42_6082);
        assert_eq!(adler32(b"Wikipedia"), 0x11e6_0398);
    }

    #[test]
    fn the_png_is_well_formed_and_round_trips_its_pixels() {
        let store = PatternStore::default();
        store.set(LANE_KICK, 0, Cell::hit(127));
        let rgba = chip_pixels(&store);
        let file = png(CHIP_W, CHIP_H, &rgba);
        let parts = chunks(&file);
        assert_eq!(parts.iter().map(|p| p.0.as_str()).collect::<Vec<_>>(), ["IHDR", "IDAT", "IEND"]);
        assert_eq!(u32::from_be_bytes(parts[0].1[..4].try_into().unwrap()) as usize, CHIP_W);
        // Undo the stored-block deflate and the row filter bytes.
        let z = &parts[1].1;
        let mut raw = Vec::new();
        let mut i = 2;
        loop {
            let last = z[i] == 1;
            let len = u16::from_le_bytes([z[i + 1], z[i + 2]]) as usize;
            raw.extend(&z[i + 5..i + 5 + len]);
            i += 5 + len;
            if last {
                break;
            }
        }
        assert_eq!(adler32(&raw), u32::from_be_bytes(z[i..i + 4].try_into().unwrap()));
        let rows: Vec<u8> = raw.chunks(CHIP_W * 4 + 1).flat_map(|r| r[1..].to_vec()).collect();
        assert_eq!(rows, rgba);
    }

    #[test]
    fn the_chip_shows_the_hits_in_their_lane_colours() {
        let store = PatternStore::default();
        store.set(LANE_KICK, 0, Cell::hit(127));
        store.set(LANE_HAT, 8, Cell::hit(127));
        let px = chip_pixels(&store);
        let at = |x: usize, y: usize| &px[(y * CHIP_W + x) * 4..][..4];
        let cell_w = (CHIP_W - 20) as f32 / 16.0;
        let row_h = (CHIP_H - 18) as f32 / LANES as f32;
        let centre = |s: usize, l: usize| (10.0 + (s as f32 + 0.5) * cell_w, 9.0 + (l as f32 + 0.5) * row_h);
        let (kx, ky) = centre(0, LANE_KICK);
        assert_eq!(&at(kx as usize, ky as usize)[..3], &[255, 85, 61]);
        let (hx, hy) = centre(8, LANE_HAT);
        assert_eq!(&at(hx as usize, hy as usize)[..3], &[46, 226, 166]);
        let (ex, ey) = centre(5, LANE_SNARE);
        assert_eq!(&at(ex as usize, ey as usize)[..3], &[20, 22, 28]);
        assert_eq!(at(0, 0)[3], 0, "the corner is cut off");
    }
}
