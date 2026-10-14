# fill_bank7f_pattern ($04:889E, RTL, 40 bytes)

Fills the upper 32 KB of bank `$7F` (WRAM) with a two-part pattern, and leaves a long pointer
`$7F:0000` in the direct page.

## Inputs and contract
- No inputs: the routine's data is its own constants. It needs a 16-bit accumulator and index
  registers on entry (its first load is a 16-bit immediate, its index starts from zero).
- Direct page 0.

## What it does
1. Writes the pointer words: `$73-$74` = `$0000`, `$75` = `$7F`.
2. First pass: for every even offset from `$0000` to `$7F7E`, the byte `$FF` at `$7F:offset`. The odd
   bytes are not touched.
3. Second pass (16-bit stores): for every even offset from `$7F80` to `$7FFE`, the word `$00FF`
   (low byte `$FF` at the even offset, `$00` at the odd one).

## Outputs
- A = `$00FF` (16-bit), Y = `$8000`, N clear, Z and C set (the last compare with `$8000`).
- V and X are kept; the data bank and the direct-page bytes other than `$73-$75` are unchanged.

## Path and cost
One path, both loops always run in full: 1,999,596 master cycles (about 5.6 frames), measured in
`Verify`. The budget is 7 frames. In the reference calls no NMI, IRQ or HDMA event falls inside its
window, so all four calls fit the timing window of `Rust` mode.

## Callers
One call site, `$04:876E` (4 calls in the replays).
