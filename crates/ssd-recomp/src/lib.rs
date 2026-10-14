//! A 65816 to Rust static recompiler (milestone M3).
//!
//! [`generate`] turns the code of the ROM into Rust functions over [`rt::R`]: one function per
//! function variant (entry address and M/X widths), with the cycles of every instruction
//! charged as the hardware spends them and a timing-window check before every basic block.
//! Whatever it cannot follow statically (computed jumps without known targets, `WAI`, `XCE`,
//! software interrupts, code outside the ROM) hands control back to the original code at that
//! instruction. The generated source is derived from the ROM and is never tracked (`CONTRIBUTING.md`); `cargo run --release -p xtask -- recompile` writes it under `.work/recomp/`.

pub mod analysis;
pub mod decode;
pub mod emit;
pub mod rt;

pub use analysis::{FuncKey, Hints};

use analysis::Analysis;
use decode::Rom;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

/// A block the original can arrive at: M/X bits, function, block number, cost, returns long.
type MidSource = (u8, FuncKey, u32, u32, bool);

/// What [`generate`] made.
#[derive(Debug, Default)]
pub struct Output {
    /// The Rust source (an `include!` target).
    pub source: String,
    pub report: Report,
    /// One line per instruction address in the generated functions: `PPPPPP ok`, or the reason
    /// the generated code hands over there (`xtask causes` reads it).
    pub map: String,
}

/// Counts for the report of the `recompile` command.
#[derive(Debug, Default)]
pub struct Report {
    /// Function variants generated.
    pub variants: usize,
    /// Distinct entry addresses registered as routines.
    pub routines: usize,
    /// Instructions in the generated functions.
    pub instructions: usize,
    /// Entries that could not be generated, with the reason.
    pub rejected: Vec<(u32, &'static str)>,
    /// Places where control is handed back to the original, by reason.
    pub bails: BTreeMap<&'static str, u32>,
    /// Bytes of generated source.
    pub bytes: usize,
    /// Addresses inside generated functions the original can be met at (`MID_TABLE`).
    pub mid_entries: usize,
}

/// Generates the Rust for the functions the hints name and everything they call.
///
/// # Panics
///
/// Never on valid input; the analysis recurses deeply, so call it on a thread with a large stack.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn generate(rom: &[u8], hints: &Hints) -> Output {
    let mut a = Analysis::new(Rom(rom), hints);
    let mut report = Report::default();
    // Roots.
    let mut roots: Vec<FuncKey> = Vec::new();
    // The sync points are generated as functions too (the code of the idle loop), but are no
    // routines: the machine reports sync events there.
    let starts: BTreeSet<u32> = hints.entries.union(&hints.stops).copied().collect();
    for &entry in &starts {
        let mxs: BTreeSet<u8> = hints
            .entry_mx
            .get(&entry)
            .cloned()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| BTreeSet::from([0x20]));
        for mx in mxs {
            roots.push(FuncKey { entry, mx });
        }
    }
    for &key in &roots {
        a.analyse(key);
    }
    // Code the replays ran that nothing above reaches: its own functions, unless an earlier one
    // of these covers it.
    let mut covered: std::collections::HashSet<u32> =
        a.graphs().values().flat_map(|g| g.nodes.values().map(|n| n.insn.pc)).collect();
    let mut starts = starts;
    for &root in &hints.extra_roots {
        if covered.contains(&root) {
            continue;
        }
        a.entries.insert(root);
        starts.insert(root);
        for mx in [0x00, 0x10, 0x20, 0x30] {
            let key = FuncKey { entry: root, mx };
            a.analyse(key);
            covered.extend(a.graphs()[&key].nodes.values().map(|n| n.insn.pc));
        }
    }
    // Everything analysed (callees included), grouped by entry address.
    let mut by_entry: BTreeMap<u32, Vec<FuncKey>> = BTreeMap::new();
    for (key, g) in a.graphs() {
        if g.viable {
            by_entry.entry(key.entry).or_default().push(*key);
        } else if starts.contains(&key.entry) {
            report.rejected.push((key.entry, if g.too_big { "too big" } else { "not code" }));
        }
    }
    for keys in by_entry.values_mut() {
        keys.sort();
    }
    // The return kind of each address: the majority over its own returns and those of the
    // functions it jumps to (a dispatcher returns through its handlers), the traces' on a tie.
    let mut own: BTreeMap<u32, [u32; 2]> = BTreeMap::new();
    let mut tails: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    for (entry, keys) in &by_entry {
        let counts = own.entry(*entry).or_default();
        for k in keys {
            let g = &a.graphs()[k];
            counts[0] += g.returns[0];
            counts[1] += g.returns[1];
            for node in g.nodes.values() {
                match &node.next {
                    analysis::Next::Tail(t) => {
                        tails.entry(*entry).or_default().insert(t.entry);
                    }
                    analysis::Next::IndJump { targets } => {
                        for (_, to) in targets {
                            if let analysis::JumpTo::Func(t) = to {
                                tails.entry(*entry).or_default().insert(t.entry);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    // The entries whose code (callees and jump targets included) reads the status registers
    // `$4210-$4212`: a poll lands the read a few master cycles from the original's, so the flags
    // the read sets (V, Z) are not compared in `Verify`.
    let mut polls: BTreeMap<u32, bool> = BTreeMap::new();
    let mut calls: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    for (entry, keys) in &by_entry {
        let reaches = calls.entry(*entry).or_default();
        let poll = polls.entry(*entry).or_default();
        for k in keys {
            for node in a.graphs()[k].nodes.values() {
                let i = &node.insn;
                if matches!(i.mode, decode::Mode::Abs | decode::Mode::Long)
                    && (0x4210..=0x4212).contains(&(i.opr & 0xFFFF))
                    && !matches!(i.mn, "STA" | "STX" | "STY" | "STZ")
                {
                    *poll = true;
                }
                match &node.next {
                    analysis::Next::Call { callee: analysis::Callee::Direct(t), .. }
                    | analysis::Next::Tail(t) => {
                        reaches.insert(t.entry);
                    }
                    analysis::Next::Call { callee: analysis::Callee::Table(table), .. } => {
                        reaches.extend(table.iter().map(|(_, t)| t.entry));
                    }
                    analysis::Next::IndJump { targets } => {
                        for (_, to) in targets {
                            if let analysis::JumpTo::Func(t) = to {
                                reaches.insert(t.entry);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    loop {
        let mut changed = false;
        for (entry, reaches) in &calls {
            if !polls[entry] && reaches.iter().any(|t| polls.get(t).copied().unwrap_or(false)) {
                polls.insert(*entry, true);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let mut total = own.clone();
    for _ in 0..8 {
        let previous = total.clone();
        for (entry, set) in &tails {
            let mut sum = own[entry];
            for t in set {
                if let Some(c) = previous.get(t) {
                    sum[0] = sum[0].saturating_add(c[0]);
                    sum[1] = sum[1].saturating_add(c[1]);
                }
            }
            total.insert(*entry, sum);
        }
    }
    let mut kinds: BTreeMap<u32, bool> = BTreeMap::new();
    for entry in by_entry.keys() {
        let [rts, rtl] = total[entry];
        let long = match rtl.cmp(&rts) {
            std::cmp::Ordering::Greater => true,
            std::cmp::Ordering::Less => false,
            std::cmp::Ordering::Equal => hints.ret_long.get(entry).copied().unwrap_or(false),
        };
        kinds.insert(*entry, long);
    }
    // Which addresses are generated, and where the generated code hands over.
    let mut map_pcs: BTreeMap<u32, &'static str> = BTreeMap::new();
    for keys in by_entry.values() {
        for k in keys {
            for node in a.graphs()[k].nodes.values() {
                let reason = match &node.next {
                    analysis::Next::Bail { reason, .. } => *reason,
                    analysis::Next::IndJump { targets } if targets.is_empty() => {
                        "indirect jump target unknown"
                    }
                    _ => "ok",
                };
                let slot = map_pcs.entry(node.insn.pc).or_insert("ok");
                if reason != "ok" {
                    *slot = reason;
                }
            }
        }
    }
    let mut map = String::new();
    for (pc, reason) in &map_pcs {
        let _ = writeln!(map, "{pc:06X} {}", reason.replace(' ', "_"));
    }
    // Functions, by bank.
    let mut banks: BTreeMap<u8, String> = BTreeMap::new();
    let mut variants_table = String::new();
    let mut cov_blocks = 0_u32;
    let mut entry_costs: BTreeMap<FuncKey, u32> = BTreeMap::new();
    // Where the original can arrive in generated code: address -> (M/X bits, function, block,
    // cost, returns long).
    let mut mids: BTreeMap<u32, Vec<MidSource>> = BTreeMap::new();
    for (entry, keys) in &by_entry {
        let long = kinds[entry];
        for k in keys {
            let g = &a.graphs()[k];
            let e = emit::emit_function(*k, g, long, cov_blocks);
            let _ = writeln!(
                variants_table,
                "Variant {{ entry: 0x{:06X}, mx: 0x{:02X}, cov_base: {cov_blocks}, blocks: {}, insns: {}, name: \"{}\" }},",
                k.entry,
                k.mx,
                e.blocks,
                e.instructions,
                k.name()
            );
            cov_blocks += e.blocks;
            report.variants += 1;
            report.instructions += e.instructions;
            for (r, n) in e.bails {
                *report.bails.entry(r).or_default() += n;
            }
            entry_costs.insert(*k, e.entry_cost);
            for l in &e.leaders {
                mids.entry(l.pc).or_default().push((l.mx, *k, l.block, l.cost, long));
            }
            banks.entry((entry >> 16) as u8).or_default().push_str(&e.source);
        }
    }
    for (r, n) in &a.bails {
        *report.bails.entry(r).or_default() += n;
    }

    // Entry wrappers and the table.
    let mut wrappers = String::new();
    let mut table = String::new();
    for (entry, keys) in &by_entry {
        if hints.stops.contains(entry) {
            continue;
        }
        let long = kinds[entry];
        let flags: u8 = if polls[entry] { 0xBD } else { 0xFF };
        let pulls = if long { 3 } else { 2 };
        let bank = entry >> 16;
        let _ = writeln!(wrappers, "pub fn w_{entry:06X}(ctx: &mut Ctx<'_>) {{");
        let _ = writeln!(
            wrappers,
            "let mut r = R::load(ctx);
r.entry = 0x{entry:06X};"
        );
        wrappers.push_str("let ret = match r.mx {\n");
        for k in keys {
            let _ = writeln!(wrappers, "0x{:02X} => {}(ctx, &mut r),", k.mx, k.name());
        }
        let _ = writeln!(wrappers, "mx => r.bail(0x{entry:06X}, mx),");
        wrappers.push_str("};\n");
        let _ = writeln!(wrappers, "r.finish(ctx, ret, {pulls});\n}}");
        let _ = writeln!(wrappers, "pub fn b_{entry:06X}(ctx: &Ctx<'_>) -> u64 {{");
        wrappers.push_str("if ctx.pbr() != 0x");
        let _ = writeln!(wrappers, "{bank:02X} {{ return u64::MAX; }}");
        wrappers.push_str("match (ctx.m8(), ctx.x8()) {\n");
        for k in keys {
            let _ = writeln!(
                wrappers,
                "({}, {}) => {},",
                k.mx & 0x20 != 0,
                k.mx & 0x10 != 0,
                entry_costs[k]
            );
        }
        wrappers.push_str("_ => u64::MAX,\n}\n}\n");
        let _ = writeln!(
            table,
            "GenEntry {{ key: 0x{entry:06X}, name: \"rc_{entry:06X}\", func: w_{entry:06X}, bound: b_{entry:06X}, long: {long}, flags: 0x{flags:02X} }},"
        );
        report.routines += 1;
    }

    let mut source = String::from(
        "// Generated by `cargo run --release -p xtask -- recompile` from the ROM. Never tracked.\n\
         use ssd_oracle::Ctx;\nuse ssd_recomp::rt::*;\n\n\
         macro_rules! g {\n\
         ($r:ident, $c:ident, $w:expr, $n:expr, $pc:expr, $mx:expr) => {\n\
         if !$r.gate($c, $w, $n) { return $r.bail($pc, $mx); }\n\
         };\n}\n\n\
         macro_rules! cv {\n\
         ($i:expr) => {\n\
         COV[$i].store(1, ::std::sync::atomic::Ordering::Relaxed)\n\
         };\n}\n\n",
    );
    for (bank, text) in &banks {
        let _ = writeln!(source, "pub use self::b{bank:02X}::*;");
        let _ = text;
    }
    for (bank, text) in &banks {
        let _ = writeln!(source, "pub mod b{bank:02X} {{\nuse super::*;");
        source.push_str(text);
        source.push_str("}\n");
    }
    source.push_str("pub mod entry {\nuse super::*;\n");
    source.push_str(&wrappers);
    source.push_str("}\npub use self::entry::*;\n");
    let _ = writeln!(
        source,
        "pub static COV: [::std::sync::atomic::AtomicU8; {cov_blocks}] = [const {{ ::std::sync::atomic::AtomicU8::new(0) }}; {cov_blocks}];"
    );
    source.push_str("pub static VARIANTS: &[Variant] = &[\n");
    source.push_str(&variants_table);
    source.push_str("];\n");
    source.push_str("pub static TABLE: &[GenEntry] = &[\n");
    source.push_str(&table);
    source.push_str("];\n");
    // The blocks the original can arrive at, for the addresses that are no entry or sync point.
    let mut mid_data = String::new();
    let mut mid_table = String::new();
    let mut n_mid = 0_usize;
    for (pc, variants) in &mids {
        if by_entry.contains_key(pc) || hints.stops.contains(pc) || pc >> 16 >= 0x80 {
            continue;
        }
        let long = variants[0].4;
        let mut seen: BTreeSet<u8> = BTreeSet::new();
        let mut list = String::new();
        for (mx, k, block, cost, l) in variants {
            if *l != long || !seen.insert(*mx) {
                continue;
            }
            let _ = write!(
                list,
                "MidVariant {{ mx: 0x{mx:02X}, func: {}, block: {block}, cost: {cost} }}, ",
                k.name()
            );
        }
        let _ = writeln!(
            mid_data,
            "MidEntry {{ key: 0x{pc:06X}, long: {long}, variants: &[{list}] }},"
        );
        let _ = writeln!(
            mid_table,
            "GenEntry {{ key: 0x{pc:06X}, name: \"rm_{pc:06X}\", func: mw::<{n_mid}>, bound: mb::<{n_mid}>, long: {long}, flags: 0xFF }},"
        );
        n_mid += 1;
    }
    report.mid_entries = n_mid;
    let _ = writeln!(source, "pub static MIDS: [MidEntry; {n_mid}] = [");
    source.push_str(&mid_data);
    source.push_str("];\n");
    source.push_str(
        "pub fn mw<const I: usize>(ctx: &mut Ctx<'_>) { run_mid(ctx, &MIDS[I]) }\n\
         pub fn mb<const I: usize>(ctx: &Ctx<'_>) -> u64 { mid_bound(ctx, &MIDS[I]) }\n",
    );
    source.push_str("pub static MID_TABLE: &[GenEntry] = &[\n");
    source.push_str(&mid_table);
    source.push_str("];\n");
    report.bytes = source.len();
    Output { source, report, map }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bank of NOPs with `code` at `$00:8000`.
    fn image(code: &[u8]) -> Vec<u8> {
        let mut rom = vec![0xEA; 0x8000];
        rom[..code.len()].copy_from_slice(code);
        rom
    }

    fn hints(entries: &[u32]) -> Hints {
        let mut h = Hints::default();
        h.entries.extend(entries.iter().copied());
        h
    }

    #[test]
    fn a_leaf_function_becomes_a_routine() {
        // LDA #$01 ; STA $10 ; RTS
        let rom = image(&[0xA9, 0x01, 0x85, 0x10, 0x60]);
        let out = generate(&rom, &hints(&[0x8000]));
        assert_eq!(out.report.routines, 1);
        assert!(out.source.contains("pub fn f_008000_2("), "{}", out.source);
        assert!(out.source.contains("GenEntry { key: 0x008000"));
        assert!(out.source.contains("long: false"));
    }

    #[test]
    fn a_call_becomes_a_direct_call_of_the_callee() {
        // 8000: JSR $8010 ; RTL    8010: INX ; RTS
        let mut rom = image(&[0x20, 0x10, 0x80, 0x6B]);
        rom[0x10..0x12].copy_from_slice(&[0xE8, 0x60]);
        let out = generate(&rom, &hints(&[0x8000]));
        assert_eq!(out.report.variants, 2);
        assert!(out.source.contains("f_008010_2(ctx, r)"));
        assert!(out.source.contains("long: true"), "the caller returns with RTL");
    }

    #[test]
    fn what_cannot_be_followed_hands_over() {
        // WAI: the code goes on after it where the original comes back.
        let rom = image(&[0xCB, 0x60]);
        let out = generate(&rom, &hints(&[0x8000]));
        assert!(out.source.contains("return r.bail(0x008000"));
        assert_eq!(out.report.bails.get("WAI"), Some(&1));
        assert!(
            out.source.contains("rm_008001"),
            "the instruction after it is a place to re-enter"
        );
        // A sync point is checked before it is run through.
        let mut h = hints(&[0x8000]);
        h.stops.insert(0x8002);
        let rom = image(&[0xE8, 0xE8, 0xE8, 0x60]);
        let out = generate(&rom, &h);
        assert!(out.source.contains("ctx.sync_due(0x008002)"));
    }

    #[test]
    fn widths_follow_rep_and_sep() {
        // REP #$30 ; LDA #$1234 ; RTS
        let rom = image(&[0xC2, 0x30, 0xA9, 0x34, 0x12, 0x60]);
        let out = generate(&rom, &hints(&[0x8000]));
        assert!(out.source.contains("r.a = 0x1234u16"), "{}", out.source);
    }

    #[test]
    fn a_jump_table_next_to_the_jump_is_scanned() {
        // 8000: ASL A ; TAX ; JMP ($8006,X)   8006: .word $8010, $8020, $8030   (the first target
        // bounds the table at $8010)   8010: RTS   8020: RTS   8030: RTS
        let mut rom = image(&[0x0A, 0xAA, 0x7C, 0x06, 0x80, 0xEA]);
        rom[6..12].copy_from_slice(&[0x10, 0x80, 0x20, 0x80, 0x30, 0x80]);
        rom[0x10] = 0x60;
        rom[0x20] = 0x60;
        rom[0x30] = 0x60;
        let out = generate(&rom, &hints(&[0x8000]));
        for target in ["0x8010 =>", "0x8020 =>", "0x8030 =>"] {
            assert!(
                out.source.contains(target),
                "{target} in
{}",
                out.source
            );
        }
        // A target the table does not list hands over.
        assert!(out.source.contains("_ => return r.bail(0x000000 | t"), "{}", out.source);
    }

    #[test]
    fn every_block_is_a_place_to_re_enter() {
        // 8000: LDX #$10 ; DEX ; BNE $8002 ; RTS
        let rom = image(&[0xA2, 0x10, 0xCA, 0xD0, 0xFD, 0x60]);
        let out = generate(&rom, &hints(&[0x8000]));
        assert!(out.report.mid_entries >= 2, "{:?}", out.report);
        assert!(out.source.contains("rm_008002"), "the loop head");
        assert!(out.source.contains("pub static MID_TABLE"));
    }

    #[test]
    fn rti_pulls_p_and_the_address_and_hands_over_there() {
        let rom = image(&[0x40]);
        let out = generate(&rom, &hints(&[0x8000]));
        assert!(out.source.contains("r.set_p(p)"), "{}", out.source);
        assert!(out.report.bails.is_empty(), "{:?}", out.report.bails);
    }

    #[test]
    fn long_stretches_are_cut_into_several_blocks() {
        let mut code = vec![0xEA; 40];
        code.push(0x60);
        let out = generate(&image(&code), &hints(&[0x8000]));
        assert!(out.source.matches("g!(r, ctx").count() > 3, "{}", out.source);
    }

    #[test]
    fn a_store_through_a_pointer_is_checked_against_the_event_registers() {
        // STA ($10),Y ; STA $4208 with a 16-bit accumulator ; RTS
        let rom = image(&[0x91, 0x10, 0xC2, 0x20, 0x8D, 0x08, 0x42, 0x60]);
        let out = generate(&rom, &hints(&[0x8000]));
        assert!(out.source.contains("r.wr8_sys("), "{}", out.source);
        assert!(out.source.contains("r.wr16_sys("), "{}", out.source);
    }
}
