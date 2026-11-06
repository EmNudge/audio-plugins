//! Writes the factory presets as `.vstpreset` files: `cargo run -p rime --example write_presets -- <dir>`.

fn main() {
    let dir = std::env::args().nth(1).expect("usage: write_presets <dir>");
    rime::presets::BANK
        .write_vstpresets(std::path::Path::new(&dir), &rime::RimeParams::default())
        .expect("could not write presets");
}
