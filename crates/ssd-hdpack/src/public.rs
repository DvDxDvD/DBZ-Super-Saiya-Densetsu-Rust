//! The format of a shared HD art pack (`hd-pack.zip`, or the same files in a folder), the art a
//! player puts next to the game and the first launch installs for the ROM they own.
//!
//! The pack holds the art and what finds it, and nothing the game's ROM holds:
//!
//! * `hd-pack.toml`: the manifest ([`Manifest`]).
//! * `class1/`, `class2/`, `class3/`, `cards/`, `bg/`, `maps/`, `mode7/`: the art, one folder
//!   per kind, each with an `index.json` that `import` reads (the layout of the art store, see
//!   [`crate::import`]); the pictures are WebP files (lossless, and lossy for the maps).
//! * `masks.bin`: the outlines of the poses ([`Masks`]), which let a pose cut by the screen's edge
//!   or half faded in find the whole pose.
//!
//! The pixels, tile tables and colours of the ROM are not in it: the first launch builds them from
//! the player's ROM (`convert`), and the art is found by hashes of that data. A palette of at
//! most sixteen colours is the only colour data an entry carries.

use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The manifest's file name.
pub const MANIFEST: &str = "hd-pack.toml";

/// The outlines' file name.
pub const MASKS: &str = "masks.bin";

/// The palettes the backgrounds' and maps' art was made in.
pub const REF_PALETTES: &str = "ref-palettes.json";

/// The version of the pack's layout.
pub const FORMAT: u32 = 1;

/// `hd-pack.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// The version of the layout ([`FORMAT`]).
    pub format: u32,
    /// What the pack is.
    pub name: String,
    /// The pack's own revision (a short text), shown to the player.
    pub revision: String,
    /// The CRC-32 of the canonical ROM the pack's keys are made for, eight hex digits.
    pub rom_crc32: String,
    /// A digest of the pack's files (their names and sizes), eight hex digits: the install says
    /// it is done when the installed pack has this digest.
    pub digest: String,
    /// What the pack holds, by kind: the number of art files of each folder.
    #[serde(default)]
    pub counts: BTreeMap<String, usize>,
}

impl Manifest {
    /// The manifest of `text`.
    ///
    /// # Errors
    ///
    /// The text is not a manifest of this layout.
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        let manifest: Self = toml::from_str(text).context("hd-pack.toml")?;
        anyhow::ensure!(
            manifest.format == FORMAT,
            "this HD pack is format {}, this game reads format {FORMAT}",
            manifest.format
        );
        Ok(manifest)
    }
}

/// The digest of a list of files: the CRC-32 of their names and sizes, in name order.
#[must_use]
pub fn digest(files: &BTreeMap<String, u64>) -> String {
    let mut text = Vec::new();
    for (name, size) in files {
        text.extend_from_slice(name.as_bytes());
        text.push(0);
        text.extend_from_slice(&size.to_le_bytes());
    }
    format!("{:08X}", ssd_rom::crc32(&text))
}

const MASKS_MAGIC: &[u8; 8] = b"SSDMASK1";

/// The outlines of the poses by their key: for each, the size of the pose's box and a bit for each
/// pixel of it (row by row, the first pixel in the low bit of the first byte), set where the pose
/// is opaque. An outline holds no colour.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Masks {
    poses: BTreeMap<u64, (u16, u16, Vec<u8>)>,
}

impl Masks {
    /// Adds the outline of the pose `key`.
    ///
    /// # Panics
    ///
    /// `bits` is not the bytes of a `width` x `height` box.
    pub fn insert(&mut self, key: u64, width: u16, height: u16, bits: Vec<u8>) {
        assert_eq!(bits.len(), (usize::from(width) * usize::from(height)).div_ceil(8));
        self.poses.insert(key, (width, height, bits));
    }

    /// The outline of the pose `key`: its width, its height and its bits.
    #[must_use]
    pub fn get(&self, key: u64) -> Option<(u16, u16, &[u8])> {
        self.poses.get(&key).map(|(w, h, bits)| (*w, *h, bits.as_slice()))
    }

    /// The number of outlines.
    #[must_use]
    pub fn len(&self) -> usize {
        self.poses.len()
    }

    /// Whether there is no outline.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.poses.is_empty()
    }

    /// Every outline the file holds: its key, the width and height of its box and its bits (a bit
    /// per pixel, the first pixel in the low bit of the first byte).
    pub fn entries(&self) -> impl Iterator<Item = (u64, u16, u16, &[u8])> {
        self.poses
            .iter()
            .map(|(key, (width, height, bits))| (*key, *width, *height, bits.as_slice()))
    }

    /// The file: the magic, the count, then for each outline its key, size and bits, and the
    /// CRC-32 of all of it.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = MASKS_MAGIC.to_vec();
        out.extend_from_slice(&(self.poses.len() as u32).to_le_bytes());
        for (key, (width, height, bits)) in &self.poses {
            out.extend_from_slice(&key.to_le_bytes());
            out.extend_from_slice(&width.to_le_bytes());
            out.extend_from_slice(&height.to_le_bytes());
            out.extend_from_slice(bits);
        }
        let crc = ssd_rom::crc32(&out);
        out.extend_from_slice(&crc.to_le_bytes());
        out
    }

    /// Reads a file of [`Masks::encode`].
    ///
    /// # Errors
    ///
    /// The bytes are not such a file (the magic, the checksum or a length is wrong).
    pub fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        let body_len = bytes.len().checked_sub(4).context("masks.bin: truncated")?;
        let (body, crc) = bytes.split_at(body_len);
        anyhow::ensure!(ssd_rom::crc32(body).to_le_bytes() == crc, "masks.bin: checksum");
        anyhow::ensure!(body.starts_with(MASKS_MAGIC), "masks.bin: not an outlines file");
        let mut at = MASKS_MAGIC.len();
        let mut take = |len: usize| -> anyhow::Result<&[u8]> {
            let end = at.checked_add(len).filter(|&end| end <= body.len());
            let end = end.context("masks.bin: truncated")?;
            let slice = &body[at..end];
            at = end;
            Ok(slice)
        };
        let count = u32::from_le_bytes(take(4)?.try_into()?);
        let mut masks = Self::default();
        for _ in 0..count {
            let key = u64::from_le_bytes(take(8)?.try_into()?);
            let width = u16::from_le_bytes(take(2)?.try_into()?);
            let height = u16::from_le_bytes(take(2)?.try_into()?);
            let bits = take((usize::from(width) * usize::from(height)).div_ceil(8))?.to_vec();
            masks.poses.insert(key, (width, height, bits));
        }
        anyhow::ensure!(at == body.len(), "masks.bin: trailing bytes");
        Ok(masks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_outlines_round_trip_and_a_damaged_file_is_refused() {
        let mut masks = Masks::default();
        masks.insert(7, 3, 3, vec![0b1010_1010, 0b1]);
        masks.insert(u64::MAX, 8, 1, vec![0xFF]);
        let bytes = masks.encode();
        assert_eq!(Masks::decode(&bytes).unwrap(), masks);
        assert_eq!(masks.get(7), Some((3, 3, &[0b1010_1010, 0b1][..])));
        assert_eq!(masks.get(8), None);
        let mut damaged = bytes.clone();
        damaged[14] ^= 1;
        assert!(Masks::decode(&damaged).is_err());
        assert!(Masks::decode(&bytes[..bytes.len() - 1]).is_err());
        assert!(Masks::decode(b"").is_err());
    }

    #[test]
    fn a_manifest_of_another_layout_is_refused() {
        let text = "format = 1\nname = \"x\"\nrevision = \"1\"\nrom_crc32 = \"A8CF4F2A\"\ndigest = \"0\"\n";
        assert!(Manifest::parse(text).is_ok());
        assert!(Manifest::parse(&text.replace("format = 1", "format = 2")).is_err());
        assert!(Manifest::parse("nonsense").is_err());
    }

    #[test]
    fn the_digest_follows_names_and_sizes() {
        let mut files = BTreeMap::new();
        files.insert("a".to_owned(), 1);
        let one = digest(&files);
        files.insert("b".to_owned(), 2);
        assert_ne!(one, digest(&files));
        assert_eq!(digest(&files), digest(&files.clone()));
    }
}
