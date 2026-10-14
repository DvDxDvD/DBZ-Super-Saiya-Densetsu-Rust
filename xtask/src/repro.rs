//! `repro <path.ssdstate>`: a divergence of `Verify` (a `.ssdstate` and its `.txt`, see
//! `ssd_oracle::report`) run again, so a fix of the Rust routine can be tried with one command.
//!
//! The ROM is the canonical one of the cache. The routine is the one the report's text names
//! (its first line, a `ssd_oracle::report::ReproHeader`), taken from `re/symbols.toml` with its
//! Rust function and installed alone, in `Verify`, whatever mode the symbols give it, with every
//! call checked. The state is the machine at the entry of the call that diverged: it is loaded,
//! and the machine runs to the first call of the routine, which the original and the Rust
//! function both run, and the comparison is printed with the text of the divergence reports. The
//! joypad is neutral (the state holds the controller registers; a call is a few hundred cycles).
//!
//! Exit codes: 0 the call now matches the original; 1 it still diverges, or an error (the ROM
//! cache, a file that cannot be read, the symbols); 3 it cannot be reproduced (the state is for
//! another ROM or emulator core, the report has no header or no text, the routine is not in the
//! symbols or moved, the state is not at the entry of the routine's call, the routine is not
//! reached within its frame budget, the call is not compared: its contract does not hold, an
//! interrupt entered it, the original does not return); 64 a bad command line. The reason of an
//! exit 3 goes to stderr. A call that matches in memory but whose cost the Rust function declares
//! wrongly (or not at all) still exits 0, with a warning on stderr: in `Rust` mode that cost is
//! what keeps lockstep in step.

use anyhow::Context;
use ssd_oracle::Symbols;
use ssd_oracle::report::{self, ReproHeader};
use ssd_oracle::repro::{Resolver, Verdict, reproduce};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The call is reproduced and the Rust routine now matches the original.
pub const EXIT_MATCHES: u8 = 0;
/// The call is reproduced and the Rust routine still differs from the original.
pub const EXIT_DIVERGES: u8 = 1;
/// The call cannot be reproduced (see the module documentation).
pub const EXIT_UNREPRODUCIBLE: u8 = 3;

/// Parses the arguments after `repro`: the state file.
///
/// # Errors
///
/// No file, an option, or more than one file.
pub fn parse_args(args: &[&str]) -> Result<PathBuf, String> {
    match args {
        [] => Err("a state file is needed: repro <path.ssdstate>".to_owned()),
        [option, ..] if option.starts_with('-') => Err(format!("unknown option `{option}`")),
        [path] => Ok(PathBuf::from(path)),
        [_, second, ..] => Err(format!("one state file only (`{second}` is a second one)")),
    }
}

/// Reads the report at `state_path` (the state and the text next to it), runs its call again
/// with [`reproduce`], prints the comparison to `out` (the reason, if it cannot be reproduced,
/// to `err`) and returns the exit code ([`EXIT_MATCHES`], [`EXIT_DIVERGES`],
/// [`EXIT_UNREPRODUCIBLE`]).
///
/// # Errors
///
/// A file cannot be read (the state, or the text if it exists), or [`reproduce`] fails.
pub fn repro(
    rom: &[u8],
    state_path: &Path,
    resolve: Resolver<'_>,
    symbols: Option<Arc<Symbols>>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> anyhow::Result<u8> {
    let mut cannot = |reason: String| -> anyhow::Result<u8> {
        writeln!(err, "repro: cannot reproduce: {reason}")?;
        Ok(EXIT_UNREPRODUCIBLE)
    };
    let state =
        std::fs::read(state_path).with_context(|| format!("reading {}", state_path.display()))?;
    let text_path = report::text_path(state_path);
    let text = match std::fs::read_to_string(&text_path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return cannot(format!(
                "{} is missing: the text of the report names the routine",
                text_path.display()
            ));
        }
        Err(error) => {
            return Err(error).with_context(|| format!("reading {}", text_path.display()));
        }
    };
    let Some(header) = ReproHeader::parse(&text) else {
        return cannot(format!(
            "{} has no `ssd-repro:` line first (a report of an older build): make the report again",
            text_path.display()
        ));
    };

    let name =
        state_path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    writeln!(out, "repro: {name}: {} at ${:06X}", header.routine, header.key)?;
    match reproduce(rom, &state, &header, resolve, symbols)? {
        Verdict::Matches { frame, cost_warning } => {
            writeln!(
                out,
                "repro: no difference at frame {frame}: `{}` now matches the original",
                header.routine
            )?;
            if let Some(warning) = cost_warning {
                writeln!(err, "repro: warning: `{}`: {warning}", header.routine)?;
            }
            Ok(EXIT_MATCHES)
        }
        Verdict::Diverges(divergence) => {
            write!(out, "{divergence}")?;
            writeln!(out, "repro: still diverges")?;
            Ok(EXIT_DIVERGES)
        }
        Verdict::Unreproducible(reason) => cannot(reason),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{LEAF, leaf_rom};
    use ssd_oracle::repro::{entry_problem, judge};
    use ssd_oracle::{
        Ctx, Divergence, HookAction, HookEntry, HookMode, HookStats, Joypad, Machine,
        MemorySaveStore, NullAudio, ReturnKind, RoutineFn,
    };

    /// CPU-busy master cycles of the leaf of the fixture ROM: `INC dp` 38, `RTS` 42.
    const LEAF_COST: u64 = 80;

    /// `INC $10`, as the leaf of the fixture ROM does, without declaring its cost.
    fn inc_counter_without_a_cost(ctx: &mut Ctx<'_>) {
        let value = ctx.read_dp8(0x10).wrapping_add(1);
        ctx.write_dp8(0x10, value);
        ctx.set_nz8(value);
    }

    /// [`inc_counter_without_a_cost`] declaring `cost`.
    fn inc_counter_costing(ctx: &mut Ctx<'_>, cost: u64) {
        inc_counter_without_a_cost(ctx);
        ctx.set_cost(cost);
    }

    /// `INC $10` and its exact cost.
    fn inc_counter(ctx: &mut Ctx<'_>) {
        inc_counter_costing(ctx, LEAF_COST);
    }

    /// `INC $10` declaring a cost 2 master cycles too long.
    fn inc_counter_with_a_wrong_cost(ctx: &mut Ctx<'_>) {
        inc_counter_costing(ctx, LEAF_COST + 2);
    }

    /// [`inc_counter`] that adds 2 when the counter is $FF: wrong on one
    /// call in 256, so the report is some frames into the run.
    fn inc_counter_wrong_on_the_256th(ctx: &mut Ctx<'_>) {
        let before = ctx.read_dp8(0x10);
        let value = before.wrapping_add(if before == 0xFF { 2 } else { 1 });
        ctx.write_dp8(0x10, value);
        ctx.set_nz8(value);
        ctx.set_cost(LEAF_COST);
    }

    fn entry_at(key: u32, func: RoutineFn) -> HookEntry {
        HookEntry {
            name: "leaf".to_owned(),
            key,
            // Whatever the symbols say, `repro` checks the routine alone, in Verify.
            mode: HookMode::Original,
            action: HookAction::Routine { func, ret: ReturnKind::Rts },
            entry_m8: Some(true),
            entry_x8: Some(false),
            flags_mask: 0xFF,
            verify_sample: 1000,
            bound: None,
            budget_frames: None,
            cycles: None,
        }
    }

    fn entry(func: RoutineFn) -> HookEntry {
        entry_at(LEAF, func)
    }

    /// A resolver that knows `entry` only.
    fn only(entry: HookEntry) -> impl Fn(&str) -> Option<HookEntry> {
        move |name| (name == entry.name).then(|| entry.clone())
    }

    fn live(rom: &[u8], func: RoutineFn) -> Machine {
        let mut live = Machine::new(rom.to_vec(), Box::new(MemorySaveStore::default())).unwrap();
        let mut verified = entry(func);
        verified.mode = HookMode::Verify;
        verified.verify_sample = 1;
        live.set_hook_table(Some([verified].into_iter().collect()));
        live
    }

    /// A report of `rom`'s wrong leaf, saved by a live run as `Verify` does.
    struct Saved {
        _dir: tempfile::TempDir,
        state: PathBuf,
        divergence: Divergence,
    }

    fn save_a_divergence(rom: &[u8]) -> Saved {
        let dir = tempfile::tempdir().unwrap();
        let mut live = live(rom, inc_counter_wrong_on_the_256th);
        live.set_divergence_dir(Some(dir.path().to_owned()));
        for _ in 0..8 {
            live.run_frame(Joypad::default(), &mut NullAudio).unwrap();
        }
        let divergence = live.divergences().first().expect("one call in 256 diverges").clone();
        assert!(divergence.frame > 0, "the state is not the first frame's: {divergence}");
        let stem = format!("leaf-{}-{}", divergence.frame, divergence.call_index);
        let state = dir.path().join(format!("{stem}.ssdstate"));
        assert!(state.is_file() && report::text_path(&state).is_file());
        Saved { _dir: dir, state, divergence }
    }

    fn header() -> ReproHeader {
        ReproHeader { routine: "leaf".to_owned(), key: LEAF }
    }

    /// `repro` on `state`, with what it printed to stdout and stderr.
    fn run_repro(rom: &[u8], state: &Path, resolve: Resolver<'_>) -> (u8, String, String) {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = repro(rom, state, resolve, None, &mut out, &mut err).unwrap();
        (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
    }

    #[test]
    fn the_wrong_routine_diverges_again_as_the_report_says() {
        let rom = leaf_rom();
        let saved = save_a_divergence(&rom);
        let state = std::fs::read(&saved.state).unwrap();
        let wrong = only(entry(inc_counter_wrong_on_the_256th));

        let verdict = reproduce(&rom, &state, &header(), &wrong, None).unwrap();
        let Verdict::Diverges(again) = verdict else { panic!("{verdict:?}") };
        // The same call, from the loaded state: its frame, and the bytes it differs in.
        assert_eq!(again.frame, saved.divergence.frame);
        assert_eq!(again.differences, saved.divergence.differences);
        assert_eq!((again.call_index, again.routine.as_str()), (0, "leaf"));
    }

    #[test]
    fn the_fixed_routine_matches() {
        let rom = leaf_rom();
        let saved = save_a_divergence(&rom);
        let state = std::fs::read(&saved.state).unwrap();
        let fixed = only(entry(inc_counter));

        let verdict = reproduce(&rom, &state, &header(), &fixed, None).unwrap();
        assert_eq!(verdict, Verdict::Matches { frame: saved.divergence.frame, cost_warning: None });
    }

    #[test]
    fn repro_prints_the_comparison_and_exits_one_when_it_still_diverges() {
        let rom = leaf_rom();
        let saved = save_a_divergence(&rom);
        let (code, out, err) =
            run_repro(&rom, &saved.state, &only(entry(inc_counter_wrong_on_the_256th)));

        assert_eq!((code, err.as_str()), (EXIT_DIVERGES, ""));
        let file = saved.state.file_name().unwrap().to_string_lossy().into_owned();
        let frame = saved.divergence.frame;
        let comparison = format!("leaf: call 0 at frame {frame}, ");
        assert!(out.starts_with(&format!("repro: {file}: leaf at $008040\n{comparison}")), "{out}");
        // The text of the divergence reports, line for line.
        let lines = out.lines().skip(2).collect::<Vec<_>>();
        let reported = saved.divergence.to_string();
        assert_eq!(lines[..lines.len() - 1], reported.lines().skip(1).collect::<Vec<_>>());
        assert_eq!(lines.last(), Some(&"repro: still diverges"));
    }

    #[test]
    fn repro_exits_zero_when_the_fixed_routine_matches() {
        let rom = leaf_rom();
        let saved = save_a_divergence(&rom);
        let (code, out, err) = run_repro(&rom, &saved.state, &only(entry(inc_counter)));

        assert_eq!((code, err.as_str()), (EXIT_MATCHES, ""));
        let frame = saved.divergence.frame;
        let expected = format!("no difference at frame {frame}: `leaf` now matches the original");
        assert!(out.ends_with(&format!("repro: {expected}\n")), "{out}");
    }

    #[test]
    fn the_comparison_names_the_variables_of_the_symbols() {
        let rom = leaf_rom();
        let saved = save_a_divergence(&rom);
        let source = "[[variable]]\nname = \"leaf_counter\"\naddress = 0x7E0010\nsize = 1\n";
        let symbols = Arc::new(Symbols::parse(source, Path::new("symbols.toml")).unwrap());
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let wrong = only(entry(inc_counter_wrong_on_the_256th));
        let code = repro(&rom, &saved.state, &wrong, Some(symbols), &mut out, &mut err).unwrap();

        assert_eq!(code, EXIT_DIVERGES);
        let out = String::from_utf8(out).unwrap();
        assert!(out.contains("leaf — wram[$7E:0010] leaf_counter: $"), "{out}");
    }

    #[test]
    fn a_state_of_another_rom_cannot_be_reproduced() {
        let rom = leaf_rom();
        let saved = save_a_divergence(&rom);
        let mut other = rom.clone();
        other[0x7000] = 0x12; // never run
        ssd_rom::fix_lorom_checksum(&mut other);
        assert_ne!(ssd_rom::crc32(&other), ssd_rom::crc32(&rom));

        let (code, _, err) =
            run_repro(&other, &saved.state, &only(entry(inc_counter_wrong_on_the_256th)));
        assert_eq!(code, EXIT_UNREPRODUCIBLE);
        assert!(err.starts_with("repro: cannot reproduce: "), "{err}");
        assert!(err.contains("save state is for ROM"), "the state's ROM is named: {err}");
    }

    #[test]
    fn a_state_that_is_not_a_state_cannot_be_reproduced() {
        let rom = leaf_rom();
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("junk.ssdstate");
        std::fs::write(&state, b"not a save state").unwrap();
        std::fs::write(report::text_path(&state), format!("{}\nleaf: call 0\n", header())).unwrap();

        let (code, _, err) = run_repro(&rom, &state, &only(entry(inc_counter)));
        assert_eq!(code, EXIT_UNREPRODUCIBLE);
        assert!(err.contains("not a save state"), "{err}");
    }

    #[test]
    fn a_report_without_its_text_or_header_cannot_be_reproduced() {
        let rom = leaf_rom();
        let saved = save_a_divergence(&rom);
        let resolve = only(entry(inc_counter));
        let text = report::text_path(&saved.state);
        let with_header = std::fs::read_to_string(&text).unwrap();

        // A report of a build before the header: the first line is the comparison's own.
        std::fs::write(&text, with_header.split_once('\n').unwrap().1).unwrap();
        let (code, _, err) = run_repro(&rom, &saved.state, &resolve);
        assert_eq!(code, EXIT_UNREPRODUCIBLE);
        assert!(err.contains("no `ssd-repro:` line") && err.contains("leaf-"), "{err}");

        std::fs::remove_file(&text).unwrap();
        let (code, _, err) = run_repro(&rom, &saved.state, &resolve);
        assert_eq!(code, EXIT_UNREPRODUCIBLE);
        assert!(err.contains("is missing") && err.contains(".txt"), "{err}");
    }

    #[test]
    fn a_missing_state_is_an_error_not_a_report_that_cannot_be_reproduced() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("leaf-1-2.ssdstate");
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let error =
            repro(&leaf_rom(), &missing, &only(entry(inc_counter)), None, &mut out, &mut err)
                .unwrap_err();
        assert!(format!("{error:#}").contains("leaf-1-2.ssdstate"), "{error:#}");
    }

    #[test]
    fn a_routine_the_symbols_do_not_know_or_moved_cannot_be_reproduced() {
        let rom = leaf_rom();
        let saved = save_a_divergence(&rom);
        let state = std::fs::read(&saved.state).unwrap();

        let unknown = |_: &str| None;
        let verdict = reproduce(&rom, &state, &header(), &unknown, None).unwrap();
        let Verdict::Unreproducible(reason) = verdict else { panic!("{verdict:?}") };
        assert!(reason.contains("`leaf` is not in the symbols"), "{reason}");

        let moved = only(entry_at(0x00_8050, inc_counter));
        let verdict = reproduce(&rom, &state, &header(), &moved, None).unwrap();
        let Verdict::Unreproducible(reason) = verdict else { panic!("{verdict:?}") };
        assert!(reason.contains("$008040") && reason.contains("$008050"), "{reason}");
    }

    #[test]
    fn a_call_that_is_not_compared_says_why() {
        let rom = leaf_rom();
        let saved = save_a_divergence(&rom);
        let state = std::fs::read(&saved.state).unwrap();

        // The leaf is called with an 8-bit accumulator: this contract does not hold.
        let mut sixteen_bit = entry(inc_counter);
        sixteen_bit.entry_m8 = Some(false);
        let verdict = reproduce(&rom, &state, &header(), &only(sixteen_bit), None).unwrap();
        let Verdict::Unreproducible(reason) = verdict else { panic!("{verdict:?}") };
        assert!(reason.contains("entry contract"), "{reason}");
    }

    #[test]
    fn the_counters_of_the_call_tell_whether_it_was_compared() {
        let judged = |stats: HookStats| judge(&HookStats { calls: 1, ..stats }, None, 9);
        let verified = HookStats { verified_ok: 1, ..HookStats::default() };
        assert_eq!(judged(verified), Verdict::Matches { frame: 9, cost_warning: None });
        let cases = [
            (HookStats { interrupted: 1, ..HookStats::default() }, "an interrupt entered"),
            (HookStats { no_return: 1, ..HookStats::default() }, "did not return within"),
            (HookStats::default(), "was not checked"),
        ];
        for (stats, expected) in cases {
            let verdict = judged(stats);
            let Verdict::Unreproducible(reason) = verdict else { panic!("{verdict:?}") };
            assert!(reason.contains(expected), "{reason}");
            // The budget is the oracle's: no number of its own to go stale.
            assert!(!reason.chars().any(|c| c.is_ascii_digit()), "{reason}");
        }
    }

    #[test]
    fn a_match_with_a_wrong_or_missing_cost_carries_a_warning() {
        let matched = |stats: HookStats| {
            let verdict = judge(&HookStats { calls: 1, verified_ok: 1, ..stats }, None, 9);
            let Verdict::Matches { cost_warning, .. } = verdict else { panic!("{verdict:?}") };
            cost_warning
        };
        assert_eq!(matched(HookStats::default()), None);
        let mismatch = matched(HookStats { cost_mismatch: 1, ..HookStats::default() }).unwrap();
        assert!(mismatch.contains("cost_mismatch") && mismatch.contains("drift"), "{mismatch}");
        let missing = matched(HookStats { cost_missing: 1, ..HookStats::default() }).unwrap();
        assert!(missing.contains("cost_missing"), "{missing}");
        // A divergence is reported as such, whatever the cost.
        let stats = HookStats { calls: 1, cost_mismatch: 1, ..HookStats::default() };
        let divergence = Divergence::new("leaf".to_owned(), 0, 9, Vec::new(), None);
        assert!(matches!(judge(&stats, Some(&divergence), 9), Verdict::Diverges(_)));
    }

    #[test]
    fn repro_warns_on_stderr_when_the_memory_matches_but_the_cost_does_not() {
        let rom = leaf_rom();
        let saved = save_a_divergence(&rom);
        for (func, expected) in [
            (inc_counter_with_a_wrong_cost as RoutineFn, "cost_mismatch"),
            (inc_counter_without_a_cost, "cost_missing"),
        ] {
            let (code, out, err) = run_repro(&rom, &saved.state, &only(entry(func)));
            assert_eq!(code, EXIT_MATCHES, "{expected}: {err}");
            assert!(out.contains("now matches the original"), "{out}");
            assert!(err.starts_with("repro: warning: `leaf`: the memory matches, but "), "{err}");
            assert!(err.contains(expected), "{err}");
            assert_eq!(err.lines().count(), 1, "{err}");
        }
    }

    #[test]
    fn the_entry_of_the_call_is_the_hooked_fetch_and_nothing_else() {
        let header = header();
        assert_eq!(entry_problem(&header, LEAF, true), None);
        let elsewhere = entry_problem(&header, 0x00_8000, true).unwrap();
        assert!(elsewhere.contains("`leaf` ($008040)") && elsewhere.contains("at $008000"));
        let mid_instruction = entry_problem(&header, LEAF, false).unwrap();
        assert!(mid_instruction.contains("not about to fetch its opcode"), "{mid_instruction}");
    }

    #[test]
    fn a_state_that_is_not_the_entry_of_the_call_cannot_be_reproduced() {
        // The machine at power-on is a valid state of this ROM, at its reset vector.
        let rom = leaf_rom();
        let power_on = Machine::new(rom.clone(), Box::new(MemorySaveStore::default())).unwrap();
        let state = power_on.save_state();
        let reproduced =
            reproduce(&rom, &state, &header(), &only(entry(inc_counter)), None).unwrap();
        let Verdict::Unreproducible(reason) = reproduced else { panic!("{reproduced:?}") };
        assert!(reason.contains("not at the entry of `leaf`") && reason.contains("$008000"));

        // The same through the command, with a report whose text names the routine.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("leaf-0-0.ssdstate");
        std::fs::write(&path, &state).unwrap();
        std::fs::write(report::text_path(&path), format!("{}\nleaf: call 0\n", header())).unwrap();
        let (code, _, err) = run_repro(&rom, &path, &only(entry(inc_counter)));
        assert_eq!(code, EXIT_UNREPRODUCIBLE);
        assert!(err.starts_with("repro: cannot reproduce: the state is not at the entry"), "{err}");
    }

    #[test]
    fn the_arguments_are_one_state_file() {
        assert_eq!(parse_args(&["a/leaf-1-2.ssdstate"]), Ok(PathBuf::from("a/leaf-1-2.ssdstate")));
        for bad in [&[][..], &["--fast"], &["-"], &["a", "b"], &["--hooks", "rust", "a"]] {
            assert!(parse_args(bad).is_err(), "{bad:?}");
        }
    }
}
