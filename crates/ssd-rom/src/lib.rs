//! Identify and normalise Dragon Ball Z: Super Saiya Densetsu ROM images (spec §5.6).

pub mod checksum;
mod identify;
pub mod ips;

pub use checksum::{fix_lorom_checksum, lorom_checksum_ok};
pub use identify::{
    CANONICAL_CRC32, JP_REV1_CRC32, JP_REV1_SHA1, KLEPTO_UNFIXED_CRC32, crc32, sha1_hex,
};
pub use ips::apply_ips;

const COPIER_HEADER: usize = 512;
const BANK: usize = 0x8000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RomSource {
    Canonical,
    KleptoUnfixedChecksum,
    JapaneseRev1Patched,
    Forced,
}

#[derive(Debug, Clone)]
pub struct NormalizedRom {
    pub bytes: Vec<u8>,
    pub source: RomSource,
}

#[derive(Debug, thiserror::Error)]
pub enum RomError {
    #[error(
        "unsupported ROM size {0} bytes: expected a multiple of 32 KiB, optionally plus a 512-byte copier header"
    )]
    UnsupportedSize(usize),
    #[error(
        "unknown ROM (CRC32 {crc32:08X}): expected the Klepto v1.02 English translation or the Japanese Rev 1 ROM"
    )]
    Unknown { crc32: u32 },
    #[error(
        "this is the Japanese Rev 1 ROM: pass the Klepto v1.02 patch with --ips path/to/dbz102.ips"
    )]
    PatchRequired,
    #[error("invalid IPS patch: {0}")]
    BadPatch(String),
    #[error(
        "patched ROM has CRC32 {crc32:08X}, not the expected Klepto v1.02 result: wrong patch file?"
    )]
    PatchResultMismatch { crc32: u32 },
}

/// Returns `input` without its 512-byte copier header, if it has one.
///
/// # Errors
///
/// [`RomError::UnsupportedSize`] unless the size is a non-zero multiple of 32 KiB, optionally
/// plus the copier header.
pub fn strip_copier_header(input: &[u8]) -> Result<&[u8], RomError> {
    match input.len() % BANK {
        0 if !input.is_empty() => Ok(input),
        COPIER_HEADER if input.len() > COPIER_HEADER => Ok(&input[COPIER_HEADER..]),
        _ => Err(RomError::UnsupportedSize(input.len())),
    }
}

/// Apply an IPS patch made for the 512-byte copier-headered image to a headerless ROM.
///
/// # Errors
///
/// [`RomError::BadPatch`] when the patch is malformed (see [`apply_ips`]).
pub fn patch_headered(rom: &[u8], patch: &[u8]) -> Result<Vec<u8>, RomError> {
    let mut headered = vec![0_u8; COPIER_HEADER];
    headered[0] = 0x80;
    headered.extend_from_slice(rom);
    apply_ips(&mut headered, patch)?;
    Ok(headered.split_off(COPIER_HEADER))
}

/// Identifies `input` and returns the canonical Klepto v1.02 image, applying `ips` when the input
/// is the Japanese Rev 1 ROM.
///
/// # Errors
///
/// [`RomError::UnsupportedSize`] and [`RomError::Unknown`] for images that are not accepted,
/// [`RomError::PatchRequired`] when Rev 1 comes without a patch, [`RomError::BadPatch`] and
/// [`RomError::PatchResultMismatch`] when the patch is malformed or yields the wrong image.
pub fn normalize(input: &[u8], ips: Option<&[u8]>) -> Result<NormalizedRom, RomError> {
    let rom = strip_copier_header(input)?;
    match crc32(rom) {
        CANONICAL_CRC32 => Ok(NormalizedRom { bytes: rom.to_vec(), source: RomSource::Canonical }),
        KLEPTO_UNFIXED_CRC32 => {
            let mut bytes = rom.to_vec();
            fix_lorom_checksum(&mut bytes);
            debug_assert_eq!(crc32(&bytes), CANONICAL_CRC32);
            Ok(NormalizedRom { bytes, source: RomSource::KleptoUnfixedChecksum })
        }
        JP_REV1_CRC32 if sha1_hex(rom) == JP_REV1_SHA1 => {
            let patch = ips.ok_or(RomError::PatchRequired)?;
            let mut bytes = patch_headered(rom, patch)?;
            let got = crc32(&bytes);
            if got != KLEPTO_UNFIXED_CRC32 {
                return Err(RomError::PatchResultMismatch { crc32: got });
            }
            fix_lorom_checksum(&mut bytes);
            debug_assert_eq!(crc32(&bytes), CANONICAL_CRC32);
            Ok(NormalizedRom { bytes, source: RomSource::JapaneseRev1Patched })
        }
        other => Err(RomError::Unknown { crc32: other }),
    }
}

/// [`normalize`], but an image that is none of the known ones is taken as it is when `force` is set.
///
/// # Errors
///
/// The same as [`normalize`], except [`RomError::Unknown`] when `force` is not set. A wrong size is
/// still an error either way: the copier header is stripped, the rest is the player's ROM.
pub fn normalize_forced(
    input: &[u8],
    ips: Option<&[u8]>,
    force: bool,
) -> Result<NormalizedRom, RomError> {
    let rom = strip_copier_header(input)?;
    if force && !matches!(crc32(rom), CANONICAL_CRC32 | KLEPTO_UNFIXED_CRC32 | JP_REV1_CRC32) {
        return Ok(NormalizedRom { bytes: rom.to_vec(), source: RomSource::Forced });
    }
    normalize(input, ips)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_exactly_one_copier_header() {
        let mut raw = vec![0xEE_u8; 512];
        raw.extend(vec![0x11_u8; 0x8000]);
        assert_eq!(strip_copier_header(&raw).unwrap().len(), 0x8000);
        assert_eq!(strip_copier_header(&raw[512..]).unwrap().len(), 0x8000);
        assert!(matches!(strip_copier_header(&raw[..1000]), Err(RomError::UnsupportedSize(1000))));
    }

    #[test]
    fn klepto_patch_offsets_include_the_copier_header() {
        // An IPS record at 0x200 + N must land on ROM byte N (this exact mistake corrupted an
        // earlier hand-patched image: header and vectors were shifted by 512 bytes).
        let rom = vec![0_u8; 0x8000];
        let mut patch = b"PATCH".to_vec();
        patch.extend_from_slice(&[0x00, 0x02, 0x10, 0x00, 0x02, 0xAB, 0xCD]); // offset 0x210, 2 bytes
        patch.extend_from_slice(b"EOF");
        let out = patch_headered(&rom, &patch).unwrap();
        assert_eq!(out.len(), 0x8000);
        assert_eq!(&out[0x10..0x12], &[0xAB, 0xCD]);
        assert!(out[..0x10].iter().all(|&b| b == 0));
    }

    #[test]
    fn unknown_rom_reports_its_crc32() {
        let rom = vec![0x42_u8; 0x8000];
        let err = normalize(&rom, None).unwrap_err();
        assert!(matches!(err, RomError::Unknown { crc32 } if crc32 == crc32_of(&rom)));
        assert!(err.to_string().contains(&format!("{:08X}", crc32_of(&rom))));
    }

    #[test]
    fn forced_takes_an_unknown_rom_as_it_is_and_the_copier_header_is_still_stripped() {
        let rom = vec![0x42_u8; 0x8000];
        assert!(matches!(normalize_forced(&rom, None, false), Err(RomError::Unknown { .. })));
        let out = normalize_forced(&rom, None, true).unwrap();
        assert_eq!(out.source, RomSource::Forced);
        assert_eq!(out.bytes, rom);
        let mut headered = vec![0x00_u8; 512];
        headered.extend_from_slice(&rom);
        assert_eq!(normalize_forced(&headered, None, true).unwrap().bytes, rom);
    }

    fn crc32_of(b: &[u8]) -> u32 {
        crc32(b)
    }
}
