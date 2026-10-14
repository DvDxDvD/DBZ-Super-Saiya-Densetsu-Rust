//! `causes [replay]` (feature `histogram`): which instructions of a replay still run as original
//! code with the recompiled table, and why. Plays the replay in `Rust` mode with the opcode
//! fetches of the original code counted by address, then sorts them with the map the generator
//! wrote (`.work/recomp/map.txt`): an address the generated code hands over at (with its reason),
//! an address inside generated code (a timing or frame-end hand-over), RAM code, or a ROM address
//! that was never generated.

use crate::shared::read_replay;
use ssd_oracle::{HookMode, NullAudio, Symbols};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Write;
use std::path::Path;

/// Plays `path` and prints the causes.
///
/// # Errors
///
/// The replay or the map cannot be read, or the run fails.
pub fn causes(
    rom: &[u8],
    symbols: &Symbols,
    root: &Path,
    path: &Path,
    write_entries: bool,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    let map_text = std::fs::read_to_string(root.join(".work").join("recomp").join("map.txt"))?;
    let mut map: BTreeMap<u32, &str> = BTreeMap::new();
    for line in map_text.lines() {
        if let Some((pc, reason)) = line.split_once(' ') {
            map.insert(u32::from_str_radix(pc, 16)?, reason);
        }
    }
    let replay = read_replay(path, rom)?;
    let table = ssd_game::hook_table(symbols, Some(HookMode::Rust))?;
    let mut machine = replay.power_on(rom.to_vec())?;
    machine.set_hook_table(Some(table));
    machine.enable_pc_histogram();
    replay.play(&mut machine, &mut NullAudio)?;
    if std::env::var_os("SSD_DUMP_WRAM").is_some() {
        let w = machine.wram();
        writeln!(out, "WRAM 1200: {:02x?}", &w[0x1200..0x1240])?;
        let needle = &w[0x1210..0x1230];
        if let Some(pos) = rom.windows(needle.len()).position(|win| win == needle) {
            writeln!(out, "ROM has the WRAM bytes at file offset {pos:#X}")?;
        }
    }
    let histogram = machine.pc_histogram();
    let coverage = machine.coverage();
    let total: u64 = histogram.iter().sum();
    writeln!(
        out,
        "{}: {} frames, original {} instructions, rust (estimated) {}, coverage {:.2}%",
        path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
        replay.frames(),
        total,
        coverage.rust_instructions_estimated,
        coverage.percent()
    )?;
    let mut by_cause: BTreeMap<String, u64> = BTreeMap::new();
    let mut by_pc: Vec<(u64, u32, String)> = Vec::new();
    for (pc, &n) in histogram.iter().enumerate() {
        if n == 0 {
            continue;
        }
        let pc = pc as u32;
        let cause = match map.get(&pc) {
            Some(&"ok") => {
                "inside generated code (timing, frame end or DMA store hand-over)".to_owned()
            }
            Some(reason) => format!("hand-over: {reason}"),
            None if pc >= 0x7E_0000 || ((pc & 0xFFFF) < 0x2000 && pc < 0x40_0000) => {
                "RAM code".to_owned()
            }
            None => "ROM never generated".to_owned(),
        };
        *by_cause.entry(cause.clone()).or_default() += n;
        by_pc.push((n, pc, cause));
    }
    if write_entries {
        // The ROM addresses nothing generated covers, for `recompile` to start functions at.
        let file = root.join(".work").join("re").join("extra-entries.txt");
        let mut all: std::collections::BTreeSet<u32> = std::fs::read_to_string(&file)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| u32::from_str_radix(l.trim(), 16).ok())
            .collect();
        all.extend(
            by_pc.iter().filter(|(_, _, cause)| cause == "ROM never generated").map(|p| p.1),
        );
        let text = all.iter().fold(String::new(), |mut text, a| {
            let _ = writeln!(text, "{a:06X}");
            text
        });
        std::fs::write(&file, text)?;
        writeln!(out, "  {} addresses in {}", all.len(), file.display())?;
    }
    let mut causes: Vec<_> = by_cause.into_iter().collect();
    causes.sort_by_key(|c| std::cmp::Reverse(c.1));
    for (cause, n) in &causes {
        writeln!(out, "  {:6.2}%  {n:>12}  {cause}", 100.0 * *n as f64 / total.max(1) as f64)?;
    }
    by_pc.sort_by_key(|p| std::cmp::Reverse(p.0));
    writeln!(out, "  top addresses:")?;
    for (n, pc, cause) in by_pc.iter().take(40) {
        writeln!(out, "    {pc:06X} {n:>12}  {cause}")?;
    }
    // By 128-byte block for the never-generated and inside-generated parts.
    let mut blocks: BTreeMap<(u32, bool), u64> = BTreeMap::new();
    for (n, pc, cause) in &by_pc {
        *blocks.entry((pc >> 7, cause.starts_with("inside"))).or_default() += n;
    }
    let mut blocks: Vec<_> = blocks.into_iter().collect();
    blocks.sort_by_key(|b| std::cmp::Reverse(b.1));
    writeln!(out, "  top 128-byte blocks:")?;
    for ((block, inside), n) in blocks.iter().take(25) {
        writeln!(
            out,
            "    {:06X} {n:>12}  {}",
            block << 7,
            if *inside { "generated" } else { "other" }
        )?;
    }
    let hook_stats = machine.hook_stats();
    let runs: u64 = hook_stats.iter().map(|(_, s)| s.rust_runs).sum();
    let fallbacks: u64 = hook_stats.iter().map(|(_, s)| s.timing_fallback).sum();
    let calls: u64 = hook_stats.iter().map(|(_, s)| s.calls).sum();
    writeln!(
        out,
        "  dispatches {calls}, rust runs {runs}, declined at entry (timing) {fallbacks}"
    )?;
    let mut top: Vec<_> = hook_stats.iter().filter(|(_, s)| s.timing_fallback > 0).collect();
    top.sort_by_key(|t| std::cmp::Reverse(t.1.timing_fallback));
    for (name, s) in top.iter().take(15) {
        writeln!(
            out,
            "    {name:12} calls {:>8} runs {:>8} declined {:>8}",
            s.calls, s.rust_runs, s.timing_fallback
        )?;
    }
    writeln!(
        out,
        "  declined at entry by bit length of cycles to the next event: {:?}",
        machine.until_declined()
    )?;
    let mut stats = ssd_recomp::rt::bail_stats();
    stats.sort_by_key(|s| std::cmp::Reverse(s.1));
    let mut by_why: BTreeMap<&str, u64> = BTreeMap::new();
    for ((_, why), n) in &stats {
        *by_why.entry(why).or_default() += n;
    }
    writeln!(out, "  hand-overs by cause: {by_why:?}")?;
    for ((pc, why), n) in stats.iter().take(30) {
        writeln!(out, "    {pc:06X} {n:>10}  {why}")?;
    }
    Ok(())
}
