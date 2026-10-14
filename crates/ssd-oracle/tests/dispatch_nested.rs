//! Nested `Verify` calls: an arrival at another `Verify` entry inside the original of a `Verify`
//! call is checked as its own call, and the outer call is checked as usual around it.

mod common;

use common::{
    CHAIN, CHAIN_COUNTER, CHAIN_LEN, NESTED_INNER, NESTED_INNER_COUNTER, NESTED_NMI_COUNTER,
    NESTED_OUTER, NESTED_OUTER_COUNTER, NESTED_OUTER_TAIL, NESTED_SPIN, NESTED_SPIN_CALLER,
    NESTED_WRITES, NMI_CALL_LONG, NMI_CALL_LONG_COUNTER, NMI_CALL_NMI_COUNTER, NMI_CALL_SHORT,
    NMI_CALL_SHORT_COUNTER, RECURSIVE_LEAF,
};
use ssd_oracle::{
    Ctx, Difference, HookAction, HookEntry, HookMode, HookStats, HookTable, Joypad, Machine,
    MemorySaveStore, NullAudio, ReturnKind, RoutineFn,
};
use std::time::{Duration, Instant};

// CPU-busy master cycles in [`common::nested_rom`] (8-cycle `SlowROM` and WRAM accesses, PPU
// registers and internal cycles 6).

/// `LDA #imm` with an 8-bit accumulator.
const LDA_IMM: u64 = 16;
/// `STA abs` and `STZ abs` to a PPU register.
const STA_PPU: u64 = 30;
/// `JSR abs`: opcode, two operand bytes, an internal cycle, two pushes.
const JSR: u64 = 46;
/// `JMP abs`.
const JMP: u64 = 24;
/// `INC dp` with D = 0.
const INC_DP: u64 = 38;
/// `RTS`: opcode, three internal cycles, two pulls.
const RTS: u64 = 42;
/// `PHX` with 16-bit X.
const PHX: u64 = 30;
/// `PLX` with 16-bit X.
const PLX: u64 = 36;
/// `LDY #imm` with 16-bit Y.
const LDY_IMM: u64 = 24;
/// `DEY`.
const DEY: u64 = 14;
/// `BNE` taken and not taken.
const BNE_TAKEN: u64 = 22;
const BNE_NOT_TAKEN: u64 = 16;
/// `inner`: `PHX; INC $10; LDA #$E0; STA $2132; LDY #$0010`, 16 `DEY; BNE`, `PLX; RTS`.
const INNER_COST: u64 = PHX
    + INC_DP
    + LDA_IMM
    + STA_PPU
    + LDY_IMM
    + 16 * DEY
    + 15 * BNE_TAKEN
    + BNE_NOT_TAKEN
    + PLX
    + RTS;
/// `outer`: `LDA #$0F; STA $2100; JSR inner; STZ $2132; INC $11; RTS`, `inner` included.
const OUTER_COST: u64 = LDA_IMM + STA_PPU + JSR + INNER_COST + STA_PPU + INC_DP + RTS;
/// `outer_tail`: `INC $11; JMP inner`, `inner` included.
const OUTER_TAIL_COST: u64 = INC_DP + JMP + INNER_COST;

fn machine_with(rom: Vec<u8>) -> Machine {
    Machine::new(rom, Box::new(MemorySaveStore::default())).unwrap()
}

fn run(m: &mut Machine, frames: usize) {
    for _ in 0..frames {
        m.run_frame(Joypad::default(), &mut NullAudio).unwrap();
    }
}

/// A routine returning with RTS, checking every call; contract: 8-bit A, 16-bit X/Y (the
/// fixtures').
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

fn table(entries: impl IntoIterator<Item = HookEntry>) -> HookTable {
    entries.into_iter().collect()
}

fn stats(m: &Machine, name: &str) -> HookStats {
    let all = m.hook_stats();
    all.iter()
        .find(|(n, _)| n == name)
        .map(|&(_, s)| s)
        .unwrap_or_else(|| panic!("{name}: {all:?}"))
}

/// The first divergence kept, as text.
fn first_divergence(m: &Machine) -> String {
    m.divergences().first().map(ToString::to_string).unwrap_or_default()
}

/// The mean of equal cycle counts is exact; the tolerance only keeps the comparison float-safe.
fn assert_mean_cycles(stats: &HookStats, expected: u64, what: &str) {
    let mean = stats.mean_cpu_cycles;
    assert!((mean - expected as f64).abs() < 1e-9, "{what}: mean {mean}, expected {expected}");
}

// The routines of `nested_rom` in Rust

/// `inner`'s work, adding `step` to its counter: X is kept (pushed and pulled), so N and Z come
/// from it.
fn inner_work(ctx: &mut Ctx<'_>, step: u8) {
    let value = ctx.read_dp8(NESTED_INNER_COUNTER as u16).wrapping_add(step);
    ctx.write_dp8(NESTED_INNER_COUNTER as u16, value);
    let [_, (coldata, color), _] = NESTED_WRITES;
    ctx.write8(coldata, color);
    ctx.set_al(color);
    ctx.set_y(0);
    let x = ctx.x();
    ctx.set_nz16(x);
}

fn inner(ctx: &mut Ctx<'_>) {
    inner_work(ctx, 1);
    ctx.set_cost(INNER_COST);
}

/// A wrong `inner`: adds 2.
fn inner_adding_two(ctx: &mut Ctx<'_>) {
    inner_work(ctx, 2);
    ctx.set_cost(INNER_COST);
}

/// `INC $11`, the last instruction of the callers before they return.
fn inc_outer_counter(ctx: &mut Ctx<'_>) {
    let value = ctx.read_dp8(NESTED_OUTER_COUNTER as u16).wrapping_add(1);
    ctx.write_dp8(NESTED_OUTER_COUNTER as u16, value);
    ctx.set_nz8(value);
}

/// `LDA #$0F; STA $2100`, before `outer`'s call of `inner`.
fn write_inidisp(ctx: &mut Ctx<'_>) {
    let [(inidisp, brightness), ..] = NESTED_WRITES;
    ctx.write8(inidisp, brightness);
}

/// `STZ $2132; INC $11`, after `outer`'s call of `inner`.
fn outer_after_the_call(ctx: &mut Ctx<'_>) {
    let [.., (coldata, color)] = NESTED_WRITES;
    ctx.write8(coldata, color);
    inc_outer_counter(ctx);
}

/// `outer` with `inner` inlined: the nested call is part of its body, so its own cost includes
/// `inner`'s time.
fn outer(ctx: &mut Ctx<'_>) {
    write_inidisp(ctx);
    inner_work(ctx, 1);
    outer_after_the_call(ctx);
    ctx.set_cost(OUTER_COST);
}

/// [`outer`] declaring only the cost of its own instructions, as if `inner`'s time were not
/// part of it.
fn outer_without_the_nested_time(ctx: &mut Ctx<'_>) {
    write_inidisp(ctx);
    inner_work(ctx, 1);
    outer_after_the_call(ctx);
    ctx.set_cost(OUTER_COST - INNER_COST);
}

/// `outer` running `inner` as original code: `inner`'s time is the called code's, measured.
fn outer_calling_inner(ctx: &mut Ctx<'_>) {
    write_inidisp(ctx);
    ctx.charge(LDA_IMM + STA_PPU + JSR);
    let Ok(()) = ctx.call_original(NESTED_INNER, ReturnKind::Rts) else { return };
    outer_after_the_call(ctx);
    ctx.set_cost(OUTER_COST - INNER_COST);
}

/// `outer_tail` with `inner` inlined.
fn outer_tail(ctx: &mut Ctx<'_>) {
    inc_outer_counter(ctx);
    inner_work(ctx, 1);
    ctx.set_cost(OUTER_TAIL_COST);
}

/// A machine on [`common::nested_rom`] with `outer`, `outer_tail` and `inner` hooked.
fn nested_machine(nmi: bool, outer: HookEntry, inner: HookEntry) -> Machine {
    let mut m = machine_with(common::nested_rom(nmi));
    let tail = routine(
        "outer_tail",
        NESTED_OUTER_TAIL,
        HookMode::Verify,
        outer_tail,
        Some(OUTER_TAIL_COST),
    );
    m.set_hook_table(Some(table([outer, tail, inner])));
    m
}

fn verify_outer(func: RoutineFn) -> HookEntry {
    routine("outer", NESTED_OUTER, HookMode::Verify, func, Some(OUTER_COST))
}

fn verify_inner(func: RoutineFn) -> HookEntry {
    routine("inner", NESTED_INNER, HookMode::Verify, func, Some(INNER_COST))
}

/// Ticks a pure [`common::nested_rom`] machine (with the user's write log as `write_log`) to
/// `live`'s master cycle.
fn twin_of(live: &Machine, nmi: bool, write_log: bool) -> Machine {
    let mut twin = machine_with(common::nested_rom(nmi));
    twin.set_cpu_write_log(write_log);
    while twin.total_master_cycles() < live.total_master_cycles() {
        twin.tick(Joypad::default(), &mut NullAudio).unwrap();
    }
    assert_eq!(twin.total_master_cycles(), live.total_master_cycles());
    twin
}

#[test]
fn a_verify_entry_reached_inside_another_verify_call_is_checked() {
    let mut m = nested_machine(false, verify_outer(outer), verify_inner(inner));
    run(&mut m, 4);
    // A frame rendered during a nested call ends `run_frame` after the outer one.
    assert_eq!(m.frame_count(), 4);

    for (name, cost) in [("outer", OUTER_COST), ("outer_tail", OUTER_TAIL_COST)] {
        let s = stats(&m, name);
        assert!(s.verified_ok > 100, "{name}: {s:?}");
        assert_eq!(s.calls, s.verified_ok, "{name}: {s:?}");
        assert_eq!(s.divergences, 0, "{name}: {}", first_divergence(&m));
        assert_eq!((s.nested, s.nested_overflow), (0, 0), "{name}: called from the main loop");
        assert_eq!((s.cost_mismatch, s.cost_over_bound), (0, 0), "{name}: {s:?}");
        // The measured cost includes the nested call, which ran on the live machine.
        assert_mean_cycles(&s, cost, name);
    }
    let (outer_calls, tail_calls) = (stats(&m, "outer").calls, stats(&m, "outer_tail").calls);
    let s = stats(&m, "inner");
    assert_eq!(s.calls, outer_calls + tail_calls, "{s:?}");
    assert_eq!((s.nested, s.verified_ok), (s.calls, s.calls), "{s:?}");
    assert_eq!((s.divergences, s.cost_mismatch, s.nested_overflow), (0, 0, 0), "{s:?}");
    assert_mean_cycles(&s, INNER_COST, "inner");

    // The live machine kept the original's results and timing.
    assert_eq!(m.save_state(), twin_of(&m, false, false).save_state());
}

#[test]
fn a_wrong_nested_routine_diverges_and_its_caller_still_verifies() {
    let mut m = nested_machine(false, verify_outer(outer), verify_inner(inner_adding_two));
    run(&mut m, 2);

    let s = stats(&m, "inner");
    assert!(s.calls > 100, "{s:?}");
    assert_eq!((s.divergences, s.verified_ok, s.nested), (s.calls, 0, s.calls), "{s:?}");
    for name in ["outer", "outer_tail"] {
        let s = stats(&m, name);
        assert!(s.verified_ok > 50, "{name}: {s:?}");
        assert_eq!((s.calls, s.divergences), (s.verified_ok, 0), "{name}: {s:?}");
    }
    assert_eq!(m.divergences().len(), 32, "the first 32 are kept");
    for divergence in m.divergences() {
        assert_eq!(divergence.routine, "inner", "{divergence}");
        let [Difference::Memory { space: "wram", address: 0x10, original, rust }] =
            divergence.differences[..]
        else {
            panic!("only the inner counter differs: {divergence}");
        };
        assert_eq!(rust, original.wrapping_add(1), "{divergence}");
    }
}

#[test]
fn the_callers_own_cost_includes_a_nested_call_unless_it_calls_it_as_original_code() {
    // Declaring the nested call's time apart is a cost mismatch on every call.
    let mut m =
        nested_machine(false, verify_outer(outer_without_the_nested_time), verify_inner(inner));
    run(&mut m, 2);
    let s = stats(&m, "outer");
    assert!(s.verified_ok > 100, "{s:?}");
    assert_eq!((s.divergences, s.cost_mismatch), (0, s.verified_ok), "{s:?}");

    // Run as original code through `call_original`, `inner`'s time is measured and added: on
    // the snapshot it runs in Rust and is charged its declared cost.
    let mut m = nested_machine(false, verify_outer(outer_calling_inner), verify_inner(inner));
    run(&mut m, 2);
    let s = stats(&m, "outer");
    assert!(s.verified_ok > 100, "{s:?}");
    assert_eq!((s.divergences, s.cost_mismatch), (0, 0), "{}", first_divergence(&m));
    assert_mean_cycles(&s, OUTER_COST, "outer");
    // Only the live machine's nested calls count: the snapshot's are discarded with it.
    let inner_calls = stats(&m, "inner").calls;
    assert_eq!(inner_calls, s.calls + stats(&m, "outer_tail").calls);
}

#[test]
fn the_users_cpu_write_log_is_intact_after_nested_calls() {
    let mut live = nested_machine(false, verify_outer(outer), verify_inner(inner));
    live.set_cpu_write_log(true);
    let mut live_writes = Vec::new();
    for _ in 0..2 {
        live.run_frame(Joypad::default(), &mut NullAudio).unwrap();
        live_writes.extend(live.take_cpu_writes());
    }
    for name in ["outer", "outer_tail", "inner"] {
        let s = stats(&live, name);
        assert!(s.verified_ok > 100, "{name}: {s:?}");
        assert_eq!(s.divergences, 0, "{name}: {}", first_divergence(&live));
    }
    let twin_writes = twin_of(&live, false, true).take_cpu_writes();
    assert!(twin_writes.len() > 1000, "every call writes");
    assert_eq!(live_writes, twin_writes);

    // With the log off, none of the nested recordings leaks into it.
    let mut m = nested_machine(false, verify_outer(outer), verify_inner(inner));
    run(&mut m, 2);
    assert!(stats(&m, "inner").verified_ok > 100);
    assert_eq!(stats(&m, "outer").divergences, 0, "{}", first_divergence(&m));
    assert_eq!(m.take_cpu_writes(), []);
}

#[test]
fn an_interrupt_inside_a_nested_call_interrupts_its_caller() {
    let mut m = nested_machine(true, verify_outer(outer), verify_inner(inner));
    let nmis = m.wram()[NESTED_NMI_COUNTER];
    run(&mut m, 12);
    assert!(m.wram()[NESTED_NMI_COUNTER].wrapping_sub(nmis) >= 10, "the NMI ran");

    let inner_stats = stats(&m, "inner");
    assert!(inner_stats.interrupted >= 5, "most NMIs land in inner: {inner_stats:?}");
    let mut callers_interrupted = 0;
    for name in ["outer", "outer_tail", "inner"] {
        let s = stats(&m, name);
        assert!(s.verified_ok > 100, "{name}: {s:?}");
        assert_eq!(s.divergences, 0, "{name}: {}", first_divergence(&m));
        assert_eq!(s.calls, s.verified_ok + s.interrupted, "{name}: {s:?}");
        if name != "inner" {
            callers_interrupted += s.interrupted;
        }
    }
    // Each interrupted inner call interrupted its caller, which is not compared either.
    assert!(callers_interrupted >= inner_stats.interrupted, "{:?}", m.hook_stats());
    assert_eq!(m.save_state(), twin_of(&m, true, false).save_state());
}

#[test]
fn entries_in_other_modes_run_as_original_inside_a_verify_call() {
    // Every call of inner comes from inside a Verify call's original: the wrong Rust function
    // never runs.
    let rust_inner =
        routine("inner", NESTED_INNER, HookMode::Rust, inner_adding_two, Some(INNER_COST));
    let mut m = nested_machine(false, verify_outer(outer), rust_inner);
    run(&mut m, 2);
    assert_eq!(stats(&m, "inner"), HookStats::default(), "never dispatched");
    for name in ["outer", "outer_tail"] {
        let s = stats(&m, name);
        assert!(s.verified_ok > 100, "{name}: {s:?}");
        assert_eq!(s.divergences, 0, "{name}: {}", first_divergence(&m));
    }
    assert_eq!(m.save_state(), twin_of(&m, false, false).save_state());
}

// Depth

/// Every routine of the chain in Rust: they all leave the counter incremented once, with N/Z
/// from it.
fn chain_routine(ctx: &mut Ctx<'_>) {
    let value = ctx.read_dp8(CHAIN_COUNTER as u16).wrapping_add(1);
    ctx.write_dp8(CHAIN_COUNTER as u16, value);
    ctx.set_nz8(value);
}

#[test]
fn at_most_16_verify_calls_are_in_progress() {
    let mut m = machine_with(common::chain_rom());
    m.set_hook_table(Some(table((0..CHAIN_LEN).map(|i| {
        let key = CHAIN + 4 * i as u32;
        routine(&format!("chain_{i:02}"), key, HookMode::Verify, chain_routine, None)
    }))));
    run(&mut m, 2);

    let calls = stats(&m, "chain_00").calls;
    assert!(calls > 10, "{calls}");
    for i in 0..CHAIN_LEN {
        let name = format!("chain_{i:02}");
        let s = stats(&m, &name);
        assert_eq!(s.calls, calls, "{name}: {s:?}");
        assert_eq!(s.divergences, 0, "{name}: {}", first_divergence(&m));
        match i {
            0 => assert_eq!((s.verified_ok, s.nested, s.nested_overflow), (calls, 0, 0)),
            // The 16th call in progress is chain_15.
            1..16 => assert_eq!((s.verified_ok, s.nested, s.nested_overflow), (calls, calls, 0)),
            _ => assert_eq!((s.verified_ok, s.nested, s.nested_overflow), (0, 0, calls)),
        }
    }
}

// Calls from interrupt handlers

/// The long routine of [`common::nmi_call_rom`]: `INC $11; LDX #$0100`, 256 `DEX; BNE`, `RTS`
/// (`LDX` and `DEX` cost as `LDY` and `DEY`).
const LONG_COST: u64 = INC_DP + LDY_IMM + 256 * DEY + 255 * BNE_TAKEN + BNE_NOT_TAKEN + RTS;
/// The short routine of [`common::nmi_call_rom`]: `INC $10; RTS`.
const SHORT_COST: u64 = INC_DP + RTS;

/// The long routine: X ends at 0, with N and Z from it.
fn long_routine(ctx: &mut Ctx<'_>) {
    let value = ctx.read_dp8(NMI_CALL_LONG_COUNTER as u16).wrapping_add(1);
    ctx.write_dp8(NMI_CALL_LONG_COUNTER as u16, value);
    ctx.set_x(0);
    ctx.set_nz16(0);
    ctx.set_cost(LONG_COST);
}

/// The short routine, adding `step`.
fn short_routine_adding(ctx: &mut Ctx<'_>, step: u8) {
    let value = ctx.read_dp8(NMI_CALL_SHORT_COUNTER as u16).wrapping_add(step);
    ctx.write_dp8(NMI_CALL_SHORT_COUNTER as u16, value);
    ctx.set_nz8(value);
    ctx.set_cost(SHORT_COST);
}

fn short_routine(ctx: &mut Ctx<'_>) {
    short_routine_adding(ctx, 1);
}

/// A wrong short routine: adds 2.
fn short_routine_adding_two(ctx: &mut Ctx<'_>) {
    short_routine_adding(ctx, 2);
}

fn verify_long() -> HookEntry {
    routine("long", NMI_CALL_LONG, HookMode::Verify, long_routine, Some(LONG_COST))
}

#[test]
fn calls_from_an_interrupt_handler_inside_a_verify_call_are_checked() {
    for (func, wrong) in [(short_routine as RoutineFn, false), (short_routine_adding_two, true)] {
        let mut m = machine_with(common::nmi_call_rom(NMI_CALL_SHORT));
        let short = routine("short", NMI_CALL_SHORT, HookMode::Verify, func, Some(SHORT_COST));
        m.set_hook_table(Some(table([verify_long(), short])));
        let nmis = m.wram()[NMI_CALL_NMI_COUNTER];
        run(&mut m, 6);
        assert!(m.wram()[NMI_CALL_NMI_COUNTER].wrapping_sub(nmis) >= 5, "the NMI ran");

        // The NMI lands inside the long routine: those calls are interrupted, not compared.
        let long = stats(&m, "long");
        assert!(long.interrupted >= 5, "{long:?}");
        assert_eq!(long.calls, long.verified_ok + long.interrupted, "{long:?}");
        assert_eq!((long.divergences, long.cost_mismatch), (0, 0), "{}", first_divergence(&m));
        // The handler's call runs inside those calls' originals, nested, and is compared: the
        // interrupt is its caller's, not its own.
        let short = stats(&m, "short");
        assert_eq!(short.nested, long.interrupted, "{short:?}");
        assert_eq!((short.interrupted, short.cost_mismatch), (0, 0), "{short:?}");
        let (verified, diverged) = if wrong { (0, short.calls) } else { (short.calls, 0) };
        assert_eq!((short.verified_ok, short.divergences), (verified, diverged), "{short:?}");
        assert_mean_cycles(&short, SHORT_COST, "short");
    }
}

#[test]
fn an_interrupt_handler_reentering_a_verify_call_in_progress_is_counted() {
    // The handler calls the long routine, whose call the NMI interrupted.
    let mut m = machine_with(common::nmi_call_rom(NMI_CALL_LONG));
    m.set_hook_table(Some(table([verify_long()])));
    run(&mut m, 6);

    let s = stats(&m, "long");
    assert!(s.interrupted >= 5, "{s:?}");
    // Each interrupted call was entered again once, by its handler; the original ran through
    // that arrival.
    assert_eq!(s.reentered, s.interrupted, "{s:?}");
    assert_eq!(s.calls, s.verified_ok + s.interrupted + s.reentered, "{s:?}");
    assert_eq!((s.divergences, s.nested), (0, 0), "{}", first_divergence(&m));

    let mut twin = machine_with(common::nmi_call_rom(NMI_CALL_LONG));
    while twin.total_master_cycles() < m.total_master_cycles() {
        twin.tick(Joypad::default(), &mut NullAudio).unwrap();
    }
    assert_eq!(m.save_state(), twin.save_state());
}

// A routine that calls itself

/// The routine of [`common::recursive_leaf_rom`]: the counter after one increment, or two if that
/// is odd (the recursion), then A = the counter shifted right and C clear.
fn recursive_leaf(ctx: &mut Ctx<'_>) {
    let incremented = ctx.read_dp8(NESTED_INNER_COUNTER as u16).wrapping_add(1);
    let counter = if incremented & 1 == 1 { incremented.wrapping_add(1) } else { incremented };
    ctx.write_dp8(NESTED_INNER_COUNTER as u16, counter);
    ctx.set_al(counter >> 1);
    ctx.set_nz8(counter >> 1);
    ctx.set_c(false);
}

#[test]
fn a_routine_calling_itself_is_one_call_with_a_reentry() {
    let mut m = machine_with(common::recursive_leaf_rom());
    m.set_hook_table(Some(table([routine(
        "recursive",
        RECURSIVE_LEAF,
        HookMode::Verify,
        recursive_leaf,
        None,
    )])));
    run(&mut m, 4);

    let s = stats(&m, "recursive");
    assert!(s.verified_ok > 100, "{s:?}");
    assert_eq!((s.divergences, s.nested, s.nested_overflow), (0, 0, 0), "{}", first_divergence(&m));
    // The JSR to its own entry arrives with S below the S of the hit: the original runs through
    // it, as part of the call, counted once for each call from the loop that recurses (all but
    // perhaps the first).
    assert!(s.reentered <= s.verified_ok && s.reentered + 1 >= s.verified_ok, "{s:?}");
    assert_eq!(s.calls, s.verified_ok + s.reentered, "{s:?}");
    // 11 instructions with the recursion (INC, LDA, LSR, BCC, JSR, then the same four and RTS,
    // then RTS), 5 without.
    let (recursions, plain) = (s.reentered as f64, (s.verified_ok - s.reentered) as f64);
    let expected = (11.0 * recursions + 5.0 * plain) / s.verified_ok as f64;
    assert!((s.mean_instructions - expected).abs() < 1e-6, "{expected}: {s:?}");
}

// A nested call that does not return

/// Never compared: its call does not return.
fn not_compared(_: &mut Ctx<'_>) {}

#[test]
fn a_nested_call_that_does_not_return_demotes_both_calls() {
    let start = Instant::now();
    let mut m = machine_with(common::nested_no_return_rom());
    m.set_hook_table(Some(table([
        routine("caller", NESTED_SPIN_CALLER, HookMode::Verify, not_compared, None),
        routine("spin", NESTED_SPIN, HookMode::Verify, not_compared, None),
    ])));
    // The caller's dispatch ends cleanly, after both budgets ran out.
    run(&mut m, 1);
    assert!(m.frame_count() >= 2, "{}", m.frame_count());

    let (caller, spin) = (stats(&m, "caller"), stats(&m, "spin"));
    assert_eq!((caller.calls, caller.no_return, caller.nested), (1, 1, 0), "{caller:?}");
    assert_eq!((spin.calls, spin.no_return, spin.nested), (1, 1, 1), "{spin:?}");
    // The spin's arrivals at its own entry, at the S of its hit, are no re-entries.
    assert_eq!(spin.reentered, 0, "{spin:?}");
    for s in [caller, spin] {
        assert_eq!(s.verified_ok + s.divergences + s.interrupted, 0, "{s:?}");
    }

    // Both demoted: the machine runs on, spinning, without dispatching them again.
    run(&mut m, 3);
    assert_eq!((stats(&m, "caller"), stats(&m, "spin")), (caller, spin));
    assert!(start.elapsed() < Duration::from_secs(5), "{:?}", start.elapsed());
}
