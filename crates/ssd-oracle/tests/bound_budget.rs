//! The per-call cost bound (`HookEntry::bound`), the per-routine `Verify` budget
//! (`HookEntry::budget_frames`), the `Verify` window-fit counter and the D and DBR setters of
//! `Ctx`, on the synthetic ROM of [`common::count_rom`].

mod common;

use common::{
    COUNT_LEAF, COUNT_NMIS, DB_LEAF, DB_LEAF_COST, count_bound, count_leaf, count_rom, db_leaf,
};
use ssd_oracle::{
    BoundFn, Ctx, HookAction, HookEntry, HookMode, HookStats, HookTable, Joypad, Machine,
    MemorySaveStore, NullAudio, OracleError, ReturnKind, RoutineFn,
};

/// Counts of at most 2 047 (73 700 master cycles, a fifth of a frame): odd, changing every call.
fn short_counts(nmi: bool) -> Vec<u8> {
    count_rom(nmi, 0, 599, 0x07FF)
}

/// Counts of 24 579 and up (more than 884 000 master cycles, 2.4 frames): a call outruns the
/// default budget.
fn long_counts(nmi: bool) -> Vec<u8> {
    count_rom(nmi, 0x6001, 2, 0x7FFF)
}

fn machine_with(rom: Vec<u8>) -> Machine {
    Machine::new(rom, Box::new(MemorySaveStore::default())).unwrap()
}

fn run(m: &mut Machine, frames: usize) {
    for _ in 0..frames {
        m.run_frame(Joypad::default(), &mut NullAudio).unwrap();
    }
}

/// A routine returning with RTS; contract: 16-bit A, X and Y (the fixture's).
fn entry(
    name: &str,
    key: u32,
    mode: HookMode,
    func: RoutineFn,
    cycles: Option<u64>,
    bound: Option<BoundFn>,
    budget_frames: Option<u64>,
) -> HookEntry {
    HookEntry {
        name: name.to_owned(),
        key,
        mode,
        action: HookAction::Routine { func, ret: ReturnKind::Rts },
        entry_m8: Some(false),
        entry_x8: Some(false),
        flags_mask: 0xFF,
        verify_sample: 1,
        cycles,
        bound,
        budget_frames,
    }
}

fn counting(
    mode: HookMode,
    cycles: Option<u64>,
    bound: Option<BoundFn>,
    budget_frames: Option<u64>,
) -> HookTable {
    [entry("count", COUNT_LEAF, mode, count_leaf, cycles, bound, budget_frames)]
        .into_iter()
        .collect()
}

fn stats(m: &Machine, name: &str) -> HookStats {
    let all = m.hook_stats();
    all.iter()
        .find(|(n, _)| n == name)
        .map(|&(_, s)| s)
        .unwrap_or_else(|| panic!("{name}: {all:?}"))
}

#[allow(clippy::type_complexity)]
fn registers(m: &Machine) -> (u16, u16, u16, u16, u16, u8, u16, u8, u8) {
    let r = m.cpu_registers();
    (r.a, r.x, r.y, r.s, r.d, r.pbr, r.pc, r.dbr, u8::from(r.p))
}

/// Ticks the pure `original` up to `hooked`'s master cycle and asserts that both are in the same
/// state there: registers and WRAM.
fn assert_on_the_same_timeline(original: &mut Machine, hooked: &Machine) {
    assert_on_the_same_timeline_but(original, hooked, 0..0);
}

/// [`assert_on_the_same_timeline`] without the WRAM bytes of `dead` (the stack bytes the original
/// code called from Rust pushed).
fn assert_on_the_same_timeline_but(
    original: &mut Machine,
    hooked: &Machine,
    dead: std::ops::Range<usize>,
) {
    while original.total_master_cycles() < hooked.total_master_cycles() {
        original.tick(Joypad::default(), &mut NullAudio).unwrap();
    }
    assert_eq!(original.total_master_cycles(), hooked.total_master_cycles(), "tick boundaries");
    assert_eq!(registers(original), registers(hooked));
    let wram = |m: &Machine| {
        let mut wram = m.wram().to_vec();
        wram[dead.clone()].fill(0);
        wram
    };
    assert!(wram(original) == wram(hooked), "WRAM differs");
}

/// The fixture's declared costs are the original's, for calls of many different lengths.
#[test]
fn the_count_leaf_costs_what_the_formula_says() {
    let mut m = machine_with(short_counts(false));
    m.set_hook_table(Some(counting(HookMode::Verify, None, Some(count_bound), None)));
    run(&mut m, 8);
    let s = stats(&m, "count");
    assert!(s.verified_ok > 50, "{s:?}");
    assert_eq!((s.divergences, s.cost_mismatch, s.cost_over_bound), (0, 0, 0), "{s:?}");
    assert!(s.mean_cpu_cycles > 10_000.0, "{s:?}");
}

#[test]
fn a_loose_static_maximum_never_opens_the_window_but_a_bound_does() {
    // A sound but loose maximum, longer than a frame: the window never opens.
    let loose = 600_000;
    let mut by_maximum = machine_with(short_counts(true));
    by_maximum.set_hook_table(Some(counting(HookMode::Rust, Some(loose), None, None)));
    run(&mut by_maximum, 6);
    let s = stats(&by_maximum, "count");
    assert!(s.calls > 20, "{s:?}");
    assert_eq!((s.rust_runs, s.timing_fallback), (0, s.calls), "{s:?}");

    // The same machine with the bound: the calls that fit before the next NMI run in Rust, the
    // others fall back; the timeline stays the original's.
    let mut hooked = machine_with(short_counts(true));
    hooked.set_hook_table(Some(counting(HookMode::Rust, Some(loose), Some(count_bound), None)));
    run(&mut hooked, 6);
    let s = stats(&hooked, "count");
    assert!(s.rust_runs > 10, "the bound lets calls run in Rust: {s:?}");
    assert!(s.timing_fallback > 0, "a call that spans the NMI still falls back: {s:?}");
    assert_eq!((s.cost_over_bound, s.cost_missing), (0, 0), "{s:?}");
    let mut original = machine_with(short_counts(true));
    assert_on_the_same_timeline(&mut original, &hooked);
    assert!(original.wram()[COUNT_NMIS] > 4, "the NMI ran");
}

#[test]
fn a_bound_stands_for_the_maximum_when_there_is_none() {
    let mut hooked = machine_with(short_counts(true));
    hooked.set_hook_table(Some(counting(HookMode::Rust, None, Some(count_bound), None)));
    run(&mut hooked, 4);
    let s = stats(&hooked, "count");
    assert!(s.rust_runs > 5, "{s:?}");
    assert_eq!(s.cost_missing, 0, "a bound is a cost: {s:?}");
    let mut original = machine_with(short_counts(true));
    assert_on_the_same_timeline(&mut original, &hooked);
}

/// A bound above a frame: no call fits, whatever it really costs.
fn a_frame_and_more(_: &Ctx<'_>) -> u64 {
    400_000
}

/// Two master cycles short of the cost of the call.
fn one_short(ctx: &Ctx<'_>) -> u64 {
    count_bound(ctx) - 2
}

#[test]
fn the_bound_decides_not_the_actual_cost() {
    let mut m = machine_with(short_counts(true));
    m.set_hook_table(Some(counting(HookMode::Rust, Some(1000), Some(a_frame_and_more), None)));
    run(&mut m, 4);
    let s = stats(&m, "count");
    assert!(s.calls > 10, "{s:?}");
    assert_eq!((s.rust_runs, s.timing_fallback), (0, s.calls), "{s:?}");
}

#[test]
fn a_declared_cost_above_the_bound_is_over_bound_in_both_modes() {
    for mode in [HookMode::Rust, HookMode::Verify] {
        let mut m = machine_with(short_counts(false));
        m.set_hook_table(Some(counting(mode, None, Some(one_short), None)));
        run(&mut m, 8);
        let s = stats(&m, "count");
        let calls = if mode == HookMode::Rust { s.rust_runs } else { s.verified_ok };
        assert!(calls > 30, "{mode:?}: {s:?}");
        assert_eq!(s.cost_over_bound, calls, "{mode:?}: {s:?}");
        assert_eq!(s.divergences, 0, "{mode:?}: {s:?}");
    }
}

#[test]
fn a_routine_without_a_bound_keeps_the_static_maximum() {
    // 100 000 holds for counts up to 2 047 (73 700); 50 000 does not for the larger ones.
    for (maximum, over) in [(100_000, false), (50_000, true)] {
        let mut m = machine_with(short_counts(false));
        m.set_hook_table(Some(counting(HookMode::Verify, Some(maximum), None, None)));
        run(&mut m, 8);
        let s = stats(&m, "count");
        assert!(s.verified_ok > 30, "{s:?}");
        assert_eq!(s.cost_over_bound > 0, over, "{maximum}: {s:?}");
    }
}

#[test]
fn verify_counts_the_calls_whose_real_cost_fits_the_window() {
    let mut m = machine_with(short_counts(true));
    m.set_hook_table(Some(counting(HookMode::Verify, None, Some(count_bound), None)));
    run(&mut m, 10);
    let s = stats(&m, "count");
    let checked = s.verified_ok + s.divergences + s.interrupted + s.no_return;
    assert!(s.window_fit > 5, "{s:?}");
    // Calls that an NMI entered span an event: they do not fit.
    assert!(s.interrupted > 0 && s.window_fit < checked, "{s:?}");
    assert!(s.window_fit <= s.verified_ok + s.divergences, "{s:?}");
    // No NMI: nothing to span, every call fits.
    let mut m = machine_with(short_counts(false));
    m.set_hook_table(Some(counting(HookMode::Verify, None, Some(count_bound), None)));
    run(&mut m, 3);
    let s = stats(&m, "count");
    assert_eq!(s.window_fit, s.verified_ok, "{s:?}");
}

#[test]
fn budget_frames_lets_a_long_verify_call_complete() {
    // Default budget (2 frames): the first call outruns it and the routine is demoted.
    let mut m = machine_with(long_counts(false));
    m.set_hook_table(Some(counting(HookMode::Verify, None, Some(count_bound), None)));
    run(&mut m, 8);
    let s = stats(&m, "count");
    assert_eq!((s.no_return, s.verified_ok), (1, 0), "{s:?}");

    // 4 frames: the calls (2.5 frames each) complete and match, cost included.
    let mut m = machine_with(long_counts(false));
    m.set_hook_table(Some(counting(HookMode::Verify, None, Some(count_bound), Some(4))));
    run(&mut m, 12);
    let s = stats(&m, "count");
    assert!(s.verified_ok >= 3, "{s:?}");
    assert_eq!((s.no_return, s.divergences, s.cost_mismatch, s.cost_over_bound), (0, 0, 0, 0));
    assert!(s.mean_cpu_cycles > 800_000.0, "{s:?}");
    // The calls are longer than a frame, but no timing event is due (NMI off): they all fit.
    assert_eq!(s.window_fit, s.verified_ok, "{s:?}");
}

#[test]
fn rust_mode_still_falls_back_on_a_call_longer_than_the_window() {
    // NMI on: every call spans an NMI, whatever its budget.
    let mut hooked = machine_with(long_counts(true));
    hooked.set_hook_table(Some(counting(HookMode::Rust, None, Some(count_bound), Some(10))));
    run(&mut hooked, 8);
    let s = stats(&hooked, "count");
    assert!(s.calls > 0, "{s:?}");
    assert_eq!((s.rust_runs, s.timing_fallback), (0, s.calls), "{s:?}");
    let mut original = machine_with(long_counts(true));
    assert_on_the_same_timeline(&mut original, &hooked);
}

/// The leaf handed back to its original: the code it calls takes its time.
fn hand_back(ctx: &mut Ctx<'_>) {
    let result = ctx.call_original(COUNT_LEAF, ReturnKind::Rts);
    if let Err(error) = &result {
        assert!(matches!(error, OracleError::Run(_)), "{error:?}");
    }
    ctx.set_cost(0);
}

#[test]
fn budget_frames_covers_the_original_code_a_routine_calls() {
    let table = |budget_frames| -> HookTable {
        [entry(
            "count",
            COUNT_LEAF,
            HookMode::Rust,
            hand_back,
            Some(1_500_000),
            None,
            budget_frames,
        )]
        .into_iter()
        .collect()
    };
    // Default: the original code outruns 2 frames, the dispatch returns the error.
    let mut m = machine_with(long_counts(false));
    m.set_hook_table(Some(table(None)));
    let error = (0..8).find_map(|_| m.run_frame(Joypad::default(), &mut NullAudio).err());
    let Some(OracleError::Run(message)) = error else { panic!("{error:?}") };
    assert!(message.contains("count") && message.contains("2 frames"), "{message}");

    // 4 frames: it returns; the time of the called code is its own (cost 0 + callee).
    let mut m = machine_with(long_counts(false));
    m.set_hook_table(Some(table(Some(4))));
    run(&mut m, 8);
    let s = stats(&m, "count");
    assert!(s.rust_runs >= 2, "{s:?}");
    assert_eq!(s.cost_over_bound, 0, "{s:?}");
    let mut original = machine_with(long_counts(false));
    assert_on_the_same_timeline_but(&mut original, &m, 0x1F00..0x2000);
}

fn db_entry(mode: HookMode, cycles: Option<u64>) -> HookTable {
    [entry("db", DB_LEAF, mode, db_leaf, cycles, None, None)].into_iter().collect()
}

#[test]
fn the_dbr_and_d_setters_leave_the_cpu_as_the_original_does() {
    // Verify compares A, D, DBR and the flags after the original with the routine's.
    let mut m = machine_with(short_counts(false));
    m.set_hook_table(Some(db_entry(HookMode::Verify, None)));
    run(&mut m, 4);
    let s = stats(&m, "db");
    assert!(s.verified_ok > 20, "{s:?}");
    assert_eq!((s.divergences, s.cost_mismatch, s.cost_over_bound), (0, 0, 0), "{s:?}");
    assert_eq!(DB_LEAF_COST, s.mean_cpu_cycles as u64, "{s:?}");

    // In Rust mode the machine stays on the original's timeline, D and DBR included.
    let mut hooked = machine_with(short_counts(true));
    hooked.set_hook_table(Some(db_entry(HookMode::Rust, Some(DB_LEAF_COST))));
    run(&mut hooked, 3);
    let s = stats(&hooked, "db");
    assert!(s.rust_runs > 10, "{s:?}");
    // (The original's `PEA` pushed two bytes the Rust routine does not: dead stack.)
    let mut original = machine_with(short_counts(true));
    assert_on_the_same_timeline_but(&mut original, &hooked, 0x1F00..0x2000);
    assert_eq!(hooked.cpu_registers().dbr, 0x7E);
}

/// Reads D and DBR back after setting them, from a Rust routine.
fn set_and_read_back(ctx: &mut Ctx<'_>) {
    ctx.set_dbr(0x7F);
    ctx.set_d(0x1234);
    assert_eq!((ctx.dbr(), ctx.d()), (0x7F, 0x1234));
    // The setters move the accesses that go through the registers.
    ctx.write_db8(0x0020, 0xA5);
    assert_eq!(ctx.read8(0x7F_0020), 0xA5);
    ctx.write_dp8(0x0001, 0x5A);
    assert_eq!(ctx.read8(0x00_1235), 0x5A);
    assert_eq!(ctx.peek_dp8(0x0001), 0x5A);
    assert_eq!(ctx.peek_db8(0x0020), 0xA5);
    // Only the named register changes.
    ctx.set_dbr(0x00);
    assert_eq!((ctx.dbr(), ctx.d()), (0x00, 0x1234));
    ctx.set_d(0x0000);
    ctx.set_cost(0);
}

#[test]
fn the_setters_round_trip_and_move_the_register_relative_accesses() {
    let mut m = machine_with(short_counts(false));
    let table: HookTable =
        [entry("rt", DB_LEAF, HookMode::Rust, set_and_read_back, Some(1000), None, None)]
            .into_iter()
            .collect();
    m.set_hook_table(Some(table));
    run(&mut m, 1);
    let s = stats(&m, "rt");
    assert!(s.rust_runs > 0, "{s:?}");
}
