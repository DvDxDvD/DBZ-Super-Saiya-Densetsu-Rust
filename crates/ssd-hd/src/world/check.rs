//! Checks of the world against what the game itself did: the map lookup against the tile map
//! words in VRAM, and the object port against the OAM.

use super::map::{FieldState, Rom};
use super::wide::window_origin;
use crate::bg::map_entry;
use ssd_oracle::{FrameLog, VramWrite};
use std::collections::{BTreeMap, HashSet};

/// The first tile where the lookup and the game's tile map disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileDifference {
    /// The tile's top left pixel in the world.
    pub world: (i32, i32),
    pub game: u16,
    pub lookup: u16,
}

/// The lookup against the game's BG2 tile map over the visible tiles of a frame.
///
/// The game builds its picture for 224 lines; this core's frame has 239 (the overscan the game
/// does not care about), and the tile map rows that only show below line 224 are not kept up to
/// date while the camera moves. Those tiles are counted apart. So are the tiles of a row the
/// game left empty (the map load of a single-block map fills 31 of the ring's 32 rows, and the
/// last one shows in the bottom lines of a room whose camera is not on a tile boundary).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TileCheck {
    /// Distinct tiles with a pixel in the first 224 lines.
    pub tiles: u64,
    /// Of those, the tiles whose word differs.
    pub differing: u64,
    /// Of those, the tiles outside the map (the lookup gives 0 there).
    pub outside: u64,
    /// Tiles of a row the game never built, where the lookup is not 0 (the tiles of the first
    /// 224 lines only, not counted in `differing`).
    pub unbuilt: u64,
    /// Tiles that only show below line 224, and the ones of them that differ.
    pub overscan_tiles: u64,
    pub overscan_differing: u64,
    pub first: Option<TileDifference>,
}

impl TileCheck {
    /// Whether BG2 shows the field's map: most of the tiles are the lookup's. The game also
    /// uses the field's mode for pictures of its own (a cutscene on BG2), where they are not.
    #[must_use]
    pub fn is_map(&self) -> bool {
        self.differing * 4 <= self.tiles
    }
}

/// The lines the game builds its picture for.
const DESIGNED_LINES: u16 = 224;

/// Compares, for every tile of the 256 x height picture that shows, the tile map word the game
/// holds in VRAM for BG2 with the one the lookup gives for the same world position. The world
/// position of the screen comes from BG2's scroll registers and the camera
/// ([`window_origin`]). Lines in forced blanking, or without BG2, show nothing and are skipped.
#[must_use]
pub fn check_tiles(rom: Rom<'_>, state: &FieldState, log: &FrameLog) -> TileCheck {
    let mut check = TileCheck::default();
    let mut seen: HashSet<(i32, i32)> = HashSet::new();
    // Per tile row of the first 224 lines: the tiles where the game and the lookup differ, and
    // how many of the row's tiles the game filled
    let mut rows: BTreeMap<i32, (Vec<TileDifference>, u32)> = BTreeMap::new();
    let mut work = Vec::new();
    if !log.vram_writes.is_empty() {
        work.extend_from_slice(&log.vram);
    }
    let mut writes = log.vram_writes.iter().peekable();
    for (index, entry) in log.entries.iter().enumerate() {
        while let Some(&VramWrite { address, value, .. }) =
            writes.next_if(|write| usize::from(write.entry) <= index)
        {
            work[usize::from(address)] = value;
        }
        let regs = &entry.regs;
        if entry.from_pixel != 0
            || entry.scanline == 0
            || regs.forced_blanking
            || regs.bg_mode != 1
            || !(regs.main_bg[1] || regs.sub_bg[1])
        {
            continue;
        }
        let vram = if log.vram_writes.is_empty() { &log.vram } else { &work };
        let (left, top) = window_origin(regs, state.camera);
        let world_y = top + i32::from(entry.scanline);
        for tile_x in (left >> 3)..=((left + 255) >> 3) {
            if !seen.insert((tile_x, world_y >> 3)) {
                continue;
            }
            let (x, y) = (tile_x * 8, (world_y >> 3) * 8);
            let game = map_entry(vram, regs, 1, (x & 0x1FF) as u16, (y & 0xFF) as u16);
            let lookup = state.map.tile_word(rom, x, y);
            if entry.scanline > DESIGNED_LINES {
                check.overscan_tiles += 1;
                check.overscan_differing += u64::from(game != lookup);
                continue;
            }
            check.tiles += 1;
            check.outside += u64::from(!state.map.contains(x, y));
            let row = rows.entry(y >> 3).or_default();
            row.1 += u32::from(game != 0);
            if game != lookup {
                row.0.push(TileDifference { world: (x, y), game, lookup });
            }
        }
    }
    for (differences, filled) in rows.into_values() {
        if filled == 0 {
            check.unbuilt += differences.len() as u64;
        } else {
            check.differing += differences.len() as u64;
            if check.first.is_none() {
                check.first = differences.first().copied();
            }
        }
    }
    check
}

/// The parts the port puts in the OAM against the entries the game's OAM holds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ObjectCheck {
    /// Parts the original builder keeps for the objects it draws.
    pub parts: u64,
    /// Of those, the ones with no equal entry in the frame's OAM.
    pub missing: u64,
    /// Frames with parts none of which is in the OAM: the object loop did not run in the frame
    /// (a map being loaded clears the OAM while the object table is already filled; a scene
    /// state may draw its own objects), so there is nothing to compare. Their parts are not
    /// counted above.
    pub frames_without_loop: u64,
}

/// Looks every part the original draws for the map objects up in the frame's OAM (position,
/// tile, attributes and size bits equal).
#[must_use]
pub fn check_objects(rom: Rom<'_>, state: &FieldState, log: &FrameLog) -> ObjectCheck {
    let mut check = ObjectCheck::default();
    let parts = state.original_parts(rom);
    let in_oam = |part: &super::objects::Part| {
        let (low, high) = part.oam_entry();
        (0..128_usize).any(|index| {
            log.oam_low[index * 4..index * 4 + 4] == low
                && (log.oam_high[index >> 2] >> (2 * (index & 3))) & 3 == high
        })
    };
    let missing = parts.iter().filter(|part| !in_oam(part)).count() as u64;
    if !parts.is_empty() && missing == parts.len() as u64 {
        check.frames_without_loop = 1;
        return check;
    }
    check.parts = parts.len() as u64;
    check.missing = missing;
    check
}
