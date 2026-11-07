//! Writes the factory presets as `.vstpreset` files: `cargo run -p specimen --example write_presets -- <dir>`.

fn main() {
    let dir = std::env::args().nth(1).expect("usage: write_presets <dir>");
    specimen::presets::BANK
        .write_vstpresets(std::path::Path::new(&dir), &specimen::SpecimenParams::default())
        .expect("could not write presets");
}
