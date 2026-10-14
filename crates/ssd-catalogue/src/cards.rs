//! Cards. The game keeps no list of cards: a card is dealt as a random combination of two values
//! from 1 to 8 and one of seven symbols (kanji), so the catalogue holds the rules of the deal.
//! Docs: `docs/re/data/cards.md`.

use crate::{CatalogueError, RomView};
use ssd_game::data as d;

/// The symbols a card can carry: the game picks a random number from 0 to 6.
pub const SYMBOLS: u8 = 7;
/// The highest card value.
pub const MAX_VALUE: u8 = 8;
/// Entries of the value table: the game indexes it with a random 5-bit number.
pub const TABLE: usize = 32;

/// A dealt card, as the hand keeps it (`card_hand`: three bytes of each six).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Card {
    /// The first value byte (1 to 8; drawn from the value table with the first random
    /// generator). Its use is not known.
    pub unknown_00: u8,
    /// The second value byte (1 to 8; drawn with the second generator). It is the number drawn
    /// on the card's dragon ball.
    pub ball: u8,
    /// The symbol (0 to 6): the kanji on the card. 0 is drawn as 界, 1 as 神 and 6 as 超.
    pub symbol: u8,
}

impl Card {
    /// A card from the three bytes of a hand entry.
    #[must_use]
    pub fn from_bytes(bytes: [u8; 3]) -> Self {
        Self { unknown_00: bytes[0], ball: bytes[1], symbol: bytes[2] }
    }
}

/// How often a value is dealt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CardValue {
    /// The value, 1 to 8.
    pub value: u8,
    /// Entries of the 32 holding it: its chance is `chances / 32`.
    pub chances: u8,
}

/// The rules of the deal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardRules {
    /// The value table as stored: 32 values, the position is a random 5-bit number.
    pub value_table: [u8; TABLE],
    /// The chance of each value, 1 to 8 (those with none are left out).
    pub values: Vec<CardValue>,
    /// The number of symbols: 7 (a property of the dealing code, not of a table).
    pub symbols: u8,
}

impl CardRules {
    /// The rules of a value table.
    #[must_use]
    pub fn from_table(value_table: [u8; TABLE]) -> Self {
        let mut counts = [0_u8; MAX_VALUE as usize + 1];
        for &value in &value_table {
            if let Some(count) = counts.get_mut(usize::from(value)) {
                *count += 1;
            }
        }
        let values = (1..=MAX_VALUE)
            .filter(|&value| counts[usize::from(value)] > 0)
            .map(|value| CardValue { value, chances: counts[usize::from(value)] })
            .collect();
        Self { value_table, values, symbols: SYMBOLS }
    }

    /// Whether the game can deal the card.
    #[must_use]
    pub fn can_deal(&self, card: &Card) -> bool {
        let possible = |value: u8| self.values.iter().any(|v| v.value == value);
        possible(card.unknown_00) && possible(card.ball) && card.symbol < self.symbols
    }
}

pub(crate) fn read(rom: RomView<'_>) -> Result<CardRules, CatalogueError> {
    let bytes = rom.slice(d::CARD_VALUE_WEIGHTS, TABLE)?;
    let table: [u8; TABLE] = bytes.try_into().expect("32 bytes");
    if table.iter().any(|&v| !(1..=MAX_VALUE).contains(&v)) {
        return Err(CatalogueError::Malformed {
            table: "card_value_weights",
            message: format!("a value outside 1 to {MAX_VALUE}"),
        });
    }
    Ok(CardRules::from_table(table))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::rom_with;

    fn table() -> [u8; TABLE] {
        // Four of each value from 1 to 8.
        std::array::from_fn(|i| u8::try_from(i / 4).unwrap() + 1)
    }

    #[test]
    fn the_chance_of_a_value_is_its_share_of_the_table() {
        let rules = CardRules::from_table(table());
        assert_eq!(rules.values.len(), 8);
        assert!(rules.values.iter().all(|v| v.chances == 4));
        assert_eq!(rules.symbols, 7);
    }

    #[test]
    fn a_card_can_be_dealt_when_its_values_and_symbol_are_in_range() {
        let mut skewed = table();
        skewed[0] = 2;
        skewed[1] = 2;
        skewed[2] = 2;
        skewed[3] = 2;
        let rules = CardRules::from_table(skewed);
        assert!(!rules.values.iter().any(|v| v.value == 1), "no entry holds 1");
        assert!(rules.can_deal(&Card::from_bytes([5, 5, 1])));
        assert!(!rules.can_deal(&Card::from_bytes([1, 5, 1])));
        assert!(!rules.can_deal(&Card::from_bytes([5, 5, 7])), "only symbols 0 to 6 exist");
    }

    #[test]
    fn the_table_is_read_from_the_rom_and_checked() {
        let rom = rom_with(&[(d::CARD_VALUE_WEIGHTS, &table())]);
        let rules = read(RomView::new(&rom).unwrap()).unwrap();
        assert_eq!(rules.value_table, table());
        // An all-zero table has no valid value.
        let zero = rom_with(&[]);
        assert!(read(RomView::new(&zero).unwrap()).is_err());
    }
}
