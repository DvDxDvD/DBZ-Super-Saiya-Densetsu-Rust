//! The Audio page (this PC): the volume of the game's sound in steps of ten, and stereo or mono. The game's
//! sound is one mix when it reaches the host, so the page has a single volume (see `docs/re/audio-voices.md`).

use super::{Kind, RowDef, Source, Values};
use crate::cheats::Cheats;

/// The steps of the volume: 0 (silence) to 10 (the sound as the game makes it).
pub const STEPS: u8 = 11;
/// The top step.
pub const FULL: u8 = STEPS - 1;
/// How the sound reaches the speakers.
pub const OUTPUTS: &[&str] = &["Stereo", "Mono"];

/// The rows of the page.
pub const ROWS: &[RowDef] = &[
    RowDef {
        key: "audio.volume",
        label: "Volume",
        group: "Volume",
        help: "Volume of the game's sound, in steps of ten. Left and Right change it by one step.",
        kind: Kind::Slider { steps: STEPS },
        source: Source::Stored { default: FULL },
    },
    RowDef {
        key: "audio.output",
        label: "Sound",
        group: "Output",
        help: "Mono plays the same sound from both sides, for one speaker or one ear.",
        kind: Kind::Choice(OUTPUTS),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_stored_is_full_volume_in_stereo() {
        let values = Values::new();
        assert_eq!(index(&values, "audio.volume"), usize::from(FULL));
        assert_eq!(index(&values, "audio.output"), 0);
    }

    #[test]
    fn the_volume_stops_at_both_ends_and_a_wild_value_clamps() {
        let row = &ROWS[0];
        let (mut values, mut cheats) = (Values::new(), Cheats::neutral());
        row.step(&mut values, &mut cheats, 0, 1);
        assert_eq!(row.index(&values, &cheats, 0), 10, "no wrap past the top");
        for _ in 0..12 {
            row.step(&mut values, &mut cheats, 0, -1);
        }
        assert_eq!(row.index(&values, &cheats, 0), 0, "no wrap past silence");
        values.set("audio.volume", 99);
        assert_eq!(row.index(&values, &cheats, 0), 10);
    }
}
