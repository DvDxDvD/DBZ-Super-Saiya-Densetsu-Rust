# hide_sprites_from_index ($00:86A0)

Routine of batch A (M2). 22 bytes, leaf, returns with RTL. Entry widths in every replay call: M8, X16.

## What it does

It hides the spare entries of the shadow low OAM table. Starting at the byte index held in direct page
`$55-$56` (`oam_next_index`), it writes the Y byte of each four-byte entry (`$0401` plus the index,
DBR-relative) with `$F0`, the first line below the screen. It goes on in steps of four while the index
is below `$0200`, the end of the 512-byte table. An index that is already at `$0200` writes nothing.

## Inputs and outputs

- Input: the 16-bit index at `$55` (read as a word; its wrap-around is part of the behaviour).
- Writes: the Y byte of every entry from the index up to the end of the table, one per pass.
- Output X: the index after the last pass (`$0200` for an index that is a multiple of four).
- Output A: its low byte becomes `$F0` if at least one pass ran; B and the rest of A are kept.
- Flags: N, Z and C come from the final compare with `$0200` (C is set once the loop ends); V is kept.

## Paths and costs

Costs are CPU-busy master cycles from the hit to the return.

- Index already at `$0200`: no write, 122 cycles. Not reached by the replays; from the instruction
  model, unverified.
- `n` passes, `n >= 1`: 110 + 156 n cycles. The replays show 20 to 128 passes (3,230 to 20,078).
- An index above `$0200` (or one that wraps past zero, `$FFFC` to `$FFFF`) runs on through the zero
  point: up to 129 passes, 20,234 cycles. That is the declared maximum (`cycles`).

## Callers

Each caller reaches it with `JSL` at: `$00:8130`, `$00:9EFF`, `$00:A37D`, `$00:A518`, `$00:9E65`,
`$00:9EAB`, `$00:80BF`, `$00:A34E`, `$00:9F0A`, `$00:A394`, `$00:A928`, `$04:8680`. Most of them set
the flags again at once or jump away; the flags are compared as N, Z and C as a precaution.
