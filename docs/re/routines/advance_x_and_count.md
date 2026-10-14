# advance_x_and_count ($01:D00B, RTL, 8 bytes)

Advances X by one and bumps two byte counters of low WRAM by one each.

## Inputs and contract
- Accumulator 8-bit and index registers 16-bit on entry (the counters are byte increments).
  Direct page 0 and data bank 0.
- Reads and writes the bytes `$071D` and `$071F` (through the data bank).

## What it does
- X goes up by one, in the index width.
- The byte `$071D` goes up by one, then the byte `$071F` by one.
- N and Z are set from the value the second counter took. C, V, A and Y are unchanged.

## Path and cost
- One path, no branches: 150 CPU-busy master cycles from the hit to the return, for every call
  (measured in `Verify` over 2,443 calls of the four replays).

## Callers
- Seven call sites: `$01:CD1E` (998 calls), `$01:CD4B` (315), `$01:CD9B` (314), `$01:CD66` (274),
  `$01:CD81` (244), `$01:CFBE` (242) and `$01:CD8A` (56).
