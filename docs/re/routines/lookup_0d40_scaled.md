# lookup_0d40_scaled ($01:C943, RTS, 23 bytes)

Reads one byte from a low-WRAM table by a quarter of X, doubles it, and returns the result
in both A and Y, with the top bits of X above bit 9 in the high byte.

## Inputs and contract
- Accumulator 8-bit on entry, index registers 16-bit (the index arithmetic is 16-bit).
  Direct page 0.
- X: the index source. The data bank is used for the table read (`DBR:0D40 + X/4`).

## What it does
- Writes `$64`, `$A2` and `$02` to `$00`, `$01` and `$02` (a constant pointer pair).
- Computes Q = X shifted right by 2 (16 bits). Reads the byte `DBR:0D40 + Q`, call it T.
- The result R = `(Q & $FF00) | ((T << 1) & $FF)`: the low byte is T doubled with bit 7
  dropped, the high byte is bits 8 to 15 of Q (which are bits 10 to 15 of X).
- A = R (full 16 bits, so B is the high byte of Q) and Y = R.
- N and Z from R. C = bit 7 of T (the bit shifted out of the doubled byte). V and X are
  unchanged.

## Path and cost
- One path, no loops: 304 CPU-busy master cycles from the hit to the return, for every
  index (measured in `Verify` over the reference replays).

## Callers
- One call site, `$01:C91F`. Its next instructions read through the long pointer in `$00`
  with Y as the index, so Y (the result) is what the caller uses.
