//! `bench`: how fast the real ROM runs headless, for the record, where the routines are called.
//! The battle script (`ssd_game::scripts::battle_input`) is played unhooked from power-on to
//! frame 4 600, in the battle, and the machine's state is saved there. Then, for each mode
//! (unhooked, and every routine of `re/symbols.toml` in `Rust`), a fresh machine of the canonical
//! ROM of the cache loads that state, gets its hook table and plays 600 frames of the script,
//! timed. The runs are repeated three times, the order of the two modes alternating, and the
//! median run of each mode is printed with the frames per second of every run. Nothing is
//! asserted: a machine's speed is not a property of the code.
//!
//! The `Rust` line also gives the hooked calls per second and how many calls ran in Rust and how
//! many fell back to the original (the timing window), so a change in speed can be told from a
//! change in what runs as Rust.
//!
//! Exit 0 with the numbers, 1 on an error (no ROM cache, the symbols, a failed run); a command
//! line with arguments exits 64.

use ssd_game::scripts::battle_input;
use ssd_oracle::{HookTable, Machine, MemorySaveStore, NullAudio};
use std::io::Write;
use std::time::{Duration, Instant};

/// The frame the state is saved at: the battle has started (the script flies into one at about
/// frame 4 500).
pub const START_FRAME: u64 = 4_600;
/// Frames of each timed run.
pub const FRAMES: u64 = 600;
/// Timed runs of each mode.
pub const REPEATS: usize = 3;

/// What one timed run came to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Measurement {
    pub frames: u64,
    pub elapsed: Duration,
    /// Dispatched calls that ran in Rust, over all routines (0 without hooks).
    pub rust_runs: u64,
    /// Dispatched calls that fell back to the original because a timing event could come
    /// within the routine's cost.
    pub timing_fallbacks: u64,
    /// Hook hits dispatched, over all routines.
    pub calls: u64,
}

impl Measurement {
    /// Frames per second of wall-clock time.
    #[must_use]
    pub fn frames_per_second(&self) -> f64 {
        self.frames as f64 / self.elapsed.as_secs_f64()
    }

    /// Hooked calls dispatched per second of wall-clock time.
    #[must_use]
    pub fn calls_per_second(&self) -> f64 {
        self.calls as f64 / self.elapsed.as_secs_f64()
    }
}

/// The two modes of the command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Unhooked,
    Rust,
}

/// The order the modes run in during `repeat` (0 = the first): they alternate, so a machine that
/// warms up or slows down favours neither.
#[must_use]
pub fn order(repeat: usize) -> [Mode; 2] {
    if repeat.is_multiple_of(2) {
        [Mode::Unhooked, Mode::Rust]
    } else {
        [Mode::Rust, Mode::Unhooked]
    }
}

/// The state of `rom` after the battle script has played unhooked from power-on to `frame`.
///
/// # Errors
///
/// `rom` is not a ROM the core takes, or the run fails.
pub fn battle_state(rom: &[u8], frame: u64) -> anyhow::Result<Vec<u8>> {
    let mut machine = Machine::new(rom.to_vec(), Box::new(MemorySaveStore::default()))?;
    while machine.frame_count() < frame {
        machine.run_frame(battle_input(machine.frame_count()), &mut NullAudio)?;
    }
    Ok(machine.save_state())
}

/// Loads `state` on a fresh machine of `rom`, installs `table` if given and times `frames`
/// frames of the battle script from the state's frame (loading and installing are not timed).
///
/// # Errors
///
/// `rom` is not a ROM the core takes, `state` is not a state of it, or the run fails.
pub fn measure(
    rom: &[u8],
    state: &[u8],
    table: Option<HookTable>,
    frames: u64,
) -> anyhow::Result<Measurement> {
    let mut machine = Machine::new(rom.to_vec(), Box::new(MemorySaveStore::default()))?;
    machine.load_state(state)?;
    machine.set_hook_table(table);
    let first = machine.frame_count();
    let start = Instant::now();
    while machine.frame_count() < first + frames {
        machine.run_frame_with(&mut battle_input, &mut NullAudio)?;
    }
    let elapsed = start.elapsed();
    let stats = machine.hook_stats();
    Ok(Measurement {
        frames: machine.frame_count() - first,
        elapsed,
        rust_runs: stats.iter().map(|(_, s)| s.rust_runs).sum(),
        timing_fallbacks: stats.iter().map(|(_, s)| s.timing_fallback).sum(),
        calls: stats.iter().map(|(_, s)| s.calls).sum(),
    })
}

/// The median run (by time) of `runs`, which are not empty.
fn median(runs: &[Measurement]) -> Measurement {
    let mut sorted = runs.to_vec();
    sorted.sort_by_key(|run| run.elapsed);
    sorted[sorted.len() / 2]
}

/// The whole command: the battle state at `start`, then `repeats` timed runs of `frames` frames
/// of each mode (`table` in `Rust`), printed to `out` as `unhooked` and `rust` lines. Returns
/// the runs of each mode, in the order they ran.
///
/// # Errors
///
/// As for [`battle_state`] and [`measure`], or `out` cannot be written.
pub fn bench(
    rom: &[u8],
    table: &HookTable,
    start: u64,
    frames: u64,
    repeats: usize,
    out: &mut dyn Write,
) -> anyhow::Result<[Vec<Measurement>; 2]> {
    writeln!(
        out,
        "bench: {frames} frames of the battle script from frame {start} (state played unhooked), \
         {repeats} runs of each mode, median shown"
    )?;
    let state = battle_state(rom, start)?;
    let (mut unhooked, mut rust) = (Vec::new(), Vec::new());
    for repeat in 0..repeats {
        for mode in order(repeat) {
            match mode {
                Mode::Unhooked => unhooked.push(measure(rom, &state, None, frames)?),
                Mode::Rust => rust.push(measure(rom, &state, Some(table.clone()), frames)?),
            }
        }
    }
    if !unhooked.is_empty() {
        writeln!(out, "  {}", line("unhooked", &unhooked))?;
    }
    if !rust.is_empty() {
        let run = median(&rust);
        writeln!(
            out,
            "  {}   {:.0} hooked calls/s (routines: {}; {} Rust runs, {} timing fallbacks in {} calls)",
            line("rust", &rust),
            run.calls_per_second(),
            table.len(),
            run.rust_runs,
            run.timing_fallbacks,
            run.calls
        )?;
    }
    Ok([unhooked, rust])
}

/// `unhooked   1.234 s   486.2 frames/s   (runs: 486.2 480.1 490.3)`: the median run, then the
/// frames per second of every run in the order they ran.
fn line(label: &str, runs: &[Measurement]) -> String {
    let run = median(runs);
    let all: Vec<String> = runs.iter().map(|r| format!("{:.1}", r.frames_per_second())).collect();
    format!(
        "{label:<9} {:>7.3} s {:>9.1} frames/s   (runs: {})",
        run.elapsed.as_secs_f64(),
        run.frames_per_second(),
        all.join(" ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{LEAF, leaf_rom};
    use ssd_oracle::{Ctx, HookAction, HookEntry, HookMode, ReturnKind};

    /// `INC $10`, as the leaf of the fixture ROM does.
    fn inc_counter(ctx: &mut Ctx<'_>) {
        let value = ctx.read_dp8(0x10).wrapping_add(1);
        ctx.write_dp8(0x10, value);
        ctx.set_nz8(value);
        ctx.set_cost(80);
    }

    fn rust_leaf() -> HookTable {
        [HookEntry {
            name: "leaf".to_owned(),
            key: LEAF,
            mode: HookMode::Rust,
            action: HookAction::Routine { func: inc_counter, ret: ReturnKind::Rts },
            entry_m8: Some(true),
            entry_x8: Some(false),
            flags_mask: 0xFF,
            verify_sample: 1,
            bound: None,
            budget_frames: None,
            cycles: Some(80),
        }]
        .into_iter()
        .collect()
    }

    #[test]
    fn the_modes_alternate_so_neither_always_goes_first() {
        assert_eq!(order(0), [Mode::Unhooked, Mode::Rust]);
        assert_eq!(order(1), [Mode::Rust, Mode::Unhooked]);
        assert_eq!(order(2), order(0));
        // Every mode goes first as often as second over an even number of repeats.
        let firsts = (0..4).filter(|&repeat| order(repeat)[0] == Mode::Rust).count();
        assert_eq!(firsts, 2);
    }

    #[test]
    fn a_run_starts_at_the_state_and_counts_its_frames_and_what_ran_in_rust() {
        let rom = leaf_rom();
        let state = battle_state(&rom, 3).unwrap();

        let plain = measure(&rom, &state, None, 4).unwrap();
        assert_eq!(plain.frames, 4);
        assert_eq!((plain.rust_runs, plain.timing_fallbacks, plain.calls), (0, 0, 0));
        assert!(plain.frames_per_second() > 0.0);

        let hooked = measure(&rom, &state, Some(rust_leaf()), 4).unwrap();
        assert_eq!(hooked.frames, 4);
        // No NMI, no HDMA: no timing event, so every call of the leaf runs in Rust.
        assert!(hooked.rust_runs > 100, "{hooked:?}");
        assert_eq!((hooked.timing_fallbacks, hooked.calls), (0, hooked.rust_runs));
        assert!(hooked.calls_per_second() > 0.0);
    }

    #[test]
    fn a_state_of_another_rom_is_an_error() {
        let state = battle_state(&leaf_rom(), 2).unwrap();
        let mut other = leaf_rom();
        other[0x7000] = 0x12; // never run
        ssd_rom::fix_lorom_checksum(&mut other);
        assert!(measure(&other, &state, None, 1).is_err());
    }

    #[test]
    fn the_median_run_is_the_middle_one_by_time() {
        let run = |millis| Measurement {
            frames: 10,
            elapsed: Duration::from_millis(millis),
            rust_runs: 0,
            timing_fallbacks: 0,
            calls: 0,
        };
        assert_eq!(median(&[run(30), run(10), run(20)]).elapsed, Duration::from_millis(20));
        assert_eq!(median(&[run(5)]).elapsed, Duration::from_millis(5));
    }

    #[test]
    fn the_command_prints_a_line_for_each_mode_and_returns_every_run() {
        let mut out = Vec::new();
        let [unhooked, rust] = bench(&leaf_rom(), &rust_leaf(), 3, 3, 3, &mut out).unwrap();
        assert_eq!((unhooked.len(), rust.len()), (3, 3));
        assert!(unhooked.iter().all(|run| run.frames == 3 && run.calls == 0));
        assert!(rust.iter().all(|run| run.frames == 3 && run.rust_runs > 50));
        let text = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3, "{text}");
        assert!(
            lines[0].starts_with("bench: 3 frames of the battle script from frame 3"),
            "{text}"
        );
        assert!(lines[1].trim_start().starts_with("unhooked") && lines[1].contains("frames/s"));
        let listed = lines[1].split("(runs: ").nth(1).map(|runs| runs.trim_end_matches(')'));
        assert_eq!(listed.map(|runs| runs.split(' ').count()), Some(3), "three runs: {text}");
        assert!(lines[2].trim_start().starts_with("rust"), "{text}");
        assert!(lines[2].contains("hooked calls/s") && lines[2].contains("routines: 1;"), "{text}");
    }
}
