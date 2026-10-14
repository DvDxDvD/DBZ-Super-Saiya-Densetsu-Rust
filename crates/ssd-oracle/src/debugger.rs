//! Debugger steps (spec §5.10): step into, over and out, on [`Machine::run`] with its stops and
//! the breakpoints of [`Machine::set_breakpoint`]. Hooked routines stay atomic: a routine that
//! runs in `Rust` or is checked in `Verify` is one step.

use crate::{AudioSink, Joypad, Machine, OracleError, Stopped, Stops};

/// Frames a step may run before it gives up ([`Stepped::NoReturn`]) by default. The steps' own
/// constant, not the budget of the `Verify` runs of the original.
const STEP_BUDGET_FRAMES: u64 = 2;

/// How [`Machine::step_over`] and [`Machine::step_out`] run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepOptions {
    /// Whether an NMI or IRQ taken meanwhile ends the step at the handler's first instruction
    /// ([`Stepped::Interrupt`]). Off by default: the handler runs through, as the instructions
    /// of the callee do, and the step ends at the return it waits for. (A breakpoint in the
    /// handler stops it either way.)
    pub stop_at_interrupts: bool,
    /// Frames the step may run before it ends as [`Stepped::NoReturn`]: 2 by default.
    pub budget_frames: u64,
}

impl Default for StepOptions {
    fn default() -> Self {
        Self { stop_at_interrupts: false, budget_frames: STEP_BUDGET_FRAMES }
    }
}

/// How a debugger step ended. The CPU is then about to fetch an opcode (but after
/// [`Stepped::NoReturn`], and after a [`Stepped::Routine`] whose return left an interrupt
/// pending: the next step enters the handler).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stepped {
    /// One instruction ran ([`Machine::step_into`], or [`Machine::step_over`] of anything but a
    /// call): the CPU is at the next one.
    Instruction,
    /// The CPU was at the entry of the hooked routine `name`, which ran as one step: a `Rust`
    /// run or a checked `Verify` call (its original ran to its return). The CPU is where the
    /// routine left it, usually its return address.
    Routine { name: String },
    /// A call returned ([`Machine::step_over`] of a call, [`Machine::step_out`]): the CPU is at
    /// the instruction the return goes to.
    Returned,
    /// The CPU entered an NMI or IRQ handler: it is at the handler's first instruction.
    /// [`Machine::step_into`] always stops there; [`Machine::step_over`] and
    /// [`Machine::step_out`] only with [`StepOptions::stop_at_interrupts`]. An interrupt taken
    /// inside a dispatched routine does not count (the routine is atomic).
    Interrupt,
    /// A breakpoint stopped the step before the instruction at it ([`Stopped::Breakpoint`]).
    Breakpoint { key: u32 },
    /// The step did not end within its frames (2, or [`StepOptions::budget_frames`]): the call ([`Machine::step_over`]) or the routine
    /// ([`Machine::step_out`]) does not return, or the instruction does not end (`STP`, or
    /// `WAI` with no interrupt coming). The CPU is where it got to.
    NoReturn,
}

/// Opcodes of the calls [`Machine::step_over`] steps over: `JSR abs`, `JSL long`,
/// `JSR (abs,X)`.
const CALLS: [u8; 3] = [0x20, 0x22, 0xFC];

impl Machine {
    /// Runs one instruction ([`Machine::run`] with [`Stops::instructions`] of 1), dispatching
    /// hook hits as [`Machine::run_frame`] does, and stops at the next instruction
    /// ([`Stepped::Instruction`]).
    ///
    /// At the entry of a hooked routine (a pending arrival, as after a step onto it) the step
    /// dispatches it ([`Stops::after_dispatch`]): if the routine ran (a `Rust` run, a checked
    /// `Verify` call), the whole routine is the step ([`Stepped::Routine`]); if the dispatch only
    /// resumed the original (a sync point, a timing fallback, a contract mismatch, a missing
    /// cost, a sampled-out `Verify` call), the step goes on as for any instruction: the
    /// original's first instruction runs and the step stops at its second. In
    /// [`HookMode::Original`](crate::HookMode::Original) a routine is code like any other.
    ///
    /// If the CPU enters an interrupt handler after the instruction, the step goes on to the
    /// handler's first instruction ([`Stepped::Interrupt`]). A breakpoint at the instruction the
    /// step starts from does not stop it; a breakpoint at the instruction it ends at is left for
    /// the next run or step to pass (see [`Machine::set_breakpoint`]): stepping onto a breakpoint
    /// and going on does not stop there again. 2-frame budget ([`Stepped::NoReturn`]).
    ///
    /// # Errors
    ///
    /// As for [`Machine::run_frame`].
    pub fn step_into(
        &mut self,
        input: &mut dyn FnMut(u64) -> Joypad,
        audio: &mut dyn AudioSink,
    ) -> Result<Stepped, OracleError> {
        self.skip_breakpoint_here();
        let stepped = self.step(true, STEP_BUDGET_FRAMES, input, audio)?;
        self.skip_breakpoint_here();
        Ok(stepped)
    }

    /// [`Machine::step_into`], but a call (`JSR`, `JSL`, `JSR (abs,X)`, its opcode read without
    /// side effects from the ROM image or WRAM; code elsewhere is no call) runs to its return:
    /// once the step entered the callee, the run goes on until a return (`RTS`, `RTL`, `RTI`, or
    /// a hooked routine that ran) leaves S where it was before the call
    /// ([`Stops::returned_above`] with that S less 1), at the instruction the return goes to
    /// ([`Stepped::Returned`]). A callee that does not return within the budget ends the step
    /// ([`Stepped::NoReturn`], the CPU where it got to), and so does a breakpoint in the callee
    /// ([`Stepped::Breakpoint`]), in an interrupt handler it is interrupted by too. A call at
    /// the entry of a hooked routine that runs is the routine ([`Stepped::Routine`]), as in
    /// `step_into`; a hooked callee is dispatched as usual, atomic.
    ///
    /// Interrupts: an NMI or IRQ taken meanwhile runs through unless
    /// [`StepOptions::stop_at_interrupts`] ([`Stepped::Interrupt`], at the handler's first
    /// instruction); the handler's `RTI` leaves S below the call's, so it is no return of the
    /// step. Neither is one taken right after the call, before the callee's first instruction.
    /// An instruction that is no call is one step as in `step_into`, ending at the handler's
    /// first instruction ([`Stepped::Interrupt`]) if an interrupt entry follows it.
    ///
    /// # Errors
    ///
    /// As for [`Machine::run_frame`].
    pub fn step_over(
        &mut self,
        options: &StepOptions,
        input: &mut dyn FnMut(u64) -> Joypad,
        audio: &mut dyn AudioSink,
    ) -> Result<Stepped, OracleError> {
        self.skip_breakpoint_here();
        let registers = self.cpu_registers();
        let pc = (u32::from(registers.pbr) << 16) | u32::from(registers.pc);
        let call =
            self.at_fetch_boundary() && self.peek_code(pc).is_some_and(|op| CALLS.contains(&op));
        // The call's own step must not stop at an interrupt entry right after it, which may come
        // before the callee's first instruction.
        let interrupt = options.stop_at_interrupts || !call;
        let mut stepped = self.step(interrupt, options.budget_frames, input, audio)?;
        if call && stepped == Stepped::Instruction {
            // In the callee, or in a handler entered right after the call: on until the return.
            let stops = Stops {
                breakpoint: true,
                interrupt: options.stop_at_interrupts,
                returned_above: Some(registers.s.saturating_sub(1)),
                ..Stops::default()
            };
            stepped = stepped_of(self.run(&stops, input, audio, options.budget_frames)?);
        }
        self.skip_breakpoint_here();
        Ok(stepped)
    }

    /// Runs until the routine the CPU is in returns: the first instruction boundary right after a
    /// return (`RTS`, `RTL`, `RTI`, each opcode read without side effects from the ROM image or
    /// WRAM; or a dispatched routine that ran, which ends with its return) with S above S at the
    /// start ([`Stops::returned_above`]): [`Stepped::Returned`], at the instruction the return
    /// goes to. Pulls do not count, so the step stops at the return of `PHX … PLX; RTS`, not
    /// after the `PLX`; the returns of the routines it calls leave S at most where it was.
    /// Hooked routines are dispatched as usual, atomic (at the entry of one that runs, the step
    /// ends after its return).
    ///
    /// The rule is on S alone, so it has a limit: a routine that, after the instruction the
    /// step starts at, pulls what it pushed before (S back above the S at the start) and then
    /// calls something, ends the step at that call's return, which leaves S above the start's
    /// too. Stepping out from the first instruction of a routine, or from before any pull of its
    /// own, is exact.
    ///
    /// An interrupt taken meanwhile runs through unless [`StepOptions::stop_at_interrupts`]
    /// ([`Stepped::Interrupt`], at the handler's first instruction). A breakpoint stops the step
    /// ([`Stepped::Breakpoint`]) either way. From the first instruction of a handler, the step
    /// ends after its `RTI`. The budget is [`StepOptions::budget_frames`], then
    /// [`Stepped::NoReturn`].
    ///
    /// # Errors
    ///
    /// As for [`Machine::run_frame`].
    pub fn step_out(
        &mut self,
        options: &StepOptions,
        input: &mut dyn FnMut(u64) -> Joypad,
        audio: &mut dyn AudioSink,
    ) -> Result<Stepped, OracleError> {
        self.skip_breakpoint_here();
        let stops = Stops {
            breakpoint: true,
            interrupt: options.stop_at_interrupts,
            returned_above: Some(self.cpu_registers().s),
            ..Stops::default()
        };
        let stepped = stepped_of(self.run(&stops, input, audio, options.budget_frames)?);
        self.skip_breakpoint_here();
        Ok(stepped)
    }

    /// One step into, without the breakpoint skips of [`Machine::step_into`].
    fn step(
        &mut self,
        interrupt: bool,
        budget_frames: u64,
        input: &mut dyn FnMut(u64) -> Joypad,
        audio: &mut dyn AudioSink,
    ) -> Result<Stepped, OracleError> {
        let stops = Stops {
            breakpoint: true,
            after_dispatch: true,
            interrupt,
            instructions: Some(1),
            ..Stops::default()
        };
        loop {
            match self.run(&stops, input, audio, budget_frames)? {
                // The original resumes: its first instruction is the step (the hit is taken, so
                // the next run ticks on).
                Stopped::Dispatched { ran: false, .. } => {}
                stopped => return Ok(stepped_of(stopped)),
            }
        }
    }
}

/// The step a run with the stops of a step ended.
fn stepped_of(stopped: Stopped) -> Stepped {
    match stopped {
        Stopped::Instructions => Stepped::Instruction,
        Stopped::Dispatched { name, .. } => Stepped::Routine { name },
        Stopped::StackAbove | Stopped::Returned => Stepped::Returned,
        Stopped::Interrupt => Stepped::Interrupt,
        Stopped::Breakpoint { key } => Stepped::Breakpoint { key },
        Stopped::Budget => Stepped::NoReturn,
        Stopped::FrameEnd | Stopped::Call { .. } | Stopped::Sync(_) => {
            unreachable!("a step does not stop there: {stopped:?}")
        }
    }
}
