# add_four_to_dp_word ($03:95A4, RTL, 15 bytes executed)

Adds four to the 16-bit word at direct-page offset `$02`.

## Inputs and contract
- The routine switches the accumulator to 16 bits and back to 8 bits before it returns, so the Rust
  version (which leaves the width alone) is declared for an 8-bit accumulator on entry. Index
  registers unused. Direct page 0.
- Reads and writes the word `$02`-`$03`.

## What it does
- The word at `$02` plus four (16-bit addition with the carry clear), stored back to `$02`.
- A is the sum (the full 16 bits, B included); N and Z are set from it; C and V come from the
  addition.

## Path and cost
- One path: 212 CPU-busy master cycles from the hit to the return, for every call (measured in
  `Verify` over 3,683 calls of the four replays).
- The listing has eight bytes between the branch at `$03:95AC` and its target at `$03:95B6`. No
  path runs them, so they are not part of the routine.

## Callers
- `$03:967E` (2,064 calls in the replays), `$03:9544` (1,613 calls) and `$03:977C` (6 calls).
