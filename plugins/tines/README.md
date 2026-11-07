# Tines

![Tines](../../docs/screenshots/tines.png)

Tuned resonator. Whatever you feed it excites a bank of modal resonators (decaying cosines, one per mode), so a voice, a drum loop or a pad starts to ring like the body of an instrument. Short decays turn it into a pitched comb filter and robot speech; long ones turn it into a bell, a drone or a room tuned to a chord.

- **Model.** `String` is the harmonic series (with Stiffness stretching the upper partials sharp, as in a piano), `Pipe` odd harmonics only, `Bar` a free-free bar (marimba, vibraphone), `Bell` a church bell's hum, prime, tierce, quint and nominal, `Drum` a membrane's Bessel zeros. Up to 16 partials per note.
- **Pitch.** `Note` is the Note knob (drag the diamond on the display), `MIDI` plays the keys you hold (up to four, and they latch on release so the resonance does not stop when your hand does), `Track` follows the pitch of the input. Octave and Fine transpose all three. Chord adds up to four notes: octave, power, major, minor, sus4, min7, maj7.
- **Decay and Damping.** Decay is the 60 dB time of the root. Damping makes the upper modes die faster (loss grows with the square of frequency), so a struck bank loses its brightness first. Tilt sets how bright the modes are to begin with.
- **Detune** pulls the left and right banks apart by up to 40 cents for a wide, chorused ring. **Drift** wanders each note slowly out of tune.
- **Feedback** puts the bank's own soft-clipped output back in front of it: lengthening the decay at first, then self-oscillating, so the input plays a singing resonator. **Pluck** injects a burst of noise at every onset of the input, so even a bass or a sine rings the upper modes. **Freeze** closes the input and stretches the decay to forty seconds, holding whatever is ringing.
- **Tone** low-passes the wet signal, **Level** trims it, and a soft limiter keeps long decays and feedback from running away. No latency.

The display is a comb of tuning-fork tines on a log-frequency spectrum. The faint white skyline is the input; every resonator is a tine that rises with the amplitude it is really ringing at, coral for the root note and violet for the rest of the chord, and the coral outline is the output. The diamond is the root: drag it sideways to retune (snapping to semitones, shift to slide) and up and down to set the decay. Twenty-three factory presets, including three starting points for a deep, metallic, Dr. Hayden voice (`Hayden`, `Hayden Tracking`, `Hayden Cathedral`).

The MIDI source needs a host that sends MIDI to effects.

```sh
scripts/install.sh tines    # build and install the VST3
```

See the [main README](../../README.md) for building, testing and the other plugins.
