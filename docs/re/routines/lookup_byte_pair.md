# lookup_byte_pair ($03:9752, RTL, 42 bytes)

Fetches a pair of bytes from a 64-entry table in bank `$03` by the low six bits of the
accumulator, and spreads it over the direct page: each byte once, and once doubled (shifted
left by one bit, bit 7 dropped).

## Inputs and contract
- A low: the index. Only bits 0-5 matter; the high byte of A (B) is discarded.
- Accumulator 8-bit on entry, index registers 16-bit. X is saved and restored, so its value
  is kept. The data bank is not used (the table is read with a long address).

## What it does
- The table `byte_pair_table` is at `$03:9AA8`, 128 bytes: pair `i` is the two bytes at
  `$03:9AA8 + 2i` and `+1`, with `i` = index & `$3F`.
- First byte goes to `$04` and, doubled, to `$14`. Second byte goes to `$06` and, doubled,
  to `$16`. Doubled means shifted left with bit 7 dropped.
- `$05`, `$07`, `$15` and `$17` are cleared.
- A = the doubled second byte, with B = 0. C = bit 7 of the second byte. N and Z come from
  the restored X. V and X are unchanged.

## Path and cost
- One path, no loops: 568 CPU-busy master cycles from the hit to the return, for every index
  (measured in `Verify` over the reference replays).

## Callers
- `$03:9695` (almost all the calls) and `$03:9606`. Each loads a word next, which sets N and Z,
  and the first sets C before any use, so neither reads the flags left here.
