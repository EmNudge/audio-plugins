//! Writes the factory presets as `.vstpreset` files: `cargo run -p serpent --example write_presets -- <dir>`.

fn main() {
    let dir = std::env::args().nth(1).expect("usage: write_presets <dir>");
    serpent::presets::BANK
        .write_vstpresets(std::path::Path::new(&dir), &serpent::SerpentParams::default())
        .expect("could not write presets");
}
