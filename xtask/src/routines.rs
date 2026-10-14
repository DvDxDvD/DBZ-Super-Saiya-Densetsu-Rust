//! `routines`: the routine table built from the Mesen traces of `.work/re/<run>/` (see
//! `tools/re/README.md`), and the ranking of port candidates.
//!
//! Entries are the targets of `calls.csv` and `jumps.csv`, the interrupt vectors and the routines
//! of `re/symbols.toml` that ran. A routine's body is the set of executed instructions reachable
//! from its entry without leaving it (`JSR`/`JSL` return into it, a `JMP` to another entry is a
//! tail call), decoded with the widths the trace saw. So a body holds the paths the traces took,
//! not every path of the code. The flags come from decoding that body: they are exact for what
//! ran, silent about what did not. Output goes to `.work/re/` (never tracked).

use crate::disasm::{Instr, Mode, Region, RomImage, Widths, data_operand, decode, region};
use crate::shared::render_table;
use crate::trace::{Edge, Run, phys};
use ssd_oracle::Symbols;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

/// Longest body that is still ranked as a candidate, in bytes.
const MAX_CANDIDATE_SIZE: u32 = 256;

/// What the body of a routine does that matters for porting it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Flags {
    /// Reads or writes a hardware register (`$2100-$43FF`).
    pub mmio: bool,
    /// Starts DMA or HDMA, or writes a DMA channel register.
    pub dma: bool,
    /// Switches or reads the stack pointer, or pulls more than it pushes (or the reverse).
    pub stack: bool,
    /// An indirect `JMP`/`JML`, or a `JSR (abs,X)`.
    pub indirect: bool,
    /// A jump to another routine's entry.
    pub tail: bool,
    /// Block moves (`MVN`, `MVP`), `WAI`, `STP`.
    pub special: bool,
    /// Reads or writes through a pointer (`(dp)`, `[dp]`, `(dp),Y`, `(sr,S),Y`).
    pub pointer: bool,
    /// Both `RTS` and `RTL` (or a return with an `RTI`).
    pub mixed_return: bool,
    /// A branch or jump back to the entry (so the entry's hits count loops too).
    pub loops_to_entry: bool,
    /// The widths or the mode differ between samples at the entry.
    pub entry_ambiguous: bool,
}

/// One routine of the table.
#[derive(Debug, Clone)]
pub struct Routine {
    pub entry: u32,
    /// Its name in `re/symbols.toml`, if it is already known.
    pub known: Option<String>,
    /// How it was found: `call`, `jump`, `vector`, `symbol`.
    pub found_by: BTreeSet<&'static str>,
    /// Bytes of the executed instructions of the body.
    pub size: u32,
    /// Executed instructions of the body (distinct addresses).
    pub instrs: u32,
    /// Instructions the body executed in the traces, summed over runs.
    pub hits: u64,
    /// Calls per run (same order as the runs).
    pub calls_by_run: Vec<u64>,
    pub callees: BTreeSet<u32>,
    pub callers: BTreeSet<u32>,
    pub leaf: bool,
    /// `rts`, `rtl`, `rti`, joined by `/`; empty if no return executed.
    pub returns: String,
    pub entry_m8: Option<bool>,
    pub entry_x8: Option<bool>,
    pub emulation_mode: bool,
    pub branches: u32,
    pub flags: Flags,
    /// WRAM bytes read and written directly (no pointer, no index), as `start-end` hex ranges,
    /// then bases of indexed accesses as `start+idx`.
    pub wram_read: Vec<String>,
    pub wram_write: Vec<String>,
    /// Whether the code is in ROM (else it ran from RAM and has no body).
    pub in_rom: bool,
}

impl Routine {
    #[must_use]
    pub fn calls(&self) -> u64 {
        self.calls_by_run.iter().sum()
    }

    /// Why this routine is not a port candidate; `None` if it is one.
    #[must_use]
    pub fn excluded(&self) -> Option<&'static str> {
        let f = &self.flags;
        if self.known.is_some() {
            Some("known")
        } else if !self.in_rom || self.size == 0 {
            Some("not ROM code")
        } else if self.emulation_mode || f.entry_ambiguous {
            Some("mode")
        } else if self.calls() == 0 {
            Some("never called")
        } else if !self.leaf {
            Some("calls")
        } else if f.tail || f.indirect {
            Some("jumps out")
        } else if f.mmio || f.dma {
            Some("mmio")
        } else if f.stack || f.special || f.mixed_return {
            Some("stack/special")
        } else if !(self.returns == "rts" || self.returns == "rtl") {
            Some("no plain return")
        } else if self.size > MAX_CANDIDATE_SIZE {
            Some("large")
        } else {
            None
        }
    }

    /// Instructions run per byte of code, weighed: hot and small first.
    #[must_use]
    pub fn score(&self) -> f64 {
        self.hits as f64 / f64::from(self.size.max(1)).sqrt()
    }
}

/// The native interrupt vectors and the reset vector: `(name, address of the vector)`.
const VECTORS: [(&str, u32); 6] = [
    ("cop", 0x00_FFE4),
    ("brk", 0x00_FFE6),
    ("nmi", 0x00_FFEA),
    ("irq", 0x00_FFEE),
    ("reset", 0x00_FFFC),
    ("abort", 0x00_FFE8),
];

fn vector_entries(rom: &RomImage<'_>) -> Vec<u32> {
    VECTORS
        .iter()
        .filter_map(|&(_, at)| {
            let lo = rom.read(at)?;
            let hi = rom.read(at + 1)?;
            Some(u32::from(u16::from_le_bytes([lo, hi])))
        })
        .filter(|&pc| pc >= 0x8000)
        .collect()
}

/// The accumulator/index width an instruction's memory access uses.
fn access_size(instr: &Instr, w: Widths) -> u32 {
    match instr.mnemonic {
        "LDX" | "LDY" | "STX" | "STY" | "CPX" | "CPY" => {
            if w.x8 {
                1
            } else {
                2
            }
        }
        "LDA" | "STA" | "ADC" | "SBC" | "AND" | "ORA" | "EOR" | "CMP" | "BIT" | "INC" | "DEC"
        | "ASL" | "LSR" | "ROL" | "ROR" | "TSB" | "TRB" | "STZ" => {
            if w.m8 {
                1
            } else {
                2
            }
        }
        _ => 1,
    }
}

/// `(reads, writes)` of a memory instruction.
fn access_kind(mnemonic: &str) -> (bool, bool) {
    match mnemonic {
        "LDA" | "LDX" | "LDY" | "ADC" | "SBC" | "AND" | "ORA" | "EOR" | "CMP" | "CPX" | "CPY"
        | "BIT" => (true, false),
        "STA" | "STX" | "STY" | "STZ" => (false, true),
        "INC" | "DEC" | "ASL" | "LSR" | "ROL" | "ROR" | "TSB" | "TRB" => (true, true),
        _ => (false, false),
    }
}

/// Merged hex ranges of direct accesses, then the bases of indexed ones.
fn format_ranges(direct: &BTreeSet<(u32, u32)>, indexed: &BTreeSet<u32>) -> Vec<String> {
    let mut merged: Vec<(u32, u32)> = Vec::new();
    for &(start, end) in direct {
        match merged.last_mut() {
            Some(last) if start <= last.1 + 1 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    let mut out: Vec<String> = merged
        .into_iter()
        .map(|(a, b)| if a == b { format!("{a:06X}") } else { format!("{a:06X}-{b:06X}") })
        .collect();
    out.extend(indexed.iter().map(|base| format!("{base:06X}+idx")));
    out
}

/// What the traces say about an address, merged over the runs.
struct Seen {
    hits: u64,
    m8: bool,
    x8: bool,
    e: bool,
    d: u16,
    dbrs: Vec<u8>,
    modes: Vec<(bool, bool, bool)>,
}

fn merge_runs(runs: &[Run]) -> BTreeMap<u32, Seen> {
    let mut seen: BTreeMap<u32, Seen> = BTreeMap::new();
    for run in runs {
        for (&pc, row) in &run.exec {
            let entry = seen.entry(pc).or_insert_with(|| Seen {
                hits: 0,
                m8: row.m8,
                x8: row.x8,
                e: row.e,
                d: row.d,
                dbrs: Vec::new(),
                modes: Vec::new(),
            });
            entry.hits += row.hits;
            entry.dbrs.push(row.dbr);
            entry.dbrs.extend(&row.dbrs);
            entry.modes.extend(&row.modes);
        }
    }
    for s in seen.values_mut() {
        s.dbrs.sort_unstable();
        s.dbrs.dedup();
        s.modes.sort_unstable();
        s.modes.dedup();
    }
    seen
}

/// Builds the routine table.
#[must_use]
pub fn build(runs: &[Run], rom: &RomImage<'_>, symbols: &Symbols) -> Vec<Routine> {
    let seen = merge_runs(runs);

    // Entries.
    let mut found: BTreeMap<u32, BTreeSet<&'static str>> = BTreeMap::new();
    let mut add = |pc: u32, how: &'static str| {
        let pc = phys(pc);
        if seen.contains_key(&pc) {
            found.entry(pc).or_default().insert(how);
        }
    };
    for run in runs {
        for e in &run.calls {
            add(e.target, "call");
        }
        for e in &run.jumps {
            add(e.target, "jump");
        }
    }
    for pc in vector_entries(rom) {
        add(pc, "vector");
    }
    for r in symbols.routines() {
        add(r.address, "symbol");
    }
    let entries: BTreeSet<u32> = found.keys().copied().collect();

    // Edges by source address.
    let mut call_edges: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    let mut jump_edges: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    let add_edge = |map: &mut BTreeMap<u32, BTreeSet<u32>>, e: &Edge| {
        map.entry(e.source).or_default().insert(e.target);
    };
    for run in runs {
        run.calls.iter().for_each(|e| add_edge(&mut call_edges, e));
        run.jumps.iter().for_each(|e| add_edge(&mut jump_edges, e));
    }

    let mut routines: Vec<Routine> = Vec::new();
    let mut bodies: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    for (&entry, how) in &found {
        let (routine, body) =
            analyse(entry, how, runs, &seen, rom, symbols, &entries, &call_edges, &jump_edges);
        bodies.insert(entry, body);
        routines.push(routine);
    }

    // Callers: the routines whose body holds the source of a call to the entry.
    let mut owners: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for (&entry, body) in &bodies {
        for &pc in body {
            owners.entry(pc).or_default().push(entry);
        }
    }
    for routine in &mut routines {
        for run in runs {
            for e in run.calls.iter().filter(|e| e.target == routine.entry) {
                if let Some(list) = owners.get(&e.source) {
                    routine.callers.extend(list.iter().copied());
                }
            }
        }
    }
    routines
}

#[allow(clippy::too_many_arguments)]
fn analyse(
    entry: u32,
    how: &BTreeSet<&'static str>,
    runs: &[Run],
    seen: &BTreeMap<u32, Seen>,
    rom: &RomImage<'_>,
    symbols: &Symbols,
    entries: &BTreeSet<u32>,
    call_edges: &BTreeMap<u32, BTreeSet<u32>>,
    jump_edges: &BTreeMap<u32, BTreeSet<u32>>,
) -> (Routine, BTreeSet<u32>) {
    let at_entry = &seen[&entry];
    let mut routine = Routine {
        entry,
        known: symbols.routines().iter().find(|r| r.address == entry).map(|r| r.name.clone()),
        found_by: how.clone(),
        size: 0,
        instrs: 0,
        hits: 0,
        calls_by_run: Vec::new(),
        callees: BTreeSet::new(),
        callers: BTreeSet::new(),
        leaf: true,
        returns: String::new(),
        entry_m8: Some(at_entry.m8),
        entry_x8: Some(at_entry.x8),
        emulation_mode: at_entry.e,
        branches: 0,
        flags: Flags { entry_ambiguous: at_entry.modes.len() > 1, ..Flags::default() },
        wram_read: Vec::new(),
        wram_write: Vec::new(),
        in_rom: rom.read(entry).is_some(),
    };
    let mut body: BTreeMap<u32, Instr> = BTreeMap::new();
    let mut work = vec![entry];
    let mut returns: BTreeSet<&'static str> = BTreeSet::new();
    let (mut pushes, mut pulls) = (0_i32, 0_i32);
    while let Some(pc) = work.pop() {
        if body.contains_key(&pc) {
            continue;
        }
        let Some(s) = seen.get(&pc) else { continue };
        if pc != entry && entries.contains(&pc) {
            // Control falls into or jumps to another routine's entry: a tail call.
            routine.flags.tail = true;
            routine.callees.insert(pc);
            continue;
        }
        let w = if s.e {
            Widths { m8: true, x8: true, m_known: true, x_known: true }
        } else {
            Widths { m8: s.m8, x8: s.x8, m_known: true, x_known: true }
        };
        let Some(instr) = decode(rom, pc, w) else { continue };
        let next = phys(instr.next());
        let target = instr.target().map(phys);
        match instr.mnemonic {
            "PHA" | "PHX" | "PHY" | "PHP" | "PHB" | "PHD" | "PHK" | "PEA" | "PEI" | "PER" => {
                pushes += 1;
            }
            "PLA" | "PLX" | "PLY" | "PLP" | "PLB" | "PLD" => pulls += 1,
            "TCS" | "TXS" | "TSC" | "TSX" => routine.flags.stack = true,
            "MVN" | "MVP" | "WAI" | "STP" => routine.flags.special = true,
            _ => {}
        }
        if matches!(
            instr.mode,
            Mode::DpInd | Mode::DpIndY | Mode::DpIndLong | Mode::DpIndLongY | Mode::StackIndY
        ) && instr.mnemonic != "PEI"
        {
            routine.flags.pointer = true;
        }
        if instr.is_call() {
            routine.leaf = false;
            let mut targets: BTreeSet<u32> = call_edges.get(&pc).cloned().unwrap_or_default();
            targets.extend(target);
            routine.callees.extend(targets);
            if instr.mode == Mode::AbsIndX {
                routine.flags.indirect = true;
            }
            work.push(next);
        } else if instr.is_return() {
            returns.insert(match instr.mnemonic {
                "RTS" => "rts",
                "RTL" => "rtl",
                _ => "rti",
            });
        } else if instr.is_indirect_jump() {
            routine.flags.indirect = true;
            routine.callees.extend(jump_edges.get(&pc).into_iter().flatten().copied());
        } else if instr.is_unconditional_jump() {
            if let Some(t) = target {
                if t == entry {
                    routine.flags.loops_to_entry = true;
                }
                routine.branches += 1;
                work.push(t);
            }
        } else {
            if instr.is_conditional_branch() {
                routine.branches += 1;
                if let Some(t) = target {
                    if t == entry {
                        routine.flags.loops_to_entry = true;
                    }
                    work.push(t);
                }
            }
            if instr.mnemonic != "STP" {
                work.push(next);
            }
        }
        body.insert(pc, instr);
    }

    // Sizes, hits, memory.
    let (mut direct_r, mut direct_w) = (BTreeSet::new(), BTreeSet::new());
    let (mut idx_r, mut idx_w) = (BTreeSet::new(), BTreeSet::new());
    let mut return_hits_by_run = vec![0_u64; runs.len()];
    for (&pc, instr) in &body {
        let s = &seen[&pc];
        routine.size += u32::from(instr.len);
        routine.instrs += 1;
        routine.hits += s.hits;
        let w = Widths { m8: s.m8, x8: s.x8, m_known: true, x_known: true };
        if instr.is_return() {
            for (i, run) in runs.iter().enumerate() {
                return_hits_by_run[i] += run.exec.get(&pc).map_or(0, |r| r.hits);
            }
        }
        let dbrs: Vec<u8> = if s.dbrs.is_empty() { vec![0] } else { s.dbrs.clone() };
        for &dbr in &dbrs {
            let Some(op) = data_operand(instr, dbr, s.d) else { continue };
            match region(op.bus) {
                Region::Mmio(register) => {
                    routine.flags.mmio = true;
                    if matches!(register, 0x420B | 0x420C | 0x4300..=0x437F) {
                        routine.flags.dma = true;
                    }
                }
                Region::Wram(offset) => {
                    let (reads, writes) = access_kind(instr.mnemonic);
                    let start = 0x7E_0000 + offset;
                    let end = start + access_size(instr, w) - 1;
                    for (on, direct, indexed) in
                        [(reads, &mut direct_r, &mut idx_r), (writes, &mut direct_w, &mut idx_w)]
                    {
                        if on {
                            if op.indexed {
                                indexed.insert(start);
                            } else {
                                direct.insert((start, end));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    routine.wram_read = format_ranges(&direct_r, &idx_r);
    routine.wram_write = format_ranges(&direct_w, &idx_w);
    routine.flags.stack |= pushes != pulls;
    routine.returns = returns.iter().copied().collect::<Vec<_>>().join("/");
    routine.flags.mixed_return = returns.len() > 1;
    routine.callees.remove(&entry);

    // Calls per run: the entry's hits, or its returns when the body loops back to the entry.
    routine.calls_by_run = runs
        .iter()
        .enumerate()
        .map(|(i, run)| {
            let entered = run.exec.get(&entry).map_or(0, |r| r.hits);
            if routine.flags.loops_to_entry && return_hits_by_run[i] > 0 {
                return_hits_by_run[i]
            } else {
                entered
            }
        })
        .collect();
    let body_pcs = body.keys().copied().collect();
    (routine, body_pcs)
}

/// The candidates ranked: hot, small, leaf, pure routines first. Excluded routines are left out.
#[must_use]
pub fn rank(routines: &[Routine]) -> Vec<&Routine> {
    let mut ranked: Vec<&Routine> = routines.iter().filter(|r| r.excluded().is_none()).collect();
    ranked.sort_by(|a, b| b.score().total_cmp(&a.score()).then(a.entry.cmp(&b.entry)));
    ranked
}

fn flag_string(r: &Routine) -> String {
    let f = &r.flags;
    let mut out = String::new();
    for (on, letter) in [
        (r.leaf, 'L'),
        (f.mmio, 'M'),
        (f.dma, 'D'),
        (f.stack, 'S'),
        (f.indirect, 'I'),
        (f.tail, 'T'),
        (f.pointer, 'P'),
        (f.special, 'X'),
        (f.mixed_return, 'R'),
    ] {
        out.push(if on { letter } else { '-' });
    }
    out
}

/// The first `n` items joined, then how many more there are.
fn short_list(items: &[String], n: usize) -> String {
    let mut text = items.iter().take(n).cloned().collect::<Vec<_>>().join(";");
    if items.len() > n {
        let _ = write!(text, ";+{}", items.len() - n);
    }
    text
}

fn hex_list(entries: &BTreeSet<u32>) -> String {
    entries.iter().map(|e| format!("{e:06X}")).collect::<Vec<_>>().join(";")
}

/// The CSV of the whole table.
#[must_use]
pub fn csv(routines: &[Routine]) -> String {
    let ranked = rank(routines);
    let rank_of = |entry: u32| ranked.iter().position(|r| r.entry == entry).map(|p| p + 1);
    let mut out = String::from(
        "entry,name,found_by,size,instrs,hits,calls,calls_by_run,callers,callees,leaf,returns,\
         entry_m8,entry_x8,emulation,mmio,dma,stack,indirect,tail,pointer,special,branches,\
         wram_read,wram_write,excluded,score,rank\n",
    );
    let calls_by_run =
        |r: &Routine| r.calls_by_run.iter().map(u64::to_string).collect::<Vec<_>>().join(";");
    let bit = |b: bool| u8::from(b);
    for r in routines {
        let f = &r.flags;
        let _ = writeln!(
            out,
            "{:06X},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{:.1},{}",
            r.entry,
            r.known.as_deref().unwrap_or(""),
            r.found_by.iter().copied().collect::<Vec<_>>().join("+"),
            r.size,
            r.instrs,
            r.hits,
            r.calls(),
            calls_by_run(r),
            hex_list(&r.callers),
            hex_list(&r.callees),
            bit(r.leaf),
            r.returns,
            r.entry_m8.map_or("", |m| if m { "1" } else { "0" }),
            r.entry_x8.map_or("", |x| if x { "1" } else { "0" }),
            bit(r.emulation_mode),
            bit(f.mmio),
            bit(f.dma),
            bit(f.stack),
            bit(f.indirect),
            bit(f.tail),
            bit(f.pointer),
            bit(f.special),
            r.branches,
            r.wram_read.join(";"),
            r.wram_write.join(";"),
            r.excluded().unwrap_or(""),
            r.score(),
            rank_of(r.entry).map_or_else(String::new, |n| n.to_string()),
        );
    }
    out
}

/// The text summary: counts, the legend and the top `top` candidates.
#[must_use]
pub fn summary(routines: &[Routine], runs: &[Run], top: usize) -> String {
    let ranked = rank(routines);
    let mut out = String::new();
    let _ = writeln!(
        out,
        "routines: {} from {} run(s) [{}] ({} frames); known {}; leaf {}; candidates {}",
        routines.len(),
        runs.len(),
        runs.iter().map(|r| r.name.as_str()).collect::<Vec<_>>().join(", "),
        runs.iter().map(|r| r.frames).sum::<u64>(),
        routines.iter().filter(|r| r.known.is_some()).count(),
        routines.iter().filter(|r| r.leaf).count(),
        ranked.len()
    );
    let mut excluded: BTreeMap<&str, usize> = BTreeMap::new();
    for r in routines {
        if let Some(why) = r.excluded() {
            *excluded.entry(why).or_default() += 1;
        }
    }
    let why: Vec<String> = excluded.iter().map(|(k, v)| format!("{k} {v}")).collect();
    let _ = writeln!(out, "excluded: {}", why.join(", "));
    let _ = writeln!(
        out,
        "flags: L leaf, M mmio, D dma, S stack trick, I indirect jump, T tail call, P pointer, X block move/wait, R mixed returns"
    );
    let _ = writeln!(
        out,
        "score = instructions run / sqrt(bytes); calls = entry hits (returns if the body loops to its entry)\n"
    );
    let rows: Vec<Vec<String>> = ranked
        .iter()
        .take(top)
        .enumerate()
        .map(|(i, r)| {
            vec![
                (i + 1).to_string(),
                format!("{:06X}", r.entry),
                r.size.to_string(),
                r.calls().to_string(),
                r.hits.to_string(),
                r.callers.len().to_string(),
                r.returns.clone(),
                flag_string(r),
                short_list(&r.wram_write, 3),
            ]
        })
        .collect();
    out.push_str(&render_table(
        &[
            "#",
            "entry",
            "size",
            "calls",
            "instrs",
            "callers",
            "ret",
            "flags",
            "wram_write (first 3)",
        ],
        &rows,
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace::Exec;

    /// Marks `pcs` as executed `hits` times in native mode M8 X16, DBR 0.
    fn exec(pcs: &[(u32, u64)]) -> BTreeMap<u32, Exec> {
        pcs.iter()
            .map(|&(pc, hits)| {
                let row = Exec {
                    m8: true,
                    x8: false,
                    e: false,
                    d: 0,
                    dbr: 0,
                    hits,
                    modes: vec![(true, false, false)],
                    dbrs: vec![0],
                };
                (pc, row)
            })
            .collect()
    }

    fn edge(target: u32, source: u32, op: &str) -> Edge {
        Edge { target, source, op: op.to_owned() }
    }

    /// A 32 KiB ROM with `code` at `$00:8000`.
    fn rom_with(code: &[(u32, &[u8])]) -> Vec<u8> {
        let mut rom = vec![0xEA_u8; 0x8000];
        for (at, bytes) in code {
            let o = (*at - 0x8000) as usize;
            rom[o..o + bytes.len()].copy_from_slice(bytes);
        }
        rom
    }

    fn table() -> (Vec<Routine>, Vec<Run>) {
        let rom = rom_with(&[
            // 8000: caller: JSR $8010 ; JSR $8020 ; JSR $8030 ; JSR $8040 ; RTS
            (
                0x8000,
                &[0x20, 0x10, 0x80, 0x20, 0x20, 0x80, 0x20, 0x30, 0x80, 0x20, 0x40, 0x80, 0x60],
            ),
            // 8010: leaf: LDA $14C2 ; AND #$01 ; STA $0010 ; RTS
            (0x8010, &[0xAD, 0xC2, 0x14, 0x29, 0x01, 0x85, 0x10, 0x60]),
            // 8020: MMIO: STZ $2100 ; RTS
            (0x8020, &[0x9C, 0x00, 0x21, 0x60]),
            // 8030: stack trick: PLA ; PLA ; RTS
            (0x8030, &[0x68, 0x68, 0x60]),
            // 8040: loops: DEX ; BNE $8040 ; RTL
            (0x8040, &[0xCA, 0xD0, 0xFD, 0x6B]),
        ]);
        let mut run = Run { name: "a".to_owned(), frames: 100, ..Run::default() };
        run.exec = exec(&[
            (0x8000, 5),
            (0x8003, 5),
            (0x8006, 5),
            (0x8009, 5),
            (0x800C, 5),
            (0x8010, 10),
            (0x8013, 10),
            (0x8015, 10),
            (0x8017, 10),
            (0x8020, 3),
            (0x8023, 3),
            (0x8030, 2),
            (0x8031, 2),
            (0x8032, 2),
            (0x8040, 40),
            (0x8041, 40),
            (0x8043, 4),
        ]);
        run.calls = vec![
            edge(0x8010, 0x8000, "JSR"),
            edge(0x8020, 0x8003, "JSR"),
            edge(0x8030, 0x8006, "JSR"),
            edge(0x8040, 0x8009, "JSR"),
        ];
        let runs = vec![run];
        let symbols = Symbols::default();
        (build(&runs, &RomImage(&rom), &symbols), runs)
    }

    fn by(routines: &[Routine], entry: u32) -> &Routine {
        routines.iter().find(|r| r.entry == entry).unwrap()
    }

    #[test]
    fn the_table_has_bodies_calls_and_the_call_graph() {
        let (routines, _) = table();
        let leaf = by(&routines, 0x8010);
        assert_eq!((leaf.size, leaf.instrs, leaf.hits, leaf.calls()), (8, 4, 40, 10));
        assert!(leaf.leaf && leaf.returns == "rts");
        assert_eq!(leaf.callers, BTreeSet::from([]), "the caller at 8000 is no entry");
        assert_eq!(leaf.wram_read, ["7E14C2"]);
        assert_eq!(leaf.wram_write, ["7E0010"]);
        assert_eq!((leaf.entry_m8, leaf.entry_x8), (Some(true), Some(false)));
    }

    #[test]
    fn callers_are_the_routines_that_hold_the_call_site() {
        let rom = rom_with(&[(0x8000, &[0x20, 0x10, 0x80, 0x60]), (0x8010, &[0x60])]);
        let run = Run {
            exec: exec(&[(0x8000, 2), (0x8003, 2), (0x8010, 2)]),
            calls: vec![edge(0x8000, 0x7000, "JSR"), edge(0x8010, 0x8000, "JSR")],
            ..Run::default()
        };
        let routines = build(&[run], &RomImage(&rom), &Symbols::default());
        let callee = by(&routines, 0x8010);
        assert_eq!(callee.callers, BTreeSet::from([0x8000]));
        let caller = by(&routines, 0x8000);
        assert_eq!(caller.callees, BTreeSet::from([0x8010]));
        assert!(!caller.leaf);
        assert_eq!(caller.excluded(), Some("calls"));
    }

    #[test]
    fn flags_mark_mmio_stack_tricks_and_loops() {
        let (routines, _) = table();
        let mmio = by(&routines, 0x8020);
        assert!(mmio.flags.mmio && !mmio.flags.dma);
        assert_eq!(mmio.excluded(), Some("mmio"));
        let stack = by(&routines, 0x8030);
        assert!(stack.flags.stack, "two pulls, no push");
        assert_eq!(stack.excluded(), Some("stack/special"));
        let looped = by(&routines, 0x8040);
        assert!(looped.flags.loops_to_entry);
        assert_eq!(looped.calls(), 4, "the returns count, not the loop's hits");
        assert_eq!(looped.returns, "rtl");
    }

    #[test]
    fn the_ranking_keeps_only_pure_leaf_candidates_hot_and_small_first() {
        let (routines, _) = table();
        let ranked: Vec<u32> = rank(&routines).iter().map(|r| r.entry).collect();
        assert_eq!(ranked, [0x8040, 0x8010]);
        assert!(routines.iter().all(|r| r.entry != 0x8000), "nobody calls the caller");
        assert_eq!(by(&routines, 0x8020).excluded(), Some("mmio"));
    }

    #[test]
    fn dma_known_routines_and_indirect_jumps_are_excluded_with_a_reason() {
        let rom = rom_with(&[
            (0x8000, &[0x8D, 0x0B, 0x42, 0x60]),
            (0x8010, &[0x6C, 0x00, 0x20]),
            (0x8020, &[0x60]),
        ]);
        let run = Run {
            exec: exec(&[(0x8000, 1), (0x8003, 1), (0x8010, 1), (0x8020, 7)]),
            calls: vec![
                edge(0x8000, 0x7000, "JSR"),
                edge(0x8010, 0x7000, "JSR"),
                edge(0x8020, 0x7000, "JSR"),
            ],
            jumps: vec![edge(0x9000, 0x8010, "JMPI")],
            ..Run::default()
        };
        let symbols = Symbols::parse(
            "[[routine]]\nname = \"done\"\naddress = 0x008020\nreturns = \"rts\"\ntiming = \"pure\"\nmode = \"original\"\n",
            std::path::Path::new("symbols.toml"),
        )
        .unwrap();
        let routines = build(&[run], &RomImage(&rom), &symbols);
        assert!(by(&routines, 0x8000).flags.dma);
        let jump = by(&routines, 0x8010);
        assert!(jump.flags.indirect);
        assert_eq!(jump.excluded(), Some("jumps out"));
        assert_eq!(by(&routines, 0x8020).excluded(), Some("known"));
        assert_eq!(by(&routines, 0x8020).known.as_deref(), Some("done"));
        assert_eq!(rank(&routines).len(), 0);
    }

    #[test]
    fn csv_and_summary_list_the_candidates() {
        let (routines, runs) = table();
        let csv = csv(&routines);
        let mut lines = csv.lines();
        let header = lines.next().unwrap();
        assert!(header.starts_with("entry,name,found_by,size"));
        let row = lines.find(|l| l.starts_with("008010,")).unwrap();
        assert_eq!(row.split(',').count(), header.split(',').count(), "{row}");
        assert!(row.contains(",7E14C2,7E0010,"), "{row}");
        assert!(row.ends_with(",2"), "ranked second: {row}");
        let text = summary(&routines, &runs, 50);
        assert!(text.contains("candidates 2"), "{text}");
        assert!(text.contains("008040") && text.contains("008010"), "{text}");
        assert!(text.contains("mmio 1"), "{text}");
    }

    #[test]
    fn indexed_accesses_and_ranges_are_formatted_and_merged() {
        let direct = BTreeSet::from([
            (0x7E_0010, 0x7E_0011),
            (0x7E_0012, 0x7E_0012),
            (0x7E_0020, 0x7E_0020),
        ]);
        let indexed = BTreeSet::from([0x7E_1400]);
        assert_eq!(format_ranges(&direct, &indexed), ["7E0010-7E0012", "7E0020", "7E1400+idx"]);
    }
}
