# derive_state_071a ($00:83E4)

Derives three state bytes from two inputs and sets a fourth from the sign of a control byte:

- $0719 = the control byte at $0716 with its top three bits cleared (its low five bits);
- $0718 = the byte at $0029 with its high nibble cleared (its low four bits);
- $071A = 2 when bit 7 of the control byte at $0716 is set, else 1.

A holds that last value at the end. Bit 7 of the control byte picks the value; nothing else in the
routine depends on it.

## Inputs and outputs
- Inputs: $0716 and the direct-page byte $0029 (D = 0 at every replayed call). Entry widths: 8-bit
  accumulator, 16-bit index registers.
- Outputs: $0718, $0719 and $071A as above; A low byte = $071A's value (B unchanged). N and Z are
  clear (the last load is a constant 1 or 2); C and V unchanged. X and Y unchanged. The routine returns
  with RTL.

## Paths and costs
- Control byte with bit 7 clear: 314 master cycles (A = 1), 16 of the 27 calls in the replays.
- Control byte with bit 7 set: 298 master cycles (A = 2), 11 calls.
The routine has no other path. The call site is $00:9738 (the only one seen).
