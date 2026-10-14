# fill_tilemap_quad ($01:C6CB, RTL, 74 bytes)

Stores a 4 x 4 block of tile words in the tile-map buffer, each word a counter plus a pattern, in
the four rows the routine places with its own steps.

## Inputs and contract
- Accumulator 8-bit and index registers 16-bit on entry. The routine switches the accumulator to
  16 bits and back to 8 bits before it returns. Direct page 0 and data bank 0.
- Reads, as words: `$20` (the row base), `$22` (the value counter) and `$02` (the pattern).
- Writes: the sixteen tile words at `$7E:4000` plus their offsets; `$00`, `$20` and `$22` as the
  routine leaves them; `$0D74` gets `$80`.

## What it does
- The loop counter at `$00` starts at three. A row is four words; the word stored is the value
  counter ORed with the pattern, and the counter goes up by one per word.
- After the first row and after the third, the row base steps by `$80`. After the second row the
  base steps back by `$40` and the value counter gains `$8` more. The loop ends after the fourth
  row, before any step.
- At the end: `$00` is `$FFFF`; the row base has moved by `$C0`; the value counter by `$18`. A is
  `$0080` (B cleared), Y is `$FFFF`, X is the last row's base plus eight. N is set, Z and C are
  clear, and V is the one the last addition gave.

## Path and cost
- One path, the same instructions for every call: 5,060 CPU-busy master cycles from the hit to the
  return (measured in `Verify` over 172 calls of the battle, landing, menus and wander replays).

## Callers
- One call site, `$01:C791` (172 calls in the replays).
