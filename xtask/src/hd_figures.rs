//! `hd-art figures`: every frame of the replays, looked at for a field figure drawn partly from the
//! art and partly from the original's pixels (or from two arts), see [`crate::scene_mixed`]. It
//! lists each case once per run of frames (replay, map, first frame, frames, the figure and the
//! piece that goes with it) and exits 1 when there is any.

use crate::scene_mixed;
use crate::shared::read_replay;
use ssd_hd::pack::Pack;
use ssd_hd::world::{Scene, WideRun};
use ssd_hd::{HdOptions, Look};
use ssd_oracle::NullAudio;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// The main loop's state in work RAM.
const GAME_MODE: usize = 0x24;
const MAP: usize = 0x1A0;

/// A case seen over a run of frames.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub map: u8,
    pub first: u64,
    pub last: u64,
    pub frames: u64,
    pub name: String,
}

/// Plays `replay` and lists the mixed figures of its field frames.
///
/// # Errors
///
/// The replay or the machine fails.
pub fn scan(rom: &[u8], pack: &Arc<Pack>, replay: &Path) -> anyhow::Result<Vec<Run>> {
    let file = read_replay(replay, rom)?;
    let mut machine = file.power_on(rom.to_vec())?;
    machine.set_line_log(true);
    let mut run = WideRun::new(rom);
    run.set_art(Some(Arc::clone(pack)));
    let options = HdOptions { scale: 1, look: Look::Crisp };
    let mut open: BTreeMap<String, Run> = BTreeMap::new();
    let mut done: Vec<Run> = Vec::new();
    while machine.frame_count() < file.frames() {
        machine.run_frame_with(&mut |f| file.input(f), &mut NullAudio)?;
        let number = machine.frame_count() - 1;
        let battle = matches!(machine.wram()[GAME_MODE] & 0x7F, 0x02 | 0x04 | 0x0C);
        let art_id = |at: u32| {
            pack.art.objects().get(at as usize).map_or_else(|| "?".to_owned(), |o| o.id.clone())
        };
        let art_size = |at: u32| pack.art.objects().get(at as usize).map(|o| (o.width, o.height));
        let names: Vec<String> = if battle {
            run.note(machine.wram(), machine.frame_log());
            Vec::new()
        } else {
            match run.note_hd(machine.wram(), machine.frame_log(), options) {
                Some(shown) if matches!(shown.scene, Scene::Field) => {
                    scene_mixed::names(&run.art_stats().poses, &art_id, &art_size)
                }
                _ => Vec::new(),
            }
        };
        let ended: Vec<String> = open
            .iter()
            .filter(|(name, r)| !names.contains(name) && number > r.last + 1)
            .map(|(name, _)| name.clone())
            .collect();
        for name in ended {
            done.extend(open.remove(&name));
        }
        for name in names {
            let map = machine.wram()[MAP];
            let entry = open.entry(name.clone()).or_insert(Run {
                map,
                first: number,
                last: number,
                frames: 0,
                name,
            });
            entry.last = number;
            entry.frames += 1;
        }
    }
    done.extend(open.into_values());
    done.sort_by_key(|r| r.first);
    Ok(done)
}

/// Runs `hd-art figures`: the replays in `files`, `jobs` at once.
///
/// # Errors
///
/// The pack or a replay fails.
pub fn run(
    rom: &[u8],
    pack_dir: &Path,
    files: &[PathBuf],
    jobs: usize,
    out: &mut (dyn Write + Send),
) -> anyhow::Result<u8> {
    let pack = Arc::new(Pack::open(pack_dir).map_err(|e| anyhow::anyhow!("the pack: {e}"))?);
    let next = AtomicUsize::new(0);
    let total = AtomicUsize::new(0);
    let out = std::sync::Mutex::new(out);
    let failed = std::sync::Mutex::new(None);
    std::thread::scope(|scope| {
        for _ in 0..jobs.clamp(1, files.len().max(1)) {
            scope.spawn(|| {
                loop {
                    let at = next.fetch_add(1, Ordering::Relaxed);
                    let Some(file) = files.get(at) else { break };
                    let name = file
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    // (each replay is written as it ends: the long ones take a while)
                    let mut text = String::new();
                    match scan(rom, &pack, file) {
                        Ok(found) => {
                            let _ = writeln!(text, "{name}: {} case(s)", found.len());
                            for r in &found {
                                let _ = writeln!(
                                    text,
                                    "  map {} frames {}..{} ({}): {}",
                                    r.map, r.first, r.last, r.frames, r.name
                                );
                            }
                            total.fetch_add(found.len(), Ordering::Relaxed);
                        }
                        Err(e) => {
                            let _ = writeln!(text, "{name}: FAILED {e:#}");
                            *failed.lock().expect("failed") = Some(e);
                        }
                    }
                    let mut out = out.lock().expect("out");
                    let _ = std::io::Write::write_all(&mut *out, text.as_bytes());
                    let _ = out.flush();
                }
            });
        }
    });
    let out = out.into_inner().expect("out");
    let total = total.into_inner();
    writeln!(out, "hd-art figures: {total} mixed figure case(s) in {} replay(s)", files.len())?;
    if let Some(e) = failed.into_inner().expect("failed") {
        return Err(e);
    }
    Ok(u8::from(total > 0))
}

/// Walks the party of `party` (the first is the leader) on map `map`: each of the four directions
/// for `frames` frames, and lists the mixed figures of the frames and every frame's poses when
/// `dump` is on. The map and the party come from a crafted save, so a figure no replay shows can
/// be looked at.
///
/// # Errors
///
/// The save or the game fails.
pub fn walk(
    rom: &[u8],
    root: &Path,
    pack: &Arc<Pack>,
    (map, party): (u8, &str),
    (frames, dump): (u32, bool),
    gaps: Option<&Path>,
    out: &mut dyn Write,
) -> anyhow::Result<usize> {
    use ssd_oracle::Joypad;
    let base = std::fs::read(crate::tour::find_base_save(root)?)?;
    let game = crate::save::Game::from_rom(rom)?;
    let Some(mut session) =
        crate::tour_visit::boot_free_with(rom, &game, &base, map, None, Some(party))?
    else {
        writeln!(out, "map {map} party {party}: the party is never free")?;
        return Ok(0);
    };
    let mut run = WideRun::new(rom);
    run.set_art(Some(Arc::clone(pack)));
    let options = HdOptions { scale: 1, look: Look::Crisp };
    let art_id = |at: u32| {
        pack.art.objects().get(at as usize).map_or_else(|| "?".to_owned(), |o| o.id.clone())
    };
    run.set_tally(gaps.is_some());
    let mut written = std::collections::BTreeSet::new();
    let mut total = 0;
    for (name, pad) in [
        ("right", Joypad { right: true, ..Joypad::default() }),
        ("down", Joypad { down: true, ..Joypad::default() }),
        ("left", Joypad { left: true, ..Joypad::default() }),
        ("up", Joypad { up: true, ..Joypad::default() }),
    ] {
        let mut cases: BTreeMap<String, u32> = BTreeMap::new();
        for t in 0..frames {
            session.step(pad)?;
            if matches!(session.mode(), 2 | 4 | 12) {
                break;
            }
            if run.note_hd(session.machine.wram(), session.machine.frame_log(), options).is_none() {
                continue;
            }
            let poses = run.art_stats().poses.clone();
            if let Some(dir) = gaps {
                for shape in run.tally().shapes.iter().filter(|s| s.field && !s.shadow) {
                    if !written.insert(shape.key) {
                        continue;
                    }
                    std::fs::create_dir_all(dir)?;
                    let json = serde_json::json!({
                        "key": format!("{:016x}", shape.key),
                        "fighter": shape.fighter,
                        "field": shape.field,
                        "shadow": shape.shadow,
                        "fate": format!("{:?}", shape.fate),
                        "width": shape.w,
                        "height": shape.h,
                        "origin": [shape.org.0, shape.org.1],
                        "oam_palette": shape.palette,
                        "palette_row": shape.row,
                        "indexes": shape.arr,
                        "first_seen": format!("map {map} party {party} {name} frame {t}"),
                    });
                    std::fs::write(
                        dir.join(format!("field-missing-{:016x}.json", shape.key)),
                        json.to_string(),
                    )?;
                    writeln!(
                        out,
                        "  gap: {:016x} {}x{} pal {}",
                        shape.key, shape.w, shape.h, shape.palette
                    )?;
                }
            }
            let found = scene_mixed::find(&poses);
            let art_size =
                |at: u32| pack.art.objects().get(at as usize).map(|o| (o.width, o.height));
            for name in scene_mixed::names(&poses, &art_id, &art_size) {
                *cases.entry(name).or_default() += 1;
            }
            if dump {
                for case in &found {
                    if let Some(p) = poses.iter().find(|p| p.key == case.other) {
                        writeln!(
                            out,
                            "  {name} frame {t}: CASE piece pal {} org {:?} size {:?} vis {} sprites {:x} field {}",
                            p.palette, p.org, p.size, p.visible, p.sprites, p.field
                        )?;
                    }
                }
                writeln!(out, "  {name} frame {t}:")?;
                for p in &poses {
                    writeln!(
                        out,
                        "    {} pal {} org {:?} size {:?} vis {} sprites {:x} art {} field {}",
                        if p.shadow {
                            "shadow"
                        } else if p.matched {
                            "ART"
                        } else {
                            "bare"
                        },
                        p.palette,
                        p.org,
                        p.size,
                        p.visible,
                        p.sprites,
                        match p.art {
                            Some(at) => art_id(at),
                            None => "-".to_owned(),
                        },
                        p.field
                    )?;
                }
            }
        }
        for (case, count) in &cases {
            writeln!(out, "map {map} party {party} {name}: {count} frame(s): {case}")?;
            total += 1;
        }
    }
    Ok(total)
}
