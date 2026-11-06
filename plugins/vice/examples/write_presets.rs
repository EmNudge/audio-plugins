//! Writes the factory presets as `.vstpreset` files: `cargo run -p vice --example write_presets -- <dir>`.

fn main() {
    let dir = std::env::args().nth(1).expect("usage: write_presets <dir>");
    vice::presets::BANK
        .write_vstpresets(std::path::Path::new(&dir), &vice::ViceParams::default())
        .expect("could not write presets");
}
