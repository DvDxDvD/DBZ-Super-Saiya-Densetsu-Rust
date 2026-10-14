# lookup_triple_index ($01:8A0E, RTL, 20 bytes)

Looks up a table byte of bank `$01` at the index three times a byte of low WRAM (modulo 256), and
stores the byte in `$01E3`.

## Inputs and contract
- Accumulator 8-bit and index registers 16-bit on entry. Direct page 0 and data bank 0.
- Reads the byte at `$0D59` (`code_byte_0d59`) and the table byte at `$01:83BD` plus the index.
  The table is part of `bank1_index_table` (the table read by `update_travel_and_follow` begins at
  `$83A0`, the two overlap).
- Writes the byte at `$01E3` (`table_result_01e3`).

## What it does
- The index is `2v + v` in 8 bits, where `v` is the byte at `$0D59`; the carry and the overflow of
  that sum are kept.
- The table byte at the index goes to `$01E3`. X becomes the index. A is that byte with B clear.
- N and Z come from the table byte; C and V come from the sum.

## Path and cost
- One path, no branches: 258 CPU-busy master cycles from the hit to the return, for every call
  (measured in `Verify` over 4,413 calls of the four replays).

## Callers
- `$01:8176` (4,408 calls in the replays), `$01:8A4A` (4 calls) and `$01:8FC8` (1 call).
