# fill_tilemap_4000 ($01:B6AC)

Routine of batch A (M2). 19 bytes, leaf, returns with RTL. Entry widths in every replay call: M8, X16.

## What it does

It fills the 2 KB tile-map buffer at `$7E:4000` (1,024 words) with the word `$00FF`: tile `$FF`,
palette 0. The loop runs from the top word down to the bottom one, two bytes per pass, and always makes
1,024 passes.

## Inputs and outputs

- Inputs: none beyond the entry widths; the routine must be entered with M = 8, because it ends by
  setting M back to 8.
- Writes: the 2 KB buffer at `$7E:4000` to `$7E:47FF`.
- Output A: `$00FF` as a 16-bit value (the routine loads it at M = 16).
- Output X: `$FFFE`, the index below the last word. Flags: N = 1 and Z = 0 from that last decrement;
  C and V are kept.

## Paths and costs

There is one path: 100,482 CPU-busy master cycles, the same on all 125 replay calls. That is about 28 %
of a frame, so in Rust mode the routine runs in Rust only when no interrupt can fall within those
cycles; otherwise the original code runs.

## Callers

`JSL` sites: `$01:C71A` (46 replay calls), `$00:98BE` (43), `$01:AABD` (25), `$00:8BE0` (6), `$00:8FD5`
(4) and `$01:A7AC` (1).
