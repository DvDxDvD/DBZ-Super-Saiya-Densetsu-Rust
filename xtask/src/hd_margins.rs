//! `hd-art margins`: the field's objects in the margins of the 16:9 view, drawn from the art as
//! the ones inside the original window are.
//!
//! A map is booted from a crafted save (as `hd-tour` does), the camera is moved over the places
//! of the map's grid (or to the places asked for), and each place is drawn as the 16:9 Enhanced
//! picture with the pack's art. Without shots the command scans: it counts, for every place, the
//! pixels of the object layer inside the original window and in the margins, and how many of each
//! are drawn from art, and exits 1 when the margins are covered less than the window. With shots
//! (`label=MAP:X,Y` or `label=MAP:X,Y:DX`, the last a sequence of three frames the party walks
//! `DX` pixels between) it writes the pictures.

use crate::save::Game;
use crate::shared::render_table;
use crate::tour_battle::Session;
use crate::tour_visit::{blocks_of, boot_free, is_free, place, places};
use anyhow::Context;
use ssd_hd::pack::Pack;
use ssd_hd::world::WideRun;
use ssd_hd::{HdOptions, KindPixels, Look};
use ssd_oracle::Joypad;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The frames a place settles for before it is drawn (the objects follow the camera each frame).
const SETTLE: u32 = 3;

/// The share (percent) the margins may fall short of the window's art coverage, once there are
/// [`MIN_PIXELS`] pixels of sprites in the margins to tell.
pub const TOLERANCE: u64 = 3;
pub const MIN_PIXELS: u64 = 400;

/// A picture asked for: the map, the camera and, for a sequence of three frames, the step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shot {
    pub label: String,
    pub map: u8,
    pub camera: (u16, u16),
    pub step: Option<i32>,
}

/// What `hd-art margins` was asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    pub maps: Vec<u8>,
    pub scale: u32,
    pub out: Option<PathBuf>,
    pub pack: Option<PathBuf>,
    pub base: Option<PathBuf>,
    pub shots: Vec<Shot>,
    /// Maps visited at once.
    pub jobs: usize,
    /// Lists the places that have sprites in the margins.
    pub list: bool,
    /// Lists the places where a group drawn from art whose id starts with this is wholly in the
    /// left margin (`--find field-buildings` finds the houses).
    pub find: Option<String>,
    /// Lists the map objects of each map (type and world position) instead of drawing places.
    pub objects: bool,
}

/// Parses a shot: `label=MAP:X,Y` or `label=MAP:X,Y:DX`.
///
/// # Errors
///
/// The text is not of that form.
pub fn parse_shot(text: &str) -> Result<Shot, String> {
    let form = "a shot is label=MAP:X,Y or label=MAP:X,Y:DX";
    let (label, rest) = text.split_once('=').ok_or(form)?;
    let mut parts = rest.split(':');
    let map = parts.next().and_then(|n| n.parse().ok()).ok_or(form)?;
    let (x, y) = parts
        .next()
        .and_then(|c| c.split_once(','))
        .and_then(|(x, y)| Some((x.parse().ok()?, y.parse().ok()?)))
        .ok_or(form)?;
    let step = match parts.next() {
        Some(step) => Some(step.parse().map_err(|_| form)?),
        None => None,
    };
    Ok(Shot { label: label.to_owned(), map, camera: (x, y), step })
}

/// The share of `part` in `whole`, in percent (0 when there is nothing).
pub(crate) fn share(part: u64, whole: u64) -> u64 {
    (part * 100).checked_div(whole).unwrap_or(0)
}

fn percent(part: u64, whole: u64) -> String {
    if whole == 0 { "-".to_owned() } else { format!("{:.1}", part as f64 * 100.0 / whole as f64) }
}

fn add(total: &mut KindPixels, more: KindPixels) {
    total.total += more.total;
    total.art += more.art;
    total.recoloured += more.recoloured;
    total.missing += more.missing;
    total.partial += more.partial;
}

/// Whether the margins' coverage is as good as the window's: the check of the scan and of
/// `hd-art report`.
#[must_use]
pub fn margins_match(inside: KindPixels, margin: KindPixels) -> bool {
    margin.total < MIN_PIXELS
        || inside.total < MIN_PIXELS
        || share(margin.art, margin.total) + TOLERANCE >= share(inside.art, inside.total)
}

/// What one map's places added up to.
#[derive(Default)]
struct MapResult {
    places: u32,
    seen: u32,
    window: KindPixels,
    margin: KindPixels,
    extras: u64,
    lines: Vec<String>,
}

/// Draws the frame the camera at `at` shows: moves the camera and the party there, plays
/// `frames` frames and returns the picture of the last. `None` when a scene took over.
fn view(
    session: &mut Session,
    run: &mut WideRun,
    at: (u16, u16),
    frames: u32,
    options: HdOptions,
) -> anyhow::Result<Option<ssd_hd::world::WideHd>> {
    let mut last = None;
    for t in 0..frames {
        place(session, at)?;
        session.step(Joypad::default())?;
        if !matches!(session.mode(), 0 | 6) {
            return Ok(None);
        }
        if t + 1 < frames {
            run.skip(session.machine.wram());
        } else {
            last = run.note_hd(session.machine.wram(), session.machine.frame_log(), options);
        }
    }
    Ok(last)
}

/// The frames the party may walk for a sequence's step.
const WALK_LIMIT: u32 = 600;

/// Plays `frames` frames with the pad `pad`, offering them to `run`, and draws the last one.
fn play(
    session: &mut Session,
    run: &mut WideRun,
    frames: u32,
    pad: Joypad,
    options: HdOptions,
) -> anyhow::Result<Option<ssd_hd::world::WideHd>> {
    let mut last = None;
    for t in 0..frames {
        session.step(pad)?;
        if !matches!(session.mode(), 0 | 6) {
            return Ok(None);
        }
        if t + 1 < frames {
            run.skip(session.machine.wram());
        } else {
            last = run.note_hd(session.machine.wram(), session.machine.frame_log(), options);
        }
    }
    Ok(last)
}

/// The camera as it stands.
fn camera_of(session: &Session) -> (u16, u16) {
    (session.wram16(0x33), session.wram16(0x35))
}

fn new_run(rom: &[u8], pack: &Arc<Pack>) -> WideRun {
    let mut run = WideRun::new(rom);
    run.set_art(Some(Arc::clone(pack)));
    run.set_trust_field(true);
    run
}

/// Visits the places of map `id`.
fn scan_map(
    rom: &[u8],
    pack: &Arc<Pack>,
    game: &Game,
    base: &[u8],
    id: u8,
    scale: u32,
    (list, find): (bool, Option<&str>),
) -> anyhow::Result<MapResult> {
    let mut result = MapResult::default();
    let Some(mut session) = boot_free(rom, game, base, id, None)? else { return Ok(result) };
    let grid = places(blocks_of(&session));
    result.places = grid.len() as u32;
    let free_state = session.machine.save_state();
    let mut run = new_run(rom, pack);
    let options = HdOptions { scale, look: Look::Crisp };
    for at in grid {
        match view(&mut session, &mut run, at, SETTLE + 1, options)? {
            Some(picture) => {
                result.seen += 1;
                let stats = run.art_stats();
                add(&mut result.margin, stats.sprites_margin);
                add(&mut result.window, stats.sprites_window);
                result.extras += u64::from(picture.stats.extra_parts);
                if let Some(prefix) = find {
                    let found: Vec<String> = stats
                        .poses
                        .iter()
                        .filter(|p| p.field && p.org.0 + p.size.0 as i32 <= 0)
                        .filter_map(|p| p.art.map(|art| (p, &pack.art.objects()[art as usize].id)))
                        .filter(|(_, id)| id.starts_with(prefix))
                        .map(|(p, id)| format!("{id} at {},{}", p.org.0, p.org.1))
                        .collect();
                    if !found.is_empty() {
                        let inside: Vec<&str> = stats
                            .poses
                            .iter()
                            .filter(|p| p.field && (0..256).contains(&p.org.0))
                            .filter_map(|p| {
                                p.art.map(|art| pack.art.objects()[art as usize].id.as_str())
                            })
                            .collect();
                        result.lines.push(format!(
                            "  map {id} camera {},{}: left margin: {}; in the window: {}",
                            at.0,
                            at.1,
                            found.join(", "),
                            inside.join(", ")
                        ));
                    }
                }
                if list && stats.sprites_margin.total > 0 {
                    let window = stats.sprites_window;
                    result.lines.push(format!(
                        "  map {id} camera {},{}: margins {} px ({}% art), window {} px ({}% art), {} extra parts",
                        at.0,
                        at.1,
                        stats.sprites_margin.total,
                        percent(stats.sprites_margin.art, stats.sprites_margin.total),
                        window.total,
                        percent(window.art, window.total),
                        picture.stats.extra_parts
                    ));
                }
            }
            None => {
                // A scene took over: the free state again
                session.machine.load_state(&free_state)?;
                session.machine.set_line_log(true);
                run = new_run(rom, pack);
                anyhow::ensure!(is_free(&session), "map {id}: the free state is not free");
            }
        }
    }
    Ok(result)
}

/// Writes the pictures of `shots`: each is a map booted with the party where the camera of a
/// window at the camera asked for centres it (the map's columns are loaded around it as the game
/// loads them), settled for a few frames. A sequence goes on from there with the pad held to the
/// side the step says, drawing a frame each time the camera has moved `|DX|` pixels.
fn run_shots(
    rom: &[u8],
    pack: &Arc<Pack>,
    game: &Game,
    base: &[u8],
    args: &Args,
    out_dir: &Path,
    out: &mut dyn Write,
) -> anyhow::Result<u8> {
    std::fs::create_dir_all(out_dir)?;
    let options = HdOptions { scale: args.scale, look: Look::Crisp };
    let mut missing = 0;
    for shot in &args.shots {
        let mut session = boot_free(rom, game, base, shot.map, Some(shot.camera))?
            .with_context(|| format!("map {}: the party was never free", shot.map))?;
        let mut run = new_run(rom, pack);
        run.set_trust_field(false);
        let frames = if shot.step.is_some() { 3 } else { 1 };
        let start = camera_of(&session);
        for n in 0..frames {
            let picture = if n == 0 {
                play(&mut session, &mut run, SETTLE + 5, Joypad::default(), options)?
            } else {
                // Walk until the camera is |DX| pixels further than at the last frame drawn
                let step = shot.step.unwrap_or(0);
                let target = i32::from(start.0) + step * n;
                let pad = Joypad { right: step > 0, left: step < 0, ..Joypad::default() };
                let mut picture = None;
                for _ in 0..WALK_LIMIT {
                    let drawn = play(&mut session, &mut run, 1, pad, options)?;
                    let x = i32::from(camera_of(&session).0);
                    if (step > 0 && x >= target) || (step < 0 && x <= target) {
                        picture = drawn;
                        break;
                    }
                }
                picture
            };
            let Some(picture) = picture else {
                writeln!(out, "{}: a scene took over, or the party did not walk", shot.label)?;
                missing += 1;
                break;
            };
            let camera = camera_of(&session);
            let name = if shot.step.is_some() {
                format!("{}-{}.png", shot.label, n + 1)
            } else {
                format!("{}.png", shot.label)
            };
            let hd = &picture.picture;
            std::fs::write(
                out_dir.join(&name),
                crate::shots::encode_png(hd.width, hd.height, &hd.rgba),
            )?;
            let stats = run.art_stats();
            writeln!(
                out,
                "{name}: map {} camera {},{} {:?} {}x{}, {} extra parts ({} without an entry); sprites: margins {}/{} px art, window {}/{}",
                shot.map,
                camera.0,
                camera.1,
                picture.scene,
                hd.width,
                hd.height,
                picture.stats.extra_parts,
                picture.stats.extra_dropped,
                stats.sprites_margin.art,
                stats.sprites_margin.total,
                stats.sprites_window.art,
                stats.sprites_window.total
            )?;
            if stats.sprites_cut > 0 {
                writeln!(
                    out,
                    "    {} px of groups cut by the view's edge with no art are left out",
                    stats.sprites_cut
                )?;
            }
            for pose in stats.poses.iter().filter(|p| p.field) {
                let art = pose.art.map(|art| pack.art.objects()[art as usize].id.as_str());
                writeln!(
                    out,
                    "    group {:016x} at {},{} {}x{} px, {} opaque, {} visible: {}",
                    pose.key,
                    pose.org.0,
                    pose.org.1,
                    pose.size.0,
                    pose.size.1,
                    pose.opaque,
                    pose.visible,
                    if pose.matched {
                        format!("{:?} {}", pose.source, art.unwrap_or("shadow"))
                    } else {
                        "no art".to_owned()
                    }
                )?;
            }
        }
    }
    Ok(u8::from(missing > 0))
}

/// Runs `hd-art margins`.
///
/// # Errors
///
/// The pack, the base save, a map or a file fails.
pub fn run(
    rom: &[u8],
    root: &Path,
    pack_dir: &Path,
    args: &Args,
    out: &mut dyn Write,
) -> anyhow::Result<u8> {
    let pack =
        Arc::new(Pack::open(pack_dir).with_context(|| format!("the pack {}", pack_dir.display()))?);
    let base_path = match &args.base {
        Some(path) => path.clone(),
        None => crate::tour::find_base_save(root)?,
    };
    let base = std::fs::read(&base_path).with_context(|| base_path.display().to_string())?;
    let game = Game::from_rom(rom)?;
    if !args.shots.is_empty() {
        let dir = args.out.clone().unwrap_or_else(|| root.join(".work").join("margins"));
        return run_shots(rom, &pack, &game, &base, args, &dir, out);
    }
    let maps: Vec<u8> = if args.maps.is_empty() { (0..48).collect() } else { args.maps.clone() };
    if args.objects {
        for &id in &maps {
            let Some(session) = boot_free(rom, &game, &base, id, None)? else { continue };
            let wram = session.machine.wram();
            let Some(state) = ssd_hd::world::FieldState::from_wram(wram) else { continue };
            let mut found: Vec<String> = state
                .map_objects()
                .map(|o| {
                    format!(
                        "{}@{},{}",
                        o.kind,
                        o.x + i32::from(state.camera.0),
                        o.y + i32::from(state.camera.1)
                    )
                })
                .collect();
            found.sort();
            writeln!(out, "map {id}: {}", found.join(" "))?;
        }
        return Ok(0);
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let done = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..args.jobs.clamp(1, maps.len()) {
            scope.spawn(|| {
                loop {
                    let at = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(&id) = maps.get(at) else { break };
                    let result = scan_map(
                        rom,
                        &pack,
                        &game,
                        &base,
                        id,
                        args.scale,
                        (args.list, args.find.as_deref()),
                    );
                    done.lock().expect("results").push((at, id, result));
                }
            });
        }
    });
    let mut results = done.into_inner().expect("results");
    results.sort_by_key(|(at, _, _)| *at);
    let mut rows = Vec::new();
    let (mut window, mut margin) = (KindPixels::default(), KindPixels::default());
    for (_, id, result) in results {
        let result = result.with_context(|| format!("map {id}"))?;
        for line in &result.lines {
            writeln!(out, "{line}")?;
        }
        rows.push(vec![
            id.to_string(),
            format!("{}/{}", result.seen, result.places),
            result.extras.to_string(),
            result.window.total.to_string(),
            percent(result.window.art, result.window.total),
            result.margin.total.to_string(),
            percent(result.margin.art, result.margin.total),
        ]);
        add(&mut window, result.window);
        add(&mut margin, result.margin);
    }
    write!(
        out,
        "{}",
        render_table(
            &[
                "map",
                "places",
                "extra parts",
                "window px",
                "window% art",
                "margin px",
                "margin% art"
            ],
            &rows
        )
    )?;
    writeln!(
        out,
        "hd-art margins: field sprites inside the window {} px, {}% drawn from art; in the margins {} px, {}%",
        window.total,
        percent(window.art, window.total),
        margin.total,
        percent(margin.art, margin.total)
    )?;
    if margins_match(window, margin) {
        Ok(0)
    } else {
        writeln!(out, "hd-art margins: FAIL, the margins are covered less than the window")?;
        Ok(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shots_are_a_label_a_map_a_camera_and_maybe_a_step() {
        let shot = parse_shot("home=3:512,256").unwrap();
        assert_eq!(shot, Shot { label: "home".into(), map: 3, camera: (512, 256), step: None });
        let shot = parse_shot("cross=9:100,40:-28").unwrap();
        assert_eq!(shot.step, Some(-28));
        assert!(parse_shot("nothing").is_err() && parse_shot("a=1:2").is_err());
    }

    #[test]
    fn the_margins_match_when_they_are_covered_as_the_window_is() {
        let k = |art, total| KindPixels { total, art, ..KindPixels::default() };
        assert!(margins_match(k(900, 1000), k(880, 1000)));
        assert!(!margins_match(k(900, 1000), k(300, 1000)), "pixelated margins");
        // Too few pixels in the margins to tell
        assert!(margins_match(k(900, 1000), k(0, 100)));
    }
}
