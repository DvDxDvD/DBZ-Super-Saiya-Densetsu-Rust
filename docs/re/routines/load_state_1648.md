# load_state_1648 ($03:DD83)

Loads the byte at data-bank address $1648 (`state_1648`) into the low byte of A. N and Z follow that
byte. Nothing else changes: B, C, V, the index registers and memory are left as they were.

When the byte has bit 7 set, the routine goes on into a dispatcher that is not part of the path the
reference replays take: it compares $01D8 with a short list of values and jumps to one of several
handlers. The Rust version hands that case to the original code (`Ctx::call_original` from the entry,
own cost 0), and its bound function declines it, so in `Rust` mode the original code runs it and the
timing window never has to cover it.

## Inputs and outputs
- Input: the byte at DBR:$1648. Entry widths: 8-bit accumulator, 16-bit index registers.
- Outputs: A low byte = the byte, B unchanged; N = bit 7 of the byte, Z = the byte is zero; C and V
  unchanged. The routine returns with RTL.

## Paths and costs
- Byte with bit 7 clear: one path, 92 master cycles, for all 1,324 calls in the four replays (battle
  343, landing 335, wander 646; the menus replay makes none).
- Byte with bit 7 set: not reached by the replays; it runs the original code (own cost 0).

## Callers
- $00:9926 (the `JSL`, the only call site seen). The caller loads $0D89 straight after the call, so it
  does not read A or the flags that this routine leaves.
