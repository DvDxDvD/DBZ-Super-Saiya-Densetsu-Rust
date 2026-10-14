//! `make-replays`: the reference replays of `.work/replays/`, recorded from the scripts of
//! `ssd_game::scripts` on the canonical ROM of the cache (no hooks, as `--record` of the app
//! records: through `ssd_oracle::Recorder`, from power-on with blank SRAM), then played back from
//! the written file with the sync points of `re/symbols.toml`: the playback must end in the
//! recorded run's state, and the count of sync events it saw is printed.

use crate::shared::read_replay;
use anyhow::Context;
use ssd_game::scripts::REPLAYS;
use ssd_oracle::{
    CORE_VERSION, HookEntry, HookTable, Joypad, Machine, MemorySaveStore, NullAudio, Recorder,
    Replay, Symbols,
};
use std::path::{Path, PathBuf};

/// What [`make`] wrote for one replay.
pub struct Made {
    pub path: PathBuf,
    pub frames: u64,
    pub bytes: u64,
    pub sync_events: u64,
}

/// Records every reference replay of `rom` into `dir` and checks it by playing the file back with
/// the sync points of `symbols`.
///
/// # Errors
///
/// A replay cannot be recorded, written, read back or played, or its playback does not end in the
/// recorded state.
pub fn make(rom: &[u8], symbols: &Symbols, dir: &Path) -> anyhow::Result<Vec<Made>> {
    let sync_points: HookTable = symbols
        .sync_points()
        .iter()
        .map(|point| HookEntry::sync_point(point.name.clone(), point.address))
        .collect();
    REPLAYS
        .iter()
        .map(|&(name, frames, script)| {
            let path = dir.join(format!("{name}.ssdreplay"));
            let recorded = record(rom, frames, script)?;
            recorded
                .replay
                .write_to(&path, CORE_VERSION)
                .with_context(|| format!("writing {}", path.display()))?;
            let bytes = std::fs::metadata(&path)?.len();
            let sync_events = play_back(rom, &path, &sync_points, &recorded.machine)
                .with_context(|| format!("checking {}", path.display()))?;
            Ok(Made { path, frames, bytes, sync_events })
        })
        .collect()
}

struct Recording {
    replay: Replay,
    machine: Machine,
}

/// `frames` frames of `script` from power-on, without hooks, recorded.
fn record(rom: &[u8], frames: u64, script: fn(u64) -> Joypad) -> anyhow::Result<Recording> {
    let mut machine = Machine::new(rom.to_vec(), Box::new(MemorySaveStore::default()))?;
    let mut recorder = Recorder::start(&machine)?;
    while machine.frame_count() < frames {
        let input = script(machine.frame_count());
        recorder.run_frame(&mut machine, input, &mut NullAudio)?;
    }
    Ok(Recording { replay: recorder.into_replay(), machine })
}

/// Plays the replay file at `path` with `sync_points` installed, counting sync events, and checks
/// that it ends in the state of `recorded`.
fn play_back(
    rom: &[u8],
    path: &Path,
    sync_points: &HookTable,
    recorded: &Machine,
) -> anyhow::Result<u64> {
    let replay = read_replay(path, rom)?;
    let mut machine = replay.power_on(rom.to_vec())?;
    machine.set_hook_table(Some(sync_points.clone()));
    let mut events = 0;
    while machine.frame_count() < replay.frames() {
        let budget = replay.frames() - machine.frame_count();
        let event =
            machine.run_until_sync(&mut |frame| replay.input(frame), &mut NullAudio, budget)?;
        events += u64::from(event.is_some());
    }
    anyhow::ensure!(
        machine.save_state() == recorded.save_state(),
        "the playback does not end in the recorded state"
    );
    Ok(events)
}
