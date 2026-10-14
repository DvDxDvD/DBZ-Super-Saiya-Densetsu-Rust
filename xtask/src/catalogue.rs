//! `catalogue`: prints the typed game catalogue extracted from the cached ROM as text tables.
//! Stdout only: what it prints is derived from the game and is never written to a tracked file.

use crate::shared::render_table;
use ssd_catalogue::{Catalogue, Slot};
use std::fmt::Write as _;

/// What to print.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Characters,
    Cards,
    Enemies,
    Items,
    Attacks,
    Encounters,
}

impl Kind {
    const ALL: [Self; 6] = [
        Self::Characters,
        Self::Cards,
        Self::Enemies,
        Self::Items,
        Self::Attacks,
        Self::Encounters,
    ];

    fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "characters" => Self::Characters,
            "cards" => Self::Cards,
            "enemies" => Self::Enemies,
            "items" => Self::Items,
            "attacks" => Self::Attacks,
            "encounters" => Self::Encounters,
            _ => return None,
        })
    }
}

/// The parsed command line: the kind to print (every kind without `--kind`).
pub fn parse_args(args: &[&str]) -> Result<Vec<Kind>, String> {
    match args {
        [] => Ok(Kind::ALL.to_vec()),
        ["--kind", name] => Kind::parse(name).map(|kind| vec![kind]).ok_or_else(|| {
            format!(
                "unknown kind `{name}`: characters, cards, enemies, items, attacks or encounters"
            )
        }),
        ["--kind"] => Err("--kind needs a kind".to_owned()),
        [other, ..] => Err(format!("unknown argument `{other}`")),
    }
}

fn row(cells: &[String]) -> Vec<String> {
    cells.to_vec()
}

fn s(value: impl ToString) -> String {
    value.to_string()
}

/// A name for a table cell: at most 28 characters (a few entries of the name tables point at
/// data that is not text and run on).
fn cell(name: &ssd_catalogue::Name) -> String {
    let text = name.trimmed();
    match text.char_indices().nth(28) {
        Some((end, _)) => format!("{}...", &text[..end]),
        None => text.to_owned(),
    }
}

/// The tables of the kinds asked for, each under a heading line.
#[must_use]
pub fn render(catalogue: &Catalogue, kinds: &[Kind]) -> String {
    let mut out = String::new();
    for &kind in kinds {
        let (title, table) = match kind {
            Kind::Characters => ("characters", characters(catalogue)),
            Kind::Cards => ("cards", cards(catalogue)),
            Kind::Enemies => ("enemies (units)", enemies(catalogue)),
            Kind::Items => ("items (named objects)", items(catalogue)),
            Kind::Attacks => ("attacks", attacks(catalogue)),
            Kind::Encounters => ("encounters", encounters(catalogue)),
        };
        let _ = writeln!(
            out,
            "{title}
{table}"
        );
    }
    out
}

fn characters(catalogue: &Catalogue) -> String {
    let rows: Vec<Vec<String>> = catalogue
        .characters
        .iter()
        .map(|c| {
            let (first, last) = (c.levels.first(), c.levels.last());
            row(&[
                s(c.id),
                cell(&c.name),
                s(c.unit_id),
                first.map_or_else(|| s("-"), |r| s(r.bp_gain)),
                first.map_or_else(|| s("-"), |r| s(r.max_hp)),
                first.map_or_else(|| s("-"), |r| s(r.ki_gain)),
                last.map_or_else(|| s("-"), |r| s(r.max_hp)),
                last.map_or_else(|| s("-"), |r| s(r.exp_start)),
                c.fixed_stats_unit
                    .map_or_else(|| "level table".to_owned(), |u| format!("unit {u}")),
            ])
        })
        .collect();
    render_table(
        &["id", "name", "unit", "BP L1", "HP L1", "KI L1", "HP L48", "EXP L48", "stats from"],
        &rows,
    )
}

fn enemies(catalogue: &Catalogue) -> String {
    let rows: Vec<Vec<String>> = catalogue
        .enemies
        .iter()
        .map(|e| {
            row(&[
                s(e.id),
                cell(&e.name),
                s(e.level),
                s(e.hp),
                s(e.bp),
                s(e.ki),
                s(e.exp_reward),
                format!("{:02X}/{:02X}/{:02X}", e.unknown_00, e.unknown_09, e.unknown_0a),
                s(e.commands.len()),
            ])
        })
        .collect();
    render_table(&["id", "name", "lv", "HP", "BP", "KI", "EXP", "u00/u09/u0A", "cmd bytes"], &rows)
}

fn cards(catalogue: &Catalogue) -> String {
    let rules = &catalogue.cards;
    let rows: Vec<Vec<String>> = rules
        .values
        .iter()
        .map(|v| {
            row(&[s(v.value), s(v.chances), format!("{:.1}%", f64::from(v.chances) * 100.0 / 32.0)])
        })
        .collect();
    let mut out = render_table(&["value", "chances of 32", "share"], &rows);
    let _ = writeln!(
        out,
        "  a card is two values drawn from this table and one of {} symbols (0-6)",
        rules.symbols
    );
    out
}

fn items(catalogue: &Catalogue) -> String {
    let rows: Vec<Vec<String>> =
        catalogue.items.iter().map(|i| row(&[s(i.id), cell(&i.name)])).collect();
    render_table(&["id", "name"], &rows)
}

fn attacks(catalogue: &Catalogue) -> String {
    let rows: Vec<Vec<String>> = catalogue
        .attacks
        .iter()
        .map(|a| {
            row(&[s(a.id), cell(&a.name), s(if a.name.is_text() { "text" } else { "kana/none" })])
        })
        .collect();
    render_table(&["id", "name", "kind"], &rows)
}

fn encounters(catalogue: &Catalogue) -> String {
    let name = |id: u8| {
        catalogue.enemies.get(usize::from(id)).map_or_else(|| format!("#{id}"), |e| cell(&e.name))
    };
    let rows: Vec<Vec<String>> = catalogue
        .encounters
        .iter()
        .map(|g| {
            let slots: Vec<String> = g
                .slots
                .iter()
                .map(|slot| match slot {
                    Slot::Empty => "-".to_owned(),
                    Slot::Unit(id) => name(*id),
                })
                .collect();
            row(&[s(g.index), slots.join(", ")])
        })
        .collect();
    render_table(&["group", "slots"], &rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssd_catalogue::CardRules;

    #[test]
    fn the_arguments_pick_a_kind_or_all_of_them() {
        assert_eq!(parse_args(&[]).unwrap().len(), 6);
        assert_eq!(parse_args(&["--kind", "cards"]).unwrap(), [Kind::Cards]);
        assert!(parse_args(&["--kind", "weapons"]).unwrap_err().contains("weapons"));
        assert!(parse_args(&["--kind"]).is_err());
        assert!(parse_args(&["cards"]).is_err());
    }

    #[test]
    fn the_card_rules_print_one_line_per_value() {
        let rules =
            CardRules::from_table(std::array::from_fn(|i| u8::try_from(i / 4).unwrap() + 1));
        let catalogue = Catalogue {
            characters: Vec::new(),
            enemies: Vec::new(),
            encounters: Vec::new(),
            cards: rules,
            attacks: Vec::new(),
            items: Vec::new(),
        };
        let text = render(&catalogue, &[Kind::Cards]);
        assert!(text.starts_with("cards\n"), "{text}");
        assert_eq!(text.lines().filter(|l| l.contains("12.5%")).count(), 8, "{text}");
        assert!(text.contains("7 symbols"), "{text}");
    }
}
