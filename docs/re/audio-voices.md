# Music and sound effects in the host's audio

Question (M7 task 4): can the host tell the music from the sound effects, to give each its own volume?

## What the host receives

The game's sound is made by the SPC700 and the S-DSP, which has eight voices. The vendored core mixes them
inside the DSP, in one step (`AudioDsp::sample`):

1. each voice's sample is scaled by that voice's left and right volume;
2. the eight are summed, clamped, and scaled by the main volume;
3. the echo unit, fed by the voices that have echo on, is added;
4. the result is one stereo pair per output sample.

The oracle hands the host only that pair (`AudioSink::push(left, right)`, fed by the core's audio output
callback). No per-voice value, no echo-free signal and no voice mask is exposed: the core has no hook or render
log for audio, unlike the PPU's render log. The DSP's voice registers are inside the emulated sound machine, which
the host does not own.

## Consequence

Telling music from effects at the host would need one of two changes to the vendored core: a per-voice (or
per-group) output next to the mix, or a voice mask applied inside the DSP. Either changes the core, so it changes
`CORE_VERSION` and invalidates every replay. Muting a voice by writing the DSP's registers instead would change the
emulated machine, which the Options never do (Classic and replays must stay unchanged).

The echo unit also mixes both kinds of sound into one tail, so even with per-voice output a clean split needs care.

## What was not checked

How the game's sound driver assigns its voices (whether effects sit on fixed voices and music on the rest, and
whether that holds across the whole game) was not researched: it only matters if a core change is approved, and it
needs the sound program the game uploads to the SPC700, which the oracle does not expose. If the core change is ever
approved, that is the next step: log the key-on writes per voice across the reference replays and see whether the
sets of voices used by songs and by effects are disjoint.

## Decision

Music and Effects collapse to one Volume row (`audio.volume`, 0 to 10, default 10) and the design's second bar is
dropped. The host scales the samples it hands to the speakers (`crates/ssd-app/src/audio.rs`); at 10 the samples
are unchanged bit for bit, at 0 they are silence. Output (stereo or mono, `audio.output`) folds the pair to its mean.
Nothing reaches the machine.
