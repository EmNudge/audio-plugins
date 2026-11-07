# Abacus

Sine resynthesiser, an open answer to Grinder. The input is analysed with a 4096-point STFT; the N strongest spectral peaks of each frame (parabolic interpolation, frequency refined from the phase difference between frames) are tracked from frame to frame so every partial keeps an identity, and rebuilt on a bank of phase-continuous sine oscillators. Freeze Freq holds each partial's pitch, Freeze Amp holds its level (the grinder drone), Resolution trades peak separation for a smeared, degraded character, Glide slews the pitches, Pitch and Stretch transpose and spread the partial set, Noise adds back the part of the spectrum the sines do not explain, and Spread detunes each partial at random. Latency is 2560 samples, reported to the host, and the dry path is delayed to match.

The display is an abacus: each partial is a bead on a wire, left to right by log frequency, strongest on the top wire, with its size and brightness set by its level and a trail behind it. Frozen beads are ringed in vermilion. Drag the board sideways to transpose, drag the cut line to set the number of partials.

```sh
scripts/install.sh abacus    # build and install the VST3
```

See the [main README](../../README.md) for building, testing and the other plugins.
