//! The Controls page's choices as the window uses them (`controls.*` of the PC file): which key or button is
//! each SNES button and each shortcut, and how a raw press becomes what the page waits for.
//!
//! The defaults are the game's keys from before the page (README, "Keys"). The map is data: the page edits the
//! stored codes, and the window builds [`Bindings`] from them.

use crate::hotkeys::Hotkey;
use crate::input::PadButton;
use sdl3::gamepad::Button;
use sdl3::keyboard::Scancode;
use ssd_menus::options::Values;
use ssd_menus::options::controls::{self, ACTIONS, BUTTONS, Capture, Device};

/// The SNES button of each of the first twelve actions, in the page's order.
const BUTTON_OF: [PadButton; BUTTONS] = [
    PadButton::Up,
    PadButton::A,
    PadButton::Down,
    PadButton::B,
    PadButton::Left,
    PadButton::X,
    PadButton::Right,
    PadButton::Y,
    PadButton::L,
    PadButton::Start,
    PadButton::R,
    PadButton::Select,
];

/// The shortcut of each of the last five actions.
const SHORTCUT_OF: [Hotkey; ACTIONS - BUTTONS] =
    [Hotkey::CycleSpeed, Hotkey::Pause, Hotkey::SaveState, Hotkey::LoadState, Hotkey::Screenshot];

/// The code the page keeps for a key (the USB usage number SDL calls the scancode); 0 for a key it cannot name.
fn key_code(sc: Scancode) -> u8 {
    u8::try_from(sc as i32).unwrap_or(0)
}

/// The code the page keeps for a gamepad button: SDL's number plus one (0 is none).
fn button_code(button: Button) -> u8 {
    u8::try_from(button as i32 + 1).unwrap_or(0)
}

/// The keys and buttons of the player's map: the code of each action, per device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bindings {
    keyboard: [u8; ACTIONS],
    gamepad: [u8; ACTIONS],
}

impl Default for Bindings {
    /// The game's keys and buttons from before the page.
    fn default() -> Self {
        Self::from_values(&Values::new())
    }
}

impl Bindings {
    /// The map of the PC file's values (a missing or wild value is the action's default).
    #[must_use]
    pub fn from_values(values: &Values) -> Self {
        let codes = |device| std::array::from_fn(|action| controls::code(values, device, action));
        Self { keyboard: codes(Device::Keyboard), gamepad: codes(Device::Gamepad) }
    }

    /// The SNES button a key stands for.
    #[must_use]
    pub(crate) fn key_button(&self, sc: Scancode) -> Option<PadButton> {
        let code = key_code(sc);
        if code == 0 {
            return None;
        }
        self.keyboard[..BUTTONS].iter().position(|&held| held == code).map(|at| BUTTON_OF[at])
    }

    /// The SNES button a gamepad button stands for.
    #[must_use]
    pub(crate) fn pad_button(&self, button: Button) -> Option<PadButton> {
        let code = button_code(button);
        self.gamepad[..BUTTONS].iter().position(|&held| held == code).map(|at| BUTTON_OF[at])
    }

    /// What a key does for the window: the shortcuts of the page, then the keys that are not remappable.
    #[must_use]
    pub(crate) fn key_hotkey(&self, sc: Scancode) -> Option<Hotkey> {
        let code = key_code(sc);
        let shortcut = if code == 0 {
            None
        } else {
            self.keyboard[BUTTONS..].iter().position(|&held| held == code)
        };
        shortcut.map(|at| SHORTCUT_OF[at]).or_else(|| crate::hotkeys::fixed_hotkey(sc))
    }

    /// What a gamepad button does for the window: a shortcut, when the player gave it one.
    #[must_use]
    pub(crate) fn pad_hotkey(&self, button: Button) -> Option<Hotkey> {
        let code = button_code(button);
        self.gamepad[BUTTONS..].iter().position(|&held| held == code).map(|at| SHORTCUT_OF[at])
    }
}

/// What a key press means to a Controls page that waits for one on `device`: Esc cancels, Backspace clears a
/// shortcut, another key is the answer on the keyboard page and nothing on the gamepad page.
#[must_use]
pub fn capture_key(device: Device, sc: Scancode) -> Option<Capture> {
    match sc {
        Scancode::Escape => Some(Capture::Cancel),
        Scancode::Backspace => Some(Capture::Clear),
        _ if device == Device::Keyboard && key_code(sc) != 0 => Some(Capture::Code(key_code(sc))),
        _ => None,
    }
}

/// What a gamepad button press means to a Controls page that waits for one on `device`.
#[must_use]
pub fn capture_button(device: Device, button: Button) -> Option<Capture> {
    (device == Device::Gamepad).then(|| Capture::Code(button_code(button)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_map_is_the_games_keys_from_before_the_page() {
        let map = Bindings::default();
        for (key, button) in [
            (Scancode::Up, PadButton::Up),
            (Scancode::Down, PadButton::Down),
            (Scancode::Left, PadButton::Left),
            (Scancode::Right, PadButton::Right),
            (Scancode::X, PadButton::A),
            (Scancode::Z, PadButton::B),
            (Scancode::S, PadButton::X),
            (Scancode::A, PadButton::Y),
            (Scancode::Q, PadButton::L),
            (Scancode::W, PadButton::R),
            (Scancode::Return, PadButton::Start),
            (Scancode::RShift, PadButton::Select),
        ] {
            assert_eq!(map.key_button(key), Some(button), "{key:?}");
        }
        assert_eq!(map.key_button(Scancode::F5), None);
        assert_eq!(map.key_button(Scancode::Unknown), None);
        for (button, snes) in [
            (Button::DPadUp, PadButton::Up),
            (Button::DPadDown, PadButton::Down),
            (Button::DPadLeft, PadButton::Left),
            (Button::DPadRight, PadButton::Right),
            (Button::East, PadButton::A),
            (Button::South, PadButton::B),
            (Button::North, PadButton::X),
            (Button::West, PadButton::Y),
            (Button::LeftShoulder, PadButton::L),
            (Button::RightShoulder, PadButton::R),
            (Button::Start, PadButton::Start),
            (Button::Back, PadButton::Select),
        ] {
            assert_eq!(map.pad_button(button), Some(snes), "{button:?}");
        }
        assert_eq!(map.pad_button(Button::Guide), None);
    }

    #[test]
    fn the_default_shortcuts_and_the_fixed_keys_are_the_hotkeys_from_before_the_page() {
        let map = Bindings::default();
        for (key, hotkey) in [
            (Scancode::F5, Hotkey::SaveState),
            (Scancode::F9, Hotkey::LoadState),
            (Scancode::F12, Hotkey::Screenshot),
            (Scancode::F1, Hotkey::TogglePanel),
            (Scancode::_1, Hotkey::Slot(1)),
            (Scancode::_4, Hotkey::Slot(4)),
            (Scancode::Tab, Hotkey::CycleSpeed),
            (Scancode::P, Hotkey::Pause),
            (Scancode::Escape, Hotkey::Quit),
        ] {
            assert_eq!(map.key_hotkey(key), Some(hotkey), "{key:?}");
        }
        assert_eq!(map.key_hotkey(Scancode::X), None);
        assert_eq!(map.pad_hotkey(Button::Start), None, "no pad shortcut by default");
        for button in [Button::South, Button::Back, Button::Guide, Button::Touchpad] {
            assert_eq!(map.pad_hotkey(button), None, "{button:?}: the screenshot has no button");
        }
    }

    #[test]
    fn the_codes_of_the_page_are_the_scancodes_and_buttons_of_sdl() {
        // The page names keys by USB usage number; this pins its table to SDL's own enum.
        for (code, key) in [
            (4, Scancode::A),
            (29, Scancode::Z),
            (30, Scancode::_1),
            (40, Scancode::Return),
            (41, Scancode::Escape),
            (42, Scancode::Backspace),
            (43, Scancode::Tab),
            (58, Scancode::F1),
            (62, Scancode::F5),
            (66, Scancode::F9),
            (69, Scancode::F12),
            (79, Scancode::Right),
            (82, Scancode::Up),
            (229, Scancode::RShift),
        ] {
            assert_eq!(key_code(key), code, "{key:?}");
        }
        assert_eq!(button_code(Button::South), 1);
        assert_eq!(button_code(Button::Back), 5);
        assert_eq!(button_code(Button::DPadRight), 15);
        assert_eq!(button_code(Button::Touchpad), 21);
    }

    #[test]
    fn a_remapped_key_moves_the_button_and_the_old_key_is_free() {
        let mut values = Values::new();
        controls::assign(&mut values, Device::Keyboard, 1, 9).unwrap(); // A on F
        controls::clear(&mut values, Device::Keyboard, 13); // no Pause key
        controls::assign(&mut values, Device::Keyboard, 14, 19).unwrap(); // Save state on P
        let map = Bindings::from_values(&values);
        assert_eq!(map.key_button(Scancode::F), Some(PadButton::A));
        assert_eq!(map.key_button(Scancode::X), None, "A's old key does nothing now");
        assert_eq!(map.key_hotkey(Scancode::P), Some(Hotkey::SaveState));
        assert_eq!(map.key_hotkey(Scancode::F5), None, "Save state's old key does nothing now");
        assert_eq!(map.key_hotkey(Scancode::F1), Some(Hotkey::TogglePanel), "the fixed keys stay");
    }

    #[test]
    fn a_gamepad_shortcut_is_a_hotkey_and_a_gamepad_button_can_move() {
        let mut values = Values::new();
        controls::assign(&mut values, Device::Gamepad, 12, button_code(Button::RightStick))
            .unwrap();
        // East is A's button, so B cannot take it.
        assert!(
            controls::assign(&mut values, Device::Gamepad, 3, button_code(Button::East)).is_err()
        );
        controls::assign(&mut values, Device::Gamepad, 1, button_code(Button::LeftStick)).unwrap();
        let map = Bindings::from_values(&values);
        assert_eq!(map.pad_hotkey(Button::RightStick), Some(Hotkey::CycleSpeed));
        assert_eq!(map.pad_button(Button::LeftStick), Some(PadButton::A));
        assert_eq!(map.pad_button(Button::East), None);
    }

    #[test]
    fn the_press_a_waiting_page_takes_depends_on_the_device() {
        assert_eq!(capture_key(Device::Keyboard, Scancode::Escape), Some(Capture::Cancel));
        assert_eq!(capture_key(Device::Gamepad, Scancode::Escape), Some(Capture::Cancel));
        assert_eq!(capture_key(Device::Keyboard, Scancode::Backspace), Some(Capture::Clear));
        assert_eq!(capture_key(Device::Keyboard, Scancode::G), Some(Capture::Code(10)));
        assert_eq!(capture_key(Device::Gamepad, Scancode::G), None);
        assert_eq!(capture_button(Device::Gamepad, Button::East), Some(Capture::Code(2)));
        assert_eq!(capture_button(Device::Keyboard, Button::East), None);
    }

    #[test]
    fn wild_values_leave_a_usable_map() {
        let mut values = Values::new();
        values.set("controls.kb.up", 9999);
        values.set("controls.pad.a", -1);
        values.set("controls.kb.nothing", 5);
        assert_eq!(Bindings::from_values(&values), Bindings::default());
    }
}
