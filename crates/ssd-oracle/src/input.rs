//! Joypad input by frame: the poller the core asks on every tick, and the joypad bits of replays.
//!
//! The core polls the input on every tick, after the tick's video work: on the tick that renders
//! a frame, `render_frame` runs before `poll`. The poller looks the joypad up by the frame the
//! tick belongs to, the count of frames rendered before it ([`Machine::frame_count`] when the
//! tick starts): the tick that renders frame `f + 1` is still frame `f`'s, its last one. That is
//! exactly the span of one [`Machine::run_frame`] call (its last tick is the one that renders),
//! so running frame by frame with `input(f)` held and running with `input` as a lookup feed the
//! same joypad to every tick. A hooked call that renders a frame and runs on (an original under
//! `Verify`, a charge, original code a Rust routine calls) gets the next frame's input from the
//! lookup after the render, as the unhooked run does: the auto-joypad read, which ends some
//! 4 350 master cycles after the render, latches the right frame's buttons.
//!
//! The frame count is a plain field of the machine, read when each tick starts (the core takes
//! one tick per call wherever this poller is used). A counter shared with the frame capture,
//! moved on by `render_frame` before `poll`, would give the rendering tick to frame `f + 1`
//! instead: the core keeps the last joypad it polled in its state, so a run looked up by frame
//! would then differ from the `run_frame(input(f))` run a recording is made with (in the saved
//! state, and in a manual joypad read on that tick).
//!
//! [`Machine::frame_count`]: crate::Machine::frame_count
//! [`Machine::run_frame`]: crate::Machine::run_frame

use crate::Joypad;
use jgenesis_common::frontend::InputPoller;
use snes_core::input::{SnesController, SnesInputs};

/// Player 1 holds `input`; no controller in port 2. Changing this, or the frame
/// [`FramePoller`] looks a tick's joypad up by, changes what replays play: bump
/// [`ORACLE_REVISION`](crate::ORACLE_REVISION).
pub(crate) fn joypad_inputs(input: Joypad) -> SnesInputs {
    SnesInputs { p1: SnesController::Gamepad(input), p2: SnesController::None }
}

/// The joypad of one frame, looked up once.
pub(crate) struct InputCache {
    /// The frame `inputs` is for; [`InputCache::EMPTY`] for none.
    frame: u64,
    inputs: SnesInputs,
}

impl InputCache {
    /// No frame cached (frame counts never get there).
    const EMPTY: u64 = u64::MAX;

    /// Forgets the cached frame: the next poll looks its frame up. Every run that takes a new
    /// lookup starts with this.
    pub(crate) fn clear(&mut self) {
        self.frame = Self::EMPTY;
    }

    #[cold]
    #[inline(never)]
    fn fill(&mut self, frame: u64, lookup: &mut dyn FnMut(u64) -> Joypad) {
        self.frame = frame;
        self.inputs = joypad_inputs(lookup(frame));
    }
}

impl Default for InputCache {
    fn default() -> Self {
        Self { frame: Self::EMPTY, inputs: SnesInputs::default() }
    }
}

/// The poller of one core tick: the joypad of `frame`, from the cache or else from `lookup`.
pub(crate) struct FramePoller<'a> {
    pub cache: &'a mut InputCache,
    /// The frame the tick belongs to: the frames rendered before it.
    pub frame: u64,
    pub lookup: &'a mut dyn FnMut(u64) -> Joypad,
}

impl InputPoller<SnesInputs> for FramePoller<'_> {
    #[inline(always)]
    fn poll(&mut self) -> &SnesInputs {
        if self.cache.frame != self.frame {
            self.cache.fill(self.frame, self.lookup);
        }
        &self.cache.inputs
    }
}

/// The buttons of a joypad word, bit by bit from bit 0: B, Y, Select, Start, Up, Down, Left,
/// Right, A, X, L, R, the order in which the controller sends them (and the core's joypad word;
/// the auto-joypad read stores the first one sent in the top bit, B is bit 15 of `JOY1`). Bits
/// 12–15 are unused.
pub const JOYPAD_BUTTONS: [&str; 12] =
    ["B", "Y", "Select", "Start", "Up", "Down", "Left", "Right", "A", "X", "L", "R"];

/// The bits of the buttons of [`JOYPAD_BUTTONS`]: bits 0–11 used, 12–15 always clear.
pub const JOYPAD_USED_BITS: u16 = 0x0FFF;

/// `joypad` as a 16-bit word, one bit per button in the order of [`JOYPAD_BUTTONS`].
#[must_use]
pub fn joypad_bits(joypad: Joypad) -> u16 {
    [
        joypad.b,
        joypad.y,
        joypad.select,
        joypad.start,
        joypad.up,
        joypad.down,
        joypad.left,
        joypad.right,
        joypad.a,
        joypad.x,
        joypad.l,
        joypad.r,
    ]
    .iter()
    .enumerate()
    .fold(0, |bits, (bit, &pressed)| bits | (u16::from(pressed) << bit))
}

/// The joypad of a word of [`joypad_bits`]; bits 12–15 are ignored.
#[must_use]
pub fn joypad_from_bits(bits: u16) -> Joypad {
    let pressed = |bit: u16| bits & (1 << bit) != 0;
    Joypad {
        b: pressed(0),
        y: pressed(1),
        select: pressed(2),
        start: pressed(3),
        up: pressed(4),
        down: pressed(5),
        left: pressed(6),
        right: pressed(7),
        a: pressed(8),
        x: pressed(9),
        l: pressed(10),
        r: pressed(11),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_word_of_the_used_bits_round_trips() {
        for bits in 0..=JOYPAD_USED_BITS {
            assert_eq!(joypad_bits(joypad_from_bits(bits)), bits);
        }
        assert_eq!(joypad_from_bits(0xF000), Joypad::default(), "unused bits are ignored");
    }

    #[test]
    fn each_button_has_its_documented_bit() {
        let single = |bit: usize| joypad_from_bits(1 << bit);
        let buttons = |j: Joypad| {
            [j.b, j.y, j.select, j.start, j.up, j.down, j.left, j.right, j.a, j.x, j.l, j.r]
        };
        for (bit, name) in JOYPAD_BUTTONS.iter().enumerate() {
            let pressed: Vec<usize> =
                (0..12).filter(|&i| buttons(single(bit))[i]).collect::<Vec<_>>();
            assert_eq!(pressed, [bit], "{name}");
        }
        assert!(single(8).a && single(7).right && single(3).start && single(11).r);
    }

    #[test]
    fn the_poller_looks_each_frame_up_once() {
        let mut cache = InputCache::default();
        let mut asked = Vec::new();
        let mut lookup = |frame: u64| {
            asked.push(frame);
            Joypad { a: frame % 2 == 1, ..Joypad::default() }
        };
        let mut a_at = |frame: u64| {
            let mut poller = FramePoller { cache: &mut cache, frame, lookup: &mut lookup };
            matches!(poller.poll().p1, SnesController::Gamepad(pad) if pad.a)
        };
        assert_eq!(
            [a_at(4), a_at(4), a_at(5), a_at(5), a_at(6)],
            [false, false, true, true, false]
        );
        assert_eq!(asked, [4, 5, 6]);

        cache.clear();
        let mut poller =
            FramePoller { cache: &mut cache, frame: 6, lookup: &mut |_| Joypad::default() };
        poller.poll();
        assert_eq!(cache.frame, 6, "a cleared cache looks its frame up again");
    }
}
