//! ROM identification via CRC32 and SHA-1.

use sha1_smol::Sha1;

pub const CANONICAL_CRC32: u32 = 0xA8CF_4F2A;
pub const KLEPTO_UNFIXED_CRC32: u32 = 0x3BB0_2A05;
pub const JP_REV1_CRC32: u32 = 0xADE7_B968;
pub const JP_REV1_SHA1: &str = "6b5725765ddde078350ec61221ddb9bb252066a7";

const CRC: crc::Crc<u32> = crc::Crc::<u32>::new(&crc::CRC_32_ISO_HDLC);

#[must_use]
pub fn crc32(bytes: &[u8]) -> u32 {
    CRC.checksum(bytes)
}

#[must_use]
pub fn sha1_hex(bytes: &[u8]) -> String {
    Sha1::from(bytes).digest().to_string()
}
