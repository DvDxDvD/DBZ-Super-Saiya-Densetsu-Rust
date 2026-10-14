# lookup_scaled_entry ($05:BEAB)

Takes the low seven bits `v` of A, reads one byte from a 512-byte table in ROM bank $1E (`scale_table`, at
index `4v`), and returns six times that byte: A holds the 16-bit product (B is its high byte) and X holds
the same value (its low byte alone when the index registers are 8-bit). The direct page receives `4v` at
$12-$13 and twice the byte at $00-$01, both as 16-bit stores.

## Inputs and outputs
- Input: A (its low seven bits; the high byte is discarded).
- Outputs: A and X (6 times the byte), $00-$01, $12-$13. Flags: N clear, Z set when the byte is zero, C clear,
  V clear.

## Path and cost
One path, 454 master cycles, for all 3,678 calls in the replays. The call sites are $05:BF31 (1,829 calls),
$05:BE7A (1,372) and $05:BEE2 (477). The routine returns with RTS.
