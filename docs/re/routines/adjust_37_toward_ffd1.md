# adjust_37_toward_ffd1 ($00:AC6B)

Moves the word at $37 one step towards $FFD1, but only while the byte at $63 is $1A. With any other value
of $63 it does nothing to memory.

## Inputs and outputs
- Inputs: the byte at $63 and the word at $37 (direct page 0). Entry widths: 8-bit accumulator, 16-bit
  index registers.
- Outputs by path:
  - $63 is not $1A: A's low byte = $63 (B unchanged); N = bit 7 of ($63 - $1A), Z clear, C = ($63 >= $1A);
    V unchanged.
  - $63 is $1A and the word is $FFD1: nothing is written; A = the word (16 bits); N clear, Z and C set; V
    unchanged.
  - $63 is $1A and the word is above $FFD1 (unsigned): the word becomes one less and is written back; A = the
    new word; N and Z from it; C set; V unchanged.
  - $63 is $1A and the word is below $FFD1: the word becomes eight more and is written back; A = the new
    word; N and Z from it; C = the carry out of that addition; V = its signed overflow.
- The routine returns with RTS. Index registers and the other memory are unchanged.

## Paths and costs
- $63 not $1A: 126 master cycles, for 352 calls in the replays.
- $63 is $1A, word $FFD1: 220 master cycles, for 1,676 calls.
- $63 is $1A, word above $FFD1: 282 master cycles, for 28 calls.
- $63 is $1A, word below $FFD1: 322 master cycles, for 108 calls.

The four costs and their instruction counts (5, 9, 12 and 14) are declared per path. The one call site seen is
$00:A374 (a `JSR`); all 2,164 calls are in the four replays and each path is reached in them.
