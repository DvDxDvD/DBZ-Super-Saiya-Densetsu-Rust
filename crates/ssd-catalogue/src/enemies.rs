//! The unit table: one row of 14 bytes per unit id (the enemies and the other fighters of the
//! story), with a name and a list of commands. Docs: `docs/re/data/units.md`.

use crate::names::table_name;
use crate::rom::le;
use crate::text::Name;
use crate::{CatalogueError, RomView};
use ssd_game::data as d;

/// Bytes of one row of `unit_stats`.
pub const ROW: usize = 14;

/// A unit of the unit table. The rows of the party's own fighters (ids 0 and 1 and the ones
/// whose stats come from the level tables) are all zero: see [`Enemy::is_placeholder`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Enemy {
    /// The unit id (the index in every unit table, and byte `$1C` of a battle record).
    pub id: u8,
    /// The name, from the unit name table (not all are English: see [`Name::is_text`]).
    pub name: Name,
    /// Byte 0 of the row, stored in byte `$1B` of the battle record (0 to 6; its use is not
    /// known).
    pub unknown_00: u8,
    /// Level (byte 1).
    pub level: u8,
    /// Hit points (bytes 2 to 3).
    pub hp: u16,
    /// Battle power (bytes 4 to 6, 24 bits). A battle starts the unit at a random value near it.
    pub bp: u32,
    /// KI (bytes 7 to 8).
    pub ki: u16,
    /// Byte 9, stored in byte `$17` of the battle record (30 for most units, 20 to 50).
    pub unknown_09: u8,
    /// Byte 10, stored in byte `$0E` of the battle record (the character level tables have the
    /// same byte; 1 to 16 here).
    pub unknown_0a: u8,
    /// Experience points the unit gives when defeated (bytes 11 to 12).
    pub exp_reward: u16,
    /// Byte 13 (zero in every row of the Klepto v1.02 image).
    pub unknown_0d: u8,
    /// The unit's command list as stored (`unit_command_lists`): a count, then groups of ids
    /// separated by `$FF`, then one word per command. The layout is not decoded yet.
    pub commands: Vec<u8>,
}

impl Enemy {
    /// Whether the row has no stats: the unit is a party fighter or an unused id.
    #[must_use]
    pub fn is_placeholder(&self) -> bool {
        self.hp == 0 && self.bp == 0
    }
}

pub(crate) fn read(rom: RomView<'_>) -> Result<Vec<Enemy>, CatalogueError> {
    let malformed = |message: String| CatalogueError::Malformed { table: "unit_stats", message };
    let count = d::UNIT_STATS_SIZE / ROW as u32;
    if count * ROW as u32 != d::UNIT_STATS_SIZE || count * 2 != d::UNIT_COMMAND_POINTERS_SIZE {
        return Err(malformed("the unit tables do not have one entry per unit".to_owned()));
    }
    let lists_end = d::UNIT_COMMAND_LISTS + d::UNIT_COMMAND_LISTS_SIZE;
    let command_pointers: Vec<u32> = (0..count)
        .map(|id| rom.pointer(d::UNIT_COMMAND_POINTERS + 2 * id, 0x02))
        .collect::<Result<_, _>>()?;
    let mut enemies = Vec::new();
    for id in 0..count {
        let row = rom.slice(d::UNIT_STATS + id * ROW as u32, ROW)?;
        let start = command_pointers[id as usize];
        if start < d::UNIT_COMMAND_LISTS || start >= lists_end {
            return Err(malformed(format!(
                "unit {id}: its command list at ${start:06X} is outside the lists"
            )));
        }
        // A list ends where the next one in the ROM begins (the pointers ascend with the ids,
        // but ask all of them, not only the next id's).
        let end =
            command_pointers.iter().copied().filter(|&p| p > start).min().unwrap_or(lists_end);
        enemies.push(Enemy {
            id: u8::try_from(id).expect("fewer than 256 units"),
            name: table_name(rom, d::UNIT_NAME_POINTERS, id)?,
            unknown_00: row[0],
            level: row[1],
            hp: u16::try_from(le(&row[2..4])).expect("two bytes"),
            bp: le(&row[4..7]),
            ki: u16::try_from(le(&row[7..9])).expect("two bytes"),
            unknown_09: row[9],
            unknown_0a: row[10],
            exp_reward: u16::try_from(le(&row[11..13])).expect("two bytes"),
            unknown_0d: row[13],
            commands: rom.slice(start, (end - start) as usize)?.to_vec(),
        });
    }
    Ok(enemies)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::rom_with;

    const UNITS: u32 = d::UNIT_STATS_SIZE / ROW as u32;

    /// Two real-looking rows (a placeholder and a fighter), the rest zero; the command lists of
    /// every unit are 3 bytes long, back to back.
    fn rom() -> Vec<u8> {
        let fighter = [3, 2, 0x37, 0, 0xB9, 0x01, 0, 0x18, 0, 0x1E, 1, 9, 0, 0];
        let pointers: Vec<u8> = (0..UNITS)
            .flat_map(|id| {
                u16::try_from((d::UNIT_COMMAND_LISTS & 0xFFFF) + 3 * id).unwrap().to_le_bytes()
            })
            .collect();
        rom_with(&[
            (d::UNIT_STATS + ROW as u32, &fighter),
            (d::UNIT_COMMAND_POINTERS, &pointers),
            (d::UNIT_COMMAND_LISTS + 3, &[1, 0, 0x10]),
            (d::UNIT_NAME_POINTERS + 2, &[0x00, 0xAD]),
            (0x02_AD00, &[6, 0x14, 0x05, 0x15, 0x07, 0x05, 0x15]),
        ])
    }

    #[test]
    fn a_row_has_stats_a_reward_and_a_name() {
        let rom = rom();
        let enemies = read(RomView::new(&rom).unwrap()).unwrap();
        assert_eq!(enemies.len(), 105);
        let unit = &enemies[1];
        assert_eq!(unit.name.text, "Saibai");
        assert_eq!((unit.level, unit.hp, unit.bp, unit.ki), (2, 55, 441, 24));
        assert_eq!((unit.unknown_00, unit.unknown_09, unit.unknown_0a), (3, 30, 1));
        assert_eq!(unit.exp_reward, 9);
        assert_eq!(unit.commands, [1, 0, 0x10]);
        assert!(!unit.is_placeholder());
    }

    #[test]
    fn an_all_zero_row_is_a_placeholder_and_the_lists_are_cut_where_the_next_begins() {
        let rom = rom();
        let enemies = read(RomView::new(&rom).unwrap()).unwrap();
        assert!(enemies[0].is_placeholder());
        assert_eq!(enemies[0].commands.len(), 3);
        assert_eq!(
            enemies[104].commands.len(),
            3 + (d::UNIT_COMMAND_LISTS_SIZE as usize - 3 * 105)
        );
    }

    #[test]
    fn a_command_pointer_outside_the_lists_is_malformed() {
        let mut rom = rom();
        let view = RomView::new(&rom).unwrap();
        let at = view.offset(d::UNIT_COMMAND_POINTERS).unwrap();
        rom[at + 1] = 0x80;
        let error = read(RomView::new(&rom).unwrap()).unwrap_err();
        assert!(error.to_string().contains("unit 0"), "{error}");
    }
}
