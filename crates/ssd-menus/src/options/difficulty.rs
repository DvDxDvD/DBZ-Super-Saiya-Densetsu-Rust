//! The Difficulty page (saved with the game): the preset, what it sets, and the way to the Cheats screen.
//!
//! A preset is three cheat values written together: the enemy power and the EXP of the Cheats screen, and the money
//! multiplier (the zeni the story's events give). Normal is all three at x1, the original game. The preset is read off
//! the cheats, never stored apart: when one of the three is changed on the Cheats screen it reads Custom, and the
//! last change wins. Easy and Hard are cheats like any other, so the save they are made on carries the CHEATS badge.

use super::{Kind, Opens, RowDef, Source, Values, View};
use crate::cheats::{Cheats, MONEY_NEUTRAL, Setting};

/// The presets, in the order of the row.
pub const PRESETS: &[&str] = &["Easy", "Normal", "Hard"];
/// The place of Normal among [`PRESETS`].
pub const NORMAL: usize = 1;

/// The key the money step is kept under in the slot file.
pub const MONEY_KEY: &str = "difficulty.money";
/// The key the preset is written under in the slot file when a save is made (informational: it is read off the cheats).
pub const PRESET_KEY: &str = "difficulty.preset";

/// What a preset sets: the step of the enemy power, of the EXP, and of the money, in their lists.
/// Easy: power x1/2, EXP x2, money x2. Normal: all x1. Hard: power x2, EXP x1, money x1/2.
const SETS: [(usize, usize, usize); 3] = [(0, 1, 2), (1, 0, MONEY_NEUTRAL), (2, 0, 0)];

/// Writes preset `index` (Easy, Normal or Hard) into `cheats`: the three values it sets, nothing else.
pub fn apply(cheats: &mut Cheats, index: usize) {
    let (power, exp, money) = SETS[index.min(SETS.len() - 1)];
    cheats.set_index(Setting::Power, power);
    cheats.set_index(Setting::Exp, exp);
    cheats.set_money_index(money);
}

/// The preset the cheats are on: none (Custom) when the three values are not those of one of the presets.
#[must_use]
pub fn preset_of(cheats: &Cheats) -> Option<usize> {
    let now = (cheats.index(Setting::Power), cheats.index(Setting::Exp), cheats.money_index());
    SETS.iter().position(|&set| set == now)
}

/// Copies the run's difficulty into the slot's values for a save: the money step and the preset (a value that is the
/// default is left out, so the file only holds what differs).
pub fn store(cheats: &Cheats, values: &mut Values) {
    if cheats.money_on() {
        values.set(MONEY_KEY, cheats.money_index() as i64);
    } else {
        values.remove(MONEY_KEY);
    }
    match preset_of(cheats) {
        Some(NORMAL) => values.remove(PRESET_KEY),
        // Custom is written as the number after the last preset.
        other => values.set(PRESET_KEY, other.unwrap_or(PRESETS.len()) as i64),
    }
}

/// Puts the slot's saved money step into `cheats` (the other values of the difficulty come with the cheats).
pub fn restore(values: &Values, cheats: &mut Cheats) {
    let index = values
        .get(MONEY_KEY)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(MONEY_NEUTRAL);
    cheats.set_money_index(index);
}

const POWER: &[&str] = &["×½", "×1", "×2", "×3", "×5"];
const EXP: &[&str] = &["×1", "×2", "×3", "×5", "×10"];
const MONEY: &[&str] = &["×½", "×1", "×2"];

/// The text of the info row `key`, given the values of the page's rows (as the screen shows them).
#[must_use]
pub fn info_text(key: &str, shown: &[usize]) -> String {
    let labels = match key {
        "difficulty.power" => POWER,
        "difficulty.exp" => EXP,
        MONEY_KEY => MONEY,
        _ => return String::new(),
    };
    let at = ROWS.iter().position(|row| row.key == key).unwrap_or(0);
    labels.get(shown.get(at).copied().unwrap_or(0)).copied().unwrap_or("?").to_owned()
}

/// The small caption left of the value of the info row `key`.
#[must_use]
pub fn info_caption(key: &str) -> &'static str {
    match key {
        "difficulty.power" => "Cheats: Enemy power",
        "difficulty.exp" => "Cheats: EXP",
        MONEY_KEY => "Zeni multiplier",
        _ => "",
    }
}

/// The rows of the page.
pub const ROWS: &[RowDef] = &[
    RowDef {
        key: PRESET_KEY,
        label: "Difficulty",
        group: "Preset",
        help: "Sets enemy power, EXP and money together. Normal is the original game. Easy and Hard mark the save with the CHEATS badge.",
        kind: Kind::Choice(PRESETS),
        source: Source::Preset,
    },
    RowDef {
        key: "difficulty.note_badge",
        label: "",
        group: "Preset",
        help: "",
        kind: Kind::Note(
            "Easy and Hard mark the save with the CHEATS badge. Only Normal is clean.",
        ),
        source: Source::None,
    },
    RowDef {
        key: "difficulty.power",
        label: "Enemy power",
        group: "The preset sets",
        help: "",
        kind: Kind::Info,
        source: Source::View(View::Power),
    },
    RowDef {
        key: "difficulty.exp",
        label: "EXP per battle",
        group: "The preset sets",
        help: "",
        kind: Kind::Info,
        source: Source::View(View::Exp),
    },
    RowDef {
        key: MONEY_KEY,
        label: "Money from events",
        group: "The preset sets",
        help: "",
        kind: Kind::Info,
        source: Source::View(View::Money),
    },
    RowDef {
        key: "difficulty.cheats",
        label: "Cheats screen",
        group: "Fine control",
        help: "Damage, defence, encounters, Z cards and more, step by step. A preset changed there reads Custom.",
        kind: Kind::Button("Open Cheats"),
        source: Source::Opens(Opens::Cheats),
    },
];

/// A line under the rows, if the page has one.
pub const NOTE: Option<&str> =
    Some("The preset and the Cheats screen write the same numbers: the last change wins.");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cheats::Multiplier;
    use crate::options::Effect;

    #[test]
    fn each_preset_sets_the_three_values_of_the_owners_table_and_is_read_back() {
        let mut cheats = Cheats::neutral();
        assert_eq!(preset_of(&cheats), Some(NORMAL), "the original is Normal");
        apply(&mut cheats, 0);
        assert_eq!(cheats.multiplier(Setting::Power), Multiplier::new(1, 2));
        assert_eq!(cheats.multiplier(Setting::Exp), Multiplier::new(2, 1));
        assert_eq!(cheats.money(), Multiplier::new(2, 1));
        assert_eq!(preset_of(&cheats), Some(0));
        apply(&mut cheats, 2);
        assert_eq!(cheats.multiplier(Setting::Power), Multiplier::new(2, 1));
        assert_eq!(cheats.multiplier(Setting::Exp), Multiplier::new(1, 1));
        assert_eq!(cheats.money(), Multiplier::new(1, 2));
        assert_eq!(preset_of(&cheats), Some(2));
        apply(&mut cheats, 1);
        assert!(cheats.is_neutral() && preset_of(&cheats) == Some(NORMAL));
    }

    #[test]
    fn the_presets_leave_the_other_cheats_alone_and_a_changed_value_reads_custom() {
        let mut cheats = Cheats::neutral();
        cheats.set_index(Setting::Damage, 4);
        cheats.set_zeni(true);
        apply(&mut cheats, 0);
        assert_eq!((cheats.index(Setting::Damage), cheats.zeni()), (4, true));
        cheats.set_index(Setting::Exp, 3);
        assert_eq!(preset_of(&cheats), None, "EXP changed on the Cheats screen: Custom");
        let mut other = Cheats::neutral();
        apply(&mut other, 2);
        other.set_money_index(2);
        assert_eq!(preset_of(&other), None, "money alone makes it Custom");
        assert!(!other.is_neutral());
    }

    #[test]
    fn the_preset_row_writes_the_cheats_and_custom_is_past_the_last_choice() {
        let row = &ROWS[0];
        let (mut values, mut cheats) = (Values::new(), Cheats::neutral());
        assert_eq!(row.index(&values, &cheats, 0), NORMAL);
        assert_eq!(row.step(&mut values, &mut cheats, 0, 1), Effect::Cheats);
        assert_eq!(row.index(&values, &cheats, 0), 2, "Hard");
        assert_eq!(cheats.money(), Multiplier::new(1, 2));
        cheats.set_index(Setting::Power, 3);
        assert_eq!(row.index(&values, &cheats, 0), PRESETS.len(), "Custom: no choice is chosen");
        assert_eq!(row.step(&mut values, &mut cheats, 0, 1), Effect::Cheats);
        assert_eq!(row.index(&values, &cheats, 0), 0, "Right from Custom wraps to Easy");
        assert_eq!(row.reset(&mut values, &mut cheats), Effect::Cheats);
        assert!(cheats.is_neutral(), "reset is Normal");
        assert!(values.is_empty(), "nothing is stored by the row: the cheats are the one truth");
    }

    #[test]
    fn the_info_rows_show_the_cheats_and_do_not_move() {
        let mut cheats = Cheats::neutral();
        apply(&mut cheats, 0);
        let values = Values::new();
        let indices: Vec<usize> = ROWS.iter().map(|row| row.index(&values, &cheats, 0)).collect();
        let shown: Vec<String> =
            ROWS[2..5].iter().map(|row| info_text(row.key, &indices)).collect();
        assert_eq!(shown, ["×½", "×2", "×2"], "Easy: power x1/2, EXP x2, money x2");
        assert_eq!(info_caption("difficulty.exp"), "Cheats: EXP");
        let mut values = Values::new();
        assert_eq!(ROWS[2].step(&mut values, &mut cheats, 0, 1), Effect::None);
        assert!(ROWS[2].is_info() && ROWS[1].is_info() && !ROWS[0].is_info());
    }

    #[test]
    fn the_slot_keeps_the_money_and_the_preset_for_a_save() {
        let mut cheats = Cheats::neutral();
        let mut values = Values::new();
        store(&cheats, &mut values);
        assert!(values.is_empty(), "Normal leaves the file clean");
        apply(&mut cheats, 2);
        store(&cheats, &mut values);
        assert_eq!((values.get(MONEY_KEY), values.get(PRESET_KEY)), (Some(0), Some(2)));
        let mut again = Cheats::neutral();
        restore(&values, &mut again);
        assert_eq!(again.money(), Multiplier::new(1, 2));
        cheats.set_index(Setting::Exp, 2);
        store(&cheats, &mut values);
        assert_eq!(values.get(PRESET_KEY), Some(3), "Custom");
        restore(&Values::new(), &mut again);
        assert!(!again.money_on());
    }
}
