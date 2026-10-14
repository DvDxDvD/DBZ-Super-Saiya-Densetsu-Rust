# lookup_bank2_pointer ($00:9C6F, RTS, 34 bytes)

Turns a one-byte index into a long pointer into bank `$02`, and leaves the pointer in the
direct page for the code that follows.

## Inputs and contract
- `$00`: the index byte. Only its low seven bits matter: the index is doubled and kept to 8
  bits, so the routine reads the table's even entries only.
- Accumulator 8-bit on entry (M = 1), index registers 16-bit (X = 0). Direct page 0.
  The data bank is not used; every access is a long address or a direct-page byte.

## What it does
- The table is the bank-`$02` pointer table `bank2_pointer_table` at `$02:DA16`, 256 bytes:
  128 little-endian 16-bit pointers.
- The entry address is `$DA16` plus the doubled index, computed in 16 bits (it cannot leave
  the table). The routine reads the 16-bit word at `$02:` entry address.
- The word becomes the low two bytes of a 24-bit pointer at `$00-$02`, whose bank byte is `$02`.
  The entry address goes to `$03-$04`, and `$05` holds the bank byte again.
- Y is set to 0. A is the word, all 16 bits (the high byte is kept in B).

## Outputs
- `$00-$01`: the word. `$02`: `$02`. `$03-$04`: the entry address. `$05`: `$02`.
- A: the word. Y: `$0000`. M and X unchanged.
- N and Z from the word; C and V cleared.

## Path and cost
- One path, with no loops: 470 CPU-busy master cycles from the hit to the return, for every
  index (measured in `Verify` over the reference replays).

## Callers
- `$00:9CE0`: the routine at `$9CD8` calls it, then reads through the pointer in `$00`.
- `$00:9C67`: calls it, then calls `$00:9C91` directly, which reads through the same pointer and
  copies the bytes into a WRAM tile buffer at `$7E:3000`.
