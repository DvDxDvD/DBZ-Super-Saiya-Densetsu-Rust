//! The tally of a frame: what each visible pixel of the Enhanced picture is made of, so a tour of
//! the game can say how much of what the player sees is drawn from the HD art and how much is the
//! pixel-art filter's (`xtask hd-tour`).
//!
//! A pixel is counted under the layer it comes from, the pack asset its character is bound to,
//! the pose it belongs to (an object pixel of a pose of the object layer, with the fate the art
//! gave the pose) and whether the art draws it. What is not drawn from art is also listed as
//! items: the pose, the pack tile or the unbound character that shows it, with the pixels and
//! the box it covers, so a report can say what is missing and where.

use super::{ArtState, NO_ASSET, NO_PICTURE, Shown};
use crate::font::TextCell;
use crate::pack::word_key;
use rustc_hash::FxHashMap;
use ssd_oracle::FrameLog;

/// The layer a counted pixel comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TallyLayer {
    Bg,
    Obj,
    /// A Mode 7 plane pixel: the plane is not made of characters.
    Plane,
}

/// What the art did with a pose of the object layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Fate {
    /// The index has art for the pose and it was drawn.
    Art,
    /// A pose the capture would have kept (40 pixels, not stripy) with no art in the index.
    Missing,
    /// A pose the capture left out (fewer than 40 pixels, or stripy: a fade-in, a cut sprite)
    /// that no pose of the index holds.
    Partial,
}

/// Visible pixels with the same description.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TallyCount {
    pub layer: TallyLayer,
    /// Bits a pixel of the character takes (4 for objects, 2 or 4 for BG; 0 for the plane).
    pub bpp: u8,
    /// The asset of the pack the character is bound to, if it is bound, and the tile of it (which
    /// tells the parts of one asset apart: the glyphs of the UI set from its cards); 0 when not
    /// bound.
    pub asset: Option<u32>,
    pub tile: u32,
    /// For an object pixel of a pose: whether the pose is a fighter's, and its fate.
    pub pose: Option<(bool, Fate)>,
    /// Whether the art draws the pixel (the HD text font included).
    pub art: bool,
    /// Whether it is the HD text font that draws it: a pixel of a cell of a glyph of the UI font.
    pub font: bool,
    pub pixels: u64,
}

/// What a pixel that is not drawn from art shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ItemKey {
    /// A pose of the object layer with no art (`key` is the pose's key, see `poses`).
    Pose { key: u64, fighter: bool, fate: Fate },
    /// A character bound to tile `tile` of asset `asset` of the pack, with no art drawn for it
    /// (`object`: an object's, else a BG's).
    Tile { asset: u32, tile: u32, object: bool },
    /// A character bound to no asset: the key of its words, as the pack keys tiles.
    Char { object: bool, bpp: u8, key: u64 },
}

/// One thing a frame shows without art, the pixels it covers and their box in the frame
/// (`x0, y0, x1, y1`, inclusive).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TallyItem {
    pub key: ItemKey,
    pub pixels: u64,
    pub bbox: (u16, u16, u16, u16),
}

/// The tally of one frame.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FrameTally {
    pub counts: Vec<TallyCount>,
    /// The poses of the frame with no art, with their pixels.
    pub shapes: Vec<super::PoseShape>,
    pub items: Vec<TallyItem>,
    /// Every pose of the frame (a fighter, an effect, a field group) with where its art came from:
    /// the audit of the tour reads which art each instance was drawn from.
    pub uses: Vec<super::PoseStat>,
    /// The portraits and faces of the frame and where their parts come from.
    pub portraits: Vec<super::PortraitUse>,
}

impl FrameTally {
    /// The visible pixels counted.
    #[must_use]
    pub fn pixels(&self) -> u64 {
        self.counts.iter().map(|count| count.pixels).sum()
    }

    /// The visible pixels the art draws.
    #[must_use]
    pub fn art_pixels(&self) -> u64 {
        self.counts.iter().filter(|count| count.art).map(|count| count.pixels).sum()
    }
}

/// What a count is keyed by: the layer, the bit depth, the asset and tile, the pose, whether the
/// art draws the pixel, and whether it is the HD font.
type CountKey = (TallyLayer, u8, u32, u32, Option<(bool, Fate)>, bool, bool);

/// A box that grows to hold pixels.
#[derive(Clone, Copy)]
struct Span {
    pixels: u64,
    bbox: (u16, u16, u16, u16),
}

impl Span {
    fn new(x: u16, y: u16) -> Self {
        Self { pixels: 1, bbox: (x, y, x, y) }
    }

    fn add(&mut self, x: u16, y: u16) {
        self.pixels += 1;
        self.bbox =
            (self.bbox.0.min(x), self.bbox.1.min(y), self.bbox.2.max(x), self.bbox.3.max(y));
    }

    fn merge(&mut self, other: Span) {
        self.pixels += other.pixels;
        self.bbox = (
            self.bbox.0.min(other.bbox.0),
            self.bbox.1.min(other.bbox.1),
            self.bbox.2.max(other.bbox.2),
            self.bbox.3.max(other.bbox.3),
        );
    }
}

fn grow<K: std::hash::Hash + Eq>(map: &mut FxHashMap<K, Span>, key: K, x: u16, y: u16) {
    map.entry(key).and_modify(|span| span.add(x, y)).or_insert_with(|| Span::new(x, y));
}

fn merge(items: &mut FxHashMap<ItemKey, Span>, key: ItemKey, span: Span) {
    items.entry(key).and_modify(|held| held.merge(span)).or_insert(span);
}

impl ArtState {
    /// Counts the pixels of the frame just drawn: call it after `find_poses`, when the record of
    /// the pixels and the poses are the frame's. `log` gives the VRAM the characters that are
    /// bound to no asset are keyed from, `text` the cells of the UI font the HD text draws.
    pub(crate) fn tally_frame(&mut self, log: &FrameLog, text: &[TextCell]) {
        // The pixels the HD text font draws: the pixels its cells show
        let mut font_pixels = vec![false; self.pixels.len()];
        for cell in text {
            for bit in 0..64_i32 {
                if cell.mask >> bit & 1 == 0 {
                    continue;
                }
                let (x, y) = (cell.x + bit % 8, cell.y + bit / 8);
                if (0..self.width as i32).contains(&x)
                    && y >= 0
                    && let Some(drawn) = font_pixels.get_mut(y as usize * self.width + x as usize)
                {
                    *drawn = true;
                }
            }
        }
        self.repair_portraits();
        let poses = &self.stats.poses;
        let fate = |at: usize| {
            let pose = &poses[at];
            if pose.matched {
                Fate::Art
            } else if pose.opaque >= 40 && pose.stripy < 200 {
                Fate::Missing
            } else {
                Fate::Partial
            }
        };
        // The pose each OAM entry belongs to: one that has art over one that has none, a fighter
        // over an effect, else the first
        let score = |at: usize| 2 * u8::from(poses[at].matched) + u8::from(poses[at].fighter);
        let mut pose_of: [Option<usize>; 128] = [None; 128];
        for (at, mask) in self.pose_sprites.iter().enumerate().take(poses.len()) {
            for (entry, slot) in pose_of.iter_mut().enumerate() {
                if mask >> entry & 1 != 0 && slot.is_none_or(|held| score(at) > score(held)) {
                    *slot = Some(at);
                }
            }
        }
        let mut counts: FxHashMap<CountKey, u64> = FxHashMap::default();
        let mut poses_seen: FxHashMap<(u64, bool, Fate), Span> = FxHashMap::default();
        let mut tiles: FxHashMap<(u32, u32, bool), Span> = FxHashMap::default();
        let mut chars: FxHashMap<(bool, u8, u16), Span> = FxHashMap::default();
        for (at, pixel) in self.pixels.iter().enumerate() {
            let layer = match pixel.shown {
                Shown::Nothing => continue,
                Shown::Bg => TallyLayer::Bg,
                Shown::Obj => TallyLayer::Obj,
                Shown::Plane => TallyLayer::Plane,
            };
            let (x, y) = ((at % self.width) as u16, (at / self.width) as u16);
            // Beyond the map the original's window shows the filler of its tile map: not the map's
            if layer == TallyLayer::Bg && pixel.bg.beyond {
                continue;
            }
            let pose = (pixel.oam != crate::NO_OBJECT)
                .then(|| pose_of[usize::from(pixel.oam) & 127])
                .flatten();
            let pose_fate = pose.map(|at| (poses[at].fighter, fate(at)));
            let font = layer == TallyLayer::Bg && font_pixels[at];
            // A layer with art (a field map's, a battle background's, a picture's) draws the pixel
            // from it (the tally leaves the drawing out, but the record says which pixels the art
            // would take)
            let layer_art = layer == TallyLayer::Bg
                && pixel.bg.kind == crate::bgart::Kind::Layer
                && pixel.bg.pix.iter().any(crate::bgart::BgPix::is_set);
            let art = font
                || layer_art
                // A picture covers a pixel of a pose too (a face drawn as sprites)
                || pixel.picture != NO_PICTURE
                || pose_fate.is_some_and(|(_, fate)| fate == Fate::Art);
            let asset = pixel.asset;
            let tile = if asset == NO_ASSET { 0 } else { pixel.tile };
            *counts.entry((layer, pixel.bpp, asset, tile, pose_fate, art, font)).or_default() += 1;
            if art || layer == TallyLayer::Plane {
                continue;
            }
            match (pose, pose_fate) {
                (Some(at), Some((fighter, fate))) => {
                    grow(&mut poses_seen, (poses[at].key, fighter, fate), x, y);
                }
                _ if asset != NO_ASSET => {
                    grow(&mut tiles, (asset, pixel.tile, layer == TallyLayer::Obj), x, y);
                }
                _ => grow(&mut chars, (layer == TallyLayer::Obj, pixel.bpp, pixel.char), x, y),
            }
        }
        let mut tally = FrameTally::default();
        for ((layer, bpp, asset, tile, pose, art, font), pixels) in counts {
            let asset = (asset != NO_ASSET).then_some(asset);
            tally.counts.push(TallyCount { layer, bpp, asset, tile, pose, art, font, pixels });
        }
        tally.counts.sort();
        let mut items: FxHashMap<ItemKey, Span> = FxHashMap::default();
        for ((key, fighter, fate), span) in poses_seen {
            merge(&mut items, ItemKey::Pose { key, fighter, fate }, span);
        }
        for ((asset, tile, object), span) in tiles {
            merge(&mut items, ItemKey::Tile { asset, tile, object }, span);
        }
        for ((object, bpp, address), span) in chars {
            let words = if bpp == 4 { 16 } else { 8 };
            let from = usize::from(address);
            let Some(chars) = log.vram.get(from..from + words) else { continue };
            merge(&mut items, ItemKey::Char { object, bpp, key: word_key(chars) }, span);
        }
        tally.items = items
            .into_iter()
            .map(|(key, span)| TallyItem { key, pixels: span.pixels, bbox: span.bbox })
            .collect();
        tally.items.sort_by_key(|item| item.key);
        for (at, shape) in std::mem::take(&mut self.dumps) {
            tally.shapes.push(super::PoseShape { fate: fate(at), ..shape });
        }
        tally.uses.clone_from(&self.stats.poses);
        tally.portraits = self.portrait_uses();
        self.tally = tally;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_span_grows_to_hold_its_pixels() {
        let mut span = Span::new(5, 7);
        span.add(2, 9);
        span.add(6, 7);
        assert_eq!((span.pixels, span.bbox), (3, (2, 7, 6, 9)));
        let mut other = Span::new(0, 0);
        other.merge(span);
        assert_eq!((other.pixels, other.bbox), (4, (0, 0, 6, 9)));
    }

    #[test]
    fn a_tally_adds_up_its_pixels() {
        let count = |art, pixels| TallyCount {
            layer: TallyLayer::Bg,
            bpp: 4,
            asset: Some(1),
            tile: 0,
            pose: None,
            art,
            font: false,
            pixels,
        };
        let tally = FrameTally {
            counts: vec![count(true, 30), count(false, 10)],
            shapes: vec![],
            items: vec![],
            uses: vec![],
            portraits: vec![],
        };
        assert_eq!((tally.pixels(), tally.art_pixels()), (40, 30));
    }
}
