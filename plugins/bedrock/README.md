# Bedrock

![Bedrock](../../docs/screenshots/bedrock.png)

Low-end exciter, the counterpart of [Aurora Air](../aurora_air).

- **Body** adds the 2nd and 3rd harmonics of the bass, so it is heard on speakers that cannot reproduce it. Polyphonic, so it works on whole mixes.
- **Sub** adds a tracked octave below. It fades in only while one steady note is found, and steps aside under chords.
- **Floor** keeps the sub out of frequencies a system cannot play.
- No latency. The dry signal passes straight through.

## Display

- Aurora Air's night sky, upside down.
- The dry spectrum is a mountain ridge above the ground line.
- What is added hangs below as magma: violet for the octave, vermilion for the harmonics.
- While the sub is locked, a bridge arcs from the tracked note to its octave.
- Drag the left half down for Sub, the right half for Body, and the dashed line for Floor.

Seventeen factory presets.

## Notes

Body divides the 30 to 130 Hz band by its own envelope and feeds it through Chebyshev polynomials,
so a note gains its 2nd and 3rd harmonics in the same proportion at any level.
Sub is a Schmitt trigger and a flip-flop. It locks once three cycles in a row agree to within 7%.

```sh
scripts/install.sh bedrock
```

See the [main README](../../README.md) for building, testing and the other plugins.
