# blend_palette_slots ($05:C116, RTS, 166 bytes)

Updates a row of palette slots (sixteen words at `$0200` plus the index register X, in the data
bank) from the index in A (its low seven bits) and the flags of the byte at `$4F`. The routine's
code is followed by a table of blend words that it reads at the index times four.

## Inputs and contract
- A low: the index byte (only its low seven bits matter); its high byte is kept.
- `$4F`: bit 2 selects blending (when set, with the low bits clear), the low bits otherwise.
- X: the slot row (`$0120` or `$0140` in the reference replays). Accumulator 8-bit, index 16-bit.
- Table `blend_table_c1b8` (`$05:C1B8`): four bytes per index; the first word is used for a slot that
  is not zero, read at the index times four.

## Paths
1. Index zero, or bit 2 of `$4F` clear: copy 32 bytes from `$15E1` plus X to `$0200` plus X.
   Cost 4,968 when the index is zero, 5,018 otherwise.
2. Index non-zero, bit 2 set, low bits of `$4F` set: nothing but the bookkeeping below. Cost 222.
3. Index non-zero, bit 2 set, low bits clear: blend the sixteen slots. Each slot, when it is not zero,
   takes the table word of the index; the three channel sums (5-bit red, 5-bit green at bit 5,
   5-bit blue at bit 10) are added, and a channel whose sum reaches its limit is clamped by the
   original. The port only blends when no slot is zero and no channel is clamped; its cost is
   19,308 master cycles, measured in `Verify` (43 blend calls in the replays, all this cost).

Every path increments `copy_done` (`$7E:0715`, a byte).

## Outputs
- Copy: A low = the last byte copied (B kept); X = X + 32; Y = `$0020`; C set; V kept; N and Z from the
  incremented flag.
- Nothing: `$00` = index; A low = the low bits of `$4F`; N clear, Z clear; C and V kept.
- Blend: `$00-$01` = index times four; `$02-$03` = `$FFFF`; `$04-$05` = the table word; `$06-$07` = that
  word's blue part; `$10-$11` = the red and green parts of the last slot; A = the last slot; X = index
  times four; Y = X + 32; C and V clear; N and Z from the incremented flag.

## Port
The bound returns the cost of the path the port runs and `u64::MAX` for every other call (an index
register other than the two reference values, a blend that would clamp, or a zero slot), so only the
reference situations run in Rust; the others run the original code from the entry.

## Callers
Two call sites: `$05:C112` (345 calls) and `$05:C0FE` (7 calls) in the replays.
