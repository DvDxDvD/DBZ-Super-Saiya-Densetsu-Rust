//! Batch E of the M2 porting: a bank-`$7F` fill, a bit-plane unpack, a bit-tree decoder, a block
//! bit set, a gated entity pass, a palette blend, a decimal tens step, a bit-field store and a table
//! lookup on the split bits of a byte. Each routine has its own section below (in the order the
//! batch ports them); its semantics, contract and callers are in `docs/re/routines/<name>.md`.
//!
//! Costs are CPU-busy master cycles from the hit to the return. The 65816 spends 8 master cycles on
//! each bus access and 6 on each internal cycle, so a path's cost follows from its instructions (see
//! the notes); `probe --verify` checks every declared cost against the original.

// --- fill_bank7f_pattern ---

/// The bank the fill writes (`$7F`): the long pointer the routine builds at `$73` is `$7F:0000`.
const FILL_BASE: u32 = 0x7F_0000;
/// Where the second pass starts (`$7F80`): its words are `$00FF`, so their odd bytes clear.
const FILL_SPLIT: u32 = 0x7F80;
/// The end of the fill (`$8000`).
const FILL_END: u32 = 0x8000;
/// CPU-busy master cycles of a call: one path (both loops always run in full), measured in `Verify`.
pub(crate) const FILL_BANK7F_PATTERN_COST: u64 = 1_999_596;

/// `fill_bank7f_pattern`: in bank `$7F`, `$FF` at every even offset below `$7F80` (the odd bytes keep
/// their values), then the word `$00FF` at every even offset from `$7F80` to `$7FFE` (the odd bytes
/// become `$00`). The pointer words at `$73-$75` end as `$7F:0000`. Leaves A = `$00FF` (16-bit),
/// Y = `$8000`, N clear, Z and C set (the last compare with `$8000`); V and X are kept.
pub(crate) fn fill_bank7f_pattern(ctx: &mut ssd_oracle::Ctx<'_>) {
    for offset in (0..FILL_SPLIT).step_by(2) {
        ctx.write8(FILL_BASE + offset, 0xFF);
    }
    for offset in (FILL_SPLIT..FILL_END).step_by(2) {
        ctx.write16(FILL_BASE + offset, 0x00FF);
    }
    ctx.write_dp16(0x73, 0x0000);
    ctx.write_dp8(0x75, 0x7F);
    ctx.set_a(0x00FF);
    ctx.set_y(FILL_END as u16);
    ctx.set_n(false);
    ctx.set_z(true);
    ctx.set_c(true);
    ctx.set_cost(FILL_BANK7F_PATTERN_COST);
}

#[cfg(test)]
mod tests_fill_bank7f_pattern {
    use super::*;
    use crate::routines::tests::max_cost;

    #[test]
    fn the_fill_cost_is_the_measured_one() {
        assert_eq!(max_cost("fill_bank7f_pattern"), FILL_BANK7F_PATTERN_COST);
    }

    #[test]
    fn the_two_passes_split_at_7f80_and_end_at_8000() {
        assert_eq!(FILL_SPLIT, 0x7F80);
        assert_eq!((FILL_END - FILL_SPLIT) / 2, 0x40, "64 word stores in the second pass");
        assert_eq!(FILL_BASE + FILL_END - 1, 0x7F_7FFF, "the fill stays in bank $7F");
    }
}
// --- unpack_bitplanes_to_7f ---

/// Entry of the routine (`$04:83B2`), for the hand-back to the original code.
const UNPACK_ENTRY: u32 = 0x04_83B2;
/// The output bytes: one per pixel, at `$7F:0001` plus X (two bytes apart).
const UNPACK_OUT: u32 = 0x7F_0001;
/// The offset of the second plane word from the first one (`$0010`).
const UNPACK_PLANE_OFFSET: u32 = 0x10;
/// The reference call (the only one the port runs): the planes start at `$9000` in the bank at `$88`
/// (`$7E`), and the span word at `$89` is `$0660` (the end is `$9660`).
const UNPACK_START: u16 = 0x9000;
const UNPACK_BANK: u8 = 0x7E;
const UNPACK_SPAN: u16 = 0x0660;
/// CPU-busy master cycles of the reference call (51 blocks of eight rows), measured in `Verify`.
pub(crate) const UNPACK_BITPLANES_COST: u64 = 1_313_224;

/// Whether the call is the reference one: the start, the bank and the span of the planes.
pub(crate) fn is_reference_unpack(start: u16, bank: u8, span: u16) -> bool {
    start == UNPACK_START && bank == UNPACK_BANK && span == UNPACK_SPAN
}

/// The bound of a call of `unpack_bitplanes_to_7f`: the reference call's cost, and no bound on any
/// other call (`u64::MAX`): its loop length depends on the entry, so the original runs it.
pub(crate) fn unpack_bitplanes_bound(ctx: &ssd_oracle::Ctx<'_>) -> u64 {
    if is_reference_unpack(ctx.peek_dp16(0x86), ctx.peek_dp8(0x88), ctx.peek_dp16(0x89)) {
        UNPACK_BITPLANES_COST
    } else {
        u64::MAX
    }
}

/// The eight pixels of one row of the planes: the first word holds the low and high bytes of plane
/// pair 1 and the second word those of pair 2; pixel `k` takes bit `7 - k` of each byte, and its
/// value is (high of pair 2, low of pair 2, high of pair 1, low of pair 1), most significant first.
pub(crate) fn unpack_row(first: u16, second: u16) -> [u8; 8] {
    let mut pixels = [0_u8; 8];
    for (k, pixel) in pixels.iter_mut().enumerate() {
        let bit = 7 - k;
        let plane = |word: u16, shift: usize| ((word >> (bit + shift)) & 0x01) as u8;
        *pixel = (plane(second, 8) << 3)
            | (plane(second, 0) << 2)
            | (plane(first, 8) << 1)
            | plane(first, 0);
    }
    pixels
}

/// `unpack_bitplanes_to_7f`: for the planes of the reference call (`$86` = start, `$88` = bank, `$89`
/// = span; the end is their sum), each row of eight pixels is unpacked from the words at the row's
/// offset and the offset plus `$10` (both in the bank at `$88`). Rows follow each other by two bytes;
/// after the eighth row of a block the offset moves on by `$20`. The pixels go to `$7F:0001` plus X,
/// X moving by two per pixel. Leaves A = Y = the end offset reached (16-bit), X = the output index
/// reached; the direct page holds its scratch values (`$73`, `$75`, `$8D`, `$8F`, `$89` shifted,
/// `$91`-`$94` zero); flags, the data bank and the accumulator width are kept.
///
/// Only the reference call runs in Rust; any other call runs the original code.
pub(crate) fn unpack_bitplanes_to_7f(ctx: &mut ssd_oracle::Ctx<'_>) {
    let start = ctx.read_dp16(0x86);
    let bank = ctx.read_dp8(0x88);
    let span = ctx.read_dp16(0x89);
    if !is_reference_unpack(start, bank, span) {
        let Ok(()) = ctx.call_original(UNPACK_ENTRY, ssd_oracle::ReturnKind::Rtl) else { return };
        ctx.set_cost(0);
        return;
    }
    let end = span.wrapping_add(start);
    let source = u32::from(bank) << 16;
    let mut x = ctx.x();
    let mut y = start;
    loop {
        let first = ctx.read16(source + u32::from(y));
        let second = ctx.read16(source + u32::from(y) + UNPACK_PLANE_OFFSET);
        for pixel in unpack_row(first, second) {
            ctx.write8(UNPACK_OUT + u32::from(x), pixel);
            x = x.wrapping_add(2);
        }
        y = y.wrapping_add(2);
        if y & 0x000F != 0 {
            continue;
        }
        y = y.wrapping_add(0x10);
        if y >= end {
            break;
        }
    }
    ctx.write_dp16(0x73, 0x000F);
    ctx.write_dp16(0x75, 0x0010);
    ctx.write_dp16(0x8D, end);
    ctx.write_dp8(0x8F, 0xFF);
    ctx.write_dp16(0x89, span << 1);
    for offset in [0x91, 0x92, 0x93, 0x94] {
        ctx.write_dp8(offset, 0);
    }
    ctx.set_a(y);
    ctx.set_x(x);
    ctx.set_y(y);
    ctx.set_cost(UNPACK_BITPLANES_COST);
}

#[cfg(test)]
mod tests_unpack_bitplanes_to_7f {
    use super::*;

    #[test]
    fn the_unpacked_pixels_take_one_bit_of_each_plane_per_pixel() {
        // Pair 1: low $80 (bit 7 only), high $00; pair 2: low $00, high $80.
        let pixels = unpack_row(0x0080, 0x8000);
        assert_eq!(
            pixels[0], 0b1001,
            "bit 7 of the high byte of pair 2 and of the low byte of pair 1"
        );
        assert!(pixels[1..].iter().all(|&p| p == 0));
        // The last pixel takes bit 0 of each byte.
        assert_eq!(unpack_row(0x0001, 0x0100)[7], 0b1001, "bit 0 of each byte");
    }

    #[test]
    fn the_reference_call_is_the_measured_one() {
        assert!(is_reference_unpack(0x9000, 0x7E, 0x0660));
        assert!(!is_reference_unpack(0x9000, 0x7E, 0x0680));
        assert_eq!(UNPACK_BITPLANES_COST, 1_313_224);
    }
}
// --- decode_bit_tree_to_9000 ---

/// Entry of the routine (`$00:FDB5`), for the hand-back to the original code.
const DECODE_ENTRY: u32 = 0x00_FDB5;
/// The bit stream the routine reads from `$00:F150`, one byte at a time from the entry index.
const DECODE_STREAM: u32 = 0x00_F150;
/// The node table: a pair of words per node, read at the node's doubled index (`$00:FD49`).
const DECODE_TREE: u32 = 0x00_FD49;
/// The bytes the routine writes, two per symbol (`$7E:9000`, the data bank it leaves set).
const DECODE_OUT: u32 = 0x7E_9000;
/// The first node of the walk (the root).
const DECODE_ROOT: u16 = 0x001A;
/// The index the output stops at (`X` is `$1000` at exit).
const DECODE_END: u16 = 0x1000;
/// CPU-busy master cycles of a call from the entry index 0 and data index 0 (the only entry the
/// reference replays make), measured in `Verify`.
pub(crate) const DECODE_TREE_COST: u64 = 3_294_484;

/// The cost bound of a call of `decode_bit_tree_to_9000`: the measured cost from the reference
/// entry, and no bound (`u64::MAX`) from any other entry, which the original code runs.
pub(crate) fn decode_bit_tree_bound(ctx: &ssd_oracle::Ctx<'_>) -> u64 {
    if ctx.x() == 0 && ctx.y() == 0 { DECODE_TREE_COST } else { u64::MAX }
}

/// `decode_bit_tree_to_9000`: walks the node table once per output symbol, a bit of the stream per
/// step (LSB first; the stream index advances after bit 7), until a node whose high byte has bit 7
/// set. Each symbol's low byte goes to `$7E:9000 + X` with a zero byte after it; X counts by 2 from
/// its entry value to `$1000`. Leaves X = `$1000`, Y = the stream index reached, DBR = `$7E`, A = the
/// entry word of `$00` (the direct page is restored, so its bytes are unchanged), N and Z from that
/// word, C set, V from the high byte of the last node.
///
/// Only the entry with X = 0 and Y = 0 (the reference replays' calls) runs in Rust; other entries run
/// the original code.
pub(crate) fn decode_bit_tree_to_9000(ctx: &mut ssd_oracle::Ctx<'_>) {
    if ctx.x() != 0 || ctx.y() != 0 {
        let Ok(()) = ctx.call_original(DECODE_ENTRY, ssd_oracle::ReturnKind::Rtl) else { return };
        ctx.set_cost(0);
        return;
    }
    let saved = ctx.read_dp16(0x00);
    let mut stream = 0_u16;
    let mut mask = 0x01_u8;
    let mut out = 0_u16;
    let mut last_high: u8;
    loop {
        let mut node = DECODE_ROOT;
        loop {
            let byte = ctx.read8(DECODE_STREAM + u32::from(stream));
            let bit = byte & mask != 0;
            let index = (node << 2).wrapping_add(if bit { 2 } else { 0 });
            node = ctx.read16(DECODE_TREE + u32::from(index));
            if mask & 0x80 != 0 {
                mask = 0x01;
                stream = stream.wrapping_add(1);
            } else {
                mask <<= 1;
            }
            last_high = (node >> 8) as u8;
            if last_high & 0x80 != 0 {
                break;
            }
        }
        ctx.write8(DECODE_OUT + u32::from(out), node as u8);
        ctx.write8(DECODE_OUT + 1 + u32::from(out), 0);
        out = out.wrapping_add(2);
        if out == DECODE_END {
            break;
        }
    }
    ctx.set_a(saved);
    ctx.set_x(DECODE_END);
    ctx.set_y(stream);
    ctx.set_dbr(0x7E);
    ctx.set_n(saved & 0x8000 != 0);
    ctx.set_z(saved == 0);
    ctx.set_c(true);
    ctx.set_v(last_high & 0x40 != 0);
    ctx.set_cost(DECODE_TREE_COST);
}

#[cfg(test)]
mod tests_decode_bit_tree_to_9000 {
    use super::*;

    #[test]
    fn the_decoder_writes_2048_symbols_from_the_root() {
        assert_eq!(DECODE_ROOT, 0x001A);
        assert_eq!(DECODE_END / 2, 2048);
        assert_eq!(DECODE_TREE_COST, 3_294_484);
    }
}
// --- set_bit12_block_9000 ---

/// The first byte of the block the routine ORs bit 12 into (`$7E:9000`, 1024 words).
const BIT12_BLOCK: u32 = 0x7E_9000;
/// CPU-busy master cycles of a call: one path (the loop always runs in full), measured in `Verify`.
pub(crate) const SET_BIT12_BLOCK_COST: u64 = 174_184;

/// `set_bit12_block_9000`: bit 12 is set in each word of `$7E:9000` to `$7E:97FE`, from the last word
/// down. Leaves A = the word at `$7E:9000` with bit 12 set (16-bit), X = `$FFFE`, N set and Z clear
/// (from the last decrement); C and V are kept. The accumulator is 8-bit at exit, as the original
/// leaves it, so the routine's entry must be 8-bit too (`entry_m8`).
pub(crate) fn set_bit12_block_9000(ctx: &mut ssd_oracle::Ctx<'_>) {
    let mut word = 0;
    for offset in (0..=0x07FE_u32).rev().step_by(2) {
        word = ctx.read16(BIT12_BLOCK + offset) | 0x1000;
        ctx.write16(BIT12_BLOCK + offset, word);
    }
    ctx.set_a(word);
    ctx.set_x(0xFFFE);
    ctx.set_n(true);
    ctx.set_z(false);
    ctx.set_cost(SET_BIT12_BLOCK_COST);
}

#[cfg(test)]
mod tests_set_bit12_block_9000 {
    use super::*;
    use crate::routines::tests::max_cost;

    #[test]
    fn the_block_is_1024_words_from_the_last_down() {
        let offsets: Vec<u32> = (0..=0x07FE_u32).rev().step_by(2).collect();
        assert_eq!(offsets.len(), 1024);
        assert_eq!((offsets[0], offsets[1023]), (0x07FE, 0));
        assert_eq!(max_cost("set_bit12_block_9000"), SET_BIT12_BLOCK_COST);
    }
}
// --- entity_draw_pass_60_80 ---

/// Entry of the routine (`$01:F9AC`), for the hand-back to the original code.
const ENTITY_PASS_ENTRY: u32 = 0x01_F9AC;
/// The gate byte (`gate_flag_01bc`), read through the data bank.
const GATE_FLAG: u16 = (crate::data::GATE_FLAG_01BC & 0xFFFF) as u16;
/// CPU-busy master cycles of the gated return: a load, an AND, a taken branch and the return.
pub(crate) const ENTITY_PASS_GATED_COST: u64 = 114;

/// Whether the call returns at once: bit 0 of the gate byte is set.
pub(crate) fn is_gated(gate: u8) -> bool {
    gate & 0x01 != 0
}

/// The cost bound of a call of `entity_draw_pass_60_80`: the gated return's cost, and no bound
/// (`u64::MAX`) on the other path, which therefore always runs the original code.
pub(crate) fn entity_pass_bound(ctx: &ssd_oracle::Ctx<'_>) -> u64 {
    if is_gated(ctx.peek_db8(GATE_FLAG)) { ENTITY_PASS_GATED_COST } else { u64::MAX }
}

/// `entity_draw_pass_60_80`: when bit 0 of the gate byte is set, the call returns at once with A low
/// = 1 (B kept) and N and Z of that value (both clear); C, V, X and Y are kept. Otherwise the original
/// code runs from the entry (`call_original`, own cost 0): the entity slots of the pass are only
/// reached on that path.
pub(crate) fn entity_draw_pass_60_80(ctx: &mut ssd_oracle::Ctx<'_>) {
    let gate = ctx.read_db8(GATE_FLAG);
    if !is_gated(gate) {
        let Ok(()) = ctx.call_original(ENTITY_PASS_ENTRY, ssd_oracle::ReturnKind::Rtl) else {
            return;
        };
        ctx.set_cost(0);
        return;
    }
    ctx.set_al(1);
    ctx.set_nz8(1);
    ctx.set_cost(ENTITY_PASS_GATED_COST);
}

#[cfg(test)]
mod tests_entity_draw_pass_60_80 {
    use super::*;

    #[test]
    fn the_gate_is_bit_zero() {
        assert!(is_gated(0x01) && is_gated(0xFF));
        assert!(!is_gated(0x00) && !is_gated(0xFE));
    }

    #[test]
    fn the_gated_cost_is_the_cost_of_its_four_instructions() {
        // Load (4 accesses), AND (2), taken branch (2 accesses and one internal), return (6).
        assert_eq!(ENTITY_PASS_GATED_COST, 4 * 8 + 2 * 8 + (2 * 8 + 6) + (4 * 8 + 2 * 6));
    }
}
// --- blend_palette_slots ---

/// Entry of the routine (`$05:C116`), for the hand-back to the original code.
const BLEND_ENTRY: u32 = 0x05_C116;
/// The palette slots the routine copies into and blends (`$0200` plus X, DBR-relative).
const PALETTE: u16 = 0x0200;
/// The source of the copy (`$15E1` plus X, DBR-relative).
const COPY_SOURCE: u16 = 0x15E1;
/// The table of blend words: four bytes per index (`$7F` times four), the first word for a palette
/// word that is not zero (`blend_table_c1b8`, bank `$05`).
const BLEND_TABLE: u32 = crate::data::BLEND_TABLE_C1B8;
/// The copy-done flag the routine increments (`copy_done`).
const COPY_DONE: u16 = (crate::data::COPY_DONE & 0xFFFF) as u16;
/// The index registers the reference replays use at entry: the only ones this port runs.
const REFERENCE_X: [u16; 2] = [0x0120, 0x0140];
/// CPU-busy master cycles of each path, measured in `Verify`: nothing to do, the copy (with the index
/// zero, and otherwise), and the blend of sixteen slots (no zero slot, no clamp).
pub(crate) const BLEND_COST_NOTHING: u64 = 222;
pub(crate) const BLEND_COST_COPY_ZERO: u64 = 4968;
pub(crate) const BLEND_COST_COPY: u64 = 5018;
pub(crate) const BLEND_COST_BLEND: u64 = 19308;
/// Instructions of each path (opcode fetches from the hit to the return): the copy from index zero,
/// the copy otherwise, the bookkeeping only, and the sixteen blended slots (40 each, no zero slot or
/// clamp).
pub(crate) const BLEND_INSTRUCTIONS_COPY_ZERO: u64 = 198;
pub(crate) const BLEND_INSTRUCTIONS_COPY: u64 = 201;
pub(crate) const BLEND_INSTRUCTIONS_NOTHING: u64 = 10;
pub(crate) const BLEND_INSTRUCTIONS_BLEND: u64 = 661;

/// The path of a call, from the low byte of A (the index `$7F`) and the byte at `$4F`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlendPath {
    /// The 32-byte copy; `zero` when the index is zero.
    Copy { zero: bool },
    /// The low bits of `$4F` are set: nothing but the bookkeeping.
    Nothing,
    /// The sixteen slots are blended with the table word of the index.
    Blend,
}

/// The path a call takes ([`BlendPath`]).
pub(crate) fn blend_path(a_low: u8, bits: u8) -> BlendPath {
    let index = a_low & 0x7F;
    if index == 0 || bits & 0x04 == 0 {
        BlendPath::Copy { zero: index == 0 }
    } else if bits & 0x03 != 0 {
        BlendPath::Nothing
    } else {
        BlendPath::Blend
    }
}

/// One slot blended with the table word: each channel's sum, kept only while it stays below the
/// routine's limit (red below 32, green below `$0400`, blue below `$8000`). `None` when a channel
/// would be clamped, which the port leaves to the original code.
pub(crate) fn blend_slot(slot: u16, table: u16) -> Option<u16> {
    let red = (slot & 0x001F) + (table & 0x001F);
    let green = (slot & 0x03E0) + (table & 0x03E0);
    let blue = (slot & 0x7C00) + (table & 0x7C00);
    (red < 0x20 && green < 0x0400 && blue < 0x8000).then_some(red | green | blue)
}

/// The sixteen blended slots, or `None` when one is zero or would be clamped ([`blend_slot`]).
pub(crate) fn blend_slots(slots: &[u16; 16], table: u16) -> Option<[u16; 16]> {
    let mut out = [0_u16; 16];
    for (result, &slot) in out.iter_mut().zip(slots) {
        if slot == 0 {
            return None;
        }
        *result = blend_slot(slot, table)?;
    }
    Some(out)
}

/// The address of the table word for the index (`$7F` times four).
fn blend_table_address(index: u8) -> u32 {
    BLEND_TABLE + u32::from(index) * 4
}

/// The addresses of the sixteen slots of a call at palette index `x` (`$0200` plus X, in the data
/// bank).
fn palette_addresses(dbr: u8, x: u16) -> [u32; 16] {
    let base = (u32::from(dbr) << 16) + u32::from(PALETTE) + u32::from(x);
    std::array::from_fn(|i| base + 2 * i as u32)
}

/// The cost bound of a call of `blend_palette_slots`: the path's cost where the port runs it, and no
/// bound (`u64::MAX`) where the original code runs.
pub(crate) fn blend_palette_bound(ctx: &ssd_oracle::Ctx<'_>) -> u64 {
    let x = ctx.x();
    if !REFERENCE_X.contains(&x) {
        return u64::MAX;
    }
    match blend_path(ctx.al(), ctx.peek_dp8(0x4F)) {
        BlendPath::Copy { zero: true } => BLEND_COST_COPY_ZERO,
        BlendPath::Copy { zero: false } => BLEND_COST_COPY,
        BlendPath::Nothing => BLEND_COST_NOTHING,
        BlendPath::Blend => {
            let table = ctx.peek16(blend_table_address(ctx.al() & 0x7F));
            let slots = palette_addresses(ctx.dbr(), x).map(|address| ctx.peek16(address));
            if blend_slots(&slots, table).is_some() { BLEND_COST_BLEND } else { u64::MAX }
        }
    }
}

/// `blend_palette_slots`, for the palette index of the reference replays. Index zero, or a `$4F` with
/// bit 2 clear, copies 32 bytes from `$15E1` plus X to `$0200` plus X (cost by the index). Otherwise
/// `$4F` with its low bits set changes nothing (A = those bits). Otherwise the sixteen slots from
/// `$0200` plus X are blended with the table word of the index ([`blend_slots`]). Each path increments
/// `copy_done`. The index is kept at `$00` (and `$00-$01` is the index times four on the blend path);
/// the other states are as the original leaves them (see the notes). A call the port does not run
/// (another index register, a zero or clamped slot) runs the original code from the entry.
pub(crate) fn blend_palette_slots(ctx: &mut ssd_oracle::Ctx<'_>) {
    let x = ctx.x();
    let a_low = ctx.al();
    let bits = ctx.read_dp8(0x4F);
    let path = blend_path(a_low, bits);
    let index = a_low & 0x7F;
    let slots = palette_addresses(ctx.dbr(), x).map(|address| ctx.read16(address));
    let table = ctx.read16(blend_table_address(index));
    let blended = match path {
        BlendPath::Blend => blend_slots(&slots, table),
        _ => None,
    };
    if !REFERENCE_X.contains(&x) || (path == BlendPath::Blend && blended.is_none()) {
        let Ok(()) = ctx.call_original(BLEND_ENTRY, ssd_oracle::ReturnKind::Rts) else { return };
        ctx.set_cost(0);
        return;
    }
    ctx.write_dp8(0x00, index);
    match path {
        BlendPath::Copy { zero } => {
            let mut last = 0;
            for offset in 0..0x20_u16 {
                let source = (u32::from(ctx.dbr()) << 16)
                    + u32::from(COPY_SOURCE)
                    + u32::from(x.wrapping_add(offset));
                last = ctx.read8(source);
                let target = (u32::from(ctx.dbr()) << 16)
                    + u32::from(PALETTE)
                    + u32::from(x.wrapping_add(offset));
                ctx.write8(target, last);
            }
            ctx.set_al(last);
            ctx.set_x(x.wrapping_add(0x20));
            ctx.set_y(0x0020);
            ctx.set_c(true);
            increment_copy_done(ctx);
            ctx.set_cost(if zero { BLEND_COST_COPY_ZERO } else { BLEND_COST_COPY });
            ctx.set_instructions(if zero {
                BLEND_INSTRUCTIONS_COPY_ZERO
            } else {
                BLEND_INSTRUCTIONS_COPY
            });
        }
        BlendPath::Nothing => {
            ctx.set_al(bits & 0x03);
            ctx.set_nz8(bits & 0x03);
            ctx.set_cost(BLEND_COST_NOTHING);
            ctx.set_instructions(BLEND_INSTRUCTIONS_NOTHING);
        }
        BlendPath::Blend => {
            let out = blended.unwrap_or_default();
            for (offset, word) in (0..16_u32).zip(out) {
                let target =
                    (u32::from(ctx.dbr()) << 16) + u32::from(PALETTE) + u32::from(x) + 2 * offset;
                ctx.write16(target, word);
            }
            let last = out[15];
            ctx.write_dp16(0x00, u16::from(index) << 2);
            ctx.write_dp16(0x02, 0xFFFF);
            ctx.write_dp16(0x04, table);
            ctx.write_dp16(0x06, table & 0x7C00);
            ctx.write_dp16(0x10, last & 0x03FF);
            ctx.set_a(last);
            ctx.set_x(u16::from(index) << 2);
            ctx.set_y(x.wrapping_add(0x20));
            ctx.set_c(false);
            ctx.set_v(false);
            increment_copy_done(ctx);
            ctx.set_cost(BLEND_COST_BLEND);
            ctx.set_instructions(BLEND_INSTRUCTIONS_BLEND);
        }
    }
}

/// The `INC` of `copy_done` (a byte, DBR-relative): N and Z from the new value.
fn increment_copy_done(ctx: &mut ssd_oracle::Ctx<'_>) {
    let value = ctx.read_db8(COPY_DONE).wrapping_add(1);
    ctx.write_db8(COPY_DONE, value);
    ctx.set_nz8(value);
}

#[cfg(test)]
mod tests_blend_palette_slots {
    use super::*;

    #[test]
    fn the_blend_paths_follow_the_index_and_the_bits_of_4f() {
        assert_eq!(blend_path(0x80, 0x00), BlendPath::Copy { zero: true }, "index zero copies");
        assert_eq!(blend_path(0x03, 0x00), BlendPath::Copy { zero: false });
        assert_eq!(blend_path(0x03, 0x04), BlendPath::Blend, "bit 2 set, no low bits");
        assert_eq!(blend_path(0x03, 0x05), BlendPath::Nothing, "low bits set");
        assert_eq!(blend_path(0x00, 0x04), BlendPath::Copy { zero: true });
    }

    #[test]
    fn a_slot_blends_per_channel_and_clamps_are_refused() {
        assert_eq!(blend_slot(0x0421, 0x0102), Some(0x0523));
        assert_eq!(blend_slot(0x001F, 0x0001), None, "red reaches 32");
        assert_eq!(blend_slot(0x7C00, 0x0400), None, "blue overflows");
        let slots = [0x0421_u16; 16];
        assert!(blend_slots(&slots, 0x0102).is_some());
        assert!(blend_slots(&[0_u16; 16], 0x0102).is_none(), "a zero slot is not blended");
    }

    #[test]
    fn the_costs_are_the_measured_ones() {
        assert_eq!((BLEND_COST_NOTHING, BLEND_COST_COPY_ZERO, BLEND_COST_COPY), (222, 4968, 5018));
        assert_eq!(BLEND_COST_BLEND, 19308);
    }
}
// --- bin_to_bcd_from_tens ---

/// CPU-busy master cycles of one pass of the loop (a load, a subtraction, a store, an addition, a
/// store and the taken branch), and of the two exits after the passes: the borrow exit (the ones
/// byte below ten) and the wrap exit (the tens byte back to zero, the pass that wraps counted in the
/// passes).
const TENS_PASS_COST: u64 = 194;
const TENS_BORROW_EXIT_COST: u64 = 192;
const TENS_WRAP_EXIT_COST: u64 = 110;
/// Instructions of one pass of the loop, and of the exits (the borrow exit's failing test and the
/// final step; the wrap exit has the final step only).
const TENS_PASS_INSTRUCTIONS: u64 = 10;
const TENS_BORROW_EXIT_INSTRUCTIONS: u64 = 8;
const TENS_WRAP_EXIT_INSTRUCTIONS: u64 = 4;

/// Overflow flag of `a - b` (the result `r`).
fn sub_overflows(a: u8, b: u8, r: u8) -> bool {
    (a ^ b) & (a ^ r) & 0x80 != 0
}

/// Overflow flag of `a + b` (the result `r`).
fn add_overflows(a: u8, b: u8, r: u8) -> bool {
    !(a ^ b) & (a ^ r) & 0x80 != 0
}

/// CPU-busy master cycles of a call of `bin_to_bcd_from_tens` with `passes` passes: the borrow exit
/// counts the full passes only, the wrap exit counts the passes including the one that wraps.
pub(crate) fn tens_cost(passes: u64, wrapped: bool) -> u64 {
    if wrapped {
        TENS_PASS_COST * passes + TENS_WRAP_EXIT_COST
    } else {
        TENS_PASS_COST * passes + TENS_BORROW_EXIT_COST
    }
}

/// Instructions of a call of `bin_to_bcd_from_tens` with `passes` passes, counted as [`tens_cost`].
pub(crate) fn tens_instructions(passes: u64, wrapped: bool) -> u64 {
    if wrapped {
        TENS_PASS_INSTRUCTIONS * passes + TENS_WRAP_EXIT_INSTRUCTIONS
    } else {
        TENS_PASS_INSTRUCTIONS * passes + TENS_BORROW_EXIT_INSTRUCTIONS
    }
}

/// `bin_to_bcd_from_tens`: while the ones byte at `$00` is at least ten, it drops ten and the tens
/// byte at `$03` gains sixteen; the loop ends when the ones byte is below ten, or when the tens byte
/// wraps to zero. Then `$03` takes its OR with `$00`, and A low is that value (B kept). N and Z come
/// from that value; C and V from the last subtraction or addition.
pub(crate) fn bin_to_bcd_from_tens(ctx: &mut ssd_oracle::Ctx<'_>) {
    let mut ones = ctx.read_dp8(0x00);
    let mut tens = ctx.read_dp8(0x03);
    let mut passes = 0;
    let (carry, overflow, wrapped) = loop {
        if ones < 10 {
            break (false, sub_overflows(ones, 10, ones.wrapping_sub(10)), false);
        }
        ones -= 10;
        ctx.write_dp8(0x00, ones);
        let sum = tens.wrapping_add(0x10);
        let carry = u16::from(tens) + 0x10 > 0xFF;
        let overflow = add_overflows(tens, 0x10, sum);
        tens = sum;
        ctx.write_dp8(0x03, tens);
        passes += 1;
        if tens == 0 {
            break (carry, overflow, true);
        }
    };
    let value = tens | ones;
    ctx.write_dp8(0x03, value);
    ctx.set_al(value);
    ctx.set_nz8(value);
    ctx.set_c(carry);
    ctx.set_v(overflow);
    ctx.set_cost(tens_cost(passes, wrapped));
    ctx.set_instructions(tens_instructions(passes, wrapped));
}

#[cfg(test)]
mod tests_bin_to_bcd_from_tens {
    use super::*;
    use crate::routines::tests::max_cost;

    #[test]
    fn tens_passes_follow_the_formulas() {
        // A borrow exit after no pass: 192; each pass adds 194.
        assert_eq!(tens_cost(0, false), 192);
        assert_eq!(tens_cost(1, false), 386);
        assert_eq!(tens_cost(4, false), 968);
        assert_eq!(tens_cost(8, false), 1744);
        // A wrap exit: 110 plus 194 for each pass, the wrapping pass included.
        assert_eq!(tens_cost(1, true), 304);
        assert_eq!(tens_cost(16, true), 3214);
        assert_eq!(tens_instructions(0, false), 8);
        assert_eq!(tens_instructions(1, false), 18);
        assert_eq!(tens_instructions(1, true), 14);
    }

    #[test]
    fn the_static_maximum_covers_every_path() {
        // The most passes: 25 subtractions of ten from a byte of 255 (a borrow exit).
        assert_eq!(tens_cost(25, false), 5042);
        assert_eq!(max_cost("bin_to_bcd_from_tens"), 5042);
        // A wrap needs a tens byte with a zero low nibble: at most 16 passes.
        assert!((0..=16).all(|passes| tens_cost(passes, true) <= 5042));
    }

    #[test]
    fn the_overflow_flags_are_the_signed_ones() {
        assert!(sub_overflows(0x80, 0x0A, 0x76), "-128 - 10 overflows");
        assert!(!sub_overflows(0x0A, 0x0A, 0x00));
        assert!(add_overflows(0x70, 0x10, 0x80), "112 + 16 overflows");
        assert!(!add_overflows(0xF0, 0x10, 0x00), "-16 + 16 does not");
    }
}
// --- store_code_from_low_bits ---

/// The source byte read when bit 0 of the gate byte is set (`source_byte_0048`).
const SOURCE_SET: u16 = (crate::data::SOURCE_BYTE_0048 & 0xFFFF) as u16;
/// The source byte read when bit 0 of the gate byte is clear (`source_byte_0044`).
const SOURCE_CLEAR: u16 = (crate::data::SOURCE_BYTE_0044 & 0xFFFF) as u16;
/// The byte the code is stored to (`code_byte_0d59`).
const CODE_BYTE: u16 = (crate::data::CODE_BYTE_0D59 & 0xFFFF) as u16;

/// The code of the lowest set bit of the low nibble: bit 0 gives 3, bit 1 gives 2, bit 2 gives 0 and
/// bit 3 gives 1; `None` when the nibble is empty.
pub(crate) fn code_of_low_bits(value: u8) -> Option<u8> {
    if value & 0x01 != 0 {
        Some(3)
    } else if value & 0x02 != 0 {
        Some(2)
    } else if value & 0x04 != 0 {
        Some(0)
    } else if value & 0x08 != 0 {
        Some(1)
    } else {
        None
    }
}

/// CPU-busy master cycles and instructions of a call of `store_code_from_low_bits` on a path: the gate
/// bit (set: the `$0048` source and a taken branch to the shared part; clear: `$0044`), then the shifts
/// up to the lowest set bit of the source's low nibble (`code`; `None`: no bit set).
pub(crate) fn store_path(gate_set: bool, code: Option<u8>) -> (u64, u64) {
    let (prefix_cost, prefix_instructions) = if gate_set { (118, 5) } else { (102, 4) };
    let (shift_cost, shift_instructions) = match code {
        Some(0) => (216, 10),
        Some(1) => (230, 11),
        Some(2) => (180, 8),
        Some(3) => (144, 6),
        Some(_) => unreachable!("the codes are 0 to 3"),
        None => (188, 9),
    };
    (prefix_cost + shift_cost, prefix_instructions + shift_instructions)
}

/// `store_code_from_low_bits`: A low takes the byte at `$0048` when bit 0 of the gate byte at `$01BC`
/// is set, else the byte at `$0044` (DBR-relative). The code of its low nibble ([`code_of_low_bits`])
/// is stored to `$0D59` (DBR-relative) and returned in A low. With no code, A low is the high nibble
/// and nothing is stored. N is clear; Z is set for a zero value; C is set for a code and clear
/// otherwise. B, V, X and Y are kept.
pub(crate) fn store_code_from_low_bits(ctx: &mut ssd_oracle::Ctx<'_>) {
    let gate_set = ctx.read_db8(GATE_FLAG) & 0x01 != 0;
    let source = if gate_set { ctx.read_db8(SOURCE_SET) } else { ctx.read_db8(SOURCE_CLEAR) };
    let code = code_of_low_bits(source);
    match code {
        Some(code) => {
            ctx.write_db8(CODE_BYTE, code);
            ctx.set_al(code);
            ctx.set_nz8(code);
            ctx.set_c(true);
        }
        None => {
            let high = source >> 4;
            ctx.set_al(high);
            ctx.set_nz8(high);
            ctx.set_c(false);
        }
    }
    let (cost, instructions) = store_path(gate_set, code);
    ctx.set_cost(cost);
    ctx.set_instructions(instructions);
}

#[cfg(test)]
mod tests_store_code_from_low_bits {
    use super::*;
    use crate::routines::tests::max_cost;

    #[test]
    fn the_code_follows_the_lowest_set_bit_of_the_low_nibble() {
        assert_eq!(code_of_low_bits(0x01), Some(3));
        assert_eq!(code_of_low_bits(0x06), Some(2), "bit 1 before bit 2");
        assert_eq!(code_of_low_bits(0x14), Some(0));
        assert_eq!(code_of_low_bits(0x28), Some(1));
        assert_eq!(code_of_low_bits(0xF0), None, "the high nibble is ignored");
    }

    #[test]
    fn store_paths_add_the_prefix_and_the_shifts() {
        assert_eq!(store_path(true, Some(3)), (262, 11));
        assert_eq!(store_path(true, Some(2)), (298, 13));
        assert_eq!(store_path(true, Some(0)), (334, 15));
        assert_eq!(store_path(true, Some(1)), (348, 16));
        assert_eq!(store_path(true, None), (306, 14));
        assert_eq!(store_path(false, Some(3)), (246, 10));
        assert_eq!(store_path(false, None), (290, 13));
        assert_eq!(max_cost("store_code_from_low_bits"), 348);
    }
}
// --- split_index_table_lookup ---

/// The eight-byte table of `split_index_table_lookup` (`index_table_9b9b`, bank `$03`).
const SPLIT_TABLE: u32 = crate::data::INDEX_TABLE_9B9B;
/// CPU-busy master cycles of a call (one path, no loop), from the hit to the return.
pub(crate) const SPLIT_INDEX_COST: u64 = 344;

/// `split_index_table_lookup`: the input byte in A low goes to `$10`; its bits 3 to 7, the index, go
/// to `$12`, with `$13` cleared; its bits 0 to 2 pick the table byte, stored at `$11`. A is that table
/// byte with B cleared, X is the index. N is clear; Z is set for a zero index; C is bit 2 of the input;
/// V is kept.
pub(crate) fn split_index_table_lookup(ctx: &mut ssd_oracle::Ctx<'_>) {
    let input = ctx.al();
    let index = input >> 3;
    let value = ctx.read8(SPLIT_TABLE + u32::from(input & 0x07));
    ctx.write_dp8(0x10, input);
    ctx.write_dp8(0x12, index);
    ctx.write_dp8(0x13, 0);
    ctx.write_dp8(0x11, value);
    ctx.set_a(u16::from(value));
    ctx.set_x(u16::from(index));
    ctx.set_n(false);
    ctx.set_z(index == 0);
    ctx.set_c(input & 0x04 != 0);
    ctx.set_cost(SPLIT_INDEX_COST);
}

#[cfg(test)]
mod tests_split_index_table_lookup {
    use super::*;
    use crate::routines::tests::max_cost;

    #[test]
    fn the_split_index_cost_is_the_constant_path() {
        assert_eq!(crate::data::INDEX_TABLE_9B9B_SIZE, 8);
        assert_eq!(SPLIT_TABLE, 0x03_9B9B);
        assert_eq!(max_cost("split_index_table_lookup"), SPLIT_INDEX_COST);
    }
}
