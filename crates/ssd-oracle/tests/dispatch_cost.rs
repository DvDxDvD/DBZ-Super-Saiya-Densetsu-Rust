//! Exact costs ([`Ctx::charge`], [`Ctx::set_cost`]) and the timing-event window of `Rust` mode:
//! a Rust routine runs only where no timing event can land inside the original, so a machine
//! with Rust routines stays cycle for cycle on the pure original's timeline.

mod common;

use common::{
    LEAF, LEAF_COUNTER, RACE_IDLE, RACE_LEAF, RACE_LEAF_COST, RACE_NMI_COUNTER,
    TWO_PATH_ODD_COUNTER, race_leaf,
};
use ssd_oracle::{
    Ctx, HookAction, HookEntry, HookMode, HookStats, HookTable, Joypad, Machine, MemorySaveStore,
    NullAudio, ReturnKind, RoutineFn,
};

/// CPU-busy master cycles of the even path of [`common::two_path_leaf_rom`]'s leaf (8-cycle
/// slow ROM and WRAM accesses, internal cycles 6): `LDA dp` 24, `AND #` 16, taken `BEQ` 22,
/// `INC dp` 38, `RTS` 42.
const TWO_PATH_EVEN: u64 = 142;
/// Its odd path: `LDA dp` 24, `AND #` 16, untaken `BEQ` 16, two `INC dp` of 38, `RTS` 42.
const TWO_PATH_ODD: u64 = 174;
/// Sync events the race tests compare (one per frame).
const RACE_EVENTS: usize = 60;

fn machine_with(rom: Vec<u8>) -> Machine {
    Machine::new(rom, Box::new(MemorySaveStore::default())).unwrap()
}

fn run(m: &mut Machine, frames: usize) {
    for _ in 0..frames {
        m.run_frame(Joypad::default(), &mut NullAudio).unwrap();
    }
}

/// A routine returning with RTS; contract: 8-bit A, 16-bit X/Y (the fixtures').
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

/// The CPU registers as a comparable tuple: A, X, Y, S, D, PBR, PC, DBR, P, emulation mode.
#[allow(clippy::type_complexity)]
fn registers(m: &Machine) -> (u16, u16, u16, u16, u16, u8, u16, u8, u8, bool) {
    let r = m.cpu_registers();
    (r.a, r.x, r.y, r.s, r.d, r.pbr, r.pc, r.dbr, u8::from(r.p), r.emulation_mode)
}

fn stats(m: &Machine, name: &str) -> HookStats {
    let all = m.hook_stats();
    all.iter()
        .find(|(n, _)| n == name)
        .map(|&(_, s)| s)
        .unwrap_or_else(|| panic!("{name}: {all:?}"))
}

// The two-path leaf

/// The leaf of [`common::two_path_leaf_rom`] in Rust, declaring `cost(odd path)` if any.
fn two_path_declaring(ctx: &mut Ctx<'_>, cost: impl Fn(bool) -> Option<u64>) {
    let counter = ctx.read_dp8(LEAF_COUNTER as u16);
    let odd = counter & 0x01 != 0;
    if odd {
        let other = ctx.read_dp8(TWO_PATH_ODD_COUNTER as u16).wrapping_add(1);
        ctx.write_dp8(TWO_PATH_ODD_COUNTER as u16, other);
    }
    let next = counter.wrapping_add(1);
    ctx.write_dp8(LEAF_COUNTER as u16, next);
    ctx.set_al(counter & 0x01);
    ctx.set_nz8(next);
    if let Some(cost) = cost(odd) {
        ctx.set_cost(cost);
    }
}

/// Each path's own cost.
fn two_path(ctx: &mut Ctx<'_>) {
    two_path_declaring(ctx, |odd| Some(if odd { TWO_PATH_ODD } else { TWO_PATH_EVEN }));
}

/// The even path's cost for both paths: wrong on the odd one.
fn two_path_one_cost(ctx: &mut Ctx<'_>) {
    two_path_declaring(ctx, |_| Some(TWO_PATH_EVEN));
}

/// No cost declared.
fn two_path_no_cost(ctx: &mut Ctx<'_>) {
    two_path_declaring(ctx, |_| None);
}

/// Part of the cost charged up front, the total declared at the end.
fn two_path_charging_first(ctx: &mut Ctx<'_>) {
    ctx.charge(100);
    two_path(ctx);
}

/// `func` in Verify on [`common::two_path_leaf_rom`] for 3 frames.
fn verify_two_path(func: RoutineFn) -> HookStats {
    let mut m = machine_with(common::two_path_leaf_rom());
    m.set_hook_table(Some(table([routine("leaf", LEAF, HookMode::Verify, func, None)])));
    run(&mut m, 3);
    stats(&m, "leaf")
}

#[test]
fn verify_accepts_the_exact_cost_of_each_path() {
    let leaf = verify_two_path(two_path);
    assert!(leaf.verified_ok > 100, "{leaf:?}");
    assert_eq!((leaf.divergences, leaf.cost_mismatch, leaf.cost_missing), (0, 0, 0), "{leaf:?}");
    assert_eq!(leaf.cost_over_bound, 0, "{leaf:?}");
    // The calls alternate between the paths.
    let mean = (TWO_PATH_EVEN + TWO_PATH_ODD) as f64 / 2.0;
    assert!((leaf.mean_cpu_cycles - mean).abs() < 1.0, "{leaf:?}");
}

#[test]
fn verify_counts_a_wrong_declared_cost() {
    let leaf = verify_two_path(two_path_one_cost);
    assert!(leaf.verified_ok > 100, "a wrong cost is no divergence: {leaf:?}");
    assert_eq!((leaf.divergences, leaf.cost_missing), (0, 0), "{leaf:?}");
    // Every odd call.
    assert!(leaf.cost_mismatch.abs_diff(leaf.verified_ok / 2) <= 1, "{leaf:?}");
}

#[test]
fn verify_counts_a_missing_cost() {
    let leaf = verify_two_path(two_path_no_cost);
    assert!(leaf.verified_ok > 100, "{leaf:?}");
    assert_eq!(leaf.cost_missing, leaf.verified_ok, "{leaf:?}");
    assert_eq!(leaf.cost_mismatch, 0, "{leaf:?}");
}

#[test]
fn rust_mode_without_a_maximum_cost_runs_the_original() {
    let mut m = machine_with(common::two_path_leaf_rom());
    m.set_hook_table(Some(table([routine("leaf", LEAF, HookMode::Rust, two_path, None)])));
    let before = m.wram()[LEAF_COUNTER];
    run(&mut m, 3);
    let leaf = stats(&m, "leaf");
    assert!(leaf.calls > 100, "{leaf:?}");
    assert_eq!((leaf.rust_runs, leaf.cost_missing), (0, leaf.calls), "{leaf:?}");
    // The original ran every call; a hit pending at the last frame's end has not yet.
    let delta = u64::from(m.wram()[LEAF_COUNTER].wrapping_sub(before));
    assert!(delta == leaf.calls % 256 || delta == (leaf.calls - 1) % 256, "{delta}, {leaf:?}");
}

/// Ticks the pure `original` until it reaches `hooked`'s master cycle and asserts that both are
/// in the same state there: registers and WRAM.
fn assert_on_the_same_timeline(original: &mut Machine, hooked: &Machine) {
    while original.total_master_cycles() < hooked.total_master_cycles() {
        original.tick(Joypad::default(), &mut NullAudio).unwrap();
    }
    assert_eq!(original.total_master_cycles(), hooked.total_master_cycles(), "tick boundaries");
    assert_eq!(registers(original), registers(hooked));
    assert!(original.wram() == hooked.wram(), "WRAM differs");
}

#[test]
fn rust_mode_charges_the_declared_cost_of_each_path() {
    for func in [two_path as RoutineFn, two_path_charging_first] {
        let mut hooked = machine_with(common::two_path_leaf_rom());
        let leaf = routine("leaf", LEAF, HookMode::Rust, func, Some(TWO_PATH_ODD));
        hooked.set_hook_table(Some(table([leaf])));
        run(&mut hooked, 5);
        let leaf = stats(&hooked, "leaf");
        assert!(leaf.rust_runs > 200, "{leaf:?}");
        assert_eq!((leaf.timing_fallback, leaf.cost_missing), (0, 0), "no timing event: {leaf:?}");
        assert_eq!(leaf.cost_over_bound, 0, "{leaf:?}");

        let mut original = machine_with(common::two_path_leaf_rom());
        assert_on_the_same_timeline(&mut original, &hooked);
    }
}

/// Declares 400 master cycles for both paths.
fn two_path_declaring_400(ctx: &mut Ctx<'_>) {
    two_path_declaring(ctx, |_| Some(400));
}

#[test]
fn an_undeclared_cost_is_charged_the_maximum() {
    let hooked = |func: RoutineFn| {
        let mut m = machine_with(common::two_path_leaf_rom());
        m.set_hook_table(Some(table([routine("leaf", LEAF, HookMode::Rust, func, Some(400))])));
        run(&mut m, 5);
        m
    };
    let undeclared = hooked(two_path_no_cost);
    let declared = hooked(two_path_declaring_400);
    let leaf = stats(&undeclared, "leaf");
    assert!(leaf.rust_runs > 200, "{leaf:?}");
    assert_eq!(leaf.cost_missing, leaf.rust_runs, "{leaf:?}");
    assert_eq!(stats(&declared, "leaf").cost_missing, 0);
    // Charged the maximum, 400: the same timeline as when 400 is declared.
    assert_eq!(undeclared.total_master_cycles(), declared.total_master_cycles());
    assert_eq!(registers(&undeclared), registers(&declared));
    assert!(undeclared.wram() == declared.wram(), "WRAM differs");
    assert!(leaf.rust_runs < stats(&hooked(two_path), "leaf").rust_runs, "exact costs are less");
}

/// The even path's cost declared on both paths, after charging more than that.
fn two_path_charging_beyond_its_cost(ctx: &mut Ctx<'_>) {
    ctx.charge(200);
    two_path_one_cost(ctx);
}

/// [`two_path_charging_beyond_its_cost`] that also forgets the odd counter: wrong on odd calls.
fn two_path_wrong_and_charging_beyond_its_cost(ctx: &mut Ctx<'_>) {
    ctx.charge(200);
    let counter = ctx.read_dp8(LEAF_COUNTER as u16);
    let next = counter.wrapping_add(1);
    ctx.write_dp8(LEAF_COUNTER as u16, next);
    ctx.set_al(counter & 0x01);
    ctx.set_nz8(next);
    ctx.set_cost(TWO_PATH_EVEN);
}

#[test]
fn charges_beyond_the_declared_cost_are_over_bound() {
    let mut m = machine_with(common::two_path_leaf_rom());
    let leaf = routine("leaf", LEAF, HookMode::Rust, two_path_charging_beyond_its_cost, Some(400));
    m.set_hook_table(Some(table([leaf])));
    run(&mut m, 3);
    let leaf = stats(&m, "leaf");
    assert!(leaf.rust_runs > 100, "{leaf:?}");
    assert_eq!(leaf.cost_over_bound, leaf.rust_runs, "{leaf:?}");

    // Verify counts it without a maximum cost, and a correct routine still verifies.
    let leaf = verify_two_path(two_path_charging_beyond_its_cost);
    assert!(leaf.verified_ok > 100, "{leaf:?}");
    assert_eq!((leaf.divergences, leaf.cost_over_bound), (0, leaf.verified_ok), "{leaf:?}");
}

#[test]
fn a_declared_cost_above_the_maximum_is_over_bound() {
    for mode in [HookMode::Rust, HookMode::Verify] {
        let mut m = machine_with(common::two_path_leaf_rom());
        // Odd calls declare 174, above the maximum of 160.
        m.set_hook_table(Some(table([routine("leaf", LEAF, mode, two_path, Some(160))])));
        run(&mut m, 3);
        let leaf = stats(&m, "leaf");
        let calls = if mode == HookMode::Rust { leaf.rust_runs } else { leaf.verified_ok };
        assert!(calls > 100, "{mode:?}: {leaf:?}");
        assert!(leaf.cost_over_bound.abs_diff(calls / 2) <= 1, "{mode:?}: odd calls: {leaf:?}");
        assert_eq!(leaf.divergences, 0, "{mode:?}: {leaf:?}");
    }
}

#[test]
fn a_divergence_names_a_cost_over_bound() {
    let mut m = machine_with(common::two_path_leaf_rom());
    let func = two_path_wrong_and_charging_beyond_its_cost;
    m.set_hook_table(Some(table([routine("leaf", LEAF, HookMode::Verify, func, Some(400))])));
    run(&mut m, 1);
    let leaf = stats(&m, "leaf");
    assert!(leaf.divergences > 10, "{leaf:?}");
    assert_eq!(leaf.cost_over_bound, leaf.divergences + leaf.verified_ok, "{leaf:?}");
    for divergence in m.divergences() {
        let text = divergence.to_string();
        let expected = format!(
            "leaf — cost over bound: charged 200 before the return, declared {TWO_PATH_EVEN}"
        );
        assert!(text.contains(&expected), "{text}");
    }
}

/// The leaf of [`common::dma_leaf_rom`] (`INC $10; RTS`), its whole cost charged before its
/// write (as a routine placing a register write does) and declared.
fn dma_leaf(ctx: &mut Ctx<'_>) {
    ctx.charge(DMA_LEAF_COST);
    let value = ctx.read_dp8(LEAF_COUNTER as u16).wrapping_add(1);
    ctx.write_dp8(LEAF_COUNTER as u16, value);
    ctx.set_nz8(value);
    ctx.set_cost(DMA_LEAF_COST);
}

/// CPU-busy master cycles of the leaf of [`common::dma_leaf_rom`]: `INC dp` 38, `RTS` 42 (the
/// transfer requested before it is DMA time, not CPU time).
const DMA_LEAF_COST: u64 = 80;

#[test]
fn a_transfer_drained_before_the_call_is_not_spent_by_it() {
    let mut m = machine_with(common::dma_leaf_rom());
    let leaf = routine("leaf", common::DMA_LEAF, HookMode::Verify, dma_leaf, Some(DMA_LEAF_COST));
    m.set_hook_table(Some(table([leaf])));
    run(&mut m, 3);
    let leaf = stats(&m, "leaf");
    assert!(leaf.verified_ok > 100, "{leaf:?}");
    assert_eq!((leaf.divergences, leaf.cost_mismatch), (0, 0), "{leaf:?}");
    assert_eq!(leaf.cost_over_bound, 0, "{leaf:?}");
}

// The NMI race

/// `rom` with its idle loop as a sync point, plus `leaf` if given.
fn race_machine(rom: Vec<u8>, leaf: Option<HookEntry>) -> Machine {
    let mut m = machine_with(rom);
    let idle = HookEntry::sync_point("idle", RACE_IDLE);
    m.set_hook_table(Some(table([idle].into_iter().chain(leaf))));
    m
}

/// The race leaf in `mode` with `cycles` as its maximum cost.
fn race_leaf_entry(mode: HookMode, cycles: Option<u64>) -> HookEntry {
    routine("leaf", RACE_LEAF, mode, race_leaf, cycles)
}

/// Runs the pure `original` and `hooked` from sync event to sync event; returns the first event
/// at which they differ (event, frame, NMI count, master cycle, registers, WRAM), if any.
fn first_difference(original: &mut Machine, hooked: &mut Machine) -> Option<String> {
    for event in 0..RACE_EVENTS {
        let next = |m: &mut Machine| {
            m.run_until_sync(&mut |_| Joypad::default(), &mut NullAudio, 3)
                .unwrap()
                .expect("an event every frame")
        };
        let (a, b) = (next(original), next(hooked));
        if a != b {
            return Some(format!("event {event}: {a:?} vs {b:?}"));
        }
        if registers(original) != registers(hooked) {
            return Some(format!("event {event}: registers differ"));
        }
        if let Some(i) =
            (0..original.wram().len()).find(|&i| original.wram()[i] != hooked.wram()[i])
        {
            return Some(format!("event {event}: WRAM ${i:05X} differs"));
        }
    }
    None
}

#[test]
fn the_race_leaf_cost_is_exact_and_the_nmi_usually_lands_inside_it() {
    let mut m = race_machine(common::nmi_race_rom(), Some(race_leaf_entry(HookMode::Verify, None)));
    run(&mut m, 10);
    let leaf = stats(&m, "leaf");
    assert!(leaf.verified_ok > 1000, "{leaf:?}");
    assert_eq!((leaf.divergences, leaf.cost_mismatch, leaf.cost_missing), (0, 0, 0), "{leaf:?}");
    assert!(leaf.interrupted >= 5, "most NMIs land inside the leaf: {leaf:?}");
    assert_eq!(m.nmi_count() % 256, u64::from(m.wram()[RACE_NMI_COUNTER]), "the handler ran");
}

/// Checks the race leaf in Rust mode with `cycles` against a pure twin on `rom`; returns its
/// stats and the first difference.
fn race(rom: fn() -> Vec<u8>, cycles: u64) -> (HookStats, Option<String>) {
    let mut original = race_machine(rom(), None);
    let mut hooked = race_machine(rom(), Some(race_leaf_entry(HookMode::Rust, Some(cycles))));
    let difference = first_difference(&mut original, &mut hooked);
    (stats(&hooked, "leaf"), difference)
}

#[test]
fn near_the_nmi_the_original_runs_and_the_timeline_stays_exact() {
    let (leaf, difference) = race(common::nmi_race_rom, RACE_LEAF_COST);
    assert_eq!(difference, None, "{leaf:?}");
    assert!(leaf.rust_runs > 1000, "{leaf:?}");
    assert!(leaf.timing_fallback >= RACE_EVENTS as u64 / 2, "{leaf:?}");
    assert_eq!(leaf.cost_missing, 0);
}

#[test]
fn a_window_shorter_than_the_cost_lets_the_nmi_race_diverge() {
    // The window then ends before the original would return: the NMI can land inside it.
    let (leaf, difference) = race(common::nmi_race_rom, 0);
    assert!(difference.is_some(), "{leaf:?}");
}

#[test]
fn near_hdma_the_original_runs_and_the_timeline_stays_exact() {
    let (leaf, difference) = race(common::hdma_idle_rom, RACE_LEAF_COST);
    assert_eq!(difference, None, "{leaf:?}");
    assert!(leaf.rust_runs > 1000, "{leaf:?}");
    // HDMA starts on every visible line: far more fallbacks than NMIs.
    assert!(leaf.timing_fallback > 20 * RACE_EVENTS as u64, "{leaf:?}");
}

#[test]
fn a_window_shorter_than_the_cost_lets_hdma_shift_the_timeline() {
    let (leaf, difference) = race(common::hdma_idle_rom, 0);
    assert!(difference.is_some(), "{leaf:?}");
}

// The time to the next timing event

/// How late the timing events came after a pure machine's predictions.
#[derive(Debug, Default)]
struct Lateness {
    nmis: u64,
    hdmas: u64,
    /// The most master cycles an NMI was taken after its predicted edge.
    nmi: u64,
    /// The most master cycles HDMA became busy after its predicted position.
    hdma: u64,
}

/// Ticks a pure machine on `rom` for `frames` frames, predicting at every opcode fetch when the
/// next timing event comes, and checks each prediction against the next event seen: an NMI the
/// CPU takes (after the instruction in progress at the edge) or, if `hdma`, HDMA becoming busy
/// (on the first DMA tick at or past the trigger position; without a channel enabled, the
/// frame-start reload check makes it busy without taking cycles). Every prediction since the
/// previous event must be at or before the event.
fn check_predictions(rom: Vec<u8>, frames: u64, hdma: bool) -> Lateness {
    let mut m = machine_with(rom);
    // Settings change while the program starts up; check from the first frame on.
    run(&mut m, 1);
    let mut lateness = Lateness::default();
    // The earliest and latest predicted event positions since the last event.
    let mut predicted: Option<(u64, u64)> = None;
    while m.frame_count() < frames {
        let now = m.total_master_cycles();
        if m.at_fetch_boundary() {
            let until = m.master_cycles_until_timing_event();
            assert_ne!(until, u64::MAX, "an event always comes");
            let at = now + until;
            predicted = Some(predicted.map_or((at, at), |(lo, hi)| (lo.min(at), hi.max(at))));
        }
        let (nmi_count, hdma_busy) = (m.nmi_count(), m.hdma_busy());
        m.tick(Joypad::default(), &mut NullAudio).unwrap();
        let nmi = m.nmi_count() != nmi_count;
        let hdma_event = hdma && !hdma_busy && m.hdma_busy();
        if !(nmi || hdma_event) {
            continue;
        }
        let (earliest, latest) = predicted.take().expect("a prediction since the last event");
        // `now` is the start of the tick that took the NMI or made HDMA busy.
        assert!(latest <= now, "event at {now} before a prediction of {latest}");
        let late = now - earliest;
        if nmi {
            lateness.nmis += 1;
            lateness.nmi = lateness.nmi.max(late);
        } else {
            lateness.hdmas += 1;
            lateness.hdma = lateness.hdma.max(late);
        }
    }
    lateness
}

/// An NMI is taken at most this long after its edge: the instruction in progress (at most 8 CPU
/// cycles in these fixtures) and a memory refresh.
const NMI_LATENCY: u64 = 8 * 8 + 40;
/// HDMA becomes busy at most one CPU cycle after its position (no refresh is near it).
const HDMA_LATENCY: u64 = 12;

#[test]
fn the_time_to_the_next_timing_event_matches_the_emulation() {
    let nmi = check_predictions(common::nmi_race_rom(), 6, false);
    assert!(nmi.nmis >= 4 && nmi.nmi <= NMI_LATENCY, "{nmi:?}");
    let hdma = check_predictions(common::hdma_idle_rom(), 4, true);
    // The frame-start reload and a transfer on each of the 225 visible lines, every frame.
    assert!(hdma.nmis >= 2 && hdma.hdmas >= 3 * 226, "{hdma:?}");
    assert!(hdma.nmi <= NMI_LATENCY && hdma.hdma <= HDMA_LATENCY, "{hdma:?}");
}
