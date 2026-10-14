//! Batch B of the M2 porting: a bank-`$02` pointer lookup, a byte-pair lookup, a scaled table
//! lookup, a WRAM buffer fill and a slot clear. Each routine's semantics, contract and callers
//! are in `docs/re/routines/<name>.md`.

use crate::data;
use ssd_oracle::Ctx;

/// The bank-`$02` table of 16-bit pointers (`bank2_pointer_table`): one word per even offset.
const POINTER_TABLE: u32 = data::BANK2_POINTER_TABLE;
/// CPU-busy master cycles of a call, from the hit to the return: one path for every index
/// (measured in `Verify`).
pub(crate) const LOOKUP_BANK2_POINTER_COST: u64 = 470;

/// The address in the table of the pointer for `index`: twice the index, kept to 8 bits, so the
/// table is read through the even entries only.
pub(crate) fn table_offset(index: u8) -> u16 {
    u16::from(index << 1)
}

/// The low 16 bits of the table entry for `index`: the table's own low word plus the offset,
/// which cannot leave the table's 256 bytes.
pub(crate) fn entry_pointer(index: u8) -> u16 {
    (POINTER_TABLE as u16).wrapping_add(table_offset(index))
}

/// `lookup_bank2_pointer`: the word of the table entry for the index byte in `$00` becomes the
/// low two bytes of a long pointer at `$00-$02` (bank `$02`). The pointer's own address goes to
/// `$03-$05`, `$05` holds the bank again, and Y = 0. A = the word, N/Z of that word, C and V
/// clear.
pub(crate) fn lookup_bank2_pointer(ctx: &mut Ctx<'_>) {
    let bank = (POINTER_TABLE >> 16) as u8;
    let index = ctx.read_dp8(0);
    let pointer = entry_pointer(index);
    let word = ctx.read16((u32::from(bank) << 16) | u32::from(pointer));
    ctx.write_dp16(0, word);
    ctx.write_dp8(2, bank);
    ctx.write_dp16(3, pointer);
    ctx.write_dp8(5, bank);
    ctx.set_a(word);
    ctx.set_y(0);
    ctx.set_nz16(word);
    ctx.set_c(false);
    ctx.set_v(false);
    ctx.set_cost(LOOKUP_BANK2_POINTER_COST);
}

/// The 1 KB buffer `fill_buffer_4800` fills (`buffer_4800`, in WRAM bank `$7E`).
const BUFFER: u32 = data::BUFFER_4800;
/// Its length in bytes.
const BUFFER_LEN: u32 = data::BUFFER_4800_SIZE;
/// The word `fill_buffer_4800` clears through the data bank (`word_0d7d`, low WRAM).
const WORD_0D7D: u16 = (data::WORD_0D7D & 0xFFFF) as u16;
/// CPU-busy master cycles of a call, from the hit to the return: one path, a fixed loop of 512
/// stores (measured in `Verify`).
pub(crate) const FILL_BUFFER_4800_COST: u64 = 50346;

/// The offsets of the buffer that the routine stores a word at: every even offset, so each
/// word covers two bytes.
pub(crate) fn buffer_offsets() -> impl Iterator<Item = u32> {
    (0..BUFFER_LEN).step_by(2)
}

/// `fill_buffer_4800`: `$FFFF` at every even offset of the buffer (all its bytes become `$FF`),
/// then zero at `DBR:0D7D`. A = `$FFFF`, X = `$FFFE`, N set and Z clear from that X; C and V
/// are unchanged.
pub(crate) fn fill_buffer_4800(ctx: &mut Ctx<'_>) {
    for offset in buffer_offsets() {
        ctx.write16(BUFFER + offset, 0xFFFF);
    }
    ctx.write_db16(WORD_0D7D, 0);
    ctx.set_a(0xFFFF);
    ctx.set_x(0xFFFE);
    ctx.set_nz16(0xFFFE);
    ctx.set_cost(FILL_BUFFER_4800_COST);
}

/// The 128-byte table of `lookup_byte_pair` (`byte_pair_table`, bank `$03`): 64 pairs of bytes.
const PAIR_TABLE: u32 = data::BYTE_PAIR_TABLE;
/// CPU-busy master cycles of a call, from the hit to the return: one path for every index
/// (measured in `Verify`).
pub(crate) const LOOKUP_BYTE_PAIR_COST: u64 = 568;

/// The address of the pair for `index`: its low six bits pick one of the 64 pairs.
pub(crate) fn pair_address(index: u8) -> u32 {
    PAIR_TABLE + u32::from(index & 0x3F) * 2
}

/// A byte as the routine doubles it: shifted left, with bit 7 dropped.
fn doubled(byte: u8) -> u8 {
    byte << 1
}

/// `lookup_byte_pair`: the pair for the index in A's low byte goes to `$04` and `$06`, each
/// also doubled into `$14` and `$16`, and `$05`, `$07`, `$15` and `$17` are cleared. A = the
/// doubled second byte with B = 0; C = bit 7 of the second byte; N and Z from X (kept as it
/// was); V and X unchanged.
pub(crate) fn lookup_byte_pair(ctx: &mut Ctx<'_>) {
    let address = pair_address(ctx.al());
    let first = ctx.read8(address);
    let second = ctx.read8(address + 1);
    ctx.write_dp8(0x04, first);
    ctx.write_dp8(0x14, doubled(first));
    ctx.write_dp8(0x06, second);
    ctx.write_dp8(0x16, doubled(second));
    for offset in [0x05, 0x07, 0x15, 0x17] {
        ctx.write_dp8(offset, 0);
    }
    ctx.set_a(u16::from(doubled(second)));
    let x = ctx.x();
    if ctx.x8() {
        ctx.set_nz8(x as u8);
    } else {
        ctx.set_nz16(x);
    }
    ctx.set_c(second & 0x80 != 0);
    ctx.set_cost(LOOKUP_BYTE_PAIR_COST);
}

/// The low-WRAM block `clear_slot_plus_8` clears in (`slot_block_0800`, bank `$7E`); the
/// routine addresses it through the data bank as `$0800`.
const SLOT_BLOCK: u32 = data::SLOT_BLOCK_0800 & 0xFFFF;
/// CPU-busy master cycles of a call, from the hit to the return: one path for every index
/// (measured in `Verify`).
pub(crate) const CLEAR_SLOT_PLUS_8_COST: u64 = 260;

/// The slot cleared for X: X + 8 in the index width, so an 8-bit X wraps at 8 bits.
pub(crate) fn slot_index(x: u16, x8: bool) -> u16 {
    if x8 { u16::from((x as u8).wrapping_add(8)) } else { x.wrapping_add(8) }
}

/// The 24-bit address of the store: the data bank's `$0800` plus the slot index. A carry out
/// of the 16-bit sum goes into the bank byte, as the CPU's indexed addressing does.
pub(crate) fn slot_address(dbr: u8, index: u16) -> u32 {
    ((u32::from(dbr) << 16) + SLOT_BLOCK + u32::from(index)) & 0xFF_FFFF
}

/// `clear_slot_plus_8`: zero at the slot of X + 8 (one byte with an 8-bit accumulator, two
/// with a 16-bit one). X is restored, so N and Z come from X in its width; C, V and A are
/// unchanged.
pub(crate) fn clear_slot_plus_8(ctx: &mut Ctx<'_>) {
    let x = ctx.x();
    let x8 = ctx.x8();
    let address = slot_address(ctx.dbr(), slot_index(x, x8));
    if ctx.m8() {
        ctx.write8(address, 0);
    } else {
        ctx.write16(address, 0);
    }
    if x8 {
        ctx.set_nz8(x as u8);
    } else {
        ctx.set_nz16(x);
    }
    ctx.set_cost(CLEAR_SLOT_PLUS_8_COST);
}

/// The low-WRAM table `lookup_0d40_scaled` reads (`index_table_0d40`), at `$0D40` through the
/// data bank.
const INDEX_TABLE: u16 = (data::INDEX_TABLE_0D40 & 0xFFFF) as u16;
/// CPU-busy master cycles of a call, from the hit to the return: one path for every index
/// (measured in `Verify`).
pub(crate) const LOOKUP_0D40_SCALED_COST: u64 = 304;

/// X divided by four: the table index, 16 bits.
pub(crate) fn table_index(x: u16) -> u16 {
    x >> 2
}

/// The result: the high byte of the index (its bits 8 to 15, that is bits 10 to 17 of X) above
/// the table byte doubled with bit 7 dropped.
pub(crate) fn scaled(index: u16, byte: u8) -> u16 {
    (index & 0xFF00) | u16::from(byte << 1)
}

/// `lookup_0d40_scaled`: writes `$64`, `$A2` and `$02` to `$00`-`$02`, then A = Y = the scaled
/// table byte ([`scaled`]) with its high byte from the index. N and Z from that value; C = bit
/// 7 of the table byte; V and X unchanged.
pub(crate) fn lookup_0d40_scaled(ctx: &mut Ctx<'_>) {
    ctx.write_dp8(0, 0x64);
    ctx.write_dp8(1, 0xA2);
    ctx.write_dp8(2, 0x02);
    let index = table_index(ctx.x());
    let address = (u32::from(ctx.dbr()) << 16) + u32::from(INDEX_TABLE) + u32::from(index);
    let byte = ctx.read8(address);
    let value = scaled(index, byte);
    ctx.set_a(value);
    ctx.set_y(value);
    ctx.set_nz16(value);
    ctx.set_c(byte & 0x80 != 0);
    ctx.set_cost(LOOKUP_0D40_SCALED_COST);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routines::tests::max_cost;

    #[test]
    fn the_cost_is_constant() {
        assert_eq!(max_cost("lookup_bank2_pointer"), LOOKUP_BANK2_POINTER_COST);
    }

    #[test]
    fn the_table_is_256_bytes_in_bank_2() {
        assert_eq!(data::BANK2_POINTER_TABLE, 0x02_DA16);
        assert_eq!(data::BANK2_POINTER_TABLE_SIZE, 256);
    }

    #[test]
    fn the_index_is_doubled_and_kept_to_8_bits() {
        assert_eq!(table_offset(0x00), 0x00);
        assert_eq!(table_offset(0x35), 0x6A);
        assert_eq!(table_offset(0x7F), 0xFE);
        assert_eq!(table_offset(0x80), 0x00, "bit 7 of the index is shifted out");
        assert_eq!(table_offset(0xFF), 0xFE);
    }

    #[test]
    fn every_index_points_at_an_even_entry_of_the_table() {
        for index in 0..=u8::MAX {
            let pointer = entry_pointer(index);
            assert!((0xDA16..=0xDB15).contains(&pointer), "index {index:02X}: {pointer:04X}");
            assert_eq!((pointer - 0xDA16) % 2, 0, "index {index:02X}");
        }
        assert_eq!(entry_pointer(0x00), 0xDA16);
        assert_eq!(entry_pointer(0x35), 0xDA16 + 0x6A);
    }

    #[test]
    fn fill_buffer_4800_cost_is_constant() {
        assert_eq!(max_cost("fill_buffer_4800"), FILL_BUFFER_4800_COST);
    }

    #[test]
    fn the_buffer_is_one_kilobyte_of_even_word_stores() {
        let offsets: Vec<u32> = buffer_offsets().collect();
        assert_eq!(offsets.len(), 512);
        assert_eq!((offsets[0], offsets[511]), (0, 0x3FE));
        assert_eq!((BUFFER, BUFFER_LEN), (0x7E_4800, 0x400));
        assert_eq!(WORD_0D7D, 0x0D7D);
    }

    #[test]
    fn lookup_byte_pair_cost_is_constant() {
        assert_eq!(max_cost("lookup_byte_pair"), LOOKUP_BYTE_PAIR_COST);
    }

    #[test]
    fn the_index_picks_one_of_64_pairs_of_the_128_byte_table() {
        assert_eq!(data::BYTE_PAIR_TABLE_SIZE, 128);
        assert_eq!(pair_address(0x00), PAIR_TABLE);
        assert_eq!(pair_address(0x3F), PAIR_TABLE + 126);
        assert_eq!(pair_address(0x40), PAIR_TABLE, "bits 6 and 7 are ignored");
        assert_eq!(pair_address(0xFF), PAIR_TABLE + 126);
    }

    #[test]
    fn doubling_drops_bit_7() {
        assert_eq!(doubled(0x0C), 0x18);
        assert_eq!(doubled(0x80), 0x00);
        assert_eq!(doubled(0x7F), 0xFE);
    }

    #[test]
    fn clear_slot_plus_8_cost_is_constant() {
        assert_eq!(max_cost("clear_slot_plus_8"), CLEAR_SLOT_PLUS_8_COST);
    }

    #[test]
    fn the_slot_is_x_plus_8_in_the_index_width() {
        assert_eq!(slot_index(0x0000, false), 0x0008);
        assert_eq!(slot_index(0x0010, false), 0x0018);
        assert_eq!(slot_index(0xFFF8, false), 0x0000, "16-bit X wraps");
        assert_eq!(slot_index(0x00F8, true), 0x0000, "8-bit X wraps at 8 bits");
        assert_eq!(slot_index(0x0108, true), 0x0010, "the high byte of an 8-bit X is 0");
    }

    #[test]
    fn the_address_carries_into_the_bank() {
        assert_eq!(slot_address(0x00, 0x0008), 0x00_0808);
        assert_eq!(slot_address(0x7E, 0xFFFF), 0x7F_07FF);
        assert_eq!(slot_address(0xFF, 0xFFFF), 0x00_07FF, "the bank wraps at 24 bits");
    }

    #[test]
    fn lookup_0d40_scaled_cost_is_constant() {
        assert_eq!(max_cost("lookup_0d40_scaled"), LOOKUP_0D40_SCALED_COST);
    }

    #[test]
    fn the_index_is_x_over_four() {
        assert_eq!(table_index(0x0004), 1);
        assert_eq!(table_index(0x0C00), 0x0300);
        assert_eq!(table_index(0xFFFF), 0x3FFF);
    }

    #[test]
    fn the_result_takes_the_index_high_byte_and_the_doubled_byte() {
        assert_eq!(scaled(0x0300, 0x81), 0x0302, "bit 7 of the byte is dropped by the shift");
        assert_eq!(scaled(0x3FFF, 0x40), 0x3F80);
        assert_eq!(scaled(0x0001, 0x00), 0x0000);
    }
}
