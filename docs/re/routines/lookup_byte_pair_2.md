# lookup_byte_pair_2 ($03:95D2)

The second of two byte-pair lookups of the game: the same routine as `lookup_byte_pair` ($03:9752), with
its pairs read from a different 128-byte table in bank $03 (`byte_pair_table_2`, at $03:9A28, directly
before `byte_pair_table`). Its semantics are those of `lookup_byte_pair`, written from its own
behaviour: the routine keeps X and the other index register as they were.

## Inputs and outputs
- Input: the low six bits of A select one of 64 byte pairs (the index is twice those bits). Entry
  widths: 8-bit accumulator, 16-bit index registers.
- Outputs: $04 = the first byte of the pair and $14 = that byte doubled (its bit 7 dropped); $06 =
  the second byte and $16 = that byte doubled. $05, $07, $15 and $17 are cleared. A = the doubled second
  byte, with B = 0. C = bit 7 of the second byte; N and Z come from X (its width), as X is restored
  before the return; V and X are unchanged. The routine returns with RTL.

## Path and cost
One path, 568 master cycles, for all 1,745 calls in the four replays. The replays call it with one index
only (the low bits of A are $0C in every call), so the other 63 pairs are checked by the unit tests only.
The call sites are $03:9550 (1,744 calls) and $03:964E (one call).
