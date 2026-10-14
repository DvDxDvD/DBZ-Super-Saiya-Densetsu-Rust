# fill_buffer_4800 ($00:8996, RTL, 22 bytes)

Fills a 1 KB WRAM buffer with `$FF` bytes and clears one word of low WRAM (`$0D7D`).

## Inputs and contract
- Accumulator 8-bit on entry, index registers 16-bit. The routine works with a 16-bit
  accumulator and returns with an 8-bit one, so an 8-bit entry is what keeps the width the
  same on exit. The clear is a data-bank absolute store (`DBR:0D7D`).

## What it does
- Writes `$FFFF` to every even offset of `$7E:4800`, 512 words, so the 1024 bytes
  `$7E:4800-$7E:4BFF` all read `$FF`.
- Clears the word at `DBR:0D7D` (`$0D7D` and `$0D7E`).
- Leaves A = `$FFFF` (B = `$FF`) and X = `$FFFE`. N is set and Z clear, from that final X.
  C and V are unchanged.

## Path and cost
- One path, a fixed loop of 512 iterations: 50,346 CPU-busy master cycles from the hit to
  the return, measured in `Verify` over the reference replays.

## Callers
- `$00:982C` (the next instruction loads `$01E7`, which sets N and Z) and `$01:9E1F` (loads
  `$0C36` next). Neither reads the flags left here before setting them.
