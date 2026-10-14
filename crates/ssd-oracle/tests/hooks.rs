mod common;

use common::{LEAF, LEAF_COUNTER};
use ssd_oracle::{
    HookAction, HookEntry, HookMode, HookTable, Joypad, Machine, MemorySaveStore, NullAudio,
    TickEvent,
};

/// The idle loop of [`common::red_backdrop_rom`]: `BRA $801A`.
const IDLE_LOOP: u32 = 0x00_801A;

fn machine_with(rom: Vec<u8>) -> Machine {
    Machine::new(rom, Box::new(MemorySaveStore::default())).unwrap()
}

/// Hooks `addresses` as sync points: every arrival is reported and the original code resumes.
fn sync_points(addresses: &[u32]) -> HookTable {
    addresses
        .iter()
        .map(|&key| HookEntry {
            name: format!("sync_{key:06x}"),
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
        })
        .collect()
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

fn run(m: &mut Machine, frames: usize) {
    for _ in 0..frames {
        m.run_frame(Joypad::default(), &mut NullAudio).unwrap();
    }
}

#[test]
fn hook_fires_before_the_routines_first_instruction() {
    let mut m = machine_with(common::jsr_leaf_rom());
    m.set_hook_table(Some(sync_points(&[LEAF])));
    tick_until(&mut m, 2, |_, event| event == TickEvent::Hook(LEAF));
    assert_eq!(m.cpu_registers().pc, 0x8040);
    assert!(m.at_fetch_boundary());
    let before = m.wram()[LEAF_COUNTER];

    m.set_hook_table(None);
    tick_until(&mut m, 1, |m, _| m.at_fetch_boundary() && m.cpu_registers().pc == 0x8042);
    assert_eq!(m.wram()[LEAF_COUNTER], before.wrapping_add(1), "INC $10 ran after the hit");
}

#[test]
fn every_arrival_hits_exactly_once() {
    let mut m = machine_with(common::jsr_leaf_rom());
    m.set_hook_table(Some(sync_points(&[LEAF])));
    let mut hits = 0;
    let mut at_first_hit = 0;
    tick_until(&mut m, 5, |m, event| {
        if event == TickEvent::Hook(LEAF) {
            hits += 1;
            if hits == 1 {
                at_first_hit = m.wram()[LEAF_COUNTER];
            }
        }
        hits == 101
    });
    // Each hit comes before its call's INC: 100 calls completed between hits 1 and 101.
    assert_eq!(m.wram()[LEAF_COUNTER], at_first_hit.wrapping_add(100));
}

#[test]
fn an_arrival_while_the_dma_unit_has_nothing_to_do_is_reported() {
    // With no HDMA channel on, the DMA unit is not idle for one tick at the start of every
    // frame (the reload check), and its next tick takes no cycle. An arrival then is reported
    // like any other: the arrivals at the idle loop come 22 master cycles apart (`BRA`), 62 with
    // a memory refresh between them, never more.
    let mut m = machine_with(common::red_backdrop_rom());
    m.set_hook_table(Some(sync_points(&[IDLE_LOOP])));
    let mut last = None;
    let mut gaps = std::collections::BTreeSet::new();
    while m.frame_count() < 12 {
        if tick(&mut m) == TickEvent::Hook(IDLE_LOOP) {
            let now = m.total_master_cycles();
            gaps.extend(last.map(|last| now - last));
            last = Some(now);
        }
    }
    assert_eq!(gaps, [22, 62].into());
}

#[test]
fn only_opcode_fetches_hit() {
    // $8008 (JSR operand) and $8041 (INC operand) are PC values in the middle of an instruction.
    let mut m = machine_with(common::jsr_leaf_rom());
    m.set_hook_table(Some(sync_points(&[0x00_8008, 0x00_8041])));
    let frames = m.frame_count();
    while m.frame_count() < frames + 2 {
        assert!(!matches!(tick(&mut m), TickEvent::Hook(_)));
    }
}

#[test]
fn held_ticks_report_no_hit() {
    let mut m = machine_with(common::jsr_leaf_rom());
    m.set_hook_table(Some(sync_points(&[LEAF])));
    tick_until(&mut m, 2, |_, event| event == TickEvent::Hook(LEAF));
    let before = m.wram()[LEAF_COUNTER];

    // The CPU stays at the hooked fetch while held; that is not a new arrival.
    m.advance_idle(Joypad::default(), &mut NullAudio, 1000).unwrap();
    assert!(m.at_fetch_boundary() && m.cpu_registers().pc == 0x8040);
    assert_eq!(tick(&mut m), TickEvent::None, "the next tick fetches the opcode");
    tick_until(&mut m, 1, |m, _| m.at_fetch_boundary() && m.cpu_registers().pc == 0x8042);
    assert_eq!(m.wram()[LEAF_COUNTER], before.wrapping_add(1));
}

#[test]
fn hook_events_do_not_advance_the_machine() {
    let mut m = machine_with(common::jsr_leaf_rom());
    m.set_hook_table(Some(sync_points(&[LEAF])));
    let mut hits = 0;
    for _ in 0..3 {
        loop {
            let cycles = m.total_master_cycles();
            let event = tick(&mut m);
            match event {
                TickEvent::Hook(_) => {
                    hits += 1;
                    assert_eq!(m.total_master_cycles(), cycles, "a hook event does not tick");
                }
                TickEvent::None => assert!(m.total_master_cycles() > cycles),
                TickEvent::Frame => break,
            }
        }
    }
    assert!((120..=180).contains(&hits), "about 50 calls per frame, got {hits} in 3 frames");
}

#[test]
fn a_tick_that_renders_and_hits_yields_the_frame_first() {
    // The red backdrop ROM idles on `BRA $801A`, so most frames end at the hooked loop.
    let mut m = machine_with(common::red_backdrop_rom());
    m.set_hook_table(Some(sync_points(&[IDLE_LOOP])));
    for _ in 0..30 {
        while tick(&mut m) != TickEvent::Frame {}
        if m.at_fetch_boundary() && m.cpu_registers().pc == 0x801A {
            let cycles = m.total_master_cycles();
            assert_eq!(tick(&mut m), TickEvent::Hook(IDLE_LOOP), "the hit comes next");
            assert_eq!(m.total_master_cycles(), cycles);
            return;
        }
    }
    panic!("no frame was rendered by a tick that ended at the idle loop");
}

#[test]
fn run_frame_resumes_hooked_routines_as_original() {
    let mut hooked = machine_with(common::jsr_leaf_rom());
    let mut plain = machine_with(common::jsr_leaf_rom());
    hooked.set_hook_table(Some(sync_points(&[LEAF])));
    run(&mut hooked, 5);
    run(&mut plain, 5);
    assert_eq!(hooked.frame_count(), 5);
    assert_eq!(hooked.save_state(), plain.save_state(), "hits do not perturb emulation");
    assert_eq!(hooked.opcode_fetches(), plain.opcode_fetches());
}

#[test]
fn hooks_still_fire_after_load_state() {
    let mut m = machine_with(common::jsr_leaf_rom());
    m.set_hook_table(Some(sync_points(&[LEAF])));
    run(&mut m, 1);
    let state = m.save_state();
    run(&mut m, 1);
    m.load_state(&state).unwrap();
    tick_until(&mut m, 1, |_, event| event == TickEvent::Hook(LEAF));
}

#[test]
fn removed_hooks_stay_removed_after_load_state() {
    let mut m = machine_with(common::jsr_leaf_rom());
    m.set_hook_table(Some(sync_points(&[LEAF])));
    run(&mut m, 1);
    let state = m.save_state();
    m.set_hook_table(None);
    m.load_state(&state).unwrap();
    let frames = m.frame_count();
    while m.frame_count() < frames + 2 {
        assert!(!matches!(tick(&mut m), TickEvent::Hook(_)));
    }
}

#[test]
fn snapshot_keeps_the_hook_table() {
    let mut m = machine_with(common::jsr_leaf_rom());
    m.set_hook_table(Some(sync_points(&[LEAF])));
    run(&mut m, 1);
    let mut snap = m.snapshot();
    tick_until(&mut snap, 1, |_, event| event == TickEvent::Hook(LEAF));
}

#[test]
fn a_snapshot_keeps_an_arrival_the_machine_has_not_taken() {
    let mut m = machine_with(common::red_backdrop_rom());
    m.set_hook_table(Some(sync_points(&[IDLE_LOOP])));
    until_a_frame_ends_at_the_idle_loop(&mut m);
    // The arrival is pending: the copy reports it first thing, as the machine does.
    let mut copy = m.snapshot();
    let cycles = m.total_master_cycles();
    assert_eq!(tick(&mut copy), TickEvent::Hook(IDLE_LOOP));
    assert_eq!(copy.total_master_cycles(), cycles);
    assert_eq!(tick(&mut m), TickEvent::Hook(IDLE_LOOP));
    // Once taken, a copy has none: its next tick fetches the opcode.
    let mut copy = m.snapshot();
    assert_ne!(tick(&mut copy), TickEvent::Hook(IDLE_LOOP));
    assert!(copy.total_master_cycles() > cycles);
}

#[test]
fn an_armed_arrival_is_reported() {
    let mut m = machine_with(common::red_backdrop_rom());
    run(&mut m, 2);
    m.set_hook_table(Some(sync_points(&[IDLE_LOOP, 0x00_8000])));
    // Put the CPU at the first hooked address by hand.
    while !m.at_fetch_boundary() {
        tick(&mut m);
    }
    let mut registers = m.cpu_registers();
    registers.pbr = 0;
    registers.pc = 0x8000;
    m.set_cpu_registers(registers);
    m.arm_arrival();
    assert_eq!(tick(&mut m), TickEvent::Hook(0x00_8000));
}

#[test]
fn original_entries_are_not_hooked() {
    let mut table = sync_points(&[LEAF]);
    assert!(table.demote(LEAF));
    let mut m = machine_with(common::jsr_leaf_rom());
    m.set_hook_table(Some(table));
    let frames = m.frame_count();
    while m.frame_count() < frames + 2 {
        assert!(!matches!(tick(&mut m), TickEvent::Hook(_)));
    }
}

/// Ticks until a frame is rendered by a tick that ends at the hooked idle loop of
/// [`common::red_backdrop_rom`], so the hit of that tick is pending.
fn until_a_frame_ends_at_the_idle_loop(m: &mut Machine) {
    for _ in 0..30 {
        while tick(m) != TickEvent::Frame {}
        if m.at_fetch_boundary() && m.cpu_registers().pc == 0x801A {
            return;
        }
    }
    panic!("no frame was rendered by a tick that ended at the idle loop");
}

#[test]
fn a_pending_hit_survives_a_table_that_still_hooks_it() {
    let mut m = machine_with(common::red_backdrop_rom());
    m.set_hook_table(Some(sync_points(&[IDLE_LOOP])));
    until_a_frame_ends_at_the_idle_loop(&mut m);
    m.set_hook_table(Some(sync_points(&[IDLE_LOOP, 0x00_8000])));
    let cycles = m.total_master_cycles();
    assert_eq!(tick(&mut m), TickEvent::Hook(IDLE_LOOP));
    assert_eq!(m.total_master_cycles(), cycles);
}

#[test]
fn a_pending_hit_is_dropped_by_a_table_without_it() {
    let mut m = machine_with(common::red_backdrop_rom());
    m.set_hook_table(Some(sync_points(&[IDLE_LOOP])));
    until_a_frame_ends_at_the_idle_loop(&mut m);
    m.set_hook_table(Some(sync_points(&[0x00_8000])));
    assert_eq!(tick(&mut m), TickEvent::None, "the CPU fetches the opcode without a hit");
}

#[test]
fn a_pending_hit_is_dropped_once_the_cpu_left_its_address() {
    let mut m = machine_with(common::red_backdrop_rom());
    m.set_hook_table(Some(sync_points(&[IDLE_LOOP])));
    until_a_frame_ends_at_the_idle_loop(&mut m);
    let mut registers = m.cpu_registers();
    registers.pc = 0x8017; // STA $2100 (A = $0F), then BRA $801A
    m.set_cpu_registers(registers);
    assert_eq!(tick(&mut m), TickEvent::None, "the stale hit is not reported");
    tick_until(&mut m, 1, |_, event| event == TickEvent::Hook(IDLE_LOOP));
}

#[test]
fn load_state_at_a_hooked_fetch_reports_the_arrival() {
    let mut m = machine_with(common::jsr_leaf_rom());
    m.set_hook_table(Some(sync_points(&[LEAF])));
    tick_until(&mut m, 2, |_, event| event == TickEvent::Hook(LEAF));
    let state = m.save_state();
    run(&mut m, 1);
    m.load_state(&state).unwrap();
    let cycles = m.total_master_cycles();
    assert_eq!(tick(&mut m), TickEvent::Hook(LEAF), "the loaded state is at the hooked fetch");
    assert_eq!(m.total_master_cycles(), cycles);
}

#[test]
fn an_nmi_return_after_wai_hits_once_per_frame() {
    let mut m = machine_with(common::wai_nmi_rom());
    m.set_hook_table(Some(sync_points(&[common::WAI_RETURN])));
    run(&mut m, 2);
    for _ in 0..10 {
        let mut hits = 0;
        loop {
            match tick(&mut m) {
                TickEvent::Hook(address) => {
                    assert_eq!(address, common::WAI_RETURN);
                    hits += 1;
                    let wram = m.wram();
                    assert_eq!(
                        wram[common::NMI_COUNTER],
                        wram[LEAF_COUNTER].wrapping_add(1),
                        "the NMI handler of this frame ran before the hit"
                    );
                    assert_eq!(m.cpu_registers().s, common::MAIN_STACK, "S as before the WAI");
                }
                TickEvent::Frame => break,
                TickEvent::None => {}
            }
        }
        assert_eq!(hits, 1, "one arrival per frame");
    }
}

#[test]
fn the_installed_table_can_be_read_back() {
    let mut m = machine_with(common::jsr_leaf_rom());
    assert!(m.hook_table().is_none());
    m.set_hook_table(Some(sync_points(&[LEAF])));
    let table = m.hook_table().expect("a table is installed");
    assert_eq!(table.len(), 1);
    assert_eq!(table.get(LEAF).map(|entry| entry.name.as_str()), Some("sync_008040"));
    assert!(format!("{table:?}").contains("sync_008040"));
    m.set_hook_table(None);
    assert!(m.hook_table().is_none());
}
