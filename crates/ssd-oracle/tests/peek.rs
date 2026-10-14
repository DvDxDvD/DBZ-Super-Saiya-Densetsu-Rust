//! `Machine::peek`: side-effect-free reads that agree with the bus wherever the bus reads memory.

mod common;

use ssd_oracle::{Joypad, Machine, MemorySaveStore, NullAudio};

fn machine() -> Machine {
    Machine::new(common::jsr_leaf_rom(), Box::new(MemorySaveStore::default())).unwrap()
}

#[test]
fn peek_gives_what_the_bus_gives_in_wram_and_in_the_rom() {
    let mut m = machine();
    for _ in 0..3 {
        m.run_frame(Joypad::default(), &mut NullAudio).unwrap();
    }
    // WRAM: the first and last bytes of both banks, the low-RAM mirror, the upper half of $7F.
    let wram = [
        0x7E_0000, 0x7E_0010, 0x7E_1234, 0x7E_FFFF, 0x7F_0000, 0x7F_8000, 0x7F_FFFF, 0x00_0010,
        0x00_1FFF, 0x3F_1FF0, 0x80_0010, 0xBF_1FFF,
    ];
    // The ROM image: the LoROM window of banks that mirror it, the ends of the bus.
    let rom = [
        0x00_8000, 0x00_8123, 0x00_FFFF, 0x01_8000, 0x3F_C000, 0x40_8000, 0x6F_FFFF, 0x80_8000,
        0xC0_8000, 0xEF_FFFF, 0xFF_FFFF,
    ];
    for address in wram.into_iter().chain(rom) {
        let peeked = m.peek(address);
        assert_eq!(peeked, Some(m.bus_read(address)), "${address:06X}");
    }
    // The mirror is WRAM: the same byte as the bank it mirrors.
    assert_eq!(m.peek(0x00_1234), m.peek(0x7E_1234));
    assert_eq!(m.peek(0xBF_0042), m.peek(0x7E_0042));
}

#[test]
fn peek_takes_no_side_effects_and_is_none_outside_wram_and_the_rom() {
    let m = machine();
    let (fetches, frames, wram) = (m.opcode_fetches(), m.frame_count(), m.wram().to_vec());
    // Registers (the WRAM port included, which a read would step), SRAM, open bus.
    let nothing = [0x00_2100, 0x00_2180, 0x3F_4016, 0x00_6000, 0x00_7000, 0x70_0000, 0x7D_0000];
    for address in nothing {
        assert_eq!(m.peek(address), None, "${address:06X}");
    }
    // Reads through `&Machine` change nothing the machine counts or holds.
    for address in [0x7E_0000, 0x00_8000] {
        let _ = m.peek(address);
    }
    assert_eq!((m.opcode_fetches(), m.frame_count()), (fetches, frames));
    assert_eq!(m.wram(), wram.as_slice());
}
