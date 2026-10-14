//! The 65816 instruction set: opcode table and decoding over the ROM image.

/// Addressing modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Imp,
    Acc,
    /// Immediate, as wide as the accumulator.
    ImmM,
    /// Immediate, as wide as the index registers.
    ImmX,
    /// Immediate byte.
    Imm8,
    Dp,
    DpX,
    DpY,
    DpInd,
    DpIndX,
    DpIndY,
    DpIndLong,
    DpIndLongY,
    Abs,
    AbsX,
    AbsY,
    Long,
    LongX,
    AbsInd,
    AbsIndX,
    AbsIndLong,
    Rel8,
    Rel16,
    Stack,
    StackIndY,
    Block,
}

use Mode::{
    Abs, AbsInd, AbsIndLong, AbsIndX, AbsX, AbsY, Acc, Block, Dp, DpInd, DpIndLong, DpIndLongY,
    DpIndX, DpIndY, DpX, DpY, Imm8, ImmM, ImmX, Imp, Long, LongX, Rel8, Rel16, Stack, StackIndY,
};

#[rustfmt::skip]
pub const OPCODES: [(&str, Mode); 256] = [
    ("BRK", Imm8), ("ORA", DpIndX), ("COP", Imm8), ("ORA", Stack), ("TSB", Dp), ("ORA", Dp), ("ASL", Dp), ("ORA", DpIndLong),
    ("PHP", Imp), ("ORA", ImmM), ("ASL", Acc), ("PHD", Imp), ("TSB", Abs), ("ORA", Abs), ("ASL", Abs), ("ORA", Long),
    ("BPL", Rel8), ("ORA", DpIndY), ("ORA", DpInd), ("ORA", StackIndY), ("TRB", Dp), ("ORA", DpX), ("ASL", DpX), ("ORA", DpIndLongY),
    ("CLC", Imp), ("ORA", AbsY), ("INC", Acc), ("TCS", Imp), ("TRB", Abs), ("ORA", AbsX), ("ASL", AbsX), ("ORA", LongX),
    ("JSR", Abs), ("AND", DpIndX), ("JSL", Long), ("AND", Stack), ("BIT", Dp), ("AND", Dp), ("ROL", Dp), ("AND", DpIndLong),
    ("PLP", Imp), ("AND", ImmM), ("ROL", Acc), ("PLD", Imp), ("BIT", Abs), ("AND", Abs), ("ROL", Abs), ("AND", Long),
    ("BMI", Rel8), ("AND", DpIndY), ("AND", DpInd), ("AND", StackIndY), ("BIT", DpX), ("AND", DpX), ("ROL", DpX), ("AND", DpIndLongY),
    ("SEC", Imp), ("AND", AbsY), ("DEC", Acc), ("TSC", Imp), ("BIT", AbsX), ("AND", AbsX), ("ROL", AbsX), ("AND", LongX),
    ("RTI", Imp), ("EOR", DpIndX), ("WDM", Imm8), ("EOR", Stack), ("MVP", Block), ("EOR", Dp), ("LSR", Dp), ("EOR", DpIndLong),
    ("PHA", Imp), ("EOR", ImmM), ("LSR", Acc), ("PHK", Imp), ("JMP", Abs), ("EOR", Abs), ("LSR", Abs), ("EOR", Long),
    ("BVC", Rel8), ("EOR", DpIndY), ("EOR", DpInd), ("EOR", StackIndY), ("MVN", Block), ("EOR", DpX), ("LSR", DpX), ("EOR", DpIndLongY),
    ("CLI", Imp), ("EOR", AbsY), ("PHY", Imp), ("TCD", Imp), ("JML", Long), ("EOR", AbsX), ("LSR", AbsX), ("EOR", LongX),
    ("RTS", Imp), ("ADC", DpIndX), ("PER", Rel16), ("ADC", Stack), ("STZ", Dp), ("ADC", Dp), ("ROR", Dp), ("ADC", DpIndLong),
    ("PLA", Imp), ("ADC", ImmM), ("ROR", Acc), ("RTL", Imp), ("JMP", AbsInd), ("ADC", Abs), ("ROR", Abs), ("ADC", Long),
    ("BVS", Rel8), ("ADC", DpIndY), ("ADC", DpInd), ("ADC", StackIndY), ("STZ", DpX), ("ADC", DpX), ("ROR", DpX), ("ADC", DpIndLongY),
    ("SEI", Imp), ("ADC", AbsY), ("PLY", Imp), ("TDC", Imp), ("JMP", AbsIndX), ("ADC", AbsX), ("ROR", AbsX), ("ADC", LongX),
    ("BRA", Rel8), ("STA", DpIndX), ("BRL", Rel16), ("STA", Stack), ("STY", Dp), ("STA", Dp), ("STX", Dp), ("STA", DpIndLong),
    ("DEY", Imp), ("BIT", ImmM), ("TXA", Imp), ("PHB", Imp), ("STY", Abs), ("STA", Abs), ("STX", Abs), ("STA", Long),
    ("BCC", Rel8), ("STA", DpIndY), ("STA", DpInd), ("STA", StackIndY), ("STY", DpX), ("STA", DpX), ("STX", DpY), ("STA", DpIndLongY),
    ("TYA", Imp), ("STA", AbsY), ("TXS", Imp), ("TXY", Imp), ("STZ", Abs), ("STA", AbsX), ("STZ", AbsX), ("STA", LongX),
    ("LDY", ImmX), ("LDA", DpIndX), ("LDX", ImmX), ("LDA", Stack), ("LDY", Dp), ("LDA", Dp), ("LDX", Dp), ("LDA", DpIndLong),
    ("TAY", Imp), ("LDA", ImmM), ("TAX", Imp), ("PLB", Imp), ("LDY", Abs), ("LDA", Abs), ("LDX", Abs), ("LDA", Long),
    ("BCS", Rel8), ("LDA", DpIndY), ("LDA", DpInd), ("LDA", StackIndY), ("LDY", DpX), ("LDA", DpX), ("LDX", DpY), ("LDA", DpIndLongY),
    ("CLV", Imp), ("LDA", AbsY), ("TSX", Imp), ("TYX", Imp), ("LDY", AbsX), ("LDA", AbsX), ("LDX", AbsY), ("LDA", LongX),
    ("CPY", ImmX), ("CMP", DpIndX), ("REP", Imm8), ("CMP", Stack), ("CPY", Dp), ("CMP", Dp), ("DEC", Dp), ("CMP", DpIndLong),
    ("INY", Imp), ("CMP", ImmM), ("DEX", Imp), ("WAI", Imp), ("CPY", Abs), ("CMP", Abs), ("DEC", Abs), ("CMP", Long),
    ("BNE", Rel8), ("CMP", DpIndY), ("CMP", DpInd), ("CMP", StackIndY), ("PEI", DpInd), ("CMP", DpX), ("DEC", DpX), ("CMP", DpIndLongY),
    ("CLD", Imp), ("CMP", AbsY), ("PHX", Imp), ("STP", Imp), ("JML", AbsIndLong), ("CMP", AbsX), ("DEC", AbsX), ("CMP", LongX),
    ("CPX", ImmX), ("SBC", DpIndX), ("SEP", Imm8), ("SBC", Stack), ("CPX", Dp), ("SBC", Dp), ("INC", Dp), ("SBC", DpIndLong),
    ("INX", Imp), ("SBC", ImmM), ("NOP", Imp), ("XBA", Imp), ("CPX", Abs), ("SBC", Abs), ("INC", Abs), ("SBC", Long),
    ("BEQ", Rel8), ("SBC", DpIndY), ("SBC", DpInd), ("SBC", StackIndY), ("PEA", Abs), ("SBC", DpX), ("INC", DpX), ("SBC", DpIndLongY),
    ("SED", Imp), ("SBC", AbsY), ("PLX", Imp), ("XCE", Imp), ("JSR", AbsIndX), ("SBC", AbsX), ("INC", AbsX), ("SBC", LongX),
];

/// One decoded instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Insn {
    /// 24-bit address of the opcode.
    pub pc: u32,
    pub op: u8,
    pub mn: &'static str,
    pub mode: Mode,
    /// Length in bytes, opcode included.
    pub len: u8,
    /// The raw operand (little-endian bytes after the opcode); for a block move
    /// `dest | src << 8`; branches keep the signed offset in `target`.
    pub opr: u32,
    /// The target of a relative branch, `BRA`/`BRL`/`PER`, or a `JMP`/`JSR`/`JML`/`JSL` to a
    /// fixed address (24-bit).
    pub target: Option<u32>,
    /// M and X widths this instruction was decoded with.
    pub m8: bool,
    pub x8: bool,
}

impl Insn {
    /// The address of the next instruction (same bank, wrapping at 64 KiB as the PC does).
    #[must_use]
    pub fn next(&self) -> u32 {
        (self.pc & 0xFF_0000) | (self.pc.wrapping_add(u32::from(self.len)) & 0xFFFF)
    }
}

/// The ROM image as the core maps it (`LoROM`), read through the CPU's addresses.
pub struct Rom<'a>(pub &'a [u8]);

impl Rom<'_> {
    /// The byte at a 24-bit CPU address if it is cartridge ROM (`$8000-$FFFF` of the banks but
    /// `$7E-$7F`, and all of `$40-$6F` and `$C0-$EF`); `None` elsewhere.
    #[must_use]
    pub fn read(&self, address: u32) -> Option<u8> {
        let (bank, offset) = ((address >> 16) as u8, address as u16);
        let mapped = match (bank, offset) {
            (0x7E | 0x7F, _) => false,
            (0x40..=0x6F | 0xC0..=0xEF, _) | (_, 0x8000..=0xFFFF) => true,
            _ => false,
        };
        if !mapped || self.0.is_empty() {
            return None;
        }
        let offset = ((address & 0x7F_0000) >> 1) | (address & 0x7FFF);
        self.0.get(offset as usize & (self.0.len().next_power_of_two() - 1)).copied()
    }
}

/// Decodes the instruction at `pc` with the widths `m8`, `x8`; `None` if its bytes are not
/// ROM.
#[must_use]
pub fn decode(rom: &Rom<'_>, pc: u32, m8: bool, x8: bool) -> Option<Insn> {
    let op = rom.read(pc)?;
    let (mn, mode) = OPCODES[usize::from(op)];
    let len: u8 = match mode {
        Imp | Acc => 1,
        ImmM => 1 + if m8 { 1 } else { 2 },
        ImmX => 1 + if x8 { 1 } else { 2 },
        Imm8 | Dp | DpX | DpY | DpInd | DpIndX | DpIndY | DpIndLong | DpIndLongY | Stack
        | StackIndY | Rel8 => 2,
        Abs | AbsX | AbsY | AbsInd | AbsIndX | AbsIndLong | Rel16 | Block => 3,
        Long | LongX => 4,
    };
    let mut opr = 0_u32;
    for i in 1..u32::from(len) {
        opr |= u32::from(rom.read((pc + i) & 0xFF_FFFF)?) << (8 * (i - 1));
    }
    let bank = pc & 0xFF_0000;
    let next = (pc.wrapping_add(u32::from(len))) & 0xFFFF;
    let target = match (mn, mode) {
        (_, Rel8) => Some(bank | (next.wrapping_add(i32::from(opr as u8 as i8) as u32) & 0xFFFF)),
        ("PER", Rel16) => None,
        (_, Rel16) => {
            Some(bank | (next.wrapping_add(i32::from(opr as u16 as i16) as u32) & 0xFFFF))
        }
        ("JSR" | "JMP", Abs) => Some(bank | (opr & 0xFFFF)),
        ("JSL" | "JML", Long) => Some(opr & 0xFF_FFFF),
        _ => None,
    };
    Some(Insn { pc, op, mn, mode, len, opr, target, m8, x8 })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-bank image whose code starts at `$00:8000`.
    fn image(code: &[u8]) -> Vec<u8> {
        let mut rom = vec![0xEA; 0x8000];
        rom[..code.len()].copy_from_slice(code);
        rom
    }

    #[test]
    fn immediate_operands_follow_the_widths() {
        let rom = image(&[0xA9, 0x34, 0x12, 0xA2, 0x78, 0x56]);
        let r = Rom(&rom);
        let lda8 = decode(&r, 0x8000, true, false).unwrap();
        assert_eq!((lda8.mn, lda8.len, lda8.opr), ("LDA", 2, 0x34));
        let lda16 = decode(&r, 0x8000, false, false).unwrap();
        assert_eq!((lda16.len, lda16.opr), (3, 0x1234));
        let ldx16 = decode(&r, 0x8003, true, false).unwrap();
        assert_eq!((ldx16.mn, ldx16.len, ldx16.opr), ("LDX", 3, 0x5678));
    }

    #[test]
    fn branches_and_calls_have_targets_in_the_bank() {
        let rom = image(&[0xD0, 0xFE, 0x20, 0x34, 0x92, 0x22, 0x56, 0x80, 0x03]);
        let r = Rom(&rom);
        assert_eq!(decode(&r, 0x8000, true, false).unwrap().target, Some(0x8000));
        assert_eq!(decode(&r, 0x8002, true, false).unwrap().target, Some(0x9234));
        assert_eq!(decode(&r, 0x8005, true, false).unwrap().target, Some(0x03_8056));
    }

    #[test]
    fn addresses_outside_the_rom_do_not_decode() {
        let rom = image(&[0xEA]);
        let r = Rom(&rom);
        assert!(decode(&r, 0x0000_1223, true, false).is_none(), "low RAM");
        assert!(decode(&r, 0x007E_8000, true, false).is_none(), "WRAM");
    }
}
