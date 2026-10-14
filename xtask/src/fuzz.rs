//! `recomp-cover` and `fuzz-recomp`: how much of the recompiled code the replays run, and a
//! differential fuzzer for the rest.
//!
//! `recomp-cover` plays replays with the recompiled table in `Rust` mode and writes which blocks
//! of the generated code ran (the generated `COV` flags). `fuzz-recomp` takes the real entry
//! states of the functions from replays played on the pure original (a breakpoint at every
//! generated entry), perturbs them (registers, flags, the WRAM bytes the function reads) and runs
//! the original and the generated function from the same state with a `Verify` call, comparing
//! registers, flags, memories, the CPU's writes, the return and the cycles. Functions no replay
//! reaches get synthesised states (a world snapshot, the registers set to the entry, a return
//! address pushed). Everything is seeded and reproducible. `fuzz-report` merges the results.

use crate::shared::read_replay;
use anyhow::Context;
use ssd_oracle::{
    HookEntry, HookMode, HookTable, Joypad, Machine, MemorySaveStore, NullAudio, StatusRegister,
    Stopped, Stops, Symbols, joypad_from_bits,
};
use ssd_recomp::rt::Variant;
use ssd_recomp_gen::{COV, TABLE, VARIANTS};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering::Relaxed;
use std::time::{Duration, Instant};

// ----------------------------------------------------------------------------------------------
// Coverage

fn cov_reset() {
    for flag in &COV {
        flag.store(0, Relaxed);
    }
}

fn cov_bytes() -> Vec<u8> {
    COV.iter().map(|flag| flag.load(Relaxed)).collect()
}

fn read_cov(path: &Path) -> anyhow::Result<Vec<u8>> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    anyhow::ensure!(
        bytes.len() == COV.len(),
        "{} has {} flags, the generated code has {} (regenerate and rerun)",
        path.display(),
        bytes.len(),
        COV.len()
    );
    Ok(bytes)
}

/// `recomp-cover <out> <replay>...`: plays the replays in `Rust` mode (the recompiled table with
/// the hand-written routines, as `verify-replays` runs it) and writes the flags of the blocks that
/// ran to `out`.
///
/// # Errors
///
/// A replay cannot be read or played.
pub fn cover(
    rom: &[u8],
    symbols: &Symbols,
    out_path: &Path,
    replays: &[PathBuf],
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    cov_reset();
    let mut seen = vec![0_u8; COV.len()];
    for path in replays {
        let replay = read_replay(path, rom)?;
        let table = ssd_game::hook_table(symbols, Some(HookMode::Rust))?;
        let mut machine = replay.power_on(rom.to_vec())?;
        machine.set_hook_table(Some(table));
        let started = Instant::now();
        replay.play(&mut machine, &mut NullAudio)?;
        let now = cov_bytes();
        let new = now.iter().zip(&seen).filter(|(a, b)| **a != 0 && **b == 0).count();
        for (s, n) in seen.iter_mut().zip(&now) {
            *s |= *n;
        }
        writeln!(
            out,
            "{}: {} frames, {:.0} s, {} blocks new, {} in all",
            path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
            replay.frames(),
            started.elapsed().as_secs_f64(),
            new,
            seen.iter().filter(|b| **b != 0).count()
        )?;
        out.flush()?;
        std::fs::write(out_path, &seen)?;
    }
    Ok(())
}

// ----------------------------------------------------------------------------------------------
// Random numbers

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed ^ 0x9E37_79B9_7F4A_7C15)
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }

    fn byte(&mut self) -> u8 {
        match self.next() % 10 {
            0 => 0x00,
            1 => 0xFF,
            2 => 0x80,
            3 => 0x01,
            4 => 0x7F,
            5 => (self.next() % 8) as u8,
            _ => self.next() as u8,
        }
    }

    fn word(&mut self) -> u16 {
        match self.next() % 12 {
            0 => 0,
            1 => 1,
            2 => 0x00FF,
            3 => 0x0100,
            4 => 0x7FFF,
            5 => 0x8000,
            6 => 0xFFFF,
            7 => (self.next() % 0x40) as u16,
            8 => (self.next() % 0x2000) as u16,
            _ => self.next() as u16,
        }
    }
}

fn mix(a: u64, b: u64) -> u64 {
    let mut r = Rng::new(a ^ b.rotate_left(32));
    r.next()
}

// ----------------------------------------------------------------------------------------------
// The state of an iteration

/// What an iteration changes in the entry state.
#[derive(Debug, Clone, Default)]
struct Edits {
    /// The M/X bits of P the call runs with.
    mx: u8,
    a: Option<u16>,
    x: Option<u16>,
    y: Option<u16>,
    d: Option<u16>,
    dbr: Option<u8>,
    /// New N, V, Z and C (bits 7, 6, 1, 0 of P), if changed.
    nvzc: Option<u8>,
    /// Bytes to write on the bus, in order.
    mem: Vec<(u32, u8)>,
    /// The joypad of the frames the call runs through.
    pad: u16,
}

fn apply(edits: &Edits, machine: &mut Machine) {
    let mut regs = machine.cpu_registers();
    regs.emulation_mode = false;
    let mut p = u8::from(regs.p);
    p = (p & !0x38) | (edits.mx & 0x30);
    if let Some(nvzc) = edits.nvzc {
        p = (p & !0xC3) | (nvzc & 0xC3);
    }
    regs.p = StatusRegister::from(p);
    if let Some(a) = edits.a {
        regs.a = a;
    }
    if let Some(x) = edits.x {
        regs.x = x;
    }
    if let Some(y) = edits.y {
        regs.y = y;
    }
    if edits.mx & 0x10 != 0 {
        regs.x &= 0x00FF;
        regs.y &= 0x00FF;
    }
    if let Some(d) = edits.d {
        regs.d = d;
    }
    if let Some(dbr) = edits.dbr {
        regs.dbr = dbr;
    }
    machine.set_cpu_registers(regs);
    for &(address, value) in &edits.mem {
        machine.bus_write(address, value);
    }
}

/// The address a read can be changed at: work RAM (and its low mirror) and SRAM.
fn writable(address: u32) -> Option<u32> {
    let bank = (address >> 16) as u8;
    let offset = address & 0xFFFF;
    match bank {
        0x7E | 0x7F => Some(address),
        0x00..=0x3F | 0x80..=0xBF if offset < 0x2000 => Some(0x7E_0000 + offset),
        0x70..=0x7D if offset < 0x8000 => Some(address),
        _ => None,
    }
}

fn make_edits(
    rng: &mut Rng,
    mxs: &[u8],
    iteration: usize,
    reads: &[u32],
    corpus: &[Edits],
    protect: std::ops::Range<u32>,
    main_mx: u8,
) -> Edits {
    let mut e = if !corpus.is_empty() && rng.chance(50) {
        corpus[rng.below(corpus.len())].clone()
    } else {
        Edits::default()
    };
    // Mostly the widths the real caller had (the first mx), the other variants now and then.
    e.mx = if rng.chance(70) { main_mx } else { mxs[iteration % mxs.len()] };
    // Index-like values mostly: a wild index jumps into data, which says nothing about the code.
    let reg = |rng: &mut Rng| if rng.chance(65) { (rng.next() % 0x30) as u16 } else { rng.word() };
    if rng.chance(70) {
        e.a = Some(reg(rng));
    }
    if rng.chance(50) {
        e.x = Some(reg(rng));
    }
    if rng.chance(50) {
        e.y = Some(reg(rng));
    }
    if rng.chance(40) {
        e.nvzc = Some(rng.next() as u8);
    }
    if rng.chance(4) {
        e.d = Some(((rng.next() % 0x20) as u16) << 8);
    }
    if rng.chance(12) {
        e.dbr = Some([0x00, 0x7E, 0x7F, 0x80, 0x7E, 0x01][rng.below(6)]);
    }
    if rng.chance(30) {
        e.pad = rng.next() as u16 & 0x0FFF;
    }
    // The stack at and above the call's S holds return addresses: not data to fuzz.
    let allowed = |address: u32| {
        !(0x7E_0000..0x80_0000).contains(&address) || !protect.contains(&(address - 0x7E_0000))
    };
    let candidates: Vec<u32> =
        reads.iter().filter_map(|a| writable(*a)).filter(|a| allowed(*a)).collect();
    let edits = 1 + rng.below(4);
    for _ in 0..edits {
        if candidates.is_empty() || rng.chance(10) {
            // Noise anywhere in the first 8 KiB of work RAM or in the second bank.
            let address = if rng.chance(70) {
                0x7E_0000 + (rng.next() % 0x2000) as u32
            } else {
                0x7E_0000 + (rng.next() % 0x2_0000) as u32
            };
            if allowed(address) {
                e.mem.push((address, rng.byte()));
            }
            continue;
        }
        let at = candidates[rng.below(candidates.len())];
        match rng.next() % 6 {
            0 => {
                // A run of bytes around the read.
                let start = at.saturating_sub(rng.below(8) as u32);
                for i in 0..(1 + rng.below(16)) as u32 {
                    if allowed(start + i) {
                        e.mem.push((start + i, rng.byte()));
                    }
                }
            }
            1 => {
                // A whole word.
                let w = rng.word();
                if allowed(at + 1) {
                    e.mem.push((at, w as u8));
                    e.mem.push((at + 1, (w >> 8) as u8));
                }
            }
            _ => e.mem.push((at, rng.byte())),
        }
    }
    e
}

// ----------------------------------------------------------------------------------------------
// Results

#[derive(Debug, Clone, Copy, Default)]
struct Tally {
    iters: u32,
    verified: u32,
    diverged: u32,
    cost: u32,
    insns: u32,
    interrupted: u32,
    no_return: u32,
    handed_over: u32,
    other: u32,
    early: u32,
    /// An iteration of the entry did not end in 10 s (the run was stopped and resumed).
    hung: u32,
}

impl Tally {
    fn add(&mut self, o: &Self) {
        self.iters += o.iters;
        self.verified += o.verified;
        self.diverged += o.diverged;
        self.cost += o.cost;
        self.insns += o.insns;
        self.interrupted += o.interrupted;
        self.no_return += o.no_return;
        self.handed_over += o.handed_over;
        self.other += o.other;
        self.early += o.early;
        self.hung += o.hung;
    }
}

#[derive(Default)]
struct Results {
    /// By (entry, mx).
    tally: BTreeMap<(u32, u8), Tally>,
    /// Divergence texts by entry, at most a few each.
    notes: BTreeMap<u32, Vec<String>>,
    /// Blocks the fuzzer ran.
    fuzz_cov: Vec<u8>,
    /// Times spent by entry.
    spent: BTreeMap<u32, Duration>,
}

impl Results {
    fn new() -> Self {
        Self { fuzz_cov: vec![0; COV.len()], ..Self::default() }
    }

    fn write(&self, path: &Path) -> anyhow::Result<()> {
        let mut text = String::new();
        for ((entry, mx), t) in &self.tally {
            let _ = writeln!(
                text,
                "{entry:06X}\t{mx:02X}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}	{}	{}",
                t.iters,
                t.verified,
                t.diverged,
                t.cost,
                t.insns,
                t.interrupted,
                t.no_return,
                t.handed_over,
                t.other,
                t.early,
                t.hung
            );
        }
        std::fs::write(path, text)?;
        std::fs::write(path.with_extension("cov"), &self.fuzz_cov)?;
        let mut notes = String::new();
        for (entry, list) in &self.notes {
            for note in list {
                let _ = writeln!(notes, "== {entry:06X}\n{note}");
            }
        }
        std::fs::write(path.with_extension("div"), notes)?;
        Ok(())
    }
}

fn read_tallies(path: &Path) -> anyhow::Result<BTreeMap<(u32, u8), Tally>> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let mut map: BTreeMap<(u32, u8), Tally> = BTreeMap::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 11 {
            continue;
        }
        let n = |i: usize| f[i].parse::<u32>().unwrap_or(0);
        let t = Tally {
            iters: n(2),
            verified: n(3),
            diverged: n(4),
            cost: n(5),
            insns: n(6),
            interrupted: n(7),
            no_return: n(8),
            handed_over: n(9),
            other: n(10),
            early: if f.len() > 11 { n(11) } else { 0 },
            hung: if f.len() > 12 { n(12) } else { 0 },
        };
        let key = (u32::from_str_radix(f[0], 16)?, u8::from_str_radix(f[1], 16)?);
        map.entry(key).or_default().add(&t);
    }
    Ok(map)
}

// ----------------------------------------------------------------------------------------------
// The fuzzer

/// Options of `fuzz-recomp`.
#[derive(Debug, Clone)]
pub struct Options {
    pub seed: u64,
    /// Iterations per entry (all its variants and seeds together).
    pub iters: u32,
    /// Seeds (real states) per entry, at most.
    pub seeds: u32,
    /// Seconds per entry, at most.
    pub seconds: f64,
    /// Restrict to these entries.
    pub only: BTreeSet<u32>,
    /// Print the divergences as they are found.
    pub verbose: bool,
    /// Frames of a call's original before it counts as not returning.
    pub budget_frames: u64,
    /// Continue an earlier run of the same output file: its entries are not fuzzed again.
    pub resume: bool,
    /// Only the entries whose number is `part.0` modulo `part.1` (to run several processes).
    pub part: (usize, usize),
    /// Switch the NMI, the IRQ and the joypad read off in every state (so a long call is not
    /// cut by the vertical blank).
    pub no_nmi: bool,
    /// `harvest`: save the first arrival at the `--only` entries at or after this frame (and
    /// nothing else), to look into one call of a replay.
    pub at_frame: Option<u64>,
    /// Switch HDMA off in every state (a control run: the open bus a DMA read leaves behind
    /// depends on where in an instruction the transfer lands).
    pub no_hdma: bool,
    /// Entry, seed tag and iteration to look into: where does the generated code depart from
    /// the original there?
    pub repro: Option<(u32, u64, usize)>,
}

struct Fuzzer<'a> {
    opts: &'a Options,
    full: HookTable,
    results: Results,
    /// Variants by entry: (mx, index into VARIANTS).
    variants: BTreeMap<u32, Vec<(u8, usize)>>,
    long: BTreeMap<u32, bool>,
    /// Per entry: iterations done and the seeds taken.
    done: BTreeMap<u32, u32>,
    /// Per entry: all its variants' tallies.
    totals: BTreeMap<u32, Tally>,
    /// Iterations (entry, seed tag, number) that did not end in an earlier run: skipped.
    skip: BTreeSet<(u32, u64, usize)>,
    out: &'a mut dyn Write,
    started: Instant,
    out_path: PathBuf,
}

fn single_table(full: &HookTable, key: u32) -> HookTable {
    let mut table = HookTable::new();
    if let Some(entry) = full.get(key) {
        let entry: HookEntry = entry.clone();
        table.insert(entry);
    }
    table
}

impl<'a> Fuzzer<'a> {
    fn new(opts: &'a Options, out: &'a mut dyn Write) -> Self {
        let full = ssd_game::recompiled_table(HookMode::Verify, opts.budget_frames);
        let mut variants: BTreeMap<u32, Vec<(u8, usize)>> = BTreeMap::new();
        for (i, v) in VARIANTS.iter().enumerate() {
            variants.entry(v.entry).or_default().push((v.mx, i));
        }
        let long = TABLE.iter().map(|e| (e.key, e.long)).collect();
        Self {
            opts,
            full,
            results: Results::new(),
            variants,
            long,
            done: BTreeMap::new(),
            totals: BTreeMap::new(),
            skip: BTreeSet::new(),
            out,
            started: Instant::now(),
            out_path: PathBuf::new(),
        }
    }

    /// Takes over what an earlier run of the same output file did: its entries are closed.
    fn resume(&mut self, path: &Path) {
        if let Ok(tallies) = read_tallies(path) {
            for ((entry, mx), t) in tallies {
                self.results.tally.entry((entry, mx)).or_default().add(&t);
                self.totals.entry(entry).or_default().add(&t);
                self.done.insert(entry, self.opts.iters);
            }
        }
        if let Ok(bytes) = std::fs::read(path.with_extension("cov"))
            && bytes.len() == self.results.fuzz_cov.len()
        {
            self.results.fuzz_cov = bytes;
        }
        if let Ok(text) = std::fs::read_to_string(path.with_extension("hang")) {
            // `KKKKKK iteration I tag 0xT ...`
            for line in text.lines() {
                let f: Vec<&str> = line.split_whitespace().collect();
                if let [key, "iteration", i, "tag", tag, ..] = f.as_slice()
                    && let (Ok(key), Ok(i), Ok(tag)) = (
                        u32::from_str_radix(key, 16),
                        i.parse::<usize>(),
                        u64::from_str_radix(tag.trim_start_matches("0x"), 16),
                    )
                {
                    self.skip.insert((key, tag, i));
                }
            }
        }
        if let Ok(text) = std::fs::read_to_string(path.with_extension("div")) {
            let mut current = 0;
            for line in text.lines() {
                if let Some(rest) = line.strip_prefix("== ") {
                    current = u32::from_str_radix(rest.trim(), 16).unwrap_or(0);
                    self.results.notes.entry(current).or_default().push(String::new());
                } else if let Some(last) =
                    self.results.notes.get_mut(&current).and_then(|l| l.last_mut())
                {
                    last.push_str(line);
                    last.push('\n');
                }
            }
        }
    }

    fn wants(&self, key: u32) -> bool {
        (self.opts.only.is_empty() || self.opts.only.contains(&key))
            && self.long.contains_key(&key)
            && self.variants.contains_key(&key)
    }

    fn spent_seconds(&self, key: u32) -> f64 {
        self.results.spent.get(&key).map_or(0.0, Duration::as_secs_f64)
    }

    /// Whether the entry still wants iterations.
    fn open(&self, key: u32) -> bool {
        self.done.get(&key).copied().unwrap_or(0) < self.opts.iters
            && self.spent_seconds(key) < self.opts.seconds
            && !self.given_up(key)
    }

    /// An entry whose iterations are all skipped after a dozen of them is left alone.
    fn given_up(&self, key: u32) -> bool {
        self.totals.get(&key).is_some_and(|t| t.iters >= 14 && t.verified == 0)
    }

    /// Fuzzes `key` from `base`, a machine at the entry's opcode fetch with the arrival armed.
    fn fuzz(&mut self, key: u32, base: &Machine, share: u32, tag: u64) {
        let began = Instant::now();
        let mut base = base.snapshot();
        base.set_hook_table(Some(single_table(&self.full, key)));
        base.log_reads(true);
        base.set_divergence_dir(None);
        let variants = self.variants[&key].clone();
        let mxs: Vec<u8> = variants.iter().map(|(mx, _)| *mx).collect();
        // The mx the real state has first.
        let real_mx = u8::from(base.cpu_registers().p) & 0x30;
        let start = mxs.iter().position(|m| *m == real_mx).unwrap_or(0);
        let mut rng = Rng::new(mix(self.opts.seed ^ u64::from(key), tag));
        let sp = u32::from(base.cpu_registers().s);
        let protect = sp.saturating_sub(0x10)..sp + 0x100;
        let mut reads: Vec<u32> = Vec::new();
        let mut corpus: Vec<Edits> = Vec::new();
        let mut local_seen = vec![0_u8; COV.len()];
        let stops = Stops { after_dispatch: true, ..Stops::default() };
        for i in 0..share as usize {
            if !self.open(key) {
                break;
            }
            let mut edits = if i == 0 {
                Edits { mx: real_mx, ..Edits::default() }
            } else {
                make_edits(&mut rng, &mxs, i + start, &reads, &corpus, protect.clone(), mxs[start])
            };
            if self.skip.contains(&(key, tag, i)) {
                let first = self.variants[&key][0].0;
                let hung = Tally { iters: 1, hung: 1, ..Tally::default() };
                self.results.tally.entry((key, first)).or_default().add(&hung);
                self.totals.entry(key).or_default().add(&hung);
                *self.done.entry(key).or_default() += 1;
                continue;
            }
            let mx = edits.mx;
            if !mxs.contains(&mx) {
                edits.mx = mxs[start];
            }
            let mx = edits.mx;
            let pad = edits.pad;
            let mut m = base.snapshot();
            if i != 0 || !mxs.contains(&real_mx) {
                apply(&edits, &mut m);
            }
            if self.opts.no_hdma {
                m.bus_write(0x00_420C, 0);
            }
            if self.opts.no_nmi {
                m.bus_write(0x00_4200, 0);
            }
            cov_reset();
            set_current(Some(format!("{key:06X} iteration {i} tag {tag:#x} {edits:?}")));
            if std::env::var_os("SSD_FUZZ_TRACE").is_some() {
                eprintln!("{key:06X} iteration {i} {edits:?}");
            }
            let mut input = |_frame: u64| -> Joypad { joypad_from_bits(pad) };
            if std::env::var_os("SSD_FUZZ_PCS").is_some() {
                trace_original(&m, &mut input);
            }
            if self.opts.repro == Some((key, tag, i)) {
                let report = bisect(&m, &mut input);
                let _ = write!(self.out, "{report}");
            }
            let result = m.run(&stops, &mut input, &mut NullAudio, 1);
            let result_text = format!("{result:?}");
            set_current(None);
            let mut tally = Tally { iters: 1, ..Tally::default() };
            let stats = m.hook_stats().into_iter().map(|(_, s)| s).next();
            match (result, stats) {
                // (A call that never returns is demoted, which ends the run by its budget.)
                (Ok(_), Some(s)) => {
                    if s.divergences > 0 {
                        tally.diverged = 1;
                    } else if s.cost_mismatch > 0 {
                        tally.cost = 1;
                    } else if s.verified_ok > 0 {
                        tally.verified = 1;
                        tally.insns = u32::from(s.instructions_mismatch > 0);
                    } else if s.early_exit > 0 {
                        tally.early = 1;
                    } else if s.interrupted > 0 {
                        tally.interrupted = 1;
                    } else if s.no_return > 0 {
                        tally.no_return = 1;
                    } else if s.sampled_out > 0 {
                        tally.handed_over = 1;
                    } else {
                        tally.other = 1;
                    }
                }
                _ => tally.other = 1,
            }
            if tally.other > 0 && std::env::var_os("SSD_FUZZ_OTHER").is_some() {
                let _ = writeln!(
                    self.out,
                    "{key:06X} iteration {i}: cannot run: {result_text:?} {stats:?}"
                );
            }
            if tally.diverged > 0 || tally.cost > 0 {
                let notes = self.results.notes.entry(key).or_default();
                if notes.len() < 4 {
                    let mut text =
                        format!("iteration {i} tag {tag:#x} seed {} {edits:?}\n", self.opts.seed);
                    for d in m.divergences() {
                        let _ = write!(text, "{d}");
                    }
                    if tally.cost > 0 {
                        text.push_str("  (cycle count differs)\n");
                    }
                    if self.opts.verbose {
                        let _ = writeln!(self.out, "{key:06X}: {text}");
                    }
                    notes.push(text);
                }
            }
            // New blocks, new reads: this state is worth mutating again.
            let flags = cov_bytes();
            // New to this seed's iterations (not to the run: an iteration must not depend on which
            // entries came before, or it could not be reproduced).
            let mut fresh = false;
            for ((seen, local), now) in
                self.results.fuzz_cov.iter_mut().zip(local_seen.iter_mut()).zip(&flags)
            {
                if *now != 0 && *local == 0 {
                    fresh = true;
                }
                *local |= *now;
                *seen |= *now;
            }
            let new_reads = m.take_check_reads();
            if i == 0 || fresh {
                if !new_reads.is_empty() {
                    let known: BTreeSet<u32> = reads.iter().copied().collect();
                    for r in new_reads {
                        if !known.contains(&r) {
                            reads.push(r);
                        }
                    }
                    reads.sort_unstable();
                    reads.dedup();
                    reads.truncate(4096);
                }
                if i != 0 && corpus.len() < 24 {
                    corpus.push(edits.clone());
                }
            }
            self.results.tally.entry((key, mx)).or_default().add(&tally);
            self.totals.entry(key).or_default().add(&tally);
            *self.done.entry(key).or_default() += 1;
        }
        *self.results.spent.entry(key).or_default() += began.elapsed();
        if std::env::var_os("SSD_FUZZ_LOG").is_some() {
            let t = self.totals.get(&key).copied().unwrap_or_default();
            let _ = writeln!(
                self.out,
                "  {key:06X} seed {tag:#x}: {share} planned, {:.2} s, total iters {} ok {} diff {} cost {} early {} int {} noret {} hand {} other {}",
                began.elapsed().as_secs_f64(),
                t.iters,
                t.verified,
                t.diverged,
                t.cost,
                t.early,
                t.interrupted,
                t.no_return,
                t.handed_over,
                t.other
            );
        }
    }
}

// ----------------------------------------------------------------------------------------------
// Finding where the generated code departs from the original

/// The state of `rust` (the machine after the generated function ran) against `original` (the
/// pure machine after the same number of instructions): what differs, as lines of text.
fn state_differences(original: &Machine, rust: &Machine) -> Vec<String> {
    let mut out = Vec::new();
    let (a, b) = (original.cpu_registers(), rust.cpu_registers());
    let regs = [
        ("A", u32::from(a.a), u32::from(b.a)),
        ("X", u32::from(a.x), u32::from(b.x)),
        ("Y", u32::from(a.y), u32::from(b.y)),
        ("S", u32::from(a.s), u32::from(b.s)),
        ("D", u32::from(a.d), u32::from(b.d)),
        ("DBR", u32::from(a.dbr), u32::from(b.dbr)),
        ("PBR", u32::from(a.pbr), u32::from(b.pbr)),
        ("PC", u32::from(a.pc), u32::from(b.pc)),
        ("P", u32::from(u8::from(a.p)), u32::from(u8::from(b.p))),
    ];
    for (name, x, y) in regs {
        if x != y {
            out.push(format!("{name}: original {x:04X}, generated {y:04X}"));
        }
    }
    let (wa, wb) = (original.wram(), rust.wram());
    let wram: Vec<usize> = (0..wa.len()).filter(|i| wa[*i] != wb[*i]).collect();
    for &i in wram.iter().take(8) {
        out.push(format!("wram[7E:{i:05X}]: original {:02X}, generated {:02X}", wa[i], wb[i]));
    }
    if wram.len() > 8 {
        out.push(format!("... {} WRAM bytes in all", wram.len()));
    }
    let vram = original.vram().iter().zip(rust.vram()).filter(|(x, y)| x != y).count();
    if vram > 0 {
        out.push(format!("{vram} VRAM words differ"));
    }
    let oam = original.oam();
    let oam_n = oam.0.iter().zip(rust.oam().0).filter(|(x, y)| x != y).count()
        + oam.1.iter().zip(rust.oam().1).filter(|(x, y)| x != y).count();
    if oam_n > 0 {
        out.push(format!("{oam_n} OAM bytes differ"));
    }
    let cg = original.cgram().iter().zip(rust.cgram()).filter(|(x, y)| x != y).count();
    if cg > 0 {
        out.push(format!("{cg} CGRAM words differ"));
    }
    out
}

/// The generated function run on a copy of `m` until it hands over at the first block boundary
/// at or after `limit` instructions.
fn rust_after(
    m: &Machine,
    limit: u64,
    input: &mut dyn FnMut(u64) -> Joypad,
) -> Option<(Machine, ssd_oracle::RustRun)> {
    ssd_recomp::rt::STOP_AFTER.store(limit, Relaxed);
    let run = m.run_rust_unchecked(input, &mut NullAudio).ok();
    ssd_recomp::rt::STOP_AFTER.store(u64::MAX, Relaxed);
    run
}

/// The original on a copy of `m` after `count` instructions.
fn original_after(
    m: &Machine,
    count: u64,
    input: &mut dyn FnMut(u64) -> Joypad,
) -> Option<Machine> {
    let mut t = m.snapshot();
    t.set_hook_table(None);
    if count > 0 {
        let stops = Stops { instructions: Some(count), ..Stops::default() };
        t.run(&stops, input, &mut NullAudio, 8).ok()?;
    }
    Some(t)
}

/// Finds the first block of the generated function after which its state differs from the
/// original's after the same instructions, and says what the original ran there.
fn bisect(m: &Machine, input: &mut dyn FnMut(u64) -> Joypad) -> String {
    let mut text = String::new();
    let Some((_, whole)) = rust_after(m, u64::MAX, input) else {
        return "bisect: the generated function does not run\n".to_owned();
    };
    let total = whole.instructions.unwrap_or(0);
    let check = |limit: u64, input: &mut dyn FnMut(u64) -> Joypad| {
        let (rust, run) = rust_after(m, limit, input)?;
        let count = run.instructions.unwrap_or(0);
        // A block move counts one instruction more or less than the original's fetches.
        let mut first = None;
        for delta in [0_i64, -1, 1, -2, 2] {
            let n = count.saturating_add_signed(delta);
            let original = original_after(m, n, input)?;
            let mut diffs = state_differences(&original, &rust);
            let cycles = original.cpu_busy_cycles() - m.cpu_busy_cycles();
            if let Some(declared) = run.cost
                && declared != cycles
            {
                diffs.push(format!("cycles: original {cycles}, generated {declared}"));
            }
            if diffs.is_empty() {
                return Some((count, run.resume, diffs));
            }
            first.get_or_insert(diffs);
        }
        Some((count, run.resume, first.unwrap_or_default()))
    };
    let (mut good, mut bad) = (0_u64, total + 1);
    while bad - good > 1 {
        let mid = good + (bad - good) / 2;
        match check(mid, input) {
            Some((_, _, d)) if d.is_empty() => good = mid,
            Some(_) => bad = mid,
            None => return "bisect: a run failed\n".to_owned(),
        }
    }
    let _ = writeln!(text, "bisect: {total} instructions in the generated run");
    if bad > total {
        text.push_str("bisect: no difference after any block\n");
        return text;
    }
    let before = if good == 0 { Some((0, None, Vec::new())) } else { check(good, input) };
    let after = check(bad, input);
    if let (Some((k0, resume0, _)), Some((k1, resume1, diffs))) = (before, after) {
        let _ = writeln!(
            text,
            "bisect: equal after {k0} instructions (generated handed over at {}), different after {k1} (handed over at {}):",
            resume0.map_or("-".to_owned(), |p| format!("{p:06X}")),
            resume1.map_or("-".to_owned(), |p| format!("{p:06X}"))
        );
        for d in diffs {
            let _ = writeln!(text, "    {d}");
        }
        // The instructions the original ran in between.
        if let Some(mut t) = original_after(m, k0, input) {
            if let Some(address) =
                std::env::var("SSD_FUZZ_PEEK").ok().and_then(|v| u32::from_str_radix(&v, 16).ok())
            {
                let before = t.bus_read(address);
                let _ = writeln!(
                    text,
                    "    peek {address:06X} on the original after {k0}: {before:02X}"
                );
            }
            let d = usize::from(t.cpu_registers().d);
            let dp = &t.wram()[d..d + 16];
            let _ = writeln!(text, "    direct page at D = {dp:02X?}");
            let stops = Stops { instructions: Some(1), ..Stops::default() };
            for n in k0..k1 {
                let r = t.cpu_registers();
                let _ = writeln!(
                    text,
                    "    #{n} {:02X}:{:04X} A {:04X} X {:04X} Y {:04X} S {:04X} D {:04X} P {:02X}",
                    r.pbr,
                    r.pc,
                    r.a,
                    r.x,
                    r.y,
                    r.s,
                    r.d,
                    u8::from(r.p)
                );
                if t.run(&stops, input, &mut NullAudio, 8).is_err() {
                    break;
                }
            }
        }
    }
    text
}

/// Debugging (`SSD_FUZZ_PCS=1`): the addresses the pure original runs from this state, each the
/// first time, until it returns or after 200 000 instructions.
fn trace_original(m: &Machine, input: &mut dyn FnMut(u64) -> Joypad) {
    let mut t = m.snapshot();
    t.set_hook_table(None);
    let s0 = t.cpu_registers().s;
    let mut seen = BTreeSet::new();
    let mut order = Vec::new();
    let mut steps = 0_u64;
    let stops = Stops { instructions: Some(1), ..Stops::default() };
    while steps < 200_000 {
        let r = t.cpu_registers();
        let pc = (u32::from(r.pbr) << 16) | u32::from(r.pc);
        if seen.insert(pc) {
            order.push(format!("{pc:06X}"));
        }
        if r.s > s0 {
            break;
        }
        if let Err(error) = t.run(&stops, input, &mut NullAudio, 4) {
            order.push(format!("ERROR {error}"));
            break;
        }
        steps += 1;
    }
    eprintln!(
        "original: {steps} instructions, frames {}, S {:04X} (entry {s0:04X}), addresses: {}",
        t.frame_count(),
        t.cpu_registers().s,
        order.join(" ")
    );
}

/// What the watchdog sees: the iteration in progress and when it began.
static CURRENT: std::sync::Mutex<Option<(String, Instant)>> = std::sync::Mutex::new(None);

fn set_current(text: Option<String>) {
    *CURRENT.lock().unwrap_or_else(std::sync::PoisonError::into_inner) =
        text.map(|t| (t, Instant::now()));
}

/// A thread that ends the process (exit code 3) if one iteration takes more than 10 s, after
/// writing it to `<out>.hang`; a run with `--resume` then skips that entry.
fn start_watchdog(out_path: &Path) {
    let hang = out_path.with_extension("hang");
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(1));
            let stuck = CURRENT
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_ref()
                .filter(|(_, since)| since.elapsed() > Duration::from_secs(5))
                .map(|(text, _)| text.clone());
            if let Some(text) = stuck {
                eprintln!("fuzz-recomp: an iteration does not end: {text}");
                let mut old = std::fs::read_to_string(&hang).unwrap_or_default();
                old.push_str(&text);
                old.push('\n');
                let _ = std::fs::write(&hang, old);
                std::process::exit(3);
            }
        }
    });
}

fn parse_hex_list(text: &str) -> BTreeSet<u32> {
    text.split(',')
        .filter_map(|p| u32::from_str_radix(p.trim().trim_start_matches("0x"), 16).ok())
        .collect()
}

/// Parses the options of `fuzz-recomp` (before the replay paths).
///
/// # Errors
///
/// An unknown or incomplete option.
pub fn parse_options(args: &[&str]) -> Result<(Options, Vec<PathBuf>), String> {
    let mut o = Options {
        seed: 1,
        iters: 96,
        seeds: 2,
        seconds: 20.0,
        only: BTreeSet::new(),
        verbose: false,
        budget_frames: 6,
        resume: false,
        part: (0, 1),
        repro: None,
        no_hdma: false,
        no_nmi: false,
        at_frame: None,
    };
    let mut paths = Vec::new();
    let mut it = args.iter();
    while let Some(&arg) = it.next() {
        let mut value = |name: &str| -> Result<&str, String> {
            it.next().copied().ok_or_else(|| format!("{name} needs a value"))
        };
        match arg {
            "--seed" => o.seed = value("--seed")?.parse().map_err(|_| "bad --seed".to_owned())?,
            "--iters" => {
                o.iters = value("--iters")?.parse().map_err(|_| "bad --iters".to_owned())?;
            }
            "--seeds" => {
                o.seeds = value("--seeds")?.parse().map_err(|_| "bad --seeds".to_owned())?;
            }
            "--seconds" => {
                o.seconds = value("--seconds")?.parse().map_err(|_| "bad --seconds".to_owned())?;
            }
            "--budget" => {
                o.budget_frames =
                    value("--budget")?.parse().map_err(|_| "bad --budget".to_owned())?;
            }
            "--only" => o.only = parse_hex_list(value("--only")?),
            "--verbose" => o.verbose = true,
            "--resume" => o.resume = true,
            "--no-hdma" => o.no_hdma = true,
            "--no-nmi" => o.no_nmi = true,
            "--at-frame" => {
                o.at_frame =
                    Some(value("--at-frame")?.parse().map_err(|_| "bad --at-frame".to_owned())?);
            }
            "--repro" => {
                // KEY:TAG:ITERATION, as the divergence notes print them (key and tag in hex).
                let text = value("--repro")?;
                let parts: Vec<&str> = text.split(':').collect();
                let [key, tag, i] = parts.as_slice() else {
                    return Err("--repro needs KEY:TAG:ITERATION".to_owned());
                };
                let parsed = (
                    u32::from_str_radix(key, 16),
                    u64::from_str_radix(tag.trim_start_matches("0x"), 16),
                    i.parse::<usize>(),
                );
                let (Ok(key), Ok(tag), Ok(i)) = parsed else {
                    return Err("--repro needs KEY:TAG:ITERATION".to_owned());
                };
                o.repro = Some((key, tag, i));
                o.only.insert(key);
                o.verbose = true;
            }
            "--part" => {
                let text = value("--part")?;
                o.part = text
                    .split_once('/')
                    .and_then(|(i, n)| Some((i.parse().ok()?, n.parse().ok()?)))
                    .filter(|(i, n)| i < n)
                    .ok_or_else(|| "--part needs I/N with I < N".to_owned())?;
            }
            other if other.starts_with("--") => return Err(format!("unknown option {other}")),
            other => paths.push(PathBuf::from(other)),
        }
    }
    Ok((o, paths))
}

/// `fuzz-recomp pervcall <dir> <replay>`: plays the replay with every routine in `Verify` (each
/// call compared on its own) and prints the divergences; the states of those go to `dir` for
/// `repro`.
///
/// # Errors
///
/// The replay cannot be read or played.
pub fn pervcall(
    rom: &[u8],
    symbols: &Symbols,
    dir: &Path,
    replay_path: &Path,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    let replay = read_replay(replay_path, rom)?;
    let table = ssd_game::hook_table(symbols, Some(HookMode::Verify))?;
    let mut machine = replay.power_on(rom.to_vec())?;
    machine.set_hook_table(Some(table));
    std::fs::create_dir_all(dir)?;
    machine.set_divergence_dir(Some(dir.to_path_buf()));
    replay.play(&mut machine, &mut NullAudio)?;
    for d in machine.divergences() {
        write!(out, "{d}")?;
    }
    for (name, stats) in machine.hook_stats() {
        if stats.divergences > 0 || stats.cost_mismatch > 0 {
            writeln!(
                out,
                "{name}: {} divergences, {} cost mismatches of {} verified",
                stats.divergences, stats.cost_mismatch, stats.verified_ok
            )?;
        }
    }
    Ok(())
}

/// The seeds of `dir`: `KKKKKK-<replay>-<n>.state` files by entry, and the world states.
fn list_states(dir: &Path) -> (BTreeMap<u32, Vec<PathBuf>>, Vec<PathBuf>) {
    let mut seeds: BTreeMap<u32, Vec<PathBuf>> = BTreeMap::new();
    let mut worlds = Vec::new();
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|d| d.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    files.sort();
    for path in files {
        let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        if path.extension().is_none_or(|e| !e.eq_ignore_ascii_case("state")) {
            continue;
        }
        if name.starts_with("world-") {
            worlds.push(path);
        } else if let Some(key) =
            name.split('-').next().and_then(|k| u32::from_str_radix(k, 16).ok())
        {
            seeds.entry(key).or_default().push(path);
        }
    }
    (seeds, worlds)
}

/// A machine in the state a file holds.
fn load_machine(rom: &[u8], path: &Path) -> anyhow::Result<Machine> {
    let mut machine = Machine::new(rom.to_vec(), Box::new(MemorySaveStore::default()))?;
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    machine.load_state(&bytes)?;
    Ok(machine)
}

/// `fuzz-recomp harvest <dir> <replay>...`: plays the replays on the pure original with a
/// breakpoint at every generated entry and saves the machine at some arrivals (the first, the
/// 16th, the 256th ...; `--seeds` per entry in all) and at a few places of each replay (the
/// worlds the synthesised states start from).
///
/// # Errors
///
/// A replay cannot be read or played, or a state cannot be written.
pub fn harvest(
    rom: &[u8],
    opts: &Options,
    dir: &Path,
    replays: &[PathBuf],
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    let keys: Vec<u32> = TABLE
        .iter()
        .map(|e| e.key)
        .filter(|k| opts.only.is_empty() || opts.only.contains(k))
        .collect();
    for path in replays {
        let replay = read_replay(path, rom)?;
        let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let stem = name.trim_end_matches(".ssdreplay").to_owned();
        let mut machine = replay.power_on(rom.to_vec())?;
        for &key in &keys {
            machine.set_breakpoint(key);
        }
        let stops = Stops { breakpoint: true, ..Stops::default() };
        let mut hits: BTreeMap<u32, u64> = BTreeMap::new();
        let (mut next_world, mut world_n) = (600_u64, 0_u32);
        let t0 = Instant::now();
        let mut saved = 0;
        let mut entries: BTreeSet<u32> = BTreeSet::new();
        while machine.frame_count() < replay.frames() {
            let left = replay.frames() - machine.frame_count();
            let stopped = machine.run(&stops, &mut |f| replay.input(f), &mut NullAudio, left)?;
            match stopped {
                Stopped::Breakpoint { key } if opts.at_frame.is_some() => {
                    let frame = machine.frame_count();
                    if opts.at_frame.is_some_and(|f| frame >= f) {
                        std::fs::write(
                            dir.join(format!("{key:06X}-{stem}-at{frame}.state")),
                            machine.save_state(),
                        )?;
                        saved += 1;
                        entries.insert(key);
                        break;
                    }
                }
                Stopped::Breakpoint { key } => {
                    if machine.frame_count() >= next_world && world_n < 4 {
                        next_world = machine.frame_count() + 2500;
                        std::fs::write(
                            dir.join(format!("world-{stem}-{world_n}.state")),
                            machine.save_state(),
                        )?;
                        world_n += 1;
                    }
                    let n = hits.entry(key).or_default();
                    *n += 1;
                    let n = *n;
                    // Another harvest may be saving the same entry: look at the directory.
                    if matches!(n, 1 | 16 | 256 | 4096 | 65536)
                        && list_states(dir).0.get(&key).map_or(0, Vec::len) < opts.seeds as usize
                    {
                        std::fs::write(
                            dir.join(format!("{key:06X}-{stem}-{n}.state")),
                            machine.save_state(),
                        )?;
                        saved += 1;
                        entries.insert(key);
                    }
                }
                Stopped::Budget => break,
                _ => {}
            }
        }
        writeln!(
            out,
            "{name}: {} frames in {:.0} s, {saved} seeds saved for {} entries",
            replay.frames(),
            t0.elapsed().as_secs_f64(),
            entries.len(),
        )?;
        out.flush()?;
    }
    Ok(())
}

/// `fuzz-recomp run <dir> <out.tsv>`: fuzzes every entry from its seeds, then the entries that
/// are still unverified from synthesised states.
///
/// # Errors
///
/// A state cannot be read or the results cannot be written.
pub fn run(
    rom: &[u8],
    opts: &Options,
    dir: &Path,
    out_path: &Path,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    let mut fuzzer = Fuzzer::new(opts, out);
    start_watchdog(out_path);
    fuzzer.out_path = out_path.to_path_buf();
    if opts.resume {
        fuzzer.resume(out_path);
    }
    let (seeds, world_files) = list_states(dir);
    let mut count = 0;
    for (index, (key, files)) in seeds.iter().enumerate() {
        if !fuzzer.wants(*key) || !fuzzer.open(*key) || index % opts.part.1 != opts.part.0 {
            continue;
        }
        let share = (opts.iters / opts.seeds.max(1)).max(1);
        for (n, file) in files.iter().take(opts.seeds as usize).enumerate() {
            if !fuzzer.open(*key) {
                break;
            }
            let base = load_machine(rom, file)?;
            fuzzer.fuzz(*key, &base, share, (n as u64 + 1) << 40);
        }
        count += 1;
        fuzzer.results.write(out_path)?;
        if count % 100 == 0 {
            let seconds = fuzzer.started.elapsed().as_secs_f64();
            writeln!(fuzzer.out, "  {count} entries fuzzed from seeds, {seconds:.0} s")?;
            fuzzer.out.flush()?;
        }
    }
    let seconds = fuzzer.started.elapsed().as_secs_f64();
    writeln!(fuzzer.out, "seeded pass done: {count} entries, {seconds:.0} s")?;
    // Synthesised states for what is still unverified.
    let worlds: Vec<Machine> =
        world_files.iter().map(|p| load_machine(rom, p)).collect::<anyhow::Result<_>>()?;
    if !worlds.is_empty() {
        synth(&mut fuzzer, &worlds, out_path)?;
    }
    fuzzer.results.write(out_path)?;
    writeln!(fuzzer.out, "fuzz-recomp: done in {:.0} s", fuzzer.started.elapsed().as_secs_f64())?;
    Ok(())
}

/// Synthesised entry states for every entry no run verified.
fn synth(fuzzer: &mut Fuzzer<'_>, worlds: &[Machine], out_path: &Path) -> anyhow::Result<()> {
    let todo: Vec<u32> = fuzzer
        .long
        .keys()
        .copied()
        .filter(|k| {
            fuzzer.wants(*k)
                && fuzzer.open(*k)
                && (*k as usize) % fuzzer.opts.part.1 == fuzzer.opts.part.0
                && fuzzer.totals.get(k).is_none_or(|t| t.verified == 0 && t.diverged + t.cost == 0)
        })
        .collect();
    writeln!(fuzzer.out, "synthesised states for {} entries, {} worlds", todo.len(), worlds.len())?;
    for (n, key) in todo.into_iter().enumerate() {
        let long = fuzzer.long[&key];
        let mut rng = Rng::new(mix(fuzzer.opts.seed, u64::from(key) | (1 << 33)));
        let share = (fuzzer.opts.iters / 4).max(1);
        for round in 0..4_u64 {
            if !fuzzer.open(key) {
                break;
            }
            let world = &worlds[rng.below(worlds.len())];
            let mut m = world.snapshot();
            let mut regs = m.cpu_registers();
            regs.emulation_mode = false;
            // A call from some code at $00:8003.
            let mut s = regs.s;
            if long {
                m.bus_write(u32::from(s), 0x00);
                s = s.wrapping_sub(1);
            }
            for byte in [0x80_u8, 0x02] {
                m.bus_write(u32::from(s), byte);
                s = s.wrapping_sub(1);
            }
            regs.s = s;
            regs.pbr = (key >> 16) as u8;
            regs.pc = key as u16;
            let variants = fuzzer.variants[&key].clone();
            regs.p = StatusRegister::from((u8::from(regs.p) & !0x38) | variants[0].0);
            if variants[0].0 & 0x10 != 0 {
                regs.x &= 0x00FF;
                regs.y &= 0x00FF;
            }
            m.set_cpu_registers(regs);
            m.arm_arrival();
            let tag = (1_u64 << 50) | (round << 32) | u64::from(key);
            fuzzer.fuzz(key, &m, share, tag);
        }
        fuzzer.results.write(out_path)?;
        if n % 100 == 99 {
            writeln!(fuzzer.out, "  {} synthesised so far", n + 1)?;
            fuzzer.out.flush()?;
        }
    }
    Ok(())
}

// ----------------------------------------------------------------------------------------------
// The report

fn count(bytes: &[u8], range: std::ops::Range<usize>) -> usize {
    bytes[range].iter().filter(|b| **b != 0).count()
}

/// `fuzz-report [--cover f]... [--fuzz f.tsv]...`: the numbers.
///
/// # Errors
///
/// A file cannot be read.
pub fn report(args: &[&str], out: &mut dyn Write) -> anyhow::Result<()> {
    let mut cover: Vec<u8> = vec![0; COV.len()];
    let mut fuzz_cov: Vec<u8> = vec![0; COV.len()];
    let mut tally: BTreeMap<(u32, u8), Tally> = BTreeMap::new();
    let mut control: BTreeMap<(u32, u8), Tally> = BTreeMap::new();
    let mut notes: BTreeMap<u32, Vec<String>> = BTreeMap::new();
    let mut have_cover = false;
    let mut it = args.iter();
    while let Some(&arg) = it.next() {
        let file = PathBuf::from(it.next().context("an option needs a file")?);
        match arg {
            "--cover" => {
                have_cover = true;
                for (c, b) in cover.iter_mut().zip(read_cov(&file)?) {
                    *c |= b;
                }
            }
            "--control" => {
                for (k, t) in read_tallies(&file)? {
                    control.entry(k).or_default().add(&t);
                }
                for (c, b) in fuzz_cov.iter_mut().zip(read_cov(&file.with_extension("cov"))?) {
                    *c |= b;
                }
            }
            "--fuzz" => {
                for (k, t) in read_tallies(&file)? {
                    tally.entry(k).or_default().add(&t);
                }
                for (c, b) in fuzz_cov.iter_mut().zip(read_cov(&file.with_extension("cov"))?) {
                    *c |= b;
                }
                if let Ok(text) = std::fs::read_to_string(file.with_extension("div")) {
                    let mut current = 0;
                    for line in text.lines() {
                        if let Some(rest) = line.strip_prefix("== ") {
                            current = u32::from_str_radix(rest.trim(), 16).unwrap_or(0);
                            notes.entry(current).or_default().push(String::new());
                        } else if let Some(last) =
                            notes.get_mut(&current).and_then(|l| l.last_mut())
                        {
                            last.push_str(line);
                            last.push('\n');
                        }
                    }
                }
            }
            other => anyhow::bail!("unknown option {other}"),
        }
    }
    let routines: BTreeSet<u32> = TABLE.iter().map(|e| e.key).collect();
    let total_blocks: usize = COV.len();
    writeln!(
        out,
        "generated: {} function variants, {} routines (entries), {} blocks",
        VARIANTS.len(),
        routines.len(),
        total_blocks
    )?;
    if have_cover {
        let range = |v: &Variant| v.cov_base as usize..(v.cov_base + v.blocks) as usize;
        let never: Vec<&Variant> =
            VARIANTS.iter().filter(|v| count(&cover, range(v)) == 0).collect();
        let entry_ran: BTreeSet<u32> =
            VARIANTS.iter().filter(|v| count(&cover, range(v)) > 0).map(|v| v.entry).collect();
        let entries_never: BTreeSet<u32> =
            routines.iter().copied().filter(|e| !entry_ran.contains(e)).collect();
        let blocks_ran = cover.iter().filter(|b| **b != 0).count();
        let insns_never: u32 = never.iter().map(|v| v.insns).sum();
        let insns_all: u32 = VARIANTS.iter().map(|v| v.insns).sum();
        writeln!(out, "the replays run:")?;
        writeln!(
            out,
            "  variants never executed: {} of {} ({} instructions of {})",
            never.len(),
            VARIANTS.len(),
            insns_never,
            insns_all
        )?;
        writeln!(
            out,
            "  routines (entries) never executed: {} of {}",
            entries_never.len(),
            routines.len()
        )?;
        writeln!(
            out,
            "  blocks never executed: {} of {} ({:.1} %)",
            total_blocks - blocks_ran,
            total_blocks,
            100.0 * (total_blocks - blocks_ran) as f64 / total_blocks as f64
        )?;
        let partial = VARIANTS
            .iter()
            .filter(|v| {
                let c = count(&cover, range(v));
                c > 0 && c < v.blocks as usize
            })
            .count();
        writeln!(out, "  variants run only in part: {partial}")?;
    }
    if !tally.is_empty() {
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
        enum Verdict {
            Verified,
            VerifiedQuiet,
            Artefact,
            Divergent,
            Skipped,
            NotTried,
            SyncCode,
        }
        let ok = |t: &Tally| t.verified > 0 && t.diverged + t.cost == 0;
        let bad = |t: &Tally| t.diverged + t.cost > 0;
        let reason_of = |t: &Tally| {
            [
                (t.hung, "an iteration hangs (garbage execution)"),
                (t.early, "drops the caller's return address (an early exit, not a unit)"),
                (t.interrupted, "an interrupt (NMI, wait on the frame) enters the original"),
                (t.no_return, "the original does not return in the budget (waits)"),
                (t.handed_over, "the generated code hands over to the original"),
                (t.other, "the call cannot run"),
            ]
            .into_iter()
            .max_by_key(|(n, _)| *n)
            .map_or("none", |(_, r)| r)
        };
        let range = |v: &Variant| v.cov_base as usize..(v.cov_base + v.blocks) as usize;
        let mut counts: BTreeMap<Verdict, usize> = BTreeMap::new();
        let mut counts_never: BTreeMap<Verdict, usize> = BTreeMap::new();
        let mut by_reason: BTreeMap<&str, usize> = BTreeMap::new();
        let mut divergent_list = Vec::new();
        let mut artefact_list = Vec::new();
        let mut skipped_list: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        for v in VARIANTS {
            let key = (v.entry, v.mx);
            let (main, ctl) = (tally.get(&key), control.get(&key));
            let name = format!("{:06X}/{:02X}", v.entry, v.mx);
            let verdict = match (main, ctl) {
                _ if !routines.contains(&v.entry) => Verdict::SyncCode,
                (Some(m), c) if bad(m) => match c {
                    Some(c) if ok(c) => Verdict::Artefact,
                    _ => Verdict::Divergent,
                },
                (Some(m), _) if ok(m) => Verdict::Verified,
                (_, Some(c)) if bad(c) => Verdict::Divergent,
                (_, Some(c)) if ok(c) => Verdict::VerifiedQuiet,
                (Some(m), c) => {
                    let mut t = *m;
                    if let Some(c) = c {
                        t.add(c);
                    }
                    *by_reason.entry(reason_of(&t)).or_default() += 1;
                    skipped_list.entry(reason_of(&t)).or_default().push(name.clone());
                    Verdict::Skipped
                }
                (None, Some(c)) => {
                    *by_reason.entry(reason_of(c)).or_default() += 1;
                    skipped_list.entry(reason_of(c)).or_default().push(name.clone());
                    Verdict::Skipped
                }
                (None, None) => Verdict::NotTried,
            };
            *counts.entry(verdict).or_default() += 1;
            if have_cover && count(&cover, range(v)) == 0 {
                *counts_never.entry(verdict).or_default() += 1;
            }
            let (d, c) = (main.map_or(0, |t| t.diverged), main.map_or(0, |t| t.cost));
            match verdict {
                Verdict::Divergent => divergent_list.push(format!("{name} ({d} diff, {c} cost)")),
                Verdict::Artefact => artefact_list.push(format!("{name} ({d} diff, {c} cost)")),
                _ => {}
            }
        }
        let line = |m: &BTreeMap<Verdict, usize>| {
            let n = |v: Verdict| m.get(&v).copied().unwrap_or(0);
            format!(
                "verified {} (of them {} only with the NMI and HDMA off), divergent {}, \
                 divergent only with HDMA or the NMI on {}, skipped {}, not tried {}, \
                 sync-point code {}",
                n(Verdict::Verified) + n(Verdict::VerifiedQuiet),
                n(Verdict::VerifiedQuiet),
                n(Verdict::Divergent),
                n(Verdict::Artefact),
                n(Verdict::Skipped),
                n(Verdict::NotTried),
                n(Verdict::SyncCode)
            )
        };
        writeln!(out, "the fuzzer, per function variant: {}", line(&counts))?;
        if have_cover {
            let never: usize = counts_never.values().sum();
            writeln!(out, "  of the {never} variants no replay runs: {}", line(&counts_never))?;
        }
        for (reason, n) in &by_reason {
            writeln!(out, "    skipped, {reason}: {n}")?;
        }
        let total_iters: u64 =
            tally.values().chain(control.values()).map(|t| u64::from(t.iters)).sum();
        let total_ok: u64 =
            tally.values().chain(control.values()).map(|t| u64::from(t.verified)).sum();
        writeln!(out, "  iterations: {total_iters}, verified calls: {total_ok}")?;
        let insn_diffs: u32 = tally.values().map(|t| t.insns).sum();
        writeln!(out, "  verified calls whose instruction count differs: {insn_diffs}")?;
        for d in &divergent_list {
            writeln!(out, "  divergent: {d}")?;
        }
        for d in &artefact_list {
            writeln!(out, "  divergent only with HDMA or the NMI on: {d}")?;
        }
        if have_cover {
            let mut union = cover.clone();
            for (u, f) in union.iter_mut().zip(&fuzz_cov) {
                *u |= *f;
            }
            let ran = union.iter().filter(|b| **b != 0).count();
            let replays_only = cover.iter().filter(|b| **b != 0).count();
            let fuzz_only = fuzz_cov.iter().filter(|b| **b != 0).count();
            let never_union = VARIANTS.iter().filter(|v| count(&union, range(v)) == 0).count();
            writeln!(
                out,
                "  blocks run: replays {replays_only}, fuzzer {fuzz_only}, together {ran} of {total_blocks}; variants run by neither: {never_union}"
            )?;
        }
        for (reason, list) in &skipped_list {
            if list.len() <= 60 {
                writeln!(out, "  skipped ({reason}): {}", list.join(" "))?;
            }
        }
        for (entry, list) in &notes {
            for text in list.iter().take(1) {
                let head = text.lines().take(10).collect::<Vec<_>>().join("\n");
                writeln!(out, "-- {entry:06X}\n{head}")?;
            }
        }
    }
    Ok(())
}
