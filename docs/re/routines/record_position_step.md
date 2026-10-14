# record_position_step ($05:9B91)

Nudges one of two 16-bit positions in the direct page by a step taken from ROM tables, for the battle
side record at Y (the records at $1400 and $1440, read through the data bank). The record gives a selector
byte at +$21 and an index byte at +$34, and a flags byte at +$22. The selector, when it is six to $11,
picks a step byte `t` from a 12-entry table; the index picks a 16-bit step `w` from a 32-entry table.

## Inputs and outputs
- Inputs: Y (the record), the selector (+$21), the index (+$34), the flags byte (+$22), the step tables.
- Outputs: `t` at $20; the position at $0A (if bit 2 or bit 3 of `t` is set) or at $0C (if bit 1 is set);
  X = twice the index's low five bits; A and the flags as the last operation leaves them.

## Paths and costs (master cycles, from the hit to the return)
| Route | Condition | Cost | Replay calls |
|---|---|---|---|
| Selector high | selector at least $12 | 154 | 342 |
| Selector low | selector below 6 | 200 | 3,417 |
| No step | bits 0 to 3 of `t` clear | 554 | 459 |
| Position $0A down | bit 2 of `t` set | 602 | 452 |
| Position $0A up | bit 3 of `t` set, bit 2 clear | 640 | 1,629 |
| Position $0C up | bit 1 set, bit 0 clear, flags bit 6 clear | 788 | 70 |
| Position $0C down | bit 1 set, bit 0 clear, flags bit 6 set | 816 | 168 |
| Bit 0 alone | bit 0 set, bits 2 and 3 clear | original code | never reached in the replays |

The replays make 6,540 calls from one call site, $05:9B68, of which three were interrupted (no cost).
The cost histogram has exactly the seven measured values above. The bit-0 route is handed back to the
original code, from the entry, before anything is written. Its own cost is zero, its cost is the original's;
by the timing model it is 756 (add) or 772 (subtract), which the maximum of 816 covers.

## Caller
JSR from $05:9B68, followed by an immediate load of A, so no caller reads the flags.
