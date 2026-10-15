//! Batch F of the M2 porting: a copy of four words into the direct page, a return-only routine,
//! the follow steps of the two axes and the travel update, a tile-map block, slot and tile-map
//! writers, two counters, a table lookup, a copy from bank `$09` and a 16-bit step. Each
//! routine's semantics, contract and callers are in `docs/re/routines/<name>.md`.
//!
//! Routines whose cost or result depends on a branch take a path only where the reference
//! replays take it (see each routine's `*_cost` table or `OBSERVED_*` list): any other path runs
//! the original code, through the cost bound (`u64::MAX` declines the call) or `Ctx::call_original`.

use crate::data;
use ssd_oracle::{Ctx, ReturnKind};

// ---- copy_four_words_to_dp: four words of low WRAM into the direct page ----

/// The four words the routine reads (`follow_block_17d7`), low WRAM read through the data bank.
const FOLLOW_BLOCK: u16 = (data::FOLLOW_BLOCK_17D7 & 0xFFFF) as u16;
/// The direct-page offsets the words go to, in the order they are read: the word at block offset
/// `2 * n` goes to `TARGETS[n]`.
const TARGETS: [u16; 4] = [0x37, 0x39, 0x33, 0x35];
/// CPU-busy master cycles of a call, from the hit to the return: one path for every call
/// (measured in `Verify`).
pub(crate) const COPY_FOUR_WORDS_TO_DP_COST: u64 = 330;

/// `copy_four_words_to_dp`: the four words at `DBR:17D7`, `17D9`, `17DB` and `17DD` go to the
/// direct page at `$37`, `$39`, `$33` and `$35`, in that order. X ends as the last word read,
/// and N and Z are set from it; the accumulator, C and V are unchanged.
pub(crate) fn copy_four_words_to_dp(ctx: &mut Ctx<'_>) {
    let mut last = 0;
    for (index, &target) in TARGETS.iter().enumerate() {
        last = ctx.read_db16(FOLLOW_BLOCK + 2 * index as u16);
        ctx.set_x(last);
        ctx.write_dp16(target, last);
    }
    ctx.set_nz16(last);
    ctx.set_cost(COPY_FOUR_WORDS_TO_DP_COST);
}

#[cfg(test)]
mod copy_four_words_to_dp_tests {
    use super::*;
    use crate::routines::tests::max_cost;

    #[test]
    fn copy_four_words_to_dp_cost_is_constant() {
        assert_eq!(max_cost("copy_four_words_to_dp"), COPY_FOUR_WORDS_TO_DP_COST);
    }

    #[test]
    fn the_four_words_come_from_the_block_in_order() {
        assert_eq!((data::FOLLOW_BLOCK_17D7, data::FOLLOW_BLOCK_17D7_SIZE), (0x7E_17D7, 8));
        assert_eq!(FOLLOW_BLOCK, 0x17D7);
        assert_eq!(TARGETS, [0x37, 0x39, 0x33, 0x35]);
    }
}

// ---- empty_return: a routine that does nothing but return ----

/// CPU-busy master cycles of a call, from the hit to the return: one path for every call
/// (measured in `Verify`).
pub(crate) const EMPTY_RETURN_COST: u64 = 42;

/// `empty_return`: nothing changes; the routine only returns.
pub(crate) fn empty_return(ctx: &mut Ctx<'_>) {
    ctx.set_cost(EMPTY_RETURN_COST);
}

#[cfg(test)]
mod empty_return_tests {
    use super::*;
    use crate::routines::tests::max_cost;

    #[test]
    fn empty_return_cost_is_constant() {
        assert_eq!(max_cost("empty_return"), EMPTY_RETURN_COST);
    }
}

// ---- follow_step_x: the X follower moves towards its leader, keeping a rest distance of $80 ----

/// The follower's X limit byte (`follow_limit_x`), through the data bank.
const FOLLOW_LIMIT_X: u16 = (data::FOLLOW_LIMIT_X & 0xFFFF) as u16;
/// The leader's X position (`follow_target_x`, a word), through the data bank.
const FOLLOW_TARGET_X: u16 = (data::FOLLOW_TARGET_X & 0xFFFF) as u16;
/// The follow flags byte (`follow_flags`), through the data bank.
const FOLLOW_FLAGS: u16 = (data::FOLLOW_FLAGS & 0xFFFF) as u16;
/// The entry of the routine (`$06:EDB3`), for the hand-back to the original code.
const FOLLOW_X_ENTRY: u32 = 0x06_EDB3;

/// 8-bit `a + b + carry`: the result, C and V.
pub(crate) fn adc8(a: u8, b: u8, carry: bool) -> (u8, bool, bool) {
    let sum = u16::from(a) + u16::from(b) + u16::from(carry);
    let result = sum as u8;
    let overflow = (a ^ result) & (b ^ result) & 0x80 != 0;
    (result, sum > 0xFF, overflow)
}

/// 8-bit `a - b` with the carry as SBC takes it (set: no borrow in): the result, C (no borrow
/// out) and V.
pub(crate) fn sbc8(a: u8, b: u8, carry: bool) -> (u8, bool, bool) {
    adc8(a, !b, carry)
}

/// The inputs of one call of `follow_step_x`, read on entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FollowXInput {
    /// The leader's X position (`follow_target_x`, a word).
    pub leader: u16,
    /// The follower's X position (`follow_pos_x`, a word: direct page `$33`).
    pub position: u16,
    /// The limit byte of the X follower (`follow_limit_x`).
    pub limit: u8,
    /// The follow flags before the call (`follow_flags`).
    pub flags: u8,
    /// The high byte of A on entry (B), which the routine keeps.
    pub entry_b: u8,
}

/// The branches a call of `follow_step_x` takes: its cost depends on them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FollowKey {
    /// The leader is at the rest distance: the routine returns at once.
    Rest,
    /// A step back. `negative`: the high byte of the difference is negative, so the step is four;
    /// otherwise the distance is below the rest distance, and `measured` says whether the
    /// distance itself is the step (below four) or the step is four.
    Back { negative: bool, measured: bool, zeroed: bool, tail: FollowTail },
    /// A step forward. `positive`: the high byte of the difference is positive, so the step is
    /// four; otherwise the distance is above the rest distance, `measured` as for `Back`.
    Forward { positive: bool, measured: bool, zeroed: bool, tail: FollowTail },
}

/// The end of a step, by bit 2 of the new low byte and bit 0 of the stored flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FollowTail {
    /// Bit 2 clear and bit 0 clear: bits 0 and 1 of the flags are cleared.
    ClearLow,
    /// Bit 2 clear and bit 0 set: the flags are kept.
    KeepSet,
    /// Bit 2 set and bit 0 clear: the flags are kept.
    KeepClear,
    /// Bit 2 set and bit 0 set: bits 0 and 1 of the flags are cleared.
    ClearHigh,
}

/// What `follow_step_x` leaves for its inputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FollowX {
    pub key: FollowKey,
    /// The low byte of A (B is kept).
    pub a_low: u8,
    /// The direct-page byte `$00` after the call.
    pub dp00: u8,
    /// The new position, written to `$33-$34` and to the scratch bytes `$10-$11`; `None` when the
    /// position stays.
    pub position: Option<(u8, u8)>,
    /// The follow flags written to `$0700`; `None` when not written.
    pub flags: Option<u8>,
    pub n: bool,
    pub z: bool,
    pub c: bool,
    pub v: bool,
}

/// The end of a step: the flags it stores, and the flags it leaves with the branch it takes.
fn follow_tail(
    new_lo: u8,
    new_hi: u8,
    or_value: u8,
    flags_old: u8,
    v: bool,
) -> (FollowX, FollowTail) {
    let stored = or_value | flags_old;
    let bit2 = new_lo & 0x04 != 0;
    let bit0 = stored & 0x01 != 0;
    let (tail, cleared) = match (bit2, bit0) {
        (false, false) => (FollowTail::ClearLow, true),
        (false, true) => (FollowTail::KeepSet, false),
        (true, false) => (FollowTail::KeepClear, false),
        (true, true) => (FollowTail::ClearHigh, true),
    };
    let flags = if cleared { stored & 0xFC } else { stored };
    // The clearing routes leave the cleared flags in A, the others the flags shifted right.
    let a_low = if cleared { flags } else { stored >> 1 };
    let result = FollowX {
        key: FollowKey::Rest,
        a_low,
        dp00: 0,
        position: Some((new_lo, new_hi)),
        flags: Some(flags),
        n: a_low & 0x80 != 0,
        z: a_low == 0,
        c: bit0,
        v,
    };
    (result, tail)
}

/// The step of `follow_step_x` for the inputs: the branches it takes and what it leaves.
pub(crate) fn plan_follow_x(input: FollowXInput) -> FollowX {
    let [lead_lo, lead_hi] = input.leader.to_le_bytes();
    let [pos_lo, pos_hi] = input.position.to_le_bytes();
    let (d_lo, carry_lo, _) = sbc8(lead_lo, pos_lo, true);
    let (d_hi, _, v_hi) = sbc8(lead_hi, pos_hi, carry_lo);
    if d_hi == 0 && d_lo == 0x80 {
        // At the rest distance the compare with $80 leaves N clear and Z and C set; V is the one
        // of the high byte's subtraction.
        return FollowX {
            key: FollowKey::Rest,
            a_low: d_lo,
            dp00: d_lo,
            position: None,
            flags: None,
            n: false,
            z: true,
            c: true,
            v: v_hi,
        };
    }
    // A negative high byte steps back by four and a positive one forward by four. With a zero
    // high byte the distance from the rest distance is the step when below four, else four; a
    // distance below the rest distance steps back, one above it steps forward.
    if d_hi & 0x80 != 0 {
        plan_back(input, pos_lo, pos_hi, 4, true, false)
    } else if d_hi != 0 {
        plan_forward(input, d_lo, pos_lo, pos_hi, 4, true, false)
    } else if d_lo >= 0x81 {
        let (over, _, _) = sbc8(d_lo, 0x80, true);
        if over < 4 {
            plan_forward(input, d_lo, pos_lo, pos_hi, over, false, true)
        } else {
            plan_forward(input, d_lo, pos_lo, pos_hi, 4, false, false)
        }
    } else {
        let (under, _, _) = sbc8(0x80, d_lo, true);
        if under < 4 {
            plan_back(input, pos_lo, pos_hi, under, false, true)
        } else {
            plan_back(input, pos_lo, pos_hi, 4, false, false)
        }
    }
}

/// The step back of [`plan_follow_x`]: the position goes down by `step`, and is zeroed when that
/// would take it below zero. `$00` ends as the step.
fn plan_back(
    input: FollowXInput,
    pos_lo: u8,
    pos_hi: u8,
    step: u8,
    negative: bool,
    measured: bool,
) -> FollowX {
    let (lo, carry_lo, _) = sbc8(pos_lo, step, true);
    let (hi, carry_hi, v) = sbc8(pos_hi, 0, carry_lo);
    let zeroed = !carry_hi;
    let (new_lo, new_hi) = if zeroed { (0, 0) } else { (lo, hi) };
    let (mut result, tail) = follow_tail(new_lo, new_hi, 0x03, input.flags, v);
    result.dp00 = step;
    result.key = FollowKey::Back { negative, measured, zeroed, tail };
    result
}

/// The step forward of [`plan_follow_x`]: the position goes up by `step`, and its low byte is
/// zeroed when its high byte reaches the limit minus one. `$00` keeps the distance.
fn plan_forward(
    input: FollowXInput,
    d_lo: u8,
    pos_lo: u8,
    pos_hi: u8,
    step: u8,
    positive: bool,
    measured: bool,
) -> FollowX {
    let (sum_lo, carry_lo, _) = adc8(step, pos_lo, false);
    let (sum_hi, _, _) = adc8(0, pos_hi, carry_lo);
    let (limit, _, v) = sbc8(input.limit, 1, true);
    let zeroed = limit == sum_hi;
    let new_lo = if zeroed { 0 } else { sum_lo };
    let (mut result, tail) = follow_tail(new_lo, sum_hi, 0x02, input.flags, v);
    result.dp00 = d_lo;
    result.key = FollowKey::Forward { positive, measured, zeroed, tail };
    result
}

/// CPU-busy master cycles of the branches the reference replays take, from the hit to the return
/// (measured in `Verify`); `None` for the others, whose calls run the original code.
pub(crate) fn follow_cost(key: FollowKey) -> Option<u64> {
    use FollowTail::{ClearHigh, ClearLow, KeepClear, KeepSet};
    let cost = match key {
        FollowKey::Rest => 314,
        FollowKey::Back { negative: false, measured: true, zeroed: false, tail: ClearHigh } => 986,
        FollowKey::Back { negative: false, measured: true, zeroed: false, tail: KeepSet } => 922,
        FollowKey::Forward { positive: false, measured: false, zeroed: true, tail: ClearLow } => {
            1014
        }
        FollowKey::Forward { positive: false, measured: true, zeroed: false, tail: ClearLow } => {
            970
        }
        FollowKey::Forward { positive: false, measured: true, zeroed: false, tail: KeepClear } => {
            896
        }
        FollowKey::Forward { positive: false, measured: true, zeroed: true, tail: ClearLow } => {
            1004
        }
        _ => return None,
    };
    Some(cost)
}

/// The inputs of [`plan_follow_x`] read from the machine.
fn follow_input(ctx: &Ctx<'_>) -> FollowXInput {
    FollowXInput {
        leader: ctx.peek_db16(FOLLOW_TARGET_X),
        position: ctx.peek_dp16(0x33),
        limit: ctx.peek_db8(FOLLOW_LIMIT_X),
        flags: ctx.peek_db8(FOLLOW_FLAGS),
        entry_b: (ctx.a() >> 8) as u8,
    }
}

/// The instructions the original code runs on the branch of `key`, from the hit to the return (the
/// return included): declared with `set_instructions`, as the count varies with the branch.
pub(crate) fn follow_instructions(key: FollowKey) -> u64 {
    match key {
        FollowKey::Rest => 14,
        FollowKey::Back { negative, measured, zeroed, tail } => {
            // Up to the step's first instruction: the negative high byte, or the distance below
            // the rest distance (its step is the distance when `measured`, else four).
            let before = if negative {
                8
            } else if measured {
                18
            } else {
                19
            };
            let arithmetic = if zeroed { 13 } else { 11 };
            before + arithmetic + tail_instructions(tail)
        }
        FollowKey::Forward { positive, measured, zeroed, tail } => {
            let before = match (positive, measured, zeroed) {
                (true, _, false) => 21,
                (true, _, true) => 23,
                (false, true, false) => 28,
                (false, true, true) => 30,
                (false, false, false) => 29,
                (false, false, true) => 31,
            };
            before + tail_instructions(tail)
        }
    }
}

/// The instructions from the end of the step's arithmetic to the return, by the way the step ends.
fn tail_instructions(tail: FollowTail) -> u64 {
    match tail {
        FollowTail::ClearLow | FollowTail::ClearHigh => 15,
        FollowTail::KeepSet => 13,
        FollowTail::KeepClear => 12,
    }
}

/// The cost bound of `follow_step_x`: the cost of its branch, `u64::MAX` (decline) for a branch
/// the replays do not take.
pub(crate) fn follow_step_x_bound(ctx: &Ctx<'_>) -> u64 {
    follow_cost(plan_follow_x(follow_input(ctx)).key).unwrap_or(u64::MAX)
}

/// `follow_step_x`: the X follower steps as [`plan_follow_x`] gives. `$00` ends as the plan leaves
/// it, `$10-$11` and `$33-$34` hold the new position, `$0700` the flags it stores; A keeps its
/// high byte, and N, Z, C and V are the plan's.
pub(crate) fn follow_step_x(ctx: &mut Ctx<'_>) {
    let plan = plan_follow_x(follow_input(ctx));
    let Some(cost) = follow_cost(plan.key) else {
        let Ok(()) = ctx.call_original(FOLLOW_X_ENTRY, ReturnKind::Rtl) else { return };
        ctx.set_cost(0);
        return;
    };
    ctx.write_dp8(0x00, plan.dp00);
    if let Some((lo, hi)) = plan.position {
        ctx.write_dp8(0x10, lo);
        ctx.write_dp8(0x11, hi);
        ctx.write_dp8(0x33, lo);
        ctx.write_dp8(0x34, hi);
    }
    if let Some(flags) = plan.flags {
        ctx.write_db8(FOLLOW_FLAGS, flags);
    }
    ctx.set_al(plan.a_low);
    ctx.set_n(plan.n);
    ctx.set_z(plan.z);
    ctx.set_c(plan.c);
    ctx.set_v(plan.v);
    ctx.set_instructions(follow_instructions(plan.key));
    ctx.set_cost(cost);
}

#[cfg(test)]
mod follow_step_x_tests {
    use super::*;

    fn input(leader: u16, position: u16, limit: u8, flags: u8) -> FollowXInput {
        FollowXInput { leader, position, limit, flags, entry_b: 0x0A }
    }

    #[test]
    fn at_the_rest_distance_nothing_moves() {
        let plan = plan_follow_x(input(0x0180, 0x0100, 9, 0));
        assert_eq!(plan.key, FollowKey::Rest);
        assert_eq!((plan.a_low, plan.dp00, plan.position, plan.flags), (0x80, 0x80, None, None));
        assert_eq!((plan.n, plan.z, plan.c, plan.v), (false, true, true, false));
    }

    #[test]
    fn a_distance_just_above_the_rest_distance_steps_forward_by_its_excess() {
        let plan = plan_follow_x(input(0x0182, 0x0100, 9, 0));
        assert_eq!(
            plan.key,
            FollowKey::Forward {
                positive: false,
                measured: true,
                zeroed: false,
                tail: FollowTail::ClearLow
            }
        );
        assert_eq!(plan.dp00, 0x82, "the distance stays in $00");
        assert_eq!(plan.position, Some((0x02, 0x01)));
        assert_eq!(plan.flags, Some(0));
        assert_eq!((plan.a_low, plan.n, plan.z, plan.c, plan.v), (0, false, true, false, false));
    }

    #[test]
    fn a_step_below_zero_is_zeroed_and_keeps_the_flags_bit_zero() {
        let plan = plan_follow_x(input(0x0000, 0x0000, 9, 0));
        assert_eq!(
            plan.key,
            FollowKey::Back {
                negative: false,
                measured: false,
                zeroed: true,
                tail: FollowTail::KeepSet
            }
        );
        assert_eq!((plan.dp00, plan.position, plan.flags), (4, Some((0, 0)), Some(3)));
        assert_eq!((plan.a_low, plan.n, plan.z, plan.c, plan.v), (1, false, false, true, false));
    }

    #[test]
    fn the_stored_flags_pick_the_end_of_the_step() {
        // Bit 2 of the new low byte and bit 0 of the flags: four ends.
        let clear_low = plan_follow_x(input(0x0182, 0x0100, 9, 0));
        let keep_set = plan_follow_x(input(0x0182, 0x0100, 9, 1));
        assert_eq!(clear_low.flags, Some(0));
        assert_eq!(keep_set.flags, Some(3), "bit 0 set: the flags are kept, with the step's bit");
        assert_eq!(keep_set.a_low, 1, "A is the flags shifted right");
    }

    #[test]
    fn the_instruction_counts_follow_the_branches_of_the_original() {
        assert_eq!(follow_instructions(FollowKey::Rest), 14);
        let back = |tail| FollowKey::Back { negative: false, measured: true, zeroed: false, tail };
        assert_eq!(follow_instructions(back(FollowTail::ClearHigh)), 44);
        assert_eq!(follow_instructions(back(FollowTail::KeepSet)), 42);
        let forward =
            |zeroed, tail| FollowKey::Forward { positive: false, measured: true, zeroed, tail };
        assert_eq!(follow_instructions(forward(false, FollowTail::KeepClear)), 40);
        assert_eq!(follow_instructions(forward(false, FollowTail::ClearLow)), 43);
        assert_eq!(follow_instructions(forward(true, FollowTail::ClearLow)), 45);
        let unmeasured = FollowKey::Forward {
            positive: false,
            measured: false,
            zeroed: true,
            tail: FollowTail::ClearLow,
        };
        assert_eq!(follow_instructions(unmeasured), 46);
    }

    #[test]
    fn the_cost_table_declines_the_branches_the_replays_do_not_take() {
        assert_eq!(follow_cost(FollowKey::Rest), Some(314));
        let clamped = FollowKey::Back {
            negative: true,
            measured: false,
            zeroed: false,
            tail: FollowTail::ClearLow,
        };
        assert_eq!(follow_cost(clamped), None);
    }
}

// ---- fill_tilemap_quad: a 4 x 4 block of tile words in the tile-map buffer ----

/// The tile-map buffer the block is written in (`tilemap_buffer_4000`), addressed from bank `$7E`.
const TILEMAP: u32 = data::TILEMAP_BUFFER_4000;
/// CPU-busy master cycles of a call, from the hit to the return: one path for every call
/// (measured in `Verify`).
pub(crate) const FILL_TILEMAP_QUAD_COST: u64 = 5060;

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

/// What `fill_tilemap_quad` leaves: the sixteen words it stores, as (tile-map offset, value) in
/// the order it stores them; the row base, the value counter and the loop counter it ends with;
/// the index after its last store; and V, as its last addition or subtraction left it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct QuadFill {
    pub writes: [(u16, u16); 16],
    pub base: u16,
    pub value: u16,
    pub counter: u16,
    pub index: u16,
    pub overflow: bool,
}

/// The block of [`fill_tilemap_quad`] for the row base `base`, the value counter `value` and the
/// pattern `pattern`. Four rows of four words are stored, each word the value counter plus the
/// pattern; the counter goes up by one per word. The row base steps by `$80` after the first and
/// the third row; after the second row it steps back by `$40` and the value counter by `$8`. The
/// fourth row ends the routine before any step. The loop counter starts at three and ends at `$FFFF`.
pub(crate) fn plan_quad_fill(base: u16, value: u16, pattern: u16) -> QuadFill {
    let mut fill =
        QuadFill { writes: [(0, 0); 16], base, value, counter: 3, index: base, overflow: false };
    let mut stored = 0;
    loop {
        let mut index = fill.base;
        for _ in 0..4 {
            fill.writes[stored] = (index, fill.value | pattern);
            stored += 1;
            index = index.wrapping_add(2);
            fill.value = fill.value.wrapping_add(1);
        }
        fill.index = index;
        let odd = fill.counter & 1 != 0;
        fill.counter = fill.counter.wrapping_sub(1);
        if odd {
            let (sum, _, overflow) = add16(fill.base, 0x0080);
            fill.base = sum;
            fill.overflow = overflow;
        } else {
            if fill.counter & 0x8000 != 0 {
                return fill;
            }
            fill.value = fill.value.wrapping_add(8);
            let (difference, _, overflow) = sub16(fill.base, 0x0040);
            fill.base = difference;
            fill.overflow = overflow;
        }
    }
}

/// `fill_tilemap_quad`: the sixteen words of [`plan_quad_fill`] at `$7E:4000` plus their offsets,
/// from the row base at `$20`, the value counter at `$22` and the pattern at `$02`. The
/// direct-page words `$00`, `$20` and `$22` end as the plan leaves them, and `$0D74` is `$80`. A is
/// `$0080`, X the index after the last store, Y is `$FFFF`; N is set, Z and C are clear, and V is
/// the plan's.
pub(crate) fn fill_tilemap_quad(ctx: &mut Ctx<'_>) {
    let fill = plan_quad_fill(ctx.read_dp16(0x20), ctx.read_dp16(0x22), ctx.read_dp16(0x02));
    for &(index, value) in &fill.writes {
        ctx.write16(TILEMAP + u32::from(index), value);
    }
    ctx.write_dp16(0x00, fill.counter);
    ctx.write_dp16(0x20, fill.base);
    ctx.write_dp16(0x22, fill.value);
    ctx.write_db8(0x0D74, 0x80);
    ctx.set_a(0x0080);
    ctx.set_x(fill.index);
    ctx.set_y(0xFFFF);
    ctx.set_n(true);
    ctx.set_z(false);
    ctx.set_c(false);
    ctx.set_v(fill.overflow);
    ctx.set_cost(FILL_TILEMAP_QUAD_COST);
}

#[cfg(test)]
mod fill_tilemap_quad_tests {
    use super::*;
    use crate::routines::tests::max_cost;

    #[test]
    fn fill_tilemap_quad_cost_is_constant() {
        assert_eq!(max_cost("fill_tilemap_quad"), FILL_TILEMAP_QUAD_COST);
    }

    #[test]
    fn the_four_rows_are_placed_by_their_steps() {
        let fill = plan_quad_fill(0x0020, 0x1000, 0x0100);
        let row = |base: u16, value: u16| -> Vec<(u16, u16)> {
            (0..4).map(|k| (base + 2 * k, (value + k) | 0x0100)).collect()
        };
        let mut expected = row(0x0020, 0x1000);
        expected.extend(row(0x00A0, 0x1004));
        expected.extend(row(0x0060, 0x1010));
        expected.extend(row(0x00E0, 0x1014));
        assert_eq!(fill.writes.to_vec(), expected);
    }

    #[test]
    fn the_block_leaves_its_counters_and_flags_as_the_routine_does() {
        let fill = plan_quad_fill(0x0020, 0x1000, 0x0100);
        assert_eq!((fill.base, fill.value, fill.counter), (0x00E0, 0x1018, 0xFFFF));
        assert_eq!(fill.index, 0x00E8, "X is the last row's base plus eight");
        assert!(!fill.overflow, "the last addition ($60 + $80) does not overflow");
    }

    #[test]
    fn the_last_step_can_overflow() {
        // Row three steps the base from $7FC0 by $80: a signed overflow, which the block leaves in V.
        let fill = plan_quad_fill(0x7F80, 0, 0);
        assert!(fill.overflow);
    }

    #[test]
    fn the_adders_are_the_cpus_16_bit_ones() {
        assert_eq!(add16(0xFFFF, 1), (0, true, false));
        assert_eq!(add16(0x7FFF, 1), (0x8000, false, true));
        assert_eq!(sub16(0x0000, 1), (0xFFFF, false, false));
        assert_eq!(sub16(0x8000, 1), (0x7FFF, true, true));
    }
}

// ---- write_slot_record: an eight-byte record in the slot block at X, through the data bank ----

/// Where the record starts: the slot block (`slot_block_0800`), low WRAM addressed as `$0800`.
const SLOT_BLOCK: u32 = data::SLOT_BLOCK_0800 & 0xFFFF;
/// The word the record takes from the direct page (`$00`).
const RECORD_SOURCE: u16 = 0x00;
/// The word the record takes from low WRAM (`word_0d77`), through the data bank.
const WORD_0D77: u16 = (data::WORD_0D77 & 0xFFFF) as u16;
/// CPU-busy master cycles of a call, from the hit to the return: one path for every call
/// (measured in `Verify`).
pub(crate) const WRITE_SLOT_RECORD_COST: u64 = 430;

/// The 24-bit address of byte `offset` of the record for index `x`: the data bank's `$0800` plus
/// the offset and the index, a carry out of the 16-bit sum going into the bank byte.
pub(crate) fn record_address(dbr: u8, x: u16, offset: u32) -> u32 {
    ((u32::from(dbr) << 16) + SLOT_BLOCK + offset + u32::from(x)) & 0xFF_FFFF
}

/// `write_slot_record`: writes eight bytes at the slot for X: `$80`; the word `$00` of the direct
/// page; the word `word_0d77`; `$7E`; and the word `$0100` at offset 6 (bytes `$00` and `$01`). A
/// ends as `$017E` (B is `$01`); N and Z are clear from `$7E`; C, V, X and Y are unchanged.
pub(crate) fn write_slot_record(ctx: &mut Ctx<'_>) {
    let x = ctx.x();
    let dbr = ctx.dbr();
    ctx.write8(record_address(dbr, x, 0), 0x80);
    let source = ctx.read_dp16(RECORD_SOURCE);
    ctx.write16(record_address(dbr, x, 1), source);
    let word = ctx.read_db16(WORD_0D77);
    ctx.write16(record_address(dbr, x, 3), word);
    ctx.write8(record_address(dbr, x, 5), 0x7E);
    ctx.write16(record_address(dbr, x, 6), 0x0100);
    ctx.set_a(0x017E);
    ctx.set_nz8(0x7E);
    ctx.set_cost(WRITE_SLOT_RECORD_COST);
}

#[cfg(test)]
mod write_slot_record_tests {
    use super::*;
    use crate::routines::tests::max_cost;

    #[test]
    fn write_slot_record_cost_is_constant() {
        assert_eq!(max_cost("write_slot_record"), WRITE_SLOT_RECORD_COST);
    }

    #[test]
    fn the_record_sits_at_the_slot_of_x_in_the_data_bank() {
        assert_eq!(record_address(0x00, 0x0010, 0), 0x00_0810);
        assert_eq!(record_address(0x00, 0x0010, 6), 0x00_0816);
        assert_eq!(record_address(0x7E, 0x0000, 3), 0x7E_0803);
    }

    #[test]
    fn the_address_carries_into_the_bank() {
        assert_eq!(record_address(0x00, 0xFFF8, 6), 0x01_07FE, "a carry goes into the bank");
        assert_eq!(record_address(0xFF, 0xFFFF, 7), 0x00_0806, "the bank wraps at 24 bits");
        assert_eq!((SLOT_BLOCK, WORD_0D77), (0x0800, 0x0D77));
    }
}

// ---- fill_word_run: a run of one word in bank `$7E`, from X, as long as the count in `$0E` ----

/// The run starts here (`$7E:3000`), at X.
const RUN_BASE: u32 = 0x7E_3000;
/// The entry of the routine (`$01:BC71`), for the hand-back to the original code.
const FILL_WORD_RUN_ENTRY: u32 = 0x01_BC71;
/// The counts the reference replays take. The cost bound declines every other count, so the
/// original code runs for it.
const OBSERVED_COUNTS: [u16; 2] = [11, 16];

/// The number of words the run stores for the byte `raw` at `$0E` (with `$0F` cleared): the loop
/// stores at least one.
pub(crate) fn run_count(raw: u8) -> u16 {
    u16::from(raw).max(1)
}

/// CPU-busy master cycles of a call that stores `count` words, from the hit to the return: a fixed
/// part and one loop pass per word (measured in `Verify` for the counts 11 and 16).
pub(crate) fn fill_word_run_cost(count: u16) -> u64 {
    162 + 144 * u64::from(count)
}

/// The instructions the original code runs for `count` words, from the hit to the return (the
/// return included): four before the loop, six per word, two after it. Declared with
/// `set_instructions`.
pub(crate) fn fill_word_run_instructions(count: u16) -> u64 {
    6 * u64::from(count) + 6
}

/// The cost bound of `fill_word_run`: the cost of its count when the reference replays take it,
/// `u64::MAX` (declined: the original runs) for any other.
pub(crate) fn fill_word_run_bound(ctx: &Ctx<'_>) -> u64 {
    let count = run_count(ctx.peek_dp8(0x0E));
    if OBSERVED_COUNTS.contains(&count) { fill_word_run_cost(count) } else { u64::MAX }
}

/// `fill_word_run`: `$0F` is cleared, then the word at `$10` is stored at `$7E:3000 + X` for the
/// count of [`run_count`] of `$0E`, two bytes further each time. X ends after the last word, Y is
/// the count and A the word. N is clear and Z is set when the count is the byte at `$0E`; C is set;
/// V is unchanged.
pub(crate) fn fill_word_run(ctx: &mut Ctx<'_>) {
    ctx.write_dp8(0x0F, 0);
    let raw = ctx.read_dp8(0x0E);
    let count = run_count(raw);
    if !OBSERVED_COUNTS.contains(&count) {
        let Ok(()) = ctx.call_original(FILL_WORD_RUN_ENTRY, ReturnKind::Rtl) else { return };
        ctx.set_cost(0);
        return;
    }
    let value = ctx.read_dp16(0x10);
    let start = ctx.x();
    for n in 0..count {
        let index = start.wrapping_add(2 * n);
        ctx.write16(RUN_BASE + u32::from(index), value);
    }
    ctx.set_x(start.wrapping_add(2 * count));
    ctx.set_y(count);
    ctx.set_a(value);
    ctx.set_nz16(count.wrapping_sub(u16::from(raw)));
    ctx.set_c(true);
    ctx.set_instructions(fill_word_run_instructions(count));
    ctx.set_cost(fill_word_run_cost(count));
}

#[cfg(test)]
mod fill_word_run_tests {
    use super::*;

    #[test]
    fn a_count_of_zero_stores_one_word() {
        assert_eq!(run_count(0), 1);
        assert_eq!(run_count(11), 11);
        assert_eq!(run_count(255), 255);
    }

    #[test]
    fn the_cost_is_linear_in_the_count_and_matches_the_measured_counts() {
        assert_eq!(fill_word_run_cost(11), 1746);
        assert_eq!(fill_word_run_cost(16), 2466);
    }

    #[test]
    fn the_instruction_count_is_six_per_word_and_six_more() {
        assert_eq!(fill_word_run_instructions(11), 72);
        assert_eq!(fill_word_run_instructions(16), 102);
    }

    #[test]
    fn only_the_observed_counts_are_ported() {
        assert_eq!(OBSERVED_COUNTS, [11, 16]);
        assert!(OBSERVED_COUNTS.contains(&run_count(11)));
        assert!(!OBSERVED_COUNTS.contains(&run_count(12)));
    }
}

// ---- advance_x_and_count: X steps by one, and two byte counters of low WRAM step by one ----

/// The first byte counter the routine increments (`counter_071d`), through the data bank.
const COUNTER_071D: u16 = (data::COUNTER_071D & 0xFFFF) as u16;
/// The second byte counter (`counter_071f`), through the data bank.
const COUNTER_071F: u16 = (data::COUNTER_071F & 0xFFFF) as u16;
/// CPU-busy master cycles of a call, from the hit to the return: one path for every call
/// (measured in `Verify`).
pub(crate) const ADVANCE_X_AND_COUNT_COST: u64 = 150;

/// Increments the byte at `DBR:address` and returns the result.
fn increment_byte(ctx: &mut Ctx<'_>, address: u16) -> u8 {
    let value = ctx.read_db8(address).wrapping_add(1);
    ctx.write_db8(address, value);
    value
}

/// `advance_x_and_count`: X goes up by one in the index width, then `$071D` and `$071F` each go up
/// by one (bytes). N and Z come from the value of `$071F`; C, V, A and Y are unchanged.
pub(crate) fn advance_x_and_count(ctx: &mut Ctx<'_>) {
    ctx.set_x(ctx.x().wrapping_add(1));
    increment_byte(ctx, COUNTER_071D);
    let last = increment_byte(ctx, COUNTER_071F);
    ctx.set_nz8(last);
    ctx.set_cost(ADVANCE_X_AND_COUNT_COST);
}

#[cfg(test)]
mod advance_x_and_count_tests {
    use super::*;
    use crate::routines::tests::max_cost;

    #[test]
    fn advance_x_and_count_cost_is_constant() {
        assert_eq!(max_cost("advance_x_and_count"), ADVANCE_X_AND_COUNT_COST);
    }

    #[test]
    fn the_two_counters_are_separate_bytes() {
        assert_eq!((data::COUNTER_071D, data::COUNTER_071D_SIZE), (0x7E_071D, 1));
        assert_eq!((data::COUNTER_071F, data::COUNTER_071F_SIZE), (0x7E_071F, 1));
    }
}

// ---- lookup_triple_index: a table byte picked by three times the byte at `$0D59` ----

/// The table the index picks from: the bank-`$01` table `bank1_index_table` from `$83BD` on, the
/// offset of the entry `2v + v` reaches (the table overlays the one `update_travel_and_follow`
/// reads from its start).
const TRIPLE_TABLE: u32 = data::BANK1_INDEX_TABLE + 0x1D;
/// The byte the routine reads (`code_byte_0d59`), through the data bank.
const INDEX_SOURCE: u16 = (data::CODE_BYTE_0D59 & 0xFFFF) as u16;
/// The byte the routine stores (`table_result_01e3`), through the data bank.
const TABLE_RESULT: u16 = (data::TABLE_RESULT_01E3 & 0xFFFF) as u16;
/// CPU-busy master cycles of a call, from the hit to the return: one path for every call
/// (measured in `Verify`).
pub(crate) const LOOKUP_TRIPLE_INDEX_COST: u64 = 258;

/// Three times `v` as the routine forms it: `2v` plus `v` in 8 bits, with the carry and the
/// overflow of that sum. The sum is the index, since the high byte of the index is zero.
pub(crate) fn triple_index(v: u8) -> (u8, bool, bool) {
    adc8(v << 1, v, false)
}

/// `lookup_triple_index`: `$01E3` gets the table byte at the index [`triple_index`] gives for the
/// byte at `$0D59`. A is that byte with B clear; N and Z are set from it; C and V are the sum's;
/// X is the index.
pub(crate) fn lookup_triple_index(ctx: &mut Ctx<'_>) {
    let (index, carry, overflow) = triple_index(ctx.read_db8(INDEX_SOURCE));
    let byte = ctx.read8(TRIPLE_TABLE + u32::from(index));
    ctx.write_db8(TABLE_RESULT, byte);
    ctx.set_x(u16::from(index));
    ctx.set_a(u16::from(byte));
    ctx.set_nz8(byte);
    ctx.set_c(carry);
    ctx.set_v(overflow);
    ctx.set_cost(LOOKUP_TRIPLE_INDEX_COST);
}

#[cfg(test)]
mod lookup_triple_index_tests {
    use super::*;
    use crate::routines::tests::max_cost;

    #[test]
    fn lookup_triple_index_cost_is_constant() {
        assert_eq!(max_cost("lookup_triple_index"), LOOKUP_TRIPLE_INDEX_COST);
    }

    #[test]
    fn the_index_is_three_times_the_byte_modulo_256() {
        assert_eq!(triple_index(0), (0, false, false));
        assert_eq!(triple_index(1), (3, false, false));
        assert_eq!(triple_index(0x55), (0xFF, false, false));
        assert_eq!(triple_index(0x80), (0x80, false, false), "bit 7 leaves 2v, so the sum is v");
    }

    #[test]
    fn a_positive_sum_past_127_overflows() {
        assert_eq!(triple_index(0x30), (0x90, false, true));
        assert_eq!(triple_index(0x60), (0x20, true, false));
    }

    #[test]
    fn the_table_entry_is_at_the_offset_of_the_bank_1_table() {
        assert_eq!(TRIPLE_TABLE, 0x01_83BD);
    }

    // The largest index stays inside the table: checked when the test module is compiled.
    const _: () =
        assert!(TRIPLE_TABLE + 255 < data::BANK1_INDEX_TABLE + data::BANK1_INDEX_TABLE_SIZE);
}

// ---- update_travel_and_follow: the leader's travel steps, and the follow of the two axes ----

/// The entry of the routine (`$01:82E3`), for the hand-back to the original code.
const TRAVEL_ENTRY: u32 = 0x01_82E3;
/// The travel flags byte (`gate_flag_01bc`), through the data bank.
const GATE_FLAG_01BC: u16 = (data::GATE_FLAG_01BC & 0xFFFF) as u16;
/// The byte whose low nibble path A takes (`source_byte_0048`), through the data bank.
const SOURCE_BYTE_0048: u16 = (data::SOURCE_BYTE_0048 & 0xFFFF) as u16;
/// The byte path B takes (`source_byte_0044`), through the data bank.
const TRAVEL_BYTE_44: u16 = (data::SOURCE_BYTE_0044 & 0xFFFF) as u16;
/// The copy gate (`copy_gate`), read for its bit 7 only: the cost depends on it.
const COPY_GATE: u16 = (data::COPY_GATE & 0xFFFF) as u16;
/// The copy select (`copy_select`), read for bit 1: the cost depends on it.
const COPY_SELECT: u16 = (data::COPY_SELECT & 0xFFFF) as u16;
/// The nibble path A keeps (`travel_nibble`), through the data bank.
const TRAVEL_NIBBLE: u16 = (data::TRAVEL_NIBBLE & 0xFFFF) as u16;
/// The Y limit byte (`follow_limit`), through the data bank.
const FOLLOW_LIMIT: u16 = (data::FOLLOW_LIMIT & 0xFFFF) as u16;
/// The leader's Y position (`follow_target`, a word), through the data bank.
const FOLLOW_TARGET: u16 = (data::FOLLOW_TARGET & 0xFFFF) as u16;
/// The X step of the leader: the word at `travel_mode` (its high byte is part of the step).
const TRAVEL_STEP_X: u16 = (data::TRAVEL_MODE & 0xFFFF) as u16;
/// The Y step of the leader (`travel_step_y`, a word), through the data bank.
const TRAVEL_STEP_Y: u16 = (data::TRAVEL_STEP_Y & 0xFFFF) as u16;

/// The inputs of one call of `update_travel_and_follow`, read on entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TravelInput {
    /// The travel flags byte (`$01BC`): bit 0 chooses path A or B, bit 5 ends path A early.
    pub bc: u8,
    /// The low nibble of `$0048`: path A stores it to `$0D5E` when it is not zero.
    pub nibble: u8,
    /// The byte path B takes as its value (`$0044`).
    pub byte_44: u8,
    /// The copy gate (`$01A1`); only its bit 7 matters, and only for the cost.
    pub gate: u8,
    /// The copy select (`$01E7`); with the gate clear, bit 1 of it sets the carry the branch after the
    /// copy tests, and a branch not taken costs one cycle less.
    pub select: u8,
    /// The byte path A takes as its value when its nibble is zero (`$0D5E`).
    pub stored_nibble: u8,
    /// The X and the Y limit bytes (`$0703` and `$0702`).
    pub limit_x: u8,
    pub limit_y: u8,
    /// The X and the Y steps (`$0D55` and `$0D57`, words).
    pub step_x: u16,
    pub step_y: u16,
    /// The leader's X and Y positions (`$01A2` and `$01A4`, words).
    pub lead_x: u16,
    pub lead_y: u16,
    /// The index byte of the flags table (`$0D59`).
    pub index: u8,
    /// The table byte at that index (`$01:83A0 + index`), which a stepping call stores.
    pub table: u8,
    /// The high byte of A on entry (B), which the calls that do not step keep.
    pub entry_b: u8,
}

/// The route of one call of `update_travel_and_follow`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TravelRoute {
    /// Path A with bit 5 of `$01BC` set: returns at once.
    Early,
    /// Bits 0 to 3 of the value clear: returns with the value's high nibble.
    Idle,
    /// X at or above its limit: no step.
    XBlocked,
    /// X steps up by its step.
    XUp,
    /// X below sixteen: no step.
    XLow,
    /// X steps down by its step.
    XDown,
    /// Y at or above its limit: no step.
    YBlocked,
    /// Y steps up by its step.
    YUp,
    /// Y below sixteen: no step.
    YLow,
    /// Y steps down by its step.
    YDown,
}

/// The key of a call's cost: its route and the branches the cost depends on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TravelKey {
    pub route: TravelRoute,
    /// Path B (bit 0 of `$01BC` clear).
    pub path_b: bool,
    /// Path A with a zero nibble (the store to `$0D5E` is skipped).
    pub nibble_zero: bool,
    /// Bit 7 of `$01A1` clear (the copy branch that reads `$01E7`).
    pub gate_clear: bool,
    /// The gate clear and bit 1 of `$01E7` set: the branch after the copy is not taken (6 cycles less).
    pub copy_carry: bool,
}

/// What `update_travel_and_follow` leaves for its inputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TravelOutput {
    pub key: TravelKey,
    /// The new `$01BC`, when path A clears bit 5.
    pub bc: Option<u8>,
    /// The new `$0D5E`, when path A stores its nibble.
    pub stored_nibble: Option<u8>,
    /// `$00` and `$01`, when the X or the Y branch sets them.
    pub scratch: Option<(u8, u8)>,
    /// The new leader X, when it steps.
    pub lead_x: Option<u16>,
    /// The new leader Y, when it steps.
    pub lead_y: Option<u16>,
    /// The flags byte after the call (`$0700`): the table byte when the leader stepped, else 0.
    pub flags: u8,
    /// The new X (the index), when the leader stepped.
    pub x: Option<u8>,
    /// The full A after the call (B included).
    pub a: u16,
    pub n: bool,
    pub z: bool,
    pub c: bool,
    /// V, `None` when the call leaves it as it was.
    pub v: Option<bool>,
}

/// N, Z and C of the 16-bit compare of `lead` with `value`.
fn compare16(lead: u16, value: u16) -> (bool, bool, bool) {
    let diff = lead.wrapping_sub(value);
    (diff & 0x8000 != 0, diff == 0, lead >= value)
}

/// Sets what a stepping call leaves: the table byte in `$0700`, A (with B clear), X, and N, Z, C
/// and V of the step.
fn stepped(out: &mut TravelOutput, input: &TravelInput, carry: bool, overflow: bool) {
    out.flags = input.table;
    out.x = Some(input.index);
    out.a = u16::from(input.table);
    out.n = input.table & 0x80 != 0;
    out.z = input.table == 0;
    out.c = carry;
    out.v = Some(overflow);
}

/// The travel and follow step of `update_travel_and_follow` for the inputs.
pub(crate) fn plan_travel(input: TravelInput) -> TravelOutput {
    let path_b = input.bc & 0x01 == 0;
    let mut out = TravelOutput {
        key: TravelKey {
            route: TravelRoute::Idle,
            path_b,
            nibble_zero: false,
            gate_clear: input.gate & 0x80 == 0,
            copy_carry: input.gate & 0x80 == 0 && input.select & 0x02 != 0,
        },
        bc: None,
        stored_nibble: None,
        scratch: None,
        lead_x: None,
        lead_y: None,
        flags: 0,
        x: None,
        a: u16::from(input.entry_b) << 8,
        n: false,
        z: false,
        c: input.bc & 0x01 != 0,
        v: None,
    };
    let value = if path_b {
        input.byte_44
    } else {
        out.key.nibble_zero = input.nibble == 0;
        if input.nibble != 0 {
            out.stored_nibble = Some(input.nibble);
            out.bc = Some(input.bc & 0xDF);
        }
        let bc_now = out.bc.unwrap_or(input.bc);
        if bc_now & 0x20 != 0 {
            out.key.route = TravelRoute::Early;
            out.a |= 0x20;
            out.n = false;
            out.z = false;
            return out;
        }
        if input.nibble != 0 { input.nibble } else { input.stored_nibble }
    };
    if value & 0x01 != 0 {
        let wd = input.limit_x.wrapping_sub(1);
        let threshold = u16::from_le_bytes([0xF0, wd]);
        out.scratch = Some((0xF0, wd));
        let lead = input.lead_x;
        if lead >= threshold {
            out.key.route = TravelRoute::XBlocked;
            (out.n, out.z, out.c) = compare16(lead, threshold);
            out.a = lead;
        } else {
            let (sum, carry, overflow) = add16(lead, input.step_x);
            out.key.route = TravelRoute::XUp;
            out.lead_x = Some(sum);
            stepped(&mut out, &input, carry, overflow);
        }
    } else if value & 0x02 != 0 {
        let lead = input.lead_x;
        (out.n, out.z, out.c) = compare16(lead, 0x0010);
        if out.c {
            let (diff, carry, overflow) = sub16(lead, input.step_x);
            out.key.route = TravelRoute::XDown;
            out.lead_x = Some(diff);
            stepped(&mut out, &input, carry, overflow);
        } else {
            out.key.route = TravelRoute::XLow;
            out.a = lead;
        }
    } else if value & 0x04 != 0 {
        let wd = input.limit_y.wrapping_sub(1);
        let threshold = u16::from_le_bytes([0xF0, wd]);
        out.scratch = Some((0xF0, wd));
        let lead = input.lead_y;
        if lead >= threshold {
            out.key.route = TravelRoute::YBlocked;
            (out.n, out.z, out.c) = compare16(lead, threshold);
            out.a = lead;
        } else {
            let (sum, carry, overflow) = add16(lead, input.step_y);
            out.key.route = TravelRoute::YUp;
            out.lead_y = Some(sum);
            stepped(&mut out, &input, carry, overflow);
        }
    } else if value & 0x08 != 0 {
        let lead = input.lead_y;
        (out.n, out.z, out.c) = compare16(lead, 0x0010);
        if out.c {
            let (diff, carry, overflow) = sub16(lead, input.step_y);
            out.key.route = TravelRoute::YDown;
            out.lead_y = Some(diff);
            stepped(&mut out, &input, carry, overflow);
        } else {
            out.key.route = TravelRoute::YLow;
            out.a = lead;
        }
    } else {
        let shifted = value >> 4;
        out.key.route = TravelRoute::Idle;
        out.a |= u16::from(shifted);
        out.n = false;
        out.z = shifted == 0;
        out.c = false;
    }
    out
}

/// CPU-busy master cycles of the branches the reference replays take, from the hit to the return
/// (measured in `Verify`); `None` for the others, whose calls run the original code.
pub(crate) fn travel_cost(key: TravelKey) -> Option<u64> {
    use TravelRoute::{Early, Idle, XBlocked, XDown, XUp, YDown, YUp};
    if !key.gate_clear {
        return None;
    }
    let cost = match (key.route, key.path_b, key.nibble_zero) {
        (Early, false, true) => 272,
        (Idle, true, false) => 528,
        (XBlocked, false, true) => 796,
        (XDown, false, false) => 1106,
        (XDown, false, true) => 1000,
        (XDown, true, false) => 844,
        (XUp, false, false) => 1188,
        (XUp, false, true) => 1082,
        (XUp, true, false) => 926,
        (YDown, false, false) => 1156,
        (YDown, false, true) => 1050,
        (YDown, true, false) => 894,
        (YUp, false, false) => 1260,
        (YUp, false, true) => 1154,
        (YUp, true, false) => 998,
        _ => return None,
    };
    Some(if key.copy_carry { cost - 6 } else { cost })
}

/// The inputs of [`plan_travel`] read from the machine (reads with no side effects).
fn travel_input(ctx: &Ctx<'_>) -> TravelInput {
    let index = ctx.peek_db8(INDEX_SOURCE);
    TravelInput {
        bc: ctx.peek_db8(GATE_FLAG_01BC),
        nibble: ctx.peek_db8(SOURCE_BYTE_0048) & 0x0F,
        byte_44: ctx.peek_db8(TRAVEL_BYTE_44),
        gate: ctx.peek_db8(COPY_GATE),
        select: ctx.peek_db8(COPY_SELECT),
        stored_nibble: ctx.peek_db8(TRAVEL_NIBBLE),
        limit_x: ctx.peek_db8(FOLLOW_LIMIT_X),
        limit_y: ctx.peek_db8(FOLLOW_LIMIT),
        step_x: ctx.peek_db16(TRAVEL_STEP_X),
        step_y: ctx.peek_db16(TRAVEL_STEP_Y),
        lead_x: ctx.peek_db16(FOLLOW_TARGET_X),
        lead_y: ctx.peek_db16(FOLLOW_TARGET),
        index,
        table: ctx.peek8(data::BANK1_INDEX_TABLE + u32::from(index)),
        entry_b: (ctx.a() >> 8) as u8,
    }
}

/// The instructions the original code runs on the branch of `key`, from the hit to the return (the
/// return included): declared with `set_instructions`, as the count varies with the branch. Up to
/// the test of bit 0 of the value the count depends on the path (path B, path A with a zero
/// nibble, path A with a nonzero one); the copy branch on bit 7 of `$01A1` takes one instruction
/// more when that bit is clear.
pub(crate) fn travel_instructions(key: TravelKey) -> u64 {
    let path = match (key.path_b, key.nibble_zero) {
        (true, _) => 5,
        (false, true) => 12,
        (false, false) => 16,
    };
    let copy_branch = if key.gate_clear { 10 } else { 9 };
    let rest = match key.route {
        TravelRoute::Early => return 11,
        TravelRoute::Idle | TravelRoute::XLow => 8,
        TravelRoute::XBlocked => 11,
        TravelRoute::XUp | TravelRoute::YDown => 22,
        TravelRoute::XDown => 19,
        TravelRoute::YBlocked => 15,
        TravelRoute::YUp => 26,
        TravelRoute::YLow => 12,
    };
    path + copy_branch + rest
}

/// The cost bound of `update_travel_and_follow`: the cost of its branch, `u64::MAX` (decline) for a
/// branch the replays do not take.
pub(crate) fn update_travel_and_follow_bound(ctx: &Ctx<'_>) -> u64 {
    travel_cost(plan_travel(travel_input(ctx)).key).unwrap_or(u64::MAX)
}

/// `update_travel_and_follow`: the leader's travel and the follow state change as
/// [`plan_travel`] gives; the direct-page scratch `$00-$01` is set when its branch sets it.
pub(crate) fn update_travel_and_follow(ctx: &mut Ctx<'_>) {
    let plan = plan_travel(travel_input(ctx));
    let Some(cost) = travel_cost(plan.key) else {
        let Ok(()) = ctx.call_original(TRAVEL_ENTRY, ReturnKind::Rtl) else { return };
        ctx.set_cost(0);
        return;
    };
    ctx.write_db8(FOLLOW_FLAGS, plan.flags);
    if let Some(bc) = plan.bc {
        ctx.write_db8(GATE_FLAG_01BC, bc);
    }
    if let Some(nibble) = plan.stored_nibble {
        ctx.write_db8(TRAVEL_NIBBLE, nibble);
    }
    if let Some((a, b)) = plan.scratch {
        ctx.write_dp8(0x00, a);
        ctx.write_dp8(0x01, b);
    }
    if let Some(x) = plan.lead_x {
        ctx.write_db16(FOLLOW_TARGET_X, x);
    }
    if let Some(y) = plan.lead_y {
        ctx.write_db16(FOLLOW_TARGET, y);
    }
    if let Some(x) = plan.x {
        ctx.set_x(u16::from(x));
    }
    ctx.set_a(plan.a);
    ctx.set_n(plan.n);
    ctx.set_z(plan.z);
    ctx.set_c(plan.c);
    if let Some(v) = plan.v {
        ctx.set_v(v);
    }
    ctx.set_instructions(travel_instructions(plan.key));
    ctx.set_cost(cost);
}

#[cfg(test)]
mod update_travel_and_follow_tests {
    use super::*;

    fn input() -> TravelInput {
        TravelInput {
            bc: 0x00,
            nibble: 0,
            byte_44: 0,
            gate: 0,
            select: 0,
            stored_nibble: 0,
            limit_x: 9,
            limit_y: 11,
            step_x: 2,
            step_y: 2,
            lead_x: 0x0100,
            lead_y: 0x0200,
            index: 3,
            table: 0x20,
            entry_b: 0x0A,
        }
    }

    #[test]
    fn a_path_a_call_with_bit_5_returns_at_once() {
        let out = plan_travel(TravelInput { bc: 0x21, ..input() });
        assert_eq!(out.key.route, TravelRoute::Early);
        assert_eq!((out.a, out.n, out.z, out.c, out.v), (0x0A20, false, false, true, None));
        assert_eq!((out.bc, out.flags, out.lead_x), (None, 0, None));
    }

    #[test]
    fn an_idle_value_leaves_its_high_nibble_in_a() {
        let out = plan_travel(TravelInput { byte_44: 0x10, ..input() });
        assert_eq!(out.key.route, TravelRoute::Idle);
        assert_eq!((out.a, out.n, out.z, out.c), (0x0A01, false, false, false));
    }

    #[test]
    fn x_steps_up_by_its_step_and_stores_the_table_byte() {
        let out = plan_travel(TravelInput { byte_44: 0x01, ..input() });
        assert_eq!(out.key.route, TravelRoute::XUp);
        assert_eq!((out.lead_x, out.scratch), (Some(0x0102), Some((0xF0, 0x08))));
        assert_eq!((out.flags, out.x, out.a), (0x20, Some(3), 0x0020));
        assert_eq!((out.n, out.z, out.c, out.v), (false, false, false, Some(false)));
    }

    #[test]
    fn x_at_its_limit_does_not_step() {
        let out = plan_travel(TravelInput { byte_44: 0x01, lead_x: 0x0900, ..input() });
        assert_eq!(out.key.route, TravelRoute::XBlocked);
        assert_eq!((out.lead_x, out.a, out.flags), (None, 0x0900, 0));
        assert_eq!((out.n, out.z, out.c, out.v), (false, false, true, None));
    }

    #[test]
    fn the_instruction_counts_follow_the_path_and_the_route() {
        let key = |route, path_b, nibble_zero| TravelKey {
            route,
            path_b,
            nibble_zero,
            gate_clear: true,
            copy_carry: false,
        };
        assert_eq!(travel_instructions(key(TravelRoute::Early, false, true)), 11);
        assert_eq!(travel_instructions(key(TravelRoute::Idle, true, false)), 23);
        assert_eq!(travel_instructions(key(TravelRoute::XBlocked, false, true)), 33);
        assert_eq!(travel_instructions(key(TravelRoute::XUp, false, false)), 48);
        assert_eq!(travel_instructions(key(TravelRoute::YUp, true, false)), 41);
        assert_eq!(travel_instructions(key(TravelRoute::YDown, false, true)), 44);
    }

    #[test]
    fn the_costs_are_the_measured_ones_and_the_gate_declines() {
        let early = TravelKey {
            route: TravelRoute::Early,
            path_b: false,
            nibble_zero: true,
            gate_clear: true,
            copy_carry: false,
        };
        assert_eq!(travel_cost(early), Some(272));
        assert_eq!(travel_cost(TravelKey { copy_carry: true, ..early }), Some(266));
        assert_eq!(travel_cost(TravelKey { gate_clear: false, ..early }), None);
        let unseen = TravelKey {
            route: TravelRoute::YLow,
            path_b: false,
            nibble_zero: false,
            gate_clear: true,
            copy_carry: false,
        };
        assert_eq!(travel_cost(unseen), None);
    }
}

// ---- copy_bank9_block: a 64-byte block of bank `$09` into low WRAM, then the copy counter ----

/// The ROM block copied (`bank9_copy_block`, bank `$09`).
const BANK9_BLOCK: u32 = data::BANK9_COPY_BLOCK;
/// Its destination in low WRAM (`copy_target_02c0`), through the data bank.
const BANK9_TARGET: u16 = (data::COPY_TARGET_02C0 & 0xFFFF) as u16;
/// The copy counter the routine increments (`copy_done`), through the data bank.
const COPY_DONE_COUNTER: u16 = (data::COPY_DONE & 0xFFFF) as u16;
/// The block's length in bytes.
const BANK9_LEN: u16 = 0x40;
/// CPU-busy master cycles of a call, from the hit to the return: one path for every call
/// (measured in `Verify`).
pub(crate) const COPY_BANK9_BLOCK_COST: u64 = 5528;

/// `copy_bank9_block`: the 64 bytes at `$09:D200` go to `DBR:02C0` as 32 words, the last one
/// left in A (B:A), then `$0715` goes up by one. X ends at `$0040`, C is set (the last compare of
/// the loop was equal), and N and Z come from the increment; V is unchanged.
pub(crate) fn copy_bank9_block(ctx: &mut Ctx<'_>) {
    let mut last = 0;
    for offset in (0..BANK9_LEN).step_by(2) {
        last = ctx.read16(BANK9_BLOCK + u32::from(offset));
        ctx.write_db16(BANK9_TARGET + offset, last);
    }
    ctx.set_x(BANK9_LEN);
    ctx.set_a(last);
    ctx.set_c(true);
    let counter = ctx.read_db8(COPY_DONE_COUNTER).wrapping_add(1);
    ctx.write_db8(COPY_DONE_COUNTER, counter);
    ctx.set_nz8(counter);
    ctx.set_cost(COPY_BANK9_BLOCK_COST);
}

#[cfg(test)]
mod copy_bank9_block_tests {
    use super::*;
    use crate::routines::tests::max_cost;

    #[test]
    fn copy_bank9_block_cost_is_constant() {
        assert_eq!(max_cost("copy_bank9_block"), COPY_BANK9_BLOCK_COST);
    }

    #[test]
    fn the_block_is_64_bytes_of_bank_9_into_the_slot_area_at_02c0() {
        assert_eq!((BANK9_BLOCK, data::BANK9_COPY_BLOCK_SIZE), (0x09_D200, 64));
        assert_eq!((BANK9_TARGET, data::COPY_TARGET_02C0_SIZE), (0x02C0, 64));
        assert_eq!(BANK9_LEN, 64);
    }
}

// ---- add_four_to_dp_word: the direct-page word at `$02` grows by four ----

/// The direct-page offset of the word the routine steps (`$02`).
const STEP_WORD: u16 = 0x02;
/// The amount the routine adds.
const STEP: u16 = 4;
/// CPU-busy master cycles of a call, from the hit to the return: one path for every call
/// (measured in `Verify`).
pub(crate) const ADD_FOUR_TO_DP_WORD_COST: u64 = 212;

/// `add_four_to_dp_word`: the word at `$02` plus four, stored back. A is the sum, N and Z are set
/// from it, C and V from the addition; the rest is unchanged.
pub(crate) fn add_four_to_dp_word(ctx: &mut Ctx<'_>) {
    let (sum, carry, overflow) = add16(ctx.read_dp16(STEP_WORD), STEP);
    ctx.write_dp16(STEP_WORD, sum);
    ctx.set_a(sum);
    ctx.set_nz16(sum);
    ctx.set_c(carry);
    ctx.set_v(overflow);
    ctx.set_cost(ADD_FOUR_TO_DP_WORD_COST);
}

#[cfg(test)]
mod add_four_to_dp_word_tests {
    use super::*;
    use crate::routines::tests::max_cost;

    #[test]
    fn add_four_to_dp_word_cost_is_constant() {
        assert_eq!(max_cost("add_four_to_dp_word"), ADD_FOUR_TO_DP_WORD_COST);
    }

    #[test]
    fn the_sum_carries_and_overflows_as_the_16_bit_add_does() {
        assert_eq!(add16(0xFFFE, STEP), (0x0002, true, false));
        assert_eq!(add16(0x7FFE, STEP), (0x8002, false, true));
    }
}
