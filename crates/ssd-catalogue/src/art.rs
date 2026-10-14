//! The pictures of the objects, decoded from the ROM for the HD menus (`docs/re/data/menus.md`,
//! "Portraits"). Nothing is stored: the decoders read the ROM image they are given, and the
//! pictures live in memory only.
//!
//! Each object has a compressed set of 32 tiles (`00:C559`) that holds two pictures: tiles 0 to 7
//! and 16 to 23 (the first half) and tiles 8 to 15 and 24 to 31 (the second), each a 4 by 4 grid of
//! 8 by 8 tiles. Two objects share most sets (their pointers are the same), and one bit of a table
//! per object (`object_portrait_halves`) says which half the object shows. A tile is 4 bits per
//! pixel, stored as four planes of eight bytes; the palette is the object's own row of sixteen
//! colours in bank `$09`.

use crate::CatalogueError;
use crate::names::next_address;
use crate::rom::RomView;
use ssd_game::data as d;

/// Side of a picture in pixels.
pub const SIDE: usize = 32;
/// Colours of a palette row.
pub const COLOURS: usize = 16;
/// Bytes of one 4 bits per pixel tile.
const TILE_BYTES: usize = 32;
/// Tiles of one object's set: two pictures of 16 tiles.
const TILES: usize = 32;
/// The tiles of the first picture of a set, row by row of its 4 by 4 grid. Tile numbers index the
/// 32 tiles of the set, as the item screen's tile map places them.
const PICTURE: [usize; 16] = [0, 1, 2, 3, 16, 17, 18, 19, 4, 5, 6, 7, 20, 21, 22, 23];
/// The tiles of the second picture of a set, in the same arrangement.
const SECOND_PICTURE: [usize; 16] = [8, 9, 10, 11, 24, 25, 26, 27, 12, 13, 14, 15, 28, 29, 30, 31];
/// The bank of the palettes (the pointers of `object_portrait_palettes` are its words).
const PALETTE_BANK: u8 = 0x09;
/// The bank of the picture pointers is the third byte of each entry of `object_portrait_pointers`.
const PICTURE_ENTRY: u32 = 3;

/// The picture of an object: palette indexes of its pixels, and its palette.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Portrait {
    /// Which half of the object's set the picture is (0 or 1).
    pub half: u8,
    /// The sixteen characters of the picture as they sit in video memory (four-bit characters of
    /// sixteen words, row by row of the 4 by 4 grid): what the game uploads and the HD pack's tiles
    /// are keyed by.
    pub characters: Vec<[u16; 16]>,
    /// Palette index of each pixel, row by row. Index 0 is transparent.
    pub pixels: Vec<u8>,
    /// The sixteen colours, as the SNES stores them: `0BBBBBGGGGGRRRRR`.
    pub palette: [u16; COLOURS],
}

/// Decodes the compressed picture set at `pointer` (`00:C559`): a 16-bit output size, then groups of
/// eight items led by a flag byte read least-significant bit first. A set bit is a literal byte; a
/// clear bit a back reference of two bytes, whose low nibble plus 2 is the length code and whose
/// other twelve bits are the distance. The copy runs one byte longer than the length code, and a
/// source before the start of the output reads as zero.
///
/// # Errors
///
/// [`CatalogueError::OutOfRom`] when the stream leaves the ROM image.
pub fn lz_decode(rom: RomView<'_>, pointer: u32) -> Result<Vec<u8>, CatalogueError> {
    let mut reader = Reader { rom, address: pointer };
    let size = usize::from(reader.word()?);
    let mut out: Vec<u8> = Vec::with_capacity(size);
    let mut flags = 0_u8;
    let mut bits_left = 0_u8;
    while out.len() < size {
        if bits_left == 0 {
            flags = reader.byte()?;
            bits_left = 8;
        }
        let literal = flags & 1 != 0;
        flags >>= 1;
        bits_left -= 1;
        if literal {
            out.push(reader.byte()?);
        } else {
            let low = reader.byte()?;
            let high = reader.byte()?;
            let distance = (usize::from(high) << 4) | usize::from(low >> 4);
            let count = usize::from(low & 0x0F) + 3;
            for _ in 0..count {
                if out.len() >= size {
                    break;
                }
                let source = out.len().checked_sub(distance);
                // A source that is not written yet (a distance of zero) reads as zero too.
                out.push(source.and_then(|source| out.get(source).copied()).unwrap_or(0));
            }
        }
    }
    Ok(out)
}

/// The byte reader of a compressed stream: it follows the ROM across bank boundaries.
struct Reader<'a> {
    rom: RomView<'a>,
    address: u32,
}

impl Reader<'_> {
    fn byte(&mut self) -> Result<u8, CatalogueError> {
        let value = self.rom.u8(self.address)?;
        self.address = next_address(self.address);
        Ok(value)
    }

    fn word(&mut self) -> Result<u16, CatalogueError> {
        Ok(u16::from(self.byte()?) | (u16::from(self.byte()?) << 8))
    }
}

/// Which half of its set object `object` shows: its bit of `object_portrait_halves`.
fn half_of(rom: RomView<'_>, object: u8) -> Result<u8, CatalogueError> {
    let byte = rom.u8(d::OBJECT_PORTRAIT_HALVES + u32::from(object / 8))?;
    Ok((byte >> (object % 8)) & 1)
}

/// The picture of object `object`: its set decoded, the half of it the object shows, and its
/// palette read.
///
/// # Errors
///
/// [`CatalogueError::OutOfRom`] for an object without a pointer in the ROM, or a set that is
/// shorter than its two pictures.
pub fn object_portrait(rom: RomView<'_>, object: u8) -> Result<Portrait, CatalogueError> {
    let entry = d::OBJECT_PORTRAIT_POINTERS + PICTURE_ENTRY * u32::from(object);
    let bank = rom.u8(entry + 2)?;
    let set = lz_decode(rom, rom.pointer(entry, bank)?)?;
    if set.len() < TILES * TILE_BYTES {
        return Err(CatalogueError::Malformed {
            table: "object_portrait_pointers",
            message: format!(
                "object {object}: a set of {} bytes, expected {}",
                set.len(),
                TILES * TILE_BYTES
            ),
        });
    }
    let palette_address =
        rom.pointer(d::OBJECT_PORTRAIT_PALETTES + 2 * u32::from(object), PALETTE_BANK)?;
    let mut palette = [0_u16; COLOURS];
    for (index, colour) in palette.iter_mut().enumerate() {
        *colour = rom.u16(palette_address + 2 * index as u32)?;
    }
    let half = half_of(rom, object)?;
    let cells = if half == 0 { &PICTURE } else { &SECOND_PICTURE };
    let mut pixels = vec![0_u8; SIDE * SIDE];
    let mut characters = Vec::with_capacity(cells.len());
    for (cell, &tile) in cells.iter().enumerate() {
        let bytes = &set[tile * TILE_BYTES..(tile + 1) * TILE_BYTES];
        // The character as video memory holds it: row by row the first two planes (the low byte
        // is plane 0), then the last two
        characters.push(std::array::from_fn(|word| {
            let (low, high) = if word < 8 { (0, 1) } else { (2, 3) };
            let row = word % 8;
            u16::from(bytes[low * 8 + row]) | (u16::from(bytes[high * 8 + row]) << 8)
        }));
        for row in 0..8 {
            for column in 0..8 {
                let bit = 7 - column;
                let index = (0..4).fold(0_u8, |index, plane| {
                    index | (((bytes[plane * 8 + row] >> bit) & 1) << plane)
                });
                let x = (cell % 4) * 8 + column;
                let y = (cell / 4) * 8 + row;
                pixels[y * SIDE + x] = index;
            }
        }
    }
    Ok(Portrait { half, characters, pixels, palette })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::rom_with;

    /// A ROM image whose bytes at bank `$02` `$8000` hold `stream`, read by [`lz_decode`].
    fn decode(stream: &[u8]) -> Result<Vec<u8>, CatalogueError> {
        let rom = rom_with(&[(0x02_8000, stream)]);
        lz_decode(RomView::new(&rom).unwrap(), 0x02_8000)
    }

    #[test]
    fn literals_are_copied_as_they_come() {
        // Size 3, flags 1 1 1 (three literals), then the three bytes.
        assert_eq!(decode(&[3, 0, 0b0000_0111, 10, 20, 30]).unwrap(), [10, 20, 30]);
    }

    #[test]
    fn a_back_reference_copies_from_its_distance_one_byte_longer_than_its_length_code() {
        // Size 5: literals 1 and 2, then a clear flag: low byte $20 = distance 2 (high nibble) with
        // length code 0 (copies 3), high byte 0. The copy is 1 2 1, filling the five bytes.
        let stream = [5, 0, 0b0000_0011, 1, 2, 0x20, 0x00];
        assert_eq!(decode(&stream).unwrap(), [1, 2, 1, 2, 1]);
    }

    #[test]
    fn a_source_before_the_start_of_the_output_is_zero() {
        // Size 3, a clear flag at once: distance 2 from an empty output reads two zeros, then the
        // first zero is the source of the third.
        assert_eq!(decode(&[3, 0, 0b0000_0000, 0x20, 0x00]).unwrap(), [0, 0, 0]);
    }

    #[test]
    fn a_stream_that_leaves_the_rom_is_an_error() {
        // A size of 2 at the very end of the image: the flag byte after it lies outside the ROM.
        let rom = rom_with(&[(0x1F_FFFE, &[2, 0])]);
        let view = RomView::new(&rom).unwrap();
        let error = lz_decode(view, 0x1F_FFFE).unwrap_err();
        assert!(matches!(error, CatalogueError::OutOfRom { .. }), "{error}");
    }

    #[test]
    fn the_planes_of_a_tile_make_its_pixel_indexes() {
        // A tile whose plane 0 is all ones and whose plane 3 is one row of ones: pixel 1 of row 0
        // has index 9 (planes 0 and 3), the others index 1.
        let mut tile = [0_u8; TILE_BYTES];
        tile[..8].fill(0xFF);
        tile[24] = 0xFF;
        let mut pixels = [0_u8; 64];
        for row in 0..8 {
            for column in 0..8 {
                let bit = 7 - column;
                pixels[row * 8 + column] = (0..4).fold(0_u8, |index, plane| {
                    index | (((tile[plane * 8 + row] >> bit) & 1) << plane)
                });
            }
        }
        assert_eq!(pixels[0], 9);
        assert_eq!(pixels[8], 1);
    }

    #[test]
    fn the_item_picture_takes_its_tiles_from_both_halves_of_the_set() {
        // Tiles 0-7 and 16-23 form the picture: a set whose first byte of tile n is n is a picture
        // that names its own tiles.
        assert_eq!(PICTURE.len(), 16);
        let mut seen = PICTURE.to_vec();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen, [0, 1, 2, 3, 4, 5, 6, 7, 16, 17, 18, 19, 20, 21, 22, 23]);
    }

    #[test]
    fn the_picture_and_palette_tables_are_named_by_the_symbols_file() {
        assert_eq!(d::OBJECT_PORTRAIT_POINTERS_SIZE / PICTURE_ENTRY, 40);
        assert_eq!(d::OBJECT_PORTRAIT_PALETTES_SIZE / 2, 40);
    }

    /// A ROM with one object (number 1) whose set is 32 tiles, tile `n` having the byte `n + 1`
    /// in every byte of its plane 0, `0xFF` in its plane 1 when it is in the second half, and
    /// 0 in the others; `halves` is the first byte of the half table.
    fn rom_of_a_set(halves: u8) -> Vec<u8> {
        let mut stream = vec![0_u8, 4]; // the output size, 1024
        for group in 0..128 {
            stream.push(0xFF); // eight literals
            for item in 0..8 {
                let at = group * 8 + item;
                let (tile, byte) = (at / 32, at % 32);
                stream.push(match byte {
                    0..8 => tile as u8 + 1,
                    8..16 if tile / 8 % 2 == 1 => 0xFF,
                    _ => 0,
                });
            }
        }
        let mut entry = [0_u8; 3];
        entry[..2].copy_from_slice(&0x8000_u16.to_le_bytes());
        entry[2] = 0x16;
        rom_with(&[
            (d::OBJECT_PORTRAIT_POINTERS + PICTURE_ENTRY, &entry),
            (d::OBJECT_PORTRAIT_PALETTES + 2, &0xD000_u16.to_le_bytes()),
            (d::OBJECT_PORTRAIT_HALVES, &[halves]),
            (0x16_8000, &stream),
            (0x09_D000, &[0, 0, 0x1F, 0]),
        ])
    }

    #[test]
    fn an_object_shows_the_half_of_its_set_that_its_bit_says() {
        // Object 1: bit 1 of the first byte. Clear: the first half, tiles 0 to 7 and 16 to 23;
        // set: the second, tiles 8 to 15 and 24 to 31 (every pixel's plane 0 is its tile's byte)
        let marker = |portrait: &Portrait, cell: usize| {
            portrait.pixels[(cell / 4) * 8 * 32 + (cell % 4) * 8] >> 1
        };
        let first = object_portrait(RomView::new(&rom_of_a_set(0b00)).unwrap(), 1).unwrap();
        assert_eq!(first.half, 0);
        assert!((0..16).all(|cell| marker(&first, cell) == 0), "no tile of the second half");
        let second = object_portrait(RomView::new(&rom_of_a_set(0b10)).unwrap(), 1).unwrap();
        assert_eq!(second.half, 1);
        assert!((0..16).all(|cell| marker(&second, cell) == 1), "only tiles of the second half");
        // The other objects' bits do not matter
        let other = object_portrait(RomView::new(&rom_of_a_set(0b1111_1101)).unwrap(), 1).unwrap();
        assert_eq!(other.half, 0);
    }

    #[test]
    fn the_characters_are_the_tiles_as_video_memory_holds_them() {
        let portrait = object_portrait(RomView::new(&rom_of_a_set(0)).unwrap(), 1).unwrap();
        assert_eq!(portrait.characters.len(), 16);
        // Tile 0: plane 0 is 1 and plane 1 is 0, in each row's word (the low byte is plane 0);
        // planes 2 and 3 are 0
        assert_eq!(portrait.characters[0], [1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0]);
        // Cell 4 is tile 16, whose byte is 17
        assert_eq!(portrait.characters[4][3], 17);
    }
}
