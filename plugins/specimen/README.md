# Specimen

![Specimen](../../docs/screenshots/specimen.png)

Spectral freeze and morph, an open-source answer to GRM Tools Evolution. A 4x-overlap STFT (2048 points, 4096 above 70 kHz) with four snapshot slots: capture the input's magnitude spectrum (averaged over a few frames) into a slide, then sweep **Position** along the rack to morph between slides (log-magnitude interpolation with a **Curve**, plus **Scale** and **Shift** on the frequency axis). **Cross** mode imposes the snapshot's spectral envelope on the live input, keeping its phase and fine structure and its loudness; **Resynth** mode plays the frozen spectrum back with phase-vocoder phase advance so a held frame sustains without buzzing. **Freeze** holds the live frame. **Evolve** moves the position by a sine LFO or a random walk, **Glide** smooths every move, and latency (one FFT length) is reported with the dry path delayed to match.

The display is a rack of four glass microscope slides, each showing its captured spectrum as a pink silhouette. Click a slide to capture into it (right-click to clear), drag along the rack to move the objective lens, and watch the acid-yellow output shape morph between the slides over the cyan live input. Twenty factory presets.

```sh
scripts/install.sh specimen    # build and install the VST3
```

Captured slides live in memory only: they are not saved with the project.

See the [main README](../../README.md) for building, testing and the other plugins.
