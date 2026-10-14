# split_03_nibbles ($06:EAA8)

Splits the byte at $03 into its two nibbles and stores each one as a map-cell digit with its top bit set:

- $0D6D = $80 OR the high nibble of $03;
- $0D6C = $80 OR the low nibble of $03.

Both bytes are written from zero, so the previous contents of $0D6C and $0D6D do not matter.

## Inputs and outputs
- Input: $03 (direct page). Entry widths: 8-bit accumulator, 16-bit index registers.
- Outputs: $0D6C and $0D6D as above. A = $80 OR the low nibble of $03, so N is set and Z is clear. C is
  clear (the last shift moved out a zero bit). V, X, Y and the direct page are unchanged. The routine
  returns with RTL.

## Path and cost
One path with no branches, 556 master cycles, for all 1,002 calls in the four replays. It has 11 call
sites, the most used being $01:BFBE (388 calls) and $01:AA76 and $01:AAA0 (110 each).
