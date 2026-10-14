//! Batch G of the M2 porting: small routines, one section per routine in batch order.
//!
//! Each section is written from the routine's own behaviour (`docs/re/routines/`). Addresses come
//! from the generated `data` constants (`re/symbols.toml`). A path the reference replays never
//! take goes back to the original code (`Ctx::call_original` from the entry, before anything is
//! written), and its bound function declines it, so the timing window never runs it in Rust.

use crate::data;
use ssd_oracle::{Ctx, ReturnKind};

// ---- load_state_1648: the byte at $1648 into A's low byte ----

/// Entry of the routine (`$03:DD83`), for the hand-back to the original code.
const LOAD_STATE_1648_ENTRY: u32 = 0x03_DD83;
/// The byte the routine loads (`state_1648`, read through the data bank as `$1648`).
const STATE_1648: u16 = (data::STATE_1648 & 0xFFFF) as u16;
/// CPU-busy master cycles of the one path the replays take (a byte that is not negative), from
/// the hit to the return (`Verify`).
const LOAD_STATE_1648_COST: u64 = 92;

/// Whether the loaded byte takes the path the replays never reach (bit 7 set: the original code
/// continues into a dispatcher).
pub(crate) fn load_state_1648_leaves_to_original(value: u8) -> bool {
    value & 0x80 != 0
}

/// `load_state_1648`: A's low byte becomes the byte at `DBR:$1648`, N and Z follow it, B, C and V
/// are unchanged. A negative byte runs the original code ([`load_state_1648_leaves_to_original`]).
pub(crate) fn load_state_1648(ctx: &mut Ctx<'_>) {
    let value = ctx.read_db8(STATE_1648);
    if load_state_1648_leaves_to_original(value) {
        let Ok(()) = ctx.call_original(LOAD_STATE_1648_ENTRY, ReturnKind::Rtl) else { return };
        ctx.set_cost(0);
        return;
    }
    ctx.set_al(value);
    ctx.set_nz8(value);
    ctx.set_cost(LOAD_STATE_1648_COST);
}

/// The cost bound of `load_state_1648`: the measured path, and `u64::MAX` (declined) for the
/// negative byte.
pub(crate) fn load_state_1648_bound(ctx: &Ctx<'_>) -> u64 {
    if load_state_1648_leaves_to_original(ctx.peek_db8(STATE_1648)) {
        u64::MAX
    } else {
        LOAD_STATE_1648_COST
    }
}

#[cfg(test)]
mod tests_load_state_1648 {
    use super::{LOAD_STATE_1648_COST, load_state_1648_leaves_to_original};
    use crate::data;
    use crate::routines::tests::max_cost;

    #[test]
    fn the_cost_is_the_measured_one() {
        assert_eq!(max_cost("load_state_1648"), LOAD_STATE_1648_COST);
    }

    #[test]
    fn only_a_negative_byte_goes_to_the_original() {
        assert!(!load_state_1648_leaves_to_original(0x00));
        assert!(!load_state_1648_leaves_to_original(0x7F));
        assert!(load_state_1648_leaves_to_original(0x80));
        assert!(load_state_1648_leaves_to_original(0xFF));
    }

    #[test]
    fn the_byte_is_the_state_variable() {
        assert_eq!(data::STATE_1648 & 0xFFFF, 0x1648);
    }
}

// ---- advance_y_by_20: Y += $20 as a 16-bit sum, which the accumulator also takes ----

/// CPU-busy master cycles of the one path, from the hit to the return (`Verify`).
const ADVANCE_Y_BY_20_COST: u64 = 154;

/// The sum of Y and `$20` as a 16-bit value, with its carry out and its signed overflow.
pub(crate) fn advance_y_sum(y: u16) -> (u16, bool, bool) {
    let (sum, carry) = y.overflowing_add(0x20);
    let overflow = (y as i16).checked_add(0x20).is_none();
    (sum, carry, overflow)
}

/// `advance_y_by_20`: Y and the whole accumulator (B:A) become Y + `$20`. N and Z come from the sum
/// in the index width (the transfer into Y sets them), C and V from the 16-bit addition. The
/// accumulator keeps its 8-bit width; X and memory are unchanged.
pub(crate) fn advance_y_by_20(ctx: &mut Ctx<'_>) {
    let x8 = ctx.x8();
    let (sum, carry, overflow) = advance_y_sum(ctx.y());
    ctx.set_a(sum);
    ctx.set_y(sum);
    if x8 {
        ctx.set_nz8(sum as u8);
    } else {
        ctx.set_nz16(sum);
    }
    ctx.set_c(carry);
    ctx.set_v(overflow);
    ctx.set_cost(ADVANCE_Y_BY_20_COST);
}

#[cfg(test)]
mod tests_advance_y {
    use super::{ADVANCE_Y_BY_20_COST, advance_y_sum};
    use crate::routines::tests::max_cost;

    #[test]
    fn the_cost_is_the_measured_one() {
        assert_eq!(max_cost("advance_y_by_20"), ADVANCE_Y_BY_20_COST);
    }

    #[test]
    fn adds_twenty_hex_with_carry_and_signed_overflow() {
        assert_eq!(advance_y_sum(0x0000), (0x0020, false, false));
        assert_eq!(advance_y_sum(0x00E0), (0x0100, false, false));
        assert_eq!(advance_y_sum(0xFFE0), (0x0000, true, false), "carry out, no signed overflow");
        assert_eq!(advance_y_sum(0x7FF0), (0x8010, false, true), "positive into negative");
        assert_eq!(advance_y_sum(0x8000), (0x8020, false, false));
    }
}

// ---- lookup_byte_pair_2: the byte-pair lookup into the second table ----

/// The 128-byte table of `lookup_byte_pair_2` (`byte_pair_table_2`, bank `$03`): 64 pairs of bytes,
/// directly before `byte_pair_table`.
const PAIR_TABLE_2: u32 = data::BYTE_PAIR_TABLE_2;
/// CPU-busy master cycles of the one path, from the hit to the return (`Verify`).
const LOOKUP_BYTE_PAIR_2_COST: u64 = 568;

/// The address of the pair for `index`: its low six bits pick one of the 64 pairs.
pub(crate) fn pair_address_2(index: u8) -> u32 {
    PAIR_TABLE_2 + u32::from(index & 0x3F) * 2
}

/// A byte as the routine doubles it: shifted left, with bit 7 dropped.
fn doubled_2(byte: u8) -> u8 {
    byte << 1
}

/// `lookup_byte_pair_2`: the pair for the index in A's low byte goes to `$04` and `$06`, each also
/// doubled into `$14` and `$16`, and `$05`, `$07`, `$15` and `$17` are cleared. A = the doubled
/// second byte with B = 0; C = bit 7 of the second byte; N and Z from X (kept as it was); V and X
/// unchanged.
pub(crate) fn lookup_byte_pair_2(ctx: &mut Ctx<'_>) {
    let address = pair_address_2(ctx.al());
    let first = ctx.read8(address);
    let second = ctx.read8(address + 1);
    ctx.write_dp8(0x04, first);
    ctx.write_dp8(0x14, doubled_2(first));
    ctx.write_dp8(0x06, second);
    ctx.write_dp8(0x16, doubled_2(second));
    for offset in [0x05, 0x07, 0x15, 0x17] {
        ctx.write_dp8(offset, 0);
    }
    ctx.set_a(u16::from(doubled_2(second)));
    let x = ctx.x();
    if ctx.x8() {
        ctx.set_nz8(x as u8);
    } else {
        ctx.set_nz16(x);
    }
    ctx.set_c(second & 0x80 != 0);
    ctx.set_cost(LOOKUP_BYTE_PAIR_2_COST);
}

#[cfg(test)]
mod tests_pair_2 {
    use super::{PAIR_TABLE_2, doubled_2, pair_address_2};
    use crate::data;

    #[test]
    fn the_second_table_sits_right_before_the_first() {
        assert_eq!(PAIR_TABLE_2 + 128, data::BYTE_PAIR_TABLE);
        assert_eq!(data::BYTE_PAIR_TABLE_2_SIZE, 128);
    }

    #[test]
    fn the_index_takes_its_low_six_bits_as_pairs() {
        assert_eq!(pair_address_2(0x00), PAIR_TABLE_2);
        assert_eq!(pair_address_2(0x01), PAIR_TABLE_2 + 2);
        assert_eq!(pair_address_2(0x0C), PAIR_TABLE_2 + 24);
        assert_eq!(pair_address_2(0x3F), PAIR_TABLE_2 + 126);
        assert_eq!(pair_address_2(0x40), PAIR_TABLE_2, "bits 6 and 7 are not part of the index");
        assert_eq!(pair_address_2(0xFF), PAIR_TABLE_2 + 126);
    }

    #[test]
    fn doubling_drops_bit_seven() {
        assert_eq!(doubled_2(0x41), 0x82);
        assert_eq!(doubled_2(0x80), 0x00);
        assert_eq!(doubled_2(0xFF), 0xFE);
    }
}

// ---- derive_state_071a: two state bytes from $0716 and $0029, and $071A from bit 7 of $0716 ----

/// The control byte (`control_0716`), read through the data bank as `$0716`.
const CONTROL_0716: u16 = (data::CONTROL_0716 & 0xFFFF) as u16;
/// The direct-page byte the routine reads (`state_0029`, offset `$29`).
const STATE_0029: u16 = (data::STATE_0029 & 0xFFFF) as u16;
/// The two fields the routine writes (`field_0718`, `field_0719`) and the state byte (`state_071a`).
const FIELD_0718: u16 = (data::FIELD_0718 & 0xFFFF) as u16;
const FIELD_0719: u16 = (data::FIELD_0719 & 0xFFFF) as u16;
const STATE_071A: u16 = (data::STATE_071A & 0xFFFF) as u16;
/// CPU-busy master cycles of the two paths, from the hit to the return (`Verify`): bit 7 of the
/// control byte set (the state is 2), and clear (the state is 1).
const COST_BIT7_SET: u64 = 298;
const COST_BIT7_CLEAR: u64 = 314;
/// Instructions of the two paths (opcode fetches, `Verify`).
const INSTRUCTIONS_BIT7_SET: u64 = 11;
const INSTRUCTIONS_BIT7_CLEAR: u64 = 12;

/// The state byte for a control byte: 2 when bit 7 is set, 1 otherwise.
pub(crate) fn state_for_control(control: u8) -> u8 {
    if control & 0x80 != 0 { 2 } else { 1 }
}

/// `derive_state_071a`: `$0719` = the control byte AND `$1F`, `$0718` = `$0029` AND `$0F`, and
/// `$071A` = [`state_for_control`]; A's low byte becomes that state (B kept). The last load is a
/// constant, so N and Z are clear. C, V, X and Y are unchanged.
pub(crate) fn derive_state_071a(ctx: &mut Ctx<'_>) {
    let control = ctx.read_db8(CONTROL_0716);
    let low = ctx.read_dp8(STATE_0029);
    ctx.write_db8(FIELD_0719, control & 0x1F);
    ctx.write_db8(FIELD_0718, low & 0x0F);
    let state = state_for_control(control);
    ctx.write_db8(STATE_071A, state);
    ctx.set_al(state);
    ctx.set_nz8(state);
    if control & 0x80 != 0 {
        ctx.set_instructions(INSTRUCTIONS_BIT7_SET);
        ctx.set_cost(COST_BIT7_SET);
    } else {
        ctx.set_instructions(INSTRUCTIONS_BIT7_CLEAR);
        ctx.set_cost(COST_BIT7_CLEAR);
    }
}

#[cfg(test)]
mod tests_state_071a {
    use super::{COST_BIT7_CLEAR, state_for_control};
    use crate::routines::tests::max_cost;

    #[test]
    fn the_cost_is_the_larger_measured_path() {
        assert_eq!(max_cost("derive_state_071a"), COST_BIT7_CLEAR);
    }

    #[test]
    fn bit_seven_of_the_control_byte_picks_the_state() {
        assert_eq!(state_for_control(0x00), 1);
        assert_eq!(state_for_control(0x41), 1);
        assert_eq!(state_for_control(0x7F), 1);
        assert_eq!(state_for_control(0x80), 2);
        assert_eq!(state_for_control(0xA1), 2);
    }
}

// ---- load_flag_01d6: the byte at $01D6 into A's low byte ----

/// The byte the routine loads (`flag_01d6`, read through the data bank as `$01D6`).
const FLAG_01D6: u16 = (data::FLAG_01D6 & 0xFFFF) as u16;
/// Entry of the routine (`$1D:8ABD`), for the hand-back to the original code.
const LOAD_FLAG_01D6_ENTRY: u32 = 0x1D_8ABD;
/// CPU-busy master cycles of the one path the replays take, from the hit to the return (`Verify`).
const LOAD_FLAG_01D6_COST: u64 = 92;

/// Whether the loaded byte takes the path the replays never reach (bit 7 set).
pub(crate) fn load_flag_01d6_leaves_to_original(value: u8) -> bool {
    value & 0x80 != 0
}

/// `load_flag_01d6`: A's low byte becomes the byte at `DBR:$01D6`, N and Z follow it, B, C and V are
/// unchanged. A negative byte runs the original code ([`load_flag_01d6_leaves_to_original`]).
pub(crate) fn load_flag_01d6(ctx: &mut Ctx<'_>) {
    let value = ctx.read_db8(FLAG_01D6);
    if load_flag_01d6_leaves_to_original(value) {
        let Ok(()) = ctx.call_original(LOAD_FLAG_01D6_ENTRY, ReturnKind::Rtl) else { return };
        ctx.set_cost(0);
        return;
    }
    ctx.set_al(value);
    ctx.set_nz8(value);
    ctx.set_cost(LOAD_FLAG_01D6_COST);
}

/// The cost bound of `load_flag_01d6`: the measured path, and `u64::MAX` (declined) for a negative
/// byte.
pub(crate) fn load_flag_01d6_bound(ctx: &Ctx<'_>) -> u64 {
    if load_flag_01d6_leaves_to_original(ctx.peek_db8(FLAG_01D6)) {
        u64::MAX
    } else {
        LOAD_FLAG_01D6_COST
    }
}

#[cfg(test)]
mod tests_load_flag_01d6 {
    use super::{LOAD_FLAG_01D6_COST, load_flag_01d6_leaves_to_original};
    use crate::routines::tests::max_cost;

    #[test]
    fn the_cost_is_the_measured_one() {
        assert_eq!(max_cost("load_flag_01d6"), LOAD_FLAG_01D6_COST);
    }

    #[test]
    fn only_a_negative_byte_goes_to_the_original() {
        assert!(!load_flag_01d6_leaves_to_original(0x00));
        assert!(!load_flag_01d6_leaves_to_original(0x7F));
        assert!(load_flag_01d6_leaves_to_original(0x80));
    }
}
// ---- load_counter_1570: the byte at $1570 into A's low byte ----

/// The byte the routine loads (`counter_1570`, read through the data bank as `$1570`).
const COUNTER_1570: u16 = (data::COUNTER_1570 & 0xFFFF) as u16;
/// Entry of the routine (`$1D:8843`), for the hand-back to the original code.
const LOAD_COUNTER_1570_ENTRY: u32 = 0x1D_8843;
/// CPU-busy master cycles of the one path the replays take, from the hit to the return (`Verify`).
const LOAD_COUNTER_1570_COST: u64 = 92;

/// Whether the loaded byte takes the path the replays never reach (bit 7 set).
pub(crate) fn load_counter_1570_leaves_to_original(value: u8) -> bool {
    value & 0x80 != 0
}

/// `load_counter_1570`: A's low byte becomes the byte at `DBR:$1570`, N and Z follow it, B, C and V
/// are unchanged. A negative byte runs the original code ([`load_counter_1570_leaves_to_original`]).
pub(crate) fn load_counter_1570(ctx: &mut Ctx<'_>) {
    let value = ctx.read_db8(COUNTER_1570);
    if load_counter_1570_leaves_to_original(value) {
        let Ok(()) = ctx.call_original(LOAD_COUNTER_1570_ENTRY, ReturnKind::Rtl) else { return };
        ctx.set_cost(0);
        return;
    }
    ctx.set_al(value);
    ctx.set_nz8(value);
    ctx.set_cost(LOAD_COUNTER_1570_COST);
}

/// The cost bound of `load_counter_1570`: the measured path, and `u64::MAX` (declined) for a
/// negative byte.
pub(crate) fn load_counter_1570_bound(ctx: &Ctx<'_>) -> u64 {
    if load_counter_1570_leaves_to_original(ctx.peek_db8(COUNTER_1570)) {
        u64::MAX
    } else {
        LOAD_COUNTER_1570_COST
    }
}

#[cfg(test)]
mod tests_load_counter_1570 {
    use super::{LOAD_COUNTER_1570_COST, load_counter_1570_leaves_to_original};
    use crate::routines::tests::max_cost;

    #[test]
    fn the_cost_is_the_measured_one() {
        assert_eq!(max_cost("load_counter_1570"), LOAD_COUNTER_1570_COST);
    }

    #[test]
    fn only_a_negative_byte_goes_to_the_original() {
        assert!(!load_counter_1570_leaves_to_original(0x00));
        assert!(!load_counter_1570_leaves_to_original(0x7F));
        assert!(load_counter_1570_leaves_to_original(0x80));
    }
}
// ---- noop_9735: a routine that is only its return ----

/// CPU-busy master cycles of the one path (the return alone), from the hit to the return (`Verify`).
const NOOP_9735_COST: u64 = 44;

/// `noop_9735`: changes nothing; the return is the whole routine.
pub(crate) fn noop_9735(ctx: &mut Ctx<'_>) {
    ctx.set_cost(NOOP_9735_COST);
}

#[cfg(test)]
mod tests_noop_9735 {
    use super::NOOP_9735_COST;
    use crate::routines::tests::max_cost;

    #[test]
    fn the_cost_is_the_measured_one() {
        assert_eq!(max_cost("noop_9735"), NOOP_9735_COST);
    }
}
// ---- clear_48_and_index_1220: bit 7 of $48 off, X = twice the byte at $1220 ----

/// The direct-page byte the routine clears bit 7 of (`source_byte_0048`: the one name of `$7E:0048`).
const SOURCE_BYTE_0048: u16 = (data::SOURCE_BYTE_0048 & 0xFFFF) as u16;
/// The byte the index is made from (`value_1220`, read through the data bank as `$1220`).
const VALUE_1220: u16 = (data::VALUE_1220 & 0xFFFF) as u16;
/// CPU-busy master cycles of the one path, from the hit to the return (`Verify`).
const CLEAR_48_AND_INDEX_1220_COST: u64 = 202;

/// `clear_48_and_index_1220`: `$48` = `$48` AND `$7F`; the value is the byte at `$1220` doubled, its
/// high bit dropped. A = the value (B = 0), X = the value, N and Z from X in its width (for 16-bit
/// index registers N is clear), C = bit 7 of the byte at `$1220`. V and Y are unchanged.
pub(crate) fn clear_48_and_index_1220(ctx: &mut Ctx<'_>) {
    let bits = ctx.read_dp8(SOURCE_BYTE_0048) & 0x7F;
    ctx.write_dp8(SOURCE_BYTE_0048, bits);
    let source = ctx.read_db8(VALUE_1220);
    let doubled = source << 1;
    ctx.set_a(u16::from(doubled));
    ctx.set_x(u16::from(doubled));
    if ctx.x8() {
        ctx.set_nz8(doubled);
    } else {
        ctx.set_nz16(u16::from(doubled));
    }
    ctx.set_c(source & 0x80 != 0);
    ctx.set_cost(CLEAR_48_AND_INDEX_1220_COST);
}

#[cfg(test)]
mod tests_clear_48 {
    use super::CLEAR_48_AND_INDEX_1220_COST;
    use crate::routines::tests::max_cost;

    #[test]
    fn the_cost_is_the_measured_one() {
        assert_eq!(max_cost("clear_48_and_index_1220"), CLEAR_48_AND_INDEX_1220_COST);
    }
}
// ---- split_03_nibbles: the low byte of $03 into two map-cell digits ----

/// The byte split (the low byte of the word at `$03`, `divisor_0003`).
const SPLIT_SOURCE: u16 = (data::DIVISOR_0003 & 0xFFFF) as u16;
/// The two digit cells, low then high (`digit_cells_0d6c`, bytes `$0D6C` and `$0D6D`).
const DIGIT_CELLS: u16 = (data::DIGIT_CELLS_0D6C & 0xFFFF) as u16;
/// CPU-busy master cycles of the one path, from the hit to the return (`Verify`).
const SPLIT_03_NIBBLES_COST: u64 = 556;

/// The two digit cells for a byte: the low nibble (low cell) and the high nibble (high cell), each
/// with its top bit set.
pub(crate) fn digit_cells(value: u8) -> (u8, u8) {
    (0x80 | (value & 0x0F), 0x80 | (value >> 4))
}

/// `split_03_nibbles`: the low cell becomes `$80` OR the low nibble of `$03`, the high cell `$80` OR
/// its high nibble. A = the low cell (B kept); N set, Z and C clear. V, X, Y are unchanged.
pub(crate) fn split_03_nibbles(ctx: &mut Ctx<'_>) {
    let (low, high) = digit_cells(ctx.read_dp8(SPLIT_SOURCE));
    ctx.write_db8(DIGIT_CELLS, low);
    ctx.write_db8(DIGIT_CELLS + 1, high);
    ctx.set_al(low);
    ctx.set_n(true);
    ctx.set_z(false);
    ctx.set_c(false);
    ctx.set_cost(SPLIT_03_NIBBLES_COST);
}

#[cfg(test)]
mod tests_split_03 {
    use super::{SPLIT_03_NIBBLES_COST, digit_cells};
    use crate::routines::tests::max_cost;

    #[test]
    fn the_cost_is_the_measured_one() {
        assert_eq!(max_cost("split_03_nibbles"), SPLIT_03_NIBBLES_COST);
    }

    #[test]
    fn each_nibble_becomes_a_cell_with_its_top_bit_set() {
        assert_eq!(digit_cells(0x00), (0x80, 0x80));
        assert_eq!(digit_cells(0xAB), (0x8B, 0x8A));
        assert_eq!(digit_cells(0xF0), (0x80, 0x8F));
    }
}

// ---- divide_24_by_16: the 24-bit dividend in $00-$02 over the 16-bit divisor in $03-$04 ----

/// The dividend, which becomes the quotient (`div_dividend`, bytes `$00` to `$02`).
const DIVIDEND: u16 = (data::DIVIDEND_0000 & 0xFFFF) as u16;
/// The divisor (`div_divisor`, bytes `$03` and `$04`).
const DIVISOR: u16 = (data::DIVISOR_0003 & 0xFFFF) as u16;
/// The remainder (`div_remainder`, bytes `$05` to `$07`).
const REMAINDER: u16 = (data::REMAINDER_0005 & 0xFFFF) as u16;
/// The difference scratch bytes of the last step (`div_scratch`, bytes `$08` and `$09`).
const SCRATCH: u16 = (data::SCRATCH_0008 & 0xFFFF) as u16;
/// Steps of the division: one per bit of the dividend.
const STEPS: u32 = 24;
/// CPU-busy master cycles of a call that subtracts in no step, and the extra cost of each step that
/// subtracts, from the hit to the return (`Verify`): the cost is `COST_NONE + COST_STEP * n`, `n` the
/// number of subtractions (the bits set in the quotient).
const COST_NONE: u64 = 11786;
const COST_STEP: u64 = 152;
/// Instructions of a call: the set-up, 18 per step that does not subtract, 6 more per step that does,
/// and the return (opcode fetches, `Verify`).
const INSTRUCTIONS_BASE: u64 = 438;
const INSTRUCTIONS_STEP: u64 = 6;

/// Binary subtraction with borrow, as the CPU does it: the byte, the carry out (set when there is no
/// borrow) and the signed overflow.
pub(crate) fn sbc8(a: u8, m: u8, carry: bool) -> (u8, bool, bool) {
    let borrow = i16::from(!carry);
    let diff = i16::from(a) - i16::from(m) - borrow;
    let result = diff as u8;
    let overflow = (a ^ m) & (a ^ result) & 0x80 != 0;
    (result, diff >= 0, overflow)
}

/// The state the routine leaves: the quotient, the remainder, the scratch bytes and the low byte of
/// A of the last step, the flags of its last subtraction, and how many steps subtracted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Division {
    pub quotient: u32,
    pub remainder: u32,
    pub scratch_low: u8,
    pub scratch_mid: u8,
    pub a: u8,
    pub carry: bool,
    pub overflow: bool,
    pub subtractions: u32,
}

/// The restoring division of the routine, step by step: the dividend's top bit moves into the
/// remainder, and the remainder less the divisor is kept when it does not borrow, the step's quotient
/// bit being set then. A zero divisor never borrows.
pub(crate) fn divide(dividend: u32, divisor: u16) -> Division {
    let mut shifted = dividend & 0xFF_FFFF;
    let mut remainder: u32 = 0;
    let mut last = Division {
        quotient: 0,
        remainder: 0,
        scratch_low: 0,
        scratch_mid: 0,
        a: 0,
        carry: false,
        overflow: false,
        subtractions: 0,
    };
    for _ in 0..STEPS {
        let top = (shifted >> 23) & 1;
        shifted = (shifted << 1) & 0xFF_FFFF;
        let moved = ((remainder << 1) | top) & 0xFF_FFFF;
        let (low, carry, _) = sbc8(moved as u8, divisor as u8, true);
        let (mid, carry, _) = sbc8((moved >> 8) as u8, (divisor >> 8) as u8, carry);
        let (high, carry, overflow) = sbc8((moved >> 16) as u8, 0, carry);
        if carry {
            remainder = (u32::from(high) << 16) | (u32::from(mid) << 8) | u32::from(low);
            shifted |= 1;
            last.subtractions += 1;
        } else {
            remainder = moved;
        }
        last.a = if carry { low } else { high };
        last.carry = carry;
        last.overflow = overflow;
        last.scratch_low = low;
        last.scratch_mid = mid;
    }
    last.quotient = shifted;
    last.remainder = remainder;
    last
}

/// The cost of a call with `subtractions` subtracting steps.
pub(crate) fn divide_cost(subtractions: u32) -> u64 {
    COST_NONE + COST_STEP * u64::from(subtractions)
}

/// `divide_24_by_16`: the quotient replaces the dividend in `$00-$02`, the remainder goes to
/// `$05-$07`, and `$08`/`$09` keep the last step's difference. A = the last step's low difference
/// (a subtracting last step) or its high one; Y = 0; N clear, Z set; C and V from the last step's
/// subtraction. X, the divisor and the other memory are unchanged.
pub(crate) fn divide_24_by_16(ctx: &mut Ctx<'_>) {
    let dividend = u32::from(ctx.read_dp8(DIVIDEND))
        | (u32::from(ctx.read_dp8(DIVIDEND + 1)) << 8)
        | (u32::from(ctx.read_dp8(DIVIDEND + 2)) << 16);
    let divisor = ctx.read_dp16(DIVISOR);
    let result = divide(dividend, divisor);
    let quotient = result.quotient.to_le_bytes();
    let remainder = result.remainder.to_le_bytes();
    for (offset, byte) in quotient.iter().take(3).enumerate() {
        ctx.write_dp8(DIVIDEND + offset as u16, *byte);
    }
    for (offset, byte) in remainder.iter().take(3).enumerate() {
        ctx.write_dp8(REMAINDER + offset as u16, *byte);
    }
    ctx.write_dp8(SCRATCH, result.scratch_low);
    ctx.write_dp8(SCRATCH + 1, result.scratch_mid);
    ctx.set_al(result.a);
    ctx.set_y(0);
    ctx.set_n(false);
    ctx.set_z(true);
    ctx.set_c(result.carry);
    ctx.set_v(result.overflow);
    ctx.set_instructions(INSTRUCTIONS_BASE + INSTRUCTIONS_STEP * u64::from(result.subtractions));
    ctx.set_cost(divide_cost(result.subtractions));
}

/// The cost bound of `divide_24_by_16`: its cost for the division the entry state holds (exact).
pub(crate) fn divide_24_by_16_bound(ctx: &Ctx<'_>) -> u64 {
    let dividend = u32::from(ctx.peek_dp8(DIVIDEND))
        | (u32::from(ctx.peek_dp8(DIVIDEND + 1)) << 8)
        | (u32::from(ctx.peek_dp8(DIVIDEND + 2)) << 16);
    divide_cost(divide(dividend, ctx.peek_dp16(DIVISOR)).subtractions)
}

#[cfg(test)]
mod tests_divide {
    use super::{COST_NONE, COST_STEP, divide, divide_cost, sbc8};
    use crate::routines::tests::max_cost;

    #[test]
    fn the_largest_cost_is_the_all_subtracting_call() {
        assert_eq!(divide_cost(0), COST_NONE);
        assert_eq!(divide_cost(24), 15434);
        assert_eq!(max_cost("divide_24_by_16"), divide_cost(24));
    }

    #[test]
    fn the_measured_costs_fit_the_model() {
        // Costs seen in the replays, with the subtractions of their inputs.
        assert_eq!(divide_cost(5), 12546);
        assert_eq!(divide_cost(4), 12394);
        assert_eq!(divide_cost(1), 11938);
        assert_eq!(divide_cost(0), 11786);
        assert_eq!(COST_STEP, 152);
    }

    #[test]
    fn the_quotient_and_remainder_are_the_true_ones() {
        for (dividend, divisor) in
            [(824_u32, 2_u16), (326, 1), (14080, 32), (14080, 440), (0xFF_FFFF, 3), (5, 7)]
        {
            let result = divide(dividend, divisor);
            assert_eq!(result.quotient, dividend / u32::from(divisor), "{dividend}/{divisor}");
            assert_eq!(result.remainder, dividend % u32::from(divisor), "{dividend}%{divisor}");
        }
    }

    #[test]
    fn a_zero_divisor_subtracts_at_every_step() {
        let result = divide(0x12_3456, 0);
        assert_eq!(result.quotient, 0xFF_FFFF);
        assert_eq!(result.remainder, 0x12_3456);
        assert_eq!(result.subtractions, 24);
        assert_eq!(divide(0, 0x20).subtractions, 0);
    }

    #[test]
    fn subtraction_borrows_as_the_cpu_does() {
        assert_eq!(sbc8(0x05, 0x03, true), (0x02, true, false));
        assert_eq!(sbc8(0x03, 0x05, true), (0xFE, false, false));
        assert_eq!(sbc8(0x80, 0x01, true), (0x7F, true, true), "signed overflow");
        assert_eq!(sbc8(0x05, 0x03, false), (0x01, true, false), "with a borrow in");
    }
}

// ---- adjust_37_toward_ffd1: when $63 = $1A, the word at $37 moves towards $FFD1 ----

/// The mode byte (`mode_0063`), read as `$63`.
const MODE_0063: u16 = (data::MODE_0063 & 0xFFFF) as u16;
/// The word the routine adjusts (`word_0037`, bytes `$37` and `$38`).
const WORD_0037: u16 = (data::WORD_0037 & 0xFFFF) as u16;
/// The mode value that enables the adjustment, and the word's target.
const MODE_ENABLED: u8 = 0x1A;
const TARGET_0037: u16 = 0xFFD1;
/// CPU-busy master cycles and instructions (opcode fetches, `Verify`) of the four paths, from the
/// hit to the return: the mode is not `$1A`; the word is the target; the word is above the target
/// (one less); the word is below it (eight more).
const COST_MODE_OTHER: u64 = 126;
const COST_TARGET: u64 = 220;
const COST_ABOVE: u64 = 282;
const COST_BELOW: u64 = 322;
const INSTRUCTIONS_MODE_OTHER: u64 = 5;
const INSTRUCTIONS_TARGET: u64 = 9;
const INSTRUCTIONS_ABOVE: u64 = 12;
const INSTRUCTIONS_BELOW: u64 = 14;

/// The word after the adjustment, and the path that made it: `Some(true)` above the target,
/// `Some(false)` below it, `None` at the target (nothing changes).
pub(crate) fn adjust_toward_target(word: u16) -> (u16, Option<bool>) {
    match word.cmp(&TARGET_0037) {
        std::cmp::Ordering::Equal => (word, None),
        std::cmp::Ordering::Greater => (word.wrapping_sub(1), Some(true)),
        std::cmp::Ordering::Less => (word.wrapping_add(8), Some(false)),
    }
}

/// `adjust_37_toward_ffd1`. With `$63` other than `$1A` the routine only compares `$63` with `$1A`
/// (N from the difference, Z and C as a compare sets them). With `$63` = `$1A` it compares the word
/// at `$37` with `$FFD1`: at the target nothing changes; above it the word drops by one; below it the
/// word rises by eight (its carry and overflow are those of that addition). The accumulator ends as
/// the word (16 bits) on those paths; index registers and memory are unchanged otherwise.
pub(crate) fn adjust_37_toward_ffd1(ctx: &mut Ctx<'_>) {
    let mode = ctx.read_dp8(MODE_0063);
    if mode != MODE_ENABLED {
        ctx.set_al(mode);
        ctx.set_n(mode.wrapping_sub(MODE_ENABLED) & 0x80 != 0);
        ctx.set_z(false);
        ctx.set_c(mode >= MODE_ENABLED);
        ctx.set_instructions(INSTRUCTIONS_MODE_OTHER);
        ctx.set_cost(COST_MODE_OTHER);
        return;
    }
    let word = ctx.read_dp16(WORD_0037);
    ctx.set_a(word);
    match adjust_toward_target(word) {
        (_, None) => {
            ctx.set_n(false);
            ctx.set_z(true);
            ctx.set_c(true);
            ctx.set_instructions(INSTRUCTIONS_TARGET);
            ctx.set_cost(COST_TARGET);
        }
        (result, Some(true)) => {
            ctx.write_dp16(WORD_0037, result);
            ctx.set_a(result);
            ctx.set_nz16(result);
            ctx.set_c(true);
            ctx.set_instructions(INSTRUCTIONS_ABOVE);
            ctx.set_cost(COST_ABOVE);
        }
        (result, Some(false)) => {
            let (_, carry) = word.overflowing_add(8);
            let overflow = (word as i16).checked_add(8).is_none();
            ctx.write_dp16(WORD_0037, result);
            ctx.set_a(result);
            ctx.set_nz16(result);
            ctx.set_c(carry);
            ctx.set_v(overflow);
            ctx.set_instructions(INSTRUCTIONS_BELOW);
            ctx.set_cost(COST_BELOW);
        }
    }
}

#[cfg(test)]
mod tests_adjust_37 {
    use super::{COST_BELOW, adjust_toward_target};
    use crate::routines::tests::max_cost;

    #[test]
    fn the_largest_cost_is_the_below_path() {
        assert_eq!(max_cost("adjust_37_toward_ffd1"), COST_BELOW);
    }

    #[test]
    fn the_word_moves_towards_the_target() {
        assert_eq!(adjust_toward_target(0xFFD1), (0xFFD1, None));
        assert_eq!(adjust_toward_target(0xFFD2), (0xFFD1, Some(true)));
        assert_eq!(adjust_toward_target(0xFFFF), (0xFFFE, Some(true)));
        assert_eq!(adjust_toward_target(0xFFD0), (0xFFD8, Some(false)));
        assert_eq!(adjust_toward_target(0x0000), (0x0008, Some(false)));
    }
}

// ---- draw_map_entry_tiles: two tile bytes at $7E:3000 + an offset, for the map entry in $0D66 ----

/// Entry of the routine (`$01:B90F`), for the hand-back to the original code.
const MAP_ENTRY_ENTRY: u32 = 0x01_B90F;

/// The map entry index and value (`map_entry_index`, `map_entry_value`), read through the data bank.
const ENTRY_INDEX: u16 = (data::MAP_ENTRY_INDEX & 0xFFFF) as u16;
const ENTRY_VALUE: u16 = (data::MAP_ENTRY_VALUE & 0xFFFF) as u16;
/// The byte the routine stores its value to (`map_entry_0d64`), the flag whose bit 7 picks the word
/// table (`map_flag_0d6a`), and the byte that indexes the byte table (`map_entry_0d63`).
const ENTRY_OUT: u16 = (data::MAP_ENTRY_0D64 & 0xFFFF) as u16;
const ENTRY_FLAG: u16 = (data::MAP_FLAG_0D6A & 0xFFFF) as u16;
const ENTRY_0D63: u16 = (data::MAP_ENTRY_0D63 & 0xFFFF) as u16;
/// Parameter bytes of the special indexes (`map_param_01bb`, `map_param_01b3`, `map_param_0c34`).
const PARAM_01BB: u16 = (data::MAP_PARAM_01BB & 0xFFFF) as u16;
const PARAM_01B3: u16 = (data::MAP_PARAM_01B3 & 0xFFFF) as u16;
const PARAM_0C34: u16 = (data::MAP_PARAM_0C34 & 0xFFFF) as u16;
/// The record table (`map_record_table`, bank `$02`): three bytes per index, a value byte then a word.
const RECORD_TABLE: u32 = data::MAP_RECORD_TABLE;
/// The word table (`map_word_table`, bank `$01`), read at twice the value; the view from `$BF79` is
/// `WORD_TABLE_FLAGGED` bytes further on.
const WORD_TABLE: u32 = data::MAP_WORD_TABLE;
const WORD_TABLE_FLAGGED: u32 = 0x18;
/// The byte table (`map_byte_table`, bank `$01`), read at `$0D63`.
const BYTE_TABLE: u32 = data::MAP_BYTE_TABLE;
/// The tile buffer the two bytes are stored in: `$7E:3000` plus the offset the word gives.
const TILE_BASE: u32 = 0x7E_3000;
/// Index values the routine handles by their own routes (the others leave the table path).
const INDEX_SKIPPED: u8 = 0x33;
const INDEX_BY_01BB: [u8; 2] = [0x02, 0x14];
const INDEX_BY_01B3: u8 = 0x0B;
const INDEX_BY_0C34: u8 = 0x2D;
const INDEX_BY_BYTE_TABLE: u8 = 0x26;

/// Master cycles of the instructions the routine runs, at its widths (8-bit accumulator, 16-bit
/// index registers): a bus access takes 8 master cycles, an internal cycle 6 (`docs/re/routines/`).
mod timing {
    pub(crate) const IMM8: u64 = 16;
    pub(crate) const IMM16: u64 = 24;
    pub(crate) const ABS8: u64 = 32;
    pub(crate) const DP8: u64 = 24;
    pub(crate) const DP16: u64 = 32;
    pub(crate) const LONG8: u64 = 40;
    pub(crate) const LONG16: u64 = 48;
    pub(crate) const IMPLIED: u64 = 14;
    pub(crate) const XBA: u64 = 20;
    pub(crate) const PHX: u64 = 30;
    pub(crate) const PLX: u64 = 36;
    pub(crate) const REP_SEP: u64 = 22;
    pub(crate) const NOT_TAKEN: u64 = 16;
    pub(crate) const TAKEN: u64 = 22;
    pub(crate) const JMP: u64 = 24;
    pub(crate) const RTL: u64 = 44;
}

/// What the routine reads at the hit (all of it side-effect free: ROM and WRAM).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Entry {
    /// `$0D66`, the map entry index.
    pub index: u8,
    /// `$0D62`, the map entry value.
    pub value: u8,
    /// `$0D6A`, whose bit 7 picks the word table view.
    pub flag: u8,
    /// Direct-page `$00`.
    pub q: u8,
    /// `$01BB`, `$01B3` and `$0C34`: the parameter bytes of the special indexes.
    pub param_01bb: u8,
    pub param_01b3: u8,
    pub param_0c34: u8,
    /// The byte of `$01:B9E3 + $0D63`, stored to `$0F` by the `$26` route.
    pub table_byte: u8,
    /// The record of the index: its value byte, and the word after it (`$02:E232 + 3 * index`).
    pub record_byte: u8,
    pub record_word: u16,
    /// The word of the word table at twice `$0D62` (`$01:BF61`, or `$01:BF79` with bit 7 of the
    /// flag set).
    pub word: u16,
}

/// What the path the routine takes in Rust leaves behind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Trace {
    pub cost: u64,
    pub instructions: u64,
    /// Stored to `$0D64`.
    pub out: u8,
    /// Direct-page bytes the path writes: `$0E`, `$0F`, `$10`, and the word at `$04`.
    pub dp_0e: Option<u8>,
    pub dp_0f: Option<u8>,
    pub dp_10: Option<u8>,
    pub dp_04: Option<u16>,
    /// The two tile bytes stored at `TILE_BASE + x` and `TILE_BASE + x + 1` (the main path only).
    pub tiles: Option<[u8; 2]>,
    /// X, the low byte of A (its high byte is 0), and N, Z, C, V.
    pub x: u16,
    pub a: u8,
    pub n: bool,
    pub z: bool,
    pub c: bool,
    pub v: bool,
}

/// Cost and instructions of a path so far.
#[derive(Debug, Default, Clone, Copy)]
struct Count {
    cost: u64,
    instructions: u64,
}

impl Count {
    fn step(&mut self, cost: u64) {
        self.cost += cost;
        self.instructions += 1;
    }

    /// A compare with an immediate and the branch that follows it: taken when `hit`.
    fn compare_and_branch(&mut self, hit: bool) {
        self.step(timing::IMM8);
        self.step(if hit { timing::TAKEN } else { timing::NOT_TAKEN });
    }
}

/// An 8-bit add with no carry in: the sum, the carry out and the signed overflow.
fn adc8(a: u8, b: u8) -> (u8, bool, bool) {
    let sum = u16::from(a) + u16::from(b);
    let result = sum as u8;
    (result, sum > 0xFF, (a ^ result) & (b ^ result) & 0x80 != 0)
}

/// A 16-bit add with no carry in: the sum, the carry out and the signed overflow.
fn adc16(a: u16, b: u16) -> (u16, bool, bool) {
    let (sum, carry) = a.overflowing_add(b);
    (sum, carry, (a ^ sum) & (b ^ sum) & 0x8000 != 0)
}

/// The tail of every path that stores a non-zero value: `$10` = `$0D62`, the word into `$04`, the
/// index into the record (`$0D66` times 3), then the tile bytes at the address the two words make.
/// Its cost follows bit 7 of the flag (the word view) and whether `$00` is 1; its flags are its own.
fn main_tail(
    mut count: Count,
    entry: &Entry,
    out: u8,
    dp_0e: Option<u8>,
    dp_0f: Option<u8>,
) -> Trace {
    count.step(timing::ABS8); // LDA $0D62
    count.step(timing::DP8); // STA $10
    count.step(timing::IMPLIED); // INX
    count.step(timing::PHX); // PHX
    count.step(timing::IMM8); // LDA #$00
    count.step(timing::XBA); // XBA
    count.step(timing::DP8); // LDA $10
    count.step(timing::IMPLIED); // ASL A
    count.step(timing::IMPLIED); // TAX
    count.step(timing::ABS8); // LDA $0D6A
    if entry.flag & 0x80 != 0 {
        count.step(timing::NOT_TAKEN); // BPL falls through
        count.step(timing::REP_SEP); // REP #$20
        count.step(timing::LONG16); // LDA $01:BF79,X
        count.step(timing::TAKEN); // BRA
    } else {
        count.step(timing::TAKEN); // BPL taken
        count.step(timing::REP_SEP); // REP #$20
        count.step(timing::LONG16); // LDA $01:BF61,X
    }
    count.step(timing::DP16); // STA $04
    count.step(timing::PLX); // PLX
    count.step(timing::LONG16); // LDA $02:E232,X
    count.step(timing::IMPLIED); // CLC
    count.step(timing::DP16); // ADC $04
    count.step(timing::IMPLIED); // TAX
    count.step(timing::IMM16); // LDA #$0000
    count.step(timing::REP_SEP); // SEP #$20
    count.step(timing::DP8); // LDA $00
    count.step(timing::LONG8); // STA $7E:3000,X
    count.step(timing::IMM8); // CMP #$01
    let (sum, _, overflow) = adc16(entry.record_word, entry.word);
    let second = if entry.q == 1 {
        count.step(timing::TAKEN); // BEQ taken
        count.step(timing::IMM8); // LDA #$28
        0x28
    } else {
        count.step(timing::NOT_TAKEN); // BEQ falls through
        count.step(timing::IMM8); // LDA #$29
        count.step(timing::TAKEN); // BNE
        0x29
    };
    count.step(timing::LONG8); // STA $7E:3001,X
    count.step(timing::RTL);
    Trace {
        cost: count.cost,
        instructions: count.instructions,
        out,
        dp_0e,
        dp_0f,
        dp_10: Some(entry.value),
        dp_04: Some(entry.word),
        tiles: Some([entry.q, second]),
        x: sum,
        a: second,
        n: false,
        z: false,
        c: entry.q != 0,
        v: overflow,
    }
}

/// The value stored to `$0D64` leads to the tail when it is not zero. A zero value returns from the
/// routine at once: that path is not one the replays take, so it is left to the original code.
fn store_then_tail(
    mut count: Count,
    entry: &Entry,
    value: u8,
    dp_0e: Option<u8>,
    dp_0f: Option<u8>,
) -> Option<Trace> {
    count.step(timing::ABS8); // STA $0D64
    if value == 0 {
        return None;
    }
    count.step(timing::NOT_TAKEN); // BEQ falls through
    Some(main_tail(count, entry, value, dp_0e, dp_0f))
}

/// The path the routine takes for an entry, with its cost, instructions and results; `None` when the
/// path is not one the reference replays take (the original code runs it instead).
pub(crate) fn trace(entry: &Entry) -> Option<Trace> {
    if entry.index == INDEX_SKIPPED {
        return None;
    }
    let mut count = Count::default();
    // LDA #$00; XBA; LDA $0D66; CMP #$33; BEQ (not taken); ASL A; CLC; ADC $0D66; TAX; LDA record.
    count.step(timing::IMM8);
    count.step(timing::XBA);
    count.step(timing::ABS8);
    count.compare_and_branch(false);
    count.step(timing::IMPLIED);
    count.step(timing::IMPLIED);
    count.step(timing::ABS8);
    count.step(timing::IMPLIED);
    count.step(timing::LONG8);
    if entry.record_byte & 0x80 == 0 {
        count.step(timing::NOT_TAKEN); // BMI falls through
        count.step(timing::ABS8); // STA $0D64
        if entry.record_byte == 0 {
            // The value is zero: BEQ returns from the routine with nothing else written.
            count.step(timing::TAKEN);
            count.step(timing::RTL);
            let (_, carry, overflow) = adc8(entry.index << 1, entry.index);
            return Some(Trace {
                cost: count.cost,
                instructions: count.instructions,
                out: 0,
                dp_0e: None,
                dp_0f: None,
                dp_10: None,
                dp_04: None,
                tiles: None,
                x: (u16::from(entry.index) * 3) & 0xFF,
                a: 0,
                n: false,
                z: true,
                c: carry,
                v: overflow,
            });
        }
        count.step(timing::NOT_TAKEN); // BEQ falls through
        return Some(main_tail(count, entry, entry.record_byte, None, None));
    }
    // Bit 7 of the record byte is set: the special indexes.
    count.step(timing::TAKEN); // BMI taken
    count.step(timing::ABS8); // LDA $0D66
    let index = entry.index;
    if INDEX_BY_01BB.contains(&index) {
        count.compare_and_branch(index == INDEX_BY_01BB[0]);
        if index != INDEX_BY_01BB[0] {
            count.compare_and_branch(false); // CMP #$0B
            count.compare_and_branch(index == INDEX_BY_01BB[1]);
        }
        count.step(timing::ABS8); // LDA $01BB
        count.step(timing::TAKEN); // BRA
        return store_then_tail(count, entry, entry.param_01bb, None, None);
    }
    if index == INDEX_BY_01B3 {
        count.compare_and_branch(false); // CMP #$02
        count.compare_and_branch(true); // CMP #$0B
        count.step(timing::ABS8); // LDA $01B3
        count.step(timing::JMP); // JMP
        return store_then_tail(count, entry, entry.param_01b3, None, None);
    }
    if index == INDEX_BY_0C34 {
        for other in [0x02, 0x0B, 0x14, 0x26, 0x28, 0x13] {
            count.compare_and_branch(index == other);
        }
        count.compare_and_branch(true); // CMP #$2D
        count.step(timing::ABS8); // LDA $0C34
        count.step(timing::JMP); // JMP
        return store_then_tail(count, entry, entry.param_0c34, None, None);
    }
    if index == INDEX_BY_BYTE_TABLE {
        for other in [0x02, 0x0B, 0x14] {
            count.compare_and_branch(index == other);
        }
        count.compare_and_branch(true); // CMP #$26
        // LDA #$00; XBA; LDA $0D63; PHX; TAX; LDA $01:B9E3,X; STA $0F; PLX.
        count.step(timing::IMM8);
        count.step(timing::XBA);
        count.step(timing::ABS8);
        count.step(timing::PHX);
        count.step(timing::IMPLIED);
        count.step(timing::LONG8);
        count.step(timing::DP8);
        count.step(timing::PLX);
        // LDA $01BB; CMP $0F; BCS (not taken: the value is below $0F).
        let below = entry.param_01bb < entry.table_byte;
        count.step(timing::ABS8);
        count.step(timing::DP8);
        count.step(if below { timing::NOT_TAKEN } else { timing::TAKEN });
        if !below {
            return None;
        }
        // STZ $0E; LDA $01BB; CMP #$04; BCC (taken: below 4).
        count.step(timing::DP8);
        count.step(timing::ABS8);
        count.compare_and_branch(entry.param_01bb < 4);
        if entry.param_01bb >= 4 {
            return None;
        }
        // LDA $01BB; SEC; SBC $0E; AND #$03; JMP.
        count.step(timing::ABS8);
        count.step(timing::IMPLIED);
        count.step(timing::DP8);
        count.step(timing::IMM8);
        count.step(timing::JMP);
        let value = entry.param_01bb & 0x03;
        return store_then_tail(count, entry, value, Some(0), Some(entry.table_byte));
    }
    // The other special indexes ($28, $13 and the rest) are not seen in the replays.
    None
}

/// Reads the entry the routine works on, without side effects.
pub(crate) fn read_entry(ctx: &Ctx<'_>) -> Entry {
    let index = ctx.peek_db8(ENTRY_INDEX);
    let value = ctx.peek_db8(ENTRY_VALUE);
    let flag = ctx.peek_db8(ENTRY_FLAG);
    let record = (u32::from(index) * 3) & 0xFF;
    let word_offset = (u32::from(value) * 2) & 0xFF;
    let word_base = if flag & 0x80 != 0 { WORD_TABLE + WORD_TABLE_FLAGGED } else { WORD_TABLE };
    Entry {
        index,
        value,
        flag,
        q: ctx.peek_dp8(0),
        param_01bb: ctx.peek_db8(PARAM_01BB),
        param_01b3: ctx.peek_db8(PARAM_01B3),
        param_0c34: ctx.peek_db8(PARAM_0C34),
        table_byte: ctx.peek8(BYTE_TABLE + u32::from(ctx.peek_db8(ENTRY_0D63))),
        record_byte: ctx.peek8(RECORD_TABLE + record),
        record_word: ctx.peek16(RECORD_TABLE + record + 1),
        word: ctx.peek16(word_base + word_offset),
    }
}

/// `draw_map_entry_tiles`: the map entry's routes of [`trace`] run in Rust; the others run the
/// original code from the entry, before anything is written.
pub(crate) fn draw_map_entry_tiles(ctx: &mut Ctx<'_>) {
    let entry = read_entry(ctx);
    let Some(result) = trace(&entry) else {
        let Ok(()) = ctx.call_original(MAP_ENTRY_ENTRY, ReturnKind::Rtl) else { return };
        ctx.set_cost(0);
        return;
    };
    ctx.write_db8(ENTRY_OUT, result.out);
    if let Some(byte) = result.dp_0e {
        ctx.write_dp8(0x0E, byte);
    }
    if let Some(byte) = result.dp_0f {
        ctx.write_dp8(0x0F, byte);
    }
    if let Some(byte) = result.dp_10 {
        ctx.write_dp8(0x10, byte);
    }
    if let Some(word) = result.dp_04 {
        ctx.write_dp16(0x04, word);
    }
    if let Some([first, second]) = result.tiles {
        ctx.write8(TILE_BASE + u32::from(result.x), first);
        ctx.write8(TILE_BASE + u32::from(result.x) + 1, second);
    }
    ctx.set_x(result.x);
    ctx.set_a(u16::from(result.a));
    ctx.set_n(result.n);
    ctx.set_z(result.z);
    ctx.set_c(result.c);
    ctx.set_v(result.v);
    ctx.set_instructions(result.instructions);
    ctx.set_cost(result.cost);
}

/// The cost bound of `draw_map_entry_tiles`: the cost of the route the entry takes, or `u64::MAX`
/// for a route left to the original code.
pub(crate) fn draw_map_entry_tiles_bound(ctx: &Ctx<'_>) -> u64 {
    trace(&read_entry(ctx)).map_or(u64::MAX, |result| result.cost)
}

#[cfg(test)]
mod tests_map_entry {
    use super::{Entry, trace};
    use crate::routines::tests::max_cost;

    /// An entry on the table path: the record byte of the index, no special parameters, the given flag and
    /// the given `$00`.
    fn entry(index: u8, record_byte: u8, flag: u8, q: u8) -> Entry {
        Entry {
            index,
            value: 0x05,
            flag,
            q,
            param_01bb: 0x02,
            param_01b3: 0x05,
            param_0c34: 0x05,
            table_byte: 0x40,
            record_byte,
            record_word: 0x1234,
            word: 0x0042,
        }
    }

    fn cost(entry: &Entry) -> u64 {
        trace(entry).expect("a ported route").cost
    }

    #[test]
    fn the_cost_is_the_measured_one_for_each_observed_route() {
        assert_eq!(cost(&entry(0x10, 0x00, 0x00, 1)), 328, "a zero value returns at once");
        assert_eq!(cost(&entry(0x10, 0x01, 0x00, 1)), 1014);
        assert_eq!(cost(&entry(0x10, 0x01, 0x80, 1)), 1030);
        assert_eq!(cost(&entry(0x10, 0x01, 0x00, 2)), 1030);
        assert_eq!(cost(&entry(0x10, 0x01, 0x80, 2)), 1046);
        assert_eq!(cost(&entry(0x02, 0x80, 0x00, 1)), 1144, "index 2 by $01BB");
        assert_eq!(cost(&entry(0x02, 0x80, 0x80, 1)), 1160);
        assert_eq!(cost(&entry(0x14, 0x80, 0x00, 1)), 1208, "index $14 by $01BB");
        assert_eq!(cost(&entry(0x14, 0x80, 0x80, 1)), 1224);
        assert_eq!(cost(&entry(0x0B, 0x80, 0x80, 1)), 1194, "index $0B by $01B3");
        assert_eq!(cost(&entry(0x2D, 0x80, 0x80, 1)), 1354, "index $2D by $0C34");
        assert_eq!(cost(&entry(0x26, 0x80, 0x00, 1)), 1674, "index $26 below $0F and 4");
        assert_eq!(cost(&entry(0x26, 0x80, 0x80, 1)), 1690);
    }

    #[test]
    fn the_largest_ported_route_is_the_index_26_route_with_a_high_second_byte() {
        assert_eq!(cost(&entry(0x26, 0x80, 0x80, 0)), 1706);
        assert_eq!(max_cost("draw_map_entry_tiles"), 1706);
    }

    #[test]
    fn the_instructions_follow_the_routes() {
        assert_eq!(trace(&entry(0x10, 0x00, 0x00, 1)).map(|t| t.instructions), Some(14));
        assert_eq!(trace(&entry(0x10, 0x01, 0x00, 1)).map(|t| t.instructions), Some(41));
        assert_eq!(trace(&entry(0x10, 0x01, 0x80, 1)).map(|t| t.instructions), Some(42));
        assert_eq!(trace(&entry(0x10, 0x01, 0x80, 2)).map(|t| t.instructions), Some(43));
    }

    #[test]
    fn the_routes_the_replays_do_not_take_are_left_to_the_original() {
        assert!(trace(&entry(0x33, 0x01, 0x00, 1)).is_none(), "index $33 returns at once");
        assert!(trace(&entry(0x28, 0x80, 0x00, 1)).is_none(), "index $28");
        assert!(trace(&entry(0x13, 0x80, 0x00, 1)).is_none(), "index $13");
        assert!(trace(&entry(0x44, 0x80, 0x00, 1)).is_none(), "any other index with bit 7 set");
        let mut zero = entry(0x02, 0x80, 0x00, 1);
        zero.param_01bb = 0;
        assert!(trace(&zero).is_none(), "a zero value after index 2");
        let mut above = entry(0x26, 0x80, 0x00, 1);
        above.param_01bb = 0x40;
        assert!(trace(&above).is_none(), "index $26 with the byte at least $0F");
        let mut four = entry(0x26, 0x80, 0x00, 1);
        four.param_01bb = 0x04;
        assert!(trace(&four).is_none(), "index $26 with the byte at least 4");
    }

    #[test]
    fn index_26_writes_the_scratch_bytes_of_its_route() {
        let mut two = entry(0x26, 0x80, 0x00, 1);
        two.param_01bb = 0x02;
        let t = trace(&two).expect("the route");
        assert_eq!((t.dp_0e, t.dp_0f, t.out), (Some(0), Some(0x40), 0x02));
    }
}
