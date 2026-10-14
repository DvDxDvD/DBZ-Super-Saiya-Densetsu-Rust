# copy_source_block ($06:EBC1)

Copies 16 words (32 bytes) from a 32-byte block in ROM bank $0C into WRAM at `copy_dest` ($7E:0280),
then sets `copy_done` ($7E:0715) to 1. The source is `copy_source_offsets` (a four-entry table of 16-bit
offsets in ROM, read at bank $06) indexed by bits 3 and 4 of the direct-page byte $4F, doubled.

## Inputs
- `copy_gate` ($7E:01A1): bit 7 set means "do nothing" (the routine returns at once).
- `copy_select` ($7E:01E7): bit 7 set means "do nothing"; bit 0 set adds $0100 to the source offset.
- Direct-page byte $4F: its low three bits, when set, pick the short path below.

## Outputs
- Only on the copy path: `copy_dest`, the direct-page pointer bytes $00-$02, X (the table index), Y ($20),
  `copy_done`, and A (the high byte of the last word read, with the low byte 1).
- Flags on the copy path: N clear, Z clear, C set, V clear.

## Paths and costs (master cycles, from the hit to the return)
| Path | Condition | Cost | Replays |
|---|---|---|---|
| Low bits | `$4F & 7` nonzero | 190 | 3,959 calls, all this cost |
| Copy | `$4F & 7` zero, selector bits 7 and 0 clear | 3490 | 582 calls, all this cost |
| Gate | gate bit 7 set | original code | never reached in the replays |
| Selector | `$4F & 7` zero, selector bit 7 or bit 0 set | original code | never reached in the replays |

The last two paths run the original code from the entry (`call_original`, nothing changed yet), so the
Rust function declares no own cost for them. Their cost is the original's, whatever it is. The maximum
over all paths, `cycles` = 3522, is the copy cost plus one increment of $01 (38 master cycles) minus one
branch that is not taken (6): the bit-0 variant. That value comes from the timing model that reproduces the
measured routes; the bit-0 variant itself is not observed in the replays.

## Callers
JSL from $00:9790 (3,664 calls), $00:9B0C (450), $00:9981 (421) and $00:8D09 (6). The callee of the first
two begins by loading $01E7 and the others load or store A straight away, so no caller reads the flags.
