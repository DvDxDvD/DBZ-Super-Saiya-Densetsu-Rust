//! `framecheck`: plays a replay on two machines, the original and one with every routine in
//! `Rust`, frame by frame, and reports the first frame at which their WRAM or master cycles
//! differ, with the routines that ran in Rust during it. A debugging aid for the recompiler.

use crate::shared::read_replay;
use anyhow::Context;
use ssd_oracle::{HookMode, NullAudio, Stopped, Stops, Symbols};
use std::io::Write;
use std::path::Path;

/// Plays `replay` in both modes until the first frame that differs.
///
/// # Errors
///
/// The replay cannot be read or a run fails.
pub fn run(
    rom: &[u8],
    symbols: &Symbols,
    replay: &Path,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    let replay = read_replay(replay, rom)?;
    let table = ssd_game::hook_table(symbols, Some(HookMode::Rust))?;
    let mut a = replay.power_on(rom.to_vec())?;
    let mut b = replay.power_on(rom.to_vec())?;
    b.set_hook_table(Some(table));
    let stops = Stops { after_dispatch: true, ..Stops::default() };
    let mut last_ran = String::new();
    let mut dispatches = 0_u64;
    // Dead stack (below S, down to the lowest S the original reached since the two machines
    // last agreed on those bytes) may differ: a routine that pushes and pulls leaves its pushed
    // bytes there, which the Rust function does not write.
    let mut stack_floor = a.cpu_registers().s;
    let mut shown = 0;
    let trace_range = std::env::var("FRAMECHECK_A_TRACE").ok().and_then(|v| {
        let (from, to) = v.split_once(',')?;
        Some((from.parse::<u64>().ok()?, to.parse::<u64>().ok()?))
    });
    while b.frame_count() < replay.frames() {
        let stopped =
            b.run(&stops, &mut |f| replay.input(f), &mut NullAudio, 1).context("machine B")?;
        let Stopped::Dispatched { name, ran: true, .. } = stopped else { continue };
        dispatches += 1;
        // A runs on to the master cycle B reached.
        while a.total_master_cycles() < b.total_master_cycles() {
            a.tick(replay.input(a.frame_count()), &mut NullAudio).context("machine A")?;
            stack_floor = stack_floor.min(a.cpu_registers().s);
            if let Some((from, to)) = trace_range
                && (from..to).contains(&a.total_master_cycles())
                && !a.is_mid_instruction()
            {
                eprintln!("A {:06X} {}", a.cpu_registers().pc, a.total_master_cycles());
            }
        }
        let cycles = b.total_master_cycles().wrapping_sub(a.total_master_cycles()) as i64;
        let (ra, rb) = (a.cpu_registers(), b.cpu_registers());
        let same_regs = format!("{ra:?}") == format!("{rb:?}");
        let (wa, wb) = (a.wram(), b.wram());
        let dead_stack = usize::from(stack_floor) + 1..=usize::from(ra.s);
        let differs = |i: &usize| wa[*i] != wb[*i];
        let all_differing: Vec<usize> = (0..wa.len()).filter(differs).collect();
        let differing: Vec<usize> =
            all_differing.iter().copied().filter(|i| !dead_stack.contains(i)).collect();
        stack_floor = dead_stack.clone().find(differs).map_or(ra.s, |i| (i - 1) as u16);
        let same_phase = a.is_mid_instruction() == b.is_mid_instruction();
        if (-12..=0).contains(&cycles) && same_regs && same_phase && differing.is_empty() {
            if cycles != 0 && shown < 30 {
                shown += 1;
                writeln!(out, "  note: after dispatch #{dispatches} of {name}: B - A = {cycles}")?;
            }
            last_ran = name;
            continue;
        }
        writeln!(
            out,
            "framecheck: after dispatch #{dispatches} of {name} (frame {}; the dispatch before:              {last_ran}): master cycles B - A = {cycles}, same registers {same_regs}, same phase              {same_phase}, {} WRAM bytes differ",
            b.frame_count(),
            differing.len()
        )?;
        writeln!(out, "  A: {ra:?}")?;
        writeln!(out, "  B: {rb:?}")?;
        writeln!(
            out,
            "  mid-instruction: A {} B {}",
            a.is_mid_instruction(),
            b.is_mid_instruction()
        )?;
        for &i in differing.iter().take(24) {
            writeln!(out, "  wram[{i:05X}] A={:02X} B={:02X}", wa[i], wb[i])?;
        }
        return Ok(());
    }
    writeln!(out, "framecheck: {dispatches} dispatches identical")?;
    Ok(())
}
