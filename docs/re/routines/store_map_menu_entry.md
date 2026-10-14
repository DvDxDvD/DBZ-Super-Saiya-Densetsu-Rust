# store_map_menu_entry ($01:B86A)

Stores a byte into the map menu block: the byte at WRAM $7E:0D62 goes to `map_menu_cursor` plus the index
byte at $7E:0D66 (indices 0 to 255, so the block is $7E:1100 to $7E:11FF). The index is also left in Y.

## Inputs and outputs
- Inputs: the index byte ($0D66, `map_entry_index`) and the value byte ($0D62, `map_entry_value`).
- Outputs: the block entry, Y = the index, A = the value with B cleared, N and Z from the value; C and V
  are kept, X and the direct page are unchanged.
- The routine needs an 8-bit accumulator at entry.

## Path and cost
One path, 196 master cycles, for all 5,739 calls in the replays. The call sites are $01:B82D (5,649 calls) and
$01:BCCD (90). The routine returns with RTL.
