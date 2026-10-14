//! Sync points: the per-frame moments where lockstep compares two machines (M1b).
//!
//! The game runs its per-frame logic inside the NMI handler and waits in an idle loop between
//! frames. A sync event is the first arrival at a sync point (the idle loop) after each NMI: the
//! frame's work is done and the machine waits for the next one. A lag frame (an NMI that lands
//! while the frame's work still runs) shows as an NMI delta of 2 between two events.

use crate::machine::MAX_STACK_MOVE;
use crate::{AudioSink, Joypad, Machine, OracleError, Stopped, Stops};

/// The first arrival at a sync point after an NMI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncEvent {
    /// Hook key of the sync point (the physical `PBR << 16 | PC`).
    pub key: u32,
    /// [`Machine::frame_count`] at the event.
    pub frame: u64,
    /// [`Machine::nmi_count`] at the event; the difference to the previous event's is the
    /// number of NMIs (frames started) in between.
    pub nmi_count: u64,
    /// [`Machine::total_master_cycles`] at the event.
    pub master_cycles: u64,
}

impl Machine {
    /// Runs like [`Machine::run_frame_with`] (hook hits dispatched as described in
    /// [`Machine::run_frame`]) until a sync event, and returns it; `Ok(None)` once
    /// `budget_frames` frames have been rendered since the call without one (at once for a
    /// budget of 0). [`Machine::run`] with [`Stops::sync`]. Each tick holds the input of the
    /// frame it belongs to, `input(frame)`, a dispatch that renders a frame and runs on included
    /// (see [`Machine::run_frame_with`]): each call asks `input` for each frame it runs, in
    /// order, from the current one (a `Verify` snapshot may ask again; `input` must give the same
    /// joypad for the same frame).
    ///
    /// A sync event is an arrival at a sync point (a [`HookAction::SyncPoint`] entry of the
    /// installed table, see [`HookEntry::sync_point`]) with [`Machine::nmi_count`] above the count
    /// of the last event. The arrival is not dispatched: the machine stops with the CPU about to
    /// fetch the sync point's opcode, which the next tick fetches (the hit is taken, so it is not
    /// reported again). Other arrivals at sync points resume the original code; every arrival
    /// counts in the sync point's [`HookStats::calls`], except those inside the original of a
    /// `Verify` call.
    ///
    /// Only arrivals at the top level of the run are events. An arrival inside a dispatch (the
    /// original of a `Verify` call, original code a Rust routine calls) that would be one counts
    /// in the sync point's [`HookStats::sync_inside_dispatch`] instead: the dispatch runs on, and
    /// the event comes at the first arrival after it, later than on a machine without that
    /// routine.
    ///
    /// The count of the last event starts at 0 (power-on) and is set to the current count by
    /// [`Machine::load_state`] (the counter is not part of the state): after a load, the next
    /// event follows the next NMI. [`Machine::snapshot`] copies it. Only runs with
    /// [`Stops::sync`] report and record events: [`Machine::run_frame`] and [`Machine::tick`]
    /// pass sync points without recording an event.
    ///
    /// # Errors
    ///
    /// As for [`Machine::run_frame`].
    ///
    /// [`HookAction::SyncPoint`]: crate::HookAction::SyncPoint
    /// [`HookEntry::sync_point`]: crate::HookEntry::sync_point
    /// [`HookStats::calls`]: crate::HookStats::calls
    /// [`HookStats::sync_inside_dispatch`]: crate::HookStats::sync_inside_dispatch
    pub fn run_until_sync(
        &mut self,
        input: &mut dyn FnMut(u64) -> Joypad,
        audio: &mut dyn AudioSink,
        budget_frames: u64,
    ) -> Result<Option<SyncEvent>, OracleError> {
        let stops = Stops { sync: true, ..Stops::default() };
        match self.run(&stops, input, audio, budget_frames)? {
            Stopped::Sync(event) => Ok(Some(event)),
            _ => Ok(None),
        }
    }
}

/// Bytes a single tick may move S down by and stay on its stack: an allocated stack frame
/// (`TSC; SEC; SBC #n; TCS`). A longer jump down switches stacks.
const MAX_FRAME_BYTES: u16 = 256;

/// The stack a machine is on, for the dead stack of lockstep (see [`crate::lockstep`]): its
/// lowest S (the floor), whose bytes from one above it up to S at a sync event were pushed to
/// and are free, and its base, the highest S on it.
///
/// It follows S after every tick, the ticks of the original code a dispatch runs included (a
/// `Verify` call's original, code called from a Rust routine), all with the same rule: a tick
/// moving S by at most 4, or by more inside an interrupt entry, pushes or pulls on the stack. A
/// longer move (`TCS`, `TXS`) stays on the stack if it lands between the floor and the base
/// (a deallocation, `TSC; CLC; ADC #n; TCS`) or at most 256 bytes below S (an allocation);
/// otherwise it switches stacks, and the floor and the base start over at the new S (the
/// game's reset code moves S from the power-on page up to its own stack). The base is the S the
/// stack was set to, raised by any S above it; a move above it lands on another stack (or
/// returns from a buffer filled through the stack, whose floor is then lost: a narrower dead
/// stack, never a wider one). After the return of a Rust routine (performed, not ticked), S
/// is taken as is.
///
/// Lockstep raises the floor after each event that compared equal ([`StackWatch::raise_floor`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StackWatch {
    floor: u16,
    base: u16,
    previous: u16,
}

impl StackWatch {
    /// A watch on a stack set at the stack pointer `s`.
    pub(crate) fn new(s: u16) -> Self {
        Self { floor: s, base: s, previous: s }
    }

    /// The lowest stack pointer on the current stack, since it was set or the floor raised.
    pub(crate) fn floor(self) -> u16 {
        self.floor
    }

    /// Raises the floor to `floor`, at most to the current S (a lower floor stays).
    pub(crate) fn raise_floor(&mut self, floor: u16) {
        self.floor = self.floor.max(floor.min(self.previous));
    }

    /// A tick left S at `s`; `handling_interrupt`: the CPU is in an interrupt entry.
    pub(crate) fn ticked(&mut self, s: u16, handling_interrupt: bool) {
        let moved = s.abs_diff(self.previous);
        let jump = !handling_interrupt && moved > MAX_STACK_MOVE;
        let switch = jump && (s > self.base || (s < self.previous && moved > MAX_FRAME_BYTES));
        if switch {
            *self = Self::new(s);
        } else {
            self.floor = self.floor.min(s);
            self.base = self.base.max(s);
            self.previous = s;
        }
    }

    /// A dispatch left S at `s` (its original code's ticks already watched).
    pub(crate) fn dispatched(&mut self, s: u16) {
        self.floor = self.floor.min(s);
        self.base = self.base.max(s);
        self.previous = s;
    }
}

#[cfg(test)]
mod tests {
    use super::StackWatch;

    #[test]
    fn the_watch_follows_pushes_and_starts_over_at_a_stack_switch() {
        let mut watch = StackWatch::new(0x01FF);
        watch.ticked(0x01FD, false);
        assert_eq!(watch.floor(), 0x01FD);
        // TXS up to the game's stack: the power-on page is no stack anymore.
        watch.ticked(0x0FFF, false);
        assert_eq!(watch.floor(), 0x0FFF);
        for s in [0x0FFE, 0x0FFC, 0x0FF8, 0x0FFC, 0x0FFF] {
            watch.ticked(s, false);
        }
        assert_eq!(watch.floor(), 0x0FF8, "pushes of a few bytes per tick");
        // An interrupt entry is no switch, whatever S does meanwhile.
        watch.ticked(0x0FF0, true);
        assert_eq!(watch.floor(), 0x0FF0);
        // A dispatch returns a whole call at once.
        watch.dispatched(0x0FFF);
        assert_eq!(watch.floor(), 0x0FF0);
        watch.ticked(0x0FFE, false);
        assert_eq!(watch.floor(), 0x0FF0, "a push after the return");
    }

    #[test]
    fn frames_allocated_and_freed_in_one_tick_stay_on_the_stack() {
        let mut watch = StackWatch::new(0x0FFF);
        watch.ticked(0x0FF0, true);
        watch.ticked(0x0FF8, false);
        // `TCS` allocating 32 bytes, then freeing them; then a 256-byte frame.
        watch.ticked(0x0FD8, false);
        assert_eq!(watch.floor(), 0x0FD8);
        watch.ticked(0x0FF8, false);
        assert_eq!(watch.floor(), 0x0FD8, "a deallocation keeps the floor");
        watch.ticked(0x0FD8 - 256 + 0x20, false);
        watch.ticked(0x0FFF, false);
        assert_eq!(watch.floor(), 0x0FD8 - 256 + 0x20, "256 bytes is a frame");
        // Back at the base: still the same stack.
        assert_eq!((watch.base, watch.previous), (0x0FFF, 0x0FFF));
    }

    #[test]
    fn far_jumps_down_and_jumps_above_the_base_switch_stacks() {
        let mut watch = StackWatch::new(0x1FF0);
        watch.ticked(0x1FEE, false);
        // A buffer filled through the stack, far below.
        watch.ticked(0x00F0, false);
        assert_eq!((watch.floor(), watch.base), (0x00F0, 0x00F0));
        watch.ticked(0x00EC, false);
        // Back: above the buffer's base, another stack.
        watch.ticked(0x1FEE, false);
        assert_eq!((watch.floor(), watch.base), (0x1FEE, 0x1FEE));
        // A pull above the base raises it; a jump above it switches.
        watch.ticked(0x1FF0, false);
        assert_eq!(watch.base, 0x1FF0);
        watch.ticked(0x1FE8, false);
        watch.ticked(0x1FF0, false);
        assert_eq!(watch.floor(), 0x1FE8, "a deallocation up to the base");
        watch.ticked(0x1FF8, false);
        assert_eq!(watch.floor(), 0x1FF8, "a jump above the base");
        // 257 bytes down is a switch.
        watch.ticked(0x1FF8 - 257, false);
        assert_eq!(watch.base, 0x1FF8 - 257);
    }

    #[test]
    fn the_floor_is_raised_up_to_s_at_most() {
        let mut watch = StackWatch::new(0x0FFF);
        watch.ticked(0x0FC0, false);
        watch.ticked(0x0FFF, false);
        watch.raise_floor(0x0FE0);
        assert_eq!(watch.floor(), 0x0FE0);
        watch.raise_floor(0x0FD0);
        assert_eq!(watch.floor(), 0x0FE0, "never lowered");
        watch.raise_floor(0x2000);
        assert_eq!(watch.floor(), 0x0FFF, "at most S");
        watch.ticked(0x0FFD, false);
        assert_eq!(watch.floor(), 0x0FFD, "pushes lower it again");
    }
}
