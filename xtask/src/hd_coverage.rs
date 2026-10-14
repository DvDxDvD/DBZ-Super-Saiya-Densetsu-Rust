//! `hd-coverage`: for each replay, the share of the visible BG and OBJ pixels whose character the
//! HD pack binds to an asset, and the characters that are not bound. Plays every frame with the
//! PPU's render log on and renders it with `ssd-hd`, bound to the pack's index; a pixel is counted
//! when the main screen shows a character (a non-transparent BG or object pixel). Mode 7 lines are
//! left out: the plane is not made of characters.

use crate::shared::{read_replay, render_table};
use ssd_hd::pack::Pack;
use ssd_hd::{Binder, Frame, Renderer, Source, WIDTH};
use ssd_oracle::{FrameLog, NullAudio};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// The command line of `hd-coverage`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    /// Replay files, or directories of them; empty: `.work/replays`.
    pub paths: Vec<PathBuf>,
    /// Replays played at once.
    pub jobs: usize,
    /// Whether to list the assets the replays bind, with their visible pixels.
    pub assets: bool,
}

/// Parses the arguments after `hd-coverage`: `[--jobs N] [--assets] [replay|dir...]`.
///
/// # Errors
///
/// An unknown option, or `--jobs` without a number.
pub fn parse_args(args: &[&str]) -> Result<Args, String> {
    let mut parsed = Args { paths: Vec::new(), jobs: 4, assets: false };
    let mut args = args.iter();
    while let Some(&arg) = args.next() {
        match arg {
            "--jobs" => {
                parsed.jobs = args
                    .next()
                    .and_then(|value| value.parse::<usize>().ok())
                    .filter(|&n| n > 0)
                    .ok_or_else(|| "--jobs needs a number".to_owned())?;
            }
            "--assets" => parsed.assets = true,
            option if option.starts_with('-') => return Err(format!("unknown option `{option}`")),
            path => parsed.paths.push(PathBuf::from(path)),
        }
    }
    Ok(parsed)
}

/// The characters a replay showed that the pack does not bind, by bit depth, object or BG, and
/// VRAM word address, with the visible pixels each one drew.
pub type Unbound = BTreeMap<(bool, u8, u16), u64>;

/// What checking one replay found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    pub name: String,
    pub frames: u64,
    /// Frames the renderer refused, and the frames without a log.
    pub refused: u64,
    /// Visible non-transparent BG and OBJ pixels, outside Mode 7 lines.
    pub pixels: u64,
    /// Of those, the pixels whose character is bound to an asset.
    pub bound: u64,
    pub unbound: Unbound,
    /// The bound pixels of each asset of the pack, by its index.
    pub by_asset: BTreeMap<u32, u64>,
}

impl Outcome {
    /// The share of the pixels bound, in percent (100 when there are none).
    #[must_use]
    pub fn percent(&self) -> f64 {
        if self.pixels == 0 { 100.0 } else { self.bound as f64 * 100.0 / self.pixels as f64 }
    }
}

/// The scanlines of `log` that are in Mode 7 on any of their parts.
fn mode_7_lines(log: &FrameLog, height: usize) -> Vec<bool> {
    let mut lines = vec![false; height];
    for entry in &log.entries {
        let line = usize::from(entry.scanline);
        if entry.regs.bg_mode == 7 && (1..=height).contains(&line) {
            lines[line - 1] = true;
        }
    }
    lines
}

/// Adds the pixels of one rendered frame to `outcome`.
fn tally(outcome: &mut Outcome, log: &FrameLog, frame: &Frame) {
    let mode_7 = mode_7_lines(log, frame.height);
    for (at, source) in frame.sources.iter().enumerate() {
        if mode_7[at / WIDTH] {
            continue;
        }
        let Source::Char { object, bpp, address, tile } = *source else { continue };
        outcome.pixels += 1;
        match tile {
            Some(tile) => {
                outcome.bound += 1;
                *outcome.by_asset.entry(tile.asset).or_default() += 1;
            }
            None => *outcome.unbound.entry((object, bpp, address)).or_default() += 1,
        }
    }
}

/// Plays `path` and tallies its frames against the pack `pack`.
///
/// # Errors
///
/// The replay cannot be read or played.
pub fn check_replay(rom: &[u8], pack: &Pack, path: &Path) -> anyhow::Result<Outcome> {
    let replay = read_replay(path, rom)?;
    let name = path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    let mut outcome = Outcome { name, frames: replay.frames(), ..Outcome::default() };
    let mut machine = replay.power_on(rom.to_vec())?;
    machine.set_line_log(true);
    let mut renderer = Renderer::new();
    let mut binder = Binder::new(&pack.index);
    while machine.frame_count() < replay.frames() {
        machine.run_frame_with(&mut |f| replay.input(f), &mut NullAudio)?;
        let Some(log) = machine.frame_log() else {
            outcome.refused += 1;
            continue;
        };
        match renderer.render_bound(log, &mut binder) {
            Ok(frame) => tally(&mut outcome, log, &frame),
            Err(_) => outcome.refused += 1,
        }
    }
    Ok(outcome)
}

/// Checks the replays (`args`) against the pack in `dir` and writes the report; the code is 0.
///
/// # Errors
///
/// The pack cannot be read, a replay cannot be read or played, or the report cannot be written.
pub fn run(
    rom: &[u8],
    dir: &Path,
    files: &[PathBuf],
    args: &Args,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    let pack = Pack::open(dir)?;
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<(usize, anyhow::Result<Outcome>)>> = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..args.jobs.min(files.len()) {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(path) = files.get(index) else { break };
                    let outcome = check_replay(rom, &pack, path);
                    results.lock().expect("a worker panicked").push((index, outcome));
                }
            });
        }
    });
    let mut results = results.into_inner().expect("a worker panicked");
    results.sort_by_key(|(index, _)| *index);

    let mut rows = Vec::new();
    let mut details = String::new();
    let mut lowest = f64::MAX;
    let mut total = 0.0;
    let mut assets: BTreeMap<u32, u64> = BTreeMap::new();
    for (_, outcome) in results {
        let outcome = outcome?;
        let percent = outcome.percent();
        lowest = lowest.min(percent);
        total += percent;
        for (&asset, &pixels) in &outcome.by_asset {
            *assets.entry(asset).or_default() += pixels;
        }
        let distinct = outcome.unbound.keys().count();
        rows.push(vec![
            outcome.name.clone(),
            outcome.frames.to_string(),
            outcome.refused.to_string(),
            outcome.pixels.to_string(),
            format!("{percent:.2}"),
            distinct.to_string(),
            (outcome.pixels - outcome.bound).to_string(),
        ]);
        if !outcome.unbound.is_empty() {
            let mut by_pixels: Vec<_> = outcome.unbound.iter().collect();
            by_pixels.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
            let shown: Vec<String> = by_pixels
                .iter()
                .take(8)
                .map(|((object, bpp, address), pixels)| {
                    let kind = if *object { "OBJ" } else { "BG" };
                    format!("{kind} {bpp}bpp ${address:04X}: {pixels}")
                })
                .collect();
            let _ = writeln!(details, "  {}: unbound {}", outcome.name, shown.join(", "));
        }
    }
    let header = ["replay", "frames", "refused", "pixels", "bound %", "unbound", "unbound px"];
    write!(out, "{}", render_table(&header, &rows))?;
    write!(out, "{details}")?;
    let count = rows.len().max(1);
    writeln!(
        out,
        "hd-coverage: {} replays, bound min {lowest:.2} %, mean {:.2} %",
        rows.len(),
        total / count as f64
    )?;
    if args.assets {
        writeln!(out, "hd-coverage: assets bound (visible pixels over the replays):")?;
        for (asset, pixels) in &assets {
            writeln!(out, "  {:<36} {pixels:>9}", pack.index.asset(*asset).id)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_line_takes_jobs_and_replays() {
        let args = parse_args(&["--jobs", "2", "a.ssdreplay", "dir"]).unwrap();
        assert_eq!((args.jobs, args.paths.len()), (2, 2));
        assert!(parse_args(&["--jobs"]).unwrap_err().contains("needs a number"));
        assert!(parse_args(&["--fast"]).unwrap_err().contains("unknown option"));
    }

    #[test]
    fn a_replay_with_no_visible_character_is_fully_bound() {
        assert_eq!(Outcome::default().percent(), 100.0);
        let half = Outcome { pixels: 4, bound: 2, ..Outcome::default() };
        assert_eq!(half.percent(), 50.0);
    }
}
