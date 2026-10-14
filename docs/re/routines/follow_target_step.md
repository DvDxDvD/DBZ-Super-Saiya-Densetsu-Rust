# follow_target_step ($06:EE42)

Moves the 16-bit position in the direct page (`follow_pos`, $35-$36) a few units towards a target word
(`follow_target`, $01A4-$01A5), by at most four per call. Inside a dead band the position stays where it
is: the low byte of the difference is $78 when the high bytes are equal. The routine also compares the
new position's high byte with a limit byte (`follow_limit`, $0702) minus one, and it updates a flags byte
(`follow_flags`, $0700): bits 2 and 3 are cleared when bit 2 of the new position's low byte and bit 2 of
the flags byte agree; otherwise the byte keeps the value it got from the OR with $08 or $0C.

## Inputs and outputs
- Read: target ($01A4-$01A5), position ($35-$36), limit ($0702), flags ($0700).
- Written: $00 (the low difference, or the step), $10-$11 and $35-$36 (the new position), and $0700.
- Registers: only the low byte of A changes (B and X and Y are kept); N, Z, C and V are set by the last
  operation of the path taken.

## Routes and costs (master cycles, from the hit to the return)
| Route | Condition | Cost | Replay calls |
|---|---|---|---|
| Rest | high byte of the difference zero, low byte $78 | 314 | 185 |
| Step down, clear | difference low byte below $78 by 1..3; bits 2 and 3 cleared | 988 | 133 |
| Step down, keep | the same step; bits kept | 924 | 128 |
| Step up, unequal, clear | difference low byte $79..$7B (step 1..3), limit compare not equal | 972 | 38 |
| Step up, unequal, keep | the same, bits kept | 898 | 44 |
| Step up, equal, clear | step 1..3, limit compare equal | 1006 | 65 |
| Step up of four, equal, clear | difference low byte above $80 (step 4), limit compare equal | 1016 | 2,899 |

Routes the replays never take (a high byte that is not zero, a step down of four or more, a step that
would go below zero, and the unequal or kept combinations of the step-four route) go back to the original
code, from the entry, before anything is written. Their own cost is zero; their cost is the original's.

The maximum over the measured routes (`cycles`) is 1016. For the original-code routes the cost is not
measured; by the timing model that matches every measured route, each one is at most 1016 (about 10 to 40
master cycles below the clamped route), so `cycles` still bounds them.

## Callers
JSL from $06:ED88 (3,128 calls), which then calls $06:EDB3, whose first instructions set N and Z and then
C and V before reading them; and from $06:ED82 (364 calls), which branches to a load of the flags byte.
No caller reads the flags left by this routine.
