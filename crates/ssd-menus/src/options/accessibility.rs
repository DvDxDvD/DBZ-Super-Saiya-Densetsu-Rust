//! The Accessibility page (this PC): the text size, high contrast and the cursor of the HD menus, with a sample
//! window under the rows that shows them at once. See [`crate::style::Style`] for what they do.

use super::{Kind, RowDef, Source, Values};
use crate::cheats::Cheats;
use crate::style::Style;

/// The rows of the page.
pub const ROWS: &[RowDef] = &[
    RowDef {
        key: "access.text_size",
        label: "Text size",
        group: "Text",
        help: "Large makes the text of the menus a quarter bigger where it has room.",
        kind: Kind::Choice(&["Normal", "Large"]),
        source: Source::Stored { default: 0 },
    },
    RowDef {
        key: "access.contrast",
        label: "High contrast",
        group: "Menus",
        help: "White windows with black text and a black frame, instead of cream and orange.",
        kind: Kind::Choice(&["Off", "On"]),
        source: Source::Stored { default: 0 },
    },
    RowDef {
        key: "access.cursor",
        label: "Cursor",
        group: "Menus",
        help: "Safe is a bigger blue arrow with a bar under the row: it does not rely on orange against red.",
        kind: Kind::Choice(&["Orange", "Safe"]),
        source: Source::Stored { default: 0 },
    },
];

/// A line under the rows, if the page has one.
pub const NOTE: Option<&str> = None;

/// The index of the row `key` in `values` (its default when nothing is stored).
#[must_use]
pub fn index(values: &Values, key: &str) -> usize {
    ROWS.iter().find(|row| row.key == key).map_or(0, |row| row.index(values, &Cheats::neutral(), 0))
}

/// The style of the PC file's values: the default (the design as drawn) when nothing is stored.
#[must_use]
pub fn style(values: &Values) -> Style {
    Style {
        large_text: index(values, "access.text_size") == 1,
        contrast: index(values, "access.contrast") == 1,
        safe_cursor: index(values, "access.cursor") == 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_stored_is_the_design_as_drawn() {
        assert!(style(&Values::new()).is_default());
    }

    #[test]
    fn each_row_drives_its_own_choice() {
        let mut values = Values::new();
        values.set("access.contrast", 1);
        assert_eq!(style(&values), Style { contrast: true, ..Style::default() });
        values.set("access.text_size", 1);
        values.set("access.cursor", 1);
        assert_eq!(style(&values), Style { large_text: true, contrast: true, safe_cursor: true });
        values.set("access.cursor", 9);
        assert!(style(&values).safe_cursor, "a wild value clamps to the last choice");
    }
}
