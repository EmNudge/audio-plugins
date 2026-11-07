//! Writes the factory presets as `.vstpreset` files: `cargo run -p bedrock --example write_presets -- <dir>`.

fn main() {
    let dir = std::env::args().nth(1).expect("usage: write_presets <dir>");
    bedrock::presets::BANK
        .write_vstpresets(std::path::Path::new(&dir), &bedrock::BedrockParams::default())
        .expect("could not write presets");
}
