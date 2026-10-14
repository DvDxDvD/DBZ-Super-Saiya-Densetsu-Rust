# entity_draw_pass_60_80 ($01:F9AC, RTL)

Entry of a two-slot pass over the map's entity table, which runs only when bit 0 of the gate byte
`$01BC` is clear. Its work on the two slots (at `$0E60` and `$0E80` in the entity table of stride
`$20`) is a sprite draw: a non-empty slot is tested with `entity_id_in_special_set`, its id picks
a word from a table at `$01:FD10`, and the sprite is laid out with the slot's position bytes and
drawn by the metasprite routine at `$00:85F5`.

## Inputs and contract
- Entry: A8, X16 (the loop loads Y with a 16-bit constant), DBR and D as the caller leaves them
  (the reference calls run with DBR `$00` and D `$0000`).
- Gate: the byte `$01BC` (`gate_flag_01bc`), read through the data bank.

## Paths and costs (CPU-busy master cycles from the hit to the return)
| Path | Condition | Cost | Replays |
|---|---|---|---|
| Gated return | bit 0 of `$01BC` set | 114 | 1,234 calls, all this cost |
| Pass | bit 0 clear | original code | never reached in the replays |

The gated return is a load, an AND, a taken branch and the return. Its outputs: A low = 1 (B
unchanged), N and Z of that value (both clear), and the other flags, X and Y kept.

## Port
- `entity_pass_bound`: the bound returns 114 for the gated return and no bound (`u64::MAX`) for the
  pass, so in `Rust` mode the pass always runs the original code and the timing window never opens
  for it.
- The Rust function runs the gated return; any other call hands the whole routine to the original
  (`call_original` from the entry, own cost 0).

## Callers
- `$01:F97B`: the one call site in the replays (1,234 calls).
