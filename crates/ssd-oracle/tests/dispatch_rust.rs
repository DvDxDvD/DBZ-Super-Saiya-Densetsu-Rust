//! `Rust` hook mode: Rust functions replace hooked routines on the live machine.

mod common;

use common::{LEAF, LEAF_COUNTER};
use ssd_oracle::{
    Ctx, HookAction, HookEntry, HookMode, HookStats, HookTable, Joypad, Machine, MemorySaveStore,
    NullAudio, ReturnKind, RoutineFn, TickEvent,
};

/// Master cycles in one NTSC frame (262 lines of 1364).
const NTSC_FRAME_MCLK: u64 = 262 * 1364;
/// Where the leaf of [`common::jsr_leaf_rom`] returns to: `LDX #$00C8` after `JSR $8040`.
const LEAF_RETURN: u32 = 0x00_800A;
/// WRAM index of a scratch counter the test routines increment (`$00:0012`).
const PROBE_COUNTER: usize = 0x12;
/// CPU-busy master cycles of the leaf of [`common::jsr_leaf_rom`]: `INC dp` 38, `RTS` 42.
const LEAF_COST: u64 = 80;

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

/// Ticks until `done` holds after a tick; panics after `max_frames` frames.
fn tick_until(m: &mut Machine, max_frames: u64, mut done: impl FnMut(&Machine, TickEvent) -> bool) {
    let limit = m.frame_count() + max_frames;
    while m.frame_count() < limit {
        let event = tick(m);
        if done(m, event) {
            return;
        }
    }
    panic!("condition not reached within {max_frames} frames");
}

/// A routine in `Rust` mode that returns with RTS; contract: 8-bit A, 16-bit X/Y (the fixtures').
fn rust_routine(name: &str, key: u32, func: RoutineFn, cycles: Option<u64>) -> HookEntry {
    HookEntry {
        name: name.to_owned(),
        key,
        mode: HookMode::Rust,
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

/// An address whose arrivals are reported and resume the original code.
fn sync_point(name: &str, key: u32) -> HookEntry {
    HookEntry {
        name: name.to_owned(),
        key,
        mode: HookMode::Verify,
        action: HookAction::SyncPoint,
        entry_m8: None,
        entry_x8: None,
        flags_mask: 0xFF,
        verify_sample: 1,
        bound: None,
        budget_frames: None,
        cycles: None,
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

/// The leaf `INC $10` (8-bit memory) in Rust, without a cost.
fn inc_counter_uncosted(ctx: &mut Ctx<'_>) {
    let value = ctx.read_dp8(0x10).wrapping_add(1);
    ctx.write_dp8(0x10, value);
    ctx.set_nz8(value);
}

/// The leaf `INC $10` in Rust, declaring its cost.
fn inc_counter(ctx: &mut Ctx<'_>) {
    inc_counter_uncosted(ctx);
    ctx.set_cost(LEAF_COST);
}

#[test]
fn rust_mode_replaces_the_routine() {
    let mut m = machine_with(common::jsr_leaf_rom());
    m.set_hook_table(Some(table([rust_routine("leaf", LEAF, inc_counter, Some(LEAF_COST))])));
    let before = m.wram()[LEAF_COUNTER];
    run(&mut m, 3);

    let leaf = stats(&m, "leaf");
    assert_eq!(m.frame_count(), 3);
    assert!(leaf.rust_runs > 100, "about 50 calls per frame: {leaf:?}");
    assert_eq!(leaf.calls, leaf.rust_runs);
    assert_eq!(leaf.contract_mismatch, 0);
    assert_eq!(m.wram()[LEAF_COUNTER], before.wrapping_add(leaf.rust_runs as u8));
}

#[test]
fn a_contract_mismatch_runs_the_original() {
    for (entry_m8, entry_x8) in [(Some(false), None), (None, Some(true))] {
        let mut entry = rust_routine("leaf", LEAF, inc_counter, None);
        entry.entry_m8 = entry_m8;
        entry.entry_x8 = entry_x8;
        let mut m = machine_with(common::jsr_leaf_rom());
        m.set_hook_table(Some(table([entry])));
        let before = m.wram()[LEAF_COUNTER];
        run(&mut m, 3);

        let leaf = stats(&m, "leaf");
        assert!(leaf.contract_mismatch > 100, "{leaf:?}");
        assert_eq!(leaf.rust_runs, 0);
        assert_eq!(leaf.calls, leaf.contract_mismatch);
        // Every mismatched call ran `INC $10`; a hit pending at the last frame's end has not yet.
        let delta = m.wram()[LEAF_COUNTER].wrapping_sub(before);
        assert!(
            delta == leaf.calls as u8 || delta == (leaf.calls - 1) as u8,
            "delta {delta}, {leaf:?}"
        );
    }
}

/// CPU-busy master cycles of one original call of the leaf, from the hit at its entry to the
/// opcode fetch at the return address.
fn measured_leaf_cost() -> u64 {
    let mut m = machine_with(common::jsr_leaf_rom());
    m.set_hook_table(Some(table([sync_point("leaf", LEAF)])));
    tick_until(&mut m, 2, |_, event| event == TickEvent::Hook(LEAF));
    let start = m.cpu_busy_cycles();
    tick_until(&mut m, 1, |m, _| m.at_fetch_boundary() && m.cpu_registers().pc == 0x800A);
    m.cpu_busy_cycles() - start
}

/// Calls of the leaf in `frames` frames with the leaf in `Rust` mode as `func`.
fn rust_calls(frames: usize, func: RoutineFn) -> u64 {
    let mut m = machine_with(common::jsr_leaf_rom());
    m.set_hook_table(Some(table([rust_routine("leaf", LEAF, func, Some(LEAF_COST))])));
    run(&mut m, frames);
    stats(&m, "leaf").rust_runs
}

/// The leaf `INC $10` in Rust, declaring no time at all.
fn inc_counter_for_free(ctx: &mut Ctx<'_>) {
    inc_counter_uncosted(ctx);
    ctx.set_cost(0);
}

#[test]
fn charging_the_measured_cost_keeps_the_original_call_rate() {
    const FRAMES: usize = 10;
    assert_eq!(measured_leaf_cost(), LEAF_COST, "INC dp (38 master cycles) + RTS (42)");

    let mut original = machine_with(common::jsr_leaf_rom());
    original.set_hook_table(Some(table([sync_point("leaf", LEAF)])));
    run(&mut original, FRAMES);
    let original_calls = stats(&original, "leaf").calls;

    let charged = rust_calls(FRAMES, inc_counter);
    let free = rust_calls(FRAMES, inc_counter_for_free);
    assert!(charged.abs_diff(original_calls) <= 1, "{charged} vs {original_calls} original");
    assert!(free > original_calls + 2, "uncharged calls are faster: {free} vs {original_calls}");
}

/// The leaf `INC $10` in Rust, declaring 5000 master cycles.
fn inc_counter_slowly(ctx: &mut Ctx<'_>) {
    inc_counter_uncosted(ctx);
    ctx.set_cost(5000);
}

#[test]
fn a_return_to_a_hooked_address_is_reported() {
    for (func, cycles) in [(inc_counter as RoutineFn, LEAF_COST), (inc_counter_slowly, 5000)] {
        let mut m = machine_with(common::jsr_leaf_rom());
        m.set_hook_table(Some(table([
            rust_routine("leaf", LEAF, func, Some(cycles)),
            sync_point("return", LEAF_RETURN),
        ])));
        run(&mut m, 3);

        let leaf = stats(&m, "leaf");
        let ret = stats(&m, "return");
        assert!(leaf.rust_runs > 0);
        // One arrival per call; the last one may still be pending at the end of the frame.
        assert!(
            ret.calls == leaf.rust_runs || ret.calls + 1 == leaf.rust_runs,
            "cycles {cycles}: {} returns for {} calls",
            ret.calls,
            leaf.rust_runs
        );
    }
}

/// The leaf `INC $10` in Rust, declaring half a frame.
fn inc_counter_for_half_a_frame(ctx: &mut Ctx<'_>) {
    inc_counter_uncosted(ctx);
    ctx.set_cost(NTSC_FRAME_MCLK / 2);
}

#[test]
fn a_call_whose_cost_would_span_the_nmi_runs_the_original() {
    let mut m = machine_with(common::nmi_leaf_rom());
    // Half a frame per call: a call in the half frame before the NMI would span it.
    let leaf = rust_routine("leaf", LEAF, inc_counter_for_half_a_frame, Some(NTSC_FRAME_MCLK / 2));
    m.set_hook_table(Some(table([leaf])));
    let start = m.wram()[common::NMI_COUNTER];
    run(&mut m, 8);

    let leaf = stats(&m, "leaf");
    assert!(leaf.rust_runs >= 4, "a call early in each frame runs in Rust: {leaf:?}");
    assert!(leaf.timing_fallback > 100, "{leaf:?}");
    assert_eq!(leaf.calls, leaf.rust_runs + leaf.timing_fallback, "{leaf:?}");
    // No NMI is raised while a call is charged: the handler counts one per frame.
    let nmis = m.wram()[common::NMI_COUNTER].wrapping_sub(start);
    assert!(nmis >= 7, "{nmis} NMIs in 8 frames");
}

/// Exercises the flag helpers; every assertion runs inside the dispatch.
fn flag_probe(ctx: &mut Ctx<'_>) {
    ctx.set_c(true);
    ctx.set_v(true);
    ctx.set_nz8(0x00);
    assert!(ctx.z() && !ctx.n());
    ctx.set_nz8(0x80);
    assert!(!ctx.z() && ctx.n());
    ctx.set_nz8(0x7F);
    assert!(!ctx.z() && !ctx.n());
    ctx.set_nz16(0x0000);
    assert!(ctx.z() && !ctx.n());
    ctx.set_nz16(0x8000);
    assert!(!ctx.z() && ctx.n());
    ctx.set_nz16(0x0100);
    assert!(!ctx.z() && !ctx.n(), "a zero low byte is not zero");
    ctx.set_nz16(0x0080);
    assert!(!ctx.z() && !ctx.n(), "bit 7 is not the sign of a 16-bit value");
    assert!(ctx.c() && ctx.v(), "only N and Z change");
    assert!(ctx.m8() && !ctx.x8(), "the widths are untouched");
    ctx.set_c(false);
    ctx.set_v(false);
    ctx.set_z(true);
    ctx.set_n(true);
    assert!(!ctx.c() && !ctx.v() && ctx.z() && ctx.n());
    inc_counter(ctx);
}

#[test]
fn ctx_flag_helpers() {
    let mut m = machine_with(common::jsr_leaf_rom());
    m.set_hook_table(Some(table([rust_routine("leaf", LEAF, flag_probe, Some(LEAF_COST))])));
    run(&mut m, 1);
    assert!(stats(&m, "leaf").rust_runs > 0);
}

/// Exercises the width setters; every assertion runs inside the dispatch.
fn width_probe(ctx: &mut Ctx<'_>) {
    assert!(ctx.m8() && !ctx.x8(), "the fixtures' widths");
    ctx.set_x(0x1234);
    ctx.set_y(0xABCD);
    ctx.set_m8(false);
    assert!(!ctx.m8() && !ctx.x8());
    ctx.set_x8(true);
    assert!(ctx.x8() && !ctx.m8());
    assert_eq!((ctx.x(), ctx.y()), (0x0034, 0x00CD), "going 8-bit clears the high bytes");
    ctx.set_x8(false);
    ctx.set_m8(true);
    assert!(ctx.m8() && !ctx.x8());
    assert_eq!((ctx.x(), ctx.y()), (0x0034, 0x00CD), "going 16-bit keeps them");
    inc_counter(ctx);
}

#[test]
fn ctx_width_setters() {
    let mut m = machine_with(common::jsr_leaf_rom());
    m.set_hook_table(Some(table([rust_routine("leaf", LEAF, width_probe, Some(LEAF_COST))])));
    run(&mut m, 1);
    assert!(stats(&m, "leaf").rust_runs > 0);
}

/// Exercises registers and memory with D = $0100 and DBR = $7E; every assertion runs inside the
/// dispatch.
fn bus_probe(ctx: &mut Ctx<'_>) {
    assert_eq!((ctx.d(), ctx.dbr(), ctx.pbr()), (0x0100, 0x7E, 0x00));
    assert_eq!(ctx.s() & 0xFF00, 0x0100, "the stack of the reset");

    ctx.set_a(0x1234);
    assert_eq!((ctx.a(), ctx.al()), (0x1234, 0x34));
    ctx.set_al(0x56);
    assert_eq!(ctx.a(), 0x1256, "set_al keeps the B byte");
    ctx.set_x(0xABCD);
    ctx.set_y(0x0102);
    assert_eq!((ctx.x(), ctx.y()), (0xABCD, 0x0102), "16-bit index registers");

    ctx.write_dp8(0x10, 0x5A);
    assert_eq!(ctx.read8(0x7E_0110), 0x5A, "direct page is D-relative in bank 0");
    assert_eq!(ctx.read_dp8(0x10), 0x5A);
    ctx.write_dp16(0x20, 0xBEEF);
    assert_eq!((ctx.read8(0x00_0120), ctx.read8(0x00_0121)), (0xEF, 0xBE));
    assert_eq!(ctx.read_dp16(0x20), 0xBEEF);
    ctx.write_dp8(0xFF00, 0x77);
    assert_eq!(ctx.read8(0x7E_0000), 0x77, "D + offset wraps within bank 0");

    ctx.write_db16(0x2000, 0x1234);
    assert_eq!((ctx.read8(0x7E_2000), ctx.read8(0x7E_2001)), (0x34, 0x12));
    assert_eq!(ctx.read_db8(0x2001), 0x12);
    assert_eq!(ctx.read_db16(0x2000), 0x1234);

    ctx.write16(0x7E_FFFF, 0xCAFE);
    assert_eq!((ctx.read8(0x7E_FFFF), ctx.read8(0x7F_0000)), (0xFE, 0xCA), "crosses the bank");
    assert_eq!(ctx.read_db16(0xFFFF), 0xCAFE);
    ctx.write_db8(0xFFFF, 0x11);
    assert_eq!(ctx.read16(0x7E_FFFF), 0xCA11);
    ctx.write8(0x00_0000, 0x99);
    assert_eq!(ctx.read16(0xFF_FFFF) >> 8, 0x99, "$FF:FFFF wraps to $00:0000");

    inc_counter(ctx);
}

#[test]
fn ctx_registers_and_memory() {
    let mut m = machine_with(common::jsr_leaf_rom());
    let mut registers = m.cpu_registers();
    registers.d = 0x0100;
    registers.dbr = 0x7E;
    m.set_cpu_registers(registers);
    m.set_hook_table(Some(table([rust_routine("leaf", LEAF, bus_probe, Some(LEAF_COST))])));
    m.set_cpu_write_log(true);
    run(&mut m, 1);
    assert!(stats(&m, "leaf").rust_runs > 0);
    assert!(
        m.take_cpu_writes().contains(&(0x7E_FFFF, 0xFE)),
        "Ctx writes are CPU writes in the log"
    );
}

/// Copies ROM `$00:8000..$8010` to WRAM `$0200` (through MDMAEN at `$00:420B`) and to `$0300`
/// (through its mirror at `$80:420B`); asserts each transfer is over when `write8` returns.
fn dma_probe(ctx: &mut Ctx<'_>) {
    for (destination, mdmaen) in [(0x0200_u32, 0x00_420B_u32), (0x0300, 0x80_420B)] {
        ctx.write8(0x7E_0000 | destination, 0x00);
        let setup: &[(u32, u8)] = &[
            (0x2181, destination as u8),
            (0x2182, (destination >> 8) as u8),
            (0x2183, 0x00),
            (0x4300, 0x00), // channel 0: A to B, increment, one register
            (0x4301, 0x80), // B = WMDATA ($2180)
            (0x4302, 0x00), // A = $00:8000 (ROM)
            (0x4303, 0x80),
            (0x4304, 0x00),
            (0x4305, 0x10), // 16 bytes
            (0x4306, 0x00),
        ];
        for &(address, value) in setup {
            ctx.write8(address, value);
        }
        ctx.write8(mdmaen, 0x01);
        for i in 0..16 {
            assert_eq!(ctx.read8(0x7E_0000 | (destination + i)), ctx.read8(0x00_8000 + i));
        }
    }
    inc_counter(ctx);
}

#[test]
fn a_write_to_mdmaen_drains_the_transfer() {
    let mut m = machine_with(common::jsr_leaf_rom());
    m.set_hook_table(Some(table([rust_routine("leaf", LEAF, dma_probe, Some(LEAF_COST))])));
    run(&mut m, 1);
    assert!(stats(&m, "leaf").rust_runs > 0);
}

/// The leaf of [`common::dma_leaf_rom`]; also counts in `$12` the calls that find the GPDMA
/// requested just before the call already done.
fn inc_counter_counting_copies(ctx: &mut Ctx<'_>) {
    if ctx.read8(0x7E_0000 | common::DMA_DESTINATION as u32) == ctx.read8(0x00_8000) {
        let copies = ctx.read_dp8(PROBE_COUNTER as u16).wrapping_add(1);
        ctx.write_dp8(PROBE_COUNTER as u16, copies);
    }
    inc_counter(ctx);
}

#[test]
fn a_transfer_requested_right_before_the_call_runs_the_original() {
    // The transfer's phase depends on the CPU cycle it starts after, which a held CPU does not
    // reproduce: the call is a timing fallback.
    let mut m = machine_with(common::dma_leaf_rom());
    let leaf = rust_routine("leaf", common::DMA_LEAF, inc_counter_counting_copies, Some(80));
    m.set_hook_table(Some(table([leaf])));
    let before = (m.wram()[LEAF_COUNTER], m.wram()[PROBE_COUNTER]);
    run(&mut m, 3);

    let leaf = stats(&m, "leaf");
    assert!(leaf.calls > 100, "about 45 calls per frame: {leaf:?}");
    assert_eq!((leaf.rust_runs, leaf.timing_fallback), (0, leaf.calls), "{leaf:?}");
    // The original ran every call; a hit pending at the last frame's end has not yet.
    let delta = u64::from(m.wram()[LEAF_COUNTER].wrapping_sub(before.0));
    assert!(delta == leaf.calls % 256 || delta == (leaf.calls - 1) % 256, "{delta}, {leaf:?}");
    assert_eq!(m.wram()[PROBE_COUNTER], before.1, "the Rust function never ran");
}

/// CPU-busy master cycles of the loop path of [`common::jsr_leaf_rom`] in the original: from the
/// arrival at the leaf's return address to the next arrival at the leaf.
fn measured_loop_path_cost() -> u64 {
    let mut m = machine_with(common::jsr_leaf_rom());
    m.set_hook_table(Some(table([sync_point("leaf", LEAF), sync_point("return", LEAF_RETURN)])));
    tick_until(&mut m, 2, |_, event| event == TickEvent::Hook(LEAF_RETURN));
    let start = m.cpu_busy_cycles();
    tick_until(&mut m, 1, |_, event| event == TickEvent::Hook(LEAF));
    m.cpu_busy_cycles() - start
}

/// Not a multiple of 6: charging with fast held cycles only would overshoot.
const TARGET: u64 = 400_000;

/// The leaf `INC $10` in Rust, declaring [`TARGET`] after charging part of it.
fn inc_counter_for_target(ctx: &mut Ctx<'_>) {
    ctx.charge(1000);
    inc_counter_uncosted(ctx);
    ctx.set_cost(TARGET);
}

#[test]
fn charges_are_exact_in_cpu_busy_cycles() {
    let loop_path = measured_loop_path_cost();
    let mut m = machine_with(common::jsr_leaf_rom());
    let leaf = rust_routine("leaf", LEAF, inc_counter_for_target, Some(TARGET));
    m.set_hook_table(Some(table([leaf])));
    // A charge spans more than a frame, so a run_frame that dispatches returns at the first frame
    // it renders, with the rest of the charge pending; now and then a frame ends on the loop
    // path instead, without a call.
    let mut last_return: Option<(u64, u64)> = None;
    let mut checked = 0;
    while checked < 5 {
        assert!(m.frame_count() < 40, "too few calls");
        let runs_before = stats(&m, "leaf").rust_runs;
        run(&mut m, 1);
        let runs = stats(&m, "leaf").rust_runs;
        if runs == runs_before {
            continue;
        }
        assert_eq!(runs, runs_before + 1, "one call per run_frame");
        assert_eq!(u32::from(m.cpu_registers().pc), LEAF_RETURN & 0xFFFF, "right after it");
        assert!(m.charge_pending(), "a charge of {TARGET} spans frames");
        while m.charge_pending() {
            tick(&mut m);
        }
        assert_eq!(u32::from(m.cpu_registers().pc), LEAF_RETURN & 0xFFFF, "the hold is over");
        let busy = m.cpu_busy_cycles();
        if let Some((last_runs, last_busy)) = last_return
            && last_runs + 1 == runs
        {
            assert_eq!(busy - last_busy - loop_path, TARGET, "loop path {loop_path}");
            checked += 1;
        }
        last_return = Some((runs, busy));
    }
}

/// Calls of the leaf counted for `name`, after one frame with `entry` installed under LEAF.
fn reinstall_with(m: &mut Machine, entry: HookEntry) -> u64 {
    let name = entry.name.clone();
    m.set_hook_table(Some(table([entry])));
    stats(m, &name).calls
}

/// The leaf `INC $10` in Rust, written differently (identical functions may share an address).
fn inc_counter_again(ctx: &mut Ctx<'_>) {
    let value = ctx.read8(0x7E_0010).wrapping_add(1);
    ctx.write8(0x7E_0010, value);
    ctx.set_nz8(value);
}

#[test]
fn stats_reset_when_another_routine_is_installed_under_the_key() {
    let leaf = rust_routine("leaf", LEAF, inc_counter, None);
    let mut m = machine_with(common::jsr_leaf_rom());
    m.set_hook_table(Some(table([leaf.clone()])));
    run(&mut m, 1);
    let calls = stats(&m, "leaf").calls;
    assert!(calls > 0);

    let mut demoted = table([leaf.clone()]);
    demoted.demote(LEAF);
    m.set_hook_table(Some(demoted));
    assert_eq!(stats(&m, "leaf").calls, calls, "same routine, other mode: kept");
    assert_eq!(reinstall_with(&mut m, leaf.clone()), calls, "same routine: kept");

    let mut renamed = leaf.clone();
    renamed.name = "renamed".to_owned();
    assert_eq!(reinstall_with(&mut m, renamed), 0, "other name: reset");
    run(&mut m, 1);
    assert!(stats(&m, "renamed").calls > 0);
    assert_eq!(reinstall_with(&mut m, leaf.clone()), 0, "other name again: reset");
    run(&mut m, 1);

    let other_func = rust_routine("leaf", LEAF, inc_counter_again, None);
    assert_eq!(reinstall_with(&mut m, other_func), 0, "other function: reset");
    run(&mut m, 1);
    let mut other_return = rust_routine("leaf", LEAF, inc_counter_again, None);
    other_return.action = HookAction::Routine { func: inc_counter_again, ret: ReturnKind::Rtl };
    assert_eq!(reinstall_with(&mut m, other_return), 0, "other return: reset");
    run(&mut m, 1);
    assert_eq!(reinstall_with(&mut m, sync_point("leaf", LEAF)), 0, "sync point: reset");
}

/// The leaf `INC $10` in Rust, declaring two and a half frames.
fn inc_counter_for_two_and_a_half_frames(ctx: &mut Ctx<'_>) {
    inc_counter_uncosted(ctx);
    ctx.set_cost(NTSC_FRAME_MCLK * 5 / 2);
}

fn long_leaf_machine() -> Machine {
    let mut m = machine_with(common::jsr_leaf_rom());
    let leaf = rust_routine(
        "leaf",
        LEAF,
        inc_counter_for_two_and_a_half_frames,
        Some(NTSC_FRAME_MCLK * 5 / 2),
    );
    m.set_hook_table(Some(table([leaf])));
    m
}

#[test]
fn a_call_that_spans_frames_still_gives_one_frame_per_run() {
    // The charge of the call crosses two frame boundaries. The run stops at each one, at the
    // tick that rendered it (the same master cycle in every frame), and the rest of the charge
    // is held by the ticks that follow.
    let mut m = long_leaf_machine();
    let mut ends = Vec::new();
    for frame in 1..=10 {
        m.run_frame(Joypad::default(), &mut NullAudio).unwrap();
        assert_eq!(m.frame_count(), frame, "one frame per run_frame");
        ends.push(m.total_master_cycles());
    }
    for pair in ends.windows(2) {
        // The tick that renders a frame ends a few master cycles apart from one frame to the next.
        assert!(pair[1].abs_diff(pair[0] + NTSC_FRAME_MCLK) <= 12, "{ends:?}");
    }
    let leaf = stats(&m, "leaf");
    assert!(leaf.rust_runs >= 3, "{leaf:?}");
}

#[test]
fn the_charge_left_at_a_frame_end_survives_a_save_state() {
    let mut a = long_leaf_machine();
    run(&mut a, 1);
    // The first call is being held: the leaf ran once, two frames of its charge are left.
    assert_eq!(stats(&a, "leaf").rust_runs, 1);
    assert!(a.charge_pending());
    let state = a.save_state();
    let mut b = long_leaf_machine();
    b.load_state(&state).unwrap();
    for _ in 0..7 {
        run(&mut a, 1);
        run(&mut b, 1);
        assert_eq!(a.total_master_cycles(), b.total_master_cycles());
        assert_eq!(a.wram(), b.wram());
        assert_eq!(format!("{:?}", a.cpu_registers()), format!("{:?}", b.cpu_registers()));
    }
    assert_eq!(a.save_state(), b.save_state());
    // A state made with nothing pending is the core's alone: it loads on a machine that held a
    // charge, which is dropped.
    let idle = machine_with(common::jsr_leaf_rom());
    let plain = idle.save_state();
    b.load_state(&plain).unwrap();
    assert_eq!(b.save_state(), plain);
}
