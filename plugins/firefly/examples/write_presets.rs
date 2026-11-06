//! Writes the factory presets as `.vstpreset` files: `cargo run -p firefly --example write_presets -- <dir>`.

fn main() {
    let dir = std::env::args().nth(1).expect("usage: write_presets <dir>");
    firefly::presets::BANK
        .write_vstpresets(std::path::Path::new(&dir), &firefly::FireflyParams::default())
        .expect("could not write presets");
}
