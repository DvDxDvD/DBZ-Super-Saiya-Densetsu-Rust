//! Reading the Mesen traces of `tools/re/run_trace.sh` (`.work/re/<run>/`): `exec.csv` (one row
//! per executed address), `calls.csv` and `jumps.csv`. Shared by `disasm` (observed widths) and
//! `routines` (the routine table). The files are derived from the game and never tracked.

use anyhow::{Context, bail};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One row of `exec.csv`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exec {
    pub m8: bool,
    pub x8: bool,
    /// Emulation mode at the first sample.
    pub e: bool,
    pub d: u16,
    pub dbr: u8,
    pub hits: u64,
    /// Every `(m8, x8, e)` the samples saw.
    pub modes: Vec<(bool, bool, bool)>,
    /// Every data bank the samples saw.
    pub dbrs: Vec<u8>,
}

/// A `calls.csv` or `jumps.csv` row: `target,caller,op`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edge {
    pub target: u32,
    pub source: u32,
    pub op: String,
}

/// One trace run.
#[derive(Debug, Default)]
pub struct Run {
    pub name: String,
    /// By physical address (`& $7FFFFF`).
    pub exec: BTreeMap<u32, Exec>,
    pub calls: Vec<Edge>,
    pub jumps: Vec<Edge>,
    pub frames: u64,
}

/// Mesen's bus addresses folded onto the physical ones used by `re/symbols.toml`.
#[must_use]
pub fn phys(address: u32) -> u32 {
    address & 0x7F_FFFF
}

fn hex(field: &str) -> anyhow::Result<u32> {
    u32::from_str_radix(field.trim(), 16).with_context(|| format!("`{field}` is not hexadecimal"))
}

/// Parses the text of an `exec.csv`.
///
/// # Errors
///
/// A malformed row (the message names its line).
pub fn parse_exec(text: &str) -> anyhow::Result<BTreeMap<u32, Exec>> {
    let mut rows = BTreeMap::new();
    for (i, line) in text.lines().enumerate().skip(1) {
        if line.trim().is_empty() {
            continue;
        }
        let row = (|| -> anyhow::Result<(u32, Exec)> {
            let f: Vec<&str> = line.split(',').collect();
            if f.len() < 8 {
                bail!("expected 8 columns");
            }
            let mut modes = Vec::new();
            let mut dbrs = Vec::new();
            for mode in f[7].split(';').filter(|m| !m.is_empty()) {
                let (flags, rest) = mode.split_once('/').context("mode without `/`")?;
                let b: Vec<bool> = flags.chars().map(|c| c == '1').collect();
                if b.len() != 3 {
                    bail!("mode flags `{flags}`");
                }
                modes.push((b[0], b[1], b[2]));
                if let Some((_, dbr)) = rest.split_once('/') {
                    dbrs.push(dbr.parse::<u8>().unwrap_or(0));
                }
            }
            modes.sort_unstable();
            modes.dedup();
            dbrs.sort_unstable();
            dbrs.dedup();
            Ok((
                phys(hex(f[0])?),
                Exec {
                    m8: f[1] == "1",
                    x8: f[2] == "1",
                    e: f[3] == "1",
                    d: hex(f[4])? as u16,
                    dbr: hex(f[5])? as u8,
                    hits: f[6].parse().context("hits")?,
                    modes,
                    dbrs,
                },
            ))
        })()
        .with_context(|| format!("exec.csv line {}", i + 1))?;
        rows.insert(row.0, row.1);
    }
    Ok(rows)
}

/// Parses the text of a `calls.csv` or `jumps.csv`.
///
/// # Errors
///
/// A malformed row.
pub fn parse_edges(text: &str) -> anyhow::Result<Vec<Edge>> {
    let mut edges = Vec::new();
    for (i, line) in text.lines().enumerate().skip(1) {
        if line.trim().is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split(',').collect();
        if f.len() < 3 {
            bail!("line {}: expected 3 columns", i + 1);
        }
        edges.push(Edge {
            target: phys(hex(f[0])?),
            source: phys(hex(f[1])?),
            op: f[2].to_owned(),
        });
    }
    Ok(edges)
}

/// Loads the run in `dir` (`exec.csv` is required, the others optional).
///
/// # Errors
///
/// `exec.csv` is missing or a file is malformed.
pub fn load_run(dir: &Path) -> anyhow::Result<Run> {
    let name = dir.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let read = |file: &str| std::fs::read_to_string(dir.join(file));
    let exec = read("exec.csv").with_context(|| format!("reading {}/exec.csv", dir.display()))?;
    let mut run = Run { name, exec: parse_exec(&exec)?, ..Run::default() };
    if let Ok(text) = read("calls.csv") {
        run.calls = parse_edges(&text).with_context(|| format!("{}/calls.csv", dir.display()))?;
    }
    if let Ok(text) = read("jumps.csv") {
        run.jumps = parse_edges(&text).with_context(|| format!("{}/jumps.csv", dir.display()))?;
    }
    if let Ok(text) = read("summary.txt") {
        run.frames = text
            .lines()
            .find_map(|line| line.strip_prefix("frames="))
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0);
    }
    Ok(run)
}

/// The run directories of `work_re` (`.work/re`): those with an `exec.csv`, by name.
#[must_use]
pub fn discover(work_re: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(work_re) else { return Vec::new() };
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.join("exec.csv").is_file())
        .collect();
    dirs.sort();
    dirs
}

/// The widths the traces agree on at each address: `(m8, x8)` for addresses seen only in native
/// mode with one width pair in every sample.
#[must_use]
pub fn observed_widths(runs: &[Run]) -> std::collections::HashMap<u32, (bool, bool)> {
    let mut seen: BTreeMap<u32, Vec<(bool, bool, bool)>> = BTreeMap::new();
    for run in runs {
        for (&pc, row) in &run.exec {
            seen.entry(pc).or_default().extend(row.modes.iter().copied());
        }
    }
    seen.into_iter()
        .filter_map(|(pc, mut modes)| {
            modes.sort_unstable();
            modes.dedup();
            match modes.as_slice() {
                [(m8, x8, false)] => Some((pc, (*m8, *x8))),
                _ => None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_rows_are_parsed_with_their_sampled_modes() {
        let text = "pc,m8,x8,e,d,dbr,hits,modes\n\
                    808B0E,1,0,0,0000,00,12,100/0/0;110/0/0\n\
                    001223,1,0,0,0100,7E,3,100/256/126\n";
        let rows = parse_exec(text).unwrap();
        let row = &rows[&0x00_8B0E];
        assert_eq!((row.m8, row.x8, row.e, row.hits), (true, false, false, 12));
        assert_eq!(row.modes, [(true, false, false), (true, true, false)]);
        let ram = &rows[&0x00_1223];
        assert_eq!((ram.d, ram.dbr, ram.dbrs.clone()), (0x100, 0x7E, vec![126]));
        assert!(parse_exec("pc,m8\n1,2\n").is_err());
    }

    #[test]
    fn edges_fold_mirrors_and_widths_need_one_native_mode() {
        let edges = parse_edges("target,caller,op\n85B572,05C38A,JSR\n").unwrap();
        assert_eq!(edges, [Edge { target: 0x05_B572, source: 0x05_C38A, op: "JSR".to_owned() }]);
        let exec = parse_exec(
            "pc,m8,x8,e,d,dbr,hits,modes\n\
             008000,1,0,0,0000,00,1,100/0/0\n\
             008002,1,0,0,0000,00,1,100/0/0;110/0/0\n\
             008004,1,1,1,0000,00,1,111/0/0\n",
        )
        .unwrap();
        let run = Run { exec, ..Run::default() };
        let seen = observed_widths(&[run]);
        assert_eq!(seen.get(&0x8000), Some(&(true, false)));
        assert_eq!(seen.get(&0x8002), None, "two width pairs");
        assert_eq!(seen.get(&0x8004), None, "emulation mode");
    }
}
