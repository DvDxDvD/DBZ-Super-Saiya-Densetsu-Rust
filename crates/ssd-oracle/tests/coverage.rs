//! The coverage metric: the instructions of the original per call (`HookStats::mean_instructions`,
//! measured by `Verify`) and the share of the executed instructions that run as Rust
//! (`Machine::coverage`).

mod common;

use common::{
    LEAF, LEAF_COUNTER, NESTED_INNER, NESTED_INNER_COUNTER, NESTED_OUTER, NESTED_OUTER_COUNTER,
    NESTED_OUTER_TAIL, NESTED_WRITES, NMI_CALL_LONG, NMI_CALL_LONG_COUNTER, NMI_CALL_NMI_COUNTER,
    NMI_CALL_SHORT, NMI_CALL_SHORT_COUNTER, TWO_PATH_ODD_COUNTER,
};
use ssd_oracle::{
    Coverage, Ctx, HookAction, HookEntry, HookMode, HookStats, HookTable, Joypad, Machine,
    MemorySaveStore, NullAudio, ReturnKind, RoutineFn,
};

/// Master cycles of a frame (262 lines of 1364).
const FRAME_MCLK: u64 = 262 * 1364;
/// Instructions of the leaf of [`common::jsr_leaf_rom`]: `INC $10; RTS`.
const LEAF_INSTRUCTIONS: f64 = 2.0;
/// Master cycles of the leaf (`INC dp` 38, `RTS` 42), its declared cost.
const LEAF_COST: u64 = 80;

fn machine_with(rom: Vec<u8>) -> Machine {
    Machine::new(rom, Box::new(MemorySaveStore::default())).unwrap()
}

fn run(m: &mut Machine, frames: usize) {
    for _ in 0..frames {
        m.run_frame(Joypad::default(), &mut NullAudio).unwrap();
    }
}

/// A routine returning with RTS, contract 8-bit A and 16-bit X/Y (the fixtures').
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

/// A mean of equal counts is exact; the tolerance only keeps the comparison float-safe.
fn assert_mean(actual: f64, expected: f64, what: &str) {
    assert!((actual - expected).abs() < 1e-9, "{what}: mean {actual}, expected {expected}");
}

/// The first divergence kept, as text.
fn first_divergence(m: &Machine) -> String {
    m.divergences().first().map(ToString::to_string).unwrap_or_default()
}

// The routines, in Rust

/// The leaf `INC $10` (8-bit memory).
fn leaf(ctx: &mut Ctx<'_>) {
    let value = ctx.read_dp8(LEAF_COUNTER as u16).wrapping_add(1);
    ctx.write_dp8(LEAF_COUNTER as u16, value);
    ctx.set_nz8(value);
    ctx.set_cost(LEAF_COST);
}

/// `inner` of [`common::nested_rom`]: X is kept (pushed and pulled), so N and Z come from it.
fn inner_work(ctx: &mut Ctx<'_>) {
    let value = ctx.read_dp8(NESTED_INNER_COUNTER as u16).wrapping_add(1);
    ctx.write_dp8(NESTED_INNER_COUNTER as u16, value);
    let [_, (coldata, color), _] = NESTED_WRITES;
    ctx.write8(coldata, color);
    ctx.set_al(color);
    ctx.set_y(0);
    let x = ctx.x();
    ctx.set_nz16(x);
}

/// `INC $11`, the last instruction of the callers before they return.
fn inc_outer_counter(ctx: &mut Ctx<'_>) {
    let value = ctx.read_dp8(NESTED_OUTER_COUNTER as u16).wrapping_add(1);
    ctx.write_dp8(NESTED_OUTER_COUNTER as u16, value);
    ctx.set_nz8(value);
}

/// `outer` with `inner` inlined.
fn outer(ctx: &mut Ctx<'_>) {
    let [(inidisp, brightness), .., (coldata, off)] = NESTED_WRITES;
    ctx.write8(inidisp, brightness);
    inner_work(ctx);
    ctx.write8(coldata, off);
    inc_outer_counter(ctx);
}

/// `outer_tail` with `inner` inlined.
fn outer_tail(ctx: &mut Ctx<'_>) {
    inc_outer_counter(ctx);
    inner_work(ctx);
}

/// The long routine of [`common::nmi_call_rom`]: X ends at 0, with N and Z from it.
fn long_routine(ctx: &mut Ctx<'_>) {
    let value = ctx.read_dp8(NMI_CALL_LONG_COUNTER as u16).wrapping_add(1);
    ctx.write_dp8(NMI_CALL_LONG_COUNTER as u16, value);
    ctx.set_x(0);
    ctx.set_nz16(0);
}

/// The short routine of [`common::nmi_call_rom`]: `INC $10; RTS`.
fn short_routine(ctx: &mut Ctx<'_>) {
    let value = ctx.read_dp8(NMI_CALL_SHORT_COUNTER as u16).wrapping_add(1);
    ctx.write_dp8(NMI_CALL_SHORT_COUNTER as u16, value);
    ctx.set_nz8(value);
}

/// The leaf of [`common::two_path_leaf_rom`]: `INC $11` on an odd counter, `INC $10`; A = the old
/// bit 0, N/Z from the new counter.
fn two_path_leaf(ctx: &mut Ctx<'_>) {
    let counter = ctx.read_dp8(LEAF_COUNTER as u16);
    if counter & 1 != 0 {
        let odd = ctx.read_dp8(TWO_PATH_ODD_COUNTER as u16).wrapping_add(1);
        ctx.write_dp8(TWO_PATH_ODD_COUNTER as u16, odd);
    }
    ctx.write_dp8(LEAF_COUNTER as u16, counter.wrapping_add(1));
    ctx.set_al(counter & 1);
    ctx.set_nz8(counter.wrapping_add(1));
}

// The mean instructions of a call

#[test]
fn a_leaf_of_two_instructions_has_a_mean_of_two() {
    let mut m = machine_with(common::jsr_leaf_rom());
    m.set_hook_table(Some(table([routine("leaf", LEAF, HookMode::Verify, leaf, Some(80))])));
    assert_eq!(stats(&m, "leaf").mean_instructions, 0.0, "nothing measured yet");
    run(&mut m, 4);

    let s = stats(&m, "leaf");
    assert!(s.verified_ok > 100, "{s:?}");
    assert_eq!((s.calls, s.divergences), (s.verified_ok, 0), "{}", first_divergence(&m));
    assert_mean(s.mean_instructions, LEAF_INSTRUCTIONS, "leaf");
}

#[test]
fn a_diverged_call_counts_like_a_verified_one() {
    // Instructions are those of the original, whatever the Rust function does.
    fn wrong(ctx: &mut Ctx<'_>) {
        ctx.write_dp8(LEAF_COUNTER as u16, 0x55);
    }
    let mut m = machine_with(common::jsr_leaf_rom());
    m.set_hook_table(Some(table([routine("leaf", LEAF, HookMode::Verify, wrong, Some(80))])));
    run(&mut m, 2);

    let s = stats(&m, "leaf");
    assert!(s.divergences > 50, "{s:?}");
    assert_mean(s.mean_instructions, LEAF_INSTRUCTIONS, "leaf");
}

#[test]
fn only_the_calls_verify_checks_are_measured() {
    // The leaf of `two_path_leaf_rom` alternates between its even path (LDA, AND, BEQ taken,
    // INC, RTS = 5 instructions) and its odd one (LDA, AND, BEQ, INC, INC, RTS = 6).
    let mean_of = |verify_sample: u32| {
        let mut entry = routine("leaf", LEAF, HookMode::Verify, two_path_leaf, None);
        entry.verify_sample = verify_sample;
        let mut m = machine_with(common::two_path_leaf_rom());
        m.set_hook_table(Some(table([entry])));
        run(&mut m, 12);
        let s = stats(&m, "leaf");
        assert_eq!(s.divergences, 0, "{}", first_divergence(&m));
        s
    };

    // Every call: both paths, in turn.
    let every = mean_of(1);
    assert!(every.verified_ok > 400 && every.sampled_out == 0, "{every:?}");
    assert!((5.45..5.55).contains(&every.mean_instructions), "{every:?}");
    // One call in two is checked, and the calls alternate: only the even path is ever measured.
    let half = mean_of(2);
    assert!(half.sampled_out > 200 && half.verified_ok > 200, "{half:?}");
    assert_mean(half.mean_instructions, 5.0, "every other call");
    // One in three meets both paths in turn.
    let third = mean_of(3);
    assert!(third.sampled_out > 300 && third.verified_ok > 150, "{third:?}");
    assert!((5.4..5.6).contains(&third.mean_instructions), "{third:?}");
}

#[test]
fn a_nested_call_counts_toward_the_original_instructions_of_its_caller() {
    // `inner`: PHX, INC, LDA, STA, LDY, 16 x (DEY, BNE), PLX, RTS = 39.
    // `outer`: LDA, STA, JSR, inner, STZ, INC, RTS = 6 + 39. `outer_tail`: INC, JMP, inner = 2 + 39.
    const INNER: f64 = 39.0;
    let mut m = machine_with(common::nested_rom(false));
    m.set_hook_table(Some(table([
        routine("outer", NESTED_OUTER, HookMode::Verify, outer, None),
        routine("outer_tail", NESTED_OUTER_TAIL, HookMode::Verify, outer_tail, None),
        routine("inner", NESTED_INNER, HookMode::Verify, inner_work, None),
    ])));
    run(&mut m, 4);

    for (name, instructions) in
        [("outer", INNER + 6.0), ("outer_tail", INNER + 2.0), ("inner", INNER)]
    {
        let s = stats(&m, name);
        assert!(s.verified_ok > 100, "{name}: {s:?}");
        assert_eq!(s.divergences, 0, "{name}: {}", first_divergence(&m));
        assert_mean(s.mean_instructions, instructions, name);
    }
}

#[test]
fn an_interrupt_inside_a_nested_call_keeps_its_callers_out_of_the_means_too() {
    // The routines above with the NMI on. The handler (PHA, INC, LDA, PLA, RTI) lands inside
    // `inner`, which takes most of each iteration, and so inside `outer` or `outer_tail` around
    // it: those calls are interrupted, not compared, and add nothing to a mean. A mean that
    // took them in would hold the handler's five instructions.
    const INNER: f64 = 39.0;
    let mut m = machine_with(common::nested_rom(true));
    m.set_hook_table(Some(table([
        routine("outer", NESTED_OUTER, HookMode::Verify, outer, None),
        routine("outer_tail", NESTED_OUTER_TAIL, HookMode::Verify, outer_tail, None),
        routine("inner", NESTED_INNER, HookMode::Verify, inner_work, None),
    ])));
    run(&mut m, 12);

    let (outer_stats, tail, inner) =
        (stats(&m, "outer"), stats(&m, "outer_tail"), stats(&m, "inner"));
    assert!(inner.interrupted >= 5, "most NMIs land in inner: {inner:?}");
    assert!(
        outer_stats.interrupted + tail.interrupted >= inner.interrupted,
        "an interrupted nested call interrupts its caller: {:?}",
        m.hook_stats()
    );
    for (name, instructions) in
        [("outer", INNER + 6.0), ("outer_tail", INNER + 2.0), ("inner", INNER)]
    {
        let s = stats(&m, name);
        assert!(s.verified_ok > 100, "{name}: {s:?}");
        assert_eq!(s.divergences, 0, "{name}: {}", first_divergence(&m));
        assert_mean(s.mean_instructions, instructions, name);
    }
}

#[test]
fn the_instructions_of_an_interrupt_handler_are_not_in_the_mean() {
    // The long routine: INC, LDX, 256 x (DEX, BNE), RTS = 515. The NMI lands inside it about once
    // a frame; the handler (PHA, PHX, JSR short, INC, LDA, PLX, PLA, RTI and the short routine)
    // would add its instructions to those calls, which are not compared.
    let mut m = machine_with(common::nmi_call_rom(NMI_CALL_SHORT));
    m.set_hook_table(Some(table([
        routine("long", NMI_CALL_LONG, HookMode::Verify, long_routine, None),
        routine("short", NMI_CALL_SHORT, HookMode::Verify, short_routine, None),
    ])));
    let nmis = m.wram()[NMI_CALL_NMI_COUNTER];
    run(&mut m, 8);
    assert!(m.wram()[NMI_CALL_NMI_COUNTER].wrapping_sub(nmis) >= 6, "the NMI ran");

    let long = stats(&m, "long");
    assert!(long.interrupted >= 6 && long.verified_ok > 100, "{long:?}");
    assert_eq!(long.divergences, 0, "{}", first_divergence(&m));
    assert_mean(long.mean_instructions, 515.0, "long");
    // The handler's own call is nested in an interrupted one, and compared.
    let short = stats(&m, "short");
    assert_eq!(short.nested, long.interrupted, "{short:?}");
    assert_mean(short.mean_instructions, LEAF_INSTRUCTIONS, "short");
}

#[test]
fn a_call_that_never_got_compared_has_no_mean() {
    let mut m = machine_with(common::jsr_leaf_rom());
    let mut entry = routine("leaf", LEAF, HookMode::Verify, leaf, Some(80));
    entry.entry_m8 = Some(false); // the fixture's A is 8-bit: the contract never holds
    m.set_hook_table(Some(table([entry])));
    run(&mut m, 2);

    let s = stats(&m, "leaf");
    assert!(s.contract_mismatch > 50, "{s:?}");
    assert_eq!((s.verified_ok, s.mean_instructions), (0, 0.0), "{s:?}");
}

// The coverage

#[test]
fn the_percent_is_the_rust_share_of_all_the_instructions() {
    let share = |original_instructions, rust_instructions_estimated| {
        Coverage { original_instructions, rust_instructions_estimated }.percent()
    };
    assert_mean(share(900, 100), 10.0, "100 of 1000");
    assert_mean(share(1, 3), 75.0, "3 of 4");
    assert_mean(share(1000, 0), 0.0, "nothing in Rust");
    assert_mean(share(0, 5), 100.0, "everything in Rust");
    assert_mean(share(0, 0), 0.0, "no instruction at all");
}

#[test]
fn excluding_routines_takes_their_instructions_out_of_the_rust_share_only() {
    let all = Coverage { original_instructions: 900, rust_instructions_estimated: 100 };
    let without = all.excluding(40);
    assert_eq!(without.original_instructions, 900, "the original instructions stay");
    assert_eq!(without.rust_instructions_estimated, 60);
    assert_mean(without.percent(), 100.0 * 60.0 / 960.0, "60 of 960");
    assert_mean(all.excluding(0).percent(), all.percent(), "nothing excluded");
    let none_left = all.excluding(500);
    assert_eq!(none_left.rust_instructions_estimated, 0, "never below zero");
    assert_mean(none_left.percent(), 0.0, "everything excluded");
}

/// The means a `Verify` machine learned on the leaf, and a `Rust` machine of the leaf with them.
fn rust_leaf_machine() -> Machine {
    let mut verify = machine_with(common::jsr_leaf_rom());
    verify.set_hook_table(Some(table([routine(
        "leaf",
        LEAF,
        HookMode::Verify,
        leaf,
        Some(LEAF_COST),
    )])));
    run(&mut verify, 2);
    let learned: Vec<(String, f64)> =
        verify.hook_stats().into_iter().map(|(name, s)| (name, s.mean_instructions)).collect();
    assert_eq!(learned.len(), 1);

    let mut m = machine_with(common::jsr_leaf_rom());
    m.set_hook_table(Some(table([routine("leaf", LEAF, HookMode::Rust, leaf, Some(LEAF_COST))])));
    m.set_mean_instructions(&learned);
    m
}

#[test]
fn coverage_counts_each_rust_run_at_the_learned_mean() {
    let mut m = rust_leaf_machine();
    run(&mut m, 4);

    let s = stats(&m, "leaf");
    assert!(s.rust_runs > 100, "{s:?}");
    assert_eq!(s.timing_fallback, 0, "no timing event in this ROM: {s:?}");
    assert_mean(s.mean_instructions, LEAF_INSTRUCTIONS, "leaf");
    assert_eq!(s.estimated_rust_instructions(), 2 * s.rust_runs);

    // What the CPU fetched is what ran as original code; the Rust runs are 2 instructions each.
    let coverage = m.coverage();
    assert_eq!(coverage.original_instructions, m.opcode_fetches());
    assert_eq!(coverage.rust_instructions_estimated, 2 * s.rust_runs);
    // Hand-computed from the counters: rust / (rust + original), in percent.
    let (rust, original) = (2.0 * s.rust_runs as f64, m.opcode_fetches() as f64);
    assert_mean(coverage.percent(), 100.0 * rust / (rust + original), "percent");
    // Each pass of the loop is JSR, LDX, 200 x (DEX, BNE), BRA = 403 original instructions around
    // the 2 of the leaf: about 2 / 405 of the total.
    assert!((0.45..0.5).contains(&coverage.percent()), "{}", coverage.percent());
}

#[test]
fn the_estimate_adds_up_to_what_the_original_would_have_executed() {
    // The Rust machine stays on the original's timeline, and a run ends between two instructions
    // of it: original + Rust instructions are the pure machine's at the same master cycle, a
    // constant leaf counted exactly.
    let mut m = rust_leaf_machine();
    let mut pure = machine_with(common::jsr_leaf_rom());
    for frame in 1..=8 {
        run(&mut m, 1);
        while pure.total_master_cycles() < m.total_master_cycles() {
            pure.tick(Joypad::default(), &mut NullAudio).unwrap();
        }
        assert_eq!(pure.total_master_cycles(), m.total_master_cycles(), "frame {frame}");

        let coverage = m.coverage();
        let total = coverage.original_instructions + coverage.rust_instructions_estimated;
        assert_eq!(total, pure.opcode_fetches(), "frame {frame}: {coverage:?}");
    }
    assert!(m.coverage().rust_instructions_estimated > 400);
}

#[test]
fn a_rust_routine_without_a_learned_mean_adds_nothing() {
    let mut m = machine_with(common::jsr_leaf_rom());
    m.set_hook_table(Some(table([routine("leaf", LEAF, HookMode::Rust, leaf, Some(LEAF_COST))])));
    run(&mut m, 2);

    let s = stats(&m, "leaf");
    assert!(s.rust_runs > 50 && s.mean_instructions == 0.0, "{s:?}");
    assert_eq!(m.coverage().rust_instructions_estimated, 0);
    assert_eq!(m.coverage().original_instructions, m.opcode_fetches());
}

#[test]
fn learned_means_go_to_the_routines_of_the_table_by_name_and_stay_with_them() {
    let mut m = machine_with(common::jsr_leaf_rom());
    m.set_hook_table(Some(table([routine("leaf", LEAF, HookMode::Rust, leaf, Some(LEAF_COST))])));
    m.set_mean_instructions(&[("leaf".to_owned(), 2.5), ("no_such_routine".to_owned(), 9.0)]);
    assert_mean(stats(&m, "leaf").mean_instructions, 2.5, "leaf");
    assert_eq!(m.hook_stats().len(), 1, "an unknown name adds no entry");

    // The same routine in another mode keeps its counters, the learned mean included; another
    // routine under the key starts again.
    m.set_hook_table(Some(table([routine("leaf", LEAF, HookMode::Verify, leaf, Some(LEAF_COST))])));
    assert_mean(stats(&m, "leaf").mean_instructions, 2.5, "leaf in Verify");
    m.set_hook_table(Some(table([routine("other", LEAF, HookMode::Rust, leaf, Some(LEAF_COST))])));
    assert_eq!(stats(&m, "other").mean_instructions, 0.0);
}

#[test]
fn a_machine_without_hooks_has_no_rust_instructions() {
    let mut m = machine_with(common::jsr_leaf_rom());
    run(&mut m, 2);
    let coverage = m.coverage();
    assert_eq!(coverage.rust_instructions_estimated, 0);
    assert_eq!(coverage.original_instructions, m.opcode_fetches());
    assert!(coverage.original_instructions > 10_000);
    assert_mean(coverage.percent(), 0.0, "percent");
}

// Per-frame statistics

#[test]
fn per_frame_statistics_of_a_rust_leaf_are_its_runs_times_the_mean() {
    let mut m = rust_leaf_machine();
    assert_eq!(m.last_frame_stats(), None, "before the first render");
    let (mut sums, mut last_runs) = (Coverage::default(), 0);
    let mut dispatches = 0;
    for frame in 0..30 {
        run(&mut m, 1);
        let f = m.last_frame_stats().expect("a frame was rendered");
        assert_eq!(f.frame, frame);
        let runs = stats(&m, "leaf").rust_runs;
        // A run is counted before its dispatch holds the CPU, which can render the frame: a run
        // across the render is in the frame it started in, and the sums never dip.
        let in_frame = f.rust_instructions_estimated / 2;
        assert_eq!(f.rust_instructions_estimated % 2, 0);
        assert_eq!(in_frame + last_runs, runs, "frame {frame}");
        last_runs += in_frame;
        if frame > 0 {
            assert!(in_frame > 10 && f.original_instructions > 1000, "{f:?}");
            let expected = 100.0 * f.rust_instructions_estimated as f64
                / (f.rust_instructions_estimated + f.original_instructions) as f64;
            assert_mean(f.coverage_percent(), expected, "frame percent");
        }
        sums.original_instructions += f.original_instructions;
        sums.rust_instructions_estimated += f.rust_instructions_estimated;
        dispatches += f.dispatches;
    }
    // Counted at the arrival, so a dispatch across the render is in the frame it started in.
    assert_eq!(dispatches, stats(&m, "leaf").calls);
    // Whole frames: the sums are the machine's coverage at the last render.
    assert_eq!(sums.original_instructions, m.coverage().original_instructions);
    assert_eq!(sums.rust_instructions_estimated, m.coverage().rust_instructions_estimated);
}

#[test]
fn per_frame_cycles_nmis_and_dispatches_add_up_to_the_machines() {
    let mut m = machine_with(common::nmi_call_rom(NMI_CALL_SHORT));
    m.set_hook_table(Some(table([
        routine("long", NMI_CALL_LONG, HookMode::Verify, long_routine, None),
        routine("short", NMI_CALL_SHORT, HookMode::Verify, short_routine, None),
    ])));
    let (mut cycles, mut nmis, mut dispatches) = (0, 0, 0);
    for frame in 0..10 {
        run(&mut m, 1);
        let f = m.last_frame_stats().unwrap();
        assert!(f.nmis <= 2 && f.dispatches > 0, "{f:?}");
        assert!(frame == 0 || f.master_cycles.abs_diff(FRAME_MCLK) < 100, "{f:?}");
        (cycles, nmis, dispatches) =
            (cycles + f.master_cycles, nmis + f.nmis, dispatches + f.dispatches);
    }
    // The tick or dispatch that rendered the last frame ended after the render: the machine's
    // totals run a little past the sums.
    assert!((0..20_000).contains(&(m.total_master_cycles() - cycles)));
    assert!((0..=1).contains(&(m.nmi_count() - nmis)));
    let calls: u64 = m.hook_stats().iter().map(|(_, s)| s.calls).sum();
    // The calls after the last render are not in the sums.
    assert!(calls > 100 && (0..=3).contains(&(calls - dispatches)), "{dispatches} {calls}");
}

#[test]
fn a_loaded_state_starts_the_statistics_again() {
    let mut m = rust_leaf_machine();
    run(&mut m, 3);
    let state = m.save_state();
    run(&mut m, 3);
    let before = m.last_frame_stats().unwrap();
    m.load_state(&state).unwrap();
    assert_eq!(m.last_frame_stats(), None);
    run(&mut m, 1);
    let after = m.last_frame_stats().unwrap();
    assert_eq!(after.frame, 3);
    // An ordinary frame, not the difference of counters across the load.
    assert!(after.original_instructions.abs_diff(before.original_instructions) < 100, "{after:?}");
    assert!(after.master_cycles.abs_diff(before.master_cycles) < 1000, "{after:?}");
}

// Original code a Rust routine runs

// CPU-busy master cycles in `nested_rom` (8-cycle `SlowROM` and WRAM accesses, PPU registers and
// internal cycles 6), as in dispatch_nested.rs.
const LDA_IMM: u64 = 16;
const STA_PPU: u64 = 30;
const JSR: u64 = 46;
const JMP: u64 = 24;
const INC_DP: u64 = 38;
const RTS: u64 = 42;
const PHX: u64 = 30;
const PLX: u64 = 36;
const LDY_IMM: u64 = 24;
const DEY: u64 = 14;
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

fn inner_costed(ctx: &mut Ctx<'_>) {
    inner_work(ctx);
    ctx.set_cost(INNER_COST);
}

fn outer_tail_costed(ctx: &mut Ctx<'_>) {
    outer_tail(ctx);
    ctx.set_cost(OUTER_TAIL_COST);
}

/// `outer` running `inner` as original code (hooked or not), whose time is measured and added.
fn outer_calling_inner(ctx: &mut Ctx<'_>) {
    let [(inidisp, brightness), .., (coldata, off)] = NESTED_WRITES;
    ctx.write8(inidisp, brightness);
    ctx.charge(LDA_IMM + STA_PPU + JSR);
    let Ok(()) = ctx.call_original(NESTED_INNER, ReturnKind::Rts) else { return };
    ctx.write8(coldata, off);
    inc_outer_counter(ctx);
    ctx.set_cost(OUTER_COST - INNER_COST);
}

/// A `Rust` machine on `nested_rom` with `outer` calling `inner` as original code, and the means
/// a `Verify` machine learned with all three routines inline: `outer` 45, `outer_tail` 41,
/// `inner` 39. `inner` is a routine of the table in `Rust` mode too if `inner_in_rust`, else
/// plain original code.
fn nested_rust_machine(inner_in_rust: bool) -> Machine {
    let mut verify = machine_with(common::nested_rom(false));
    verify.set_hook_table(Some(table([
        routine("outer", NESTED_OUTER, HookMode::Verify, outer, None),
        routine("outer_tail", NESTED_OUTER_TAIL, HookMode::Verify, outer_tail, None),
        routine("inner", NESTED_INNER, HookMode::Verify, inner_work, None),
    ])));
    run(&mut verify, 2);
    let learned: Vec<(String, f64)> =
        verify.hook_stats().into_iter().map(|(name, s)| (name, s.mean_instructions)).collect();
    assert_eq!(learned.len(), 3);

    let mut entries = vec![
        routine("outer", NESTED_OUTER, HookMode::Rust, outer_calling_inner, Some(OUTER_COST)),
        routine(
            "outer_tail",
            NESTED_OUTER_TAIL,
            HookMode::Rust,
            outer_tail_costed,
            Some(OUTER_TAIL_COST),
        ),
    ];
    if inner_in_rust {
        entries.push(routine(
            "inner",
            NESTED_INNER,
            HookMode::Rust,
            inner_costed,
            Some(INNER_COST),
        ));
    }
    let mut m = machine_with(common::nested_rom(false));
    m.set_hook_table(Some(table(entries)));
    m.set_mean_instructions(&learned);
    m
}

#[test]
fn original_code_a_rust_routine_runs_is_not_counted_twice() {
    // `outer`'s mean (45) holds `inner`'s 39 instructions. When its Rust run has `inner` run as
    // original code, those are fetched by the machine (original side); when `inner` is in Rust
    // too, its own runs count them. Either way `outer` stands for its own 6 only, and the sum is
    // exactly what the pure machine executes.
    for inner_in_rust in [false, true] {
        let mut m = nested_rust_machine(inner_in_rust);
        let mut pure = machine_with(common::nested_rom(false));
        for frame in 1..=6 {
            run(&mut m, 1);
            while pure.total_master_cycles() < m.total_master_cycles() {
                pure.tick(Joypad::default(), &mut NullAudio).unwrap();
            }
            assert_eq!(pure.total_master_cycles(), m.total_master_cycles());
            let coverage = m.coverage();
            let total = coverage.original_instructions + coverage.rust_instructions_estimated;
            assert_eq!(
                total,
                pure.opcode_fetches(),
                "inner in Rust {inner_in_rust}, frame {frame}: {coverage:?}"
            );
        }

        let (outer, tail) = (stats(&m, "outer"), stats(&m, "outer_tail"));
        let inner = m.hook_stats().into_iter().find(|(name, _)| name == "inner").map(|(_, s)| s);
        for (name, s) in [("outer", outer), ("outer_tail", tail)] {
            assert!(s.rust_runs > 100 && s.timing_fallback == 0, "{name}: {s:?}");
            assert_eq!((s.cost_mismatch, s.cost_over_bound), (0, 0), "{name}: {s:?}");
        }
        assert_mean(outer.callee_instructions, 39.0 * outer.rust_runs as f64, "outer's callee");
        assert_eq!(outer.estimated_rust_instructions(), 6 * outer.rust_runs);
        assert_mean(tail.callee_instructions, 0.0, "outer_tail calls nothing");
        assert_eq!(tail.estimated_rust_instructions(), 41 * tail.rust_runs);
        if let Some(inner) = inner {
            assert!(inner_in_rust);
            assert_eq!(
                inner.rust_runs, outer.rust_runs,
                "reached through call_original: {inner:?}"
            );
            assert_eq!(inner.estimated_rust_instructions(), 39 * inner.rust_runs);
        } else {
            assert!(!inner_in_rust, "inner is not in the table");
        }
        // Counted at the plain mean, `outer` would add 39 instructions per run to the pure run's.
        let plain: u64 = [Some(outer), Some(tail), inner]
            .iter()
            .flatten()
            .map(|s| (s.rust_runs as f64 * s.mean_instructions).round() as u64)
            .sum();
        assert_eq!(
            plain - m.coverage().rust_instructions_estimated,
            39 * outer.rust_runs,
            "inner in Rust {inner_in_rust}"
        );
    }
}

/// `outer_tail_costed` declaring its 41 instructions (the number `Verify` measures for it).
fn outer_tail_declared(ctx: &mut Ctx<'_>) {
    outer_tail_costed(ctx);
    ctx.set_instructions(41);
}

/// `outer_tail_costed` declaring a wrong count.
fn outer_tail_declared_wrong(ctx: &mut Ctx<'_>) {
    outer_tail_costed(ctx);
    ctx.set_instructions(40);
}

#[test]
fn declared_instructions_are_checked_in_verify_and_replace_the_mean_in_rust() {
    let tail = |func: RoutineFn, mode: HookMode| {
        routine("outer_tail", NESTED_OUTER_TAIL, mode, func, Some(OUTER_TAIL_COST))
    };
    // Verify: a right declaration is no mismatch, a wrong one is one on every compared call.
    for (func, wrong) in
        [(outer_tail_declared as RoutineFn, false), (outer_tail_declared_wrong, true)]
    {
        let mut verify = machine_with(common::nested_rom(false));
        verify.set_hook_table(Some(table([tail(func, HookMode::Verify)])));
        run(&mut verify, 2);
        let s = stats(&verify, "outer_tail");
        assert!(s.verified_ok > 100, "{s:?}");
        let expected = if wrong { s.verified_ok + s.divergences } else { 0 };
        assert_eq!(s.instructions_mismatch, expected, "{s:?}");
        assert_mean(s.mean_instructions, 41.0, "outer_tail's mean");
    }

    // Rust, with a mean that is wrong on purpose: the declared count is what the run stands for,
    // so the sum is still exactly what the pure machine executes.
    let mut m = machine_with(common::nested_rom(false));
    m.set_hook_table(Some(table([tail(outer_tail_declared, HookMode::Rust)])));
    m.set_mean_instructions(&[("outer_tail".to_owned(), 1000.0)]);
    let mut pure = machine_with(common::nested_rom(false));
    run(&mut m, 3);
    while pure.total_master_cycles() < m.total_master_cycles() {
        pure.tick(Joypad::default(), &mut NullAudio).unwrap();
    }
    let s = stats(&m, "outer_tail");
    assert!(s.rust_runs > 100 && s.declared_runs == s.rust_runs, "{s:?}");
    assert_eq!(s.estimated_rust_instructions(), 41 * s.rust_runs);
    let coverage = m.coverage();
    assert_eq!(
        coverage.original_instructions + coverage.rust_instructions_estimated,
        pure.opcode_fetches(),
        "{coverage:?}"
    );
}
