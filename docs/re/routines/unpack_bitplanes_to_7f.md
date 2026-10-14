# unpack_bitplanes_to_7f ($04:83B2, RTL, 95 bytes)

Converts planar 4-bit pixel data into one byte per pixel. Each row of eight pixels comes from two
16-bit words: the first word and the word 16 bytes further on, in the data bank given in direct page
`$88`. The pixels go to bank `$7F`, at the odd offsets from `$7F:0001` plus the index register X.

## Inputs and contract
- Direct page: `$86` (word) = start offset of the source; `$88` (byte) = data bank; `$89` (word) =
  span, so the end offset is the start plus the span.
- Index registers 16-bit (the routine sets 16-bit indexes itself); the entry widths are restored at
  exit as the routine leaves them (PHP and PLP), so the flags are unchanged.
- The reference call: start `$9000`, bank `$7E`, span `$0660`, X = 0 (end `$9660`).

## Pixel rule
For pixel `k` (0 first, 7 last), with bit `b = 7 - k` of each byte: the value is four bits, most
significant first: bit `b` of the high byte of the second word, of its low byte, of the high byte of
the first word, of its low byte.

## Row and block walk
- A row reads the two words at the offset and offset + `$10`, writes its eight pixels, and advances
  the offset by two.
- When the offset is a multiple of sixteen after an advance, the offset moves on by a further `$10`
  (the second words of the block are skipped) and the walk stops once the offset reaches the end.
- The reference call runs 51 blocks of eight rows (408 rows, 3,264 pixels).

## Outputs
- A = Y = the end offset reached (16-bit); X = the output index reached (+2 per pixel).
- Direct page: `$73-$74` = `$000F`; `$75-$76` = `$0010`; `$8D-$8E` = the end; `$8F` = `$FF`;
  `$89-$8A` = the span shifted left once; `$91-$94` = 0. The other bytes are unchanged.

## Path and cost
Only the reference call runs in Rust (the cost is 1,313,224 master cycles, measured in `Verify`).
Any other call runs the original code (its loop length depends on the entry), so the bound declines
it (`unpack_bitplanes_bound`). Budget: 5 frames.

## Callers
One call site, `$04:8768` (4 calls in the replays).
