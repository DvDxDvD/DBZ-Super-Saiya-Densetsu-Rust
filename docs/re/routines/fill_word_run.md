# fill_word_run ($01:BC71, RTL, 23 bytes)

Stores one word from the direct page into a run of low-bank WRAM, as many times as the count byte
at `$0E` says (at least once).

## Inputs and contract
- Accumulator 8-bit and index registers 16-bit on entry. The routine switches the accumulator to
  16 bits for the stores and back to 8 bits before it returns. Direct page 0 and data bank 0.
- Reads: the byte at `$0E` (the count) and the word at `$10` (the value).
- Writes: `$0F` is cleared first; then the run `$7E:3000` plus X, one word per count, two bytes
  further each time.

## What it does
- Clears `$0F`, so the count word is the byte at `$0E`.
- Stores the value word at `$7E:3000 + X`, X going up by two per word, for a count of the byte at
  `$0E`; a count of zero stores once (the loop body runs before the test).
- Leaves X after the last word, Y as the count, A as the value word.
- N is clear. Z is set when the count equals the byte at `$0E` (so it is clear for a zero count),
  C is set, and V is unchanged.

## Paths and costs
- The cost is linear in the count: 162 plus 144 per word, in CPU-busy master cycles from the hit to
  the return. Measured in `Verify` for the two counts the replays take: 11 words (1,746 cycles, 9
  calls in the menus replay) and 16 words (2,466 cycles, 150 calls in the wander replay).
- The other counts are not observed. Their calls are declined by the cost bound, so the original
  code runs them (the routine's own fallback would run the original too).
- The instruction count is six per word plus six (72 for eleven words, 102 for sixteen), declared
  with `set_instructions`; the replays confirm both.

## Callers
- `$01:8EFB` (150 calls in the replays) and `$01:94CE` (9 calls).
