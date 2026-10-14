# write_slot_record ($01:CAAB, RTL, 32 bytes)

Writes an eight-byte record into the slot block (`slot_block_0800`) at the slot index in X.

## Inputs and contract
- Accumulator 8-bit and index registers 16-bit on entry. Direct page 0 and data bank 0.
- Reads: the word at `$00` (direct page) and the word at `$0D77` (`word_0d77`, through the data
  bank).
- Writes the eight bytes at the slot: the slot block's `$0800` plus X (through the data bank, a
  carry going into the bank byte).

## What it does
- Byte 0 of the record is `$80`.
- Bytes 1 and 2 are the word at `$00`.
- Bytes 3 and 4 are the word at `$0D77`.
- Byte 5 is `$7E`.
- Bytes 6 and 7 are `$0100` (low byte first: `$00`, then `$01`).
- A ends as `$017E` (B is `$01`, the word stored at byte 6). N and Z are clear (from `$7E`). C, V,
  X and Y are unchanged.

## Path and cost
- One path, no branches: 430 CPU-busy master cycles from the hit to the return for every call
  (measured in `Verify` over 1,070 calls of the battle, landing, menus and wander replays).

## Callers
- Two `JSR` instructions, at `$01:CA72` and `$01:CA90`, 535 calls each in the replays.
