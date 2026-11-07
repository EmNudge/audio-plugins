# Chassis

![Chassis](../../docs/screenshots/chassis.png)

A robot-voice rig in one plugin.

- **Pitch and formant** shift with PSOLA, using Detent's tracker. The pitch drops up to 8 semitones, and formants are set separately.
- **Gate** looks ahead by the shifter's latency, so it opens before the first word.
- **Body EQ**: a dynamic 400 Hz cut and a 100 Hz low shelf.
- **Chest** (low): octave-down sub, then an 8:1 squash.
- **Body** (mid): ring mod or 16-band vocoder, frequency shifter, comb, saturation.
- **Shell** (high): cavity delay, granular shine, metal plate.
- **Master**: glue compressor, three notches, true-peak limiter.

## Display

- The voice as a spectrum of vent-grille slats in three coloured bays.
- Lit segments are the output. A white tick on each column is the input.
- Drag the bulkheads between the bays to move the splits.
- Dashed needle: pitch in. Solid needle: pitch out.
- In Drone mode, a lime diamond marks the carrier.
- The eyes light with the output and shut when the gate closes.

Fifteen factory presets.

## Notes

```
in -> pitch + formant shift -> gate -> body EQ -> 3-way split
       chest (low):  sub-octave -> squash
       body  (mid):  ring mod / vocoder -> frequency shifter -> comb -> saturation
       shell (high): cavity delay -> granular shine -> plate
     -> sum -> glue -> notches -> push -> true-peak limiter -> mix
```

The split is two Linkwitz-Riley 4th-order crossovers that sum back flat.
Latency is the shifter's grains plus the limiter's lookahead. It is shown in the header and reported to the host.

```sh
scripts/install.sh chassis
```

See the [main README](../../README.md) for building, testing and the other plugins.
