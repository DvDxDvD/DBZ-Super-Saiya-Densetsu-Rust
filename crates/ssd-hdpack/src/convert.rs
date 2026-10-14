//! `hd-convert`: builds the HD pack from the cached ROM, under the cache directory (never in the
//! repository). The pack holds the index of every tile the game can show (the keys of
//! `ssd_hd::pack`), the base art of each asset (the pixel-art filter of `tiles`) and the template
//! sheets for art production. Nothing in it is written from anything but the ROM, so the same
//! ROM and the same converter give the same bytes.
//!
//! The streams are found in the ROM itself: from the start of each run (`RUNS`, the first stream of
//! each group of streams that `gfx-log` shows the replays load), following the streams one after
//! the other while each decodes to whole tiles of its bit depth (`bits_of`). The English UI tile
//! set is the overlay `11:8036` (uncompressed) that the game writes over the decoded `08:8000`, so
//! that overlay, not the Japanese stream, is the asset.

use crate::tiles;
use anyhow::Context;
use ssd_catalogue::{RomView, graphics};
use ssd_hd::pack::{Asset, Index, Manifest, Mode, tile_key};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

/// The version of the converter: the pack's directory name, so a pack made by another one is
/// never mistaken for this one's.
pub const CONVERTER: u32 = 1;

/// The English UI tile set: 384 tiles of 2 bits, written over the decoded `08:8000` by the game.
const UI_OVERLAY: u32 = 0x11_8036;
const UI_OVERLAY_LEN: usize = 0x1800;

/// The tiles per row of an asset's art and of its sheet.
const COLUMNS: usize = 16;

/// Where a stream of the ROM is: the bank and offset of its first byte, as a 24-bit address.
fn address_of(offset: usize) -> u32 {
    let bank = offset / 0x8000;
    (bank as u32) << 16 | (0x8000 + (offset % 0x8000)) as u32
}

/// A stream found in the ROM, decoded and in the SNES tile layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// The address the stream starts at (its bank and offset).
    pub source: u32,
    /// What it is, for the production gate and the sheets' folders (see [`class_of`]).
    pub class: &'static str,
    /// Bits a tile takes: 2 or 4.
    pub bpp: u8,
    /// The stream's bytes in the SNES tile layout: whole tiles.
    pub data: Vec<u8>,
}

impl Found {
    /// The logical id of the asset: `<class>/<bank>:<offset>/<bits>bpp`. A stream read at two bit
    /// depths is two assets, and the id tells them apart.
    #[must_use]
    pub fn id(&self) -> String {
        format!(
            "{}/{:02X}:{:04X}/{}bpp",
            self.class,
            self.source >> 16,
            self.source & 0xFFFF,
            self.bpp
        )
    }

    /// The file name of the asset's art, without its folder: `<bank>-<offset>-<bits>bpp.png`.
    #[must_use]
    pub fn file(&self) -> String {
        format!("{:02X}-{:04X}-{}bpp.png", self.source >> 16, self.source & 0xFFFF, self.bpp)
    }

    /// The bytes of each tile, in order.
    #[must_use]
    pub fn tiles(&self) -> Vec<&[u8]> {
        self.data.chunks_exact(usize::from(self.bpp) * 8).collect()
    }
}

/// What a bank's graphics are for. The classes organise the pack and the production gate; they
/// do not decide how a tile is read.
#[must_use]
pub fn class_of(source: u32) -> &'static str {
    match source >> 16 {
        0x08 if source & 0xFFFF == 0x8000 => "ui",
        0x08 => "field",
        0x0F => "sprite",
        0x16 => "portrait",
        0x09 | 0x0A | 0x0E | 0x11..=0x13 => "picture",
        0x00 => "title",
        0x07 | 0x0C | 0x0D | 0x10 | 0x14 | 0x15 | 0x17 | 0x1B..=0x1F => "fighter",
        _ => "other",
    }
}

/// Decodes the LZ stream at `offset` and puts its bytes in the SNES tile layout: the bytes the
/// stream takes from the ROM, and its data.
fn stream_at(rom: &[u8], offset: usize) -> Option<(usize, Vec<u8>)> {
    let stream = graphics::lz_decode(rom, offset)?;
    let mut data = stream.data;
    graphics::regroup(&mut data);
    Some((stream.consumed, data))
}

/// Finds the streams of the ROM: the UI overlay, and every stream from the starts of the runs
/// ([`RUNS`]), following each one after the other while the next decodes to whole tiles of its
/// bit depth ([`bits_of`]).
///
/// # Errors
///
/// The image is not a `LoROM` ROM of at least 1 MiB.
pub fn discover(rom: &[u8]) -> anyhow::Result<Vec<Found>> {
    let view = RomView::new(rom)?;
    let mut found = BTreeMap::new();
    let overlay = view.offset(UI_OVERLAY).context("the UI overlay is not in the ROM")?;
    let overlay_data = rom
        .get(overlay..overlay + UI_OVERLAY_LEN)
        .context("the UI overlay runs past the ROM")?
        .to_vec();
    found.insert(UI_OVERLAY, Found { source: UI_OVERLAY, class: "ui", bpp: 2, data: overlay_data });
    for &start in RUNS {
        let mut at = view.offset(start).context("a run starts outside the ROM")?;
        while let Some((consumed, data)) = stream_at(rom, at) {
            let source = address_of(at);
            let bpp = bits_of(source);
            if data.is_empty() || data.len() % (usize::from(bpp) * 8) != 0 {
                break;
            }
            // The decoded UI stream is superseded by the overlay: the run goes on past it
            if source != UI_STREAM_ADDRESS {
                let class = class_of(source);
                found.entry(source).or_insert(Found { source, class, bpp, data });
            }
            at += consumed;
        }
    }
    Ok(found.into_values().collect())
}

/// The bit depth of the tiles of the stream at `source`: 2 for the battle pictures of banks
/// `$0E` and `$11` (the attack pictures the animation scene draws in mode 0, where every layer
/// has 2 bits) and for the title's kana font, 4 for the rest. Measured on the reference replays:
/// `hd-coverage --assets` binds these streams at this depth and no other stream at 2 bits.
#[must_use]
pub fn bits_of(source: u32) -> u8 {
    if TWO_BIT_BANKS.contains(&((source >> 16) as u8)) || TWO_BIT_STREAMS.contains(&source) {
        2
    } else {
        4
    }
}

/// The banks whose streams are all 2-bit tiles (see [`bits_of`]).
const TWO_BIT_BANKS: &[u8] = &[0x0E, 0x11];

/// The 2-bit streams outside [`TWO_BIT_BANKS`].
const TWO_BIT_STREAMS: &[u32] = &[0x00_DF39];

/// The address of the decoded UI stream, which the overlay replaces (it is not an asset).
const UI_STREAM_ADDRESS: u32 = 0x08_8000;

/// The starts of the runs of the ROM's graphics: the first stream of each group that follows
/// the one before it in the ROM (the groups of `docs/re/data/graphics.md`, "What is stored
/// where", and the streams the reference replays load). A run ends where the next stream
/// stops decoding to whole tiles, which is where the next group starts.
const RUNS: &[u32] = &[
    0x00_C7DA, // 5 streams
    0x00_EB97, // 3 streams
    0x00_F504, // 1 streams
    0x07_D824, // 13 streams
    0x07_E0CC, // 2 streams
    0x07_E342, // 7 streams
    0x07_EB37, // 1 streams
    0x08_8000, // 1 streams
    0x08_8D4C, // 1 streams
    0x08_BF20, // 2 streams
    0x08_EB84, // 1 streams
    0x09_8000, // 2 streams
    0x09_B7B6, // 1 streams
    0x0A_8000, // 3 streams
    0x0A_86DA, // 8 streams
    0x0A_A096, // 2 streams
    0x0A_AAD8, // 1 streams
    0x0A_AF77, // 1 streams
    0x0A_C35F, // 4 streams
    0x0A_CDE1, // 2 streams
    0x0A_D3B7, // 2 streams
    0x0A_D9B7, // 4 streams
    0x0A_E1D0, // 1 streams
    0x0A_E400, // 1 streams
    0x0A_F0A0, // 1 streams
    0x0A_F610, // 1 streams
    0x0C_DB00, // 1 streams
    0x0C_E194, // 1 streams
    0x0D_86A3, // 1 streams
    0x0D_8B33, // 1 streams
    0x0D_8E34, // 3 streams
    0x0D_906B, // 2 streams
    0x0D_91C8, // 2 streams
    0x0D_9574, // 3 streams
    0x0D_98D8, // 1 streams
    0x0D_9AFF, // 1 streams
    0x0D_A423, // 1 streams
    0x0D_A902, // 6 streams
    0x0D_B9E6, // 4 streams
    0x0D_C4D9, // 14 streams
    0x0D_D77F, // 6 streams
    0x0D_E3F4, // 2 streams
    0x0D_F3FF, // 15 streams
    0x0D_FA71, // 3 streams
    0x0D_FF28, // 1 streams
    0x0E_8000, // 1 streams
    0x0E_91F0, // 4 streams
    0x0E_BA2C, // 5 streams
    0x0E_EE45, // 1 streams
    0x0F_8000, // 6 streams
    0x0F_9F59, // 1 streams
    0x0F_A0E2, // 1 streams
    0x0F_A241, // 10 streams
    0x0F_B8E1, // 1 streams
    0x0F_BCD6, // 6 streams
    0x0F_C8CF, // 6 streams
    0x0F_D64D, // 2 streams
    0x10_9693, // 1 streams
    0x10_9B69, // 1 streams
    0x10_CD6A, // 14 streams
    0x10_DD32, // 1 streams
    0x10_F08E, // 4 streams
    0x10_F295, // 6 streams
    0x11_C000, // 2 streams
    0x12_8000, // 1 streams
    0x12_8612, // 3 streams
    0x12_9BFC, // 1 streams
    0x12_A992, // 1 streams
    0x12_B9DC, // 1 streams
    0x12_D690, // 1 streams
    0x12_E7CC, // 1 streams
    0x12_F8C6, // 1 streams
    0x13_82A6, // 2 streams
    0x13_A594, // 1 streams
    0x13_A798, // 1 streams
    0x13_ABE2, // 1 streams
    0x13_AE64, // 2 streams
    0x13_C87D, // 1 streams
    0x13_E5A2, // 2 streams
    0x13_EA49, // 1 streams
    0x14_A039, // 13 streams
    0x14_A80B, // 2 streams
    0x14_AA72, // 3 streams
    0x14_AC84, // 2 streams
    0x14_ADE5, // 2 streams
    0x14_B7E3, // 1 streams
    0x14_BBA8, // 4 streams
    0x14_C286, // 13 streams
    0x14_D038, // 1 streams
    0x14_DAFD, // 1 streams
    0x14_E22F, // 1 streams
    0x14_E8D5, // 3 streams
    0x14_EC26, // 12 streams
    0x14_F679, // 4 streams
    0x15_D522, // 1 streams
    0x15_D9D7, // 3 streams
    0x15_E0DD, // 4 streams
    0x15_E6F5, // 15 streams
    0x16_8000, // 2 streams
    0x16_872A, // 8 streams
    0x16_A780, // 1 streams
    0x16_AF64, // 4 streams
    0x16_C60A, // 4 streams
    0x16_D941, // 1 streams
    0x16_E3EA, // 1 streams
    0x16_F1E8, // 1 streams
    0x16_FB8D, // 1 streams
    0x17_86AC, // 3 streams
    0x17_8D3E, // 1 streams
    0x1B_A642, // 2 streams
    0x1B_A960, // 9 streams
    0x1B_AE3A, // 6 streams
    0x1B_B1B9, // 11 streams
    0x1B_C5BF, // 1 streams
    0x1B_C95A, // 5 streams
    0x1B_CF4E, // 3 streams
    0x1B_D33F, // 1 streams
    0x1B_DA41, // 11 streams
    0x1C_8C6E, // 1 streams
    0x1C_9000, // 20 streams
    0x1C_A57A, // 15 streams
    0x1C_C88F, // 2 streams
    0x1C_D1E0, // 14 streams
    0x1C_DF32, // 8 streams
    0x1C_E27C, // 1 streams
    0x1C_E384, // 1 streams
    0x1C_EC73, // 6 streams
    0x1C_EDD0, // 22 streams
    0x1C_F72F, // 9 streams
    0x1C_FB39, // 6 streams
    0x1C_FE50, // 2 streams
    0x1C_FFE0, // 1 streams
    0x1D_9729, // 1 streams
    0x1D_98CE, // 9 streams
    0x1D_9E0E, // 2 streams
    0x1D_9FE3, // 3 streams
    0x1D_A2BF, // 6 streams
    0x1D_B482, // 2 streams
    0x1D_B954, // 9 streams
    0x1D_C2CE, // 6 streams
    0x1D_C9AF, // 1 streams
    0x1D_CC38, // 8 streams
    0x1D_CEEC, // 2 streams
    0x1D_D27E, // 1 streams
    0x1D_E057, // 2 streams
    0x1D_F0A7, // 7 streams
    0x1E_CF34, // 1 streams
    0x1E_D1EB, // 14 streams
    0x1E_DE87, // 3 streams
    0x1E_E2FD, // 4 streams
    0x1E_E623, // 1 streams
    0x1E_FD91, // 1 streams
    0x1F_92BF, // 1 streams
    0x1F_9B21, // 1 streams
    0x1F_B754, // 13 streams
    0x1F_C2E7, // 2 streams
    0x1F_CDE9, // 16 streams
    0x1F_D8D5, // 1 streams
];

/// The pack's directory under `cache`: `<cache>/hd/<ROM CRC-32>/<converter>`.
#[must_use]
pub fn pack_dir(cache: &Path, rom_crc32: u32) -> PathBuf {
    cache.join("hd").join(format!("{rom_crc32:08X}")).join(CONVERTER.to_string())
}

/// The asset of a found stream: its keys, and where its base art is.
#[must_use]
pub fn asset_of(item: &Found) -> Asset {
    let keys = item.tiles().iter().map(|tile| tile_key(tile)).collect();
    Asset {
        id: item.id(),
        bpp: item.bpp,
        mode: Mode::Indexed,
        columns: item.tiles().len().min(COLUMNS) as u16,
        keys,
        art: format!("base/{}/{}", item.class, item.file()),
    }
}

/// Writes the base art and, when `sheets`, the template sheets of `item` into `dir`.
fn write_art(dir: &Path, item: &Found, sheets: bool) -> anyhow::Result<()> {
    let indexed: Vec<[u8; 64]> =
        item.tiles().iter().map(|tile| tiles::decode_tile(tile, item.bpp)).collect();
    let (width, height, pixels) = tiles::base_image(&indexed, indexed.len().min(COLUMNS));
    let base = dir.join("base").join(item.class).join(item.file());
    ssd_hd::png::write_indexed(
        &base,
        width as u32,
        height as u32,
        &pixels,
        &tiles::grey_palette(),
        Some(0),
    )
    .with_context(|| format!("writing {}", base.display()))?;
    if !sheets {
        return Ok(());
    }
    let (width, height, pixels) = tiles::sheet_indexed(&indexed, indexed.len().min(COLUMNS));
    let sheet = dir.join("sheets").join(item.class).join(item.file());
    ssd_hd::png::write_indexed(
        &sheet,
        width as u32,
        height as u32,
        &pixels,
        &tiles::sheet_palette(),
        None,
    )
    .with_context(|| format!("writing {}", sheet.display()))
}

/// Builds the pack for `rom` in `dir`, replacing what is there, and reports what it holds.
///
/// # Errors
///
/// The ROM is not a `LoROM` image, or the pack cannot be written.
pub fn run(rom: &[u8], dir: &Path, out: &mut dyn Write) -> anyhow::Result<()> {
    run_with(rom, dir, true, out)
}

/// [`run`], with the template sheets (`sheets`, for making art; a player's pack has no use for
/// them) or without.
///
/// # Errors
///
/// The ROM is not a `LoROM` image, or the pack cannot be written.
pub fn run_with(rom: &[u8], dir: &Path, sheets: bool, out: &mut dyn Write) -> anyhow::Result<()> {
    let found = discover(rom)?;
    if dir.exists() {
        std::fs::remove_dir_all(dir).with_context(|| format!("clearing {}", dir.display()))?;
    }
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let mut assets = Vec::with_capacity(found.len());
    for item in &found {
        write_art(dir, item, sheets)?;
        assets.push(asset_of(item));
    }
    let index = Index::new(assets);
    let rom_crc32 = ssd_rom::crc32(rom);
    let manifest = Manifest {
        format: ssd_hd::pack::FORMAT,
        scale: tiles::SCALE as u32,
        converter: CONVERTER,
        rom_crc32: format!("{rom_crc32:08X}"),
        assets: index.assets().len(),
    };
    std::fs::write(dir.join("index.bin"), index.encode())
        .with_context(|| format!("writing {}", dir.join("index.bin").display()))?;
    std::fs::write(dir.join("hdpack.toml"), toml::to_string(&manifest).context("the manifest")?)
        .with_context(|| format!("writing {}", dir.join("hdpack.toml").display()))?;
    report(out, dir, &found, &index)?;
    Ok(())
}

/// The counts of the pack, per class, and the keys that more than one tile of the index names
/// (the run rule decides those).
fn report(out: &mut dyn Write, dir: &Path, found: &[Found], index: &Index) -> anyhow::Result<()> {
    let mut classes: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for item in found {
        let entry = classes.entry(item.class).or_default();
        entry.0 += 1;
        entry.1 += item.tiles().len();
    }
    let mut shared = std::collections::BTreeSet::new();
    let mut tiles = 0;
    for asset in index.assets() {
        tiles += asset.keys.len();
        for &key in &asset.keys {
            if index.candidates(asset.bpp, key).len() > 1 {
                shared.insert((asset.bpp, key));
            }
        }
    }
    writeln!(
        out,
        "hd-convert: {} assets, {tiles} tiles, into {}",
        index.assets().len(),
        dir.display()
    )?;
    for (class, (assets, tiles)) in &classes {
        writeln!(out, "  {class:<9} {assets:>5} assets {tiles:>7} tiles")?;
    }
    writeln!(
        out,
        "hd-convert: {} keys name more than one tile (the run rule decides)",
        shared.len()
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssd_hd::pack::TileRef;

    #[test]
    fn a_source_address_is_a_bank_and_an_offset() {
        assert_eq!(address_of(0), 0x00_8000);
        assert_eq!(address_of(0x8000 * 0x16 + 0x0123), 0x16_8123);
    }

    #[test]
    fn the_classes_follow_the_banks_of_the_research_notes() {
        assert_eq!(class_of(0x08_8000), "ui");
        assert_eq!(class_of(0x08_8D4C), "field");
        assert_eq!(class_of(0x16_8000), "portrait");
        assert_eq!(class_of(0x1C_A57A), "fighter");
        assert_eq!(class_of(0x11_C000), "picture", "the big pictures of bank $11");
    }

    #[test]
    fn a_found_stream_names_itself_and_splits_into_tiles() {
        let found = Found { source: 0x16_8000, class: "portrait", bpp: 4, data: vec![0; 64] };
        assert_eq!(found.id(), "portrait/16:8000/4bpp");
        assert_eq!(found.file(), "16-8000-4bpp.png");
        assert_eq!(found.tiles().len(), 2);
    }

    #[test]
    fn a_pack_is_named_by_the_rom_and_the_converter() {
        assert_eq!(
            pack_dir(Path::new("C:/cache"), 0xA8CF_4F2A),
            Path::new("C:/cache").join("hd").join("A8CF4F2A").join("1")
        );
    }

    /// Every file under `dir` with its CRC-32, sorted by path.
    fn tree(dir: &Path) -> Vec<(PathBuf, u32)> {
        let mut files = Vec::new();
        let mut pending = vec![dir.to_path_buf()];
        while let Some(folder) = pending.pop() {
            for entry in std::fs::read_dir(&folder).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    pending.push(path);
                } else {
                    let crc = ssd_rom::crc32(&std::fs::read(&path).unwrap());
                    files.push((path.strip_prefix(dir).unwrap().to_path_buf(), crc));
                }
            }
        }
        files.sort();
        files
    }

    #[test]
    #[ignore = "needs the cached ROM"]
    fn the_english_font_is_indexed_and_the_japanese_kana_is_not_shown() {
        let rom = crate::cached_rom().expect("the ROM cache");
        let found = discover(&rom).unwrap();
        let ui = found.iter().position(|item| item.source == UI_OVERLAY).unwrap();
        let font = &found[ui];
        let index = Index::new(found.iter().map(asset_of).collect());

        // A glyph of the English font resolves to its own tile of the overlay asset
        let glyph = font
            .tiles()
            .iter()
            .enumerate()
            .find(|(number, tile)| {
                tile.iter().any(|&byte| byte != 0)
                    && index.candidates(2, tile_key(tile)).len() == 1
                    && *number > 0
            })
            .map(|(number, _)| number)
            .expect("a unique glyph");
        let key = tile_key(font.tiles()[glyph]);
        assert!(
            index.candidates(2, key).contains(&TileRef { asset: ui as u32, tile: glyph as u32 }),
            "glyph {glyph:#X} of 11:8036 resolves to the English font"
        );

        // The Japanese stream under the overlay: its tile at the same place is not what the
        // English game shows, so it is not bound to the font
        let offset = RomView::new(&rom).unwrap().offset(0x08_8000).unwrap();
        let japanese = stream_at(&rom, offset).expect("the Japanese set decodes").1;
        let kana = &japanese[glyph * 16..glyph * 16 + 16];
        assert_ne!(kana, font.tiles()[glyph], "the kana differs from the English glyph");
        assert!(
            !index.candidates(2, tile_key(kana)).iter().any(|r| r.asset == ui as u32),
            "the kana of 08:8000 is not bound to the English font"
        );
    }

    #[test]
    #[ignore = "needs the cached ROM"]
    fn two_conversions_of_the_same_rom_are_byte_identical() {
        let rom = crate::cached_rom().expect("the ROM cache");
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let mut sink = Vec::new();
        run(&rom, &first.path().join("pack"), &mut sink).unwrap();
        run(&rom, &second.path().join("pack"), &mut sink).unwrap();
        let (one, two) = (tree(&first.path().join("pack")), tree(&second.path().join("pack")));
        assert!(one.len() > 100, "a pack with its art: {} files", one.len());
        assert_eq!(one, two);
    }
}
