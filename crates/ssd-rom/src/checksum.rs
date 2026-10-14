//! `LoROM` internal header checksum (header at file offset 0x7FC0).

const COMPLEMENT: usize = 0x7FDC;
const CHECKSUM: usize = 0x7FDE;

/// Sum of all bytes modulo 0x10000, computed with the checksum fields set to
/// complement = 0xFFFF, checksum = 0x0000 (their contribution is constant: 0x1FE).
fn byte_sum_with_neutral_fields(rom: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    for (i, &b) in rom.iter().enumerate() {
        let b = match i {
            COMPLEMENT | 0x7FDD => 0xFF,
            CHECKSUM | 0x7FDF => 0x00,
            _ => b,
        };
        sum = sum.wrapping_add(u32::from(b));
    }
    sum as u16
}

/// Recompute the header checksum of a `LoROM` image whose size is a power of two.
///
/// # Panics
///
/// When `rom` is smaller than 32 KiB or its length is not a power of two.
pub fn fix_lorom_checksum(rom: &mut [u8]) {
    assert!(
        rom.len() >= 0x8000 && rom.len().is_power_of_two(),
        "LoROM size must be a power of two ≥ 32 KiB"
    );
    let checksum = byte_sum_with_neutral_fields(rom);
    rom[COMPLEMENT..COMPLEMENT + 2].copy_from_slice(&(checksum ^ 0xFFFF).to_le_bytes());
    rom[CHECKSUM..CHECKSUM + 2].copy_from_slice(&checksum.to_le_bytes());
}

#[must_use]
pub fn lorom_checksum_ok(rom: &[u8]) -> bool {
    if rom.len() < 0x8000 {
        return false;
    }
    let complement = u16::from_le_bytes([rom[COMPLEMENT], rom[COMPLEMENT + 1]]);
    let checksum = u16::from_le_bytes([rom[CHECKSUM], rom[CHECKSUM + 1]]);
    complement ^ checksum == 0xFFFF && byte_sum_with_neutral_fields(rom) == checksum
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_lorom(len: usize) -> Vec<u8> {
        let mut rom = vec![0_u8; len];
        for (i, b) in rom.iter_mut().enumerate() {
            *b = (i.wrapping_mul(31) ^ (i >> 7)) as u8;
        }
        rom
    }

    #[test]
    fn fixed_checksum_matches_byte_sum_and_complement() {
        let mut rom = synthetic_lorom(0x10_0000);
        fix_lorom_checksum(&mut rom);
        let complement = u16::from_le_bytes([rom[0x7FDC], rom[0x7FDD]]);
        let checksum = u16::from_le_bytes([rom[0x7FDE], rom[0x7FDF]]);
        assert_eq!(complement ^ checksum, 0xFFFF);
        let sum = rom.iter().fold(0_u32, |acc, &b| acc + u32::from(b)) as u16;
        assert_eq!(sum, checksum);
        assert!(lorom_checksum_ok(&rom));
    }

    #[test]
    fn broken_checksum_is_detected() {
        let mut rom = synthetic_lorom(0x8000);
        fix_lorom_checksum(&mut rom);
        rom[0x100] ^= 0xFF;
        assert!(!lorom_checksum_ok(&rom));
    }
}
