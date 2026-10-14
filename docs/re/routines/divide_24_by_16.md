# divide_24_by_16 ($01:C129)

Divides the 24-bit value in $00-$02 by the 16-bit value in $03-$04, by the restoring method, one bit per step
over 24 steps. The quotient replaces the dividend in $00-$02, the remainder is left in $05-$07, and $08-$09
keep the low and middle bytes of the difference that the last step computed.

Each step shifts the dividend's top bit into the remainder. If the remainder is at least the divisor, the
divisor is subtracted and the quotient's low bit is set; otherwise the remainder is kept as it is. A zero
divisor never fails that test, so every step subtracts and the quotient is all ones.

## Inputs and outputs
- Inputs: the dividend at $00-$02 (low byte first) and the divisor at $03-$04. Direct page 0; entry widths
  8-bit accumulator, 16-bit index registers.
- Outputs: $00-$02 = the quotient; $05-$07 = the remainder; $08 and $09 = the low and middle bytes of the
  last step's difference (written in every step). A low byte = the low difference byte when the last step
  subtracted, else the high difference byte; B is unchanged. Y = 0, N clear, Z set. C = whether the last
  step subtracted, V = the signed overflow of the last step's high-byte subtraction. X and $03-$04 are
  unchanged. The routine returns with RTL.

## Paths and costs
The cost depends on the number of steps that subtract, n (the bits set in the quotient):
- 11,786 + 152 n master cycles, from n = 0 (no subtraction) to n = 24 (15,434).
- 438 + 6 n instructions (the set-up, 18 instructions per step that does not subtract, 6 more per step that
  does, and the return).

The Rust version computes n from the entry state and declares that cost and instruction count per call (a
bound function gives the same cost before the call). The static maximum (`cycles`) is 15,434. The 38 calls in
the replays (battle 12, landing 6, wander 20) were checked in Verify; their costs fit the formula exactly.

## Callers
- $01:AFF7 (16 calls), $01:B01A (16 calls), $03:8021 (3 calls) and $03:8038 (3 calls).
