# find_positive_slot ($00:8711, RTL, 19 bytes)

Looks along the slot block at `$0800` (`slot_block_0800`, in the data bank) for the first slot byte
whose top bit is clear. The slots are eight bytes apart; the scan stops at the first positive one.

## Inputs and outputs
- Input: the slot block only. Entry: accumulator 8-bit, index registers 16-bit (all 11,333 replayed
  calls).
- Output: the low byte of A is the slot byte found, B is unchanged. X is the offset of that slot:
  0, 8 or 16 for the first, second or third slot (eight times the number of negative slots passed).
- N is clear. Z is set when the slot byte is zero. C and V are unchanged. The routine writes nothing.

## Paths and costs
- The scan passes `n` slots whose top bit is set, then stops at a positive one.
- The replays take `n` = 0, 1 and 2 only: 128 CPU-busy master cycles (6,804 calls), 316 (4,271) and
  504 (258). The cost is `128 + 188 n`; the instructions are `4 + 11 n`.
- A scan that passes three negative slots is not on any replayed path. It runs the original code
  (`call_original` from the entry, own cost 0), and the bound function declines it.
- The bound function gives each call its exact cost from the slot block at entry (the scan only
  reads). Of the calls, 9,801 of 11,333 fit the timing window with that bound (86.5%).

## Callers
- JSL from `$1D:8315` (3,539 calls), `$1D:8342` (3,189), `$01:B681` (2,648), `$01:CA76` (535),
  `$05:BEF9` (477), `$06:EFD3` (437), `$06:EF11` (336), `$01:B6BF` (129), `$05:F57D` (39) and
  `$04:8938` (4).
