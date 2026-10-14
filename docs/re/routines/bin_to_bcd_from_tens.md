# bin_to_bcd_from_tens ($06:EA0E, RTL, 25 bytes)

The tens step of the 24-bit binary-to-BCD conversion (`bin24_to_packed_bcd`, `$06:E969`, whose
notes are in `m1-candidates.md`): a separate entry that takes the remainder in the direct page and
returns the packed byte. The original's suffix entries are reached by their own callers, not only
from the head of that routine.

## Inputs and contract
- `$00`: the remainder (the ones are taken from it, ten at a time).
- `$03`: the packed byte of the result, whose high nibble counts the tens.
- Accumulator 8-bit (the routine's constants are 8-bit), index registers any; direct page 0.

## What it does
- While `$00` is at least ten, `$00` drops ten and `$03` gains `$10`. The loop ends when `$00` is
  below ten, or when `$03` wraps back to zero (then `$00` has already dropped ten for that pass).
- Then `$03` takes its OR with `$00`, and A low is that value (B kept). N and Z come from the value;
  C and V come from the last subtraction or addition.

## Outputs
- `$00`, `$03`, A low, N, Z, C, V as above. X, Y, B and the other direct-page bytes are unchanged.

## Paths and costs
Each pass costs 194 master cycles and ten instructions. The two exits:
- borrow exit (the ones byte below ten): 192 + 194 per full pass, 8 + 10 per full pass instructions;
- wrap exit (the tens byte back to zero): 110 + 194 per pass counting the wrapping pass, 4 + 10 per
  pass instructions.

The costs follow from the 65816 timing: 8 master cycles per bus access and 6 per internal cycle.
The reference replays show costs 192, 386, 580, 968, 1,162, 1,356, 1,550 and 1,744 (no wrap exit),
which are exactly 192 + 194 k. The static maximum (`cycles`) is 25 passes: 5,042.

## Callers
Twelve call sites (JSL), `$01:BFBA` (388 calls), `$01:AA72` and `$01:AA9C`, `$01:BF9C`,
`$01:C043`, `$01:EE80`, `$01:CAFD`, `$1D:84B0`, `$01:A9B4`, `$01:AC35`, `$1C:8044` and `$01:F786`
(1,005 calls in the replays).
