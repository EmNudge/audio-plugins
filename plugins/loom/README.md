# Loom

Spectral playheads, an open-source take on the idea behind GRM Tools Fusion. The input is analysed by a 4x-overlap STFT (2048 points) into a few seconds of spectrogram history. Up to eight playheads each read that history at their own delay (0 to 4 s), with their own pitch offset and glide (a phase vocoder, so shifted and delayed heads resynthesise without clicks), their own band-pass window in frequency, level and pan. The heads are summed, overlap-added, mixed with the delayed dry signal and optionally fed back into the history (bounded by a soft clipper). Latency is 2048 samples and is reported.

The display is a loom. The orange cloth is the spectrogram, a warp of vertical threads that scrolls left, and every head is a heddle you drag across it: the hollow slat marks the band it reads, the teal slat is where it writes. Drag the teal slat for delay (across) and pitch offset (up and down, in semitones, hold shift for fine), drag the hollow slat to move the band, drag either end of a slat for the band width. Threads under a head light up teal, and the finished cloth, the live output spectrum, hangs off the right edge.

```sh
scripts/install.sh loom    # build and install the VST3
```

See the [main README](../../README.md) for building, testing and the other plugins.
