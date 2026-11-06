//! Writes the factory presets as `.vstpreset` files: `cargo run -p fake_vibrato --example write_presets -- <dir>`.

fn main() {
    let dir = std::env::args().nth(1).expect("usage: write_presets <dir>");
    fake_vibrato::presets::BANK
        .write_vstpresets(std::path::Path::new(&dir), &fake_vibrato::VibratoParams::default())
        .expect("could not write presets");
}
