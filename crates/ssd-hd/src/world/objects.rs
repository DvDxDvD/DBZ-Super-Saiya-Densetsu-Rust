//! The field's map objects (NPCs, Dragon Balls, chests): a port of the loop that draws them
//! (`03:9158`) and of the metasprite builder it calls (`00:85F5`), with the culling made explicit
//! so a wider view can place what the original drops.
//!
//! The original works out `screen = world - camera` for each object of the table at `$1000` and
//! skips it unless both coordinates are below 255 (unsigned), so an object whose origin is just
//! outside the 256-pixel window is never drawn, even if its sprite would reach into it. For the
//! rest, the builder hides a part whose x ends up above 255 and one whose y is not in `0..0xF0`.
//! Positions here are relative to the original window (the camera), so the wide view can place
//! them wherever it puts that window.

use super::map::{FieldState, Rom};

/// Bank of the metasprite lists and of the word table (`02:802E`) that points at them, by type.
const LIST_BANK: u8 = 0x02;
const LIST_TABLE: u16 = 0x802E;
/// A list ends with a `0x80` in the place of a part's y offset; this stops a damaged one.
const MAX_PARTS: usize = 128;
/// `03:91D4` sets the attribute bits to OR in (`$10` = 1: the second name table) and the tile base.
const ATTRIBUTE_OR: u8 = 0x01;

/// One OAM entry the builder writes for a part of an object's metasprite, before the original's
/// horizontal clipping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Part {
    /// X of the left edge relative to the original window (negative: left of it).
    pub x: i32,
    pub y: u8,
    pub tile: u8,
    pub attributes: u8,
    /// A large (16 x 16 in the field) sprite, the list's first byte bit 0.
    pub large: bool,
}

impl Part {
    /// Whether the original builder keeps the part: it hides one whose x is above 255.
    #[must_use]
    pub fn in_original(&self) -> bool {
        self.x <= 255
    }

    /// The OAM entry as the builder writes it: the 9-bit x, y, tile and attribute bytes, and the
    /// two bits of the high table (x bit 8, size).
    #[must_use]
    pub fn oam_entry(&self) -> ([u8; 4], u8) {
        let x = (self.x & 0x1FF) as u16;
        (
            [x as u8, self.y, self.tile, self.attributes],
            ((x >> 8) as u8) | (u8::from(self.large) << 1),
        )
    }
}

/// Places the parts of the metasprite list of object type `kind` with its origin at
/// `(base_x, base_y)` relative to the window, the way `00:85F5` does but without hiding a part
/// whose x is above 255. A part whose y is not in `0..0xF0` is hidden, as in the original.
#[must_use]
pub fn metasprite(rom: Rom<'_>, kind: u8, base_x: i32, base_y: u8) -> Vec<Part> {
    let list = rom.word(LIST_BANK, LIST_TABLE.wrapping_add(u16::from(kind & 0x1F) * 2));
    let large = rom.byte(LIST_BANK, list) & 1 != 0;
    let mut parts = Vec::new();
    let mut at = list.wrapping_add(1);
    for _ in 0..MAX_PARTS {
        let y_offset = rom.byte(LIST_BANK, at);
        if y_offset == 0x80 {
            break;
        }
        let tile = rom.byte(LIST_BANK, at.wrapping_add(1));
        let attributes = rom.byte(LIST_BANK, at.wrapping_add(2));
        let x_offset = rom.byte(LIST_BANK, at.wrapping_add(3));
        at = at.wrapping_add(4);
        let y = i32::from(base_y) + i32::from(y_offset as i8);
        if !(0..0xF0).contains(&y) {
            continue;
        }
        parts.push(Part {
            x: base_x + i32::from(x_offset as i8),
            y: y as u8,
            tile,
            attributes: attributes | ATTRIBUTE_OR,
            large,
        });
    }
    parts
}

/// A map object in use, relative to the camera.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapObject {
    /// The type: the low 5 bits of the first byte, which pick the metasprite list.
    pub kind: u8,
    pub x: i32,
    pub y: i32,
}

impl MapObject {
    /// Whether `03:9158` draws the object: both coordinates below 255 as unsigned numbers.
    #[must_use]
    pub fn drawn_by_original(&self) -> bool {
        (0..0xFF).contains(&self.x) && (0..0xFF).contains(&self.y)
    }
}

impl FieldState {
    /// The map objects in use (the first byte's bit 7), relative to the camera.
    pub fn map_objects(&self) -> impl Iterator<Item = MapObject> + '_ {
        self.objects.iter().filter(|object| object[0] & 0x80 != 0).map(|object| {
            let word = |at: usize| u16::from(object[at]) | u16::from(object[at + 1]) << 8;
            MapObject {
                kind: object[0] & 0x1F,
                x: i32::from(word(2).wrapping_sub(self.camera.0) as i16),
                y: i32::from(word(4).wrapping_sub(self.camera.1) as i16),
            }
        })
    }

    /// The parts the original puts in the OAM for the map objects, in the order it does.
    #[must_use]
    pub fn original_parts(&self, rom: Rom<'_>) -> Vec<Part> {
        if self.copy_select & 0x80 != 0 {
            return Vec::new();
        }
        self.map_objects()
            .filter(MapObject::drawn_by_original)
            .flat_map(|object| metasprite(rom, object.kind, object.x, object.y as u8))
            .filter(Part::in_original)
            .collect()
    }

    /// Every part of every map object a wide view can show, in the order the original's loop
    /// writes them (the objects of the table, each one's list in order), with whether the original
    /// puts it in the OAM. An object culled vertically stays culled, and the other drawing path
    /// (`$01E7` bit 7) is not ported.
    #[must_use]
    pub fn all_parts(&self, rom: Rom<'_>) -> Vec<ObjectPart> {
        if self.copy_select & 0x80 != 0 {
            return Vec::new();
        }
        let mut parts = Vec::new();
        for object in self.map_objects().filter(|object| (0..0xFF).contains(&object.y)) {
            let drawn = object.drawn_by_original();
            parts.extend(
                metasprite(rom, object.kind, object.x, object.y as u8)
                    .into_iter()
                    .map(|part| ObjectPart { part, in_original: drawn && part.in_original() }),
            );
        }
        parts
    }

    /// The parts the original drops and a wider view can show: those of an object whose origin
    /// it culls horizontally, and the parts of the others that reach past x = 255. An object
    /// culled vertically stays culled, and the other drawing path (`$01E7` bit 7) is not ported.
    #[must_use]
    pub fn extra_parts(&self, rom: Rom<'_>) -> Vec<Part> {
        self.all_parts(rom)
            .into_iter()
            .filter(|object| !object.in_original)
            .map(|object| object.part)
            .collect()
    }
}

/// A part of a map object as the wide view sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectPart {
    pub part: Part,
    /// Whether the original's loop writes it into the OAM.
    pub in_original: bool,
}

/// Where the original would have written each part of `all` ([`FieldState::all_parts`]) had it
/// kept them all: for a part the original does drop, the OAM entry it follows in the order of
/// that loop (the last entry written for the parts before it, or the one before the objects'
/// block when it comes first), `None` for every part when the block of the objects cannot be
/// found in the OAM (`low` and `high` are the frame's tables). The pieces of one object stay next
/// to each other that way, whatever side of the window's edge each lies on, which is what the
/// field art's grouping (runs of one palette, in the order of the entries) needs to see an object
/// whole.
#[must_use]
pub fn drop_anchors(all: &[ObjectPart], low: &[u8], high: &[u8]) -> Vec<Option<i32>> {
    let written: Vec<&ObjectPart> = all.iter().filter(|object| object.in_original).collect();
    let entry_is = |at: usize, object: &ObjectPart| {
        let (bytes, bits) = object.part.oam_entry();
        low[at * 4..at * 4 + 4] == bytes && (high[at >> 2] >> (2 * (at & 3))) & 3 == bits & 3
    };
    let start = (0..=128_usize.saturating_sub(written.len()))
        .find(|&start| written.iter().enumerate().all(|(n, object)| entry_is(start + n, object)));
    let (Some(start), false) = (start, written.is_empty()) else {
        return vec![None; all.len()];
    };
    let mut last = start as i32 - 1;
    all.iter()
        .map(|object| {
            if object.in_original {
                last += 1;
                None
            } else {
                Some(last)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::map::tests::put;
    use super::*;

    /// Metasprite lists in bank 2: type 1 is two 8 x 8 parts (one at the origin, one 12 to the
    /// right and 2 above), type 2 is large with a part at (-4, 6) and one below the origin that
    /// reaches y = 0xF0.
    fn rom() -> Vec<u8> {
        let mut rom = vec![0; 0x20000];
        put(&mut rom, 2, LIST_TABLE + 2, &0x9000_u16.to_le_bytes());
        put(&mut rom, 2, LIST_TABLE + 4, &0x9100_u16.to_le_bytes());
        put(&mut rom, 2, 0x9000, &[0, 0, 0x10, 0x20, 0, 0xFE, 0x11, 0x22, 12, 0x80]);
        put(&mut rom, 2, 0x9100, &[1, 6, 0x30, 0x04, 0xFC, 0x7F, 0x31, 0x00, 0, 0x80]);
        rom
    }

    fn state(objects: &[(u8, u16, u16)], camera: (u16, u16)) -> FieldState {
        let mut wram = vec![0; 0x20000];
        wram[0x702] = 4;
        wram[0x703] = 4;
        for (index, &(kind, x, y)) in objects.iter().enumerate() {
            let at = 0x1000 + index * 6;
            wram[at] = 0x80 | kind;
            wram[at + 2..at + 4].copy_from_slice(&x.to_le_bytes());
            wram[at + 4..at + 6].copy_from_slice(&y.to_le_bytes());
        }
        wram[0x33..0x35].copy_from_slice(&camera.0.to_le_bytes());
        wram[0x35..0x37].copy_from_slice(&camera.1.to_le_bytes());
        FieldState::from_wram(&wram).unwrap()
    }

    #[test]
    fn a_list_is_placed_part_by_part() {
        let rom = rom();
        let parts = metasprite(Rom(&rom), 1, 100, 50);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0], Part { x: 100, y: 50, tile: 0x10, attributes: 0x21, large: false });
        // y offset 0xFE is -2, x offset 12
        assert_eq!(parts[1], Part { x: 112, y: 48, tile: 0x11, attributes: 0x23, large: false });
        // Large list: the second part (y + 127) overflows and is hidden
        let parts = metasprite(Rom(&rom), 2, 10, 0xE0);
        assert_eq!(parts, [Part { x: 6, y: 0xE6, tile: 0x30, attributes: 0x05, large: true }]);
    }

    #[test]
    fn a_part_above_the_top_or_below_the_last_line_is_hidden() {
        let rom = rom();
        // Type 2's parts are at y + 6 and y + 127: both are in from y = 0; from 0xEA the first
        // reaches 0xF0 and the second overflows
        let parts = metasprite(Rom(&rom), 2, 0, 0);
        assert_eq!(parts.iter().map(|p| p.y).collect::<Vec<_>>(), [6, 127]);
        let parts = metasprite(Rom(&rom), 2, 0, 0xEA);
        assert_eq!(parts.iter().map(|p| p.y).collect::<Vec<_>>(), [] as [u8; 0]);
        let parts = metasprite(Rom(&rom), 1, 0, 1);
        assert_eq!(parts.iter().map(|p| p.y).collect::<Vec<_>>(), [1], "the part at -1 is hidden");
    }

    #[test]
    fn the_oam_entry_has_the_ninth_bit_and_the_size() {
        let part = Part { x: -4, y: 9, tile: 1, attributes: 3, large: true };
        assert_eq!(part.oam_entry(), ([0xFC, 9, 1, 3], 0b11));
        let part = Part { x: 200, y: 9, tile: 1, attributes: 3, large: false };
        assert_eq!(part.oam_entry(), ([200, 9, 1, 3], 0));
    }

    #[test]
    fn objects_are_culled_on_their_origin_at_255() {
        let state = state(
            &[(1, 1000, 80), (1, 1254, 80), (1, 1255, 80), (1, 999, 80), (1, 1100, 335)],
            (1000, 80),
        );
        let drawn: Vec<bool> = state.map_objects().map(|o| o.drawn_by_original()).collect();
        // x 0 and 254 are drawn; 255 and -1 are not; y 255 is not
        assert_eq!(drawn, [true, true, false, false, false]);
    }

    #[test]
    fn extra_parts_are_those_the_original_drops() {
        let rom = rom();
        // A: inside; its second part is at x 254 + 12 = 266 > 255: dropped by the builder.
        // B: origin at 256 (culled): both parts are extra. C: origin at -8 (culled).
        // D: culled vertically: nothing.
        let state =
            state(&[(1, 1254, 130), (1, 1256, 130), (1, 992, 130), (1, 1100, 400)], (1000, 80));
        let original: Vec<(i32, u8)> =
            state.original_parts(Rom(&rom)).iter().map(|p| (p.x, p.y)).collect();
        assert_eq!(original, [(254, 50)]);
        let extra: Vec<(i32, u8)> =
            state.extra_parts(Rom(&rom)).iter().map(|p| (p.x, p.y)).collect();
        assert_eq!(extra, [(266, 48), (256, 50), (268, 48), (-8, 50), (4, 48)]);
    }

    #[test]
    fn the_other_drawing_path_is_not_ported() {
        let rom = rom();
        let mut state = state(&[(1, 1256, 130)], (1000, 80));
        assert_eq!(state.extra_parts(Rom(&rom)).len(), 2);
        state.copy_select = 0x80;
        assert_eq!(state.extra_parts(Rom(&rom)), Vec::<Part>::new());
        assert_eq!(state.original_parts(Rom(&rom)), Vec::<Part>::new());
    }
}
