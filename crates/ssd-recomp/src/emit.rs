//! Rust source for one function variant: instruction semantics, cycle charging and control flow.
//!
//! Every instruction becomes a block of statements on the register file `r` and the bus access
//! `ctx` ([`crate::rt`]). Static cycles (opcode and operand fetches, idle cycles) accumulate and
//! are added to `r.cy` just before the next memory access, so a hardware-register access sees
//! the exact time; accesses add their own cycles (their speed depends on the address).

use crate::analysis::{Callee, FuncKey, Graph, JumpTo, Next, Node, SKey};
use crate::decode::{Insn, Mode};
use crate::rt::win;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;

/// The worst-case master cycles after which a straight stretch of code starts a new block (and
/// so a new window check and a new place the original can be met at).
const BLOCK_CAP: u32 = 72;

const F: u32 = 8;
const I: u32 = 6;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Acc {
    Read,
    Write,
    Rmw,
}

/// A block of a function variant that can be entered from outside (where the original arrives).
#[derive(Debug, Clone, Copy)]
pub struct Leader {
    pub pc: u32,
    /// The M/X bits the block runs with.
    pub mx: u8,
    /// The block's number in the function's `match`.
    pub block: u32,
    /// The worst-case cost of the block.
    pub cost: u32,
}

/// What the emitter produced for a function variant.
pub struct Emitted {
    pub source: String,
    /// The blocks the original can arrive at.
    pub leaders: Vec<Leader>,
    /// The worst-case cost of the first block (the entry's timing bound).
    pub entry_cost: u32,
    /// Instructions in the function.
    pub instructions: usize,
    /// Whether the function returns with RTL (else RTS).
    pub long_return: bool,
    /// Blocks that end in a bail, by reason.
    pub bails: BTreeMap<&'static str, u32>,
    /// Blocks of the function (each sets its flag in the generated `COV` table).
    pub blocks: u32,
}

struct Em {
    out: String,
    pend: u32,
    cost: u32,
    /// The durations of the static cycles added to `pend`.
    durs: Vec<u32>,
}

impl Em {
    fn new() -> Self {
        Self { out: String::new(), pend: 0, cost: 0, durs: Vec::new() }
    }

    fn cy(&mut self, n: u32) {
        self.pend += n;
        self.cost += n;
    }

    fn f(&mut self, n: u32) {
        self.cy(F * n);
        self.durs.extend(std::iter::repeat_n(F, n as usize));
    }

    fn i(&mut self, n: u32) {
        self.cy(I * n);
        self.durs.extend(std::iter::repeat_n(I, n as usize));
    }

    /// The pending static run as `(cycles, k, h)` for `R::add`, and clears it.
    fn take(&mut self) -> (u32, u32, u32) {
        let k = self.durs.len().min(3);
        let mut h = 0;
        for &d in &self.durs[self.durs.len() - k..] {
            h = (h << 2) | u32::from(d != I);
        }
        let n = self.pend;
        self.pend = 0;
        self.durs.clear();
        (n, k as u32, h)
    }

    /// Writes the pending static cycles.
    fn sync(&mut self) {
        if self.pend > 0 {
            let (n, k, h) = self.take();
            let _ = writeln!(self.out, "r.add({n}, {k}, {h});");
        }
    }

    fn line(&mut self, s: &str) {
        self.out.push_str(s);
        self.out.push('\n');
    }

    /// A dynamic cycle addition with its worst case.
    fn dyn_cy(&mut self, expr: &str, worst: u32) {
        self.sync();
        let _ = writeln!(self.out, "r.addp({expr});");
        self.cost += worst;
    }

    /// Worst-case cost of `bytes` data accesses (counted statically; the helper adds the real
    /// cycles).
    fn data(&mut self, bytes: u32, worst: u32) {
        self.cost += bytes * worst;
    }
}

fn lit(v: u32, wide: bool) -> String {
    if wide { format!("0x{:04X}u16", v & 0xFFFF) } else { format!("0x{:02X}u8", v & 0xFF) }
}

/// Worst-case master cycles of a data access at the static `offset` (unknown bank).
fn worst_at(offset: u32) -> u32 {
    if (0x4000..=0x41FF).contains(&(offset & 0xFFFF)) { 12 } else { 8 }
}

/// Whether the instruction writes to a static address that can be a hardware register: the
/// block ends there, so the next stretch is checked against the new timing.
fn writes_io(insn: &Insn) -> bool {
    let writes = matches!(
        insn.mn,
        "STA"
            | "STX"
            | "STY"
            | "STZ"
            | "INC"
            | "DEC"
            | "ASL"
            | "LSR"
            | "ROL"
            | "ROR"
            | "TSB"
            | "TRB"
    ) && insn.mode != Mode::Acc;
    let offset = insn.opr & 0xFFFF;
    writes
        && matches!(insn.mode, Mode::Abs | Mode::AbsX | Mode::AbsY | Mode::Long | Mode::LongX)
        && (0x2000..=0x5FFF).contains(&offset)
}

/// Instructions after which the next stretch is checked on its own: a hardware-register write,
/// a change of the interrupt-disable flag, or a block move (its iterations check their own
/// windows, so what follows them is checked at the time it runs).
fn splits_after(insn: &Insn) -> bool {
    match insn.mn {
        "CLI" | "PLP" | "MVN" | "MVP" => true,
        "REP" => insn.opr & 0x04 != 0,
        _ => writes_io(insn),
    }
}

fn branch_cond(mn: &str) -> &'static str {
    match mn {
        "BPL" => "!r.n",
        "BMI" => "r.n",
        "BVC" => "!r.v",
        "BVS" => "r.v",
        "BCC" => "!r.c",
        "BCS" => "r.c",
        "BNE" => "!r.z",
        "BEQ" => "r.z",
        _ => "true",
    }
}

/// The effective address of a memory operand, as `let ea = ...;`; returns whether a 16-bit
/// access wraps within the 16-bit offset (direct page and stack-relative).
fn ea(em: &mut Em, insn: &Insn, acc: Acc, bytes: u32) -> bool {
    let off = insn.opr & 0xFF;
    let abs = insn.opr & 0xFFFF;
    let x8 = insn.x8;
    match insn.mode {
        Mode::Dp | Mode::DpX | Mode::DpY => {
            em.f(2);
            if insn.mode != Mode::Dp {
                em.i(1);
            }
            em.dyn_cy("r.dpx()", 6);
            let idx = match insn.mode {
                Mode::DpX => "r.x",
                Mode::DpY => "r.y",
                _ => "0",
            };
            let _ = writeln!(em.out, "let ea = r.dp(0x{off:02X}, {idx});");
            em.data(bytes, 8);
            true
        }
        Mode::DpInd | Mode::DpIndX => {
            em.f(2);
            if insn.mode == Mode::DpIndX {
                em.i(1);
            }
            em.dyn_cy("r.dpx()", 6);
            em.sync();
            let idx = if insn.mode == Mode::DpIndX { "r.x" } else { "0" };
            if insn.mode == Mode::DpIndX {
                // The index can take the pointer out of the RAM, to the open bus.
                let last = (insn.pc & 0xFF_0000)
                    | (insn.pc.wrapping_add(u32::from(insn.len) - 1) & 0xFFFF);
                let _ = writeln!(
                    em.out,
                    "let pa = r.dp(0x{off:02X}, {idx}); if is_open(pa) {{ ctx.read8(0x{last:06X}); }} let p = r.rd16w(ctx, pa);"
                );
            } else {
                let _ = writeln!(em.out, "let p = r.rd16w(ctx, r.dp(0x{off:02X}, {idx}));");
            }
            em.data(2 + bytes, 8);
            em.line("let ea = r.db(p, 0);");
            false
        }
        Mode::DpIndY => {
            em.f(2);
            em.dyn_cy("r.dpx()", 6);
            em.sync();
            let _ = writeln!(em.out, "let p = r.rd16w(ctx, r.dp(0x{off:02X}, 0));");
            em.data(2 + bytes, 8);
            if acc == Acc::Read && x8 {
                em.dyn_cy("pcx(p, r.y)", 6);
            } else {
                em.i(1);
            }
            em.line("let ea = r.db(p, r.y);");
            false
        }
        Mode::DpIndLong | Mode::DpIndLongY => {
            em.f(2);
            em.dyn_cy("r.dpx()", 6);
            em.sync();
            let _ = writeln!(em.out, "let p = r.rd24w(ctx, r.dp(0x{off:02X}, 0));");
            em.data(3 + bytes, 8);
            if insn.mode == Mode::DpIndLongY {
                em.line("let ea = p.wrapping_add(u32::from(r.y)) & 0xFF_FFFF;");
            } else {
                em.line("let ea = p;");
            }
            false
        }
        Mode::Abs => {
            em.f(3);
            let _ = writeln!(em.out, "let ea = r.db(0x{abs:04X}, 0);");
            em.data(bytes, worst_at(abs));
            false
        }
        Mode::AbsX | Mode::AbsY => {
            em.f(3);
            let idx = if insn.mode == Mode::AbsX { "r.x" } else { "r.y" };
            if acc == Acc::Read && x8 {
                em.dyn_cy(&format!("pcx(0x{abs:04X}, {idx})"), 6);
            } else {
                em.i(1);
            }
            let _ = writeln!(em.out, "let ea = r.db(0x{abs:04X}, {idx});");
            em.data(bytes, worst_at(abs));
            false
        }
        Mode::Long => {
            em.f(4);
            let _ = writeln!(em.out, "let ea = 0x{:06X}u32;", insn.opr & 0xFF_FFFF);
            em.data(bytes, worst_at(abs));
            false
        }
        Mode::LongX => {
            em.f(4);
            let _ = writeln!(
                em.out,
                "let ea = (0x{:06X}u32 + u32::from(r.x)) & 0xFF_FFFF;",
                insn.opr & 0xFF_FFFF
            );
            em.data(bytes, worst_at(abs));
            false
        }
        Mode::Stack => {
            em.f(2);
            em.i(1);
            let _ = writeln!(em.out, "let ea = u32::from(r.s.wrapping_add(0x{off:02X}));");
            em.data(bytes, 8);
            true
        }
        Mode::StackIndY => {
            em.f(2);
            em.i(1);
            em.sync();
            let _ =
                writeln!(em.out, "let p = r.rd16w(ctx, u32::from(r.s.wrapping_add(0x{off:02X})));");
            em.data(2 + bytes, 8);
            em.i(1);
            em.line("let ea = r.db(p, r.y);");
            false
        }
        _ => unreachable!("not a memory mode: {:?}", insn.mode),
    }
}

/// Reads the operand: returns the name or literal of the value.
fn read_val(em: &mut Em, insn: &Insn, wide: bool) -> String {
    if matches!(insn.mode, Mode::ImmM | Mode::ImmX) {
        em.f(u32::from(insn.len));
        return lit(insn.opr, wide);
    }
    let w = ea(em, insn, Acc::Read, if wide { 2 } else { 1 });
    em.sync();
    open_bus(em, insn);
    let call = match (wide, w) {
        (false, _) => "rd8",
        (true, true) => "rd16w",
        (true, false) => "rd16",
    };
    let _ = writeln!(em.out, "let v = r.{call}(ctx, ea);");
    "v".to_owned()
}

fn store(em: &mut Em, insn: &Insn, value: &str, wide: bool) {
    let w = ea(em, insn, Acc::Write, if wide { 2 } else { 1 });
    em.sync();
    if !wide
        && matches!(insn.mode, Mode::Abs | Mode::Long)
        && (0x4200..=0x420A).contains(&(insn.opr & 0xFFFF))
    {
        // A store to the interrupt and timer registers: the original recognises an interrupt
        // it raises after the next instruction.
        let mx = (u32::from(insn.m8) << 5) | (u32::from(insn.x8) << 4);
        let _ = writeln!(
            em.out,
            "if r.wr8_ctl(ctx, ea, {value}) {{ return r.bail(0x{:06X}, 0x{mx:02X}); }}",
            insn.next()
        );
        return;
    }
    if !wide
        && matches!(insn.mode, Mode::Abs | Mode::Long)
        && insn.opr & 0xFFFF == 0x420B
        && insn.mn != "STZ"
    {
        // A store to MDMAEN: a transfer it starts is left to the original code.
        let mx = (u32::from(insn.m8) << 5) | (u32::from(insn.x8) << 4);
        let _ = writeln!(
            em.out,
            "if r.wr8_dma(ctx, ea, {value}, 0x{:06X}) {{ return r.bail(0x{:06X}, 0x{mx:02X}); }}",
            insn.next(),
            insn.next()
        );
        return;
    }
    if may_hit_event_registers(insn, wide) {
        // A computed address (or a word) that can land on `$4200-$420B`: checked as it runs.
        let mx = (u32::from(insn.m8) << 5) | (u32::from(insn.x8) << 4);
        let next = insn.next();
        let hit = if wide {
            format!("r.wr16_sys(ctx, ea, {value}, {w})")
        } else {
            format!("r.wr8_sys(ctx, ea, {value})")
        };
        let _ = writeln!(em.out, "if {hit} {{ return r.bail(0x{next:06X}, 0x{mx:02X}); }}");
        return;
    }
    let call = match (wide, w) {
        (false, _) => "wr8",
        (true, true) => "wr16w",
        (true, false) => "wr16",
    };
    let _ = writeln!(em.out, "r.{call}(ctx, ea, {value});");
}

/// Whether a store can write one of the CPU's event registers (`$4200-$420B`) at an address the
/// generator does not know: a pointer or an index, or a word whose high byte lands there.
fn may_hit_event_registers(insn: &Insn, wide: bool) -> bool {
    let base = insn.opr & 0xFFFF;
    match insn.mode {
        Mode::DpInd
        | Mode::DpIndX
        | Mode::DpIndY
        | Mode::DpIndLong
        | Mode::DpIndLongY
        | Mode::StackIndY => true,
        Mode::AbsX | Mode::AbsY | Mode::LongX => !insn.x8 || (0x4100..=0x420B).contains(&base),
        Mode::Abs | Mode::Long => wide && (0x41FF..=0x420B).contains(&base),
        _ => false,
    }
}

/// A read of a hardware register returns, in its undriven bits, the last byte the CPU read from
/// the cartridge: the last byte of the instruction. The core keeps that byte for open-bus
/// reads; a recompiled instruction does not fetch, so it is read again just before. An indexed
/// address that can land in `$2000-$5FFF` at run time (the index decides) is checked as it runs.
fn open_bus(em: &mut Em, insn: &Insn) {
    let offset = insn.opr & 0xFFFF;
    let last = (insn.pc & 0xFF_0000) | (insn.pc.wrapping_add(u32::from(insn.len) - 1) & 0xFFFF);
    match insn.mode {
        Mode::Abs | Mode::AbsX | Mode::AbsY | Mode::Long | Mode::LongX
            if (0x2000..=0x7FFF).contains(&offset) =>
        {
            let _ = writeln!(em.out, "ctx.read8(0x{last:06X});");
        }
        Mode::AbsX | Mode::AbsY | Mode::LongX
            if !insn.x8 || (0x1F01..=0x5FFF).contains(&offset) =>
        {
            let _ = writeln!(em.out, "if is_open(ea) {{ ctx.read8(0x{last:06X}); }}");
        }
        // The pointer is read from RAM, which leaves the open bus alone; an index can take a
        // direct-page address out of the RAM too.
        Mode::DpX
        | Mode::DpY
        | Mode::DpInd
        | Mode::DpIndX
        | Mode::DpIndY
        | Mode::DpIndLong
        | Mode::DpIndLongY
        | Mode::StackIndY => {
            let _ = writeln!(em.out, "if is_open(ea) {{ ctx.read8(0x{last:06X}); }}");
        }
        _ => {}
    }
}

/// Read-modify-write; `op` is an expression on `v` giving the new value.
fn rmw(em: &mut Em, insn: &Insn, wide: bool, op: &str) {
    let w = ea(em, insn, Acc::Rmw, if wide { 4 } else { 2 });
    em.sync();
    open_bus(em, insn);
    let (rd, wr) = match (wide, w) {
        (false, _) => ("rd8", "wr8"),
        (true, true) => ("rd16w", "wr16rw"),
        (true, false) => ("rd16", "wr16r"),
    };
    let _ = writeln!(em.out, "let v = r.{rd}(ctx, ea);");
    em.i(1);
    em.sync();
    let _ = writeln!(em.out, "let n = {op};");
    let _ = writeln!(em.out, "r.{wr}(ctx, ea, n);");
}

/// Emits an instruction that is not a control transfer.
#[allow(clippy::too_many_lines)]
fn plain(em: &mut Em, insn: &Insn) {
    let (m8, x8) = (insn.m8, insn.x8);
    let mw = !m8;
    let xw = !x8;
    let a = if m8 { "(r.a as u8)" } else { "r.a" };
    let xr = if x8 { "(r.x as u8)" } else { "r.x" };
    let yr = if x8 { "(r.y as u8)" } else { "r.y" };
    let nz_a = if m8 { "r.nz8" } else { "r.nz16" };
    let nz_x = if x8 { "r.nz8" } else { "r.nz16" };
    let _ = writeln!(em.out, "// {:06X} {}", insn.pc, insn.mn);
    match insn.mn {
        "LDA" => {
            let v = read_val(em, insn, mw);
            if m8 {
                let _ = writeln!(em.out, "r.set_al({v});");
            } else {
                let _ = writeln!(em.out, "r.a = {v};");
            }
            let _ = writeln!(em.out, "{nz_a}({v});");
        }
        "LDX" | "LDY" => {
            let v = read_val(em, insn, xw);
            let reg = if insn.mn == "LDX" { "r.x" } else { "r.y" };
            let _ = writeln!(em.out, "{reg} = u16::from({v});");
            let _ = writeln!(em.out, "{nz_x}({v});");
        }
        "AND" | "ORA" | "EOR" => {
            let v = read_val(em, insn, mw);
            let op = match insn.mn {
                "AND" => "&",
                "ORA" => "|",
                _ => "^",
            };
            if m8 {
                let _ = writeln!(em.out, "let t = {a} {op} {v}; r.set_al(t); r.nz8(t);");
            } else {
                let _ = writeln!(em.out, "r.a = r.a {op} {v}; r.nz16(r.a);");
            }
        }
        "ADC" | "SBC" => {
            let v = read_val(em, insn, mw);
            let f = format!("{}{}", insn.mn.to_ascii_lowercase(), if m8 { "8" } else { "16" });
            let _ = writeln!(em.out, "r.{f}({v});");
        }
        "CMP" => {
            let v = read_val(em, insn, mw);
            let f = if m8 { "cmp8" } else { "cmp16" };
            let _ = writeln!(em.out, "r.{f}({a}, {v});");
        }
        "CPX" | "CPY" => {
            let v = read_val(em, insn, xw);
            let f = if x8 { "cmp8" } else { "cmp16" };
            let reg = if insn.mn == "CPX" { xr } else { yr };
            let _ = writeln!(em.out, "r.{f}({reg}, {v});");
        }
        "BIT" => {
            let v = read_val(em, insn, mw);
            if matches!(insn.mode, Mode::ImmM) {
                let _ = writeln!(em.out, "r.z = {a} & {v} == 0;");
            } else {
                let f = if m8 { "bit8" } else { "bit16" };
                let _ = writeln!(em.out, "r.{f}({v});");
            }
        }
        "STA" => store(em, insn, if m8 { "r.a as u8" } else { "r.a" }, mw),
        "STX" => store(em, insn, if x8 { "r.x as u8" } else { "r.x" }, xw),
        "STY" => store(em, insn, if x8 { "r.y as u8" } else { "r.y" }, xw),
        "STZ" => store(em, insn, if m8 { "0u8" } else { "0u16" }, mw),
        "ASL" | "LSR" | "ROL" | "ROR" => {
            let f = format!("{}{}", insn.mn.to_ascii_lowercase(), if m8 { "8" } else { "16" });
            if insn.mode == Mode::Acc {
                em.f(1);
                em.i(1);
                if m8 {
                    let _ = writeln!(em.out, "let t = r.{f}({a}); r.set_al(t);");
                } else {
                    let _ = writeln!(em.out, "r.a = r.{f}(r.a);");
                }
            } else {
                rmw(em, insn, mw, &format!("r.{f}(v)"));
            }
        }
        "INC" | "DEC" => {
            let step = if insn.mn == "INC" { "wrapping_add" } else { "wrapping_sub" };
            if insn.mode == Mode::Acc {
                em.f(1);
                em.i(1);
                if m8 {
                    let _ = writeln!(em.out, "let t = {a}.{step}(1); r.set_al(t); r.nz8(t);");
                } else {
                    let _ = writeln!(em.out, "r.a = r.a.{step}(1); r.nz16(r.a);");
                }
            } else {
                let nz = if m8 { "r.nz8" } else { "r.nz16" };
                rmw(em, insn, mw, &format!("{{ let n = v.{step}(1); {nz}(n); n }}"));
            }
        }
        "TSB" | "TRB" => {
            let tsb = insn.mn == "TSB";
            let (z, val) = if m8 {
                (
                    "r.z = (r.a as u8) & v == 0;",
                    if tsb { "v | (r.a as u8)" } else { "v & !(r.a as u8)" },
                )
            } else {
                ("r.z = r.a & v == 0;", if tsb { "v | r.a" } else { "v & !r.a" })
            };
            rmw(em, insn, mw, &format!("{{ {z} {val} }}"));
        }
        "INX" | "INY" | "DEX" | "DEY" => {
            em.f(1);
            em.i(1);
            let reg = if insn.mn.ends_with('X') { "r.x" } else { "r.y" };
            let step = if insn.mn.starts_with("IN") { "wrapping_add" } else { "wrapping_sub" };
            if x8 {
                let _ = writeln!(
                    em.out,
                    "let t = ({reg} as u8).{step}(1); {reg} = u16::from(t); r.nz8(t);"
                );
            } else {
                let _ = writeln!(em.out, "{reg} = {reg}.{step}(1); r.nz16({reg});");
            }
        }
        "CLC" | "SEC" | "CLI" | "SEI" | "CLD" | "SED" | "CLV" | "NOP" => {
            em.f(1);
            em.i(1);
            let s = match insn.mn {
                "CLC" => "r.c = false;",
                "SEC" => "r.c = true;",
                "CLI" => "r.irq = false;",
                "SEI" => "r.irq = true;",
                "CLD" => "r.dec = false;",
                "SED" => "r.dec = true;",
                "CLV" => "r.v = false;",
                _ => "",
            };
            em.line(s);
        }
        "REP" | "SEP" => {
            em.f(2);
            em.i(1);
            let set = insn.mn == "SEP";
            let bits = (insn.opr & 0xFF) as u8;
            let other = bits & !0x30;
            if other != 0 {
                let _ = writeln!(em.out, "r.set_flags(0x{other:02X}, {set});");
            }
            if set && bits & 0x10 != 0 {
                em.line("r.x &= 0x00FF; r.y &= 0x00FF;");
            }
        }
        "TAX" | "TAY" => {
            em.f(1);
            em.i(1);
            let reg = if insn.mn == "TAX" { "r.x" } else { "r.y" };
            if x8 {
                let _ = writeln!(em.out, "{reg} = r.a & 0x00FF; r.nz8({reg} as u8);");
            } else {
                let _ = writeln!(em.out, "{reg} = r.a; r.nz16({reg});");
            }
        }
        "TXA" | "TYA" => {
            em.f(1);
            em.i(1);
            let reg = if insn.mn == "TXA" { "r.x" } else { "r.y" };
            if m8 {
                let _ = writeln!(em.out, "let t = {reg} as u8; r.set_al(t); r.nz8(t);");
            } else {
                let _ = writeln!(em.out, "r.a = {reg}; r.nz16(r.a);");
            }
        }
        "TXY" | "TYX" => {
            em.f(1);
            em.i(1);
            let (to, from) = if insn.mn == "TXY" { ("r.y", "r.x") } else { ("r.x", "r.y") };
            let _ =
                writeln!(em.out, "{to} = {from}; {nz_x}({to}{});", if x8 { " as u8" } else { "" });
        }
        "TSX" => {
            em.f(1);
            em.i(1);
            if x8 {
                em.line("r.x = r.s & 0x00FF; r.nz8(r.x as u8);");
            } else {
                em.line("r.x = r.s; r.nz16(r.x);");
            }
        }
        "TXS" => {
            em.f(1);
            em.i(1);
            em.line("r.s = r.x;");
        }
        "TCS" => {
            em.f(1);
            em.i(1);
            em.line("r.s = r.a;");
        }
        "TCD" => {
            em.f(1);
            em.i(1);
            em.line("r.d = r.a; r.nz16(r.d);");
        }
        "TDC" => {
            em.f(1);
            em.i(1);
            em.line("r.a = r.d; r.nz16(r.a);");
        }
        "TSC" => {
            em.f(1);
            em.i(1);
            em.line("r.a = r.s; r.nz16(r.a);");
        }
        "XBA" => {
            em.f(1);
            em.i(2);
            em.line("r.a = r.a.swap_bytes(); r.nz8(r.a as u8);");
        }
        "PHA" | "PHX" | "PHY" => {
            em.f(1);
            em.i(1);
            em.sync();
            let (reg, wide) = match insn.mn {
                "PHA" => ("r.a", mw),
                "PHX" => ("r.x", xw),
                _ => ("r.y", xw),
            };
            em.data(if wide { 2 } else { 1 }, 8);
            if wide {
                let _ = writeln!(em.out, "r.push16(ctx, {reg});");
            } else {
                let _ = writeln!(em.out, "r.push8(ctx, {reg} as u8);");
            }
        }
        "PHB" | "PHK" | "PHP" | "PHD" => {
            em.f(1);
            em.i(1);
            em.sync();
            em.data(if insn.mn == "PHD" { 2 } else { 1 }, 8);
            match insn.mn {
                "PHB" => em.line("r.push8(ctx, r.dbr);"),
                "PHK" => {
                    let _ = writeln!(em.out, "r.push8(ctx, 0x{:02X});", insn.pc >> 16);
                }
                "PHP" => {
                    let mx = (u32::from(m8) << 5) | (u32::from(x8) << 4);
                    let _ = writeln!(em.out, "let p = r.p(0x{mx:02X}); r.push8(ctx, p);");
                }
                _ => em.line("r.push16(ctx, r.d);"),
            }
        }
        "PLA" | "PLX" | "PLY" => {
            em.f(1);
            em.i(2);
            em.sync();
            let (reg, wide) = match insn.mn {
                "PLA" => ("r.a", mw),
                "PLX" => ("r.x", xw),
                _ => ("r.y", xw),
            };
            em.data(if wide { 2 } else { 1 }, 8);
            if wide {
                let _ = writeln!(em.out, "{reg} = r.pull16(ctx); r.nz16({reg});");
            } else if insn.mn == "PLA" {
                em.line("let t = r.pull8(ctx); r.set_al(t); r.nz8(t);");
            } else {
                let _ = writeln!(em.out, "let t = r.pull8(ctx); {reg} = u16::from(t); r.nz8(t);");
            }
        }
        "PLB" => {
            em.f(1);
            em.i(2);
            em.sync();
            em.data(1, 8);
            em.line("r.dbr = r.pull8(ctx); r.nz8(r.dbr);");
        }
        "PLD" => {
            em.f(1);
            em.i(2);
            em.sync();
            em.data(2, 8);
            em.line("r.d = r.pull16(ctx); r.nz16(r.d);");
        }
        "PLP" => {
            em.f(1);
            em.i(2);
            em.sync();
            em.data(1, 8);
            em.line("let p = r.pull8(ctx); let m = r.set_p(p);");
        }
        "PEA" => {
            em.f(3);
            em.sync();
            em.data(2, 8);
            let _ = writeln!(em.out, "r.push16(ctx, 0x{:04X});", insn.opr & 0xFFFF);
        }
        "PEI" => {
            em.f(2);
            em.dyn_cy("r.dpx()", 6);
            em.sync();
            em.data(4, 8);
            let _ = writeln!(
                em.out,
                "let v = r.rd16w(ctx, r.dp(0x{:02X}, 0)); r.push16(ctx, v);",
                insn.opr & 0xFF
            );
        }
        "PER" => {
            em.f(3);
            em.i(1);
            em.sync();
            em.data(2, 8);
            let v = (insn.next() & 0xFFFF).wrapping_add(insn.opr & 0xFFFF) & 0xFFFF;
            let _ = writeln!(em.out, "r.push16(ctx, 0x{v:04X});");
        }
        "MVN" | "MVP" => {
            let dst = insn.opr & 0xFF;
            let src = (insn.opr >> 8) & 0xFF;
            let (step, mask) =
                if insn.mn == "MVN" { ("wrapping_add", "") } else { ("wrapping_sub", "") };
            let _ = mask;
            em.sync();
            let _ = writeln!(em.out, "r.dbr = 0x{dst:02X};");
            em.line("loop {");
            let iteration = F * 3 + I * 2 + 16;
            let _ = writeln!(
                em.out,
                "g!(r, ctx, {}, 1, 0x{:06X}, 0x{:02X});",
                win(iteration),
                insn.pc,
                (u32::from(m8) << 5) | (u32::from(x8) << 4)
            );
            // The cycles in their order: three fetches, the read, the write, two idle cycles.
            let _ = writeln!(em.out, "r.add({}, 3, {});", F * 3, 0b01_01_01);
            // The read of the source can land on the open bus: the last byte fetched is the
            // source bank.
            let last = (insn.pc & 0xFF_0000) | (insn.pc.wrapping_add(2) & 0xFFFF);
            let _ = writeln!(
                em.out,
                "let sa = (0x{src:02X}u32 << 16) | u32::from(r.x); if is_open(sa) {{ ctx.read8(0x{last:06X}); }} let v = r.rd8(ctx, sa);"
            );
            let _ = writeln!(em.out, "r.wr8(ctx, (0x{dst:02X}u32 << 16) | u32::from(r.y), v);");
            let _ = writeln!(em.out, "r.add({}, 2, 0);", I * 2);
            em.line("r.a = r.a.wrapping_sub(1);");
            let _ = writeln!(em.out, "r.x = r.x.{step}(1); r.y = r.y.{step}(1);");
            if x8 {
                em.line("r.x &= 0x00FF; r.y &= 0x00FF;");
            }
            em.line("if r.a == 0xFFFF { break; }");
            em.line("}");
            // Counted once in the block's cost (the iterations gate themselves).
            em.cost += iteration;
        }
        other => unreachable!("not a plain instruction: {other}"),
    }
}

struct FnEmit<'a> {
    g: &'a Graph,
    ids: HashMap<SKey, u32>,
    leaders: HashSet<SKey>,
    long_return: bool,
    bails: BTreeMap<&'static str, u32>,
    /// The worst-case cost of each block, by id.
    costs: HashMap<u32, u32>,
    /// The index of the function's first block in the generated `COV` table.
    cov_base: u32,
}

impl FnEmit<'_> {
    fn id(&self, s: SKey) -> u32 {
        self.ids[&s]
    }

    fn mxbits(m8: bool, x8: bool) -> u8 {
        (u8::from(m8) << 5) | (u8::from(x8) << 4)
    }

    /// The block starting at `start`: its text.
    #[allow(clippy::too_many_lines)]
    fn block(&mut self, start: SKey, out: &mut String, entry_cost: &mut Option<u32>) -> usize {
        let mut em = Em::new();
        let mut cur = start;
        let mut count = 0_usize;
        let mut last: Option<&Node>;
        loop {
            let node = &self.g.nodes[&cur];
            last = Some(node);
            match &node.next {
                Next::Seq(n) if !self.leaders.contains(n) && !splits_after(&node.insn) => {
                    plain(&mut em, &node.insn);
                    count += 1;
                    cur = *n;
                }
                _ => break,
            }
        }
        let node = last.expect("a block has a node");
        let insn = &node.insn;
        let mx = Self::mxbits(insn.m8, insn.x8);
        let ret_mx = format!("0x{mx:02X}");
        let bank = insn.pc >> 16;
        let next_pc = insn.next();
        count += 1;
        match &node.next {
            Next::Seq(n) => {
                plain(&mut em, insn);
                em.sync();
                if insn.mn == "PLP" {
                    // The widths the pull was expected to bring back; anything else hands over.
                    let _ = writeln!(
                        em.out,
                        "if m != 0x{:02X} {{ r.mx = m; return r.bail_dyn(0x{next_pc:06X}); }}",
                        n.mx
                    );
                }
                let _ = writeln!(em.out, "b = {}; continue;", self.id(*n));
            }
            Next::Branch { taken, fall } => {
                em.f(2);
                em.cost += I;
                em.sync();
                let _ = writeln!(
                    em.out,
                    "if {} {{ r.add(6, 1, 0); b = {}; }} else {{ b = {}; }} continue;",
                    branch_cond(insn.mn),
                    self.id(*taken),
                    self.id(*fall)
                );
            }
            Next::Jump(n) => {
                match insn.mn {
                    "BRA" => {
                        em.f(2);
                        em.i(1);
                    }
                    "BRL" => {
                        em.f(3);
                        em.i(1);
                    }
                    "JMP" => em.f(3),
                    _ => em.f(4),
                }
                em.sync();
                let _ = writeln!(em.out, "b = {}; continue;", self.id(*n));
            }
            Next::Tail(callee) => {
                em.sync();
                match insn.mn {
                    "BRA" => {
                        em.f(2);
                        em.i(1);
                    }
                    "BRL" => {
                        em.f(3);
                        em.i(1);
                    }
                    "JMP" => em.f(3),
                    _ => em.f(4),
                }
                // The depth check precedes the instruction's cycles.
                let (cycles, k, h) = em.take();
                let _ = writeln!(
                    em.out,
                    "if r.depth >= MAX_DEPTH {{ return r.bail(0x{:06X}, {ret_mx}); }}",
                    insn.pc
                );
                let _ = writeln!(
                    em.out,
                    "r.add({cycles}, {k}, {h}); r.depth += 1; let ret = {}(ctx, r); r.depth -= 1; return ret;",
                    callee.name()
                );
            }
            Next::Call { callee, long, ret, conts } => {
                em.sync();
                let _ = writeln!(
                    em.out,
                    "if r.depth >= MAX_DEPTH {{ return r.bail(0x{:06X}, {ret_mx}); }}",
                    insn.pc
                );
                let expect = if *long { (bank << 16) | ret } else { *ret };
                match callee {
                    Callee::Direct(_) if !*long => {
                        em.f(3);
                        em.i(1);
                        em.sync();
                        em.data(2, 8);
                        let _ = writeln!(em.out, "r.push16(ctx, 0x{ret:04X});");
                    }
                    Callee::Direct(_) => {
                        em.f(4);
                        em.i(1);
                        em.sync();
                        em.data(3, 8);
                        let _ = writeln!(
                            em.out,
                            "r.push8(ctx, 0x{bank:02X}); r.push16(ctx, 0x{ret:04X});"
                        );
                    }
                    Callee::Table(_) => {
                        em.f(3);
                        em.i(1);
                        em.sync();
                        em.data(4, 8);
                        let _ = writeln!(em.out, "r.push16(ctx, 0x{ret:04X});");
                    }
                }
                match callee {
                    Callee::Direct(k) => {
                        let _ = writeln!(
                            em.out,
                            "r.depth += 1; let ret = {}(ctx, r); r.depth -= 1;",
                            k.name()
                        );
                    }
                    Callee::Table(table) => {
                        let abs = insn.opr & 0xFFFF;
                        let _ = writeln!(
                            em.out,
                            "let t = r.rd16w(ctx, (0x{bank:02X}u32 << 16) | u32::from(0x{abs:04X}u16.wrapping_add(r.x)));"
                        );
                        em.line("r.depth += 1;");
                        em.line("let ret = match t {");
                        for (t, k) in table {
                            let _ =
                                writeln!(em.out, "0x{:04X} => {}(ctx, r),", t & 0xFFFF, k.name());
                        }
                        let _ = writeln!(
                            em.out,
                            "_ => {{ r.depth -= 1; return r.bail(0x{bank:02X}0000 | u32::from(t), {ret_mx}); }} }};"
                        );
                        em.line("r.depth -= 1;");
                    }
                }
                let _ = writeln!(
                    em.out,
                    "if ret != 0x{expect:X} {{ return if ret == BAIL {{ BAIL }} else {{ r.bail_ret(ret, r.rl, 0x{bank:02X}) }}; }}"
                );
                em.line("match r.mx {");
                for (cmx, n) in conts {
                    let _ = writeln!(em.out, "0x{cmx:02X} => {{ b = {}; continue; }}", self.id(*n));
                }
                let _ = writeln!(em.out, "_ => return r.bail_dyn(0x{next_pc:06X}),");
                em.line("}");
            }
            Next::Return { long } => {
                if *long != self.long_return {
                    em.sync();
                    *self.bails.entry("return of the other kind").or_default() += 1;
                    let _ = writeln!(em.out, "return r.bail(0x{:06X}, {ret_mx});", insn.pc);
                } else if *long {
                    em.f(1);
                    em.i(2);
                    em.sync();
                    em.data(3, 8);
                    em.line("let lo = r.pull16r(ctx); let bk = r.pull8r(ctx); r.rl = true;");
                    let _ = writeln!(
                        em.out,
                        "r.mx = {ret_mx}; return u32::from(lo) | (u32::from(bk) << 16);"
                    );
                } else {
                    em.f(1);
                    em.i(2);
                    em.sync();
                    em.data(2, 8);
                    em.line("let pc = r.pull16r(ctx); r.rl = false;");
                    em.i(1);
                    em.sync();
                    let _ = writeln!(em.out, "r.mx = {ret_mx}; return u32::from(pc);");
                }
            }
            Next::Rti => {
                // Native mode: opcode, two internal cycles, then P, the address and the bank.
                em.f(1);
                em.i(2);
                em.sync();
                em.data(4, 8);
                em.line("let p = r.pull8r(ctx); let lo = r.pull16r(ctx); let bk = r.pull8r(ctx);");
                em.line("let m = r.set_p(p);");
                em.line("return r.bail(u32::from(lo) | (u32::from(bk) << 16), m);");
            }
            Next::Bail { .. } => {
                em.sync();
                let _ = writeln!(em.out, "return r.bail(0x{:06X}, {ret_mx});", insn.pc);
            }
            Next::Switch { conts } => {
                plain(&mut em, insn);
                em.sync();
                em.line("match m {");
                for (cmx, n) in conts {
                    let _ = writeln!(
                        em.out,
                        "0x{cmx:02X} => {{ r.mx = m; b = {}; continue; }}",
                        self.id(*n)
                    );
                }
                let _ =
                    writeln!(em.out, "_ => {{ r.mx = m; return r.bail_dyn(0x{next_pc:06X}); }}");
                em.line("}");
            }
            Next::IndJump { targets } => {
                let abs = insn.opr & 0xFFFF;
                match insn.mode {
                    Mode::AbsIndX => {
                        em.f(3);
                        em.i(1);
                        em.sync();
                        em.data(2, 8);
                        let _ = writeln!(
                            em.out,
                            "let t = u32::from(r.rd16w(ctx, (0x{bank:02X}u32 << 16) | u32::from(0x{abs:04X}u16.wrapping_add(r.x))));"
                        );
                    }
                    Mode::AbsInd => {
                        em.f(3);
                        em.sync();
                        em.data(2, 8);
                        let _ = writeln!(em.out, "let t = u32::from(r.rd16w(ctx, 0x{abs:04X}));");
                    }
                    _ => {
                        em.f(3);
                        em.sync();
                        em.data(3, 8);
                        let _ = writeln!(em.out, "let t = r.rd24w(ctx, 0x{abs:04X});");
                    }
                }
                let pbr_here = format!("0x{bank:02X}0000");
                em.line("match t {");
                for (t, to) in targets {
                    let key =
                        if insn.mode == Mode::AbsIndLong { *t & 0xFF_FFFF } else { *t & 0xFFFF };
                    match to {
                        JumpTo::State(s) => {
                            let _ = writeln!(
                                em.out,
                                "0x{key:X} => {{ b = {}; continue; }}",
                                self.id(*s)
                            );
                        }
                        JumpTo::Func(k) => {
                            let _ = writeln!(
                                em.out,
                                "0x{key:X} => {{ if r.depth >= MAX_DEPTH {{ return r.bail({pbr_here} | t, {ret_mx}); }} r.depth += 1; let ret = {}(ctx, r); r.depth -= 1; return ret; }}",
                                k.name()
                            );
                        }
                    }
                }
                if insn.mode == Mode::AbsIndLong {
                    let _ = writeln!(em.out, "_ => return r.bail(t, {ret_mx}),");
                } else {
                    let _ = writeln!(em.out, "_ => return r.bail({pbr_here} | t, {ret_mx}),");
                }
                em.line("}");
            }
        }
        em.sync();
        let cost = em.cost;
        if entry_cost.is_none() {
            *entry_cost = Some(cost);
        }
        let id = self.id(start);
        self.costs.insert(id, cost);
        let _ = writeln!(out, "{id} => {{");
        if self.g.nodes[&start].sync {
            // The machine reports a sync event at the first arrival after an NMI: leave it to
            // the original.
            let _ = writeln!(
                out,
                "if !r.nogate && ctx.sync_due(0x{:06X}) {{ return r.bail(0x{:06X}, 0x{:02X}); }}",
                start.pc, start.pc, start.mx
            );
        }
        let _ = writeln!(
            out,
            "g!(r, ctx, {}, {count}, 0x{:06X}, 0x{:02X});",
            win(cost),
            start.pc,
            start.mx
        );
        let _ = writeln!(out, "cv!({});", self.cov_base + id);
        out.push_str(&em.out);
        out.push_str("}\n");
        count
    }
}

/// Generates the Rust function for a viable graph.
///
/// # Panics
///
/// If `g` has no entry.
#[must_use]
pub fn emit_function(key: FuncKey, g: &Graph, long_return: bool, cov_base: u32) -> Emitted {
    let entry = g.entry.expect("a viable graph has an entry");
    // Leaders.
    let mut in_degree: HashMap<SKey, u32> = HashMap::new();
    let mut leaders: HashSet<SKey> = HashSet::new();
    leaders.insert(entry);
    for node in g.nodes.values() {
        let mut edge = |s: SKey, always: bool| {
            *in_degree.entry(s).or_default() += 1;
            if always {
                leaders.insert(s);
            }
        };
        match &node.next {
            Next::Seq(n) => edge(*n, splits_after(&node.insn)),
            Next::Branch { taken, fall } => {
                edge(*taken, true);
                edge(*fall, true);
            }
            Next::Jump(n) => edge(*n, true),
            Next::Call { conts, .. } | Next::Switch { conts } => {
                for (_, n) in conts {
                    edge(*n, true);
                }
            }
            Next::IndJump { targets } => {
                for (_, t) in targets {
                    if let JumpTo::State(n) = t {
                        edge(*n, true);
                    }
                }
            }
            Next::Bail { conts, .. } => {
                for n in conts {
                    edge(*n, true);
                }
            }
            Next::Tail(_) | Next::Return { .. } | Next::Rti => {}
        }
    }
    for (s, d) in &in_degree {
        if *d >= 2 {
            leaders.insert(*s);
        }
    }
    // Long stretches are cut into blocks of about `BLOCK_CAP` master cycles, so that a timing
    // event near the code keeps the original from running more than a few instructions.
    let mut starts: Vec<SKey> = leaders.iter().copied().collect();
    starts.sort();
    for start in starts {
        let mut cur = start;
        let mut cost = 0;
        while let Next::Seq(n) = &g.nodes[&cur].next {
            let insn = &g.nodes[&cur].insn;
            if leaders.contains(n) || splits_after(insn) {
                break;
            }
            let mut scratch = Em::new();
            plain(&mut scratch, insn);
            cost += scratch.cost;
            if cost >= BLOCK_CAP {
                leaders.insert(*n);
                cost = 0;
            }
            cur = *n;
        }
    }
    let mut order: Vec<SKey> = leaders.iter().copied().filter(|s| *s != entry).collect();
    order.sort();
    order.insert(0, entry);
    let ids: HashMap<SKey, u32> = order.iter().enumerate().map(|(i, s)| (*s, i as u32)).collect();
    let mut fe = FnEmit {
        g,
        ids,
        leaders,
        long_return,
        bails: BTreeMap::new(),
        costs: HashMap::new(),
        cov_base,
    };
    let mut body = String::new();
    let mut entry_cost = None;
    let mut instructions = 0;
    for s in &order {
        instructions += fe.block(*s, &mut body, &mut entry_cost);
    }
    let mut source = String::new();
    let _ = writeln!(source, "pub fn {}(ctx: &mut Ctx<'_>, r: &mut R) -> u32 {{", key.name());
    source.push_str("let mut b: u32 = std::mem::take(&mut r.start);\nloop {\nmatch b {\n");
    source.push_str(&body);
    source.push_str("_ => return r.bail_dyn(0),\n}\n}\n}\n");
    // Where the original can be met again: every block. (After a store that starts a GPDMA the
    // machine declines the arrival: it does not run a routine with a transfer requested.)
    let entries: Vec<Leader> = order
        .iter()
        .map(|s| Leader { pc: s.pc, mx: s.mx, block: fe.id(*s), cost: fe.costs[&fe.id(*s)] })
        .collect();
    Emitted {
        source,
        leaders: entries,
        entry_cost: entry_cost.unwrap_or(0),
        instructions,
        long_return,
        bails: fe.bails,
        blocks: order.len() as u32,
    }
}
