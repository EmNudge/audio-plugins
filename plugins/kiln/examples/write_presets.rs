//! Writes the factory presets as `.vstpreset` files: `cargo run -p kiln --example write_presets -- <dir>`.

fn main() {
    let dir = std::env::args().nth(1).expect("usage: write_presets <dir>");
    kiln::presets::BANK
        .write_vstpresets(std::path::Path::new(&dir), &kiln::KilnParams::default())
        .expect("could not write presets");
}
