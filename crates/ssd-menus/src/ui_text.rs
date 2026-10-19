//! The words of the HD menus in another language: a table from the English of a text run to its translation,
//! applied to a finished [`DrawList`](crate::DrawList) ([`DrawList::map_text`](crate::DrawList::map_text)). A row is
//! either a whole run (`Talk` → `對話`) or a pattern with `{}` holes for the parts the menus fill in (a number, a
//! name: `Page {}/{}` → `第{}/{}頁`), which go to the translation's holes in their order. A run the table lacks keeps
//! its English.
//!
//! The tables are this repository's own text (`ui-zh-hk.tsv`, next to this file): the menus' words, none of the
//! ROM's.

use crate::Language;
use std::collections::HashMap;
use std::sync::OnceLock;

/// The Traditional Chinese (Hong Kong) table: `english\tchinese` rows, `#` comments.
const ZH_HK: &str = include_str!("ui-zh-hk.tsv");

/// A table of the menus' words.
#[derive(Debug, Default)]
pub struct UiText {
    whole: HashMap<String, String>,
    /// The English split at its holes, and the translation (with as many holes).
    patterns: Vec<(Vec<String>, String)>,
}

impl UiText {
    /// The table of `text` (`english\ttranslation` rows; blank rows and `#` comments are skipped).
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let mut table = Self::default();
        for row in text.lines() {
            if row.trim().is_empty() || row.starts_with('#') {
                continue;
            }
            let Some((english, translated)) = row.split_once('\t') else { continue };
            if english.contains("{}") {
                let pieces = english.split("{}").map(str::to_owned).collect();
                table.patterns.push((pieces, translated.to_owned()));
            } else {
                table.whole.insert(english.to_owned(), translated.to_owned());
            }
        }
        table
    }

    /// The table of the menus in `language`; `None` for English.
    #[must_use]
    pub fn of(language: Language) -> Option<&'static Self> {
        static ZH: OnceLock<UiText> = OnceLock::new();
        match language {
            Language::English => None,
            Language::ZhHk => Some(ZH.get_or_init(|| Self::parse(ZH_HK))),
        }
    }

    /// The translation of the run `text`; `None` when the table has neither it nor a pattern it fits.
    #[must_use]
    pub fn get(&self, text: &str) -> Option<String> {
        if let Some(translated) = self.whole.get(text) {
            return Some(translated.clone());
        }
        self.patterns.iter().find_map(|(pieces, translated)| {
            let holes = fill(pieces, text)?;
            let mut out = String::new();
            let mut rest = translated.as_str();
            for hole in holes {
                let (before, after) = rest.split_once("{}")?;
                out.push_str(before);
                out.push_str(&self.get(hole).unwrap_or_else(|| hole.to_owned()));
                rest = after;
            }
            out.push_str(rest);
            Some(out)
        })
    }
}

/// The parts of `text` in the holes between `pieces` (the English of a pattern split at its holes), when it fits:
/// it starts with the first piece, ends with the last, and has the others in order, each hole not empty.
fn fill<'a>(pieces: &[String], text: &'a str) -> Option<Vec<&'a str>> {
    let (first, rest) = pieces.split_first()?;
    let (last, middle) = rest.split_last()?;
    let mut text = text.strip_prefix(first.as_str())?.strip_suffix(last.as_str())?;
    let mut holes = Vec::new();
    for piece in middle {
        // the hole is the shortest that is not empty before the piece
        let at = text
            .char_indices()
            .skip(1)
            .map(|(at, _)| at)
            .find(|&at| text[at..].starts_with(piece.as_str()))?;
        holes.push(&text[..at]);
        text = &text[at + piece.len()..];
    }
    (!text.is_empty()).then(|| holes.push(text))?;
    Some(holes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> UiText {
        UiText::parse(
            "# made-up rows\nTalk\t對話\nPage {}/{}\t第{}/{}頁\nZeni {}P\t金幣 {}P\nOn: {}\t開啟：{}\n",
        )
    }

    #[test]
    fn a_whole_run_or_a_pattern_is_translated_and_the_holes_keep_their_parts() {
        let table = table();
        assert_eq!(table.get("Talk").as_deref(), Some("對話"));
        assert_eq!(table.get("Page 2/10").as_deref(), Some("第2/10頁"));
        assert_eq!(table.get("Zeni 000P").as_deref(), Some("金幣 000P"));
        assert_eq!(table.get("Look"), None, "not in the table: English");
        assert_eq!(table.get("Page /3"), None, "an empty hole does not fit");
    }

    #[test]
    fn a_hole_that_is_a_word_of_the_table_is_translated_too() {
        assert_eq!(table().get("On: Talk").as_deref(), Some("開啟：對話"));
    }

    #[test]
    fn the_shipped_table_reads_and_its_patterns_have_matching_holes() {
        let table = UiText::of(Language::ZhHk).unwrap();
        assert!(!table.whole.is_empty());
        for (pieces, translated) in &table.patterns {
            assert_eq!(
                pieces.len() - 1,
                translated.matches("{}").count(),
                "{pieces:?} → {translated}"
            );
        }
        assert!(UiText::of(Language::English).is_none());
    }

    #[test]
    fn the_video_pages_look_row_is_not_the_field_commands_look() {
        let table = UiText::of(Language::ZhHk).unwrap();
        let row = crate::options::video::ROWS.iter().find(|row| row.key == "video.look").unwrap();
        assert_ne!(row.label, "Look");
        assert_ne!(table.get(row.label), table.get("Look"));
        assert!(table.get(row.label).is_some());
    }
}
