//! `hd-art`: what the HD art does in the reference replays.
//!
//! * `report` plays replays with the pack's art on, draws every frame in the wide view as the
//!   Enhanced picture and says how much of what the art covers is drawn from it: the pixels of
//!   fighters' and effects' sprites by what became of them (art drawn, a pose the index lacks, a
//!   partial or cut pose), the pixels drawn from portraits, and the time a frame took.
//! * `colours` compares the colours the art draws with those of the original's pixels it replaces,
//!   per art file (the mean colour inside the pose's mask), and lists the ones that differ most.
//! * `shots` writes chosen frames twice, with the art and without, at the wide 16:9 view
//!   (`<label>-art.png`, `<label>-plain.png`, and the two side by side as `<label>-both.png`).

use crate::shared::{read_replay, render_table};
use anyhow::Context;
use ssd_hd::pack::Pack;
use ssd_hd::world::{Scene, WIDTH_16_9, WideHd, WideRun};
use ssd_hd::{ArtStats, HdOptions, KindPixels, Look};
use ssd_oracle::NullAudio;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The main loop's state in work RAM.
const GAME_MODE: usize = 0x24;

/// What `hd-art` was asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Args {
    Report {
        paths: Vec<PathBuf>,
        scale: u32,
        frames: Option<u64>,
        jobs: usize,
        pack: Option<PathBuf>,
        /// Draw the art (without it the report times the look alone).
        art: bool,
    },
    Colours {
        paths: Vec<PathBuf>,
        scale: u32,
        frames: Option<u64>,
        jobs: usize,
        pack: Option<PathBuf>,
    },
    BgScan {
        paths: Vec<PathBuf>,
    },
    PicScan {
        out: PathBuf,
        paths: Vec<PathBuf>,
    },
    /// The pictures of three packs compared (see `run_diff`).
    Diff {
        paths: Vec<PathBuf>,
        packs: [PathBuf; 3],
        every: u64,
    },
    /// The palettes the art of the backgrounds and maps was made in.
    RefPal {
        paths: Vec<PathBuf>,
        pack: Option<PathBuf>,
        maps_dump: Option<PathBuf>,
        out: Option<PathBuf>,
    },
    BgDump {
        replay: PathBuf,
        frames: Vec<u64>,
        out: PathBuf,
    },
    M7Scan {
        paths: Vec<PathBuf>,
    },
    M7Dump {
        replay: PathBuf,
        frame: u64,
        out: PathBuf,
    },
    Scan {
        replay: PathBuf,
        every: u64,
        pack: Option<PathBuf>,
    },
    /// The picture of every object, as the original and the HD Items menu draw it.
    Items {
        out: PathBuf,
        pack: Option<PathBuf>,
    },
    Shots {
        out: Option<PathBuf>,
        scale: u32,
        shots: Vec<(String, PathBuf, u64)>,
        pack: Option<PathBuf>,
        /// Draw the HD text font (the default of the app).
        font: bool,
    },
    /// The field's sprites in the margins of the 16:9 view against the ones in the window.
    Margins(crate::hd_margins::Args),
}

/// Parses the arguments after `hd-art`.
///
/// # Errors
///
/// An unknown subcommand or option, or a bad number.
pub fn parse_args(args: &[&str]) -> Result<Args, String> {
    let mut iter = args.iter();
    let command = iter
        .next()
        .copied()
        .ok_or("a subcommand: report, colours, shots, margins, items or scan")?;
    let mut every = 300_u64;
    let (mut scale, mut frames, mut out, mut jobs) = (1_u32, None, None, 4_usize);
    let mut pack = None;
    let mut maps_dump = None;
    let (mut pack_b, mut pack_c) = (None, None);
    let mut art = true;
    let mut font = true;
    let (mut paths, mut shots) = (Vec::new(), Vec::new());
    let (mut maps, mut base, mut list, mut margin_shots) = (Vec::new(), None, false, Vec::new());
    let (mut find, mut objects) = (None, false);
    while let Some(&arg) = iter.next() {
        match arg {
            "--scale" => {
                scale = iter
                    .next()
                    .and_then(|v| v.parse().ok())
                    .filter(|s| (1..=8).contains(s))
                    .ok_or("--scale takes a number from 1 to 8")?;
            }
            "--frames" => {
                frames = Some(
                    iter.next().and_then(|v| v.parse().ok()).ok_or("--frames takes a number")?,
                );
            }
            "--jobs" => {
                jobs = iter
                    .next()
                    .and_then(|v| v.parse().ok())
                    .filter(|&n| n > 0)
                    .ok_or("--jobs takes a number")?;
            }
            "--every" => {
                every = iter
                    .next()
                    .and_then(|v| v.parse().ok())
                    .filter(|&n| n > 0)
                    .ok_or("--every takes a number")?;
            }
            "--maps" => {
                maps = iter
                    .next()
                    .ok_or("--maps takes a list of map numbers")?
                    .split(',')
                    .map(|n| n.parse().map_err(|_| "--maps takes a list of map numbers".to_owned()))
                    .collect::<Result<_, _>>()?;
            }
            "--base" => base = Some(PathBuf::from(iter.next().ok_or("--base takes a save")?)),
            "--list" => list = true,
            "--objects" => objects = true,
            "--find" => {
                find = Some((*iter.next().ok_or("--find takes an art id prefix")?).to_owned());
            }
            "--no-art" => art = false,
            "--no-font" => font = false,
            "--pack" => pack = Some(PathBuf::from(iter.next().ok_or("--pack takes a folder")?)),
            "--pack-b" => {
                pack_b = Some(PathBuf::from(iter.next().ok_or("--pack-b takes a folder")?));
            }
            "--pack-c" => {
                pack_c = Some(PathBuf::from(iter.next().ok_or("--pack-c takes a folder")?));
            }
            "--maps-dump" => {
                maps_dump = Some(PathBuf::from(iter.next().ok_or("--maps-dump takes a folder")?));
            }
            "--out" => out = Some(PathBuf::from(iter.next().ok_or("--out takes a folder")?)),
            option if option.starts_with("--") => return Err(format!("unknown option `{option}`")),
            value if command == "margins" => {
                margin_shots.push(crate::hd_margins::parse_shot(value)?);
            }
            value if command == "shots" => {
                let (label, rest) = value.split_once('=').ok_or("label=replay:frame")?;
                let (replay, frame) = rest.rsplit_once(':').ok_or("label=replay:frame")?;
                let frame = frame.parse().map_err(|_| "label=replay:frame")?;
                shots.push((label.to_owned(), PathBuf::from(replay), frame));
            }
            value => paths.push(PathBuf::from(value)),
        }
    }
    match command {
        "report" => Ok(Args::Report { paths, scale, frames, jobs, pack, art }),
        "colours" => Ok(Args::Colours { paths, scale, frames, jobs, pack }),
        "bgscan" => Ok(Args::BgScan { paths }),
        "refpal" => Ok(Args::RefPal { paths, pack, maps_dump, out }),
        "diff" => match (pack, pack_b, pack_c) {
            (Some(a), Some(b), Some(c)) => Ok(Args::Diff {
                paths,
                packs: [a, b, c],
                every: if every == 300 { 4 } else { every },
            }),
            _ => Err("diff needs --pack A --pack-b B --pack-c C".to_owned()),
        },
        "m7scan" => Ok(Args::M7Scan { paths }),
        "m7dump" => match paths.as_slice() {
            [replay, frame, out] => Ok(Args::M7Dump {
                replay: replay.clone(),
                frame: frame
                    .to_string_lossy()
                    .parse()
                    .map_err(|_| "m7dump: a frame number".to_owned())?,
                out: out.clone(),
            }),
            _ => Err("m7dump <replay> <frame> <out dir>".to_owned()),
        },
        "picscan" => match paths.split_first() {
            Some((out, rest)) => Ok(Args::PicScan { out: out.clone(), paths: rest.to_vec() }),
            None => Err("picscan <out dir> [replay|dir...]".to_owned()),
        },
        "bgdump" => match paths.as_slice() {
            [replay, frames, out] => Ok(Args::BgDump {
                replay: replay.clone(),
                frames: frames
                    .to_string_lossy()
                    .split(',')
                    .map(|n| n.parse().map_err(|_| "bgdump: frame numbers".to_owned()))
                    .collect::<Result<_, _>>()?,
                out: out.clone(),
            }),
            _ => Err("bgdump <replay> <frame,frame...> <out dir>".to_owned()),
        },
        "items" => {
            Ok(Args::Items { out: out.unwrap_or_else(|| PathBuf::from(".work/items/menu")), pack })
        }
        "scan" => match paths.as_slice() {
            [replay] => Ok(Args::Scan { replay: replay.clone(), every, pack }),
            _ => Err("scan takes one replay".to_owned()),
        },
        "shots" => {
            Ok(Args::Shots { out, scale: if scale == 1 { 4 } else { scale }, shots, pack, font })
        }
        "margins" => Ok(Args::Margins(crate::hd_margins::Args {
            maps,
            scale: if scale == 1 && !margin_shots.is_empty() { 4 } else { scale },
            out,
            pack,
            base,
            shots: margin_shots,
            jobs,
            list,
            find,
            objects,
        })),
        other => Err(format!("unknown subcommand `{other}`")),
    }
}

fn open_pack(dir: &Path) -> anyhow::Result<Arc<Pack>> {
    let pack = Pack::open(dir).with_context(|| format!("opening the pack {}", dir.display()))?;
    anyhow::ensure!(
        !pack.art.pictures().is_empty() || !pack.art.objects().is_empty(),
        "the pack {} has no art: run hd-import",
        dir.display()
    );
    Ok(Arc::new(pack))
}

/// The battle's scenes: the command screen (`$02`), the animation (`$04`) and the last fights
/// (`$0C`). Fighters are counted in all of them, effects in the animations only: the command
/// screen's other sprites are its cursor and its cards.
fn is_battle(wram: &[u8]) -> bool {
    matches!(wram[GAME_MODE] & 0x7F, 0x02 | 0x04 | 0x0C)
}

fn is_animation(wram: &[u8]) -> bool {
    matches!(wram[GAME_MODE] & 0x7F, 0x04 | 0x0C)
}

/// What one replay's frames added up to.
#[derive(Debug, Default, Clone)]
struct Outcome {
    name: String,
    frames: u64,
    refused: u64,
    battle: u64,
    /// Battle frames with a fighter or an effect on screen.
    with_poses: u64,
    fighters: KindPixels,
    effects: KindPixels,
    /// The field's sprite groups, in the frames that are not the battle's.
    field: KindPixels,
    /// Every pixel of the object layer in the frames drawn as the field, inside the original window
    /// and in the margins of the 16:9 view (see [`ArtStats::sprites_margin`]).
    field_window: KindPixels,
    field_margin: KindPixels,
    /// The same by the game mode (`$24` of work RAM), with the frames of each.
    field_modes: BTreeMap<u8, (u64, KindPixels)>,
    /// Frames with a pose whose art draws under half the opaque pixels of its sprites, by game
    /// mode: the frames and the poses.
    thin: BTreeMap<u8, (u64, u64)>,
    /// The keys of those poses: the frames, and the first frame and replay.
    thin_keys: BTreeMap<u64, (u64, u64, String)>,
    picture_pixels: u64,
    /// Pixels of battle backgrounds, in the battle's frames, and those drawn from the art of a layer.
    bg_pixels: u64,
    bg_art: u64,
    picture_recoloured: u64,
    portrait_without_art: u64,
    /// Frames where any picture pixel was drawn, and where any pose was.
    picture_frames: u64,
    pose_frames: u64,
    time: Duration,
    /// The time of each frame to draw, in milliseconds: the battle's frames and the others.
    battle_ms: Vec<f32>,
    other_ms: Vec<f32>,
    /// The keys of poses the index lacks, with the pixels they showed.
    missing: BTreeMap<u64, (u64, u32, u32)>,
}

fn add(total: &mut KindPixels, more: KindPixels) {
    total.total += more.total;
    total.art += more.art;
    total.recoloured += more.recoloured;
    total.missing += more.missing;
    total.partial += more.partial;
}

/// The poses of a frame whose art draws fewer opaque pixels than `THIN_SHARE` of the pixels their
/// sprites show (at least `THIN_MIN` of them): art that is missing or erased, or placed wrong.
fn thin_poses(stats: &ArtStats, scale: u32) -> Vec<u64> {
    let per_pixel = 255 * u64::from(scale) * u64::from(scale);
    stats
        .drawn
        .iter()
        .filter(|d| !d.luma && d.drawn.visible >= THIN_MIN)
        .filter(|d| d.drawn.weight * 100 < u64::from(d.drawn.visible) * per_pixel * THIN_SHARE)
        .map(|d| d.key)
        .collect()
}

/// The share (percent) of a sprite's visible pixels its art must draw to pass, and the fewest
/// visible pixels that count.
const THIN_SHARE: u64 = 50;
const THIN_MIN: u32 = 40;

fn tally(
    outcome: &mut Outcome,
    stats: &ArtStats,
    (mode, frame, scale): (u8, u64, u32),
    battle: bool,
    animation: bool,
    field_scene: bool,
) {
    if field_scene {
        add(&mut outcome.field_window, stats.sprites_window);
        add(&mut outcome.field_margin, stats.sprites_margin);
    }
    let thin = thin_poses(stats, scale);
    if !thin.is_empty() {
        let entry = outcome.thin.entry(mode).or_default();
        entry.0 += 1;
        entry.1 += thin.len() as u64;
        for key in thin {
            let entry = outcome.thin_keys.entry(key).or_insert((0, frame, outcome.name.clone()));
            entry.0 += 1;
        }
    }
    if battle {
        outcome.battle += 1;
        if stats.fighters.total + stats.effects.total > 0 {
            outcome.with_poses += 1;
        }
        add(&mut outcome.fighters, stats.fighters);
        outcome.bg_pixels += stats.bg_pixels;
        outcome.bg_art += stats.bg_art;
        if animation {
            add(&mut outcome.effects, stats.effects);
        }
        if stats.fighters.art + stats.effects.art > 0 {
            outcome.pose_frames += 1;
        }
        for pose in stats.poses.iter().filter(|p| !p.matched && p.opaque >= 40 && p.stripy < 200) {
            let entry = outcome.missing.entry(pose.key).or_default();
            entry.0 += u64::from(pose.visible);
            entry.1 += 1;
            entry.2 = pose.opaque;
        }
    } else {
        add(&mut outcome.field, stats.field);
        let entry = outcome.field_modes.entry(mode).or_default();
        entry.0 += 1;
        add(&mut entry.1, stats.field);
        for pose in
            stats.poses.iter().filter(|p| p.field && !p.matched && p.opaque >= 40 && p.stripy < 200)
        {
            let entry = outcome.missing.entry(pose.key).or_default();
            entry.0 += u64::from(pose.visible);
            entry.1 += 1;
            entry.2 = pose.opaque;
        }
    }
    outcome.picture_pixels += stats.picture_pixels;
    outcome.picture_recoloured += stats.picture_recoloured;
    outcome.portrait_without_art += stats.portrait_pixels_without_art;
    if stats.picture_pixels > 0 {
        outcome.picture_frames += 1;
    }
}

fn check_replay(
    rom: &[u8],
    pack: Option<&Arc<Pack>>,
    path: &Path,
    scale: u32,
    limit: Option<u64>,
) -> anyhow::Result<Outcome> {
    let replay = read_replay(path, rom)?;
    let name = path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    let frames = replay.frames().min(limit.unwrap_or(u64::MAX));
    let mut outcome = Outcome { name, frames, ..Outcome::default() };
    let mut machine = replay.power_on(rom.to_vec())?;
    machine.set_line_log(true);
    let mut run = WideRun::new(rom);
    run.set_art(pack.cloned());
    run.set_art_measure(true);
    let options = HdOptions { scale, look: Look::Crisp };
    while machine.frame_count() < frames {
        machine.run_frame_with(&mut |f| replay.input(f), &mut NullAudio)?;
        let battle = is_battle(machine.wram());
        let animation = is_animation(machine.wram());
        let started = Instant::now();
        let picture = run.note_hd(machine.wram(), machine.frame_log(), options);
        let spent = started.elapsed();
        outcome.time += spent;
        let list = if battle { &mut outcome.battle_ms } else { &mut outcome.other_ms };
        list.push(spent.as_secs_f32() * 1000.0);
        match &picture {
            Some(picture) => {
                tally(
                    &mut outcome,
                    &run.art_stats(),
                    (machine.wram()[GAME_MODE] & 0x7F, machine.frame_count() - 1, scale),
                    battle,
                    animation,
                    picture.scene == Scene::Field,
                );
            }
            None => outcome.refused += 1,
        }
    }
    Ok(outcome)
}

fn percent(part: u64, whole: u64) -> String {
    if whole == 0 { "-".to_owned() } else { format!("{:.1}", part as f64 * 100.0 / whole as f64) }
}

/// How a report is made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReportOptions {
    /// The scale the pictures are drawn at.
    pub scale: u32,
    /// Frames played of each replay (all by default).
    pub limit: Option<u64>,
    /// Replays played at once.
    pub jobs: usize,
    /// Whether the art is drawn (without it the report times the look alone).
    pub art: bool,
}

/// Plays the replays with the art and reports.
///
/// # Errors
///
/// The pack has no art, or a replay cannot be read or played.
pub fn run_report(
    rom: &[u8],
    pack_dir: &Path,
    files: &[PathBuf],
    options: ReportOptions,
    out: &mut dyn Write,
) -> anyhow::Result<u8> {
    let ReportOptions { scale, limit, jobs, art } = options;
    let pack = if art { Some(open_pack(pack_dir)?) } else { None };
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<(usize, anyhow::Result<Outcome>)>> = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..jobs.min(files.len()) {
            scope.spawn(|| {
                loop {
                    let at = next.fetch_add(1, Ordering::Relaxed);
                    let Some(path) = files.get(at) else { break };
                    let result = check_replay(rom, pack.as_ref(), path, scale, limit);
                    results.lock().expect("results").push((at, result));
                }
            });
        }
    });
    let mut results = results.into_inner().expect("results");
    results.sort_by_key(|(at, _)| *at);
    let mut outcomes = Vec::new();
    for (_, result) in results {
        outcomes.push(result?);
    }
    let mut rows = Vec::new();
    let mut total = Outcome::default();
    for o in &outcomes {
        rows.push(vec![
            o.name.clone(),
            o.frames.to_string(),
            o.battle.to_string(),
            percent(o.fighters.art, o.fighters.total),
            percent(o.fighters.recoloured, o.fighters.total),
            percent(o.fighters.missing, o.fighters.total),
            percent(o.fighters.partial, o.fighters.total),
            percent(o.effects.art, o.effects.total),
            percent(o.effects.missing + o.effects.partial, o.effects.total),
            percent(o.field.art, o.field.total),
            o.picture_pixels.to_string(),
            percent(o.bg_art, o.bg_pixels),
            o.portrait_without_art.to_string(),
            format!("{:.2}", o.time.as_secs_f64() * 1000.0 / o.frames.max(1) as f64),
        ]);
        total.frames += o.frames;
        total.refused += o.refused;
        total.battle += o.battle;
        add(&mut total.fighters, o.fighters);
        add(&mut total.effects, o.effects);
        add(&mut total.field, o.field);
        add(&mut total.field_window, o.field_window);
        add(&mut total.field_margin, o.field_margin);
        for (mode, (frames, poses)) in &o.thin {
            let entry = total.thin.entry(*mode).or_default();
            entry.0 += frames;
            entry.1 += poses;
        }
        for (key, (frames, first, replay)) in &o.thin_keys {
            let entry = total.thin_keys.entry(*key).or_insert((0, *first, replay.clone()));
            entry.0 += frames;
        }
        for (mode, (frames, k)) in &o.field_modes {
            let entry = total.field_modes.entry(*mode).or_default();
            entry.0 += frames;
            add(&mut entry.1, *k);
        }
        total.picture_pixels += o.picture_pixels;
        total.bg_pixels += o.bg_pixels;
        total.bg_art += o.bg_art;
        total.picture_recoloured += o.picture_recoloured;
        total.portrait_without_art += o.portrait_without_art;
        total.time += o.time;
        total.battle_ms.extend_from_slice(&o.battle_ms);
        total.other_ms.extend_from_slice(&o.other_ms);
        for (key, value) in &o.missing {
            let entry = total.missing.entry(*key).or_default();
            entry.0 += value.0;
            entry.1 += value.1;
            entry.2 = value.2;
        }
    }
    write!(
        out,
        "{}",
        render_table(
            &[
                "replay",
                "frames",
                "battle",
                "fight% art",
                "recol",
                "miss",
                "part",
                "fx% art",
                "fx other",
                "field% art",
                "portrait px",
                "bg% art",
                "no art px",
                "ms/frame",
            ],
            &rows
        )
    )?;
    writeln!(
        out,
        "hd-art report at {scale}x, 16:9 ({WIDTH_16_9} wide): {} frames, {} refused",
        total.frames, total.refused
    )?;
    let show = |name: &str, k: KindPixels, out: &mut dyn Write| {
        writeln!(
            out,
            "  {name}: {} visible pixels, {} drawn from HD art ({}%, of which {} recoloured from another palette's art, {}% of all), {} of poses the index lacks ({}%), {} of partial or cut poses ({}%)",
            k.total,
            k.art,
            percent(k.art, k.total),
            k.recoloured,
            percent(k.recoloured, k.total),
            k.missing,
            percent(k.missing, k.total),
            k.partial,
            percent(k.partial, k.total)
        )
    };
    show("fighters", total.fighters, out)?;
    show("effects ", total.effects, out)?;
    show("field   ", total.field, out)?;
    for (mode, (frames, k)) in &total.field_modes {
        if k.total * 100 >= total.field.total {
            writeln!(
                out,
                "    mode {mode:02X}: {frames} frames, {} sprite pixels, {}% drawn from HD art, {}% of poses the index lacks",
                k.total,
                percent(k.art, k.total),
                percent(k.missing, k.total)
            )?;
        }
    }
    writeln!(
        out,
        "  field sprites by where they are (every pixel of the object layer in the field's frames): inside the original window {} px, {}% drawn from HD art; in the margins of the view {} px, {}%",
        total.field_window.total,
        percent(total.field_window.art, total.field_window.total),
        total.field_margin.total,
        percent(total.field_margin.art, total.field_margin.total)
    )?;
    let margins_fail = !crate::hd_margins::margins_match(total.field_window, total.field_margin);
    if margins_fail {
        writeln!(
            out,
            "  FAIL: the sprites in the margins are drawn from art {} points less than the ones in the window",
            crate::hd_margins::share(total.field_window.art, total.field_window.total)
                .saturating_sub(crate::hd_margins::share(
                    total.field_margin.art,
                    total.field_margin.total
                ))
        )?;
    }
    let flagged: u64 = total.thin.values().map(|(frames, _)| frames).sum();
    let by_mode: Vec<String> = total
        .thin
        .iter()
        .map(|(mode, (frames, poses))| format!(", mode {mode:02X}: {frames} ({poses} poses)"))
        .collect();
    let by_mode = by_mode.concat();
    writeln!(
        out,
        "  thin art (a pose whose art draws under {THIN_SHARE}% of the opaque pixels of its sprites): {flagged} frames{by_mode}"
    )?;
    let mut thin_keys: Vec<_> = total.thin_keys.iter().collect();
    thin_keys.sort_by_key(|(_, v)| std::cmp::Reverse(v.0));
    for (key, (frames, first, replay)) in thin_keys.iter().take(8) {
        writeln!(out, "    thin {key:016x}: {frames} frames, first {replay} frame {first}")?;
    }
    writeln!(
        out,
        "  battle backgrounds: {} visible pixels of the layers and their backdrop, {} drawn from HD art ({}%)",
        total.bg_pixels,
        total.bg_art,
        percent(total.bg_art, total.bg_pixels)
    )?;
    writeln!(
        out,
        "  portraits and faces: {} pixels drawn from HD art ({} of them recoloured to the palette row the game uses), {} pixels of portrait characters without art",
        total.picture_pixels, total.picture_recoloured, total.portrait_without_art
    )?;
    writeln!(
        out,
        "  mean {:.2} ms a frame to draw the Enhanced picture",
        total.time.as_secs_f64() * 1000.0 / total.frames.max(1) as f64
    )?;
    let summary = |name: &str, times: &mut Vec<f32>, out: &mut dyn Write| {
        if times.is_empty() {
            return Ok(());
        }
        times.sort_by(f32::total_cmp);
        let mean = times.iter().sum::<f32>() / times.len() as f32;
        let p99 = times[(times.len() * 99 / 100).min(times.len() - 1)];
        writeln!(
            out,
            "  {name}: {} frames, mean {mean:.2} ms, 99th percentile {p99:.2} ms, worst {:.2} ms",
            times.len(),
            times[times.len() - 1]
        )
    };
    summary("battle frames", &mut total.battle_ms, out)?;
    summary("other frames ", &mut total.other_ms, out)?;
    let mut missing: Vec<_> = total.missing.iter().collect();
    missing.sort_by_key(|(_, v)| std::cmp::Reverse(v.0));
    for (key, (pixels, frames, opaque)) in missing.iter().take(8) {
        writeln!(
            out,
            "  lacks {key:016x}: {pixels} pixels in {frames} frames ({opaque} opaque px)"
        )?;
    }
    Ok(u8::from(margins_fail))
}

/// The colours drawn and the original's under them, summed over the frames of a pose's art.
#[derive(Debug, Default, Clone, Copy)]
struct ColourSum {
    frames: u64,
    weight: u64,
    shown: [u64; 3],
    original: [u64; 3],
    luma: bool,
    fighter: bool,
}

fn colours_of_replay(
    rom: &[u8],
    pack: &Arc<Pack>,
    path: &Path,
    scale: u32,
    limit: Option<u64>,
) -> anyhow::Result<BTreeMap<u64, ColourSum>> {
    let replay = read_replay(path, rom)?;
    let frames = replay.frames().min(limit.unwrap_or(u64::MAX));
    let mut machine = replay.power_on(rom.to_vec())?;
    machine.set_line_log(true);
    let mut run = WideRun::new(rom);
    run.set_art(Some(Arc::clone(pack)));
    run.set_art_measure(true);
    let options = HdOptions { scale, look: Look::Crisp };
    let mut sums: BTreeMap<u64, ColourSum> = BTreeMap::new();
    while machine.frame_count() < frames {
        machine.run_frame_with(&mut |f| replay.input(f), &mut NullAudio)?;
        if run.note_hd(machine.wram(), machine.frame_log(), options).is_none() {
            continue;
        }
        for pose in run.art_stats().drawn {
            let entry = sums.entry(pose.key).or_default();
            entry.frames += 1;
            entry.weight += pose.drawn.weight;
            for channel in 0..3 {
                entry.shown[channel] += pose.drawn.shown[channel];
                entry.original[channel] += pose.drawn.original[channel];
            }
            entry.luma = pose.luma;
            entry.fighter = pose.fighter;
        }
    }
    Ok(sums)
}

/// The mean colour of a sum (alpha-weighted), per channel.
fn mean_colour(sum: [u64; 3], weight: u64) -> [f64; 3] {
    sum.map(|value| value as f64 / weight.max(1) as f64)
}

/// Euclidean distance of two colours.
fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// Plays the replays with the art, measuring for each art file the mean colour it draws inside
/// the pose's mask against the mean colour of the original's pixels there, and lists the files
/// whose colours differ most, by kind (fighters and effects, drawn from colour art or luma art).
///
/// # Errors
///
/// The pack has no art, or a replay cannot be read or played.
pub fn run_colours(
    rom: &[u8],
    pack_dir: &Path,
    files: &[PathBuf],
    scale: u32,
    limit: Option<u64>,
    jobs: usize,
    out: &mut dyn Write,
) -> anyhow::Result<u8> {
    let pack = open_pack(pack_dir)?;
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<anyhow::Result<BTreeMap<u64, ColourSum>>>> = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..jobs.min(files.len()) {
            scope.spawn(|| {
                loop {
                    let at = next.fetch_add(1, Ordering::Relaxed);
                    let Some(path) = files.get(at) else { break };
                    let result = colours_of_replay(rom, &pack, path, scale, limit);
                    results.lock().expect("results").push(result);
                }
            });
        }
    });
    let mut total: BTreeMap<u64, ColourSum> = BTreeMap::new();
    for result in results.into_inner().expect("results") {
        for (key, sum) in result? {
            let entry = total.entry(key).or_default();
            entry.frames += sum.frames;
            entry.weight += sum.weight;
            for channel in 0..3 {
                entry.shown[channel] += sum.shown[channel];
                entry.original[channel] += sum.original[channel];
            }
            entry.luma = sum.luma;
            entry.fighter = sum.fighter;
        }
    }
    let name = |key: u64| -> String {
        pack.art
            .objects_with(key)
            .first()
            .map_or_else(|| format!("{key:016x}"), |&at| pack.art.objects()[at as usize].id.clone())
    };
    for (kind, fighter, luma) in [
        ("fighters, colour art", true, false),
        ("fighters, luma art", true, true),
        ("effects, colour art", false, false),
        ("effects, luma art", false, true),
    ] {
        let mut rows: Vec<(f64, u64, ColourSum)> = total
            .iter()
            .filter(|(_, sum)| sum.fighter == fighter && sum.luma == luma && sum.weight > 0)
            .map(|(&key, &sum)| {
                let shown = mean_colour(sum.shown, sum.weight);
                let original = mean_colour(sum.original, sum.weight);
                (distance(shown, original), key, sum)
            })
            .collect();
        if rows.is_empty() {
            continue;
        }
        let weight: u64 = rows.iter().map(|(_, _, sum)| sum.weight).sum();
        let mean =
            rows.iter().map(|(d, _, sum)| d * sum.weight as f64).sum::<f64>() / weight as f64;
        let over = |limit: f64| rows.iter().filter(|(d, _, _)| *d > limit).count();
        writeln!(
            out,
            "{kind}: {} art files, mean colour distance {mean:.1} (weighted by pixels); over 20: {}, over 40: {}",
            rows.len(),
            over(20.0),
            over(40.0)
        )?;
        rows.sort_by(|a, b| b.0.total_cmp(&a.0));
        for (d, key, sum) in rows.iter().take(8) {
            let shown = mean_colour(sum.shown, sum.weight);
            let original = mean_colour(sum.original, sum.weight);
            writeln!(
                out,
                "  {:>5.1}  {}  {} frames, {} px: drawn {:.0},{:.0},{:.0} against original {:.0},{:.0},{:.0}",
                d,
                name(*key),
                sum.frames,
                sum.weight / 255,
                shown[0],
                shown[1],
                shown[2],
                original[0],
                original[1],
                original[2]
            )?;
        }
    }
    Ok(0)
}

fn png(hd: &ssd_hd::HdFrame) -> Vec<u8> {
    crate::shots::encode_png(hd.width, hd.height, &hd.rgba)
}

/// The two pictures side by side (the plain one on the left).
fn side_by_side(plain: &WideHd, art: &WideHd) -> Vec<u8> {
    let (a, b) = (&plain.picture, &art.picture);
    let width = a.width + b.width;
    let mut rgba = Vec::with_capacity(width as usize * a.height as usize * 4);
    for y in 0..a.height as usize {
        rgba.extend_from_slice(&a.rgba[y * a.width as usize * 4..(y + 1) * a.width as usize * 4]);
        rgba.extend_from_slice(&b.rgba[y * b.width as usize * 4..(y + 1) * b.width as usize * 4]);
    }
    crate::shots::encode_png(width, a.height, &rgba)
}

/// Writes the frames of `shots`.
///
/// # Errors
///
/// A replay cannot be read or played, or a file cannot be written.
pub fn run_shots(
    rom: &[u8],
    pack_dir: &Path,
    out_dir: &Path,
    scale: u32,
    shots: &[(String, PathBuf, u64)],
    font: bool,
    out: &mut dyn Write,
) -> anyhow::Result<u8> {
    let pack = open_pack(pack_dir)?;
    // The plain picture has the font but not the pack's card art
    let plain_font = if font { Some(Arc::new(ssd_hd::Font::standard(rom)?)) } else { None };
    let font = if font {
        Some(Arc::new(ssd_hd::Font::standard_with_pack(rom, Some(&pack.dir))?))
    } else {
        None
    };
    std::fs::create_dir_all(out_dir)?;
    let options = HdOptions { scale, look: Look::Crisp };
    let mut missing = 0;
    // One play of each replay serves all its frames
    let mut replays: Vec<&PathBuf> = shots.iter().map(|(_, path, _)| path).collect();
    replays.sort();
    replays.dedup();
    for path in replays {
        let wanted: Vec<&(String, PathBuf, u64)> =
            shots.iter().filter(|(_, other, _)| other == path).collect();
        let last = wanted.iter().map(|(_, _, frame)| *frame).max().unwrap_or(0);
        let replay = read_replay(path, rom)?;
        let mut machine = replay.power_on(rom.to_vec())?;
        machine.set_line_log(true);
        let mut with = WideRun::new(rom);
        with.set_art(Some(Arc::clone(&pack)));
        with.set_font(font.clone());
        let mut without = WideRun::new(rom);
        without.set_font(plain_font.clone());
        let mut written = vec![false; wanted.len()];
        while machine.frame_count() <= last && machine.frame_count() < replay.frames() {
            machine.run_frame_with(&mut |f| replay.input(f), &mut NullAudio)?;
            let number = machine.frame_count() - 1;
            let Some(at) = wanted.iter().position(|(_, _, frame)| *frame == number) else {
                // Every frame goes through the runs (their sprite line buffers carry), cheaply
                with.note(machine.wram(), machine.frame_log());
                without.note(machine.wram(), machine.frame_log());
                continue;
            };
            let label = &wanted[at].0;
            let a = with.note_hd(machine.wram(), machine.frame_log(), options);
            let b = without.note_hd(machine.wram(), machine.frame_log(), options);
            if let (Some(a), Some(b)) = (a, b) {
                std::fs::write(out_dir.join(format!("{label}-art.png")), png(&a.picture))?;
                std::fs::write(out_dir.join(format!("{label}-plain.png")), png(&b.picture))?;
                std::fs::write(out_dir.join(format!("{label}-both.png")), side_by_side(&b, &a))?;
                let stats = with.art_stats();
                writeln!(
                    out,
                    "{label}: frame {number}, {:?}, {}x{}; fighters {}/{} px art, effects {}/{}, portraits {} px",
                    a.scene,
                    a.picture.width,
                    a.picture.height,
                    stats.fighters.art,
                    stats.fighters.total,
                    stats.effects.art,
                    stats.effects.total,
                    stats.picture_pixels
                )?;
                written[at] = true;
            }
        }
        for (done, (label, _, frame)) in written.iter().zip(&wanted) {
            if !done {
                missing += 1;
                writeln!(out, "{label}: frame {frame} of {} was not drawn", path.display())?;
            }
        }
    }
    Ok(u8::from(missing > 0))
}

/// Lists every `every`th frame of a replay with what the art does in it, to choose frames to show.
///
/// # Errors
///
/// The pack has no art, or the replay cannot be read or played.
pub fn run_scan(
    rom: &[u8],
    pack_dir: &Path,
    replay: &Path,
    every: u64,
    out: &mut dyn Write,
) -> anyhow::Result<u8> {
    let pack = open_pack(pack_dir)?;
    let replay_file = read_replay(replay, rom)?;
    let mut machine = replay_file.power_on(rom.to_vec())?;
    machine.set_line_log(true);
    let mut run = WideRun::new(rom);
    run.set_art(Some(pack));
    let options = HdOptions { scale: 1, look: Look::Crisp };
    while machine.frame_count() < replay_file.frames() {
        machine.run_frame_with(&mut |f| replay_file.input(f), &mut NullAudio)?;
        let number = machine.frame_count() - 1;
        if number % every != 0 {
            run.note(machine.wram(), machine.frame_log());
            continue;
        }
        if run.note_hd(machine.wram(), machine.frame_log(), options).is_some() {
            let stats = run.art_stats();
            let units: Vec<String> = stats
                .poses
                .iter()
                .map(|p| {
                    format!(
                        "{}{:x}{}{}",
                        if !p.matched {
                            "-"
                        } else if p.recoloured {
                            "~"
                        } else {
                            "+"
                        },
                        p.key & 0xFFFF,
                        if p.field {
                            "g"
                        } else if p.fighter {
                            "f"
                        } else {
                            "e"
                        },
                        p.palette
                    )
                })
                .collect();
            writeln!(
                out,
                "{number:6} mode {:02X} battle {} fighters {}/{} fx {}/{} portraits {} (no art {}) recoloured {}+{} poses {}",
                machine.wram()[GAME_MODE],
                u8::from(is_battle(machine.wram())),
                stats.fighters.art,
                stats.fighters.total,
                stats.effects.art,
                stats.effects.total,
                stats.picture_pixels,
                stats.portrait_pixels_without_art,
                stats.fighters.recoloured + stats.effects.recoloured,
                stats.picture_recoloured,
                units.join(",")
            )?;
        }
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_takes_a_scale_a_limit_jobs_a_pack_and_replays() {
        let args = parse_args(&[
            "report",
            "--scale",
            "4",
            "--frames",
            "100",
            "--jobs",
            "2",
            "--pack",
            "p",
            "a.ssdreplay",
        ])
        .unwrap();
        assert_eq!(
            args,
            Args::Report {
                paths: vec![PathBuf::from("a.ssdreplay")],
                scale: 4,
                frames: Some(100),
                jobs: 2,
                pack: Some(PathBuf::from("p")),
                art: true,
            }
        );
        assert_eq!(
            parse_args(&["report"]).unwrap(),
            Args::Report {
                paths: Vec::new(),
                scale: 1,
                frames: None,
                jobs: 4,
                pack: None,
                art: true
            }
        );
    }

    #[test]
    fn shots_are_named_by_label_replay_and_frame_and_default_to_4x() {
        let args = parse_args(&["shots", "beam=C:/r/battle.ssdreplay:5600"]).unwrap();
        assert_eq!(
            args,
            Args::Shots {
                out: None,
                scale: 4,
                shots: vec![("beam".to_owned(), PathBuf::from("C:/r/battle.ssdreplay"), 5600)],
                pack: None,
                font: true,
            }
        );
        let args = parse_args(&["shots", "--no-font", "a=b:1"]).unwrap();
        assert!(matches!(args, Args::Shots { font: false, .. }));
        assert!(parse_args(&["shots", "nope"]).is_err());
    }

    #[test]
    fn scan_takes_one_replay_and_a_step() {
        let args = parse_args(&["scan", "--every", "50", "x.ssdreplay"]).unwrap();
        assert_eq!(
            args,
            Args::Scan { replay: PathBuf::from("x.ssdreplay"), every: 50, pack: None }
        );
        assert!(parse_args(&["scan"]).is_err());
        assert!(parse_args(&["scan", "--every", "0", "x"]).is_err());
    }

    #[test]
    fn items_takes_an_output_folder_and_a_pack() {
        let args = parse_args(&["items", "--out", "o", "--pack", "p"]).unwrap();
        assert_eq!(args, Args::Items { out: PathBuf::from("o"), pack: Some(PathBuf::from("p")) });
        assert!(matches!(parse_args(&["items"]).unwrap(), Args::Items { pack: None, .. }));
    }

    #[test]
    fn a_bad_command_line_says_why() {
        assert!(parse_args(&[]).unwrap_err().contains("subcommand"));
        assert!(parse_args(&["report", "--scale", "9"]).unwrap_err().contains("1 to 8"));
        assert!(parse_args(&["report", "--nope"]).unwrap_err().contains("--nope"));
        assert!(parse_args(&["other"]).unwrap_err().contains("other"));
    }

    #[test]
    fn battle_scenes_are_the_command_screen_the_animation_and_the_last_fights() {
        let mut wram = vec![0_u8; 0x400];
        for (mode, battle, animation) in
            [(0x02, true, false), (0x04, true, true), (0x8C, true, true), (0x06, false, false)]
        {
            wram[GAME_MODE] = mode;
            assert_eq!((is_battle(&wram), is_animation(&wram)), (battle, animation), "{mode:02X}");
        }
    }
}

/// What `hd-art diff` found in one replay.
#[derive(Debug, Default, Clone)]
struct DiffOutcome {
    name: String,
    frames: u64,
    /// Frames where the control (no source, no palette) differs from the first pack: the frames in
    /// which the recolouring of the layers' art matters.
    recolour_frames: u64,
    /// Frames where the second pack differs from the first.
    differ_frames: u64,
    differ_pixels: u64,
    /// The first and last picture row where the second pack differs, and the picture's height.
    rows: Option<(usize, usize, usize)>,
    /// The first differing frames: `(frame, pixels)`.
    first: Vec<(u64, u64)>,
}

/// `hd-art diff <replay|dir...> --pack A --pack-b B --pack-c C [--every N]`: plays the replays and
/// draws every `every`th frame (default 4) in the 16:9 view with the art of three packs: A the
/// reference, B the pack to compare with it, C a control that recolours nothing. It reports, per
/// replay, the frames where C differs from A (the frames where the recolouring of backgrounds and
/// maps matters) and the frames where B differs from A (which must be none).
///
/// # Errors
///
/// A pack has no art, or a replay cannot be played.
pub fn run_diff(
    rom: &[u8],
    packs: [&Path; 3],
    files: &[PathBuf],
    every: u64,
    out: &mut dyn Write,
) -> anyhow::Result<u8> {
    let packs: Vec<Arc<Pack>> = packs.iter().map(|dir| open_pack(dir)).collect::<Result<_, _>>()?;
    let next = AtomicUsize::new(0);
    let results = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..3.min(files.len().max(1)) {
            scope.spawn(|| {
                while let Some(path) = files.get(next.fetch_add(1, Ordering::Relaxed)) {
                    let outcome = diff_replay(rom, &packs, path, every);
                    eprintln!("diff: {}", path.display());
                    results.lock().expect("results").push(outcome);
                }
            });
        }
    });
    let mut all: Vec<DiffOutcome> = Vec::new();
    for outcome in results.into_inner().expect("results") {
        all.push(outcome?);
    }
    all.sort_by(|a, b| a.name.cmp(&b.name));
    writeln!(out, "  replay                     drawn  recolour-matters  B-differs  pixels")?;
    let mut total = DiffOutcome::default();
    for r in &all {
        writeln!(
            out,
            "  {:<26} {:>6} {:>17} {:>10} {:>7}  {:?}",
            r.name, r.frames, r.recolour_frames, r.differ_frames, r.differ_pixels, r.first
        )?;
        if let Some((first, last, height)) = r.rows {
            writeln!(out, "      B differs in the rows {first} to {last} of {height}")?;
        }
        total.frames += r.frames;
        total.recolour_frames += r.recolour_frames;
        total.differ_frames += r.differ_frames;
        total.differ_pixels += r.differ_pixels;
    }
    writeln!(
        out,
        "hd-art diff: {} frames drawn, the recolouring matters in {}, B differs from A in {} ({} pixels)",
        total.frames, total.recolour_frames, total.differ_frames, total.differ_pixels
    )?;
    Ok(u8::from(total.differ_frames > 0))
}

fn diff_replay(
    rom: &[u8],
    packs: &[Arc<Pack>],
    path: &Path,
    every: u64,
) -> anyhow::Result<DiffOutcome> {
    let replay = read_replay(path, rom)?;
    let mut machine = replay.power_on(rom.to_vec())?;
    machine.set_line_log(true);
    let mut runs: Vec<WideRun> = packs
        .iter()
        .map(|pack| {
            let mut run = WideRun::new(rom);
            run.set_art(Some(Arc::clone(pack)));
            run
        })
        .collect();
    let options = HdOptions { scale: 1, look: Look::Crisp };
    let mut outcome = DiffOutcome {
        name: path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned()),
        ..DiffOutcome::default()
    };
    while machine.frame_count() < replay.frames() {
        machine.run_frame_with(&mut |f| replay.input(f), &mut NullAudio)?;
        let number = machine.frame_count() - 1;
        if number % every != 0 {
            for run in &mut runs {
                run.note(machine.wram(), machine.frame_log());
            }
            continue;
        }
        let frames: Vec<_> = runs
            .iter_mut()
            .map(|run| run.note_hd(machine.wram(), machine.frame_log(), options))
            .collect();
        let [Some(reference), Some(other), Some(control)] = frames.as_slice() else { continue };
        outcome.frames += 1;
        let count = |x: &WideHd, y: &WideHd| {
            x.picture
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .zip(y.picture.rgba.as_chunks::<4>().0)
                .filter(|(p, q)| p != q)
                .count() as u64
        };
        if count(reference, control) > 0 {
            outcome.recolour_frames += 1;
        }
        let pixels = count(reference, other);
        if pixels > 0 {
            let width = reference.picture.width as usize * 4;
            let rows: Vec<usize> = reference
                .picture
                .rgba
                .chunks_exact(width)
                .zip(other.picture.rgba.chunks_exact(width))
                .enumerate()
                .filter(|(_, (p, q))| p != q)
                .map(|(row, _)| row)
                .collect();
            let (first, last) = (rows[0], rows[rows.len() - 1]);
            let height = reference.picture.height as usize;
            outcome.rows = Some(match outcome.rows {
                Some((low, high, tall)) => (low.min(first), high.max(last), tall.max(height)),
                None => (first, last, height),
            });
        }
        if pixels > 0 {
            outcome.differ_frames += 1;
            outcome.differ_pixels += pixels;
            if outcome.first.len() < 6 {
                outcome.first.push((number, pixels));
            }
        }
    }
    Ok(outcome)
}
