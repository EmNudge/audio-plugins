//! Writes the factory presets as `.vstpreset` files: `cargo run -p carousel --example write_presets -- <dir>`.

fn main() {
    let dir = std::env::args().nth(1).expect("usage: write_presets <dir>");
    carousel::presets::BANK
        .write_vstpresets(std::path::Path::new(&dir), &carousel::CarouselParams::default())
        .expect("could not write presets");
}
