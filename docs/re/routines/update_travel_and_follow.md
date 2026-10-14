# update_travel_and_follow ($01:82E3, RTL, 186 bytes)

Applies the leader's travel steps on the X and Y axes, chosen by a flags byte and by a nibble or
a byte; the follow flags are set from a table when a step happens.

## Inputs and contract
- Accumulator 8-bit on entry; index registers not used. Direct page 0 and data bank 0.
- Reads (through the data bank): the travel flags `$01BC`; the low nibble of `$0048`; `$0044`; the
  copy gate `$01A1` (only bit 7, for the cost); `$0D5E`; the X and Y limit bytes `$0703` and `$0702`;
  the X step (the word at `$0D55`, the `travel_mode` symbol) and the Y step (the word at `$0D57`);
  the leader's X and Y (words at `$01A2` and `$01A4`); the index byte `$0D59`. The table byte is
  read from bank `$01` at `$83A0` plus the index.
- Writes: `$0700` (the flags, always: zero, or the table byte when the leader steps); `$01BC`
  (bit 5 cleared, path A with a nonzero nibble); `$0D5E` (path A with a nonzero nibble); `$00` and
  `$01` (the X or Y branch: `$F0` and the limit minus one); the leader's X or Y when it steps. X
  becomes the index when the leader steps.

## What it does
- The travel flags byte decides the path. Bit 0 set: path A. Path A takes the nibble of `$0048`
  as its value, and stores it to `$0D5E` when it is not zero (clearing bit 5 of `$01BC` then). Bit 5
  of `$01BC` still set after that: the routine returns at once, with A equal to `$20` in its low
  byte. Bit 0 clear: path B takes the byte at `$0044` as its value.
- The value's bits pick the axis and the direction. Bit 0: X up by its step, unless the leader's X is
  at or above the word whose low byte is `$F0` and whose high byte is the X limit minus one; that
  case leaves the position alone (bit 2 does the same on the Y axis, with the Y limit). Bit 1: X down
  by its step, if the leader's X is at least `$10`. Bit 3: Y down by its step, the same test on Y.
  A value with bits 0 to 3 clear returns with its high nibble in the low byte of A.
- A stepping call stores the table byte `$01:83A0` plus the index (`$0D59`) in `$0700`, leaves A
  as that byte with B clear, X as the index, and N and Z from the byte, C and V from the step.
- A call that does not step leaves N, Z and C from the compare or the shift it ended with, and V as
  it was.

## Paths and costs
- Costs depend on the route, the path (A or B), whether path A's nibble was zero, and the copy gate's
  bit 7. The reference replays (3,492 calls at the one call site) take 15 combinations, all with the
  gate's bit 7 clear; each has one measured cost in CPU-busy master cycles from the hit to the return:
  272 (path A, nibble zero, early return), 528 (path B, idle), 796 (X at the limit, nibble zero),
  844 to 1,106 (X down), 926 to 1,188 (X up), 894 to 1,156 (Y down) and 998 to 1,260 (Y up).
- The other combinations (the Y limit and lower-bound exits, the gate bit 7 set) are not observed.
  The cost bound declines them, and the original code runs them.
- The instruction count of each branch is declared with `set_instructions`: from 11 for the early
  return to 52 for the longest step, the path (B, or A with a zero or nonzero nibble) and the route
  each add to it. The replays confirm the counts on every call.

## Callers
- `$00:9784` (3,492 calls in the replays).
