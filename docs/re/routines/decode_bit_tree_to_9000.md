# decode_bit_tree_to_9000 ($00:FDB5, RTL, 108 bytes)

Expands a variable-length bit code into 2,048 symbols: a walk through a binary node table, one
stream bit per step, until a leaf. Each symbol is written as a word (its low byte, then a zero byte)
to WRAM bank `$7E` at `$9000` plus the index register X.

## Inputs and contract
- X: the output index (the reference entry is 0; the output runs to `$1000`).
- Y: the stream index (the reference entry is 0).
- Direct page `$00`: saved and restored by the routine; its entry word becomes the accumulator at
  exit. Accumulator 16-bit and index registers 16-bit on entry.
- Tables: the node table at `$00:FD49` (a pair of words per node, read at the node's doubled index,
  the second word of a pair chosen by the bit) and the stream at `$00:F150`, one byte per eight bits,
  least significant bit first.

## What it does
- Each symbol starts at the root node `$001A`. A step reads the stream bit (the byte at the stream
  index AND the bit mask); it picks the child word of the node at `node * 4 + 2 * bit`, then moves
  the mask on (the stream index advances after bit 7). The walk ends at a node whose high byte has
  bit 7 set; the symbol is that node's low byte.
- The symbol is written at `$7E:9000 + X` and a zero byte after it; X counts by two up to `$1000`.

## Outputs
- X = `$1000`; Y = the stream index reached; DBR = `$7E` (left so by the original);
- A = the entry word of direct page `$00`; N and Z from that word; C set; V from bit 6 of the last
  node's high byte.
- The direct page holds its entry bytes (the original saves and restores `$00-$05`).

## Path and cost
Only the reference entry (X = 0, Y = 0) runs in Rust, with a bound of 3,294,484 master cycles
(about 9.2 frames, measured in `Verify`); other entries run the original code and are declined by the
bound. Budget: 11 frames.

## Callers
One call site, `$00:FD3D` (4 calls in the replays, all at the reference entry).
