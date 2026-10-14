//! `Verify` hook mode: the original runs on the live machine, the Rust function on a snapshot of
//! the entry state, and the live machine keeps the original's result.

mod common;

use common::{LEAF, LEAF_COUNTER};
use ssd_oracle::{
    Ctx, Difference, HookAction, HookEntry, HookMode, HookStats, HookTable, Joypad, Machine,
    MemorySaveStore, NullAudio, ReturnKind, RoutineFn, Symbols, TickEvent,
};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The leaf's `RTS` in [`common::jsr_leaf_rom`]: an arrival inside the verified call.
const LEAF_RTS: u32 = 0x00_8042;
/// Where the leaf of [`common::jsr_leaf_rom`] returns to: `LDX #$00C8` after `JSR $8040`.
const LEAF_RETURN: u32 = 0x00_800A;
/// CPU-busy master cycles of the long leaf of [`common::nmi_writer_rom`] and
/// [`common::hdma_leaf_rom`], from the hit to the return (`SlowROM` and WRAM accesses take 8, idle
/// cycles 6): `INC dp` 38, `LDX #imm16` 24, 256 `DEX` of 14, 255 taken `BNE` of 22 and the last
/// one of 16, `RTS` 42.
const LONG_LEAF_COST: u64 = 38 + 24 + 256 * 14 + 255 * 22 + 16 + 42;
const LONG_LEAF_CYCLES: f64 = LONG_LEAF_COST as f64;
/// CPU-busy master cycles of the short leaf (`INC $10; RTS`, `LEAF` in
/// [`common::hdma_cgram_leaf_rom`]): `INC dp` 38, `RTS` 42.
const SHORT_LEAF_COST: u64 = 38 + 42;
/// Divergences kept (and saved) per routine; later ones are only counted.
const KEPT_DIVERGENCES: usize = 32;
/// Differences kept per divergence; the others are only counted (`truncated`).
const KEPT_DIFFERENCES: usize = 256;

/// The mean of equal cycle counts is exact; the tolerance only keeps the comparison float-safe.
fn assert_mean_cycles(actual: f64, expected: f64, what: &str) {
    assert!((actual - expected).abs() < 1e-9, "{what}: mean {actual}, expected {expected}");
}

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

/// A routine in `Verify` mode that returns with RTS, checking every call; contract: 8-bit A,
/// 16-bit X/Y (the fixtures').
fn verify_routine(name: &str, key: u32, func: RoutineFn) -> HookEntry {
    HookEntry {
        name: name.to_owned(),
        key,
        mode: HookMode::Verify,
        action: HookAction::Routine { func, ret: ReturnKind::Rts },
        entry_m8: Some(true),
        entry_x8: Some(false),
        flags_mask: 0xFF,
        verify_sample: 1,
        bound: None,
        budget_frames: None,
        cycles: None,
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

/// The leaf `INC $10` (8-bit memory) in Rust.
fn inc_counter(ctx: &mut Ctx<'_>) {
    let value = ctx.read_dp8(0x10).wrapping_add(1);
    ctx.write_dp8(0x10, value);
    ctx.set_nz8(value);
}

/// A wrong reimplementation of the leaf: adds 2.
fn add_two(ctx: &mut Ctx<'_>) {
    let value = ctx.read_dp8(0x10).wrapping_add(2);
    ctx.write_dp8(0x10, value);
    ctx.set_nz8(value);
}

/// The leaf of [`common::nmi_writer_rom`]: `INC $10`, then the delay loop leaves X = 0 with Z set
/// and N clear (the last `DEX`).
fn inc_counter_and_delay(ctx: &mut Ctx<'_>) {
    inc_counter(ctx);
    ctx.set_x(0);
    ctx.set_nz16(0);
}

/// [`inc_counter`] without the flags: right whenever N and Z are not compared.
fn inc_counter_keeping_flags(ctx: &mut Ctx<'_>) {
    let value = ctx.read_dp8(0x10).wrapping_add(1);
    ctx.write_dp8(0x10, value);
}

/// [`inc_counter`] that also flips bit 8 of A (the B byte), which the original never touches.
fn inc_counter_flipping_b(ctx: &mut Ctx<'_>) {
    inc_counter(ctx);
    ctx.set_a(ctx.a() ^ 0x0100);
}

/// [`inc_counter`] that also inverts 300 WRAM bytes the original never touches.
fn inc_counter_and_clobber_300_bytes(ctx: &mut Ctx<'_>) {
    inc_counter(ctx);
    for address in 0x7E_1000..0x7E_1000 + 300 {
        let value = ctx.read8(address);
        ctx.write8(address, !value);
    }
}

/// The leaf of [`common::stack_leaf_rom`] without its push: `PHX; INC $10; PLX` leaves X as it
/// was, with N and Z from it.
fn stack_leaf(ctx: &mut Ctx<'_>) {
    inc_counter(ctx);
    let x = ctx.x();
    ctx.set_nz16(x);
}

/// [`stack_leaf`] that also inverts a dead stack byte (`$1FED`, where the original's `PHX`
/// leaves the low byte of X, below S after the return) and a live one (`$1FF1`, above it).
fn stack_leaf_writing_around_the_stack(ctx: &mut Ctx<'_>) {
    stack_leaf(ctx);
    for address in [0x00_1FED, 0x00_1FF1] {
        let value = ctx.read8(address);
        ctx.write8(address, !value);
    }
}

/// The leaf of [`common::stack_switch_rom`] writing `fill` to the buffer instead of pushing it.
fn stack_switch_leaf_with(ctx: &mut Ctx<'_>, fill: [u8; 4]) {
    let s = ctx.s();
    for (address, value) in (common::STACK_SWITCH_BUFFER as u32..).zip(fill) {
        ctx.write8(address, value);
    }
    ctx.set_a(0x00F0);
    ctx.set_x(s);
    ctx.set_nz16(0x00F0);
}

fn stack_switch_leaf(ctx: &mut Ctx<'_>) {
    stack_switch_leaf_with(ctx, common::STACK_SWITCH_FILL);
}

/// [`stack_switch_leaf`] with the buffer's bytes in the wrong order.
fn stack_switch_leaf_reversed(ctx: &mut Ctx<'_>) {
    let mut fill = common::STACK_SWITCH_FILL;
    fill.reverse();
    stack_switch_leaf_with(ctx, fill);
}

/// The leaf of [`common::mmio_leaf_rom`].
fn mmio_leaf(ctx: &mut Ctx<'_>) {
    for (address, value) in common::MMIO_LEAF_WRITES {
        ctx.write8(address, value);
    }
    ctx.set_al(0xE0);
    ctx.set_nz8(0xE0);
}

/// [`mmio_leaf`] through mirrors of the registers (banks `$80` and `$BF`).
fn mmio_leaf_in_mirrors(ctx: &mut Ctx<'_>) {
    let [(inidisp, brightness), (coldata, color)] = common::MMIO_LEAF_WRITES;
    ctx.write8(0x80_0000 | inidisp, brightness);
    ctx.write8(0xBF_0000 | coldata, color);
    ctx.set_al(0xE0);
    ctx.set_nz8(0xE0);
}

/// [`mmio_leaf`] writing the two registers in the opposite order: same final state.
fn mmio_leaf_reversed(ctx: &mut Ctx<'_>) {
    for (address, value) in common::MMIO_LEAF_WRITES.into_iter().rev() {
        ctx.write8(address, value);
    }
    ctx.set_al(0xE0);
    ctx.set_nz8(0xE0);
}

/// Symbols from `source`, loaded through a file as the app does.
fn symbols(source: &str) -> Arc<Symbols> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("symbols.toml");
    std::fs::write(&path, source).unwrap();
    Arc::new(Symbols::load(&path).unwrap())
}

/// Names of the files in `dir`, sorted.
fn file_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

/// The leaf of [`common::dma_leaf_rom`]; asserts that the GPDMA requested just before the call
/// is done (the assertion runs inside the dispatch).
fn inc_counter_after_the_copy(ctx: &mut Ctx<'_>) {
    let first = ctx.read8(0x7E_0000 | common::DMA_DESTINATION as u32);
    assert_eq!(first, ctx.read8(0x00_8000), "the transfer ran before the call");
    inc_counter(ctx);
}

#[test]
fn a_correct_routine_verifies() {
    let mut m = machine_with(common::jsr_leaf_rom());
    m.set_hook_table(Some(table([verify_routine("leaf", LEAF, inc_counter)])));
    let before = m.wram()[LEAF_COUNTER];
    run(&mut m, 3);

    let leaf = stats(&m, "leaf");
    assert_eq!(m.frame_count(), 3);
    assert!(leaf.verified_ok > 100, "about 50 calls per frame: {leaf:?}");
    assert_eq!(leaf.divergences, 0, "{:?}", m.divergences().first());
    assert_eq!(m.divergences(), []);
    assert_eq!(leaf.calls, leaf.verified_ok, "every call was checked: {leaf:?}");
    assert_eq!(leaf.rust_runs, 0, "the Rust function never runs on the live machine");
    assert_eq!(
        m.wram()[LEAF_COUNTER],
        before.wrapping_add(leaf.calls as u8),
        "the original ran once per call"
    );
    assert_eq!(leaf.mean_cpu_cycles, 80.0, "INC dp + RTS, from the hit to the return");
    assert_eq!(leaf.hdma_crossed, 0, "no HDMA channel is on");
}

#[test]
fn a_wrong_routine_diverges_and_the_live_machine_keeps_the_original() {
    let dir = tempfile::tempdir().unwrap();
    let mut live = machine_with(common::jsr_leaf_rom());
    live.set_hook_table(Some(table([verify_routine("leaf", LEAF, add_two)])));
    live.set_divergence_dir(Some(dir.path().to_owned()));
    run(&mut live, 3);

    let leaf = stats(&live, "leaf");
    assert!(leaf.divergences > 100, "{leaf:?}");
    assert_eq!(leaf.verified_ok, 0);
    assert_eq!(live.divergences().len(), KEPT_DIVERGENCES, "the first ones are kept");
    for (index, divergence) in live.divergences().iter().enumerate() {
        assert_eq!(divergence.routine, "leaf");
        assert_eq!(divergence.call_index, index as u64, "every call is checked");
        assert!(divergence.frame < 3);
        // The original incremented $10 once, the Rust function twice.
        assert!(
            divergence.differences.iter().any(|difference| matches!(
                *difference,
                Difference::Memory { space: "wram", address, original, rust }
                    if address == LEAF_COUNTER as u32 && rust == original.wrapping_add(1)
            )),
            "{divergence:?}"
        );
    }
    assert_eq!(live.divergences()[0].frame, 0);

    // A pure original run up to the same master cycle is identical: Verify timing is the
    // original's, and the Rust results never reach the live machine.
    let mut twin = machine_with(common::jsr_leaf_rom());
    while twin.total_master_cycles() < live.total_master_cycles() {
        twin.tick(Joypad::default(), &mut NullAudio).unwrap();
    }
    assert_eq!(twin.total_master_cycles(), live.total_master_cycles());
    assert_eq!(live.wram()[LEAF_COUNTER], twin.wram()[LEAF_COUNTER]);
    assert_eq!(live.save_state(), twin.save_state());
}

#[test]
fn a_transfer_requested_before_the_call_is_drained_on_the_snapshot() {
    let mut m = machine_with(common::dma_leaf_rom());
    let leaf = verify_routine("leaf", common::DMA_LEAF, inc_counter_after_the_copy);
    m.set_hook_table(Some(table([leaf])));
    run(&mut m, 3);

    let leaf = stats(&m, "leaf");
    assert!(leaf.verified_ok > 100, "about 45 calls per frame: {leaf:?}");
    assert_eq!(leaf.divergences, 0, "{:?}", m.divergences().first());
}

#[test]
fn an_original_that_never_returns_is_demoted() {
    let start = Instant::now();
    let mut m = machine_with(common::no_return_rom());
    m.set_hook_table(Some(table([verify_routine("leaf", LEAF, inc_counter)])));
    let power_on = m.save_state();

    run(&mut m, 1);
    assert_eq!(m.frame_count(), 2, "the 2-frame budget ran inside one dispatch, counted once");
    run(&mut m, 2);
    let leaf = stats(&m, "leaf");
    assert_eq!(leaf.no_return, 1, "{leaf:?}");
    assert_eq!(leaf.calls, 1, "the leaf's own arrivals are not dispatched: {leaf:?}");
    assert_eq!(leaf.reentered, 0, "a loop back to the entry is no re-entry: {leaf:?}");
    assert_eq!(leaf.verified_ok + leaf.divergences + leaf.interrupted, 0, "{leaf:?}");

    // Demoted for the rest of the run: the same arrival after a load is not dispatched.
    m.load_state(&power_on).unwrap();
    run(&mut m, 3);
    assert_eq!(stats(&m, "leaf"), leaf);
    assert!(start.elapsed() < Duration::from_secs(2), "{:?}", start.elapsed());
}

#[test]
fn interrupted_calls_are_counted_and_not_compared() {
    let mut m = machine_with(common::nmi_writer_rom());
    m.set_hook_table(Some(table([verify_routine("leaf", LEAF, inc_counter_and_delay)])));
    let handler_writes = m.wram()[common::NMI_WRITER_TARGET];
    run(&mut m, 3);

    let leaf = stats(&m, "leaf");
    assert!(leaf.interrupted > 0, "{leaf:?}");
    assert!(leaf.verified_ok > 0, "{leaf:?}");
    assert_eq!(leaf.divergences, 0, "{:?}", m.divergences().first());
    assert_eq!(leaf.calls, leaf.verified_ok + leaf.interrupted, "{leaf:?}");
    assert_ne!(m.wram()[common::NMI_WRITER_TARGET], handler_writes, "the NMI handler ran");
    assert_eq!(leaf.hdma_crossed, 0, "no HDMA channel is on, whatever the frame start does");
    assert_mean_cycles(
        leaf.mean_cpu_cycles,
        LONG_LEAF_CYCLES,
        "interrupted calls are not measured",
    );
}

#[test]
fn sampling_checks_one_call_in_n() {
    let mut leaf = verify_routine("leaf", LEAF, inc_counter);
    leaf.verify_sample = 10;
    let mut m = machine_with(common::jsr_leaf_rom());
    m.set_hook_table(Some(table([leaf])));
    let before = m.wram()[LEAF_COUNTER];
    run(&mut m, 3);

    let leaf = stats(&m, "leaf");
    assert!(leaf.verified_ok > 10, "{leaf:?}");
    assert_eq!(leaf.calls, leaf.verified_ok + leaf.sampled_out, "{leaf:?}");
    assert_eq!(leaf.verified_ok, leaf.calls.div_ceil(10), "the first call of every ten");
    assert!(leaf.sampled_out.abs_diff(9 * leaf.verified_ok) <= 9, "{leaf:?}");
    assert_eq!(m.wram()[LEAF_COUNTER], before.wrapping_add(leaf.calls as u8));
}

#[test]
fn sync_points_inside_a_verified_call_run_as_original_and_its_return_is_reported() {
    let mut m = machine_with(common::jsr_leaf_rom());
    m.set_hook_table(Some(table([
        verify_routine("leaf", LEAF, inc_counter),
        sync_point("rts", LEAF_RTS),
        sync_point("return", LEAF_RETURN),
    ])));
    run(&mut m, 3);

    let leaf = stats(&m, "leaf");
    assert!(leaf.verified_ok > 100, "{leaf:?}");
    assert_eq!(stats(&m, "rts").calls, 0, "sync points inside a verified call are not dispatched");
    // The return arrival is recorded on the call's last tick; the last one may still be pending.
    let returns = stats(&m, "return").calls;
    assert!(returns == leaf.calls || returns + 1 == leaf.calls, "{returns} for {leaf:?}");
}

/// The state at the entry of a call of the leaf of [`common::nmi_leaf_rom`] whose return is
/// followed by the NMI: the RTS's last cycle sees the NMI, so the CPU takes it right after.
fn entry_of_a_call_returning_into_the_nmi() -> Vec<u8> {
    let leaf_machine = || {
        let mut m = machine_with(common::nmi_leaf_rom());
        m.set_hook_table(Some(table([sync_point("leaf", LEAF)])));
        m
    };
    // Find the call, then reach its entry again on a fresh machine (saving every entry is slow).
    let mut m = leaf_machine();
    let mut calls = 0;
    let call = loop {
        assert!(m.frame_count() < 120, "no call returned into the NMI");
        if tick(&mut m) != TickEvent::Hook(LEAF) {
            continue;
        }
        calls += 1;
        let s_entry = m.cpu_registers().s;
        while m.is_mid_instruction() || m.cpu_registers().s <= s_entry {
            tick(&mut m);
        }
        // Returned. The next CPU cycle (after any refresh) enters the NMI or fetches an opcode.
        while !m.is_mid_instruction() {
            tick(&mut m);
        }
        if m.is_handling_interrupt() {
            break calls;
        }
    };
    let mut m = leaf_machine();
    for _ in 0..call {
        while tick(&mut m) != TickEvent::Hook(LEAF) {}
    }
    m.save_state()
}

#[test]
fn a_call_that_returns_with_the_nmi_pending_is_compared() {
    let entry = entry_of_a_call_returning_into_the_nmi();
    let mut m = machine_with(common::nmi_leaf_rom());
    m.set_hook_table(Some(table([verify_routine("leaf", LEAF, inc_counter)])));
    // The loaded state is at the hooked fetch: the first tick reports the arrival.
    m.load_state(&entry).unwrap();
    let nmis = m.wram()[common::NMI_COUNTER];
    run(&mut m, 1);

    let leaf = stats(&m, "leaf");
    assert_eq!((leaf.calls, leaf.verified_ok), (1, 1), "{leaf:?}");
    assert_eq!(leaf.divergences, 0, "{:?}", m.divergences().first());
    // The frame was rendered during the call, so run_frame returned right after it.
    assert_eq!(m.cpu_registers().pc, common::NMI_LEAF_RETURN);
    assert_eq!(m.wram()[common::NMI_COUNTER], nmis, "the NMI is pending, not taken");
    while !m.at_fetch_boundary() {
        tick(&mut m);
    }
    assert_eq!(m.cpu_registers().pc, common::NMI_HANDLER, "the NMI comes first");
}

#[test]
fn a_contract_mismatch_resumes_the_original_unchecked() {
    let mut leaf = verify_routine("leaf", LEAF, add_two);
    leaf.entry_m8 = Some(false); // the fixture calls the leaf with an 8-bit accumulator
    let mut m = machine_with(common::jsr_leaf_rom());
    m.set_hook_table(Some(table([leaf])));
    let before = m.wram()[LEAF_COUNTER];
    run(&mut m, 3);

    let leaf = stats(&m, "leaf");
    assert!(leaf.contract_mismatch > 100, "{leaf:?}");
    assert_eq!(leaf.calls, leaf.contract_mismatch, "{leaf:?}");
    assert_eq!(leaf.verified_ok + leaf.divergences + leaf.interrupted + leaf.sampled_out, 0);
    assert_eq!(m.divergences(), []);
    assert_eq!(m.wram()[LEAF_COUNTER], before.wrapping_add(leaf.calls as u8), "original ran");
}

#[test]
fn registers_are_compared_in_full_and_flags_under_the_mask() {
    // The leaf leaves N and Z from the increment; this version keeps the caller's.
    let mut m = machine_with(common::jsr_leaf_rom());
    m.set_hook_table(Some(table([verify_routine("leaf", LEAF, inc_counter_keeping_flags)])));
    run(&mut m, 3);
    assert!(stats(&m, "leaf").divergences > 100, "{:?}", stats(&m, "leaf"));
    for divergence in m.divergences() {
        let [Difference::Register { name: "P", original, rust }] = divergence.differences[..]
        else {
            panic!("only P differs: {divergence:?}");
        };
        assert_eq!((original ^ rust) & !0x82, 0, "only N and Z differ: {divergence}");
    }

    let mut leaf = verify_routine("leaf", LEAF, inc_counter_keeping_flags);
    leaf.flags_mask = !0x82; // N and Z not compared
    let mut m = machine_with(common::jsr_leaf_rom());
    m.set_hook_table(Some(table([leaf])));
    run(&mut m, 3);
    let leaf = stats(&m, "leaf");
    assert!(leaf.verified_ok > 100, "{leaf:?}");
    assert_eq!(leaf.divergences, 0, "{:?}", m.divergences().first());

    // Other registers are compared in full, B included.
    let mut m = machine_with(common::jsr_leaf_rom());
    m.set_hook_table(Some(table([verify_routine("leaf", LEAF, inc_counter_flipping_b)])));
    run(&mut m, 1);
    assert_ne!(m.divergences(), []);
    for divergence in m.divergences() {
        let [Difference::Register { name: "A", original, rust }] = divergence.differences[..]
        else {
            panic!("only A differs: {divergence:?}");
        };
        assert_eq!(original ^ rust, 0x0100, "{divergence}");
    }
}

#[test]
fn stack_bytes_left_dead_by_the_original_are_not_compared() {
    let mut m = machine_with(common::stack_leaf_rom());
    m.set_hook_table(Some(table([verify_routine("leaf", LEAF, stack_leaf)])));
    let nmis = m.wram()[common::NMI_COUNTER];
    run(&mut m, 4);

    // The NMI handler, between calls, overwrites the bytes the leaf pushes X to; the next
    // call's `PHX` writes them again, the Rust function (no push) does not.
    assert!(m.wram()[common::NMI_COUNTER].wrapping_sub(nmis) >= 3, "the NMI ran");
    let leaf = stats(&m, "leaf");
    assert!(leaf.verified_ok > 100, "{leaf:?}");
    let first = m.divergences().first().map(ToString::to_string);
    assert_eq!(leaf.divergences, 0, "{}", first.unwrap_or_default());
    assert_eq!(leaf.calls, leaf.verified_ok + leaf.interrupted, "{leaf:?}");
    assert_eq!(leaf.stack_switched, 0, "pushes and pulls do not move the stack: {leaf:?}");
}

#[test]
fn a_wrong_live_byte_next_to_the_dead_stack_is_still_caught() {
    let mut m = machine_with(common::stack_leaf_rom());
    let leaf = verify_routine("leaf", LEAF, stack_leaf_writing_around_the_stack);
    m.set_hook_table(Some(table([leaf])));
    run(&mut m, 2);

    let leaf = stats(&m, "leaf");
    assert_eq!(leaf.verified_ok, 0, "{leaf:?}");
    assert!(leaf.divergences > 50, "{leaf:?}");
    for divergence in m.divergences() {
        let [Difference::Memory { space: "wram", address: 0x1FF1, original, rust }] =
            divergence.differences[..]
        else {
            panic!("only the live byte differs: {divergence}");
        };
        assert_eq!(rust, !original);
    }
}

#[test]
fn a_buffer_filled_through_a_moved_stack_is_compared() {
    // The leaf moves S below its frame to fill a buffer with pushes: those bytes are output,
    // not dead stack.
    let mut m = machine_with(common::stack_switch_rom());
    m.set_hook_table(Some(table([verify_routine("leaf", LEAF, stack_switch_leaf)])));
    run(&mut m, 2);
    let leaf = stats(&m, "leaf");
    assert!(leaf.verified_ok > 50, "{leaf:?}");
    assert_eq!(leaf.divergences, 0, "{:?}", m.divergences().first());
    assert_eq!(leaf.stack_switched, leaf.calls, "{leaf:?}");
    let buffer = common::STACK_SWITCH_BUFFER;
    assert_eq!(m.wram()[buffer..buffer + 4], common::STACK_SWITCH_FILL);

    let mut m = machine_with(common::stack_switch_rom());
    m.set_hook_table(Some(table([verify_routine("leaf", LEAF, stack_switch_leaf_reversed)])));
    run(&mut m, 2);
    let leaf = stats(&m, "leaf");
    assert_eq!(leaf.verified_ok, 0, "{leaf:?}");
    assert!(leaf.divergences > 50, "{leaf:?}");
    for divergence in m.divergences() {
        let addresses: Vec<u32> = divergence
            .differences
            .iter()
            .map(|difference| match *difference {
                Difference::Memory { space: "wram", address, .. } => address,
                other => panic!("only the buffer differs: {other:?}"),
            })
            .collect();
        let buffer = buffer as u32;
        assert_eq!(addresses, [buffer, buffer + 1, buffer + 2, buffer + 3], "{divergence}");
    }
}

#[test]
fn a_report_that_cannot_be_saved_does_not_stop_the_run() {
    let dir = tempfile::tempdir().unwrap();
    let not_a_directory = dir.path().join("divergences");
    std::fs::write(&not_a_directory, b"a file").unwrap();
    let mut m = machine_with(common::jsr_leaf_rom());
    m.set_hook_table(Some(table([verify_routine("leaf", LEAF, add_two)])));
    m.set_divergence_dir(Some(not_a_directory));
    run(&mut m, 3);

    assert_eq!(m.frame_count(), 3);
    let leaf = stats(&m, "leaf");
    assert!(leaf.divergences > 100, "{leaf:?}");
    assert_eq!(m.divergences().len(), KEPT_DIVERGENCES, "kept although not saved");
    assert_eq!(leaf.report_errors, KEPT_DIVERGENCES as u64, "{leaf:?}");
}

#[test]
fn calls_crossed_by_hdma_are_counted() {
    let mut m = machine_with(common::hdma_leaf_rom());
    m.set_hook_table(Some(table([verify_routine("leaf", LEAF, inc_counter_and_delay)])));
    run(&mut m, 3);

    let leaf = stats(&m, "leaf");
    assert_eq!(leaf.divergences, 0, "{:?}", m.divergences().first());
    assert_eq!(leaf.calls, leaf.verified_ok, "{leaf:?}");
    // HDMA copies one byte per visible scanline; each call lasts about 7 scanlines, so calls in
    // the active display cross it and the 4 or 5 calls of each vblank (37 lines) do not.
    assert!(leaf.hdma_crossed > 60, "{leaf:?}");
    assert!(leaf.hdma_crossed + 8 < leaf.calls, "{leaf:?}");
    assert_mean_cycles(leaf.mean_cpu_cycles, LONG_LEAF_CYCLES, "HDMA cycles are not CPU-busy");
}

/// [`inc_counter`] that takes about two scanlines longer than the original leaf, so HDMA runs
/// on the snapshot while it does.
fn inc_counter_and_dawdle(ctx: &mut Ctx<'_>) {
    inc_counter(ctx);
    ctx.charge(3000);
}

/// [`inc_counter_and_delay`] declaring the exact cost of the long leaf.
fn long_leaf_with_its_cost(ctx: &mut Ctx<'_>) {
    inc_counter_and_delay(ctx);
    ctx.set_cost(LONG_LEAF_COST);
}

/// [`inc_counter_and_delay`] declaring a cost 2 master cycles above the long leaf's.
fn long_leaf_with_a_cost_2_too_high(ctx: &mut Ctx<'_>) {
    inc_counter_and_delay(ctx);
    ctx.set_cost(LONG_LEAF_COST + 2);
}

#[test]
fn the_cost_of_a_call_crossed_by_hdma_is_compared_like_any_other() {
    // About 7 scanlines a call, so most calls in the display are crossed. HDMA steals bus time,
    // not CPU-busy cycles, and the declared cost is CPU-busy cycles too: the exact cost matches
    // whether HDMA crossed the call or not, a wrong one is a mismatch in every call.
    let mut m = machine_with(common::hdma_leaf_rom());
    m.set_hook_table(Some(table([verify_routine("leaf", LEAF, long_leaf_with_its_cost)])));
    run(&mut m, 3);
    let leaf = stats(&m, "leaf");
    assert_eq!(leaf.divergences, 0, "{:?}", m.divergences().first());
    assert!(leaf.hdma_crossed > 60 && leaf.hdma_crossed < leaf.calls, "{leaf:?}");
    assert_eq!((leaf.cost_mismatch, leaf.cost_missing), (0, 0), "{leaf:?}");

    let mut m = machine_with(common::hdma_leaf_rom());
    m.set_hook_table(Some(table([verify_routine("leaf", LEAF, long_leaf_with_a_cost_2_too_high)])));
    run(&mut m, 3);
    let leaf = stats(&m, "leaf");
    assert!(leaf.hdma_crossed > 60 && leaf.hdma_crossed < leaf.calls, "{leaf:?}");
    assert_eq!(leaf.cost_mismatch, leaf.calls, "the crossed calls too: {leaf:?}");
    assert_eq!(leaf.calls, leaf.verified_ok, "{leaf:?}");
}

/// [`inc_counter_and_dawdle`] declaring the short leaf's exact cost.
fn short_leaf_dawdling_with_its_cost(ctx: &mut Ctx<'_>) {
    inc_counter_and_dawdle(ctx);
    ctx.set_cost(SHORT_LEAF_COST);
}

/// [`inc_counter_and_dawdle`] declaring a cost 2 master cycles above the short leaf's.
fn short_leaf_dawdling_with_a_cost_2_too_high(ctx: &mut Ctx<'_>) {
    inc_counter_and_dawdle(ctx);
    ctx.set_cost(SHORT_LEAF_COST + 2);
}

#[test]
fn the_cost_is_compared_when_hdma_crosses_the_snapshot_only() {
    // The original leaf lasts 80 master cycles and is rarely crossed; the Rust function charges
    // 3000 and is, in the display. The declared cost is what is compared, not the time charged.
    let mut m = machine_with(common::hdma_cgram_leaf_rom());
    m.set_hook_table(Some(table([verify_routine(
        "leaf",
        LEAF,
        short_leaf_dawdling_with_its_cost,
    )])));
    run(&mut m, 4);
    let leaf = stats(&m, "leaf");
    assert_eq!(leaf.divergences, 0, "{:?}", m.divergences().first());
    assert!(leaf.hdma_crossed * 10 > leaf.calls * 7, "{leaf:?}");
    assert_eq!((leaf.cost_mismatch, leaf.cost_missing), (0, 0), "{leaf:?}");

    let mut m = machine_with(common::hdma_cgram_leaf_rom());
    m.set_hook_table(Some(table([verify_routine(
        "leaf",
        LEAF,
        short_leaf_dawdling_with_a_cost_2_too_high,
    )])));
    run(&mut m, 4);
    let leaf = stats(&m, "leaf");
    assert!(leaf.hdma_crossed * 10 > leaf.calls * 7, "{leaf:?}");
    assert_eq!(leaf.cost_mismatch, leaf.calls, "the crossed calls too: {leaf:?}");
}

#[test]
fn hdma_copying_while_the_rust_function_runs_excuses_the_ppu_memories() {
    // The original leaf lasts 80 master cycles and rarely meets an HDMA transfer, the Rust one
    // charges 3000 and does in the display: HDMA writes palette RAM on the snapshot only.
    let mut m = machine_with(common::hdma_cgram_leaf_rom());
    m.set_hook_table(Some(table([verify_routine("leaf", LEAF, inc_counter_and_dawdle)])));
    run(&mut m, 4);

    let leaf = stats(&m, "leaf");
    assert!(leaf.calls > 100, "{leaf:?}");
    assert_eq!(
        leaf.divergences,
        0,
        "{}",
        m.divergences().first().map(ToString::to_string).unwrap_or_default()
    );
    assert_eq!(leaf.calls, leaf.verified_ok, "{leaf:?}");
    // Most calls are in the display (225 of 262 lines), and all of those are crossed.
    assert!(leaf.hdma_crossed * 10 > leaf.calls * 7, "{leaf:?}");
    assert!(leaf.hdma_crossed < leaf.calls, "the calls in vblank are not: {leaf:?}");
    // The live machine kept the original's result: HDMA on the live machine alone.
    let mut twin = machine_with(common::hdma_cgram_leaf_rom());
    while twin.total_master_cycles() < m.total_master_cycles() {
        tick(&mut twin);
    }
    assert_eq!(m.save_state(), twin.save_state());
}

#[test]
fn the_order_of_observable_writes_is_compared() {
    let mut m = machine_with(common::mmio_leaf_rom());
    m.set_hook_table(Some(table([verify_routine("leaf", LEAF, mmio_leaf_reversed)])));
    run(&mut m, 2);
    let [first, second] = common::MMIO_LEAF_WRITES;
    assert!(stats(&m, "leaf").divergences > 50, "{:?}", stats(&m, "leaf"));
    for divergence in m.divergences() {
        assert_eq!(
            divergence.differences,
            [
                Difference::Write { index: 0, original: Some(first), rust: Some(second) },
                Difference::Write { index: 1, original: Some(second), rust: Some(first) },
            ],
            "{divergence}"
        );
    }

    // Mirrors of the registers are the same registers.
    for func in [mmio_leaf as RoutineFn, mmio_leaf_in_mirrors] {
        let mut m = machine_with(common::mmio_leaf_rom());
        m.set_hook_table(Some(table([verify_routine("leaf", LEAF, func)])));
        run(&mut m, 2);
        let leaf = stats(&m, "leaf");
        assert!(leaf.verified_ok > 50, "{leaf:?}");
        assert_eq!(leaf.divergences, 0, "{:?}", m.divergences().first());
    }
}

#[test]
fn verify_leaves_the_users_cpu_write_log_as_without_hooks() {
    let mut live = machine_with(common::mmio_leaf_rom());
    live.set_hook_table(Some(table([verify_routine("leaf", LEAF, mmio_leaf)])));
    live.set_cpu_write_log(true);
    let mut live_writes = Vec::new();
    for _ in 0..2 {
        live.run_frame(Joypad::default(), &mut NullAudio).unwrap();
        live_writes.extend(live.take_cpu_writes());
    }
    assert!(stats(&live, "leaf").verified_ok > 50, "{:?}", stats(&live, "leaf"));

    let mut twin = machine_with(common::mmio_leaf_rom());
    twin.set_cpu_write_log(true);
    while twin.total_master_cycles() < live.total_master_cycles() {
        twin.tick(Joypad::default(), &mut NullAudio).unwrap();
    }
    assert_eq!(twin.total_master_cycles(), live.total_master_cycles());
    let twin_writes = twin.take_cpu_writes();
    assert!(twin_writes.len() > 100, "the leaf writes two registers per call");
    assert_eq!(live_writes, twin_writes);

    // With the log off, Verify's own recording does not leak into it.
    let mut m = machine_with(common::mmio_leaf_rom());
    m.set_hook_table(Some(table([verify_routine("leaf", LEAF, mmio_leaf)])));
    run(&mut m, 2);
    assert!(stats(&m, "leaf").verified_ok > 50);
    assert_eq!(m.take_cpu_writes(), []);
}

#[test]
fn the_first_32_divergences_of_a_routine_are_kept_and_saved() {
    let dir = tempfile::tempdir().unwrap();
    let mut m = machine_with(common::jsr_leaf_rom());
    m.set_hook_table(Some(table([verify_routine("leaf", LEAF, add_two)])));
    m.set_divergence_dir(Some(dir.path().to_owned()));
    run(&mut m, 3);

    let leaf = stats(&m, "leaf");
    assert!(leaf.divergences > 100, "every call is counted: {leaf:?}");
    assert_eq!(m.divergences().len(), KEPT_DIVERGENCES);
    for (index, divergence) in m.divergences().iter().enumerate() {
        assert_eq!(divergence.call_index, index as u64, "the first ones");
    }
    let mut expected: Vec<String> = m
        .divergences()
        .iter()
        .flat_map(|d| {
            let stem = format!("leaf-{}-{}", d.frame, d.call_index);
            [format!("{stem}.ssdstate"), format!("{stem}.txt")]
        })
        .collect();
    expected.sort();
    assert_eq!(file_names(dir.path()), expected);
}

#[test]
fn a_saved_entry_state_reproduces_the_divergence() {
    let dir = tempfile::tempdir().unwrap();
    let mut live = machine_with(common::jsr_leaf_rom());
    live.set_hook_table(Some(table([verify_routine("leaf", LEAF, add_two)])));
    live.set_divergence_dir(Some(dir.path().to_owned()));
    run(&mut live, 1);
    let first = live.divergences()[0].clone();
    let stem = dir.path().join(format!("leaf-{}-{}", first.frame, first.call_index));
    let text = std::fs::read_to_string(stem.with_extension("txt")).unwrap();
    assert_eq!(text, format!("ssd-repro: key=$008040 routine=leaf\n{first}"));

    // The state is at the call's hooked opcode fetch: loading it replays the call.
    let mut replay = machine_with(common::jsr_leaf_rom());
    replay.set_hook_table(Some(table([verify_routine("leaf", LEAF, add_two)])));
    replay.load_state(&std::fs::read(stem.with_extension("ssdstate")).unwrap()).unwrap();
    assert_eq!(replay.frame_count(), first.frame);
    let registers = replay.cpu_registers();
    assert_eq!((u32::from(registers.pbr) << 16) | u32::from(registers.pc), LEAF);
    assert!(replay.at_fetch_boundary());
    run(&mut replay, 1);
    let again = &replay.divergences()[0];
    assert_eq!((again.frame, &again.differences), (first.frame, &first.differences));
}

#[test]
fn a_divergence_keeps_256_differences_and_counts_the_rest() {
    let mut m = machine_with(common::jsr_leaf_rom());
    let leaf = verify_routine("leaf", LEAF, inc_counter_and_clobber_300_bytes);
    m.set_hook_table(Some(table([leaf])));
    run(&mut m, 1);

    let divergence = &m.divergences()[0];
    assert_eq!(divergence.differences.len(), KEPT_DIFFERENCES);
    assert_eq!(divergence.truncated, 300 - KEPT_DIFFERENCES);
    assert!(divergence.to_string().contains("44 more differences"), "{divergence}");
}

#[test]
fn divergence_reports_name_variables() {
    let dir = tempfile::tempdir().unwrap();
    let mut m = machine_with(common::jsr_leaf_rom());
    m.set_hook_table(Some(table([verify_routine("leaf", LEAF, add_two)])));
    m.set_symbols(Some(symbols(
        "[[variable]]\nname = \"leaf_counter\"\naddress = 0x7E0010\nsize = 1\n",
    )));
    m.set_divergence_dir(Some(dir.path().to_owned()));
    run(&mut m, 1);

    let divergence = &m.divergences()[0];
    let text = divergence.to_string();
    assert!(text.contains("leaf — wram[$7E:0010] leaf_counter: $"), "{text}");
    let stem = format!("leaf-{}-{}", divergence.frame, divergence.call_index);
    let saved = std::fs::read_to_string(dir.path().join(stem).with_extension("txt")).unwrap();
    assert_eq!(saved, format!("ssd-repro: key=$008040 routine=leaf\n{text}"));
}

fn run_until_call(m: &mut Machine, address: u32, budget_frames: u64) -> bool {
    m.run_until_call(address, &mut |_| Joypad::default(), &mut NullAudio, budget_frames).unwrap()
}

#[test]
fn run_until_call_stops_after_the_first_dispatched_call() {
    let mut m = machine_with(common::jsr_leaf_rom());
    m.set_hook_table(Some(table([verify_routine("leaf", LEAF, inc_counter)])));
    let before = m.wram()[LEAF_COUNTER];
    assert!(run_until_call(&mut m, LEAF, 2));
    let leaf = stats(&m, "leaf");
    assert_eq!((leaf.calls, leaf.verified_ok), (1, 1), "{leaf:?}");
    assert_eq!(m.wram()[LEAF_COUNTER], before.wrapping_add(1), "the original ran, once");
    assert_eq!(m.frame_count(), 0, "well inside the first frame");

    // The next call is the next arrival, through a mirror of the address as well.
    assert!(run_until_call(&mut m, 0x80_0000 | LEAF, 2));
    assert_eq!((stats(&m, "leaf").calls, m.wram()[LEAF_COUNTER]), (2, before.wrapping_add(2)));
}

#[test]
fn run_until_call_dispatches_the_other_hooked_arrivals_on_the_way() {
    let mut m = machine_with(common::jsr_leaf_rom());
    // The `JSR` that precedes every call is a sync point: reported, original resumed.
    m.set_hook_table(Some(table([
        sync_point("before", 0x00_8007),
        verify_routine("leaf", LEAF, inc_counter),
    ])));
    assert!(run_until_call(&mut m, LEAF, 2));
    assert_eq!((stats(&m, "before").calls, stats(&m, "leaf").calls), (1, 1));
}

#[test]
fn run_until_call_gives_up_after_the_budget_in_frames() {
    let mut m = machine_with(common::jsr_leaf_rom());
    // Hooked, but code the loop never runs.
    let never = 0x00_8050;
    m.set_hook_table(Some(table([verify_routine("never", never, inc_counter)])));
    assert!(!run_until_call(&mut m, never, 2));
    assert_eq!(m.frame_count(), 2);
    assert_eq!(stats(&m, "never").calls, 0);

    // A budget of 0 does not run at all.
    let cycles = m.total_master_cycles();
    assert!(!run_until_call(&mut m, never, 0));
    assert_eq!(m.total_master_cycles(), cycles);
}
