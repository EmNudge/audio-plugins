# What a drum is made of, and what Tympan models

A drum sound is a chain of coupled things, each with its own material and its own physics. A model
with one membrane and a noise burst gets the pitch and the decay and little else. This is the list
of the parts the acoustics literature identifies, what each contributes, and what Tympan does
about it. "Validated" means measured against recordings with `examples/fit.rs`; the rest is built
from the literature and checked by tests of the physics itself.

## The stick, and its contact with the head

- **Contact force.** A stick or beater meeting a head is a collision: a Hertzian spring (force
  goes as compression to the 3/2) with hysteresis, the Hunt-Crossley law
  `f = k x^1.5 (1 + 1.5 (1 - e) x' / v0)` (Avanzini and Rocchesso, DAFx 2001, and "Physical
  modelling of impacts: theory and experiments on contact time and spectral centroid", SMC 2004).
  Contact lasts about 3 to 8 ms for a stick on a tom or snare (Wagner, KTH 2006; Dahl, review in
  *Acoustical Science and Technology*), and the Hertz law makes it shorter for a harder or faster
  blow by `v^(-1/5)`. Its spectrum is a low-pass at about one over the contact time, which is how
  a harder blow comes out brighter.
  **In Tympan:** the mallet is a mass colliding with the head's FD grid through this law
  (`drum.rs`, `Mallet`). Hardness sets the stiffness through the Hertz contact-time formula; the
  contact time is not typed in. Tests check that contact times land in the published ranges and
  follow the velocity law, and that the mallet bounces off with less energy than it came in.
- **The stick's own wood.** A stick is a free-free beam, whose bending modes are at 1, 2.76, 5.40,
  8.93 ... times the lowest (Euler-Bernoulli). A 40 cm hickory stick's lowest is a few hundred Hz.
  Impact rings them for a few milliseconds: the tick of wood on a head.
  **In Tympan:** a six-mode beam bank driven by the contact force (`Bank<STICK_MODES>`), with the
  stick's pitch and ring time fitted per drum. Softer strikers (felt, a hand) simply ring less.

## The head

- **Membrane modes.** An ideal circular membrane's modes are the zeros of Bessel functions
  (1, 1.59, 2.14, 2.30, 2.65 ...), not harmonics. **In Tympan:** a 2D finite-difference solution
  (Bilbao, *Numerical Sound Synthesis*), which also gives the position dependence of which modes a
  hit excites. Validated against the analytic ratios in `membrane.rs`.
- **Tension modulation.** The head stiffens as it swings, so a hard hit starts sharp (Avanzini;
  Bilbao's von Karman membrane for the full version). **In Tympan:** wave speed rises with the
  energy in the skin: the kick's pitch drop.
- **Air loading.** Air on both sides adds mass and damps the lowest mode; in a kettledrum it
  shifts the modes into a near-harmonic series (Christian et al., JASA 1984; Rossing). Not modelled:
  a timpani needs a Green's-function treatment of the kettle, which is future work.

## The whack: how hard it is hit, and the energy the contact loses

Open-source and hobby drum synths (Geonkick, LMMS Kicker, ChowKick, the Sound on Sound series)
all build a kick from the same anatomy: a **transient** (the click of the beater, short and bright),
a **body** whose pitch sweeps down fast from about twice the fundamental, and a **sub**, with
saturation adding the harmonics that make it feel meaty; and a snare from two tonal head modes plus
a noise layer for the wires, with its own envelope, and a spike at the stick hit. Each of those has a
physical counterpart, and two were missing here:

- **How hard it is hit.** A kick is struck far harder than a snare. A per-drum `drive` multiplies
  the speed of the blow: the head swings further, so it bends more in pitch (tension modulation:
  the sweep) and loses more energy in the contact.
- **Impact noise from the lost energy.** A collision loses energy (the Hunt-Crossley restitution
  term), and what is lost goes into vibrations far above what the head's grid can hold: the
  broadband crack of an impact. It is computed from the contact (the damping force times the speed
  of compression), scaled to the energy a standard blow loses, low-passed by how hard the tip is
  (a hard tip is bright, a felt mallet dull), and gone in a few milliseconds. So a heavier or
  faster blow cracks more and a softer one does not. Earlier versions had no top end at the
  moment of impact at all (nothing above 1 kHz in the first 10 ms of a kick).

**The snare in particular.** A snare is mostly not its head's pitch: the recordings' energy is
nearly flat from 300 Hz to 10 kHz, because the wires dominate and also damp the head. So the snare
head decays quickly (it is loaded by the wires), the stick hits hard (a high `drive` and a hard tip,
so the contact is short and the impact noise bright), and the wires, which sound from about 1 to 7
kHz, answer the strike at once and buzz in proportion to how hard the head shakes them. When the
wires were a weak, clamped hiss and the head rang for 300 ms the result was a bongo; when the impact
was only 5 dB above the wire noise that followed it, the result was a white-noise machine. The crack
should be 15 to 20 dB above the sizzle, and the sizzle gone within a couple of hundred milliseconds.

## Two heads and the air between them

Zhao (Northern Illinois, 1990, supervised by Rossing) found the two heads of a snare or tom
interact mainly through the enclosed air rather than the shell. A sealed cavity is a spring that
pushes back on the sum of the volumes the two heads sweep, so only the axisymmetric modes (the
ones that sweep volume) couple, and each splits into a pair: a lower one with the heads moving
together, and a higher one where they breathe in and out.

**In Tympan:** the stiffness comes from the drum's length, not a knob. A head that sweeps a volume
`A u` compresses the air in a drum of volume `A L` by `u / L`, so the pressure is
`rho c^2 u / L` and a head of areal mass `sigma` is pushed with `rho c^2 / (sigma L)` per unit
displacement: a shorter drum is a stiffer spring. The second head is four analytic (0, n) modes.
Tests check that the breathing mode lands where the formula says (within 8%) and rises as the drum
gets shorter.

*One calibration:* a sealed, rigid drum would have the whole spring, and then the breathing mode of
a deep drum sits several times above the fundamental and swamps it (the first version did exactly
this: every drum's loudest partial came out an octave above its tuning). Real drums have vent holes
and leaks and flexing shells, which relieve most of the pressure, so only 2% of the spring acts
(`AIR_ACTING` in `drum.rs`). That number stands for all of that, was chosen to keep the tuned pitch
the loudest partial, and is not a measurement. The air's own axial resonances along the drum's
length are not modelled.

## The shell

A thin cylinder, defined by its diameter, length, wall thickness and material (Young's modulus,
density, Poisson's ratio, loss factor). Its modes have `n` waves around and `m` half-waves along
(free ends), with frequencies from the Donnell-Mushtari shell equations (stretching plus bending),
so a longer, thicker, wider or stiffer shell moves every mode, and nothing is a typed-in ratio.
Damping is the material's loss factor: wood (about 0.015) loses its modes quickly, steel (about
0.001) rings (Rossing, "Acoustics of Drums", *Physics Today* 1992, on the shell's role).

**In Tympan:** `shell.rs`. The shell is tied to the head's rim cell by cell with a spring, so the
head drives the shell and the shell pushes back on the head (the potential is
`1/2 k sum (u_i - w_i)^2`). Tests check the scaling laws (twice as thick is twice the frequency,
four times stiffer is twice, four times denser is half, twice as wide is under 0.4), that a longer
shell lowers the axial modes and not the ring modes, that a low-loss shell keeps ringing after a
lossy one has gone, and that a head tied to a shell dies sooner than one on a rigid rim. What is
not physics yet is how loud the shell is: it is a calibration constant set so that a wooden shell
sits about 20 dB below the head, not derived from the shell's radiation.

## Snare wires

Collisions between the wires and the resonant head (Torin, Hamilton and Bilbao, DAFx 2014;
Bilbao, Torin and Chatziioannou, *Acta Acustica*). **In Tympan:** noise whose level follows how
hard the drum is shaking; a collision model with a threshold is the obvious next step.

## Cymbals and hats

**What they are.** A cymbal is a circular plate of bronze, usually B20 (80% copper, 20% tin; B12 and
B8 are cheaper), hand-hammered or machine-formed, then lathed to thickness. It has a shallow bow, a
taper that thins it towards the edge, and on most a raised bell. Sizes by purpose: hi-hats are 13 to
15 inches, a pair with the bottom a little thicker than the top; crashes are 14 to 22 inches (16 to
18 is typical) and thin, so they speak fast and wash; rides are 20 to 22 inches, thicker and flatter,
with a large bell, for a clear ping and a wash that stays out of the way. Thicker plates sound
higher, heavier rides give more stick definition, and a bigger bell adds more overtones (maker
documentation from Paiste, Sabian and Meinl; the specific weights and thicknesses I could not find
and used typical values for, in `engine.rs`).

**What the modes are.** Rossing and Wilbur measured over 100 modes of a crash cymbal and about 300
in a 16 inch cymbal; the frequencies fit a modified Chladni law, `f` proportional to `(n + 2 s)^p`
with `n` nodal diameters and `s` nodal circles; the lowest modes are pure with no close neighbours
(Perrin et al.), while a dense clustering in the mid frequencies gives the noisy timbre, and a
linear model cannot capture a crash, which is a nonlinear effect (Bilbao; Fletcher and Rossing).

**In Tympan:** `metal.rs` builds each plate from its geometry. Diameter, thickness and alloy give
the bending-wave constant `kappa = h c_p / sqrt(12)` and so the mode frequencies
`pi kappa / (8 a^2) (n + 2 s)^2` and their density (about `area / 2 kappa` per Hz: a 14 inch hat has
roughly 900 modes per plate). Many `(n, s)` share each `n + 2 s`, so modes come in clusters: few and
tonal at the bottom, merging into noise above where the irregularities of hammering split them. The
bow raises the axisymmetric modes above a floor `c_p / (2 pi R)`; the bell is a smaller, stiffer
plate of its own whose sparse high modes are the ping, and the ride bell (MIDI 53) strikes it.
A hi-hat is two plates (the bottom 25% thicker), and pressing them together adds friction that
takes energy from every mode at the same rate.

**What I learned from the recordings** (analysed to understand the instruments, not fitted):
the closed hats are noise-like but not white (a mild line structure); the open hat and crash have
discrete persistent partials, the crash a beating pair about 44 Hz apart; the two rides are
strongly tuned, sharing partials near 328, 551, 1780, 2736 and 3643 Hz. And the decay: roughly
the same rate at every frequency, about 6/s for an open hat, 4/s for a ride and 1.5/s for a crash.
So the loss is a per-plate decay rate (air radiation, which does not depend on the mode) plus a
small frequency-dependent material term. My first version let high modes die much faster, which
removed the sizzle that real hats keep for a second.

**The wash:** a statistical energy cascade (wave turbulence; Duering, Josserand and Rica 2006): every few milliseconds a fraction of each band's energy moves to the band above with random phases, in proportion to how hard the plate is swinging, so energy is conserved, taps stay clean and a hard crash washes out. Per-plate strength: crash highest, ride lowest. It is a statistical stand-in, not a von Karman simulation.

**Not simulated (as a plate equation):** the nonlinear exchange of energy between modes that builds a crash's top end
(the von Karman equations: Bilbao 2010; Ducceschi and Touze; Nguyen and Touze on taper). A struck
plate here stiffens slightly (modes about 1% sharp, settling), which gives a slow wobble but is not
that cascade, so a crash will sound a little less alive than a real one.

## Sources

Read as abstracts and excerpts while researching this, not in full; the formulas above that are
not in a paper's abstract (the Hertz contact-time constant, the beam and shell mode ratios, the
cavity coupling) are standard results that I implemented and checked with tests.

- Avanzini and Rocchesso, "Modeling Collision Sounds: Non-Linear Contact Force", DAFx 2001, and
  "Physical Modeling of Impacts: Theory and Experiments on Contact Time and Spectral Centroid", SMC 2004.
- Bilbao, Torin and Chatziioannou, on collisions in musical instruments (mallet on membrane, wire on membrane), *Acta Acustica*.
- Torin, Hamilton and Bilbao, "An energy conserving finite difference scheme for the simulation of
  collisions in snare drums", DAFx 2014; Torin, PhD thesis, "Percussion instrument modelling in 3D", Edinburgh 2016.
- Bilbao, "Percussion Synthesis Based on Models of Nonlinear Shell Vibration", IEEE 2010; Bilbao, Torin
  and Ducceschi, a bass drum model of two membranes, a shell and the surrounding air, SMAC 2013.
- Avanzini et al., modular modal synthesis of drums with air loading, two-head coupling and tension modulation, IEEE TASLP 2010.
- Rossing, "Acoustics of Drums", *Physics Today* 45(3), 1992; Fletcher and Rossing, *The Physics of
  Musical Instruments*, chapter "Drums".
- Zhao, Northern Illinois University thesis on snare drum modes and radiation, 1990 (Rossing, advisor).
- Christian et al., "Effects of air loading on timpani membrane vibrations", JASA 1984.
- Wagner, KTH master's thesis on the contact between a drumstick and a head, 2006; Dahl, review of
  drum-stroke physics in *Acoustical Science and Technology*.
- Bao, Touze et al., tapered cymbals, JASA 2019; Ducceschi and Touze, energy-conserving modal
  synthesis of thin plates in the strongly nonlinear regime.
- Wilbur and Rossing, modes of crash and 16 inch cymbals by holography; Perrin, Swallowe, Zietlow and
  Moore, "The normal modes of cymbals" (18 inch crash); Paiste, Sabian and Meinl documentation on
  cymbal construction, alloys and sizes; the Sound on Sound article "Analysing metallic percussion".
- Raman-style loaded-membrane work on the tabla ("Acoustics of the Indian Drum", arXiv math-ph/0001030):
  a uniform membrane gives no harmonic partials, a loaded one can.

## Where it stands: simulated, and stand-in

Simulated from first principles: the head (2D wave equation, with a simplified tension
modulation), the stick's collision with it (Hunt-Crossley contact), the air spring between the
two heads (stiffness from the drum's length; the second head is four analytic modes, not a
simulated membrane), and the shell (Donnell-Mushtari modes from its geometry and material, coupled
to the rim both ways).

Stand-ins, not yet physics: how loud the shell is (a calibration constant); the stick's wood
(beam-mode ratios with a free base frequency and ring time, not yet derived from the stick's
length and material); the snare wires (a noise follower, not a collision model); the "pickup"
(70% the head's volume velocity, which is what radiates, and 30% the skin's velocity at two
points: a mix, not a radiation calculation); and the hats and cymbals (a modal plate
bank, not a plate simulation).

The grid is 25 x 25, so a drum tuned low has nothing above about 13 times its fundamental. That
is a limit of the model, and there is deliberately nothing in it that paints over it: the sound is
clean and a little dark in the top octaves.

Deliberately out of scope: the room and any other part of the environment (apply reverb
afterwards; the instrument sits in a dead space), electronic drums, and fitting to recordings.
An earlier version was fitted to recordings of acoustic drums. The fits kept finding ways to score
well that did not sound like drums (a noise field that made the head inaudible, a room standing in
for the drum), so that approach and everything it produced were removed.

## Why an open hat sounds like metal and not like noise

Compared against a recording of an open hat, the first model had a spectrum that was too flat, a
spike at the top of the band, and a decay that fell at one steady rate. Three physical changes:

- **Each mode is driven by its shape at the strike point.** A blow drives a mode in proportion to the
  mode's shape where the stick lands; with hundreds of modes those shapes are effectively random, so
  some modes are nearly silent (a nodal line close by) and a few ring hard. The hats draw that
  amplitude from a Gaussian magnitude raised to a power (`peakiness`), which gives the peaks 15 dB above
  the valleys that the recording has. Crash and ride keep their smoother, denser drive.
- **Loud modes decay fastest (`radiation`).** A mode that is driven hard radiates hard, so it loses its
  energy to the air sooner, and quietly driven modes ring on. The result drops quickly and then keeps a
  thin shimmer, as the recording does (about 8 dB per 100 ms at first, then 2 to 3).
- **No wash on the hats.** The cascade moves energy upward, and on a hat it simply piled up in the top
  band as hiss. A hat has no nonlinear sustained wash; that belongs to the crash.
