//! Keyboard and gamepad to SNES joypad mapping. Keyboard and gamepad are tracked separately so a
//! button stays pressed while either device holds it.

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

pub(crate) fn key_to_button(sc: Scancode) -> Option<PadButton> {
    Some(match sc {
        Scancode::Up => PadButton::Up,
        Scancode::Down => PadButton::Down,
        Scancode::Left => PadButton::Left,
        Scancode::Right => PadButton::Right,
        Scancode::X => PadButton::A,
        Scancode::Z => PadButton::B,
        Scancode::S => PadButton::X,
        Scancode::A => PadButton::Y,
        Scancode::Q => PadButton::L,
        Scancode::W => PadButton::R,
        Scancode::Return => PadButton::Start,
        Scancode::RShift => PadButton::Select,
        _ => return None,
    })
}

/// SNES layout on a standard gamepad: positions match (East = A, South = B, North = X, West = Y).
pub(crate) fn gamepad_to_button(b: Button) -> Option<PadButton> {
    Some(match b {
        Button::DPadUp => PadButton::Up,
        Button::DPadDown => PadButton::Down,
        Button::DPadLeft => PadButton::Left,
        Button::DPadRight => PadButton::Right,
        Button::East => PadButton::A,
        Button::South => PadButton::B,
        Button::North => PadButton::X,
        Button::West => PadButton::Y,
        Button::LeftShoulder => PadButton::L,
        Button::RightShoulder => PadButton::R,
        Button::Start => PadButton::Start,
        Button::Back => PadButton::Select,
        _ => return None,
    })
}

#[derive(Default)]
pub struct InputState {
    keys: [bool; 12],
    pad: [bool; 12],
}

fn idx(b: PadButton) -> usize {
    ALL.iter().position(|&x| x == b).expect("listed")
}

impl InputState {
    pub fn key(&mut self, sc: Scancode, pressed: bool) {
        if let Some(b) = key_to_button(sc) {
            self.keys[idx(b)] = pressed;
        }
    }

    pub fn gamepad_button(&mut self, b: Button, pressed: bool) {
        if let Some(b) = gamepad_to_button(b) {
            self.pad[idx(b)] = pressed;
        }
    }

    /// Release everything (focus lost, gamepad removed).
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn joypad(&self) -> Joypad {
        let on = |b: PadButton| self.keys[idx(b)] || self.pad[idx(b)];
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

    #[test]
    fn keyboard_layout_matches_readme() {
        assert_eq!(key_to_button(Scancode::X), Some(PadButton::A));
        assert_eq!(key_to_button(Scancode::Z), Some(PadButton::B));
        assert_eq!(key_to_button(Scancode::S), Some(PadButton::X));
        assert_eq!(key_to_button(Scancode::A), Some(PadButton::Y));
        assert_eq!(key_to_button(Scancode::Return), Some(PadButton::Start));
        assert_eq!(key_to_button(Scancode::RShift), Some(PadButton::Select));
        assert_eq!(key_to_button(Scancode::F5), None);
    }

    #[test]
    fn keyboard_and_gamepad_combine_and_release_independently() {
        let mut s = InputState::default();
        s.key(Scancode::X, true);
        s.gamepad_button(sdl3::gamepad::Button::East, true);
        assert!(s.joypad().a);
        s.key(Scancode::X, false);
        assert!(s.joypad().a, "gamepad still holds A");
        s.gamepad_button(sdl3::gamepad::Button::East, false);
        assert!(!s.joypad().a);
    }
}
