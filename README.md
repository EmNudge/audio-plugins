# Audio plugins

Free VST3 plugins written in Rust with [nih-plug](https://github.com/robbert-vdh/nih-plug) and egui.

Each plugin has its own crate under `plugins/`.

![All the plugins](docs/screenshots/all.png)

## Plugins

**Pitch and modulation**

- [`detent`](plugins/detent): automatic pitch correction.
- [`fake_vibrato`](plugins/fake_vibrato): vibrato from a modulated delay.
- [`carousel`](plugins/carousel): three-voice BBD chorus.
- [`doppel`](plugins/doppel): automatic double tracking.
- [`sunder`](plugins/sunder): ring modulator and frequency shifter.
- [`chassis`](plugins/chassis): robot-voice rig.

**Dynamics and EQ**

- [`rime`](plugins/rime): dynamic resonance suppressor.
- [`serpent`](plugins/serpent): de-esser.
- [`firefly`](plugins/firefly): opto compressor.
- [`vice`](plugins/vice): three-band upward and downward compressor.

**Colour and space**

- [`aurora_air`](plugins/aurora_air): high-frequency exciter.
- [`bedrock`](plugins/bedrock): low-end exciter.
- [`kiln`](plugins/kiln): three-band saturator.
- [`smolder`](plugins/smolder): distortion.
- [`vault`](plugins/vault): algorithmic reverb.
- [`skip`](plugins/skip): character delay.

**Spectral**

- [`specimen`](plugins/specimen): spectral freeze and morph.
- [`loom`](plugins/loom): spectral playheads.
- [`abacus`](plugins/abacus): sine-bank resynthesis.
- [`tines`](plugins/tines): tuned resonator.

## Build and install

```sh
scripts/install.sh              # every plugin
scripts/install.sh detent       # just one
```

This bundles into `target/bundled/`, copies to `~/Library/Audio/Plug-Ins/VST3`, and writes factory
presets to `~/Library/Audio/Presets/Clowd/<name>` as `.vstpreset` files.

Restart your DAW afterwards. A re-scan only updates its plugin list, and the old binary stays loaded
until it quits.

To build without installing: `cargo xtask bundle <plugin> --release`.

Display names live in `bundler.toml`.

## Test

```sh
cargo test --release
```

Each plugin tests its DSP on synthetic signals.

## Adding a plugin

1. Create `plugins/<name>/` with a `Cargo.toml` like the others (`crate-type = ["cdylib"]`, workspace
   dependencies on `nih_plug` and `nih_plug_egui`) and a `src/lib.rs`.
2. Add an entry to `bundler.toml`.
3. Pick a unique 16-byte `VST3_CLASS_ID`.

## Presets

Factory presets live in each plugin's `presets.rs`, built on `crates/preset_kit`.

A preset lists only the parameters it changes from the defaults. One table drives both the `< preset >`
picker in the editor (it reads CUSTOM once you move anything) and the `.vstpreset` files.

To add presets, copy `plugins/kiln`'s `presets.rs`, `examples/write_presets.rs` and `Cargo.toml`
changes, then call `preset_kit::ui::picker` from the editor.

## Screenshots

Regenerate them, and the montage above, with `scripts/screenshots.sh [plugin ...]`.

They are rendered headlessly from the real editors, with each plugin's DSP running on a synthetic
signal. Parameter edits are ignored, so knobs show their defaults.

To add one, make the plugin's `ui` and `dsp` modules `pub`, add `ui_preview` as a dev-dependency and
copy `plugins/rime/examples/screenshot.rs`.

## CI and releases

CI runs on every push to `main` and every pull request.

`scripts/affected.py` works out which plugins a change touches, and each one is tested, bundled and
signature-checked on macOS as its own job. The `ci` job is the single check to require.

To release, push a version tag:

```sh
jj tag set v1.0.0 -r main && jj git push
```

`release.yml` tests the workspace and creates a GitHub release with one zip per plugin, an all-in-one
zip and `SHA256SUMS`. Builds are Apple silicon and ad-hoc signed.

## Licence

The VST3 bindings that nih-plug uses are GPLv3, so these plugins are GPL-3.0-or-later.
