//! Sync points: the per-frame moments where lockstep compares two machines, the first arrival at
//! the idle loop after each NMI.
//!
//! The real-ROM test needs the game's ROM:
//! `SSD_ROM_PATH=... cargo test -p ssd-oracle --release --test sync -- --ignored --nocapture`.

mod common;

use common::{
    SYNC_FLAG, SYNC_IDLE, SYNC_NMI_COUNTER, SYNC_TASK_DEFAULT, SYNC_TASK_LENGTH, sync_idle_rom,
};
use ssd_game::scripts::{BATTLE_FRAMES, battle_input};
use ssd_oracle::{
    HookEntry, HookTable, Joypad, Machine, MemorySaveStore, NullAudio, Symbols, SyncEvent,
};
use std::collections::BTreeMap;

/// Frames [`Machine::run_until_sync`] may run in these tests before giving up.
const BUDGET: u64 = 10;
/// A frame task of 12 288 iterations of 36 master cycles, about 1.25 frames: the next NMI lands
/// inside it.
const LONG_TASK: u16 = 0x3000;
/// `NMITIMEN`: NMI enable is bit 7, auto-joypad read bit 0.
const NMITIMEN: u32 = 0x00_4200;
/// `JOY1L`, the low byte of the auto-joypad read of port 1: A is bit 7.
const JOY1L: u32 = 0x00_4218;

fn pure_machine() -> Machine {
    Machine::new(sync_idle_rom(), Box::new(MemorySaveStore::default())).unwrap()
}

/// [`sync_idle_rom`] with its idle loop hooked as a sync point.
fn machine() -> Machine {
    let mut m = pure_machine();
    let table: HookTable = [HookEntry::sync_point("idle", SYNC_IDLE)].into_iter().collect();
    m.set_hook_table(Some(table));
    m
}

fn no_input(_: u64) -> Joypad {
    Joypad::default()
}

/// The next sync event, which must come within [`BUDGET`] frames.
fn next_event(m: &mut Machine) -> SyncEvent {
    m.run_until_sync(&mut no_input, &mut NullAudio, BUDGET)
        .unwrap()
        .expect("a sync event within the budget")
}

/// The machine is where `event` says: about to fetch the idle loop's first opcode on its first
/// pass after the NMI (the frame flag is still set), at the event's frame, NMI count and master
/// cycle.
fn assert_at(m: &Machine, event: &SyncEvent) {
    let r = m.cpu_registers();
    assert_eq!(event.key, SYNC_IDLE);
    assert_eq!((u32::from(r.pbr) << 16) | u32::from(r.pc), SYNC_IDLE, "at the sync point");
    assert!(m.at_fetch_boundary(), "about to fetch its opcode");
    assert_ne!(m.wram()[SYNC_FLAG] & 0x80, 0, "the first arrival after the NMI");
    assert_eq!(event.frame, m.frame_count());
    assert_eq!(event.nmi_count, m.nmi_count());
    assert_eq!(event.master_cycles, m.total_master_cycles());
}

/// The machine's NMI count equals the count of NMIs its handler entered (8-bit in WRAM).
fn assert_counted(m: &Machine) {
    assert_eq!(m.nmi_count() % 256, u64::from(m.wram()[SYNC_NMI_COUNTER]), "NMIs taken");
}

fn set_task_length(m: &mut Machine, iterations: u16) {
    let [lo, hi] = iterations.to_le_bytes();
    m.bus_write(SYNC_TASK_LENGTH as u32, lo);
    m.bus_write(SYNC_TASK_LENGTH as u32 + 1, hi);
}

#[test]
fn the_nmi_count_counts_every_nmi_taken() {
    let mut m = pure_machine();
    assert_eq!(m.nmi_count(), 0, "none at power-on");
    for _ in 0..20 {
        m.run_frame(Joypad::default(), &mut NullAudio).unwrap();
    }
    assert!(m.nmi_count() >= 19, "about one per frame: {}", m.nmi_count());
    assert_counted(&m);

    // A frame task longer than a frame: the NMI that lands in it is taken too.
    set_task_length(&mut m, LONG_TASK);
    for _ in 0..6 {
        m.run_frame(Joypad::default(), &mut NullAudio).unwrap();
    }
    assert_counted(&m);
}

#[test]
fn one_event_per_frame_with_an_nmi_delta_of_one() {
    let mut m = machine();
    let mut previous = next_event(&mut m);
    assert_at(&m, &previous);
    assert_counted(&m);
    assert_eq!(previous.nmi_count, 1, "the first event follows the first NMI");
    for _ in 0..60 {
        let event = next_event(&mut m);
        assert_at(&m, &event);
        assert_counted(&m);
        assert_eq!(event.nmi_count - previous.nmi_count, 1, "{event:?} after {previous:?}");
        assert_eq!(event.frame - previous.frame, 1, "{event:?} after {previous:?}");
        previous = event;
    }
    let (name, stats) = &m.hook_stats()[0];
    assert_eq!(name, "idle");
    assert!(stats.calls > 61 * 2, "every arrival is counted, events or not: {}", stats.calls);
}

#[test]
fn a_frame_task_longer_than_a_frame_shows_an_nmi_delta_of_two() {
    let mut m = machine();
    let mut previous = next_event(&mut m);
    set_task_length(&mut m, LONG_TASK);
    let lag = next_event(&mut m);
    assert_at(&m, &lag);
    assert_counted(&m);
    assert_eq!(lag.nmi_count - previous.nmi_count, 2, "the NMI that landed in the task");
    assert_eq!(lag.frame - previous.frame, 2);

    set_task_length(&mut m, SYNC_TASK_DEFAULT);
    previous = lag;
    for _ in 0..3 {
        let event = next_event(&mut m);
        assert_at(&m, &event);
        assert_eq!(event.nmi_count - previous.nmi_count, 1, "back in sync");
        assert_eq!(event.frame - previous.frame, 1);
        previous = event;
    }
}

#[test]
fn without_nmis_the_budget_runs_out() {
    let mut m = machine();
    let event = next_event(&mut m);
    // NMI off, as in a transition loop: the idle loop runs on, but no NMI follows the event.
    m.bus_write(NMITIMEN, 0x00);
    let start = m.frame_count();
    assert_eq!(m.run_until_sync(&mut no_input, &mut NullAudio, 5).unwrap(), None);
    assert_eq!(m.frame_count(), start + 5, "exactly the budget");
    assert_eq!(m.nmi_count(), event.nmi_count);
    assert!(m.hook_stats()[0].1.calls > 5, "arrivals went on");

    // A zero budget runs nothing.
    let cycles = m.total_master_cycles();
    assert_eq!(m.run_until_sync(&mut no_input, &mut NullAudio, 0).unwrap(), None);
    assert_eq!(m.total_master_cycles(), cycles);

    // NMI on again: the next event follows the next NMI.
    m.bus_write(NMITIMEN, 0x80);
    let next = next_event(&mut m);
    assert_at(&m, &next);
    assert_eq!(next.nmi_count - event.nmi_count, 1);
}

#[test]
fn events_do_not_change_emulation() {
    let mut m = machine();
    let mut twin = pure_machine();
    for _ in 0..20 {
        next_event(&mut m);
    }
    let event = next_event(&mut m);
    while twin.total_master_cycles() < event.master_cycles {
        twin.tick(Joypad::default(), &mut NullAudio).unwrap();
    }
    assert_eq!(twin.total_master_cycles(), event.master_cycles, "the same tick boundaries");
    assert!(twin.save_state() == m.save_state(), "the pure twin is in the same state");
}

#[test]
fn the_input_is_looked_up_by_frame_number() {
    let mut m = machine();
    next_event(&mut m);
    // Auto-joypad read on (the main code has enabled the NMI by now): it latches each frame's
    // input after the frame's NMI is raised, before its event.
    m.bus_write(NMITIMEN, 0x81);
    let a_on_odd_frames = |frame: u64| Joypad { a: frame % 2 == 1, ..Joypad::default() };
    let mut asked = Vec::new();
    for _ in 0..8 {
        let start = m.frame_count();
        asked.clear();
        let mut input = |frame| {
            asked.push(frame);
            a_on_odd_frames(frame)
        };
        let event = m.run_until_sync(&mut input, &mut NullAudio, BUDGET).unwrap().unwrap();
        assert_eq!(asked, (start..=event.frame).collect::<Vec<_>>(), "each frame, in order");
        let a_pressed = m.bus_read(JOY1L) & 0x80 != 0;
        assert_eq!(a_pressed, a_on_odd_frames(event.frame).a, "frame {}", event.frame);
    }
}

#[test]
fn after_run_frame_the_next_event_follows_the_next_nmi() {
    let mut m = machine();
    let first = next_event(&mut m);
    for _ in 0..3 {
        m.run_frame(Joypad::default(), &mut NullAudio).unwrap();
    }
    let event = next_event(&mut m);
    assert_at(&m, &event);
    assert_eq!(event.frame, first.frame + 3);
}

#[test]
fn snapshots_and_loaded_states_keep_the_count_and_wait_for_the_next_nmi() {
    let mut m = machine();
    let first = next_event(&mut m);

    // A snapshot copies the count and the last event: it runs on as the original does.
    let mut copy = m.snapshot();
    assert_eq!(copy.nmi_count(), m.nmi_count());
    let state = m.save_state();
    assert_eq!(next_event(&mut copy), next_event(&mut m));

    for _ in 0..4 {
        next_event(&mut m);
    }
    let count = m.nmi_count();
    m.load_state(&state).unwrap();
    assert_eq!(m.nmi_count(), count, "a counter outside the state: carried over");

    // The loaded state is at the first event's arrival again, reported by the next tick; that
    // NMI was already the subject of an event, so the next event follows the next NMI.
    let event = next_event(&mut m);
    assert_at(&m, &event);
    assert_eq!(event.nmi_count, count + 1);
    assert_eq!(event.frame, first.frame + 1, "the loaded timeline");
}

#[test]
#[ignore = "needs SSD_ROM_PATH"]
fn real_rom_events_per_frame_over_the_play_through() {
    const FRAMES: u64 = BATTLE_FRAMES;
    const WINDOW: u64 = 600;
    const EVENT_BUDGET: u64 = 60;

    let path = std::env::var("SSD_ROM_PATH").expect("SSD_ROM_PATH");
    let rom = ssd_rom::normalize(&std::fs::read(path).unwrap(), None).unwrap();
    assert_eq!(ssd_rom::crc32(&rom.bytes), ssd_rom::CANONICAL_CRC32, "not the canonical ROM");
    let new_machine = || Machine::new(rom.bytes.clone(), Box::new(MemorySaveStore::default()));
    let mut m = new_machine().unwrap();
    let symbols =
        Symbols::load(concat!(env!("CARGO_MANIFEST_DIR"), "/../../re/symbols.toml")).unwrap();
    let table: HookTable = symbols
        .sync_points()
        .iter()
        .map(|point| HookEntry::sync_point(point.name.clone(), point.address))
        .collect();
    assert!(!table.is_empty(), "no sync point in re/symbols.toml");
    m.set_hook_table(Some(table));

    let windows = (FRAMES / WINDOW) as usize;
    let mut events = vec![0_u64; windows];
    let mut nmis = vec![0_u64; windows];
    let mut expired = vec![0_u64; windows];
    let mut nmi_deltas = BTreeMap::<u64, u64>::new();
    let mut frame_deltas = BTreeMap::<u64, u64>::new();
    let mut previous: Option<SyncEvent> = None;
    let mut window_start_nmis = 0;
    while m.frame_count() < FRAMES {
        let window = (m.frame_count() / WINDOW) as usize;
        let budget = EVENT_BUDGET.min((window as u64 + 1) * WINDOW - m.frame_count());
        let result = m.run_until_sync(&mut battle_input, &mut NullAudio, budget).unwrap();
        match result {
            Some(event) => {
                events[(event.frame / WINDOW) as usize] += 1;
                if let Some(previous) = previous {
                    *nmi_deltas.entry(event.nmi_count - previous.nmi_count).or_default() += 1;
                    *frame_deltas.entry(event.frame - previous.frame).or_default() += 1;
                }
                previous = Some(event);
            }
            // A shorter budget only stopped at the window's end.
            None if budget == EVENT_BUDGET => expired[window] += 1,
            None => {}
        }
        if m.frame_count() == (window as u64 + 1) * WINDOW {
            nmis[window] = m.nmi_count() - window_start_nmis;
            window_start_nmis = m.nmi_count();
        }
    }

    eprintln!(
        "{:>11} {:>7} {:>7} {:>13} {:>8}",
        "frames", "events", "NMIs", "events/frame", "expired"
    );
    for window in 0..windows {
        let start = window as u64 * WINDOW;
        eprintln!(
            "{:>5}-{:<5} {:>7} {:>7} {:>13.3} {:>8}",
            start,
            start + WINDOW - 1,
            events[window],
            nmis[window],
            events[window] as f64 / WINDOW as f64,
            expired[window]
        );
    }
    eprintln!("NMI deltas between events (delta: count): {nmi_deltas:?}");
    eprintln!("frame deltas between events (delta: count): {frame_deltas:?}");
    let total: u64 = events.iter().sum();
    eprintln!("{total} events in {FRAMES} frames");

    assert!(total > 0, "the sync point was never reached after an NMI");
    assert!(!nmi_deltas.contains_key(&0), "an event without a new NMI");

    // The events changed nothing: a pure twin run frame by frame is in the same state.
    let mut twin = new_machine().unwrap();
    while twin.frame_count() < FRAMES {
        twin.run_frame(battle_input(twin.frame_count()), &mut NullAudio).unwrap();
    }
    assert!(twin.save_state() == m.save_state(), "the pure twin differs at frame {FRAMES}");
}
