//! Writes the factory presets as `.vstpreset` files: `cargo run -p skip --example write_presets -- <dir>`.

fn main() {
    let dir = std::env::args().nth(1).expect("usage: write_presets <dir>");
    skip::presets::BANK
        .write_vstpresets(std::path::Path::new(&dir), &skip::SkipParams::default())
        .expect("could not write presets");
}
