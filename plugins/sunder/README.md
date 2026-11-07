# Sunder

![Sunder](../../docs/screenshots/sunder.png)

Ring modulator and frequency shifter. A ring modulator multiplies the input by a carrier, so every partial of the input comes out twice: once at the sum of its frequency and the carrier's, once at the difference, with nothing left at either of the originals. Pitch and harmony are replaced by metal.

- **Circuit.** `Diode` is Julian Parker's model of the classic four-diode ring (DAFx-11), where Drive sets how hard the carrier hits the diodes: starved diodes gate and crackle, hammered ones multiply by a square wave. `Ideal` is a clean multiplier. Output level is matched between them.
- **Carrier.** `Free` is the Freq knob, `MIDI` is the last note played times Ratio, `Track` follows the input's own pitch times Ratio (so the result stays in tune), `Sidechain` uses the carrier input. Sine, triangle, square (PolyBLEP) or saw. Follow bends the carrier with the input level, Wobble with an LFO.
- **Sideband.** Slides from both sidebands (ring modulation) to the upper alone or the lower alone: a Bode-style single-sideband frequency shifter, built on a Hilbert all-pass pair. Low carrier frequencies shift every partial by the same number of hertz.
- **Bias** leaks the input back in, which with the Ideal circuit is amplitude modulation, down to tremolo. **Spread** offsets the right channel's carrier. **Tone** low-passes the wet signal.
- The non-linear part runs at 1x to 8x oversampling, with the latency reported and the dry path delayed to match.

The display is the spectrum split by the carrier (the yellow blade, which you can drag). The input rises above the centre line; the output hangs below it. Cyan bars are sum sidebands, magenta are differences, dashed magenta is a difference that has folded back off 0 Hz, and yellow is output that sums and differences cannot account for (diode intermodulation, carrier leak). Thin outlines under the line are where the arithmetic says the sidebands should be. Below, a scope shows the input, the carrier and their product. Twenty-eight factory presets.

The MIDI carrier needs a host that sends MIDI to effects; the sidechain carrier needs one that routes a sidechain.

```sh
scripts/install.sh sunder    # build and install the VST3
```

See the [main README](../../README.md) for building, testing and the other plugins.
