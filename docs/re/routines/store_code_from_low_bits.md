# store_code_from_low_bits ($01:8401, RTL, 45 bytes)

Picks a source byte by one bit of the gate byte `$01BC`, finds the lowest set bit of that
source's low nibble, and stores a small code for it in `$0D59`.

## Inputs and contract
- Gate `$01BC` (`gate_flag_01bc`): bit 0 selects the source.
- Source: `$0048` (`source_byte_0048`) when bit 0 of the gate is set, else `$0044`
  (`source_byte_0044`). Both are read through the data bank (`$00` in the replays).
- Accumulator 8-bit, index registers 16-bit.

## Mapping
The low nibble's lowest set bit gives the code: bit 0 gives 3, bit 1 gives 2, bit 2 gives 0 and
bit 3 gives 1. The code is stored to `$0D59` (`code_byte_0d59`) and returned in A low.

If the low nibble is empty nothing is stored, and A low is the high nibble of the source.

## Outputs
- With a code: A low = code, `$0D59` = code, N clear, Z set for code 0, C set. B, V, X and Y kept.
- Without a code: A low = high nibble, N clear, Z set when it is zero, C clear.

## Paths and costs (master cycles, from the hit to the return)
| Gate bit | Shift path | Cost | Instructions | Replays |
|---|---|---|---|---|
| set | code 3 / 2 / 0 / 1 / none | 262 / 298 / 334 / 348 / 306 | 11 / 13 / 15 / 16 / 14 | 3 / 2 / 1 / 2 / 1,781 |
| clear | code 3 / 2 / 0 / 1 / none | 246 / 282 / 318 / 332 / 290 | 10 / 12 / 14 / 15 / 13 | 16 / 24 / 32 / 40 / 1,763 |

Each path is the gate test (about 102 or 118 master cycles) followed by the shifts of its branch
chain. Every path is observed in the replays (3,664 calls) and matches the timing model exactly.
`cycles` is the largest path, 348.

## Callers
One call site, `$00:976E` (3,664 calls in the replays).
