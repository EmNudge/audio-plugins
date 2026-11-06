//! Writes the factory presets as `.vstpreset` files: `cargo run -p detent --example write_presets -- <dir>`.

fn main() {
    let dir = std::env::args().nth(1).expect("usage: write_presets <dir>");
    detent::presets::BANK
        .write_vstpresets(std::path::Path::new(&dir), &detent::DetentParams::default())
        .expect("could not write presets");
}
