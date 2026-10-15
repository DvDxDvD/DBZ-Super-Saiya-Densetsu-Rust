//! The parts of an `hd-tour` and how each one is played: every part makes a machine produce frames
//! (a replay, a forced battle, a map visited at a grid of camera positions, a scripted walk through
//! the menus, the title and the intro), feeds them to the wide Enhanced renderer with the art
//! index and counts what it draws ([`Recorder`]). A part's result is a file, so a tour can be
//! stopped and resumed (`tour.rs`).

use crate::save::Game;
use crate::shared::read_replay;
use crate::tour_account::{AssetTable, Class, Context, Totals, Where};
use crate::tour_battle::{Episode, Scenario, Session, crafted_bytes, run_episode};
use crate::tour_sheet::{self, Crop};
use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use ssd_hd::pack::Pack;
use ssd_hd::world::{FieldState, Rom, Scene, WideRun, check_tiles};
use ssd_hd::{HdOptions, Look};
use ssd_oracle::{Machine, NullAudio};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// The version of the part files: bump it when what a part counts changes.
pub const FORMAT: u32 = 1;

/// Crops kept for each class of a part (the things that took the most pixels).
const CROPS_PER_CLASS: usize = 48;

/// The brightness (sum of the channels) of the frame each portrait stream was written from (the
/// parts run on several threads).
static GAPS_LIGHT: Mutex<std::collections::BTreeMap<String, u64>> =
    Mutex::new(std::collections::BTreeMap::new());

/// What every part of a tour shares.
pub struct Env<'a> {
    pub rom: &'a [u8],
    pub pack: Arc<Pack>,
    pub table: Arc<AssetTable>,
    pub game: &'a Game,
    /// The battery save the crafted saves are made from.
    pub base_save: Vec<u8>,
    /// One frame in `stride` of a long part is drawn and counted.
    pub stride: u64,
    /// With the scene audit, `burst` consecutive frames of each `stride` are counted instead.
    pub burst: u64,
    /// The scene audit stops a replay after this many frames (`SCENES_UNTIL`, to look at a scene).
    pub limit: Option<u64>,
    /// A folder to write the pictures of the frames counted to (for checking the tour itself),
    /// and every how many counted frames.
    pub dump: Option<(PathBuf, u64)>,
    /// A folder to write what has no art to, for making it (`--gaps`): each pose with none (the
    /// palette indexes and colours of its sprites, `<kind>-<fate>-<key>.json`, and an effect's
    /// colours as a PNG) and, for each portrait stream with an unbound tile, the 1x picture of the
    /// first frame that showed it (`portrait-<asset>.png` and its `.json`). Derived from the game:
    /// never tracked.
    pub gaps: Option<PathBuf>,
    /// A folder for the art audit of the battles' fighters (`--audit`, see `tour_audit`), and one
    /// for a 4x picture of each battle's formation (`--formations`: the frame of the first 3000
    /// where the most fighters show).
    pub audit: Option<PathBuf>,
    pub formations: Option<PathBuf>,
    /// The HD text font, whose cells count as drawn from art (`None`: the text is the filter's).
    pub font: Option<Arc<ssd_hd::Font>>,
    /// Where `hd-art scenes` collects the scenes the frames make (see `scenes`).
    pub scenes: Option<Arc<crate::scenes::Sink>>,
    /// The state of each scenario's map, once booted: the episodes of a scenario start from it.
    boots: Mutex<BTreeMap<String, Arc<[u8]>>>,
}

impl<'a> Env<'a> {
    #[must_use]
    pub fn new(
        rom: &'a [u8],
        pack: Arc<Pack>,
        game: &'a Game,
        base_save: Vec<u8>,
        stride: u64,
    ) -> Self {
        let table = Arc::new(AssetTable::new(&pack.index));
        Self {
            rom,
            pack,
            table,
            game,
            base_save,
            stride: stride.max(1),
            burst: 1,
            limit: None,
            dump: None,
            gaps: None,
            audit: None,
            formations: None,
            font: None,
            scenes: None,
            boots: Mutex::new(BTreeMap::new()),
        }
    }
}

/// What a part found.
#[derive(Debug, Default)]
pub struct PartOutput {
    pub totals: Totals,
    /// The picture of the first sighting of some of the items of `totals`, by item key.
    pub crops: BTreeMap<String, Crop>,
    /// What the part says about itself (a scene it could not reach, the menu screens it saw).
    pub notes: Vec<String>,
}

/// Counts the frames of a part.
pub struct Recorder<'a> {
    env: &'a Env<'a>,
    run: WideRun,
    part: String,
    totals: Totals,
    crops: BTreeMap<String, Crop>,
    /// The frames offered.
    offered: u64,
    /// Whether frames of a field state are drawn as the field whatever BG2 holds.
    trust: bool,
    /// The files of `Env::gaps` this recorder has written or found.
    gaps_done: std::collections::HashSet<String>,
    audit: Option<crate::tour_audit::Audit>,
    /// The run that draws the formation pictures at 4x (art, no tally), the most enemies seen and
    /// the picture of that frame.
    big: Option<WideRun>,
    best: Option<Formation>,
    /// The poses of the frame just counted (for the formation picture's list).
    uses: Vec<ssd_hd::PoseStat>,
    /// The fighters of the frame just counted, when it is a battle's.
    enemies: Option<usize>,
    /// The scene audit of `hd-art scenes`.
    watcher: Option<crate::scenes::Watcher>,
}

/// The frame of a battle where the most enemies show: its 4x picture and its poses.
struct Formation {
    enemies: usize,
    frame: u64,
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    uses: Vec<ssd_hd::PoseStat>,
}

impl Formation {
    /// `<part>.png` and `<part>.json` (the frame and, for each fighter, where it is and where its
    /// art came from).
    fn write(&self, dir: &Path, part: &str, pack: &Pack) {
        let _ = std::fs::create_dir_all(dir);
        let _ = ssd_hd::png::write_rgba(
            &dir.join(format!("{part}.png")),
            self.width,
            self.height,
            &self.rgba,
        );
        let objects = pack.art.objects();
        let fighters: Vec<_> = self
            .uses
            .iter()
            .filter(|p| p.opaque >= 100)
            .map(|p| {
                serde_json::json!({
                    "fighter": p.fighter,
                    "field": p.field,
                    "key": format!("{:016x}", p.key),
                    "palette": p.palette,
                    "org": [p.org.0, p.org.1],
                    "size": [p.size.0, p.size.1],
                    "opaque": p.opaque,
                    "source": format!("{:?}", p.source),
                    "art": p.art.and_then(|at| objects.get(at as usize)).map(|o| o.id.clone()),
                })
            })
            .collect();
        let json = serde_json::json!({ "part": part, "frame": self.frame, "fighters": fighters });
        let _ = std::fs::write(dir.join(format!("{part}.json")), json.to_string());
    }
}

/// The frames of a battle a formation picture is chosen among.
const FORMATION_FRAMES: u64 = 3000;

impl<'a> Recorder<'a> {
    #[must_use]
    pub fn new(env: &'a Env<'a>, part: &str) -> Self {
        Self {
            env,
            run: new_run(env),
            part: part.to_owned(),
            totals: Totals::default(),
            crops: BTreeMap::new(),
            offered: 0,
            trust: false,
            gaps_done: std::collections::HashSet::new(),
            audit: env.audit.is_some().then(crate::tour_audit::Audit::default),
            big: env.formations.is_some().then(|| big_run(env)),
            best: None,
            uses: Vec::new(),
            enemies: None,
            watcher: env.scenes.as_ref().map(|sink| {
                crate::scenes::Watcher::new(
                    Arc::clone(sink),
                    part,
                    env.rom,
                    &env.pack,
                    env.font.as_ref(),
                )
            }),
        }
    }

    /// Frames of a field state are drawn as the field without checking BG2 (the camera was moved by
    /// writing the machine's variables); the party must be free on the map.
    pub fn trust_field(&mut self, trust: bool) {
        self.trust = trust;
        self.run.set_trust_field(trust);
        if let Some(watcher) = &mut self.watcher {
            watcher.set_trust(trust);
        }
    }

    /// Forgets the frames seen: the machine jumped (a loaded state), so the field variables the
    /// next frame starts from are not the last frame's.
    pub fn reset(&mut self) {
        self.run = new_run(self.env);
        self.run.set_trust_field(self.trust);
        if let Some(watcher) = &mut self.watcher {
            watcher.reset(self.env.rom, self.env.font.as_ref());
        }
    }

    /// A frame ran and is not counted.
    pub fn skip(&mut self, machine: &Machine) {
        self.run.skip(machine.wram());
        if let Some(watcher) = &mut self.watcher {
            watcher.skip(machine);
        }
        if let Some(big) = &mut self.big {
            big.skip(machine.wram());
        }
    }

    /// The formation run follows every frame: it draws one at 4x when it shows more enemies than
    /// any before it (of the first frames of the battle).
    fn big_frame(&mut self, machine: &Machine, frame: u64) {
        let Some(big) = &mut self.big else { return };
        let best = self.best.as_ref().map_or(0, |best| best.enemies);
        let more = self.enemies.is_some_and(|n| frame <= FORMATION_FRAMES && n > best);
        if more
            && let Some(picture) = big.note_hd(
                machine.wram(),
                machine.frame_log(),
                HdOptions { scale: 4, look: Look::Crisp },
            )
        {
            let hd = &picture.picture;
            self.best = Some(Formation {
                enemies: self.enemies.unwrap_or(0),
                frame,
                width: hd.width,
                height: hd.height,
                rgba: hd.rgba.clone(),
                uses: std::mem::take(&mut self.uses),
            });
        } else {
            big.skip(machine.wram());
        }
    }

    /// A frame ran: it is counted when it is one of the stride.
    pub fn feed(&mut self, machine: &Machine, frame: u64) {
        self.offered += 1;
        self.enemies = None;
        let (stride, burst) = if self.env.scenes.is_some() && !self.part.starts_with("replay-") {
            (1, 1)
        } else {
            (self.env.stride, self.env.burst)
        };
        if (self.offered - 1) % stride < burst {
            self.count(machine, frame);
        } else {
            self.run.skip(machine.wram());
            // the scene audit's runs follow every frame too: their field state must not lag the gap
            if let Some(watcher) = &mut self.watcher {
                watcher.skip(machine);
            }
        }
        self.big_frame(machine, frame);
    }

    /// A frame ran and is counted.
    pub fn count(&mut self, machine: &Machine, frame: u64) {
        let wram = machine.wram();
        let options = HdOptions { scale: 1, look: Look::Crisp };
        let Some(picture) = self.run.note_hd(wram, machine.frame_log(), options) else {
            self.totals.refused += 1;
            if let Some(watcher) = &mut self.watcher {
                watcher.skip(machine);
            }
            return;
        };
        if picture.scene == Scene::Bordered
            && std::env::var_os("TOUR_DEBUG").is_some()
            && let (Some(state), Some(log)) = (FieldState::from_wram(wram), machine.frame_log())
        {
            let check = check_tiles(Rom(self.env.rom), &state, log);
            eprintln!(
                "{}: frame {frame} bordered: mode {:02X} camera {:?} {check:?}",
                self.part, wram[0x24], state.camera
            );
        }
        match picture.scene {
            Scene::Field => self.totals.field_frames += 1,
            Scene::Flight => self.totals.flight_frames += 1,
            Scene::Bordered => self.totals.picture_frames += 1,
        }
        let tally = self.run.tally();
        let ctx = Context::of_wram(wram);
        if let Some(watcher) = &mut self.watcher {
            watcher.observe(machine, frame, &tally, picture.scene, &self.env.table);
        }
        if ctx.battle {
            if let Some(audit) = &mut self.audit {
                audit.note(&tally.uses, &self.env.pack, &self.part, frame);
            }
            if self.big.is_some() {
                self.uses.clone_from(&tally.uses);
                // Whole fighters (not the stripes of a fade-in)
                self.enemies = Some(
                    tally
                        .uses
                        .iter()
                        .filter(|p| p.fighter && !p.field && p.opaque >= 100 && p.stripy < 200)
                        .count(),
                );
            }
        }
        let at = Where { part: self.part.clone(), frame };
        let fresh = self.totals.add_frame(&tally, ctx, &self.env.table, &at);
        for key in fresh {
            let bbox = self.totals.items[&key].bbox;
            let rgba = &picture.picture.rgba;
            let (width, height) = (picture.picture.width, picture.picture.height);
            if let Some(crop) = tour_sheet::crop(rgba, width, height, bbox) {
                self.crops.insert(key, crop);
            }
        }
        if let Some(dir) = &self.env.gaps {
            self.write_gaps(dir, &tally, &picture.picture, &at);
        }
        if let Some((dir, every)) = &self.env.dump
            && self.totals.frames % every == 1
        {
            let name = format!("{}-{frame:06}.png", self.part);
            let _ = std::fs::create_dir_all(dir);
            let _ = ssd_hd::png::write_rgba(
                &dir.join(name),
                picture.picture.width,
                picture.picture.height,
                &picture.picture.rgba,
            );
        }
    }

    /// Writes what the frame shows without art (see [`Env::gaps`]), once for each pose and each
    /// portrait stream.
    fn write_gaps(
        &mut self,
        dir: &Path,
        tally: &ssd_hd::FrameTally,
        picture: &ssd_hd::HdFrame,
        at: &Where,
    ) {
        let _ = std::fs::create_dir_all(dir);
        let seen = format!("{} frame {}", at.part, at.frame);
        for shape in &tally.shapes {
            let name = format!(
                "{}-{}-{:016x}",
                if shape.field {
                    "field"
                } else if shape.fighter {
                    "fighter"
                } else {
                    "effect"
                },
                if shape.fate == ssd_hd::Fate::Partial { "partial" } else { "missing" },
                shape.key
            );
            if !self.gaps_done.insert(name.clone()) || dir.join(format!("{name}.json")).exists() {
                continue;
            }
            if !shape.rgba.is_empty() {
                let _ = ssd_hd::png::write_rgba(
                    &dir.join(format!("{name}.png")),
                    shape.w as u32,
                    shape.h as u32,
                    &shape.rgba,
                );
            }
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
                "first_seen": seen,
            });
            let _ = std::fs::write(dir.join(format!("{name}.json")), json.to_string());
        }
        // The portrait streams with a tile that has no art: the first frame where each shows at
        // its best (most contrast in the box of its tiles: a fade-in starts dark, a fade-out ends
        // white), with the tiles' box
        let mut boxes: std::collections::BTreeMap<u32, (u16, u16, u16, u16, u32)> =
            std::collections::BTreeMap::new();
        for item in &tally.items {
            let ssd_hd::ItemKey::Tile { asset, tile, .. } = item.key else { continue };
            if self.env.table.kind(asset) != Some(crate::tour_account::Kind::Portrait) {
                continue;
            }
            let held = boxes.entry(asset).or_insert((u16::MAX, u16::MAX, 0, 0, tile));
            held.0 = held.0.min(item.bbox.0);
            held.1 = held.1.min(item.bbox.1);
            held.2 = held.2.max(item.bbox.2);
            held.3 = held.3.max(item.bbox.3);
        }
        for (asset, (x0, y0, x1, y1, tile)) in boxes {
            let id = self.env.table.id(asset).replace(['/', ':'], "-");
            let name = format!("portrait-{id}");
            let mut sum = 0_u64;
            let mut square = 0_u64;
            let mut count = 0_u64;
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let at = (usize::from(y) * picture.width as usize + usize::from(x)) * 4;
                    if let Some(p) = picture.rgba.get(at..at + 3) {
                        let luma =
                            (u64::from(p[0]) * 3 + u64::from(p[1]) * 6 + u64::from(p[2])) / 10;
                        sum += luma;
                        square += luma * luma;
                        count += 1;
                    }
                }
            }
            if count == 0 {
                continue;
            }
            // The variance of the luminance over the box, in sixteenths
            let contrast = (square * count - sum * sum) * 16 / (count * count);
            let mut lights = GAPS_LIGHT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if contrast <= lights.get(&name).copied().unwrap_or(0) {
                continue;
            }
            lights.insert(name.clone(), contrast);
            drop(lights);
            let _ = ssd_hd::png::write_rgba(
                &dir.join(format!("{name}.png")),
                picture.width,
                picture.height,
                &picture.rgba,
            );
            let json = serde_json::json!({
                "asset": self.env.table.id(asset),
                "tile": tile,
                "bbox": [x0, y0, x1, y1],
                "first_seen": seen,
            });
            let _ = std::fs::write(dir.join(format!("{name}.json")), json.to_string());
        }
    }

    /// The result: the totals and the crops of the items that took the most pixels in each class.
    #[must_use]
    pub fn finish(mut self, notes: Vec<String>) -> PartOutput {
        if let Some(watcher) = &mut self.watcher {
            watcher.finish();
        }
        let mut keep = std::collections::BTreeSet::new();
        for class in crate::tour_account::CLASSES {
            for (key, _) in self.totals.unbound(class).into_iter().take(CROPS_PER_CLASS) {
                keep.insert(key.clone());
            }
        }
        let crops = self.crops.into_iter().filter(|(key, _)| keep.contains(key)).collect();
        if let (Some(audit), Some(dir)) = (&self.audit, &self.env.audit) {
            audit.write(dir, &self.part);
        }
        if let (Some(best), Some(dir)) = (&self.best, &self.env.formations) {
            best.write(dir, &self.part, &self.env.pack);
        }
        let mut notes = notes;
        notes.push(format!(
            "frames drawn as: field {}, flight {}, picture {}",
            self.totals.field_frames, self.totals.flight_frames, self.totals.picture_frames
        ));
        PartOutput { totals: self.totals, crops, notes }
    }
}

/// The run that draws the formation pictures: the art at 4x, no tally.
fn big_run(env: &Env<'_>) -> WideRun {
    let mut run = WideRun::new(env.rom);
    run.set_art(Some(Arc::clone(&env.pack)));
    run.set_font(env.font.clone());
    run
}

fn new_run(env: &Env<'_>) -> WideRun {
    let mut run = WideRun::new(env.rom);
    run.set_art(Some(Arc::clone(&env.pack)));
    run.set_font(env.font.clone());
    run.set_tally(true);
    run
}

// ---------------------------------------------------------------------------------------------
// The part files

/// What is kept of a part on disk: its result and what it was made with.
#[derive(Debug, Serialize, Deserialize)]
pub struct PartFile {
    pub format: u32,
    /// The pack, ROM and core the part was counted with, and the part's own parameters: a part
    /// is counted again when any of them changed.
    pub fingerprint: String,
    pub name: String,
    pub seconds: f64,
    pub notes: Vec<String>,
    pub totals: Totals,
    /// The crops of the items, by item key: files under the part's folder.
    pub crops: BTreeMap<String, String>,
}

/// The file name of `key`'s crop: a stable hash of the key.
fn crop_file(key: &str) -> String {
    let hash = key.bytes().fold(0xCBF2_9CE4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01B3)
    });
    format!("{hash:016x}.png")
}

/// Writes a part's result under `parts`.
///
/// # Errors
///
/// A file cannot be written.
pub fn save_part(
    parts: &Path,
    name: &str,
    fingerprint: &str,
    output: &PartOutput,
    seconds: f64,
) -> anyhow::Result<()> {
    let folder = parts.join(name);
    if folder.exists() {
        std::fs::remove_dir_all(&folder)?;
    }
    let mut crops = BTreeMap::new();
    if !output.crops.is_empty() {
        std::fs::create_dir_all(&folder)?;
    }
    for (key, crop) in &output.crops {
        let file = crop_file(key);
        ssd_hd::png::write_rgba(&folder.join(&file), crop.width, crop.height, &crop.rgba)?;
        crops.insert(key.clone(), format!("{name}/{file}"));
    }
    let file = PartFile {
        format: FORMAT,
        fingerprint: fingerprint.to_owned(),
        name: name.to_owned(),
        seconds,
        notes: output.notes.clone(),
        totals: output.totals.clone(),
        crops,
    };
    std::fs::create_dir_all(parts)?;
    // The file comes last: a part without it is not done
    let path = parts.join(format!("{name}.json"));
    let temporary = parts.join(format!("{name}.json.part"));
    std::fs::write(&temporary, serde_json::to_vec(&file)?)?;
    std::fs::rename(temporary, path)?;
    Ok(())
}

/// The result of part `name`, when it was made with `fingerprint`.
#[must_use]
pub fn load_part(parts: &Path, name: &str, fingerprint: &str) -> Option<PartFile> {
    let bytes = std::fs::read(parts.join(format!("{name}.json"))).ok()?;
    let file: PartFile = serde_json::from_slice(&bytes).ok()?;
    (file.format == FORMAT && file.fingerprint == fingerprint && file.name == name).then_some(file)
}

// ---------------------------------------------------------------------------------------------
// Replays

/// Plays a replay and counts its frames.
///
/// # Errors
///
/// The replay cannot be read or played.
pub fn play_replay(env: &Env<'_>, name: &str, path: &Path) -> anyhow::Result<PartOutput> {
    let replay = read_replay(path, env.rom)?;
    let stem = path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    // The reference replays other than `battle` begin with its input: the intro, the title, the
    // new game's opening and the first map are not counted again
    let from = if SHARED_PREFIX_REPLAYS.contains(&stem.as_str()) { SHARED_PREFIX } else { 0 };
    let mut machine = replay.power_on(env.rom.to_vec())?;
    machine.set_line_log(true);
    let mut recorder = Recorder::new(env, name);
    while machine.frame_count() < replay.frames().min(env.limit.unwrap_or(u64::MAX)) {
        machine.run_frame_with(&mut |f| replay.input(f), &mut NullAudio)?;
        if machine.frame_count() <= from {
            recorder.skip(&machine);
        } else {
            recorder.feed(&machine, machine.frame_count());
        }
    }
    let notes = if from > 0 {
        vec![format!("the first {from} frames are the battle replay's: not counted")]
    } else {
        Vec::new()
    };
    Ok(recorder.finish(notes))
}

/// The reference replays that begin with the first [`SHARED_PREFIX`] frames of the `battle`
/// replay's input (`ssd_game::scripts`).
const SHARED_PREFIX_REPLAYS: [&str; 4] = ["wander", "menus", "landing", "story-1"];
const SHARED_PREFIX: u64 = 3690;

// ---------------------------------------------------------------------------------------------
// Forced battles

/// The sets of a scenario's crafted save, with the map and position every battle starts from.
fn scenario_sets(scenario: &Scenario) -> Vec<(String, String)> {
    scenario.sets.clone()
}

/// The state of the map of `scenario` once booted (shared by its episodes).
fn boot_state(env: &Env<'_>, scenario: &Scenario) -> anyhow::Result<(Vec<u8>, Arc<[u8]>)> {
    let sram = crafted_bytes(env.game, &env.base_save, &scenario_sets(scenario))?;
    let cached = env.boots.lock().expect("boots").get(&scenario.name).cloned();
    let state = match cached {
        Some(state) => state,
        None => {
            let session = Session::boot(env.rom, &sram)?;
            let state: Arc<[u8]> = Arc::from(session.machine.save_state());
            env.boots.lock().expect("boots").insert(scenario.name.clone(), Arc::clone(&state));
            state
        }
    };
    Ok((sram, state))
}

/// The level a party is played at again when its first battle ended before any command.
const WEAK_LEVEL: &str = "10";

/// Whether a field of a crafted save sets a character's level (`goku.level`).
fn is_level(field: &str) -> bool {
    field.rsplit_once('.').is_some_and(|(_, what)| what == "level")
}

/// How many rounds of commands a battle's note says it played.
fn rounds_of(note: &str) -> Option<u32> {
    let (before, _) = note.split_once(" rounds")?;
    before.rsplit(", ").next()?.parse().ok()
}

/// Plays one battle of a scenario and counts its frames. A party much stronger than the enemies
/// makes them run away before the first command: when that happens and the party has levels, the
/// battle is played again with the party at level [`WEAK_LEVEL`] and the better of the two is kept.
///
/// # Errors
///
/// The save or the battle fails.
pub fn play_episode(
    env: &Env<'_>,
    name: &str,
    scenario: &Scenario,
    episode: &Episode,
) -> anyhow::Result<PartOutput> {
    let first = play_battle(env, name, scenario, episode)?;
    let played = first.notes.first().and_then(|note| rounds_of(note));
    let leveled = scenario.sets.iter().any(|(field, _)| is_level(field));
    if played != Some(0) || !leveled {
        return Ok(first);
    }
    let sets = scenario
        .sets
        .iter()
        .map(|(field, value)| {
            let value = if is_level(field) { WEAK_LEVEL } else { value };
            (field.clone(), value.to_owned())
        })
        .collect();
    let weak = Scenario { name: format!("{}-weak", scenario.name), sets, ..scenario.clone() };
    let mut second = play_battle(env, name, &weak, episode)?;
    if second.notes.first().and_then(|note| rounds_of(note)).is_some_and(|rounds| rounds > 0) {
        second.notes.push(format!(
            "played again with the party at level {WEAK_LEVEL}: the first battle ended before a command"
        ));
        return Ok(second);
    }
    Ok(first)
}

/// Plays one battle of a scenario from its booted map and counts its frames.
fn play_battle(
    env: &Env<'_>,
    name: &str,
    scenario: &Scenario,
    episode: &Episode,
) -> anyhow::Result<PartOutput> {
    let (sram, state) = boot_state(env, scenario)?;
    let mut saves = ssd_oracle::MemorySaveStore::default();
    saves.0.insert("sav".to_owned(), sram);
    let mut machine = Machine::new(env.rom.to_vec(), Box::new(saves))?;
    machine.set_breakpoint(0x01_F692);
    machine.set_line_log(true);
    let mut session = Session { machine, group: None, force_encounters: false, zone: 1 };
    let mut recorder = Recorder::new(env, name);
    let mut frame = 0_u64;
    // The group the battle code set up (`encounter_group_index`), at its first frame
    let mut shown = None;
    let result = run_episode(&mut session, &state, scenario, episode, &mut |s, _t| {
        frame += 1;
        shown.get_or_insert_with(|| s.wram(0x0EA3));
        recorder.feed(&s.machine, frame);
        Ok(())
    })
    .with_context(|| name.to_owned())?;
    let mut notes = vec![result];
    if shown != Some(episode.group) {
        notes.push(format!("group {shown:?} was set up, not {}", episode.group));
    }
    Ok(recorder.finish(notes))
}

/// Whether `class` can be seen in a part of this kind (what `--class` runs).
#[must_use]
pub fn kind_shows(kind: &str, class: Class) -> bool {
    match kind {
        "group" => matches!(
            class,
            Class::Portraits
                | Class::Fighters
                | Class::Effects
                | Class::BattleBg
                | Class::Text
                | Class::CardsUi
                | Class::Pictures
        ),
        "map" => matches!(class, Class::Map | Class::FieldSprites | Class::Text),
        "walk" => matches!(class, Class::Map | Class::FieldSprites),
        "replay" => true,
        "menus" => {
            matches!(class, Class::Portraits | Class::Text | Class::CardsUi | Class::Pictures)
        }
        "title" => matches!(class, Class::Pictures | Class::Text | Class::CardsUi | Class::Map),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_crop_file_of_a_key_is_stable_and_distinct() {
        assert_eq!(crop_file("map|tile:a#1"), crop_file("map|tile:a#1"));
        assert_ne!(crop_file("map|tile:a#1"), crop_file("map|tile:a#2"));
        assert_eq!(crop_file("x").len(), 20);
        assert!(crop_file("x").ends_with("png"));
    }

    #[test]
    fn a_part_is_saved_and_loaded_only_with_the_fingerprint_it_was_made_with() {
        let dir = tempfile::tempdir().unwrap();
        let mut output = PartOutput::default();
        output.notes.push("a note".to_owned());
        output.totals.frames = 3;
        output.totals.items.insert(
            "map|tile:x#1".to_owned(),
            crate::tour_account::ItemTotals {
                class: Class::Map,
                id: "tile:x#1".to_owned(),
                pixels: 4,
                frames: 1,
                first: Where { part: "p".to_owned(), frame: 2 },
                bbox: (0, 0, 1, 1),
            },
        );
        output
            .crops
            .insert("map|tile:x#1".to_owned(), Crop { width: 2, height: 2, rgba: vec![9; 16] });
        save_part(dir.path(), "p", "fp1", &output, 1.5).unwrap();
        let loaded = load_part(dir.path(), "p", "fp1").unwrap();
        assert_eq!((loaded.totals.frames, loaded.notes.len(), loaded.seconds), (3, 1, 1.5));
        let file = &loaded.crops["map|tile:x#1"];
        assert!(dir.path().join(file).exists(), "{file}");
        assert!(load_part(dir.path(), "p", "fp2").is_none(), "another fingerprint");
        assert!(load_part(dir.path(), "q", "fp1").is_none(), "another part");
        // Saving again replaces the crops
        output.crops.clear();
        save_part(dir.path(), "p", "fp1", &output, 2.0).unwrap();
        assert!(!dir.path().join(file).exists());
        assert!(load_part(dir.path(), "p", "fp1").unwrap().crops.is_empty());
    }

    #[test]
    fn the_rounds_a_battle_played_are_read_from_its_note() {
        assert_eq!(rounds_of("plan used up, 3 rounds, battle entered after 1 frames"), Some(3));
        assert_eq!(
            rounds_of("battle over at 457 (mode 0), 0 rounds, battle entered after 1 frames"),
            Some(0)
        );
        assert_eq!(rounds_of("something else"), None);
    }

    #[test]
    fn a_class_is_run_by_the_parts_that_can_show_it() {
        assert!(kind_shows("group", Class::Fighters) && !kind_shows("map", Class::Fighters));
        assert!(kind_shows("map", Class::Map) && !kind_shows("group", Class::Map));
        assert!(kind_shows("replay", Class::BattleBg));
        assert!(!kind_shows("other", Class::Map));
    }
}
