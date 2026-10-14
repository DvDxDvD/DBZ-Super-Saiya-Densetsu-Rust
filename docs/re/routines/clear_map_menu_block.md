# clear_map_menu_block ($01:C116)

Zeroes 96 bytes of the map menu buffer, which starts at `map_menu_cursor` ($7E:1100) and runs to $7E:115F,
as 48 word stores. The variable is not renamed for the block: the loader rejects a second variable that
overlaps `map_menu_cursor`, so the routine addresses the block from that name.

## Inputs and outputs
- No inputs. The routine needs X to be 16-bit at entry (its first instruction loads a 16-bit immediate).
- Outputs: the block is zero, X = $0060, A = $0000 (B cleared), N clear, Z set, C set. V and the
  direct page are unchanged. The exit mode is 8-bit accumulator, as at entry.

## Path and cost
One path, 5880 master cycles, for all 89 calls observed in the replays.

## Callers
JSL from $00:981D (45 calls), $00:98D0 (43) and $01:DDCF (1 call).
