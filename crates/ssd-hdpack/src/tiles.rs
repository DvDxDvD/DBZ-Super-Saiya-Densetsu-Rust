//! The pictures of the HD base layer, built from the tiles of the ROM: each tile's palette indexes
//! are enlarged by the pixel-art filter (Scale2x, twice: 4 x), with integer operations only, so
//! the same ROM gives the same bytes on any machine. The template sheets show the same tiles on a
//! labelled grid for art production.

/// The scale of the base layer: 8 x 8 tiles become 32 x 32 pixels.
pub const SCALE: usize = 4;

/// Scale2x of an image of palette indexes, `width x height` in, `2 width x 2 height` out. A
/// neighbour outside the image counts as the pixel itself, so the edges add nothing.
#[must_use]
pub fn scale2x(source: &[u8], width: usize, height: usize) -> Vec<u8> {
    let mut out = vec![0_u8; 4 * width * height];
    for y in 0..height {
        for x in 0..width {
            let at = |dx: isize, dy: isize| {
                let (xx, yy) = (x as isize + dx, y as isize + dy);
                if xx < 0 || yy < 0 || xx >= width as isize || yy >= height as isize {
                    source[y * width + x]
                } else {
                    source[yy as usize * width + xx as usize]
                }
            };
            // The Scale2x neighbourhood: the centre, and the pixels above, to the left, to the
            // right and below it
            let centre = at(0, 0);
            let (up, left, right, down) = (at(0, -1), at(-1, 0), at(1, 0), at(0, 1));
            let (mut top_left, mut top_right) = (centre, centre);
            let (mut bottom_left, mut bottom_right) = (centre, centre);
            if up != down && left != right {
                if left == up {
                    top_left = left;
                }
                if up == right {
                    top_right = right;
                }
                if left == down {
                    bottom_left = left;
                }
                if down == right {
                    bottom_right = right;
                }
            }
            let row = 2 * y * (2 * width);
            out[row + 2 * x] = top_left;
            out[row + 2 * x + 1] = top_right;
            out[row + 2 * width + 2 * x] = bottom_left;
            out[row + 2 * width + 2 * x + 1] = bottom_right;
        }
    }
    out
}

/// The palette indexes of the 8 x 8 tile of `bpp` bits (2 or 4) at `bytes`, in the SNES layout:
/// the first 16 bytes hold bitplanes 0 and 1 (each row as a pair of bytes, plane 0 then plane
/// 1), a 4-bit tile's next 16 bytes hold planes 2 and 3 the same way. Returned row by row, the
/// leftmost pixel first.
///
/// # Panics
///
/// `bytes` is shorter than the tile (16 bytes for 2 bits, 32 for 4).
#[must_use]
pub fn decode_tile(bytes: &[u8], bpp: u8) -> [u8; 64] {
    let mut pixels = [0_u8; 64];
    for row in 0..8 {
        for column in 0..8 {
            let bit = 7 - column;
            let mut index = 0_u8;
            for plane in 0..usize::from(bpp / 2) {
                let low = bytes[16 * plane + 2 * row];
                let high = bytes[16 * plane + 2 * row + 1];
                index |= ((low >> bit) & 1) << (2 * plane);
                index |= ((high >> bit) & 1) << (2 * plane + 1);
            }
            pixels[row * 8 + column] = index;
        }
    }
    pixels
}

/// The 16 greys of the base layer's palette and the sheets: index `i` is `i * 17`. The game's own
/// palettes are not in the pack: indexed art is resolved through the CGRAM row the game uses.
#[must_use]
pub fn grey_palette() -> Vec<[u8; 3]> {
    (0..16).map(|i| [i * 17; 3]).collect()
}

/// The base art of `tiles` (the palette indexes of each 8 x 8 tile), `columns` tiles to a row,
/// as `(width, height, indexes)`: each tile enlarged by [`scale2x`] twice.
///
/// # Panics
///
/// `columns` is zero, or there are no tiles.
#[must_use]
pub fn base_image(tiles: &[[u8; 64]], columns: usize) -> (usize, usize, Vec<u8>) {
    assert!(columns > 0 && !tiles.is_empty(), "a picture of tiles");
    let rows = tiles.len().div_ceil(columns);
    let (width, height) = (columns * 8 * SCALE, rows * 8 * SCALE);
    let mut image = vec![0_u8; width * height];
    for (number, tile) in tiles.iter().enumerate() {
        let art = scale_tile(tile);
        let (left, top) = ((number % columns) * 8 * SCALE, (number / columns) * 8 * SCALE);
        for y in 0..8 * SCALE {
            let start = (top + y) * width + left;
            image[start..start + 8 * SCALE]
                .copy_from_slice(&art[y * 8 * SCALE..(y + 1) * 8 * SCALE]);
        }
    }
    (width, height, image)
}

/// One 8 x 8 tile enlarged to 32 x 32 by two passes of [`scale2x`].
fn scale_tile(tile: &[u8; 64]) -> Vec<u8> {
    let once = scale2x(tile, 8, 8);
    scale2x(&once, 16, 16)
}

/// Cell size of a template sheet: a tile at [`SCALE`] and a one-pixel gap on each side.
const CELL: usize = 8 * SCALE + 2;

/// The palette indexes of the sheets' own colours, after the sixteen greys: the two tones of the
/// checkerboard that stands for index 0, the black box behind a label and the label's yellow.
const SHEET_LIGHT: u8 = 16;
const SHEET_DARK: u8 = 17;
const SHEET_BOX: u8 = 18;
const SHEET_TEXT: u8 = 19;

/// The palette of the template sheets: the sixteen greys of [`grey_palette`], then the two tones
/// of the checkerboard that stands for index 0 (transparent in the game), then the black box and
/// the yellow of the labels.
#[must_use]
pub fn sheet_palette() -> Vec<[u8; 3]> {
    let mut palette = grey_palette();
    palette.extend([[96; 3], [64; 3], [0; 3], [255, 255, 0]]);
    palette
}

/// The template sheet of `tiles`: the tiles at [`SCALE`] on a grid of `columns`, each cell
/// labelled with its tile number in hexadecimal, as palette indexes of [`sheet_palette`]. Index 0
/// (transparent in the game) is a checkerboard, the others are the greys of [`grey_palette`].
///
/// # Panics
///
/// `columns` is zero, or there are no tiles.
#[must_use]
pub fn sheet_indexed(tiles: &[[u8; 64]], columns: usize) -> (usize, usize, Vec<u8>) {
    assert!(columns > 0 && !tiles.is_empty(), "a sheet of tiles");
    let rows = tiles.len().div_ceil(columns);
    let (width, height) = (columns * CELL, rows * CELL);
    let mut image = vec![0_u8; width * height];
    let mut put = |x: usize, y: usize, value: u8| image[y * width + x] = value;
    for (number, tile) in tiles.iter().enumerate() {
        let (left, top) = ((number % columns) * CELL + 1, (number / columns) * CELL + 1);
        let art = scale_tile(tile);
        for y in 0..8 * SCALE {
            for x in 0..8 * SCALE {
                let index = art[y * 8 * SCALE + x];
                let value = if index == 0 {
                    if ((x / 4) + (y / 4)) % 2 == 0 { SHEET_LIGHT } else { SHEET_DARK }
                } else {
                    index
                };
                put(left + x, top + y, value);
            }
        }
        draw_label(&mut put, left, top, number, SHEET_BOX, SHEET_TEXT);
    }
    (width, height, image)
}

/// A 3 x 5 glyph per hexadecimal digit, one bit per pixel, the leftmost in bit 2.
const GLYPHS: [[u8; 5]; 16] = [
    [0b111, 0b101, 0b101, 0b101, 0b111],
    [0b010, 0b110, 0b010, 0b010, 0b111],
    [0b111, 0b001, 0b111, 0b100, 0b111],
    [0b111, 0b001, 0b111, 0b001, 0b111],
    [0b101, 0b101, 0b111, 0b001, 0b001],
    [0b111, 0b100, 0b111, 0b001, 0b111],
    [0b111, 0b100, 0b111, 0b101, 0b111],
    [0b111, 0b001, 0b010, 0b010, 0b010],
    [0b111, 0b101, 0b111, 0b101, 0b111],
    [0b111, 0b101, 0b111, 0b001, 0b111],
    [0b010, 0b101, 0b111, 0b101, 0b101],
    [0b110, 0b101, 0b110, 0b101, 0b110],
    [0b011, 0b100, 0b100, 0b100, 0b011],
    [0b110, 0b101, 0b101, 0b101, 0b110],
    [0b111, 0b100, 0b110, 0b100, 0b111],
    [0b111, 0b100, 0b110, 0b100, 0b100],
];

/// The number `number` in hexadecimal (at least two digits), in a black box at the top left of
/// a cell.
fn draw_label(
    put: &mut impl FnMut(usize, usize, u8),
    left: usize,
    top: usize,
    number: usize,
    box_index: u8,
    text_index: u8,
) {
    let digits = if number > 0xFF { 3 } else { 2 };
    let text: Vec<usize> = (0..digits).rev().map(|shift| (number >> (4 * shift)) & 0xF).collect();
    for dy in 0..6 {
        for dx in 0..=(4 * digits) {
            put(left + dx, top + dy, box_index);
        }
    }
    for (position, &digit) in text.iter().enumerate() {
        for (row, bits) in GLYPHS[digit].iter().enumerate() {
            for column in 0..3 {
                if bits >> (2 - column) & 1 == 1 {
                    put(left + 1 + 4 * position + column, top + 1 + row, text_index);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale2x_keeps_a_flat_area_and_rounds_a_corner() {
        // A 2 x 2 image of one index: nothing changes, and every output pixel is that index
        assert_eq!(scale2x(&[3, 3, 3, 3], 2, 2), vec![3; 16]);
        // A corner: pixel (1, 1) is 0, with 1 above and 1 to its left. Its top left output pixel
        // takes the colour the two neighbours agree on; the other three stay 0
        let out = scale2x(&[1, 1, 0, 1, 0, 0, 0, 0, 0], 3, 3);
        assert_eq!(out.len(), 36);
        assert_eq!(out[2 * 6 + 2], 1, "the top left of pixel (1, 1)");
        assert_eq!(out[2 * 6 + 3], 0, "its top right");
        assert_eq!(out[3 * 6 + 2], 0, "its bottom left");
    }

    #[test]
    fn a_tile_decodes_by_its_planes_leftmost_pixel_first() {
        let mut bytes = [0_u8; 32];
        // Row 0, plane 0: the leftmost pixel only; plane 3: the rightmost pixel only
        bytes[0] = 0b1000_0000;
        bytes[16 + 1] = 0b0000_0001;
        let pixels = decode_tile(&bytes, 4);
        assert_eq!(pixels[0], 1);
        assert_eq!(pixels[7], 8);
        assert_eq!(pixels[8..16], [0; 8]);
        let two = decode_tile(&bytes[..16], 2);
        assert_eq!(two[0], 1);
        assert_eq!(two[7], 0);
    }

    #[test]
    fn a_base_image_is_the_tiles_enlarged_in_a_grid() {
        let mut first = [0_u8; 64];
        first[0] = 5;
        let (width, height, image) = base_image(&[first, [2; 64]], 2);
        assert_eq!((width, height), (64, 32));
        assert_eq!(image[0], 5, "the first tile's corner");
        assert_eq!(image[32], 2, "the second tile starts at column 32");
        assert_eq!(image.len(), width * height);
    }

    #[test]
    fn the_sheet_labels_each_cell_and_is_the_same_every_time() {
        let tiles = [[1_u8; 64]; 17];
        let first = sheet_indexed(&tiles, 16);
        assert_eq!((first.0, first.1), (16 * CELL, 2 * CELL));
        assert_eq!(first, sheet_indexed(&tiles, 16));
        assert_eq!(first.2.len(), first.0 * first.1);
        // Every index is in the palette, and a label is drawn (the box and the text are there)
        assert!(first.2.iter().all(|&index| usize::from(index) < sheet_palette().len()));
        assert!(first.2.contains(&18) && first.2.contains(&19));
    }
}
