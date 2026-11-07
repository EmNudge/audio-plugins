//! Writes the factory presets as `.vstpreset` files: `cargo run -p sunder --example write_presets -- <dir>`.

fn main() {
    let dir = std::env::args().nth(1).expect("usage: write_presets <dir>");
    sunder::presets::BANK
        .write_vstpresets(std::path::Path::new(&dir), &sunder::SunderParams::default())
        .expect("could not write presets");
}
