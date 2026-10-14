# timer_mask_block ($05:C1EE, RTL, 113 bytes)

Counts down a per-slot timer byte and, when the timer is still running, masks the words of the slot's
block with a value taken from a ROM table. The slot is chosen by the caller's X through two tables: a
ROM word gives the slot index Y, and a WRAM word gives the block's record index.

## Inputs
- X: the caller's index (0, 2, 4 or 6; entry index registers 16-bit, accumulator 8-bit: all 14,180
  replayed calls).
- Y = the ROM word at `$05:9B89 + X` (`index_word_table`); the timer is the byte at `$0029 + Y`, the
  flag byte is at `$0021 + Y` (data bank 0).
- The side byte at `$1474` (`side_record_b` plus `$34`). The replays read `$03` there.
- The record index: the WRAM word at `$15E9 + X` (`slot_index_table`). The block's words are at
  `$0803 + index`, `$0805 + index` (the pointer: low word and bank word) and `$0806 + index` (the top
  word, which sets the pass count).

## Outputs
- Short path (the timer's low seven bits are zero): nothing is written. A = Y with its low byte
  cleared, Y is kept, C is set when the side byte is at least `$14` (it is clear on replayed calls),
  N is clear, Z is set. X and V are unchanged.
- Long path (the low seven bits are set): the timer is decremented and written back. When the
  decremented value's low seven bits are set, the flag byte gets bit 7 set. The mask word is read from
  `$05:C273` at the offset of the value shifted left once (the decremented timer, or its low seven bits
  with `$10` added when the timer is negative; the shift drops bit 7). The mask goes to `$10`, the pointer to `$00` and
  `$02`. Then the block is masked from the top: for each index from the top word less two down by two,
  while the next index is below `$8000`, the word at the pointer plus that index is ANDed with `$10`
  and stored back. Exit: A = the last word stored, Y = the index after the last pass (`$8000` or more),
  N set, Z and C clear, V and X unchanged. C is clear because the shift of a value below `$80`
  leaves it so.

## Paths and costs
- Short path: 328 CPU-busy master cycles, 13 instructions. Measured on 13,920 calls.
- Long path: `1290 + 194 n` cycles for `n` passes, with `+26` when the decremented timer is negative
  and `-86` when the flag is not updated. The four combinations are measured: 44,746 (updated, positive,
  224 passes), 38,564 (updated, negative), 44,660 (not updated, positive), 38,478 (not updated,
  negative), and 78,890 for 400 passes. A pass costs 194 cycles; the last pass costs 6 less on its exit
  branch, which is in the base.
- Instructions: `48 + 6 n`, plus 2 when the decremented timer is negative, minus 3 when the flag is not
  updated.
- Not on any replayed path, so the original runs (`call_original`, own cost 0) and the bound declines
  them: a side byte of `$13` or `$14`, and a long path whose flag byte has bit 7 set (the early return).
- The bound function computes the path and the pass count from the entry state (reads only), so
  each call's cost is known before the window check. Of 14,180 calls, 9,511 fit the timing window (67%).

## Callers
- JSL from `$05:8276` (3,545 calls), `$05:8290` (3,545), `$05:82A5` (3,545) and `$05:82BA` (3,545).
