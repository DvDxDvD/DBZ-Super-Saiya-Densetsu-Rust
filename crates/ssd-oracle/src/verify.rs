//! `Verify` hook mode (spec §5.4, M1a rulings): the original routine runs on the live machine
//! and the Rust function on a snapshot of the entry state; the outcomes are compared and the live
//! machine keeps the original's result, so its timing stays exactly the original's.

use crate::compare::{Divergence, Outcome, compare_except};
use crate::hooks::{BoundFn, HookStats, ReturnKind, RoutineFn, cost_over_bound};
use crate::machine::Dispatch;
use crate::timing::call_fits_window;
use crate::{AudioSink, Joypad, Machine, NullAudio, OracleError};
use std::ops::Range;

/// Divergences kept (and saved) per routine; later ones are only counted.
const KEPT_DIVERGENCES: u64 = 32;
/// Bank-0 addresses below this are WRAM, offset for offset (low RAM).
const LOW_RAM_END: usize = 0x2000;

/// A `Verify` call at its entry: the hit is taken and the entry contract holds.
pub(crate) struct VerifyCall {
    pub key: u32,
    pub func: RoutineFn,
    pub ret: ReturnKind,
    pub flags_mask: u8,
    pub verify_sample: u32,
    /// The entry's maximum cost, for [`HookStats::cost_over_bound`].
    pub cycles: Option<u64>,
    /// The entry's per-call cost bound: it replaces `cycles` for [`HookStats::cost_over_bound`].
    pub bound: Option<BoundFn>,
    /// Frames of master cycles the original may take (see [`HookEntry::budget_frames`]).
    ///
    /// [`HookEntry::budget_frames`]: crate::HookEntry::budget_frames
    pub budget_frames: u64,
}

/// Dispatches a `Verify` call as described in [`Machine::run_frame`]; returns whether it ran
/// (checked: its original ran, to its return or for its whole budget) and rendered a frame, or
/// was sampled out (the original resumes). The call is over when this returns: nothing of it
/// stays in flight. Nested `Verify` calls in its original are dispatched here again, from
/// [`Machine::run_original`].
///
/// The snapshot's core cannot fail in practice (its saves go to memory, its frame capture and
/// audio sink cannot fail); if it did while the Rust function runs, the error is returned after
/// the original's call is complete on the live machine, uncounted. For a nested call the error
/// then propagates through its callers' runs, which stop there with no outcome counted: their
/// originals are left unfinished on the live machine, at the instruction boundary where the
/// nested call returned (a valid state to run on or to save).
pub(crate) fn verify(
    live: &mut Machine,
    call: &VerifyCall,
    input: &mut dyn FnMut(u64) -> Joypad,
    audio: &mut dyn AudioSink,
) -> Result<Dispatch, OracleError> {
    let stats = live.stats_mut(call.key);
    if !checks_this_call(stats, call.verify_sample) {
        stats.sampled_out += 1;
        return Ok(Dispatch::Resumed);
    }
    let call_index = stats.calls - 1;
    let below_cap = stats.divergences < KEPT_DIVERGENCES;
    let keep_entry_state = below_cap && live.divergence_dir().is_some();
    let frame = live.frame_count();

    // Both runs record their CPU writes; the snapshot inherits the enabled log.
    let capture = live.start_cpu_write_capture();
    let mut snap = live.verify_snapshot();
    let entry_state = keep_entry_state.then(|| snap.verify_snapshot());
    // The bound of this call, from the entry state (the snapshot has not run yet).
    let max_cost = match call.bound {
        Some(bound) => Some(snap.cost_bound(call.key, bound, input, &mut NullAudio)),
        None => call.cycles,
    };
    // The timing window as `Rust` mode would see it at this entry.
    let until_event = live.master_cycles_until_timing_event();
    let gpdma_requested = live.gpdma_requested();
    let run = live.run_original(call.key, call.budget_frames, input, audio);
    let original_writes = live.finish_cpu_write_capture(capture);
    let run = run?;

    if !run.reached {
        live.stats_mut(call.key).no_return += 1;
        let name = live.hook_name(call.key);
        log::warn!(
            "{name}: the original did not return within {} frames; \
             demoted to Original for the rest of the run",
            call.budget_frames
        );
        live.demote_hook(call.key);
        return Ok(Dispatch::Ran { rendered: run.rendered });
    }
    if run.interrupted {
        let stats = live.stats_mut(call.key);
        stats.interrupted += 1;
        stats.hdma_crossed += u64::from(run.hdma_crossed);
        stats.stack_switched += u64::from(run.stack_switched);
        return Ok(Dispatch::Ran { rendered: run.rendered });
    }

    // The Rust function on the entry state, as `Rust` mode runs it but without the final charge.
    // The original code it calls runs there too, and the hits in it count on the snapshot only.
    let snap_hdma_start = snap.hdma_copy_units();
    let snap_nmis = snap.nmi_count();
    // The Rust function does what the original did: far more instructions than it needed is an
    // endless loop.
    snap.set_runaway_limit(run.instructions.saturating_mul(8).saturating_add(100_000));
    let tracing = std::env::var_os("SSD_VERIFY_TRACE").is_some();
    if tracing {
        eprintln!(
            "verify {:06X}: original {} instructions, {} cycles; running the Rust function",
            call.key, run.instructions, run.cpu_cycles
        );
    }
    let rust = snap.run_routine(call.key, call.func, call.ret, input, &mut NullAudio)?;
    if tracing {
        eprintln!(
            "verify {:06X}: rust cost {:?} instructions {:?} resume {:?} dropped {} runaway {}",
            call.key, rust.cost, rust.instructions, rust.resume, rust.dropped, rust.runaway
        );
    }
    let reads = snap.take_read_log();
    live.set_check_reads(reads);
    if snap.nmi_count() != snap_nmis {
        // The snapshot, which does not check the timing window, took an NMI the original did
        // not meet during its call: the two timelines differ, nothing to compare.
        live.stats_mut(call.key).sampled_out += 1;
        return Ok(Dispatch::Ran { rendered: run.rendered });
    }
    if rust.dropped {
        live.stats_mut(call.key).early_exit += 1;
        return Ok(Dispatch::Ran { rendered: run.rendered });
    }
    if rust.runaway {
        let stats = live.stats_mut(call.key);
        stats.divergences += 1;
        let routine = live.hook_name(call.key);
        let at = rust.resume.unwrap_or(0);
        let mut divergence =
            Divergence::new(routine, call_index, frame, Vec::new(), live.symbols());
        divergence.notes.push(format!(
            "the generated code ran away (an endless loop) and gave up at ${at:06X}; the original returned"
        ));
        if let Err(error) = live.record_divergence(divergence, entry_state.as_ref()) {
            log::warn!("cannot save the divergence report: {error}");
        }
        return Ok(Dispatch::Ran { rendered: run.rendered });
    }
    if rust.resume.is_some() {
        // A recompiled routine that hands over before its return leaves the snapshot mid-routine:
        // nothing to compare (counted as sampled out).
        live.stats_mut(call.key).sampled_out += 1;
        return Ok(Dispatch::Ran { rendered: run.rendered });
    }
    let rust_writes = snap.take_cpu_writes();
    // HDMA copying while the Rust function ran (the time it charged, the original code it
    // called) writes the PPU memories on the snapshot just as it does on the live machine: the
    // call is crossed if either saw it.
    let hdma_crossed = run.hdma_crossed || snap.hdma_copy_units() != snap_hdma_start;
    // Down to the lowest S of the original (its nested calls included; after a stack switch,
    // its entry S, see `RunOutcome::min_s`) and of the code the Rust function called.
    let lowest = rust.lowest_s.map_or(run.min_s, |rust_low| run.min_s.min(rust_low));
    let dead_stack = dead_stack(lowest, live.cpu_registers().s);
    // A routine past its kept divergences needs only to know whether the outcomes differ.
    let limit = if below_cap { usize::MAX } else { 1 };
    let differences = compare_except(
        &Outcome::of(live, &original_writes),
        &Outcome::of(&snap, &rust_writes),
        call.flags_mask,
        hdma_crossed,
        dead_stack,
        limit,
    );
    let bound = if rust.gated {
        None
    } else {
        cost_over_bound(rust.cost, rust.callee_cycles, rust.spent, max_cost)
    };
    let stats = live.stats_mut(call.key);
    stats.window_fit += u64::from(call_fits_window(until_event, gpdma_requested, run.cpu_cycles));
    stats.hdma_crossed += u64::from(hdma_crossed);
    stats.stack_switched += u64::from(run.stack_switched);
    stats.cost_over_bound += u64::from(bound.is_some());
    match rust.cost {
        None => stats.cost_missing += 1,
        // Both sides count CPU-busy cycles, which leave out the DMA and refresh time HDMA adds,
        // so a call HDMA crossed is compared like any other.
        Some(own) if own + rust.callee_cycles != run.cpu_cycles => stats.cost_mismatch += 1,
        Some(_) => {}
    }
    if let Some(own) = rust.instructions {
        let declared = (own as f64 + rust.callee_instructions).round() as u64;
        stats.instructions_mismatch += u64::from(declared != run.instructions);
    }
    if differences.is_empty() {
        stats.verified_ok += 1;
    } else {
        stats.divergences += 1;
    }
    let compared = (stats.verified_ok + stats.divergences) as f64;
    stats.mean_cpu_cycles += (run.cpu_cycles as f64 - stats.mean_cpu_cycles) / compared;
    // The instructions of the original, nested calls included (they run on the live machine).
    stats.mean_instructions += (run.instructions as f64 - stats.mean_instructions) / compared;
    if !differences.is_empty() && stats.divergences <= KEPT_DIVERGENCES {
        let routine = live.hook_name(call.key);
        let mut divergence =
            Divergence::new(routine.clone(), call_index, frame, differences, live.symbols());
        divergence.notes.extend(bound);
        // A report that cannot be saved must not stop the run.
        if let Err(error) = live.record_divergence(divergence, entry_state.as_ref()) {
            log::warn!("{routine}: cannot save the divergence report: {error}");
            live.stats_mut(call.key).report_errors += 1;
        }
    }
    Ok(Dispatch::Ran { rendered: run.rendered })
}

/// Whether this call is checked: the first of every `verify_sample` `Verify` calls (0 counts
/// as 1); the others are sampled out.
fn checks_this_call(stats: &HookStats, verify_sample: u32) -> bool {
    let checked = stats.verified_ok + stats.divergences + stats.interrupted + stats.no_return;
    (checked + stats.sampled_out).is_multiple_of(u64::from(verify_sample.max(1)))
}

/// WRAM offsets of the stack bytes left dead: pushed during the call (from `min_s + 1`, the
/// lowest stack pointer the original or the code the Rust function called reached) and free
/// after the original's return (up to `s_exit`, the stack pointer after it). The Rust function
/// does not push, so these bytes may differ without consequence. Only the bank-0 stack below
/// `$2000` is WRAM.
pub(crate) fn dead_stack(min_s: u16, s_exit: u16) -> Range<usize> {
    let start = usize::from(min_s) + 1;
    let end = (usize::from(s_exit) + 1).min(LOW_RAM_END);
    if start < end { start..end } else { 0..0 }
}

#[cfg(test)]
mod tests {
    use super::dead_stack;

    #[test]
    fn dead_stack_spans_the_pushed_bytes_up_to_the_exit_stack_pointer() {
        // JSR to S = $1FEE, PHX to $1FEC, PLX, RTS to $1FF0.
        assert_eq!(dead_stack(0x1FEC, 0x1FF0), 0x1FED..0x1FF1);
        // No push inside: the return address bytes only.
        assert_eq!(dead_stack(0x1FEE, 0x1FF0), 0x1FEF..0x1FF1);
        // Above low RAM the stack is not WRAM.
        assert_eq!(dead_stack(0x1FFE, 0x2001), 0x1FFF..0x2000);
        assert_eq!(dead_stack(0x2100, 0x2104), 0..0);
    }
}
