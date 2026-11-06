//! Writes the factory presets as `.vstpreset` files: `cargo run -p aurora_air --example write_presets -- <dir>`.

fn main() {
    let dir = std::env::args().nth(1).expect("usage: write_presets <dir>");
    aurora_air::presets::BANK
        .write_vstpresets(std::path::Path::new(&dir), &aurora_air::AirParams::default())
        .expect("could not write presets");
}
