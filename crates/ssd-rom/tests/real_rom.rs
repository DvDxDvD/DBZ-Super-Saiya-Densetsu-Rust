//! Needs the game's ROM: `SSD_ROM_PATH=... cargo test -p ssd-rom -- --ignored`

#[test]
#[ignore = "needs SSD_ROM_PATH"]
fn canonical_rom_is_accepted_and_unchanged() {
    let path = std::env::var("SSD_ROM_PATH").expect("SSD_ROM_PATH");
    let bytes = std::fs::read(path).unwrap();
    let rom = ssd_rom::normalize(&bytes, None).unwrap();
    assert_eq!(ssd_rom::crc32(&rom.bytes), ssd_rom::CANONICAL_CRC32);
    assert!(ssd_rom::checksum::lorom_checksum_ok(&rom.bytes));
}

#[test]
#[ignore = "needs SSD_ROM_PATH"]
fn copier_header_variant_normalises_to_canonical() {
    let path = std::env::var("SSD_ROM_PATH").expect("SSD_ROM_PATH");
    let canonical = std::fs::read(path).unwrap();
    let mut with_header = vec![0_u8; 512];
    with_header.extend_from_slice(&canonical);
    let result = ssd_rom::normalize(&with_header, None).unwrap();
    assert_eq!(result.source, ssd_rom::RomSource::Canonical);
    assert_eq!(ssd_rom::crc32(&result.bytes), ssd_rom::CANONICAL_CRC32);
}

#[test]
#[ignore = "needs SSD_ROM_PATH"]
fn klepto_unfixed_checksum_variant_normalises_to_canonical() {
    let path = std::env::var("SSD_ROM_PATH").expect("SSD_ROM_PATH");
    let canonical = std::fs::read(path).unwrap();
    let mut variant = canonical.clone();

    // Write the Klepto unfixed checksum bytes at 0x7FDC..0x7FE0
    // These are: complement [0x02, 0xCC], checksum [0xFD, 0x33]
    variant[0x7FDC] = 0x02;
    variant[0x7FDD] = 0xCC;
    variant[0x7FDE] = 0xFD;
    variant[0x7FDF] = 0x33;

    // Verify it has the Klepto unfixed CRC32
    assert_eq!(ssd_rom::crc32(&variant), ssd_rom::KLEPTO_UNFIXED_CRC32);

    // Normalize it and verify the result
    let result = ssd_rom::normalize(&variant, None).unwrap();
    assert_eq!(result.source, ssd_rom::RomSource::KleptoUnfixedChecksum);
    assert_eq!(ssd_rom::crc32(&result.bytes), ssd_rom::CANONICAL_CRC32);
    assert_eq!(result.bytes, canonical);
}
