//! What a Rust routine works on while it runs in place of the original (spec §5.3).

use crate::hooks::{P_M, P_X, split_charge};
use crate::{AudioSink, Joypad, Machine, OracleError, Registers, ReturnKind, StatusRegister};

const P_N: u8 = 0x80;
const P_V: u8 = 0x40;
const P_Z: u8 = 0x02;
const P_C: u8 = 0x01;

/// The live machine as seen by a Rust routine: CPU registers and flags, and the bus as the CPU
/// sees it. The routine starts at the hooked opcode fetch; the return (RTS/RTL) is performed
/// after it.
///
/// Reads and writes have the CPU's side effects and take no time; writes are recorded in the
/// CPU-write log when it is enabled. A write that starts a GPDMA (a nonzero value to `$420B`)
/// lets the transfer finish, CPU held, before it returns. Time passes only through
/// [`Ctx::charge`], through the original code run by [`Ctx::call_original`] and, after the
/// routine, the rest of the cost it declares with [`Ctx::set_cost`].
pub struct Ctx<'a> {
    machine: &'a mut Machine,
    /// Hook key of the routine running.
    routine: u32,
    /// The joypad by frame of the run the routine is called in.
    input: &'a mut dyn FnMut(u64) -> Joypad,
    audio: &'a mut dyn AudioSink,
    /// Whether a tick made on behalf of the routine rendered a frame.
    frame_rendered: bool,
    /// The first core error of such a tick; later ticks are skipped.
    error: Option<OracleError>,
    /// The own cost declared with [`Ctx::set_cost`].
    cost: Option<u64>,
    /// The own instructions declared with [`Ctx::set_instructions`].
    instructions: Option<u64>,
    /// CPU-busy master cycles of the original code run by [`Ctx::call_original`].
    callee_cycles: u64,
    /// Instructions of the code called by [`Ctx::call_original`]: those it fetched, and what the
    /// `Rust` runs it dispatched stand for.
    callee_instructions: f64,
    /// The lowest stack pointer the original code run by [`Ctx::call_original`] reached (its
    /// return address pushed first).
    lowest_s: Option<u16>,
    /// Where the original code goes on instead of the routine returning ([`Ctx::resume_at`]).
    resume: Option<u32>,
    /// The routine checks the timing window itself ([`Ctx::mark_gated`]).
    gated: bool,
    /// The routine gave up: it ran far past any real call ([`Ctx::mark_runaway`]).
    runaway: bool,
    /// The routine drops its caller's return address ([`Ctx::mark_dropped`]).
    dropped: bool,
    /// The interrupt lines were sampled by the routine ([`Ctx::suppress_final_poll`]).
    no_poll: bool,
}

/// What a Rust routine left when it returned.
pub(crate) struct Finished {
    /// A frame was rendered while it ran.
    pub rendered: bool,
    /// The own cost it declared ([`Ctx::set_cost`]).
    pub cost: Option<u64>,
    /// The own instructions it declared ([`Ctx::set_instructions`]).
    pub instructions: Option<u64>,
    /// CPU-busy master cycles of the original code it ran ([`Ctx::call_original`]).
    pub callee_cycles: u64,
    /// Instructions of the code it called ([`Ctx::call_original`]): those the machine fetched,
    /// and what the `Rust` runs inside it stand for.
    pub callee_instructions: f64,
    /// CPU-busy master cycles from the hit to the return, `callee_cycles` included (set by
    /// [`Machine::run_routine`]).
    pub spent: u64,
    /// The lowest stack pointer the original code it ran reached, if it ran any: the stack
    /// bytes from there up to the return are dead.
    pub lowest_s: Option<u16>,
    /// The routine did not return: the original goes on at this 24-bit address
    /// ([`Ctx::resume_at`]).
    pub resume: Option<u32>,
    /// The routine checked the timing window itself ([`Ctx::mark_gated`]).
    pub gated: bool,
    /// The routine gave up as a runaway ([`Ctx::mark_runaway`]).
    pub runaway: bool,
    /// The routine dropped its caller's return address ([`Ctx::mark_dropped`]).
    pub dropped: bool,
    /// The machine does not sample the interrupt lines after the routine
    /// ([`Ctx::suppress_final_poll`]).
    pub no_poll: bool,
}

impl<'a> Ctx<'a> {
    pub(crate) fn new(
        machine: &'a mut Machine,
        routine: u32,
        input: &'a mut dyn FnMut(u64) -> Joypad,
        audio: &'a mut dyn AudioSink,
    ) -> Self {
        Self {
            machine,
            routine,
            input,
            audio,
            frame_rendered: false,
            error: None,
            cost: None,
            instructions: None,
            callee_cycles: 0,
            callee_instructions: 0.0,
            lowest_s: None,
            resume: None,
            gated: false,
            runaway: false,
            dropped: false,
            no_poll: false,
        }
    }

    /// What the routine left (`spent` is 0), or the first core error.
    pub(crate) fn finish(self) -> Result<Finished, OracleError> {
        match self.error {
            Some(error) => Err(error),
            None => Ok(Finished {
                rendered: self.frame_rendered,
                cost: self.cost,
                instructions: self.instructions,
                callee_cycles: self.callee_cycles,
                callee_instructions: self.callee_instructions,
                spent: 0,
                lowest_s: self.lowest_s,
                resume: self.resume,
                gated: self.gated,
                runaway: self.runaway,
                dropped: self.dropped,
                no_poll: self.no_poll,
            }),
        }
    }

    // Time

    /// Lets `master_cycles` CPU-busy master cycles pass now, CPU held, as the original spends
    /// them at this point of the routine: a routine that writes timing-sensitive registers
    /// charges the cycles before each write, so the write lands where the original makes it,
    /// and one that calls original code charges the cycles before the call (e.g. its `JSR`), so
    /// the called code runs when the original runs it. Exact for 0, 6, 8 and every even value
    /// from 12 up (other values are rounded to the nearest of these). The cycles count towards
    /// the cost of [`Ctx::set_cost`].
    pub fn charge(&mut self, master_cycles: u64) {
        if self.error.is_some() {
            return;
        }
        let (fast, slow) = split_charge(master_cycles);
        match self.machine.hold_cpu_cycles(&mut *self.input, &mut *self.audio, fast, slow) {
            Ok(rendered) => self.frame_rendered |= rendered,
            Err(error) => self.error = Some(error),
        }
    }

    /// Lets a memory refresh that came due at the end of the cycles charged so far happen
    /// before the next access, as the original's next cycle would find it: call it after
    /// [`Ctx::charge`] and before a read whose time matters (a hardware register). A write
    /// needs no settling: it is applied at the end of its own cycle, which is charged first.
    pub fn settle(&mut self) {
        if self.error.is_some() {
            return;
        }
        match self.machine.settle(&mut *self.input, &mut *self.audio) {
            Ok(rendered) => self.frame_rendered |= rendered,
            Err(error) => self.error = Some(error),
        }
    }

    /// Declares the routine's own cost for this call: the CPU-busy master cycles the original
    /// spends from the hit to its return on the path this call takes, minus the time of the
    /// original code this call runs through [`Ctx::call_original`] (which passes as that code
    /// runs, and is measured). The routines the original calls are part of its time, hooked
    /// ones included (checked as nested calls in `Verify`): the own cost includes every one
    /// this routine does in Rust, and excludes those it runs with [`Ctx::call_original`].
    ///
    /// In `Rust` mode the CPU is held after the routine until the declared cost has passed
    /// since the hit outside the called code (what [`Ctx::charge`] and GPDMA drains already
    /// spent counts; nothing more if they spent more): the call lasts the declared cost plus
    /// the called code's time. The final hold is exact if what remains is 0, 6, 8 or an even
    /// value from 12 up: a remainder of 2, 4 or 10 master cycles after [`Ctx::charge`] cannot be
    /// charged exactly (it is rounded to 0, 6 or 8), so charge in steps that leave none of
    /// them. In `Verify` the declared cost plus the called code's time is compared with the
    /// original's measured cost (`cost_mismatch`).
    ///
    /// Spending more than declared before the return, or a declared cost plus called code's
    /// time above the entry's `cycles`, counts in
    /// [`HookStats::cost_over_bound`](crate::HookStats::cost_over_bound). The last declaration
    /// counts.
    pub fn set_cost(&mut self, own: u64) {
        self.cost = Some(own);
    }

    /// Declares the routine's own instructions for this call: the opcode fetches the original
    /// makes from the hit to its return on the path this call takes, minus the instructions of
    /// the original code this call runs through [`Ctx::call_original`] (those are measured). The
    /// coverage metric ([`Machine::coverage`](crate::Machine::coverage)) takes it for this run in
    /// place of the routine's mean over the `Verify` calls
    /// ([`HookStats::mean_instructions`](crate::HookStats::mean_instructions)). The mean is right
    /// only if the `Rust` runs are a random sample of the calls, and they are not: the timing
    /// window sends the calls near a timing event to the original, so a routine whose length
    /// depends on where in the frame it is called is estimated off by the mean, and exactly by
    /// the count. In `Verify` the declared count plus the called code's instructions is compared
    /// with the original's (`instructions_mismatch`). Optional; the last declaration counts.
    pub fn set_instructions(&mut self, own: u64) {
        self.instructions = Some(own);
    }

    /// Runs original code: the routine at the 24-bit `address`, which returns with `ret`, runs
    /// on the machine as emulated code (it pushes a return address and starts there) until it
    /// returns, then this routine goes on with the machine as the code left it, PBR and PC
    /// excepted (both are this routine's again). The called code's time passes as it runs
    /// (counted apart from the cost of [`Ctx::set_cost`]); hook hits in it are dispatched as
    /// usual, its entry's included, so a hooked routine called this way runs as its mode says,
    /// except a routine already running in Rust (this one, or one it runs inside): its original
    /// code runs. A routine can so hand a path it does not port back to its own original
    /// (`address` = its own entry, from its hit, before any change; its own cost is then 0).
    ///
    /// The code returns to a trap: `bank:21FF`, `bank` being `address`'s. With `ret` =
    /// [`ReturnKind::Rts`], `$21FE` is pushed (PBR = `bank`, as a `JSR` within the bank leaves
    /// it); with [`ReturnKind::Rtl`], `bank` then `$21FE` (as a `JSL`). An arrival there is the
    /// return only with S back where it was before the push (in banks `$40–$7F`, `$21FF` can
    /// be memory the code runs through). The stack bytes it
    /// pushes and the code's own pushes are dead after the return; a `Verify` call does not
    /// compare them. An NMI or IRQ raised while this routine's time was charged is taken before
    /// the called code's first instruction (as after the original's `JSR`); one that comes
    /// where the code returns to the trap is taken there, and the call completes after it. Its
    /// entry then lasts 4 master cycles less than at `SlowROM` code: its first two cycles read
    /// at the CPU's address, and the trap is in a 6-master-cycle region in banks `$00–$3F` and
    /// `$80–$BF`. In `Rust` mode, with a right `cycles`, no interrupt reaches a call.
    ///
    /// In `Rust` mode the code runs on the live machine. In `Verify`, the Rust routine runs on a
    /// snapshot of the entry state and so does the code it calls: there, `Verify` hooks run as
    /// in `Rust` mode, and the counters of the hits are discarded with the snapshot. The timing
    /// window of `Rust` mode covers the whole call, the called code included: see
    /// [`HookEntry::cycles`](crate::HookEntry::cycles).
    ///
    /// # Errors
    ///
    /// Returns [`OracleError::Run`] if the code has not returned within the routine's budget (2
    /// frames of master cycles, or its `budget_frames`; naming this routine and `address`), or if
    /// the core fails. The CPU registers are then put back as before the call (the code's other effects stay), after the instruction
    /// in progress when the budget ran out is over (within a frame); every later
    /// `call_original` returns the error again, and the dispatch returns it after this routine.
    /// After a core error the CPU may be left between two cycles of an instruction: do not run
    /// the machine on.
    pub fn call_original(&mut self, address: u32, ret: ReturnKind) -> Result<(), OracleError> {
        if let Some(error) = &self.error {
            return Err(duplicate(error));
        }
        let accounted = self.machine.rust_instructions_run();
        let call = self.machine.call_original(
            self.routine,
            address,
            ret,
            &mut *self.input,
            &mut *self.audio,
        );
        match call {
            Ok(run) => {
                self.frame_rendered |= run.rendered;
                self.callee_cycles += run.cpu_cycles;
                self.callee_instructions +=
                    run.instructions as f64 + (self.machine.rust_instructions_run() - accounted);
                self.lowest_s = Some(self.lowest_s.map_or(run.min_s, |s| s.min(run.min_s)));
                Ok(())
            }
            Err(error) => {
                let returned = duplicate(&error);
                self.error = Some(error);
                Err(returned)
            }
        }
    }

    // Recompiled routines (generated code that manages its own timing windows)

    /// Ends the routine without a return: the original code goes on at the 24-bit `address`
    /// with the registers this routine left (PBR and PC are set from `address`). The stack must
    /// hold what the original holds at that point. A routine that resumes at its own entry
    /// before it spent any time is treated as declined (the original runs it, as after a
    /// timing fallback).
    pub fn resume_at(&mut self, address: u32) {
        self.resume = Some(address & 0xFF_FFFF);
    }

    /// Declares that the routine checks the timing window itself before every stretch of code
    /// it runs ([`Ctx::cycles_until_event`], [`Ctx::window`]) and declares the cost it spent
    /// ([`Ctx::set_cost`]), so the entry's `bound` covers only its first stretch and
    /// `cost_over_bound` does not apply to it.
    pub fn mark_gated(&mut self) {
        self.gated = true;
    }

    /// The instructions a generated function may run on a `Verify` snapshot before it counts as
    /// running away: a multiple of what the original needed.
    #[must_use]
    pub fn runaway_limit(&self) -> u64 {
        self.machine.runaway_limit()
    }

    /// Declares that the routine gave up because it ran far longer than any call of the original
    /// can (an endless loop): on a `Verify` snapshot this counts as a divergence.
    pub fn mark_runaway(&mut self) {
        self.runaway = true;
    }

    /// Declares that the routine pulled its caller's return address off the stack other than by
    /// returning (`PLA PLA` and the like: an early exit of the caller). On a `Verify` snapshot
    /// the call is not comparable and is counted in [`HookStats::early_exit`].
    ///
    /// [`HookStats::early_exit`]: crate::HookStats::early_exit
    pub fn mark_dropped(&mut self) {
        self.dropped = true;
    }

    /// Records that the routine pushed down to the stack pointer `s` (a recompiled routine
    /// pushes without S moving on the machine): the bytes from there up to the return are dead
    /// stack for the comparison.
    pub fn note_lowest_s(&mut self, s: u16) {
        self.lowest_s = Some(self.lowest_s.map_or(s, |low| low.min(s)));
    }

    /// Samples the NMI and IRQ lines as the last cycle of an instruction does, before that cycle's
    /// bus access takes effect: a recompiled routine calls it before a write that changes the
    /// lines (`NMITIMEN`, the IRQ timers), so the interrupt is recognised after the next
    /// instruction, as on the hardware.
    pub fn poll_lines(&mut self) {
        self.machine.poll_interrupt_lines();
    }

    /// The routine sampled the lines itself ([`Ctx::poll_lines`]) at the end of its last
    /// instruction: the machine does not sample them again after it.
    pub fn suppress_final_poll(&mut self) {
        self.no_poll = true;
    }

    /// Whether an arrival at the 24-bit `address` would be a sync event now: it is a sync point of
    /// the machine's table and an NMI came since the last event; or it is hooked by a routine that
    /// is not generated code (a cheat wrapper). A recompiled routine that runs through a sync point
    /// or such a stop hands over there when this is true, so the machine reports the event or runs
    /// the hook.
    #[must_use]
    pub fn sync_due(&self, address: u32) -> bool {
        self.machine.sync_due(address)
    }

    /// Master cycles until the next timing event (an NMI, an IRQ the CPU can take, HDMA); see
    /// [`Machine::master_cycles_until_timing_event`]. The I flag of the machine's registers
    /// decides whether an IRQ counts ([`Ctx::set_irq_disable`]).
    #[must_use]
    pub fn cycles_until_event(&self) -> u64 {
        self.machine.master_cycles_until_timing_event()
    }

    /// The master cycles a stretch costing `cost` CPU-busy master cycles can span (see the
    /// timing window of `Rust` mode): the stretch is clear of timing events if
    /// [`Ctx::cycles_until_event`] is above it.
    #[must_use]
    pub fn window(cost: u64) -> u64 {
        crate::timing::window(cost)
    }

    /// Whether a GPDMA was requested and has not run yet.
    #[must_use]
    pub fn gpdma_requested(&self) -> bool {
        self.machine.gpdma_requested()
    }

    /// Sets the I flag of the machine's registers, which [`Ctx::cycles_until_event`] reads.
    pub fn set_irq_disable(&mut self, on: bool) {
        self.set_flag(0x04, on);
    }

    /// Whether this runs on the snapshot a `Verify` call computes on (timing windows do not
    /// apply there: the original's call is the reference and events are not replayed).
    #[must_use]
    pub fn on_verify_snapshot(&self) -> bool {
        self.machine.is_verify_snapshot()
    }

    /// Master cycles since power-on (debugging).
    #[must_use]
    pub fn total_master_cycles(&self) -> u64 {
        self.machine.total_master_cycles()
    }

    /// Whether a frame was rendered while this routine ran (its time passed through a frame
    /// boundary). A routine that runs long returns to the machine's loop soon after, which
    /// checks its stops there.
    #[must_use]
    pub fn frame_rendered(&self) -> bool {
        self.frame_rendered
    }

    /// Whether a tick made for this routine failed (the machine must not run on).
    #[must_use]
    pub fn failed(&self) -> bool {
        self.error.is_some()
    }

    /// All CPU registers at once.
    #[must_use]
    pub fn registers(&self) -> Registers {
        self.machine.cpu_registers()
    }

    /// Replaces all CPU registers at once (the state a recompiled routine leaves).
    pub fn set_registers(&mut self, registers: Registers) {
        self.machine.set_cpu_registers(registers);
    }

    // Registers

    /// The full 16-bit accumulator (B:A), whatever the accumulator width.
    #[must_use]
    pub fn a(&self) -> u16 {
        self.machine.cpu_registers().a
    }

    /// Sets the full 16-bit accumulator (B:A).
    pub fn set_a(&mut self, value: u16) {
        self.update_registers(|r| r.a = value);
    }

    /// The low byte of the accumulator.
    #[must_use]
    pub fn al(&self) -> u8 {
        self.a() as u8
    }

    /// Sets the low byte of the accumulator; B is kept.
    pub fn set_al(&mut self, value: u8) {
        self.update_registers(|r| r.a = (r.a & 0xFF00) | u16::from(value));
    }

    #[must_use]
    pub fn x(&self) -> u16 {
        self.machine.cpu_registers().x
    }

    /// Sets X; with 8-bit index registers the high byte is 0, as on the CPU.
    pub fn set_x(&mut self, value: u16) {
        let value = self.index_value(value);
        self.update_registers(|r| r.x = value);
    }

    #[must_use]
    pub fn y(&self) -> u16 {
        self.machine.cpu_registers().y
    }

    /// Sets Y; with 8-bit index registers the high byte is 0, as on the CPU.
    pub fn set_y(&mut self, value: u16) {
        let value = self.index_value(value);
        self.update_registers(|r| r.y = value);
    }

    /// The stack pointer.
    #[must_use]
    pub fn s(&self) -> u16 {
        self.machine.cpu_registers().s
    }

    /// The direct page register.
    #[must_use]
    pub fn d(&self) -> u16 {
        self.machine.cpu_registers().d
    }

    /// Sets the direct page register, for a routine whose original leaves D changed (`PHD`,
    /// `LDA #`, `TCD`, ... with no `PLD` before the return). The routine's own direct-page
    /// accesses ([`Ctx::read_dp8`] and the others) follow it from then on, as the original's do.
    pub fn set_d(&mut self, value: u16) {
        self.update_registers(|r| r.d = value);
    }

    /// The data bank register.
    #[must_use]
    pub fn dbr(&self) -> u8 {
        self.machine.cpu_registers().dbr
    }

    /// Sets the data bank register, for a routine whose original leaves DBR changed (a `PLB`
    /// that pulls another bank and is not undone). The routine's own `DBR:` accesses
    /// ([`Ctx::read_db8`] and the others) follow it from then on, as the original's do.
    pub fn set_dbr(&mut self, value: u8) {
        self.update_registers(|r| r.dbr = value);
    }

    /// The program bank register.
    #[must_use]
    pub fn pbr(&self) -> u8 {
        self.machine.cpu_registers().pbr
    }

    fn index_value(&self, value: u16) -> u16 {
        if self.x8() { value & 0x00FF } else { value }
    }

    fn update_registers(&mut self, update: impl FnOnce(&mut crate::Registers)) {
        let mut registers = self.machine.cpu_registers();
        update(&mut registers);
        self.machine.set_cpu_registers(registers);
    }

    // Flags

    fn flag(&self, bit: u8) -> bool {
        u8::from(self.machine.cpu_registers().p) & bit != 0
    }

    fn set_flags(&mut self, mask: u8, bits: u8) {
        self.update_registers(|r| r.p = StatusRegister::from((u8::from(r.p) & !mask) | bits));
    }

    fn set_flag(&mut self, bit: u8, on: bool) {
        self.set_flags(bit, if on { bit } else { 0 });
    }

    /// Carry.
    #[must_use]
    pub fn c(&self) -> bool {
        self.flag(P_C)
    }

    pub fn set_c(&mut self, on: bool) {
        self.set_flag(P_C, on);
    }

    /// Zero.
    #[must_use]
    pub fn z(&self) -> bool {
        self.flag(P_Z)
    }

    pub fn set_z(&mut self, on: bool) {
        self.set_flag(P_Z, on);
    }

    /// Negative.
    #[must_use]
    pub fn n(&self) -> bool {
        self.flag(P_N)
    }

    pub fn set_n(&mut self, on: bool) {
        self.set_flag(P_N, on);
    }

    /// Overflow.
    #[must_use]
    pub fn v(&self) -> bool {
        self.flag(P_V)
    }

    pub fn set_v(&mut self, on: bool) {
        self.set_flag(P_V, on);
    }

    /// Whether the accumulator is 8-bit (M flag).
    #[must_use]
    pub fn m8(&self) -> bool {
        self.flag(P_M)
    }

    /// Whether the index registers are 8-bit (X flag).
    #[must_use]
    pub fn x8(&self) -> bool {
        self.flag(P_X)
    }

    /// Sets the accumulator width as `SEP #$20` (8-bit) or `REP #$20` (16-bit) does: the flag
    /// only (the accumulator keeps its 16 bits, its high byte hidden while 8-bit).
    pub fn set_m8(&mut self, on: bool) {
        self.set_flag(P_M, on);
    }

    /// Sets the index width as `SEP #$10` (8-bit) or `REP #$10` (16-bit) does: going 8-bit clears
    /// the high bytes of X and Y, as on the hardware.
    pub fn set_x8(&mut self, on: bool) {
        if on {
            self.update_registers(|r| {
                r.x &= 0x00FF;
                r.y &= 0x00FF;
            });
        }
        self.set_flag(P_X, on);
    }

    /// Sets N and Z from an 8-bit result; other flags are kept.
    pub fn set_nz8(&mut self, value: u8) {
        let n = if value & 0x80 != 0 { P_N } else { 0 };
        let z = if value == 0 { P_Z } else { 0 };
        self.set_flags(P_N | P_Z, n | z);
    }

    /// Sets N and Z from a 16-bit result; other flags are kept.
    pub fn set_nz16(&mut self, value: u16) {
        let n = if value & 0x8000 != 0 { P_N } else { 0 };
        let z = if value == 0 { P_Z } else { 0 };
        self.set_flags(P_N | P_Z, n | z);
    }

    // Memory

    /// Reads a 24-bit address.
    pub fn read8(&mut self, address: u32) -> u8 {
        self.machine.note_read(address & 0xFF_FFFF);
        self.machine.bus_read(address & 0xFF_FFFF)
    }

    /// Writes a 24-bit address. A nonzero value written to MDMAEN (`$420B` in banks `$00–$3F`
    /// and `$80–$BF`) returns once the GPDMA it started is over.
    pub fn write8(&mut self, address: u32, value: u8) {
        let address = address & 0xFF_FFFF;
        self.machine.bus_write(address, value);
        if value != 0 && is_mdmaen(address) {
            self.drain_gpdma();
        }
    }

    /// Writes a 24-bit address without letting a GPDMA it starts run: for a routine that hands
    /// the machine back to the original code right after the write, which runs the transfer
    /// from the cycle the hardware starts it at.
    pub fn write8_raw(&mut self, address: u32, value: u8) {
        self.machine.bus_write(address & 0xFF_FFFF, value);
    }

    /// Reads a little-endian word; the high byte is at the next 24-bit address (it may be in the
    /// next bank, and `$FF:FFFF` wraps to `$00:0000`), as with absolute-long addressing.
    pub fn read16(&mut self, address: u32) -> u16 {
        u16::from_le_bytes([self.read8(address), self.read8(address.wrapping_add(1))])
    }

    /// Writes a little-endian word, low byte first as the CPU's 16-bit stores do; addressing as
    /// [`Ctx::read16`]. The order matters for MMIO register pairs that act on one of the two
    /// writes (e.g. `$2118/$2119`, whose address increment follows the byte VMAIN selects): to
    /// mirror an original that writes them in another order, use two [`Ctx::write8`] calls.
    pub fn write16(&mut self, address: u32, value: u16) {
        let [low, high] = value.to_le_bytes();
        self.write8(address, low);
        self.write8(address.wrapping_add(1), high);
    }

    // Read-only memory, for bound functions (`HookEntry::bound`)

    /// The byte at a 24-bit address, read without side effects and through a shared reference
    /// (so a [`BoundFn`](crate::BoundFn) can use it): [`Machine::peek`], which sees the ROM and
    /// WRAM only. Anything else (registers, SRAM, open bus) reads 0.
    #[must_use]
    pub fn peek8(&self, address: u32) -> u8 {
        self.machine.peek(address & 0xFF_FFFF).unwrap_or(0)
    }

    /// A little-endian word as [`Ctx::peek8`] reads it; addressing as [`Ctx::read16`].
    #[must_use]
    pub fn peek16(&self, address: u32) -> u16 {
        u16::from_le_bytes([self.peek8(address), self.peek8(address.wrapping_add(1))])
    }

    /// [`Ctx::peek8`] at `DBR:address`.
    #[must_use]
    pub fn peek_db8(&self, address: u16) -> u8 {
        self.peek8(self.db_address(address))
    }

    /// [`Ctx::peek16`] at `DBR:address`.
    #[must_use]
    pub fn peek_db16(&self, address: u16) -> u16 {
        self.peek16(self.db_address(address))
    }

    /// [`Ctx::peek8`] at direct page offset `offset` (as [`Ctx::read_dp8`]).
    #[must_use]
    pub fn peek_dp8(&self, offset: u16) -> u8 {
        self.peek8(self.dp_address(offset))
    }

    /// A word of the direct page as [`Ctx::peek8`] reads it (both bytes in bank 0).
    #[must_use]
    pub fn peek_dp16(&self, offset: u16) -> u16 {
        u16::from_le_bytes([self.peek_dp8(offset), self.peek_dp8(offset.wrapping_add(1))])
    }

    /// Reads `DBR:address` (absolute addressing).
    pub fn read_db8(&mut self, address: u16) -> u8 {
        let address = self.db_address(address);
        self.read8(address)
    }

    /// Reads a word at `DBR:address`; the high byte may be in the next bank.
    pub fn read_db16(&mut self, address: u16) -> u16 {
        let address = self.db_address(address);
        self.read16(address)
    }

    pub fn write_db8(&mut self, address: u16, value: u8) {
        let address = self.db_address(address);
        self.write8(address, value);
    }

    /// Writes a word at `DBR:address`, low byte first (see [`Ctx::write16`]).
    pub fn write_db16(&mut self, address: u16, value: u16) {
        let address = self.db_address(address);
        self.write16(address, value);
    }

    /// Reads direct page offset `offset` (bank 0, `D + offset` wrapping in 16 bits; an indexed
    /// access passes `offset + X`).
    pub fn read_dp8(&mut self, offset: u16) -> u8 {
        let address = self.dp_address(offset);
        self.read8(address)
    }

    /// Reads a word from the direct page; both bytes are in bank 0.
    pub fn read_dp16(&mut self, offset: u16) -> u16 {
        u16::from_le_bytes([self.read_dp8(offset), self.read_dp8(offset.wrapping_add(1))])
    }

    pub fn write_dp8(&mut self, offset: u16, value: u8) {
        let address = self.dp_address(offset);
        self.write8(address, value);
    }

    /// Writes a word to the direct page, low byte first; both bytes are in bank 0.
    pub fn write_dp16(&mut self, offset: u16, value: u16) {
        let [low, high] = value.to_le_bytes();
        self.write_dp8(offset, low);
        self.write_dp8(offset.wrapping_add(1), high);
    }

    fn db_address(&self, address: u16) -> u32 {
        (u32::from(self.dbr()) << 16) | u32::from(address)
    }

    fn dp_address(&self, offset: u16) -> u32 {
        u32::from(self.d().wrapping_add(offset))
    }

    /// Holds the CPU until the requested GPDMA is over (see [`Machine::drain_gpdma`]).
    fn drain_gpdma(&mut self) {
        if self.error.is_some() {
            return;
        }
        match self.machine.drain_gpdma(&mut *self.input, &mut *self.audio) {
            Ok(rendered) => self.frame_rendered |= rendered,
            Err(error) => self.error = Some(error),
        }
    }
}

/// A copy of `error` for the routine, the original being kept for the dispatch (core errors are
/// [`OracleError::Run`]).
fn duplicate(error: &OracleError) -> OracleError {
    match error {
        OracleError::Run(message) => OracleError::Run(message.clone()),
        other => OracleError::Run(other.to_string()),
    }
}

/// Whether a 24-bit address is MDMAEN (`$420B`) in one of the banks that map it.
fn is_mdmaen(address: u32) -> bool {
    let bank = (address >> 16) as u8;
    bank & 0x7F <= 0x3F && address & 0xFFFF == 0x420B
}

#[cfg(test)]
mod tests {
    use super::is_mdmaen;

    #[test]
    fn mdmaen_is_mapped_in_the_system_banks() {
        for bank in [0x00, 0x3F, 0x80, 0xBF] {
            assert!(is_mdmaen((bank << 16) | 0x420B), "bank {bank:02X}");
        }
        for address in [0x40_420B, 0x7E_420B, 0xC0_420B, 0x00_420A, 0x00_430B] {
            assert!(!is_mdmaen(address), "{address:06X}");
        }
    }
}
