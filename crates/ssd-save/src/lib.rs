//! The battery save of the game (8 KiB of SRAM): slots, checksum, backup copy and a typed view
//! of what a slot holds. Layout notes: `docs/re/data/save-format.md`.
//!
//! The crate holds no game data: it reads and writes bytes the game itself saved (a save file is
//! the player's own data, never tracked). The three slots live at [`SLOT_BASES`]; each is a copy
//! of two areas of the game's memory (see [`Slot`]) followed by a 16-bit checksum, and the whole
//! slot is copied again [`BACKUP_OFFSET`] bytes higher, which the game falls back on when the
//! checksum of the first copy fails. [`SaveFile::set_slot`] writes both, like the game's save
//! routine.

mod slot;

pub use slot::{
    Card, FLAG_BYTES, HAND_CARDS, ITEM_SLOTS, Item, PARTY_MAX, Position, RECORD_LEN, ROSTER_LEN,
    Record, Slot, ZENI_MAX,
};

/// Size of the cartridge's SRAM.
pub const SRAM_SIZE: usize = 0x2000;
/// Number of save slots.
pub const SLOT_COUNT: usize = 3;
/// Offset in the SRAM of the first byte of each slot.
pub const SLOT_BASES: [usize; SLOT_COUNT] = [0x100, 0x600, 0xB00];
/// Bytes of a slot: [`IMAGE_LEN`] + [`PAGE_LEN`] + the checksum word (the last two bytes).
pub const SLOT_LEN: usize = 0x4F0;
/// Distance from a slot to its backup copy.
pub const BACKUP_OFFSET: usize = 0x1000;
/// Bytes copied from the game's memory at `$7E:2800` (the roster and the story state).
pub const IMAGE_LEN: usize = 0x3F0;
/// Bytes copied from the game's memory at `$00:0100` (the working page: items, cards, travel
/// and story flags); the last two are overwritten by the checksum in the file.
pub const PAGE_LEN: usize = 0x100;
/// Offset in a slot of the checksum word: the 16-bit sum, modulo 65 536, of the words before it.
pub const CHECKSUM_OFFSET: usize = SLOT_LEN - 2;
/// Offset in the image of the mark that says the slot holds a save.
pub const MARK_OFFSET: usize = 0x3E0;
/// The mark: eight bytes, the hexadecimal digits from F down to 0 in pairs.
pub const MARK: [u8; 8] = [0xFE, 0xDC, 0xBA, 0x98, 0x76, 0x54, 0x32, 0x10];

/// What is wrong with a save file or a request on it.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SaveError {
    /// The file is not [`SRAM_SIZE`] bytes.
    #[error("a save file is {SRAM_SIZE} bytes, this one has {0}")]
    BadSize(usize),
    /// A slot number out of range (the slots are 0 to 2 here, 1 to 3 in the game's menus).
    #[error("slot {0} does not exist (0 to 2)")]
    BadSlot(usize),
    /// The slot holds no save: its mark is missing.
    #[error("slot {0} is empty (no save mark)")]
    EmptySlot(usize),
    /// A field name this crate does not know.
    #[error("unknown field `{0}`")]
    UnknownField(String),
    /// A value that does not fit the field, or does not parse.
    #[error("bad value for `{field}`: {message}")]
    BadValue { field: String, message: String },
}

/// The 16-bit checksum of a slot's bytes: the wrapping sum of the little-endian words before the
/// checksum word.
#[must_use]
pub fn checksum(slot: &[u8; SLOT_LEN]) -> u16 {
    slot[..CHECKSUM_OFFSET]
        .as_chunks::<2>()
        .0
        .iter()
        .fold(0_u16, |sum, &w| sum.wrapping_add(u16::from_le_bytes(w)))
}

/// How a slot of a file stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotStatus {
    /// No mark: nothing was ever saved here (or it was wiped).
    Empty,
    /// The main copy is good.
    Good,
    /// The main copy fails its checksum but the backup is good: the game restores it on load.
    FromBackup,
    /// Marked, but neither copy has a good checksum.
    Corrupt,
}

/// A save file: the SRAM image, read and written as a whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveFile {
    sram: Vec<u8>,
}

impl SaveFile {
    /// A save file with no saves: every byte `$FF`, as a fresh cartridge.
    #[must_use]
    pub fn blank() -> Self {
        Self { sram: vec![0xFF; SRAM_SIZE] }
    }

    /// Wraps the bytes of a save file.
    ///
    /// # Errors
    ///
    /// [`SaveError::BadSize`] unless there are [`SRAM_SIZE`] bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SaveError> {
        if bytes.len() == SRAM_SIZE {
            Ok(Self { sram: bytes.to_vec() })
        } else {
            Err(SaveError::BadSize(bytes.len()))
        }
    }

    /// The bytes of the file.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.sram
    }

    fn range(index: usize, backup: bool) -> Result<std::ops::Range<usize>, SaveError> {
        let base = *SLOT_BASES.get(index).ok_or(SaveError::BadSlot(index))?;
        let start = base + if backup { BACKUP_OFFSET } else { 0 };
        Ok(start..start + SLOT_LEN)
    }

    fn raw(&self, index: usize, backup: bool) -> Result<[u8; SLOT_LEN], SaveError> {
        let mut out = [0_u8; SLOT_LEN];
        out.copy_from_slice(&self.sram[Self::range(index, backup)?]);
        Ok(out)
    }

    /// How slot `index` (0 to 2) stands.
    ///
    /// # Errors
    ///
    /// [`SaveError::BadSlot`].
    pub fn status(&self, index: usize) -> Result<SlotStatus, SaveError> {
        let good = |backup| -> Result<bool, SaveError> {
            let raw = self.raw(index, backup)?;
            Ok(raw[MARK_OFFSET..MARK_OFFSET + MARK.len()] == MARK
                && checksum(&raw)
                    == u16::from_le_bytes([raw[CHECKSUM_OFFSET], raw[CHECKSUM_OFFSET + 1]]))
        };
        let marked = self.raw(index, false)?[MARK_OFFSET..MARK_OFFSET + MARK.len()] == MARK;
        Ok(if good(false)? {
            SlotStatus::Good
        } else if good(true)? {
            SlotStatus::FromBackup
        } else if marked {
            SlotStatus::Corrupt
        } else {
            SlotStatus::Empty
        })
    }

    /// The contents of slot `index` (0 to 2), from the main copy, or from the backup when only
    /// that one is good (what the game does).
    ///
    /// # Errors
    ///
    /// [`SaveError::BadSlot`]; [`SaveError::EmptySlot`] when the slot holds no save, and a
    /// corrupt slot is read as it is.
    pub fn slot(&self, index: usize) -> Result<Slot, SaveError> {
        match self.status(index)? {
            SlotStatus::Empty => Err(SaveError::EmptySlot(index)),
            SlotStatus::FromBackup => Ok(Slot::from_raw(&self.raw(index, true)?)),
            SlotStatus::Good | SlotStatus::Corrupt => Ok(Slot::from_raw(&self.raw(index, false)?)),
        }
    }

    /// Stores `slot` as save number `index` (0 to 2): the mark, the checksum and the backup copy
    /// are written as the game's save routine writes them.
    ///
    /// # Errors
    ///
    /// [`SaveError::BadSlot`].
    pub fn set_slot(&mut self, index: usize, slot: &Slot) -> Result<(), SaveError> {
        let main = Self::range(index, false)?;
        let backup = Self::range(index, true)?;
        let mut raw = slot.to_raw();
        raw[MARK_OFFSET..MARK_OFFSET + MARK.len()].copy_from_slice(&MARK);
        let sum = checksum(&raw);
        raw[CHECKSUM_OFFSET..].copy_from_slice(&sum.to_le_bytes());
        self.sram[main].copy_from_slice(&raw);
        self.sram[backup].copy_from_slice(&raw);
        Ok(())
    }

    /// Empties slot `index` (0 to 2): both copies back to `$FF`.
    ///
    /// # Errors
    ///
    /// [`SaveError::BadSlot`].
    pub fn clear_slot(&mut self, index: usize) -> Result<(), SaveError> {
        for backup in [false, true] {
            let range = Self::range(index, backup)?;
            self.sram[range].fill(0xFF);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blank_file_has_three_empty_slots() {
        let file = SaveFile::blank();
        for i in 0..SLOT_COUNT {
            assert_eq!(file.status(i), Ok(SlotStatus::Empty));
            assert_eq!(file.slot(i), Err(SaveError::EmptySlot(i)));
        }
        assert_eq!(file.status(3), Err(SaveError::BadSlot(3)));
    }

    #[test]
    fn the_size_is_checked() {
        assert_eq!(SaveFile::from_bytes(&[0; 10]), Err(SaveError::BadSize(10)));
        assert!(SaveFile::from_bytes(&vec![0; SRAM_SIZE]).is_ok());
    }

    #[test]
    fn a_stored_slot_has_mark_checksum_and_backup() {
        let mut file = SaveFile::blank();
        let mut slot = Slot::empty();
        slot.set_image_byte(0x10, 0x42);
        file.set_slot(1, &slot).unwrap();
        assert_eq!(file.status(1), Ok(SlotStatus::Good));
        assert_eq!(file.status(0), Ok(SlotStatus::Empty));
        let bytes = file.as_bytes();
        assert_eq!(bytes[SLOT_BASES[1] + 0x10], 0x42);
        assert_eq!(
            bytes[SLOT_BASES[1]..SLOT_BASES[1] + SLOT_LEN],
            bytes[SLOT_BASES[1] + BACKUP_OFFSET..SLOT_BASES[1] + BACKUP_OFFSET + SLOT_LEN]
        );
        assert_eq!(file.slot(1).unwrap(), slot);
        // The checksum is the sum of the words: 0x4242 + 0x4242... here one 0x42 byte plus the
        // mark's words.
        let mut sum = 0x42_u16;
        for w in MARK.chunks(2) {
            sum = sum.wrapping_add(u16::from_le_bytes([w[0], w[1]]));
        }
        let stored = u16::from_le_bytes([
            bytes[SLOT_BASES[1] + CHECKSUM_OFFSET],
            bytes[SLOT_BASES[1] + CHECKSUM_OFFSET + 1],
        ]);
        assert_eq!(stored, sum);
    }

    #[test]
    fn a_damaged_main_copy_is_read_from_the_backup() {
        let mut file = SaveFile::blank();
        let mut slot = Slot::empty();
        slot.set_image_byte(0, 7);
        file.set_slot(0, &slot).unwrap();
        file.sram[SLOT_BASES[0] + 5] ^= 0xFF;
        assert_eq!(file.status(0), Ok(SlotStatus::FromBackup));
        assert_eq!(file.slot(0).unwrap(), slot);
        file.sram[SLOT_BASES[0] + BACKUP_OFFSET + 5] ^= 0xFF;
        assert_eq!(file.status(0), Ok(SlotStatus::Corrupt));
        file.clear_slot(0).unwrap();
        assert_eq!(file.status(0), Ok(SlotStatus::Empty));
    }
}
