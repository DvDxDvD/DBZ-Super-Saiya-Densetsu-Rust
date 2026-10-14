//! The HD cards: the battle cards (a black body in a cream frame, a Dragon Ball with its stars in
//! one corner, a coloured square with a kanji in the middle, a second ball with a kanji numeral in
//! the opposite corner) drawn again from the approved art.
//!
//! The game builds a card from tiles of its 2-bit UI set: a card is 4 x 6 tiles (frame pieces, a
//! 2 x 2 ball, a 2 x 2 kanji, a 2 x 2 ball), and the same tile pieces make every combination of
//! stars, kanji, numeral and colour. The art is therefore made tile by tile: for each tile of the
//! UI set that a card uses (found by its content, like the HD text's glyphs: `font`) the art store
//! holds the 8 x 8 game pixels redrawn at 8x (64 x 64 art pixels), in up to two layers:
//!
//! * `base`, fixed colours (the frame and the ball, painted for the game's own white card
//!   palette): it is drawn only where the game's palette colour 1 is white, scaled by the colour the
//!   white shows in (a fade of the screen darkens it);
//! * `roles`, an overlay whose channels are weights of the tile palette's colours: red the weight
//!   of colour 3, green of colour 2, blue of colour 1, alpha the coverage. The kanji, the numerals
//!   and the stars are drawn in it, so they take the colours the game's palette gives them (the
//!   blue squares, the yellow and pink one, a fade).
//!
//! The pass lives in the HD text's machinery: a card tile is a "glyph" with an index from
//! [`CARD_BASE`] up, found while the 1x frame is drawn (which pixels of the tile show, and the
//! colours of the palette through the colour math), and drawn over the picture after the look and
//! the art. The art covers the pixels the tile shows, and nothing else.

use crate::font::TextCell;
use rustc_hash::FxHashMap;
use std::path::Path;
use std::sync::OnceLock;

/// The first glyph index that is a card tile (the text glyphs are below).
pub(crate) const CARD_BASE: u8 = 128;
/// The most card tiles the art may hold.
pub const MAX_TILES: usize = 127;
/// Art pixels along a game tile's edge.
pub const TILE: usize = 64;
/// Art pixels along the edge of a tooltip picture (14 game pixels at 8x).
pub const TIP_PX: usize = 112;
/// The tile of the pick cursor (a down arrow above the card).
pub const CURSOR_CODE: u16 = 0x111;
/// The cards' index in the pack: `<pack>/art/cards/cards.toml`.
pub const INDEX: &str = "art/cards/cards.toml";

/// Why the cards' art cannot be read.
#[derive(Debug, thiserror::Error)]
pub enum CardsError {
    #[error("cards: {0}")]
    Read(String),
}

/// `cards.toml`: the tiles with art.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Index {
    pub tile: Vec<Entry>,
    /// The pictures of the tooltip (a kanji in dark ink, or an emblem in colour), by the code of the
    /// tile at the top left of the card's kanji.
    #[serde(default)]
    pub tip: Vec<TipEntry>,
}

/// A tooltip picture: [`TIP_PX`] pixels square.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct TipEntry {
    pub code: u16,
    pub image: String,
}

/// One tile of the index: its code in the UI set (`11:8036`, 384 tiles) and its art files, relative
/// to the folder of the index.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Entry {
    pub code: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roles: Option<String>,
}

/// The art of one tile: 64 x 64 RGBA, straight alpha.
struct Art {
    code: u16,
    base: Option<Vec<u8>>,
    roles: Option<Vec<u8>>,
}

/// A tile's art resized to the picture's scale: `8 * scale` pixels square.
struct Scaled {
    base: Option<Vec<u8>>,
    roles: Option<Vec<u8>>,
}

/// The card art of a pack.
pub struct CardArt {
    tiles: Vec<Art>,
    scaled: [OnceLock<Vec<Scaled>>; 8],
    /// The tooltip pictures: the code of the kanji tile they go with, and the pixels.
    pub(crate) tips: Vec<(u16, Vec<u8>)>,
    tip_scaled: [OnceLock<Vec<Vec<u8>>>; 8],
    /// The tooltip last shown (the cursor blinks, the tooltip stays).
    pub(crate) seen: std::sync::Mutex<Option<crate::tooltip::Seen>>,
}

impl std::fmt::Debug for CardArt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CardArt").field("tiles", &self.tiles.len()).finish_non_exhaustive()
    }
}

impl CardArt {
    /// The art listed by `<pack>/art/cards/cards.toml`; `None` when the pack has none.
    ///
    /// # Errors
    ///
    /// The index or an art file cannot be read, or an art file is not 64 x 64.
    pub fn load(pack: &Path) -> Result<Option<Self>, CardsError> {
        let index_path = pack.join(INDEX);
        let Ok(text) = std::fs::read_to_string(&index_path) else { return Ok(None) };
        let index: Index = toml::from_str(&text).map_err(|e| CardsError::Read(e.to_string()))?;
        let folder = index_path.parent().unwrap_or(pack);
        let read_sized = |name: &str, side: usize| -> Result<Vec<u8>, CardsError> {
            let bytes = std::fs::read(folder.join(name))
                .map_err(|e| CardsError::Read(format!("{name}: {e}")))?;
            let (w, h, rgba) = crate::png::read_rgba(&bytes)
                .map_err(|e| CardsError::Read(format!("{name}: {e}")))?;
            if (w as usize, h as usize) != (side, side) {
                return Err(CardsError::Read(format!("{name}: {w}x{h}, expected {side}x{side}")));
            }
            Ok(rgba)
        };
        let read = |name: &str| read_sized(name, TILE);
        if index.tile.len() > MAX_TILES {
            return Err(CardsError::Read(format!("more than {MAX_TILES} tiles")));
        }
        let mut tiles = Vec::new();
        for entry in &index.tile {
            tiles.push(Art {
                code: entry.code,
                base: entry.base.as_deref().map(read).transpose()?,
                roles: entry.roles.as_deref().map(read).transpose()?,
            });
        }
        let mut tips = Vec::new();
        for tip in &index.tip {
            tips.push((tip.code, read_sized(&tip.image, TIP_PX)?));
        }
        Ok(Some(Self::new(tiles, tips)))
    }

    fn new(tiles: Vec<Art>, tips: Vec<(u16, Vec<u8>)>) -> Self {
        Self {
            tiles,
            scaled: std::array::from_fn(|_| OnceLock::new()),
            tips,
            tip_scaled: std::array::from_fn(|_| OnceLock::new()),
            seen: std::sync::Mutex::new(None),
        }
    }

    /// Card art of tiles that draw nothing but a flat colour, for the tests of the passes built on it.
    #[cfg(test)]
    pub(crate) fn flat(codes: &[u16], tips: Vec<(u16, Vec<u8>)>) -> Self {
        let tiles = codes
            .iter()
            .map(|&code| Art {
                code,
                base: None,
                roles: Some([255, 0, 0, 255].repeat(TILE * TILE)),
            })
            .collect();
        Self::new(tiles, tips)
    }

    /// The glyph index of the tile with code `code`, when it has art.
    pub(crate) fn glyph_of(&self, code: u16) -> Option<u8> {
        self.tiles.iter().position(|tile| tile.code == code).map(|n| CARD_BASE + n as u8)
    }

    /// The code of the tile of glyph `glyph`.
    pub(crate) fn code_of(&self, glyph: u8) -> Option<u16> {
        self.tiles.get(usize::from(glyph.checked_sub(CARD_BASE)?)).map(|tile| tile.code)
    }

    /// The tooltip picture that goes with the kanji tile `code`, 14 game pixels square at `scale`.
    pub(crate) fn tip_picture(&self, scale: usize, code: u16) -> Option<&[u8]> {
        let all = self.tip_scaled[scale - 1].get_or_init(|| {
            self.tips
                .iter()
                .map(|(_, art)| resize_rgba(art, TIP_PX, TIP_PX, 14 * scale, 14 * scale))
                .collect()
        });
        let at = self.tips.iter().position(|(tip, _)| *tip == code)?;
        all.get(at).map(Vec::as_slice)
    }

    /// The codes of the tiles with art, in glyph order (glyph `CARD_BASE + n` is the `n`th).
    pub(crate) fn codes(&self) -> impl Iterator<Item = u16> + '_ {
        self.tiles.iter().map(|tile| tile.code)
    }

    fn scaled(&self, scale: usize) -> &[Scaled] {
        self.scaled[scale - 1].get_or_init(|| {
            let side = 8 * scale;
            self.tiles
                .iter()
                .map(|tile| Scaled {
                    base: tile.base.as_deref().map(|art| resize(art, side)),
                    roles: tile.roles.as_deref().map(|art| resize(art, side)),
                })
                .collect()
        })
    }

    /// Draws the card tile `cell` into the picture `rgba` (`width` pixels wide, `scale` times the
    /// 1x frame): the pixels of the tile that show.
    pub(crate) fn draw(&self, rgba: &mut [u8], width: usize, scale: usize, cell: &TextCell) {
        if !(2..=crate::MAX_SCALE as usize).contains(&scale) {
            return;
        }
        let Some(art) = self.scaled(scale).get(usize::from(cell.glyph - CARD_BASE)) else {
            return;
        };
        let height = rgba.len() / 4 / width;
        let side = 8 * scale;
        for bit in 0..64_usize {
            if cell.mask >> bit & 1 == 0 {
                continue;
            }
            let (column, row) = (bit % 8, bit / 8);
            for sy in 0..scale {
                let y = (cell.y + row as i32) * scale as i32 + sy as i32;
                if !(0..height as i32).contains(&y) {
                    continue;
                }
                for sx in 0..scale {
                    let x = (cell.x + column as i32) * scale as i32 + sx as i32;
                    if !(0..width as i32).contains(&x) {
                        continue;
                    }
                    let at = (row * scale + sy) * side + column * scale + sx;
                    let out = (y as usize * width + x as usize) * 4;
                    if let (Some(base), Some(white)) = (&art.base, cell.white) {
                        let p = &base[at * 4..at * 4 + 4];
                        let colour = [0, 1, 2]
                            .map(|c| ((u32::from(p[c]) * u32::from(white[c]) + 127) / 255) as u8);
                        blend(&mut rgba[out..out + 3], colour, p[3]);
                    }
                    if let Some(roles) = &art.roles {
                        let p = &roles[at * 4..at * 4 + 4];
                        let weight = [u32::from(p[0]), u32::from(p[1]), u32::from(p[2])];
                        let total = (weight[0] + weight[1] + weight[2]).max(1);
                        let colour = [0, 1, 2].map(|c| {
                            let mixed = weight[0] * u32::from(cell.fg[c])
                                + weight[1] * u32::from(cell.mid[c])
                                + weight[2] * u32::from(cell.bg[c]);
                            ((mixed + total / 2) / total) as u8
                        });
                        blend(&mut rgba[out..out + 3], colour, p[3]);
                    }
                }
            }
        }
    }
}

/// `under` mixed with `colour` by `alpha`.
fn blend(under: &mut [u8], colour: [u8; 3], alpha: u8) {
    if alpha == 0 {
        return;
    }
    for c in 0..3 {
        under[c] = ((u32::from(under[c]) * (255 - u32::from(alpha))
            + u32::from(colour[c]) * u32::from(alpha)
            + 127)
            / 255) as u8;
    }
}

/// A 64 x 64 RGBA tile resized to `side` pixels square (see [`resize_rgba`]).
fn resize(art: &[u8], side: usize) -> Vec<u8> {
    if side == TILE { art.to_vec() } else { resize_rgba(art, TILE, TILE, side, side) }
}

/// An RGBA image of `sw` x `sh` pixels resized to `ow` x `oh`: the area average of the source
/// pixels each output pixel covers, weighted by alpha (integer arithmetic, so the output does not
/// depend on the machine).
pub(crate) fn resize_rgba(art: &[u8], sw: usize, sh: usize, ow: usize, oh: usize) -> Vec<u8> {
    let mut out = vec![0_u8; ow * oh * 4];
    // Each output pixel covers a range of source pixels, in sixteenths
    let span = |i: usize, o: usize, s: usize| ((i * s * 16) / o, ((i + 1) * s * 16) / o);
    for y in 0..oh {
        let (y0, y1) = span(y, oh, sh);
        for x in 0..ow {
            let (x0, x1) = span(x, ow, sw);
            let mut sums = [0_u64; 4];
            let mut area = 0_u64;
            for sy in y0 / 16..y1.div_ceil(16) {
                let wy = (y1.min((sy + 1) * 16) - y0.max(sy * 16)) as u64;
                for sx in x0 / 16..x1.div_ceil(16) {
                    let wx = (x1.min((sx + 1) * 16) - x0.max(sx * 16)) as u64;
                    let weight = wx * wy;
                    let p = &art[(sy * sw + sx) * 4..][..4];
                    let alpha = u64::from(p[3]);
                    for c in 0..3 {
                        sums[c] += u64::from(p[c]) * alpha * weight;
                    }
                    sums[3] += alpha * weight;
                    area += weight;
                }
            }
            let at = (y * ow + x) * 4;
            if sums[3] == 0 {
                continue;
            }
            for c in 0..3 {
                out[at + c] = ((sums[c] + sums[3] / 2) / sums[3]) as u8;
            }
            out[at + 3] = ((sums[3] + area / 2) / area) as u8;
        }
    }
    out
}

/// Maps the tile contents of the codes of `art` (read from the cartridge image's UI set) to their
/// glyph indexes (`CARD_BASE + n`), without overriding the glyphs already there.
pub(crate) fn register(
    rom: &[u8],
    art: &CardArt,
    tiles: &mut FxHashMap<[u16; 8], u8>,
) -> Result<(), crate::font::FontError> {
    for (n, code) in art.codes().enumerate() {
        let words = crate::font::tile_words(rom, code).ok_or(crate::font::FontError::ShortRom)?;
        tiles.entry(words).or_insert(CARD_BASE + n as u8);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn art(code: u16, base: Option<[u8; 4]>, roles: Option<[u8; 4]>) -> Art {
        let fill = |p: [u8; 4]| p.iter().copied().cycle().take(TILE * TILE * 4).collect();
        Art { code, base: base.map(fill), roles: roles.map(fill) }
    }

    fn cards(tiles: Vec<Art>) -> CardArt {
        CardArt::new(tiles, Vec::new())
    }

    fn cell(glyph: u8, mask: u64, white: Option<[u8; 3]>) -> TextCell {
        TextCell {
            x: 1,
            y: 1,
            glyph,
            half: 0,
            clear: false,
            bold: false,
            layer: 0,
            fg: [10, 20, 30],
            mid: [100, 110, 120],
            bg: [200, 210, 220],
            white,
            mask,
        }
    }

    #[test]
    fn a_resize_keeps_a_flat_tile_flat_and_weights_colours_by_alpha() {
        let flat = art(1, Some([10, 20, 30, 255]), None).base.unwrap();
        for side in [16, 24, 40, 48, 56] {
            let small = resize(&flat, side);
            assert_eq!(small.len(), side * side * 4);
            assert!(small.chunks(4).all(|p| p == [10, 20, 30, 255]), "side {side}");
        }
        // A half transparent red left half and an opaque blue right half
        let mut tile = vec![0_u8; TILE * TILE * 4];
        for (at, p) in tile.chunks_mut(4).enumerate() {
            p.copy_from_slice(if at % TILE < TILE / 2 {
                &[255, 0, 0, 0]
            } else {
                &[0, 0, 255, 255]
            });
        }
        let small = resize(&tile, 16);
        assert_eq!(&small[..4], [0, 0, 0, 0], "a transparent pixel stays so");
        assert_eq!(&small[15 * 4..16 * 4], [0, 0, 255, 255]);
    }

    #[test]
    fn the_base_takes_the_white_it_shows_in_and_the_roles_the_palette_colours() {
        let both = art(1, Some([255, 255, 255, 255]), Some([0, 255, 0, 255]));
        let cards = cards(vec![both]);
        let (width, scale) = (16 * 4, 4);
        let mut rgba = vec![7_u8; width * width * 4];
        // Only pixel (0, 0) of the tile shows
        let shown = cell(CARD_BASE, 1, Some([128, 128, 128]));
        cards.draw(&mut rgba, width, scale, &shown);
        let at = |x: usize, y: usize| &rgba[(y * width + x) * 4..][..3];
        // The roles layer is over the base: colour 2 (green weight)
        assert_eq!(at(scale, scale), [100, 110, 120]);
        assert_eq!(at(scale + scale - 1, scale + scale - 1), [100, 110, 120]);
        assert_eq!(at(2 * scale, scale), [7, 7, 7], "the next pixel is not shown");
        // Without a white, the base is not drawn
        let only_base = cards_with_base_only();
        let mut rgba = vec![7_u8; width * width * 4];
        only_base.draw(&mut rgba, width, scale, &cell(CARD_BASE, 1, None));
        assert_eq!(&rgba[(scale * width + scale) * 4..][..3], [7, 7, 7]);
        let mut rgba = vec![7_u8; width * width * 4];
        only_base.draw(&mut rgba, width, scale, &cell(CARD_BASE, 1, Some([128, 64, 0])));
        assert_eq!(&rgba[(scale * width + scale) * 4..][..3], [128, 64, 0]);
    }

    fn cards_with_base_only() -> CardArt {
        cards(vec![art(1, Some([255, 255, 255, 255]), None)])
    }

    #[test]
    fn a_tile_is_found_by_its_content() {
        let mut rom = vec![0_u8; crate::font::FONT_OFFSET + 384 * 16];
        let at = crate::font::FONT_OFFSET + 5 * 16;
        rom[at] = 0xAA;
        rom[at + 1] = 0x55;
        let cards = cards(vec![art(5, Some([1, 2, 3, 255]), None)]);
        let mut tiles = FxHashMap::default();
        register(&rom, &cards, &mut tiles).unwrap();
        let mut words = [0_u16; 8];
        words[0] = 0x55AA;
        assert_eq!(tiles.get(&words), Some(&CARD_BASE));
        assert!(register(&[0; 10], &cards, &mut tiles).is_err());
    }
}
