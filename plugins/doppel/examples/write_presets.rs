//! Writes the factory presets as `.vstpreset` files: `cargo run -p doppel --example write_presets -- <dir>`.

fn main() {
    let dir = std::env::args().nth(1).expect("usage: write_presets <dir>");
    doppel::presets::BANK
        .write_vstpresets(std::path::Path::new(&dir), &doppel::DoppelParams::default())
        .expect("could not write presets");
}
