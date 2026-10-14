# copy_four_words_to_dp ($05:8321, RTS, 21 bytes)

Copies four 16-bit words from low WRAM into four words of the direct page, the follow positions
among them.

## Inputs and contract
- Index registers 16-bit on entry; the accumulator width does not matter (no accumulator use).
  Direct page 0 and data bank 0 at every observed call.
- Reads the words of the block `$17D7` to `$17DE` through the data bank.

## What it does
- Word 1 of the block (`$17D7`) goes to the direct page at `$37`.
- Word 2 (`$17D9`) goes to `$39`.
- Word 3 (`$17DB`) goes to `$33`.
- Word 4 (`$17DD`) goes to `$35` (the variable `follow_pos` of `re/symbols.toml`).
- X ends as word 4, and N and Z are set from it. The accumulator, the index Y, C, V and the
  register widths are unchanged.

## Path and cost
- One path, no branches: 330 CPU-busy master cycles from the hit to the return, for every call
  (measured in `Verify` over the battle and wander replays: 3,652 calls).
- Every observed call fits the timing window of Rust mode.

## Callers
- `$05:831D`, which calls it and returns straight away (`RTL`). The flags it leaves are the
  caller's business, so every flag is compared.
- `$05:81DA`, which calls the routine and then calls another routine next.
