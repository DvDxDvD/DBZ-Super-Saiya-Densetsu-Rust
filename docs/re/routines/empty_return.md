# empty_return ($05:FDB8, RTS, 1 byte)

A routine that is a bare return: the only byte is the return instruction.

## Inputs and contract
- Nothing is read. Any register and width is accepted; every observed call had 8-bit accumulator
  and 16-bit index registers, direct page 0 and data bank 0.

## What it does
- Nothing. No register, flag or memory byte changes.

## Path and cost
- One path: 42 CPU-busy master cycles from the hit to the return, for every call (measured in
  `Verify` over the battle and wander replays: 7,090 calls).
- About 88 percent of the calls fit the timing window of Rust mode. The rest span a timing event
  and run the original code.

## Callers
- Two `JSR` instructions, at `$05:82C5` and `$05:82CB`, each seen 3,545 times in the replays.
  What the callers do after the call was not traced further.
