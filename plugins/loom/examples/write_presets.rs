//! Writes the factory presets as `.vstpreset` files: `cargo run -p loom --example write_presets -- <dir>`.

fn main() {
    let dir = std::env::args().nth(1).expect("usage: write_presets <dir>");
    loom::presets::BANK
        .write_vstpresets(std::path::Path::new(&dir), &loom::LoomParams::default())
        .expect("could not write presets");
}
