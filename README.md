# Audio plugins

Free VST3 plugins written in Rust with [nih-plug](https://github.com/robbert-vdh/nih-plug) and egui.
Each plugin has its own crate under `plugins/`.

| Plugin | What it does |
|---|---|
| [`fake_vibrato`](plugins/fake_vibrato) | Adds vibrato with a modulated delay line. Drag a live pitch trace to set rate and depth. |
| [`aurora_air`](plugins/aurora_air) | Two-band high-frequency exciter. The interface is a live spectrum skyline with the added air as an aurora. |
| [`detent`](plugins/detent) | Automatic pitch correction: MPM pitch tracking, scale or MIDI targets, humanize, formant control, a low-latency Live mode and Learn Key. A pitch dial and scrolling tape show what it is doing. |

## Build and install

```sh
scripts/install.sh              # build and install every plugin
scripts/install.sh detent       # just one
```

This bundles the plugins into `target/bundled/` and copies them to
`~/Library/Audio/Plug-Ins/VST3`. Restart your DAW afterwards: a re-scan only updates its plugin list,
and the DAW keeps the old binary loaded until it quits.

To build without installing: `cargo xtask bundle <plugin> --release`.
Plugin display names live in `bundler.toml`.

## Test

```sh
cargo test --release
```

Detent has the most tests. They run its pitch tracker, note logic and shifter on synthetic voices.
`DETENT_FILES=name cargo test -p detent --release offline -- --ignored --nocapture` runs it over
raw mono f32 recordings in `/tmp/detent`; see `plugins/detent/src/tests.rs`.

## Adding a plugin

1. Create `plugins/<name>/` with a `Cargo.toml` like the others (`crate-type = ["cdylib"]`, workspace
   dependencies on `nih_plug` and `nih_plug_egui`) and a `src/lib.rs`.
2. Add an entry to `bundler.toml`.
3. Pick a unique 16-byte `VST3_CLASS_ID`.

## Licence

The VST3 bindings that nih-plug uses are GPLv3, so these plugins are GPL-3.0-or-later.
