//! The check of the portraits and faces a frame shows: each one must be drawn from the art of ONE
//! character, whole. A portrait is a 4 x 4 picture of characters; the pixels that show a cell of a
//! picture say which picture, and where its cell lies, so the characters that belong to one
//! portrait on screen are found by the place the picture's origin would have. A portrait is
//! faulty when its cells come from more than one picture or asset (the half of one character, the
//! half of another), when the characters it shows are bound to more than one asset or to another
//! asset than the picture's, or when some of its characters show the original's pixels beside the
//! cells the art draws.

use super::{ArtPixel, ArtState, NO_ASSET, NO_PICTURE, Shown};
use rustc_hash::FxHashMap;

/// The width and height of a portrait in pixels.
const SIDE: i32 = 32;

/// The fewest pixels of a portrait the check looks at (a window that is only starting to show one
/// is not a portrait yet).
const MIN_PIXELS: u32 = 32;

/// The most the colours of a portrait's row may be from the colours its art was made in (summed
/// channel differences over the palette): the art of one character recoloured to the palette of
/// another is a different face. The palettes of one face seen twice differ by under ten.
const RECOLOUR_MAX: u32 = 150;

/// One portrait or face the frame shows, with where its parts come from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PortraitUse {
    /// The frame position of the picture's top left corner.
    pub x: i32,
    pub y: i32,
    /// Pixels drawn from pictures, and the cells (a bit each) that have any.
    pub pixels: u32,
    pub cells: u16,
    /// The names of the pictures the cells come from, with their pixels.
    pub pictures: Vec<(String, u32)>,
    /// The ids of the assets of those pictures.
    pub picture_assets: Vec<String>,
    /// The ids of the assets the characters of the pixels are bound to. More than the pictures'
    /// assets, or others: the game's VRAM held the tiles of two faces in this portrait and it was
    /// drawn whole from one picture (see [`ArtState::repair_portraits`]).
    pub bound_assets: Vec<String>,
    /// The CGRAM rows (see [`super::ArtPixel::row`]) the pixels are in, with their pixels: one
    /// portrait is drawn in one row.
    pub rows: Vec<(u8, u32)>,
    /// How far the colours the art was made in are from the colours of the portrait's row in
    /// this frame (the summed channel differences over the colours the art uses; 0 when the art is
    /// drawn as it was made): the art is recoloured by that much.
    pub distance: u32,
    /// Pixels of the box, of the layer and palette of the portrait, that no picture draws: the
    /// original's own pixels.
    pub bare: u32,
}

impl PortraitUse {
    /// What is wrong with the portrait, if anything.
    #[must_use]
    pub fn fault(&self) -> Option<String> {
        if self.pictures.len() > 1 || self.picture_assets.len() > 1 {
            let parts: Vec<String> =
                self.pictures.iter().map(|(name, pixels)| format!("{name} {pixels}px")).collect();
            return Some(format!("mixed art: {}", parts.join(" + ")));
        }
        if self.rows.len() > 1 {
            let parts: Vec<String> =
                self.rows.iter().map(|(row, pixels)| format!("row {row} {pixels}px")).collect();
            return Some(format!("palettes of more than one row: {}", parts.join(" + ")));
        }
        if self.bound_assets != self.picture_assets {
            return Some(format!(
                "the game's characters are of {} but the portrait is drawn from {}",
                self.bound_assets.join(" + "),
                self.picture_assets.join(" + ")
            ));
        }
        if self.distance > RECOLOUR_MAX {
            return Some(format!(
                "{} drawn in colours {} away from the ones its art was made in",
                self.picture_assets.join(" + "),
                self.distance
            ));
        }
        (self.bare > 0).then(|| {
            format!("{} px of the original beside {}", self.bare, self.picture_assets.join(" + "))
        })
    }

    /// A name for the portrait in reports.
    #[must_use]
    pub fn name(&self) -> String {
        self.picture_assets.first().cloned().unwrap_or_else(|| "none".to_owned())
    }
}

/// The frame position of the top left corner of the 4 x 4 picture that the pixel `pixel`, at index
/// `at` of a frame `width` pixels wide, shows a cell of.
fn picture_origin(at: usize, width: i32, pixel: &ArtPixel) -> (i32, i32) {
    let (x, y) = (at as i32 % width, at as i32 / width);
    let (col, row) = (i32::from(pixel.pos & 7), i32::from((pixel.pos >> 3) & 7));
    let (x_flip, y_flip) = (pixel.pos & 0x40 != 0, pixel.pos & 0x80 != 0);
    let (cell_x, cell_y) = (i32::from(pixel.cell % 4), i32::from(pixel.cell / 4));
    (
        x - (if x_flip { 7 - col } else { col }) - cell_x * 8,
        y - (if y_flip { 7 - row } else { row }) - cell_y * 8,
    )
}

/// How far the colours `row` shows are from the palette the art was made in, after the palette is
/// scaled to the row's brightness: a fade (the game lowers or raises every colour of a row) is
/// no other character's palette.
fn shaded_distance(palette: &crate::art_index::Palette, row: &[[u8; 3]; 16]) -> u32 {
    let used = |index: &usize| palette.mask & (1 << index) != 0;
    let sum = |colours: &[[u8; 3]; 16]| -> u32 {
        (1..16).filter(used).map(|i| colours[i].iter().map(|&c| u32::from(c)).sum::<u32>()).sum()
    };
    let (made, shown) = (sum(&palette.colors), sum(row));
    if made == 0 || shown == 0 {
        return palette.distance(row);
    }
    let mut scaled = palette.colors;
    for colour in &mut scaled {
        for channel in colour {
            *channel = (u32::from(*channel) * shown / made).min(255) as u8;
        }
    }
    crate::art_index::Palette { mask: palette.mask, colors: scaled }.distance(row)
}

/// A picture some cells of a portrait come from: its pixels, the CGRAM row they are in, and the
/// sum of the cell rows (0 to 3) of those pixels, which says how low in the portrait they are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Share {
    picture: u32,
    row: u8,
    pixels: u32,
    depth: u32,
}

/// The share a portrait whose cells come from several pictures is drawn whole from: `distance` is
/// how far a share's palette is from the colours the game shows its pixels in. One that has over
/// two thirds of the pixels is the portrait; else the one whose colours are nearest the game's,
/// then the one that holds the lower cells (the face's lower rows are the ones the game writes
/// last), then the lower index.
fn whole_share(shares: &[Share], distance: impl Fn(&Share) -> u32) -> Share {
    let total: u32 = shares.iter().map(|share| share.pixels).sum();
    if let Some(share) = shares.iter().find(|share| share.pixels * 3 > total * 2) {
        return *share;
    }
    *shares
        .iter()
        .min_by_key(|share| {
            // (the mean cell row, in thousandths, larger first)
            let depth = share.depth * 1000 / share.pixels.max(1);
            (distance(share), std::cmp::Reverse(depth), share.picture)
        })
        .expect("a portrait has a picture")
}

impl ArtState {
    /// Draws each portrait whole from one picture, in one palette row. The game's VRAM can hold the
    /// tiles of two faces in one portrait (the top rows of the one before, the bottom rows of the
    /// speaker); each tile then binds to its own face's art and the portrait is half one character,
    /// half another, in two palettes. All the cells are taken from the picture [`whole_share`]
    /// names, each in the cell it already is, in the row of the cells that picture had. Returns how
    /// many portraits were put right.
    pub(crate) fn repair_portraits(&mut self) -> usize {
        let width = self.width as i32;
        if width == 0 {
            return 0;
        }
        let mut groups: FxHashMap<(i32, i32), Vec<usize>> = FxHashMap::default();
        for (at, pixel) in self.pixels.iter().enumerate() {
            if pixel.picture != NO_PICTURE {
                groups.entry(picture_origin(at, width, pixel)).or_default().push(at);
            }
        }
        let pictures = self.pack.art.pictures();
        let mut repaired = 0;
        for members in groups.into_values() {
            let mut shares: Vec<Share> = Vec::new();
            for &at in &members {
                let pixel = &self.pixels[at];
                let depth = u32::from(pixel.cell / 4);
                match shares.iter_mut().find(|share| share.picture == pixel.picture) {
                    Some(share) => {
                        share.pixels += 1;
                        share.depth += depth;
                    }
                    None => shares.push(Share {
                        picture: pixel.picture,
                        row: pixel.row,
                        pixels: 1,
                        depth,
                    }),
                }
            }
            if shares.len() < 2 {
                continue;
            }
            shares.sort_unstable_by_key(|share| share.picture);
            let distance = |share: &Share| {
                let base = usize::from(share.row) * 16;
                let palette = &pictures[share.picture as usize].palette;
                match self.cgram.get(base..base + 16) {
                    Some(words) if palette.mask != 0 => {
                        let colours: [[u8; 3]; 16] =
                            std::array::from_fn(|n| super::poses::rgb555(words[n]));
                        palette.distance(&colours)
                    }
                    _ => u32::MAX,
                }
            };
            let whole = whole_share(&shares, distance);
            for at in members {
                self.pixels[at].picture = whole.picture;
                self.pixels[at].row = whole.row;
            }
            repaired += 1;
        }
        repaired
    }

    /// The portraits of the frame, found in the record of its pixels (the tally is on).
    pub(crate) fn portrait_uses(&self) -> Vec<PortraitUse> {
        let pictures = self.pack.art.pictures();
        let width = self.width as i32;
        // The pixels of each picture origin
        let mut groups: FxHashMap<(i32, i32), PortraitUse> = FxHashMap::default();
        let mut by_picture: FxHashMap<(i32, i32), FxHashMap<u32, u32>> = FxHashMap::default();
        let mut rows: FxHashMap<(i32, i32), u8> = FxHashMap::default();
        for (at, pixel) in self.pixels.iter().enumerate() {
            if pixel.picture == NO_PICTURE {
                continue;
            }
            let origin = picture_origin(at, width, pixel);
            let group = groups.entry(origin).or_insert_with(|| PortraitUse {
                x: origin.0,
                y: origin.1,
                ..PortraitUse::default()
            });
            group.pixels += 1;
            match group.rows.iter_mut().find(|(row, _)| *row == pixel.row) {
                Some((_, pixels)) => *pixels += 1,
                None => group.rows.push((pixel.row, 1)),
            }
            if pixel.shown == Shown::Bg {
                rows.insert(origin, pixel.row);
            }
            group.cells |= 1 << pixel.cell;
            *by_picture.entry(origin).or_default().entry(pixel.picture).or_default() += 1;
            let asset = &pictures[pixel.picture as usize].asset;
            if !group.picture_assets.contains(asset) {
                group.picture_assets.push(asset.clone());
            }
            if pixel.asset != NO_ASSET {
                let bound = self.pack.index.asset(pixel.asset).id.clone();
                if !group.bound_assets.contains(&bound) {
                    group.bound_assets.push(bound);
                }
            }
        }
        let mut found: Vec<PortraitUse> = Vec::new();
        for (origin, mut group) in groups {
            if group.pixels < MIN_PIXELS {
                continue;
            }
            if let Some(held) = by_picture.get(&origin) {
                let mut held: Vec<(u32, u32)> = held.iter().map(|(&k, &v)| (k, v)).collect();
                held.sort_unstable();
                group.pictures = held
                    .into_iter()
                    .map(|(at, pixels)| (pictures[at as usize].name.clone(), pixels))
                    .collect();
            }
            // The distance of the art's palette from the game's colours of the row
            let row = group.rows.first().map_or(0, |&(row, _)| usize::from(row));
            if let Some(words) = self.cgram.get(row * 16..row * 16 + 16) {
                let colours: [[u8; 3]; 16] =
                    std::array::from_fn(|n| super::poses::rgb555(words[n]));
                for (at, _) in by_picture.get(&origin).into_iter().flatten() {
                    let palette = &pictures[*at as usize].palette;
                    group.distance = group.distance.max(shaded_distance(palette, &colours));
                }
            }
            group.rows.sort_unstable();
            group.picture_assets.sort();
            group.bound_assets.sort();
            // The characters of the box the art does not draw: of the layer and palette of the
            // portrait, bound or not
            if let Some(&row) = rows.get(&origin) {
                for y in origin.1.max(0)..(origin.1 + SIDE).min(self.height as i32) {
                    for x in origin.0.max(0)..(origin.0 + SIDE).min(width) {
                        let pixel = &self.pixels[(y * width + x) as usize];
                        if pixel.picture == NO_PICTURE
                            && pixel.shown == Shown::Bg
                            && pixel.bpp == 4
                            && pixel.bg4
                            && pixel.bg_palette == row
                        {
                            group.bare += 1;
                        }
                    }
                }
            }
            found.push(group);
        }
        found.sort_by_key(|p| (p.y, p.x));
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn whole(name: &str, asset: &str) -> PortraitUse {
        PortraitUse {
            pixels: 1024,
            cells: 0xFFFF,
            pictures: vec![(name.to_owned(), 1024)],
            picture_assets: vec![asset.to_owned()],
            bound_assets: vec![asset.to_owned()],
            rows: vec![(11, 1024)],
            ..PortraitUse::default()
        }
    }

    #[test]
    fn a_portrait_of_one_picture_in_one_row_is_whole() {
        assert_eq!(whole("face-a", "portrait/16:8A9B/4bpp").fault(), None);
    }

    #[test]
    fn two_pictures_in_one_portrait_are_a_fault_even_of_one_stream() {
        let mut portrait = whole("face-a", "portrait/16:9C00/4bpp");
        portrait.pictures = vec![("obj-30".to_owned(), 512), ("face-9C00-0v".to_owned(), 512)];
        let fault = portrait.fault().unwrap();
        assert!(fault.contains("mixed art") && fault.contains("obj-30"), "{fault}");
    }

    #[test]
    fn two_assets_in_one_portrait_are_a_fault() {
        let mut portrait = whole("face-a", "portrait/16:A780/4bpp");
        portrait.picture_assets.push("portrait/16:8A9B/4bpp".to_owned());
        portrait.bound_assets.push("portrait/16:8A9B/4bpp".to_owned());
        assert!(portrait.fault().unwrap().contains("mixed art"));
    }

    #[test]
    fn characters_of_another_stream_than_the_picture_are_a_fault() {
        let mut portrait = whole("face-a", "portrait/16:A780/4bpp");
        portrait.bound_assets =
            vec!["portrait/16:8A9B/4bpp".to_owned(), "portrait/16:A780/4bpp".to_owned()];
        assert!(portrait.fault().unwrap().contains("characters are of"));
    }

    #[test]
    fn art_recoloured_far_from_its_palette_is_a_fault_and_a_near_one_is_not() {
        let mut portrait = whole("face-a", "portrait/16:8A9B/4bpp");
        portrait.distance = 9;
        assert_eq!(portrait.fault(), None);
        portrait.distance = 2847;
        assert!(portrait.fault().unwrap().contains("2847"));
    }

    #[test]
    fn a_fade_is_not_another_palette() {
        let mut palette = crate::art_index::Palette { mask: 0b110, ..Default::default() };
        palette.colors[1] = [200, 100, 50];
        palette.colors[2] = [100, 200, 40];
        let mut row = [[0_u8; 3]; 16];
        // The same colours at half brightness
        row[1] = [100, 50, 25];
        row[2] = [50, 100, 20];
        assert!(shaded_distance(&palette, &row) < 10);
        // Another palette of the same total brightness
        row[1] = [50, 100, 25];
        row[2] = [100, 50, 20];
        assert!(shaded_distance(&palette, &row) > 150);
    }

    fn share(picture: u32, pixels: u32, depth: u32) -> Share {
        Share { picture, row: 11, pixels, depth }
    }

    #[test]
    fn the_picture_with_most_of_a_portrait_is_the_portrait_and_a_tie_goes_to_palette_then_depth() {
        let near = |at: u32| move |share: &Share| if share.picture == at { 5 } else { 900 };
        let whole = |shares: &[Share], d: &dyn Fn(&Share) -> u32| whole_share(shares, d).picture;
        assert_eq!(whole(&[share(3, 20, 0), share(7, 4, 9)], &|_| 0), 3);
        assert_eq!(whole(&[share(3, 8, 0), share(7, 8, 0)], &near(7)), 7, "the nearer colours");
        assert_eq!(whole(&[share(3, 9, 0), share(7, 7, 0)], &near(7)), 7, "no two thirds");
        assert_eq!(whole(&[share(3, 8, 0), share(7, 8, 16)], &|_| 0), 7, "the lower cells");
        assert_eq!(whole(&[share(3, 8, 8), share(7, 8, 8)], &|_| 0), 3, "the lower index");
    }

    #[test]
    fn a_portrait_in_two_palette_rows_is_a_fault() {
        let mut portrait = whole("face-a", "portrait/16:8A9B/4bpp");
        portrait.rows = vec![(11, 512), (12, 512)];
        assert!(portrait.fault().unwrap().contains("more than one row"));
    }

    #[test]
    fn original_pixels_beside_the_art_are_a_fault() {
        let mut portrait = whole("face-a", "portrait/16:8A9B/4bpp");
        portrait.bare = 256;
        assert!(portrait.fault().unwrap().contains("original"));
    }
}
