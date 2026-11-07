//! Writes the factory presets as `.vstpreset` files: `cargo run -p chassis --example write_presets -- <dir>`.

fn main() {
    let dir = std::env::args().nth(1).expect("usage: write_presets <dir>");
    chassis::presets::BANK
        .write_vstpresets(std::path::Path::new(&dir), &chassis::ChassisParams::default())
        .expect("could not write presets");
}
