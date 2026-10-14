//! Synthetic ROMs for the unit tests of the commands (no retail bytes).

/// The leaf of [`leaf_rom`].
pub const LEAF: u32 = 0x00_8040;

/// A 32 KiB `LoROM` that calls a leaf routine (`INC $10; RTS`) in a loop, with 200 delay
/// iterations of `DEX; BNE` between the calls (NMI off, native mode, 8-bit A, 16-bit X/Y).
pub fn leaf_rom() -> Vec<u8> {
    let mut rom = vec![0xFF_u8; 0x8000];
    #[rustfmt::skip] // keep the one-instruction-per-line layout with address comments
    let main: &[u8] = &[
        0x78,             // 8000 SEI
        0x18,             // 8001 CLC
        0xFB,             // 8002 XCE           -> native mode
        0xC2, 0x10,       // 8003 REP #$10      -> 16-bit X, Y
        0xE2, 0x20,       // 8005 SEP #$20      -> 8-bit A
        0x20, 0x40, 0x80, // 8007 JSR $8040     loop: call the leaf
        0xA2, 0xC8, 0x00, // 800A LDX #$00C8    200 delay iterations
        0xCA,             // 800D DEX           delay:
        0xD0, 0xFD,       // 800E BNE $800D
        0x80, 0xF5,       // 8010 BRA $8007
        0x40,             // 8012 RTI           target of every unused vector
    ];
    rom[..main.len()].copy_from_slice(main);
    rom[0x40..0x43].copy_from_slice(&[0xE6, 0x10, 0x60]);
    rom[0x7FC0..0x7FD5].copy_from_slice(b"SSD XTASK FIXTURE    ");
    rom[0x7FD5] = 0x20; // LoROM, SlowROM
    rom[0x7FD7] = 0x05; // 32 KiB
    rom[0x7FD9] = 0x01; // North America -> NTSC
    for v in (0x7FE4..0x7FF0).step_by(2).chain((0x7FF4..0x8000).step_by(2)) {
        rom[v..v + 2].copy_from_slice(&0x8012_u16.to_le_bytes());
    }
    rom[0x7FFC..0x7FFE].copy_from_slice(&0x8000_u16.to_le_bytes());
    ssd_rom::fix_lorom_checksum(&mut rom);
    rom
}
