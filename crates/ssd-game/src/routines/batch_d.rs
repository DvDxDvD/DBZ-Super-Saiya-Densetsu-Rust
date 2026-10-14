//! Batch D of the M2 porting batch: routines whose cost depends on the entry state (a size, a scan
//! or a count), each with a bound function that gives the cost of one call from that state
//! (`registry.rs`). Semantics are in `docs/re/routines/`.
//!
//! Each section is written from the routine's own behaviour. Addresses come from the generated
//! `data` constants (`re/symbols.toml`); direct-page scratch bytes stay implicit. A path that a
//! reference replay never takes runs the original code (`Ctx::call_original` from the entry, before
//! anything is written), and its bound declines it (`u64::MAX`), so the window never opens for it.

use crate::data;
use crate::routines::batch_c::{add16, sub16};
use ssd_oracle::{Ctx, ReturnKind};

// ---- draw_tile_box: a box of tile words, W by H cells, at the buffer position in $02 ----

/// Entry of the routine (`$01:B55A`), for the hand-back to the original code.
const BOX_ENTRY: u32 = 0x01_B55A;
/// The box width and height (direct page `$00` and `$01`) and the buffer position, a word at `$02`.
const BOX_WIDTH: u16 = 0x00;
const BOX_HEIGHT: u16 = 0x01;
const BOX_BASE: u16 = 0x02;
/// The buffer the box is written into, `$7E:3000` plus the position. It is not a named variable:
/// its extent is the boxes drawn, and it overlaps the named `fill_block` further up.
const BOX_BUFFER: u32 = 0x7E_3000;
/// The interior flag (`box_interior_flag`): a nonzero byte skips the interior fill of the middle
/// rows; the routine clears it.
const BOX_INTERIOR_FLAG: u16 = (data::BOX_INTERIOR_FLAG & 0xFFFF) as u16;
/// Bytes from one row of the box to the next: one row is 32 tiles.
const ROW_BYTES: u16 = 0x0040;
/// The tile words: the four corners, the top and bottom edges, the left and right edges, and the
/// fill of the interior.
const TILE_TOP_LEFT: u16 = 0x2832;
const TILE_TOP: u16 = 0x2833;
const TILE_TOP_RIGHT: u16 = 0x2834;
const TILE_LEFT: u16 = 0x2835;
const TILE_RIGHT: u16 = 0x2836;
const TILE_BOTTOM_LEFT: u16 = 0x283B;
const TILE_BOTTOM: u16 = 0x283C;
const TILE_BOTTOM_RIGHT: u16 = 0x283D;
const TILE_INTERIOR: u16 = 0x2801;
/// CPU-busy master cycles of a call on the paths the replays take, from the hit to the return: a
/// fixed part; per interior column of the top and bottom rows together (their two loops); per
/// middle row; and per interior cell of a middle row (the interior fill is written).
const COST_FIXED: u64 = 1200;
const COST_EDGE_COLUMN: u64 = 564;
const COST_MIDDLE_ROW: u64 = 588;
const COST_INTERIOR_CELL: u64 = 314;
/// Instructions (opcode fetches) of the same call, counted on the listing with the same shape:
/// the fixed part, then per column of the two edge rows, per middle row, and per interior cell.
const INSTR_FIXED: u64 = 46;
const INSTR_EDGE_COLUMN: u64 = 24;
const INSTR_MIDDLE_ROW: u64 = 23;
const INSTR_INTERIOR_CELL: u64 = 13;

/// Whether the sizes take a path the replays take: two cells or more each way, and with three rows
/// or more a width of three or more (a middle row of two cells runs its interior loop 256 times,
/// because its 8-bit counter wraps).
pub(crate) fn box_is_exact(width: u8, height: u8) -> bool {
    width >= 2 && height >= 2 && (width >= 3 || height == 2)
}

/// CPU-busy master cycles of a call with the interior fill on an exact size.
pub(crate) fn box_cost(width: u8, height: u8) -> u64 {
    let (columns, rows) = (u64::from(width) - 2, u64::from(height) - 2);
    COST_FIXED
        + COST_EDGE_COLUMN * columns
        + COST_MIDDLE_ROW * rows
        + COST_INTERIOR_CELL * rows * columns
}

/// Instructions of the same call.
pub(crate) fn box_instructions(width: u8, height: u8) -> u64 {
    let (columns, rows) = (u64::from(width) - 2, u64::from(height) - 2);
    INSTR_FIXED
        + INSTR_EDGE_COLUMN * columns
        + INSTR_MIDDLE_ROW * rows
        + INSTR_INTERIOR_CELL * rows * columns
}

/// Writes one row of a box that starts at `start`: `left` at its first cell, `inner` at the cells
/// between, `right` at its last. Returns the position of that last cell.
fn put_row(
    put: &mut impl FnMut(u16, u16),
    start: u16,
    width: u8,
    [left, inner, right]: [u16; 3],
) -> u16 {
    let last_column = u16::from(width) - 1;
    put(start, left);
    for column in 1..last_column {
        put(start.wrapping_add(2 * column), inner);
    }
    let last = start.wrapping_add(2 * last_column);
    put(last, right);
    last
}

/// Where a box leaves the registers: the start of its bottom row (`$02`), the position of its last
/// cell (X), and the row the last advance started from (its overflow sets V).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BoxEnd {
    pub row: u16,
    pub x: u16,
    pub advanced_from: u16,
}

/// The box of `width` by `height` cells from `base`: `put(x, tile)` is called for each word in the
/// routine's order, `x` being the offset into the buffer. The interior of the middle rows is filled
/// (the caller has checked the interior flag).
pub(crate) fn draw_box(base: u16, width: u8, height: u8, mut put: impl FnMut(u16, u16)) -> BoxEnd {
    let mut row = base;
    let mut advanced_from = base;
    put_row(&mut put, row, width, [TILE_TOP_LEFT, TILE_TOP, TILE_TOP_RIGHT]);
    row = row.wrapping_add(ROW_BYTES);
    // Each middle row is written from the row where the previous one advanced to.
    for _ in 2..height {
        put_row(&mut put, row, width, [TILE_LEFT, TILE_INTERIOR, TILE_RIGHT]);
        advanced_from = row;
        row = row.wrapping_add(ROW_BYTES);
    }
    let x = put_row(&mut put, row, width, [TILE_BOTTOM_LEFT, TILE_BOTTOM, TILE_BOTTOM_RIGHT]);
    BoxEnd { row, x, advanced_from }
}

/// `draw_tile_box`: draws the box of [`draw_box`] at the position in `$02`, with the width in `$00`
/// and the height in `$01`. A call with the interior flag set, or on a size [`box_is_exact`]
/// rejects, runs the original code (its own cost is 0).
///
/// On the replayed paths it leaves `$02` at the bottom row, `$10` = width - 1, `$11` = height - 1,
/// the interior flag cleared, A = `$283D` (16-bit, B = `$28`), X at the last cell of the bottom row,
/// N and Z clear, C set, and V from the last row advance (the signed overflow of its position plus
/// `$40`).
pub(crate) fn draw_tile_box(ctx: &mut Ctx<'_>) {
    let width = ctx.read_dp8(BOX_WIDTH);
    let height = ctx.read_dp8(BOX_HEIGHT);
    let interior = ctx.read_db8(BOX_INTERIOR_FLAG);
    if interior != 0 || !box_is_exact(width, height) {
        let Ok(()) = ctx.call_original(BOX_ENTRY, ReturnKind::Rtl) else { return };
        ctx.set_cost(0);
        ctx.set_instructions(0);
        return;
    }
    let base = ctx.read_dp16(BOX_BASE);
    let end = draw_box(base, width, height, |x, tile| ctx.write16(BOX_BUFFER + u32::from(x), tile));
    ctx.write_dp16(BOX_BASE, end.row);
    ctx.write_dp8(0x10, width - 1);
    ctx.write_dp8(0x11, height - 1);
    ctx.write_db8(BOX_INTERIOR_FLAG, 0);
    ctx.set_a(TILE_BOTTOM_RIGHT);
    ctx.set_x(end.x);
    ctx.set_n(false);
    ctx.set_z(false);
    ctx.set_c(true);
    ctx.set_v(add16(end.advanced_from, ROW_BYTES).2);
    ctx.set_cost(box_cost(width, height));
    ctx.set_instructions(box_instructions(width, height));
}

/// The cost bound of [`draw_tile_box`] for one call, from the entry state: its exact cost on the
/// paths the replays take, `u64::MAX` (declined, so the original runs) on the others.
pub(crate) fn draw_tile_box_bound(ctx: &Ctx<'_>) -> u64 {
    let width = ctx.peek_dp8(BOX_WIDTH);
    let height = ctx.peek_dp8(BOX_HEIGHT);
    if ctx.peek_db8(BOX_INTERIOR_FLAG) != 0 || !box_is_exact(width, height) {
        u64::MAX
    } else {
        box_cost(width, height)
    }
}

#[cfg(test)]
mod tests_box {
    use super::*;
    use crate::routines::tests::max_cost;

    /// The words [`draw_box`] writes, as (offset, tile) in order, and where it ends.
    fn writes(base: u16, width: u8, height: u8) -> (Vec<(u16, u16)>, BoxEnd) {
        let mut out = Vec::new();
        let end = draw_box(base, width, height, |x, tile| out.push((x, tile)));
        (out, end)
    }

    #[test]
    fn a_three_by_three_box_writes_its_nine_tiles_in_order() {
        let (words, end) = writes(0x0100, 3, 3);
        assert_eq!(
            words,
            [
                (0x0100, 0x2832),
                (0x0102, 0x2833),
                (0x0104, 0x2834),
                (0x0140, 0x2835),
                (0x0142, 0x2801),
                (0x0144, 0x2836),
                (0x0180, 0x283B),
                (0x0182, 0x283C),
                (0x0184, 0x283D),
            ]
        );
        assert_eq!(end, BoxEnd { row: 0x0180, x: 0x0184, advanced_from: 0x0140 });
    }

    #[test]
    fn a_two_row_box_advances_once_from_its_base() {
        let (words, end) = writes(0x0000, 2, 2);
        assert_eq!(words, [(0x0000, 0x2832), (0x0002, 0x2834), (0x0040, 0x283B), (0x0042, 0x283D)]);
        assert_eq!(end, BoxEnd { row: 0x0040, x: 0x0042, advanced_from: 0x0000 });
    }

    #[test]
    fn a_position_wraps_as_the_16_bit_index_does() {
        let (words, end) = writes(0xFFF8, 3, 2);
        let offsets: Vec<u16> = words.iter().map(|&(x, _)| x).collect();
        assert_eq!(offsets, [0xFFF8, 0xFFFA, 0xFFFC, 0x0038, 0x003A, 0x003C]);
        assert_eq!((end.row, end.x), (0x0038, 0x003C));
    }

    #[test]
    fn the_costs_of_the_measured_boxes() {
        // Measured by `probe` over the replays: 10 by 4 is the least (11,912), 7 by 6 (12,652),
        // 12 by 4 (14,296), 28 by 8 (68,376) and 28 by 25 the most (217,160).
        assert_eq!(box_cost(10, 4), 11_912);
        assert_eq!(box_cost(7, 6), 12_652);
        assert_eq!(box_cost(12, 4), 14_296);
        assert_eq!(box_cost(28, 8), 68_376);
        assert_eq!(box_cost(28, 25), 217_160);
    }

    #[test]
    fn the_smallest_box_costs_and_counts_its_listing() {
        // 1200 + 564 + 588 + 314 cycles; the listing counts 46 + 24 + 23 + 13 instructions.
        assert_eq!(box_cost(3, 3), 2666);
        assert_eq!(box_instructions(3, 3), 106);
    }

    #[test]
    fn the_declared_maximum_is_the_largest_exact_box() {
        assert_eq!(max_cost("draw_tile_box"), box_cost(255, 255));
    }

    #[test]
    fn sizes_the_replays_never_take_are_declined() {
        assert!(box_is_exact(2, 2) && box_is_exact(3, 2) && box_is_exact(3, 3));
        assert!(!box_is_exact(2, 3), "a two-cell middle row wraps its interior loop");
        assert!(!box_is_exact(1, 5) && !box_is_exact(5, 1) && !box_is_exact(0, 4));
    }
}

// ---- find_positive_slot: the first slot byte, eight bytes apart from `$0800`, with bit 7 clear ----

/// Entry of the routine (`$00:8711`), for the hand-back to the original code.
const FIND_ENTRY: u32 = 0x00_8711;
/// The slot bytes the scan reads (`slot_block_0800`, in the data bank), and the distance between two
/// of them.
const SLOT_BLOCK: u16 = (data::SLOT_BLOCK_0800 & 0xFFFF) as u16;
const SLOT_STRIDE: u16 = 8;
/// The most negative slots the scan passes on the paths the replays take (they pass at most two);
/// a scan that passes more runs the original code.
const FIND_MAX_PASSES: u16 = 2;
/// CPU-busy master cycles of a call that passes `n` negative slots, from the hit to the return: the
/// index load, then for each pass a load, a branch not taken, eight increments and a branch back;
/// then the load of the positive slot, the branch taken and the return.
const COST_FIND_BASE: u64 = 128;
const COST_FIND_PASS: u64 = 188;
/// Instructions of the same call: four, and eleven for each pass.
const INSTR_FIND_BASE: u64 = 4;
const INSTR_FIND_PASS: u64 = 11;

/// The number of negative slots the scan passes before the first slot with bit 7 clear, for the
/// passes the replays take (`None` past [`FIND_MAX_PASSES`]). `slot(offset)` reads the data bank at
/// `offset`.
pub(crate) fn find_passes(mut slot: impl FnMut(u16) -> u8) -> Option<u16> {
    (0..=FIND_MAX_PASSES).find(|&passes| slot(SLOT_BLOCK + passes * SLOT_STRIDE) & 0x80 == 0)
}

/// CPU-busy master cycles of a call that passes `passes` negative slots.
pub(crate) fn find_cost(passes: u16) -> u64 {
    COST_FIND_BASE + COST_FIND_PASS * u64::from(passes)
}

/// Instructions of the same call.
pub(crate) fn find_instructions(passes: u16) -> u64 {
    INSTR_FIND_BASE + INSTR_FIND_PASS * u64::from(passes)
}

/// `find_positive_slot`: A's low byte becomes the first slot byte with bit 7 clear, and X its index
/// (eight times the number of negative slots passed). N is clear and Z is set when that byte is
/// zero; C, V and B are kept. The scan only reads, so a scan past the replayed passes runs the
/// original code from the entry.
pub(crate) fn find_positive_slot(ctx: &mut Ctx<'_>) {
    let Some(passes) = find_passes(|offset| ctx.peek_db8(offset)) else {
        let Ok(()) = ctx.call_original(FIND_ENTRY, ReturnKind::Rtl) else { return };
        ctx.set_cost(0);
        ctx.set_instructions(0);
        return;
    };
    let index = passes * SLOT_STRIDE;
    let slot = ctx.read_db8(SLOT_BLOCK + index);
    ctx.set_al(slot);
    ctx.set_x(index);
    ctx.set_nz8(slot);
    ctx.set_cost(find_cost(passes));
    ctx.set_instructions(find_instructions(passes));
}

/// The cost bound of [`find_positive_slot`] for one call, from the entry state: its exact cost on
/// the replayed passes, `u64::MAX` (the original runs) past them.
pub(crate) fn find_positive_slot_bound(ctx: &Ctx<'_>) -> u64 {
    find_passes(|offset| ctx.peek_db8(offset)).map_or(u64::MAX, find_cost)
}

#[cfg(test)]
mod tests_find {
    use super::*;
    use crate::routines::tests::max_cost;

    /// Slot bytes at the first three slots (`$0800`, `$0808`, `$0810`); every other byte is zero.
    fn slots(bytes: [u8; 3]) -> impl FnMut(u16) -> u8 {
        move |offset| match offset.wrapping_sub(SLOT_BLOCK) {
            0 => bytes[0],
            8 => bytes[1],
            16 => bytes[2],
            _ => 0,
        }
    }

    #[test]
    fn the_costs_are_the_ones_the_replays_measured() {
        // Measured over the replays: 128 cycles (6,804 calls), 316 (4,271) and 504 (258).
        assert_eq!(find_cost(0), 128);
        assert_eq!(find_cost(1), 316);
        assert_eq!(find_cost(2), 504);
        assert_eq!(max_cost("find_positive_slot"), 504);
        assert_eq!((find_instructions(0), find_instructions(1), find_instructions(2)), (4, 15, 26));
    }

    #[test]
    fn the_scan_passes_negative_slots_eight_bytes_apart() {
        assert_eq!(find_passes(slots([0x00, 0xFF, 0xFF])), Some(0));
        assert_eq!(find_passes(slots([0x80, 0x12, 0xFF])), Some(1));
        assert_eq!(find_passes(slots([0x81, 0x90, 0x05])), Some(2));
        assert_eq!(find_passes(slots([0x80, 0x00, 0x00])), Some(1), "only slot bytes are read");
    }

    #[test]
    fn a_scan_past_the_replayed_passes_is_declined() {
        assert_eq!(find_passes(slots([0x80, 0x80, 0x80])), None);
        assert_eq!(find_passes(slots([0x80, 0x80, 0x00])), Some(2));
    }
}

// ---- scan_record_table: the ten-byte records from `$074C`, up to the `$FFFF` end word ----

/// Entry of the routine (`$06:EC76`), for the hand-back to the original code.
const SCAN_ENTRY: u32 = 0x06_EC76;
/// The byte whose bit 0, when set, makes the routine return at once (`$01BC`, data bank).
const SCAN_SKIP_BYTE: u16 = 0x01BC;
/// The word each record's position is compared with (`$01A2`, data bank).
const SCAN_BASE_WORD: u16 = 0x01A2;
/// The record table: a word position at each record, and the flags byte nine bytes on. The table
/// ends at a record whose position is `$FFFF`.
const SCAN_TABLE: u16 = 0x074C;
const SCAN_STRIDE: u16 = 0x000A;
const SCAN_FLAGS_OFFSET: u16 = 0x0009;
const SCAN_END: u16 = 0xFFFF;
/// A record is passed when its position minus the base word, plus eight, is at least `$10`
/// (unsigned): the routine acts on the others.
const SCAN_WINDOW_OFFSET: u16 = 0x0008;
const SCAN_WINDOW: u16 = 0x0010;
/// CPU-busy master cycles of the two paths the replays take, from the hit to the return. Immediate:
/// the byte load, the shift, the branch not taken and the return. `TwoRecords`: a prefix (the byte
/// load, the shift, the branch taken and the index load: 92), two passed records (514 each: the
/// position and flags loads, the compares, the step to the next record) and the end word (174: its
/// load, its compare and the return).
const COST_SCAN_IMMEDIATE: u64 = 106;
const COST_SCAN_PREFIX: u64 = 92;
const COST_SCAN_RECORD: u64 = 514;
const COST_SCAN_END: u64 = 174;
const COST_SCAN_TWO_RECORDS: u64 = COST_SCAN_PREFIX + 2 * COST_SCAN_RECORD + COST_SCAN_END;
/// Instructions of the same two paths: four for Immediate; for `TwoRecords` 4 (the prefix), 21 per
/// passed record, and 6 for the end word.
const INSTR_SCAN_IMMEDIATE: u64 = 4;
const INSTR_SCAN_TWO_RECORDS: u64 = 4 + 2 * 21 + 6;

/// The paths of [`scan_record_table`] the replays take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScanPath {
    /// Bit 0 of `$01BC` is set: the byte shifted right is left in A, and the routine returns.
    Immediate,
    /// Bit 0 is clear, the first two records are passed (flags zero, position outside the window)
    /// and the end word is the third record's position.
    TwoRecords,
}

/// The path of a call, from the bytes and words it reads through the data bank (`byte(offset)` and
/// `word(offset)`). `None` for any other path, which runs the original code.
pub(crate) fn scan_path(byte: impl Fn(u16) -> u8, word: impl Fn(u16) -> u16) -> Option<ScanPath> {
    if byte(SCAN_SKIP_BYTE) & 0x01 != 0 {
        return Some(ScanPath::Immediate);
    }
    let base = word(SCAN_BASE_WORD);
    let mut passed: u16 = 0;
    loop {
        let record = SCAN_TABLE + passed * SCAN_STRIDE;
        let position = word(record);
        if position == SCAN_END {
            return (passed == 2).then_some(ScanPath::TwoRecords);
        }
        let outside = position.wrapping_sub(base).wrapping_add(SCAN_WINDOW_OFFSET) >= SCAN_WINDOW;
        if passed == 2 || byte(record + SCAN_FLAGS_OFFSET) != 0 || !outside {
            return None;
        }
        passed += 1;
    }
}

/// `scan_record_table`: the two paths of [`ScanPath`] on the machine; neither writes memory. For
/// Immediate, A's low byte becomes the byte at `$01BC` shifted right, N is clear, Z follows that
/// result, C is set, and B and V are kept. For `TwoRecords`, A = `$FFFF`, Y = `$0014`, N is clear, Z
/// and C are set, and V is clear (the last step of Y does not overflow). Any other path runs the
/// original code from the entry, with its own cost 0.
pub(crate) fn scan_record_table(ctx: &mut Ctx<'_>) {
    let Some(path) = scan_path(|offset| ctx.peek_db8(offset), |offset| ctx.peek_db16(offset))
    else {
        let Ok(()) = ctx.call_original(SCAN_ENTRY, ReturnKind::Rtl) else { return };
        ctx.set_cost(0);
        ctx.set_instructions(0);
        return;
    };
    match path {
        ScanPath::Immediate => {
            let shifted = ctx.read_db8(SCAN_SKIP_BYTE) >> 1;
            ctx.set_al(shifted);
            ctx.set_nz8(shifted);
            ctx.set_c(true);
            ctx.set_cost(COST_SCAN_IMMEDIATE);
            ctx.set_instructions(INSTR_SCAN_IMMEDIATE);
        }
        ScanPath::TwoRecords => {
            ctx.set_a(SCAN_END);
            ctx.set_y(2 * SCAN_STRIDE);
            ctx.set_n(false);
            ctx.set_z(true);
            ctx.set_c(true);
            ctx.set_v(false);
            ctx.set_cost(COST_SCAN_TWO_RECORDS);
            ctx.set_instructions(INSTR_SCAN_TWO_RECORDS);
        }
    }
}

/// The cost bound of [`scan_record_table`] for one call, from the entry state: the cost of its path,
/// `u64::MAX` (declined: the original runs) on the others.
pub(crate) fn scan_record_table_bound(ctx: &Ctx<'_>) -> u64 {
    match scan_path(|offset| ctx.peek_db8(offset), |offset| ctx.peek_db16(offset)) {
        Some(ScanPath::Immediate) => COST_SCAN_IMMEDIATE,
        Some(ScanPath::TwoRecords) => COST_SCAN_TWO_RECORDS,
        None => u64::MAX,
    }
}

#[cfg(test)]
mod tests_scan {
    use super::*;
    use crate::routines::tests::max_cost;

    /// Memory cells `(offset, byte)`, the last one at an offset winning; every other offset reads
    /// zero. Returns the byte reader and the word reader (little-endian) over them.
    fn memory(cells: &[(u16, u8)]) -> (impl Fn(u16) -> u8 + '_, impl Fn(u16) -> u16 + '_) {
        let byte = move |offset: u16| {
            cells.iter().rev().find(|&&(at, _)| at == offset).map_or(0, |&(_, value)| value)
        };
        let word =
            move |offset: u16| u16::from_le_bytes([byte(offset), byte(offset.wrapping_add(1))]);
        (byte, word)
    }

    fn put_word(cells: &mut Vec<(u16, u8)>, at: u16, value: u16) {
        cells.push((at, value as u8));
        cells.push((at + 1, (value >> 8) as u8));
    }

    /// The base word is `$0870`; records 0 and 1 sit at `$0200` (records before `first` are at
    /// `first`) with their flags zero, and the end word is at record `end`.
    fn table(first: u16, end: u16) -> Vec<(u16, u8)> {
        let mut cells = Vec::new();
        put_word(&mut cells, SCAN_BASE_WORD, 0x0870);
        for record in 0..end {
            let position = if record == 0 { first } else { 0x0200 };
            put_word(&mut cells, SCAN_TABLE + record * SCAN_STRIDE, position);
        }
        put_word(&mut cells, SCAN_TABLE + end * SCAN_STRIDE, SCAN_END);
        cells
    }

    #[test]
    fn the_costs_are_the_ones_the_replays_measured() {
        // Measured over the replays: 106 cycles (1,789 calls) and 1,294 (1,875 calls).
        assert_eq!(COST_SCAN_IMMEDIATE, 106);
        assert_eq!(COST_SCAN_TWO_RECORDS, 1294);
        assert_eq!(max_cost("scan_record_table"), 1294);
        assert_eq!((INSTR_SCAN_IMMEDIATE, INSTR_SCAN_TWO_RECORDS), (4, 52));
    }

    #[test]
    fn bit_zero_of_the_skip_byte_takes_the_immediate_path() {
        let cells = [(SCAN_SKIP_BYTE, 0xA1)];
        let (byte, word) = memory(&cells);
        assert_eq!(scan_path(&byte, &word), Some(ScanPath::Immediate));
    }

    #[test]
    fn two_far_records_then_the_end_word_take_the_two_records_path() {
        let cells = table(0x0100, 2);
        let (byte, word) = memory(&cells);
        assert_eq!(scan_path(&byte, &word), Some(ScanPath::TwoRecords));
    }

    #[test]
    fn the_window_edges_decide_whether_a_record_is_passed() {
        // Eight above the base is outside the window ($10 after the offset of eight); eight below is
        // inside it, and such a call is declined.
        let cells = table(0x0870 + 8, 2);
        let (byte, word) = memory(&cells);
        assert_eq!(scan_path(&byte, &word), Some(ScanPath::TwoRecords));
        let cells = table(0x0870 - 8, 2);
        let (byte, word) = memory(&cells);
        assert_eq!(scan_path(&byte, &word), None, "inside the window");
    }

    #[test]
    fn other_paths_are_declined() {
        let mut flagged = table(0x0100, 2);
        flagged.push((SCAN_TABLE + SCAN_FLAGS_OFFSET, 0x40));
        let (byte, word) = memory(&flagged);
        assert_eq!(scan_path(&byte, &word), None, "a record with flags is not passed");
        let cells = table(0x0100, 0);
        let (byte, word) = memory(&cells);
        assert_eq!(scan_path(&byte, &word), None, "the end word at the first record");
        let cells = table(0x0100, 3);
        let (byte, word) = memory(&cells);
        assert_eq!(scan_path(&byte, &word), None, "three records before the end word");
    }
}

// ---- timer_mask_block: a slot's timer byte is counted down, and a block of words is masked ----

/// Entry of the routine (`$05:C1EE`), for the hand-back to the original code.
const TIMER_ENTRY: u32 = 0x05_C1EE;
/// The ROM word table read at the caller's X (`index_word_table`): the slot index of the call.
const TIMER_INDEX_TABLE: u32 = data::INDEX_WORD_TABLE;
/// The ROM table of mask words (`mask_word_table`), read at the shifted timer value.
const TIMER_MASK_TABLE: u32 = data::MASK_WORD_TABLE;
/// The WRAM word table read at the caller's X (`slot_index_table`): the record index of the block.
const TIMER_SLOT_TABLE: u16 = (data::SLOT_INDEX_TABLE & 0xFFFF) as u16;
/// The side record byte the routine tests: `side_record_b` plus `$34`. The values `$13` and `$14`
/// take another path, which the replays never take.
const TIMER_SIDE_BYTE: u16 = (data::SIDE_RECORD_B & 0xFFFF) as u16 + 0x34;
/// The per-slot timer bytes (`$0029,Y`) and flag bytes (`$0021,Y`), with the slot index Y.
const TIMER_BYTES: u16 = 0x0029;
const TIMER_FLAGS: u16 = 0x0021;
/// The block's words for the slot: the pointer's low word (`$0803,Y`), its bank word (`$0805,Y`)
/// and the word that sets the pass count (`$0806,Y`). The mask and the pointer live in the direct page.
const TIMER_POINTER: u16 = 0x0803;
const TIMER_BANK: u16 = 0x0805;
const TIMER_TOP: u16 = 0x0806;
const TIMER_POINTER_DP: u16 = 0x00;
const TIMER_BANK_DP: u16 = 0x02;
const TIMER_MASK_DP: u16 = 0x10;
/// CPU-busy master cycles and instructions of the short path (the timer's low seven bits are zero),
/// from the hit to the return.
const COST_TIMER_SHORT: u64 = 328;
const INSTR_TIMER_SHORT: u64 = 13;
/// The long path, in master cycles: `COST_TIMER_BASE` (which includes the 6 cycles that the last pass
/// saves on its exit branch) for a timer that stays positive and a flag that is updated;
/// `COST_TIMER_NEGATIVE` more when the decremented timer is negative; `COST_TIMER_NO_UPDATE` less when
/// the flag is not updated; and `COST_TIMER_PASS` for each pass over the block.
const COST_TIMER_BASE: u64 = 1290;
const COST_TIMER_NEGATIVE: u64 = 26;
const COST_TIMER_NO_UPDATE: u64 = 86;
const COST_TIMER_PASS: u64 = 194;
/// The instructions of the long path: the base, `INSTR_TIMER_NEGATIVE` more and `INSTR_TIMER_NO_UPDATE`
/// less in the same cases, and `INSTR_TIMER_PASS` for each pass.
const INSTR_TIMER_BASE: u64 = 48;
const INSTR_TIMER_NEGATIVE: u64 = 2;
const INSTR_TIMER_NO_UPDATE: u64 = 3;
const INSTR_TIMER_PASS: u64 = 6;

/// The path of a call with a timer whose low seven bits are set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TimerPath {
    /// The timer's low seven bits are zero: nothing is written.
    Short,
    /// The timer is counted down and the block masked in `passes` passes.
    Long { timer: u8, passes: u64 },
}

/// Whether the decremented timer updates the flag byte: its low seven bits are set.
pub(crate) fn timer_updates_flag(decremented: u8) -> bool {
    decremented & 0x7F != 0
}

/// The passes over the block: the first index is the top word less two, and the passes go on down
/// by two while the next index is below `$8000`.
pub(crate) fn timer_passes(top: u16) -> u64 {
    let mut index = top.wrapping_sub(2);
    let mut passes = 1;
    while index.wrapping_sub(2) & 0x8000 == 0 {
        index = index.wrapping_sub(2);
        passes += 1;
    }
    passes
}

/// The path of a call from the bytes it reads: the side byte, the slot's timer and flag bytes, and
/// the top word of its block. `None` for the paths the replays never take: a side byte of `$13` or
/// `$14`, and a flag with bit 7 set (the early return of a long path).
pub(crate) fn timer_path(side: u8, timer: u8, flag: u8, top: u16) -> Option<TimerPath> {
    if side == 0x13 || side == 0x14 || (timer & 0x7F != 0 && flag & 0x80 != 0) {
        return None;
    }
    if timer & 0x7F == 0 {
        Some(TimerPath::Short)
    } else {
        Some(TimerPath::Long { timer, passes: timer_passes(top) })
    }
}

/// CPU-busy master cycles of a path, from the hit to the return.
pub(crate) fn timer_cost(path: TimerPath) -> u64 {
    match path {
        TimerPath::Short => COST_TIMER_SHORT,
        TimerPath::Long { timer, passes } => {
            let decremented = timer.wrapping_sub(1);
            let mut cost = COST_TIMER_BASE + COST_TIMER_PASS * passes;
            if decremented & 0x80 != 0 {
                cost += COST_TIMER_NEGATIVE;
            }
            if !timer_updates_flag(decremented) {
                cost -= COST_TIMER_NO_UPDATE;
            }
            cost
        }
    }
}

/// Instructions of a path.
pub(crate) fn timer_instructions(path: TimerPath) -> u64 {
    match path {
        TimerPath::Short => INSTR_TIMER_SHORT,
        TimerPath::Long { timer, passes } => {
            let decremented = timer.wrapping_sub(1);
            let mut count = INSTR_TIMER_BASE + INSTR_TIMER_PASS * passes;
            if decremented & 0x80 != 0 {
                count += INSTR_TIMER_NEGATIVE;
            }
            if !timer_updates_flag(decremented) {
                count -= INSTR_TIMER_NO_UPDATE;
            }
            count
        }
    }
}

/// `timer_mask_block`. The slot index Y is the ROM word at the caller's X, the timer and flag are
/// its bytes, and the block's record index is the WRAM word at the caller's X. The short path clears
/// the low byte of A (A = Y's high byte and zero), sets Y, sets C when the side byte is at least
/// `$14`, and sets Z with N clear. The long path counts
/// the timer down, updates the flag byte (bit 7 set, when the decremented timer's low seven bits are
/// set), reads the mask from the table at the shifted timer value, and masks each word of the block
/// from the top down. It leaves A = the last masked word, Y past the block, N set, Z and C clear, and
/// V and X unchanged. Any path `timer_path` declines runs the original code from the entry.
pub(crate) fn timer_mask_block(ctx: &mut Ctx<'_>) {
    let x = ctx.x();
    let index = ctx.peek16(TIMER_INDEX_TABLE + u32::from(x));
    let side = ctx.peek_db8(TIMER_SIDE_BYTE);
    let timer = ctx.peek_db8(TIMER_BYTES.wrapping_add(index));
    let flag = ctx.peek_db8(TIMER_FLAGS.wrapping_add(index));
    let record = ctx.peek_db16(TIMER_SLOT_TABLE.wrapping_add(x));
    let top = ctx.peek_db16(TIMER_TOP.wrapping_add(record));
    let Some(path) = timer_path(side, timer, flag, top) else {
        let Ok(()) = ctx.call_original(TIMER_ENTRY, ReturnKind::Rtl) else { return };
        ctx.set_cost(0);
        ctx.set_instructions(0);
        return;
    };
    match path {
        TimerPath::Short => {
            // The last compare of the side byte (with `$14`) leaves C, and the final AND clears N
            // and sets Z.
            ctx.set_a(index & 0xFF00);
            ctx.set_y(index);
            ctx.set_c(side >= 0x14);
            ctx.set_nz8(0);
        }
        TimerPath::Long { timer, .. } => {
            let decremented = timer.wrapping_sub(1);
            ctx.write_db8(TIMER_BYTES.wrapping_add(index), decremented);
            if timer_updates_flag(decremented) {
                let flag = ctx.read_db8(TIMER_FLAGS.wrapping_add(index));
                ctx.write_db8(TIMER_FLAGS.wrapping_add(index), flag | 0x80);
            }
            // The shifted value is the decremented timer, or its low seven bits with `$10` set
            // when it is negative; the shift drops bit 7.
            let before_shift =
                if decremented & 0x80 != 0 { (decremented & 0x7F) | 0x10 } else { decremented };
            let mask = ctx.read16(TIMER_MASK_TABLE + u32::from(before_shift << 1));
            ctx.write_dp16(TIMER_MASK_DP, mask);
            let record = ctx.read_db16(TIMER_SLOT_TABLE.wrapping_add(x));
            let pointer = ctx.read_db16(TIMER_POINTER.wrapping_add(record));
            ctx.write_dp16(TIMER_POINTER_DP, pointer);
            let bank = ctx.read_db16(TIMER_BANK.wrapping_add(record));
            ctx.write_dp16(TIMER_BANK_DP, bank);
            // The pass count comes from the top word as the loop reads it now.
            let top = ctx.read_db16(TIMER_TOP.wrapping_add(record));
            let passes = timer_passes(top);
            let mut offset = top.wrapping_sub(2);
            let mut last = 0;
            for _ in 0..passes {
                let base = u32::from(ctx.read_dp8(TIMER_POINTER_DP))
                    | u32::from(ctx.read_dp8(TIMER_POINTER_DP + 1)) << 8
                    | u32::from(ctx.read_dp8(TIMER_BANK_DP)) << 16;
                let address = (base + u32::from(offset)) & 0xFF_FFFF;
                last = ctx.read16(address) & ctx.read_dp16(TIMER_MASK_DP);
                ctx.write16(address, last);
                offset = offset.wrapping_sub(2);
            }
            ctx.set_a(last);
            ctx.set_y(offset);
            ctx.set_n(true);
            ctx.set_z(false);
            ctx.set_c(false);
            ctx.set_cost(timer_cost(TimerPath::Long { timer, passes }));
            ctx.set_instructions(timer_instructions(TimerPath::Long { timer, passes }));
            return;
        }
    }
    ctx.set_cost(timer_cost(path));
    ctx.set_instructions(timer_instructions(path));
}

/// The cost bound of [`timer_mask_block`] for one call, from the entry state: the cost of its path,
/// `u64::MAX` (declined) on the others.
pub(crate) fn timer_mask_block_bound(ctx: &Ctx<'_>) -> u64 {
    let x = ctx.x();
    let index = ctx.peek16(TIMER_INDEX_TABLE + u32::from(x));
    let side = ctx.peek_db8(TIMER_SIDE_BYTE);
    let timer = ctx.peek_db8(TIMER_BYTES.wrapping_add(index));
    let flag = ctx.peek_db8(TIMER_FLAGS.wrapping_add(index));
    let record = ctx.peek_db16(TIMER_SLOT_TABLE.wrapping_add(x));
    let top = ctx.peek_db16(TIMER_TOP.wrapping_add(record));
    timer_path(side, timer, flag, top).map_or(u64::MAX, timer_cost)
}

#[cfg(test)]
mod tests_timer {
    use super::*;
    use crate::routines::tests::max_cost;

    #[test]
    fn the_passes_count_the_indexes_from_the_top_word_down() {
        assert_eq!(timer_passes(0x01C0), 224, "indexes $01BE down to zero");
        assert_eq!(timer_passes(0x0320), 400);
        assert_eq!(timer_passes(0x0002), 1);
        assert_eq!(timer_passes(0x0000), 1, "the index wraps and the pass ends");
        assert_eq!(timer_passes(0x8004), 1, "the next index is already negative");
    }

    #[test]
    fn the_costs_are_the_ones_the_replays_measured() {
        // Measured over the replays: 328 (13,920 calls); 44,746 (224 passes, flag updated, timer
        // positive); 38,564 (negative, flag updated); 44,660 (no update, positive); 38,478 (no
        // update, negative); 78,890 (400 passes).
        assert_eq!(timer_cost(TimerPath::Short), 328);
        assert_eq!(timer_cost(TimerPath::Long { timer: 0x02, passes: 224 }), 44_746);
        assert_eq!(timer_cost(TimerPath::Long { timer: 0x82, passes: 192 }), 38_564);
        assert_eq!(timer_cost(TimerPath::Long { timer: 0x01, passes: 224 }), 44_660);
        assert_eq!(timer_cost(TimerPath::Long { timer: 0x81, passes: 192 }), 38_478);
        assert_eq!(timer_cost(TimerPath::Long { timer: 0x02, passes: 400 }), 78_890);
        assert_eq!(max_cost("timer_mask_block"), 78_890);
    }

    #[test]
    fn the_long_path_counts_its_instructions_and_the_short_path_its_own() {
        assert_eq!(timer_instructions(TimerPath::Short), 13);
        assert_eq!(timer_instructions(TimerPath::Long { timer: 0x02, passes: 1 }), 48 + 6);
        assert_eq!(timer_instructions(TimerPath::Long { timer: 0x81, passes: 1 }), 47 + 6);
    }

    #[test]
    fn the_paths_the_replays_never_take_are_declined() {
        assert_eq!(timer_path(0x03, 0x00, 0x00, 0x0320), Some(TimerPath::Short));
        assert_eq!(timer_path(0x13, 0x02, 0x00, 0x0320), None, "the side byte $13");
        assert_eq!(timer_path(0x14, 0x02, 0x00, 0x0320), None, "the side byte $14");
        assert_eq!(timer_path(0x03, 0x02, 0x80, 0x0320), None, "the early return on the flag");
        assert_eq!(timer_path(0x03, 0x80, 0x80, 0x0320), Some(TimerPath::Short));
    }
}

// ---- skip_negative_records: five-byte records from `$0C43`; the scan ends at a positive one ----

/// Entry of the routine (`$03:994C`), for the hand-back to the original code.
const SKIP_ENTRY: u32 = 0x03_994C;
/// The records: a byte at `$0C43 + X` (negative: the record is looked at) and a word at `$0C44 + X`,
/// five bytes from one record to the next.
const SKIP_RECORDS: u16 = 0x0C43;
/// The reference word (direct page `$33`) the record words are compared with, and the word the
/// routine stores its difference in (direct page `$0C`).
const SKIP_REFERENCE: u16 = 0x0033;
const SKIP_DIFFERENCE: u16 = 0x000C;
/// A difference at or above this value skips the record at its first test.
const SKIP_LIMIT: u16 = 0x00FF;
/// The replays skip the records at X = 0 and X = 5, and the record at X = 10 is positive, which ends
/// the scan. Any other sequence runs the original code.
const SKIP_PASSED: [u16; 2] = [0, 5];
const SKIP_END: u16 = 10;
/// CPU-busy master cycles, from the hit to the return: the index load (24); per skipped record 384
/// (the byte load and branch not taken, the word difference and its store, the compare and branch,
/// the step to the next record, and the compare of the index with the end and its branch back); and
/// the end record (104: the byte load, the branch taken and the return).
const COST_SKIP_LOAD: u64 = 24;
const COST_SKIP_RECORD: u64 = 384;
const COST_SKIP_END: u64 = 104;
/// Instructions of the same parts: 1, then 17 per skipped record, then 3.
const INSTR_SKIP_LOAD: u64 = 1;
const INSTR_SKIP_RECORD: u64 = 17;
const INSTR_SKIP_END: u64 = 3;

/// Whether a call takes the replayed sequence: each skipped record is negative and its word less
/// the reference is at least [`SKIP_LIMIT`] (unsigned), and the end record is positive. `flag` and
/// `word` read through the data bank.
pub(crate) fn skip_path(
    flag: impl Fn(u16) -> u8,
    word: impl Fn(u16) -> u16,
    reference: u16,
) -> bool {
    let skipped = SKIP_PASSED.iter().all(|&x| {
        flag(SKIP_RECORDS + x) & 0x80 != 0
            && word(SKIP_RECORDS + x + 1).wrapping_sub(reference) >= SKIP_LIMIT
    });
    skipped && flag(SKIP_RECORDS + SKIP_END) & 0x80 == 0
}

/// CPU-busy master cycles of the replayed sequence.
pub(crate) fn skip_cost() -> u64 {
    COST_SKIP_LOAD + COST_SKIP_RECORD * SKIP_PASSED.len() as u64 + COST_SKIP_END
}

/// Instructions of the replayed sequence.
pub(crate) fn skip_instructions() -> u64 {
    INSTR_SKIP_LOAD + INSTR_SKIP_RECORD * SKIP_PASSED.len() as u64 + INSTR_SKIP_END
}

/// `skip_negative_records`. On the replayed sequence, each skipped record stores its difference
/// (the word less the reference, 16-bit) at `$0C`, and the scan stops at the record at X = 10. The
/// exit has X = 10, A = the last difference's high byte with the end record's byte as its low byte,
/// N clear, Z by that byte, and C clear (the compare of X with `$28`). V is the overflow of the last
/// subtraction. Y and the other records are unchanged. Any other sequence runs the original code.
pub(crate) fn skip_negative_records(ctx: &mut Ctx<'_>) {
    let reference = ctx.peek_dp16(SKIP_REFERENCE);
    if !skip_path(|a| ctx.peek_db8(a), |a| ctx.peek_db16(a), reference) {
        let Ok(()) = ctx.call_original(SKIP_ENTRY, ReturnKind::Rtl) else { return };
        ctx.set_cost(0);
        ctx.set_instructions(0);
        return;
    }
    let mut last = 0;
    let mut overflow = false;
    for x in SKIP_PASSED {
        let (difference, _, v) = sub16(ctx.read_db16(SKIP_RECORDS + x + 1), reference);
        ctx.write_dp16(SKIP_DIFFERENCE, difference);
        last = difference;
        overflow = v;
    }
    let end = ctx.read_db8(SKIP_RECORDS + SKIP_END);
    ctx.set_a((last & 0xFF00) | u16::from(end));
    ctx.set_x(SKIP_END);
    ctx.set_nz8(end);
    ctx.set_c(false);
    ctx.set_v(overflow);
    ctx.set_cost(skip_cost());
    ctx.set_instructions(skip_instructions());
}

/// The cost bound of [`skip_negative_records`] for one call, from the entry state: the cost of the
/// replayed sequence, `u64::MAX` (declined) on any other.
pub(crate) fn skip_negative_records_bound(ctx: &Ctx<'_>) -> u64 {
    let reference = ctx.peek_dp16(SKIP_REFERENCE);
    if skip_path(|a| ctx.peek_db8(a), |a| ctx.peek_db16(a), reference) {
        skip_cost()
    } else {
        u64::MAX
    }
}

#[cfg(test)]
mod tests_skip {
    use super::*;
    use crate::routines::tests::max_cost;

    /// Records at X = 0 and X = 5 (flag negative, word `$0200` and `$0300`), and the end record at
    /// X = 10 (flag zero); the reference word is `$0010`.
    fn records(first_word: u16) -> (impl Fn(u16) -> u8, impl Fn(u16) -> u16) {
        let flag = |a: u16| match a {
            0x0C43 | 0x0C48 => 0x80,
            _ => 0x00,
        };
        let word = move |a: u16| match a {
            0x0C44 => first_word,
            0x0C49 => 0x0300,
            _ => 0,
        };
        (flag, word)
    }

    #[test]
    fn the_cost_is_the_one_the_replays_measured() {
        // Measured over the replays: 896 cycles on all 12,026 calls.
        assert_eq!(skip_cost(), 896);
        assert_eq!(skip_instructions(), 38);
        assert_eq!(max_cost("skip_negative_records"), 896);
    }

    #[test]
    fn the_replayed_sequence_takes_the_skip_path() {
        let (flag, word) = records(0x0200);
        assert!(skip_path(flag, word, 0x0010));
    }

    #[test]
    fn a_difference_below_the_limit_or_a_positive_record_is_declined() {
        // A difference of $FE is below $FF: the record is not skipped at its first test.
        let (flag, word) = records(0x0010 + 0x00FE);
        assert!(!skip_path(flag, word, 0x0010));
        // A positive record at X = 0 ends the scan at once: not the replayed sequence.
        let positive = |a: u16| if a == 0x0C43 { 0x00 } else { 0x80 };
        assert!(!skip_path(positive, |_| 0x0200, 0x0010));
    }
}
