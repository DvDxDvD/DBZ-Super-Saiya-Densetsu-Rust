//! Keyboard and gamepad to SNES joypad mapping (the map itself is `options::controls::Bindings`, the Controls
//! page's data). Keyboard and gamepad are tracked separately so a
//! button stays pressed while either device holds it.

use crate::options::controls::Bindings;
use sdl3::gamepad::Button;
use sdl3::keyboard::Scancode;
use ssd_oracle::Joypad;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PadButton {
    Up,
    Down,
    Left,
    Right,
    A,
    B,
    X,
    Y,
    L,
    R,
    Start,
    Select,
}

const ALL: [PadButton; 12] = [
    PadButton::Up,
    PadButton::Down,
    PadButton::Left,
    PadButton::Right,
    PadButton::A,
    PadButton::B,
    PadButton::X,
    PadButton::Y,
    PadButton::L,
    PadButton::R,
    PadButton::Start,
    PadButton::Select,
];

#[derive(Default)]
pub struct InputState {
    keys: [bool; 12],
    pad: [bool; 12],
    /// The touch controls (M9), in the order of [`ALL`].
    touch: [bool; 12],
}

fn idx(b: PadButton) -> usize {
    ALL.iter().position(|&x| x == b).expect("listed")
}

impl InputState {
    pub fn key(&mut self, bindings: &Bindings, sc: Scancode, pressed: bool) {
        if let Some(b) = bindings.key_button(sc) {
            self.keys[idx(b)] = pressed;
        }
    }

    pub fn gamepad_button(&mut self, bindings: &Bindings, b: Button, pressed: bool) {
        if let Some(b) = bindings.pad_button(b) {
            self.pad[idx(b)] = pressed;
        }
    }

    /// The buttons the touch controls hold now (in the order up, down, left, right, A, B, X, Y, L, R, Start, Select).
    pub fn set_touch(&mut self, held: [bool; 12]) {
        self.touch = held;
    }

    /// Release everything (focus lost, gamepad removed).
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn joypad(&self) -> Joypad {
        let on = |b: PadButton| self.keys[idx(b)] || self.pad[idx(b)] || self.touch[idx(b)];
        Joypad {
            up: on(PadButton::Up),
            down: on(PadButton::Down),
            left: on(PadButton::Left),
            right: on(PadButton::Right),
            a: on(PadButton::A),
            b: on(PadButton::B),
            x: on(PadButton::X),
            y: on(PadButton::Y),
            l: on(PadButton::L),
            r: on(PadButton::R),
            start: on(PadButton::Start),
            select: on(PadButton::Select),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sdl3::keyboard::Scancode;
    use ssd_menus::options::{
        Values,
        controls::{self, Device},
    };

    #[test]
    fn keyboard_and_gamepad_combine_and_release_independently() {
        let map = Bindings::default();
        let mut s = InputState::default();
        s.key(&map, Scancode::X, true);
        s.gamepad_button(&map, sdl3::gamepad::Button::East, true);
        assert!(s.joypad().a);
        s.key(&map, Scancode::X, false);
        assert!(s.joypad().a, "gamepad still holds A");
        s.gamepad_button(&map, sdl3::gamepad::Button::East, false);
        assert!(!s.joypad().a);
    }

    #[test]
    fn a_remapped_key_drives_the_joypad_and_the_old_one_does_not() {
        let mut values = Values::new();
        controls::assign(&mut values, Device::Keyboard, 1, 9).unwrap();
        let map = Bindings::from_values(&values);
        let mut s = InputState::default();
        s.key(&map, Scancode::F, true);
        assert!(s.joypad().a);
        s.key(&map, Scancode::F, false);
        s.key(&map, Scancode::X, true);
        assert!(!s.joypad().a, "X is free now");
        assert_eq!(s.joypad(), ssd_oracle::Joypad::default());
    }
}
