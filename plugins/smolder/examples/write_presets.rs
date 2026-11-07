//! Writes the factory presets as `.vstpreset` files: `cargo run -p smolder --example write_presets -- <dir>`.

fn main() {
    let dir = std::env::args().nth(1).expect("usage: write_presets <dir>");
    smolder::presets::BANK
        .write_vstpresets(std::path::Path::new(&dir), &smolder::SmolderParams::default())
        .expect("could not write presets");
}
