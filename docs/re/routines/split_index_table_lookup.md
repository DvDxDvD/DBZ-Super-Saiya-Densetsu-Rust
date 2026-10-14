# split_index_table_lookup ($03:AEAC, RTL, 26 bytes)

Splits a byte into an index (its high five bits) and a sub-index (its low three bits), keeps both
in the direct page, and reads one byte of an eight-entry table at the sub-index.

## Inputs and contract
- A low: the byte. Accumulator 8-bit on entry; index registers 16-bit.
- Table: `index_table_9b9b` (`$03:9B9B`, eight bytes), read at the sub-index.

## Outputs
- `$10` = the input byte; `$12` = the index (input >> 3); `$13` = 0; `$11` = the table byte.
- A = the table byte with B cleared (16-bit A equals the table byte); X = the index.
- N clear; Z set when the index is zero; C = bit 2 of the input; V kept.

## Path and cost
One path, no loop: 344 master cycles, 15 instructions (the reference replays: 1,875 calls, all
344). The cost is the sum of the instruction timings (8 master cycles per bus access, 6 per
internal cycle), as in the other notes.

## Callers
One call site, `$03:F214` (1,875 calls in the replays).
