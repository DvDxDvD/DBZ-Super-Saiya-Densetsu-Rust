# scan_record_table ($06:EC76, RTL)

Walks a table of ten-byte records in WRAM from `$074C`, one record after another, until a record
whose position word is `$FFFF` (the end). Each record holds a position word at its start and a flags
byte nine bytes on. Every position is compared with the word at `$01A2`: a record whose flags byte is
zero and whose position minus that base, plus eight, is at least `$10` (unsigned) is passed over. Any
other record is acted on instead: the routine copies fields of it into WRAM,
stores several WRAM bytes and calls two routines outside this one. That path is not on any replayed
call, so it runs the original code.

Before the table, bit 0 of `$01BC` is tested: when it is set the routine returns at once.

## Inputs and outputs
- Inputs: `$01BC`, `$01A2`, and the records from `$074C`, all read through the data bank (`$00`).
  Entry: accumulator 8-bit, index registers 16-bit (all 3,664 replayed calls).
- Immediate path (bit 0 of `$01BC` set): A's low byte becomes that byte shifted right, B is kept,
  N is clear, Z follows the shifted value, C is set, V is kept. Nothing is written.
- Two-record path (bit 0 clear, two records passed, the end word as the third record): A = `$FFFF`,
  Y = `$0014`, N clear, Z and C set, V clear. X and memory are unchanged.

## Paths and costs
- Immediate: 106 CPU-busy master cycles, 4 instructions. Measured on 1,789 calls.
- Two-record path: 1,294 cycles, which is 92 for the prefix, 514 for each passed record and 174 for
  the end word; 52 instructions. Measured on 1,875 calls. Every replayed call takes one of these two.
- Any other path (a record with flags or inside the window, an end word at the first or a later
  record than the third, a third record that is not the end) runs the original code from the entry,
  with its own cost 0. The bound function declines those, so the window never opens for them.
- All 3,664 calls fit the timing window with the bound (100%).

## Callers
- JSL from `$00:97A0` (every replayed call).
