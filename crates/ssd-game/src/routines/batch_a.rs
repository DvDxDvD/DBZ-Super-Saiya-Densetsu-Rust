//! Batch A of the M2 ports: shadow-OAM and palette writers, and a tile-map fill. They work on
//! WRAM, the direct page and, for the palette copy, ROM data; the semantics are in
//! `docs/re/routines/`.

use crate::data;
use ssd_oracle::Ctx;

// --- hide_sprites_from_index ---

/// The shadow low OAM table (`shadow_oam_low`): the Y byte of entry `i` is at `$0401 + i`
/// (absolute, so DBR-relative).
const OAM_Y_BYTE: u16 = (data::SHADOW_OAM_LOW & 0xFFFF) as u16 + 1;
/// Direct-page word holding the byte index of the next free entry (`oam_next_index`).
const NEXT_INDEX: u16 = (data::OAM_NEXT_INDEX & 0xFFFF) as u16;
/// Y byte that hides a sprite: the first line below the screen.
const HIDDEN_Y: u8 = 0xF0;
/// The index where the loop stops: the end of the 512-byte low table.
const TABLE_END: u16 = 0x0200;
/// CPU-busy master cycles of a call that finds the index at the table end and writes nothing,
/// from the hit to the return.
const COST_EMPTY: u64 = 122;
/// Master cycles of a call with `n >= 1` passes, from the hit to the return: the fixed part (the
/// load, the first compare, the exit) plus [`COST_PER_PASS`] for each pass, the last pass's exit
/// branch costing 6 less.
const COST_FIXED: u64 = 110;
const COST_PER_PASS: u64 = 156;

/// Passes of the loop for a call that starts at byte index `start`: one write, then the index
/// moves on by 4, and the loop goes on while it is below the table end (the first pass always
/// runs unless the index is already at the end).
pub(crate) fn passes(start: u16) -> u32 {
    let mut index = start;
    let mut count = 0;
    if index != TABLE_END {
        loop {
            count += 1;
            index = index.wrapping_add(4);
            if index >= TABLE_END {
                break;
            }
        }
    }
    count
}

/// CPU-busy master cycles of a call with `passes` passes.
pub(crate) fn hide_cost(passes: u32) -> u64 {
    if passes == 0 { COST_EMPTY } else { COST_FIXED + COST_PER_PASS * u64::from(passes) }
}

/// Instructions of a call with `passes` passes: the load, the compare, the branch and the return
/// (4, the same when the branch is taken out of an empty call), and 8 for each pass (load,
/// store, four increments, compare, branch). Declared per call: the calls the timing window
/// leaves to the original are shorter on the replays (about 410 instructions against 860 for the
/// ones that run in Rust), so the mean of the `Verify` calls would be off.
pub(crate) fn hide_instructions(passes: u32) -> u64 {
    4 + 8 * u64::from(passes)
}

/// `hide_sprites_from_index`: writes [`HIDDEN_Y`] to the Y byte of every shadow entry from the
/// index in `oam_next_index` to the table end. Leaves X = the index reached, A low = [`HIDDEN_Y`]
/// (unless no pass ran), and N, Z, C of the final compare with the table end; V, Y and memory
/// outside the table are untouched.
pub(crate) fn hide_sprites_from_index(ctx: &mut Ctx<'_>) {
    let start = ctx.read_dp16(NEXT_INDEX);
    let count = passes(start);
    let bank = u32::from(ctx.dbr()) << 16;
    let mut index = start;
    for _ in 0..count {
        ctx.write8(bank + u32::from(OAM_Y_BYTE) + u32::from(index), HIDDEN_Y);
        index = index.wrapping_add(4);
    }
    if count > 0 {
        ctx.set_al(HIDDEN_Y);
    }
    // The last compare is the one with the table end, on the index left.
    ctx.set_x(index);
    ctx.set_n(index.wrapping_sub(TABLE_END) & 0x8000 != 0);
    ctx.set_z(index == TABLE_END);
    ctx.set_c(index >= TABLE_END);
    ctx.set_cost(hide_cost(count));
    ctx.set_instructions(hide_instructions(count));
}

// --- load_palette_from_pointer ---

/// Bank the routine stores in direct page `$02` before it reads the pointer.
const POINTER_BANK: u8 = 0x09;
/// Direct-page word holding the low 16 bits of the source pointer.
const POINTER_LOW: u16 = 0x0000;
/// Direct-page byte that picks the palette slot (even values; the buffer offset is slot x 16).
const SLOT_BYTE: u16 = 0x0003;
/// Direct-page byte that receives the pointer's bank.
const BANK_BYTE: u16 = 0x0002;
/// Absolute base of the palette buffer, as the DBR-relative address `$0300`. Its extent is not
/// named in `re/symbols.toml`: the slots reach past `in_battle` (`$0364`), which is a named
/// variable of their own, so the buffer is left implicit.
const PALETTE_BASE: u16 = 0x0300;
/// Words copied per call: 16 colours.
const WORDS: u16 = 16;
/// CPU-busy master cycles of a call, from the hit to the return: one path (measured in `Verify`).
pub(crate) const PALETTE_COST: u64 = 3556;

/// Offset in the palette buffer of slot `slot_byte`: the low bit is dropped and the slot is
/// scaled by 16 (bytes per palette).
pub(crate) fn palette_offset(slot_byte: u8) -> u16 {
    u16::from(slot_byte & 0xFE) << 4
}

/// `load_palette_from_pointer`: stores bank [`POINTER_BANK`] in direct page `$02`, then copies 16
/// words from the 24-bit pointer at direct page `$00` into the palette buffer at the slot's
/// offset, word by word. Leaves A = the last word copied (16-bit), X = the slot offset plus 32,
/// Y = 32, and N = 0, Z = 1, C = 1 from the final compare; V is untouched.
pub(crate) fn load_palette_from_pointer(ctx: &mut Ctx<'_>) {
    ctx.write_dp8(BANK_BYTE, POINTER_BANK);
    let offset = palette_offset(ctx.read_dp8(SLOT_BYTE));
    let source = (u32::from(POINTER_BANK) << 16) | u32::from(ctx.read_dp16(POINTER_LOW));
    let mut last = 0;
    for word in 0..WORDS {
        let y = word * 2;
        last = ctx.read16((source + u32::from(y)) & 0xFF_FFFF);
        ctx.write_db16(PALETTE_BASE.wrapping_add(offset).wrapping_add(y), last);
    }
    ctx.set_a(last);
    ctx.set_x(offset + WORDS * 2);
    ctx.set_y(WORDS * 2);
    ctx.set_n(false);
    ctx.set_z(true);
    ctx.set_c(true);
    ctx.set_cost(PALETTE_COST);
}

// --- fill_tilemap_4000 ---

/// The 2 KB tile-map buffer at `$7E:4000` (`tilemap_buffer_4000`).
const TILEMAP_BUFFER: u32 = data::TILEMAP_BUFFER_4000;
/// The word every entry takes: tile `$FF`, palette 0.
const FILL_WORD: u16 = 0x00FF;
/// The index of the last word written (the loop counts down in steps of 2).
const LAST_INDEX: u16 = 0x07FE;
/// CPU-busy master cycles of a call, from the hit to the return: one path (measured in `Verify`).
pub(crate) const TILEMAP_COST: u64 = 100_482;

/// The byte indices the loop writes, in order: from [`LAST_INDEX`] down to 0 in steps of 2.
pub(crate) fn fill_indices() -> impl Iterator<Item = u16> {
    (0..=LAST_INDEX).rev().step_by(2)
}

/// `fill_tilemap_4000`: writes [`FILL_WORD`] to every word of the 2 KB buffer at `$7E:4000`.
/// Leaves A = [`FILL_WORD`] (16-bit), X = `$FFFE` (the index after the last word, with N set),
/// and V and C untouched; M must be 8 on entry, as the routine ends with that width.
pub(crate) fn fill_tilemap_4000(ctx: &mut Ctx<'_>) {
    for index in fill_indices() {
        ctx.write16(TILEMAP_BUFFER + u32::from(index), FILL_WORD);
    }
    ctx.set_a(FILL_WORD);
    ctx.set_x(0xFFFE);
    ctx.set_nz16(0xFFFE);
    ctx.set_cost(TILEMAP_COST);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routines::tests::max_cost;

    // --- hide_sprites_from_index ---

    #[test]
    fn hide_instructions_follow_the_listing() {
        // Measured in Verify: 164 for 20 passes, 1028 for 128; an empty call is 4.
        assert_eq!(hide_instructions(0), 4);
        assert_eq!(hide_instructions(20), 164);
        assert_eq!(hide_instructions(128), 1028);
    }

    #[test]
    fn hide_costs_match_the_measured_paths() {
        // Measured by `probe` in the replays: 26,153 calls; 3,230 is 20 passes, 5,102 is 32, the
        // maximum of the replays 20,078 is 128.
        assert_eq!(hide_cost(20), 3230);
        assert_eq!(hide_cost(32), 5102);
        assert_eq!(hide_cost(128), 20078);
        assert_eq!(passes(0x01B0), 20);
        assert_eq!(passes(0x0180), 32);
        assert_eq!(passes(0x0000), 128);
    }

    #[test]
    fn an_index_at_the_end_writes_nothing() {
        assert_eq!(passes(TABLE_END), 0);
        assert_eq!(hide_cost(0), COST_EMPTY);
        // Above the end: one pass, which writes and leaves the index above the end.
        assert_eq!(passes(0x0300), 1);
        assert_eq!(hide_cost(1), 266);
    }

    #[test]
    fn a_wrapping_index_runs_on_past_zero() {
        // $FFFC + 4 wraps to 0 and the loop goes on up to the table end: 1 + 128 passes.
        assert_eq!(passes(0xFFFC), 129);
        assert_eq!(passes(0xFFFF), 129);
    }

    #[test]
    fn the_symbols_cost_is_the_worst_case() {
        let worst = (0..=0xFFFF_u16).map(|start| hide_cost(passes(start))).max();
        assert_eq!(worst, Some(20234));
        assert_eq!(max_cost("hide_sprites_from_index"), 20234);
    }

    // --- load_palette_from_pointer ---

    #[test]
    fn the_palette_slot_is_the_even_part_times_16() {
        assert_eq!(palette_offset(0x00), 0x0000);
        assert_eq!(palette_offset(0x01), 0x0000, "the low bit is dropped");
        assert_eq!(palette_offset(0x02), 0x0020);
        assert_eq!(palette_offset(0x0E), 0x00E0);
        assert_eq!(palette_offset(0xFF), 0x0FE0);
    }

    #[test]
    fn the_palette_cost_is_constant() {
        assert_eq!(max_cost("load_palette_from_pointer"), PALETTE_COST);
    }

    // --- fill_tilemap_4000 ---

    #[test]
    fn the_tilemap_loop_writes_every_word_once_from_the_top() {
        let indices: Vec<u16> = fill_indices().collect();
        assert_eq!(indices.len(), 1024);
        assert_eq!(indices.first(), Some(&0x07FE));
        assert_eq!(indices.last(), Some(&0x0000));
        assert!(indices.windows(2).all(|pair| pair[0] == pair[1] + 2));
    }

    #[test]
    fn the_tilemap_cost_is_constant() {
        assert_eq!(max_cost("fill_tilemap_4000"), TILEMAP_COST);
    }
}
