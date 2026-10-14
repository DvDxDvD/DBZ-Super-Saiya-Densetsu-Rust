//! Function discovery and control flow: the instruction graph of one function variant, with the
//! M and X widths tracked along every path.
//!
//! A *function variant* is an entry address with the M and X widths the code runs with there
//! ([`FuncKey`]). Its graph has a node for every reachable instruction in every state it is
//! reached in (address, widths, and the pushes of `PHP` the function can still pull), and each
//! node says where control goes next ([`Next`]). Anything the generator cannot follow
//! statically ends in [`Next::Bail`]: the generated code then hands control back to the original
//! at that instruction.

use crate::decode::{Insn, Mode, Rom, decode};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// A function variant: entry address (24-bit, bank < $80) and the M/X bits of P it is entered
/// with (`0x00`, `0x10`, `0x20`, `0x30`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FuncKey {
    pub entry: u32,
    pub mx: u8,
}

impl FuncKey {
    /// The name of the generated Rust function.
    #[must_use]
    pub fn name(self) -> String {
        format!("f_{:06X}_{}", self.entry, self.mx >> 4)
    }
}

/// What the generator was told about the code (from traces and the symbols file).
#[derive(Debug, Clone, Default)]
pub struct Hints {
    /// Function entries (24-bit physical addresses).
    pub entries: BTreeSet<u32>,
    /// The M/X bits (native mode) a function was entered with in the traces.
    pub entry_mx: BTreeMap<u32, BTreeSet<u8>>,
    /// The M/X bits (native mode) the traces saw at an address.
    pub widths_at: BTreeMap<u32, BTreeSet<u8>>,
    /// The targets the traces saw for an indirect jump or call at an address.
    pub ind_targets: BTreeMap<u32, BTreeSet<u32>>,
    /// Addresses the generated code must not run through (sync points): control is handed to
    /// the original there.
    pub stops: BTreeSet<u32>,
    /// Addresses the replays executed as original code that no generated function covers: each
    /// not covered by the functions made so far starts one, in every combination of widths.
    pub extra_roots: BTreeSet<u32>,
    /// `true` if the function returns with RTL, `false` RTS, as far as the traces say.
    pub ret_long: BTreeMap<u32, bool>,
}

/// The set of P bytes `PHP` pushed that the function can pull again, as the bytes on its
/// stack (`None` once it has done something that makes the stack unknowable).
type Pst = Option<Vec<u8>>;

const PST_UNKNOWN: u8 = 0x01;
const PST_P: u8 = 0x80;
const PST_LIMIT: usize = 16;
/// The most entries of a jump table the scan reads.
const MAX_TABLE: u32 = 128;

/// The state of the code at an instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SKey {
    pub pc: u32,
    pub mx: u8,
    pub pst: u32,
}

/// Where control goes after an instruction.
#[derive(Debug, Clone)]
pub enum Next {
    /// To the following instruction.
    Seq(SKey),
    /// A conditional branch.
    Branch { taken: SKey, fall: SKey },
    /// An unconditional jump inside the function.
    Jump(SKey),
    /// A jump to another function.
    Tail(FuncKey),
    /// A call. `ret` is the address pushed (the last byte of the instruction), `conts` the
    /// states the code continues in by the widths the callee returns with.
    Call { callee: Callee, long: bool, ret: u32, conts: Vec<(u8, SKey)> },
    /// A return (`long` for RTL).
    Return { long: bool },
    /// `RTI`: P and the address are pulled and the original goes on there.
    Rti,
    /// Control is handed to the original at this instruction, before it executes. `conts` are the
    /// states the original comes back to afterwards (the instruction after a `WAI`, `XCE` or a
    /// call the generator could not follow): they are generated and can be entered where the
    /// original arrives at them.
    Bail { reason: Reason, conts: Vec<SKey> },
    /// `PLP` of an unknown P: the code continues by the widths pulled.
    Switch { conts: Vec<(u8, SKey)> },
    /// An indirect jump; `targets` are the destinations the traces saw.
    IndJump { targets: Vec<(u32, JumpTo)> },
}

/// What a call goes to.
#[derive(Debug, Clone)]
pub enum Callee {
    Direct(FuncKey),
    /// `JSR (abs,X)`: the targets the traces saw.
    Table(Vec<(u32, FuncKey)>),
}

/// One destination of an indirect jump.
#[derive(Debug, Clone, Copy)]
pub enum JumpTo {
    State(SKey),
    Func(FuncKey),
}

/// One node of the graph.
#[derive(Debug, Clone)]
pub struct Node {
    pub insn: Insn,
    pub next: Next,
    /// A sync point: the generated code hands over here when the machine would report a sync
    /// event.
    pub sync: bool,
}

/// The analysed function variant.
#[derive(Debug, Default)]
pub struct Graph {
    pub entry: Option<SKey>,
    pub nodes: HashMap<SKey, Node>,
    /// The widths the returns happen with.
    pub exits: BTreeSet<u8>,
    /// How many returns of each kind (RTS, RTL).
    pub returns: [u32; 2],
    pub viable: bool,
    pub too_big: bool,
}

/// Why a bail was generated (for the report).
pub type Reason = &'static str;

/// Everything the analysis shares between functions.
pub struct Analysis<'a> {
    pub rom: Rom<'a>,
    pub hints: &'a Hints,
    pub entries: BTreeSet<u32>,
    psts: Vec<Pst>,
    pst_ids: HashMap<Pst, u32>,
    graphs: HashMap<FuncKey, Graph>,
    in_progress: BTreeSet<FuncKey>,
    /// Bail reasons by count.
    pub bails: BTreeMap<Reason, u32>,
    /// Instruction graph size limit per function.
    pub node_limit: usize,
    /// Variants whose jump tables are not scanned (the scan made the graph too big).
    no_scan: BTreeSet<FuncKey>,
}

/// The widths of a node's state (nodes keep their instruction's decode widths).
fn state_of(node: &Node) -> u8 {
    (u8::from(node.insn.m8) << 5) | (u8::from(node.insn.x8) << 4)
}

impl<'a> Analysis<'a> {
    #[must_use]
    pub fn new(rom: Rom<'a>, hints: &'a Hints) -> Self {
        let mut a = Self {
            rom,
            hints,
            entries: hints.entries.clone(),
            psts: Vec::new(),
            pst_ids: HashMap::new(),
            graphs: HashMap::new(),
            in_progress: BTreeSet::new(),
            bails: BTreeMap::new(),
            node_limit: 12_000,
            no_scan: BTreeSet::new(),
        };
        let empty = a.intern(Some(Vec::new()));
        debug_assert_eq!(empty, 0);
        a
    }

    fn intern(&mut self, pst: Pst) -> u32 {
        if let Some(&id) = self.pst_ids.get(&pst) {
            return id;
        }
        let id = self.psts.len() as u32;
        self.psts.push(pst.clone());
        self.pst_ids.insert(pst, id);
        id
    }

    /// The graphs analysed so far.
    #[must_use]
    pub fn graphs(&self) -> &HashMap<FuncKey, Graph> {
        &self.graphs
    }

    /// Takes the graph of a variant (analysing it first if needed). A graph that is not
    /// viable has no entry node.
    pub fn graph(&mut self, key: FuncKey) -> &Graph {
        self.analyse(key);
        &self.graphs[&key]
    }

    /// The widths a call to `key` can return with, if the function can be analysed (recursion
    /// is not: the entry widths are assumed there).
    fn exits_of(&mut self, key: FuncKey) -> Option<Vec<u8>> {
        if self.in_progress.contains(&key) {
            return Some(vec![key.mx]);
        }
        self.analyse(key);
        let g = &self.graphs[&key];
        if !g.viable {
            return None;
        }
        if g.exits.is_empty() {
            Some(vec![key.mx])
        } else {
            Some(g.exits.iter().copied().collect())
        }
    }

    /// Analyses a variant once.
    pub fn analyse(&mut self, key: FuncKey) {
        if self.graphs.contains_key(&key) || self.in_progress.contains(&key) {
            return;
        }
        self.in_progress.insert(key);
        let mut graph = self.build(key);
        if graph.too_big && self.no_scan.insert(key) {
            // A scanned table led into data: use the traced targets only.
            graph = self.build(key);
        }
        self.in_progress.remove(&key);
        self.graphs.insert(key, graph);
    }

    fn bail(&mut self, reason: Reason) -> Next {
        *self.bails.entry(reason).or_default() += 1;
        Next::Bail { reason, conts: Vec::new() }
    }

    /// A hand-over after which the original comes back at the instruction after `insn`.
    fn bail_then(&mut self, reason: Reason, insn: &Insn, mx: u8, pst: &Pst) -> Next {
        *self.bails.entry(reason).or_default() += 1;
        let next_pc = insn.next();
        let mut cands: BTreeSet<u8> = BTreeSet::from([mx]);
        if let Some(w) = self.hints.widths_at.get(&next_pc) {
            cands.extend(w.iter().copied());
        }
        let conts = cands.into_iter().map(|c| self.state(next_pc, c, pst)).collect();
        Next::Bail { reason, conts }
    }

    fn build(&mut self, key: FuncKey) -> Graph {
        let mut g = Graph::default();
        let entry = SKey { pc: key.entry, mx: key.mx, pst: 0 };
        if decode(&self.rom, key.entry, key.mx & 0x20 != 0, key.mx & 0x10 != 0).is_none() {
            return g;
        }
        let mut work = vec![entry];
        while let Some(sk) = work.pop() {
            if g.nodes.contains_key(&sk) {
                continue;
            }
            if g.nodes.len() >= self.node_limit {
                g.too_big = true;
                return Graph { too_big: true, ..Graph::default() };
            }
            let (m8, x8) = (sk.mx & 0x20 != 0, sk.mx & 0x10 != 0);
            let Some(insn) = decode(&self.rom, sk.pc, m8, x8) else {
                // Not ROM: hand over (a stub node so the state exists).
                let insn = Insn {
                    pc: sk.pc,
                    op: 0xEA,
                    mn: "NOP",
                    mode: Mode::Imp,
                    len: 1,
                    opr: 0,
                    target: None,
                    m8,
                    x8,
                };
                let next = self.bail("not rom");
                g.nodes.insert(sk, Node { insn, next, sync: false });
                continue;
            };
            let next = self.step(key, &insn, sk, &mut g);
            match &next {
                Next::Seq(n) | Next::Jump(n) => work.push(*n),
                Next::Branch { taken, fall } => {
                    work.push(*fall);
                    work.push(*taken);
                }
                Next::Call { conts, .. } | Next::Switch { conts } => {
                    work.extend(conts.iter().map(|&(_, n)| n));
                }
                Next::IndJump { targets } => {
                    for (_, t) in targets {
                        if let JumpTo::State(n) = t {
                            work.push(*n);
                        }
                    }
                }
                Next::Bail { conts, .. } => work.extend(conts.iter().copied()),
                Next::Tail(_) | Next::Return { .. } | Next::Rti => {}
            }
            let sync = self.hints.stops.contains(&insn.pc);
            g.nodes.insert(sk, Node { insn, next, sync });
        }
        g.entry = Some(entry);
        g.viable = true;
        // Exits and the return kinds.
        for node in g.nodes.values() {
            match &node.next {
                Next::Return { long } => {
                    g.returns[usize::from(*long)] += 1;
                    g.exits.insert(state_of(node));
                }
                Next::Tail(callee) => {
                    // The callee's own exits are the function's.
                    if let Some(exits) = self.exits_of(*callee) {
                        g.exits.extend(exits);
                    }
                }
                _ => {}
            }
        }
        g
    }

    fn pst_of(&self, id: u32) -> Pst {
        self.psts[id as usize].clone()
    }

    fn state(&mut self, pc: u32, mx: u8, pst: &Pst) -> SKey {
        SKey { pc, mx, pst: self.intern(pst.clone()) }
    }

    /// The successor of `insn` in state `sk`.
    #[allow(clippy::too_many_lines)]
    fn step(&mut self, func: FuncKey, insn: &Insn, sk: SKey, g: &mut Graph) -> Next {
        let _ = g;
        let mx = sk.mx;
        let mut pst = self.pst_of(sk.pst);
        let next_pc = insn.next();
        let (m_bytes, x_bytes) = (if insn.m8 { 1 } else { 2 }, if insn.x8 { 1 } else { 2 });

        let push = |pst: &mut Pst, n: usize, byte: u8| {
            if let Some(v) = pst {
                if v.len() + n > PST_LIMIT {
                    *pst = None;
                } else {
                    v.extend(std::iter::repeat_n(byte, n));
                }
            }
        };
        let pull = |pst: &mut Pst, n: usize| -> Option<u8> {
            let v = pst.as_mut()?;
            let mut last = None;
            for _ in 0..n {
                last = v.pop();
            }
            if last.is_none() {
                *pst = None;
            }
            last
        };

        let mut new_mx = mx;
        match insn.mn {
            "REP" | "SEP" => {
                let set = insn.mn == "SEP";
                let bits = (insn.opr as u8) & 0x30;
                new_mx = if set { mx | bits } else { mx & !bits };
            }
            "PHA" => push(&mut pst, m_bytes, PST_UNKNOWN),
            "PHX" | "PHY" => push(&mut pst, x_bytes, PST_UNKNOWN),
            "PHB" | "PHK" => push(&mut pst, 1, PST_UNKNOWN),
            "PHD" | "PEA" | "PEI" | "PER" => push(&mut pst, 2, PST_UNKNOWN),
            "PHP" => push(&mut pst, 1, PST_P | (mx >> 4)),
            "PLA" => {
                pull(&mut pst, m_bytes);
            }
            "PLX" | "PLY" => {
                pull(&mut pst, x_bytes);
            }
            "PLB" => {
                pull(&mut pst, 1);
            }
            "PLD" => {
                pull(&mut pst, 2);
            }
            "TXS" | "TCS" => pst = None,
            _ => {}
        }

        match (insn.mn, insn.mode) {
            ("XCE", _) => self.bail_then("XCE", insn, mx, &pst),
            ("WAI", _) => self.bail_then("WAI", insn, mx, &pst),
            ("RTI", _) => Next::Rti,
            ("STP" | "BRK" | "COP" | "WDM", _) => self.bail("software interrupt or stop"),
            ("RTS", _) => Next::Return { long: false },
            ("RTL", _) => Next::Return { long: true },
            ("PLP", _) => {
                let top = pull(&mut pst, 1);
                match top {
                    Some(b) if b & PST_P != 0 => {
                        let nmx = (b & 0x03) << 4;
                        let n = self.state(next_pc, nmx, &pst);
                        Next::Seq(n)
                    }
                    _ => {
                        let mut cands: BTreeSet<u8> = BTreeSet::new();
                        if let Some(w) = self.hints.widths_at.get(&next_pc) {
                            cands.extend(w.iter().copied());
                        }
                        cands.insert(mx);
                        let conts =
                            cands.into_iter().map(|c| (c, self.state(next_pc, c, &pst))).collect();
                        Next::Switch { conts }
                    }
                }
            }
            ("BRA" | "BRL", _) => {
                let t = insn.target.expect("branch target");
                self.jump_to(func, t, mx, &pst)
            }
            (_, Mode::Rel8)
            | ("BCC" | "BCS" | "BEQ" | "BNE" | "BMI" | "BPL" | "BVC" | "BVS", _) => {
                let t = insn.target.expect("branch target");
                let taken = self.state(t, mx, &pst);
                let fall = self.state(next_pc, mx, &pst);
                Next::Branch { taken, fall }
            }
            ("JMP", Mode::Abs) | ("JML", Mode::Long) => {
                let t = insn.target.expect("jump target");
                self.jump_to(func, t, mx, &pst)
            }
            ("JMP", Mode::AbsInd | Mode::AbsIndX) | ("JML", Mode::AbsIndLong) => {
                let bank = insn.pc & 0xFF_0000;
                let mut targets = Vec::new();
                let ts: BTreeSet<u32> = if insn.mode == Mode::AbsIndX {
                    self.table_targets(func, insn)
                } else {
                    self.hints.ind_targets.get(&insn.pc).cloned().unwrap_or_default()
                };
                {
                    for t in ts {
                        let t = if insn.mn == "JML" { t } else { bank | (t & 0xFFFF) };
                        let to = match self.jump_to(func, t, mx, &pst) {
                            Next::Jump(s) => JumpTo::State(s),
                            Next::Tail(f) => JumpTo::Func(f),
                            _ => continue,
                        };
                        targets.push((t, to));
                    }
                }
                *self.bails.entry("indirect jump target unknown").or_default() +=
                    u32::from(targets.is_empty());
                Next::IndJump { targets }
            }
            ("JSR", Mode::Abs) | ("JSL", Mode::Long) => {
                let t = insn.target.expect("call target");
                let long = insn.mn == "JSL";
                self.entries.insert(t);
                let callee = FuncKey { entry: t, mx };
                self.call(callee, long, insn, &pst, mx)
            }
            ("JSR", Mode::AbsIndX) => {
                let bank = insn.pc & 0xFF_0000;
                let mut table = Vec::new();
                {
                    for t in self.table_targets(func, insn) {
                        let t = bank | (t & 0xFFFF);
                        self.entries.insert(t);
                        table.push((t, FuncKey { entry: t, mx }));
                    }
                }
                if table.is_empty() {
                    return self.bail_then("indirect call target unknown", insn, mx, &pst);
                }
                let mut exits: Option<BTreeSet<u8>> = Some(BTreeSet::new());
                for (_, k) in &table {
                    match (self.exits_of(*k), exits.as_mut()) {
                        (Some(e), Some(set)) => set.extend(e),
                        _ => exits = None,
                    }
                }
                let table: Vec<_> = table
                    .into_iter()
                    .filter(|(_, k)| self.graphs.get(k).is_none_or(|gr| gr.viable))
                    .collect();
                if table.is_empty() || exits.is_none() {
                    return self.bail_then("indirect call target not viable", insn, mx, &pst);
                }
                let conts = exits
                    .unwrap_or_default()
                    .into_iter()
                    .map(|c| (c, self.state(next_pc, c, &pst)))
                    .collect();
                Next::Call {
                    callee: Callee::Table(table),
                    long: false,
                    ret: next_pc.wrapping_sub(1) & 0xFFFF,
                    conts,
                }
            }
            _ => {
                // An ordinary instruction.
                let n = self.state(next_pc, new_mx, &pst);
                Next::Seq(n)
            }
        }
    }

    /// The 16-bit targets of the `JMP (abs,X)` or `JSR (abs,X)` at `insn`: those the traces saw
    /// and the entries of the table at `abs` in the program bank (the table ends where the
    /// first of its targets above it begins, or at a word below `$8000`).
    fn table_targets(&self, func: FuncKey, insn: &Insn) -> BTreeSet<u32> {
        let mut out: BTreeSet<u32> = BTreeSet::new();
        if let Some(ts) = self.hints.ind_targets.get(&insn.pc) {
            out.extend(ts.iter().map(|t| t & 0xFFFF));
        }
        if insn.mode != Mode::AbsIndX || self.no_scan.contains(&func) {
            return out;
        }
        let bank = insn.pc & 0xFF_0000;
        let base = insn.opr & 0xFFFF;
        let mut limit = out.iter().copied().filter(|&t| t > base).min().unwrap_or(u32::MAX);
        for i in 0..MAX_TABLE {
            let off = base + 2 * i;
            if off + 1 > 0xFFFF || off >= limit {
                break;
            }
            let (Some(lo), Some(hi)) = (self.rom.read(bank | off), self.rom.read(bank | (off + 1)))
            else {
                break;
            };
            let w = u32::from(u16::from_le_bytes([lo, hi]));
            if w < 0x8000 {
                break;
            }
            if w > base {
                limit = limit.min(w);
            }
            out.insert(w);
        }
        out
    }

    fn call(&mut self, callee: FuncKey, long: bool, insn: &Insn, pst: &Pst, mx: u8) -> Next {
        let Some(exits) = self.exits_of(callee) else {
            return self.bail_then("callee not viable", insn, mx, pst);
        };
        if !self.in_progress.contains(&callee) && !self.graphs[&callee].viable {
            return self.bail_then("callee not viable", insn, mx, pst);
        }
        let next_pc = insn.next();
        let ret = next_pc.wrapping_sub(1) & 0xFFFF;
        let conts = exits.into_iter().map(|c| (c, self.state(next_pc, c, pst))).collect();
        Next::Call { callee: Callee::Direct(callee), long, ret, conts }
    }

    fn jump_to(&mut self, func: FuncKey, target: u32, mx: u8, pst: &Pst) -> Next {
        let callee = FuncKey { entry: target, mx };
        if callee == func {
            return Next::Jump(SKey { pc: target, mx, pst: 0 });
        }
        if self.entries.contains(&target) && target != func.entry {
            // Another function: a tail call, if it can be generated.
            self.analyse(callee);
            let viable = self.in_progress.contains(&callee) || self.graphs[&callee].viable;
            if viable {
                return Next::Tail(callee);
            }
        }
        Next::Jump(self.state(target, mx, pst))
    }
}
