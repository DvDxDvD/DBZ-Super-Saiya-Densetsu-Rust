//! HD Mode 7: the rotation and scaling plane (the world map the flights fly over) drawn again from
//! the approved art.
//!
//! The plane is 128 x 128 tiles, 1024 x 1024 game pixels, made of the tile map (the low bytes of
//! the first 16384 words of VRAM) and the 8-bit characters (their high bytes). The art is one
//! picture of the whole plane at [`SCALE`] art pixels to a game pixel, found by the content of that
//! VRAM ([`plane_key`]) and drawn through the same matrix the game gives each line (HDMA changes it
//! from line to line: the perspective of a flight), sampled bilinearly between the art's pixels. The
//! art is made in the colours of one palette (`palette`, 256 colours): a game colour that is not
//! the one the art was made for (a fade, a flash) changes the art as it changed the game's pixel
//! ([`crate::bgart::adapt`]).
//!
//! A plane that is far away on a line (the horizon of a flight) is sampled from a smaller copy of
//! the art, as a texture is: the copies are halved with a box filter and the two nearest are
//! blended, so the horizon does not shimmer. Everything is integer arithmetic: the picture does
//! not depend on the machine.

use std::path::Path;

/// The art's index in the pack: `<pack>/art/mode7/mode7.toml`.
pub const INDEX: &str = "art/mode7/mode7.toml";

/// Art pixels along a game pixel's edge in the art files.
pub const SCALE: usize = 4;

/// Game pixels along a tile of the art's edge.
pub const TILE: usize = 256;

/// The plane's size in game pixels.
pub const PLANE: usize = 1024;

/// The words of VRAM that make the plane: the tile map and the characters.
const PLANE_WORDS: usize = 0x4000;

/// Why the plane's art cannot be read.
#[derive(Debug, thiserror::Error)]
pub enum Mode7Error {
    #[error("mode 7: {0}")]
    Read(String),
}

/// `mode7.toml`: the planes with art.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Index {
    #[serde(default)]
    pub plane: Vec<Entry>,
}

/// The art of one plane.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Entry {
    /// The name of the art (its folder), for reports.
    pub id: String,
    /// [`plane_key`] of the plane's VRAM, 16 hexadecimal digits.
    pub key: String,
    /// The art tiles (RGBA, `TILE * SCALE` pixels square: a pixel with alpha 0 has no art), relative to the index's folder, with
    /// `{x}` and `{y}` the tile's column and row.
    pub tiles: String,
    /// The 256 colours of the palette the art was made in, `RRGGBB` each (index 0 included).
    pub palette: Vec<String>,
}

impl Entry {
    /// The key as a number.
    #[must_use]
    pub fn key_value(&self) -> u64 {
        u64::from_str_radix(&self.key, 16).unwrap_or(0)
    }

    /// The file of the tile in column `x` and row `y`, relative to the index's folder.
    #[must_use]
    pub fn tile_file(&self, x: usize, y: usize) -> String {
        self.tiles.replace("{x}", &x.to_string()).replace("{y}", &y.to_string())
    }

    /// The tiles along the plane's edge.
    #[must_use]
    pub const fn tiles_across() -> usize {
        PLANE / TILE
    }

    /// The palette as colours, `None` if it is not 256 entries of six hexadecimal digits.
    #[must_use]
    pub fn colours(&self) -> Option<[[u8; 3]; 256]> {
        if self.palette.len() != 256 {
            return None;
        }
        let mut out = [[0_u8; 3]; 256];
        for (colour, text) in out.iter_mut().zip(&self.palette) {
            let value = u32::from_str_radix(text, 16).ok().filter(|_| text.len() == 6)?;
            *colour = [(value >> 16) as u8, (value >> 8) as u8, value as u8];
        }
        Some(out)
    }
}

impl Index {
    /// The index of `<pack>/art/mode7/mode7.toml`; empty when the pack has none.
    ///
    /// # Errors
    ///
    /// The file is not a valid index.
    pub fn open(pack: &Path) -> Result<Self, Mode7Error> {
        let Ok(text) = std::fs::read_to_string(pack.join(INDEX)) else {
            return Ok(Self::default());
        };
        toml::from_str(&text).map_err(|e| Mode7Error::Read(e.to_string()))
    }

    /// The plane whose content is `key`, if the index has art for it.
    #[must_use]
    pub fn find(&self, key: u64) -> Option<usize> {
        self.plane.iter().position(|entry| entry.key_value() == key)
    }
}

/// The key of a plane: a hash (FNV-1a, 64 bits) of the words of VRAM the plane is made of.
#[must_use]
pub fn plane_key(vram: &[u16]) -> u64 {
    let mut hash = 0xCBF2_9CE4_8422_2325_u64;
    for &word in &vram[..PLANE_WORDS.min(vram.len())] {
        for byte in word.to_le_bytes() {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01B3);
        }
    }
    hash
}

/// The art of a plane as a texture: RGBA (premultiplied inside), the art's size and each copy half
/// the one before.
#[derive(Debug)]
pub struct PlaneArt {
    levels: Vec<Level>,
    /// The colours the art was made in.
    pub palette: [[u8; 3]; 256],
}

/// A level of the texture: colours multiplied by their alpha (0 to 255 each), then the alpha, four
/// bytes a pixel.
#[derive(Debug)]
struct Level {
    side: usize,
    premultiplied: Vec<u8>,
}

impl PlaneArt {
    /// A texture of the `side` x `side` RGBA pixels `rgba` (a power of two), with its smaller copies.
    /// A pixel with alpha 0 is where the plane has no art (the game's own pixel shows).
    ///
    /// # Panics
    ///
    /// `side` is not a power of two, or `rgba` is not `side * side * 4` bytes.
    #[must_use]
    pub fn new(side: usize, rgba: &[u8], palette: [[u8; 3]; 256]) -> Self {
        assert!(side.is_power_of_two() && rgba.len() == side * side * 4);
        let mut base = vec![0_u8; rgba.len()];
        for (to, from) in base.as_chunks_mut::<4>().0.iter_mut().zip(rgba.as_chunks::<4>().0) {
            let alpha = u32::from(from[3]);
            for c in 0..3 {
                to[c] = ((u32::from(from[c]) * alpha + 127) / 255) as u8;
            }
            to[3] = from[3];
        }
        let mut levels = vec![Level { side, premultiplied: base }];
        while levels.last().is_some_and(|last| last.side > 1) && levels.len() < 6 {
            let last = levels.last().expect("a level");
            let half = last.side / 2;
            let mut next = vec![0_u8; half * half * 4];
            for y in 0..half {
                for x in 0..half {
                    for c in 0..4 {
                        let at = |dx: usize, dy: usize| {
                            u32::from(
                                last.premultiplied[((2 * y + dy) * last.side + 2 * x + dx) * 4 + c],
                            )
                        };
                        next[(y * half + x) * 4 + c] =
                            ((at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1) + 2) / 4) as u8;
                    }
                }
            }
            levels.push(Level { side: half, premultiplied: next });
        }
        Self { levels, palette }
    }

    /// The number of levels (1 is the art alone).
    #[must_use]
    pub fn levels(&self) -> usize {
        self.levels.len()
    }

    /// The colour and alpha at the plane position `(x, y)` in 1/256 of a game pixel, wrapped around
    /// the plane, for a footprint of `step` art pixels per picture pixel in 1/256 (256 is one: the
    /// art's own size; 512 two, and so on). Bilinear in the level the footprint says, blended with
    /// the next smaller one; the colour is not mixed with the colour of a transparent texel.
    #[must_use]
    pub fn sample(&self, x: i64, y: i64, step: i64) -> [u8; 4] {
        // The level (in 1/256) the footprint is at: log2 of the step, in steps of 1/256
        let step = step.max(256);
        let whole = (63 - i64::from((step as u64).leading_zeros())) - 8;
        let level = (whole.max(0) as usize).min(self.levels.len() - 1);
        let base = 256_i64 << level.min(40);
        // The fraction between this level and the next, by the step's place between the two powers
        let fraction = if level + 1 >= self.levels.len() {
            0
        } else {
            (((step - base) * 256) / base).clamp(0, 255)
        };
        let mut sum = self.bilinear(level, x, y);
        if fraction != 0 {
            let far = self.bilinear(level + 1, x, y);
            for c in 0..4 {
                sum[c] = (sum[c] * (256 - fraction) + far[c] * fraction + 128) >> 8;
            }
        }
        let alpha = sum[3];
        if alpha == 0 {
            return [0; 4];
        }
        let un = |v: i64| ((v * 255 + alpha / 2) / alpha).clamp(0, 255) as u8;
        [un(sum[0]), un(sum[1]), un(sum[2]), alpha.clamp(0, 255) as u8]
    }

    /// Bilinear sample of the premultiplied level `level` at the plane position `(x, y)` (1/256
    /// game pixel): the four channels, 0 to 255 each.
    fn bilinear(&self, level: usize, px: i64, py: i64) -> [i64; 4] {
        let lv = &self.levels[level];
        let side = lv.side as i64;
        // Texel centres lie half a texel in; the level has `side` texels across the plane, so a
        // position in 1/256 game pixel is `px * side / PLANE` in 1/256 texel
        let tu = (px * side).div_euclid(PLANE as i64) - 128;
        let tv = (py * side).div_euclid(PLANE as i64) - 128;
        let (u0, v0) = (tu.div_euclid(256), tv.div_euclid(256));
        let (fu, fv) = (tu.rem_euclid(256), tv.rem_euclid(256));
        let at = |tx: i64, ty: i64| {
            let (tx, ty) = (tx.rem_euclid(side) as usize, ty.rem_euclid(side) as usize);
            let o = (ty * lv.side + tx) * 4;
            [
                i64::from(lv.premultiplied[o]),
                i64::from(lv.premultiplied[o + 1]),
                i64::from(lv.premultiplied[o + 2]),
                i64::from(lv.premultiplied[o + 3]),
            ]
        };
        let (p00, p10, p01, p11) = (at(u0, v0), at(u0 + 1, v0), at(u0, v0 + 1), at(u0 + 1, v0 + 1));
        let mut out = [0_i64; 4];
        for ch in 0..4 {
            let top = p00[ch] * (256 - fu) + p10[ch] * fu;
            let bottom = p01[ch] * (256 - fu) + p11[ch] * fu;
            out[ch] = (top * (256 - fv) + bottom * fv + 32768) >> 16;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(side: usize, colour: [u8; 3]) -> PlaneArt {
        let rgba: Vec<u8> = [colour[0], colour[1], colour[2], 255]
            .iter()
            .copied()
            .cycle()
            .take(side * side * 4)
            .collect();
        PlaneArt::new(side, &rgba, [[0; 3]; 256])
    }

    #[test]
    fn a_flat_texture_samples_flat_at_any_place_and_footprint() {
        let art = flat(64, [10, 200, 30]);
        for (x, y, step) in [(0, 0, 256), (12345, 999, 256), (-5000, 70000, 700), (3, 3, 5000)] {
            assert_eq!(art.sample(x, y, step), [10, 200, 30, 255]);
        }
    }

    #[test]
    fn the_texture_wraps_around_the_plane() {
        let side = 8;
        let mut rgba = vec![255_u8; side * side * 4];
        for (n, v) in rgba.iter_mut().enumerate().filter(|(n, _)| n % 4 != 3) {
            *v = (n % 251) as u8;
        }
        let art = PlaneArt::new(side, &rgba, [[0; 3]; 256]);
        let at = |x: i64, y: i64| art.sample(x, y, 256);
        let plane = (PLANE as i64) * 256;
        assert_eq!(at(1000, 2000), at(1000 + plane, 2000 - plane));
    }

    #[test]
    fn a_texel_centre_gives_the_texel_and_halfway_the_mean() {
        // 2 x 2 texels over the plane: centres at a quarter and three quarters
        let rgba = vec![0, 0, 0, 255, 200, 0, 0, 255, 0, 100, 0, 255, 0, 0, 40, 255];
        let art = PlaneArt::new(2, &rgba, [[0; 3]; 256]);
        let quarter = (PLANE as i64) * 256 / 4;
        assert_eq!(art.sample(quarter, quarter, 256), [0, 0, 0, 255]);
        assert_eq!(art.sample(3 * quarter, quarter, 256), [200, 0, 0, 255]);
        assert_eq!(art.sample(2 * quarter, quarter, 256), [100, 0, 0, 255]);
        assert_eq!(art.levels(), 2);
    }

    #[test]
    fn a_far_plane_is_read_from_a_smaller_copy() {
        // a one-pixel checkerboard: at a footprint of two texels or more it is the mean grey
        let side = 16;
        let mut rgba = vec![255_u8; side * side * 4];
        for y in 0..side {
            for x in 0..side {
                let v = if (x + y) % 2 == 0 { 255 } else { 0 };
                rgba[(y * side + x) * 4..][..3].fill(v);
            }
        }
        let art = PlaneArt::new(side, &rgba, [[0; 3]; 256]);
        let c = art.sample(40_000, 52_000, 512);
        assert!((120..=136).contains(&c[0]), "{c:?}");
    }

    #[test]
    fn a_transparent_texel_neither_tints_nor_hides_the_art_beside_it() {
        // 2 x 2 texels: the left column has no art (its colour is junk), the right is solid
        let rgba = vec![250, 0, 250, 0, 0, 90, 0, 255, 250, 0, 250, 0, 0, 90, 0, 255];
        let art = PlaneArt::new(2, &rgba, [[0; 3]; 256]);
        let quarter = (PLANE as i64) * 256 / 4;
        assert_eq!(art.sample(quarter, quarter, 256), [0, 0, 0, 0]);
        assert_eq!(art.sample(3 * quarter, quarter, 256), [0, 90, 0, 255]);
        // halfway between them: half the alpha, the colour of the art alone
        assert_eq!(art.sample(2 * quarter, quarter, 256), [0, 90, 0, 128]);
    }

    #[test]
    fn the_key_follows_the_plane_words_only() {
        let mut vram = vec![0_u16; 0x8000];
        let key = plane_key(&vram);
        vram[0x4000] = 7;
        assert_eq!(plane_key(&vram), key);
        vram[0x3FFF] = 1;
        assert_ne!(plane_key(&vram), key);
    }

    #[test]
    fn a_palette_is_256_six_digit_colours() {
        let mut entry = Entry {
            id: "p".to_owned(),
            key: "00000000000000ff".to_owned(),
            tiles: "p/t{x}_{y}.png".to_owned(),
            palette: vec!["102030".to_owned(); 256],
        };
        assert_eq!(entry.key_value(), 255);
        assert_eq!(entry.colours().unwrap()[5], [0x10, 0x20, 0x30]);
        assert_eq!(entry.tile_file(3, 2), "p/t3_2.png");
        entry.palette[9] = "12".to_owned();
        assert!(entry.colours().is_none());
        entry.palette.pop();
        assert!(entry.colours().is_none());
    }

    #[test]
    fn the_index_round_trips_and_a_missing_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Index::open(dir.path()).unwrap(), Index::default());
        let index = Index {
            plane: vec![Entry {
                id: "plane".to_owned(),
                key: "00000000000000ab".to_owned(),
                tiles: "plane/t{x}_{y}.png".to_owned(),
                palette: vec!["000000".to_owned(); 256],
            }],
        };
        std::fs::create_dir_all(dir.path().join("art/mode7")).unwrap();
        std::fs::write(dir.path().join(INDEX), toml::to_string(&index).unwrap()).unwrap();
        assert_eq!(Index::open(dir.path()).unwrap(), index);
        assert_eq!(index.find(0xAB), Some(0));
        assert_eq!(index.find(1), None);
    }
}
