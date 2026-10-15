//! HD battle backgrounds: a BG layer of the battle (the sky and sea of the animation scene, its
//! ground band, the command screen's picture) drawn again from the approved art, found by the
//! content of the layer (its tile map and the tiles the map uses), whatever the scroll.
//!
//! The art is one picture of the whole layer, `SCALE` art pixels to a game pixel, made from the
//! layer's tiles with the palette it was made for (`source`, one game pixel per pixel: the colours
//! the art replaces). Where the game shows the layer, the picture is sampled at the layer's
//! position: the pixel `x` of a line shows the layer at `x + h_scroll` (and the line the layer at
//! `line + v_scroll`) of that line's registers, so a parallax made by an HDMA table or by
//! different scrolls of two layers is the tiles' own. The picture takes the place of what the
//! tiles drew: where the winner of the pixel is the layer itself, the backdrop (the sky of a layer
//! whose art includes it) or an object that an HD pose replaces; the colour math and the
//! brightness of the pixel apply to it as to a portrait's art ([`crate::art`]).
//!
//! A layer the game shows in several palettes has art for each palette that persists (the entries
//! with a [`Entry::palette_key`], bound by [`palette_key`] of the frame first). A palette that is not
//! the one the art was made for (a fade, a flash, a variant with no art of its own) changes the
//! colour of the game's pixel; the art takes the same change through one smooth transfer of its
//! colour ([`crate::transfer`]). [`adapt`] is the Mode 7 plane's per-colour rule.

use crate::screen::{Layer, Plan, Rendered};
use rustc_hash::FxHashMap;
use std::path::Path;

/// The art's index in the pack: `<pack>/art/bg/bg.toml`.
pub const INDEX: &str = "art/bg/bg.toml";

/// Art pixels along a game pixel's edge in the art files.
pub const SCALE: usize = 4;

/// Layers drawn at a time (the far one and the near one).
pub(crate) const MAX_SLOTS: usize = 2;

/// Why the art of the backgrounds cannot be read.
#[derive(Debug, thiserror::Error)]
pub enum BgError {
    #[error("backgrounds: {0}")]
    Read(String),
}

/// `bg.toml`: the layers with art.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Index {
    #[serde(default)]
    pub layer: Vec<Entry>,
}

/// The art of one layer's content.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Entry {
    /// The name of the art (its files' stem), for reports.
    pub id: String,
    /// [`layer_key`] of the content, 16 hexadecimal digits.
    pub key: String,
    /// The BG (0 for BG1) and the registers that say which layer it is.
    pub bg: u8,
    pub mode: u8,
    pub map_base: u16,
    pub char_base: u16,
    pub size: u8,
    /// The tile rows the key and the art cover: first and past the last. A layer that is only
    /// partly a picture (the command screen's lower rows are windows) has art for those rows.
    pub rows: [u16; 2],
    /// The art covers every pixel of its rows (the sky of a layer is drawn where the layer is
    /// transparent and nothing is in front); otherwise only where the layer's tiles are opaque,
    /// by the art's alpha.
    pub fill: bool,
    /// The art file (RGBA, `SCALE` pixels to a game pixel) and the source file (RGBA, one pixel to
    /// a game pixel, the colours the layer had when the art was made), relative to the pack. A
    /// pack made for sharing has no source (it holds no picture of the game): the art is then
    /// drawn as it is whatever palette the layer is shown in, where a source recolours it.
    pub art: String,
    #[serde(default)]
    pub source: String,
    /// [`palette_key`] of the palette this art was made for, 16 hexadecimal digits, when the art is
    /// a variant of a layer's content that the game also shows in other palettes: the art of the
    /// same content whose palette is the one in use is bound first. Empty: the art of the content
    /// in whatever palette (recoloured to the one in use).
    #[serde(default)]
    pub palette_key: String,
    /// The CGRAM the art's colours were made in (256 colours of 15 bits, four hexadecimal digits
    /// each, index 0 first), or empty: what a layer pixel's colour was at CGRAM index `i` when the
    /// art was made, for the recolouring ([`adapt`]) when the pack has no source picture.
    #[serde(default)]
    pub palette: String,
}

impl Entry {
    /// The key as a number.
    #[must_use]
    pub fn key_value(&self) -> u64 {
        u64::from_str_radix(&self.key, 16).unwrap_or(0)
    }

    /// The palette key as a number, `None` for art that is not a variant.
    #[must_use]
    pub fn palette_value(&self) -> Option<u64> {
        u64::from_str_radix(&self.palette_key, 16).ok()
    }

    /// The tile and pixel size of the layer's map (see [`layer_size`]) with large tiles off.
    #[must_use]
    pub fn class(&self) -> Class {
        Class {
            bg: self.bg,
            mode: self.mode,
            map_base: self.map_base,
            char_base: self.char_base,
            size: self.size,
            rows: self.rows,
        }
    }

    /// The pixels the art covers: width, and height of its rows.
    #[must_use]
    pub fn art_size(&self, large: bool) -> (usize, usize) {
        let (width, _) = layer_size(self.size, large);
        (width, usize::from(self.rows[1] - self.rows[0]) * if large { 16 } else { 8 })
    }
}

/// What says which layer of a frame an entry is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Class {
    pub bg: u8,
    pub mode: u8,
    pub map_base: u16,
    pub char_base: u16,
    pub size: u8,
    pub rows: [u16; 2],
}

impl Index {
    /// The index of `<pack>/art/bg/bg.toml`; empty when the pack has none.
    ///
    /// # Errors
    ///
    /// The file is not a valid index.
    pub fn open(pack: &Path) -> Result<Self, BgError> {
        let Ok(text) = std::fs::read_to_string(pack.join(INDEX)) else {
            return Ok(Self::default());
        };
        toml::from_str(&text).map_err(|e| BgError::Read(e.to_string()))
    }
}

/// `(width, height)` in game pixels of a layer's map: `size` is the `BGnSC` size (0 one screen, 1
/// 64 x 32 tiles, 2 32 x 64, 3 64 x 64), a tile 8 or 16 pixels.
#[must_use]
pub fn layer_size(size: u8, large: bool) -> (usize, usize) {
    let unit = if large { 16 } else { 8 };
    let (w, h) = match size {
        0 => (32, 32),
        1 => (64, 32),
        2 => (32, 64),
        _ => (64, 64),
    };
    (w * unit, h * unit)
}

/// The tile map word at tile `(tx, ty)` of a layer's map (the screens of 32 x 32 tiles follow
/// each other as the PPU's map does).
fn map_word(vram: &[u16], map_base: u16, size: u8, tx: usize, ty: usize) -> u16 {
    let mut screen = 0;
    if tx >= 32 {
        screen += 1;
    }
    if ty >= 32 {
        screen += if size == 2 { 1 } else { 2 };
    }
    let at = usize::from(map_base) + screen * 1024 + (ty % 32) * 32 + tx % 32;
    vram[at & 0x7FFF]
}

/// The content key of a layer's rows: a hash of its tile map words in the tile rows `rows` and of
/// the character data of the tiles they use (`planes` bit planes each, 16 x 16 tiles as four
/// 8 x 8 characters), as they sit in the `vram` words. The palette is not part of it.
#[must_use]
pub fn layer_key(
    vram: &[u16],
    map_base: u16,
    char_base: u16,
    size: u8,
    large: bool,
    planes: usize,
    rows: [u16; 2],
) -> u64 {
    let (width, _) = layer_size(size, large);
    let columns = width / if large { 16 } else { 8 };
    let mut hash = 0xCBF2_9CE4_8422_2325_u64;
    let mut feed = |word: u16| {
        for byte in word.to_le_bytes() {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01B3);
        }
    };
    let mut used = std::collections::BTreeSet::new();
    for ty in usize::from(rows[0])..usize::from(rows[1]) {
        for tx in 0..columns {
            let entry = map_word(vram, map_base, size, tx, ty);
            feed(entry);
            used.insert(entry & 0x3FF);
        }
    }
    let words = planes * 4;
    for tile in used {
        let pieces: &[usize] = if large { &[0, 1, 16, 17] } else { &[0] };
        for piece in pieces {
            let at = usize::from(char_base) + (usize::from(tile) + piece) * words;
            for word in 0..words {
                feed(vram[(at + word) & 0x7FFF]);
            }
        }
    }
    hash
}

/// A hash of the colours a layer shows its rows in: the CGRAM entries of the palette rows its tile
/// map (the keyed `rows`) names, and the backdrop colour for a layer that `fill`s. Two frames of
/// one content that show it in the same colours have the same key, so the art made for a palette
/// variant of the layer is told from the others'. `cgram` holds the 256 colours of the frame's
/// start; `vram` and the registers are as for [`layer_key`], `bg` is 0 for BG1.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn palette_key(
    vram: &[u16],
    cgram: &[u16],
    bg: usize,
    mode: u8,
    map_base: u16,
    size: u8,
    large: bool,
    planes: usize,
    rows: [u16; 2],
    fill: bool,
) -> u64 {
    let (width, _) = layer_size(size, large);
    let columns = width / if large { 16 } else { 8 };
    let mut used = std::collections::BTreeSet::new();
    for ty in usize::from(rows[0])..usize::from(rows[1]) {
        for tx in 0..columns {
            used.insert(usize::from(map_word(vram, map_base, size, tx, ty) >> 10 & 7));
        }
    }
    let mut bytes = Vec::new();
    for row in used {
        let (first, n) = if mode == 0 {
            (bg * 32 + row * 4, 4)
        } else if planes == 4 {
            (row * 16, 16)
        } else {
            (row * 4, 4)
        };
        for i in 0..n {
            bytes.extend_from_slice(&cgram[(first + i) & 255].to_le_bytes());
        }
    }
    if fill {
        bytes.extend_from_slice(&cgram[0].to_le_bytes());
    }
    crate::pack::tile_key(&bytes)
}

/// The layers of the index found in a frame: for each, the slot it draws in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Slot {
    /// The entry of the index.
    pub entry: u32,
    pub bg: u8,
    /// Whether the layer's tiles are 16 x 16.
    pub large: bool,
    pub fill: bool,
    pub mode: u8,
    pub map_base: u16,
    pub char_base: u16,
    pub size: u8,
    /// The first pixel row and the pixel rows of the art.
    pub first_row: u16,
    pub rows: u16,
    /// A field map's layer (`entry` is of the pack's `maps` index, not of `bg`): the picture of
    /// the whole map, found by the world position of the pixel (`Probe::world`), `field_size`
    /// pixels (width, height) of it.
    pub field: bool,
    pub field_size: (u16, u16),
}

impl Slot {
    /// The slot of the Mode 7 plane (`entry` is of the pack's `mode7` index): BG1 of mode 7, found
    /// by the content of VRAM ([`crate::mode7art::plane_key`]) and drawn through each line's matrix
    /// ([`crate::art::draw::draw_plane`]), not by a position of the art.
    pub(crate) fn plane(entry: u32) -> Self {
        Self {
            entry,
            bg: 0,
            large: false,
            fill: false,
            mode: 7,
            map_base: 0,
            char_base: 0,
            size: 0,
            first_row: 0,
            rows: 0,
            field: false,
            field_size: (0, 0),
        }
    }

    pub(crate) fn is_plane(&self) -> bool {
        self.mode == 7
    }

    /// Whether the registers of a line show this layer.
    pub(crate) fn shown_by(&self, regs: &ssd_oracle::LineRegs) -> bool {
        let bg = usize::from(self.bg);
        if self.is_plane() {
            // Only the plane alone (BG1): its mosaic and direct colour are the look's
            return regs.bg_mode == 7
                && regs.main_bg[0]
                && !regs.direct_color
                && !(regs.mosaic_bg[0] && regs.mosaic_size > 0);
        }
        if self.field {
            // The field's map is BG2 of mode 1; the layer is drawn by the wide renderer, so the
            // registers of its own tile map do not matter, nor does the mosaic of a fade (the
            // position is the mosaic block's)
            return regs.bg_mode == 1 && regs.main_bg[bg];
        }
        regs.bg_mode == self.mode
            && regs.main_bg[bg]
            && regs.bg_map_base[bg] == self.map_base
            && regs.bg_char_base[bg] == self.char_base
            && regs.bg_screen_size[bg] == self.size
            && regs.bg_large_tiles[bg] == self.large
    }

    /// Whether the layer is drawn with the mosaic on this line: the game's pixel is then the one at
    /// the top left of its block of `MOSAIC` size + 1 pixels (a layer of the field map is not).
    pub(crate) fn mosaic(&self, regs: &ssd_oracle::LineRegs) -> bool {
        !self.field && regs.mosaic_bg[usize::from(self.bg)] && regs.mosaic_size > 0
    }

    /// The pixel `(x, y)` of the art that the pixel of `probe` shows, if it is in the art: for a
    /// field map the world position of the pixel, for a layer of the registers' own scroll.
    pub(crate) fn locate(
        &self,
        regs: &ssd_oracle::LineRegs,
        probe: &Probe<'_>,
    ) -> Option<(u16, u16)> {
        if self.is_plane() {
            // The plane's place is the matrix's, found where the pixel is drawn
            return Some((0, 0));
        }
        if self.field {
            let (x, y) = probe.world?;
            let (width, height) = self.field_size;
            return (0..i32::from(width))
                .contains(&x)
                .then_some(())
                .and((0..i32::from(height)).contains(&y).then_some(()))
                .map(|()| (x as u16, y as u16));
        }
        self.position(regs, probe.window_x, probe.line)
    }

    /// The pixel `(x, y)` of the art that the pixel `window_x` of line `line` (1-based) shows, if
    /// it is in the art's rows.
    pub(crate) fn position(
        &self,
        regs: &ssd_oracle::LineRegs,
        window_x: i32,
        line: usize,
    ) -> Option<(u16, u16)> {
        let bg = usize::from(self.bg);
        let (width, height) = layer_size(self.size, self.large);
        // With the mosaic the pixel shows what its block's top left pixel shows
        let (window_x, line) = if self.mosaic(regs) {
            let block = i32::from(regs.mosaic_size) + 1;
            (
                window_x - window_x.rem_euclid(block),
                line.saturating_sub(usize::from(regs.mosaic_v_offset)),
            )
        } else {
            (window_x, line)
        };
        let x = (window_x + i32::from(regs.bg_h_scroll[bg])).rem_euclid(width as i32) as usize;
        let y = (line + usize::from(regs.bg_v_scroll[bg])) % height;
        let first = usize::from(self.first_row);
        (y >= first && y < first + usize::from(self.rows)).then(|| (x as u16, (y - first) as u16))
    }
}

/// The classes of an index, with the entries of each by key.
#[derive(Debug, Default)]
pub(crate) struct Classes {
    by_class: Vec<(Class, FxHashMap<u64, Vec<Variant>>, bool)>,
}

/// The art of one content: for the palette it was made for (`None`: any), the entry.
#[derive(Debug, Clone, Copy)]
struct Variant {
    palette: Option<u64>,
    entry: u32,
}

impl Classes {
    pub(crate) fn new(index: &Index) -> Self {
        let mut by_class: Vec<(Class, FxHashMap<u64, Vec<Variant>>, bool)> = Vec::new();
        for (at, entry) in index.layer.iter().enumerate() {
            let class = entry.class();
            let variant = Variant { palette: entry.palette_value(), entry: at as u32 };
            match by_class.iter_mut().find(|(c, _, _)| *c == class) {
                Some((_, keys, _)) => keys.entry(entry.key_value()).or_default().push(variant),
                None => {
                    let mut keys: FxHashMap<u64, Vec<Variant>> = FxHashMap::default();
                    keys.entry(entry.key_value()).or_default().push(variant);
                    by_class.push((class, keys, entry.fill));
                }
            }
        }
        Self { by_class }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.by_class.is_empty()
    }

    /// The layers of `index` a frame shows: by its first lines that show a layer of each class.
    pub(crate) fn find(&self, index: &Index, log: &ssd_oracle::FrameLog) -> Vec<Slot> {
        let mut slots = Vec::new();
        // A frame that loads VRAM while it is displayed is not a picture of one layer
        if !log.vram_writes.is_empty() {
            return slots;
        }
        for (class, keys, _) in &self.by_class {
            let bg = usize::from(class.bg);
            let shown = log.entries.iter().find(|entry| {
                let regs = &entry.regs;
                !regs.forced_blanking
                    && regs.bg_mode == class.mode
                    && regs.main_bg[bg]
                    && regs.bg_map_base[bg] == class.map_base
                    && regs.bg_char_base[bg] == class.char_base
                    && regs.bg_screen_size[bg] == class.size
            });
            let Some(shown) = shown else { continue };
            let regs = &shown.regs;
            let large = regs.bg_large_tiles[bg];
            let planes = if class.mode == 0 || bg >= 2 { 2 } else { 4 };
            let key = layer_key(
                &log.vram,
                class.map_base,
                class.char_base,
                class.size,
                large,
                planes,
                class.rows,
            );
            // The art made for the palette in use first, else the art of the content in any palette
            let chosen = keys.get(&key).and_then(|variants| {
                let here = palette_key(
                    &log.vram,
                    &log.cgram_start,
                    bg,
                    class.mode,
                    class.map_base,
                    class.size,
                    large,
                    planes,
                    class.rows,
                    index.layer[variants[0].entry as usize].fill,
                );
                variants
                    .iter()
                    .find(|v| v.palette == Some(here))
                    .or_else(|| variants.iter().find(|v| v.palette.is_none()))
                    .or_else(|| variants.first())
                    .map(|v| v.entry)
            });
            if let Some(entry) = chosen {
                let tile = if large { 16 } else { 8 };
                slots.push(Slot {
                    entry,
                    bg: class.bg,
                    large,
                    fill: index.layer[entry as usize].fill,
                    mode: class.mode,
                    map_base: class.map_base,
                    char_base: class.char_base,
                    size: class.size,
                    first_row: class.rows[0] * tile,
                    rows: (class.rows[1] - class.rows[0]) * tile,
                    field: false,
                    field_size: (0, 0),
                });
            }
        }
        // Far to near: the higher BG number is behind (the layers of modes 0 and 1)
        slots.sort_by_key(|slot| std::cmp::Reverse(slot.bg));
        slots.truncate(MAX_SLOTS);
        slots
    }
}

/// `BgPix::colour` of a pixel whose layer draws nothing there (the art's own colour stands).
pub(crate) const NO_COLOUR: u16 = 0x8000;

/// `BgPix::index` of a pixel with no CGRAM index.
pub(crate) const NO_INDEX: u16 = u16::MAX;

/// The `Entry::palette` word of a CGRAM entry the art's palette does not know (no pixel of the art
/// used it when the palette was found): the pixel keeps the art's own colour in every palette.
pub const UNKNOWN_COLOUR: u16 = 0x8000;

/// The 256 colours of a palette text (`Entry::palette`), or `None` when it is empty or malformed.
#[must_use]
pub fn parse_palette(text: &str) -> Option<[u16; 256]> {
    if text.len() != 1024 || !text.is_ascii() {
        return None;
    }
    let mut out = [0_u16; 256];
    for (at, colour) in out.iter_mut().enumerate() {
        *colour = u16::from_str_radix(&text[at * 4..at * 4 + 4], 16).ok()?;
    }
    Some(out)
}

/// The text of a palette ([`parse_palette`] reads it back).
#[must_use]
pub fn palette_text(colours: &[u16; 256]) -> String {
    use std::fmt::Write;
    colours.iter().fold(String::with_capacity(1024), |mut text, colour| {
        let _ = write!(text, "{colour:04X}");
        text
    })
}

/// How a pixel's winner stands to the art of the layers behind it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Another layer (a window, the HUD) is in front: the art is not drawn.
    Hidden,
    /// The winner is a layer of the art, or the backdrop.
    Layer,
    /// An object is in front: the art is drawn under it when an HD pose hides the object.
    Object,
    /// BG1 is in front (the ground shadow of the command screen): the art is drawn under it where
    /// the shadow pass takes the pixel for a ground shadow it draws again ([`BgRecord::shadow`]).
    Under,
}

/// Where a pixel is in the art of one slot, and how it is shaded.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BgPix {
    /// The pixel of the art's rows (game pixels); `u16::MAX` when the slot draws nothing here.
    pub x: u16,
    pub y: u16,
    /// The 15-bit colour the game's layer shows here, or [`NO_COLOUR`].
    pub colour: u16,
    /// The CGRAM index of that colour (so the colour it had in the palette the art was made in is
    /// known), or [`NO_INDEX`].
    pub index: u16,
    pub plan: Plan,
    /// The layer is mosaic here: the pixel is flat, one point of the art's pixel.
    pub flat: bool,
}

impl BgPix {
    pub(crate) const NONE: Self = Self {
        x: u16::MAX,
        y: 0,
        colour: NO_COLOUR,
        index: NO_INDEX,
        plan: Plan { force_black: false, math: None },
        flat: false,
    };

    pub(crate) fn is_set(&self) -> bool {
        self.x != u16::MAX
    }
}

/// What the art of the backgrounds needs of one pixel of the frame.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BgRecord {
    pub kind: Kind,
    pub pix: [BgPix; MAX_SLOTS],
    /// The pixel is part of a battle background (a layer of [`BATTLE_LAYERS`] or its backdrop),
    /// with or without art.
    pub battle: bool,
    /// A field map's layer is shown, and the pixel is beyond the map (the original's window
    /// there shows the filler of its own tile map): there is nothing to draw art for.
    pub beyond: bool,
    /// The 15-bit colour BG1 draws at the pixel whatever is in front of it (or [`NO_COLOUR`]),
    /// in a frame that shows a battle layer: the silhouette of the command screen's ground
    /// shadow, which is a BG1 layer of its own.
    pub shadow: u16,
}

impl BgRecord {
    pub(crate) const NONE: Self = Self {
        kind: Kind::Hidden,
        pix: [BgPix::NONE; MAX_SLOTS],
        battle: false,
        beyond: false,
        shadow: NO_COLOUR,
    };
}

/// What the capture of a pixel can ask of the renderer that drew it.
pub(crate) struct Probe<'a> {
    pub rendered: &'a Rendered,
    /// The colour-math plan of the pixel as if `Layer` (with an object palette row) won it.
    pub plan_as: &'a dyn Fn(Layer, u8) -> Plan,
    /// The 15-bit colour BG `n` (0 for BG1) draws at the pixel, if it draws one.
    pub layer_colour: &'a dyn Fn(usize) -> Option<u16>,
    /// The CGRAM index of the colour BG `n` draws at the pixel (see `layer_colour`).
    pub layer_index: &'a dyn Fn(usize) -> Option<u16>,
    /// The main screen's backdrop colour.
    pub backdrop: u16,
    /// The pixel's x in the window (the original 256 pixels).
    pub window_x: i32,
    /// The line, 1-based.
    pub line: usize,
    /// The world pixel of the field's map the pixel shows (BG2 of a field scene), `None` in other
    /// scenes.
    pub world: Option<(i32, i32)>,
}

impl Probe<'_> {
    pub(crate) fn plan(&self) -> Plan {
        (self.plan_as)(self.rendered.layer, self.rendered.palette)
    }
}

/// The layers that are the battle's backgrounds, by the registers that show them: BG (0 for BG1),
/// BG mode, tile map base, character base and screen size. The coverage of the art is counted over
/// the pixels they and their backdrop show.
pub const BATTLE_LAYERS: [(u8, u8, u16, u16, u8); 4] = [
    (1, 1, 0x6000, 0x3000, 0),
    (3, 0, 0x7000, 0x5000, 3),
    (2, 0, 0x6800, 0x4000, 1),
    (3, 0, 0x7000, 0x5000, 0),
];

fn bg_of(layer: Layer) -> Option<u8> {
    match layer {
        Layer::Bg1 => Some(0),
        Layer::Bg2 => Some(1),
        Layer::Bg3 => Some(2),
        Layer::Bg4 => Some(3),
        _ => None,
    }
}

fn layer_of(bg: u8) -> Layer {
    match bg {
        0 => Layer::Bg1,
        1 => Layer::Bg2,
        2 => Layer::Bg3,
        _ => Layer::Bg4,
    }
}

/// Whether a line's registers show the battle layer `(bg, mode, map, char, size)`.
fn shows_battle(regs: &ssd_oracle::LineRegs, layer: (u8, u8, u16, u16, u8)) -> bool {
    let bg = usize::from(layer.0);
    regs.bg_mode == layer.1
        && regs.main_bg[bg]
        && regs.bg_map_base[bg] == layer.2
        && regs.bg_char_base[bg] == layer.3
        && regs.bg_screen_size[bg] == layer.4
}

/// Whether the pixel `window_x` of line `line` (1-based), shown by BG `bg` in the PPU's picture, is
/// covered by the art of one of the `slots`: the HD text must leave it to the art (a plain tile
/// of a background can have the content of a glyph of the font).
pub(crate) fn covers(
    slots: &[Slot],
    regs: &ssd_oracle::LineRegs,
    bg: usize,
    window_x: i32,
    line: usize,
) -> bool {
    slots.iter().any(|slot| {
        // (a field map's layer has no text on it, and no scroll registers to locate a pixel by)
        !slot.field
            && !slot.is_plane()
            && usize::from(slot.bg) == bg
            && slot.shown_by(regs)
            && slot.position(regs, window_x, line).is_some()
    })
}

/// Records what the art of the backgrounds needs of the pixel `probe` describes.
pub(crate) fn record(slots: &[Slot], probe: &Probe<'_>, regs: &ssd_oracle::LineRegs) -> BgRecord {
    let winner = probe.rendered.layer;
    // A battle background pixel: a layer of the scenes' or the backdrop in a line that shows one
    let battle = match bg_of(winner) {
        Some(bg) => BATTLE_LAYERS.iter().any(|layer| layer.0 == bg && shows_battle(regs, *layer)),
        None => {
            winner == Layer::Backdrop
                && BATTLE_LAYERS.iter().any(|layer| shows_battle(regs, *layer))
        }
    };
    if slots.is_empty() {
        return BgRecord { battle, ..BgRecord::NONE };
    }
    let first = (probe.layer_colour)(0).unwrap_or(NO_COLOUR);
    let kind = match winner {
        Layer::Backdrop => Kind::Layer,
        Layer::Obj => Kind::Object,
        layer => {
            let bg = bg_of(layer).unwrap_or(0);
            if slots.iter().any(|slot| slot.bg == bg && slot.shown_by(regs)) {
                Kind::Layer
            } else if layer == Layer::Bg1 && regs.bg_mode < 2 {
                Kind::Under
            } else {
                Kind::Hidden
            }
        }
    };
    if kind == Kind::Hidden {
        return BgRecord { battle, shadow: first, ..BgRecord::NONE };
    }
    let mut record =
        BgRecord { kind, pix: [BgPix::NONE; MAX_SLOTS], battle, beyond: false, shadow: first };
    for (j, slot) in slots.iter().enumerate() {
        let bg = usize::from(slot.bg);
        if !slot.shown_by(regs) {
            continue;
        }
        // Behind a window that hides the layer the pixel is not the layer's
        let in_front = winner != Layer::Backdrop && winner != Layer::Obj;
        if !in_front && regs.main_bg_window_off[bg] {
            continue;
        }
        let Some((x, y)) = slot.locate(regs, probe) else {
            record.beyond |= slot.field && probe.world.is_some();
            continue;
        };
        let own = (probe.layer_colour)(bg);
        let colour = if bg_of(winner) == Some(slot.bg) {
            probe.rendered.color
        } else if let Some(colour) = own {
            colour
        } else if slot.fill {
            probe.backdrop
        } else {
            NO_COLOUR
        };
        // (the backdrop is CGRAM entry 0)
        let index = if own.is_some() || bg_of(winner) == Some(slot.bg) {
            (probe.layer_index)(bg).unwrap_or(NO_INDEX)
        } else if slot.fill {
            0
        } else {
            NO_INDEX
        };
        // A layer that is not the winner draws under it: its own colour math, unless the
        // winner is the backdrop, which the art then replaces
        let plan_layer = match winner {
            Layer::Obj => layer_of(slot.bg),
            _ if kind == Kind::Under => layer_of(slot.bg),
            other => other,
        };
        let plan = (probe.plan_as)(plan_layer, probe.rendered.palette);
        record.pix[j] = BgPix { x, y, colour, index, plan, flat: slot.mosaic(regs) };
    }
    record
}

/// The art of an entry as the picture needs it: RGBA at the picture's scale, and the colours of
/// the game's pixels it was made from, one pixel each.
#[derive(Debug)]
pub(crate) struct BgImage {
    pub art: BgArt,
    /// The CGRAM the art was made in, when the pack says it: the colours of the pixels the art
    /// replaces, by CGRAM index, for a pack with no source picture.
    pub base: Option<std::sync::Arc<[u16; 256]>>,
    /// The colours of the game's pixels the art was made from; `None` when the pack holds none.
    pub source: Option<std::sync::Arc<crate::Image>>,
}

/// The art of a layer: one picture, or (a field map, too big to load whole) the tiles of it that
/// a frame shows, `mapart::TILE` game pixels square.
#[derive(Debug)]
pub(crate) enum BgArt {
    Whole(crate::Image),
    Tiled(FxHashMap<(u16, u16), std::sync::Arc<crate::Image>>),
}

impl BgImage {
    /// The art picture that holds game pixel `(gx, gy)` and the art pixel of its top left corner
    /// in it (`scale` art pixels to a game pixel).
    pub(crate) fn block(
        &self,
        gx: usize,
        gy: usize,
        scale: usize,
    ) -> Option<(&crate::Image, usize, usize)> {
        match &self.art {
            BgArt::Whole(image) => Some((image, gx * scale, gy * scale)),
            BgArt::Tiled(tiles) => {
                let side = crate::mapart::TILE;
                let tile = tiles.get(&((gx / side) as u16, (gy / side) as u16))?;
                Some((tile, (gx % side) * scale, (gy % side) * scale))
            }
        }
    }
}

/// What a pixel of the Mode 7 plane's art becomes when the game's pixel is `current` where the art was made for
/// `reference`: the game's colour now plus the art's detail (its difference from the colour it
/// was made on), the detail scaled by how much brighter or darker the game's colour is. A fade
/// scales the art as the game's colour was scaled, and a palette of another hue (the same layer
/// is drawn in green, red and blue) gives the art's shading in that hue.
#[must_use]
pub(crate) fn adapt(art: [u8; 3], reference: [u8; 3], current: [u8; 3]) -> [u8; 3] {
    if (0..3).all(|c| reference[c].abs_diff(current[c]) <= 3) {
        return art;
    }
    let luma = |rgb: [u8; 3]| {
        (u32::from(rgb[0]) * 299 + u32::from(rgb[1]) * 587 + u32::from(rgb[2]) * 114) / 1000
    };
    let (lr, lc) = (luma(reference) as i32 + 8, luma(current) as i32 + 8);
    let scale = (lc * 256 / lr).clamp(51, 512);
    let mut out = art;
    for c in 0..3 {
        let detail = i32::from(art[c]) - i32::from(reference[c]);
        out[c] = (i32::from(current[c]) + (detail * scale + 128 * detail.signum()) / 256)
            .clamp(0, 255) as u8;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_follows_the_map_rows_and_the_tiles_they_use_and_nothing_else() {
        let mut vram = vec![0_u16; 32768];
        // 4-bit tiles at 0x1000: tile 1 is a row of ones
        for word in 0..16 {
            vram[0x1000 + 16 + word] = 0x00FF;
        }
        vram[0x0400] = 1;
        let key = |vram: &[u16], rows| layer_key(vram, 0x0400, 0x1000, 0, false, 4, rows);
        let base = key(&vram, [0, 2]);
        assert_eq!(base, key(&vram, [0, 2]));
        // A change in a row outside the keyed rows does not change it
        vram[0x0400 + 5 * 32] = 7;
        assert_eq!(base, key(&vram, [0, 2]));
        assert_ne!(base, key(&vram, [0, 6]));
        // A change in a tile that the rows use does
        vram[0x1000 + 16] ^= 1;
        assert_ne!(base, key(&vram, [0, 2]));
    }

    #[test]
    fn a_field_layer_is_found_by_the_world_position_and_not_by_the_scroll() {
        let slot = Slot {
            entry: 0,
            bg: 1,
            large: false,
            fill: true,
            mode: 1,
            map_base: 0,
            char_base: 0,
            size: 0,
            first_row: 0,
            rows: 0,
            field: true,
            field_size: (512, 256),
        };
        let mut regs = ssd_oracle::LineRegs { bg_mode: 1, ..ssd_oracle::LineRegs::default() };
        regs.bg_h_scroll[1] = 77;
        assert!(!slot.shown_by(&regs), "BG2 is off");
        regs.main_bg[1] = true;
        assert!(slot.shown_by(&regs));
        regs.bg_mode = 0;
        assert!(!slot.shown_by(&regs), "the field is mode 1");
        let rendered = crate::screen::Rendered {
            color: 0,
            palette: 0,
            layer: Layer::Bg2,
            char: 0,
            pos: 0,
            oam: crate::NO_OBJECT,
            priority: 0,
        };
        let none = |_, _| Plan { force_black: false, math: None };
        let no_colour = |_| None;
        let probe = |world| Probe {
            rendered: &rendered,
            plan_as: &none,
            layer_colour: &no_colour,
            layer_index: &no_colour,
            backdrop: 0,
            window_x: 5,
            line: 9,
            world,
        };
        assert_eq!(slot.locate(&regs, &probe(Some((300, 200)))), Some((300, 200)));
        assert_eq!(slot.locate(&regs, &probe(Some((512, 0)))), None, "beyond the map");
        assert_eq!(slot.locate(&regs, &probe(Some((-1, 0)))), None);
        assert_eq!(slot.locate(&regs, &probe(None)), None, "no world position, no art");
    }

    #[test]
    fn the_art_follows_the_colour_the_game_shows_now() {
        // The same palette: the art as it is
        assert_eq!(adapt([10, 20, 30], [100, 100, 100], [102, 99, 100]), [10, 20, 30]);
        // A fade to half: the art at about half too (detail scaled by the luminance ratio)
        let faded = adapt([120, 100, 80], [100, 100, 100], [50, 50, 50]);
        assert!(faded.iter().all(|&c| (36..=64).contains(&c)), "{faded:?}");
        // Another hue of the same luminance: the art's detail around the new colour
        let red = adapt([10, 190, 10], [0, 180, 0], [180, 40, 40]);
        assert!(red[0] > 180 && red[1] < 70 && red[2] < 70, "{red:?}");
    }

    #[test]
    fn the_text_pass_leaves_a_covered_pixel_of_a_background_to_the_art() {
        let slot = Slot {
            entry: 0,
            bg: 2,
            large: false,
            fill: false,
            mode: 0,
            map_base: 0x6800,
            char_base: 0x4000,
            size: 1,
            first_row: 0,
            rows: 256,
            field: false,
            field_size: (0, 0),
        };
        let mut regs = ssd_oracle::LineRegs { bg_mode: 0, ..ssd_oracle::LineRegs::default() };
        regs.main_bg[2] = true;
        regs.bg_map_base[2] = 0x6800;
        regs.bg_char_base[2] = 0x4000;
        regs.bg_screen_size[2] = 1;
        assert!(covers(&[slot], &regs, 2, 5, 40));
        assert!(!covers(&[slot], &regs, 1, 5, 40), "another BG");
        assert!(!covers(&[], &regs, 2, 5, 40), "no layer of the art");
        regs.bg_map_base[2] = 0x7000;
        assert!(!covers(&[slot], &regs, 2, 5, 40), "another layer on that BG");
    }

    #[test]
    fn a_mosaic_pixel_shows_the_place_of_the_top_left_of_its_block() {
        let slot = Slot {
            entry: 0,
            bg: 1,
            large: false,
            fill: true,
            mode: 1,
            map_base: 0x6000,
            char_base: 0x3000,
            size: 0,
            first_row: 0,
            rows: 176,
            field: false,
            field_size: (0, 0),
        };
        let mut regs = ssd_oracle::LineRegs { bg_mode: 1, ..ssd_oracle::LineRegs::default() };
        regs.main_bg[1] = true;
        regs.bg_map_base[1] = 0x6000;
        regs.bg_char_base[1] = 0x3000;
        regs.bg_h_scroll[1] = 3;
        assert!(!slot.mosaic(&regs));
        assert_eq!(slot.position(&regs, 21, 40), Some((24, 40)));
        // Blocks of 4 pixels: the pixel 21 shows the one at 20, the line 42 the line of its block
        regs.mosaic_bg[1] = true;
        regs.mosaic_size = 3;
        regs.mosaic_v_offset = 2;
        assert!(slot.mosaic(&regs) && slot.shown_by(&regs));
        assert_eq!(slot.position(&regs, 21, 42), Some((23, 40)));
        // A mosaic of the other layers does not touch this one
        regs.mosaic_bg[1] = false;
        assert!(!slot.mosaic(&regs));
    }

    #[test]
    fn a_position_follows_the_scroll_of_the_line_and_wraps() {
        let slot = Slot {
            entry: 0,
            bg: 3,
            large: false,
            fill: true,
            mode: 0,
            map_base: 0x7000,
            char_base: 0x5000,
            size: 3,
            first_row: 0,
            rows: 512,
            field: false,
            field_size: (0, 0),
        };
        let mut regs = ssd_oracle::LineRegs { bg_mode: 0, ..ssd_oracle::LineRegs::default() };
        regs.bg_h_scroll[3] = 500;
        regs.bg_v_scroll[3] = 510;
        assert_eq!(slot.position(&regs, 20, 3), Some((8, 1)));
        let strip = Slot { first_row: 8, rows: 16, ..slot };
        regs.bg_v_scroll[3] = 0;
        assert_eq!(strip.position(&regs, 0, 7), None);
        assert_eq!(strip.position(&regs, 0, 8), Some((500, 0)));
    }
}
