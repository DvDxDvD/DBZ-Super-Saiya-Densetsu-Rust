//! Batch C of the M2 porting batch: small routines, one section per routine in batch order.
//!
//! Each section is written from the routine's own behaviour (`docs/re/routines/`). Addresses come
//! from the generated `data` constants (`re/symbols.toml`); direct-page scratch bytes stay implicit.
//! A path a reference replay never takes runs the original code (`Ctx::call_original` from the
//! entry, before anything is written), so only the paths the replays measure are declared here.

use crate::data;
use ssd_oracle::{Ctx, ReturnKind};

// ---- copy_source_block: 16 words of a ROM block (bank $0C) into WRAM at `copy_dest` ----

/// Entry of the routine (`$06:EBC1`), for the hand-back to the original code.
const COPY_ENTRY: u32 = 0x06_EBC1;
/// Gate byte (`copy_gate`): bit 7 set skips the copy.
const COPY_GATE: u16 = (data::COPY_GATE & 0xFFFF) as u16;
/// Selector byte (`copy_select`): bit 7 set skips the copy, bit 0 offsets the source by `$100`.
const COPY_SELECT: u16 = (data::COPY_SELECT & 0xFFFF) as u16;
/// Destination of the copy (`copy_dest`), 32 bytes from `$0280`.
const COPY_DEST: u16 = (data::COPY_DEST & 0xFFFF) as u16;
/// Flag byte set to 1 after a copy (`copy_done`).
const COPY_DONE: u16 = (data::COPY_DONE & 0xFFFF) as u16;
/// Four 16-bit source offsets in ROM (`copy_source_offsets`), chosen by bits 3 and 4 of `$4F`.
const COPY_SOURCE_OFFSETS: u32 = data::COPY_SOURCE_OFFSETS;
/// Bank of the source block: the bank byte of the pointer the routine builds at `$02`.
const COPY_SOURCE_BANK: u32 = 0x0C;
/// Length of the copy in bytes (16 words).
const COPY_LEN: u16 = 0x20;
/// CPU-busy master cycles of the two measured routes, from the hit to the return (`Verify`): the
/// low bits of `$4F` set, and the copy.
const COST_LOW_BITS: u64 = 190;
const COST_COPY: u64 = 3490;

/// Index into the source offsets, from the byte at `$4F`: its bits 3 and 4, doubled.
pub(crate) fn copy_index(f: u8) -> u16 {
    u16::from((f >> 2) & 0x06)
}

/// Whether a call takes a path the reference replays never make, and so goes back to the
/// original code: the gate is set, or the low three bits of `$4F` are clear and the selector has
/// bit 7 or bit 0 set.
pub(crate) fn leaves_to_original(gate: u8, f: u8, select: u8) -> bool {
    gate & 0x80 != 0 || ((f & 0x07) == 0 && select & 0x81 != 0)
}

/// `copy_source_block`. Two paths run in Rust, with costs measured in `Verify`: the low bits of
/// `$4F` set (A = those bits, B clear, N and Z from them; `COST_LOW_BITS`), and otherwise the
/// 16-word copy from bank `$0C` at the table offset `copy_index` picks, with `$4F` = 0
/// (`COST_COPY`). The copy leaves N = 0, Z = 0, C = 1 and V = 0; A ends with the last word's high
/// byte and `$01`.
///
/// A gate set, or a selector with bit 7 or bit 0 set on the copy path, runs the original code
/// (`call_original` from the entry: nothing has changed by then, and its own cost is 0).
pub(crate) fn copy_source_block(ctx: &mut Ctx<'_>) {
    let gate = ctx.read_db8(COPY_GATE);
    let f = ctx.read_dp8(0x4F);
    let select = ctx.read_db8(COPY_SELECT);
    if leaves_to_original(gate, f, select) {
        let Ok(()) = ctx.call_original(COPY_ENTRY, ReturnKind::Rtl) else { return };
        ctx.set_cost(0);
        return;
    }
    if f & 0x07 != 0 {
        let low = f & 0x07;
        ctx.set_a(u16::from(low));
        ctx.set_nz8(low);
        ctx.set_cost(COST_LOW_BITS);
        return;
    }
    let index = copy_index(f);
    ctx.write_dp8(0x02, COPY_SOURCE_BANK as u8);
    ctx.write_dp8(0x00, 0);
    ctx.write_dp8(0x01, 0);
    ctx.set_x(index);
    let offset = ctx.read16(COPY_SOURCE_OFFSETS + u32::from(index));
    ctx.write_dp16(0x00, offset);
    let mut word = 0;
    for y in (0..COPY_LEN).step_by(2) {
        word = ctx.read16((COPY_SOURCE_BANK << 16) + u32::from(offset) + u32::from(y));
        ctx.write_db16(COPY_DEST + y, word);
    }
    ctx.set_y(COPY_LEN);
    ctx.set_a((word & 0xFF00) | 0x0001);
    ctx.write_db8(COPY_DONE, 0x01);
    ctx.set_n(false);
    ctx.set_z(false);
    ctx.set_c(true);
    ctx.set_v(false);
    ctx.set_cost(COST_COPY);
}

#[cfg(test)]
mod tests_copy {
    use super::{copy_index, leaves_to_original};

    #[test]
    fn index_takes_bits_3_and_4_doubled() {
        assert_eq!(copy_index(0x00), 0);
        assert_eq!(copy_index(0x08), 2);
        assert_eq!(copy_index(0x10), 4);
        assert_eq!(copy_index(0x18), 6);
        assert_eq!(copy_index(0xE7), 0, "bits 3 and 4 are clear; the others do not count");
        assert_eq!(copy_index(0xEF), 2, "bit 3 alone gives 2");
        assert_eq!(copy_index(0x20), 0);
    }

    #[test]
    fn the_paths_the_replays_never_take_go_back_to_the_original() {
        // The gate, or a selector with bit 7 or bit 0 on the copy path.
        assert!(leaves_to_original(0x80, 0x00, 0x00));
        assert!(leaves_to_original(0x80, 0x07, 0x00), "the gate comes first");
        assert!(leaves_to_original(0x00, 0x00, 0x80));
        assert!(leaves_to_original(0x00, 0x00, 0x01));
        assert!(leaves_to_original(0x00, 0x08, 0x81), "copy path: $4F low bits clear");
        // The paths the replays take stay in Rust; the low bits of $4F skip the selector.
        assert!(!leaves_to_original(0x00, 0x05, 0xFF), "low bits set: the selector is not read");
        assert!(!leaves_to_original(0x00, 0x00, 0x00));
        assert!(!leaves_to_original(0x00, 0x00, 0x02), "other selector bits are not offsets");
        assert!(!leaves_to_original(0x00, 0x18, 0x40));
    }
}

// ---- clear_map_menu_block: 48 zero words at the map menu cursor ----

/// The map menu's buffer starts at `map_menu_cursor` (a one-byte variable of the symbols file; the
/// loader refuses an overlapping name, so the block is addressed from that name).
const MAP_MENU_BASE: u16 = (data::MAP_MENU_CURSOR & 0xFFFF) as u16;
/// The clear covers offsets `0..$60`, in words.
const CLEAR_END: u16 = 0x0060;
/// CPU-busy master cycles of the one route, from the hit to the return (`Verify`).
const COST_CLEAR: u64 = 5880;

/// The offsets the clearing loop stores at, from zero up: every second byte of `0..$60`.
pub(crate) fn clear_indexes() -> impl Iterator<Item = u16> {
    (0..CLEAR_END).step_by(2)
}

/// `clear_map_menu_block`: 96 bytes from `map_menu_cursor` become zero (48 word stores), X ends at
/// `$60`, A is zero (B too), N is clear, Z and C are set by the final compare; V is untouched.
pub(crate) fn clear_map_menu_block(ctx: &mut Ctx<'_>) {
    for x in clear_indexes() {
        ctx.write_db16(MAP_MENU_BASE + x, 0);
    }
    ctx.set_x(CLEAR_END);
    ctx.set_a(0);
    ctx.set_n(false);
    ctx.set_z(true);
    ctx.set_c(true);
    ctx.set_cost(COST_CLEAR);
}

#[cfg(test)]
mod tests_clear {
    use super::{CLEAR_END, clear_indexes};
    use crate::data;

    #[test]
    fn the_clear_stores_48_words_over_96_bytes() {
        let offsets: Vec<u16> = clear_indexes().collect();
        assert_eq!(offsets.len(), 48);
        assert_eq!(offsets.first(), Some(&0));
        assert_eq!(offsets.last(), Some(&(CLEAR_END - 2)));
        assert_eq!(data::MAP_MENU_CURSOR & 0xFFFF, 0x1100);
    }
}

// ---- fill_word_block: 320 copies of one word over a block of bank $7E ----

/// The long base of the indexed store (`STA $7E3000,X`); the indexes run over the block.
const FILL_BASE: u32 = 0x7E_3000;
/// The word stored at every index.
const FILL_PATTERN: u16 = 0x3D40;
/// Highest index (`$07FE`) and lowest index (`$0580`) of the loop.
const FILL_TOP: u16 = 0x07FE;
const FILL_LOWEST: u16 = 0x0580;
/// X after the loop: the last index minus two, the value the final decrement leaves.
const FILL_EXIT_X: u16 = FILL_LOWEST - 2;
/// CPU-busy master cycles of the one route, from the hit to the return (`Verify`).
const COST_FILL: u64 = 39170;

/// The indexes the loop stores at, top first, down to `FILL_LOWEST` in steps of two.
pub(crate) fn fill_indexes() -> impl Iterator<Item = u16> {
    (FILL_LOWEST..=FILL_TOP).rev().step_by(2)
}

/// `fill_word_block`: stores the word `$3D40` at every index of [`fill_indexes`] (the block is
/// `fill_block`, `$7E:3580` to `$7E:37FF`). A keeps the word, X ends at `$057E`, N is set and Z
/// and C are clear from the final compare (`$057E` against `$0580`). V is untouched.
pub(crate) fn fill_word_block(ctx: &mut Ctx<'_>) {
    for x in fill_indexes() {
        ctx.write16(FILL_BASE + u32::from(x), FILL_PATTERN);
    }
    ctx.set_x(FILL_EXIT_X);
    ctx.set_a(FILL_PATTERN);
    ctx.set_n(true);
    ctx.set_z(false);
    ctx.set_c(false);
    ctx.set_cost(COST_FILL);
}

#[cfg(test)]
mod tests_fill {
    use super::{FILL_EXIT_X, FILL_LOWEST, FILL_TOP, fill_indexes};
    use crate::data;

    #[test]
    fn the_fill_stores_320_words_from_the_top_down() {
        let offsets: Vec<u16> = fill_indexes().collect();
        assert_eq!(offsets.len(), 320);
        assert_eq!(offsets.first(), Some(&FILL_TOP));
        assert_eq!(offsets.last(), Some(&FILL_LOWEST));
        assert_eq!(FILL_EXIT_X, 0x057E);
        assert_eq!(0x7E_3000 + u32::from(FILL_LOWEST), data::FILL_BLOCK);
        assert_eq!(0x7E_3000 + u32::from(FILL_TOP) + 2, data::FILL_BLOCK + data::FILL_BLOCK_SIZE);
    }
}

// ---- lookup_scaled_entry: six times a table byte, indexed by four times A's low seven bits ----

/// The 512-byte table (`scale_table`), read through its low byte at `4 * v`.
const SCALE_TABLE: u32 = data::SCALE_TABLE;
/// CPU-busy master cycles of the one route, from the hit to the return (`Verify`).
const COST_SCALE: u64 = 454;

/// The table index for the seven-bit value `v`: `4 * v`, cut to 8 bits when the index register is
/// 8-bit (the transfer keeps the low byte only).
pub(crate) fn scale_index(v: u8, x8: bool) -> u16 {
    let four = u16::from(v & 0x7F) << 2;
    if x8 { four & 0x00FF } else { four }
}

/// `lookup_scaled_entry`: from A's low seven bits `v`, reads the table byte at the index of
/// [`scale_index`], stores `4 * v` at `$12` and twice that byte at `$00` (16-bit stores), and
/// returns `6 * byte` in A and in X (X keeps the low byte with an 8-bit index). N and Z come from
/// that value (its low byte with an 8-bit index), C and V are clear. B ends as the high byte of
/// the result.
pub(crate) fn lookup_scaled_entry(ctx: &mut Ctx<'_>) {
    let v = ctx.al() & 0x7F;
    let four = u16::from(v) << 2;
    let x8 = ctx.x8();
    ctx.write_dp16(0x12, four);
    let t0 = ctx.read8(SCALE_TABLE + u32::from(scale_index(v, x8)));
    let two = u16::from(t0) << 1;
    ctx.write_dp16(0x00, two);
    let six = (u16::from(t0) << 2).wrapping_add(two);
    ctx.set_a(six);
    ctx.set_x(six);
    if x8 {
        ctx.set_nz8(six as u8);
    } else {
        ctx.set_nz16(six);
    }
    ctx.set_c(false);
    ctx.set_v(false);
    ctx.set_cost(COST_SCALE);
}

#[cfg(test)]
mod tests_scale {
    use super::scale_index;
    use crate::data;

    #[test]
    fn the_scale_index_is_four_times_the_seven_bit_value() {
        assert_eq!(scale_index(0x00, false), 0);
        assert_eq!(scale_index(0x01, false), 4);
        assert_eq!(scale_index(0x7F, false), 508);
        assert_eq!(scale_index(0xFF, false), 508, "bit 7 is ignored");
        assert_eq!(scale_index(0x40, true), 0x00, "8-bit index keeps the low byte");
        assert_eq!(scale_index(0x7F, true), 0xFC);
        assert_eq!(data::SCALE_TABLE & 0xFFFF, 0xCB09);
    }
}

// ---- store_map_menu_entry: one byte into the map menu block, at an index from `$0D66` ----

/// The index byte (`map_entry_index`) and the value byte (`map_entry_value`), both in WRAM.
const MAP_ENTRY_INDEX: u16 = (data::MAP_ENTRY_INDEX & 0xFFFF) as u16;
const MAP_ENTRY_VALUE: u16 = (data::MAP_ENTRY_VALUE & 0xFFFF) as u16;
/// CPU-busy master cycles of the one route, from the hit to the return (`Verify`).
const COST_STORE: u64 = 196;

/// `store_map_menu_entry`: `map_menu_cursor + index` := value, with the index also in Y. A holds
/// the value and B is clear; N and Z come from the value; C and V are kept.
pub(crate) fn store_map_menu_entry(ctx: &mut Ctx<'_>) {
    let index = ctx.read_db8(MAP_ENTRY_INDEX);
    let value = ctx.read_db8(MAP_ENTRY_VALUE);
    ctx.set_y(u16::from(index));
    ctx.write_db8(MAP_MENU_BASE + u16::from(index), value);
    ctx.set_a(u16::from(value));
    ctx.set_nz8(value);
    ctx.set_cost(COST_STORE);
}

#[cfg(test)]
mod tests_store {
    use super::MAP_MENU_BASE;

    #[test]
    fn the_indexes_stay_inside_the_map_block() {
        // The index is a byte: the last entry is the block's offset `$FF`.
        assert_eq!(MAP_MENU_BASE + 0xFF, 0x11FF);
    }
}

// ---- arithmetic of the steps below (the CPU's ADC and SBC) ----

/// 8-bit `a + b + carry`: the result, C and V.
pub(crate) fn adc8(a: u8, b: u8, carry: bool) -> (u8, bool, bool) {
    let sum = u16::from(a) + u16::from(b) + u16::from(carry);
    let result = sum as u8;
    let overflow = (a ^ result) & (b ^ result) & 0x80 != 0;
    (result, sum > 0xFF, overflow)
}

/// 8-bit `a - b` with the carry as the CPU's SBC takes it (set: no borrow in): the result, C (no
/// borrow out) and V.
pub(crate) fn sbc8(a: u8, b: u8, carry: bool) -> (u8, bool, bool) {
    adc8(a, !b, carry)
}

/// 16-bit `a + b` with carry in clear: the result, C and V.
pub(crate) fn add16(a: u16, b: u16) -> (u16, bool, bool) {
    let (result, carry) = a.overflowing_add(b);
    (result, carry, (a ^ result) & (b ^ result) & 0x8000 != 0)
}

/// 16-bit `a - b` with carry in set: the result, C (no borrow) and V.
pub(crate) fn sub16(a: u16, b: u16) -> (u16, bool, bool) {
    let result = a.wrapping_sub(b);
    (result, a >= b, (a ^ b) & (a ^ result) & 0x8000 != 0)
}

// ---- follow_target_step: moves the 16-bit position at `$35` a few steps towards the target ----

/// Entry of the routine (`$06:EE42`), for the hand-back to the original code.
const FOLLOW_ENTRY: u32 = 0x06_EE42;
/// The target (`follow_target`, `$01A4-$01A5`), read through the data bank.
const FOLLOW_TARGET: u16 = (data::FOLLOW_TARGET & 0xFFFF) as u16;
/// The position (`follow_pos`, direct page `$35-$36`).
const FOLLOW_POS: u16 = (data::FOLLOW_POS & 0xFFFF) as u16;
/// The limit byte (`follow_limit`, `$0702`) and the flags byte (`follow_flags`, `$0700`).
const FOLLOW_LIMIT: u16 = (data::FOLLOW_LIMIT & 0xFFFF) as u16;
const FOLLOW_FLAGS: u16 = (data::FOLLOW_FLAGS & 0xFFFF) as u16;

/// CPU-busy master cycles of the routes the replays take, from the hit to the return (`Verify`).
/// A rest (the difference's low byte is `$78`); a step down of one to three (`DOWN`); a step up
/// of one to three (`UP`) or of four (`CLAMP`), with the two limit values equal (`EQ`) or not
/// (`NE`); and whether the flags byte is cleared (`CLEAR`) or kept (`KEEP`) at the end.
const COST_REST: u64 = 314;
const COST_DOWN_CLEAR: u64 = 988;
const COST_DOWN_KEEP: u64 = 924;
const COST_UP_NE_CLEAR: u64 = 972;
const COST_UP_NE_KEEP: u64 = 898;
const COST_UP_EQ_CLEAR: u64 = 1006;
const COST_CLAMP_EQ_CLEAR: u64 = 1016;

/// What the routine does on one route: the value at `$00`, the new position (the bytes at `$10`,
/// `$11`, `$35`, `$36`) if it moves, the flags byte the end section stores, A's low byte, N, Z, C,
/// V and the cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FollowPlan {
    pub dp00: u8,
    pub new_pos: Option<(u8, u8)>,
    pub flags_store: Option<u8>,
    pub a: u8,
    pub n: bool,
    pub z: bool,
    pub c: bool,
    pub v: bool,
    pub cost: u64,
}

/// The end section of the routine: `f = a_in | flags`, stored with bits 2 and 3 cleared when bit 2
/// of the new position's low byte and bit 2 of `f` agree, else kept. Returns the stored byte, A's
/// low byte, N, Z, and whether the bits were cleared.
pub(crate) fn follow_settle(a_in: u8, flags: u8, new_lo: u8) -> (u8, u8, bool, bool, bool) {
    let f = a_in | flags;
    let pos_bit = new_lo & 0x04 != 0;
    let flag_bit = f & 0x04 != 0;
    if pos_bit == flag_bit {
        let cleared = f & 0xF3;
        (cleared, cleared, cleared & 0x80 != 0, cleared == 0, true)
    } else {
        // The last AND leaves A = bit 2 of `f`: N clear, Z clear exactly when that bit is set.
        (f, f & 0x04, false, !flag_bit, false)
    }
}

/// The plan of one call from its inputs (target, position, limit byte, flags byte), or `None` when
/// the call takes a route the replays never take: the original code runs it instead. The routes:
/// the difference's high byte is zero, and then the low byte is `$78` (a rest), below `$78` by one
/// to three (a step down, `step = $78 - low`), or above `$78` (a step up, `amount = min(low - $78,
/// 4)`, which the limit compare may turn into a clear).
pub(crate) fn follow_plan(target: u16, pos: u16, limit: u8, flags: u8) -> Option<FollowPlan> {
    let (low, low_carry, _) = sbc8(target as u8, pos as u8, true);
    let (high, _, high_overflow) = sbc8((target >> 8) as u8, (pos >> 8) as u8, low_carry);
    if high != 0 {
        // A high difference that is negative or positive: not taken by the replays.
        return None;
    }
    if low == 0x78 {
        return Some(FollowPlan {
            dp00: low,
            new_pos: None,
            flags_store: None,
            a: low,
            n: false,
            z: true,
            c: true,
            v: high_overflow,
            cost: COST_REST,
        });
    }
    if low < 0x78 {
        let step = 0x78 - low;
        if step >= 4 {
            return None;
        }
        let (new_lo, lo_carry, _) = sbc8(pos as u8, step, true);
        let (new_hi, no_borrow, hi_overflow) = sbc8((pos >> 8) as u8, 0, lo_carry);
        if !no_borrow {
            // The position would go below zero: clamped, not taken by the replays.
            return None;
        }
        let (store, a_out, n_out, z_out, cleared) = follow_settle(0x0C, flags, new_lo);
        let cost = if cleared { COST_DOWN_CLEAR } else { COST_DOWN_KEEP };
        return Some(FollowPlan {
            dp00: step,
            new_pos: Some((new_lo, new_hi)),
            flags_store: Some(store),
            a: a_out,
            n: n_out,
            z: z_out,
            c: no_borrow,
            v: hi_overflow,
            cost,
        });
    }
    let above = low - 0x78;
    let amount = if above < 4 { above } else { 4 };
    let (sum_lo, sum_carry, _) = adc8(amount, pos as u8, false);
    let (sum_hi, _, _) = adc8(0, (pos >> 8) as u8, sum_carry);
    let (limit_less_one, _, limit_overflow) = sbc8(limit, 1, true);
    let equal = limit_less_one == sum_hi;
    let carry_out = limit_less_one >= sum_hi;
    let new_lo = if equal { 0 } else { sum_lo };
    let (store, a_out, n_out, z_out, cleared) = follow_settle(0x08, flags, new_lo);
    let cost = match (above < 4, equal, cleared) {
        (true, false, true) => COST_UP_NE_CLEAR,
        (true, false, false) => COST_UP_NE_KEEP,
        (true, true, true) => COST_UP_EQ_CLEAR,
        (false, true, true) => COST_CLAMP_EQ_CLEAR,
        _ => return None,
    };
    Some(FollowPlan {
        dp00: low,
        new_pos: Some((new_lo, sum_hi)),
        flags_store: Some(store),
        a: a_out,
        n: n_out,
        z: z_out,
        c: carry_out,
        v: limit_overflow,
        cost,
    })
}

/// `follow_target_step`: applies [`follow_plan`] to the machine. Any other route runs the original
/// code from the entry, before anything is written.
pub(crate) fn follow_target_step(ctx: &mut Ctx<'_>) {
    let target = ctx.read_db16(FOLLOW_TARGET);
    let pos = ctx.read_dp16(FOLLOW_POS);
    let limit = ctx.read_db8(FOLLOW_LIMIT);
    let flags = ctx.read_db8(FOLLOW_FLAGS);
    let Some(plan) = follow_plan(target, pos, limit, flags) else {
        let Ok(()) = ctx.call_original(FOLLOW_ENTRY, ReturnKind::Rtl) else { return };
        ctx.set_cost(0);
        return;
    };
    ctx.write_dp8(0x00, plan.dp00);
    if let Some((lo, hi)) = plan.new_pos {
        ctx.write_dp8(0x10, lo);
        ctx.write_dp8(0x11, hi);
        ctx.write_dp8(0x35, lo);
        ctx.write_dp8(0x36, hi);
    }
    if let Some(store) = plan.flags_store {
        ctx.write_db8(FOLLOW_FLAGS, store);
    }
    ctx.set_al(plan.a);
    ctx.set_n(plan.n);
    ctx.set_z(plan.z);
    ctx.set_c(plan.c);
    ctx.set_v(plan.v);
    ctx.set_cost(plan.cost);
}

#[cfg(test)]
mod tests_follow {
    use super::{COST_CLAMP_EQ_CLEAR, COST_REST, COST_UP_NE_CLEAR, follow_plan};

    /// The replayed call with target `$0A80`, position `$0A00`, limit `$0B`, flags `$00`.
    #[test]
    fn a_clamped_step_with_equal_limit_compare() {
        let plan = follow_plan(0x0A80, 0x0A00, 0x0B, 0x00).unwrap();
        assert_eq!(plan.cost, COST_CLAMP_EQ_CLEAR);
        assert_eq!(plan.new_pos, Some((0x00, 0x0A)));
        assert_eq!((plan.a, plan.n, plan.z, plan.c, plan.v), (0x00, false, true, true, false));
        assert_eq!(plan.flags_store, Some(0x00));
    }

    #[test]
    fn the_low_byte_of_78_rests() {
        let plan = follow_plan(0x0A78, 0x0A00, 0x0B, 0xC0).unwrap();
        assert_eq!(plan.cost, COST_REST);
        assert_eq!(plan.new_pos, None);
        assert_eq!((plan.a, plan.n, plan.z, plan.c), (0x78, false, true, true));
    }

    #[test]
    fn a_step_up_below_four_with_unequal_limit_compare() {
        // Low difference $79: a step of one, and the limit minus one (`$0A`) is not `$09`.
        let plan = follow_plan(0x0A6C, 0x09F3, 0x0B, 0x80).unwrap();
        assert_eq!(plan.cost, 898, "keeps the flags byte");
        assert_eq!(plan.new_pos, Some((0xF4, 0x09)));
        assert_eq!(plan.flags_store, Some(0x88));
        let cleared = follow_plan(0x0A70, 0x09F7, 0x0B, 0x80).unwrap();
        assert_eq!(cleared.cost, COST_UP_NE_CLEAR);
        assert_eq!(cleared.flags_store, Some(0x80));
    }

    #[test]
    fn routes_the_replays_never_take_go_back_to_the_original() {
        assert_eq!(follow_plan(0x0A00, 0x0C00, 0x0B, 0x00), None, "high part negative");
        assert_eq!(follow_plan(0x1A00, 0x0A00, 0x0B, 0x00), None, "high part positive");
        assert_eq!(follow_plan(0x0A70, 0x0A00, 0x0B, 0x00), None, "step down of four or more");
        assert_eq!(follow_plan(0x0A80, 0x0A00, 0x0A, 0x00), None, "limit compare: not taken");
    }

    #[test]
    fn the_maximum_cost_is_the_clamped_equal_clear_route() {
        let costs = [314_u64, 988, 924, 972, 898, 1006, 1016];
        assert_eq!(costs.into_iter().max(), Some(COST_CLAMP_EQ_CLEAR));
    }
}

// ---- record_position_step: a 16-bit position nudged by a ROM step chosen by a record ----

/// Entry of the routine (`$05:9B91`), for the hand-back to the original code.
const RECORD_ENTRY: u32 = 0x05_9B91;
/// Fields of the record at Y (a battle side record, read through DBR): the selector at `$21`, the
/// flags at `$22` and the index source at `$34`.
const REC_SELECTOR: u32 = 0x21;
const REC_FLAGS: u32 = 0x22;
const REC_INDEX: u32 = 0x34;
/// Twelve step bytes (`position_step_bytes`), read at the selector minus six.
const STEP_BYTES: u32 = data::POSITION_STEP_BYTES;
/// Thirty-two step words (`position_step_words`), read at twice the index field's low five bits.
const STEP_WORDS: u32 = data::POSITION_STEP_WORDS;

/// CPU-busy master cycles of the routes the replays take, from the hit to the return (`Verify`):
/// a selector of at least `$12`; a selector below six; the step byte with no bit 0 to 3 set; bit 3
/// (position `+=`); bit 2 (position `-=`); bit 1 with the flags byte's bit 6 clear (`+=`) or set
/// (`-=`).
const COST_OUT_HIGH: u64 = 154;
const COST_OUT_LOW: u64 = 200;
const COST_NO_STEP: u64 = 554;
const COST_POS_A_DOWN: u64 = 602;
const COST_POS_A_UP: u64 = 640;
const COST_POS_B_ADD: u64 = 788;
const COST_POS_B_SUB: u64 = 816;
/// Instructions of the same routes, counted on the listing: the common prefix up to the first
/// test of the step byte is 18, the exits 6 (selector at least `$12`) and 9 (below six).
const INSTR_OUT_HIGH: u64 = 6;
const INSTR_OUT_LOW: u64 = 9;
const INSTR_NO_STEP: u64 = 26;
const INSTR_POS_A_DOWN: u64 = 26;
const INSTR_POS_A_UP: u64 = 28;
const INSTR_POS_B_ADD: u64 = 35;
const INSTR_POS_B_SUB: u64 = 36;

/// Sets A, C, V and N/Z as the subtraction of six leaves them (B is clear, as the XBA before it left
/// it).
fn set_selector_minus_six(ctx: &mut Ctx<'_>, minus_six: u8, no_borrow: bool, overflow: bool) {
    ctx.set_a(u16::from(minus_six));
    ctx.set_c(no_borrow);
    ctx.set_v(overflow);
    ctx.set_nz8(minus_six);
}

/// `record_position_step`, with the record at Y and its selector:
/// - a selector at least `$12` or below six: nothing moves;
/// - else `step_byte = step_bytes[selector - 6]` and `step_word = step_words[2 * (index & $1F)]`,
///   and the step byte is kept at `$20`: bit 2 of the step byte moves the position at `$0A` down by
///   the word; else bit 3 moves it up; else bit 1 moves the position at `$0C` by the word, added
///   when the flags byte's bit 6 is clear and subtracted when it is set; else nothing moves, and the
///   flags are those of the step byte.
///
/// A step with bit 0 of the step byte set and bits 2 and 3 clear is not one the replays take: the
/// original code runs it from the entry instead. The reads come first, so nothing is changed before
/// that decision.
pub(crate) fn record_position_step(ctx: &mut Ctx<'_>) {
    let record = ctx.y();
    let bank = u32::from(ctx.dbr()) << 16;
    let field = |offset: u32| bank + offset + u32::from(record);
    let selector = ctx.read8(field(REC_SELECTOR));
    if selector >= 0x12 {
        // The XBA before the load cleared B: A is the selector alone, C set by the compare.
        ctx.set_a(u16::from(selector));
        ctx.set_c(true);
        ctx.set_nz8(selector.wrapping_sub(0x12));
        ctx.set_cost(COST_OUT_HIGH);
        ctx.set_instructions(INSTR_OUT_HIGH);
        return;
    }
    let (minus_six, no_borrow, overflow) = sbc8(selector, 0x06, true);
    if !no_borrow {
        set_selector_minus_six(ctx, minus_six, no_borrow, overflow);
        ctx.set_cost(COST_OUT_LOW);
        ctx.set_instructions(INSTR_OUT_LOW);
        return;
    }
    let step_byte = ctx.read8(STEP_BYTES + u32::from(minus_six));
    let index = ctx.read8(field(REC_INDEX));
    let index_x2 = (index & 0x1F) << 1;
    let step_word = ctx.read16(STEP_WORDS + u32::from(index_x2));
    let flags = ctx.read8(field(REC_FLAGS));
    if (step_byte & 0x04) == 0 && (step_byte & 0x08) == 0 && (step_byte & 0x01) != 0 {
        let Ok(()) = ctx.call_original(RECORD_ENTRY, ReturnKind::Rts) else { return };
        ctx.set_cost(0);
        ctx.set_instructions(0);
        return;
    }
    set_selector_minus_six(ctx, minus_six, no_borrow, overflow);
    ctx.write_dp8(0x20, step_byte);
    ctx.set_x(u16::from(index_x2));
    ctx.set_a(u16::from(step_byte));
    ctx.set_nz8(step_byte);
    if (step_byte & 0x04) != 0 {
        let (result, carry, ovf) = sub16(ctx.read_dp16(0x0A), step_word);
        ctx.write_dp16(0x0A, result);
        ctx.set_a(result);
        ctx.set_nz16(result);
        ctx.set_c(carry);
        ctx.set_v(ovf);
        ctx.set_cost(COST_POS_A_DOWN);
        ctx.set_instructions(INSTR_POS_A_DOWN);
    } else if (step_byte & 0x08) != 0 {
        let (result, carry, ovf) = add16(ctx.read_dp16(0x0A), step_word);
        ctx.write_dp16(0x0A, result);
        ctx.set_a(result);
        ctx.set_nz16(result);
        ctx.set_c(carry);
        ctx.set_v(ovf);
        ctx.set_cost(COST_POS_A_UP);
        ctx.set_instructions(INSTR_POS_A_UP);
    } else if (step_byte & 0x02) != 0 {
        // Bit 6 of the flags byte set: subtract; clear: add.
        let pos = ctx.read_dp16(0x0C);
        let subtract = flags & 0x40 != 0;
        let (result, carry, ovf) =
            if subtract { sub16(pos, step_word) } else { add16(pos, step_word) };
        ctx.write_dp16(0x0C, result);
        ctx.set_a(result);
        ctx.set_nz16(result);
        ctx.set_c(carry);
        ctx.set_v(ovf);
        ctx.set_cost(if subtract { COST_POS_B_SUB } else { COST_POS_B_ADD });
        ctx.set_instructions(if subtract { INSTR_POS_B_SUB } else { INSTR_POS_B_ADD });
    } else {
        // Bits 0 to 3 all clear: N from the step byte, Z set by the last BIT, C clear from the
        // ASL, V clear from the subtraction of six.
        ctx.set_z(true);
        ctx.set_c(false);
        ctx.set_v(false);
        ctx.set_cost(COST_NO_STEP);
        ctx.set_instructions(INSTR_NO_STEP);
    }
}
