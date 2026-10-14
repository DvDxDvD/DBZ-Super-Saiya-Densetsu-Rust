# load_palette_from_pointer ($01:C885)

Routine of batch A (M2). 38 bytes, leaf with pointer access, returns with RTL. Entry widths in every
replay call: M8, X16.

## What it does

It copies one 16-colour palette (16 words, 32 bytes) from ROM data into the WRAM palette buffer at
`$0300`. The source is a 24-bit pointer: the low 16 bits are the direct-page word at `$00-$01`, and
the bank is always `$09`, which the routine stores into direct page `$02` first. The destination slot
comes from direct page `$03`: its low bit is dropped and the value is multiplied by 16, so slot
`$02` lands at `$0320`.

## Inputs and outputs

- Inputs: the pointer at `$00-$01`; the slot byte at `$03`.
- Writes: `$02` (set to `$09`), and the 32 bytes of the destination slot.
- Output A: the last word copied (16 bits; B is that word's high byte).
- Output X: the slot offset plus 32. Output Y: 32.
- Flags: N = 0, Z = 1, C = 1 from the final compare; V is kept.

## Paths and costs

There is one path: 3,556 CPU-busy master cycles on every one of the 10,805 replay calls. The loop
always runs 16 times.

## Callers

`JSR`/`JSL` sites in bank `$01`: `$01:C8E0` (8,708 replay calls), `$01:C8FD` (1,218) and `$01:C8C3`
(879).

## Note on the buffer

The routine does not bound the slot: a slot byte up to `$FE` writes up to `$12FF`. The palette
buffer's extent is therefore left implicit in `re/symbols.toml`: its slots reach past `in_battle`
(`$0364`), which is a variable of its own. In one battle replay call the slot offset was `$60`, and
the copy stored a byte at `$0364` (the value was `$FF`, the same as `in_battle` holds in battle). The
port makes the same writes, so such overlaps come out as in the original.
