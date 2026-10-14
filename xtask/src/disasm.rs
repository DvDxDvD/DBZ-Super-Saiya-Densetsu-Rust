//! `disasm`: a 65816 disassembler over the cached ROM image (`LoROM`, banks `$00-$1F` and their
//! mirrors), for working without a debugger. It tracks the M and X widths across `REP`,
//! `SEP`, `PLP` and `XCE`, follows local branches (so it stops at the end of a routine, not at the
//! first `RTS`), and names routines, tables and variables from `re/symbols.toml`. The listing
//! goes to stdout and is never tracked (`CONTRIBUTING.md`). [`decode`] and [`Region`] are shared
//! with `routines` and `probe`.

use anyhow::{Context, bail};
use ssd_oracle::Symbols;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

/// The ROM image as the core maps it, for reading code without side effects.
pub struct RomImage<'a>(pub &'a [u8]);

impl RomImage<'_> {
    /// The byte at a 24-bit bus address of the `LoROM` map (`$8000-$FFFF` of the banks but
    /// `$7E-$7F`, all of `$40-$6F` and `$C0-$EF`); `None` elsewhere (WRAM, registers).
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

/// Addressing modes of the 65816.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Implied,
    Accumulator,
    /// Immediate, as wide as the accumulator.
    ImmM,
    /// Immediate, as wide as the index registers.
    ImmX,
    /// Immediate byte (`REP`, `SEP`, `BRK`, `COP`, `WDM`).
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
    Abs, AbsInd, AbsIndLong, AbsIndX, AbsX, AbsY, Accumulator as Acc, Block, Dp, DpInd, DpIndLong,
    DpIndLongY, DpIndX, DpIndY, DpX, DpY, Imm8, ImmM, ImmX, Implied as Imp, Long, LongX, Rel8,
    Rel16, Stack, StackIndY,
};

#[rustfmt::skip]
const OPCODES: [(&str, Mode); 256] = [
    // 00
    ("BRK", Imm8), ("ORA", DpIndX), ("COP", Imm8), ("ORA", Stack), ("TSB", Dp), ("ORA", Dp), ("ASL", Dp), ("ORA", DpIndLong),
    ("PHP", Imp), ("ORA", ImmM), ("ASL", Acc), ("PHD", Imp), ("TSB", Abs), ("ORA", Abs), ("ASL", Abs), ("ORA", Long),
    // 10
    ("BPL", Rel8), ("ORA", DpIndY), ("ORA", DpInd), ("ORA", StackIndY), ("TRB", Dp), ("ORA", DpX), ("ASL", DpX), ("ORA", DpIndLongY),
    ("CLC", Imp), ("ORA", AbsY), ("INC", Acc), ("TCS", Imp), ("TRB", Abs), ("ORA", AbsX), ("ASL", AbsX), ("ORA", LongX),
    // 20
    ("JSR", Abs), ("AND", DpIndX), ("JSL", Long), ("AND", Stack), ("BIT", Dp), ("AND", Dp), ("ROL", Dp), ("AND", DpIndLong),
    ("PLP", Imp), ("AND", ImmM), ("ROL", Acc), ("PLD", Imp), ("BIT", Abs), ("AND", Abs), ("ROL", Abs), ("AND", Long),
    // 30
    ("BMI", Rel8), ("AND", DpIndY), ("AND", DpInd), ("AND", StackIndY), ("BIT", DpX), ("AND", DpX), ("ROL", DpX), ("AND", DpIndLongY),
    ("SEC", Imp), ("AND", AbsY), ("DEC", Acc), ("TSC", Imp), ("BIT", AbsX), ("AND", AbsX), ("ROL", AbsX), ("AND", LongX),
    // 40
    ("RTI", Imp), ("EOR", DpIndX), ("WDM", Imm8), ("EOR", Stack), ("MVP", Block), ("EOR", Dp), ("LSR", Dp), ("EOR", DpIndLong),
    ("PHA", Imp), ("EOR", ImmM), ("LSR", Acc), ("PHK", Imp), ("JMP", Abs), ("EOR", Abs), ("LSR", Abs), ("EOR", Long),
    // 50
    ("BVC", Rel8), ("EOR", DpIndY), ("EOR", DpInd), ("EOR", StackIndY), ("MVN", Block), ("EOR", DpX), ("LSR", DpX), ("EOR", DpIndLongY),
    ("CLI", Imp), ("EOR", AbsY), ("PHY", Imp), ("TCD", Imp), ("JML", Long), ("EOR", AbsX), ("LSR", AbsX), ("EOR", LongX),
    // 60
    ("RTS", Imp), ("ADC", DpIndX), ("PER", Rel16), ("ADC", Stack), ("STZ", Dp), ("ADC", Dp), ("ROR", Dp), ("ADC", DpIndLong),
    ("PLA", Imp), ("ADC", ImmM), ("ROR", Acc), ("RTL", Imp), ("JMP", AbsInd), ("ADC", Abs), ("ROR", Abs), ("ADC", Long),
    // 70
    ("BVS", Rel8), ("ADC", DpIndY), ("ADC", DpInd), ("ADC", StackIndY), ("STZ", DpX), ("ADC", DpX), ("ROR", DpX), ("ADC", DpIndLongY),
    ("SEI", Imp), ("ADC", AbsY), ("PLY", Imp), ("TDC", Imp), ("JMP", AbsIndX), ("ADC", AbsX), ("ROR", AbsX), ("ADC", LongX),
    // 80
    ("BRA", Rel8), ("STA", DpIndX), ("BRL", Rel16), ("STA", Stack), ("STY", Dp), ("STA", Dp), ("STX", Dp), ("STA", DpIndLong),
    ("DEY", Imp), ("BIT", ImmM), ("TXA", Imp), ("PHB", Imp), ("STY", Abs), ("STA", Abs), ("STX", Abs), ("STA", Long),
    // 90
    ("BCC", Rel8), ("STA", DpIndY), ("STA", DpInd), ("STA", StackIndY), ("STY", DpX), ("STA", DpX), ("STX", DpY), ("STA", DpIndLongY),
    ("TYA", Imp), ("STA", AbsY), ("TXS", Imp), ("TXY", Imp), ("STZ", Abs), ("STA", AbsX), ("STZ", AbsX), ("STA", LongX),
    // A0
    ("LDY", ImmX), ("LDA", DpIndX), ("LDX", ImmX), ("LDA", Stack), ("LDY", Dp), ("LDA", Dp), ("LDX", Dp), ("LDA", DpIndLong),
    ("TAY", Imp), ("LDA", ImmM), ("TAX", Imp), ("PLB", Imp), ("LDY", Abs), ("LDA", Abs), ("LDX", Abs), ("LDA", Long),
    // B0
    ("BCS", Rel8), ("LDA", DpIndY), ("LDA", DpInd), ("LDA", StackIndY), ("LDY", DpX), ("LDA", DpX), ("LDX", DpY), ("LDA", DpIndLongY),
    ("CLV", Imp), ("LDA", AbsY), ("TSX", Imp), ("TYX", Imp), ("LDY", AbsX), ("LDA", AbsX), ("LDX", AbsY), ("LDA", LongX),
    // C0
    ("CPY", ImmX), ("CMP", DpIndX), ("REP", Imm8), ("CMP", Stack), ("CPY", Dp), ("CMP", Dp), ("DEC", Dp), ("CMP", DpIndLong),
    ("INY", Imp), ("CMP", ImmM), ("DEX", Imp), ("WAI", Imp), ("CPY", Abs), ("CMP", Abs), ("DEC", Abs), ("CMP", Long),
    // D0
    ("BNE", Rel8), ("CMP", DpIndY), ("CMP", DpInd), ("CMP", StackIndY), ("PEI", DpInd), ("CMP", DpX), ("DEC", DpX), ("CMP", DpIndLongY),
    ("CLD", Imp), ("CMP", AbsY), ("PHX", Imp), ("STP", Imp), ("JML", AbsIndLong), ("CMP", AbsX), ("DEC", AbsX), ("CMP", LongX),
    // E0
    ("CPX", ImmX), ("SBC", DpIndX), ("SEP", Imm8), ("SBC", Stack), ("CPX", Dp), ("SBC", Dp), ("INC", Dp), ("SBC", DpIndLong),
    ("INX", Imp), ("SBC", ImmM), ("NOP", Imp), ("XBA", Imp), ("CPX", Abs), ("SBC", Abs), ("INC", Abs), ("SBC", Long),
    // F0
    ("BEQ", Rel8), ("SBC", DpIndY), ("SBC", DpInd), ("SBC", StackIndY), ("PEA", Abs), ("SBC", DpX), ("INC", DpX), ("SBC", DpIndLongY),
    ("SED", Imp), ("SBC", AbsY), ("PLX", Imp), ("XCE", Imp), ("JSR", AbsIndX), ("SBC", AbsX), ("INC", AbsX), ("SBC", LongX),
];

/// M and X widths at a point of the code, each known (from the entry, a `REP`/`SEP` or the
/// trace) or assumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Widths {
    pub m8: bool,
    pub x8: bool,
    pub m_known: bool,
    pub x_known: bool,
}

impl Widths {
    /// Nothing known: the mode the game runs in (8-bit A, 16-bit index), assumed.
    pub const ASSUMED: Self = Self { m8: true, x8: false, m_known: false, x_known: false };

    /// The widths after both paths meet: unknown unless both agree.
    #[must_use]
    pub fn merge(self, other: Self) -> Self {
        let m_same = self.m_known && other.m_known && self.m8 == other.m8;
        let x_same = self.x_known && other.x_known && self.x8 == other.x8;
        Self { m_known: m_same, x_known: x_same, ..self }
    }

    fn after(mut self, instr: &Instr) -> Self {
        match instr.mnemonic {
            "REP" | "SEP" => {
                let set = instr.mnemonic == "SEP";
                if instr.operand & 0x20 != 0 {
                    (self.m8, self.m_known) = (set, true);
                }
                if instr.operand & 0x10 != 0 {
                    (self.x8, self.x_known) = (set, true);
                }
            }
            "PLP" | "XCE" | "RTI" => {
                self.m_known = false;
                self.x_known = false;
            }
            _ => {}
        }
        self
    }
}

/// One decoded instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instr {
    /// 24-bit address of the opcode (as read: banks keep their mirror bit).
    pub addr: u32,
    pub opcode: u8,
    pub mnemonic: &'static str,
    pub mode: Mode,
    /// Length in bytes, opcode included.
    pub len: u8,
    /// The raw operand: the immediate, direct-page offset, absolute address, long address, the
    /// stack offset, the two bank bytes of a block move (`src << 8 | dest`, written source first) or, for a branch, the
    /// target (same bank as the opcode).
    pub operand: u32,
}

impl Instr {
    /// The address after the instruction.
    #[must_use]
    pub fn next(&self) -> u32 {
        self.addr.wrapping_add(u32::from(self.len)) & 0xFF_FFFF
    }

    #[must_use]
    pub fn is_return(&self) -> bool {
        matches!(self.mnemonic, "RTS" | "RTL" | "RTI")
    }

    /// `JSR`, `JSL`, `JSR (abs,X)`.
    #[must_use]
    pub fn is_call(&self) -> bool {
        matches!(self.mnemonic, "JSR" | "JSL")
    }

    #[must_use]
    pub fn is_conditional_branch(&self) -> bool {
        self.mode == Rel8 && self.mnemonic != "BRA"
    }

    /// `BRA`, `BRL`, `JMP` and `JML` to a fixed address (the target is [`Instr::operand`] with
    /// the bank of the opcode for `JMP`).
    #[must_use]
    pub fn is_unconditional_jump(&self) -> bool {
        matches!(self.mnemonic, "BRA" | "BRL" | "JMP" | "JML") && !self.is_indirect_jump()
    }

    /// `JMP (abs)`, `JMP (abs,X)`, `JML [abs]`.
    #[must_use]
    pub fn is_indirect_jump(&self) -> bool {
        matches!(self.mode, AbsInd | AbsIndX | AbsIndLong) && self.mnemonic != "JSR"
    }

    /// Whether flow never continues to the next instruction.
    #[must_use]
    pub fn ends_flow(&self) -> bool {
        self.is_return()
            || self.is_unconditional_jump()
            || self.is_indirect_jump()
            || matches!(self.mnemonic, "STP")
    }

    /// The code address a branch or fixed jump or call goes to (24-bit, as written).
    #[must_use]
    pub fn target(&self) -> Option<u32> {
        match (self.mnemonic, self.mode) {
            (_, Rel8 | Rel16) if self.mnemonic != "PER" => Some(self.operand),
            ("JSR" | "JMP", Abs) => Some((self.addr & 0xFF_0000) | self.operand),
            ("JSL" | "JML", Long) => Some(self.operand),
            _ => None,
        }
    }
}

/// Decodes the instruction at `addr` with the widths `w`; `None` if the ROM does not map the
/// bytes.
#[must_use]
pub fn decode(rom: &RomImage<'_>, addr: u32, w: Widths) -> Option<Instr> {
    let op = rom.read(addr)?;
    let (mnemonic, mode) = OPCODES[usize::from(op)];
    let len: u8 = match mode {
        Imp | Acc => 1,
        ImmM => 1 + if w.m8 { 1 } else { 2 },
        ImmX => 1 + if w.x8 { 1 } else { 2 },
        Imm8 | Dp | DpX | DpY | DpInd | DpIndX | DpIndY | DpIndLong | DpIndLongY | Stack
        | StackIndY | Rel8 => 2,
        Abs | AbsX | AbsY | AbsInd | AbsIndX | AbsIndLong | Rel16 | Block => 3,
        Long | LongX => 4,
    };
    let byte = |i: u32| rom.read(addr.wrapping_add(i) & 0xFF_FFFF);
    let mut raw = 0_u32;
    for i in 1..u32::from(len) {
        raw |= u32::from(byte(i)?) << (8 * (i - 1));
    }
    let bank = addr & 0xFF_0000;
    let operand = match mode {
        Rel8 => {
            bank | (addr.wrapping_add(2).wrapping_add(i32::from(raw as u8 as i8) as u32) & 0xFFFF)
        }
        Rel16 => {
            bank | (addr.wrapping_add(3).wrapping_add(i32::from(raw as u16 as i16) as u32) & 0xFFFF)
        }
        _ => raw,
    };
    Some(Instr { addr, opcode: op, mnemonic, mode, len, operand })
}

/// Where a bus address lies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    /// Work RAM, offset into its 128 KiB.
    Wram(u32),
    /// Hardware registers `$2100-$43FF` (PPU, APU port, CPU, DMA).
    Mmio(u16),
    /// ROM, as a physical code address (banks `$00-$1F`).
    Rom(u32),
    Other,
}

/// Classifies a 24-bit bus address (low RAM and ROM mirrors folded).
#[must_use]
pub fn region(bus: u32) -> Region {
    let (bank, offset) = ((bus >> 16) as u8, bus as u16);
    match bank {
        0x7E | 0x7F => Region::Wram((u32::from(bank & 1) << 16) | u32::from(offset)),
        0x00..=0x3F | 0x80..=0xBF => match offset {
            0..=0x1FFF => Region::Wram(u32::from(offset)),
            0x2100..=0x43FF => Region::Mmio(offset),
            0x8000..=0xFFFF => Region::Rom(bus & 0x7F_FFFF),
            _ => Region::Other,
        },
        _ => Region::Other,
    }
}

/// The bus address a data operand names as far as the instruction itself says: `dbr` completes
/// the absolute modes, `dp` the direct-page ones. Indexed modes give the base (`indexed` tells).
/// `None` for modes that read through a pointer or the stack, and for instructions that only
/// name a code address.
#[must_use]
pub fn data_operand(instr: &Instr, dbr: u8, dp: u16) -> Option<DataOperand> {
    if instr.target().is_some() || instr.mnemonic == "PEA" {
        return None;
    }
    let direct =
        |offset: u32| (dp <= 0x1FFF).then_some(0x7E_0000 | ((u32::from(dp) + offset) & 0xFFFF));
    let (bus, indexed) = match instr.mode {
        Dp => (direct(instr.operand)?, false),
        DpX | DpY => (direct(instr.operand)?, true),
        Abs => ((u32::from(dbr) << 16) | instr.operand, false),
        AbsX | AbsY => ((u32::from(dbr) << 16) | instr.operand, true),
        Long => (instr.operand, false),
        LongX => (instr.operand, true),
        _ => return None,
    };
    Some(DataOperand { bus, indexed })
}

/// See [`data_operand`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataOperand {
    pub bus: u32,
    pub indexed: bool,
}

/// Names from `re/symbols.toml` for addresses.
pub struct Namer<'a> {
    pub symbols: Option<&'a Symbols>,
}

impl Namer<'_> {
    /// The routine, sync point or table whose first byte is the physical code address.
    #[must_use]
    pub fn code(&self, address: u32) -> Option<&str> {
        let key = address & 0x7F_FFFF;
        let symbols = self.symbols?;
        symbols.routines().iter().find(|r| r.address == key).map(|r| r.name.as_str()).or_else(
            || symbols.sync_points().iter().find(|p| p.address == key).map(|p| p.name.as_str()),
        )
    }

    /// `name` or `name+off` for a variable or table containing the bus address.
    #[must_use]
    pub fn data(&self, bus: u32) -> Option<String> {
        let symbols = self.symbols?;
        match region(bus) {
            Region::Wram(offset) => {
                let variable = symbols.variable_at(0x7E_0000 + offset)?;
                let start = canonical(variable.address);
                Some(offset_name(&variable.name, 0x7E_0000 + offset - start))
            }
            Region::Rom(phys) => symbols
                .tables()
                .iter()
                .find(|t| (t.address..t.address + t.size).contains(&phys))
                .map(|t| offset_name(&t.name, phys - t.address)),
            _ => None,
        }
    }
}

/// A bus address with the low RAM mirrors folded onto bank `$7E`.
fn canonical(address: u32) -> u32 {
    let (bank, offset) = ((address >> 16) as u8, address & 0xFFFF);
    if bank & 0x7F <= 0x3F && offset < 0x2000 { 0x7E_0000 | offset } else { address & 0xFF_FFFF }
}

fn offset_name(name: &str, delta: u32) -> String {
    if delta == 0 { name.to_owned() } else { format!("{name}+{delta}") }
}

/// The start state and limits of a listing.
#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    /// Widths forced by `--m8`, `--m16`, `--x8`, `--x16`.
    pub m8: Option<bool>,
    pub x8: Option<bool>,
    /// Disassemble exactly this many bytes, ignoring returns and jumps.
    pub len: Option<u32>,
    /// Data bank and direct page for naming operands.
    pub dbr: u8,
    pub dp: u16,
}

/// A line of the listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub instr: Instr,
    pub widths: Widths,
    /// The widths the trace saw at this address, if they disagree with the tracked ones.
    pub trace_conflict: Option<(bool, bool)>,
}

/// The result of [`disassemble`].
#[derive(Debug, Default)]
pub struct Listing {
    pub lines: Vec<Line>,
    /// Branch and jump targets inside the listing.
    pub labels: BTreeSet<u32>,
    /// Bytes skipped between two parts of the routine (an embedded table or padding).
    pub gaps: Vec<(u32, u32)>,
    /// Why the listing ended.
    pub end: String,
    /// The widths at the start and where they came from.
    pub entry: Option<Widths>,
    pub entry_source: &'static str,
}

/// Where the widths at the start come from, in order of preference.
pub fn entry_widths(
    options: &Options,
    symbol: Option<(Option<bool>, Option<bool>)>,
    observed: Option<(bool, bool)>,
) -> (Widths, &'static str) {
    let mut w = Widths::ASSUMED;
    let mut source = "assumed";
    if let Some((m8, x8)) = observed {
        w = Widths { m8, x8, m_known: true, x_known: true };
        source = "trace";
    }
    if let Some((m8, x8)) = symbol {
        let known = |hint: Option<bool>, value: &mut bool, known: &mut bool| {
            if let Some(hint) = hint {
                (*value, *known) = (hint, true);
            }
        };
        known(m8, &mut w.m8, &mut w.m_known);
        known(x8, &mut w.x8, &mut w.x_known);
        if m8.is_some() || x8.is_some() {
            source = "symbols.toml";
        }
    }
    if options.m8.is_some() || options.x8.is_some() {
        if let Some(m8) = options.m8 {
            (w.m8, w.m_known) = (m8, true);
        }
        if let Some(x8) = options.x8 {
            (w.x8, w.x_known) = (x8, true);
        }
        source = "flags";
    }
    (w, source)
}

/// Longest listing without `--len`, in bytes.
const MAX_BYTES: u32 = 0x1000;

/// Disassembles from `start` with the entry widths `entry`. `observed` gives the widths a trace
/// saw at an address (physical code address), used where the tracking does not know them.
#[must_use]
pub fn disassemble(
    rom: &RomImage<'_>,
    start: u32,
    entry: Widths,
    options: &Options,
    observed: &dyn Fn(u32) -> Option<(bool, bool)>,
) -> Listing {
    let mut listing = Listing { entry: Some(entry), ..Listing::default() };
    let bank = start & 0xFF_0000;
    let end_limit = options.len.map(|len| start.wrapping_add(len));
    let mut pending: BTreeMap<u32, Widths> = BTreeMap::new();
    let mut pc = start;
    let mut w = entry;
    let mut furthest = start;
    loop {
        if let Some(limit) = end_limit {
            if pc >= limit {
                "length reached".clone_into(&mut listing.end);
                break;
            }
        } else if pc.wrapping_sub(start) > MAX_BYTES {
            "size limit reached".clone_into(&mut listing.end);
            break;
        }
        if let Some(state) = pending.remove(&pc) {
            w = w.merge(state);
        }
        // Where the tracking does not know a width, a trace may.
        let trace = observed(pc);
        let mut conflict = None;
        if let Some((m8, x8)) = trace {
            if !w.m_known {
                (w.m8, w.m_known) = (m8, true);
            } else if w.m8 != m8 {
                conflict = Some((m8, x8));
            }
            if !w.x_known {
                (w.x8, w.x_known) = (x8, true);
            } else if w.x8 != x8 {
                conflict = Some((m8, x8));
            }
        }
        let Some(instr) = decode(rom, pc, w) else {
            "the bytes are not ROM".clone_into(&mut listing.end);
            break;
        };
        listing.lines.push(Line { instr: instr.clone(), widths: w, trace_conflict: conflict });
        w = w.after(&instr);
        let next = instr.next();
        furthest = furthest.max(next);
        if let Some(target) = instr.target().filter(|_| !instr.is_call()) {
            // A branch stays in the routine; a jump only when it lands in code the routine
            // already covers or is known to reach (else it is a tail call out).
            let reach = pending.keys().next_back().copied().unwrap_or(0).max(furthest);
            let same_bank = target & 0xFF_0000 == bank && target >= start;
            let is_branch = matches!(instr.mode, Rel8 | Rel16);
            if same_bank && (is_branch || target <= reach) {
                listing.labels.insert(target);
                if target > pc {
                    pending.entry(target).and_modify(|s| *s = s.merge(w)).or_insert(w);
                    furthest = furthest.max(target + 1);
                }
            }
        }
        if instr.ends_flow() && end_limit.is_none() {
            if let Some((&at, &state)) = pending.range(next..).next() {
                if at != next {
                    listing.gaps.push((next, at));
                }
                pc = at;
                w = state;
                continue;
            }
            listing.end = match instr.mnemonic {
                "RTS" | "RTL" | "RTI" | "STP" => format!("{} reached", instr.mnemonic),
                m if instr.is_indirect_jump() => format!("indirect {m} reached"),
                "BRA" | "BRL" => "unconditional branch reached".to_owned(),
                m => format!("{m} out of the routine"),
            };
            break;
        }
        pc = next;
    }
    listing
}

/// Formats the operand of an instruction.
fn operand_text(instr: &Instr, namer: &Namer<'_>) -> String {
    let v = instr.operand;
    let code_name =
        |target: u32| namer.code(target).map_or_else(|| format!("${target:06X}"), str::to_owned);
    match instr.mode {
        Imp => String::new(),
        Acc => "A".to_owned(),
        ImmM | ImmX => {
            if instr.len == 2 {
                format!("#${v:02X}")
            } else {
                format!("#${v:04X}")
            }
        }
        Imm8 => format!("#${v:02X}"),
        Dp => format!("${v:02X}"),
        DpX => format!("${v:02X},X"),
        DpY => format!("${v:02X},Y"),
        DpInd => format!("(${v:02X})"),
        DpIndX => format!("(${v:02X},X)"),
        DpIndY => format!("(${v:02X}),Y"),
        DpIndLong => format!("[${v:02X}]"),
        DpIndLongY => format!("[${v:02X}],Y"),
        Abs => match instr.target() {
            Some(target) => code_name(target),
            None => format!("${v:04X}"),
        },
        AbsX => format!("${v:04X},X"),
        AbsY => format!("${v:04X},Y"),
        Long => match instr.target() {
            Some(target) => code_name(target),
            None => format!("${v:06X}"),
        },
        LongX => format!("${v:06X},X"),
        AbsInd => format!("(${v:04X})"),
        AbsIndX => format!("(${v:04X},X)"),
        AbsIndLong => format!("[${v:04X}]"),
        Rel8 | Rel16 => {
            if instr.mnemonic == "PER" {
                format!("${:04X}", v & 0xFFFF)
            } else {
                code_name(instr.operand)
            }
        }
        Stack => format!("${v:02X},S"),
        StackIndY => format!("(${v:02X},S),Y"),
        Block => format!("${:02X},${:02X}", (v >> 8) & 0xFF, v & 0xFF),
    }
}

/// The label of a local target: the symbol's name, else `.Lxxxxxx`.
fn label_of(address: u32, namer: &Namer<'_>) -> String {
    namer.code(address).map_or_else(|| format!(".L{address:06X}"), str::to_owned)
}

/// Renders a listing as text.
#[must_use]
pub fn render(
    listing: &Listing,
    rom: &RomImage<'_>,
    namer: &Namer<'_>,
    options: &Options,
    title: &str,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "; {title}");
    if let Some(entry) = listing.entry {
        let width = |value: bool, known: bool, name: &str, bits: (u8, u8)| {
            let n = if value { bits.0 } else { bits.1 };
            format!("{name}{n}{}", if known { "" } else { "?" })
        };
        let _ = writeln!(
            out,
            "; entry widths: {} {} ({}); `?` = assumed, not known",
            width(entry.m8, entry.m_known, "M", (8, 16)),
            width(entry.x8, entry.x_known, "X", (8, 16)),
            listing.entry_source
        );
    }
    for line in &listing.lines {
        let instr = &line.instr;
        if listing.labels.contains(&instr.addr) {
            let _ = writeln!(out, "{}:", label_of(instr.addr, namer));
        }
        let bytes: Vec<String> = (0..u32::from(instr.len))
            .map(|i| {
                rom.read(instr.addr + i).map_or_else(|| "??".to_owned(), |b| format!("{b:02X}"))
            })
            .collect();
        let mut text = instr.mnemonic.to_owned();
        let operand = if matches!(instr.mode, Rel8 | Rel16)
            && instr.mnemonic != "PER"
            && listing.labels.contains(&instr.operand)
        {
            label_of(instr.operand, namer)
        } else {
            operand_text(instr, namer)
        };
        if !operand.is_empty() {
            text.push(' ');
            text.push_str(&operand);
        }
        let mut notes = Vec::new();
        if let Some(data) = data_operand(instr, options.dbr, options.dp) {
            if let Some(name) = namer.data(data.bus) {
                notes.push(if data.indexed { format!("{name}+idx") } else { name });
            } else if let Region::Mmio(register) = region(data.bus) {
                notes.push(format!("register ${register:04X}"));
            }
        }
        match instr.mode {
            ImmM if !line.widths.m_known => notes.push("M?".to_owned()),
            ImmX if !line.widths.x_known => notes.push("X?".to_owned()),
            _ => {}
        }
        if let Some((m8, x8)) = line.trace_conflict {
            notes.push(format!(
                "trace saw M{} X{}",
                if m8 { 8 } else { 16 },
                if x8 { 8 } else { 16 }
            ));
        }
        if instr.mode == Imp && matches!(instr.mnemonic, "TCS" | "TXS") {
            notes.push("stack switch".to_owned());
        }
        let notes =
            if notes.is_empty() { String::new() } else { format!("; {}", notes.join(", ")) };
        let _ = writeln!(
            out,
            "  {:02X}:{:04X}  {:<11}  {:<30} {notes}",
            instr.addr >> 16,
            instr.addr & 0xFFFF,
            bytes.join(" "),
            text
        );
        if let Some((_, to)) = listing.gaps.iter().find(|(from, _)| *from == instr.next()) {
            let _ = writeln!(out, "  ; ... {} bytes not disassembled", to - instr.next());
        }
    }
    let _ = writeln!(out, "; end: {}", listing.end);
    out
}

/// Parses `05B572`, `$05B572`, `0x05B572` or `05:B572`.
///
/// # Errors
///
/// Not a hexadecimal 24-bit address.
pub fn parse_address(text: &str) -> anyhow::Result<u32> {
    let t = text.trim_start_matches('$').trim_start_matches("0x").replace(':', "");
    let value =
        u32::from_str_radix(&t, 16).with_context(|| format!("`{text}` is not an address"))?;
    if value > 0xFF_FFFF {
        bail!("`{text}` is not a 24-bit address");
    }
    Ok(value)
}

/// Resolves a command-line target to a code address: a routine, sync point or table name of the
/// symbols, or an address.
///
/// # Errors
///
/// Neither a known name nor an address.
pub fn resolve_target(symbols: &Symbols, text: &str) -> anyhow::Result<u32> {
    let by_name = symbols
        .routines()
        .iter()
        .find(|r| r.name == text)
        .map(|r| r.address)
        .or_else(|| symbols.sync_points().iter().find(|p| p.name == text).map(|p| p.address))
        .or_else(|| symbols.tables().iter().find(|t| t.name == text).map(|t| t.address));
    match by_name {
        Some(address) => Ok(address),
        None => parse_address(text).with_context(|| {
            format!("`{text}` is neither a name in re/symbols.toml nor an address")
        }),
    }
}

/// The command line of `disasm`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    pub target: String,
    pub len: Option<u32>,
    pub m8: Option<bool>,
    pub x8: Option<bool>,
    pub dbr: u8,
    pub dp: u16,
    pub trace: bool,
}

/// Parses `<address|symbol> [--len N] [--m8|--m16] [--x8|--x16] [--dbr XX] [--dp XXXX]
/// [--no-trace]`.
///
/// # Errors
///
/// An unknown option, a missing or bad value, or no target.
pub fn parse_args(args: &[&str]) -> Result<Args, String> {
    let mut parsed =
        Args { target: String::new(), len: None, m8: None, x8: None, dbr: 0, dp: 0, trace: true };
    let mut args = args.iter();
    while let Some(&arg) = args.next() {
        match arg {
            "--m8" => parsed.m8 = Some(true),
            "--m16" => parsed.m8 = Some(false),
            "--x8" => parsed.x8 = Some(true),
            "--x16" => parsed.x8 = Some(false),
            "--no-trace" => parsed.trace = false,
            "--len" => {
                let value = args.next().ok_or("--len needs a byte count")?;
                let len = if let Some(hex) = value.strip_prefix("0x").or(value.strip_prefix('$')) {
                    u32::from_str_radix(hex, 16)
                } else {
                    value.parse()
                };
                parsed.len = Some(len.map_err(|_| format!("bad --len `{value}`"))?);
                if parsed.len == Some(0) {
                    return Err("--len must be at least 1".to_owned());
                }
            }
            "--dbr" => {
                let value = args.next().ok_or("--dbr needs a bank in hex")?;
                parsed.dbr = u8::from_str_radix(value.trim_start_matches('$'), 16)
                    .map_err(|_| format!("bad --dbr `{value}`"))?;
            }
            "--dp" => {
                let value = args.next().ok_or("--dp needs an offset in hex")?;
                parsed.dp = u16::from_str_radix(value.trim_start_matches('$'), 16)
                    .map_err(|_| format!("bad --dp `{value}`"))?;
            }
            option if option.starts_with("--") => return Err(format!("unknown option `{option}`")),
            target if parsed.target.is_empty() => target.clone_into(&mut parsed.target),
            other => return Err(format!("one target only (`{other}` is a second one)")),
        }
    }
    if parsed.target.is_empty() {
        return Err("needs an address or a symbol".to_owned());
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 32 KiB image with `code` at `$00:8000`.
    fn image(code: &[u8]) -> Vec<u8> {
        let mut rom = vec![0xEA_u8; 0x8000];
        rom[..code.len()].copy_from_slice(code);
        rom
    }

    fn image_of(code: &[u8]) -> Vec<u8> {
        image(code)
    }

    fn list(code: &[u8], options: &Options) -> Vec<String> {
        let rom = image(code);
        let image = RomImage(&rom);
        let (w, source) = entry_widths(options, None, None);
        let mut listing = disassemble(&image, 0x00_8000, w, options, &|_| None);
        listing.entry_source = source;
        let namer = Namer { symbols: None };
        render(&listing, &image, &namer, options, "test")
            .lines()
            .filter(|line| line.starts_with("  ") && !line.trim_start().starts_with(';'))
            .map(|line| {
                // `  bank:offset  bytes(11)  text(30) notes`: the text starts at column 24.
                let (text, notes) = line[24..].split_once(';').unwrap_or((&line[24..], ""));
                if notes.is_empty() {
                    text.trim().to_owned()
                } else {
                    format!("{} ; {}", text.trim(), notes.trim())
                }
            })
            .collect()
    }

    fn texts(code: &[u8]) -> Vec<String> {
        // The listing line is `bank:offset  bytes  text ; notes`: keep the text only.
        let rom = image(code);
        let image = RomImage(&rom);
        let options = Options::default();
        let (w, _) = entry_widths(&options, None, None);
        let listing = disassemble(&image, 0x00_8000, w, &options, &|_| None);
        let namer = Namer { symbols: None };
        listing
            .lines
            .iter()
            .map(|l| {
                let o = if matches!(l.instr.mode, Rel8 | Rel16) {
                    format!("${:04X}", l.instr.operand & 0xFFFF)
                } else {
                    operand_text(&l.instr, &namer)
                };
                if o.is_empty() {
                    l.instr.mnemonic.to_owned()
                } else {
                    format!("{} {o}", l.instr.mnemonic)
                }
            })
            .collect()
    }

    #[test]
    fn every_addressing_mode_family_is_decoded_and_written() {
        let code: &[u8] = &[
            0xA9, 0x12, // LDA #$12 (M8)
            0xA5, 0x34, // LDA $34
            0xB5, 0x34, // LDA $34,X
            0xB6, 0x34, // LDX $34,Y
            0xB2, 0x34, // LDA ($34)
            0xA1, 0x34, // LDA ($34,X)
            0xB1, 0x34, // LDA ($34),Y
            0xA7, 0x34, // LDA [$34]
            0xB7, 0x34, // LDA [$34],Y
            0xAD, 0x34, 0x12, // LDA $1234
            0xBD, 0x34, 0x12, // LDA $1234,X
            0xB9, 0x34, 0x12, // LDA $1234,Y
            0xAF, 0x56, 0x34, 0x12, // LDA $123456
            0xBF, 0x56, 0x34, 0x12, // LDA $123456,X
            0xA3, 0x05, // LDA $05,S
            0xB3, 0x05, // LDA ($05,S),Y
            0x0A, // ASL A
            0xEA, // NOP
            0x54, 0x7E, 0x7F, // MVN
            0xF4, 0x34, 0x12, // PEA $1234
            0xD4, 0x20, // PEI ($20)
            0xC2, 0x30, // REP #$30
            0xA9, 0x34, 0x12, // LDA #$1234
            0x6C, 0x34, 0x12, // JMP ($1234)
        ];
        assert_eq!(
            texts(code),
            [
                "LDA #$12",
                "LDA $34",
                "LDA $34,X",
                "LDX $34,Y",
                "LDA ($34)",
                "LDA ($34,X)",
                "LDA ($34),Y",
                "LDA [$34]",
                "LDA [$34],Y",
                "LDA $1234",
                "LDA $1234,X",
                "LDA $1234,Y",
                "LDA $123456",
                "LDA $123456,X",
                "LDA $05,S",
                "LDA ($05,S),Y",
                "ASL A",
                "NOP",
                "MVN $7F,$7E",
                "PEA $1234",
                "PEI ($20)",
                "REP #$30",
                "LDA #$1234",
                "JMP ($1234)",
            ]
        );
    }

    #[test]
    fn the_immediate_width_follows_rep_and_sep() {
        // Entry M8 X16: LDX is 3 bytes; SEP #$10 narrows X; LDX is then 2 bytes.
        let code: &[u8] = &[
            0xA2, 0x34, 0x12, // LDX #$1234
            0xE2, 0x10, // SEP #$10
            0xA2, 0x34, // LDX #$34
            0xC2, 0x20, // REP #$20
            0xA9, 0x78, 0x56, // LDA #$5678
            0x60,
        ];
        assert_eq!(
            texts(code),
            ["LDX #$1234", "SEP #$10", "LDX #$34", "REP #$20", "LDA #$5678", "RTS"]
        );
    }

    #[test]
    fn plp_and_xce_make_the_widths_unknown_and_the_listing_says_so() {
        let lines = list(&[0x28, 0xA9, 0x01, 0x60], &Options::default());
        assert_eq!(lines, ["PLP", "LDA #$01 ; M?", "RTS"]);
        let lines = list(&[0xFB, 0xA2, 0x01, 0x00, 0x60], &Options::default());
        assert!(lines[1].starts_with("LDX #$0001") && lines[1].ends_with("X?"), "{lines:?}");
        // A SEP after the PLP makes it known again.
        let lines = list(&[0x28, 0xE2, 0x20, 0xA9, 0x01, 0x60], &Options::default());
        assert_eq!(lines, ["PLP", "SEP #$20", "LDA #$01", "RTS"]);
    }

    #[test]
    fn the_hint_flags_set_the_entry_widths() {
        let wide = Options { m8: Some(false), x8: Some(true), ..Options::default() };
        assert_eq!(
            list(&[0xA9, 0x34, 0x12, 0xA0, 0x07, 0x60], &wide),
            ["LDA #$1234", "LDY #$07", "RTS"]
        );
        let (w, source) = entry_widths(&wide, Some((Some(true), Some(false))), Some((true, true)));
        assert_eq!((w.m8, w.x8, w.m_known, w.x_known, source), (false, true, true, true, "flags"));
        let (w, source) = entry_widths(&Options::default(), Some((Some(false), None)), None);
        assert_eq!((w.m8, w.m_known, w.x_known, source), (false, true, false, "symbols.toml"));
        let (w, source) = entry_widths(&Options::default(), None, Some((false, false)));
        assert_eq!((w.m8, w.x8, w.m_known, source), (false, false, true, "trace"));
    }

    #[test]
    fn branches_resolve_to_their_targets_across_both_directions() {
        let code: &[u8] = &[
            0xD0, 0x02, // 8000 BNE $8004
            0xEA, // 8002 NOP
            0xEA, // 8003 NOP
            0x80, 0xFA, // 8004 BRA $8000
        ];
        let rom = image(code);
        let image = RomImage(&rom);
        let listing = disassemble(&image, 0x8000, Widths::ASSUMED, &Options::default(), &|_| None);
        let targets: Vec<u32> = listing.lines.iter().filter_map(|l| l.instr.target()).collect();
        assert_eq!(targets, [0x8004, 0x8000]);
        assert_eq!(listing.labels, BTreeSet::from([0x8000, 0x8004]));
        let rom = image_of(&[0x82, 0x00, 0x01]);
        let brl = decode(&RomImage(&rom), 0x8000, Widths::ASSUMED).unwrap();
        assert_eq!((brl.mnemonic, brl.target()), ("BRL", Some(0x8103)));
    }

    #[test]
    fn the_listing_runs_past_a_return_to_the_end_of_the_local_branches() {
        let code: &[u8] = &[
            0xF0, 0x03, // 8000 BEQ $8005
            0xA9, 0x01, // 8002 LDA #$01
            0x60, // 8004 RTS
            0xA9, 0x02, // 8005 LDA #$02
            0x6B, // 8007 RTL
            0xEA, // 8008 NOP (not part of the routine)
        ];
        let rom = image(code);
        let image = RomImage(&rom);
        let listing = disassemble(&image, 0x8000, Widths::ASSUMED, &Options::default(), &|_| None);
        let mnemonics: Vec<&str> = listing.lines.iter().map(|l| l.instr.mnemonic).collect();
        assert_eq!(mnemonics, ["BEQ", "LDA", "RTS", "LDA", "RTL"]);
        assert_eq!(listing.end, "RTL reached");
    }

    #[test]
    fn a_jump_out_ends_the_routine_and_len_overrides_the_ends() {
        let code: &[u8] = &[0xA9, 0x01, 0x4C, 0x00, 0x90, 0xEA, 0xEA];
        let rom = image(code);
        let image = RomImage(&rom);
        let listing = disassemble(&image, 0x8000, Widths::ASSUMED, &Options::default(), &|_| None);
        assert_eq!(listing.lines.len(), 2);
        assert_eq!(listing.end, "JMP out of the routine");
        let options = Options { len: Some(7), ..Options::default() };
        let listing = disassemble(&image, 0x8000, Widths::ASSUMED, &options, &|_| None);
        assert_eq!(listing.lines.len(), 4);
        assert_eq!(listing.end, "length reached");
    }

    #[test]
    fn widths_that_two_paths_disagree_on_become_unknown() {
        let code: &[u8] = &[
            0xF0, 0x02, // 8000 BEQ $8004
            0xE2, 0x20, // 8002 SEP #$20 (M8 on the fall-through path)
            0xA9, 0x11, // 8004 LDA #$11?  the other path reaches here with M16
            0x60,
        ];
        let rom = image(code);
        let image = RomImage(&rom);
        let entry = Widths { m8: false, x8: false, m_known: true, x_known: true };
        let listing = disassemble(&image, 0x8000, entry, &Options::default(), &|_| None);
        let lda = &listing.lines[2];
        assert!(!lda.widths.m_known, "the paths disagree: {lda:?}");
    }

    #[test]
    fn a_trace_fills_in_unknown_widths_and_flags_disagreements() {
        let code: &[u8] = &[0x28, 0xA9, 0x01, 0x00, 0x60];
        let rom = image(code);
        let image = RomImage(&rom);
        let observed = |pc: u32| (pc == 0x8001).then_some((false, true));
        let listing = disassemble(&image, 0x8000, Widths::ASSUMED, &Options::default(), &observed);
        assert_eq!(listing.lines[1].instr.mnemonic, "LDA");
        assert_eq!(listing.lines[1].instr.len, 3, "M16 from the trace");
        assert!(listing.lines[1].widths.m_known);
        let known = Widths { m8: true, x8: false, m_known: true, x_known: true };
        let listing = disassemble(&image, 0x8000, known, &Options::default(), &observed);
        // PLP made the widths unknown, so the trace still wins there; at the entry nothing is read.
        assert!(listing.lines[1].widths.m_known);
        let code: &[u8] = &[0xA9, 0x01, 0x60];
        let rom = image_of(code);
        let image = RomImage(&rom);
        let observed = |pc: u32| (pc == 0x8000).then_some((false, false));
        let listing = disassemble(&image, 0x8000, known, &Options::default(), &observed);
        assert_eq!(listing.lines[0].trace_conflict, Some((false, false)));
    }

    #[test]
    fn names_come_from_the_symbols_file() {
        let source = "\
[[routine]]\nname = \"helper\"\naddress = 0x008010\nreturns = \"rts\"\ntiming = \"pure\"\nmode = \"original\"\n\
[[table]]\nname = \"tab\"\naddress = 0x009000\nsize = 16\n\
[[variable]]\nname = \"counter\"\naddress = 0x7E0010\nsize = 2\n";
        let symbols = Symbols::parse(source, std::path::Path::new("symbols.toml")).unwrap();
        let namer = Namer { symbols: Some(&symbols) };
        let code: &[u8] = &[
            0x20, 0x10, 0x80, // 8000 JSR helper
            0xE6, 0x11, // 8003 INC $11
            0xBF, 0x04, 0x90, 0x00, // 8005 LDA tab+4,X
            0x60, // 8009 RTS
        ];
        let mut rom = image(code);
        rom[0x10] = 0x60;
        let image = RomImage(&rom);
        let options = Options::default();
        let listing = disassemble(&image, 0x8000, Widths::ASSUMED, &options, &|_| None);
        let text = render(&listing, &image, &namer, &options, "t");
        assert!(text.contains("JSR helper"), "{text}");
        assert!(text.contains("; counter+1"), "{text}");
        assert!(text.contains("; tab+4+idx"), "{text}");
        assert_eq!(namer.code(0x00_8010), Some("helper"));
        assert_eq!(namer.code(0x80_8010), Some("helper"), "mirrors");
        assert_eq!(namer.data(0x00_0010).as_deref(), Some("counter"));
        assert_eq!(namer.data(0x7E_0011).as_deref(), Some("counter+1"));
    }

    #[test]
    fn regions_and_data_operands_follow_the_memory_map() {
        assert_eq!(region(0x00_0010), Region::Wram(0x10));
        assert_eq!(region(0x80_1FFF), Region::Wram(0x1FFF));
        assert_eq!(region(0x7F_0001), Region::Wram(0x1_0001));
        assert_eq!(region(0x00_2118), Region::Mmio(0x2118));
        assert_eq!(region(0x00_420B), Region::Mmio(0x420B));
        assert_eq!(region(0x00_4400), Region::Other);
        assert_eq!(region(0x85_8123), Region::Rom(0x05_8123));
        assert_eq!(region(0x70_0000), Region::Other);
        let rom = image(&[0x8D, 0x0B, 0x42, 0xA5, 0x10, 0xB5, 0x10]);
        let image = RomImage(&rom);
        let sta = decode(&image, 0x8000, Widths::ASSUMED).unwrap();
        assert_eq!(data_operand(&sta, 0, 0), Some(DataOperand { bus: 0x00_420B, indexed: false }));
        let lda = decode(&image, 0x8003, Widths::ASSUMED).unwrap();
        assert_eq!(data_operand(&lda, 0, 0x0100).unwrap().bus, 0x7E_0110);
        assert_eq!(data_operand(&lda, 0, 0x4000), None, "a direct page outside low RAM");
        let ldax = decode(&image, 0x8005, Widths::ASSUMED).unwrap();
        assert!(data_operand(&ldax, 0, 0).unwrap().indexed);
        let jsr = Instr {
            addr: 0x8000,
            opcode: 0x20,
            mnemonic: "JSR",
            mode: Abs,
            len: 3,
            operand: 0x9000,
        };
        assert_eq!(data_operand(&jsr, 0, 0), None);
    }

    #[test]
    fn addresses_and_names_are_parsed() {
        for text in ["05B572", "$05B572", "0x05B572", "05:B572"] {
            assert_eq!(parse_address(text).unwrap(), 0x05_B572, "{text}");
        }
        assert!(parse_address("zzz").is_err());
        assert!(parse_address("1000000").is_err());
        let args =
            parse_args(&["foo", "--len", "0x20", "--m16", "--x8", "--dbr", "7e", "--dp", "1F00"])
                .unwrap();
        assert_eq!(
            args,
            Args {
                target: "foo".to_owned(),
                len: Some(32),
                m8: Some(false),
                x8: Some(true),
                dbr: 0x7E,
                dp: 0x1F00,
                trace: true
            }
        );
        for bad in [&[][..], &["a", "b"], &["a", "--len"], &["a", "--len", "0"], &["a", "--zip"]] {
            assert!(parse_args(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_opcode_table_has_the_right_lengths_for_every_opcode() {
        // Spot checks of the hard ones: lengths with both widths.
        let mut rom = vec![0_u8; 0x8000];
        for (opcode, m8, x8, len) in [
            (0xA9, true, false, 2),
            (0xA9, false, false, 3),
            (0xA2, true, true, 2),
            (0xA2, true, false, 3),
            (0x89, false, true, 3),
            (0xC0, true, true, 2),
            (0x00, true, true, 2),
            (0x42, true, true, 2),
            (0x62, true, true, 3),
            (0x82, true, true, 3),
            (0x22, true, true, 4),
            (0x5C, true, true, 4),
            (0xDC, true, true, 3),
            (0xFC, true, true, 3),
            (0x44, true, true, 3),
            (0xEB, true, true, 1),
        ] {
            rom[0] = opcode;
            let w = Widths { m8, x8, m_known: true, x_known: true };
            let instr = decode(&RomImage(&rom), 0x8000, w).unwrap();
            assert_eq!(instr.len, len, "opcode {opcode:02X}");
        }
        assert!(decode(&RomImage(&rom), 0x00_1000, Widths::ASSUMED).is_none());
    }
}
