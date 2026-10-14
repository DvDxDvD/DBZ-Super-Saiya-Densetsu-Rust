//! SDL3 events to `egui::RawInput`.
//!
//! The adapter turns the mouse, wheel, window focus and (while a text field has the focus) the
//! keyboard into egui events. Keys the game uses never reach egui unless a text field has the
//! keyboard: `Tab` and the letters belong to the game and its hotkeys otherwise.

use egui::{Event, Key, Modifiers, MouseWheelUnit, PointerButton, Pos2, RawInput, Rect, vec2};
use sdl3::event::{Event as SdlEvent, WindowEvent};
use sdl3::keyboard::{Mod, Scancode};
use sdl3::mouse::{MouseButton, MouseWheelDirection};

/// How SDL's window coordinates relate to egui's points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geometry {
    /// Pixels per SDL window coordinate (the window's pixel density).
    pub pixel_density: f32,
    /// Pixels per egui point (the window's display scale: what to multiply the UI by so it looks
    /// right on this display).
    pub pixels_per_point: f32,
}

impl Geometry {
    /// A position in SDL window coordinates, in egui points.
    #[must_use]
    pub fn point(self, x: f32, y: f32) -> Pos2 {
        let k = self.pixel_density / self.pixels_per_point;
        Pos2::new(x * k, y * k)
    }

    /// The screen of a window of `width` x `height` pixels, in points.
    #[must_use]
    pub fn screen_rect(self, width: u32, height: u32) -> Rect {
        Rect::from_min_size(
            Pos2::ZERO,
            vec2(width as f32 / self.pixels_per_point, height as f32 / self.pixels_per_point),
        )
    }
}

/// The egui key for a scancode, for the keys a text field uses.
#[must_use]
pub fn scancode_to_key(sc: Scancode) -> Option<Key> {
    Some(match sc {
        Scancode::Up => Key::ArrowUp,
        Scancode::Down => Key::ArrowDown,
        Scancode::Left => Key::ArrowLeft,
        Scancode::Right => Key::ArrowRight,
        Scancode::Escape => Key::Escape,
        Scancode::Tab => Key::Tab,
        Scancode::Backspace => Key::Backspace,
        Scancode::Return | Scancode::KpEnter => Key::Enter,
        Scancode::Space => Key::Space,
        Scancode::Insert => Key::Insert,
        Scancode::Delete => Key::Delete,
        Scancode::Home => Key::Home,
        Scancode::End => Key::End,
        Scancode::PageUp => Key::PageUp,
        Scancode::PageDown => Key::PageDown,
        Scancode::A => Key::A,
        Scancode::B => Key::B,
        Scancode::C => Key::C,
        Scancode::D => Key::D,
        Scancode::E => Key::E,
        Scancode::F => Key::F,
        Scancode::G => Key::G,
        Scancode::H => Key::H,
        Scancode::I => Key::I,
        Scancode::J => Key::J,
        Scancode::K => Key::K,
        Scancode::L => Key::L,
        Scancode::M => Key::M,
        Scancode::N => Key::N,
        Scancode::O => Key::O,
        Scancode::P => Key::P,
        Scancode::Q => Key::Q,
        Scancode::R => Key::R,
        Scancode::S => Key::S,
        Scancode::T => Key::T,
        Scancode::U => Key::U,
        Scancode::V => Key::V,
        Scancode::W => Key::W,
        Scancode::X => Key::X,
        Scancode::Y => Key::Y,
        Scancode::Z => Key::Z,
        Scancode::_0 => Key::Num0,
        Scancode::_1 => Key::Num1,
        Scancode::_2 => Key::Num2,
        Scancode::_3 => Key::Num3,
        Scancode::_4 => Key::Num4,
        Scancode::_5 => Key::Num5,
        Scancode::_6 => Key::Num6,
        Scancode::_7 => Key::Num7,
        Scancode::_8 => Key::Num8,
        Scancode::_9 => Key::Num9,
        _ => return None,
    })
}

#[must_use]
pub fn pointer_button(button: MouseButton) -> Option<PointerButton> {
    Some(match button {
        MouseButton::Left => PointerButton::Primary,
        MouseButton::Right => PointerButton::Secondary,
        MouseButton::Middle => PointerButton::Middle,
        MouseButton::X1 => PointerButton::Extra1,
        MouseButton::X2 => PointerButton::Extra2,
        MouseButton::Unknown => return None,
    })
}

/// The modifier keys held, as egui wants them (`command` is control: this is not a Mac).
#[must_use]
pub fn modifiers(keymod: Mod) -> Modifiers {
    let ctrl = keymod.intersects(Mod::LCTRLMOD | Mod::RCTRLMOD);
    Modifiers {
        alt: keymod.intersects(Mod::LALTMOD | Mod::RALTMOD),
        ctrl,
        shift: keymod.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD),
        mac_cmd: false,
        command: ctrl,
    }
}

/// Collects the events of a loop iteration for egui.
#[derive(Default)]
pub struct InputAdapter {
    events: Vec<Event>,
    modifiers: Modifiers,
    /// The modifiers egui was last told about.
    reported_modifiers: Modifiers,
    /// Whether the window has the focus, once SDL has said so (it starts focused).
    focused: Option<bool>,
    focus_changed: bool,
}

impl InputAdapter {
    /// Converts one SDL event. `text_focus` is whether a text field has the keyboard (as of the
    /// last frame); `paste` reads the system clipboard when Ctrl+V asks for it.
    pub fn handle(
        &mut self,
        event: &SdlEvent,
        geometry: Geometry,
        text_focus: bool,
        paste: impl FnOnce() -> Option<String>,
    ) {
        match event {
            SdlEvent::MouseMotion { x, y, .. } => {
                self.events.push(Event::PointerMoved(geometry.point(*x, *y)));
            }
            SdlEvent::MouseButtonDown { mouse_btn, x, y, .. }
            | SdlEvent::MouseButtonUp { mouse_btn, x, y, .. } => {
                let pressed = matches!(event, SdlEvent::MouseButtonDown { .. });
                if let Some(button) = pointer_button(*mouse_btn) {
                    self.events.push(Event::PointerButton {
                        pos: geometry.point(*x, *y),
                        button,
                        pressed,
                        modifiers: self.modifiers,
                    });
                }
            }
            SdlEvent::MouseWheel { x, y, direction, .. } => {
                let flip = if *direction == MouseWheelDirection::Flipped { -1.0 } else { 1.0 };
                self.events.push(Event::MouseWheel {
                    unit: MouseWheelUnit::Line,
                    delta: vec2(x * flip, y * flip),
                    phase: egui::TouchPhase::Move,
                    modifiers: self.modifiers,
                });
            }
            SdlEvent::Window { win_event, .. } => match win_event {
                WindowEvent::MouseLeave => self.events.push(Event::PointerGone),
                WindowEvent::FocusGained => self.set_focus(true),
                WindowEvent::FocusLost => self.set_focus(false),
                _ => {}
            },
            SdlEvent::TextInput { text, .. } if text_focus => {
                if !text.chars().all(char::is_control) {
                    self.events.push(Event::Text(text.clone()));
                }
            }
            SdlEvent::KeyDown { scancode: Some(sc), keymod, repeat, .. } => {
                self.modifiers = modifiers(*keymod);
                if text_focus {
                    self.key(*sc, true, *repeat, paste);
                }
            }
            SdlEvent::KeyUp { scancode: Some(sc), keymod, .. } => {
                self.modifiers = modifiers(*keymod);
                // A release is harmless to egui and a held key must not stay held.
                self.key(*sc, false, false, || None);
            }
            _ => {}
        }
    }

    fn set_focus(&mut self, focused: bool) {
        self.focused = Some(focused);
        self.focus_changed = true;
    }

    fn key(
        &mut self,
        sc: Scancode,
        pressed: bool,
        repeat: bool,
        paste: impl FnOnce() -> Option<String>,
    ) {
        let Some(key) = scancode_to_key(sc) else { return };
        if pressed && self.modifiers.command {
            // Clipboard shortcuts are events of their own, not keys.
            match key {
                Key::C => return self.events.push(Event::Copy),
                Key::X => return self.events.push(Event::Cut),
                Key::V => {
                    if let Some(text) = paste() {
                        self.events.push(Event::Paste(text));
                    }
                    return;
                }
                _ => {}
            }
        }
        self.events.push(Event::Key {
            key,
            physical_key: Some(key),
            pressed,
            repeat,
            modifiers: self.modifiers,
        });
    }

    /// The input for the next egui frame: the events since the last one.
    pub fn take_raw_input(
        &mut self,
        geometry: Geometry,
        size_in_pixels: (u32, u32),
        time: f64,
        max_texture_side: usize,
    ) -> RawInput {
        let screen_rect = geometry.screen_rect(size_in_pixels.0, size_in_pixels.1);
        let mut input = RawInput {
            screen_rect: Some(screen_rect),
            max_texture_side: Some(max_texture_side),
            time: Some(time),
            events: std::mem::take(&mut self.events),
            ..RawInput::default()
        };
        if self.modifiers != self.reported_modifiers {
            self.reported_modifiers = self.modifiers;
            input.events.insert(0, Event::ModifiersChanged(self.modifiers));
        }
        input.viewports.entry(input.viewport_id).or_default().native_pixels_per_point =
            Some(geometry.pixels_per_point);
        input.focused = self.focused.unwrap_or(true);
        if std::mem::take(&mut self.focus_changed) {
            input.events.push(Event::WindowFocused(input.focused));
        }
        input
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GEOMETRY: Geometry = Geometry { pixel_density: 1.5, pixels_per_point: 1.5 };

    fn key_down(sc: Scancode, keymod: Mod) -> SdlEvent {
        SdlEvent::KeyDown {
            timestamp: 0,
            window_id: 0,
            keycode: None,
            scancode: Some(sc),
            keymod,
            repeat: false,
            which: 0,
            raw: 0,
        }
    }

    fn key_up(sc: Scancode) -> SdlEvent {
        SdlEvent::KeyUp {
            timestamp: 0,
            window_id: 0,
            keycode: None,
            scancode: Some(sc),
            keymod: Mod::NOMOD,
            repeat: false,
            which: 0,
            raw: 0,
        }
    }

    fn no_events() -> Vec<Event> {
        Vec::new()
    }

    fn adapter_with(events: &[SdlEvent], text_focus: bool) -> Vec<Event> {
        let mut adapter = InputAdapter::default();
        for event in events {
            adapter.handle(event, GEOMETRY, text_focus, || Some("pasted".to_owned()));
        }
        let events = adapter.take_raw_input(GEOMETRY, (800, 600), 1.0, 8192).events;
        // The modifiers are told in a test of their own.
        events.into_iter().filter(|e| !matches!(e, Event::ModifiersChanged(_))).collect()
    }

    #[test]
    fn scancodes_map_to_the_keys_a_text_field_uses() {
        let table = [
            (Scancode::Left, Key::ArrowLeft),
            (Scancode::Return, Key::Enter),
            (Scancode::KpEnter, Key::Enter),
            (Scancode::Backspace, Key::Backspace),
            (Scancode::Escape, Key::Escape),
            (Scancode::A, Key::A),
            (Scancode::Z, Key::Z),
            (Scancode::_0, Key::Num0),
            (Scancode::_9, Key::Num9),
            (Scancode::PageDown, Key::PageDown),
        ];
        for (sc, key) in table {
            assert_eq!(scancode_to_key(sc), Some(key), "{sc:?}");
        }
        assert_eq!(scancode_to_key(Scancode::F5), None);
        assert_eq!(scancode_to_key(Scancode::LShift), None);
    }

    #[test]
    fn mouse_buttons_and_modifiers_map() {
        assert_eq!(pointer_button(MouseButton::Left), Some(PointerButton::Primary));
        assert_eq!(pointer_button(MouseButton::Right), Some(PointerButton::Secondary));
        assert_eq!(pointer_button(MouseButton::Unknown), None);
        let m = modifiers(Mod::LCTRLMOD | Mod::RSHIFTMOD);
        assert!(m.ctrl && m.command && m.shift && !m.alt && !m.mac_cmd);
        assert!(modifiers(Mod::NOMOD).is_none());
    }

    #[test]
    fn positions_go_from_sdl_coordinates_to_points() {
        // Density 2, display scale 1.5: 100 coordinates are 200 pixels are 133.3 points.
        let g = Geometry { pixel_density: 2.0, pixels_per_point: 1.5 };
        let p = g.point(100.0, 30.0);
        assert!((p.x - 133.333).abs() < 0.01 && (p.y - 40.0).abs() < 0.01, "{p:?}");
        let rect = g.screen_rect(1500, 750);
        assert_eq!((rect.width(), rect.height()), (1000.0, 500.0));
    }

    #[test]
    fn the_mouse_reaches_egui_in_points() {
        let g = Geometry { pixel_density: 1.0, pixels_per_point: 2.0 };
        let mut adapter = InputAdapter::default();
        let motion = SdlEvent::MouseMotion {
            timestamp: 0,
            window_id: 0,
            which: 0,
            mousestate: sdl3::mouse::MouseState::from_sdl_state(0),
            x: 100.0,
            y: 40.0,
            xrel: 0.0,
            yrel: 0.0,
        };
        adapter.handle(&motion, g, false, || None);
        let click = SdlEvent::MouseButtonDown {
            timestamp: 0,
            window_id: 0,
            which: 0,
            mouse_btn: MouseButton::Left,
            clicks: 1,
            x: 100.0,
            y: 40.0,
        };
        adapter.handle(&click, g, false, || None);
        let events = adapter.take_raw_input(g, (800, 600), 0.0, 8192).events;
        assert_eq!(events[0], Event::PointerMoved(Pos2::new(50.0, 20.0)));
        assert!(matches!(
            events[1],
            Event::PointerButton { pressed: true, button: PointerButton::Primary, .. }
        ));
    }

    #[test]
    fn the_wheel_scrolls_in_lines_and_a_flipped_wheel_is_turned_back() {
        let wheel = |direction| SdlEvent::MouseWheel {
            timestamp: 0,
            window_id: 0,
            which: 0,
            x: 0.0,
            y: 2.0,
            direction,
            mouse_x: 0.0,
            mouse_y: 0.0,
            integer_x: 0,
            integer_y: 2,
        };
        let events = adapter_with(&[wheel(MouseWheelDirection::Normal)], false);
        assert!(matches!(
            events[0],
            Event::MouseWheel { unit: MouseWheelUnit::Line, delta, .. } if delta == vec2(0.0, 2.0)
        ));
        let events = adapter_with(&[wheel(MouseWheelDirection::Flipped)], false);
        assert!(matches!(events[0], Event::MouseWheel { delta, .. } if delta == vec2(0.0, -2.0)));
    }

    #[test]
    fn the_game_keys_stay_out_of_egui_unless_a_text_field_has_the_keyboard() {
        let keys = [key_down(Scancode::X, Mod::NOMOD), key_down(Scancode::Tab, Mod::NOMOD)];
        assert_eq!(adapter_with(&keys, false), no_events());
        let events = adapter_with(&keys, true);
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], Event::Key { key: Key::X, pressed: true, .. }));
    }

    #[test]
    fn a_key_release_always_reaches_egui() {
        let events = adapter_with(&[key_up(Scancode::Left)], false);
        assert!(matches!(events[0], Event::Key { key: Key::ArrowLeft, pressed: false, .. }));
    }

    #[test]
    fn typed_text_only_goes_to_a_text_field() {
        let text = |s: &str| SdlEvent::TextInput { timestamp: 0, window_id: 0, text: s.to_owned() };
        assert_eq!(adapter_with(&[text("é")], false), no_events());
        assert_eq!(adapter_with(&[text("é")], true), [Event::Text("é".to_owned())]);
        assert_eq!(
            adapter_with(&[text("\u{7f}")], true),
            no_events(),
            "control characters are keys"
        );
    }

    #[test]
    fn clipboard_shortcuts_become_clipboard_events() {
        let ctrl = Mod::LCTRLMOD;
        assert_eq!(adapter_with(&[key_down(Scancode::C, ctrl)], true), [Event::Copy]);
        assert_eq!(adapter_with(&[key_down(Scancode::X, ctrl)], true), [Event::Cut]);
        assert_eq!(
            adapter_with(&[key_down(Scancode::V, ctrl)], true),
            [Event::Paste("pasted".to_owned())]
        );
        // Not in a text field: Ctrl+C is the game's business.
        assert_eq!(adapter_with(&[key_down(Scancode::C, ctrl)], false), no_events());
    }

    #[test]
    fn modifier_changes_are_told_once_before_the_events_that_need_them() {
        let mut adapter = InputAdapter::default();
        adapter.handle(&key_down(Scancode::LShift, Mod::LSHIFTMOD), GEOMETRY, false, || None);
        let events = adapter.take_raw_input(GEOMETRY, (800, 600), 0.0, 4096).events;
        assert_eq!(events, [Event::ModifiersChanged(Modifiers::SHIFT)]);
        assert_eq!(adapter.take_raw_input(GEOMETRY, (800, 600), 0.1, 4096).events, no_events());
    }

    #[test]
    fn leaving_and_focus_are_passed_on() {
        let window = |win_event| SdlEvent::Window { timestamp: 0, window_id: 0, win_event };
        let mut adapter = InputAdapter::default();
        adapter.handle(&window(WindowEvent::MouseLeave), GEOMETRY, false, || None);
        adapter.handle(&window(WindowEvent::FocusLost), GEOMETRY, false, || None);
        let input = adapter.take_raw_input(GEOMETRY, (800, 600), 2.0, 4096);
        assert_eq!(input.events, [Event::PointerGone, Event::WindowFocused(false)]);
        assert!(!input.focused);
        assert_eq!(input.time, Some(2.0));
        assert_eq!(input.max_texture_side, Some(4096));
        assert_eq!(input.viewport().native_pixels_per_point, Some(1.5));
        // Events are taken once.
        assert_eq!(adapter.take_raw_input(GEOMETRY, (800, 600), 3.0, 4096).events, no_events());
    }
}
