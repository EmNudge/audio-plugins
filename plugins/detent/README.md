# Detent

![Detent](../../docs/screenshots/detent.png)

Automatic pitch correction.

- MPM pitch tracking.
- Scale or MIDI targets, with Learn Key.
- Humanize and formant control.
- A low-latency Live mode.

## Display

- A pitch dial and a scrolling tape show what it is doing.

## Notes

Detent has the most tests. `DETENT_FILES=name cargo test -p detent --release offline -- --ignored --nocapture`
runs it over raw mono f32 recordings in `/tmp/detent`. See `src/tests.rs`.

```sh
scripts/install.sh detent
```

See the [main README](../../README.md) for building, testing and the other plugins.
