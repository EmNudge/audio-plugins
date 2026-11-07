# Audio plugins

Free VST3 plugins in Rust with nih-plug and egui, one crate per plugin under `plugins/`.
See `README.md` for the build, test and "adding a plugin" steps.

## Goal

Make plugins that are **visually distinctive and technically serious**. Both halves matter equally:

- The DSP has to be real: the kind of algorithm a paid plugin would use (spectral processing,
  proper oversampling, correct latency reporting), not a toy. Test it on synthetic signals.
- The interface has to be something people want to look at. Draw inspiration from FL Studio's
  playful, colourful, animated plugin UIs and from Teenage Engineering's industrial-minimalist
  design: bold flat colour, a tight palette, monospace type, tactile hardware-like controls,
  small witty details, lots of negative space.

## Visual language

- **The display is the interface.** Each plugin gets a live, animated visualisation of what its
  DSP is doing, built around a single metaphor, and that visualisation is also where the main
  controls live (drag handles on the graph) wherever that makes sense. No generic grey rows of knobs
  and a spectrum analyser.
- **One metaphor per plugin**, with its own palette, so the plugins look related but not alike.
  Existing ones: Aurora Air (night-sky skyline, neon aurora), Bedrock (the same scene upside down: a mountain-ridge spectrum above the ground line and the added bass as magma hanging below it, with a bridge from the tracked note to its octave), Detent (pitch dial and scrolling
  tape), Rime (amber ember spectrum, ice icicles for each suppressed resonance), Vault (a perspective tunnel of arches, one per slice of the reverb tail), Skip (a stone skipping across a log-time pond, one arc per trip round the delay's feedback loop), Carousel (voices riding a carousel, angle is LFO phase and radius is delay), Chassis (a robot torso seen as a spectrum: vent-grille slats in three bays, chest orange, body lime, shell cyan, split by draggable bulkheads, with needles for the pitch going in and out), Sunder (a spectrum split at the carrier: sum sidebands cyan, differences magenta folding off 0 Hz, the unexplained in yellow), Tines (a comb of tuning-fork tines on a log-frequency spectrum, each tine rising with the amplitude its resonator rings at, coral for the root note and violet for the chord, with a diamond to drag for pitch and decay), Specimen (four glass microscope slides each holding a captured spectrum, with the output spectrum morphing between them along a draggable rail: ink blue, acid yellow output, pink slides, cyan live input), Loom (the scrolling spectrogram as the warp of a loom with draggable heddles for the playheads: indigo, orange cloth, teal heads), Abacus (tracked partials as beads on wires, x is log frequency and size is amplitude, vermilion rings for frozen ones, teal residual tray).
- Dark, saturated-accent palettes with at most 3 or 4 colours. Colour should carry meaning
  (input vs. effect, detection vs. timing), not decorate.
- Monospace labels, small and dim. Big monospace numerals for the value being edited.
- Glow by layering: a wide low-alpha stroke under a thin bright one. Gradients via `Mesh` strips.
- Controls behave like hardware: vertical drag on knobs, shift for fine, double-click to reset,
  hover brightens. Always `begin_set_parameter` / `end_set_parameter` around a drag.
- Everything animates from real audio data through atomics written by the audio thread. No fake
  motion. Fall slowly, rise fast, so peaks stay readable.

## Technical rules

- No allocation or locking on the audio thread (the crates enable `assert_process_allocs`).
  Share meter data with the editor through `AtomicF32` arrays.
- Report latency with `context.set_latency_samples`, and delay the dry path to match.
- Support mono and stereo layouts.
- Every plugin has unit tests for its DSP on synthetic signals (null test at zero amount, the
  effect does what it claims, output stays finite). Run `cargo test --release`.
- Unique 16-byte `VST3_CLASS_ID` per plugin; display name in `bundler.toml`.
- Wherever an author, vendor or owner name is needed (`VENDOR`, the `VST3_CLASS_ID` prefix, Cargo
  metadata, README credits), use **Clowd**, not a personal name. New class IDs start with `Clowd`
  (e.g. `ClowdLoomPlayhds`).

## Always install

**Whenever you create or change a plugin, finish by installing it**, without being asked:

```sh
scripts/install.sh <plugin>     # e.g. scripts/install.sh rime
```

This builds the bundle, copies it to `~/Library/Audio/Plug-Ins/VST3` (trashing the old copy) and
checks the signature. Run the tests first, then install, then tell the user the plugin is installed
and that the DAW needs a restart to load the new binary. Say plainly if you could not open the
editor to check the UI visually.

## Repo conventions

- Version control: use `jj`, not git. Do not add Claude attribution lines to commit messages.
- Use `trash`, not `rm`.
- Add each new plugin to `bundler.toml` and the table in `README.md`.
