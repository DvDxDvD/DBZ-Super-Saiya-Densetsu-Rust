//! The game's text encoding: one byte per tile of the font, with our own table of what each tile
//! looks like (read off the font on screen; not ROM data).
//!
//! The English fan translation draws lower-case letters from tiles `$05-$5C` (interleaved with
//! the Japanese kana, so the codes are not in alphabetical order), digits from `$80-$89` and the
//! capitals from two places: `$04 $06 $08 $0B $0F $10 $12-$14 $45 $47 $50 $5A $8A-$8E` and
//! `$E8-$EF` (the text engine turns the codes from `$A0` up into tiles of a second set). Tiles
//! that are kana or kanji have no entry here and show as `<XX>` with their code in hex.

use std::fmt::Write as _;

/// The character a tile code stands for, if it is a Latin letter, digit or punctuation mark.
#[must_use]
pub fn decode_tile(code: u8) -> Option<char> {
    Some(match code {
        0x01 => ' ',
        0x05 => 'a',
        0x07 => 'b',
        0x09 => 'c',
        0x0A => 'd',
        0x0C => 'e',
        0x0D => 'f',
        0x0E => 'g',
        0x11 => 'h',
        0x15 => 'i',
        0x16 => 'j',
        0x17 => 'k',
        0x18 => 'l',
        0x19 => 'm',
        0x1A => 'n',
        0x1B => 'o',
        0x1D => 'p',
        0x51 => 'q',
        0x52 => 'r',
        0x53 => 's',
        0x54 => 't',
        0x55 => 'u',
        0x56 => 'v',
        0x57 => 'w',
        0x58 => 'x',
        0x5B => 'y',
        0x5C => 'z',
        0xE8 => 'A',
        0x8C => 'B',
        0xE9 => 'C',
        0xEA => 'D',
        0xEB => 'E',
        0xEC => 'F',
        0xED => 'G',
        0x8A => 'H',
        0x8E => 'I',
        0x04 => 'J',
        0x8D => 'K',
        0xEE => 'L',
        0x0F => 'M',
        0x50 => 'N',
        0x5A => 'O',
        0x8B => 'P',
        0x12 => 'Q',
        0x06 => 'R',
        0x14 => 'S',
        0x47 => 'T',
        0x0B => 'U',
        0xEF => 'V',
        0x10 => 'W',
        0x08 => 'X',
        0x45 => 'Y',
        0x13 => 'Z',
        0x80..=0x89 => char::from(b'0' + (code - 0x80)),
        0x1C => '-',
        0x3E => '?',
        0x3F => '!',
        0x40 => '…',
        0x42 => '\'',
        0x7B => ',',
        0x7C => '.',
        0x7E | 0x7F => '"',
        _ => return None,
    })
}

/// Whether the tile code is drawn as a Latin letter, a digit or a punctuation mark.
#[must_use]
pub fn is_latin(code: u8) -> bool {
    decode_tile(code).is_some()
}

/// A string of the game as read from the ROM: the tile codes and what they spell.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Name {
    /// The tile codes, without the length byte.
    pub tiles: Vec<u8>,
    /// The tiles as text; a tile without a known character is written `<XX>` (code in hex).
    pub text: String,
}

impl Name {
    /// The name of a run of tile codes.
    #[must_use]
    pub fn from_tiles(tiles: &[u8]) -> Self {
        Self { tiles: tiles.to_vec(), text: decode(tiles) }
    }

    /// Whether every tile is a Latin letter, digit or punctuation mark (the name is English).
    #[must_use]
    pub fn is_text(&self) -> bool {
        !self.tiles.is_empty() && self.tiles.iter().all(|&t| is_latin(t))
    }

    /// The name without the padding spaces the game keeps at its end.
    #[must_use]
    pub fn trimmed(&self) -> &str {
        self.text.trim_end()
    }
}

/// The text of tile codes: known tiles as their character, the others as `<XX>`.
#[must_use]
pub fn decode(tiles: &[u8]) -> String {
    let mut text = String::with_capacity(tiles.len());
    for &tile in tiles {
        match decode_tile(tile) {
            Some(c) => text.push(c),
            None => {
                let _ = write!(text, "<{tile:02X}>");
            }
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_letters_of_a_name_decode_in_their_own_tile_order() {
        // "Piccolo": a capital from the first set, then lower-case tiles that are not
        // consecutive in the font.
        let name = Name::from_tiles(&[0x8B, 0x15, 0x09, 0x09, 0x1B, 0x18, 0x1B]);
        assert_eq!(name.text, "Piccolo");
        assert!(name.is_text());
        // "Goku": the capital G is from the $E8 set.
        assert_eq!(decode(&[0xED, 0x1B, 0x17, 0x55]), "Goku");
    }

    #[test]
    fn digits_and_punctuation_decode() {
        assert_eq!(decode(&[0x80, 0x89, 0x01, 0x3F]), "09 !");
    }

    #[test]
    fn a_tile_without_a_character_shows_its_code() {
        let name = Name::from_tiles(&[0x4D, 0x05]);
        assert_eq!(name.text, "<4D>a");
        assert!(!name.is_text());
        assert!(!Name::default().is_text(), "an empty string is not a name");
    }

    #[test]
    fn padding_spaces_are_trimmed_for_display_only() {
        let name = Name::from_tiles(&[0x8B, 0x05, 0x01, 0x01]);
        assert_eq!(name.text, "Pa  ");
        assert_eq!(name.trimmed(), "Pa");
    }

    #[test]
    fn no_two_tiles_spell_the_same_letter() {
        let mut seen = std::collections::HashMap::new();
        for code in 0..=u8::MAX {
            if let Some(c) = decode_tile(code)
                && c != ' '
                && c != '"'
                && let Some(other) = seen.insert(c, code)
            {
                panic!("tiles {other:02X} and {code:02X} both spell {c}");
            }
        }
    }
}
