# clear_slot_plus_8 ($00:8724, RTL, 14 bytes)

Clears one byte or word of low WRAM at the data-bank address `$0800` plus X + 8, and leaves
X as it was.

## Inputs and contract
- X: the slot index. The routine adds 8 to it (in the index width) and clears the byte or
  word there; the index width is kept, so both widths are exact. The accumulator width sets
  the size of the store (one byte at 8-bit, two at 16-bit). The data bank gives the bank of
  the store.

## What it does
- Adds 8 to X (wrapping at 8 bits with an 8-bit X, at 16 bits otherwise), uses that as the
  index of a store at `DBR:0800 + index`, and stores zero there.
- With a 16-bit index the address may carry into the next bank; that is kept.
- X is restored to its entry value (the routine saves and restores it), so the outcome is
  the store alone. N and Z come from the restored X, in the index width. C, V and A are
  unchanged.

## Path and cost
- One path, no loops: 260 CPU-busy master cycles from the hit to the return, for every
  index (measured in `Verify` over the reference replays).

## Callers
- Nine call sites, in banks `$01`, `$04`, `$05`, `$06` and `$1D`. The most frequent are
  `$1D:831E` and `$1D:8346`. The indexes seen are 0, 8 and 16.
