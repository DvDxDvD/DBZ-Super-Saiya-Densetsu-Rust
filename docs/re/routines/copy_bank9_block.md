# copy_bank9_block ($00:8908, RTL, 25 bytes)

Copies a 64-byte block of ROM bank `$09` into low WRAM, then increments the copy counter.

## Inputs and contract
- Accumulator 8-bit on entry, index registers 16-bit (the loop counts in steps of two). Direct page
  0 and data bank 0.
- Reads the block at `$09:D200` (64 bytes, a read-only table of the ROM: `bank9_copy_block`).
- Writes `DBR:02C0` onwards (64 bytes, `copy_target_02c0`) and the byte `$0715` (`copy_done`).

## What it does
- Copies the 64 bytes as 32 words: word `n` of the block goes to word `n` of the destination.
- Leaves A as the last word read (the full 16 bits: B is its high byte), X as `$0040`.
- C is set (the last compare of the loop was equal). `$0715` goes up by one (a byte), and N and Z
  come from the new value. V is unchanged.

## Path and cost
- One path, no branches: 5,528 CPU-busy master cycles from the hit to the return, for every call
  (measured in `Verify` over 52 calls of the four replays).

## Callers
- `$01:C73E` (46 calls in the replays) and `$00:8E10` (6 calls).
