//! Packs every `docs/screenshots/<plugin>.png` into one montage, `docs/screenshots/all.png`:
//! `cargo run -p ui_preview --example stitch --release [-- dir]`.
//!
//! Columns of equal width, each image dropped into whichever column is currently shortest.

use image::{imageops, Rgba, RgbaImage};

const COLUMNS: usize = 3;
const WIDTH: u32 = 560;
const GAP: u32 = 16;
const BACKDROP: Rgba<u8> = Rgba([0x0b, 0x0b, 0x0d, 0xff]);

fn main() {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "docs/screenshots".into()));
    let mut paths: Vec<_> = std::fs::read_dir(&dir)
        .expect("could not read screenshot directory")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "png") && p.file_stem().is_some_and(|s| s != "all"))
        .collect();
    paths.sort();

    let shots: Vec<RgbaImage> = paths
        .iter()
        .map(|p| {
            let img = image::open(p).expect("could not read screenshot").to_rgba8();
            let height = (img.height() as f32 * WIDTH as f32 / img.width() as f32).round() as u32;
            imageops::resize(&img, WIDTH, height, imageops::FilterType::Lanczos3)
        })
        .collect();

    // Tallest first packs tighter.
    let mut order: Vec<usize> = (0..shots.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(shots[i].height()));
    let mut heights = [GAP; COLUMNS];
    let mut placed = Vec::new();
    for i in order {
        let col = (0..COLUMNS).min_by_key(|&c| heights[c]).unwrap();
        placed.push((i, col as u32 * (WIDTH + GAP) + GAP, heights[col]));
        heights[col] += shots[i].height() + GAP;
    }

    let total_h = *heights.iter().max().unwrap();
    let mut canvas = RgbaImage::from_pixel(COLUMNS as u32 * (WIDTH + GAP) + GAP, total_h, BACKDROP);
    for (i, x, y) in placed {
        imageops::overlay(&mut canvas, &shots[i], x as i64, y as i64);
    }
    let out = dir.join("all.png");
    canvas.save(&out).expect("could not write montage");
    println!("{}", out.display());
}
