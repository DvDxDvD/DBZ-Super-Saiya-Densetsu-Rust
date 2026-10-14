//! Entity slots (stride `$20`) of the map's entity draw loop.

use crate::data;
use ssd_oracle::Ctx;

/// The id byte of entity slot 0 (`entity_ids`, stride `$20`; absolute, so DBR-relative; Y = slot
/// offset).
const ENTITY_ID: u32 = data::ENTITY_IDS & 0xFFFF;
/// The ids the routine tests, in its order.
const SPECIAL_IDS: [u8; 5] = [0x10, 0x11, 0x4C, 0x4D, 0x4F];
/// CPU-busy master cycles of a call that finds the id at `SPECIAL_IDS[0]`, from the hit to the
/// return; each later member costs [`COST_PER_MEMBER`] more (one more comparison and branch).
const COST_FIRST_MEMBER: u64 = 134;
const COST_PER_MEMBER: u64 = 32;
/// The cost of a non-member: all five comparisons, then the exit that clears C. The last
/// member, which reaches its exit after the same five comparisons, costs most (`cycles` in
/// `re/symbols.toml`). Costs measured in `Verify`.
const COST_NOT_MEMBER: u64 = 256;

/// The flags the routine leaves for an id: C = membership, N/Z of its last comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Membership {
    pub n: bool,
    pub z: bool,
    pub c: bool,
}

/// The routine compares the id with each member in turn and stops at the first match. A member
/// leaves the flags of an equal comparison (N = 0, Z = 1) with C forced to 1; any other id
/// leaves N/Z of the comparison with `$4F`, the last member (bit 7 of `id - $4F`, Z = 0), with
/// C forced to 0.
pub(crate) fn membership(id: u8) -> Membership {
    if SPECIAL_IDS.contains(&id) {
        Membership { n: false, z: true, c: true }
    } else {
        let last = id.wrapping_sub(0x4F);
        Membership { n: last & 0x80 != 0, z: last == 0, c: false }
    }
}

/// The cost of a call for `id`: by the place of the member that ends the comparisons, or that
/// of a non-member.
pub(crate) fn cost(id: u8) -> u64 {
    SPECIAL_IDS
        .iter()
        .position(|&member| member == id)
        .map_or(COST_NOT_MEMBER, |place| COST_FIRST_MEMBER + COST_PER_MEMBER * place as u64)
}

/// `entity_id_in_special_set`: reads the id of the entity slot at offset Y; A low = the id (B
/// untouched), flags as [`membership`]. X, Y, V and memory untouched.
pub(crate) fn entity_id_in_special_set(ctx: &mut Ctx<'_>) {
    // Absolute indexed: DBR:`entity_ids` + Y may carry into the next bank.
    let address = (u32::from(ctx.dbr()) << 16) + ENTITY_ID + u32::from(ctx.y());
    let id = ctx.read8(address);
    let flags = membership(id);
    ctx.set_al(id);
    ctx.set_n(flags.n);
    ctx.set_z(flags.z);
    ctx.set_c(flags.c);
    ctx.set_cost(cost(id));
}

#[cfg(test)]
mod tests {
    use super::{COST_NOT_MEMBER, Membership, cost, membership};
    use crate::routines::tests::max_cost;

    #[test]
    fn each_member_costs_its_place_and_the_last_one_the_most() {
        let costs: Vec<u64> = [0x10, 0x11, 0x4C, 0x4D, 0x4F].into_iter().map(cost).collect();
        assert_eq!(costs, [134, 166, 198, 230, 262]);
        for id in [0x00, 0x0F, 0x12, 0x4E, 0x50, 0xCF, 0xFF] {
            assert_eq!(cost(id), COST_NOT_MEMBER, "{id:02X}");
        }
        assert_eq!((0..=255).map(cost).max(), Some(262), "the last member");
        assert_eq!(max_cost("entity_id_in_special_set"), 262);
    }

    const MEMBER: Membership = Membership { n: false, z: true, c: true };

    /// The flags of a non-member: N/Z of `id - $4F`, C = 0.
    fn other(n: bool) -> Membership {
        Membership { n, z: false, c: false }
    }

    #[test]
    fn the_five_special_ids_are_members() {
        for id in [0x10, 0x11, 0x4C, 0x4D, 0x4F] {
            assert_eq!(membership(id), MEMBER, "{id:02X}");
        }
    }

    #[test]
    fn neighbours_and_extremes_are_not_members() {
        // N is bit 7 of `id - $4F`.
        for (id, n) in [
            (0x0F, true),
            (0x12, true),
            (0x4B, true),
            (0x4E, true),
            (0x50, false),
            (0x00, true),
            (0xFF, true),
            (0xCE, false),
            (0xCF, true),
        ] {
            assert_eq!(membership(id), other(n), "{id:02X}");
        }
    }
}
