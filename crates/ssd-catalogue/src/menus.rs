//! The names and texts the HD menus show, read from the catalogue: the labels of `ssd-menus`, by
//! character id and by object id. The menus' faces and prices come with them.

use crate::Catalogue;
use ssd_menus::{CharacterLabel, Labels, ObjectLabel};

/// The labels of the menus, from the catalogue: a character's face is the picture of the object of
/// the same name (the game's own face table is not mapped yet, `docs/re/data/menus.md`).
#[must_use]
pub fn labels(catalogue: &Catalogue) -> Labels {
    let characters = catalogue
        .characters
        .iter()
        .map(|character| CharacterLabel {
            name: plain(character.name.trimmed()),
            face: catalogue
                .items
                .iter()
                .find(|item| item.name.trimmed() == character.name.trimmed())
                .map(|item| item.id),
        })
        .collect();
    let objects = catalogue
        .items
        .iter()
        .map(|item| ObjectLabel {
            name: plain(item.name.trimmed()),
            price: item.price,
            help: item.help.iter().map(|line| plain(line.trimmed())).collect(),
            attribute: item.attribute,
            battle: item.usable_in_battle(),
            field: item.usable_on_map(),
            story: item.attribute & 0x20 != 0,
            fixed: item.attribute & 0x04 != 0,
            member: item.attribute & 0x08 != 0,
            card: item.attribute & 0x10 != 0,
            ally: catalogue.characters.iter().any(|c| c.name.trimmed() == item.name.trimmed()),
        })
        .collect();
    Labels { characters, objects }
}

/// Text as the HD shows it: the codes the catalogue cannot spell (the game fills them in while it
/// runs, as a card's value) become an ellipsis.
#[must_use]
pub fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        match tail.find('>') {
            Some(end) => {
                out.push('\u{2026}');
                rest = &tail[end + 1..];
            }
            None => {
                out.push_str(tail);
                return out;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_codes_the_catalogue_cannot_spell_become_an_ellipsis() {
        assert_eq!(plain("Change a card's attack to <FC>"), "Change a card's attack to \u{2026}");
        assert_eq!(plain("<FA>!<FD>"), "\u{2026}!\u{2026}");
        assert_eq!(plain("Restores 25"), "Restores 25");
        assert_eq!(plain("open <FC"), "open <FC", "an unclosed code is kept as it is");
    }

    #[test]
    fn a_character_without_an_object_of_its_name_has_no_face() {
        let labels = Labels {
            characters: vec![CharacterLabel { name: "Krillin".into(), face: None }],
            objects: vec![ObjectLabel::default()],
        };
        assert_eq!(labels.characters[0].face, None);
    }
}
