//! Needs the game's ROM: `SSD_ROM_PATH=... cargo test -p ssd-oracle --release --test baseline -- --ignored`

use ssd_oracle::{
    HookAction, HookEntry, HookMode, HookTable, Joypad, Machine, MemorySaveStore, NullAudio,
    Symbols, TickEvent,
};
use std::collections::BTreeMap;

// Pinned in M1a Task 2: emulation must not change until a patch intentionally changes it.
// CRC32 of `save_state()` after 600 frames, so it also moves when the state layout changes.
const BASELINE_600: u32 = 0x5603_3277;
// Pinned in M1a Task 4: CRC32 of `fingerprint` after the same run. Independent of the state
// layout: a layout change may re-pin BASELINE_600 only with this value unchanged.
const FINGERPRINT_600: u32 = 0x4562_A3C9;

/// The canonical game on a fresh machine.
fn game() -> Machine {
    let path = std::env::var("SSD_ROM_PATH").expect("SSD_ROM_PATH");
    let bytes = std::fs::read(path).unwrap();
    let rom = ssd_rom::normalize(&bytes, None).unwrap();
    assert_eq!(ssd_rom::crc32(&rom.bytes), ssd_rom::CANONICAL_CRC32, "not the canonical ROM");
    Machine::new(rom.bytes, Box::new(MemorySaveStore::default())).unwrap()
}

/// 600 frames without input; with `log`, the CPU-write log is on and drained every frame.
fn run_600(log: bool) -> Machine {
    let mut m = game();
    m.set_cpu_write_log(log);
    let mut writes = 0;
    for _ in 0..600 {
        m.run_frame(Joypad::default(), &mut NullAudio).unwrap();
        writes += m.take_cpu_writes().len();
    }
    assert_eq!(writes > 0, log, "the log records writes exactly when enabled");
    m
}

/// CRC32 over WRAM, VRAM and CGRAM (little-endian words), OAM low and high tables, SRAM, the CPU
/// registers (A, X, Y, S, D, PBR, PC, DBR, P, emulation flag; little-endian) and the master-cycle
/// count (little-endian).
fn fingerprint(m: &Machine) -> u32 {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(m.wram());
    bytes.extend(m.vram().iter().flat_map(|word| word.to_le_bytes()));
    bytes.extend(m.cgram().iter().flat_map(|word| word.to_le_bytes()));
    let (oam_low, oam_high) = m.oam();
    bytes.extend_from_slice(oam_low);
    bytes.extend_from_slice(oam_high);
    if let Some(sram) = m.sram() {
        bytes.extend_from_slice(sram);
    }
    let r = m.cpu_registers();
    for word in [r.a, r.x, r.y, r.s, r.d] {
        bytes.extend(word.to_le_bytes());
    }
    bytes.push(r.pbr);
    bytes.extend(r.pc.to_le_bytes());
    bytes.push(r.dbr);
    bytes.push(u8::from(r.p));
    bytes.push(u8::from(r.emulation_mode));
    bytes.extend(m.total_master_cycles().to_le_bytes());
    ssd_rom::crc32(&bytes)
}

#[test]
#[ignore = "needs SSD_ROM_PATH"]
fn untouched_emulation_matches_the_pinned_baseline() {
    let m = run_600(false);
    assert_eq!(ssd_rom::crc32(&m.save_state()), BASELINE_600, "600-frame state CRC changed");
    assert_eq!(fingerprint(&m), FINGERPRINT_600, "600-frame fingerprint changed");
}

#[test]
#[ignore = "needs SSD_ROM_PATH"]
fn cpu_write_log_does_not_change_emulation() {
    let m = run_600(true);
    assert_eq!(ssd_rom::crc32(&m.save_state()), BASELINE_600, "600-frame state CRC changed");
    assert_eq!(fingerprint(&m), FINGERPRINT_600, "600-frame fingerprint changed");
}

/// Every routine address of `re/symbols.toml`.
fn candidate_addresses() -> Vec<u32> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../re/symbols.toml");
    let symbols = Symbols::load(path).unwrap();
    let addresses: Vec<u32> = symbols.routines().iter().map(|routine| routine.address).collect();
    assert!(!addresses.is_empty(), "no routine in {path}");
    addresses
}

/// Every candidate routine hooked as a sync point: each arrival is reported and the original
/// routine resumes.
fn candidate_table() -> HookTable {
    candidate_addresses()
        .into_iter()
        .map(|key| HookEntry {
            name: format!("candidate_{key:06X}"),
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

#[test]
#[ignore = "needs SSD_ROM_PATH"]
fn ticking_through_candidate_hits_does_not_change_emulation() {
    let mut m = game();
    m.set_hook_table(Some(candidate_table()));
    let mut hits = BTreeMap::<u32, u64>::new();
    while m.frame_count() < 600 {
        if let TickEvent::Hook(address) = m.tick(Joypad::default(), &mut NullAudio).unwrap() {
            *hits.entry(address).or_default() += 1;
        }
    }
    for (address, count) in &hits {
        eprintln!("${address:06X}: {count} hits");
    }
    assert!(!hits.is_empty(), "no candidate routine was reached");
    assert_eq!(ssd_rom::crc32(&m.save_state()), BASELINE_600, "600-frame state CRC changed");
    assert_eq!(fingerprint(&m), FINGERPRINT_600, "600-frame fingerprint changed");
}

#[test]
#[ignore = "needs SSD_ROM_PATH"]
fn run_until_sync_with_the_sync_points_matches_the_baseline() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../re/symbols.toml");
    let symbols = Symbols::load(path).unwrap();
    let table: HookTable = symbols
        .sync_points()
        .iter()
        .map(|point| HookEntry::sync_point(point.name.clone(), point.address))
        .collect();
    assert!(!table.is_empty(), "no sync point in {path}");
    let mut m = game();
    m.set_hook_table(Some(table));
    let mut events = 0;
    // The last call's budget ends on the tick that renders frame 600, where run_frame stops.
    // The intro runs with NMI off (no event before Start is pressed): this checks the budget
    // path; the play-through of `tests/sync.rs` checks a run full of events against a pure twin.
    while m.frame_count() < 600 {
        let budget = 600 - m.frame_count();
        if m.run_until_sync(&mut |_| Joypad::default(), &mut NullAudio, budget).unwrap().is_some() {
            events += 1;
        }
    }
    eprintln!("{events} sync events in 600 frames, {} NMIs", m.nmi_count());
    assert_eq!(ssd_rom::crc32(&m.save_state()), BASELINE_600, "600-frame state CRC changed");
    assert_eq!(fingerprint(&m), FINGERPRINT_600, "600-frame fingerprint changed");
}

#[test]
#[ignore = "needs SSD_ROM_PATH"]
fn run_frame_with_every_candidate_hooked_matches_the_baseline() {
    let mut m = game();
    m.set_hook_table(Some(candidate_table()));
    for _ in 0..600 {
        m.run_frame(Joypad::default(), &mut NullAudio).unwrap();
    }
    assert_eq!(ssd_rom::crc32(&m.save_state()), BASELINE_600, "600-frame state CRC changed");
    assert_eq!(fingerprint(&m), FINGERPRINT_600, "600-frame fingerprint changed");
}
