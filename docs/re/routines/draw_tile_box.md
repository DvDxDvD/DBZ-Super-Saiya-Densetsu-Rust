# draw_tile_box ($01:B55A, RTL, 196 bytes)

Draws a rectangular box of tile words into the tile buffer at `$7E:3000`: a border of corner, edge
and fill tiles, the buffer position being the word in direct page `$02`. Each row of the box is
32 tiles (64 bytes) after the one above.

## Inputs
- `$00`: width in cells (8-bit). `$01`: height in rows (8-bit).
- `$02`-`$03`: the buffer position of the top-left cell (16-bit).
- `$07B4` (`box_interior_flag`): when nonzero, the interior cells of the middle rows are not written.
- Entry: accumulator 8-bit, index registers 16-bit (all 633 replayed calls).

## What it writes
- Top row: the top-left corner tile, the top edge tile in every interior cell, the top-right corner.
- Each middle row (height minus 2 of them): the left edge tile, the interior fill tile in every
  interior cell (unless the flag is set), the right edge tile.
- Bottom row: the bottom-left corner, the bottom edge in every interior cell, the bottom-right corner.
- The top and bottom loops and the middle rows are all the box has; a box of 3 by 3 is nine words.

## Outputs
- `$02`-`$03`: the start of the bottom row (the top row's start plus 64 for each row after it).
- `$10`: width minus 1. `$11`: height minus 1. `$07B4`: cleared.
- A = `$283D` (16-bit, so B = `$28` and the low byte is `$3D`). X: the bottom row's last cell.
- N and Z clear, C set. V is the signed overflow of the last row advance (the position of the last
  middle row, or of the top row when the box has two rows, plus `$40`).
- Y, the direct page apart from the bytes above, and the data bank are unchanged.

## Paths and costs
- The paths the replays take: width and height both two or more, and width three or more unless the
  height is two. The interior flag is zero.
- Cost in CPU-busy master cycles, from the hit to the return:
  `1200 + 564 (W-2) + 588 (H-2) + 314 (W-2)(H-2)`, where W and H are the width and height.
  The top and bottom loops together cost 564 per interior column (282 each), each middle row 588
  besides its interior cells, and each interior cell 314. The measured extremes are 11,912 (10 by 4)
  and 217,160 (28 by 25); 28 by 8 costs 68,376.
- Instructions (opcode fetches): `46 + 24 (W-2) + (H-2)(23 + 13 (W-2))`.
- With the interior flag set, each interior cell costs 226 instead of 314 and nothing is written.
  No replayed call has the flag set, so that path runs the original code (`call_original`, own cost 0).
- The sizes outside the exact domain (a width or height below two, or a width of two with three or
  more rows, where the 8-bit counter of the middle rows wraps) also run the original code.
- The bound function gives each call its cost from the entry state, and declines the paths that run
  the original (the window never opens for them), so Rust mode runs this routine only on the
  replayed paths.

## Callers
- JSL from `$01:B74C` (506 calls) and `$01:B77D` (127 calls).

## Other effects
- None. The routine touches only the tile buffer and the direct-page bytes above; it has no
  hardware register access.
