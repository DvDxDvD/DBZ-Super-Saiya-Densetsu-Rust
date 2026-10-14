//! Record-stride steps of the object loops.

use ssd_oracle::Ctx;

/// CPU-busy master cycles of a call, from the hit to the return: six index increments and the
/// return, on every call (measured in `Verify`).
pub(crate) const COST: u64 = 128;

/// X + 6, wrapping at the index width.
pub(crate) fn advance(x: u16, x8: bool) -> u16 {
    let x = x.wrapping_add(6);
    if x8 { x & 0x00FF } else { x }
}

/// `advance_x_by_6`: X += 6, N/Z from X; A, Y, C, V and memory untouched.
pub(crate) fn advance_x_by_6(ctx: &mut Ctx<'_>) {
    let x8 = ctx.x8();
    let x = advance(ctx.x(), x8);
    ctx.set_x(x);
    if x8 {
        ctx.set_nz8(x as u8);
    } else {
        ctx.set_nz16(x);
    }
    ctx.set_cost(COST);
}

#[cfg(test)]
mod tests {
    use super::{COST, advance};
    use crate::routines::tests::max_cost;

    #[test]
    fn the_cost_is_constant() {
        assert_eq!(max_cost("advance_x_by_6"), COST);
    }

    #[test]
    fn adds_six_and_wraps_at_the_index_width() {
        assert_eq!(advance(0x0000, false), 0x0006);
        assert_eq!(advance(0x00C0, false), 0x00C6);
        assert_eq!(advance(0x00FC, false), 0x0102, "16-bit X carries into the high byte");
        assert_eq!(advance(0xFFFA, false), 0x0000);
        assert_eq!(advance(0xFFFC, false), 0x0002);
        assert_eq!(advance(0x00FC, true), 0x0002, "8-bit X wraps in 8 bits");
    }
}
