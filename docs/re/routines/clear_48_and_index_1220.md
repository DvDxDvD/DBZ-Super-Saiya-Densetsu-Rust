# clear_48_and_index_1220 ($04:924A)

Clears bit 7 of the direct-page byte $48, then sets the index register X to twice the byte at $1220,
keeping only the low eight bits of that doubling. A is left holding the same value, with B cleared.

## Inputs and outputs
- Inputs: $48 (direct page) and the byte at DBR:$1220. Entry widths: 8-bit accumulator, 16-bit index
  registers.
- Outputs: $48 = $48 AND $7F. X = (byte at $1220 * 2) AND $FF, as a 16-bit value (its high byte is 0).
  A = the same value, B = 0. N is clear for 16-bit index registers (bit 15 of X is 0); with 8-bit index
  registers N would be bit 7 of the value. Z is set when the value is zero. C is bit 7 of the byte at
  $1220. V and Y are unchanged. The routine returns with RTS.

## Path and cost
One path, 202 master cycles, for all 1,792 calls in the four replays. The only call site is $04:A0B3.
In the replays the byte at $1220 is between 0 and 7, so C is always clear there; the bit-7 case follows from
the code and is not seen.
