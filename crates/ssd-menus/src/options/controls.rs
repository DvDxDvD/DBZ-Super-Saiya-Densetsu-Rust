//! The Controls page (this PC): the device, then for each device the SNES buttons and the shortcuts, each a key
//! box. A key (or button) is stored as a code under `controls.kb.<action>` / `controls.pad.<action>`: the
//! keyboard's code is the USB usage number of the key (the number SDL calls a scancode), the gamepad's is the
//! SDL button number plus one; 0 means "no key". A code a key already has is refused (no swap).
//!
//! The rows are laid out two to a line, in this order (and the cursor moves in the same grid): the device
//! (alone), then Up and A, Down and B, Left and X, Right and Y, L and Start, R and Select (the d-pad and the
//! shoulders on the left, the buttons on the right), then the shortcuts two to a line.

use super::{Kind, RowDef, Source, Values};

/// The two devices of the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Device {
    Keyboard,
    Gamepad,
}

impl Device {
    /// The device's place in [`DEVICES`] and the value of `controls.device`.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Keyboard => 0,
            Self::Gamepad => 1,
        }
    }

    /// The device of place `index` (anything but 1 is the keyboard).
    #[must_use]
    pub const fn from_index(index: usize) -> Self {
        if index == 1 { Self::Gamepad } else { Self::Keyboard }
    }

    /// What the device calls what it is pressed: "Key" or "Button".
    #[must_use]
    pub const fn thing(self) -> &'static str {
        match self {
            Self::Keyboard => "Key",
            Self::Gamepad => "Button",
        }
    }
}

/// What the host gives the page while it waits for a key or button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capture {
    /// Esc: stop waiting, nothing changes.
    Cancel,
    /// Backspace: a shortcut loses its key (a SNES button keeps waiting).
    Clear,
    /// The key or button of the shown device that was pressed.
    Code(u8),
}

/// The labels of the device row.
pub const DEVICES: &[&str] = &["Keyboard", "Gamepad"];

/// The number of actions a device has: the 12 SNES buttons and the 4 shortcuts.
pub const ACTIONS: usize = 16;
/// How many of them are SNES buttons (the rest are shortcuts).
pub const BUTTONS: usize = 12;

/// The key of the page's device row.
pub const DEVICE_KEY: &str = "controls.device";

/// One action the page remaps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionDef {
    /// The suffix of the keys (`controls.kb.<id>`).
    pub id: &'static str,
    pub label: &'static str,
}

/// The actions, in the order of the page's grid (two to a line).
pub const ACTION_DEFS: [ActionDef; ACTIONS] = [
    ActionDef { id: "up", label: "Up" },
    ActionDef { id: "a", label: "A (confirm)" },
    ActionDef { id: "down", label: "Down" },
    ActionDef { id: "b", label: "B (back)" },
    ActionDef { id: "left", label: "Left" },
    ActionDef { id: "x", label: "X" },
    ActionDef { id: "right", label: "Right" },
    ActionDef { id: "y", label: "Y" },
    ActionDef { id: "l", label: "L" },
    ActionDef { id: "start", label: "Start" },
    ActionDef { id: "r", label: "R" },
    ActionDef { id: "select", label: "Select" },
    ActionDef { id: "speed", label: "Speed x1/x2/x4/x8" },
    ActionDef { id: "pause", label: "Pause" },
    ActionDef { id: "save_state", label: "Save state" },
    ActionDef { id: "load_state", label: "Load state" },
];

/// The keyboard codes that stand for today's keys (the game's own defaults), by action.
pub const KEYBOARD_DEFAULTS: [u8; ACTIONS] = [
    82,  // Up
    27,  // X: A
    81,  // Down
    29,  // Z: B
    80,  // Left
    22,  // S: X
    79,  // Right
    4,   // A: Y
    20,  // Q: L
    40,  // Return: Start
    26,  // W: R
    229, // Right Shift: Select
    43,  // Tab: speed
    19,  // P: pause
    62,  // F5: save state
    66,  // F9: load state
];

/// The gamepad codes (SDL button number plus one) that stand for today's buttons, by action; the shortcuts have none.
pub const GAMEPAD_DEFAULTS: [u8; ACTIONS] = [
    12, // D-pad Up
    2,  // East: A
    13, // D-pad Down
    1,  // South: B
    14, // D-pad Left
    4,  // North: X
    15, // D-pad Right
    3,  // West: Y
    10, // Left shoulder: L
    7,  // Start
    11, // Right shoulder: R
    5,  // Back: Select
    0, 0, 0, 0,
];

/// The keyboard keys the player cannot give to an action, with what they do: they belong to the window.
pub const RESERVED_KEYS: [(u8, &str); 6] = [
    (41, "Quit"),
    (58, "Debug panel"),
    (30, "Save slot 1"),
    (31, "Save slot 2"),
    (32, "Save slot 3"),
    (33, "Save slot 4"),
];

/// The code of Backspace, which clears a shortcut while a key is awaited.
pub const BACKSPACE: u8 = 42;
/// The code of Escape, which cancels while a key is awaited.
pub const ESCAPE: u8 = 41;

const SHOULDER_HELP: &str =
    "Press A, then the new key. Esc cancels. A key already used is refused.";

macro_rules! key_row {
    ($dev:literal, $id:literal, $label:literal, $group:literal, $help:expr, $default:expr) => {
        RowDef {
            key: concat!("controls.", $dev, ".", $id),
            label: $label,
            group: $group,
            help: $help,
            kind: Kind::Key,
            source: Source::Key { default: $default },
        }
    };
}

macro_rules! device_rows {
    ($dev:literal, $defaults:expr) => {
        [
            key_row!($dev, "up", "Up", "D-pad and shoulders", SHOULDER_HELP, $defaults[0]),
            key_row!(
                $dev,
                "a",
                "A (confirm)",
                "Buttons",
                "Confirms in the menus and talks on the map. Press A, then the new key. Esc cancels.",
                $defaults[1]
            ),
            key_row!($dev, "down", "Down", "D-pad and shoulders", SHOULDER_HELP, $defaults[2]),
            key_row!(
                $dev,
                "b",
                "B (back)",
                "Buttons",
                "Goes back in the menus. Press A, then the new key. Esc cancels.",
                $defaults[3]
            ),
            key_row!($dev, "left", "Left", "D-pad and shoulders", SHOULDER_HELP, $defaults[4]),
            key_row!($dev, "x", "X", "Buttons", SHOULDER_HELP, $defaults[5]),
            key_row!($dev, "right", "Right", "D-pad and shoulders", SHOULDER_HELP, $defaults[6]),
            key_row!($dev, "y", "Y", "Buttons", SHOULDER_HELP, $defaults[7]),
            key_row!($dev, "l", "L", "D-pad and shoulders", SHOULDER_HELP, $defaults[8]),
            key_row!($dev, "start", "Start", "Buttons", SHOULDER_HELP, $defaults[9]),
            key_row!($dev, "r", "R", "D-pad and shoulders", SHOULDER_HELP, $defaults[10]),
            key_row!($dev, "select", "Select", "Buttons", SHOULDER_HELP, $defaults[11]),
            key_row!(
                $dev,
                "speed",
                "Speed x1/x2/x4/x8",
                "Shortcuts",
                "Cycles the game speed. Works in every mode. Press A, then the new key. Esc cancels, Backspace clears.",
                $defaults[12]
            ),
            key_row!(
                $dev,
                "pause",
                "Pause",
                "Shortcuts",
                "Pauses the game. Press A, then the new key. Esc cancels, Backspace clears.",
                $defaults[13]
            ),
            key_row!(
                $dev,
                "save_state",
                "Save state",
                "Shortcuts",
                "Writes the state of the chosen slot (Classic and Enhanced). Esc cancels, Backspace clears.",
                $defaults[14]
            ),
            key_row!(
                $dev,
                "load_state",
                "Load state",
                "Shortcuts",
                "Loads the state of the chosen slot. Esc cancels, Backspace clears.",
                $defaults[15]
            ),
        ]
    };
}

const KEYBOARD_ROWS: [RowDef; ACTIONS] = device_rows!("kb", KEYBOARD_DEFAULTS);
const GAMEPAD_ROWS: [RowDef; ACTIONS] = device_rows!("pad", GAMEPAD_DEFAULTS);

/// The number of rows of the table: the device row, then the keyboard's, then the gamepad's.
const ROW_COUNT: usize = 1 + 2 * ACTIONS;

/// The rows of the page: the device, the 16 keyboard actions, the 16 gamepad actions.
pub const ROWS: &[RowDef] = &{
    let device = RowDef {
        key: DEVICE_KEY,
        label: "Device",
        group: "Device",
        help: "Each device has its own map. A gamepad is found by itself when it is plugged in.",
        kind: Kind::Choice(DEVICES),
        source: Source::Stored { default: 0 },
    };
    let mut rows = [device; ROW_COUNT];
    let mut at = 0;
    while at < ACTIONS {
        rows[1 + at] = KEYBOARD_ROWS[at];
        rows[1 + ACTIONS + at] = GAMEPAD_ROWS[at];
        at += 1;
    }
    rows
};

/// A line under the rows, if the page has one.
pub const NOTE: Option<&str> = None;

/// The row of `action` (0 to 15) of `device` in [`ROWS`].
#[must_use]
pub const fn row_of(device: Device, action: usize) -> usize {
    1 + device.index() * ACTIONS + action
}

/// The device the page shows.
#[must_use]
pub fn device(values: &Values) -> Device {
    Device::from_index(
        values.get(DEVICE_KEY).and_then(|value| usize::try_from(value).ok()).unwrap_or(0),
    )
}

/// The code of `action` on `device`: the stored one, else today's default. 0 is "no key".
#[must_use]
pub fn code(values: &Values, device: Device, action: usize) -> u8 {
    let Some(row) = ROWS.get(row_of(device, action.min(ACTIONS - 1))) else { return 0 };
    let defaults = match device {
        Device::Keyboard => KEYBOARD_DEFAULTS,
        Device::Gamepad => GAMEPAD_DEFAULTS,
    };
    values
        .get(row.key)
        .and_then(|value| u8::try_from(value).ok())
        .unwrap_or(defaults[action.min(ACTIONS - 1)])
}

/// What already holds `code` on `device`, other than `action`: the label of the action (or of the window's own key).
#[must_use]
pub fn owner(
    values: &Values,
    device: Device,
    action: usize,
    code_wanted: u8,
) -> Option<&'static str> {
    if code_wanted == 0 {
        return None;
    }
    if device == Device::Keyboard
        && let Some((_, what)) = RESERVED_KEYS.iter().find(|(reserved, _)| *reserved == code_wanted)
    {
        return Some(what);
    }
    (0..ACTIONS)
        .filter(|&other| other != action)
        .find(|&other| code(values, device, other) == code_wanted)
        .map(|other| ACTION_DEFS[other].label)
}

/// Gives `action` of `device` the code `wanted`.
///
/// # Errors
///
/// The label of the action that already uses the code: nothing changes (no swap).
pub fn assign(
    values: &mut Values,
    device: Device,
    action: usize,
    wanted: u8,
) -> Result<(), &'static str> {
    if let Some(by) = owner(values, device, action, wanted) {
        return Err(by);
    }
    store(values, device, action, wanted);
    Ok(())
}

/// Clears a shortcut (it has no key afterwards). The SNES buttons keep their key: returns whether it was cleared.
pub fn clear(values: &mut Values, device: Device, action: usize) -> bool {
    if !(BUTTONS..ACTIONS).contains(&action) {
        return false;
    }
    store(values, device, action, 0);
    true
}

/// Stores `value` for the action; a value equal to the default is forgotten, so the file holds only what differs.
fn store(values: &mut Values, device: Device, action: usize, value: u8) {
    let Some(row) = ROWS.get(row_of(device, action)) else { return };
    let default = match device {
        Device::Keyboard => KEYBOARD_DEFAULTS[action],
        Device::Gamepad => GAMEPAD_DEFAULTS[action],
    };
    if value == default {
        values.remove(row.key);
    } else {
        values.set(row.key, i64::from(value));
    }
}

/// The names of the keyboard keys by code: the ones the page can show.
const KEY_NAMES: &[(u8, &str)] = &[
    (40, "Enter"),
    (41, "Esc"),
    (42, "Backspace"),
    (43, "Tab"),
    (44, "Space"),
    (45, "-"),
    (46, "="),
    (47, "["),
    (48, "]"),
    (49, "\\"),
    (51, ";"),
    (52, "'"),
    (53, "`"),
    (54, ","),
    (55, "."),
    (56, "/"),
    (57, "Caps Lock"),
    (70, "Print Screen"),
    (71, "Scroll Lock"),
    (72, "Pause"),
    (73, "Insert"),
    (74, "Home"),
    (75, "Page Up"),
    (76, "Delete"),
    (77, "End"),
    (78, "Page Down"),
    (79, "Right"),
    (80, "Left"),
    (81, "Down"),
    (82, "Up"),
    (83, "Num Lock"),
    (84, "Num /"),
    (85, "Num *"),
    (86, "Num -"),
    (87, "Num +"),
    (88, "Num Enter"),
    (98, "Num 0"),
    (99, "Num ."),
    (224, "L-Ctrl"),
    (225, "L-Shift"),
    (226, "L-Alt"),
    (228, "R-Ctrl"),
    (229, "R-Shift"),
    (230, "R-Alt"),
];

/// The names of the gamepad buttons by code (SDL's button number plus one).
const PAD_NAMES: [&str; 26] = [
    "South",
    "East",
    "West",
    "North",
    "Back",
    "Guide",
    "Start",
    "Left stick",
    "Right stick",
    "Left bumper",
    "Right bumper",
    "D-pad Up",
    "D-pad Down",
    "D-pad Left",
    "D-pad Right",
    "Misc 1",
    "Right paddle 1",
    "Left paddle 1",
    "Right paddle 2",
    "Left paddle 2",
    "Touchpad",
    "Misc 2",
    "Misc 3",
    "Misc 4",
    "Misc 5",
    "Misc 6",
];

/// The text on a key box for `code` on `device`.
#[must_use]
pub fn name(device: Device, code: u8) -> String {
    if code == 0 {
        return "None".to_owned();
    }
    match device {
        Device::Gamepad => PAD_NAMES
            .get(usize::from(code) - 1)
            .map_or_else(|| format!("Button {code}"), |&n| n.to_owned()),
        Device::Keyboard => match code {
            4..=29 => char::from(b'A' + (code - 4)).to_string(),
            30..=38 => char::from(b'1' + (code - 30)).to_string(),
            39 => "0".to_owned(),
            58..=69 => format!("F{}", code - 57),
            89..=97 => format!("Num {}", code - 88),
            _ => KEY_NAMES
                .iter()
                .find(|(known, _)| *known == code)
                .map_or_else(|| format!("Key {code}"), |&(_, text)| text.to_owned()),
        },
    }
}

/// The code a name stands for on `device` (the names [`name`] gives, in any case), for the UI scripts.
#[must_use]
pub fn code_named(device: Device, text: &str) -> Option<u8> {
    let wanted = text.trim();
    (1..=255_u8).find(|&code| {
        name(device, code).eq_ignore_ascii_case(wanted) && !name(device, code).starts_with("Key ")
    })
}

/// The message of the help strip when `by` already holds the key the player chose.
#[must_use]
pub fn refusal(device: Device, by: &str) -> String {
    format!(
        "{} already used by {by}. Choose another {}.",
        device.thing(),
        device.thing().to_lowercase()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_rows() -> Vec<&'static RowDef> {
        ROWS.iter().collect()
    }

    #[test]
    fn the_table_has_the_device_then_sixteen_actions_per_device() {
        assert_eq!(ROWS.len(), 33);
        assert_eq!(ROWS[0].key, "controls.device");
        assert_eq!(ROWS[row_of(Device::Keyboard, 0)].key, "controls.kb.up");
        assert_eq!(ROWS[row_of(Device::Keyboard, 15)].key, "controls.kb.load_state");
        assert_eq!(ROWS[row_of(Device::Gamepad, 0)].key, "controls.pad.up");
        assert_eq!(ROWS[row_of(Device::Gamepad, 15)].key, "controls.pad.load_state");
        for device in [Device::Keyboard, Device::Gamepad] {
            for (action, def) in ACTION_DEFS.iter().enumerate() {
                let row = &ROWS[row_of(device, action)];
                assert_eq!(row.label, def.label);
                assert!(row.key.ends_with(&format!(".{}", def.id)), "{}", row.key);
            }
        }
        assert_eq!(all_rows().iter().filter(|row| row.kind == Kind::Key).count(), 32);
    }

    #[test]
    fn the_defaults_are_the_keys_and_buttons_of_the_game_today() {
        let values = Values::new();
        let keys: Vec<String> = (0..ACTIONS)
            .map(|a| name(Device::Keyboard, code(&values, Device::Keyboard, a)))
            .collect();
        assert_eq!(
            keys,
            [
                "Up", "X", "Down", "Z", "Left", "S", "Right", "A", "Q", "Enter", "W", "R-Shift",
                "Tab", "P", "F5", "F9"
            ]
        );
        let pad: Vec<String> = (0..ACTIONS)
            .map(|a| name(Device::Gamepad, code(&values, Device::Gamepad, a)))
            .collect();
        assert_eq!(
            &pad[..12],
            [
                "D-pad Up",
                "East",
                "D-pad Down",
                "South",
                "D-pad Left",
                "North",
                "D-pad Right",
                "West",
                "Left bumper",
                "Start",
                "Right bumper",
                "Back"
            ]
        );
        assert!(pad[12..].iter().all(|text| text == "None"), "no pad shortcut by default");
        assert!(values.is_empty());
    }

    #[test]
    fn a_used_key_is_refused_and_nothing_changes() {
        let mut values = Values::new();
        // Z is B's key; the A action wants it.
        assert_eq!(assign(&mut values, Device::Keyboard, 1, 29), Err("B (back)"));
        assert!(values.is_empty());
        // The window's own keys are refused by what they do.
        assert_eq!(assign(&mut values, Device::Keyboard, 1, 41), Err("Quit"));
        assert_eq!(assign(&mut values, Device::Keyboard, 1, 58), Err("Debug panel"));
        assert_eq!(assign(&mut values, Device::Keyboard, 1, 31), Err("Save slot 2"));
        // The same key on the other device is free: each device has its own map.
        assert_eq!(assign(&mut values, Device::Gamepad, 1, 1), Err("B (back)"));
        assert_eq!(assign(&mut values, Device::Gamepad, 12, 29), Ok(()));
        assert_eq!(
            refusal(Device::Keyboard, "B (back)"),
            "Key already used by B (back). Choose another key."
        );
        assert_eq!(
            refusal(Device::Gamepad, "Start"),
            "Button already used by Start. Choose another button."
        );
    }

    #[test]
    fn a_free_key_is_taken_and_gives_the_old_key_back_to_nobody() {
        let mut values = Values::new();
        assert_eq!(assign(&mut values, Device::Keyboard, 1, 9), Ok(())); // F
        assert_eq!(code(&values, Device::Keyboard, 1), 9);
        assert_eq!(values.get("controls.kb.a"), Some(9));
        // X, A's old key, is free now.
        assert_eq!(assign(&mut values, Device::Keyboard, 5, 27), Ok(()));
        // Pressing the key an action already has is not a conflict with itself, and the default is not stored.
        assert_eq!(assign(&mut values, Device::Keyboard, 1, 27), Err("X"));
        assert_eq!(assign(&mut values, Device::Keyboard, 5, 22), Ok(()));
        assert_eq!(values.get("controls.kb.x"), None, "back to the default: forgotten");
    }

    #[test]
    fn a_shortcut_clears_and_a_button_does_not() {
        let mut values = Values::new();
        assert!(clear(&mut values, Device::Keyboard, 13));
        assert_eq!(code(&values, Device::Keyboard, 13), 0);
        assert_eq!(values.get("controls.kb.pause"), Some(0));
        assert_eq!(name(Device::Keyboard, 0), "None");
        assert!(!clear(&mut values, Device::Keyboard, 1), "A keeps its key");
        assert_eq!(code(&values, Device::Keyboard, 1), 27);
        // The cleared shortcut's old key is free for another action.
        assert_eq!(assign(&mut values, Device::Keyboard, 14, 19), Ok(()));
        // A cleared (empty) shortcut conflicts with nothing: two can be empty.
        assert!(clear(&mut values, Device::Keyboard, 15));
        assert_eq!(owner(&values, Device::Keyboard, 0, 0), None);
    }

    #[test]
    fn the_values_round_trip_through_the_pc_file_and_wild_ones_are_tolerated() {
        let mut values = Values::new();
        assign(&mut values, Device::Keyboard, 0, 9).unwrap();
        assign(&mut values, Device::Gamepad, 2, 16).unwrap();
        clear(&mut values, Device::Keyboard, 12);
        values.set(DEVICE_KEY, 1);
        let read = Values::from_text(&values.to_text()).unwrap();
        assert_eq!(read, values);
        assert_eq!(device(&read), Device::Gamepad);
        assert_eq!(code(&read, Device::Keyboard, 0), 9);
        assert_eq!(code(&read, Device::Keyboard, 12), 0);
        assert_eq!(code(&read, Device::Gamepad, 2), 16);
        // Missing, unknown and out-of-range values: defaults, never a panic.
        let mut rough = Values::new();
        rough.set("controls.kb.up", 9999);
        rough.set("controls.kb.a", -3);
        rough.set("controls.pad.b", 300);
        rough.set("controls.kb.future_action", 7);
        rough.set(DEVICE_KEY, 77);
        assert_eq!(code(&rough, Device::Keyboard, 0), 82);
        assert_eq!(code(&rough, Device::Keyboard, 1), 27);
        assert_eq!(code(&rough, Device::Gamepad, 3), 1);
        assert_eq!(device(&rough), Device::Keyboard);
        assert_eq!(rough.get("controls.kb.future_action"), Some(7), "unknown keys are kept");
    }

    #[test]
    fn the_names_read_back_to_their_codes() {
        for device in [Device::Keyboard, Device::Gamepad] {
            for code in 1..=90_u8 {
                let text = name(device, code);
                if text.starts_with("Key ") || text.starts_with("Button ") {
                    continue;
                }
                assert_eq!(code_named(device, &text), Some(code), "{text}");
            }
        }
        assert_eq!(code_named(Device::Keyboard, "z"), Some(29));
        assert_eq!(code_named(Device::Keyboard, "f12"), Some(69));
        assert_eq!(code_named(Device::Keyboard, "esc"), Some(ESCAPE));
        assert_eq!(code_named(Device::Gamepad, "south"), Some(1));
        assert_eq!(code_named(Device::Keyboard, "nonsense"), None);
    }

    #[test]
    fn every_default_is_free_of_the_windows_own_keys_and_of_each_other() {
        for device in [Device::Keyboard, Device::Gamepad] {
            let values = Values::new();
            for action in 0..ACTIONS {
                let own = code(&values, device, action);
                assert_eq!(owner(&values, device, action, own), None, "{action}");
            }
        }
    }
}
