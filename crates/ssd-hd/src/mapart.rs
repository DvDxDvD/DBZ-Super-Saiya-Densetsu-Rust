//! HD field maps: the map layer (BG2 of the field) drawn again from the approved art.
//!
//! A map is not a picture in VRAM: the game keeps a grid of block ids and a metatile table in the
//! ROM and fills only about 35 columns of the layer as the camera moves (`world::FieldMap`). The art
//! is one picture of the whole map made from that lookup (the layer at 1x, in the colours the map
//! has), redrawn at [`SCALE`] art pixels to a game pixel with the same layout. A map is found by
//! what makes the layer, the pointers of its data (size in blocks, grid and metatile table: every
//! map that shares them shares the art, whatever palette it is shown in), and a pixel of the
//! picture is found by the world position of the pixel of the layer the PPU (or the wide renderer)
//! draws there.
//!
//! The art is cut into tiles of [`TILE`] x [`TILE`] game pixels so a map of 3072 x 3072 pixels is
//! never loaded whole; the renderer reads the tiles a picture shows. Next to it the index keeps the
//! `source`: the layer at 1x in the colours the art was made for, which is what a fade, a flash or
//! the water's palette cycle is measured against.

use std::path::Path;

/// The maps' index in the pack: `<pack>/art/maps/maps.toml`.
pub const INDEX: &str = "art/maps/maps.toml";

/// Art pixels along a game pixel's edge in the art files.
pub const SCALE: usize = 4;

/// Game pixels along a tile of the art's edge.
pub const TILE: usize = 256;

/// Why the maps' art cannot be read.
#[derive(Debug, thiserror::Error)]
pub enum MapArtError {
    #[error("maps: {0}")]
    Read(String),
}

/// `maps.toml`: the layers with art.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Index {
    #[serde(default)]
    pub layer: Vec<Entry>,
}

/// The art of one layer.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Entry {
    /// The name of the art (its folder), for reports.
    pub id: String,
    /// The maps of the game that show this layer.
    pub maps: Vec<u8>,
    /// The size in blocks of 256 x 256 pixels.
    pub blocks: [u8; 2],
    /// The pointers of the map's data as `BB:AAAA`: the grid of block ids, the metatile table.
    pub grid: String,
    pub metatiles: String,
    /// The source picture (RGB, one pixel to a game pixel, the whole layer), relative to the
    /// index's folder. Empty when the pack has none (a pack made for sharing holds no picture of
    /// the game): the art is then drawn as it is whatever palette the map is shown in, where a
    /// source picture recolours it.
    #[serde(default)]
    pub source: String,
    /// The CGRAM the art was made in (see `bgart::Entry::palette`), or empty.
    #[serde(default)]
    pub palette: String,
    /// The art tiles (RGB, `TILE * SCALE` pixels square), relative to the index's folder, with
    /// `{x}` and `{y}` the tile's column and row.
    pub tiles: String,
}

/// `bank:address` as `(bank, address)`.
#[must_use]
pub fn parse_pointer(text: &str) -> Option<(u8, u16)> {
    let (bank, address) = text.split_once(':')?;
    Some((u8::from_str_radix(bank, 16).ok()?, u16::from_str_radix(address, 16).ok()?))
}

impl Entry {
    /// Whether this is the layer of the map whose data is `blocks`, `grid` and `metatiles`.
    #[must_use]
    pub fn is_layer_of(&self, blocks: (u8, u8), grid: (u8, u16), metatiles: (u8, u16)) -> bool {
        (self.blocks[0], self.blocks[1]) == blocks
            && parse_pointer(&self.grid) == Some(grid)
            && parse_pointer(&self.metatiles) == Some(metatiles)
    }

    /// The layer's size in game pixels.
    #[must_use]
    pub fn size(&self) -> (usize, usize) {
        (usize::from(self.blocks[0]) * 256, usize::from(self.blocks[1]) * 256)
    }

    /// The tiles of the art: columns and rows.
    #[must_use]
    pub fn tile_grid(&self) -> (usize, usize) {
        let (width, height) = self.size();
        (width.div_ceil(TILE), height.div_ceil(TILE))
    }

    /// The file of the tile in column `x` and row `y`, relative to the index's folder.
    #[must_use]
    pub fn tile_file(&self, x: usize, y: usize) -> String {
        self.tiles.replace("{x}", &x.to_string()).replace("{y}", &y.to_string())
    }
}

impl Index {
    /// The index of `<pack>/art/maps/maps.toml`; empty when the pack has none.
    ///
    /// # Errors
    ///
    /// The file is not a valid index.
    pub fn open(pack: &Path) -> Result<Self, MapArtError> {
        let Ok(text) = std::fs::read_to_string(pack.join(INDEX)) else {
            return Ok(Self::default());
        };
        toml::from_str(&text).map_err(|e| MapArtError::Read(e.to_string()))
    }

    /// The layer of the map whose data is `blocks`, `grid` and `metatiles`, if the index has art for
    /// it.
    #[must_use]
    pub fn find(&self, blocks: (u8, u8), grid: (u8, u16), metatiles: (u8, u16)) -> Option<usize> {
        self.layer.iter().position(|entry| entry.is_layer_of(blocks, grid, metatiles))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> Entry {
        Entry {
            id: "map-00".to_owned(),
            maps: vec![0],
            blocks: [9, 11],
            grid: "06:F360".to_owned(),
            metatiles: "09:EFB0".to_owned(),
            source: "map-00/source.png".to_owned(),
            palette: String::new(),
            tiles: "map-00/t{x}_{y}.png".to_owned(),
        }
    }

    #[test]
    fn a_layer_is_found_by_its_size_and_the_pointers_of_its_data() {
        let index = Index { layer: vec![entry()] };
        assert_eq!(index.find((9, 11), (0x06, 0xF360), (0x09, 0xEFB0)), Some(0));
        assert_eq!(index.find((9, 10), (0x06, 0xF360), (0x09, 0xEFB0)), None);
        assert_eq!(index.find((9, 11), (0x06, 0xF361), (0x09, 0xEFB0)), None);
        assert_eq!(index.find((9, 11), (0x06, 0xF360), (0x08, 0xEFB0)), None);
    }

    #[test]
    fn the_tiles_cover_the_layer_and_have_their_own_files() {
        let entry = entry();
        assert_eq!(entry.size(), (2304, 2816));
        assert_eq!(entry.tile_grid(), (9, 11));
        assert_eq!(entry.tile_file(3, 10), "map-00/t3_10.png");
    }

    #[test]
    fn the_index_round_trips_through_toml_and_a_missing_file_is_empty() {
        let index = Index { layer: vec![entry()] };
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Index::open(dir.path()).unwrap(), Index::default());
        std::fs::create_dir_all(dir.path().join("art/maps")).unwrap();
        std::fs::write(dir.path().join(INDEX), toml::to_string(&index).unwrap()).unwrap();
        assert_eq!(Index::open(dir.path()).unwrap(), index);
        std::fs::write(dir.path().join(INDEX), "layer = 3").unwrap();
        assert!(Index::open(dir.path()).is_err());
    }
}
