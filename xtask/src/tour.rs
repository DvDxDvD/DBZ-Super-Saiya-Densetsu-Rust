//! `hd-tour`: the automatic coverage tour of the HD remaster. It visits what the game can show
//! (every map at a grid of camera positions, every battle group, the story scenes of the
//! recorded replays, the title and the intro, every menu screen), draws each frame as the
//! Enhanced picture with the pack's art index, and reports per class of asset how much of what is
//! visible is drawn from the HD art and how much is the pixel-art filter's, with the list of what
//! has no art and where it was first seen (`.work/tour/`, never tracked: it is derived from the
//! game).
//!
//! The tour is made of parts (a map, a battle, a replay...). Each part's result is a file under
//! `.work/tour/parts/`, so a stopped tour resumes where it was and a part is only played again when
//! the pack, the ROM, the core or its own parameters changed (`--fresh` plays everything again).
//! `--class`, `--maps`, `--groups`, `--replays`, `--menus` and `--title` choose what to play; the
//! report covers the parts chosen. The command exits 1 when a class the config
//! (`tools/hd-tour.toml`) marks done draws less than its threshold from the art.

use crate::save::Game;
use crate::shared::render_table;
use crate::tour_account::{CLASSES, Class, Config, ItemTotals, Totals};
use crate::tour_battle::{Episode, Scenario, default_plan};
use crate::tour_run::{
    Env, FORMAT, PartOutput, kind_shows, load_part, play_episode, play_replay, save_part,
};
use crate::tour_sheet::{self, Crop};
use crate::tour_visit;
use anyhow::Context as _;
use ssd_hd::pack::Pack;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// The maps of the game (`0` to `$2F`).
pub const MAPS: u8 = 48;

/// The command line of `hd-tour`.
#[derive(Debug, Clone, PartialEq)]
pub struct Args {
    /// The classes to report and to run the parts for (empty: all).
    pub classes: Vec<Class>,
    /// `Some(list)`: the maps to visit (an empty list: all of them).
    pub maps: Option<Vec<u8>>,
    /// `Some(list)`: the battle groups to visit (an empty list: the whole default plan).
    pub groups: Option<Vec<u8>>,
    /// `Some(list)`: the replays to play, by a part of their name (an empty list: all of them).
    pub replays: Option<Vec<String>>,
    pub menus: bool,
    pub title: bool,
    /// Only the parts whose name contains one of these (empty: all the chosen ones).
    pub only: Vec<String>,
    /// Where the tour's files go (default `.work/tour`).
    pub out: Option<PathBuf>,
    pub fresh: bool,
    pub jobs: usize,
    /// One frame in this many of a long part is counted.
    pub stride: u64,
    /// The length of the play of a forced battle, in frames.
    pub frames: u64,
    pub seed: u32,
    pub config: Option<PathBuf>,
    pub base: Option<PathBuf>,
    /// Write the pictures of counted frames here, every N-th (for checking the tour itself).
    pub dump: Option<(PathBuf, u64)>,
    /// A folder to write the pixels of what has no art to (see `tour_run::Env::gaps`).
    pub gaps: Option<PathBuf>,
    /// Folders for the art audit and the formation pictures (see `tour_run::Env`).
    pub audit: Option<PathBuf>,
    pub formations: Option<PathBuf>,
    /// The text is the filter's, not drawn from the HD font.
    pub no_font: bool,
    /// Print the parts and whether they are done, and stop.
    pub list: bool,
    /// Report what the parts already done say, playing none.
    pub report_only: bool,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            classes: Vec::new(),
            maps: None,
            groups: None,
            replays: None,
            menus: false,
            title: false,
            only: Vec::new(),
            out: None,
            fresh: false,
            jobs: 2,
            stride: 3,
            frames: 2400,
            seed: 1,
            config: None,
            base: None,
            dump: None,
            gaps: None,
            audit: None,
            formations: None,
            no_font: false,
            list: false,
            report_only: false,
        }
    }
}

fn numbers(list: &str) -> Result<Vec<u8>, String> {
    list.split(',')
        .filter(|item| !item.is_empty())
        .map(|item| {
            item.parse::<u8>().map_err(|_| format!("`{item}` is not a number from 0 to 255"))
        })
        .collect()
}

/// Parses the arguments after `hd-tour`.
///
/// ```text
/// [--class a,b] [--maps [N,..]] [--groups [N,..]] [--replays [name,..]] [--menus] [--title]
/// [--only part,..] [--out DIR] [--fresh] [--jobs N] [--stride N] [--frames N] [--seed N] [--config FILE]
/// [--base SAVE] [--dump DIR[:EVERY]] [--gaps DIR] [--audit DIR] [--formations DIR] [--no-font] [--list] [--report]
/// ```
///
/// # Errors
///
/// An unknown option, class or a bad number.
pub fn parse_args(args: &[&str]) -> Result<Args, String> {
    let mut parsed = Args::default();
    let mut iter = args.iter().peekable();
    let value = |iter: &mut std::iter::Peekable<std::slice::Iter<'_, &str>>, what: &str| {
        iter.next().map(|&v| v.to_owned()).ok_or_else(|| format!("{what} needs a value"))
    };
    while let Some(&arg) = iter.next() {
        // A list that may be left out: `--maps` alone means all of them
        let optional = |iter: &mut std::iter::Peekable<std::slice::Iter<'_, &str>>| {
            iter.next_if(|next| !next.starts_with("--")).map(|&v| v.to_owned())
        };
        match arg {
            "--class" => {
                for name in value(&mut iter, arg)?.split(',').filter(|n| !n.is_empty()) {
                    parsed
                        .classes
                        .push(Class::parse(name).ok_or_else(|| format!("no class `{name}`"))?);
                }
            }
            "--maps" => {
                let list = optional(&mut iter).map(|l| numbers(&l)).transpose()?;
                parsed.maps = Some(list.unwrap_or_default());
                if let Some(bad) = parsed.maps.iter().flatten().find(|&&m| m >= MAPS) {
                    return Err(format!("map {bad}: the maps go from 0 to {}", MAPS - 1));
                }
            }
            "--groups" => {
                let list = optional(&mut iter).map(|l| numbers(&l)).transpose()?;
                parsed.groups = Some(list.unwrap_or_default());
            }
            "--replays" => {
                let list = optional(&mut iter)
                    .map(|l| l.split(',').filter(|n| !n.is_empty()).map(str::to_owned).collect());
                parsed.replays = Some(list.unwrap_or_default());
            }
            "--only" => {
                parsed.only = value(&mut iter, arg)?
                    .split(',')
                    .filter(|n| !n.is_empty())
                    .map(str::to_owned)
                    .collect();
            }
            "--menus" => parsed.menus = true,
            "--title" => parsed.title = true,
            "--fresh" => parsed.fresh = true,
            "--no-font" => parsed.no_font = true,
            "--list" => parsed.list = true,
            "--report" => parsed.report_only = true,
            "--out" => parsed.out = Some(PathBuf::from(value(&mut iter, arg)?)),
            "--config" => parsed.config = Some(PathBuf::from(value(&mut iter, arg)?)),
            "--base" => parsed.base = Some(PathBuf::from(value(&mut iter, arg)?)),
            "--gaps" => parsed.gaps = Some(PathBuf::from(value(&mut iter, arg)?)),
            "--audit" => parsed.audit = Some(PathBuf::from(value(&mut iter, arg)?)),
            "--formations" => parsed.formations = Some(PathBuf::from(value(&mut iter, arg)?)),
            "--dump" => {
                let text = value(&mut iter, arg)?;
                let (dir, every) = match text.rsplit_once(':') {
                    Some((dir, every)) if every.parse::<u64>().is_ok() => {
                        (dir.to_owned(), every.parse::<u64>().unwrap_or(1))
                    }
                    _ => (text, 1),
                };
                parsed.dump = Some((PathBuf::from(dir), every.max(1)));
            }
            "--jobs" | "--stride" | "--frames" | "--seed" => {
                let text = value(&mut iter, arg)?;
                let number: u64 = text
                    .parse()
                    .ok()
                    .filter(|&n| n > 0 || arg == "--seed")
                    .ok_or_else(|| format!("{arg} takes a positive number"))?;
                match arg {
                    "--jobs" => parsed.jobs = usize::try_from(number).unwrap_or(1),
                    "--stride" => parsed.stride = number,
                    "--frames" => parsed.frames = number,
                    _ => parsed.seed = u32::try_from(number).map_err(|_| "--seed is a u32")?,
                }
            }
            option => return Err(format!("unknown option `{option}`")),
        }
    }
    Ok(parsed)
}

/// What kind a part is.
#[derive(Debug, Clone)]
pub enum Kind {
    Map(u8),
    Episode(Box<Scenario>, Episode),
    Replay(PathBuf),
    Menus,
    Title,
    /// A character leads the party on a map and walks in the four directions (a gallery of the
    /// walk cycles, for the scene audit).
    Walk(String),
}

/// One part of the tour.
#[derive(Debug, Clone)]
pub struct Job {
    pub name: String,
    /// `map`, `group`, `replay`, `menus` or `title` (what [`kind_shows`] says classes of).
    pub kind: &'static str,
    /// The parameters that change what the part counts.
    pub params: String,
    pub what: Kind,
}

/// The parts `args` choose. With no part kind named, the kinds that can show a selected class (all
/// kinds when no class is selected).
///
/// `groups` is the group indices of the table, `replays` the replay files found.
#[must_use]
pub fn plan_jobs(args: &Args, groups: &[u8], replays: &[PathBuf]) -> Vec<Job> {
    let named = args.maps.is_some()
        || args.groups.is_some()
        || args.replays.is_some()
        || args.menus
        || args.title;
    let wants = |kind: &str, named_it: bool| {
        if named {
            named_it
        } else {
            args.classes.is_empty() || args.classes.iter().any(|&class| kind_shows(kind, class))
        }
    };
    let mut jobs = Vec::new();
    if wants("map", args.maps.is_some()) {
        let ids: Vec<u8> = match &args.maps {
            Some(list) if !list.is_empty() => list.clone(),
            _ => (0..MAPS).collect(),
        };
        for id in ids {
            jobs.push(Job {
                name: format!("map-{id:02}"),
                kind: "map",
                params: format!("grid={}", tour_visit::GRID_VERSION),
                what: Kind::Map(id),
            });
        }
    }
    if wants("group", args.groups.is_some()) {
        let wanted: Vec<u8> = args.groups.clone().unwrap_or_default();
        for scenario in default_plan(groups, args.frames, args.seed) {
            for episode in &scenario.episodes {
                if !wanted.is_empty() && !wanted.contains(&episode.group) {
                    continue;
                }
                let name = format!("group-{}-{:03}", scenario.name, episode.group);
                let params = format!(
                    "frames={} stride={} seed={} fall={} sets={:?} pokes={:?}",
                    episode.frames,
                    args.stride,
                    episode.seed,
                    episode.fall,
                    scenario.sets,
                    scenario.pokes
                );
                let one = Scenario { episodes: vec![episode.clone()], ..scenario.clone() };
                jobs.push(Job {
                    name,
                    kind: "group",
                    params,
                    what: Kind::Episode(Box::new(one), episode.clone()),
                });
            }
        }
    }
    if wants("replay", args.replays.is_some()) {
        let names = args.replays.clone().unwrap_or_default();
        for path in replays {
            let stem =
                path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned());
            if names.is_empty() || names.iter().any(|n| stem.contains(n.as_str())) {
                jobs.push(Job {
                    name: format!("replay-{stem}"),
                    kind: "replay",
                    params: format!("stride={}", args.stride),
                    what: Kind::Replay(path.clone()),
                });
            }
        }
    }
    if wants("menus", args.menus) {
        jobs.push(Job {
            name: "menus".to_owned(),
            kind: "menus",
            params: format!("script={}", tour_visit::MENUS_VERSION),
            what: Kind::Menus,
        });
    }
    if wants("title", args.title) {
        jobs.push(Job {
            name: "title".to_owned(),
            kind: "title",
            params: format!("script={}", tour_visit::TITLE_VERSION),
            what: Kind::Title,
        });
    }
    if args.no_font {
        for job in &mut jobs {
            job.params.push_str(" font=off");
        }
    }
    if !args.only.is_empty() {
        jobs.retain(|job| args.only.iter().any(|part| job.name.contains(part.as_str())));
    }
    jobs
}

/// A checksum of the pack's index and art index, for the fingerprint of the parts.
fn pack_fingerprint(dir: &Path) -> String {
    let crc = |file: &str| std::fs::read(dir.join(file)).map_or(0, |bytes| ssd_rom::crc32(&bytes));
    format!("{:08X}{:08X}", crc("index.bin"), crc(ssd_hd::art_index::FILE))
}

fn fingerprint(rom_crc: u32, pack: &str, job: &Job) -> String {
    format!(
        "{FORMAT}|render={}|rom={rom_crc:08X}|core={}|pack={pack}|{}",
        ssd_hd::RENDERER_VERSION,
        ssd_oracle::CORE_VERSION,
        job.params
    )
}

/// The first of the places a base save may be: the repository's `.work/demo`, then the one of the
/// main working copy (a worktree's `.work` is its own).
///
/// # Errors
///
/// None of them has one.
pub fn find_base_save(root: &Path) -> anyhow::Result<PathBuf> {
    const NAME: &str = ".work/demo/a-goku-level-10.sav";
    let mut places = vec![root.join(NAME)];
    if let Ok(out) = std::process::Command::new("git")
        .current_dir(root)
        .args(["rev-parse", "--git-common-dir"])
        .output()
        && out.status.success()
    {
        let common = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
        let common = if common.is_absolute() { common } else { root.join(common) };
        if let Some(main) = common.parent() {
            places.push(main.join(NAME));
        }
    }
    places.into_iter().find(|p| p.exists()).context(
        "no base save: pass one with --base (a battery save with slot 1 used; \
         `.work/demo/a-goku-level-10.sav` is the default)",
    )
}

pub fn play_job(env: &Env<'_>, job: &Job) -> anyhow::Result<PartOutput> {
    match &job.what {
        Kind::Map(id) => tour_visit::play_map(env, &job.name, *id),
        Kind::Episode(scenario, episode) => play_episode(env, &job.name, scenario, episode),
        Kind::Replay(path) => play_replay(env, &job.name, path),
        Kind::Menus => tour_visit::play_menus(env, &job.name),
        Kind::Title => tour_visit::play_title(env, &job.name),
        Kind::Walk(who) => tour_visit::play_walk(env, &job.name, who),
    }
}

/// Runs the tour.
///
/// # Errors
///
/// The pack, the config or the base save cannot be read, or the report cannot be written. A
/// part that fails is reported and does not stop the others (exit code 2).
pub fn run(
    rom: &[u8],
    pack_dir: &Path,
    root: &Path,
    args: &Args,
    out: &mut (dyn Write + Send),
) -> anyhow::Result<u8> {
    let config_path =
        args.config.clone().unwrap_or_else(|| root.join("tools").join("hd-tour.toml"));
    let config = match std::fs::read_to_string(&config_path) {
        Ok(text) => Config::parse(&text)?,
        Err(_) if args.config.is_none() => Config::default(),
        Err(error) => return Err(error).with_context(|| config_path.display().to_string()),
    };
    let tour = args.out.clone().unwrap_or_else(|| root.join(".work").join("tour"));
    let parts = tour.join("parts");
    let pack =
        Arc::new(Pack::open(pack_dir).with_context(|| format!("the pack {}", pack_dir.display()))?);
    let catalogue = ssd_catalogue::Catalogue::from_rom(rom).context("reading the catalogue")?;
    let groups: Vec<u8> = catalogue
        .encounters
        .iter()
        .filter(|e| e.units().next().is_some())
        .map(|e| e.index)
        .collect();
    let replay_dir = root.join(".work").join("replays");
    let replays = if replay_dir.is_dir() {
        crate::verify_replays::replay_paths(&replay_dir)?
    } else {
        Vec::new()
    };
    let jobs = plan_jobs(args, &groups, &replays);
    anyhow::ensure!(
        !jobs.is_empty(),
        "hd-tour: nothing to visit (no replay in {}?)",
        replay_dir.display()
    );
    let rom_crc = ssd_rom::crc32(rom);
    let pack_fp = pack_fingerprint(pack_dir);
    let prints: Vec<String> = jobs.iter().map(|job| fingerprint(rom_crc, &pack_fp, job)).collect();
    let is_done =
        |at: usize| !args.fresh && load_part(&parts, &jobs[at].name, &prints[at]).is_some();
    if args.list {
        for (at, job) in jobs.iter().enumerate() {
            let state = if is_done(at) { "done" } else { "todo" };
            writeln!(out, "  {:<40} {state}", job.name)?;
        }
        let todo = (0..jobs.len()).filter(|&at| !is_done(at)).count();
        writeln!(out, "hd-tour: {} parts, {todo} to play", jobs.len())?;
        return Ok(0);
    }

    if args.fresh && !args.report_only {
        for job in &jobs {
            let _ = std::fs::remove_file(parts.join(format!("{}.json", job.name)));
        }
    }
    let todo: Vec<usize> = (0..jobs.len()).filter(|&at| !is_done(at)).collect();
    if !args.report_only && !todo.is_empty() {
        let base_path = match &args.base {
            Some(path) => path.clone(),
            None => find_base_save(root)?,
        };
        let base_save = std::fs::read(&base_path)
            .with_context(|| format!("reading the base save {}", base_path.display()))?;
        let game = Game::from_rom(rom)?;
        let mut env = Env::new(rom, Arc::clone(&pack), &game, base_save, args.stride);
        env.dump.clone_from(&args.dump);
        env.gaps.clone_from(&args.gaps);
        env.audit.clone_from(&args.audit);
        env.formations.clone_from(&args.formations);
        if !args.no_font {
            // With the pack's card art: the cards and the UI tiles it draws count as drawn from art
            env.font = Some(Arc::new(
                ssd_hd::Font::standard_with_pack(rom, Some(&pack.dir))
                    .context("the HD text font")?,
            ));
        }
        writeln!(
            out,
            "hd-tour: {} parts, {} to play ({} jobs)",
            jobs.len(),
            todo.len(),
            args.jobs
        )?;
        let next = AtomicUsize::new(0);
        let finished = AtomicUsize::new(0);
        let log = Mutex::new(&mut *out);
        let started = Instant::now();
        std::thread::scope(|scope| {
            for _ in 0..args.jobs.max(1).min(todo.len()) {
                scope.spawn(|| {
                    loop {
                        let n = next.fetch_add(1, Ordering::Relaxed);
                        let Some(&at) = todo.get(n) else { break };
                        let job = &jobs[at];
                        let begun = Instant::now();
                        // A part that panics fails alone
                        let played = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            play_job(&env, job)
                        }))
                        .unwrap_or_else(|_| Err(anyhow::anyhow!("the part panicked")));
                        let outcome = played.and_then(|output| {
                            let seconds = begun.elapsed().as_secs_f64();
                            save_part(&parts, &job.name, &prints[at], &output, seconds)?;
                            Ok((output, seconds))
                        });
                        let number = finished.fetch_add(1, Ordering::Relaxed) + 1;
                        let line = match &outcome {
                            Ok((output, seconds)) => format!(
                                "[{number}/{}] {} {} frames, {:.0} s (tour {:.0} s)",
                                todo.len(),
                                job.name,
                                output.totals.frames,
                                seconds,
                                started.elapsed().as_secs_f64()
                            ),
                            Err(error) => {
                                format!("[{number}/{}] {} FAILED: {error:#}", todo.len(), job.name)
                            }
                        };
                        if let Ok(mut log) = log.lock() {
                            let _ = writeln!(log, "{line}");
                            let _ = log.flush();
                        }
                    }
                });
            }
        });
    }

    // The report: every part chosen, from its file
    let mut all = Totals::default();
    let mut missing = Vec::new();
    let mut part_rows = Vec::new();
    let mut crops: BTreeMap<String, PathBuf> = BTreeMap::new();
    let mut notes = Vec::new();
    for (at, job) in jobs.iter().enumerate() {
        let Some(file) = load_part(&parts, &job.name, &prints[at]) else {
            if parts.join(format!("{}.json", job.name)).exists() {
                missing.push(format!("{} (made by another version: --fresh)", job.name));
            } else {
                missing.push(job.name.clone());
            }
            continue;
        };
        for (key, relative) in &file.crops {
            let first_here = file.totals.items.get(key).is_some_and(|i| i.first.part == job.name);
            if first_here {
                crops.entry(key.clone()).or_insert_with(|| parts.join(relative));
            }
        }
        for note in &file.notes {
            notes.push(format!("{}: {note}", job.name));
        }
        part_rows.push((
            job.name.clone(),
            job.kind,
            file.totals.frames,
            file.totals.refused,
            file.seconds,
        ));
        all.merge(&file.totals);
    }
    let selected: Vec<Class> =
        if args.classes.is_empty() { CLASSES.to_vec() } else { args.classes.clone() };
    write_report(&tour, &all, &part_rows, &crops, &config, &selected, &notes, &missing, out)?;
    if !missing.is_empty() {
        writeln!(
            out,
            "hd-tour: {} part(s) not played or failed: {}{}",
            missing.len(),
            missing.iter().take(6).cloned().collect::<Vec<_>>().join(", "),
            if missing.len() > 6 { ", ... (--list says which)" } else { "" }
        )?;
    }
    let failures = config.failures(&all, &selected);
    if !failures.is_empty() {
        let names: Vec<&str> = failures.iter().map(|c| c.key()).collect();
        writeln!(
            out,
            "hd-tour: FAILED: {} below {} % ({})",
            names.len(),
            config.threshold,
            names.join(", ")
        )?;
        return Ok(1);
    }
    Ok(if missing.is_empty() { 0 } else { 2 })
}

/// How many things with no art a class lists in `coverage.json`, and on its sheet.
const LISTED: usize = 500;
const SHEET: usize = 100;

#[allow(clippy::too_many_arguments)]
fn write_report(
    tour: &Path,
    all: &Totals,
    parts: &[(String, &'static str, u64, u64, f64)],
    crops: &BTreeMap<String, PathBuf>,
    config: &Config,
    selected: &[Class],
    notes: &[String],
    missing: &[String],
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(tour)?;
    let mut classes = serde_json::Map::new();
    let mut rows = Vec::new();
    for class in CLASSES {
        if !selected.contains(&class) {
            continue;
        }
        let totals = all.class(class);
        let unbound = all.unbound(class);
        // The sheet: the items that took the most pixels and have a picture
        let mut sheet_items = Vec::new();
        let mut sheet_crops: Vec<Crop> = Vec::new();
        for (key, _) in &unbound {
            if sheet_crops.len() >= SHEET {
                break;
            }
            let Some(path) = crops.get(*key) else { continue };
            let Ok(bytes) = std::fs::read(path) else { continue };
            let (width, height, rgba) = ssd_hd::png::decode_rgba(&bytes);
            sheet_items.push((*key).clone());
            sheet_crops.push(Crop { width, height, rgba });
        }
        let sheet_name = format!("{}.png", class.key());
        if sheet_crops.is_empty() {
            let _ = std::fs::remove_file(tour.join(&sheet_name));
        } else {
            tour_sheet::write_sheet(&tour.join(&sheet_name), &sheet_crops, 0)?;
        }
        let listed: Vec<serde_json::Value> = unbound
            .iter()
            .take(LISTED)
            .map(|(key, item)| {
                let cell = sheet_items.iter().position(|k| k == *key);
                item_json(item, cell.map(|c| (sheet_name.as_str(), c)))
            })
            .collect();
        let mut by_asset: BTreeMap<&str, (u64, u64)> = BTreeMap::new();
        for (_, item) in &unbound {
            if let Some(rest) = item.id.strip_prefix("tile:") {
                let asset = rest.rsplit_once('#').map_or(rest, |(asset, _)| asset);
                let entry = by_asset.entry(asset).or_default();
                entry.0 += 1;
                entry.1 += item.pixels;
            }
        }
        let by_asset: serde_json::Map<String, serde_json::Value> = by_asset
            .into_iter()
            .map(|(asset, (tiles, pixels))| {
                (asset.to_owned(), serde_json::json!({ "tiles": tiles, "pixels": pixels }))
            })
            .collect();
        let done = config.is_done(class);
        let passes = !done || totals.pixels == 0 || totals.percent() >= config.threshold;
        classes.insert(
            class.key().to_owned(),
            serde_json::json!({
                "title": class.title(),
                "pixels": totals.pixels,
                "art": totals.art,
                "percent": if totals.pixels == 0 {
                    serde_json::Value::Null
                } else {
                    serde_json::json!((totals.percent() * 1000.0).round() / 1000.0)
                },
                "missing_pose_pixels": totals.missing,
                "partial_pose_pixels": totals.partial,
                "done": done,
                "pass": passes,
                "unbound_total": unbound.len(),
                "unbound": listed,
                "unbound_tiles_by_asset": by_asset,
                "sheet": if sheet_crops.is_empty() { serde_json::Value::Null } else { serde_json::json!(sheet_name) },
            }),
        );
        rows.push(vec![
            class.title().to_owned(),
            totals.pixels.to_string(),
            if totals.pixels == 0 { "-".to_owned() } else { format!("{:.2}", totals.percent()) },
            totals.missing.to_string(),
            totals.partial.to_string(),
            unbound.len().to_string(),
            match (done, passes) {
                (false, _) => String::new(),
                (true, true) => "done".to_owned(),
                (true, false) => "FAIL".to_owned(),
            },
        ]);
    }
    let document = serde_json::json!({
        "format": 1,
        "threshold": config.threshold,
        "frames": all.frames,
        "refused_frames": all.refused,
        "classes": classes,
        "parts": parts.iter().map(|(name, kind, frames, refused, seconds)| serde_json::json!({
            "name": name, "kind": kind, "frames": frames, "refused": refused, "seconds": (seconds * 10.0).round() / 10.0,
        })).collect::<Vec<_>>(),
        "notes": notes,
        "missing_parts": missing,
    });
    std::fs::write(tour.join("coverage.json"), serde_json::to_vec_pretty(&document)?)?;
    let header = ["class", "pixels", "art %", "no-art poses", "partial", "unbound", ""];
    write!(out, "{}", render_table(&header, &rows))?;
    let mut by_kind: BTreeMap<&str, (usize, u64)> = BTreeMap::new();
    for (_, kind, frames, _, _) in parts {
        let entry = by_kind.entry(kind).or_default();
        entry.0 += 1;
        entry.1 += frames;
    }
    let visited: Vec<String> = by_kind
        .iter()
        .map(|(kind, (count, frames))| format!("{count} {kind} ({frames} frames)"))
        .collect();
    writeln!(
        out,
        "hd-tour: {} frames counted, {} refused; visited {}",
        all.frames,
        all.refused,
        visited.join(", ")
    )?;
    let mut shown = String::new();
    for class in CLASSES.iter().filter(|c| selected.contains(c)) {
        for (key, item) in all.unbound(*class).into_iter().take(3) {
            let _ = writeln!(
                shown,
                "  {:<16} {:<44} {:>9} px, first {} f{}{}",
                class.key(),
                item.id,
                item.pixels,
                item.first.part,
                item.first.frame,
                if crops.contains_key(key) { "" } else { " (no picture)" }
            );
        }
    }
    if !shown.is_empty() {
        writeln!(out, "hd-tour: most visible things with no art:")?;
        write!(out, "{shown}")?;
    }
    writeln!(out, "hd-tour: report in {}", tour.join("coverage.json").display())?;
    Ok(())
}

fn item_json(item: &ItemTotals, cell: Option<(&str, usize)>) -> serde_json::Value {
    serde_json::json!({
        "id": item.id,
        "pixels": item.pixels,
        "frames": item.frames,
        "first": { "part": item.first.part, "frame": item.first.frame },
        "bbox": [item.bbox.0, item.bbox.1, item.bbox.2, item.bbox.3],
        "sheet": cell.map(|(file, cell)| serde_json::json!({ "file": file, "cell": cell })),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Args {
        parse_args(args).unwrap()
    }

    #[test]
    fn the_command_line_chooses_the_parts() {
        let args = parse(&[
            "--class",
            "fighters,portraits",
            "--maps",
            "0,5",
            "--groups",
            "--replays",
            "battle",
            "--menus",
            "--stride",
            "2",
            "--jobs",
            "1",
            "--dump",
            "x:10",
        ]);
        assert_eq!(args.classes, vec![Class::Fighters, Class::Portraits]);
        assert_eq!(args.maps, Some(vec![0, 5]));
        assert_eq!(args.groups, Some(Vec::new()));
        assert_eq!(args.replays, Some(vec!["battle".to_owned()]));
        assert!(args.menus && !args.title);
        assert_eq!((args.stride, args.jobs), (2, 1));
        assert_eq!(args.dump, Some((PathBuf::from("x"), 10)));
        assert!(parse(&["--no-font"]).no_font && !parse(&[]).no_font);
        let audited = parse(&["--audit", "a", "--formations", "f"]);
        assert_eq!((audited.audit, audited.formations), (Some("a".into()), Some("f".into())));
        assert_eq!(parse(&["--maps"]).maps, Some(Vec::new()));
        assert_eq!(parse(&["--maps", "--menus"]).maps, Some(Vec::new()));
        assert_eq!(parse(&[]), Args::default());
    }

    #[test]
    fn bad_arguments_are_refused() {
        assert!(parse_args(&["--class", "fonts"]).unwrap_err().contains("no class"));
        assert!(parse_args(&["--maps", "48"]).unwrap_err().contains("0 to 47"));
        assert!(parse_args(&["--maps", "x"]).unwrap_err().contains("not a number"));
        assert!(parse_args(&["--stride", "0"]).is_err());
        assert!(parse_args(&["--fast"]).unwrap_err().contains("unknown option"));
        assert!(parse_args(&["--out"]).unwrap_err().contains("needs a value"));
    }

    fn names(jobs: &[Job]) -> Vec<&str> {
        jobs.iter().map(|job| job.name.as_str()).collect()
    }

    fn replays() -> Vec<PathBuf> {
        ["battle", "menus", "secret-raditz"]
            .iter()
            .map(|n| PathBuf::from(format!("r/{n}.ssdreplay")))
            .collect()
    }

    #[test]
    fn a_tour_with_no_choice_visits_everything() {
        let jobs = plan_jobs(&Args::default(), &[3, 4], &replays());
        let kinds: std::collections::BTreeSet<&str> = jobs.iter().map(|j| j.kind).collect();
        assert_eq!(
            kinds.into_iter().collect::<Vec<_>>(),
            vec!["group", "map", "menus", "replay", "title"]
        );
        assert_eq!(jobs.iter().filter(|j| j.kind == "map").count(), 48);
        assert_eq!(jobs.iter().filter(|j| j.kind == "replay").count(), 3);
    }

    #[test]
    fn a_class_runs_the_parts_that_can_show_it() {
        let args = Args { classes: vec![Class::Map], ..Args::default() };
        let jobs = plan_jobs(&args, &[3], &replays());
        assert!(jobs.iter().any(|j| j.kind == "map") && jobs.iter().all(|j| j.kind != "group"));
        let args = Args { classes: vec![Class::Fighters], ..Args::default() };
        let jobs = plan_jobs(&args, &[3], &replays());
        assert!(jobs.iter().all(|j| j.kind != "map" && j.kind != "menus"));
        assert!(jobs.iter().any(|j| j.kind == "group") && jobs.iter().any(|j| j.kind == "replay"));
    }

    #[test]
    fn naming_a_part_kind_runs_only_the_named() {
        let args = Args {
            maps: Some(vec![5, 6]),
            replays: Some(vec!["raditz".into()]),
            ..Args::default()
        };
        let jobs = plan_jobs(&args, &[3], &replays());
        assert_eq!(names(&jobs), vec!["map-05", "map-06", "replay-secret-raditz"]);
    }

    #[test]
    fn groups_pick_their_battles_from_the_plan() {
        let args = Args { groups: Some(vec![20]), ..Args::default() };
        let jobs = plan_jobs(&args, &[3, 4, 20], &replays());
        assert!(!jobs.is_empty());
        assert!(
            jobs.iter().all(|j| j.name.ends_with("-020") && j.kind == "group"),
            "{:?}",
            names(&jobs)
        );
        // Every party fighter fights group 20 (a sparring group) and so does the mixed party
        assert!(jobs.iter().any(|j| j.name == "group-party-goku-020"));
        assert!(jobs.iter().any(|j| j.name.starts_with("group-groups-")));
        let all = plan_jobs(
            &Args { groups: Some(Vec::new()), ..Args::default() },
            &[3, 4, 20],
            &replays(),
        );
        assert!(all.len() > jobs.len());
    }

    #[test]
    fn a_job_is_played_again_when_what_it_counts_changes() {
        let job =
            Job { name: "map-00".into(), kind: "map", params: "grid=1".into(), what: Kind::Map(0) };
        let one = fingerprint(1, "AA", &job);
        assert_ne!(one, fingerprint(2, "AA", &job));
        assert_ne!(one, fingerprint(1, "BB", &job));
        let other = Job { params: "grid=2".into(), ..job.clone() };
        assert_ne!(one, fingerprint(1, "AA", &other));
        assert_eq!(one, fingerprint(1, "AA", &job));
    }

    fn item(class: Class, id: &str, pixels: u64) -> (String, ItemTotals) {
        (
            format!("{}|{id}", class.key()),
            ItemTotals {
                class,
                id: id.to_owned(),
                pixels,
                frames: 2,
                first: crate::tour_account::Where { part: "map-00".to_owned(), frame: 9 },
                bbox: (1, 2, 5, 6),
            },
        )
    }

    #[test]
    fn the_report_has_the_classes_the_gate_and_the_things_with_no_art() {
        use crate::tour_account::ClassTotals;
        let dir = tempfile::tempdir().unwrap();
        let mut totals = Totals { frames: 10, refused: 1, ..Totals::default() };
        totals.classes.insert(
            Class::Portraits,
            ClassTotals { pixels: 1000, art: 990, ..ClassTotals::default() },
        );
        totals.classes.insert(Class::Map, ClassTotals { pixels: 500, ..ClassTotals::default() });
        totals.items.extend([
            item(Class::Portraits, "tile:portrait/16:8A9B/4bpp#26", 10),
            item(Class::Map, "tile:field/08:8D4C/4bpp#154", 400),
            item(Class::Map, "tile:field/08:8D4C/4bpp#157", 100),
        ]);
        // A picture for the biggest thing on the map
        let crop = dir.path().join("c.png");
        ssd_hd::png::write_rgba(&crop, 2, 2, &[7; 16]).unwrap();
        let crops = BTreeMap::from([("map|tile:field/08:8D4C/4bpp#154".to_owned(), crop)]);
        let config = Config { threshold: 99.5, done: vec!["portraits".to_owned()] };
        let selected = CLASSES.to_vec();
        let parts = vec![("map-00".to_owned(), "map", 10_u64, 1_u64, 2.5_f64)];
        let mut out = Vec::new();
        write_report(
            dir.path(),
            &totals,
            &parts,
            &crops,
            &config,
            &selected,
            &["note".to_owned()],
            &[],
            &mut out,
        )
        .unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("portraits/faces") && text.contains("99.00"), "{text}");
        assert!(text.contains("tile:field/08:8D4C/4bpp#154"), "{text}");
        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("coverage.json")).unwrap())
                .unwrap();
        let portraits = &json["classes"]["portraits"];
        assert_eq!(portraits["percent"], 99.0);
        assert_eq!(
            (portraits["done"].as_bool(), portraits["pass"].as_bool()),
            (Some(true), Some(false))
        );
        let map = &json["classes"]["map"];
        assert_eq!((map["pixels"].as_u64(), map["unbound_total"].as_u64()), (Some(500), Some(2)));
        assert_eq!(map["unbound"][0]["id"], "tile:field/08:8D4C/4bpp#154");
        assert_eq!(map["unbound"][0]["first"]["part"], "map-00");
        assert_eq!(map["unbound"][0]["sheet"]["file"], "map.png");
        assert!(map["unbound"][1]["sheet"].is_null(), "no picture for the second");
        assert_eq!(map["unbound_tiles_by_asset"]["field/08:8D4C/4bpp"]["tiles"], 2);
        assert!(dir.path().join("map.png").exists());
        assert!(!dir.path().join("fighters.png").exists());
        assert_eq!(json["classes"]["fighters"]["percent"], serde_json::Value::Null);
        assert_eq!(json["parts"][0]["name"], "map-00");
        assert_eq!(json["notes"][0], "note");
    }
}
