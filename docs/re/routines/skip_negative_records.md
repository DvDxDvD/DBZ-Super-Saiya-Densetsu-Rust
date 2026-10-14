# skip_negative_records ($03:994C, RTL, 36 bytes)

Scans five-byte records in WRAM from `$0C43`, the index stepping by five per record. A record whose
first byte has bit 7 set is looked at: its word (at offset 1) less the
reference word in direct page `$33` is stored at direct page `$0C`, and when that difference is at
least `$FF` (unsigned) the record is skipped. The scan ends at the first record whose first byte is
positive, or after the last record in its range. Records that pass the window test lead to two routines
outside this one (`$00:85E9` and `$00:85F5`), which the replays never reach.

## Inputs and outputs
- Inputs: the records at `$0C43`, the reference word at `$33`. Entry: accumulator 8-bit, index
  registers 16-bit (all 12,026 replayed calls; the entry index is not used).
- Replayed sequence: the record at X = 0 and the record at X = 5 are skipped (negative, difference at
  least `$FF`), and the record at X = 10 is positive, which ends the scan.
- Outputs: direct page `$0C-$0D` hold the last difference (the record at X = 5). X = 10. A = the last
  difference's high byte with the end record's byte as the low byte. N clear, Z set when that byte is
  zero, C clear, V the overflow of the last subtraction. Y and the records are unchanged.

## Paths and costs
- The replayed sequence costs 896 CPU-busy master cycles on all 12,026 calls: 24 for the index load,
  384 for each skipped record, 104 for the end record. It runs 38 instructions.
- Any other sequence (a record whose difference is below `$FF`, a positive first record, a scan past
  the record at X = 10) runs the original code (`call_original`, own cost 0). The bound declines those.

## Callers
- JSL from `$00:985F` (7,491 calls), `$00:979C` (3,664), `$00:9B18` (450) and `$00:997D` (421).
