//! Reads the tables of a ROM image by the physical addresses of `re/symbols.toml`.

use crate::CatalogueError;

/// A bank of the `LoROM` map holds `$8000-$FFFF`.
const BANK: usize = 0x8000;

/// The smallest image the extractor accepts: banks `$00-$1F` (1 MiB), the canonical size.
pub const MIN_ROM_SIZE: usize = 0x10_0000;

/// A ROM image read through its physical addresses (banks `$00-$1F`, offsets `$8000-$FFFF`).
#[derive(Clone, Copy)]
pub struct RomView<'a> {
    bytes: &'a [u8],
}

impl<'a> RomView<'a> {
    /// A view of a headerless `LoROM` image of at least [`MIN_ROM_SIZE`] bytes.
    ///
    /// # Errors
    ///
    /// [`CatalogueError::RomTooSmall`] for a shorter image, or one that is not a whole number
    /// of banks (a copier header is not accepted either: normalise the ROM first).
    pub fn new(bytes: &'a [u8]) -> Result<Self, CatalogueError> {
        if bytes.len() < MIN_ROM_SIZE || !bytes.len().is_multiple_of(BANK) {
            return Err(CatalogueError::RomTooSmall(bytes.len()));
        }
        Ok(Self { bytes })
    }

    /// The file offset of a physical address, if it lies in the ROM image.
    #[must_use]
    pub fn offset(self, address: u32) -> Option<usize> {
        let bank = (address >> 16) as usize & 0x7F;
        let within = address as usize & 0xFFFF;
        if within < BANK {
            return None;
        }
        let offset = bank * BANK + (within - BANK);
        (offset < self.bytes.len()).then_some(offset)
    }

    /// `len` bytes from `address`, which must lie within one bank.
    ///
    /// # Errors
    ///
    /// [`CatalogueError::OutOfRom`] when the address is not in the image or the run leaves its
    /// bank.
    pub fn slice(self, address: u32, len: usize) -> Result<&'a [u8], CatalogueError> {
        let out = || CatalogueError::OutOfRom { address, len };
        let start = self.offset(address).ok_or_else(out)?;
        let in_bank = BANK - (start % BANK);
        if len > in_bank {
            return Err(out());
        }
        self.bytes.get(start..start + len).ok_or_else(out)
    }

    /// One byte.
    ///
    /// # Errors
    ///
    /// As [`RomView::slice`].
    pub fn u8(self, address: u32) -> Result<u8, CatalogueError> {
        Ok(self.slice(address, 1)?[0])
    }

    /// A little-endian word.
    ///
    /// # Errors
    ///
    /// As [`RomView::slice`].
    pub fn u16(self, address: u32) -> Result<u16, CatalogueError> {
        let b = self.slice(address, 2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    /// The pointer word at `address`, as the 24-bit address in `bank`.
    ///
    /// # Errors
    ///
    /// As [`RomView::slice`].
    pub fn pointer(self, address: u32, bank: u8) -> Result<u32, CatalogueError> {
        Ok(u32::from(bank) << 16 | u32::from(self.u16(address)?))
    }
}

/// Little-endian value of up to four bytes.
#[must_use]
pub fn le(bytes: &[u8]) -> u32 {
    bytes.iter().rev().fold(0, |value, &b| value << 8 | u32::from(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image() -> Vec<u8> {
        let mut rom = vec![0_u8; MIN_ROM_SIZE];
        // $02:8000 is file offset $10000; $01:FFFF is the last byte of bank 1.
        rom[0x1_0000] = 0xAB;
        rom[0x1_0001] = 0xCD;
        rom[0xFFFF] = 0x7F;
        rom
    }

    #[test]
    fn a_physical_address_maps_to_its_bank_and_offset() {
        let rom = image();
        let view = RomView::new(&rom).unwrap();
        assert_eq!(view.offset(0x02_8000), Some(0x1_0000));
        assert_eq!(view.offset(0x82_8000), Some(0x1_0000), "the high-bank mirror");
        assert_eq!(view.offset(0x01_FFFF), Some(0xFFFF));
        assert_eq!(view.offset(0x02_0000), None, "offsets below $8000 are not ROM");
        assert_eq!(view.offset(0x20_8000), None, "past the image");
        assert_eq!(view.u16(0x02_8000).unwrap(), 0xCDAB);
        assert_eq!(view.pointer(0x02_8000, 2).unwrap(), 0x02_CDAB);
    }

    #[test]
    fn a_run_may_not_leave_its_bank() {
        let rom = image();
        let view = RomView::new(&rom).unwrap();
        assert_eq!(view.slice(0x01_FFFF, 1).unwrap(), [0x7F]);
        assert!(view.slice(0x01_FFFF, 2).is_err());
        assert!(view.u16(0x01_FFFF).is_err());
    }

    #[test]
    fn a_short_or_odd_sized_image_is_refused() {
        assert!(RomView::new(&[0; 100]).is_err());
        assert!(RomView::new(&vec![0; MIN_ROM_SIZE + 512]).is_err(), "a copier header");
    }

    #[test]
    fn little_endian_values_of_any_width() {
        assert_eq!(le(&[0x34, 0x12]), 0x1234);
        assert_eq!(le(&[0x01, 0x02, 0x03]), 0x03_0201);
        assert_eq!(le(&[]), 0);
    }
}
