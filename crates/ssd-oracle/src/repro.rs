//! Running a divergence report's call again (spec §5.4): the state at the entry of the call that
//! diverged, loaded, with the routine's Rust function checked against the original once more.
//! `xtask repro` and the debug panel's "Reproduce" both use [`reproduce`].
//!
//! The routine is the one the report's text names (its first line, a [`ReproHeader`]), taken
//! from the symbols with its Rust function and installed alone, in `Verify`, whatever mode the
//! symbols give it, with every call checked. The state is the machine at the entry of the call
//! that diverged: it is loaded, and the machine runs to the first call of the routine, which the
//! original and the Rust function both run, and the comparison is the result. The joypad is
//! neutral (the state holds the controller registers; a call is a few hundred cycles).

use crate::report::ReproHeader;
use crate::{
    Divergence, HookEntry, HookMode, HookStats, HookTable, Joypad, Machine, MemorySaveStore,
    NullAudio, OracleError, Symbols, hook_key,
};
use std::sync::Arc;

/// Frames the routine gets to be reached from the loaded state.
pub const BUDGET_FRAMES: u64 = 2;

/// The hook entry of a routine by name (with its Rust function), if there is one.
pub type Resolver<'a> = &'a dyn Fn(&str) -> Option<HookEntry>;

/// What running the report's call again came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The Rust result matched the original's, at this frame; `cost_warning` says so if the cost
    /// the Rust function declared did not (it differs from the original's, or it declared none).
    Matches { frame: u64, cost_warning: Option<String> },
    /// The Rust result differs from the original's: the comparison (call 0, the first call of
    /// the routine from the loaded state).
    Diverges(Divergence),
    /// The call could not be compared (the reason).
    Unreproducible(String),
}

/// Runs the call of `header`'s routine saved in `state` on `rom`: the routine of `resolve` (its
/// entry taken as `Verify`, every call checked, alone in the table) from the loaded state to its
/// first call, within [`BUDGET_FRAMES`] frames. `symbols` names the variables in the comparison.
///
/// # Errors
///
/// `rom` is not a ROM the core takes, or the run fails; the situations that cannot be reproduced
/// are [`Verdict::Unreproducible`].
pub fn reproduce(
    rom: &[u8],
    state: &[u8],
    header: &ReproHeader,
    resolve: Resolver<'_>,
    symbols: Option<Arc<Symbols>>,
) -> Result<Verdict, OracleError> {
    let unreproducible =
        |reason: String| -> Result<Verdict, OracleError> { Ok(Verdict::Unreproducible(reason)) };
    let Some(mut entry) = resolve(&header.routine) else {
        return unreproducible(format!(
            "the routine `{}` is not in the symbols file, or has no Rust function in this build",
            header.routine
        ));
    };
    if hook_key(entry.key) != header.key {
        return unreproducible(format!(
            "the report is of `{}` at ${:06X}, the symbols now put it at ${:06X}",
            header.routine,
            header.key,
            hook_key(entry.key)
        ));
    }
    entry.mode = HookMode::Verify;
    entry.verify_sample = 1;
    let key = entry.key;
    let table: HookTable = [entry].into_iter().collect();

    let mut machine = Machine::new(rom.to_vec(), Box::new(MemorySaveStore::default()))?;
    // The table goes in before the state: loading a state at a hooked opcode fetch (the entry of
    // the call, as the report saves it) reports the arrival only if the hook is installed then.
    machine.set_hook_table(Some(table));
    machine.set_symbols(symbols);
    match machine.load_state(state) {
        Ok(()) => {}
        Err(error @ OracleError::State(_)) => return unreproducible(error.to_string()),
        Err(error) => return Err(error),
    }
    // Any other state would run to the routine's next call, a different one from the report's.
    let registers = machine.cpu_registers();
    let at = hook_key((u32::from(registers.pbr) << 16) | u32::from(registers.pc));
    if let Some(problem) = entry_problem(header, at, machine.at_fetch_boundary()) {
        return unreproducible(problem);
    }
    let frame = machine.frame_count();
    let mut input = |_frame: u64| Joypad::default();
    // A safety net: from the entry of the call, the first step reports it.
    if !machine.run_until_call(key, &mut input, &mut NullAudio, BUDGET_FRAMES)? {
        return unreproducible(format!(
            "`{}` is not reached within {BUDGET_FRAMES} frames of the state (is it the state of a \
             divergence report?)",
            header.routine
        ));
    }
    let stats = machine
        .hook_stats()
        .into_iter()
        .find(|(name, _)| *name == header.routine)
        .map(|(_, stats)| stats)
        .ok_or_else(|| OracleError::Run("the routine's counters are missing".to_owned()))?;
    Ok(judge(&stats, machine.divergences().first(), frame))
}

/// Why a loaded state is not the entry of the report's call, if it is not: the CPU must be about
/// to fetch the routine's first opcode (`at`, the hook key of `PBR:PC`, is the report's and
/// `at_fetch_boundary`).
#[must_use]
pub fn entry_problem(header: &ReproHeader, at: u32, at_fetch_boundary: bool) -> Option<String> {
    let place = if at != header.key {
        format!("at ${at:06X}")
    } else if !at_fetch_boundary {
        "at that address but not about to fetch its opcode (a refresh, a DMA or an instruction \
         is in progress)"
            .to_owned()
    } else {
        return None;
    };
    Some(format!(
        "the state is not at the entry of `{}` (${:06X}): the CPU is {place} (is it the state of \
         a divergence report?)",
        header.routine, header.key
    ))
}

/// What is wrong with the cost the Rust function declared for a call that matched in memory, if
/// anything: a cost that differs from the original's measured one (`cost_mismatch`), or none
/// (`cost_missing`). In `Rust` mode the cost is charged as the time of the call, so the machine
/// would drift from the original's timeline.
#[must_use]
pub fn cost_warning(stats: &HookStats) -> Option<String> {
    let problem = if stats.cost_mismatch > 0 {
        "the cost it declares differs from the original's (cost_mismatch)"
    } else if stats.cost_missing > 0 {
        "it declares no cost (cost_missing)"
    } else {
        return None;
    };
    Some(format!(
        "the memory matches, but {problem}: in Rust mode the timing would drift from the \
         original's"
    ))
}

/// The verdict on the first call of the routine, from its counters (it is the only one
/// dispatched) and the divergence kept for it, if any; `frame` is the state's.
#[must_use]
pub fn judge(stats: &HookStats, divergence: Option<&Divergence>, frame: u64) -> Verdict {
    if let Some(divergence) = divergence {
        return Verdict::Diverges(divergence.clone());
    }
    if stats.verified_ok > 0 {
        return Verdict::Matches { frame, cost_warning: cost_warning(stats) };
    }
    let reason = if stats.contract_mismatch > 0 {
        "the CPU does not meet the routine's entry contract at the state (native mode, and the \
         accumulator and index widths the symbols require)"
            .to_owned()
    } else if stats.interrupted > 0 {
        "an interrupt entered the original during the call, so it was not compared".to_owned()
    } else if stats.no_return > 0 {
        "the original did not return within the oracle's frame budget".to_owned()
    } else {
        "the call was not checked".to_owned()
    };
    Verdict::Unreproducible(reason)
}
