# Audio plugins

Free VST3 plugins written in Rust with [nih-plug](https://github.com/robbert-vdh/nih-plug) and egui.
Each plugin has its own crate under `plugins/`.

| Plugin | What it does |
|---|---|
| [`fake_vibrato`](plugins/fake_vibrato) | Adds vibrato with a modulated delay line. Drag a live pitch trace to set rate and depth. |
| [`aurora_air`](plugins/aurora_air) | Two-band high-frequency exciter. The interface is a live spectrum skyline with the added air as an aurora. |
| [`bedrock`](plugins/bedrock) | Low-end exciter, the counterpart of Aurora Air: Body grows the 2nd and 3rd harmonics of the bass with Chebyshev shapers on a level-normalised signal (polyphonic, so it works on whole mixes, and the same proportions at any level), Sub adds a tracked octave below that fades in only while one steady note is found and steps aside under chords, and a Floor keeps the sub out of the frequencies a system cannot play. No latency. Aurora Air's night sky turned upside down: the dry spectrum is a mountain ridge, what is added hangs below the ground line as magma, and a bridge arcs from the tracked note to its octave. Seventeen factory presets. |
| [`detent`](plugins/detent) | Automatic pitch correction: MPM pitch tracking, scale or MIDI targets, humanize, formant control, a low-latency Live mode and Learn Key. A pitch dial and scrolling tape show what it is doing. |
| [`kiln`](plugins/kiln) | Three-band saturator: four styles, 4x oversampling, asymmetry, program-dependent drive, gain match, plus a tube-console chain (series pentode and triode with density, input and output transformers, a parallel 12AX7 path with a frequency flag, air, calibration). The interface is the transfer curve itself, with the live audio riding on it. |
| [`rime`](plugins/rime) | Dynamic resonance suppressor: a spectral, per-bin cut with depth, sharpness, selectivity, attack/release, stereo link, focus range and delta. Resonances show as ice icicles hanging from an ember spectrum. |
| [`serpent`](plugins/serpent) | De-esser with level-independent detection, a detector that tracks the sibilance, a dynamic bell, shelf or wideband cut, lookahead, and trigger and delta listen modes. Sibilants scroll past as a waterfall with the removed energy glowing red-hot and a snake that follows the detector. |
| [`doppel`](plugins/doppel) | Automatic double tracking: a copy of the vocal on a drifting tape delay with wandering time and pitch, tape saturation and roll-off, and independent stereo drift. Two tape lanes are tied by threads whose slant is the live delay, and a ruler sets the time. Twenty-four factory presets. |
| [`carousel`](plugins/carousel) | Three-voice chorus with analog (BBD) voicing: Juno I / II / I+II, Dimension and Ensemble modes plus a free mode with sine, triangle and random LFOs, 1 to 3 voices, stereo spread, tempo sync and a mono-compatible wet path. Each voice is a horse on a carousel, circling at its LFO phase and spiralling in and out with its delay, and dragging the platform sets the delay. Twenty factory presets. |
| [`chassis`](plugins/chassis) | Robot-voice rig in one plugin: a pitch and formant shifter (PSOLA, with the pitch tracker from Detent), a gate that looks ahead by the shifter's latency, body EQ, then a 3-way LR4 split. Chest: octave-down sub and an 8:1 squash. Body: ring mod, a 16-band vocoder with its own carrier (tracking the voice or a drone note), frequency shifter, comb, antialiased saturation. Shell: a short cavity delay, a granular shine and a metal plate. Then glue compression, three notches and a true-peak limiter, with latency reported. The voice is a spectrum drawn as vent-grille slats in three coloured bays, with draggable bulkheads for the splits and pitch needles for the shift. Fifteen factory presets. |
| [`firefly`](plugins/firefly) | Opto compressor: a two-stage photocell whose release depends on how hard and how long it has been worked, a frequency-weighted sidechain, Compress/Limit, a 4x oversampled tube output stage and parallel mix. The display hangs the gain reduction like icicles of light, the slow memory in magenta under the fast stage in lime, with the threshold as a drag handle. Twenty-four factory presets. |
| [`vice`](plugins/vice) | Three-band upward and downward compressor: LR4 crossovers, a floor that lifts quiet parts and a ceiling that pushes peaks down, depth, time, per-band gain and a phase-coherent dry mix. Two toothed jaws close on three live level bars; drag the jaws and the crossover dividers. Twenty-four factory presets. |
| [`vault`](plugins/vault) | Algorithmic reverb: five spaces (room, hall, plate, chamber, abyss), early reflections, a modulated 8-line feedback network with a three-band decay-rate EQ, a frequency shifter in the loop, freeze, ducking, pre-delay, cuts and width. The tail flies away down a vaulted tunnel of arches, low in orange, mids in lime, highs in cyan, and a graph of decay time against frequency has draggable handles. Twenty-two factory presets. |
| [`skip`](plugins/skip) | Character delay where the feedback loop is the instrument: every repeat passes through allpass diffusion, a granular pitch shifter, a low and a high cut and an antialiased tape saturator, so repeats darken, climb or fall and smear. Slewing, modulated delay time (wow and flutter), tempo sync, ping-pong and ducking. Repeats are stones skipping on a log-time pond, with the time and feedback as drag handles and a scrolling view of the real echoes beneath. Twenty-eight factory presets. |
| [`sunder`](plugins/sunder) | Ring modulator and frequency shifter: Parker's diode ring or an ideal multiplier, a carrier from a sine, triangle, square or saw oscillator, MIDI notes, the input's own tracked pitch or a sidechain, Follow and Wobble modulation, a Sideband control that slides from ring modulation to single-sideband shifting up or down, bias for amplitude modulation, stereo spread and 1x to 8x oversampling. The display splits the spectrum at the carrier: the input rises above the line, the output hangs below it, sum sidebands in cyan, differences in magenta folding off 0 Hz, and anything the sums and differences cannot explain in yellow. Drag it to move the carrier. Twenty-eight factory presets. |
| [`specimen`](plugins/specimen) | Spectral freeze and morph, the open answer to GRM Tools Evolution: four snapshot slots captured from the input, a Position that morphs between them in log magnitude (with Scale and Shift on the frequency axis), Cross mode that imposes the snapshot envelope on the live input or Resynth mode with phase-vocoder freeze, Evolve (LFO or random walk), glide and mix. Four glass slides in a rack, the output spectrum morphing between them. Twenty-two factory presets. |
| [`loom`](plugins/loom) | Spectral playheads, after GRM Tools Fusion: a four-second spectrogram history read by eight heads, each with its own delay, pitch shift and slide, band-pass window, level and pan, plus bounded feedback. The spectrogram is the cloth of a loom and the heads are heddles you drag across it. Twenty-one factory presets. |
| [`abacus`](plugins/abacus) | Sine-bank resynthesis, after GRM Tools Grinder: STFT peak tracking into up to 64 partials with persistent identity, resynthesised by phase-continuous oscillators with freeze of frequency and amplitude, resolution, glide, pitch, stretch, spread and a stochastic noise residual. Each partial is a bead on an abacus wire. Twenty-four factory presets. |
| [`tines`](plugins/tines) | Tuned resonator: a bank of up to 64 modal resonators (string with stiffness, stopped pipe, bar, bell or drum), played from the Note knob, held MIDI keys or the input's own pitch, with chords, a decay that shortens with frequency (Damping), Tilt, stereo Detune, slow Drift, a Feedback loop that sustains and self-oscillates, onset-triggered noise Pluck, Freeze and a soft limiter. No latency. The display is a comb of tuning-fork tines on a log spectrum, each rising with the amplitude its resonator is ringing at, with a diamond to drag for pitch and decay. Twenty-three factory presets, three of them for a Dr. Hayden voice. |

## Screenshots

![All the plugins](docs/screenshots/all.png)

Rendered headlessly from the real editors, with each plugin's DSP running on a synthetic signal.
Regenerate them, and the montage above, with `scripts/screenshots.sh [plugin ...]`. The individual
shots are in `docs/screenshots/<plugin>.png`.

Each plugin has an `examples/screenshot.rs` that builds its DSP and params, feeds the DSP a signal,
and hands the editor's `ui::draw` to `crates/ui_preview`, which runs it in an `egui_kittest` harness
on a wgpu renderer (no window, no host), and `crates/ui_preview`'s `stitch` example packs the results into `all.png`. Parameter edits are ignored, so the knobs show the defaults.
To add one for a new plugin, make its `ui` and `dsp` modules `pub`, add `ui_preview` as a
dev-dependency and copy `plugins/rime/examples/screenshot.rs`.
| [`smolder`](plugins/smolder) | Distortion with eight waveshapers, optional 3-band split with a model and drive per band, LFO or envelope modulation, bias, low cut, tone, 1x to 8x oversampling and mix. Fifteen factory presets and live transfer curve, waveform and harmonic views. |

## Build and install

```sh
scripts/install.sh              # build and install every plugin
scripts/install.sh detent       # just one
```

This bundles the plugins into `target/bundled/` and copies them to
`~/Library/Audio/Plug-Ins/VST3`, and plugins with factory presets get theirs written to
`~/Library/Audio/Presets/Clowd/<name>` as `.vstpreset` files. Restart your DAW afterwards: a re-scan only updates its plugin list,
and the DAW keeps the old binary loaded until it quits.

To build without installing: `cargo xtask bundle <plugin> --release`.
Plugin display names live in `bundler.toml`.

## CI and releases

`.github/workflows/ci.yml` runs on every push to `main` and every pull request. `scripts/affected.py`
works out which plugins the change touches (a plugin's own files, or a shared crate such as
`preset_kit` that it depends on; workspace-level files rebuild everything), and each one is tested,
bundled and signature-checked on macOS as its own job. The `ci` job is the single check to require.

To publish a release, push a version tag (`jj tag set v1.0.0 -r main && jj git push`, or
`git tag v1.0.0 && git push --tags`). `.github/workflows/release.yml` tests the workspace, bundles every
plugin, and creates a GitHub release with one zip per plugin, an all-in-one zip and `SHA256SUMS`.
The builds are for the runner's architecture (Apple silicon) and ad-hoc signed.

## Presets

Every plugin has factory presets in a `presets.rs`, built on the shared `crates/preset_kit`. A preset
lists only the parameters it changes from the defaults. The same table drives the `< preset >` picker
in each editor (it reads CUSTOM once you move anything) and the `.vstpreset` files that
`scripts/install.sh` writes for hosts like REAPER. To add presets to a plugin, copy `plugins/kiln`'s
`presets.rs`, `examples/write_presets.rs` and `Cargo.toml` changes, and call `preset_kit::ui::picker`
from the editor.

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
