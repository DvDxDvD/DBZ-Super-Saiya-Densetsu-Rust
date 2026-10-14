//! Id categories: a 128-byte ROM table maps a character/card id to its category (provisional
//! name, `docs/re/m1-candidates.md`). The table is read from the ROM at run time.

use crate::data;
use ssd_oracle::Ctx;

/// The category table in the ROM (`id_category_table`), 128 entries.
const CATEGORY_TABLE: u32 = data::ID_CATEGORY_TABLE;
/// CPU-busy master cycles of a call, from the hit to the return: one path for every id
/// (measured in `Verify`).
pub(crate) const COST: u64 = 250;

/// The category of `id`: entry `id & $7F` of the table, through `read_entry` (index → byte),
/// with bit 7 cleared.
pub(crate) fn category_of(id: u8, mut read_entry: impl FnMut(u8) -> u8) -> u8 {
    read_entry(id & 0x7F) & 0x7F
}

/// `lookup_id_category`: A = the category of the id in A low ([`category_of`]), with B = 0. X
/// is saved on the stack and restored at the end, and the restore leaves N/Z from X (not from
/// A); the saved copy is dead stack. C and V are untouched.
pub(crate) fn lookup_id_category(ctx: &mut Ctx<'_>) {
    let category = category_of(ctx.al(), |index| ctx.read8(CATEGORY_TABLE + u32::from(index)));
    ctx.set_a(u16::from(category));
    let x = ctx.x();
    if ctx.x8() {
        ctx.set_nz8(x as u8);
    } else {
        ctx.set_nz16(x);
    }
    ctx.set_cost(COST);
}

#[cfg(test)]
mod tests {
    use super::{COST, category_of};
    use crate::data::ID_CATEGORY_TABLE_SIZE;
    use crate::routines::tests::max_cost;

    #[test]
    fn the_cost_is_constant() {
        assert_eq!(max_cost("lookup_id_category"), COST);
    }

    /// A made-up table: every entry has bit 7 set and its index in the low bits.
    fn entry(index: u8) -> u8 {
        assert!(index < 0x80, "index {index:02X} outside the 128-byte table");
        0x80 | index
    }

    #[test]
    fn bit_7_of_the_id_and_of_the_entry_are_ignored() {
        assert_eq!(category_of(0x00, entry), 0x00);
        assert_eq!(category_of(0x34, entry), 0x34);
        assert_eq!(category_of(0x7F, entry), 0x7F);
        assert_eq!(category_of(0x80, entry), 0x00, "id bit 7 ignored");
        assert_eq!(category_of(0xB4, entry), 0x34);
        assert_eq!(category_of(0xFF, entry), 0x7F);
    }

    #[test]
    fn the_symbols_table_has_one_entry_per_masked_index() {
        assert_eq!(ID_CATEGORY_TABLE_SIZE, 0x7F + 1);
    }

    #[test]
    fn reads_one_entry() {
        let mut reads = Vec::new();
        category_of(0x95, |index| {
            reads.push(index);
            0
        });
        assert_eq!(reads, [0x15]);
    }
}
