//! The thumbnail sheets of `hd-tour`: a crop of the frame around each thing shown without art,
//! laid out on a grid with a number under each (the item's `sheet` cell in `coverage.json`).
//! Everything here is derived from the game and goes under `.work/tour/` only.

use ssd_hd::png;
use std::path::Path;

/// The biggest crop kept, in pixels of the frame.
pub const MAX_CROP: u32 = 80;

/// A piece of a frame, RGBA.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Crop {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// The part of the RGBA picture `rgba` (`width` x `height`) around `bbox` (`x0, y0, x1, y1`,
/// inclusive), with a margin of two pixels, at most [`MAX_CROP`] on a side (centred on the box when
/// it is bigger). `None` when the box is outside the picture.
#[must_use]
pub fn crop(rgba: &[u8], width: u32, height: u32, bbox: (u16, u16, u16, u16)) -> Option<Crop> {
    let (x0, y0, x1, y1) = (
        i64::from(bbox.0) - 2,
        i64::from(bbox.1) - 2,
        i64::from(bbox.2) + 3,
        i64::from(bbox.3) + 3,
    );
    let fit = |low: i64, high: i64, limit: u32| {
        let (mut low, mut high) = (low.max(0), high.min(i64::from(limit)));
        if high - low > i64::from(MAX_CROP) {
            let middle = i64::midpoint(low, high);
            low = middle - i64::from(MAX_CROP) / 2;
            high = low + i64::from(MAX_CROP);
        }
        (low, high)
    };
    let (x0, x1) = fit(x0, x1, width);
    let (y0, y1) = fit(y0, y1, height);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let (w, h) = ((x1 - x0) as u32, (y1 - y0) as u32);
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in y0..y1 {
        let from = ((y as u32 * width + x0 as u32) * 4) as usize;
        out.extend_from_slice(rgba.get(from..from + (w * 4) as usize)?);
    }
    Some(Crop { width: w, height: h, rgba: out })
}

/// The side of a cell of a sheet: the biggest crop twice as big would not fit, so crops under
/// half of it are shown at 2 x.
const CELL: u32 = MAX_CROP + 4;
/// The columns of a sheet.
pub const COLUMNS: u32 = 10;
/// The height of the number under each cell.
const LABEL: u32 = 8;

/// The 3 x 5 digits of the cell numbers, one row of three bits per line.
const DIGITS: [[u8; 5]; 10] = [
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
];

fn put(sheet: &mut [u8], width: u32, x: u32, y: u32, color: [u8; 3]) {
    let at = ((y * width + x) * 4) as usize;
    if let Some(pixel) = sheet.get_mut(at..at + 4) {
        pixel[..3].copy_from_slice(&color);
        pixel[3] = 255;
    }
}

fn number(sheet: &mut [u8], width: u32, x: u32, y: u32, value: usize) {
    for (place, digit) in value.to_string().bytes().enumerate() {
        let glyph = DIGITS[usize::from(digit - b'0')];
        for (row, bits) in glyph.iter().enumerate() {
            for column in 0..3_u32 {
                if bits >> (2 - column) & 1 != 0 {
                    put(
                        sheet,
                        width,
                        x + place as u32 * 4 + column,
                        y + row as u32,
                        [255, 255, 255],
                    );
                }
            }
        }
    }
}

/// The sheet of `crops` as RGBA: `COLUMNS` cells to a row, each crop on a grey cell (twice as big
/// when it is small), numbered from `first` under it. Returns the width, height and pixels.
#[must_use]
pub fn sheet(crops: &[Crop], first: usize) -> (u32, u32, Vec<u8>) {
    let rows = (crops.len() as u32).div_ceil(COLUMNS).max(1);
    let (width, height) = (COLUMNS * CELL, rows * (CELL + LABEL));
    let mut out = vec![0_u8; (width * height * 4) as usize];
    for pixel in out.as_chunks_mut::<4>().0 {
        *pixel = [40, 40, 48, 255];
    }
    for (at, crop) in crops.iter().enumerate() {
        let (column, row) = (at as u32 % COLUMNS, at as u32 / COLUMNS);
        let (left, top) = (column * CELL, row * (CELL + LABEL));
        let zoom = if crop.width <= MAX_CROP / 2 && crop.height <= MAX_CROP / 2 { 2 } else { 1 };
        for y in 0..crop.height * zoom {
            for x in 0..crop.width * zoom {
                let from = (((y / zoom) * crop.width + x / zoom) * 4) as usize;
                let Some(source) = crop.rgba.get(from..from + 3) else { continue };
                put(&mut out, width, left + 2 + x, top + 2 + y, [source[0], source[1], source[2]]);
            }
        }
        number(&mut out, width, left + 2, top + CELL, first + at);
    }
    (width, height, out)
}

/// Writes the sheet of `crops` to `path` as a PNG.
///
/// # Errors
///
/// The file cannot be written.
pub fn write_sheet(path: &Path, crops: &[Crop], first: usize) -> std::io::Result<()> {
    let (width, height, rgba) = sheet(crops, first);
    png::write_rgba(path, width, height, &rgba)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(width: u32, height: u32) -> Vec<u8> {
        (0..width * height).flat_map(|n| [(n % 251) as u8, 0, 0, 255]).collect()
    }

    #[test]
    fn a_crop_takes_the_box_with_a_margin() {
        let rgba = picture(40, 30);
        let c = crop(&rgba, 40, 30, (10, 10, 13, 12)).unwrap();
        assert_eq!((c.width, c.height), (4 + 4, 3 + 4));
        assert_eq!(c.rgba.len(), (c.width * c.height * 4) as usize);
        // The first pixel is the picture's (8, 8)
        assert_eq!(c.rgba[0], ((8 * 40 + 8) % 251) as u8);
    }

    #[test]
    fn a_crop_stays_inside_the_picture_and_within_the_limit() {
        let rgba = picture(200, 150);
        let edge = crop(&rgba, 200, 150, (0, 0, 1, 1)).unwrap();
        assert_eq!((edge.width, edge.height), (4, 4));
        let big = crop(&rgba, 200, 150, (0, 0, 199, 149)).unwrap();
        assert_eq!((big.width, big.height), (MAX_CROP, MAX_CROP));
        assert!(crop(&rgba, 200, 150, (300, 300, 310, 310)).is_none());
    }

    #[test]
    fn a_sheet_has_a_cell_and_a_number_for_each_crop() {
        let small = Crop { width: 8, height: 8, rgba: vec![200; 8 * 8 * 4] };
        let (width, height, rgba) = sheet(&[small.clone(), small.clone(), small], 5);
        assert_eq!((width, height), (COLUMNS * CELL, CELL + LABEL));
        assert_eq!(rgba.len(), (width * height * 4) as usize);
        // The crop is drawn twice as big: pixel (2, 2) of the cell and pixel (17, 17) are its edges
        let at = |x: u32, y: u32| rgba[((y * width + x) * 4) as usize];
        assert_eq!((at(2, 2), at(17, 17), at(18, 18)), (200, 200, 40));
        // The number is drawn under the cell
        let lit = (CELL..CELL + LABEL)
            .flat_map(|y| (0..CELL).map(move |x| (x, y)))
            .filter(|&(x, y)| at(x, y) == 255)
            .count();
        assert!(lit > 0);
    }
}
