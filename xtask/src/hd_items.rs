//! `hd-art items`: the picture of every object (the item ids) as the original draws it and as the
//! HD Items menu draws it, for checking that no picture is drawn from another picture's art.
//!
//! For each of the 40 objects it writes `orig-NN.png` (the original's pixels of the object's own
//! half of its set, in its palette, 4x) and `menu-NN.png` (what the HD Items menu draws: the pack's
//! art for exactly those characters, or the original's pixels when the pack has none), and prints
//! one line: the object, its name, the half and palette it uses, the art found (name, how far its
//! palette is, whether it was recoloured) or `original`. Everything it writes is derived from the
//! game: `.work/` only, never tracked.

use anyhow::Context;
use ssd_catalogue::{Catalogue, RomView, object_portrait};
use ssd_hd::pack::Pack;
use std::io::Write;
use std::path::Path;

/// The colours of a picture's palette as 8-bit RGB (the menus' own widening).
fn row_of(palette: &[u16; 16]) -> [[u8; 3]; 16] {
    std::array::from_fn(|index| {
        let channel = |shift: u16| {
            let value = ((palette[index] >> shift) & 0x1F) as u8;
            (value << 3) | (value >> 2)
        };
        [channel(0), channel(5), channel(10)]
    })
}

/// Writes the pictures and prints the report; returns the number of objects with no art.
///
/// # Errors
///
/// The ROM, the pack or a file cannot be read or written.
pub fn run(rom: &[u8], pack_dir: &Path, out: &Path, w: &mut dyn Write) -> anyhow::Result<u8> {
    let pack = Pack::open(pack_dir).with_context(|| format!("opening {}", pack_dir.display()))?;
    let catalogue = Catalogue::from_rom(rom)?;
    let view = RomView::new(rom)?;
    std::fs::create_dir_all(out)?;
    let mut without = 0_u8;
    for object in 0..40_u8 {
        let portrait = object_portrait(view, object)?;
        let row = row_of(&portrait.palette);
        let mut original = vec![0_u8; 32 * 32 * 4];
        for (n, &index) in portrait.pixels.iter().enumerate() {
            let [r, g, b] = row[usize::from(index)];
            original[n * 4..n * 4 + 4].copy_from_slice(&if index == 0 {
                [0, 0, 0, 0]
            } else {
                [r, g, b, 255]
            });
        }
        write_png(&out.join(format!("orig-{object:02}.png")), 32, 32, &original, 4)?;
        let name = &catalogue.items[usize::from(object)].name.text;
        let found = pack.pictures_of_characters(&portrait.characters, &row);
        let (drawn, said) = match pack.menu_picture(&portrait.characters, &row) {
            Some(picture) => {
                let distance = found.first().map_or(0, |&(_, distance)| distance);
                let said = format!(
                    "art {} (palette distance {distance}{})",
                    picture.name,
                    if picture.recoloured { ", recoloured" } else { "" }
                );
                write_png(
                    &out.join(format!("menu-{object:02}.png")),
                    picture.width,
                    picture.height,
                    &picture.rgba,
                    1,
                )?;
                (true, said)
            }
            None => {
                write_png(&out.join(format!("menu-{object:02}.png")), 32, 32, &original, 4)?;
                without += 1;
                (false, "original".to_owned())
            }
        };
        let _ = drawn;
        writeln!(
            w,
            "{object:2} {name:<9} half {} palette {:04X}: {said}",
            portrait.half,
            row_checksum(&portrait.palette)
        )?;
    }
    writeln!(w, "{without} objects with no art")?;
    Ok(0)
}

/// A short label of a palette (its first colours folded), to tell two palettes apart in the report.
fn row_checksum(palette: &[u16; 16]) -> u16 {
    palette.iter().fold(0_u16, |sum, &colour| sum.rotate_left(3) ^ colour)
}

fn write_png(
    path: &Path,
    width: usize,
    height: usize,
    rgba: &[u8],
    scale: usize,
) -> anyhow::Result<()> {
    let mut big = Vec::with_capacity(width * scale * height * scale * 4);
    for y in 0..height * scale {
        for x in 0..width * scale {
            let at = ((y / scale) * width + x / scale) * 4;
            big.extend_from_slice(&rgba[at..at + 4]);
        }
    }
    std::fs::write(
        path,
        crate::shots::encode_png((width * scale) as u32, (height * scale) as u32, &big),
    )?;
    Ok(())
}
