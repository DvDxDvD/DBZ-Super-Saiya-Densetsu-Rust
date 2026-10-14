# load_flag_01d6 ($1D:8ABD)

Loads the byte at data-bank address $01D6 (`flag_01d6`) into the low byte of A, and sets N and Z from it.
B, C, V, the index registers and memory are unchanged.

When that byte has bit 7 set, the routine goes on into code the reference replays never reach (a series
of tests on other state bytes, then further code). The Rust version hands that case to the original code
(`Ctx::call_original` from the entry, own cost 0), and its bound function declines it, so in `Rust` mode the
original code runs it.

## Inputs and outputs
- Input: the byte at DBR:$01D6. Entry widths: 8-bit accumulator, 16-bit index registers.
- Outputs (non-negative byte): A low byte = the byte, B unchanged; N = bit 7 (clear), Z = the byte is zero;
  C and V unchanged. The routine returns with RTL.

## Paths and costs
- Byte with bit 7 clear: one path, 92 master cycles, for all 3,649 calls in the replays (battle 1,618,
  wander 2,031; the landing and menus replays make none).
- Byte with bit 7 set: not reached by the replays; it runs the original code (own cost 0).

## Callers
- $05:82E0, the only call site seen (a `JSL`).
