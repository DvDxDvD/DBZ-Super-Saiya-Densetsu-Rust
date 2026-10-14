//! Battle sides.

use crate::data;
use ssd_oracle::Ctx;

/// WRAM byte whose bit 0 selects the active battle side (`active_side`; absolute, so
/// DBR-relative).
const ACTIVE_SIDE: u16 = (data::ACTIVE_SIDE & 0xFFFF) as u16;
/// Base of each side's record (`side_record_a`, `side_record_b`): side 0, side 1.
const SIDE_RECORDS: [u16; 2] =
    [(data::SIDE_RECORD_A & 0xFFFF) as u16, (data::SIDE_RECORD_B & 0xFFFF) as u16];

/// CPU-busy master cycles of a call, from the hit to the return, by side: side 0 takes the
/// short path; side 1 also loads the second record base and is the maximum (`cycles` in
/// `re/symbols.toml`). Measured in `Verify`.
const COSTS: [u64; 2] = [136, 154];
/// Instructions of a call by side: the load of the side byte, the mask, the branch and the
/// return, and for side 1 the second load of Y.
const INSTRUCTIONS: [u64; 2] = [5, 6];

/// What the routine leaves for an active-side byte: Y = the record base, A low = the side
/// (bit 0). N is always 0 and Z is set for side 0.
pub(crate) fn active_side_record(active_side: u8) -> (u16, u8) {
    let side = active_side & 0x01;
    (SIDE_RECORDS[usize::from(side)], side)
}

/// `get_active_side_record`: Y = the record base of the side selected by bit 0 of the
/// active-side byte, A low = that bit, N = 0 and Z = (side 0). B, X, C and V are untouched.
pub(crate) fn get_active_side_record(ctx: &mut Ctx<'_>) {
    let (record, side) = active_side_record(ctx.read_db8(ACTIVE_SIDE));
    ctx.set_y(record);
    ctx.set_al(side);
    // The original sets N/Z from the side bit; loading the side-1 base leaves them the same.
    ctx.set_nz8(side);
    ctx.set_cost(COSTS[usize::from(side)]);
    ctx.set_instructions(INSTRUCTIONS[usize::from(side)]);
}

#[cfg(test)]
mod tests {
    use super::{COSTS, SIDE_RECORDS, active_side_record};
    use crate::routines::tests::max_cost;

    #[test]
    fn the_symbols_cost_is_the_longer_path() {
        assert_eq!(max_cost("get_active_side_record"), COSTS[1]);
        assert!(COSTS[0] < COSTS[1]);
    }

    #[test]
    fn bit_0_selects_the_record() {
        let [side_0, side_1] = SIDE_RECORDS;
        assert_ne!(side_0, side_1);
        assert_eq!(active_side_record(0x00), (side_0, 0));
        assert_eq!(active_side_record(0x01), (side_1, 1));
        // Only bit 0 counts.
        assert_eq!(active_side_record(0xFE), (side_0, 0));
        assert_eq!(active_side_record(0xFF), (side_1, 1));
        assert_eq!(active_side_record(0x80), (side_0, 0));
    }
}
