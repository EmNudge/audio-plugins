//! Writes the factory presets as `.vstpreset` files: `cargo run -p tympan --example write_presets -- <dir>`.

fn main() {
    let dir = std::env::args().nth(1).expect("usage: write_presets <dir>");
    tympan::presets::BANK
        .write_vstpresets(std::path::Path::new(&dir), &tympan::TympanParams::default())
        .expect("could not write presets");
}
