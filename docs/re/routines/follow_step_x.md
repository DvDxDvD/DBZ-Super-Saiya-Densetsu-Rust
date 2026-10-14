# follow_step_x ($06:EDB3, RTL)

The X follower of the map: its 16-bit position moves towards the leader's X, by at most four units
a call, and settles at a rest distance of `$80` behind the leader. It also keeps the follow flags
of the X axis.

## Inputs and contract
- Accumulator 8-bit on entry; index registers not used. Direct page 0 and data bank 0 at every
  observed call.
- Reads: the leader's X (a word at `$01A2`, through the data bank), the follower's X (a word at
  `$33`), the limit byte of the X axis (`$0703`) and the follow flags (`$0700`).
- Writes: `$00`; the new position at `$33-$34` and in the scratch bytes `$10-$11`; the flags at
  `$0700` when the call steps.
- A keeps its high byte. C, V and N, Z are set by the last operation of the chosen branch.

## What it does
- Takes the difference D = leader minus follower, as a 16-bit subtraction. `$00` gets its low
  byte.
- Rest: when D is exactly `$80`, the routine returns at once. Only `$00` changes (to `$80`); A is
  `$80` in its low byte; N is clear and Z and C are set.
- A negative high byte of D steps the follower back by four. A positive one steps it forward by four.
- With a zero high byte, the distance from the rest point is the step when it is below four,
  otherwise four. A distance below the rest point steps back, one above it steps forward.
- A step back that would go below zero gives zero. A step forward whose high byte reaches the
  limit minus one (`$0703 - 1`) gives a zero low byte.
- The follow flags get 3 (a step back) or 2 (a step forward) ORed in. Then bits 0 and 1 are cleared
  or kept, by bit 2 of the new low byte and bit 0 of the flags. A and the carry come from the same
  decision.

## Paths and costs
Each branch the reference replays take has its own cost (CPU-busy master cycles from the hit to
the return), measured in `Verify` over 273 distinct calls of the battle, landing, menus and wander
replays. The branches the replays do not take (a negative or positive high byte, a step back with
a flag combination not seen, and similar) run the original code: the cost bound declines them.

The end of a step depends on bit 2 of the new low byte and bit 0 of the stored flags: bits 0 and 1
of the flags are cleared when those two bits have the same value, and kept otherwise.

| Branch | Cost |
|---|---|
| rest at `$80` | 314 |
| step back, distance below four, zeroed position, flags end kept (bit 2 clear, bit 0 set) | 922 |
| step back, distance below four, flags end cleared (bit 2 and bit 0 both set) | 986 |
| step forward by four, zeroed low byte, flags end cleared (both clear) | 1014 |
| step forward, distance below four, flags end cleared (both clear) | 970 |
| step forward, distance below four, flags end kept (bit 2 set, bit 0 clear) | 896 |
| step forward, distance below four, zeroed low byte, flags end cleared (both clear) | 1004 |

The unit tests check the branch choice on synthetic positions.

The instruction count varies with the branch as well (14 for the rest, 40 to 46 for the steps the
replays take, from the hit to the return); each branch declares its count with `set_instructions`,
and the replays confirm them.

## Callers
- `$06:ED8C` (3,128 calls in the replays) and `$06:ED7E` (364 calls).
