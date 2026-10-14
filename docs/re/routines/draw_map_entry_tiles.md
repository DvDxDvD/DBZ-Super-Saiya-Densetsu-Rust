# draw_map_entry_tiles ($01:B90F)

Writes two tile bytes for the map entry the index $0D66 names, into the tile buffer at $7E:3000 plus an
offset that comes from a record table. The entry's value is in $0D62.

What the routine does, in its own steps:

1. It takes the byte at `$0D66` (the index) and reads a record from bank $02: three bytes at `3 * index`
   (the index times 3, low byte only), a value byte and a word after it.
2. If the value byte's bit 7 is clear, it is stored to `$0D64`. A zero value ends the routine there.
3. If bit 7 is set, the index chooses a special route: index 2 or $14 stores the byte at $01BB, index $0B
   stores the byte at $01B3, index $2D stores the byte at $0C34, and index $26 compares $01BB with a byte of
   a table in bank $01 (at `$01:B9E3 + $0D63`) and stores the low two bits of $01BB. The other indexes
   (and $28, $13) run other code that the reference replays never reach.
4. Otherwise (the main part, reached with a non-zero stored value): `$10` = `$0D62`; the index record's word
   is added to a word read from bank $01 at twice `$0D62` (from `$01:BF61`, or from `$01:BF79` when bit 7 of
   `$0D6A` is set), and the sum is the offset. The byte at `$00` goes to `$7E:3000 + offset`, and `$28` (when
   `$00` is 1) or `$29` (otherwise) goes to `$7E:3001 + offset`.

## Inputs and outputs
- Inputs: `$0D66` (index), `$0D62` (value), `$0D6A` (flag), `$00` (direct page), `$0D63`, `$01BB`, `$01B3`,
  `$0C34` and the three tables. Entry widths: 8-bit accumulator, 16-bit index registers.
- Outputs on the main part: `$0D64` = the stored value, `$10` = `$0D62`, `$04-$05` = the table word, the two
  tile bytes, A low = `$28` or `$29` (B = 0), X = the offset, N and Z clear, C = (`$00` is not 0), V = the
  signed overflow of the sum. The routine returns with RTL.

## Paths and costs
The costs of the replayed calls (1,373 calls in the four replays) fall into twelve values, which the routes
add up to:
- 328: zero record value (81 calls). N clear, Z set; C and V from the add of the index and twice the index.
- 1014, 1030, 1046: the main part from a record without bit 7 (51 calls at 1014; 1030 for bit 7 of the flag
  set with `$00` = 1 or for bit 7 clear with `$00` other than 1; 1046 for bit 7 set with `$00` other than 1).
- 1144 and 1160: index 2 by `$01BB` (2 and 22 calls); 1208 and 1224: index $14 by `$01BB` (1 and 75 calls).
- 1194: index $0B by `$01B3` (51 calls). 1354: index $2D by `$0C34` (2 calls).
- 1674 and 1690: index $26 with `$01BB` below both $0F (the table byte) and 4 (4 and 36 calls).

The Rust version follows each route's steps and counts its cost and instructions (the instruction counts
range from 14 to 43). Every replayed call was checked in Verify against the original (cost, instructions,
registers, memory).

## Routes left to the original code
These are not seen in the replays and run the original code (own cost 0): index $33 (which returns at once);
index $28 and index $13; any other index whose record value has bit 7 set; a zero `$01BB`, `$01B3` or `$0C34`
after the special indexes; and index $26 with `$01BB` at least the table byte or at least 4.

## Callers
- $01:B861 (909 calls), $01:BA05 (395), $01:B807 (61) and $01:AD40 (8).
