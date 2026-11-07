//! Writes the factory presets as `.vstpreset` files: `cargo run -p abacus --example write_presets -- <dir>`.

fn main() {
    let dir = std::env::args().nth(1).expect("usage: write_presets <dir>");
    abacus::presets::BANK
        .write_vstpresets(std::path::Path::new(&dir), &abacus::AbacusParams::default())
        .expect("could not write presets");
}
