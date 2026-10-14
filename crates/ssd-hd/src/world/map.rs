//! The field map: the game's own lookup from a world pixel to a tile map word, ported from
//! `06:F089`, and the state it reads (`docs/re/data/graphics.md`, "The field map").
//!
//! The field's BG2 is not a stored picture. The game keeps a header per map (its size in blocks
//! of 256 x 256 pixels, a grid of block ids, a metatile table) and builds the 32 words of a map
//! column through this lookup as the camera crosses a tile boundary, so VRAM holds only about 35
//! columns of the map. The lookup is a pure function of those pointers and ROM tables, so a wider
//! view can ask for the columns VRAM does not hold.

/// The cartridge image (`LoROM`, no copier header).
#[derive(Debug, Clone, Copy)]
pub struct Rom<'a>(pub &'a [u8]);

impl Rom<'_> {
    /// The byte at `bank:address`; 0 where the address is not ROM or is past the image.
    #[must_use]
    pub fn byte(&self, bank: u8, address: u16) -> u8 {
        if address < 0x8000 {
            return 0;
        }
        let offset = usize::from(bank & 0x7F) * 0x8000 + usize::from(address - 0x8000);
        self.0.get(offset).copied().unwrap_or(0)
    }

    /// The little-endian word at `bank:address`.
    #[must_use]
    pub fn word(&self, bank: u8, address: u16) -> u16 {
        u16::from(self.byte(bank, address))
            | u16::from(self.byte(bank, address.wrapping_add(1))) << 8
    }
}

// Work RAM variables the field reads (`$7E:xxxx`).
pub(crate) const GAME_MODE: usize = 0x24;
const CAMERA_X: usize = 0x33;
const CAMERA_Y: usize = 0x35;
const MAP_ID: usize = 0x1A0;
const COPY_SELECT: usize = 0x1E7;
const MAP_HEIGHT: usize = 0x702;
const MAP_WIDTH: usize = 0x703;
const MAP_GRID: usize = 0x704;
const MAP_METATILES: usize = 0x707;
const OBJECTS: usize = 0x1000;
/// Map objects: 6 bytes each, up to 32.
pub(crate) const OBJECT_COUNT: usize = 32;
pub(crate) const OBJECT_SIZE: usize = 6;

/// Table of the block definitions' locations (`06:F1EE`, a word per block id).
const BLOCK_TABLE: u16 = 0xF1EE;
const BLOCK_TABLE_BANK: u8 = 0x06;

/// A map header as `00:8E26` leaves it in work RAM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldMap {
    /// Width and height in blocks of 256 x 256 pixels.
    pub blocks: (u8, u8),
    /// The grid of block ids, a byte per block, row by row (bank, address).
    grid: (u8, u16),
    /// The metatile table: four tile map words per metatile (bank, address).
    metatiles: (u8, u16),
}

impl FieldMap {
    /// Size of the map in pixels.
    #[must_use]
    pub fn size(&self) -> (i32, i32) {
        (i32::from(self.blocks.0) * 256, i32::from(self.blocks.1) * 256)
    }

    /// Where the map's data sits in the ROM: the grid of block ids and the metatile table, as
    /// `(bank, address)` each. Two maps with the same size and the same two pointers are the same
    /// layer.
    #[must_use]
    pub fn layout(&self) -> ((u8, u16), (u8, u16)) {
        (self.grid, self.metatiles)
    }

    /// Whether the world pixel is inside the map.
    #[must_use]
    pub fn contains(&self, x: i32, y: i32) -> bool {
        let (width, height) = self.size();
        (0..width).contains(&x) && (0..height).contains(&y)
    }

    /// The tile map word of the 8 x 8 tile that holds world pixel `(x, y)`, as `06:F089` gives
    /// it: a pixel outside the map gives 0 (tile 0, palette 0).
    #[must_use]
    pub fn tile_word(&self, rom: Rom<'_>, x: i32, y: i32) -> u16 {
        if !self.contains(x, y) {
            return 0;
        }
        let (x, y) = (x as u32, y as u32);
        let (block_x, block_y) = (x >> 8, y >> 8);
        let (grid_bank, grid_address) = self.grid;
        let id = rom.byte(
            grid_bank,
            grid_address.wrapping_add((block_y * u32::from(self.blocks.0) + block_x) as u16),
        );
        // The block's definition: 16 x 16 cells of 16 x 16 pixels, a word each (the metatile)
        let definition = rom.word(BLOCK_TABLE_BANK, BLOCK_TABLE.wrapping_add(u16::from(id) * 2));
        let (definition_bank, definition_address) = ((definition >> 8) as u8, definition << 8);
        let cell = (y & 0xF0) | ((x & 0xFF) >> 4);
        let metatile =
            rom.word(definition_bank, definition_address.wrapping_add((cell * 2) as u16));
        // Four words a metatile with a row stride of 0x40 bytes; bit 3 of the pixel picks the
        // quarter. The words are stored high byte first.
        let (bank, address) = self.metatiles;
        let table = u32::from(address) + ((u32::from(metatile) & 0x3FDE) << 1);
        let bank = bank.wrapping_add((table >> 16) as u8);
        let quarter = (((x & 8) >> 2) | ((y & 8) << 3)) & 0x42;
        let address = ((table & 0xFFFF) as u16).wrapping_add(quarter as u16);
        u16::from(rom.byte(bank, address)) << 8 | u16::from(rom.byte(bank, address.wrapping_add(1)))
    }
}

/// The work RAM variables the field's world is built from, copied out of the machine.
///
/// Take it from the work RAM as it stood when the frame's NMI ran, which is the end of the
/// previous frame's run: the NMI writes the scroll registers and builds the objects from it, so
/// that is what the next frame displays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldState {
    /// `$24`: the main loop's state (bit 7: just entered).
    pub game_mode: u8,
    /// `$01A0`.
    pub map_id: u8,
    /// `$01E7`; bit 7 selects the other object-drawing path.
    pub copy_select: u8,
    pub map: FieldMap,
    /// The camera in world pixels (`$33`, `$35`).
    pub camera: (u16, u16),
    /// The map object table (`$1000`).
    pub(crate) objects: [[u8; OBJECT_SIZE]; OBJECT_COUNT],
}

impl FieldState {
    /// Reads the field variables out of the 128 KiB of work RAM (`$7E:0000` is index 0). `None`
    /// when there is no map header (no size).
    #[must_use]
    pub fn from_wram(wram: &[u8]) -> Option<Self> {
        if wram.len() < OBJECTS + OBJECT_COUNT * OBJECT_SIZE {
            return None;
        }
        let word = |at: usize| u16::from(wram[at]) | u16::from(wram[at + 1]) << 8;
        let (height, width) = (wram[MAP_HEIGHT], wram[MAP_WIDTH]);
        if width == 0 || height == 0 {
            return None;
        }
        let map = FieldMap {
            blocks: (width, height),
            grid: (wram[MAP_GRID + 2], word(MAP_GRID)),
            metatiles: (wram[MAP_METATILES + 2], word(MAP_METATILES)),
        };
        let mut objects = [[0; OBJECT_SIZE]; OBJECT_COUNT];
        for (index, object) in objects.iter_mut().enumerate() {
            let at = OBJECTS + index * OBJECT_SIZE;
            object.copy_from_slice(&wram[at..at + OBJECT_SIZE]);
        }
        Some(Self {
            game_mode: wram[GAME_MODE],
            map_id: wram[MAP_ID],
            copy_select: wram[COPY_SELECT],
            map,
            camera: (word(CAMERA_X), word(CAMERA_Y)),
            objects,
        })
    }

    /// Whether the main loop is in a state that shows the field (`$00` or `$06`).
    #[must_use]
    pub fn is_field(&self) -> bool {
        matches!(self.game_mode & 0x7F, 0x00 | 0x06)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn put(rom: &mut [u8], bank: u8, address: u16, bytes: &[u8]) {
        let offset = usize::from(bank) * 0x8000 + usize::from(address - 0x8000);
        rom[offset..offset + bytes.len()].copy_from_slice(bytes);
    }

    /// The metatile at cell `(cx, cy)` of block `n` in the synthetic ROM.
    fn metatile_of(n: u8, cx: u32, cy: u32) -> u16 {
        (((cx + 16 * cy) * 4 + u32::from(n) * 8) % 0x100) as u16 * 2
    }

    /// A small `LoROM` image holding a map of 2 x 1 blocks (ids 0 and 1, the grid at `04:8000`),
    /// the block table at `06:F1EE` pointing at the definitions in bank 2, and the metatile table
    /// at `03:8000` (the quarters of metatile `m` are the tile words `0x2000 | m & 0x3FDE | q`).
    pub(crate) fn synthetic_rom() -> Vec<u8> {
        let mut rom = vec![0; 0x60000];
        put(&mut rom, 4, 0x8000, &[0, 1]);
        put(&mut rom, 6, BLOCK_TABLE, &[0x82, 0x02, 0x84, 0x02]);
        for n in 0..2_u8 {
            let mut cells = Vec::new();
            for cy in 0..16 {
                for cx in 0..16 {
                    cells.extend_from_slice(&metatile_of(n, cx, cy).to_le_bytes());
                }
            }
            put(&mut rom, 2, 0x8000 + 0x200 * (u16::from(n) + 1), &cells);
        }
        for m in 0..0x200_u16 {
            let masked = m & 0x3FDE;
            for (quarter, offset) in [(0_u16, 0_u16), (1, 2), (2, 0x40), (3, 0x42)] {
                let word = 0x2000 | masked | quarter;
                put(&mut rom, 3, 0x8000 + (masked << 1) + offset, &word.to_be_bytes());
            }
        }
        rom
    }

    pub(crate) fn synthetic_map() -> FieldMap {
        FieldMap { blocks: (2, 1), grid: (4, 0x8000), metatiles: (3, 0x8000) }
    }

    #[test]
    fn a_world_pixel_resolves_through_block_cell_and_quarter() {
        let rom = synthetic_rom();
        let map = synthetic_map();
        let points = [(0, 0), (8, 0), (0, 8), (8, 8), (100, 37), (255, 255), (256, 0), (511, 200)];
        for (x, y) in points {
            let (block, cx, cy) = (x / 256, (x % 256) / 16, y / 16);
            let metatile = metatile_of(block as u8, cx as u32, cy as u32);
            let quarter = ((x / 8) & 1) as u16 + 2 * ((y / 8) & 1) as u16;
            let expected = 0x2000 | (metatile & 0x3FDE) | quarter;
            assert_eq!(map.tile_word(Rom(&rom), x, y), expected, "pixel ({x}, {y})");
        }
    }

    #[test]
    fn a_pixel_outside_the_map_is_tile_zero() {
        let rom = synthetic_rom();
        let map = synthetic_map();
        for (x, y) in [(-1, 0), (0, -1), (512, 0), (0, 256), (i32::MIN, i32::MAX)] {
            assert!(!map.contains(x, y));
            assert_eq!(map.tile_word(Rom(&rom), x, y), 0);
        }
        assert!(map.contains(511, 255));
    }

    #[test]
    fn rom_reads_stay_inside_the_image() {
        let rom = [1_u8, 2, 3];
        let rom = Rom(&rom);
        assert_eq!(rom.byte(0, 0x8001), 2);
        assert_eq!(rom.word(0, 0x8001), 0x0302);
        assert_eq!(rom.byte(0, 0x8003), 0, "past the image");
        assert_eq!(rom.byte(0, 0x7FFF), 0, "not ROM");
    }

    #[test]
    fn the_state_is_read_from_work_ram() {
        let mut wram = vec![0; 0x20000];
        assert_eq!(FieldState::from_wram(&wram), None, "no map size");
        wram[MAP_WIDTH] = 9;
        wram[MAP_HEIGHT] = 11;
        wram[MAP_GRID..MAP_GRID + 3].copy_from_slice(&[0x34, 0x92, 0x0B]);
        wram[MAP_METATILES..MAP_METATILES + 3].copy_from_slice(&[0x00, 0xA0, 0x0C]);
        wram[CAMERA_X..CAMERA_X + 2].copy_from_slice(&0x0123_u16.to_le_bytes());
        wram[CAMERA_Y..CAMERA_Y + 2].copy_from_slice(&0x0456_u16.to_le_bytes());
        wram[GAME_MODE] = 0x86;
        wram[MAP_ID] = 5;
        let state = FieldState::from_wram(&wram).unwrap();
        assert_eq!(state.map.blocks, (9, 11));
        assert_eq!(state.map.size(), (2304, 2816));
        assert_eq!(state.map.grid, (0x0B, 0x9234));
        assert_eq!(state.map.metatiles, (0x0C, 0xA000));
        assert_eq!(state.camera, (0x123, 0x456));
        assert!(state.is_field(), "bit 7 only says 'just entered'");
        wram[GAME_MODE] = 0x07;
        assert!(!FieldState::from_wram(&wram).unwrap().is_field());
    }
}
