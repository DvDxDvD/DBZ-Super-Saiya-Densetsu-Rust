//! The art index of an HD pack: which approved pictures the renderer draws, and what
//! identifies each one on screen (`art.bin`, written by `xtask hd-import`).
//!
//! Two kinds of entry:
//!
//! * a **picture** is the art of a 32 x 32 portrait or face (16 characters of an asset of the
//!   pack, in the order of the picture's 4 x 4 cells). The renderer finds it through the pack's
//!   tile binding: a character of the asset that is on screen draws its cell of the picture. Several
//!   pictures can cover the same characters (the same drawing in different palettes), and the
//!   palette of the pixel chooses between them;
//! * an **object** is the art of one assembled sprite pose, found by the key of its pixels (the
//!   palette-index array of the pose, or for an effect its colours): the sprites of a frame are
//!   grouped as the capture of the art was, and a group whose key is in the index draws the art.
//!
//! A pose also holds the silhouette of the pose it was made from (a bit for each opaque pixel), so
//! a pose of a frame that is only part of it (a fade-in, a sprite cut by the screen's edge) can be
//! found by the silhouette that holds it.
//!
//! The index holds names, keys, palettes and silhouettes; the art is in the PNG files it names,
//! relative to the pack directory.

use crate::pack::PackError;
use rustc_hash::FxHashMap;
use std::path::Path;

const MAGIC: &[u8; 8] = b"SSDHDART";
/// The version of the file.
pub const ART_FORMAT: u32 = 1;

/// The file name of the art index in a pack directory.
pub const FILE: &str = "art.bin";

/// Up to 16 colours of a palette, and which of them the art's source uses (bit `i` for index `i`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Palette {
    pub mask: u16,
    pub colors: [[u8; 3]; 16],
}

impl Palette {
    /// How far the colours the source used are from the colours in `current` (a 16-colour row as
    /// RGB): the summed channel differences. 0 when they are the same, and 0 for a palette of no
    /// colour.
    #[must_use]
    pub fn distance(&self, current: &[[u8; 3]; 16]) -> u32 {
        let mut total = 0;
        for (index, (mine, theirs)) in self.colors.iter().zip(current).enumerate().skip(1) {
            if self.mask & (1 << index) != 0 {
                total +=
                    mine.iter().zip(theirs).map(|(a, b)| u32::from(a.abs_diff(*b))).sum::<u32>();
            }
        }
        total
    }
}

/// The art of a 32 x 32 picture of an asset of the pack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    /// The name of the art (its file's stem), for reports. It names the art's source, not the
    /// object that shows it: several objects share a set, and a name says nothing about which
    /// half or palette an object uses (the HD menus find the art by the picture's characters,
    /// `Pack::menu_picture`).
    pub name: String,
    /// The logical id of the asset whose characters the picture covers.
    pub asset: String,
    /// The character of the asset in each cell of the 4 x 4 picture, row by row.
    pub tiles: [u16; 16],
    pub palette: Palette,
    /// The art file, relative to the pack directory: RGBA, 32 art pixels to a character.
    pub art: String,
}

/// The art of one sprite pose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Object {
    /// The key of the pose's pixels (see `poses`).
    pub key: u64,
    /// The name of the art (its file's stem), for reports.
    pub id: String,
    /// The art is a greyscale shape that takes its colours from the original pixels.
    pub luma: bool,
    pub palette: Palette,
    /// The pose in game pixels.
    pub width: u16,
    pub height: u16,
    /// The silhouette of the pose: a bit for each pixel of its `width` x `height` box, row by row,
    /// the first pixel in the low bit of the first byte (set where the pose is opaque). Empty when
    /// the source did not say.
    pub mask: Vec<u8>,
    /// The art file, relative to the pack directory: RGBA, four art pixels to a game pixel.
    pub art: String,
}

impl Object {
    /// Whether the art is a sprite group of the field (class 3, whose ids start with `field-`):
    /// found among the field's groups, never among a fighter's or an effect's.
    #[must_use]
    pub fn field(&self) -> bool {
        self.id.starts_with("field-")
    }

    /// Whether pixel `(x, y)` of the pose's box is opaque in its silhouette.
    #[must_use]
    pub fn opaque_at(&self, x: usize, y: usize) -> bool {
        let at = y * usize::from(self.width) + x;
        self.mask.get(at / 8).is_some_and(|byte| byte >> (at % 8) & 1 != 0)
    }

    /// The number of opaque pixels of the silhouette.
    #[must_use]
    pub fn opaque(&self) -> u32 {
        self.mask.iter().map(|byte| byte.count_ones()).sum()
    }
}

/// The poses of one fighter palette (the colours its art was made for), the smallest first: what
/// a partial pose is looked for in.
#[derive(Debug, Clone)]
pub struct PaletteGroup {
    pub palette: Palette,
    /// Indexes of [`ArtIndex::objects`] with a silhouette, by the number of their opaque pixels.
    pub objects: Vec<u32>,
}

/// The art index of a pack.
#[derive(Debug, Clone, Default)]
pub struct ArtIndex {
    pictures: Vec<Picture>,
    objects: Vec<Object>,
    by_asset: FxHashMap<String, Vec<u32>>,
    by_key: FxHashMap<u64, Vec<u32>>,
    /// By the size and the number of opaque pixels of the silhouette, for the poses a palette
    /// changes the key of.
    by_shape: FxHashMap<(u16, u16, u32), Vec<u32>>,
    groups: Vec<PaletteGroup>,
}

impl ArtIndex {
    #[must_use]
    pub fn new(pictures: Vec<Picture>, objects: Vec<Object>) -> Self {
        let mut by_asset: FxHashMap<String, Vec<u32>> = FxHashMap::default();
        for (at, picture) in pictures.iter().enumerate() {
            by_asset.entry(picture.asset.clone()).or_default().push(at as u32);
        }
        let mut by_key: FxHashMap<u64, Vec<u32>> = FxHashMap::default();
        for (at, object) in objects.iter().enumerate() {
            by_key.entry(object.key).or_default().push(at as u32);
        }
        let mut by_shape: FxHashMap<(u16, u16, u32), Vec<u32>> = FxHashMap::default();
        for (at, object) in objects.iter().enumerate() {
            if !object.mask.is_empty() {
                by_shape
                    .entry((object.width, object.height, object.opaque()))
                    .or_default()
                    .push(at as u32);
            }
        }
        // The poses with a silhouette and a palette, by palette: the colours of the used indexes
        let mut groups: Vec<PaletteGroup> = Vec::new();
        let mut order: Vec<(u32, u32)> = objects
            .iter()
            .enumerate()
            .filter(|(_, object)| !object.mask.is_empty() && object.palette.mask != 0)
            .map(|(at, object)| (object.opaque(), at as u32))
            .collect();
        order.sort_unstable();
        for (_, at) in order {
            let palette = objects[at as usize].palette;
            let same = |group: &PaletteGroup| {
                group.palette.mask == palette.mask
                    && (0..16).all(|i| {
                        palette.mask & (1 << i) == 0 || group.palette.colors[i] == palette.colors[i]
                    })
            };
            match groups.iter_mut().find(|group| same(group)) {
                Some(group) => group.objects.push(at),
                None => groups.push(PaletteGroup { palette, objects: vec![at] }),
            }
        }
        Self { pictures, objects, by_asset, by_key, by_shape, groups }
    }

    #[must_use]
    pub fn pictures(&self) -> &[Picture] {
        &self.pictures
    }

    #[must_use]
    pub fn objects(&self) -> &[Object] {
        &self.objects
    }

    /// The pictures that cover characters of the asset `id`, as indexes of [`ArtIndex::pictures`].
    #[must_use]
    pub fn pictures_of(&self, id: &str) -> &[u32] {
        self.by_asset.get(id).map_or(&[], Vec::as_slice)
    }

    /// The fighters' poses with a silhouette, by palette.
    #[must_use]
    pub fn palette_groups(&self) -> &[PaletteGroup] {
        &self.groups
    }

    /// The objects whose silhouette is `width` x `height` with `opaque` opaque pixels (several can
    /// have another shape of that size and count: the caller compares the silhouettes).
    #[must_use]
    pub fn objects_shaped(&self, width: u16, height: u16, opaque: u32) -> &[u32] {
        self.by_shape.get(&(width, height, opaque)).map_or(&[], Vec::as_slice)
    }

    /// The objects with key `key`, as indexes of [`ArtIndex::objects`].
    #[must_use]
    pub fn objects_with(&self, key: u64) -> &[u32] {
        self.by_key.get(&key).map_or(&[], Vec::as_slice)
    }

    /// The art index read from `dir`; empty when the pack has none.
    ///
    /// # Errors
    ///
    /// The file exists and is damaged.
    pub fn open(dir: &Path) -> Result<Self, PackError> {
        match std::fs::read(dir.join(FILE)) {
            Ok(bytes) => Self::decode(&bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => {
                Err(PackError::Read { path: dir.to_path_buf(), message: format!("{FILE}: {e}") })
            }
        }
    }

    /// The index as its file: magic, version, the pictures, the objects, and the CRC-32 of
    /// everything before it (little-endian).
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        out.extend_from_slice(&ART_FORMAT.to_le_bytes());
        out.extend_from_slice(&(self.pictures.len() as u32).to_le_bytes());
        for picture in &self.pictures {
            put_str(&mut out, &picture.name);
            put_str(&mut out, &picture.asset);
            for tile in picture.tiles {
                out.extend_from_slice(&tile.to_le_bytes());
            }
            put_palette(&mut out, &picture.palette);
            put_str(&mut out, &picture.art);
        }
        out.extend_from_slice(&(self.objects.len() as u32).to_le_bytes());
        for object in &self.objects {
            out.extend_from_slice(&object.key.to_le_bytes());
            put_str(&mut out, &object.id);
            out.push(u8::from(object.luma));
            put_palette(&mut out, &object.palette);
            out.extend_from_slice(&object.width.to_le_bytes());
            out.extend_from_slice(&object.height.to_le_bytes());
            out.extend_from_slice(&(object.mask.len() as u32).to_le_bytes());
            out.extend_from_slice(&object.mask);
            put_str(&mut out, &object.art);
        }
        let crc = ssd_rom::crc32(&out);
        out.extend_from_slice(&crc.to_le_bytes());
        out
    }

    /// Reads an index written by [`ArtIndex::encode`].
    ///
    /// # Errors
    ///
    /// The bytes are not an art index of this version.
    pub fn decode(bytes: &[u8]) -> Result<Self, PackError> {
        let body_len = bytes.len().checked_sub(4).ok_or(PackError::Index("art: truncated"))?;
        let (body, crc) = bytes.split_at(body_len);
        if ssd_rom::crc32(body).to_le_bytes() != crc {
            return Err(PackError::Index("art: checksum"));
        }
        let mut reader = Reader { bytes: body, at: 0 };
        if reader.take(8)? != MAGIC {
            return Err(PackError::Index("not an art index"));
        }
        if reader.u32()? != ART_FORMAT {
            return Err(PackError::Index("art: another version"));
        }
        let mut pictures = Vec::new();
        for _ in 0..reader.u32()? {
            let name = reader.string()?;
            let asset = reader.string()?;
            let mut tiles = [0_u16; 16];
            for tile in &mut tiles {
                *tile = reader.u16()?;
            }
            let palette = reader.palette()?;
            let art = reader.string()?;
            pictures.push(Picture { name, asset, tiles, palette, art });
        }
        let mut objects = Vec::new();
        for _ in 0..reader.u32()? {
            let key = reader.u64()?;
            let id = reader.string()?;
            let luma = reader.u8()? != 0;
            let palette = reader.palette()?;
            let width = reader.u16()?;
            let height = reader.u16()?;
            let length = reader.u32()? as usize;
            let mask = reader.take(length)?.to_vec();
            let art = reader.string()?;
            objects.push(Object { key, id, luma, palette, width, height, mask, art });
        }
        if reader.at != body.len() {
            return Err(PackError::Index("art: trailing bytes"));
        }
        Ok(Self::new(pictures, objects))
    }
}

fn put_str(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(&(text.len() as u16).to_le_bytes());
    out.extend_from_slice(text.as_bytes());
}

fn put_palette(out: &mut Vec<u8>, palette: &Palette) {
    out.extend_from_slice(&palette.mask.to_le_bytes());
    for color in palette.colors {
        out.extend_from_slice(&color);
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, len: usize) -> Result<&[u8], PackError> {
        let end = self.at.checked_add(len).ok_or(PackError::Index("art: truncated"))?;
        let bytes = self.bytes.get(self.at..end).ok_or(PackError::Index("art: truncated"))?;
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
        String::from_utf8(self.take(len)?.to_vec()).map_err(|_| PackError::Index("art: a name"))
    }

    fn palette(&mut self) -> Result<Palette, PackError> {
        let mask = self.u16()?;
        let mut colors = [[0_u8; 3]; 16];
        for color in &mut colors {
            color.copy_from_slice(self.take(3)?);
        }
        Ok(Palette { mask, colors })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn palette(seed: u8) -> Palette {
        let mut colors = [[0_u8; 3]; 16];
        for (index, color) in colors.iter_mut().enumerate() {
            *color = [seed, index as u8, 3];
        }
        Palette { mask: 0b1010, colors }
    }

    fn index() -> ArtIndex {
        ArtIndex::new(
            vec![Picture {
                name: "obj-00".to_owned(),
                asset: "portrait/16:8000/4bpp".to_owned(),
                tiles: std::array::from_fn(|n| n as u16),
                palette: palette(1),
                art: "art/picture/a.png".to_owned(),
            }],
            vec![
                Object {
                    key: 0xAB,
                    id: "battle-x-000".to_owned(),
                    luma: false,
                    palette: palette(2),
                    width: 20,
                    height: 30,
                    mask: vec![0b0000_0101, 0xFF],
                    art: "art/object/a.png".to_owned(),
                },
                Object {
                    key: 0xAB,
                    id: "battle-y-000".to_owned(),
                    luma: true,
                    palette: Palette::default(),
                    width: 4,
                    height: 5,
                    mask: Vec::new(),
                    art: "art/object/b.png".to_owned(),
                },
            ],
        )
    }

    #[test]
    fn an_art_index_reads_back_as_written_and_a_damaged_one_is_refused() {
        let index = index();
        let bytes = index.encode();
        let back = ArtIndex::decode(&bytes).unwrap();
        assert_eq!(back.pictures(), index.pictures());
        assert_eq!(back.objects(), index.objects());
        assert_eq!(back.encode(), bytes);
        let mut damaged = bytes.clone();
        damaged[12] ^= 1;
        assert!(ArtIndex::decode(&damaged).is_err());
        assert!(ArtIndex::decode(&bytes[..9]).is_err());
    }

    #[test]
    fn a_silhouette_says_where_a_pose_is_opaque_and_the_poses_are_grouped_by_palette() {
        let index = index();
        let object = &index.objects()[0];
        assert!(object.opaque_at(0, 0) && !object.opaque_at(1, 0) && object.opaque_at(2, 0));
        assert!(object.opaque_at(8, 0), "the second byte is the next eight pixels");
        assert_eq!(object.opaque(), 2 + 8);
        assert!(!index.objects()[1].opaque_at(0, 0), "no silhouette: nothing is opaque");
        // Only the pose with a silhouette and a palette is in a group
        let groups = index.palette_groups();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].objects, [0]);
    }

    #[test]
    fn the_lookups_name_every_entry_of_an_asset_or_a_key() {
        let index = index();
        assert_eq!(index.pictures_of("portrait/16:8000/4bpp"), [0]);
        assert_eq!(index.pictures_of("portrait/16:9999/4bpp"), &[] as &[u32]);
        assert_eq!(index.objects_with(0xAB), [0, 1]);
        assert_eq!(index.objects_with(1), &[] as &[u32]);
    }

    #[test]
    fn a_palette_is_compared_on_the_colours_its_source_used() {
        let used = palette(1);
        assert_eq!(used.distance(&used.colors), 0);
        let mut other = used.colors;
        other[1][0] += 5;
        other[2][2] += 7; // index 2 is not in the mask (bits 1 and 3)
        other[3][1] += 2;
        assert_eq!(used.distance(&other), 5 + 2);
    }
}
