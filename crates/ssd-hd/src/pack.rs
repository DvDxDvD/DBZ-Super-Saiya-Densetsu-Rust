//! The HD pack's index and manifest: the assets the converter found in the ROM, each as the keys
//! (content hashes) of its tiles in order, and where its art is. The index holds no ROM bytes and
//! no pixel data; the art is in the pack's image files (PNG or WebP). Format 1.

use rustc_hash::FxHashMap;
use std::path::{Path, PathBuf};

/// The version of the pack and of its index (`hdpack.toml`'s `format`).
pub const FORMAT: u32 = 1;

const MAGIC: &[u8; 8] = b"SSDHDIDX";

/// Why a pack cannot be read.
#[derive(Debug, thiserror::Error)]
pub enum PackError {
    /// The manifest or the index cannot be read.
    #[error("reading the pack at {}: {message}", path.display())]
    Read { path: PathBuf, message: String },
    /// The index is not an index of this format (wrong magic, version, checksum or layout).
    #[error("index: {0}")]
    Index(&'static str),
    /// The manifest says another format or another number of assets than the index holds.
    #[error("manifest: {0}")]
    Manifest(String),
}

/// The content hash of a tile: FNV-1a over its bytes as they sit in VRAM.
#[must_use]
pub fn tile_key(bytes: &[u8]) -> u64 {
    fnv1a(bytes.iter().copied())
}

/// [`tile_key`] of the VRAM words `words` (each word's low byte first).
#[must_use]
pub fn word_key(words: &[u16]) -> u64 {
    fnv1a(words.iter().flat_map(|word| word.to_le_bytes()))
}

fn fnv1a(bytes: impl Iterator<Item = u8>) -> u64 {
    bytes.fold(0xCBF2_9CE4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01B3)
    })
}

/// How an asset's pixels are read: palette indexes resolved through the CGRAM row the game
/// uses (`Indexed`, so fades and palette cycles keep working), or colours as drawn (`Rgba`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Indexed,
    Rgba,
}

/// One asset of the pack: tiles of one bit depth, in the order of the stream they came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    /// The logical id (`<class>/<source>/<n>`): a name, not data, the same in every ROM revision.
    pub id: String,
    /// Bits a pixel takes: 2 or 4.
    pub bpp: u8,
    pub mode: Mode,
    /// Tiles per row of the art image.
    pub columns: u16,
    /// The key of each tile, in order.
    pub keys: Vec<u64>,
    /// The art file, relative to the pack directory.
    pub art: String,
}

/// A tile of the index: the asset and the position of the tile in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TileRef {
    pub asset: u32,
    pub tile: u32,
}

/// The index: the assets, and the tiles that each key names.
#[derive(Debug, Clone)]
pub struct Index {
    assets: Vec<Asset>,
    tiles: FxHashMap<(u8, u64), Vec<TileRef>>,
}

impl Index {
    /// The index of `assets`, in this order.
    #[must_use]
    pub fn new(assets: Vec<Asset>) -> Self {
        let mut tiles: FxHashMap<(u8, u64), Vec<TileRef>> = FxHashMap::default();
        for (asset, data) in assets.iter().enumerate() {
            for (tile, &key) in data.keys.iter().enumerate() {
                let reference = TileRef { asset: asset as u32, tile: tile as u32 };
                tiles.entry((data.bpp, key)).or_default().push(reference);
            }
        }
        Self { assets, tiles }
    }

    #[must_use]
    pub fn assets(&self) -> &[Asset] {
        &self.assets
    }

    /// The asset `index` names.
    ///
    /// # Panics
    ///
    /// `index` is not an asset of this index.
    #[must_use]
    pub fn asset(&self, index: u32) -> &Asset {
        &self.assets[index as usize]
    }

    /// Every tile of `bpp` bits whose key is `key`, in index order (none, one, or several when
    /// the same tile art is in more than one stream).
    #[must_use]
    pub fn candidates(&self, bpp: u8, key: u64) -> &[TileRef] {
        self.tiles.get(&(bpp, key)).map_or(&[], Vec::as_slice)
    }

    /// The index as its file: magic, format, the assets (id, bit depth, mode, columns, art
    /// path and keys, little-endian), then the CRC-32 of everything before it.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        out.extend_from_slice(&FORMAT.to_le_bytes());
        out.extend_from_slice(&(self.assets.len() as u32).to_le_bytes());
        for asset in &self.assets {
            put_str(&mut out, &asset.id);
            out.push(asset.bpp);
            out.push(match asset.mode {
                Mode::Indexed => 0,
                Mode::Rgba => 1,
            });
            out.extend_from_slice(&asset.columns.to_le_bytes());
            put_str(&mut out, &asset.art);
            out.extend_from_slice(&(asset.keys.len() as u32).to_le_bytes());
            for key in &asset.keys {
                out.extend_from_slice(&key.to_le_bytes());
            }
        }
        let crc = ssd_rom::crc32(&out);
        out.extend_from_slice(&crc.to_le_bytes());
        out
    }

    /// Reads an index written by [`Index::encode`].
    ///
    /// # Errors
    ///
    /// The bytes are not an index of [`FORMAT`]: the magic, the version, the checksum or a
    /// length is wrong.
    pub fn decode(bytes: &[u8]) -> Result<Self, PackError> {
        let body_len = bytes.len().checked_sub(4).ok_or(PackError::Index("truncated"))?;
        let (body, crc) = bytes.split_at(body_len);
        if ssd_rom::crc32(body).to_le_bytes() != crc {
            return Err(PackError::Index("checksum"));
        }
        let mut reader = Reader { bytes: body, at: 0 };
        if reader.take(8)? != MAGIC {
            return Err(PackError::Index("not an HD pack index"));
        }
        if reader.u32()? != FORMAT {
            return Err(PackError::Index("another format"));
        }
        let count = reader.u32()?;
        let mut assets = Vec::new();
        for _ in 0..count {
            let id = reader.string()?;
            let bpp = reader.u8()?;
            let mode = match reader.u8()? {
                0 => Mode::Indexed,
                1 => Mode::Rgba,
                _ => return Err(PackError::Index("a mode")),
            };
            let columns = reader.u16()?;
            let art = reader.string()?;
            let keys = (0..reader.u32()?).map(|_| reader.u64()).collect::<Result<Vec<_>, _>>()?;
            assets.push(Asset { id, bpp, mode, columns, keys, art });
        }
        if reader.at != body.len() {
            return Err(PackError::Index("trailing bytes"));
        }
        Ok(Self::new(assets))
    }
}

fn put_str(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(&(text.len() as u16).to_le_bytes());
    out.extend_from_slice(text.as_bytes());
}

/// A cursor over the index bytes; every read is checked.
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, len: usize) -> Result<&[u8], PackError> {
        let end = self.at.checked_add(len).ok_or(PackError::Index("truncated"))?;
        let bytes = self.bytes.get(self.at..end).ok_or(PackError::Index("truncated"))?;
        self.at = end;
        Ok(bytes)
    }

    fn u8(&mut self) -> Result<u8, PackError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, PackError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().expect("two bytes")))
    }

    fn u32(&mut self) -> Result<u32, PackError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().expect("four bytes")))
    }

    fn u64(&mut self) -> Result<u64, PackError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().expect("eight bytes")))
    }

    fn string(&mut self) -> Result<String, PackError> {
        let len = usize::from(self.u16()?);
        String::from_utf8(self.take(len)?.to_vec()).map_err(|_| PackError::Index("a name"))
    }
}

/// `hdpack.toml`: the pack's format, scale, converter version and the ROM it was made from.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Manifest {
    pub format: u32,
    pub scale: u32,
    pub converter: u32,
    /// The CRC-32 of the canonical ROM, as eight hex digits.
    pub rom_crc32: String,
    /// The assets of the index, checked against it on load.
    pub assets: usize,
}

/// A pack on disk: its manifest and its index (the art files are read by the renderer).
#[derive(Debug, Clone)]
pub struct Pack {
    pub dir: PathBuf,
    pub manifest: Manifest,
    pub index: Index,
    /// The approved art the pack draws (empty without `art.bin`).
    pub art: crate::art_index::ArtIndex,
    /// The art of the field maps (empty without `art/maps/maps.toml`).
    pub maps: crate::mapart::Index,
    /// The art of the battle backgrounds (empty without `art/bg/bg.toml`).
    pub bg: crate::bgart::Index,
    /// The art of the Mode 7 plane (empty without `art/mode7/mode7.toml`).
    pub mode7: crate::mode7art::Index,
}

impl Pack {
    /// The newest pack of the ROM `rom_crc32` that the converters left under `cache`
    /// (`<cache>/hd/<crc>/<converter>/`): the highest converter number whose pack opens. `None`
    /// when there is none.
    #[must_use]
    pub fn find(cache: &Path, rom_crc32: u32) -> Option<Self> {
        let folder = cache.join("hd").join(format!("{rom_crc32:08X}"));
        let mut versions: Vec<(u32, PathBuf)> = std::fs::read_dir(&folder)
            .ok()?
            .filter_map(Result::ok)
            .filter_map(|entry| Some((entry.file_name().to_str()?.parse().ok()?, entry.path())))
            .collect();
        versions.sort_by_key(|&(version, _)| std::cmp::Reverse(version));
        versions.into_iter().find_map(|(_, dir)| Self::open(&dir).ok())
    }

    /// Opens the pack in `dir`.
    ///
    /// # Errors
    ///
    /// The manifest or the index cannot be read, or they do not agree.
    pub fn open(dir: &Path) -> Result<Self, PackError> {
        let read = |name: &str| {
            std::fs::read(dir.join(name)).map_err(|e| PackError::Read {
                path: dir.to_path_buf(),
                message: format!("{name}: {e}"),
            })
        };
        let text = String::from_utf8(read("hdpack.toml")?)
            .map_err(|_| PackError::Manifest("not UTF-8".to_owned()))?;
        let manifest: Manifest =
            toml::from_str(&text).map_err(|e| PackError::Manifest(e.to_string()))?;
        if manifest.format != FORMAT {
            return Err(PackError::Manifest(format!("format {} is not {FORMAT}", manifest.format)));
        }
        let index = Index::decode(&read("index.bin")?)?;
        if index.assets().len() != manifest.assets {
            return Err(PackError::Manifest(format!(
                "{} assets, the index has {}",
                manifest.assets,
                index.assets().len()
            )));
        }
        let art = crate::art_index::ArtIndex::open(dir)?;
        let maps = crate::mapart::Index::open(dir)
            .map_err(|e| PackError::Read { path: dir.to_path_buf(), message: e.to_string() })?;
        let bg = crate::bgart::Index::open(dir)
            .map_err(|e| PackError::Read { path: dir.to_path_buf(), message: e.to_string() })?;
        let mode7 = crate::mode7art::Index::open(dir)
            .map_err(|e| PackError::Read { path: dir.to_path_buf(), message: e.to_string() })?;
        Ok(Self { dir: dir.to_path_buf(), manifest, index, art, maps, bg, mode7 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(id: &str, bpp: u8, keys: &[u64]) -> Asset {
        Asset {
            id: id.to_owned(),
            bpp,
            mode: Mode::Indexed,
            columns: 4,
            keys: keys.to_vec(),
            art: format!("base/{id}.png"),
        }
    }

    #[test]
    fn a_key_names_every_tile_of_its_bit_depth() {
        let index =
            Index::new(vec![asset("a", 4, &[1, 2]), asset("b", 4, &[2]), asset("c", 2, &[2])]);
        assert_eq!(
            index.candidates(4, 2),
            [TileRef { asset: 0, tile: 1 }, TileRef { asset: 1, tile: 0 }]
        );
        assert_eq!(index.candidates(2, 2), [TileRef { asset: 2, tile: 0 }]);
        assert_eq!(index.candidates(4, 9), &[] as &[TileRef]);
    }

    #[test]
    fn an_index_reads_back_as_it_was_written_and_a_damaged_one_is_refused() {
        let index =
            Index::new(vec![asset("ui/11:8036/0", 2, &[7, 8]), asset("gfx/16:8000/0", 4, &[])]);
        let bytes = index.encode();
        let back = Index::decode(&bytes).unwrap();
        assert_eq!(back.assets(), index.assets());
        assert_eq!(back.encode(), bytes, "the same index, the same bytes");

        let mut damaged = bytes.clone();
        damaged[20] ^= 1;
        assert!(matches!(Index::decode(&damaged), Err(PackError::Index("checksum"))));
        assert!(matches!(Index::decode(&bytes[..10]), Err(PackError::Index(_))));
    }

    fn write_pack(dir: &std::path::Path, converter: u32) {
        std::fs::create_dir_all(dir).unwrap();
        let manifest = Manifest {
            format: FORMAT,
            scale: 4,
            converter,
            rom_crc32: "A8CF4F2A".to_owned(),
            assets: 0,
        };
        std::fs::write(dir.join("hdpack.toml"), toml::to_string(&manifest).unwrap()).unwrap();
        std::fs::write(dir.join("index.bin"), Index::new(Vec::new()).encode()).unwrap();
    }

    #[test]
    fn the_newest_pack_of_a_rom_is_the_highest_converter_that_opens() {
        let cache = tempfile::tempdir().unwrap();
        let rom = cache.path().join("hd").join("A8CF4F2A");
        assert!(Pack::find(cache.path(), 0xA8CF_4F2A).is_none(), "no pack yet");
        write_pack(&rom.join("1"), 1);
        write_pack(&rom.join("2"), 2);
        assert_eq!(Pack::find(cache.path(), 0xA8CF_4F2A).unwrap().manifest.converter, 2);
        // A pack that does not open (damaged index) is passed over for the older one
        std::fs::write(rom.join("2").join("index.bin"), b"junk").unwrap();
        assert_eq!(Pack::find(cache.path(), 0xA8CF_4F2A).unwrap().manifest.converter, 1);
        assert!(Pack::find(cache.path(), 0x1234_5678).is_none(), "another ROM");
    }

    #[test]
    fn keys_are_fnv1a_of_the_vram_bytes_low_byte_first() {
        assert_eq!(word_key(&[0x3412]), tile_key(&[0x12, 0x34]));
        assert_eq!(tile_key(b""), 0xCBF2_9CE4_8422_2325);
    }
}
