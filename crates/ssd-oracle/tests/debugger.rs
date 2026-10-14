//! Breakpoints and debugger steps: `Machine::set_breakpoint`, `step_into`, `step_over`,
//! `step_out`, and the stops of `Machine::run` they use, on hand-traced synthetic code.

mod common;

use common::{
    CALL_INNER, CALL_OUTER, CALL_OUTER_COUNTER, DBG_INNER_LONG_COUNTER, DBG_INNERMOST,
    DBG_INNERMOST_COUNTER, DBG_LOOP, DBG_NMI, DBG_NMI_COUNTER, DBG_RUST_COUNTER, DBG_RUST_LEAF,
    DBG_VERIFY_COUNTER, DBG_VERIFY_LEAF, DBG_WAIT, MAIN_STACK,
};
use ssd_oracle::{
    Ctx, HookAction, HookEntry, HookMode, HookStats, HookTable, Joypad, Machine, MemorySaveStore,
    NullAudio, Recorder, ReturnKind, RoutineFn, StepOptions, Stepped, Stopped, Stops,
};

// CPU-busy master cycles of the fixtures' instructions.
const JSR: u64 = 46;
const INC_DP: u64 = 38;
const RTS: u64 = 42;
/// `INC $xx; RTS`, the leaves of [`common::debugger_rom`] and `inner` of [`common::call_rom`].
const LEAF_COST: u64 = INC_DP + RTS;
/// `outer` of [`common::call_rom`]: `JSR inner; INC $11; RTS`.
const CALL_OUTER_COST: u64 = JSR + LEAF_COST + INC_DP + RTS;

/// `PLX` in `outer` of [`common::debugger_rom`].
const OUTER_PLX: u32 = 0x00_8045;
/// The `JSR` to the Rust leaf in the main loop of [`common::debugger_rom`].
const CALL_RUST_LEAF: u32 = 0x00_8016;

fn no_input(_: u64) -> Joypad {
    Joypad::default()
}

// The routines in Rust

fn inc_dp(ctx: &mut Ctx<'_>, offset: usize) {
    let value = ctx.read_dp8(offset as u16).wrapping_add(1);
    ctx.write_dp8(offset as u16, value);
    ctx.set_nz8(value);
}

fn rust_leaf(ctx: &mut Ctx<'_>) {
    inc_dp(ctx, DBG_RUST_COUNTER);
    ctx.set_cost(LEAF_COST);
}

fn verify_leaf(ctx: &mut Ctx<'_>) {
    inc_dp(ctx, DBG_VERIFY_COUNTER);
    ctx.set_cost(LEAF_COST);
}

/// `wait` of [`common::debugger_rom`] waits for the NMI: every `Verify` call is interrupted, so
/// this never runs.
fn wait(_: &mut Ctx<'_>) {}

/// `outer` of [`common::call_rom`], running `inner` as original code.
fn call_outer(ctx: &mut Ctx<'_>) {
    ctx.charge(JSR);
    let Ok(()) = ctx.call_original(CALL_INNER, ReturnKind::Rts) else { return };
    inc_dp(ctx, CALL_OUTER_COUNTER);
    ctx.set_cost(JSR + INC_DP + RTS);
}

fn routine(
    name: &str,
    key: u32,
    mode: HookMode,
    func: RoutineFn,
    cycles: Option<u64>,
) -> HookEntry {
    HookEntry {
        name: name.to_owned(),
        key,
        mode,
        action: HookAction::Routine { func, ret: ReturnKind::Rts },
        entry_m8: Some(true),
        entry_x8: Some(false),
        flags_mask: 0xFF,
        verify_sample: 1,
        bound: None,
        budget_frames: None,
        cycles,
    }
}

/// The leaves of [`common::debugger_rom`]: `rust_leaf` in `rust` mode, `verify_leaf` in
/// `verify` mode.
fn leaves(rust: HookMode, verify: HookMode) -> Vec<HookEntry> {
    vec![
        routine("rust_leaf", DBG_RUST_LEAF, rust, rust_leaf, Some(LEAF_COST)),
        routine("verify_leaf", DBG_VERIFY_LEAF, verify, verify_leaf, Some(LEAF_COST)),
    ]
}

fn table(entries: Vec<HookEntry>) -> HookTable {
    entries.into_iter().collect()
}

// The machine

fn pure(rom: Vec<u8>) -> Machine {
    Machine::new(rom, Box::new(MemorySaveStore::default())).unwrap()
}

/// [`common::debugger_rom`] at power-on with its leaves hooked: `Rust` and `Verify`.
fn machine() -> Machine {
    let mut m = pure(common::debugger_rom());
    m.set_hook_table(Some(table(leaves(HookMode::Rust, HookMode::Verify))));
    m
}

/// The boot code run: [`machine`] at the main loop, S = [`MAIN_STACK`].
fn booted() -> Machine {
    let mut m = machine();
    for _ in 0..10 {
        assert_eq!(step_into(&mut m), Stepped::Instruction);
    }
    assert_eq!(at(&m), (DBG_LOOP, MAIN_STACK));
    m
}

fn full_pc(m: &Machine) -> u32 {
    let r = m.cpu_registers();
    (u32::from(r.pbr) << 16) | u32::from(r.pc)
}

/// Where the CPU is: `PBR:PC` and S.
fn at(m: &Machine) -> (u32, u16) {
    (full_pc(m), m.cpu_registers().s)
}

fn step_into(m: &mut Machine) -> Stepped {
    m.step_into(&mut no_input, &mut NullAudio).unwrap()
}

/// Steps that stop at interrupts, as the tests below were written; [`StepOptions::default`]
/// runs through them (see `the_default_steps_run_through_interrupts`).
const STOPPING: StepOptions = StepOptions { stop_at_interrupts: true, budget_frames: 2 };

fn step_over(m: &mut Machine) -> Stepped {
    m.step_over(&STOPPING, &mut no_input, &mut NullAudio).unwrap()
}

fn step_out(m: &mut Machine) -> Stepped {
    m.step_out(&STOPPING, &mut no_input, &mut NullAudio).unwrap()
}

fn run(m: &mut Machine, stops: Stops, budget_frames: u64) -> Stopped {
    m.run(&stops, &mut no_input, &mut NullAudio, budget_frames).unwrap()
}

fn stats(m: &Machine, name: &str) -> HookStats {
    let all = m.hook_stats();
    all.iter()
        .find(|(n, _)| n == name)
        .map(|&(_, s)| s)
        .unwrap_or_else(|| panic!("{name}: {all:?}"))
}

fn routine_step(name: &str) -> Stepped {
    Stepped::Routine { name: name.to_owned() }
}

fn breakpoint(key: u32) -> Stopped {
    Stopped::Breakpoint { key }
}

const BREAKPOINTS: Stops = Stops {
    frame_end: false,
    call: None,
    sync: false,
    breakpoint: true,
    after_dispatch: false,
    instructions: None,
    stack_above: None,
    returned_above: None,
    interrupt: false,
};

/// The counters of [`common::debugger_rom`]: Rust leaf, Verify leaf, NMI handler, `inner_long`,
/// `innermost`.
fn counters(m: &Machine) -> [u8; 5] {
    [
        DBG_RUST_COUNTER,
        DBG_VERIFY_COUNTER,
        DBG_NMI_COUNTER,
        DBG_INNER_LONG_COUNTER,
        DBG_INNERMOST_COUNTER,
    ]
    .map(|index| m.wram()[index])
}

fn incremented(before: [u8; 5], which: [bool; 5]) -> [u8; 5] {
    let mut after = before;
    for (counter, inc) in after.iter_mut().zip(which) {
        *counter = counter.wrapping_add(u8::from(inc));
    }
    after
}

// Steps

#[test]
fn step_into_follows_the_code_and_takes_a_hooked_routine_as_one_step() {
    let mut m = machine();
    // The boot code.
    for pc in [0x8001, 0x8002, 0x8003, 0x8005, 0x8007, 0x800A, 0x800B, 0x800E, 0x8010, 0x8013] {
        assert_eq!(step_into(&mut m), Stepped::Instruction);
        assert_eq!(full_pc(&m), pc);
    }
    assert_eq!(m.cpu_registers().s, MAIN_STACK);
    let start = counters(&m);

    let instruction = |pc: u32, s: u16| (Stepped::Instruction, pc, s, 1);
    // What the step did, where it left the CPU, the opcodes it fetched.
    let trace = [
        instruction(0x8040, 0x1FEE),                      // JSR outer
        instruction(0x8041, 0x1FEC),                      // PHX
        instruction(0x8050, 0x1FE9),                      // JSL inner_long
        instruction(0x8058, 0x1FE7),                      // JSR innermost
        instruction(0x805A, 0x1FE7),                      // INC $14
        instruction(0x8053, 0x1FE9),                      // RTS
        instruction(0x8055, 0x1FE9),                      // INC $13
        instruction(0x8045, 0x1FEC),                      // RTL
        instruction(0x8046, 0x1FEE),                      // PLX
        instruction(0x8016, 0x1FF0),                      // RTS
        instruction(0x8060, 0x1FEE),                      // JSR to the Rust leaf: onto its entry
        (routine_step("rust_leaf"), 0x8019, 0x1FF0, 0),   // the leaf in Rust, its return performed
        instruction(0x8070, 0x1FEE),                      // JSR ($8090,X)
        (routine_step("verify_leaf"), 0x801C, 0x1FF0, 2), // its original: INC, RTS
        instruction(0x8080, 0x1FEE),                      // JSR wait
        (Stepped::Interrupt, 0x8028, 0x1FEA, 1),          // WAI, then the NMI's entry
        instruction(0x8029, 0x1FE9),                      // PHA
        instruction(0x802B, 0x1FE9),                      // INC $12
        instruction(0x802E, 0x1FE9),                      // LDA $4210
        instruction(0x802F, 0x1FEA),                      // PLA
        instruction(0x8081, 0x1FEE),                      // RTI
        instruction(0x801F, 0x1FF0),                      // RTS
        instruction(0x8013, 0x1FF0),                      // BRA
    ];
    for (i, (stepped, pc, s, fetched)) in trace.into_iter().enumerate() {
        let fetches = m.opcode_fetches();
        assert_eq!(step_into(&mut m), stepped, "step {i}");
        assert_eq!(at(&m), (pc, s), "step {i}");
        assert_eq!(m.opcode_fetches() - fetches, fetched, "step {i}");
        assert!(m.at_fetch_boundary(), "step {i}");
    }
    assert_eq!(counters(&m), incremented(start, [true; 5]));
    assert_eq!(m.nmi_count(), 1);
    assert_eq!((stats(&m, "rust_leaf").rust_runs, stats(&m, "verify_leaf").verified_ok), (1, 1));
}

#[test]
fn step_into_a_routine_whose_dispatch_resumes_the_original_runs_its_first_instruction() {
    // Without a cost, the Rust leaf resumes the original at every call.
    let mut entries = leaves(HookMode::Rust, HookMode::Verify);
    entries[0].cycles = None;
    let mut m = booted();
    m.set_hook_table(Some(table(entries)));
    assert_eq!(step_over(&mut m), Stepped::Returned);
    assert_eq!(step_into(&mut m), Stepped::Instruction);
    assert_eq!(at(&m), (DBG_RUST_LEAF, 0x1FEE));
    let (start, fetches) = (counters(&m), m.opcode_fetches());

    // The original's first instruction, `INC $10`, is the step.
    assert_eq!(step_into(&mut m), Stepped::Instruction);
    assert_eq!(at(&m), (DBG_RUST_LEAF + 2, 0x1FEE));
    assert_eq!(m.opcode_fetches(), fetches + 1);
    assert_eq!(counters(&m), incremented(start, [true, false, false, false, false]));
    let leaf = stats(&m, "rust_leaf");
    assert_eq!((leaf.calls, leaf.cost_missing, leaf.rust_runs), (1, 1, 0), "{leaf:?}");
}

#[test]
fn after_dispatch_stops_right_after_a_dispatch_and_tells_whether_the_routine_ran() {
    let mut m = booted();
    let stops = Stops { after_dispatch: true, ..Stops::default() };
    let dispatched =
        |name: &str, ran| Stopped::Dispatched { name: name.to_owned(), ran, rendered: false };
    assert_eq!(run(&mut m, stops, 1), dispatched("rust_leaf", true));
    assert_eq!(at(&m), (0x8019, MAIN_STACK), "at its return");
    assert_eq!(run(&mut m, stops, 1), dispatched("verify_leaf", true));
    assert_eq!(at(&m), (0x801C, MAIN_STACK));

    // A Verify call sampled out resumes the original: the CPU is still at the entry.
    let mut entries = leaves(HookMode::Rust, HookMode::Verify);
    entries[1].verify_sample = 2;
    m.set_hook_table(Some(table(entries)));
    assert_eq!(run(&mut m, stops, 2), dispatched("rust_leaf", true));
    assert_eq!(run(&mut m, stops, 2), dispatched("verify_leaf", false));
    assert_eq!(at(&m), (DBG_VERIFY_LEAF, 0x1FEE));
    let leaf = stats(&m, "verify_leaf");
    assert_eq!((leaf.verified_ok, leaf.sampled_out), (1, 1), "{leaf:?}");
}

#[test]
fn step_over_runs_a_call_to_its_return() {
    let mut m = booted();
    let start = counters(&m);
    // `outer`, its `JSL` and `JSR` inside: one step of ten instructions.
    let fetches = m.opcode_fetches();
    assert_eq!(step_over(&mut m), Stepped::Returned);
    assert_eq!(at(&m), (CALL_RUST_LEAF, MAIN_STACK));
    assert_eq!(m.opcode_fetches() - fetches, 10);
    assert_eq!(counters(&m), incremented(start, [false, false, false, true, true]));

    // The hooked leaves are dispatched in the step, as usual.
    assert_eq!(step_over(&mut m), Stepped::Returned);
    assert_eq!(at(&m), (0x8019, MAIN_STACK));
    assert_eq!(stats(&m, "rust_leaf").rust_runs, 1);
    assert_eq!(step_over(&mut m), Stepped::Returned, "JSR ($8090,X)");
    assert_eq!(at(&m), (0x801C, MAIN_STACK));
    assert_eq!(stats(&m, "verify_leaf").verified_ok, 1);

    // The NMI comes while `wait` runs: the step stops at the handler's first instruction.
    assert_eq!(step_over(&mut m), Stepped::Interrupt);
    assert_eq!(at(&m), (DBG_NMI, 0x1FEA));
    // No call: one instruction.
    assert_eq!(step_over(&mut m), Stepped::Instruction);
    assert_eq!(at(&m), (DBG_NMI + 1, 0x1FE9));

    // Out of the handler and `wait`, back to the loop.
    assert_eq!(step_out(&mut m), Stepped::Returned);
    assert_eq!(at(&m), (DBG_WAIT + 1, 0x1FEE));
    assert_eq!(step_out(&mut m), Stepped::Returned);
    assert_eq!(step_over(&mut m), Stepped::Instruction, "BRA");
    assert_eq!(at(&m), (DBG_LOOP, MAIN_STACK));

    // Onto the entry of the Rust leaf, then over it: the routine.
    assert_eq!(step_over(&mut m), Stepped::Returned);
    assert_eq!(step_into(&mut m), Stepped::Instruction);
    assert_eq!(step_over(&mut m), routine_step("rust_leaf"));
    assert_eq!(at(&m), (0x8019, MAIN_STACK));
}

#[test]
fn step_over_reads_calls_in_wram_too() {
    let mut m = booted();
    // `JSR innermost; BRA *` in low RAM, `JSL inner_long; BRA *` in bank $7E.
    let code: [(u32, &[u8]); 2] = [
        (0x00_0300, &[0x20, 0x58, 0x80, 0x80, 0xFE]),
        (0x7E_0400, &[0x22, 0x50, 0x80, 0x00, 0x80, 0xFE]),
    ];
    for (address, bytes) in code {
        for (offset, &byte) in (address..).zip(bytes) {
            m.bus_write(offset, byte);
        }
    }
    for (address, length) in [(0x00_0300, 3), (0x7E_0400, 4)] {
        let mut registers = m.cpu_registers();
        (registers.pbr, registers.pc) = ((address >> 16) as u8, address as u16);
        m.set_cpu_registers(registers);
        let start = counters(&m);
        assert_eq!(step_over(&mut m), Stepped::Returned, "${address:06X}");
        assert_eq!(at(&m), (address + length, MAIN_STACK));
        assert_eq!(counters(&m)[4], start[4].wrapping_add(1), "innermost ran");
    }
}

#[test]
fn step_over_a_call_that_never_returns_ends_after_two_frames() {
    let mut m = pure(common::no_return_rom());
    for _ in 0..5 {
        assert_eq!(step_into(&mut m), Stepped::Instruction);
    }
    assert_eq!(full_pc(&m), 0x00_8007, "JSR $8040");
    let frames = m.frame_count();
    assert_eq!(step_over(&mut m), Stepped::NoReturn);
    assert_eq!(m.frame_count(), frames + 2);
    assert!((0x8040..=0x8042).contains(&full_pc(&m)), "in the callee: {:06X}", full_pc(&m));
    // Nor does stepping out of it end.
    assert_eq!(step_out(&mut m), Stepped::NoReturn);
    assert_eq!(m.frame_count(), frames + 4);
}

#[test]
fn step_out_stops_at_the_return_not_at_a_pull() {
    let mut m = booted();
    step_into(&mut m);
    step_into(&mut m);
    assert_eq!(at(&m), (0x8041, 0x1FEC), "after the PHX of outer");
    let mut twin = m.snapshot();
    // `PLX` takes S above its start; only the `RTS` is a return.
    assert_eq!(step_out(&mut m), Stepped::Returned);
    assert_eq!(at(&m), (CALL_RUST_LEAF, MAIN_STACK));
    let above = Stops { stack_above: Some(0x1FEC), ..Stops::default() };
    assert_eq!(run(&mut twin, above, 1), Stopped::StackAbove);
    assert_eq!(at(&twin), (0x8046, 0x1FEE), "S above, right after the PLX");

    // Two calls deep, out one at a time: RTS, RTL, then PLX and RTS.
    let mut m = booted();
    for _ in 0..4 {
        step_into(&mut m);
    }
    assert_eq!(at(&m), (DBG_INNERMOST, 0x1FE7));
    for (pc, s) in [(0x8053, 0x1FE9), (0x8045, 0x1FEC), (CALL_RUST_LEAF, MAIN_STACK)] {
        assert_eq!(step_out(&mut m), Stepped::Returned);
        assert_eq!(at(&m), (pc, s));
    }

    // At the entry of a hooked routine: it runs, and its return is the step's.
    assert_eq!(step_into(&mut m), Stepped::Instruction);
    assert_eq!(step_out(&mut m), Stepped::Returned);
    assert_eq!(at(&m), (0x8019, MAIN_STACK));
    assert_eq!(stats(&m, "rust_leaf").rust_runs, 1);

    // The NMI comes while `wait` runs: the step stops at the handler's first instruction; from
    // there, its RTI is the return.
    assert_eq!(step_over(&mut m), Stepped::Returned);
    assert_eq!(step_into(&mut m), Stepped::Instruction);
    assert_eq!(at(&m), (DBG_WAIT, 0x1FEE));
    assert_eq!(step_out(&mut m), Stepped::Interrupt);
    assert_eq!(at(&m), (DBG_NMI, 0x1FEA));
    assert_eq!(step_out(&mut m), Stepped::Returned);
    assert_eq!(at(&m), (DBG_WAIT + 1, 0x1FEE));
}

#[test]
fn an_interrupt_stop_comes_before_the_instruction_stop_at_the_same_boundary() {
    let mut m = booted();
    for _ in 0..3 {
        assert_eq!(step_over(&mut m), Stepped::Returned);
    }
    assert_eq!(step_into(&mut m), Stepped::Instruction);
    assert_eq!(full_pc(&m), DBG_WAIT);
    // WAI is one instruction; the NMI's entry follows it.
    let both = Stops { instructions: Some(1), interrupt: true, ..Stops::default() };
    let mut twin = m.snapshot();
    assert_eq!(run(&mut m, both, 2), Stopped::Interrupt);
    assert_eq!(run(&mut twin, Stops { instructions: Some(1), ..Stops::default() }, 2), {
        Stopped::Instructions
    });
    assert_eq!(at(&m), (DBG_NMI, 0x1FEA));
    assert_eq!(at(&twin), at(&m));
}

#[test]
fn a_return_followed_by_an_interrupt_entry_does_not_stop_at_the_handler() {
    // The leaf of `nmi_leaf_rom` (`INC $10; RTS`) in a tight loop: find an NMI taken right after
    // its RTS (the handler returns to the instruction after the call).
    let mut m = pure(common::nmi_leaf_rom());
    let interrupt = Stops { interrupt: true, ..Stops::default() };
    for _ in 0..60 {
        let mut back = m.snapshot();
        assert_eq!(run(&mut m, interrupt, 2), Stopped::Interrupt);
        let s = usize::from(m.cpu_registers().s);
        let pushed_pc = u16::from_le_bytes([m.wram()[s + 2], m.wram()[s + 3]]);
        if pushed_pc != common::NMI_LEAF_RETURN {
            continue;
        }
        // Back to that RTS: the last instruction before the handler's first.
        let rts = m.opcode_fetches() - 1 - back.opcode_fetches();
        let to_rts = Stops { instructions: Some(rts), ..Stops::default() };
        assert_eq!(run(&mut back, to_rts, 2), Stopped::Instructions);
        assert_eq!(at(&back), (common::LEAF + 2, MAIN_STACK - 2));
        // S at the handler's first instruction is above too, but no return led there: the
        // handler's RTI does.
        let returned = Stops { returned_above: Some(MAIN_STACK - 0x10), ..Stops::default() };
        assert_eq!(run(&mut back, returned, 2), Stopped::Returned);
        assert_eq!(at(&back), (u32::from(common::NMI_LEAF_RETURN), MAIN_STACK));
        return;
    }
    panic!("no NMI right after the leaf's RTS");
}

// Interrupts in step over and out

/// The default options: interrupts run through.
const DEFAULT: StepOptions = StepOptions { stop_at_interrupts: false, budget_frames: 2 };

#[test]
fn the_default_options_are_no_interrupt_stops_and_two_frames() {
    assert_eq!(StepOptions::default(), DEFAULT);
}

#[test]
fn a_callee_spanning_the_nmi_completes_by_default_and_stops_at_the_handler_with_the_option() {
    let mut stopping = booted();
    let mut through = booted();
    for m in [&mut stopping, &mut through] {
        for _ in 0..3 {
            assert_eq!(step_over(m), Stepped::Returned);
        }
        assert_eq!(full_pc(m), 0x801C, "JSR wait");
    }
    let nmis = through.wram()[DBG_NMI_COUNTER];

    // `wait` is `WAI; RTS`: the NMI comes inside it.
    assert_eq!(
        stopping.step_over(&STOPPING, &mut no_input, &mut NullAudio).unwrap(),
        Stepped::Interrupt
    );
    assert_eq!(at(&stopping), (DBG_NMI, 0x1FEA));

    let stepped = through.step_over(&DEFAULT, &mut no_input, &mut NullAudio).unwrap();
    assert_eq!(stepped, Stepped::Returned);
    assert_eq!(at(&through), (0x801F, MAIN_STACK));
    assert_eq!(through.wram()[DBG_NMI_COUNTER], nmis.wrapping_add(1), "the handler ran");
}

#[test]
fn step_out_runs_through_the_interrupt_by_default() {
    let mut m = booted();
    for _ in 0..3 {
        assert_eq!(step_over(&mut m), Stepped::Returned);
    }
    assert_eq!(step_into(&mut m), Stepped::Instruction);
    assert_eq!(at(&m), (DBG_WAIT, 0x1FEE), "at the WAI of wait");
    let nmis = m.wram()[DBG_NMI_COUNTER];
    let mut twin = m.snapshot();
    assert_eq!(
        twin.step_out(&STOPPING, &mut no_input, &mut NullAudio).unwrap(),
        Stepped::Interrupt
    );

    assert_eq!(m.step_out(&DEFAULT, &mut no_input, &mut NullAudio).unwrap(), Stepped::Returned);
    assert_eq!(at(&m), (0x801F, MAIN_STACK), "out of wait, not out of the handler");
    assert_eq!(m.wram()[DBG_NMI_COUNTER], nmis.wrapping_add(1));
    // From the handler's first instruction, the handler is what it steps out of.
    assert_eq!(twin.step_out(&DEFAULT, &mut no_input, &mut NullAudio).unwrap(), Stepped::Returned);
    assert_eq!(at(&twin), (DBG_WAIT + 1, 0x1FEE));
}

#[test]
fn a_breakpoint_in_the_handler_stops_a_step_that_ignores_interrupts() {
    let mut m = booted();
    for _ in 0..3 {
        assert_eq!(step_over(&mut m), Stepped::Returned);
    }
    m.set_breakpoint(DBG_NMI + 1);
    assert_eq!(
        m.step_over(&DEFAULT, &mut no_input, &mut NullAudio).unwrap(),
        Stepped::Breakpoint { key: DBG_NMI + 1 }
    );
    assert_eq!(at(&m), (DBG_NMI + 1, 0x1FE9));
    // The step went on from there to its return.
    assert_eq!(m.step_out(&DEFAULT, &mut no_input, &mut NullAudio).unwrap(), Stepped::Returned);
    assert_eq!(at(&m), (DBG_WAIT + 1, 0x1FEE));
}

#[test]
fn the_budget_is_an_option() {
    let mut m = pure(common::no_return_rom());
    for _ in 0..5 {
        assert_eq!(step_into(&mut m), Stepped::Instruction);
    }
    let frames = m.frame_count();
    let options = StepOptions { budget_frames: 3, ..DEFAULT };
    assert_eq!(m.step_over(&options, &mut no_input, &mut NullAudio).unwrap(), Stepped::NoReturn);
    assert_eq!(m.frame_count(), frames + 3);
    assert_eq!(m.step_out(&options, &mut no_input, &mut NullAudio).unwrap(), Stepped::NoReturn);
    assert_eq!(m.frame_count(), frames + 6);
}

/// An NMI taken right after a `JSR`, before the callee's first instruction: the machine is at
/// that `JSR` (of [`common::nmi_leaf_rom`], hooked or not).
fn at_a_call_the_nmi_follows(table: Option<HookTable>) -> Machine {
    let mut m = pure(common::nmi_leaf_rom());
    m.set_hook_table(table);
    let interrupt = Stops { interrupt: true, ..Stops::default() };
    for _ in 0..200 {
        let mut back = m.snapshot();
        assert_eq!(run(&mut m, interrupt, 2), Stopped::Interrupt);
        let s = usize::from(m.cpu_registers().s);
        let pushed_pc = u16::from_le_bytes([m.wram()[s + 2], m.wram()[s + 3]]);
        if pushed_pc != common::LEAF as u16 {
            continue;
        }
        // Back to the JSR: the last instruction before the handler's first.
        let jsr = m.opcode_fetches() - 1 - back.opcode_fetches();
        let to_jsr = Stops { instructions: Some(jsr), ..Stops::default() };
        assert_eq!(run(&mut back, to_jsr, 2), Stopped::Instructions);
        assert_eq!(at(&back), (0x8014, MAIN_STACK));
        return back;
    }
    panic!("no NMI right after the JSR");
}

#[test]
fn an_interrupt_right_after_the_call_does_not_end_a_step_over() {
    let m = at_a_call_the_nmi_follows(None);
    let nmis = m.wram()[common::NMI_COUNTER];
    let (mut through, mut stopping) = (m.snapshot(), m.snapshot());
    assert_eq!(
        through.step_over(&DEFAULT, &mut no_input, &mut NullAudio).unwrap(),
        Stepped::Returned
    );
    assert_eq!(
        at(&through),
        (u32::from(common::NMI_LEAF_RETURN), MAIN_STACK),
        "back from the leaf, not in the handler or after its RTI"
    );
    assert_eq!(through.wram()[common::NMI_COUNTER], nmis.wrapping_add(1));
    assert_eq!(through.wram()[common::LEAF_COUNTER], m.wram()[common::LEAF_COUNTER] + 1);

    assert_eq!(
        stopping.step_over(&STOPPING, &mut no_input, &mut NullAudio).unwrap(),
        Stepped::Interrupt
    );
    assert_eq!(full_pc(&stopping), u32::from(common::NMI_HANDLER));
}

#[test]
fn a_dispatched_return_followed_by_nmis_never_stops_at_a_handler() {
    // The Rust leaf of `nmi_leaf_rom` returns, and now and then an NMI takes the CPU to the
    // handler before the next opcode fetch: the step is over at the handler's RTI, back at the
    // instruction after the call, never at the handler's first instruction.
    let mut m = pure(common::nmi_leaf_rom());
    m.set_hook_table(Some(table(vec![routine(
        "leaf",
        common::LEAF,
        HookMode::Rust,
        rust_leaf,
        Some(LEAF_COST),
    )])));
    let returned = Stops { returned_above: Some(MAIN_STACK - 0x10), ..Stops::default() };
    while m.nmi_count() < 20 {
        assert_eq!(run(&mut m, returned, 2), Stopped::Returned);
        // Back from the leaf, or from a handler to the interrupted code of the loop.
        assert_ne!(full_pc(&m), u32::from(common::NMI_HANDLER));
    }
    assert!(stats(&m, "leaf").rust_runs > 1000);
}

// Breakpoints set where the CPU is

#[test]
fn a_breakpoint_set_at_the_entry_the_cpu_waits_at_is_passed_by_the_next_run() {
    let mut m = booted();
    assert_eq!(step_over(&mut m), Stepped::Returned);
    assert_eq!(step_into(&mut m), Stepped::Instruction);
    // Onto the Rust leaf's entry: the arrival is pending.
    assert_eq!(at(&m), (DBG_RUST_LEAF, 0x1FEE));
    m.set_breakpoint(DBG_RUST_LEAF);
    assert_eq!(run(&mut m, BREAKPOINTS, 2), breakpoint(DBG_RUST_LEAF));
    // Not at once with no progress: the leaf ran, and a frame later it is back.
    assert_eq!(m.nmi_count(), 1);
    assert_eq!(stats(&m, "rust_leaf").rust_runs, 1);
    assert_eq!(at(&m), (DBG_RUST_LEAF, 0x1FEE));
    // A breakpoint elsewhere than where the CPU is changes nothing.
    m.set_breakpoint(DBG_VERIFY_LEAF);
    assert_eq!(run(&mut m, BREAKPOINTS, 2), breakpoint(DBG_VERIFY_LEAF));
}

// Breakpoints

#[test]
fn a_breakpoint_at_a_hooked_entry_stops_before_its_dispatch() {
    let mut m = machine();
    m.set_breakpoint(0x80_0000 | DBG_RUST_LEAF);
    m.set_breakpoint(DBG_VERIFY_LEAF);
    let keys: Vec<u32> = m.breakpoints().iter().copied().collect();
    assert_eq!(keys, [DBG_RUST_LEAF, DBG_VERIFY_LEAF], "by hook key: a mirror is the same");

    for round in 0..3 {
        assert_eq!(run(&mut m, BREAKPOINTS, 2), breakpoint(DBG_RUST_LEAF));
        assert_eq!(at(&m), (DBG_RUST_LEAF, 0x1FEE));
        assert_eq!(m.nmi_count(), round);
        assert_eq!(stats(&m, "rust_leaf").calls, round, "not dispatched yet");
        // Going on dispatches it.
        assert_eq!(run(&mut m, BREAKPOINTS, 2), breakpoint(DBG_VERIFY_LEAF));
        assert_eq!(at(&m), (DBG_VERIFY_LEAF, 0x1FEE));
        let leaf = stats(&m, "rust_leaf");
        assert_eq!((leaf.calls, leaf.rust_runs), (round + 1, round + 1), "{leaf:?}");
        assert_eq!(stats(&m, "verify_leaf").calls, round);
    }
    assert_eq!(run(&mut m, BREAKPOINTS, 2), breakpoint(DBG_RUST_LEAF));
    let leaf = stats(&m, "verify_leaf");
    assert_eq!((leaf.calls, leaf.verified_ok), (3, 3), "{leaf:?}");
    // From a breakpoint, a step into dispatches the routine too.
    assert_eq!(step_into(&mut m), routine_step("rust_leaf"));
    assert_eq!(stats(&m, "rust_leaf").rust_runs, 4);
    // Breakpoints are no entries.
    assert_eq!(m.hook_stats().len(), 2);
}

#[test]
fn stepping_off_a_breakpoint_does_not_stop_there_again() {
    let mut m = machine();
    m.set_breakpoint(OUTER_PLX);
    m.set_breakpoint(DBG_RUST_LEAF);
    assert_eq!(run(&mut m, BREAKPOINTS, 2), breakpoint(OUTER_PLX));
    assert_eq!(at(&m), (OUTER_PLX, 0x1FEC));
    assert_eq!(step_into(&mut m), Stepped::Instruction);
    assert_eq!(at(&m), (OUTER_PLX + 1, 0x1FEE));
    assert_eq!(step_into(&mut m), Stepped::Instruction);
    // Onto a breakpoint and on: no stop there either.
    assert_eq!(step_into(&mut m), Stepped::Instruction);
    assert_eq!(at(&m), (DBG_RUST_LEAF, 0x1FEE));
    assert_eq!(step_into(&mut m), routine_step("rust_leaf"));

    // Every kind of step goes on from a breakpoint.
    assert_eq!(run(&mut m, BREAKPOINTS, 2), breakpoint(OUTER_PLX));
    assert_eq!(m.nmi_count(), 1);
    assert_eq!(step_over(&mut m), Stepped::Instruction);
    assert_eq!(run(&mut m, BREAKPOINTS, 2), breakpoint(DBG_RUST_LEAF));
    assert_eq!(step_over(&mut m), routine_step("rust_leaf"));
    assert_eq!(run(&mut m, BREAKPOINTS, 2), breakpoint(OUTER_PLX));
    assert_eq!(m.nmi_count(), 2);
    assert_eq!(step_out(&mut m), Stepped::Returned);
    assert_eq!(at(&m), (CALL_RUST_LEAF, MAIN_STACK));

    // A step that ends at a breakpoint leaves it for the next run, which goes on from there.
    assert_eq!(step_into(&mut m), Stepped::Instruction);
    assert_eq!(at(&m), (DBG_RUST_LEAF, 0x1FEE));
    let runs = stats(&m, "rust_leaf").rust_runs;
    assert_eq!(run(&mut m, BREAKPOINTS, 2), breakpoint(OUTER_PLX));
    assert_eq!(m.nmi_count(), 3);
    assert_eq!(stats(&m, "rust_leaf").rust_runs, runs + 1);
}

#[test]
fn a_breakpoint_at_a_sync_point_still_yields_its_event() {
    let mut m = machine();
    let mut entries = leaves(HookMode::Rust, HookMode::Verify);
    entries.push(HookEntry::sync_point("loop", DBG_LOOP));
    m.set_hook_table(Some(table(entries)));
    m.set_breakpoint(DBG_LOOP);
    let stops = Stops { sync: true, ..BREAKPOINTS };
    // The first arrival, before any NMI: no event.
    assert_eq!(run(&mut m, stops, 2), breakpoint(DBG_LOOP));
    assert_eq!(m.nmi_count(), 0);
    for nmis in 1..=3 {
        // The breakpoint first; going on, the event, the CPU still there.
        assert_eq!(run(&mut m, stops, 2), breakpoint(DBG_LOOP));
        assert_eq!((at(&m), m.nmi_count()), ((DBG_LOOP, MAIN_STACK), nmis));
        let cycles = m.total_master_cycles();
        let Stopped::Sync(event) = run(&mut m, stops, 2) else { panic!("no event") };
        assert_eq!((event.key, event.nmi_count), (DBG_LOOP, nmis));
        assert_eq!(m.total_master_cycles(), cycles, "the CPU has not moved");
    }
    assert_eq!(stats(&m, "loop").calls, 4, "the boot arrival and three events");
}

#[test]
fn a_breakpoint_in_the_body_of_a_dispatched_routine_is_never_hit() {
    let mut m = machine();
    let (rust_rts, verify_rts) = (DBG_RUST_LEAF + 2, DBG_VERIFY_LEAF + 2);
    m.set_breakpoint(rust_rts);
    m.set_breakpoint(verify_rts);
    assert_eq!(run(&mut m, BREAKPOINTS, 4), Stopped::Budget);
    let (rust, verify) = (stats(&m, "rust_leaf"), stats(&m, "verify_leaf"));
    assert!(rust.rust_runs >= 4 && rust.rust_runs == rust.calls, "{rust:?}");
    assert!(verify.verified_ok >= 4 && verify.verified_ok == verify.calls, "{verify:?}");

    // In Original mode the body runs at the top level: hit. Breakpoints stay with a new table,
    // and one at an Original entry does not make it hooked (no calls counted).
    m.set_hook_table(Some(table(leaves(HookMode::Original, HookMode::Original))));
    m.set_breakpoint(DBG_VERIFY_LEAF);
    assert_eq!(run(&mut m, BREAKPOINTS, 2), breakpoint(rust_rts));
    assert_eq!(at(&m), (rust_rts, 0x1FEE));
    assert_eq!(run(&mut m, BREAKPOINTS, 2), breakpoint(DBG_VERIFY_LEAF));
    assert_eq!(run(&mut m, BREAKPOINTS, 2), breakpoint(verify_rts));
    assert_eq!(stats(&m, "verify_leaf"), verify, "an Original entry is not dispatched");
    // Without a table too.
    m.set_hook_table(None);
    assert_eq!(run(&mut m, BREAKPOINTS, 2), breakpoint(rust_rts));

    // Original code a Rust routine calls runs inside its dispatch: `inner` of `call_rom`, called
    // from `outer` only.
    let mut m = pure(common::call_rom(false));
    let outer = routine("outer", CALL_OUTER, HookMode::Rust, call_outer, Some(CALL_OUTER_COST));
    m.set_hook_table(Some(table(vec![outer])));
    m.set_breakpoint(CALL_INNER);
    assert_eq!(run(&mut m, BREAKPOINTS, 3), Stopped::Budget);
    let outer = stats(&m, "outer");
    assert!(outer.rust_runs > 100 && outer.rust_runs == outer.calls, "{outer:?}");
}

#[test]
fn breakpoints_stay_across_load_state_and_snapshots_copy_them() {
    let mut m = machine();
    let power_on = m.save_state();
    m.set_breakpoint(DBG_INNERMOST);
    assert_eq!(run(&mut m, BREAKPOINTS, 2), breakpoint(DBG_INNERMOST));
    let (at_breakpoint, cycles) = (m.save_state(), m.total_master_cycles());

    // A snapshot has the breakpoints and the pending arrival: it stops at the same place first,
    // then goes on to the next one.
    let mut copy = m.snapshot();
    assert_eq!(copy.breakpoints(), m.breakpoints());
    assert_eq!(run(&mut copy, BREAKPOINTS, 2), breakpoint(DBG_INNERMOST));
    assert_eq!(copy.total_master_cycles(), cycles);
    assert_eq!(run(&mut copy, BREAKPOINTS, 2), breakpoint(DBG_INNERMOST));
    assert_eq!(copy.nmi_count(), m.nmi_count() + 1);

    m.load_state(&power_on).unwrap();
    assert_eq!(run(&mut m, BREAKPOINTS, 2), breakpoint(DBG_INNERMOST));
    assert_eq!(m.total_master_cycles(), cycles);

    // A state saved at a breakpoint stops there first thing, then goes on.
    m.load_state(&at_breakpoint).unwrap();
    assert_eq!(run(&mut m, BREAKPOINTS, 2), breakpoint(DBG_INNERMOST));
    assert_eq!(m.total_master_cycles(), cycles);
    assert_eq!(run(&mut m, BREAKPOINTS, 2), breakpoint(DBG_INNERMOST));
    assert!(m.total_master_cycles() > cycles);
    // Setting the registers ends the skip as a load does.
    m.load_state(&at_breakpoint).unwrap();
    assert_eq!(run(&mut m, BREAKPOINTS, 2), breakpoint(DBG_INNERMOST));
    m.set_cpu_registers(m.cpu_registers());
    assert_eq!(run(&mut m, BREAKPOINTS, 2), breakpoint(DBG_INNERMOST));
    assert_eq!(m.total_master_cycles(), cycles);
    // A step never stops at the breakpoint it starts from, whatever left the CPU there.
    m.load_state(&at_breakpoint).unwrap();
    assert_eq!(step_into(&mut m), Stepped::Instruction);
    assert_eq!(at(&m), (DBG_INNERMOST + 2, 0x1FE7));
    m.load_state(&at_breakpoint).unwrap();
    assert_eq!(step_over(&mut m), Stepped::Instruction);
    assert_eq!(at(&m), (DBG_INNERMOST + 2, 0x1FE7));
    m.load_state(&at_breakpoint).unwrap();
    assert_eq!(step_out(&mut m), Stepped::Returned);
    assert_eq!(at(&m), (0x8053, 0x1FE9));

    m.clear_breakpoint(DBG_INNERMOST);
    assert!(m.breakpoints().is_empty());
    assert_eq!(run(&mut m, BREAKPOINTS, 2), Stopped::Budget);
}

#[test]
fn runs_without_the_breakpoint_stop_pass_breakpoints_cycle_for_cycle() {
    let set = |m: &mut Machine| {
        for address in [DBG_LOOP, DBG_NMI, DBG_RUST_LEAF, DBG_VERIFY_LEAF + 2, OUTER_PLX] {
            m.set_breakpoint(address);
        }
    };
    // Unhooked: breakpoints are no hooks, a recording takes them.
    let mut plain = pure(common::debugger_rom());
    let mut with = pure(common::debugger_rom());
    set(&mut with);
    let mut recorder = Recorder::start(&with).unwrap();
    // Hooked.
    let (mut hooked, mut hooked_with) = (machine(), machine());
    set(&mut hooked_with);
    for _ in 0..10 {
        plain.run_frame(Joypad::default(), &mut NullAudio).unwrap();
        recorder.run_frame(&mut with, Joypad::default(), &mut NullAudio).unwrap();
        hooked.run_frame(Joypad::default(), &mut NullAudio).unwrap();
        hooked_with.run_frame(Joypad::default(), &mut NullAudio).unwrap();
    }
    assert_eq!(plain.save_state(), with.save_state());
    assert_eq!(hooked.save_state(), hooked_with.save_state());
    assert_eq!(hooked.hook_stats(), hooked_with.hook_stats());
    assert_eq!(recorder.replay().frames(), 10);
}

// Satellite stops

#[test]
fn a_call_stop_tells_whether_its_dispatch_rendered_a_frame() {
    let mut m = pure(common::debugger_rom());
    let mut entries = leaves(HookMode::Rust, HookMode::Verify);
    // The frame renders while `wait` waits for the NMI: inside its dispatch.
    entries.push(routine("wait", DBG_WAIT, HookMode::Verify, wait, None));
    m.set_hook_table(Some(table(entries)));
    for _ in 0..4 {
        let frames = m.frame_count();
        let stops = Stops { call: Some(DBG_WAIT), frame_end: true, ..Stops::default() };
        let Stopped::Call { stats, rendered } = run(&mut m, stops, 2) else { panic!("no call") };
        assert!(rendered);
        assert_eq!(m.frame_count(), frames + 1);
        assert_eq!(stats.calls, stats.interrupted, "{stats:?}");

        let stops = Stops { call: Some(DBG_RUST_LEAF), frame_end: true, ..Stops::default() };
        let Stopped::Call { stats, rendered } = run(&mut m, stops, 2) else { panic!("no call") };
        assert!(!rendered);
        assert_eq!(m.frame_count(), frames + 1);
        assert_eq!(stats.calls, stats.rust_runs, "{stats:?}");
    }
}
