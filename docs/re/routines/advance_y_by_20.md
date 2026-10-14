# advance_y_by_20 ($00:8AF1)

Adds $20 to the index register Y as a 16-bit value and leaves the sum in the whole accumulator too. It is
the Y counterpart of `advance_x_by_6`: a record-stride step of the object loops.

## Inputs and outputs
- Input: Y (16 bits; entry widths 8-bit accumulator, 16-bit index registers).
- Outputs: Y = Y + $20 and the full 16-bit accumulator (B:A) = the same sum. N and Z follow the sum (its
  16 bits with 16-bit index registers, its low 8 bits with 8-bit ones); C is the carry out of the 16-bit
  addition and V its signed overflow. The accumulator leaves in its 8-bit width, as it entered; X and
  memory are unchanged. The routine returns with RTL.

## Path and cost
One path, 154 master cycles, for all 1,183 calls in the four replays. The routine has 21 distinct call sites in
the replays; the most used are $01:C7FD (405 calls) and $03:EF54 and $03:EF7A (90 each).
