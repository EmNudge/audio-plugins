//! Stamps the build time into the plugin so the editor can show which build a host has loaded.
fn main() {
    let now = std::process::Command::new("date").arg("+%Y-%m-%d %H:%M").output().ok().and_then(|o| String::from_utf8(o.stdout).ok()).unwrap_or_default();
    println!("cargo:rustc-env=TYMPAN_BUILD={}", now.trim());
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src");
}
