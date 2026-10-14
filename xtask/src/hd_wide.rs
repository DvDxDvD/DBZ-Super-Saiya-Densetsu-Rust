//! `hd-wide`: the widescreen field view (`ssd-hd::world`) checked against the game, headless.
//!
//! `check` plays the replays with the PPU's render log on and, for every frame the main loop
//! shows the field in (`game_mode` `$00` and `$06`), checks
//!
//! * the ported map lookup against the game's own BG2 tile map words in VRAM on every visible
//!   tile, and the ported object builder against the OAM;
//! * the wide frame at each view width against the Classic frame: at 256 the frames must be
//!   equal, at wider views the original window inside the wide frame must be equal except
//!   under the parts of the objects the original does not draw;
//! * that no tile from outside the map is ever drawn.
//!
//! `shots` writes the wide frames of chosen replay frames as PNGs (never tracked).

use crate::shared::{read_replay, render_table};
use anyhow::Context;
use ssd_hd::world::{
    FieldState, HudPlacement, Rom, Scene, WideError, WideFrame, WideOptions, WideRenderer,
    check_objects, check_tiles,
};
use ssd_hd::{Frame, Renderer};
use ssd_oracle::{Color, FrameLog, NullAudio};
use std::fmt::Write as _;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// The view widths checked by default: the Classic frame, 16:9 (239 lines at pixel aspect 8:7)
/// and 21:9.
const WIDTHS: [usize; 3] = [256, 372, 488];

/// The command line of `hd-wide`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Args {
    Check {
        /// Replay files, or directories of them; empty: `.work/replays`.
        paths: Vec<PathBuf>,
        frames: Option<u64>,
        jobs: usize,
        widths: Vec<usize>,
    },
    Shots {
        widths: Vec<usize>,
        /// `(label, replay, frame)`; empty: the default set.
        shots: Vec<(String, PathBuf, u64)>,
    },
}

fn parse_widths(text: &str) -> Result<Vec<usize>, String> {
    let widths: Result<Vec<usize>, _> = text.split(',').map(str::parse::<usize>).collect();
    match widths {
        Ok(widths)
            if !widths.is_empty()
                && widths.iter().all(|w| (256..=ssd_hd::world::MAX_WIDTH).contains(w)) =>
        {
            Ok(widths)
        }
        _ => Err(format!("--widths takes numbers from 256 to {}", ssd_hd::world::MAX_WIDTH)),
    }
}

/// Parses the arguments after `hd-wide`: `check [--frames N] [--jobs N] [--widths A,B] [path...]`
/// or `shots [--widths A,B] [label=replay:frame...]`.
///
/// # Errors
///
/// An unknown subcommand or option, or a bad value.
pub fn parse_args(args: &[&str]) -> Result<Args, String> {
    let mut iter = args.iter();
    let command = iter.next().copied().ok_or("a subcommand: check or shots")?;
    let mut widths = WIDTHS.to_vec();
    let (mut frames, mut jobs) = (None, 4);
    let mut paths = Vec::new();
    let mut shots = Vec::new();
    while let Some(&arg) = iter.next() {
        match arg {
            "--widths" => {
                widths = parse_widths(iter.next().ok_or("--widths needs a list")?)?;
            }
            "--frames" | "--jobs" if command == "check" => {
                let number = iter
                    .next()
                    .and_then(|value| value.parse::<u64>().ok())
                    .filter(|&n| n > 0)
                    .ok_or_else(|| format!("{arg} needs a number"))?;
                if arg == "--frames" {
                    frames = Some(number);
                } else {
                    jobs = number as usize;
                }
            }
            option if option.starts_with('-') => return Err(format!("unknown option `{option}`")),
            value if command == "shots" => {
                let (label, target) =
                    value.split_once('=').ok_or("a shot is label=replay:frame")?;
                let (replay, frame) =
                    target.rsplit_once(':').ok_or("a shot is label=replay:frame")?;
                let frame = frame.parse::<u64>().map_err(|_| "the frame is a number")?;
                shots.push((label.to_owned(), PathBuf::from(replay), frame));
            }
            value => paths.push(PathBuf::from(value)),
        }
    }
    match command {
        "check" => Ok(Args::Check { paths, frames, jobs, widths }),
        "shots" => Ok(Args::Shots { widths, shots }),
        other => Err(format!("unknown subcommand `{other}`")),
    }
}

/// What the replay player hands to the visitor for each frame.
struct Visit<'a> {
    number: u64,
    /// The main loop's state (`$24`) as the frame's NMI saw it (the previous frame's end).
    game_mode: u8,
    /// The field variables as the frame's NMI saw them (the previous frame's end).
    state: Option<&'a FieldState>,
    log: &'a FrameLog,
    /// The Classic frame, when the plain renderer takes the frame.
    classic: Option<&'a Frame>,
}

/// The main loop's state in work RAM (`$7E:0024`).
const GAME_MODE: usize = 0x24;

/// Plays `path` with the line log on, re-renders every frame (the sprite line buffers carry
/// from frame to frame) and calls `visit`; it returns whether to go on.
fn play(
    rom: &[u8],
    path: &Path,
    limit: Option<u64>,
    mut visit: impl FnMut(&Visit<'_>) -> anyhow::Result<bool>,
) -> anyhow::Result<u64> {
    let replay = read_replay(path, rom)?;
    let frames = replay.frames().min(limit.unwrap_or(u64::MAX));
    let mut machine = replay.power_on(rom.to_vec())?;
    machine.set_line_log(true);
    let mut renderer = Renderer::new();
    let mut previous: Option<FieldState> = None;
    let mut previous_mode = machine.wram()[GAME_MODE];
    while machine.frame_count() < frames {
        machine.run_frame_with(&mut |f| replay.input(f), &mut NullAudio)?;
        let number = machine.frame_count() - 1;
        let current = FieldState::from_wram(machine.wram());
        let current_mode = machine.wram()[GAME_MODE];
        if let Some(log) = machine.frame_log() {
            let classic = renderer.render(log).ok();
            let seen = Visit {
                number,
                game_mode: previous_mode,
                state: previous.as_ref(),
                log,
                classic: classic.as_ref(),
            };
            if !visit(&seen)? {
                break;
            }
        }
        previous = current;
        previous_mode = current_mode;
    }
    Ok(frames)
}

/// The first pixel of the original window that differs from the Classic frame.
#[derive(Debug, Clone, Copy)]
struct Difference {
    frame: u64,
    x: usize,
    y: usize,
    classic: [u8; 3],
    wide: [u8; 3],
}

fn class_index(scene: Scene) -> usize {
    match scene {
        Scene::Field => 0,
        Scene::Flight => 1,
        Scene::Bordered => 2,
    }
}

/// The results for one view width.
#[derive(Debug, Clone, Default)]
struct WidthOutcome {
    width: usize,
    rendered: u64,
    /// Frames the wide renderer refused, and the first reason.
    rejected: u64,
    /// Frames rendered per scene class (field, flight, bordered).
    classes: [u64; 3],
    rejected_first: Option<(u64, WideError)>,
    /// Why frames were refused: the reason and how many.
    reasons: Vec<(String, u64)>,
    differing_frames: u64,
    /// Pixels of the original window that differ from the Classic frame, away from the extra
    /// objects and under them.
    plain_pixels: u64,
    extra_pixels: u64,
    first: Option<Difference>,
    /// Frames whose window is exactly in the middle of the view.
    centred: u64,
    outside_black: u64,
    outside_drawn: u64,
    /// Pictures (`bordered`) with a pixel beside the window that is not black.
    outside_not_black: u64,
    /// Pixels beside the window that differ from the widest view's at the same window positions
    /// (where neither shows an extra object).
    outside_mismatch: u64,
    extra_parts: u64,
    extra_dropped: u64,
}

impl WidthOutcome {
    fn note_reason(&mut self, reason: WideError) {
        let text = reason.to_string();
        match self.reasons.iter_mut().find(|(known, _)| *known == text) {
            Some(entry) => entry.1 += 1,
            None => self.reasons.push((text, 1)),
        }
    }

    fn green(&self) -> bool {
        self.rejected == 0
            && self.plain_pixels == 0
            && self.outside_drawn == 0
            && self.outside_not_black == 0
            && self.outside_mismatch == 0
            && (self.width > 256 || self.extra_pixels == 0)
    }

    fn compare(&mut self, number: u64, wide: &WideFrame, classic: &Frame) {
        self.rendered += 1;
        let offset = wide.logical.x as usize;
        let width = wide.frame.width;
        let (mut plain, mut extra) = (0, 0);
        for y in 0..classic.height {
            for x in 0..classic.width {
                let (wide_pixel, classic_pixel) = (
                    wide.frame.pixels[y * width + offset + x],
                    classic.pixels[y * classic.width + x],
                );
                if wide_pixel == classic_pixel {
                    continue;
                }
                if wide.under_extra((offset + x) as i32, y as i32) {
                    extra += 1;
                } else {
                    plain += 1;
                    self.first.get_or_insert(Difference {
                        frame: number,
                        x,
                        y,
                        classic: [classic_pixel.r, classic_pixel.g, classic_pixel.b],
                        wide: [wide_pixel.r, wide_pixel.g, wide_pixel.b],
                    });
                }
            }
        }
        self.plain_pixels += plain;
        self.extra_pixels += extra;
        self.differing_frames += u64::from(plain + extra > 0);
        self.centred += u64::from(offset == (width - 256) / 2);
        let stats = &wide.stats;
        self.outside_black += stats.outside_skipped;
        self.outside_drawn += stats.outside_drawn;
        self.extra_parts += u64::from(stats.extra_parts);
        self.extra_dropped += u64::from(stats.extra_dropped);
    }
}

/// What checking one replay found.
#[derive(Debug, Clone, Default)]
struct Outcome {
    name: String,
    frames: u64,
    field_frames: u64,
    tiles: u64,
    tile_differences: u64,
    tiles_outside: u64,
    unbuilt_tiles: u64,
    /// Field frames whose BG2 holds a picture of the game's own, not the map.
    pictures: u64,
    overscan_tiles: u64,
    overscan_differing: u64,
    first_tile: Option<(u64, String)>,
    parts: u64,
    parts_missing: u64,
    frames_without_loop: u64,
    /// Frames per scene class (field, flight, bordered).
    classes: [u64; 3],
    widths: Vec<WidthOutcome>,
    /// Maps met: id, size in blocks, frames, first frame.
    maps: Vec<(u8, (u8, u8), u64, u64)>,
}

impl Outcome {
    fn green(&self) -> bool {
        self.tile_differences == 0
            && self.parts_missing == 0
            && self.widths.iter().all(WidthOutcome::green)
    }
}

/// The pixels beside the window of a picture that are not black: a picture has nothing there.
fn outside_not_black(wide: &WideFrame) -> u64 {
    let (left, right) = (wide.logical.x, wide.logical.x + 256);
    let mut count = 0;
    for row in wide.frame.pixels.chunks(wide.frame.width) {
        for (x, &color) in row.iter().enumerate() {
            let x = x as i32;
            if (x < left || x >= right) && color != Color::BLACK {
                count += 1;
            }
        }
    }
    count
}

/// The pixels beside the window that differ between two wide frames at the same window
/// positions, left out where either shows an extra object (the field's, which the widths place
/// differently at the edges of the view).
fn outside_mismatch(a: &WideFrame, b: &WideFrame) -> u64 {
    let (a_left, b_left) = (a.logical.x, b.logical.x);
    let lowest = (-a_left).max(-b_left);
    let highest = (a.frame.width as i32 - a_left).min(b.frame.width as i32 - b_left);
    let mut count = 0;
    for y in 0..a.frame.height.min(b.frame.height) {
        for window_x in lowest..highest {
            if (0..256).contains(&window_x) {
                continue;
            }
            let (ax, bx) = (a_left + window_x, b_left + window_x);
            let wide_a = a.frame.pixels[y * a.frame.width + ax as usize];
            let wide_b = b.frame.pixels[y * b.frame.width + bx as usize];
            let extra = a.under_extra(ax, y as i32) || b.under_extra(bx, y as i32);
            if wide_a != wide_b && !extra {
                count += 1;
            }
        }
    }
    count
}

fn check_replay(
    rom: &[u8],
    path: &Path,
    limit: Option<u64>,
    widths: &[usize],
) -> anyhow::Result<Outcome> {
    let name = path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    let mut outcome = Outcome { name, ..Outcome::default() };
    outcome.widths =
        widths.iter().map(|&width| WidthOutcome { width, ..WidthOutcome::default() }).collect();
    let mut renderers: Vec<WideRenderer> = widths.iter().map(|_| WideRenderer::new()).collect();
    let rom_view = Rom(rom);
    outcome.frames = play(rom, path, limit, |visit| {
        if let Some(state) = visit.state.filter(|state| state.is_field()) {
            outcome.field_frames += 1;
            match outcome.maps.iter_mut().find(|m| m.0 == state.map_id && m.1 == state.map.blocks) {
                Some(entry) => entry.2 += 1,
                None => outcome.maps.push((state.map_id, state.map.blocks, 1, visit.number)),
            }
            let tiles = check_tiles(rom_view, state, visit.log);
            if tiles.is_map() {
                outcome.tiles += tiles.tiles;
                outcome.tile_differences += tiles.differing;
                outcome.tiles_outside += tiles.outside;
                outcome.unbuilt_tiles += tiles.unbuilt;
                outcome.overscan_tiles += tiles.overscan_tiles;
                outcome.overscan_differing += tiles.overscan_differing;
                if let Some(first) = tiles.first {
                    outcome.first_tile.get_or_insert((
                        visit.number,
                        format!(
                            "world ({}, {}): game {:04X} lookup {:04X}",
                            first.world.0, first.world.1, first.game, first.lookup
                        ),
                    ));
                }
            } else {
                outcome.pictures += 1;
            }
            let objects = check_objects(rom_view, state, visit.log);
            outcome.parts += objects.parts;
            outcome.parts_missing += objects.missing;
            outcome.frames_without_loop += objects.frames_without_loop;
        }
        let mut frames: Vec<Option<WideFrame>> = Vec::with_capacity(widths.len());
        for (width_outcome, renderer) in outcome.widths.iter_mut().zip(&mut renderers) {
            let options = WideOptions { width: width_outcome.width, hud: HudPlacement::Window };
            match renderer.render_view(rom_view, visit.game_mode, visit.state, visit.log, options) {
                Err(reason) => {
                    width_outcome.note_reason(reason);
                    width_outcome.rejected += 1;
                    width_outcome.rejected_first.get_or_insert((visit.number, reason));
                    frames.push(None);
                }
                Ok(wide) => {
                    width_outcome.classes[class_index(wide.scene)] += 1;
                    if wide.scene == Scene::Bordered {
                        width_outcome.outside_not_black += outside_not_black(&wide);
                    }
                    match visit.classic {
                        Some(classic) => width_outcome.compare(visit.number, &wide, classic),
                        None => {
                            width_outcome.rejected += 1;
                            width_outcome.rejected_first.get_or_insert((
                                visit.number,
                                WideError::Unsupported(ssd_hd::Unsupported::Incomplete),
                            ));
                        }
                    }
                    frames.push(Some(wide));
                }
            }
        }
        // Every width shows the same outside columns as the widest one (its window is checked
        // against the Classic frame above)
        let widest = frames.iter().flatten().max_by_key(|wide| wide.frame.width).cloned();
        if let Some(widest) = widest {
            for (width_outcome, wide) in outcome.widths.iter_mut().zip(&frames) {
                if let Some(wide) = wide
                    && wide.frame.width < widest.frame.width
                {
                    width_outcome.outside_mismatch += outside_mismatch(wide, &widest);
                }
            }
        }
        if let Some(scene) = frames.iter().flatten().next().map(|wide| wide.scene) {
            outcome.classes[class_index(scene)] += 1;
        }
        Ok(true)
    })?;
    Ok(outcome)
}

fn hex(color: [u8; 3]) -> String {
    color.iter().fold("#".to_owned(), |text, channel| format!("{text}{channel:02X}"))
}

fn replay_files(paths: &[PathBuf]) -> anyhow::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for path in paths {
        if path.is_dir() {
            files.extend(crate::verify_replays::replay_paths(path)?);
        } else {
            files.push(path.clone());
        }
    }
    Ok(files)
}

/// Checks the replays and writes the report; returns the exit code (0 green, 1 not).
///
/// # Errors
///
/// A replay cannot be read or played, or the report cannot be written.
pub fn run_check(
    rom: &[u8],
    default_dir: &Path,
    paths: &[PathBuf],
    frames: Option<u64>,
    jobs: usize,
    widths: &[usize],
    out: &mut dyn Write,
) -> anyhow::Result<u8> {
    let paths = if paths.is_empty() { vec![default_dir.to_path_buf()] } else { paths.to_vec() };
    let files = replay_files(&paths)?;
    anyhow::ensure!(!files.is_empty(), "no replay to check");
    writeln!(out, "hd-wide check: {} replays, widths {widths:?}", files.len())?;
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<(usize, anyhow::Result<Outcome>)>> = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..jobs.min(files.len()) {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(path) = files.get(index) else { break };
                    let outcome = check_replay(rom, path, frames, widths)
                        .with_context(|| format!("checking {}", path.display()));
                    eprintln!("  checked {}", path.display());
                    results.lock().expect("a worker panicked").push((index, outcome));
                }
            });
        }
    });
    let mut results = results.into_inner().expect("a worker panicked");
    results.sort_by_key(|(index, _)| *index);

    let mut lookup_rows = Vec::new();
    let mut scene_rows = Vec::new();
    let mut width_rows = Vec::new();
    let mut details = String::new();
    let mut green = true;
    let mut totals = Outcome {
        widths: widths
            .iter()
            .map(|&width| WidthOutcome { width, ..WidthOutcome::default() })
            .collect(),
        ..Outcome::default()
    };
    for (_, outcome) in results {
        let outcome = outcome?;
        green &= outcome.green();
        lookup_rows.push(vec![
            outcome.name.clone(),
            outcome.frames.to_string(),
            outcome.field_frames.to_string(),
            outcome.pictures.to_string(),
            outcome.tiles.to_string(),
            outcome.tile_differences.to_string(),
            outcome.tiles_outside.to_string(),
            outcome.unbuilt_tiles.to_string(),
            format!("{}/{}", outcome.overscan_differing, outcome.overscan_tiles),
            outcome.parts.to_string(),
            outcome.parts_missing.to_string(),
            outcome.frames_without_loop.to_string(),
        ]);
        if let Some((frame, what)) = &outcome.first_tile {
            let _ = writeln!(
                details,
                "  {}: first tile difference at frame {frame}: {what}",
                outcome.name
            );
        }
        if !outcome.maps.is_empty() {
            let maps: Vec<String> = outcome
                .maps
                .iter()
                .map(|(id, blocks, count, first)| {
                    format!("{id:02X} {}x{} @{first} x{count}", blocks.0, blocks.1)
                })
                .collect();
            let _ = writeln!(details, "  {} maps: {}", outcome.name, maps.join(", "));
        }
        scene_rows.push(vec![
            outcome.name.clone(),
            outcome.classes[0].to_string(),
            outcome.classes[1].to_string(),
            outcome.classes[2].to_string(),
        ]);
        for w in &outcome.widths {
            width_rows.push(vec![
                outcome.name.clone(),
                w.width.to_string(),
                w.rendered.to_string(),
                w.rejected.to_string(),
                w.differing_frames.to_string(),
                w.plain_pixels.to_string(),
                w.extra_pixels.to_string(),
                w.centred.to_string(),
                w.outside_black.to_string(),
                w.outside_drawn.to_string(),
                w.outside_not_black.to_string(),
                w.outside_mismatch.to_string(),
                format!("{}/{}", w.extra_parts, w.extra_dropped),
            ]);
            if let Some(Difference { frame, x, y, classic, wide }) = w.first {
                let _ = writeln!(
                    details,
                    "  {} @{}: first difference at frame {frame} pixel ({x}, {y}): classic {}, wide {}",
                    outcome.name,
                    w.width,
                    hex(classic),
                    hex(wide)
                );
            }
            if let Some((frame, reason)) = &w.rejected_first {
                let all: Vec<String> =
                    w.reasons.iter().map(|(reason, count)| format!("{reason} x{count}")).collect();
                let _ = writeln!(
                    details,
                    "  {} @{}: {} frames refused, the first is frame {frame}: {reason} (all: {})",
                    outcome.name,
                    w.width,
                    w.rejected,
                    all.join("; ")
                );
            }
        }
        totals.frames += outcome.frames;
        totals.field_frames += outcome.field_frames;
        totals.tiles += outcome.tiles;
        totals.tile_differences += outcome.tile_differences;
        totals.tiles_outside += outcome.tiles_outside;
        totals.unbuilt_tiles += outcome.unbuilt_tiles;
        totals.pictures += outcome.pictures;
        totals.overscan_tiles += outcome.overscan_tiles;
        totals.overscan_differing += outcome.overscan_differing;
        totals.parts += outcome.parts;
        totals.parts_missing += outcome.parts_missing;
        totals.frames_without_loop += outcome.frames_without_loop;
        for (total, w) in totals.widths.iter_mut().zip(&outcome.widths) {
            total.rendered += w.rendered;
            total.rejected += w.rejected;
            for (sum, frames) in total.classes.iter_mut().zip(w.classes) {
                *sum += frames;
            }
            total.differing_frames += w.differing_frames;
            total.plain_pixels += w.plain_pixels;
            total.extra_pixels += w.extra_pixels;
            total.centred += w.centred;
            total.outside_black += w.outside_black;
            total.outside_drawn += w.outside_drawn;
            total.outside_not_black += w.outside_not_black;
            total.outside_mismatch += w.outside_mismatch;
            total.extra_parts += w.extra_parts;
            total.extra_dropped += w.extra_dropped;
        }
        for (sum, frames) in totals.classes.iter_mut().zip(outcome.classes) {
            *sum += frames;
        }
    }
    write!(
        out,
        "{}",
        render_table(
            &[
                "replay",
                "frames",
                "field",
                "picture",
                "tiles",
                "differ",
                "outside",
                "unbuilt",
                "stale/overscan",
                "parts",
                "missing",
                "no loop",
            ],
            &lookup_rows
        )
    )?;
    write!(out, "{}", render_table(&["replay", "field", "flight", "bordered"], &scene_rows))?;
    write!(
        out,
        "{}",
        render_table(
            &[
                "replay",
                "width",
                "frames",
                "refused",
                "differ",
                "px plain",
                "px extra",
                "centred",
                "black px",
                "outside drawn",
                "not black",
                "mismatch",
                "extras/dropped",
            ],
            &width_rows
        )
    )?;
    write!(out, "{details}")?;
    writeln!(
        out,
        "hd-wide: {} frames, {} field frames ({} with a picture on BG2); lookup: {} visible tiles, {} differ ({} of them outside the map), plus {} in rows the game left empty and {} stale of {} tiles below line 224; objects: {} parts, {} not in the OAM, {} frames where the object loop did not run",
        totals.frames,
        totals.field_frames,
        totals.pictures,
        totals.tiles,
        totals.tile_differences,
        totals.tiles_outside,
        totals.unbuilt_tiles,
        totals.overscan_differing,
        totals.overscan_tiles,
        totals.parts,
        totals.parts_missing,
        totals.frames_without_loop
    )?;
    writeln!(
        out,
        "hd-wide: scenes: {} field, {} flight, {} bordered",
        totals.classes[0], totals.classes[1], totals.classes[2]
    )?;
    for w in &totals.widths {
        writeln!(
            out,
            "hd-wide: width {}: {} frames ({} field, {} flight, {} bordered), {} refused, {} differ; window pixels away from extra objects {}, under them {}; tiles from outside the map drawn: {}; beside a picture not black: {}; beside the window differing from the widest view: {}",
            w.width,
            w.rendered,
            w.classes[0],
            w.classes[1],
            w.classes[2],
            w.rejected,
            w.differing_frames,
            w.plain_pixels,
            w.extra_pixels,
            w.outside_drawn,
            w.outside_not_black,
            w.outside_mismatch
        )?;
    }
    writeln!(out, "hd-wide: {}", if green { "green" } else { "NOT green" })?;
    Ok(u8::from(!green))
}

/// The frames the default snapshot set shows (label, replay file stem, frame), one or more per
/// scene class, chosen on the reference replays: the field (the first island, a Namek village, a
/// cave (one block), the party over the Namek archipelago), the flights (the plane alone, and the
/// plane with a caption's text below), the title (the logo's Mode 1 picture, and its Mode 7
/// plane), the battle (the command screen, the animation), the story (an ending's staff roll with
/// windows, the last fight's plane) and the shop's interior.
const DEFAULT_SHOTS: [(&str, &str, u64); 13] = [
    ("island", "wander", 3300),
    ("town", "secret-maima-tsumuri", 2500),
    ("cave", "secret-fake-nail", 2300),
    ("namek-flight", "secret-namek-zfighters", 2400),
    ("flight", "battle", 2520),
    ("flight-caption", "battle", 2600),
    ("title", "battle", 2000),
    ("title-mode-7", "battle", 800),
    ("battle-command", "battle", 4500),
    ("battle-animation", "battle", 5400),
    ("staff-roll", "secret-ending-normal", 4800),
    ("last-fight", "secret-ending-normal", 2300),
    ("shop-interior", "shop", 4400),
];

fn png(frame: &Frame) -> Vec<u8> {
    let rgba: Vec<u8> = frame.pixels.iter().flat_map(|c| [c.r, c.g, c.b, 255]).collect();
    crate::shots::encode_png(frame.width as u32, frame.height as u32, &rgba)
}

/// Writes the wide frames of `shots` under `out_dir`: `<label>-<width>.png`, and
/// `<label>-classic.png` for the PPU's. Each line says the scene class and the outside check of
/// the frame: a picture must be black beside its window, and the outside columns must be the
/// same at every wide width.
///
/// # Errors
///
/// A replay cannot be read or played, a file cannot be written.
pub fn run_shots(
    rom: &[u8],
    replays_dir: &Path,
    out_dir: &Path,
    widths: &[usize],
    shots: &[(String, PathBuf, u64)],
    out: &mut dyn Write,
) -> anyhow::Result<u8> {
    let shots: Vec<(String, PathBuf, u64)> = if shots.is_empty() {
        DEFAULT_SHOTS
            .iter()
            .map(|&(label, replay, frame)| {
                (label.to_owned(), replays_dir.join(format!("{replay}.ssdreplay")), frame)
            })
            .collect()
    } else {
        shots.to_vec()
    };
    std::fs::create_dir_all(out_dir).with_context(|| format!("creating {}", out_dir.display()))?;
    let rom_view = Rom(rom);
    let mut missing = 0;
    for (label, replay, target) in &shots {
        let mut renderers: Vec<WideRenderer> = widths.iter().map(|_| WideRenderer::new()).collect();
        let mut found = false;
        play(rom, replay, Some(target + 1), |visit| {
            let mut frames: Vec<Option<WideFrame>> = Vec::with_capacity(widths.len());
            for (&width, renderer) in widths.iter().zip(&mut renderers) {
                let options = WideOptions { width, hud: HudPlacement::Window };
                // Every frame goes through the renderers, so their sprite line buffers carry as in a run
                let wide = renderer.render_view(
                    rom_view,
                    visit.game_mode,
                    visit.state,
                    visit.log,
                    options,
                );
                frames.push(wide.ok());
            }
            if visit.number != *target {
                return Ok(visit.number < *target);
            }
            let widest = frames.iter().flatten().max_by_key(|wide| wide.frame.width);
            for (&width, wide) in widths.iter().zip(&frames) {
                let Some(wide) = wide else {
                    writeln!(out, "{label}: frame {target} at width {width} was refused")?;
                    return Ok(false);
                };
                let path = out_dir.join(format!("{label}-{width}.png"));
                std::fs::write(&path, png(&wide.frame))?;
                let beside = if wide.scene == Scene::Bordered {
                    format!("beside the window {} px not black", outside_not_black(wide))
                } else {
                    String::new()
                };
                let mismatch = match widest {
                    Some(widest) if widest.frame.width > wide.frame.width => {
                        format!(
                            ", {} px beside it differ from {}",
                            outside_mismatch(wide, widest),
                            widest.frame.width
                        )
                    }
                    _ => String::new(),
                };
                writeln!(
                    out,
                    "{}: {:?}, game mode {:02X}, window at {} of {}, {} extra parts{}{}",
                    path.display(),
                    wide.scene,
                    visit.game_mode,
                    wide.logical.x,
                    width,
                    wide.stats.extra_parts,
                    if beside.is_empty() { String::new() } else { format!(", {beside}") },
                    mismatch,
                )?;
                found = true;
            }
            if let Some(classic) = visit.classic {
                std::fs::write(out_dir.join(format!("{label}-classic.png")), png(classic))?;
            }
            Ok(false)
        })?;
        if !found {
            missing += 1;
            writeln!(out, "{label}: frame {target} of {} was not written", replay.display())?;
        }
    }
    Ok(u8::from(missing > 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_takes_limits_widths_and_paths() {
        let args = parse_args(&["check", "--frames", "100", "a.ssdreplay", "--widths", "256,400"])
            .unwrap();
        assert_eq!(
            args,
            Args::Check {
                paths: vec![PathBuf::from("a.ssdreplay")],
                frames: Some(100),
                jobs: 4,
                widths: vec![256, 400]
            }
        );
    }

    #[test]
    fn shots_take_a_label_a_replay_and_a_frame() {
        let args = parse_args(&["shots", "cave=x/y.ssdreplay:42"]).unwrap();
        assert_eq!(
            args,
            Args::Shots {
                widths: WIDTHS.to_vec(),
                shots: vec![("cave".to_owned(), PathBuf::from("x/y.ssdreplay"), 42)]
            }
        );
    }

    #[test]
    fn a_bad_command_line_says_why() {
        assert!(parse_args(&[]).unwrap_err().contains("subcommand"));
        assert!(parse_args(&["wide"]).unwrap_err().contains("unknown subcommand"));
        assert!(parse_args(&["check", "--widths", "100"]).unwrap_err().contains("256 to"));
        assert!(parse_args(&["check", "--widths", "513"]).unwrap_err().contains("256 to"));
        assert!(parse_args(&["check", "--fast"]).unwrap_err().contains("unknown option"));
        assert!(parse_args(&["shots", "nope"]).unwrap_err().contains("label=replay:frame"));
    }
}
