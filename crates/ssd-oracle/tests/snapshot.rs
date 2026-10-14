mod common;

use ssd_oracle::{Joypad, Machine, MemorySaveStore, NullAudio};

fn machine_with(rom: Vec<u8>) -> Machine {
    Machine::new(rom, Box::new(MemorySaveStore::default())).unwrap()
}

#[test]
fn snapshot_runs_identically() {
    let mut a = machine_with(common::red_backdrop_rom());
    for _ in 0..3 {
        a.run_frame(Joypad::default(), &mut NullAudio).unwrap();
    }
    let mut b = a.snapshot();
    for _ in 0..5 {
        a.run_frame(Joypad::default(), &mut NullAudio).unwrap();
        b.run_frame(Joypad::default(), &mut NullAudio).unwrap();
    }
    assert_eq!(a.save_state(), b.save_state());
}

/// Needs `SSD_ROM_PATH`: the game must be on a mapper whose snapshots share the ROM.
#[test]
#[ignore = "needs SSD_ROM_PATH"]
fn snapshot_of_the_game_runs_identically() {
    let bytes = std::fs::read(std::env::var("SSD_ROM_PATH").expect("SSD_ROM_PATH")).unwrap();
    let mut a = machine_with(ssd_rom::normalize(&bytes, None).unwrap().bytes);
    for _ in 0..300 {
        a.run_frame(Joypad::default(), &mut NullAudio).unwrap();
    }
    let mut b = a.snapshot();
    for _ in 0..120 {
        a.run_frame(Joypad::default(), &mut NullAudio).unwrap();
        b.run_frame(Joypad::default(), &mut NullAudio).unwrap();
    }
    assert_eq!(a.save_state(), b.save_state());
}

#[test]
fn snapshot_saves_never_reach_the_original_store() {
    let store = common::SharedStore::default();
    let mut m = Machine::new(common::sram_counter_rom(), Box::new(store.clone())).unwrap();
    m.run_frame(Joypad::default(), &mut NullAudio).unwrap();
    m.flush_saves().unwrap();
    let before = store.get("sav").expect("sav persisted by flush");

    let mut snap = m.snapshot();
    for _ in 0..40 {
        snap.run_frame(Joypad::default(), &mut NullAudio).unwrap();
    }
    snap.flush_saves().unwrap();
    assert_ne!(snap.sram().unwrap()[0], before[0], "the snapshot's SRAM moved on");
    assert_eq!(store.get("sav").unwrap(), before, "the original's store is untouched");
    assert_eq!(m.sram().unwrap()[0], before[0], "the original's SRAM is untouched");
}

#[test]
fn flush_saves_persists_sram_between_periodic_checks() {
    let store = common::SharedStore::default();
    let mut m = Machine::new(common::sram_counter_rom(), Box::new(store.clone())).unwrap();
    for _ in 0..10 {
        m.run_frame(Joypad::default(), &mut NullAudio).unwrap();
    }
    m.flush_saves().unwrap();
    let sav = store.get("sav").expect("sav persisted by flush");
    assert_eq!(sav[0], m.sram().unwrap()[0]);
    assert_ne!(sav[0], 0xFF, "fresh SRAM is 0xFF-filled; the counter must have advanced");
}
