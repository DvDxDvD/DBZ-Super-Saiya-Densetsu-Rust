# fill_word_block ($01:B647)

Writes the word $3D40 as 320 word stores in WRAM bank $7E, one every second address from $7E:37FE down to
$7E:3580, which covers the block `fill_block` ($7E:3580 to $7E:37FF). Each store writes the low byte
first, then the high byte.

## Inputs and outputs
- No inputs; the routine needs an 8-bit accumulator and a 16-bit index register at entry (its first
  instructions switch the accumulator to 16 bits and load a 16-bit index).
- Outputs: A = $3D40 (B is its high byte), X = $057E, N set, Z clear, C clear. V, Y and the direct page
  are unchanged; the exit mode is 8-bit accumulator, as at entry.

## Path and cost
One path, 39170 master cycles, for all 16 calls in the replays. The call site is $01:B63F.
