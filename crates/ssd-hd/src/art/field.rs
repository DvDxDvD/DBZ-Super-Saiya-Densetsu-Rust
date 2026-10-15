//! The art of the field's map layer for a frame (`mapart`): the layer of the map the frame shows
//! is found by the pointers of the map's data, and the tiles of its art that the frame's pixels
//! touch are read and sized to the picture's scale (a map is up to 3072 x 3072 game pixels, so the
//! art is never loaded whole).

use super::{ArtState, Image, resample};
use crate::bgart::{BgArt, BgImage, Slot};
use crate::mapart::{self, Entry};
use crate::world::FieldState;
use rustc_hash::FxHashMap;
use std::collections::BTreeSet;
use std::sync::Arc;

/// Tiles kept at the picture's scale before the cache starts again: 4 MB each at 4x.
const MAX_TILES: usize = 48;

fn read(path: &std::path::Path) -> Option<(usize, usize, Vec<u8>)> {
    let bytes = std::fs::read(path).ok()?;
    let (width, height, rgba) = crate::png::read_rgba(&bytes).ok()?;
    Some((width as usize, height as usize, rgba))
}

/// An image whose luma reference is not needed (the field's art is never luma art).
fn plain(width: usize, height: usize, rgba: Vec<u8>) -> Image {
    Image { width, height, rgba, luma_ref: 255 }
}

impl ArtState {
    /// Finds the art of the map a field frame shows (by the size and data pointers of its layer)
    /// and adds it to the frame's background slots; call it after [`ArtState::begin_bg`].
    pub(crate) fn begin_field(&mut self, field: Option<&FieldState>) {
        let Some(entry) = self.field_entry(field) else { return };
        // The map's layer takes BG2's place and leaves room in the records (a frame can match a
        // background class of the pack besides: it keeps the layers behind the map)
        self.lookup.bg_slots.retain(|slot| slot.bg != 1);
        self.lookup.bg_slots.truncate(crate::bgart::MAX_SLOTS - 1);
        self.stats.field_map.bound = Some(entry as u32);
        let (width, height) = self.pack.maps.layer[entry].size();
        self.lookup.bg_slots.push(Slot {
            entry: entry as u32,
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
            field_size: (width as u16, height as u16),
        });
    }

    /// The layer of the maps' index that the map data of `field` names, if the pack has art for it.
    pub(crate) fn field_entry(&self, field: Option<&FieldState>) -> Option<usize> {
        let field = field?;
        if self.pack.maps.layer.is_empty() {
            return None;
        }
        let (grid, metatiles) = field.map.layout();
        self.pack.maps.find(field.map.blocks, grid, metatiles, field.map_id)
    }

    /// The source picture of layer `at` of the maps' index (the layer at 1x in the colours the art
    /// was made for).
    fn field_source(&mut self, at: u32) -> Option<Arc<Image>> {
        if let Some(held) = self.field_sources.get(&at) {
            return held.clone();
        }
        let entry: &Entry = &self.pack.maps.layer[at as usize];
        let path = self.pack.dir.join("art").join("maps").join(&entry.source);
        let loaded = read(&path)
            .filter(|(w, h, _)| (*w, *h) == entry.size())
            .map(|(w, h, rgba)| Arc::new(plain(w, h, rgba)));
        // Only the layer being shown is kept
        self.field_sources.clear();
        self.field_sources.insert(at, loaded.clone());
        loaded
    }

    /// The tiles `needed` (column, row) of the art of layer `at`, at the picture's scale.
    fn field_tiles(
        &mut self,
        at: u32,
        needed: &BTreeSet<(u16, u16)>,
    ) -> FxHashMap<(u16, u16), Arc<Image>> {
        let missing =
            needed.iter().filter(|t| !self.field_tiles.contains_key(&(at, t.0, t.1))).count();
        if self.field_tiles.len() + missing > MAX_TILES {
            self.field_tiles.clear();
        }
        let side = mapart::TILE * self.scale;
        let mut tiles = FxHashMap::default();
        for &(tx, ty) in needed {
            let key = (at, tx, ty);
            if !self.field_tiles.contains_key(&key) {
                let entry = &self.pack.maps.layer[at as usize];
                let path = self
                    .pack
                    .dir
                    .join("art")
                    .join("maps")
                    .join(entry.tile_file(usize::from(tx), usize::from(ty)));
                let art = read(&path)
                    .filter(|(w, h, _)| *w == mapart::TILE * mapart::SCALE && h == w)
                    .map(|(w, h, rgba)| {
                        Arc::new(resample::resize(&Image::new(w, h, rgba), side, side))
                    });
                self.field_tiles.insert(key, art);
            }
            if let Some(Some(tile)) = self.field_tiles.get(&key) {
                tiles.insert((tx, ty), Arc::clone(tile));
            }
        }
        tiles
    }

    /// The art of the field layer of `slot` (slot `index` of the frame's) for the pixels of the
    /// frame: the tiles they touch.
    pub(crate) fn field_image(&mut self, slot: &Slot, index: usize) -> Option<Arc<BgImage>> {
        // (a pack with no source picture draws the art as it is)
        let source = if self.pack.maps.layer[slot.entry as usize].source.is_empty() {
            None
        } else {
            Some(self.field_source(slot.entry)?)
        };
        let mut needed = BTreeSet::new();
        for pixel in &self.pixels {
            let pix = pixel.bg.pix[index];
            if pix.is_set() {
                needed.insert((pix.x / mapart::TILE as u16, pix.y / mapart::TILE as u16));
            }
        }
        let tiles = self.field_tiles(slot.entry, &needed);
        let base = crate::bgart::parse_palette(&self.pack.maps.layer[slot.entry as usize].palette)
            .map(Arc::new);
        Some(Arc::new(BgImage { art: BgArt::Tiled(tiles), base, source }))
    }
}
