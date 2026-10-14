//! `Ctx::call_original`: Rust routines that call original code, in `Rust` and `Verify` mode.

mod common;

use common::{
    CALL_FLAT, CALL_INNER, CALL_INNER_COUNTER, CALL_INNER_LONG, CALL_LOOP, CALL_NMI_COPY,
    CALL_NMI_COUNTER, CALL_OUTER, CALL_OUTER_COUNTER, CALL_OUTER_LONG, CALL_OUTER_RETURN,
    CALL_SPIN, LEAF, LEAF_COUNTER, MAIN_STACK, TWO_PATH_ODD_COUNTER,
};
use ssd_oracle::{
    Ctx, Difference, HookAction, HookEntry, HookMode, HookStats, HookTable, Joypad, Machine,
    MemorySaveStore, NullAudio, OracleError, ReturnKind, RoutineFn, SyncEvent, TickEvent,
};
use std::time::{Duration, Instant};

// CPU-busy master cycles in [`common::call_rom`] (8-cycle `SlowROM` and WRAM accesses, internal
// cycles 6).

/// `JSR abs`: opcode, two operand bytes, an internal cycle, two pushes.
const JSR: u64 = 46;
/// `JSL long`: opcode, three operand bytes, three pushes, an internal cycle.
const JSL: u64 = 62;
/// `INC dp` with D = 0.
const INC_DP: u64 = 38;
/// `RTS`: opcode, three internal cycles, two pulls.
const RTS: u64 = 42;
/// `RTL`: opcode, two internal cycles, three pulls.
const RTL: u64 = 44;
/// `inner`: `INC $10; RTS`.
const INNER_COST: u64 = INC_DP + RTS;
/// `inner_long`: `INC $10; RTL`.
const INNER_LONG_COST: u64 = INC_DP + RTL;
/// `outer`: `JSR inner; INC $11; RTS`.
const OUTER_COST: u64 = JSR + INNER_COST + INC_DP + RTS;
/// `flat`: `INC $10; INC $11; RTS`.
const FLAT_COST: u64 = 2 * INC_DP + RTS;
/// `outer_long`: `JSL inner_long; INC $11; RTS`.
const OUTER_LONG_COST: u64 = JSL + INNER_LONG_COST + INC_DP + RTS;
/// The lowest stack byte the calls and the NMI handler of [`common::call_rom`] can push to: the
/// bytes from here to [`MAIN_STACK`] are dead in the main loop.
const STACK_FLOOR: usize = 0x1FE0;
/// Sync events the timeline test compares (one per frame).
const EVENTS: usize = 60;
/// How much sooner an interrupt entry is at the trap of a call from Rust than at the original's
/// return address: its first two cycles read at the CPU's address, `$00:21FF` (6 master cycles)
/// instead of `SlowROM` (8).
const INTERRUPT_AT_THE_TRAP: u64 = 2 * (8 - 6);

fn machine_with(rom: Vec<u8>) -> Machine {
    Machine::new(rom, Box::new(MemorySaveStore::default())).unwrap()
}

fn run(m: &mut Machine, frames: usize) {
    for _ in 0..frames {
        m.run_frame(Joypad::default(), &mut NullAudio).unwrap();
    }
}

fn tick(m: &mut Machine) -> TickEvent {
    m.tick(Joypad::default(), &mut NullAudio).unwrap()
}

/// A routine returning with RTS; contract: 8-bit A, 16-bit X/Y (the fixture's).
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

// The routines in Rust

/// `INC $11`, the work every caller does after its call.
fn inc_outer_counter(ctx: &mut Ctx<'_>) {
    let value = ctx.read_dp8(CALL_OUTER_COUNTER as u16).wrapping_add(1);
    ctx.write_dp8(CALL_OUTER_COUNTER as u16, value);
    ctx.set_nz8(value);
}

/// `outer`: the time of its `JSR` first, so `inner` runs when the original runs it, then `inner`
/// as original code. Its own cost excludes `inner`'s time.
fn outer(ctx: &mut Ctx<'_>) {
    ctx.charge(JSR);
    let Ok(()) = ctx.call_original(CALL_INNER, ReturnKind::Rts) else { return };
    inc_outer_counter(ctx);
    ctx.set_cost(JSR + INC_DP + RTS);
}

/// `flat` through `inner`, whose `INC $10` is `flat`'s first instruction: its own cost is
/// `flat`'s minus `inner`'s.
fn flat(ctx: &mut Ctx<'_>) {
    let Ok(()) = ctx.call_original(CALL_INNER, ReturnKind::Rts) else { return };
    inc_outer_counter(ctx);
    ctx.set_cost(FLAT_COST - INNER_COST);
}

/// `outer_long`: the time of its `JSL`, then `inner_long` (returning with RTL).
fn outer_long(ctx: &mut Ctx<'_>) {
    ctx.charge(JSL);
    let Ok(()) = ctx.call_original(CALL_INNER_LONG, ReturnKind::Rtl) else { return };
    inc_outer_counter(ctx);
    ctx.set_cost(JSL + INC_DP + RTS);
}

/// The three callers of [`common::call_rom`] in `mode`, with their exact maximum costs.
fn callers(mode: HookMode) -> [HookEntry; 3] {
    [
        routine("outer", CALL_OUTER, mode, outer, Some(OUTER_COST)),
        routine("flat", CALL_FLAT, mode, flat, Some(FLAT_COST)),
        routine("outer_long", CALL_OUTER_LONG, mode, outer_long, Some(OUTER_LONG_COST)),
    ]
}

/// The mean of equal cycle counts is exact; the tolerance only keeps the comparison float-safe.
fn assert_mean_cycles(stats: &HookStats, expected: u64, what: &str) {
    let mean = stats.mean_cpu_cycles;
    assert!((mean - expected as f64).abs() < 1e-9, "{what}: mean {mean}, expected {expected}");
}

#[test]
fn verify_accepts_rust_routines_calling_original_code() {
    let mut m = machine_with(common::call_rom(false));
    m.set_hook_table(Some(table(callers(HookMode::Verify))));
    run(&mut m, 2);

    for (name, cost) in
        [("outer", OUTER_COST), ("flat", FLAT_COST), ("outer_long", OUTER_LONG_COST)]
    {
        let s = stats(&m, name);
        assert!(s.verified_ok > 100, "{name}: {s:?}");
        let first = m.divergences().first().map(ToString::to_string);
        assert_eq!(s.divergences, 0, "{name}: {}", first.unwrap_or_default());
        // The declared own cost plus the callee's measured time is the original's cost.
        assert_eq!((s.cost_mismatch, s.cost_missing, s.cost_over_bound), (0, 0, 0), "{name}");
        assert_eq!(s.calls, s.verified_ok, "{name}: {s:?}");
        assert_mean_cycles(&s, cost, name);
    }
    assert_eq!(stats(&m, "outer").stack_switched, 0, "the trap push is no stack switch");
}

/// Runs the pure `original` from sync event to sync event next to `hooked`; returns the first
/// event at which they differ (event, frame, NMI count, master cycle, registers, WRAM except the
/// dead stack bytes of the main loop), if any.
fn first_difference(original: &mut Machine, hooked: &mut Machine) -> Option<String> {
    let dead_stack = STACK_FLOOR..=usize::from(MAIN_STACK);
    for event in 0..EVENTS {
        let next = |m: &mut Machine| {
            m.run_until_sync(&mut |_| Joypad::default(), &mut NullAudio, 3)
                .unwrap()
                .expect("an event every frame")
        };
        let (a, b) = (next(original), next(hooked));
        if a != b {
            return Some(format!("event {event}: {a:?} vs {b:?}"));
        }
        if let Some(difference) = state_difference(original, hooked, &dead_stack) {
            return Some(format!("event {event}: {difference}"));
        }
    }
    None
}

/// How `hooked` differs from `original`: registers, or a WRAM byte outside `dead_stack`.
fn state_difference(
    original: &Machine,
    hooked: &Machine,
    dead_stack: &std::ops::RangeInclusive<usize>,
) -> Option<String> {
    let (a, b) = (original.cpu_registers(), hooked.cpu_registers());
    let registers = |r: &ssd_oracle::Registers| {
        (r.a, r.x, r.y, r.s, r.d, r.pbr, r.pc, r.dbr, u8::from(r.p), r.emulation_mode)
    };
    if registers(&a) != registers(&b) {
        return Some(format!("registers {a:?} vs {b:?}"));
    }
    (0..original.wram().len())
        .find(|i| original.wram()[*i] != hooked.wram()[*i] && !dead_stack.contains(i))
        .map(|i| format!("WRAM ${i:05X} differs"))
}

/// A machine on [`common::call_rom`] with the NMI on, its main loop as the sync point, and
/// `entries`.
fn loop_machine(entries: impl IntoIterator<Item = HookEntry>) -> Machine {
    let mut m = machine_with(common::call_rom(true));
    let sync_point = HookEntry::sync_point("loop", CALL_LOOP);
    m.set_hook_table(Some(table([sync_point].into_iter().chain(entries))));
    m
}

#[test]
fn rust_mode_with_call_original_stays_on_the_original_timeline() {
    let mut original = loop_machine([]);
    let mut hooked = loop_machine(callers(HookMode::Rust));
    let difference = first_difference(&mut original, &mut hooked);
    let all = hooked.hook_stats();
    assert_eq!(difference, None, "{all:?}");
    for name in ["outer", "flat", "outer_long"] {
        let s = stats(&hooked, name);
        assert!(s.rust_runs > 1000, "{name}: {s:?}");
        assert_eq!((s.cost_missing, s.cost_over_bound), (0, 0), "{name}: {s:?}");
    }
    // Each call with the NMI in its window, callee time included, ran as original.
    let fallbacks: u64 = ["outer", "flat", "outer_long"]
        .into_iter()
        .map(|name| stats(&hooked, name).timing_fallback)
        .sum();
    assert!(fallbacks >= EVENTS as u64 / 2, "{all:?}");
}

#[test]
fn verify_runs_the_called_original_code_on_the_snapshot_only() {
    let mut live = machine_with(common::call_rom(true));
    live.set_hook_table(Some(table(callers(HookMode::Verify))));
    run(&mut live, 3);
    for name in ["outer", "flat", "outer_long"] {
        let s = stats(&live, name);
        assert!(s.verified_ok > 100, "{name}: {s:?}");
        assert_eq!(s.divergences, 0, "{name}: {:?}", live.divergences().first());
    }

    // A pure original run up to the same master cycle is identical: the callee ran on the
    // snapshot, never on the live machine.
    let mut twin = machine_with(common::call_rom(true));
    while twin.total_master_cycles() < live.total_master_cycles() {
        tick(&mut twin);
    }
    assert_eq!(twin.total_master_cycles(), live.total_master_cycles());
    assert!(twin.wram()[CALL_NMI_COUNTER] >= 2, "the NMI ran");
    assert_eq!(live.save_state(), twin.save_state());
}

/// `inner` in Rust, adding 2 instead of 1.
fn inner_adding_two(ctx: &mut Ctx<'_>) {
    let value = ctx.read_dp8(CALL_INNER_COUNTER as u16).wrapping_add(2);
    ctx.write_dp8(CALL_INNER_COUNTER as u16, value);
    ctx.set_nz8(value);
    ctx.set_cost(INNER_COST);
}

/// `inner` in Rust.
fn inner(ctx: &mut Ctx<'_>) {
    let value = ctx.read_dp8(CALL_INNER_COUNTER as u16).wrapping_add(1);
    ctx.write_dp8(CALL_INNER_COUNTER as u16, value);
    ctx.set_nz8(value);
    ctx.set_cost(INNER_COST);
}

/// `outer` in Verify next to `inner` hooked in Verify as `inner_func` with `inner_cycles`, for 2
/// frames.
fn outer_over_hooked_inner(inner_func: RoutineFn, inner_cycles: Option<u64>) -> Machine {
    let mut m = machine_with(common::call_rom(false));
    m.set_hook_table(Some(table([
        routine("outer", CALL_OUTER, HookMode::Verify, outer, Some(OUTER_COST)),
        routine("inner", CALL_INNER, HookMode::Verify, inner_func, inner_cycles),
    ])));
    run(&mut m, 2);
    m
}

/// The counters of `inner` in `m`: the live machine checked it in each call of `outer`'s
/// original only (a nested call); what the snapshots dispatched was not counted. Returns how
/// many of its calls diverged.
fn live_inner_divergences(m: &Machine) -> u64 {
    let (s, outer_calls) = (stats(m, "inner"), stats(m, "outer").calls);
    assert_eq!((s.calls, s.nested), (outer_calls, outer_calls), "{s:?}");
    assert_eq!(s.calls, s.verified_ok + s.divergences, "{s:?}");
    s.divergences
}

#[test]
fn on_the_snapshot_verify_hooks_run_as_rust_and_their_counters_are_discarded() {
    // The wrong inner runs as Rust inside the outer's call on the snapshot: the outer diverges.
    let m = outer_over_hooked_inner(inner_adding_two, Some(INNER_COST));
    let s = stats(&m, "outer");
    assert!(s.divergences > 100, "{s:?}");
    assert_eq!(s.verified_ok, 0, "{s:?}");
    for divergence in m.divergences() {
        let [Difference::Memory { space: "wram", address: 0x10, original, rust }] =
            divergence.differences[..]
        else {
            panic!("only the inner counter differs: {divergence}");
        };
        assert_eq!(rust, original.wrapping_add(1), "{divergence}");
    }
    assert_eq!(live_inner_divergences(&m), s.calls);

    let m = outer_over_hooked_inner(inner, Some(INNER_COST));
    let s = stats(&m, "outer");
    assert!(s.verified_ok > 100, "{s:?}");
    assert_eq!((s.divergences, s.cost_mismatch), (0, 0), "{:?}", m.divergences().first());
    assert_eq!(live_inner_divergences(&m), 0);

    // As in Rust mode, an entry without a maximum cost runs as original on the snapshot: the
    // outer verifies. The nested call on the live machine checks the wrong function.
    let m = outer_over_hooked_inner(inner_adding_two, None);
    let s = stats(&m, "outer");
    assert!(s.verified_ok > 100, "{s:?}");
    assert_eq!(s.divergences, 0, "{:?}", m.divergences().first());
    assert_eq!(live_inner_divergences(&m), s.calls);
}

/// `inner` through `inner_long` (`INC $10; RTL`), which does all its work: a call of original
/// code from a routine that runs inside another one's call. Its cost is 2 master cycles more
/// than `inner`'s (RTL against RTS); it declares none of its own.
fn inner_through_inner_long(ctx: &mut Ctx<'_>) {
    let Ok(()) = ctx.call_original(CALL_INNER_LONG, ReturnKind::Rtl) else { return };
    ctx.set_cost(0);
}

#[test]
fn the_stack_bytes_of_nested_calls_of_original_code_are_dead() {
    // On the snapshot, outer's call runs inner in Rust, whose own call pushes its return to the
    // trap (3 bytes) below outer's: the live original never wrote them, and they are dead.
    let m = outer_over_hooked_inner(inner_through_inner_long, Some(INNER_LONG_COST));
    let s = stats(&m, "outer");
    assert!(s.verified_ok > 100, "{s:?}");
    assert_eq!(
        s.divergences,
        0,
        "{}",
        m.divergences().first().map(ToString::to_string).unwrap_or_default()
    );
    // The nested call is 2 master cycles longer than the original inner.
    assert_eq!(s.cost_mismatch, s.verified_ok, "{s:?}");
}

/// The state at the hit of a call of `outer` whose `inner` returns into the NMI: the CPU takes
/// the NMI right after `inner`'s RTS, where a call from Rust leaves the CPU at the trap.
fn entry_of_an_outer_call_whose_inner_returns_into_the_nmi() -> Vec<u8> {
    let search_machine = || {
        let mut m = machine_with(common::call_rom(true));
        m.set_hook_table(Some(table([
            HookEntry::sync_point("outer", CALL_OUTER),
            HookEntry::sync_point("inner", CALL_INNER),
        ])));
        m
    };
    // Find the call, then reach its entry again on a fresh machine (saving every entry is slow).
    let mut m = search_machine();
    let mut calls = 0;
    let call = loop {
        assert!(m.frame_count() < 300, "no inner returned into the NMI");
        match tick(&mut m) {
            TickEvent::Hook(CALL_OUTER) => calls += 1,
            TickEvent::Hook(CALL_INNER) => {
                let s_entry = m.cpu_registers().s;
                while m.is_mid_instruction() || m.cpu_registers().s <= s_entry {
                    tick(&mut m);
                }
                // Returned. The next CPU cycle (after any refresh) enters the NMI or fetches.
                while !m.is_mid_instruction() {
                    tick(&mut m);
                }
                if m.is_handling_interrupt() {
                    break calls;
                }
            }
            _ => {}
        }
    };
    let mut m = search_machine();
    let mut seen = 0;
    while seen < call {
        if tick(&mut m) == TickEvent::Hook(CALL_OUTER) {
            seen += 1;
        }
    }
    m.save_state()
}

#[test]
fn an_nmi_taken_where_the_callee_returns_to_the_trap_completes_the_call() {
    let entry = entry_of_an_outer_call_whose_inner_returns_into_the_nmi();
    let event_machine = |entries: Vec<HookEntry>| {
        let mut m = machine_with(common::call_rom(true));
        let sync_point = HookEntry::sync_point("return", CALL_OUTER_RETURN);
        m.set_hook_table(Some(table([sync_point].into_iter().chain(entries))));
        // The loaded state is at the hooked fetch: the first tick reports the arrival.
        m.load_state(&entry).unwrap();
        m
    };
    // A maximum cost of 0 (a window of 48 master cycles) lets the call run in Rust although the
    // NMI comes before it would end: the case the bound counter reports.
    let mut hooked =
        event_machine(vec![routine("outer", CALL_OUTER, HookMode::Rust, outer, Some(0))]);
    let mut original = event_machine(Vec::new());
    let nmis = original.nmi_count();

    let next = |m: &mut Machine| {
        m.run_until_sync(&mut |_| Joypad::default(), &mut NullAudio, 2).unwrap().expect("an event")
    };
    let (a, b) = (next(&mut original), next(&mut hooked));
    let sooner = SyncEvent { master_cycles: a.master_cycles - INTERRUPT_AT_THE_TRAP, ..a };
    assert_eq!(b, sooner, "the same arrival after the call, the NMI entry at the trap sooner");
    assert_eq!(b.nmi_count, nmis + 1, "the NMI came during the call");
    let s = stats(&hooked, "outer");
    assert_eq!((s.rust_runs, s.timing_fallback, s.cost_over_bound), (1, 0, 1), "{s:?}");
    // The handler ran between inner's return and the INC $11 after it, as in the original.
    let wram = hooked.wram();
    assert_eq!(wram[CALL_NMI_COPY], wram[CALL_OUTER_COUNTER].wrapping_sub(1));
    let dead_stack = STACK_FLOOR..=usize::from(MAIN_STACK);
    assert_eq!(state_difference(&original, &hooked, &dead_stack), None);
}

/// `outer` calling the routine that never returns.
fn outer_calling_the_spin(ctx: &mut Ctx<'_>) {
    let result = ctx.call_original(CALL_SPIN, ReturnKind::Rts);
    assert!(matches!(result, Err(OracleError::Run(_))), "{result:?}");
    ctx.set_cost(0);
}

#[test]
fn a_callee_that_does_not_return_within_2_frames_is_an_error() {
    let start = Instant::now();
    let mut m = machine_with(common::call_rom(false));
    let spin = routine("outer", CALL_OUTER, HookMode::Rust, outer_calling_the_spin, Some(500));
    m.set_hook_table(Some(table([spin])));
    let frames = m.frame_count();
    let error = m.run_frame(Joypad::default(), &mut NullAudio).unwrap_err();
    let OracleError::Run(message) = &error else { panic!("{error:?}") };
    assert!(message.contains("outer") && message.contains("$008070"), "{message}");
    assert!(message.contains("2 frames"), "{message}");
    assert!(m.frame_count() >= frames + 2, "the budget ran");
    // The return was performed from an instruction boundary: the CPU is back in the main loop.
    assert_eq!(u32::from(m.cpu_registers().pc), CALL_OUTER_RETURN & 0xFFFF);
    assert!(!m.is_mid_instruction(), "the spin's instruction in progress was finished");
    assert!(start.elapsed() < Duration::from_secs(5), "{:?}", start.elapsed());
}

#[test]
fn a_call_longer_than_its_maximum_cost_is_over_bound() {
    // The own cost (126) fits in 150; with inner's time (80) the call does not.
    for mode in [HookMode::Rust, HookMode::Verify] {
        let mut m = machine_with(common::call_rom(false));
        m.set_hook_table(Some(table([routine("outer", CALL_OUTER, mode, outer, Some(150))])));
        run(&mut m, 2);
        let s = stats(&m, "outer");
        let calls = if mode == HookMode::Rust { s.rust_runs } else { s.verified_ok };
        assert!(calls > 100, "{mode:?}: {s:?}");
        assert_eq!(s.cost_over_bound, calls, "{mode:?}: {s:?}");
        // No divergence for it: a correct routine verifies.
        assert_eq!((s.divergences, s.cost_mismatch), (0, 0), "{mode:?}: {s:?}");
        assert_eq!(m.divergences(), []);
    }
}

/// A machine on [`common::call_rom`] at the main loop, about to run code in WRAM that jumps to the
/// trap address of bank `$7E`: `JMP $21FF` at `$7E:21FC`, then `INC $14; JML $008016` (back to
/// the main loop) at `$7E:21FF`.
fn machine_jumping_to_a_trap_address() -> Machine {
    let mut m = machine_with(common::call_rom(false));
    let table = table([HookEntry::sync_point("loop", CALL_LOOP)]);
    assert!(table.is_hooked(0x7E_21FF) && table.is_hooked(0x80_21FF), "trap keys");
    m.set_hook_table(Some(table));
    run(&mut m, 1);
    while tick(&mut m) != TickEvent::Hook(CALL_LOOP) {}
    let program: [u8; 9] = [0x4C, 0xFF, 0x21, 0xE6, 0x14, 0x5C, 0x16, 0x80, 0x00];
    for (address, byte) in (0x7E_21FC..).zip(program) {
        m.bus_write(address, byte);
    }
    let mut registers = m.cpu_registers();
    (registers.pbr, registers.pc) = (0x7E, 0x21FC);
    m.set_cpu_registers(registers);
    m
}

#[test]
fn a_trap_hit_with_no_call_in_flight_resumes_the_original() {
    // The arrival is reported, as at any hooked address.
    let mut m = machine_jumping_to_a_trap_address();
    let mut ticks = 0;
    while tick(&mut m) != TickEvent::Hook(0x7E_21FF) {
        ticks += 1;
        assert!(ticks < 100, "the arrival at the trap address is reported");
    }

    // Dispatched with no call in flight, it resumes the original.
    let mut m = machine_jumping_to_a_trap_address();
    let (loops, before) = (stats(&m, "loop").calls, m.wram()[0x14]);
    run(&mut m, 2);
    assert_eq!(m.wram()[0x14], before.wrapping_add(1), "the code at the trap address ran once");
    assert!(stats(&m, "loop").calls > loops + 100, "the main loop runs on");
}

// Handing a path back to the routine's own original

/// CPU-busy master cycles of the even path of [`common::two_path_leaf_rom`]'s leaf.
const TWO_PATH_EVEN: u64 = 142;
/// Its odd path.
const TWO_PATH_ODD: u64 = 174;

/// The leaf of [`common::two_path_leaf_rom`] with only its even path in Rust: the odd one runs
/// the leaf's own original code, whose time is all of the call's.
fn two_path_even_only(ctx: &mut Ctx<'_>) {
    let counter = ctx.read_dp8(LEAF_COUNTER as u16);
    if counter & 0x01 != 0 {
        let Ok(()) = ctx.call_original(LEAF, ReturnKind::Rts) else { return };
        ctx.set_cost(0);
        return;
    }
    let next = counter.wrapping_add(1);
    ctx.write_dp8(LEAF_COUNTER as u16, next);
    ctx.set_al(0);
    ctx.set_nz8(next);
    ctx.set_cost(TWO_PATH_EVEN);
}

#[test]
fn a_routine_hands_a_path_back_to_its_own_original() {
    let leaf = |mode| routine("leaf", LEAF, mode, two_path_even_only, Some(TWO_PATH_ODD));
    let mut hooked = machine_with(common::two_path_leaf_rom());
    hooked.set_hook_table(Some(table([leaf(HookMode::Rust)])));
    run(&mut hooked, 5);
    let s = stats(&hooked, "leaf");
    assert!(s.rust_runs > 200, "{s:?}");
    // The arrivals at its own entry from its calls of original code are not calls.
    assert_eq!(s.calls, s.rust_runs, "{s:?}");
    assert_eq!((s.timing_fallback, s.cost_missing, s.cost_over_bound), (0, 0, 0), "{s:?}");
    assert!(hooked.wram()[TWO_PATH_ODD_COUNTER] != 0, "the odd path ran");

    // On the pure original's timeline. The fixture keeps the stack of the reset, `$01FF`: the
    // loop's `JSR` leaves S = `$01FD`, and the trap address that `call_original` pushes there
    // (two bytes, `$01FC` and `$01FD`) is the only difference from the original.
    let mut original = machine_with(common::two_path_leaf_rom());
    while original.total_master_cycles() < hooked.total_master_cycles() {
        tick(&mut original);
    }
    assert_eq!(original.total_master_cycles(), hooked.total_master_cycles());
    let dead_stack = 0x01FC..=0x01FD;
    assert_eq!(state_difference(&original, &hooked, &dead_stack), None);

    let mut m = machine_with(common::two_path_leaf_rom());
    m.set_hook_table(Some(table([leaf(HookMode::Verify)])));
    run(&mut m, 3);
    let s = stats(&m, "leaf");
    assert!(s.verified_ok > 100, "{s:?}");
    let first = m.divergences().first().map(ToString::to_string);
    assert_eq!(s.divergences, 0, "{}", first.unwrap_or_default());
    assert_eq!((s.cost_mismatch, s.cost_over_bound), (0, 0), "{s:?}");
}

// The trap address in banks where it is memory

/// Code in WRAM at `$7E:21FC` that runs through `$7E:21FF`, the trap address of its bank:
/// `INC $14; NOP; INC $15; RTL`, the second `INC` at `$7E:21FF`.
const WRAM_CODE: u32 = 0x7E_21FC;
const WRAM_CODE_BYTES: [u8; 6] = [0xE6, 0x14, 0xEA, 0xE6, 0x15, 0x6B];

/// `flat` after calling [`WRAM_CODE`].
fn flat_after_the_wram_code(ctx: &mut Ctx<'_>) {
    let Ok(()) = ctx.call_original(WRAM_CODE, ReturnKind::Rtl) else { return };
    for counter in [CALL_INNER_COUNTER, CALL_OUTER_COUNTER] {
        let value = ctx.read_dp8(counter as u16).wrapping_add(1);
        ctx.write_dp8(counter as u16, value);
        ctx.set_nz8(value);
    }
    ctx.set_cost(FLAT_COST);
}

#[test]
fn code_running_through_the_trap_address_of_its_bank_goes_on() {
    let mut m = machine_with(common::call_rom(false));
    for (address, byte) in (WRAM_CODE..).zip(WRAM_CODE_BYTES) {
        m.bus_write(address, byte);
    }
    let entry = routine("flat", CALL_FLAT, HookMode::Rust, flat_after_the_wram_code, Some(1000));
    m.set_hook_table(Some(table([entry])));
    let before = (m.wram()[0x14], m.wram()[0x15]);
    run(&mut m, 2);
    let s = stats(&m, "flat");
    assert!(s.rust_runs > 100, "{s:?}");
    // The arrival at $7E:21FF before the RTL is not the return: both INCs ran every call.
    let runs = s.rust_runs as u8;
    assert_eq!(
        (m.wram()[0x14], m.wram()[0x15]),
        (before.0.wrapping_add(runs), before.1.wrapping_add(runs))
    );
    assert_eq!(m.cpu_registers().s & 0xFFF0, MAIN_STACK & 0xFFF0, "the stack is balanced");
}
