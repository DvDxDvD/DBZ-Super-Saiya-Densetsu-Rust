# M1 first candidates: `pure` leaf routines (DBZ Super Saiya Densetsu)

Sources: Ghidra `/dbz_import.sfc` (933 functions), traces `C:\Tools\re-dbz\out\run1..run8` (342,000 frames
total). Hit counts are summed over run1-8. Where a loop branches back to the entry, the call count is
taken from the RTL/RTS hit count. Mode notation `MXE`: `100` = M8, X16, native. D (direct page) = 0 and
DBR = $00 at every sampled hit of every candidate. Note that DBR is only sampled at hits 1/16/256/4096/65536.

## Method
- Static scan of all 933 functions: JSR/JSL, JMP/JML out of body, WAI/STP, MVN/MVP, and any data reference
  or address operand in $2100-$21FF or $4000-$43FF (banks $00-$3F/$80-$BF). That gives 302 executed
  functions with no call, no out-of-body jump and no MMIO, which I then ranked by hits.
- Decimal mode: there is no `SED` in the 32,097 disassembled instructions, and no `SEP` with bit 3 that
  ever executes. The only SEPs with bit 3 are $03E21D `SEP #$6B` and $03EA85 `SEP #$EE`, which are
  decode artefacts with 0 hits. `PLP` only restores values pushed earlier. So D=0 is a safe invariant,
  and ADC/SBC are binary in every candidate.
- No traced call or indirect-jump target lands inside a candidate body, except the documented suffix
  entries of the BCD routine.

## GLOBAL CAVEAT: the idle loop makes cycle counts observable
At $0080CE-$0080DC the main loop calls rng_next_a once and rng_next_b twice, then tests the sign of $24
and goes round again, until the NMI sets bit 7 of $24. That is about 173 calls of A and 346 of B per
frame. The RNG state therefore depends on how many CPU cycles each frame's game logic takes.
Any hook (not only the RNG) that shortens or lengthens a frame's work changes the RNG sequence, and a
whole-run lockstep comparison against the original will then diverge. M1 has two options:
1. Charge the original routine's cycle count when a hook runs (preferred). Body cycle counts are given below.
2. Verify per call only: snapshot the inputs, run the original and the Rust version, and compare the outputs.

M1b: each Rust routine declares its exact cost per call (per path), and Rust mode runs it only when no
timing event (NMI edge, H/V IRQ, HDMA) can land within its maximum cost (`cycles`); otherwise the original
runs. The NMI usually lands inside an RNG call of the idle loop, so those calls stay original, and the idle
loop itself stays original until the whole frame task is Rust (spec §5.3 order).
All addresses below $2000 used here are low-WRAM mirrors, so DBR only matters if it is ever $7F or
$40-$7D/$C0-$FF. It never is in the traces.

---

## 1. $008B0E `advance_x_by_6`: X += 6 (record-stride step)
- Contract: entry `100` (M8/X16) only. Input X. Output X+6 (wraps 16-bit; would wrap 8-bit if X8, never
  seen). Flags N/Z from the final X; C/V untouched. A, Y and memory untouched. RTL. DBR is irrelevant.
- Callers: 14 static, 9 seen. 039189/039732 do `CPX #$C0` right after, and 039530/03957C do `INX; CPX #$70`.
  So no caller consumes the routine's flags.
- Purity: 6×INX + RTL, no memory access at all.
- Frequency: 6,676,922 calls (about 19.5 per frame), mostly 039189 (4.28M) and 039530 (2.14M), the bank-03
  object loops.
- Cost: 18 CPU cycles (6×2 + RTL 6).
- Why first: zero memory side effects. It is the ideal smoke test for the hook, compare and return path (RTL stack pop).
- Siblings with the same shape, not renamed: $008B07 Y+=6 (9.6k), $008AF1 Y+=$20 (20.2k, 27 callers),
  $008AFC X+=$20 (6k). The last two do `REP #$20 … SEP #$20`, so they exit with M=8 and N/Z/C/V from the 16-bit ADC.
- **Status (M1a):** Rust (`ssd-game`), verified against the original: 63,149 calls checked, 0 divergences,
  in the 7,200-frame play-through of `crates/ssd-game/tests/verify_real_rom.rs`. Cost 128 master cycles.

## 2. $05B6B2 `get_active_side_record`: Y = base of the active battle-side record
- Contract: entry `100`. Reads WRAM $14C2 (absolute, so DBR-relative; DBR=00 observed).
  Output Y = $1400 if ($14C2 & 1) == 0, else $1440. A low byte = $14C2 & 1, B unchanged. Flags: N=0,
  Z = !(bit 0). The LDY #$1440 on the odd path also leaves Z=0. C/V untouched. RTS (bank 05 only).
- Callers: 56 seen, all JSR from bank 05. 05CAB3/05DED0 do `CPY $20`, 05D521 does `LDA #$80; CPY $20`, and
  05FD19 does `LDX #$22; LDA $12,Y`. Y is consumed; A and flags are not (A is overwritten), but replicate them anyway.
- Purity: one WRAM read, no calls, no MMIO.
- Frequency: 115,691 calls. Top callers: 05CAB3 (57.8k), 05D521 (22.4k), 05FD19 (14.5k), 05DED0 (12.2k).
- Cost: 18 cycles (even path) or 20 (odd path), so it is data-dependent.
- **Status (M1a):** Rust, verified: 3,877 calls checked (battle), 0 divergences.
  Cost 136 master cycles for side 0, 154 for side 1 (`cycles` = 154), declared per call.

## 3. $05B572 `lookup_id_category`: A = ROM[$0D8200 + (A & $7F)] & $7F
- Contract: entry `100`. Input A low byte; bit 7 is ignored. Reads the ROM table at **$0D:8200, 128 bytes**
  through long-indexed addressing, so DBR is irrelevant. Output: A low = entry & $7F, B = 0 (the `XBA; LDA #0;
  XBA` sequence clears B before TAX). X is preserved through PHX/PLX, so the stack is balanced. Flags N/Z come
  from **PLX**, i.e. from the restored X, not from A. C/V untouched. RTS.
  A JSL wrapper at $05B56E (`JSR $B572; RTL`, 49,742 calls from 05C38A) is caught by the same hook.
- Callers: 19 static, 13 seen. 05C38E `CMP #$0D`, 05C1CD `CMP #$00`, 05A253 `CMP $00` and 058617 `CMP #$09`
  all compare A immediately, so the result value is consumed but the exit flags are not.
- Purity: one ROM read, no WRAM writes except the stack, no MMIO.
- Frequency: 51,043 calls (49,742 through the wrapper during battle).
- Cost: 34 cycles.
- Note: the input looks like a character/card id from $0B01,Y, $1401 or $20,Y, and the output a category
  (callers test $09/$0D/$0F/$34). The name is provisional.
- **Status (M1a):** Rust, verified: 2,735 calls checked (battle), 0 divergences. Cost 250 master cycles.

## 4. $0082A3 `rng_next_a` / $0082B5 `rng_next_b`: 16-bit LCG s = s*5 + $11
These are two identical routines on different seeds: A uses WRAM $2F-$30 and B uses $31-$32 (DP, D=0).
- Contract: entry `100`. rng_next_a was also seen with `110` (X8, from 00ABE2); X is unused, so this is
  harmless. The routine does `REP #$20` and loads s (16-bit), then s' = (s<<2) + s + $0011 (mod $10000),
  stores s' and does `SEP #$20` before RTL.
  - Outputs: seed word = s'; **A (16-bit C register) = s'**, so the low byte is the "random" value and B
    holds the high byte. M=1 on exit; the entry M is ignored because REP forces M16 inside.
  - Flags: N, V, Z and **C** come from the final 16-bit `ADC #$0011`, where the carry-in is 0 after CLC.
    The intermediate ADC's carry is discarded by CLC. X/Y untouched. DBR is irrelevant (DP only).
- **Callers DO consume C:** 01C27C runs `JSL rng_next_a; LDA $51; SBC $2F`, and that SBC uses the carry left by
  the routine. 1D880C/1D8818 do `AND #$03`/`AND #$07` on A directly, and 00ABE2 does `STA $65,X` with
  the A low byte. 01C258/0594EE do `LDA #0; XBA; LDA $2F`, re-reading the seed.
  So the Rust version must return A (all 16 bits) and the exact N/V/Z/C.
- Purity: DP read/modify/write only, no calls, no MMIO.
- Frequency: A has 59,149,165 calls (13 callers; 99.99% from the idle loop at 0080CE). B has 118,294,865 calls
  (7 callers; 0080D2 and 0080D6 in the idle loop). In-game consumers: 01C257 (enemy/encounter
  table pick), 0594A1, 1D8807, 00ABB5, 05B20E/05B2B9, plus 0082C7.
- Cost: 35 CPU cycles each, plus 8 for the JSL. See the global caveat: these are the routines that turn
  any cycle drift into gameplay divergence.
- Follow-up candidate, not renamed: $0082C7 (127 calls) loops `rng_next_a` until ($2F & $7F) < $65,
  which gives a 0..100 roll. It is pure and its only callee is pure.
- **Status (M1a):** both Rust, verified one call in 64: A 9,453 of 605,293 calls, B 18,890 of 1,210,259,
  0 divergences. Cost 264 master cycles each.

## 5. $01F660 `entity_id_in_special_set`: C = ($0E01+Y) ∈ {$10,$11,$4C,$4D,$4F}
- Contract: entry `100`. Input Y = entity slot offset (stride $20; callers pass 0/$20/$40/$60). Reads
  WRAM $0E01+Y (absolute, so DBR-relative; DBR=00 observed). Output: A low = that byte, B unchanged.
  - On a match: C=1 and Z=1 (from the equal CMP), N=0.
  - On no match: C=0 (from an explicit CLC), and N/Z from `CMP #$4F`.
  - X/Y and memory untouched. RTL.
- Callers: 4 static, 3 seen. 01F95E does `BCS` (then `INC $12`, a sprite variant). 01F55A does `BCC`, then
  `STA $0D01`, so **A is consumed**. 01FBC6 and 01F9BB (unexecuted) both do `BCS`.
- Purity: one WRAM read, CMP chain, no calls, no MMIO.
- Frequency: 64,751 calls, 64,626 of them from 01F95E (the per-frame entity draw loop at 01F91F).
- **Coverage gap:** the true path (SEC at $01F679) has **0 hits** in all 8 runs. The Rust version needs
  synthetic tests for the five ids, and the verification only covers the false path until story progress
  reaches those ids.
- **Status (M1a):** Rust, verified: 299 false-path calls checked, 0 divergences. The true path is checked by
  setting the id to each of the five members at real calls (the original then stops at that member).
  Cost by the member that ends the comparisons: 134, 166, 198, 230, 262 master cycles for $10, $11, $4C,
  $4D, $4F, and 256 for any other id (`cycles` = 262), declared per call.

## 6. $06E969 `bin24_to_packed_bcd`: 24-bit binary to packed BCD (repeated subtraction)
One routine with three more **suffix entries**, each falling through to the same RTL at $06EA26:
`bin_to_bcd_from_thousands` $06E9DB, `bin_to_bcd_from_hundreds` $06E9F7 and `bin_to_bcd_from_tens` $06EA0E.
- Logic: `SEP #$20` (M8 forced), zero $03-$06, then work on V = $02:$01:$00 in six stages:
  1. While V ≥ 1,000,000: subtract, INC $06 (the millions are a binary count).
  2. While V ≥ 100,000: subtract, $05 += $10.
  3. While V ≥ 10,000: subtract, INC $05.
  4. [$06E9DB] Using only $01:$00: while ≥ 1000, subtract and $04 += $10.
  5. [$06E9F7] While ≥ 100: subtract, INC $04.
  6. [$06EA0E] Using only $00: while ≥ 10, $00 -= 10 and $03 += $10.
  Finally $03 |= $00 and RTL.
  Each `+= $10` stage loops with BNE, so it **also exits if its digit byte wraps to 0** (after 16 steps).
  Out-of-range direct entries reproduce that quirk: for example, entering at 06EA0E with $00 ≥ 160 gives
  garbage. Do not use native /10 or %10 without replicating it.
- Contract: entry `100`; the suffix entries assume M8, which callers set with SEP. Inputs: $00-$02 (the
  24-bit entry) or $00-$01 / $00 (suffix entries). Callers of the suffix entries pre-zero $03/$04
  themselves (`STZ $03` in 16-bit mode, or STZ $03/$04).
  - Outputs: $03 = tens|ones, $04 = thousands|hundreds, $05 = 100k|10k, $06 = millions.
  - $00-$02 end as the remainder. Scratch $07 is written, and $08 too on the 24-bit stages.
  - A = final $03. N/Z come from the ORA. C = 0 (last SBC borrowed), or 1 on the wrap exit. V comes from the last SBC.
  - X/Y untouched. DBR is irrelevant (DP only).
- Callers: 06E969 has 7 static and 7 seen (01AC35, 01C043, 1D84B0, …). 06E9DB has 4 seen (01AA72, 01AA9C,
  01BFBA, 01A9B4). 06E9F7 has 01CAFD and 06EA0E has 01BF9C. Every one is immediately followed by
  `JSL $06EA27/$06EA7D/$06EAA8`, a digit-to-tile writer that reads $03-$06 and starts with STZ, so **no
  caller consumes the exit flags**.
- Purity: DP only, no calls, no MMIO. The cycle count is data-dependent (loop counts).
- Frequency: 11,825 RTLs in total. Of those, 2,721 enter at 06E969, 7,860 directly at 06E9DB, 534 at 06E9F7
  and 710 at 06EA0E. This is HUD/status number display.
- Hooking: a PC hook that implements "from this PC to the RTL" is correct at every entry. It stays correct
  even when the PC is reached by fall-through or by the BNE loop-back to 06E9DB/06E9F7/06EA0E, because each
  suffix depends only on the DP state. Hook all four entries, or only 06E969 and leave the suffixes on the
  interpreter.
- **Status (M1a):** Rust at $06E969 only, verified: 29 calls checked, 0 divergences; flags not compared.
  The three suffix entries stay on the original. Cost 1202 master cycles plus, per subtraction that goes
  through, 378 / 418 / 378 / 306 / 266 / 194 for stages 1-6 (a closed formula of the loop counts); worst
  case 20,930 at 15,999,990 (`cycles`), declared per call.

---

## Rejected or deferred (pure but not first)
- $0085F5 / $00849C (392k / 221k calls): metasprite to shadow-OAM ($0400/$0600) writers. They are pure WRAM,
  but 96-130 instructions, and $00849C is also entered with M16 (`000`). Better as M1 phase-2.
- $0086A0 (291k): fills OAM Y with $F0 from X=$55 to $200. Pure, but its loop length is data-dependent.
- $009C6F (379k, JSR-only from $009CDE): pointer-table fetch from $02:DA16 into $00-$05, with side
  effects on $02/$05/Y. Simple, but it has only one caller chain.
- $0083CC: contains `JMP ($83DC,X)`, an indirect dispatch.
- No software multiply/divide was found among the executed pure leaves; the game uses the hardware
  $4202/$4204 units (MMIO, excluded).

## Ghidra changes (saved)
Renamed, each with an "M1 candidate: …" plate: 008B0E advance_x_by_6, 05B6B2 get_active_side_record,
05B572 lookup_id_category, 0082A3 rng_next_a, 0082B5 rng_next_b, 01F660 entity_id_in_special_set,
06E969 bin24_to_packed_bcd, 06E9DB bin_to_bcd_from_thousands, 06E9F7 bin_to_bcd_from_hundreds,
06EA0E bin_to_bcd_from_tens.
