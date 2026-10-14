//! The groups of units a battle can start with: five slots each, a unit id or `$FF` for an empty
//! slot. The group index of the battle is in `encounter_group_index`. Docs:
//! `docs/re/data/units.md`.

use crate::{CatalogueError, RomView};
use ssd_game::data as d;

/// Slots of a group.
pub const SLOTS: usize = 5;

/// What a slot holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    /// No enemy.
    Empty,
    /// The enemy with this unit id.
    Unit(u8),
}

/// One group of enemies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Encounter {
    /// The group index.
    pub index: u8,
    /// The five slots, in the order the battle fills its enemy records.
    pub slots: [Slot; SLOTS],
}

impl Encounter {
    /// The unit ids of the occupied slots.
    pub fn units(&self) -> impl Iterator<Item = u8> + '_ {
        self.slots.iter().filter_map(|slot| match slot {
            Slot::Unit(id) => Some(*id),
            Slot::Empty => None,
        })
    }
}

pub(crate) fn read(rom: RomView<'_>) -> Result<Vec<Encounter>, CatalogueError> {
    let count = d::ENCOUNTER_GROUPS_SIZE / SLOTS as u32;
    if count * SLOTS as u32 != d::ENCOUNTER_GROUPS_SIZE {
        return Err(CatalogueError::Malformed {
            table: "encounter_groups",
            message: "the table is not a whole number of groups".to_owned(),
        });
    }
    (0..count)
        .map(|index| {
            let bytes = rom.slice(d::ENCOUNTER_GROUPS + index * SLOTS as u32, SLOTS)?;
            let mut slots = [Slot::Empty; SLOTS];
            for (slot, &byte) in slots.iter_mut().zip(bytes) {
                if byte != 0xFF {
                    *slot = Slot::Unit(byte);
                }
            }
            Ok(Encounter { index: u8::try_from(index).expect("fewer than 256 groups"), slots })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::rom_with;

    #[test]
    fn a_group_has_five_slots_and_ff_leaves_one_empty() {
        let rom = rom_with(&[(d::ENCOUNTER_GROUPS + 5, &[0xFF, 4, 4, 0xFF, 0xFF])]);
        let groups = read(RomView::new(&rom).unwrap()).unwrap();
        assert_eq!(groups.len(), 112);
        let group = groups[1];
        assert_eq!(group.index, 1);
        assert_eq!(group.slots[0], Slot::Empty);
        assert_eq!(group.slots[1], Slot::Unit(4));
        assert_eq!(group.units().collect::<Vec<_>>(), [4, 4]);
        assert_eq!(groups[0].units().count(), 5, "zero is the unit id 0, not an empty slot");
    }
}
