//! Fixtures for unit tests: a synthetic ROM and a save store the test can inspect.

use ssd_oracle::{Machine, SaveStore};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// A 32 KiB `LoROM` with 2 KiB of battery-backed SRAM that increments SRAM byte 0 once per frame.
pub fn sram_counter_rom() -> Vec<u8> {
    let mut rom = vec![0xFF_u8; 0x8000];
    #[rustfmt::skip] // keep the one-instruction-per-line layout with address comments
    let code: &[u8] = &[
        0x78,                   // 8000 SEI
        0x18,                   // 8001 CLC
        0xFB,                   // 8002 XCE           -> native mode
        0xE2, 0x30,             // 8003 SEP #$30      -> 8-bit A, X, Y
        0xA9, 0x01,             // 8005 LDA #$01
        0x8D, 0x00, 0x42,       // 8007 STA $4200     auto-joypad read only, NMI disabled
        0xAD, 0x10, 0x42,       // 800A LDA $4210     loop: read and clear the vblank flag
        0x10, 0xFB,             // 800D BPL $800A     wait for bit 7
        0xAF, 0x00, 0x00, 0x70, // 800F LDA $700000   SRAM byte 0
        0x1A,                   // 8013 INC A
        0x8F, 0x00, 0x00, 0x70, // 8014 STA $700000
        0x80, 0xF0,             // 8018 BRA $800A
        0x40,                   // 801A RTI           target of every unused vector
    ];
    rom[..code.len()].copy_from_slice(code);
    rom[0x7FC0..0x7FD5].copy_from_slice(b"SSD APP TEST         ");
    rom[0x7FD5] = 0x20; // LoROM, SlowROM
    rom[0x7FD6] = 0x02; // ROM + RAM + battery
    rom[0x7FD7] = 0x05; // 32 KiB
    rom[0x7FD8] = 0x01; // 2 KiB of SRAM
    rom[0x7FD9] = 0x01; // North America -> NTSC
    for v in (0x7FE4..0x7FF0).step_by(2).chain((0x7FF4..0x8000).step_by(2)) {
        rom[v..v + 2].copy_from_slice(&0x801A_u16.to_le_bytes());
    }
    rom[0x7FFC..0x7FFE].copy_from_slice(&0x8000_u16.to_le_bytes());
    ssd_rom::fix_lorom_checksum(&mut rom);
    rom
}

/// In-memory save store whose clones share one map, so a test sees what a machine persisted. It
/// can be told to refuse writes, as a full disk would.
#[derive(Clone, Default)]
pub struct SharedStore {
    files: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    refuse_writes: Arc<AtomicBool>,
}

impl SharedStore {
    pub fn get(&self, ext: &str) -> Option<Vec<u8>> {
        self.files.lock().unwrap().get(ext).cloned()
    }

    pub fn refuse_writes(&self, refuse: bool) {
        self.refuse_writes.store(refuse, Ordering::SeqCst);
    }
}

impl SaveStore for SharedStore {
    fn load(&mut self, ext: &str) -> std::io::Result<Option<Vec<u8>>> {
        Ok(self.get(ext))
    }
    fn persist(&mut self, ext: &str, bytes: &[u8]) -> std::io::Result<()> {
        if self.refuse_writes.load(Ordering::SeqCst) {
            return Err(std::io::Error::other("disk full"));
        }
        self.files.lock().unwrap().insert(ext.to_owned(), bytes.to_vec());
        Ok(())
    }
}

/// A machine running [`sram_counter_rom`] and the store its saves go to.
pub fn sram_counter_machine() -> (Machine, SharedStore) {
    let store = SharedStore::default();
    let machine = Machine::new(sram_counter_rom(), Box::new(store.clone())).unwrap();
    (machine, store)
}
