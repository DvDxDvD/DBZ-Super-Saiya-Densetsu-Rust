//! `coverage [--exclude name[,name...]] [replay]`: what share of the instructions a replay
//! executes now runs as Rust (default replay `.work/replays/battle.ssdreplay`).
//!
//! Two passes over the replay, each from power-on on the canonical ROM of the cache:
//!
//! 1. every routine of `re/symbols.toml` in `Verify`: the original runs and each call is
//!    checked, which measures the mean instructions of the original per call
//!    (`HookStats::mean_instructions`);
//! 2. every routine in `Rust` with those means: the machine executes the original code that is
//!    left (its opcode fetches) and counts the Rust runs, each standing for its routine's mean
//!    (`Machine::coverage`; the formula is in `ssd_oracle`'s coverage module).
//!
//! It prints each routine's calls, Rust runs, timing fallbacks, mean instructions and estimated
//! Rust instructions, then the totals and the percent.
//!
//! The percent counts instructions, not game logic: routines called from the idle wait between
//! frames dominate it (the two RNGs, about 250 calls a frame on the battle replay), and it grows
//! with the idle time per frame. `--exclude` names the routines to leave out for a second,
//! conservative figure, `(rust - excluded) / (rust - excluded + original)`; none are left out by
//! default.
//!
//! Exit 0 with a result (a divergence of the `Verify` pass is a warning: `verify-replays` is the
//! check); a bad command line (an unknown option or routine name included) exits 64, an
//! unreadable replay or a failed run exits 1.

use crate::shared::{read_replay, render_table};
use anyhow::Context;
use ssd_oracle::{Coverage, HookMode, HookStats, HookTable, NullAudio, Replay, Symbols};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// The command line of `coverage`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    /// The replay file; `None`: `.work/replays/battle.ssdreplay` of the repository.
    pub replay: Option<PathBuf>,
    /// The routines to leave out of the second figure (`--exclude`), in the order given.
    pub exclude: Vec<String>,
}

/// Parses the arguments after `coverage`: `[--exclude name[,name...]] [replay]`, the option in
/// any place and as often as wanted.
///
/// # Errors
///
/// An unknown option, `--exclude` without names or with an empty one, or more than one replay.
pub fn parse_args(args: &[&str]) -> Result<Args, String> {
    let mut parsed = Args { replay: None, exclude: Vec::new() };
    let mut args = args.iter();
    while let Some(&arg) = args.next() {
        match arg {
            "--exclude" => {
                let Some(&names) = args.next() else {
                    return Err("--exclude needs routine names: name[,name...]".to_owned());
                };
                for name in names.split(',') {
                    if name.is_empty() {
                        return Err(format!("--exclude `{names}` has an empty routine name"));
                    }
                    parsed.exclude.push(name.to_owned());
                }
            }
            option if option.starts_with('-') => return Err(format!("unknown option `{option}`")),
            replay if parsed.replay.is_none() => parsed.replay = Some(PathBuf::from(replay)),
            replay => return Err(format!("one replay only (`{replay}` is a second one)")),
        }
    }
    Ok(parsed)
}

/// Checks that every name of `exclude` is a routine of `symbols`.
///
/// # Errors
///
/// The first name that is not, with the routines there are.
pub fn check_excluded(symbols: &Symbols, exclude: &[String]) -> Result<(), String> {
    let known: Vec<&str> = symbols.routines().iter().map(|routine| routine.name.as_str()).collect();
    match exclude.iter().find(|name| !known.contains(&name.as_str())) {
        Some(name) => Err(format!(
            "--exclude: `{name}` is not a routine of the symbols file ({})",
            known.join(", ")
        )),
        None => Ok(()),
    }
}

/// What the two passes measured.
pub struct Measured {
    /// The counters of the `Verify` pass (pass 1), by routine.
    pub verify_stats: Vec<(String, HookStats)>,
    /// The counters of the `Rust` pass (pass 2), by routine.
    pub rust_stats: Vec<(String, HookStats)>,
    /// The coverage of the `Rust` pass.
    pub coverage: Coverage,
}

/// The two passes: `replay` played from power-on on `rom` with `verify` installed (learning the
/// means), then with `rust` and those means (counting).
///
/// # Errors
///
/// The replay is not for `rom` or this emulator core, or a run fails.
pub fn measure(
    rom: &[u8],
    replay: &Replay,
    verify: HookTable,
    rust: HookTable,
) -> anyhow::Result<Measured> {
    let mut learning = replay.power_on(rom.to_vec())?;
    learning.set_hook_table(Some(verify));
    replay.play(&mut learning, &mut NullAudio).context("the Verify pass")?;
    let verify_stats = learning.hook_stats();
    let means: Vec<(String, f64)> =
        verify_stats.iter().map(|(name, stats)| (name.clone(), stats.mean_instructions)).collect();

    let mut counting = replay.power_on(rom.to_vec())?;
    counting.set_hook_table(Some(rust));
    counting.set_mean_instructions(&means);
    replay.play(&mut counting, &mut NullAudio).context("the Rust pass")?;
    Ok(Measured { verify_stats, rust_stats: counting.hook_stats(), coverage: counting.coverage() })
}

/// The routines of `symbols` in `mode` (the Rust pass warns about those without `cycles`).
///
/// # Errors
///
/// A routine of `symbols` without a Rust function.
pub fn table(symbols: &Symbols, mode: HookMode) -> anyhow::Result<HookTable> {
    Ok(ssd_game::hook_table(symbols, Some(mode))?)
}

/// What looks wrong with a measurement, one line each: divergences in the `Verify` pass (the
/// means were measured on the original, so they stand, but the routine is not right), and
/// routines that ran in `Rust` without a mean to count them by.
#[must_use]
pub fn warnings(measured: &Measured) -> Vec<String> {
    let mut warnings = Vec::new();
    let diverged: Vec<String> = measured
        .verify_stats
        .iter()
        .filter(|(_, stats)| stats.divergences > 0)
        .map(|(name, stats)| format!("{name} {}", stats.divergences))
        .collect();
    if !diverged.is_empty() {
        warnings.push(format!(
            "the Verify pass found divergences ({}): run `verify-replays` and fix them first",
            diverged.join(", ")
        ));
    }
    let unmeasured: Vec<&str> = measured
        .rust_stats
        .iter()
        .filter(|(_, stats)| {
            stats.rust_runs > stats.declared_runs && stats.mean_instructions <= 0.0
        })
        .map(|(name, _)| name.as_str())
        .collect();
    if !unmeasured.is_empty() {
        warnings.push(format!(
            "no mean instructions for {} (no call was compared in the Verify pass): their Rust \
             runs count for nothing, the percent is too low",
            unmeasured.join(", ")
        ));
    }
    warnings
}

const COLUMNS: [&str; 6] = [
    "routine",
    "calls",
    "rust_runs",
    "timing_fallback",
    "mean_instructions",
    "estimated_instructions",
];

/// The table of the `Rust` pass: a header and a row per routine, indented, names to the left and
/// numbers to the right (nothing without routines).
#[must_use]
pub fn routine_table(stats: &[(String, HookStats)]) -> String {
    let rows: Vec<Vec<String>> = stats
        .iter()
        .filter(|(name, s)| s.calls > 0 || !(name.starts_with("rc_") || name.starts_with("rm_")))
        .map(|(name, s)| {
            vec![
                name.clone(),
                s.calls.to_string(),
                s.rust_runs.to_string(),
                s.timing_fallback.to_string(),
                format!("{:.1}", s.mean_instructions),
                s.estimated_rust_instructions().to_string(),
            ]
        })
        .collect();
    render_table(&COLUMNS, &rows)
}

/// The Rust instructions the routines named in `exclude` stand for (the estimate of each, in
/// `stats`).
#[must_use]
pub fn excluded_instructions(stats: &[(String, HookStats)], exclude: &[String]) -> u64 {
    stats
        .iter()
        .filter(|(name, _)| exclude.contains(name))
        .map(|(_, s)| s.estimated_rust_instructions())
        .sum()
}

/// The totals: the instructions that ran as original code, the Rust ones (estimated) and the
/// percent, with a note on what drives it; if `exclude` is not empty, also the instructions of
/// those routines (`excluded`) and the conservative percent without them.
#[must_use]
pub fn totals(coverage: &Coverage, exclude: &[String], excluded: u64) -> String {
    let counts = format!(
        "original instructions   {:>14}  (opcode fetches of the Rust pass)\n\
         rust instructions       {:>14}  (estimated: rust_runs x mean_instructions, less what the \
         runs had run elsewhere)\n\
         coverage                {:>13.3}%  (rust / (rust + original))\n",
        coverage.original_instructions,
        coverage.rust_instructions_estimated,
        coverage.percent()
    );
    let rest = if exclude.is_empty() {
        "note: this counts instructions, not game logic. Routines called from the idle wait \
         dominate it, and it grows with the idle time per frame; `--exclude name[,name...]` \
         gives a second, conservative figure without them.\n"
            .to_owned()
    } else {
        format!(
            "excluded instructions   {excluded:>14}  ({})\n\
             coverage excluding them {:>13.3}%  ((rust - excluded) / (rust - excluded + original))\n\
             note: the first figure counts instructions, not game logic: routines called from the \
             idle wait dominate it, and it grows with the idle time per frame. The second leaves \
             the excluded routines out.\n",
            exclude.join(", "),
            coverage.excluding(excluded).percent()
        )
    };
    counts + &rest
}

/// Measures the replay file at `path` on `rom` with the routines of `symbols` and prints the
/// result to `out` (the warnings to `err`); `exclude` names the routines the conservative
/// figure leaves out.
///
/// # Errors
///
/// The replay cannot be read, is not for `rom` or this emulator core (record it again with
/// `make-replays`), a routine of `symbols` has no Rust function, or a run fails.
pub fn coverage(
    rom: &[u8],
    symbols: &Symbols,
    path: &Path,
    exclude: &[String],
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> anyhow::Result<()> {
    let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let replay = read_replay(path, rom)?;
    let verify = table(symbols, HookMode::Verify)?;
    let rust = table(symbols, HookMode::Rust)?;
    if let Some(warning) = ssd_game::missing_cycles_warning(&rust) {
        writeln!(err, "coverage: warning: {warning}")?;
    }
    writeln!(
        out,
        "coverage: {name}, {} frames, {} routines (pass 1 Verify: the means; pass 2 Rust: the count)",
        replay.frames(),
        rust.len()
    )?;
    out.flush()?;
    let start = Instant::now();
    let measured = measure(rom, &replay, verify, rust)?;
    writeln!(out, "both passes took {:.1} s", start.elapsed().as_secs_f64())?;
    write!(out, "{}", routine_table(&measured.rust_stats))?;
    let excluded = excluded_instructions(&measured.rust_stats, exclude);
    write!(out, "{}", totals(&measured.coverage, exclude, excluded))?;
    for warning in warnings(&measured) {
        writeln!(err, "coverage: warning: {warning}")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{LEAF, leaf_rom};
    use ssd_oracle::{Ctx, HookAction, HookEntry, Joypad, Machine, MemorySaveStore, ReturnKind};

    /// Its instructions: `INC $10; RTS`.
    const LEAF_INSTRUCTIONS: f64 = 2.0;

    /// `INC $10`, costing 80 master cycles (`INC dp` 38, `RTS` 42).
    fn leaf(ctx: &mut Ctx<'_>) {
        let value = ctx.read_dp8(0x10).wrapping_add(1);
        ctx.write_dp8(0x10, value);
        ctx.set_nz8(value);
        ctx.set_cost(80);
    }

    fn leaf_table(mode: HookMode) -> HookTable {
        [HookEntry {
            name: "leaf".to_owned(),
            key: LEAF,
            mode,
            action: HookAction::Routine { func: leaf, ret: ReturnKind::Rts },
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

    /// `frames` frames of no input on `rom` from power-on.
    fn idle_replay(rom: &[u8], frames: usize) -> Replay {
        let machine = Machine::new(rom.to_vec(), Box::new(MemorySaveStore::default())).unwrap();
        Replay { inputs: vec![Joypad::default(); frames], ..Replay::of(&machine) }
    }

    /// A mean of equal counts is exact; the tolerance only keeps the comparison float-safe.
    fn assert_close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1e-9, "{actual} against {expected}");
    }

    fn stats(rows: &[(String, HookStats)], name: &str) -> HookStats {
        rows.iter().find(|(n, _)| n == name).unwrap().1
    }

    #[test]
    fn the_arguments_are_an_optional_replay_and_routines_to_exclude() {
        let names = |names: &[&str]| names.iter().map(|&n| n.to_owned()).collect::<Vec<_>>();
        assert_eq!(parse_args(&[]), Ok(Args { replay: None, exclude: Vec::new() }));
        assert_eq!(
            parse_args(&["some/other.ssdreplay"]),
            Ok(Args { replay: Some(PathBuf::from("some/other.ssdreplay")), exclude: Vec::new() })
        );
        assert_eq!(
            parse_args(&["--exclude", "rng_next_a,rng_next_b"]),
            Ok(Args { replay: None, exclude: names(&["rng_next_a", "rng_next_b"]) })
        );
        // The option goes anywhere and can be repeated.
        assert_eq!(
            parse_args(&["--exclude", "a", "x.ssdreplay", "--exclude", "b,c"]),
            Ok(Args {
                replay: Some(PathBuf::from("x.ssdreplay")),
                exclude: names(&["a", "b", "c"])
            })
        );
        for bad in [
            &["--hooks"][..],
            &["-x"],
            &["a", "b"],
            &["a", "--fast"],
            &["--exclude"],
            &["--exclude", ""],
            &["--exclude", "a,,b"],
            &["--exclude", "a,"],
        ] {
            assert!(parse_args(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn only_routines_of_the_symbols_file_can_be_excluded() {
        let symbols =
            Symbols::load(concat!(env!("CARGO_MANIFEST_DIR"), "/../re/symbols.toml")).unwrap();
        let names = |names: &[&str]| names.iter().map(|&n| n.to_owned()).collect::<Vec<_>>();
        assert_eq!(check_excluded(&symbols, &[]), Ok(()));
        assert_eq!(check_excluded(&symbols, &names(&["rng_next_a", "rng_next_b"])), Ok(()));
        let error = check_excluded(&symbols, &names(&["rng_next_a", "rng_next_c"])).unwrap_err();
        assert!(error.contains("`rng_next_c`") && error.contains("rng_next_b"), "{error}");
        // A sync point is not a routine.
        assert!(check_excluded(&symbols, &names(&["main_loop_idle"])).is_err());
    }

    #[test]
    fn the_means_of_the_verify_pass_count_the_rust_runs_of_the_second() {
        let rom = leaf_rom();
        let replay = idle_replay(&rom, 5);
        let measured =
            measure(&rom, &replay, leaf_table(HookMode::Verify), leaf_table(HookMode::Rust))
                .unwrap();

        let verify = stats(&measured.verify_stats, "leaf");
        assert!(verify.verified_ok > 100 && verify.divergences == 0, "{verify:?}");
        assert_close(verify.mean_instructions, LEAF_INSTRUCTIONS);
        let rust = stats(&measured.rust_stats, "leaf");
        assert!(rust.rust_runs > 100, "{rust:?}");
        assert_eq!(rust.timing_fallback, 0);
        assert_close(rust.mean_instructions, LEAF_INSTRUCTIONS);

        let runs = rust.rust_runs;
        assert_eq!(measured.coverage.rust_instructions_estimated, 2 * runs);
        // A pass of the loop is JSR, LDX, 200 x (DEX, BNE) and BRA around the leaf's 2: the
        // original instructions are 403 per run, plus the 5 of the start-up and the pass in
        // progress at the end of the replay.
        let original = measured.coverage.original_instructions;
        assert!(original.abs_diff(5 + 403 * runs) <= 403, "{original} for {runs} runs");
        let expected = 100.0 * (2 * runs) as f64 / (2 * runs + original) as f64;
        assert!((measured.coverage.percent() - expected).abs() < 1e-9);
        assert!((0.45..0.5).contains(&measured.coverage.percent()));
        assert_eq!(warnings(&measured), Vec::<String>::new());
    }

    #[test]
    fn a_routine_in_rust_that_verify_never_compared_is_named_in_a_warning() {
        let rom = leaf_rom();
        let replay = idle_replay(&rom, 2);
        // The Verify pass has the routine unhooked: no mean to learn.
        let measured =
            measure(&rom, &replay, leaf_table(HookMode::Original), leaf_table(HookMode::Rust))
                .unwrap();
        assert_eq!(measured.coverage.rust_instructions_estimated, 0);
        let warnings = warnings(&measured);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("no mean instructions for leaf"), "{warnings:?}");
    }

    #[test]
    fn divergences_of_the_verify_pass_are_a_warning() {
        fn wrong(ctx: &mut Ctx<'_>) {
            ctx.write_dp8(0x10, 0x55);
        }
        let rom = leaf_rom();
        let replay = idle_replay(&rom, 2);
        let mut verify = leaf_table(HookMode::Verify);
        verify.insert(HookEntry {
            action: HookAction::Routine { func: wrong, ret: ReturnKind::Rts },
            ..verify.get(LEAF).unwrap().clone()
        });
        let measured = measure(&rom, &replay, verify, leaf_table(HookMode::Rust)).unwrap();
        // The means were measured on the original all the same.
        assert_close(stats(&measured.verify_stats, "leaf").mean_instructions, LEAF_INSTRUCTIONS);
        let warnings = warnings(&measured);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("divergences (leaf "), "{warnings:?}");
    }

    #[test]
    fn a_replay_for_another_rom_is_refused() {
        let rom = leaf_rom();
        let replay = Replay { rom_crc32: 0x1234_5678, ..idle_replay(&rom, 1) };
        let error = measure(&rom, &replay, HookTable::new(), HookTable::new()).err().unwrap();
        assert!(format!("{error:#}").contains("ROM"), "{error:#}");
    }

    #[test]
    fn the_table_has_a_header_and_a_row_per_routine() {
        assert_eq!(routine_table(&[]), "");
        let stats = HookStats {
            calls: 1000,
            rust_runs: 990,
            timing_fallback: 10,
            mean_instructions: 31.5,
            ..HookStats::default()
        };
        let table = routine_table(&[("rng_next_a".to_owned(), stats)]);
        let rows: Vec<Vec<&str>> =
            table.lines().map(|line| line.split_whitespace().collect()).collect();
        assert_eq!(
            rows,
            [COLUMNS.to_vec(), vec!["rng_next_a", "1000", "990", "10", "31.5", "31185"]]
        );
        assert!(table.lines().all(|line| line.starts_with("  ")), "{table}");
    }

    #[test]
    fn the_totals_give_both_counts_the_percent_and_what_drives_it() {
        let text = totals(
            &Coverage { original_instructions: 900, rust_instructions_estimated: 100 },
            &[],
            0,
        );
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 4, "{text}");
        assert!(lines[0].starts_with("original instructions") && lines[0].contains(" 900 "));
        assert!(lines[1].starts_with("rust instructions") && lines[1].contains(" 100 "));
        assert!(lines[2].starts_with("coverage") && lines[2].contains("10.000%"), "{text}");
        // No promise that the figure is small: the note says what drives it, and how to see
        // the figure without it.
        assert!(!text.contains("small"), "{text}");
        assert!(lines[3].starts_with("note:") && lines[3].contains("idle wait"), "{text}");
        assert!(
            lines[3].contains("--exclude") && lines[3].contains("idle time per frame"),
            "{text}"
        );
    }

    #[test]
    fn excluded_routines_give_a_second_conservative_figure() {
        let exclude = vec!["rng_next_a".to_owned(), "rng_next_b".to_owned()];
        let text = totals(
            &Coverage { original_instructions: 900, rust_instructions_estimated: 100 },
            &exclude,
            40,
        );
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 6, "{text}");
        assert!(lines[2].starts_with("coverage ") && lines[2].contains("10.000%"), "{text}");
        assert!(
            lines[3].starts_with("excluded instructions")
                && lines[3].contains(" 40 ")
                && lines[3].contains("(rng_next_a, rng_next_b)"),
            "{text}"
        );
        // (100 - 40) / (100 - 40 + 900) = 6.25%.
        assert!(
            lines[4].starts_with("coverage excluding them") && lines[4].contains("6.250%"),
            "{text}"
        );
        assert!(text.contains("(rust - excluded) / (rust - excluded + original)"), "{text}");
        assert!(text.contains("The second leaves the excluded routines out"), "{text}");
    }

    #[test]
    fn the_excluded_instructions_are_the_estimates_of_the_named_routines() {
        let routine = |name: &str, rust_runs, mean_instructions| {
            (name.to_owned(), HookStats { rust_runs, mean_instructions, ..HookStats::default() })
        };
        let rows = [
            routine("rng_next_a", 10, 11.0),
            routine("rng_next_b", 20, 11.0),
            routine("advance_x_by_6", 5, 7.0),
        ];
        let named = |names: &[&str]| names.iter().map(|&n| n.to_owned()).collect::<Vec<_>>();
        assert_eq!(excluded_instructions(&rows, &[]), 0);
        assert_eq!(excluded_instructions(&rows, &named(&["rng_next_a", "rng_next_b"])), 330);
        assert_eq!(excluded_instructions(&rows, &named(&["advance_x_by_6"])), 35);
        // Twice in the list is still once.
        assert_eq!(excluded_instructions(&rows, &named(&["rng_next_a", "rng_next_a"])), 110);
    }
}
