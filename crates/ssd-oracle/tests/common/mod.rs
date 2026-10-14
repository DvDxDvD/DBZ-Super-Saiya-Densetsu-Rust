//! Shared fixtures for oracle tests: synthetic 32 KiB `LoROM`s and an inspectable save store.

// Each test binary uses a subset of these fixtures.
#![allow(dead_code)]

use ssd_oracle::{Ctx, SaveStore};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Sets backdrop color 0 to red and idles.
pub fn red_backdrop_rom() -> Vec<u8> {
    let mut rom = vec![0xFF_u8; 0x8000];
    #[rustfmt::skip] // keep the one-instruction-per-line layout with address comments
    let code: &[u8] = &[
        0x78,             // 8000 SEI
        0x18,             // 8001 CLC
        0xFB,             // 8002 XCE           -> native mode
        0xE2, 0x30,       // 8003 SEP #$30      -> 8-bit A, X, Y
        0xA9, 0x80,       // 8005 LDA #$80
        0x8D, 0x00, 0x21, // 8007 STA $2100     forced blank while writing CGRAM
        0x9C, 0x21, 0x21, // 800A STZ $2121     CGRAM address 0
        0xA9, 0x1F,       // 800D LDA #$1F
        0x8D, 0x22, 0x21, // 800F STA $2122     color low byte: red = 31
        0x9C, 0x22, 0x21, // 8012 STZ $2122     color high byte
        0xA9, 0x0F,       // 8015 LDA #$0F
        0x8D, 0x00, 0x21, // 8017 STA $2100     display on, brightness 15
        0x80, 0xFE,       // 801A BRA $801A     idle forever
        0x40,             // 801C RTI           target of every unused vector
    ];
    rom[..code.len()].copy_from_slice(code);
    write_header(&mut rom, b"SSD ORACLE TEST      ", 0x00, 0x00, 0x801C);
    rom
}

/// Address of the leaf routine in [`jsr_leaf_rom`].
pub const LEAF: u32 = 0x00_8040;
/// WRAM index of the counter the leaf increments (`$00:0010`, direct page 0).
pub const LEAF_COUNTER: usize = 0x10;

/// Calls a leaf routine (`INC $10; RTS`) about 50 times per frame.
///
/// Native mode, 8-bit A, 16-bit X/Y; NMI is disabled. Between calls a delay loop runs 200
/// iterations of `DEX; BNE` (about 1000 CPU cycles).
pub fn jsr_leaf_rom() -> Vec<u8> {
    #[rustfmt::skip]
    let leaf: &[u8] = &[
        0xE6, 0x10,       // 8040 INC $10       leaf: WRAM $00:0010
        0x60,             // 8042 RTS
    ];
    jsr_loop_rom(leaf, b"SSD JSR LEAF TEST    ")
}

/// The main loop of [`jsr_leaf_rom`] calling `leaf`, placed at [`LEAF`].
fn jsr_loop_rom(leaf: &[u8], title: &[u8; 21]) -> Vec<u8> {
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
        0xD0, 0xFD,       // 800E BNE $800D     (0x0D - 0x10 = -3)
        0x80, 0xF5,       // 8010 BRA $8007     (0x07 - 0x12 = -11)
        0x40,             // 8012 RTI           target of every unused vector
    ];
    rom[..main.len()].copy_from_slice(main);
    rom[0x40..0x40 + leaf.len()].copy_from_slice(leaf);
    write_header(&mut rom, title, 0x00, 0x00, 0x8012);
    rom
}

/// Values the leaf of [`mmio_leaf_rom`] writes: INIDISP (`$2100`), then COLDATA (`$2132`).
pub const MMIO_LEAF_WRITES: [(u32, u8); 2] = [(0x00_2100, 0x0F), (0x00_2132, 0xE0)];

/// [`jsr_leaf_rom`]'s loop calling a leaf that writes two PPU registers whose order is visible
/// only in the write log: `LDA #$0F; STA $2100; LDA #$E0; STA $2132; RTS` (see
/// [`MMIO_LEAF_WRITES`]). It returns with A = `$E0`, N set and Z clear.
pub fn mmio_leaf_rom() -> Vec<u8> {
    #[rustfmt::skip]
    let leaf: &[u8] = &[
        0xA9, 0x0F,       // 8040 LDA #$0F
        0x8D, 0x00, 0x21, // 8042 STA $2100     display on, brightness 15
        0xA9, 0xE0,       // 8045 LDA #$E0
        0x8D, 0x32, 0x21, // 8047 STA $2132     fixed color: R, G and B at 0
        0x60,             // 804A RTS
    ];
    jsr_loop_rom(leaf, b"SSD MMIO LEAF TEST   ")
}

/// Address in [`wai_nmi_rom`] where the NMI handler returns to after the main loop's `WAI`.
pub const WAI_RETURN: u32 = 0x00_8015;
/// Stack pointer of the main loop in [`wai_nmi_rom`] and [`nmi_leaf_rom`].
pub const MAIN_STACK: u16 = 0x1FF0;
/// WRAM index of the counter the NMI handlers increment (`$00:0011`).
pub const NMI_COUNTER: usize = 0x11;
/// NMI handler of [`wai_nmi_rom`] and [`nmi_leaf_rom`].
pub const NMI_HANDLER: u16 = 0x8020;

/// Waits for the NMI in a loop: `WAI`, then `INC $10` at [`WAI_RETURN`]; the NMI handler
/// increments `$11` and acknowledges the NMI.
///
/// Native mode, 8-bit A, 16-bit X/Y, S = [`MAIN_STACK`]; both counters start at 0.
pub fn wai_nmi_rom() -> Vec<u8> {
    let mut rom = vec![0xFF_u8; 0x8000];
    #[rustfmt::skip] // keep the one-instruction-per-line layout with address comments
    let main: &[u8] = &[
        0x78,             // 8000 SEI
        0x18,             // 8001 CLC
        0xFB,             // 8002 XCE           -> native mode
        0xC2, 0x10,       // 8003 REP #$10      -> 16-bit X, Y
        0xE2, 0x20,       // 8005 SEP #$20      -> 8-bit A
        0xA2, 0xF0, 0x1F, // 8007 LDX #$1FF0
        0x9A,             // 800A TXS
        0x64, 0x10,       // 800B STZ $10
        0x64, 0x11,       // 800D STZ $11
        0xA9, 0x81,       // 800F LDA #$81
        0x8D, 0x00, 0x42, // 8011 STA $4200     NMI and auto-joypad read on
        0xCB,             // 8014 WAI           loop: wait for the NMI
        0xE6, 0x10,       // 8015 INC $10       the NMI returns here
        0x80, 0xFB,       // 8017 BRA $8014     (0x14 - 0x19 = -5)
        0x40,             // 8019 RTI           target of every unused vector
    ];
    #[rustfmt::skip]
    let nmi: &[u8] = &[
        0xE6, 0x11,       // 8020 INC $11       NMI handler
        0xAD, 0x10, 0x42, // 8022 LDA $4210     acknowledge
        0x40,             // 8025 RTI
    ];
    rom[..main.len()].copy_from_slice(main);
    rom[0x20..0x20 + nmi.len()].copy_from_slice(nmi);
    write_header(&mut rom, b"SSD WAI NMI TEST     ", 0x00, 0x00, 0x8019);
    set_native_nmi_vector(&mut rom, NMI_HANDLER);
    rom
}

/// Where the leaf of [`nmi_leaf_rom`] returns to: `LDA $11` after `JSR $8040`.
pub const NMI_LEAF_RETURN: u16 = 0x8017;

/// [`jsr_leaf_rom`]'s leaf (`INC $10; RTS` at [`LEAF`]) called in a tight loop with the NMI on;
/// after each call the loop copies `$11` to `$12`. The NMI handler preserves A, increments `$11`
/// and acknowledges the NMI.
///
/// Native mode, 8-bit A, 16-bit X/Y, S = [`MAIN_STACK`].
pub fn nmi_leaf_rom() -> Vec<u8> {
    let mut rom = vec![0xFF_u8; 0x8000];
    #[rustfmt::skip] // keep the one-instruction-per-line layout with address comments
    let main: &[u8] = &[
        0x78,             // 8000 SEI
        0x18,             // 8001 CLC
        0xFB,             // 8002 XCE           -> native mode
        0xC2, 0x10,       // 8003 REP #$10      -> 16-bit X, Y
        0xE2, 0x20,       // 8005 SEP #$20      -> 8-bit A
        0xA2, 0xF0, 0x1F, // 8007 LDX #$1FF0
        0x9A,             // 800A TXS
        0x64, 0x11,       // 800B STZ $11
        0x64, 0x12,       // 800D STZ $12
        0xA9, 0x80,       // 800F LDA #$80
        0x8D, 0x00, 0x42, // 8011 STA $4200     NMI on
        0x20, 0x40, 0x80, // 8014 JSR $8040     loop: call the leaf
        0xA5, 0x11,       // 8017 LDA $11
        0x85, 0x12,       // 8019 STA $12
        0x80, 0xF7,       // 801B BRA $8014     (0x14 - 0x1D = -9)
        0x40,             // 801D RTI           target of every unused vector
    ];
    #[rustfmt::skip]
    let nmi: &[u8] = &[
        0x48,             // 8020 PHA           NMI handler
        0xE6, 0x11,       // 8021 INC $11
        0xAD, 0x10, 0x42, // 8023 LDA $4210     acknowledge
        0x68,             // 8026 PLA
        0x40,             // 8027 RTI
    ];
    #[rustfmt::skip]
    let leaf: &[u8] = &[
        0xE6, 0x10,       // 8040 INC $10       leaf
        0x60,             // 8042 RTS
    ];
    rom[..main.len()].copy_from_slice(main);
    rom[0x20..0x20 + nmi.len()].copy_from_slice(nmi);
    rom[0x40..0x40 + leaf.len()].copy_from_slice(leaf);
    write_header(&mut rom, b"SSD NMI LEAF TEST    ", 0x00, 0x00, 0x801D);
    set_native_nmi_vector(&mut rom, NMI_HANDLER);
    rom
}

/// Leaf of [`dma_leaf_rom`], entered right after the `STA $420B` that starts a GPDMA.
pub const DMA_LEAF: u32 = 0x00_8045;
/// WRAM index where every GPDMA of [`dma_leaf_rom`] copies ROM `$00:8000..$8010`.
pub const DMA_DESTINATION: usize = 0x200;

/// Calls `JSR $8040` in a loop like [`jsr_leaf_rom`]; the callee starts a 16-byte GPDMA of ROM
/// `$00:8000` to WRAM [`DMA_DESTINATION`] (`LDA #$01; STA $420B`) and falls through into the leaf
/// `INC $10; RTS` at [`DMA_LEAF`]. Before each call the loop zeroes the first destination byte.
///
/// Native mode, 8-bit A, 16-bit X/Y; NMI is disabled.
pub fn dma_leaf_rom() -> Vec<u8> {
    let mut rom = vec![0xFF_u8; 0x8000];
    #[rustfmt::skip] // keep the one-instruction-per-line layout with address comments
    let main: &[u8] = &[
        0x78,             // 8000 SEI
        0x18,             // 8001 CLC
        0xFB,             // 8002 XCE           -> native mode
        0xC2, 0x10,       // 8003 REP #$10      -> 16-bit X, Y
        0xE2, 0x20,       // 8005 SEP #$20      -> 8-bit A
        0x9C, 0x00, 0x43, // 8007 STZ $4300     channel 0: A to B, increment, one register
        0xA9, 0x80,       // 800A LDA #$80
        0x8D, 0x01, 0x43, // 800C STA $4301     B = WMDATA ($2180)
        0x8D, 0x03, 0x43, // 800F STA $4303     A = $xx:80xx
        0x9C, 0x04, 0x43, // 8012 STZ $4304     A bank = $00
        0x9C, 0x02, 0x43, // 8015 STZ $4302     loop: A = $00:8000
        0xA9, 0x10,       // 8018 LDA #$10
        0x8D, 0x05, 0x43, // 801A STA $4305     16 bytes
        0x9C, 0x06, 0x43, // 801D STZ $4306
        0x9C, 0x81, 0x21, // 8020 STZ $2181     WMADD = $00:0200
        0xA9, 0x02,       // 8023 LDA #$02
        0x8D, 0x82, 0x21, // 8025 STA $2182
        0x9C, 0x83, 0x21, // 8028 STZ $2183
        0x9C, 0x00, 0x02, // 802B STZ $0200     zero the first destination byte
        0x20, 0x40, 0x80, // 802E JSR $8040
        0xA2, 0xC8, 0x00, // 8031 LDX #$00C8    200 delay iterations
        0xCA,             // 8034 DEX           delay:
        0xD0, 0xFD,       // 8035 BNE $8034     (0x34 - 0x37 = -3)
        0x80, 0xDC,       // 8037 BRA $8015     (0x15 - 0x39 = -36)
        0x40,             // 8039 RTI           target of every unused vector
    ];
    #[rustfmt::skip]
    let callee: &[u8] = &[
        0xA9, 0x01,       // 8040 LDA #$01
        0x8D, 0x0B, 0x42, // 8042 STA $420B     start channel 0
        0xE6, 0x10,       // 8045 INC $10       leaf: entered with the GPDMA requested
        0x60,             // 8047 RTS
    ];
    rom[..main.len()].copy_from_slice(main);
    rom[0x40..0x40 + callee.len()].copy_from_slice(callee);
    write_header(&mut rom, b"SSD DMA LEAF TEST    ", 0x00, 0x00, 0x8039);
    rom
}

/// Calls a leaf that never returns: `JSR $8040`, with `BRA $8040` at [`LEAF`]. Every iteration of
/// the leaf is a new arrival at [`LEAF`].
///
/// Native mode, 8-bit A, 16-bit X/Y; NMI is disabled.
pub fn no_return_rom() -> Vec<u8> {
    let mut rom = vec![0xFF_u8; 0x8000];
    #[rustfmt::skip] // keep the one-instruction-per-line layout with address comments
    let main: &[u8] = &[
        0x78,             // 8000 SEI
        0x18,             // 8001 CLC
        0xFB,             // 8002 XCE           -> native mode
        0xC2, 0x10,       // 8003 REP #$10      -> 16-bit X, Y
        0xE2, 0x20,       // 8005 SEP #$20      -> 8-bit A
        0x20, 0x40, 0x80, // 8007 JSR $8040     never returns
        0x80, 0xFB,       // 800A BRA $8007     (0x07 - 0x0C = -5)
        0x40,             // 800C RTI           target of every unused vector
    ];
    #[rustfmt::skip]
    let leaf: &[u8] = &[
        0x80, 0xFE,       // 8040 BRA $8040     leaf: spins forever
    ];
    rom[..main.len()].copy_from_slice(main);
    rom[0x40..0x40 + leaf.len()].copy_from_slice(leaf);
    write_header(&mut rom, b"SSD NO RETURN TEST   ", 0x00, 0x00, 0x800C);
    rom
}

/// WRAM index the NMI handler of [`nmi_writer_rom`] increments (`$00:0020`).
pub const NMI_WRITER_TARGET: usize = 0x20;

/// Calls a long leaf in a tight loop with the NMI on, so the NMI usually lands inside the leaf.
/// The leaf at [`LEAF`] increments `$10`, then counts X down from `$0100` to 0 (`DEX; BNE`) and
/// returns with X = 0, Z set and N clear. The NMI handler preserves A, increments
/// [`NMI_WRITER_TARGET`] and acknowledges the NMI.
///
/// Native mode, 8-bit A, 16-bit X/Y, S = [`MAIN_STACK`].
pub fn nmi_writer_rom() -> Vec<u8> {
    let mut rom = vec![0xFF_u8; 0x8000];
    #[rustfmt::skip] // keep the one-instruction-per-line layout with address comments
    let main: &[u8] = &[
        0x78,             // 8000 SEI
        0x18,             // 8001 CLC
        0xFB,             // 8002 XCE           -> native mode
        0xC2, 0x10,       // 8003 REP #$10      -> 16-bit X, Y
        0xE2, 0x20,       // 8005 SEP #$20      -> 8-bit A
        0xA2, 0xF0, 0x1F, // 8007 LDX #$1FF0
        0x9A,             // 800A TXS
        0xA9, 0x80,       // 800B LDA #$80
        0x8D, 0x00, 0x42, // 800D STA $4200     NMI on
        0x20, 0x40, 0x80, // 8010 JSR $8040     loop: call the leaf
        0x80, 0xFB,       // 8013 BRA $8010     (0x10 - 0x15 = -5)
        0x40,             // 8015 RTI           target of every unused vector
    ];
    #[rustfmt::skip]
    let nmi: &[u8] = &[
        0x48,             // 8020 PHA           NMI handler
        0xE6, 0x20,       // 8021 INC $20
        0xAD, 0x10, 0x42, // 8023 LDA $4210     acknowledge
        0x68,             // 8026 PLA
        0x40,             // 8027 RTI
    ];
    #[rustfmt::skip]
    let leaf: &[u8] = &[
        0xE6, 0x10,       // 8040 INC $10       leaf
        0xA2, 0x00, 0x01, // 8042 LDX #$0100    256 delay iterations
        0xCA,             // 8045 DEX           delay:
        0xD0, 0xFD,       // 8046 BNE $8045     (0x45 - 0x48 = -3)
        0x60,             // 8048 RTS
    ];
    rom[..main.len()].copy_from_slice(main);
    rom[0x20..0x20 + nmi.len()].copy_from_slice(nmi);
    rom[0x40..0x40 + leaf.len()].copy_from_slice(leaf);
    write_header(&mut rom, b"SSD NMI WRITER TEST  ", 0x00, 0x00, 0x8015);
    set_native_nmi_vector(&mut rom, NMI_HANDLER);
    rom
}

/// WRAM index of the first byte the leaf of [`stack_switch_rom`] fills (`$00:00ED`).
pub const STACK_SWITCH_BUFFER: usize = 0xED;
/// What the leaf of [`stack_switch_rom`] leaves at [`STACK_SWITCH_BUFFER`] onwards.
pub const STACK_SWITCH_FILL: [u8; 4] = [0x78, 0x56, 0x34, 0x12];

/// [`jsr_leaf_rom`]'s loop calling a leaf that fills a WRAM buffer below the stack by moving the
/// stack onto it: `TSX`, `REP #$20`, `LDA #$00F0`, `TCS`, `PEA $1234`, `PEA $5678`, `TXS`,
/// `SEP #$20`, `RTS`. The buffer ends up as [`STACK_SWITCH_FILL`] at [`STACK_SWITCH_BUFFER`];
/// the leaf returns with A = `$00F0`, X = its entry S, N and Z clear.
pub fn stack_switch_rom() -> Vec<u8> {
    #[rustfmt::skip]
    let leaf: &[u8] = &[
        0xBA,             // 8040 TSX           save S
        0xC2, 0x20,       // 8041 REP #$20      -> 16-bit A
        0xA9, 0xF0, 0x00, // 8043 LDA #$00F0
        0x1B,             // 8046 TCS           S = $00F0: the pushes fill $00ED-$00F0
        0xF4, 0x34, 0x12, // 8047 PEA $1234
        0xF4, 0x78, 0x56, // 804A PEA $5678
        0x9A,             // 804D TXS           restore S
        0xE2, 0x20,       // 804E SEP #$20      -> 8-bit A
        0x60,             // 8050 RTS
    ];
    jsr_loop_rom(leaf, b"SSD STACK SWITCH TEST")
}

/// X in the main loop of [`stack_leaf_rom`]: the value its leaf pushes.
pub const STACK_LEAF_X: u16 = MAIN_STACK;

/// A leaf that saves X on the stack: `PHX; INC $10; PLX; RTS` at [`LEAF`], called from a loop
/// with the NMI on. The loop waits 200 iterations of `DEY; BNE` between calls, so the NMI
/// usually lands outside the leaf and its handler (as in [`nmi_leaf_rom`]: `PHA`, `INC $11`,
/// acknowledge, `PLA`, `RTI`) overwrites the stack bytes the leaf pushes X to. X is
/// [`STACK_LEAF_X`] throughout; the leaf returns with N and Z from it (`PLX`).
///
/// Native mode, 8-bit A, 16-bit X/Y, S = [`MAIN_STACK`] in the loop.
pub fn stack_leaf_rom() -> Vec<u8> {
    let mut rom = vec![0xFF_u8; 0x8000];
    #[rustfmt::skip] // keep the one-instruction-per-line layout with address comments
    let main: &[u8] = &[
        0x78,             // 8000 SEI
        0x18,             // 8001 CLC
        0xFB,             // 8002 XCE           -> native mode
        0xC2, 0x10,       // 8003 REP #$10      -> 16-bit X, Y
        0xE2, 0x20,       // 8005 SEP #$20      -> 8-bit A
        0xA2, 0xF0, 0x1F, // 8007 LDX #$1FF0
        0x9A,             // 800A TXS
        0x64, 0x11,       // 800B STZ $11
        0xA9, 0x80,       // 800D LDA #$80
        0x8D, 0x00, 0x42, // 800F STA $4200     NMI on
        0xA0, 0xC8, 0x00, // 8012 LDY #$00C8    loop: 200 delay iterations
        0x88,             // 8015 DEY           delay:
        0xD0, 0xFD,       // 8016 BNE $8015     (0x15 - 0x18 = -3)
        0x20, 0x40, 0x80, // 8018 JSR $8040     call the leaf
        0x80, 0xF5,       // 801B BRA $8012     (0x12 - 0x1D = -11)
        0x40,             // 801D RTI           target of every unused vector
    ];
    #[rustfmt::skip]
    let nmi: &[u8] = &[
        0x48,             // 8020 PHA           NMI handler
        0xE6, 0x11,       // 8021 INC $11
        0xAD, 0x10, 0x42, // 8023 LDA $4210     acknowledge
        0x68,             // 8026 PLA
        0x40,             // 8027 RTI
    ];
    #[rustfmt::skip]
    let leaf: &[u8] = &[
        0xDA,             // 8040 PHX           leaf
        0xE6, 0x10,       // 8041 INC $10
        0xFA,             // 8043 PLX
        0x60,             // 8044 RTS
    ];
    rom[..main.len()].copy_from_slice(main);
    rom[0x20..0x20 + nmi.len()].copy_from_slice(nmi);
    rom[0x40..0x40 + leaf.len()].copy_from_slice(leaf);
    write_header(&mut rom, b"SSD STACK LEAF TEST  ", 0x00, 0x00, 0x801D);
    set_native_nmi_vector(&mut rom, NMI_HANDLER);
    rom
}

/// HDMA copy units per frame in [`hdma_leaf_rom`]: one per visible scanline (0–224).
pub const HDMA_UNITS_PER_FRAME: u64 = 225;

/// [`nmi_writer_rom`]'s long leaf (`INC $10`, 256 iterations of `DEX; BNE`, `RTS`; about 7
/// scanlines) called in a tight loop with HDMA channel 1 on: every visible scanline it copies one
/// byte to COLDATA (`$2132`) from a table in unused ROM (`$00:9000`, all `$FF`: line counts of
/// `$FF` repeat 127 lines, and the data byte sets the fixed color to white, which nothing uses).
///
/// Native mode, 8-bit A, 16-bit X/Y; NMI is disabled. Until the first frame-start reload the
/// channel runs from its power-on line counter and table address (also harmless writes to
/// COLDATA).
pub fn hdma_leaf_rom() -> Vec<u8> {
    let mut rom = vec![0xFF_u8; 0x8000];
    #[rustfmt::skip] // keep the one-instruction-per-line layout with address comments
    let main: &[u8] = &[
        0x78,             // 8000 SEI
        0x18,             // 8001 CLC
        0xFB,             // 8002 XCE           -> native mode
        0xC2, 0x10,       // 8003 REP #$10      -> 16-bit X, Y
        0xE2, 0x20,       // 8005 SEP #$20      -> 8-bit A
        0x9C, 0x10, 0x43, // 8007 STZ $4310     channel 1: direct table, A to B, one register
        0xA9, 0x32,       // 800A LDA #$32
        0x8D, 0x11, 0x43, // 800C STA $4311     B = COLDATA ($2132)
        0x9C, 0x12, 0x43, // 800F STZ $4312     table = $00:9000
        0xA9, 0x90,       // 8012 LDA #$90
        0x8D, 0x13, 0x43, // 8014 STA $4313
        0x9C, 0x14, 0x43, // 8017 STZ $4314
        0xA9, 0x02,       // 801A LDA #$02
        0x8D, 0x0C, 0x42, // 801C STA $420C     HDMA channel 1 on
        0x20, 0x40, 0x80, // 801F JSR $8040     loop: call the leaf
        0x80, 0xFB,       // 8022 BRA $801F     (0x1F - 0x24 = -5)
        0x40,             // 8024 RTI           target of every unused vector
    ];
    #[rustfmt::skip]
    let leaf: &[u8] = &[
        0xE6, 0x10,       // 8040 INC $10       leaf
        0xA2, 0x00, 0x01, // 8042 LDX #$0100    256 delay iterations
        0xCA,             // 8045 DEX           delay:
        0xD0, 0xFD,       // 8046 BNE $8045     (0x45 - 0x48 = -3)
        0x60,             // 8048 RTS
    ];
    rom[..main.len()].copy_from_slice(main);
    rom[0x40..0x40 + leaf.len()].copy_from_slice(leaf);
    write_header(&mut rom, b"SSD HDMA LEAF TEST   ", 0x00, 0x00, 0x8024);
    rom
}

/// [`jsr_leaf_rom`]'s short leaf (`INC $10; RTS` at [`LEAF`], called every 5 scanlines or so) with
/// HDMA channel 1 on in forced blank: every scanline of the display it writes one byte of a table
/// in unused ROM (`$00:9000`: two runs of 127 lines, the data byte of line `n` being `n`) to
/// CGDATA (`$2122`), so HDMA changes palette RAM and a call HDMA crosses can show it.
///
/// Native mode, 8-bit A, 16-bit X/Y; NMI is disabled.
pub fn hdma_cgram_leaf_rom() -> Vec<u8> {
    let mut rom = vec![0xFF_u8; 0x8000];
    #[rustfmt::skip] // keep the one-instruction-per-line layout with address comments
    let main: &[u8] = &[
        0x78,             // 8000 SEI
        0x18,             // 8001 CLC
        0xFB,             // 8002 XCE           -> native mode
        0xC2, 0x10,       // 8003 REP #$10      -> 16-bit X, Y
        0xE2, 0x20,       // 8005 SEP #$20      -> 8-bit A
        0xA9, 0x80,       // 8007 LDA #$80
        0x8D, 0x00, 0x21, // 8009 STA $2100     forced blank: palette RAM is always writable
        0x9C, 0x21, 0x21, // 800C STZ $2121     CGRAM address 0
        0x9C, 0x10, 0x43, // 800F STZ $4310     channel 1: direct table, A to B, one register
        0xA9, 0x22,       // 8012 LDA #$22
        0x8D, 0x11, 0x43, // 8014 STA $4311     B = CGDATA ($2122)
        0x9C, 0x12, 0x43, // 8017 STZ $4312     table = $00:9000
        0xA9, 0x90,       // 801A LDA #$90
        0x8D, 0x13, 0x43, // 801C STA $4313
        0x9C, 0x14, 0x43, // 801F STZ $4314
        0xA9, 0x02,       // 8022 LDA #$02
        0x8D, 0x0C, 0x42, // 8024 STA $420C     HDMA channel 1 on
        0x20, 0x40, 0x80, // 8027 JSR $8040     loop: call the leaf
        0xA2, 0xC8, 0x00, // 802A LDX #$00C8    200 delay iterations
        0xCA,             // 802D DEX           delay:
        0xD0, 0xFD,       // 802E BNE $802D     (0x2D - 0x30 = -3)
        0x80, 0xF5,       // 8030 BRA $8027     (0x27 - 0x32 = -11)
        0x40,             // 8032 RTI           target of every unused vector
    ];
    let leaf: &[u8] = &[0xE6, 0x10, 0x60]; // 8040 INC $10; RTS
    rom[..main.len()].copy_from_slice(main);
    rom[0x40..0x40 + leaf.len()].copy_from_slice(leaf);
    // The table: a count of 127 lines with the repeat flag, then a byte for each line; twice
    for run in 0..2 {
        let table = 0x1000 + run * 128;
        rom[table] = 0xFF;
        for line in 0..127_u8 {
            rom[table + 1 + usize::from(line)] = line;
        }
    }
    rom[0x1100] = 0x00; // the end of the table
    write_header(&mut rom, b"SSD HDMA CGRAM TEST  ", 0x00, 0x00, 0x8032);
    rom
}

/// Address of the routine of [`recursive_leaf_rom`].
pub const RECURSIVE_LEAF: u32 = 0x00_8040;

/// [`jsr_leaf_rom`]'s loop calling a routine that calls itself while its counter is odd: `INC $10;
/// LDA $10; LSR A; BCC done; JSR $8040; done: RTS`. Entered with the counter odd after the
/// increment it calls itself once, and the second call, which makes it even, returns at once; so
/// from the second call of the loop on, every call recurses. It returns with the counter even
/// (`$10`), A = the counter shifted right, N clear, Z from A and C clear.
pub fn recursive_leaf_rom() -> Vec<u8> {
    #[rustfmt::skip]
    let leaf: &[u8] = &[
        0xE6, 0x10,       // 8040 INC $10       routine
        0xA5, 0x10,       // 8042 LDA $10
        0x4A,             // 8044 LSR A         carry = the counter's bit 0
        0x90, 0x03,       // 8045 BCC $804A     even: done (0x4A - 0x47 = 3)
        0x20, 0x40, 0x80, // 8047 JSR $8040     odd: recursion
        0x60,             // 804A RTS
    ];
    jsr_loop_rom(leaf, b"SSD RECURSIVE LEAF   ")
}

/// The idle loop of [`sync_idle_rom`] (`LDA $24`), its sync point.
pub const SYNC_IDLE: u32 = 0x00_801B;
/// WRAM index of the frame flag of [`sync_idle_rom`] (`$00:0024`): the NMI handler sets bit 7,
/// the main loop clears the byte on its first pass through the idle loop after it.
pub const SYNC_FLAG: usize = 0x24;
/// WRAM index of the 16-bit length of the frame task of [`sync_idle_rom`] (`$00:0026`), in
/// iterations of `DEX; BNE` (36 master cycles each); 0 runs no task.
pub const SYNC_TASK_LENGTH: usize = 0x26;
/// The frame task length [`sync_idle_rom`] starts with: about 27 scanlines, well inside vblank.
pub const SYNC_TASK_DEFAULT: u16 = 0x0400;
/// WRAM index of the 8-bit count of NMIs the handler of [`sync_idle_rom`] has entered
/// (`$00:0028`), nested ones included.
pub const SYNC_NMI_COUNTER: usize = 0x28;

/// A game-like frame loop: the per-frame work runs in the NMI handler and the main loop waits
/// for it in an idle loop at [`SYNC_IDLE`] (`loop: LDA $24; BPL loop`), then clears the frame
/// flag ([`SYNC_FLAG`]) and waits again.
///
/// The NMI handler (A and X preserved) acknowledges the NMI, counts it in [`SYNC_NMI_COUNTER`],
/// sets bit 7 of the frame flag and runs the frame task: [`SYNC_TASK_LENGTH`] iterations of
/// `DEX; BNE`. An NMI that arrives while the task runs (the task is longer than a frame) only
/// counts itself and sets the flag, guarded by a busy byte at `$25`, so the frame it starts is a
/// lag frame.
///
/// Native mode, 8-bit A, 16-bit X/Y, S = [`MAIN_STACK`]; NMI on, auto-joypad read off.
pub fn sync_idle_rom() -> Vec<u8> {
    idle_rom(true, b"SSD SYNC IDLE TEST   ")
}

/// [`sync_idle_rom`] with the NMI never enabled, as a game stuck in a transition loop: the idle
/// loop at [`SYNC_IDLE`] runs on, and no sync event ever comes.
pub fn nmi_off_idle_rom() -> Vec<u8> {
    idle_rom(false, b"SSD NMI OFF IDLE TEST")
}

fn idle_rom(nmi: bool, title: &[u8; 21]) -> Vec<u8> {
    let mut rom = vec![0xFF_u8; 0x8000];
    let nmitimen = if nmi { 0x80 } else { 0x00 };
    let [task_lo, task_hi] = SYNC_TASK_DEFAULT.to_le_bytes();
    #[rustfmt::skip] // keep the one-instruction-per-line layout with address comments
    let main: &[u8] = &[
        0x78,                   // 8000 SEI
        0x18,                   // 8001 CLC
        0xFB,                   // 8002 XCE           -> native mode
        0xC2, 0x10,             // 8003 REP #$10      -> 16-bit X, Y
        0xE2, 0x20,             // 8005 SEP #$20      -> 8-bit A
        0xA2, 0xF0, 0x1F,       // 8007 LDX #$1FF0
        0x9A,                   // 800A TXS
        0x64, 0x24,             // 800B STZ $24       frame flag
        0x64, 0x25,             // 800D STZ $25       frame task busy
        0x64, 0x28,             // 800F STZ $28       NMI counter
        0xA2, task_lo, task_hi, // 8011 LDX #default  frame task length
        0x86, 0x26,             // 8014 STX $26
        0xA9, nmitimen,         // 8016 LDA #nmi
        0x8D, 0x00, 0x42,       // 8018 STA $4200     NMI on (or off)
        0xA5, 0x24,             // 801B LDA $24       idle: wait for the frame flag
        0x10, 0xFC,             // 801D BPL $801B     (0x1B - 0x1F = -4)
        0x64, 0x24,             // 801F STZ $24       clear it
        0x80, 0xF8,             // 8021 BRA $801B     (0x1B - 0x23 = -8)
        0x40,                   // 8023 RTI           target of every unused vector
    ];
    #[rustfmt::skip]
    let nmi: &[u8] = &[
        0x48,             // 8040 PHA           NMI handler
        0xDA,             // 8041 PHX
        0xAD, 0x10, 0x42, // 8042 LDA $4210     acknowledge
        0xE6, 0x28,       // 8045 INC $28       count the NMI
        0xA9, 0x80,       // 8047 LDA #$80
        0x04, 0x24,       // 8049 TSB $24       a frame is ready
        0xA5, 0x25,       // 804B LDA $25       frame task already running?
        0xD0, 0x0B,       // 804D BNE $805A     yes: lag frame (0x5A - 0x4F = 11)
        0xE6, 0x25,       // 804F INC $25
        0xA6, 0x26,       // 8051 LDX $26       frame task: $26 iterations
        0xF0, 0x03,       // 8053 BEQ $8058     (0x58 - 0x55 = 3)
        0xCA,             // 8055 DEX           task:
        0xD0, 0xFD,       // 8056 BNE $8055     (0x55 - 0x58 = -3)
        0x64, 0x25,       // 8058 STZ $25
        0xFA,             // 805A PLX
        0x68,             // 805B PLA
        0x40,             // 805C RTI
    ];
    rom[..main.len()].copy_from_slice(main);
    rom[0x40..0x40 + nmi.len()].copy_from_slice(nmi);
    write_header(&mut rom, title, 0x00, 0x00, 0x8023);
    set_native_nmi_vector(&mut rom, 0x8040);
    rom
}

/// WRAM index of the second counter of the leaf of [`two_path_leaf_rom`] (`$00:0011`).
pub const TWO_PATH_ODD_COUNTER: usize = 0x11;

/// [`jsr_leaf_rom`]'s loop calling a leaf with two paths, chosen by bit 0 of [`LEAF_COUNTER`]:
/// `LDA $10; AND #$01; BEQ +2; INC $11; INC $10; RTS`. On an odd counter it also increments
/// [`TWO_PATH_ODD_COUNTER`]; it returns with A = the old bit 0 and N/Z from the new counter.
/// The calls alternate between the paths.
pub fn two_path_leaf_rom() -> Vec<u8> {
    #[rustfmt::skip]
    let leaf: &[u8] = &[
        0xA5, 0x10,       // 8040 LDA $10       leaf
        0x29, 0x01,       // 8042 AND #$01
        0xF0, 0x02,       // 8044 BEQ $8048     even: skip
        0xE6, 0x11,       // 8046 INC $11       odd only
        0xE6, 0x10,       // 8048 INC $10
        0x60,             // 804A RTS
    ];
    jsr_loop_rom(leaf, b"SSD TWO PATH TEST    ")
}

/// The idle loop of [`nmi_race_rom`] and [`hdma_idle_rom`] (`JSR` to the leaf), their sync
/// point.
pub const RACE_IDLE: u32 = 0x00_8017;
/// The RNG-like leaf of [`nmi_race_rom`] and [`hdma_idle_rom`].
pub const RACE_LEAF: u32 = 0x00_8060;
/// WRAM index of the 16-bit seed of [`nmi_race_rom`] (`$00:0040`), which the leaf and the NMI
/// handler both advance.
pub const RACE_SEED: usize = 0x40;
/// WRAM index of the frame flag of [`nmi_race_rom`] (`$00:0024`): the NMI handler sets bit 7,
/// the idle loop clears the byte after its first call following it.
pub const RACE_FLAG: usize = 0x24;
/// WRAM index of the 8-bit count of NMIs the handler of [`nmi_race_rom`] has entered
/// (`$00:0042`).
pub const RACE_NMI_COUNTER: usize = 0x42;

/// CPU-busy master cycles of the leaf of [`nmi_race_rom`]: `REP` 22, 16-bit `LDA dp` 32,
/// `ASL A` 14, `CLC` 14, 16-bit `ADC dp` 32, `CLC` 14, 16-bit `ADC #` 24, 16-bit `STA dp` 32,
/// `SEP` 22, `RTS` 42.
pub const RACE_LEAF_COST: u64 = 248;

/// The leaf of [`nmi_race_rom`] in Rust: `seed = seed * 3 + $1D`, A = the new seed, N/V/Z/C
/// from the final addition; declares [`RACE_LEAF_COST`].
pub fn race_leaf(ctx: &mut Ctx<'_>) {
    let seed = ctx.read_dp16(RACE_SEED as u16);
    // `seed * 2 + seed`; the carry of that addition is cleared before the next one.
    let tripled = seed.wrapping_mul(3);
    let (next, carry) = tripled.overflowing_add(0x001D);
    let overflow = (tripled ^ next) & (0x001D ^ next) & 0x8000 != 0;
    ctx.write_dp16(RACE_SEED as u16, next);
    ctx.set_a(next);
    ctx.set_nz16(next);
    ctx.set_v(overflow);
    ctx.set_c(carry);
    ctx.set_cost(RACE_LEAF_COST);
}

/// A game-like idle loop whose NMI usually lands inside the leaf it calls: at [`RACE_IDLE`] the
/// loop calls an RNG-like leaf at [`RACE_LEAF`] (`seed = seed * 3 + $1D` on the 16-bit seed at
/// [`RACE_SEED`], computed as `seed + seed * 2`, 16-bit accumulator inside, returning with A =
/// the new seed and N/V/Z/C from the final addition), then tests the frame flag
/// ([`RACE_FLAG`]) and clears it once set. The leaf takes about three quarters of each
/// iteration.
///
/// The NMI handler (A and P preserved) flips bits of the seed (`EOR #$5A5A`), acknowledges the
/// NMI, sets bit 7 of the frame flag and counts itself in [`RACE_NMI_COUNTER`]. When it lands
/// between the leaf's load and store of the seed, the leaf's store overwrites the flip: the
/// outcome depends on where in the leaf the NMI lands.
///
/// Native mode, 8-bit A, 16-bit X/Y, S = [`MAIN_STACK`]; NMI on, no HDMA.
pub fn nmi_race_rom() -> Vec<u8> {
    race_rom(false, b"SSD NMI RACE TEST    ")
}

/// [`nmi_race_rom`] with HDMA channel 1 on during the idle loop, as in [`hdma_leaf_rom`]: every
/// visible scanline it copies one byte from unused ROM to COLDATA.
pub fn hdma_idle_rom() -> Vec<u8> {
    race_rom(true, b"SSD HDMA IDLE TEST   ")
}

fn race_rom(hdma: bool, title: &[u8; 21]) -> Vec<u8> {
    let mut rom = vec![0xFF_u8; 0x8000];
    #[rustfmt::skip] // keep the one-instruction-per-line layout with address comments
    let main: &[u8] = &[
        0x78,             // 8000 SEI
        0x18,             // 8001 CLC
        0xFB,             // 8002 XCE           -> native mode
        0xC2, 0x10,       // 8003 REP #$10      -> 16-bit X, Y
        0xE2, 0x20,       // 8005 SEP #$20      -> 8-bit A
        0xA2, 0xF0, 0x1F, // 8007 LDX #$1FF0
        0x9A,             // 800A TXS
        0x64, 0x24,       // 800B STZ $24       frame flag
        0x64, 0x42,       // 800D STZ $42       NMI counter
        0x20, 0xA0, 0x80, // 800F JSR $80A0     HDMA setup (hdma_idle_rom only)
        0xA9, 0x80,       // 8012 LDA #$80
        0x8D, 0x00, 0x42, // 8014 STA $4200     NMI on
        0x20, 0x60, 0x80, // 8017 JSR $8060     idle: call the leaf
        0xA5, 0x24,       // 801A LDA $24
        0x10, 0xF9,       // 801C BPL $8017     (0x17 - 0x1E = -7)
        0x64, 0x24,       // 801E STZ $24
        0x80, 0xF5,       // 8020 BRA $8017     (0x17 - 0x22 = -11)
        0x40,             // 8022 RTI           target of every unused vector
    ];
    #[rustfmt::skip]
    let nmi: &[u8] = &[
        0x48,             // 8040 PHA           NMI handler (A may be 16-bit)
        0x08,             // 8041 PHP
        0xC2, 0x20,       // 8042 REP #$20
        0xA5, 0x40,       // 8044 LDA $40
        0x49, 0x5A, 0x5A, // 8046 EOR #$5A5A    flip bits of the seed
        0x85, 0x40,       // 8049 STA $40
        0xE2, 0x20,       // 804B SEP #$20
        0xAD, 0x10, 0x42, // 804D LDA $4210     acknowledge
        0xA9, 0x80,       // 8050 LDA #$80
        0x04, 0x24,       // 8052 TSB $24       a frame is ready
        0xE6, 0x42,       // 8054 INC $42       count the NMI
        0x28,             // 8056 PLP
        0x68,             // 8057 PLA
        0x40,             // 8058 RTI
    ];
    #[rustfmt::skip]
    let leaf: &[u8] = &[
        0xC2, 0x20,       // 8060 REP #$20      leaf: 16-bit A
        0xA5, 0x40,       // 8062 LDA $40
        0x0A,             // 8064 ASL A         seed * 2
        0x18,             // 8065 CLC
        0x65, 0x40,       // 8066 ADC $40       + seed
        0x18,             // 8068 CLC
        0x69, 0x1D, 0x00, // 8069 ADC #$001D
        0x85, 0x40,       // 806C STA $40
        0xE2, 0x20,       // 806E SEP #$20
        0x60,             // 8070 RTS
    ];
    #[rustfmt::skip]
    let hdma_setup: &[u8] = &[
        0x9C, 0x10, 0x43, // 80A0 STZ $4310     channel 1: direct table, A to B, one register
        0xA9, 0x32,       // 80A3 LDA #$32
        0x8D, 0x11, 0x43, // 80A5 STA $4311     B = COLDATA ($2132)
        0x9C, 0x12, 0x43, // 80A8 STZ $4312     table = $00:9000 (unused ROM, all $FF)
        0xA9, 0x90,       // 80AB LDA #$90
        0x8D, 0x13, 0x43, // 80AD STA $4313
        0x9C, 0x14, 0x43, // 80B0 STZ $4314
        0xA9, 0x02,       // 80B3 LDA #$02
        0x8D, 0x0C, 0x42, // 80B5 STA $420C     HDMA channel 1 on
        0x60,             // 80B8 RTS
    ];
    let setup: &[u8] = if hdma { hdma_setup } else { &[0x60] }; // 80A0 RTS: nothing to set up
    rom[..main.len()].copy_from_slice(main);
    rom[0x40..0x40 + nmi.len()].copy_from_slice(nmi);
    rom[0x60..0x60 + leaf.len()].copy_from_slice(leaf);
    rom[0xA0..0xA0 + setup.len()].copy_from_slice(setup);
    write_header(&mut rom, title, 0x00, 0x00, 0x8022);
    set_native_nmi_vector(&mut rom, 0x8040);
    rom
}

/// The idle loop of [`deep_stack_rom`] (`JSR` to [`DEEP_LEAF`]), its sync point.
pub const DEEP_IDLE: u32 = 0x00_8017;
/// The routine of [`deep_stack_rom`] that pushes deep once, at boot.
pub const DEEP: u32 = 0x00_8060;
/// CPU-busy master cycles of [`DEEP`]: 8 `PHX` (16-bit) of 30, `REP` 22, `TSC` 14, `CLC` 14,
/// 16-bit `ADC #` 24, `TCS` 14, `SEP` 22, `RTS` 42.
pub const DEEP_COST: u64 = 392;
/// The leaf the idle loop of [`deep_stack_rom`] calls: `INC $10; RTS`.
pub const DEEP_LEAF: u32 = 0x00_8080;
/// CPU-busy master cycles of [`DEEP_LEAF`]: `INC dp` 38, `RTS` 42.
pub const DEEP_LEAF_COST: u64 = 80;
/// WRAM index of the counter [`DEEP_LEAF`] increments (`$00:0010`).
pub const DEEP_LEAF_COUNTER: usize = 0x10;
/// WRAM index of the 8-bit count of NMIs the handler of [`deep_stack_rom`] has entered
/// (`$00:0011`).
pub const DEEP_NMI_COUNTER: usize = 0x11;
/// The S [`DEEP`] is entered with (`JSR` from the boot code at S = [`MAIN_STACK`]).
pub const DEEP_ENTRY_S: u16 = MAIN_STACK - 2;
/// The lowest S the NMI handler of [`deep_stack_rom`] reaches: an NMI inside [`DEEP_LEAF`]
/// (S = [`MAIN_STACK`] − 2) pushes 4 bytes, then `PHA` one. No later code goes deeper.
pub const DEEP_NMI_LOW: u16 = MAIN_STACK - 7;

/// A stack pushed deep once, at boot, and never that deep again. Before turning the NMI on, the
/// boot code calls [`DEEP`] once: 8 `PHX` (16 bytes of X = [`MAIN_STACK`] below
/// [`DEEP_ENTRY_S`]), then frees them with `REP #$20; TSC; CLC; ADC #$0010; TCS; SEP #$20` (a
/// deallocation of 16 bytes in one step, no stack switch) and `RTS`; it returns with A = its
/// entry S and N/V/Z/C from the addition. The idle loop at [`DEEP_IDLE`] then calls the leaf
/// [`DEEP_LEAF`], tests the frame flag (`$24`) and clears it once set. The NMI handler (A
/// preserved) acknowledges the NMI, counts it in [`DEEP_NMI_COUNTER`] and sets the frame flag;
/// it reaches no lower than [`DEEP_NMI_LOW`].
///
/// Native mode, 8-bit A, 16-bit X/Y, S = [`MAIN_STACK`] in the loop.
pub fn deep_stack_rom() -> Vec<u8> {
    let mut rom = vec![0xFF_u8; 0x8000];
    #[rustfmt::skip] // keep the one-instruction-per-line layout with address comments
    let main: &[u8] = &[
        0x78,             // 8000 SEI
        0x18,             // 8001 CLC
        0xFB,             // 8002 XCE           -> native mode
        0xC2, 0x10,       // 8003 REP #$10      -> 16-bit X, Y
        0xE2, 0x20,       // 8005 SEP #$20      -> 8-bit A
        0xA2, 0xF0, 0x1F, // 8007 LDX #$1FF0
        0x9A,             // 800A TXS
        0x64, 0x24,       // 800B STZ $24       frame flag
        0x64, 0x11,       // 800D STZ $11       NMI counter
        0x20, 0x60, 0x80, // 800F JSR $8060     deep, once, NMI still off
        0xA9, 0x80,       // 8012 LDA #$80
        0x8D, 0x00, 0x42, // 8014 STA $4200     NMI on
        0x20, 0x80, 0x80, // 8017 JSR $8080     idle: call the leaf
        0xA5, 0x24,       // 801A LDA $24
        0x10, 0xF9,       // 801C BPL $8017     (0x17 - 0x1E = -7)
        0x64, 0x24,       // 801E STZ $24
        0x80, 0xF5,       // 8020 BRA $8017     (0x17 - 0x22 = -11)
        0x40,             // 8022 RTI           target of every unused vector
    ];
    #[rustfmt::skip]
    let nmi: &[u8] = &[
        0x48,             // 8040 PHA           NMI handler
        0xAD, 0x10, 0x42, // 8041 LDA $4210     acknowledge
        0xE6, 0x11,       // 8044 INC $11       count the NMI
        0xA9, 0x80,       // 8046 LDA #$80
        0x04, 0x24,       // 8048 TSB $24       a frame is ready
        0x68,             // 804A PLA
        0x40,             // 804B RTI
    ];
    #[rustfmt::skip]
    let deep: &[u8] = &[
        0xDA, 0xDA, 0xDA, 0xDA, // 8060 PHX x4
        0xDA, 0xDA, 0xDA, 0xDA, // 8064 PHX x4
        0xC2, 0x20,             // 8068 REP #$20
        0x3B,                   // 806A TSC
        0x18,                   // 806B CLC
        0x69, 0x10, 0x00,       // 806C ADC #$0010  free the 16 bytes
        0x1B,                   // 806F TCS
        0xE2, 0x20,             // 8070 SEP #$20
        0x60,                   // 8072 RTS
    ];
    #[rustfmt::skip]
    let leaf: &[u8] = &[
        0xE6, 0x10,       // 8080 INC $10       leaf
        0x60,             // 8082 RTS
    ];
    rom[..main.len()].copy_from_slice(main);
    rom[0x40..0x40 + nmi.len()].copy_from_slice(nmi);
    rom[0x60..0x60 + deep.len()].copy_from_slice(deep);
    rom[0x80..0x80 + leaf.len()].copy_from_slice(leaf);
    write_header(&mut rom, b"SSD DEEP STACK TEST  ", 0x00, 0x00, 0x8022);
    set_native_nmi_vector(&mut rom, 0x8040);
    rom
}

/// [`DEEP`] of [`deep_stack_rom`] in Rust, without its pushes: A = its entry S and N/V/Z/C as
/// the original's `ADC`; declares [`DEEP_COST`].
pub fn deep_in_rust(ctx: &mut Ctx<'_>) {
    let s = ctx.s();
    let pushed = s.wrapping_sub(16);
    let (sum, carry) = pushed.overflowing_add(0x0010);
    let overflow = (pushed ^ sum) & (0x0010 ^ sum) & 0x8000 != 0;
    ctx.set_a(sum);
    ctx.set_nz16(sum);
    ctx.set_v(overflow);
    ctx.set_c(carry);
    ctx.set_cost(DEEP_COST);
}

/// The main loop of [`call_rom`], its sync point: `JSR` to [`CALL_OUTER`].
pub const CALL_LOOP: u32 = 0x00_8016;
/// `outer` of [`call_rom`]: `JSR inner; INC $11; RTS`.
pub const CALL_OUTER: u32 = 0x00_8040;
/// Where `outer` of [`call_rom`] returns to: `JSR flat` in the main loop.
pub const CALL_OUTER_RETURN: u32 = 0x00_8019;
/// `flat` of [`call_rom`]: `INC $10; INC $11; RTS`, the code of `outer` with `inner` inlined.
pub const CALL_FLAT: u32 = 0x00_8050;
/// `outer_long` of [`call_rom`]: `JSL inner_long; INC $11; RTS`.
pub const CALL_OUTER_LONG: u32 = 0x00_8058;
/// `inner` of [`call_rom`]: `INC $10; RTS`.
pub const CALL_INNER: u32 = 0x00_8060;
/// `inner_long` of [`call_rom`]: `INC $10; RTL`.
pub const CALL_INNER_LONG: u32 = 0x00_8068;
/// A routine of [`call_rom`] that never returns (`BRA` to itself); nothing calls it.
pub const CALL_SPIN: u32 = 0x00_8070;
/// WRAM index of the counter `inner`, `inner_long` and `flat` of [`call_rom`] increment
/// (`$00:0010`).
pub const CALL_INNER_COUNTER: usize = 0x10;
/// WRAM index of the counter the callers of [`call_rom`] increment after their call (`$00:0011`).
pub const CALL_OUTER_COUNTER: usize = 0x11;
/// WRAM index of the count of NMIs the handler of [`call_rom`] has entered (`$00:0012`).
pub const CALL_NMI_COUNTER: usize = 0x12;
/// WRAM index where the NMI handler of [`call_rom`] copies [`CALL_OUTER_COUNTER`] (`$00:0013`).
pub const CALL_NMI_COPY: usize = 0x13;

/// Routines that call other routines, for Rust routines that call original code. The main loop
/// at [`CALL_LOOP`] calls, in a tight loop:
///
/// - `outer` ([`CALL_OUTER`]): `JSR inner; INC $11; RTS`, with `inner` ([`CALL_INNER`]) =
///   `INC $10; RTS`;
/// - `flat` ([`CALL_FLAT`]): `INC $10; INC $11; RTS`, the same work without the call;
/// - `outer_long` ([`CALL_OUTER_LONG`]): `JSL inner_long; INC $11; RTS`, with `inner_long`
///   ([`CALL_INNER_LONG`]) = `INC $10; RTL`.
///
/// Each returns with A unchanged and N/Z from its last `INC`. If `nmi`, the NMI is on and its
/// handler (A preserved) counts itself in [`CALL_NMI_COUNTER`] and copies [`CALL_OUTER_COUNTER`]
/// to [`CALL_NMI_COPY`], so where it lands in a call shows.
///
/// Native mode, 8-bit A, 16-bit X/Y, S = [`MAIN_STACK`] in the loop.
pub fn call_rom(nmi: bool) -> Vec<u8> {
    let mut rom = vec![0xFF_u8; 0x8000];
    let nmitimen = if nmi { 0x80 } else { 0x00 };
    #[rustfmt::skip] // keep the one-instruction-per-line layout with address comments
    let main: &[u8] = &[
        0x78,             // 8000 SEI
        0x18,             // 8001 CLC
        0xFB,             // 8002 XCE           -> native mode
        0xC2, 0x10,       // 8003 REP #$10      -> 16-bit X, Y
        0xE2, 0x20,       // 8005 SEP #$20      -> 8-bit A
        0xA2, 0xF0, 0x1F, // 8007 LDX #$1FF0
        0x9A,             // 800A TXS
        0x64, 0x10,       // 800B STZ $10
        0x64, 0x11,       // 800D STZ $11
        0x64, 0x12,       // 800F STZ $12
        0xA9, nmitimen,   // 8011 LDA #nmi
        0x8D, 0x00, 0x42, // 8013 STA $4200     NMI on or off
        0x20, 0x40, 0x80, // 8016 JSR $8040     loop: outer
        0x20, 0x50, 0x80, // 8019 JSR $8050     flat
        0x20, 0x58, 0x80, // 801C JSR $8058     outer_long
        0x80, 0xF5,       // 801F BRA $8016     (0x16 - 0x21 = -11)
        0x40,             // 8021 RTI           target of every unused vector
    ];
    #[rustfmt::skip]
    let nmi_handler: &[u8] = &[
        0x48,             // 8028 PHA           NMI handler
        0xE6, 0x12,       // 8029 INC $12       count the NMI
        0xA5, 0x11,       // 802B LDA $11
        0x85, 0x13,       // 802D STA $13       copy the outer counter
        0xAD, 0x10, 0x42, // 802F LDA $4210     acknowledge
        0x68,             // 8032 PLA
        0x40,             // 8033 RTI
    ];
    #[rustfmt::skip]
    let routines: &[(usize, &[u8])] = &[
        (0x40, &[
            0x20, 0x60, 0x80,       // 8040 JSR $8060     outer
            0xE6, 0x11,             // 8043 INC $11
            0x60,                   // 8045 RTS
        ]),
        (0x50, &[
            0xE6, 0x10,             // 8050 INC $10       flat
            0xE6, 0x11,             // 8052 INC $11
            0x60,                   // 8054 RTS
        ]),
        (0x58, &[
            0x22, 0x68, 0x80, 0x00, // 8058 JSL $008068   outer_long
            0xE6, 0x11,             // 805C INC $11
            0x60,                   // 805E RTS
        ]),
        (0x60, &[
            0xE6, 0x10,             // 8060 INC $10       inner
            0x60,                   // 8062 RTS
        ]),
        (0x68, &[
            0xE6, 0x10,             // 8068 INC $10       inner_long
            0x6B,                   // 806A RTL
        ]),
        (0x70, &[
            0x80, 0xFE,             // 8070 BRA $8070     spin
        ]),
    ];
    rom[..main.len()].copy_from_slice(main);
    rom[0x28..0x28 + nmi_handler.len()].copy_from_slice(nmi_handler);
    for &(offset, code) in routines {
        rom[offset..offset + code.len()].copy_from_slice(code);
    }
    write_header(&mut rom, b"SSD CALL ORIGINAL    ", 0x00, 0x00, 0x8021);
    set_native_nmi_vector(&mut rom, 0x8028);
    rom
}

/// `outer` of [`nested_rom`]: `LDA #$0F; STA $2100; JSR inner; STZ $2132; INC $11; RTS`.
pub const NESTED_OUTER: u32 = 0x00_8040;
/// `outer_tail` of [`nested_rom`]: `INC $11; JMP inner` (`inner` returns for it).
pub const NESTED_OUTER_TAIL: u32 = 0x00_8050;
/// `inner` of [`nested_rom`].
pub const NESTED_INNER: u32 = 0x00_8060;
/// WRAM index of the counter `inner` of [`nested_rom`] increments (`$00:0010`).
pub const NESTED_INNER_COUNTER: usize = 0x10;
/// WRAM index of the counter the callers of [`nested_rom`] increment (`$00:0011`).
pub const NESTED_OUTER_COUNTER: usize = 0x11;
/// WRAM index of the count of NMIs the handler of [`nested_rom`] has entered (`$00:0012`).
pub const NESTED_NMI_COUNTER: usize = 0x12;
/// The observable writes of a call of [`nested_rom`]'s `outer`: INIDISP before its call of
/// `inner`, COLDATA in `inner`, COLDATA again after it.
pub const NESTED_WRITES: [(u32, u8); 3] = [(0x00_2100, 0x0F), (0x00_2132, 0xE0), (0x00_2132, 0)];

/// Routines that call another routine, for nested `Verify` calls. The main loop loads X from the
/// two counters (`LDX $10`, so X changes every iteration), then calls:
///
/// - `outer` ([`NESTED_OUTER`]): `LDA #$0F; STA $2100; JSR inner; STZ $2132; INC $11; RTS`;
/// - `outer_tail` ([`NESTED_OUTER_TAIL`]): `INC $11; JMP inner`, a tail call.
///
/// `inner` ([`NESTED_INNER`]) pushes X, increments `$10`, writes `$E0` to COLDATA (`$2132`),
/// waits 16 iterations of `DEY; BNE`, pulls X and returns: A = `$E0`, Y = 0, X kept, N/Z from
/// X. `outer` returns with A = `$E0`, Y = 0 and N/Z from its `INC $11`. `inner` takes most of
/// each iteration. If `nmi`, the NMI is on and its handler (A preserved) counts itself in
/// [`NESTED_NMI_COUNTER`].
///
/// Native mode, 8-bit A, 16-bit X/Y, S = [`MAIN_STACK`] in the loop.
pub fn nested_rom(nmi: bool) -> Vec<u8> {
    let mut rom = vec![0xFF_u8; 0x8000];
    let nmitimen = if nmi { 0x80 } else { 0x00 };
    #[rustfmt::skip] // keep the one-instruction-per-line layout with address comments
    let main: &[u8] = &[
        0x78,             // 8000 SEI
        0x18,             // 8001 CLC
        0xFB,             // 8002 XCE           -> native mode
        0xC2, 0x10,       // 8003 REP #$10      -> 16-bit X, Y
        0xE2, 0x20,       // 8005 SEP #$20      -> 8-bit A
        0xA2, 0xF0, 0x1F, // 8007 LDX #$1FF0
        0x9A,             // 800A TXS
        0x64, 0x10,       // 800B STZ $10
        0x64, 0x11,       // 800D STZ $11
        0x64, 0x12,       // 800F STZ $12
        0xA9, nmitimen,   // 8011 LDA #nmi
        0x8D, 0x00, 0x42, // 8013 STA $4200     NMI on or off
        0xA6, 0x10,       // 8016 LDX $10       loop: X = the counters
        0x20, 0x40, 0x80, // 8018 JSR $8040     outer
        0x20, 0x50, 0x80, // 801B JSR $8050     outer_tail
        0x80, 0xF6,       // 801E BRA $8016     (0x16 - 0x20 = -10)
        0x40,             // 8020 RTI           target of every unused vector
    ];
    #[rustfmt::skip]
    let nmi_handler: &[u8] = &[
        0x48,             // 8028 PHA           NMI handler
        0xE6, 0x12,       // 8029 INC $12       count the NMI
        0xAD, 0x10, 0x42, // 802B LDA $4210     acknowledge
        0x68,             // 802E PLA
        0x40,             // 802F RTI
    ];
    #[rustfmt::skip]
    let routines: &[(usize, &[u8])] = &[
        (0x40, &[
            0xA9, 0x0F,             // 8040 LDA #$0F      outer
            0x8D, 0x00, 0x21,       // 8042 STA $2100     display on, brightness 15
            0x20, 0x60, 0x80,       // 8045 JSR $8060
            0x9C, 0x32, 0x21,       // 8048 STZ $2132     fixed color off again
            0xE6, 0x11,             // 804B INC $11
            0x60,                   // 804D RTS
        ]),
        (0x50, &[
            0xE6, 0x11,             // 8050 INC $11       outer_tail
            0x4C, 0x60, 0x80,       // 8052 JMP $8060
        ]),
        (0x60, &[
            0xDA,                   // 8060 PHX           inner
            0xE6, 0x10,             // 8061 INC $10
            0xA9, 0xE0,             // 8063 LDA #$E0
            0x8D, 0x32, 0x21,       // 8065 STA $2132     fixed color: R, G and B at 0
            0xA0, 0x10, 0x00,       // 8068 LDY #$0010    16 delay iterations
            0x88,                   // 806B DEY           delay:
            0xD0, 0xFD,             // 806C BNE $806B     (0x6B - 0x6E = -3)
            0xFA,                   // 806E PLX
            0x60,                   // 806F RTS
        ]),
    ];
    rom[..main.len()].copy_from_slice(main);
    rom[0x28..0x28 + nmi_handler.len()].copy_from_slice(nmi_handler);
    for &(offset, code) in routines {
        rom[offset..offset + code.len()].copy_from_slice(code);
    }
    write_header(&mut rom, b"SSD NESTED VERIFY    ", 0x00, 0x00, 0x8020);
    set_native_nmi_vector(&mut rom, 0x8028);
    rom
}

/// The long routine [`nmi_call_rom`]'s main loop calls: `INC $11`, 256 iterations of `DEX; BNE`,
/// `RTS`.
pub const NMI_CALL_LONG: u32 = 0x00_8040;
/// The short routine of [`nmi_call_rom`]: `INC $10; RTS`.
pub const NMI_CALL_SHORT: u32 = 0x00_8050;
/// WRAM index of the counter the short routine of [`nmi_call_rom`] increments (`$00:0010`).
pub const NMI_CALL_SHORT_COUNTER: usize = 0x10;
/// WRAM index of the counter the long routine of [`nmi_call_rom`] increments (`$00:0011`).
pub const NMI_CALL_LONG_COUNTER: usize = 0x11;
/// WRAM index of the count of NMIs the handler of [`nmi_call_rom`] has entered (`$00:0012`).
pub const NMI_CALL_NMI_COUNTER: usize = 0x12;

/// A main loop calling the long routine ([`NMI_CALL_LONG`]: `INC $11`, then X counts down from
/// `$0100` to 0, returning with X = 0, Z set and N clear; about 7 scanlines) in a tight loop with
/// the NMI on, so the NMI almost always lands inside it. The NMI handler preserves A and X, calls
/// `handler_calls` (the long routine, or the short one at [`NMI_CALL_SHORT`]: `INC $10; RTS`),
/// counts itself in [`NMI_CALL_NMI_COUNTER`] and acknowledges the NMI.
///
/// Native mode, 8-bit A, 16-bit X/Y, S = [`MAIN_STACK`] in the loop.
pub fn nmi_call_rom(handler_calls: u32) -> Vec<u8> {
    let mut rom = vec![0xFF_u8; 0x8000];
    #[rustfmt::skip] // keep the one-instruction-per-line layout with address comments
    let main: &[u8] = &[
        0x78,             // 8000 SEI
        0x18,             // 8001 CLC
        0xFB,             // 8002 XCE           -> native mode
        0xC2, 0x10,       // 8003 REP #$10      -> 16-bit X, Y
        0xE2, 0x20,       // 8005 SEP #$20      -> 8-bit A
        0xA2, 0xF0, 0x1F, // 8007 LDX #$1FF0
        0x9A,             // 800A TXS
        0x64, 0x10,       // 800B STZ $10
        0x64, 0x11,       // 800D STZ $11
        0x64, 0x12,       // 800F STZ $12
        0xA9, 0x80,       // 8011 LDA #$80
        0x8D, 0x00, 0x42, // 8013 STA $4200     NMI on
        0x20, 0x40, 0x80, // 8016 JSR $8040     loop: the long routine
        0x80, 0xFB,       // 8019 BRA $8016     (0x16 - 0x1B = -5)
        0x40,             // 801B RTI           target of every unused vector
    ];
    let [target_lo, target_hi] = (handler_calls as u16).to_le_bytes();
    #[rustfmt::skip]
    let nmi_handler: &[u8] = &[
        0x48,                         // 8028 PHA           NMI handler
        0xDA,                         // 8029 PHX
        0x20, target_lo, target_hi,   // 802A JSR target
        0xE6, 0x12,                   // 802D INC $12       count the NMI
        0xAD, 0x10, 0x42,             // 802F LDA $4210     acknowledge
        0xFA,                         // 8032 PLX
        0x68,                         // 8033 PLA
        0x40,                         // 8034 RTI
    ];
    #[rustfmt::skip]
    let routines: &[(usize, &[u8])] = &[
        (0x40, &[
            0xE6, 0x11,             // 8040 INC $11       long
            0xA2, 0x00, 0x01,       // 8042 LDX #$0100    256 delay iterations
            0xCA,                   // 8045 DEX           delay:
            0xD0, 0xFD,             // 8046 BNE $8045     (0x45 - 0x48 = -3)
            0x60,                   // 8048 RTS
        ]),
        (0x50, &[
            0xE6, 0x10,             // 8050 INC $10       short
            0x60,                   // 8052 RTS
        ]),
    ];
    rom[..main.len()].copy_from_slice(main);
    rom[0x28..0x28 + nmi_handler.len()].copy_from_slice(nmi_handler);
    for &(offset, code) in routines {
        rom[offset..offset + code.len()].copy_from_slice(code);
    }
    write_header(&mut rom, b"SSD NMI CALL TEST    ", 0x00, 0x00, 0x801B);
    set_native_nmi_vector(&mut rom, 0x8028);
    rom
}

/// The routine [`nested_no_return_rom`]'s main loop calls: `JSR` [`NESTED_SPIN`]`; RTS`.
pub const NESTED_SPIN_CALLER: u32 = LEAF;
/// The routine of [`nested_no_return_rom`] that never returns: `BRA` to itself.
pub const NESTED_SPIN: u32 = 0x00_8050;

/// [`jsr_leaf_rom`]'s loop calling a routine at [`NESTED_SPIN_CALLER`] (`JSR $8050; RTS`) whose
/// callee at [`NESTED_SPIN`] never returns (`BRA $8050`).
pub fn nested_no_return_rom() -> Vec<u8> {
    let mut code = [0xFF_u8; 0x12];
    code[..4].copy_from_slice(&[0x20, 0x50, 0x80, 0x60]); // 8040 JSR $8050; 8043 RTS
    code[0x10..].copy_from_slice(&[0x80, 0xFE]); // 8050 BRA $8050
    jsr_loop_rom(&code, b"SSD NESTED NO RETURN ")
}

/// The first routine of the chain of [`chain_rom`].
pub const CHAIN: u32 = 0x00_8100;
/// Routines in the chain of [`chain_rom`].
pub const CHAIN_LEN: usize = 18;
/// WRAM index of the counter the last routine of [`chain_rom`] increments (`$00:0010`).
pub const CHAIN_COUNTER: usize = 0x10;

/// A chain of [`CHAIN_LEN`] routines, each calling the next: routine `i` at [`CHAIN`] + 4i is
/// `JSR next; RTS`, the last one `INC $10; RTS`. Every routine returns with N/Z from the
/// increment. The main loop calls the first routine, then waits 1024 iterations of `DEX; BNE`
/// (about a tenth of a frame).
///
/// Native mode, 8-bit A, 16-bit X/Y, S = [`MAIN_STACK`] in the loop; NMI is disabled.
pub fn chain_rom() -> Vec<u8> {
    let mut rom = vec![0xFF_u8; 0x8000];
    #[rustfmt::skip] // keep the one-instruction-per-line layout with address comments
    let main: &[u8] = &[
        0x78,             // 8000 SEI
        0x18,             // 8001 CLC
        0xFB,             // 8002 XCE           -> native mode
        0xC2, 0x10,       // 8003 REP #$10      -> 16-bit X, Y
        0xE2, 0x20,       // 8005 SEP #$20      -> 8-bit A
        0xA2, 0xF0, 0x1F, // 8007 LDX #$1FF0
        0x9A,             // 800A TXS
        0x20, 0x00, 0x81, // 800B JSR $8100     loop: the chain
        0xA2, 0x00, 0x04, // 800E LDX #$0400    1024 delay iterations
        0xCA,             // 8011 DEX           delay:
        0xD0, 0xFD,       // 8012 BNE $8011     (0x11 - 0x14 = -3)
        0x80, 0xF5,       // 8014 BRA $800B     (0x0B - 0x16 = -11)
        0x40,             // 8016 RTI           target of every unused vector
    ];
    rom[..main.len()].copy_from_slice(main);
    let chain_offset = (CHAIN & 0x7FFF) as usize;
    for i in 0..CHAIN_LEN {
        let offset = chain_offset + 4 * i;
        if i + 1 < CHAIN_LEN {
            let [low, high] = ((CHAIN as usize + 4 * (i + 1)) as u16).to_le_bytes();
            rom[offset..offset + 4].copy_from_slice(&[0x20, low, high, 0x60]); // JSR next; RTS
        } else {
            rom[offset..offset + 3].copy_from_slice(&[0xE6, 0x10, 0x60]); // INC $10; RTS
        }
    }
    write_header(&mut rom, b"SSD VERIFY CHAIN     ", 0x00, 0x00, 0x8016);
    rom
}

/// The long routine [`joypad_log_rom`]'s main loop calls, as [`NMI_CALL_LONG`] of
/// [`nmi_call_rom`] (the same bytes at the same address).
pub const JOYPAD_LOG_LONG: u32 = 0x00_8040;
/// The short routine [`joypad_log_rom`]'s NMI handler calls, as [`NMI_CALL_SHORT`].
pub const JOYPAD_LOG_SHORT: u32 = 0x00_8050;
/// WRAM index of the count of NMIs the handler of [`joypad_log_rom`] has logged (`$00:0012`).
pub const JOYPAD_LOG_COUNT: usize = 0x12;
/// WRAM index of the log of `JOY1L` (`$4218`) of [`joypad_log_rom`], one byte per NMI.
pub const JOYPAD_LOG_LOW: usize = 0x100;
/// WRAM index of the log of `JOY1H` (`$4219`) of [`joypad_log_rom`], one byte per NMI.
pub const JOYPAD_LOG_HIGH: usize = 0x200;

/// [`nmi_call_rom`]`(NMI_CALL_SHORT)` with the auto-joypad read on, whose NMI handler logs the
/// joypad: it calls the short routine, waits for the auto-joypad read to start and to end
/// (`$4212` bit 0), then copies `JOY1L` and `JOY1H` to [`JOYPAD_LOG_LOW`] and
/// [`JOYPAD_LOG_HIGH`] at the index [`JOYPAD_LOG_COUNT`] (256 entries), and counts itself there.
/// The NMI almost always lands inside the long routine, which is then still running (interrupted)
/// when the read latches the joypad, some 4 300 master cycles after the frame is rendered.
///
/// Native mode, 8-bit A, 16-bit X/Y, S = [`MAIN_STACK`] in the loop.
pub fn joypad_log_rom() -> Vec<u8> {
    let mut rom = vec![0xFF_u8; 0x8000];
    #[rustfmt::skip] // keep the one-instruction-per-line layout with address comments
    let main: &[u8] = &[
        0x78,             // 8000 SEI
        0x18,             // 8001 CLC
        0xFB,             // 8002 XCE           -> native mode
        0xC2, 0x10,       // 8003 REP #$10      -> 16-bit X, Y
        0xE2, 0x20,       // 8005 SEP #$20      -> 8-bit A
        0xA2, 0xF0, 0x1F, // 8007 LDX #$1FF0
        0x9A,             // 800A TXS
        0x64, 0x10,       // 800B STZ $10
        0x64, 0x11,       // 800D STZ $11
        0x64, 0x12,       // 800F STZ $12       log index
        0x64, 0x13,       // 8011 STZ $13       its high byte (read by LDX $12)
        0xA9, 0x81,       // 8013 LDA #$81
        0x8D, 0x00, 0x42, // 8015 STA $4200     NMI and auto-joypad read on
        0x20, 0x40, 0x80, // 8018 JSR $8040     loop: the long routine
        0x80, 0xFB,       // 801B BRA $8018     (0x18 - 0x1D = -5)
        0x40,             // 801D RTI           target of every unused vector
    ];
    #[rustfmt::skip]
    let routines: &[(usize, &[u8])] = &[
        (0x40, &[
            0xE6, 0x11,             // 8040 INC $11       long
            0xA2, 0x00, 0x01,       // 8042 LDX #$0100    256 delay iterations
            0xCA,                   // 8045 DEX           delay:
            0xD0, 0xFD,             // 8046 BNE $8045     (0x45 - 0x48 = -3)
            0x60,                   // 8048 RTS
        ]),
        (0x50, &[
            0xE6, 0x10,             // 8050 INC $10       short
            0x60,                   // 8052 RTS
        ]),
        (0x60, &[
            0x48,                   // 8060 PHA           NMI handler
            0xDA,                   // 8061 PHX
            0x20, 0x50, 0x80,       // 8062 JSR $8050     the short routine
            0xAD, 0x12, 0x42,       // 8065 LDA $4212     until the auto-joypad read runs
            0x4A,                   // 8068 LSR A         bit 0 into C
            0x90, 0xFA,             // 8069 BCC $8065     (0x65 - 0x6B = -6)
            0xAD, 0x12, 0x42,       // 806B LDA $4212     until it is over
            0x4A,                   // 806E LSR A
            0xB0, 0xFA,             // 806F BCS $806B     (0x6B - 0x71 = -6)
            0xA6, 0x12,             // 8071 LDX $12       the log index
            0xAD, 0x18, 0x42,       // 8073 LDA $4218     JOY1L
            0x9D, 0x00, 0x01,       // 8076 STA $0100,X
            0xAD, 0x19, 0x42,       // 8079 LDA $4219     JOY1H
            0x9D, 0x00, 0x02,       // 807C STA $0200,X
            0xE6, 0x12,             // 807F INC $12       count the NMI
            0xAD, 0x10, 0x42,       // 8081 LDA $4210     acknowledge
            0xFA,                   // 8084 PLX
            0x68,                   // 8085 PLA
            0x40,                   // 8086 RTI
        ]),
    ];
    rom[..main.len()].copy_from_slice(main);
    for &(offset, code) in routines {
        rom[offset..offset + code.len()].copy_from_slice(code);
    }
    write_header(&mut rom, b"SSD JOYPAD LOG TEST  ", 0x00, 0x00, 0x801D);
    set_native_nmi_vector(&mut rom, 0x8060);
    rom
}

/// The main loop of [`debugger_rom`] (`JSR outer`).
pub const DBG_LOOP: u32 = 0x00_8013;
/// The NMI handler of [`debugger_rom`]: `PHA; INC $12; LDA $4210; PLA; RTI`.
pub const DBG_NMI: u32 = 0x00_8028;
/// `outer` of [`debugger_rom`]: `PHX; JSL inner_long; PLX; RTS`.
pub const DBG_OUTER: u32 = 0x00_8040;
/// `inner_long` of [`debugger_rom`]: `JSR innermost; INC $13; RTL`.
pub const DBG_INNER_LONG: u32 = 0x00_8050;
/// `innermost` of [`debugger_rom`]: `INC $14; RTS`.
pub const DBG_INNERMOST: u32 = 0x00_8058;
/// The leaf of [`debugger_rom`] hooked in `Rust`: `INC $10; RTS`.
pub const DBG_RUST_LEAF: u32 = 0x00_8060;
/// The leaf of [`debugger_rom`] hooked in `Verify`: `INC $11; RTS`, called through
/// `JSR ($8090,X)`.
pub const DBG_VERIFY_LEAF: u32 = 0x00_8070;
/// `wait` of [`debugger_rom`]: `WAI; RTS`.
pub const DBG_WAIT: u32 = 0x00_8080;
/// WRAM indexes of the counters of [`debugger_rom`]: the Rust leaf (`$10`), the Verify leaf
/// (`$11`), the NMI handler (`$12`), `inner_long` (`$13`) and `innermost` (`$14`).
pub const DBG_RUST_COUNTER: usize = 0x10;
pub const DBG_VERIFY_COUNTER: usize = 0x11;
pub const DBG_NMI_COUNTER: usize = 0x12;
pub const DBG_INNER_LONG_COUNTER: usize = 0x13;
pub const DBG_INNERMOST_COUNTER: usize = 0x14;

/// Code to step through: the main loop at [`DBG_LOOP`] calls, once per frame,
///
/// - `outer` ([`DBG_OUTER`]): `PHX; JSL inner_long; PLX; RTS`, with `inner_long`
///   ([`DBG_INNER_LONG`]) = `JSR innermost; INC $13; RTL` and `innermost` ([`DBG_INNERMOST`]) =
///   `INC $14; RTS`;
/// - a leaf to hook in `Rust` ([`DBG_RUST_LEAF`], `INC $10; RTS`);
/// - a leaf to hook in `Verify` ([`DBG_VERIFY_LEAF`], `INC $11; RTS`) through `JSR ($8090,X)`
///   (X = 0);
/// - `wait` ([`DBG_WAIT`]): `WAI; RTS`, which waits for the NMI.
///
/// The NMI handler ([`DBG_NMI`]) preserves A, counts itself in `$12` and acknowledges the NMI.
/// The counters start as the power-on WRAM fill left them.
///
/// Native mode, 8-bit A, 16-bit X/Y, S = [`MAIN_STACK`] in the loop, X = 0; NMI on.
pub fn debugger_rom() -> Vec<u8> {
    let mut rom = vec![0xFF_u8; 0x8000];
    #[rustfmt::skip] // keep the one-instruction-per-line layout with address comments
    let main: &[u8] = &[
        0x78,             // 8000 SEI
        0x18,             // 8001 CLC
        0xFB,             // 8002 XCE           -> native mode
        0xC2, 0x10,       // 8003 REP #$10      -> 16-bit X, Y
        0xE2, 0x20,       // 8005 SEP #$20      -> 8-bit A
        0xA2, 0xF0, 0x1F, // 8007 LDX #$1FF0
        0x9A,             // 800A TXS
        0xA2, 0x00, 0x00, // 800B LDX #$0000
        0xA9, 0x80,       // 800E LDA #$80
        0x8D, 0x00, 0x42, // 8010 STA $4200     NMI on
        0x20, 0x40, 0x80, // 8013 JSR $8040     loop: outer
        0x20, 0x60, 0x80, // 8016 JSR $8060     the Rust leaf
        0xFC, 0x90, 0x80, // 8019 JSR ($8090,X) the Verify leaf
        0x20, 0x80, 0x80, // 801C JSR $8080     wait
        0x80, 0xF2,       // 801F BRA $8013     (0x13 - 0x21 = -14)
        0x40,             // 8021 RTI           target of every unused vector
    ];
    #[rustfmt::skip]
    let routines: &[(usize, &[u8])] = &[
        (0x28, &[
            0x48,                   // 8028 PHA           NMI handler
            0xE6, 0x12,             // 8029 INC $12
            0xAD, 0x10, 0x42,       // 802B LDA $4210     acknowledge
            0x68,                   // 802E PLA
            0x40,                   // 802F RTI
        ]),
        (0x40, &[
            0xDA,                   // 8040 PHX           outer
            0x22, 0x50, 0x80, 0x00, // 8041 JSL $008050
            0xFA,                   // 8045 PLX
            0x60,                   // 8046 RTS
        ]),
        (0x50, &[
            0x20, 0x58, 0x80,       // 8050 JSR $8058     inner_long
            0xE6, 0x13,             // 8053 INC $13
            0x6B,                   // 8055 RTL
        ]),
        (0x58, &[
            0xE6, 0x14,             // 8058 INC $14       innermost
            0x60,                   // 805A RTS
        ]),
        (0x60, &[
            0xE6, 0x10,             // 8060 INC $10       the Rust leaf
            0x60,                   // 8062 RTS
        ]),
        (0x70, &[
            0xE6, 0x11,             // 8070 INC $11       the Verify leaf
            0x60,                   // 8072 RTS
        ]),
        (0x80, &[
            0xCB,                   // 8080 WAI           wait
            0x60,                   // 8081 RTS
        ]),
        (0x90, &[
            0x70, 0x80,             // 8090 .dw $8070     the table of JSR ($8090,X)
        ]),
    ];
    rom[..main.len()].copy_from_slice(main);
    for &(offset, code) in routines {
        rom[offset..offset + code.len()].copy_from_slice(code);
    }
    write_header(&mut rom, b"SSD DEBUGGER TEST    ", 0x00, 0x00, 0x8021);
    set_native_nmi_vector(&mut rom, 0x8028);
    rom
}

/// Increments SRAM byte 0 (`$70:0000`) once per frame, at the start of every vblank.
///
/// Cartridge with 2 KiB of battery-backed SRAM; NMI is disabled, so the loop polls the vblank flag
/// in `$4210` (bit 7, cleared by the read).
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
        0x10, 0xFB,             // 800D BPL $800A     wait for bit 7 (0x0A - 0x0F = -5)
        0xAF, 0x00, 0x00, 0x70, // 800F LDA $700000   SRAM byte 0
        0x1A,                   // 8013 INC A
        0x8F, 0x00, 0x00, 0x70, // 8014 STA $700000
        0x80, 0xF0,             // 8018 BRA $800A     (0x0A - 0x1A = -16)
        0x40,                   // 801A RTI           target of every unused vector
    ];
    rom[..code.len()].copy_from_slice(code);
    // Cartridge type $02 = ROM + RAM + battery; SRAM size code $01 = 1 << (10 + 1) = 2 KiB.
    write_header(&mut rom, b"SSD SRAM TEST        ", 0x02, 0x01, 0x801A);
    rom
}

/// `LoROM` internal header at 0x7FC0 plus vectors: every vector points at `rti`, emulation RESET at
/// $8000. Fixes the checksum last.
fn write_header(rom: &mut [u8], title: &[u8; 21], cart_type: u8, sram_size: u8, rti: u16) {
    rom[0x7FC0..0x7FD5].copy_from_slice(title);
    rom[0x7FD5] = 0x20; // LoROM, SlowROM
    rom[0x7FD6] = cart_type;
    rom[0x7FD7] = 0x05; // 32 KiB
    rom[0x7FD8] = sram_size;
    rom[0x7FD9] = 0x01; // North America -> NTSC
    rom[0x7FDA] = 0x00;
    rom[0x7FDB] = 0x00;

    for v in (0x7FE4..0x7FF0).step_by(2).chain((0x7FF4..0x8000).step_by(2)) {
        rom[v..v + 2].copy_from_slice(&rti.to_le_bytes());
    }
    rom[0x7FFC..0x7FFE].copy_from_slice(&0x8000_u16.to_le_bytes());

    ssd_rom::fix_lorom_checksum(rom);
}

/// Points the native-mode NMI vector at `handler` and fixes the checksum again.
fn set_native_nmi_vector(rom: &mut [u8], handler: u16) {
    rom[0x7FEA..0x7FEC].copy_from_slice(&handler.to_le_bytes());
    ssd_rom::fix_lorom_checksum(rom);
}

/// In-memory save store whose clones share one map, so a test can inspect what a machine persisted.
#[derive(Clone, Default)]
pub struct SharedStore(Arc<Mutex<HashMap<String, Vec<u8>>>>);

impl SharedStore {
    pub fn get(&self, ext: &str) -> Option<Vec<u8>> {
        self.0.lock().unwrap().get(ext).cloned()
    }
}

impl SaveStore for SharedStore {
    fn load(&mut self, ext: &str) -> std::io::Result<Option<Vec<u8>>> {
        Ok(self.get(ext))
    }
    fn persist(&mut self, ext: &str, bytes: &[u8]) -> std::io::Result<()> {
        self.0.lock().unwrap().insert(ext.to_owned(), bytes.to_vec());
        Ok(())
    }
}

// A count-driven leaf, for the cost bound and the Verify budget

/// The leaf of [`count_rom`] whose cost follows a count (`LDX $10; DEX; BNE; RTS`).
pub const COUNT_LEAF: u32 = 0x00_8040;
/// The leaf of [`count_rom`] that leaves D and DBR changed.
pub const DB_LEAF: u32 = 0x00_8090;
/// The count of [`count_rom`] (`$00:0010`, a word).
pub const COUNT: usize = 0x10;
/// The NMI counter of [`count_rom`] (`$00:0012`).
pub const COUNT_NMIS: usize = 0x12;

/// CPU-busy master cycles of the leaf of [`count_rom`] for count `n` (0 loops 65 536 times):
/// 16-bit `LDX dp` 32, `n` times `DEX` 14 and `BNE` 22 taken or 16 not taken, `RTS` 42.
pub fn count_cost(n: u16) -> u64 {
    let n = if n == 0 { 65_536 } else { u64::from(n) };
    36 * n + 68
}

/// CPU-busy master cycles of the leaf at [`DB_LEAF`]: 16-bit `LDA #` 24, `TCD` 14, `PEA` 40, two
/// `PLB` 56, `RTS` 42.
pub const DB_LEAF_COST: u64 = 176;

/// The count leaf in Rust: X ends 0 (N clear, Z set); declares [`count_cost`].
pub fn count_leaf(ctx: &mut Ctx<'_>) {
    let n = ctx.read_dp16(COUNT as u16);
    ctx.set_x(0);
    ctx.set_nz16(0);
    ctx.set_cost(count_cost(n));
}

/// The exact bound of [`count_leaf`], from the count at entry.
pub fn count_bound(ctx: &Ctx<'_>) -> u64 {
    count_cost(ctx.peek_dp16(COUNT as u16))
}

/// The leaf at [`DB_LEAF`] in Rust: A = `$0100`, D = `$0100`, DBR = `$7E`, N and Z from the
/// last pull.
pub fn db_leaf(ctx: &mut Ctx<'_>) {
    ctx.set_a(0x0100);
    ctx.set_d(0x0100);
    ctx.set_dbr(0x7E);
    ctx.set_nz8(0x7E);
    ctx.set_cost(DB_LEAF_COST);
}

/// A loop that calls two leaves, native mode with 16-bit A, X and Y. Each round it sets the count
/// at [`COUNT`] to `((count + step) & mask) | 1`, calls the count leaf at [`COUNT_LEAF`] (a
/// delay loop of `count` rounds of `DEX; BNE`, so the cost follows the count: see
/// [`count_cost`]), then the leaf at [`DB_LEAF`] (D = `$0100` and DBR = `$7E` on return, as a
/// `TCD` and two `PLB` leave them), then puts D back to 0 itself. The count starts at `init`.
/// With `nmi`, the vertical-blank NMI is on and its handler counts in [`COUNT_NMIS`].
pub fn count_rom(nmi: bool, init: u16, step: u16, mask: u16) -> Vec<u8> {
    let mut rom = vec![0xFF_u8; 0x8000];
    let [init_lo, init_hi] = init.to_le_bytes();
    let [step_lo, step_hi] = step.to_le_bytes();
    let [mask_lo, mask_hi] = mask.to_le_bytes();
    #[rustfmt::skip] // keep the one-instruction-per-line layout with address comments
    let main: &[u8] = &[
        0x78,                         // 8000 SEI
        0x18,                         // 8001 CLC
        0xFB,                         // 8002 XCE           -> native mode
        0xC2, 0x30,                   // 8003 REP #$30      -> 16-bit A, X, Y
        0xA2, 0xF0, 0x1F,             // 8005 LDX #$1FF0
        0x9A,                         // 8008 TXS
        0xA9, init_lo, init_hi,       // 8009 LDA #init
        0x85, 0x10,                   // 800C STA $10       the count
        0xE2, 0x20,                   // 800E SEP #$20
        0xA9, if nmi { 0x80 } else { 0x00 }, // 8010 LDA #nmi
        0x8D, 0x00, 0x42,             // 8012 STA $4200     NMI on or off
        0xC2, 0x20,                   // 8015 REP #$20
        0xA5, 0x10,                   // 8017 LDA $10       loop:
        0x18,                         // 8019 CLC
        0x69, step_lo, step_hi,       // 801A ADC #step
        0x29, mask_lo, mask_hi,       // 801D AND #mask
        0x09, 0x01, 0x00,             // 8020 ORA #$0001    never 0
        0x85, 0x10,                   // 8023 STA $10
        0x20, 0x40, 0x80,             // 8025 JSR $8040     the count leaf
        0x20, 0x90, 0x80,             // 8028 JSR $8090     the D and DBR leaf
        0xF4, 0x00, 0x00,             // 802B PEA $0000
        0x2B,                         // 802E PLD           D back to 0
        0x80, 0xE6,                   // 802F BRA $8017     (0x17 - 0x31 = -26)
        0x40,                         // 8031 RTI           target of every unused vector
    ];
    #[rustfmt::skip]
    let count_leaf: &[u8] = &[
        0xA6, 0x10,                   // 8040 LDX $10       the count
        0xCA,                         // 8042 DEX
        0xD0, 0xFD,                   // 8043 BNE $8042
        0x60,                         // 8045 RTS
    ];
    #[rustfmt::skip]
    let nmi_handler: &[u8] = &[
        0xE2, 0x20,                   // 8060 SEP #$20
        0x48,                         // 8062 PHA
        0xAF, 0x10, 0x42, 0x00,       // 8063 LDA $004210   acknowledge
        0xE6, 0x12,                   // 8067 INC $12       count the NMI
        0x68,                         // 8069 PLA
        0x40,                         // 806A RTI
    ];
    #[rustfmt::skip]
    let db_leaf: &[u8] = &[
        0xA9, 0x00, 0x01,             // 8090 LDA #$0100
        0x5B,                         // 8093 TCD           D = $0100
        0xF4, 0x7E, 0x7E,             // 8094 PEA $7E7E
        0xAB,                         // 8097 PLB           DBR = $7E
        0xAB,                         // 8098 PLB
        0x60,                         // 8099 RTS
    ];
    rom[..main.len()].copy_from_slice(main);
    rom[0x40..0x40 + count_leaf.len()].copy_from_slice(count_leaf);
    rom[0x60..0x60 + nmi_handler.len()].copy_from_slice(nmi_handler);
    rom[0x90..0x90 + db_leaf.len()].copy_from_slice(db_leaf);
    write_header(&mut rom, b"SSD COUNT LEAF TEST  ", 0x00, 0x00, 0x8031);
    set_native_nmi_vector(&mut rom, 0x8060);
    rom
}
