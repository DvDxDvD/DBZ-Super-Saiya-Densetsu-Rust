# load_counter_1570 ($1D:8843)

Loads the byte at DBR:$1570 into the low byte of A, and sets N and Z from it. B, C and V are unchanged.

When that byte has bit 7 set, the routine goes on into a loop over the counters that start at $1570 (it
walks the bytes from $1571 upwards, decrementing the ones that hold ordinary values and handling the
special values $FF and $FE), and then into code that scales a pair of direct-page words. The reference
replays never take that path. The Rust version hands it to the original code (`Ctx::call_original` from
the entry, own cost 0), and its bound function declines it, so in `Rust` mode the original code runs it.

## Inputs and outputs
- Input: the byte at DBR:$1570. Entry widths: 8-bit accumulator, 16-bit index registers.
- Outputs (non-negative byte): A low byte = the byte, B unchanged; N = bit 7 (clear), Z = the byte is zero;
  C and V unchanged. The routine returns with RTL.

## Paths and costs
- Byte with bit 7 clear: one path, 92 master cycles, for all 3,545 calls in the replays (battle 1,576,
  wander 1,969). In every one of them the byte was zero, so the replays do not test a non-zero value.
- Byte with bit 7 set: not reached by the replays; it runs the original code (own cost 0).

## Callers
- $05:8268, the only call site seen (a `JSL`).
