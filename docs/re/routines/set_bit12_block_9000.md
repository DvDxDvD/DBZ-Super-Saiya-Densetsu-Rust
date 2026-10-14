# set_bit12_block_9000 ($00:BC3E, RTS, 23 bytes)

Sets bit 12 in each of the 1,024 words of the WRAM block at `$7E:9000` to `$7E:97FE`, walking down
from the last word.

## Inputs and contract
- No data inputs. The index register is 16-bit on entry (the routine's first index load is a 16-bit
  immediate). The routine switches to a 16-bit accumulator for its loop and back to an 8-bit one at
  the end, so the accumulator width must be 8-bit on entry (the port declares it).

## Outputs
- A = the word at `$7E:9000` with bit 12 set (16-bit, the last word of the walk), X = `$FFFE`.
- N set and Z clear (from the last decrement); C and V kept; the accumulator is 8-bit at exit.

## Path and cost
One path, the loop always runs in full: 174,184 master cycles, measured in `Verify` (8 of 8
reference calls). The timing window fits all eight calls.

## Callers
One call site, `$00:A1B9` (8 calls in the replays).
