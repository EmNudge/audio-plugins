//! Writes the factory presets as `.vstpreset` files: `cargo run -p vault --example write_presets -- <dir>`.

fn main() {
    let dir = std::env::args().nth(1).expect("usage: write_presets <dir>");
    vault::presets::BANK
        .write_vstpresets(std::path::Path::new(&dir), &vault::VaultParams::default())
        .expect("could not write presets");
}
