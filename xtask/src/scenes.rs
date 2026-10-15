//! `hd-art scenes`: the systematic audit of what the HD remaster draws without its art.
//!
//! It plays every frame of the parts of the coverage tour (the reference replays, a visit of every
//! map, the menus and the title; the battle groups are the tour's own), looks in each frame for a
//! visible map or background layer or a visible character sprite that is not drawn from the art
//! (original pixels, the filter's fallback, or art drawn wrongly dark or translucent), and groups
//! the frames into scenes: contiguous ranges of one map and one game state with the same faults.
//! The report is `index.html` (one representative 4x frame per scene, plain and with the art, the
//! replay and frame range, the map and what is missing) and `scenes.json`, in `.work/scenes/`;
//! the command fails while a scene is not listed as an exception in `tools/hd-scenes.toml`.
//! Everything it writes is derived from the game and never tracked.

use crate::tour_account::{AssetTable, Class, Context, classify_count, classify_item};
use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use ssd_hd::pack::Pack;
use ssd_hd::world::{Scene as Drawn, WideRun};
use ssd_hd::{ArtStats, FrameTally, HdOptions, ItemKey, Look};
use ssd_oracle::Machine;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// What is wrong with a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    /// A map layer (the field's tiles, a flight's plane) not drawn from the art of the maps.
    Map,
    /// A big picture (a cutscene, the title, an ending) not drawn from art.
    Picture,
    /// A battle background not drawn from art.
    Background,
    /// A character sprite (the party, people, objects, fighters, effects) not drawn from art.
    Sprite,
    /// A portrait or a face not drawn from art.
    Portrait,
    /// A sprite drawn from art that is much darker than the original (a silhouette).
    Dark,
    /// A sprite whose art covers less than half of its pixels (translucent, erased or misplaced).
    Thin,
    /// A sprite whose pixels did not change from one frame to the next but whose art did.
    Flicker,
    /// A field sprite whose art differs from the original's colour regions (off model).
    OffModel,
    /// A picture (a window with black beside it) with something else than black beside the window.
    Margin,
}

impl Reason {
    pub(crate) fn bit(self) -> u32 {
        1 << (self as u32)
    }

    /// The name of the reason in reports and exceptions.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Map => "map",
            Self::Picture => "picture",
            Self::Background => "background",
            Self::Sprite => "sprite",
            Self::Portrait => "portrait",
            Self::Dark => "dark",
            Self::Thin => "thin",
            Self::Flicker => "flicker",
            Self::OffModel => "off_model",
            Self::Margin => "margin",
        }
    }

    /// The reason a class of pixels not drawn from art is a fault, in a frame of `ctx`. Outside a
    /// battle the 2-bit characters that are bound to no asset are classed as text (the dialogue's
    /// window and font), but a picture built from 2-bit layers (the Snake Way's road, sky and
    /// clouds) is classed the same way: its bare pixels are a picture's.
    fn of_class_in(class: Class, ctx: Context) -> Option<Self> {
        match class {
            Class::Text if !ctx.battle => Some(Self::Picture),
            _ => Self::of_class(class),
        }
    }

    fn of_class(class: Class) -> Option<Self> {
        match class {
            Class::Map => Some(Self::Map),
            Class::Pictures => Some(Self::Picture),
            Class::BattleBg => Some(Self::Background),
            Class::FieldSprites | Class::Fighters | Class::Effects => Some(Self::Sprite),
            Class::Portraits => Some(Self::Portrait),
            Class::Text | Class::CardsUi => None,
        }
    }
}

/// The fewest pixels not drawn from art of a layer (a map, a picture, a background) that make a
/// frame a fault, and the share of the layer's pixels the art must draw.
const LAYER_MIN: u64 = 600;
const LAYER_SHARE: u64 = 97;
/// The fewest pixels of sprites not drawn from art that make a frame a fault.
const SPRITE_MIN: u64 = 10;
/// The first line of the overscan: what lies below the 224 lines the game draws its picture in.
const OVERSCAN: u16 = 224;

/// The fewest visible pixels of a pose the colour checks look at.
const POSE_MIN: u32 = 60;

/// What is wrong with one frame: the reasons, and the things that are (with their pixels).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Verdict {
    pub reasons: u32,
    pub items: BTreeMap<String, u64>,
}

impl Verdict {
    pub(crate) fn add(&mut self, reason: Reason, item: String, pixels: u64) {
        self.reasons |= reason.bit();
        *self.items.entry(format!("{}:{item}", reason.name())).or_default() += pixels;
    }

    /// Whether the frame has any fault.
    #[must_use]
    pub fn is_fault(&self) -> bool {
        self.reasons != 0
    }
}

/// The reasons of a frame by the tally of its pixels and the art's own stats.
#[must_use]
pub fn evaluate(
    tally: &FrameTally,
    stats: &ArtStats,
    ctx: Context,
    table: &AssetTable,
    scale: u32,
    art_id: &dyn Fn(u32) -> String,
) -> Verdict {
    let mut verdict = Verdict::default();
    // Pixels by class: all and the ones not drawn from art
    let mut totals: BTreeMap<Class, (u64, u64)> = BTreeMap::new();
    for count in &tally.counts {
        let class = classify_count(count, table, ctx);
        let entry = totals.entry(class).or_default();
        entry.0 += count.pixels;
        if !count.art {
            entry.1 += count.pixels;
        }
    }
    let mut failing = Vec::new();
    for (&class, &(all, bare)) in &totals {
        let Some(reason) = Reason::of_class_in(class, ctx) else { continue };
        let min = match reason {
            Reason::Sprite | Reason::Portrait => SPRITE_MIN,
            _ => LAYER_MIN,
        };
        let layer = matches!(reason, Reason::Map | Reason::Picture | Reason::Background);
        if bare >= min && (!layer || (all - bare) * 100 < all * LAYER_SHARE) {
            failing.push(class);
        }
    }
    for item in &tally.items {
        let (class, id) = classify_item(&item.key, table, ctx);
        if failing.contains(&class)
            && let Some(reason) = Reason::of_class_in(class, ctx)
        {
            // Pieces of sprites that no pose holds: those that start in the lines below the 224 the
            // game was made for are told apart
            let id = if matches!(
                item.key,
                ItemKey::Tile { object: true, .. } | ItemKey::Char { object: true, .. }
            ) && item.bbox.1 >= OVERSCAN
            {
                format!("{id} (overscan)")
            } else {
                id
            };
            verdict.add(reason, id, item.pixels);
        }
    }
    // A failing class whose pixels name no item still counts
    for class in failing {
        if let Some(reason) = Reason::of_class_in(class, ctx)
            && verdict.reasons & reason.bit() == 0
        {
            verdict.add(reason, "unnamed".to_owned(), totals[&class].1);
        }
    }
    // Art that is drawn but wrong: dark or thin
    let per_pixel = 255 * u64::from(scale) * u64::from(scale);
    for drawn in stats.drawn.iter().filter(|d| !d.luma && d.drawn.visible >= POSE_MIN) {
        let d = &drawn.drawn;
        let shown: u64 = d.shown.iter().sum();
        let original: u64 = d.original.iter().sum();
        let name = format!("{:016x}", drawn.key);
        let character = stats
            .poses
            .iter()
            .find(|p| p.key == drawn.key && p.art.is_some())
            .and_then(|p| p.art)
            .is_some_and(|at| is_character_art(&art_id(at)));
        if drawn.field
            && !ctx.battle
            && character
            && let Some(what) = off_model(d)
        {
            verdict.add(Reason::OffModel, format!("{name}: {what}"), u64::from(d.visible));
        }
        if d.weight * 100 < u64::from(d.visible) * per_pixel * 50 {
            verdict.add(Reason::Thin, name.clone(), u64::from(d.visible));
        }
        // Mean channel of the original above 40 (not a dark part) and the art under 45 percent of it
        if original > u64::from(d.visible) * 3 * 40 * u64::from(scale * scale) * 255 / 255
            && shown * 100 < original * 45
        {
            verdict.add(Reason::Dark, name, u64::from(d.visible));
        }
    }
    verdict
}

/// The units of the field's art that are characters (the on-model check reads their costumes): the
/// ids are `field-<unit>-<number>`. Portraits, buildings, objects, effects and the soldiers whose
/// palettes are variants of one drawing are not looked at.
const CHARACTER_UNITS: [&str; 9] =
    ["goku", "gohan", "piccolo", "krillin", "vegeta", "tien-chiaotzu", "roshi", "bulma", "baba"];

fn is_character_art(id: &str) -> bool {
    id.strip_prefix("field-").and_then(|rest| rest.rsplit_once('-')).is_some_and(
        |(unit, number)| {
            number.bytes().all(|b| b.is_ascii_digit()) && CHARACTER_UNITS.contains(&unit)
        },
    )
}

/// How a pose's art differs from the original's colour regions, when it does: a colour the
/// original has a good share of that the art lacks (the orange gi that became a bare chest), or
/// skin the art has far more of than the original. Shares are of the pose's drawn pixels.
fn off_model(d: &ssd_hd::Drawn) -> Option<String> {
    const NAMES: [&str; ssd_hd::COLOUR_CLASSES] = [
        "dark", "white", "grey", "red", "orange", "skin", "brown", "yellow", "green", "blue",
        "purple",
    ];
    let all = |c: &[u64]| c.iter().sum::<u64>().max(1) as f64;
    let (so, ss) = (all(&d.classes_original), all(&d.classes_shown));
    let share =
        |c: &[u64], at: &[usize], total: f64| at.iter().map(|&i| c[i]).sum::<u64>() as f64 / total;
    let skin = share(&d.classes_shown, &[5], ss) - share(&d.classes_original, &[5], so);
    if skin > SKIN_GAIN {
        return Some(format!("skin +{:.0}%", skin * 100.0));
    }
    // The warm colours (orange and brown are one: shading moves a gi from one to the other)
    let groups: [(&str, &[usize]); 8] = [
        ("red", &[3]),
        ("warm", &[4, 6]),
        ("yellow", &[7]),
        ("green", &[8]),
        ("blue", &[9]),
        ("purple", &[10]),
        ("white", &[1]),
        ("skin", &[5]),
    ];
    for (name, at) in groups {
        let (o, s) = (share(&d.classes_original, at, so), share(&d.classes_shown, at, ss));
        if o >= COLOUR_HAD && s < o * COLOUR_KEPT {
            return Some(format!("{name} {:.0}% -> {:.0}%", o * 100.0, s * 100.0));
        }
    }
    let _ = NAMES;
    None
}

/// The share of skin the art may gain over the original's, the share of a colour that makes it
/// one the original has, and the part of it the art must keep.
const SKIN_GAIN: f64 = 0.12;
const COLOUR_HAD: f64 = 0.18;
const COLOUR_KEPT: f64 = 0.35;

/// `SCENES_SNAP=N` writes the pictures of every Nth frame (to look at what a part shows).
fn snap_every() -> Option<u64> {
    std::env::var("SCENES_SNAP").ok()?.parse().ok().filter(|&n| n > 0)
}

/// `SCENES_TRACE=A-B` prints what the frames A to B are made of, to look at a scene.
fn trace_range() -> Option<(u64, u64)> {
    let text = std::env::var("SCENES_TRACE").ok()?;
    let (a, b) = text.split_once('-')?;
    Some((a.parse().ok()?, b.parse().ok()?))
}

#[allow(clippy::too_many_arguments)]
fn trace(
    part: &str,
    frame: u64,
    tally: &FrameTally,
    stats: &ArtStats,
    ctx: Context,
    table: &AssetTable,
    scene: Drawn,
    wram: &[u8],
) {
    let mut classes: BTreeMap<&str, (u64, u64)> = BTreeMap::new();
    for count in &tally.counts {
        let class = classify_count(count, table, ctx);
        let entry = classes.entry(class.key()).or_default();
        entry.0 += count.pixels;
        if count.art {
            entry.1 += count.pixels;
        }
    }
    eprintln!(
        "{part} f{frame} {scene:?} mode {:02X} map {} classes(art/all) {classes:?}",
        wram[0x24], wram[0x1A0]
    );
    for pose in &stats.poses {
        eprintln!(
            "    pal {} {}{} key {:016x} org {:?} size {:?} vis {} opaque {} matched {} {:?} art {:?} recol {}",
            pose.palette,
            if pose.field { "field " } else { "" },
            if pose.fighter { "fighter" } else { "" },
            pose.key,
            pose.org,
            pose.size,
            pose.visible,
            pose.opaque,
            pose.matched,
            pose.source,
            pose.art,
            pose.recoloured
        );
    }
    for item in &tally.items {
        let (class, id) = classify_item(&item.key, table, ctx);
        eprintln!("    item {} {id} {} px", class.key(), item.pixels);
    }
}

/// What a scene is made of, written to `scenes.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneRange {
    pub part: String,
    pub first: u64,
    pub last: u64,
    /// Frames of the range with a fault.
    pub faulty: u64,
    pub map: u8,
    pub mode: u8,
    pub drawn_as: String,
    pub reasons: Vec<String>,
    /// What is missing, with the pixels it took over the range, the largest first.
    pub items: Vec<(String, u64)>,
    /// The frame of the pictures, and their files relative to the report.
    pub shown: u64,
    pub plain: String,
    pub art: String,
    /// Why the scene is an accepted exception, when it is.
    pub exception: Option<String>,
}

/// Where the scenes of every part are collected.
pub struct Sink {
    pub dir: PathBuf,
    scenes: Mutex<Vec<SceneRange>>,
}

impl Sink {
    #[must_use]
    pub fn new(dir: PathBuf) -> Self {
        Self { dir, scenes: Mutex::new(Vec::new()) }
    }

    /// The scenes found so far.
    #[must_use]
    pub fn scenes(&self) -> Vec<SceneRange> {
        self.scenes.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

/// Frames of the parts that are looked at: `BURST` consecutive frames of every `STRIDE` (the
/// flicker check needs consecutive frames).
const STRIDE: u64 = 8;
const BURST: u64 = 3;

/// The characters whose walk cycles the gallery parts show.
const WALKERS: &[&str] =
    &["goku", "gohan", "krillin", "piccolo", "vegeta", "yamcha", "tien", "chiaotzu"];

/// The pixels beside the window of a picture that are not black (or, with `only`, that are of this
/// colour), and how many make a fault.
fn margin_pixels(picture: &ssd_hd::HdFrame, only: Option<[u8; 3]>) -> u64 {
    let (width, height) = (picture.width as usize, picture.height as usize);
    let (left, right) = (
        picture.logical.x as usize,
        ((picture.logical.x + picture.logical.width) as usize).min(width),
    );
    let mut lost = 0_u64;
    for y in 0..height {
        let row = &picture.rgba[y * width * 4..(y + 1) * width * 4];
        for x in (0..left).chain(right..width) {
            let rgb = [row[x * 4], row[x * 4 + 1], row[x * 4 + 2]];
            // (black is no fault: only what is not black, or, with `only`, that colour)
            if rgb != [0, 0, 0] && only.is_none_or(|colour| colour == rgb) {
                lost += 1;
            }
        }
    }
    lost
}

const MARGIN_MIN: u64 = 64;

/// How many things of each reason a scene lists.
const ITEMS_PER_REASON: usize = 80;

/// Frames without a fault after which a scene is over.
const QUIET: u64 = 90;

struct Open {
    range: SceneRange,
    key: (u8, u8, u32),
    last_fault: u64,
    reasons: u32,
    items: BTreeMap<String, u64>,
}

/// Watches the frames of one part.
pub struct Watcher {
    sink: Arc<Sink>,
    part: String,
    pack: Arc<Pack>,
    /// The run that draws the art at the picture's scale 1 (to measure it) and the two that draw
    /// the representative 4x pictures.
    measure: WideRun,
    plain: WideRun,
    art: WideRun,
    open: Option<Open>,
    /// Whether frames of a field state are drawn as the field whatever BG2 holds (a part that moves
    /// the camera by writing the machine's variables).
    trust: bool,
    flicker: crate::scene_flicker::Tracker,
}

fn fresh_run(rom: &[u8], pack: Option<&Arc<Pack>>, font: Option<Arc<ssd_hd::Font>>) -> WideRun {
    let mut run = WideRun::new(rom);
    run.set_art(pack.cloned());
    run.set_font(font);
    run
}

impl Watcher {
    #[must_use]
    pub fn new(
        sink: Arc<Sink>,
        part: &str,
        rom: &[u8],
        pack: &Arc<Pack>,
        font: Option<&Arc<ssd_hd::Font>>,
    ) -> Self {
        let mut measure = fresh_run(rom, Some(pack), None);
        measure.set_art_measure(true);
        Self {
            sink,
            part: part.to_owned(),
            pack: Arc::clone(pack),
            measure,
            plain: fresh_run(rom, None, font.cloned()),
            art: fresh_run(rom, Some(pack), font.cloned()),
            open: None,
            trust: false,
            flicker: crate::scene_flicker::Tracker::default(),
        }
    }

    /// Frames of a field state are drawn as the field without checking BG2 (see
    /// `Recorder::trust_field`).
    pub fn set_trust(&mut self, trust: bool) {
        self.trust = trust;
        for run in [&mut self.measure, &mut self.plain, &mut self.art] {
            run.set_trust_field(trust);
        }
    }

    /// The machine jumped: the next frame does not follow the last.
    pub fn reset(&mut self, rom: &[u8], font: Option<&Arc<ssd_hd::Font>>) {
        self.measure = fresh_run(rom, Some(&self.pack), None);
        self.measure.set_art_measure(true);
        self.plain = fresh_run(rom, None, font.cloned());
        self.art = fresh_run(rom, Some(&self.pack), font.cloned());
        self.set_trust(self.trust);
        self.flicker.clear();
    }

    /// A frame ran and is not counted.
    pub fn skip(&mut self, machine: &Machine) {
        self.measure.skip(machine.wram());
        self.plain.skip(machine.wram());
        self.art.skip(machine.wram());
        self.flicker.clear();
    }

    /// A frame ran and is counted: `tally` and `scene` are the tour run's.
    pub fn observe(
        &mut self,
        machine: &Machine,
        frame: u64,
        tally: &FrameTally,
        scene: Drawn,
        table: &AssetTable,
    ) {
        let wram = machine.wram();
        let ctx = Context::of_wram(wram);
        let measured = self.measure.note_hd(
            wram,
            machine.frame_log(),
            HdOptions { scale: 1, look: Look::Crisp },
        );
        let stats = self.measure.art_stats();
        // A frame faded to black, or in the mosaic of a fade (a map loading, a battle starting),
        // shows nothing to cover
        let transition = machine.frame_log().is_none_or(|log| {
            log.entries.iter().any(|entry| {
                let regs = &entry.regs;
                !regs.forced_blanking
                    && (regs.brightness < 15
                        || (regs.mosaic_size > 0 && regs.mosaic_bg.iter().any(|&on| on)))
            })
        });
        let dark = transition
            || measured.as_ref().is_none_or(|m| {
                let rgba = &m.picture.rgba;
                let sum: u64 = rgba.iter().step_by(64).map(|&c| u64::from(c)).sum();
                sum < rgba.len() as u64 / 64 * 3 / 2
            });
        let mut verdict = if dark {
            Verdict::default()
        } else {
            evaluate(tally, &stats, ctx, table, 1, &|at| {
                self.pack
                    .art
                    .objects()
                    .get(at as usize)
                    .map_or_else(String::new, |object| object.id.clone())
            })
        };
        if verdict.reasons & Reason::Map.bit() != 0
            && let Some(state) = ssd_hd::world::FieldState::from_wram(wram)
        {
            let (grid, metatiles) = state.map.layout();
            let (w, h) = state.map.blocks;
            verdict.add(
                Reason::Map,
                format!(
                    "layout {w}x{h} {:02X}:{:04X} {:02X}:{:04X} ({} the field)",
                    grid.0,
                    grid.1,
                    metatiles.0,
                    metatiles.1,
                    if state.is_field() { "state is" } else { "state is not" }
                ),
                1,
            );
        }
        if let Some((from, to)) = trace_range()
            && (from..=to).contains(&frame)
        {
            trace(&self.part, frame, tally, &stats, ctx, table, scene, wram);
        }
        if let Some(m) = measured.as_ref().filter(|_| !transition) {
            match m.scene {
                Drawn::Bordered => {
                    let lost = margin_pixels(&m.picture, None);
                    if lost >= MARGIN_MIN {
                        verdict.add(Reason::Margin, "not black beside the window".to_owned(), lost);
                    }
                }
                // Beside a flight's window the plane goes on; the backdrop does not
                Drawn::Flight => {
                    if let Some(log) = machine.frame_log() {
                        let c = log.cgram_start[0];
                        let channel =
                            |shift: u32| (((u32::from(c >> shift) & 31) * 255 + 15) / 31) as u8;
                        let backdrop = [channel(0), channel(5), channel(10)];
                        let lost = margin_pixels(&m.picture, Some(backdrop));
                        if lost >= MARGIN_MIN {
                            verdict.add(
                                Reason::Margin,
                                "the backdrop beside a flight's window".to_owned(),
                                lost,
                            );
                        }
                    }
                }
                Drawn::Field => {}
            }
        }
        // (the field groups are checked; the effects are what can cover them)
        self.flicker.check(&stats.poses, &mut verdict);

        let key = (wram[0x1A0], wram[0x24] & 0x7F, verdict.reasons);
        let continues = self.open.as_ref().is_some_and(|open| {
            frame <= open.last_fault + QUIET && (!verdict.is_fault() || open.key.0 == key.0)
        });
        if !continues {
            self.close();
        }
        if !verdict.is_fault() {
            // The runs of the pictures follow the frame
            if let Some(every) = snap_every()
                && frame.is_multiple_of(every)
            {
                let _ = self.pictures(machine, frame);
            } else {
                self.plain.note(wram, machine.frame_log());
                self.art.note(wram, machine.frame_log());
            }
            return;
        }
        if self.open.is_none() {
            let (plain, art) = self.pictures(machine, frame);
            self.open = Some(Open {
                range: SceneRange {
                    part: self.part.clone(),
                    first: frame,
                    last: frame,
                    faulty: 0,
                    map: key.0,
                    mode: key.1,
                    drawn_as: format!("{scene:?}"),
                    reasons: Vec::new(),
                    items: Vec::new(),
                    shown: frame,
                    plain,
                    art,
                    exception: None,
                },
                key,
                last_fault: frame,
                reasons: 0,
                items: BTreeMap::new(),
            });
        } else {
            self.plain.note(wram, machine.frame_log());
            self.art.note(wram, machine.frame_log());
        }
        if let Some(open) = &mut self.open {
            open.range.last = frame;
            open.range.faulty += 1;
            open.last_fault = frame;
            open.reasons |= verdict.reasons;
            for (item, pixels) in verdict.items {
                *open.items.entry(item).or_default() += pixels;
            }
        }
    }

    /// The 4x pictures of the frame, plain and with the art, written to the sink's folder.
    fn pictures(&mut self, machine: &Machine, frame: u64) -> (String, String) {
        let options = HdOptions { scale: 4, look: Look::Crisp };
        let folder = self.sink.dir.join("img");
        let _ = std::fs::create_dir_all(&folder);
        let write = |run: &mut WideRun, kind: &str| -> String {
            let name = format!("{}-{frame:06}-{kind}.png", self.part);
            if let Some(picture) = run.note_hd(machine.wram(), machine.frame_log(), options) {
                let hd = &picture.picture;
                let _ = ssd_hd::png::write_rgba(&folder.join(&name), hd.width, hd.height, &hd.rgba);
                format!("img/{name}")
            } else {
                String::new()
            }
        };
        let plain = write(&mut self.plain, "plain");
        let art = write(&mut self.art, "art");
        (plain, art)
    }

    fn close(&mut self) {
        let Some(open) = self.open.take() else { return };
        let mut range = open.range;
        for reason in ALL_REASONS {
            if open.reasons & reason.bit() != 0 {
                range.reasons.push(reason.name().to_owned());
            }
        }
        // The largest few things of each reason
        let mut items: Vec<(String, u64)> = open.items.into_iter().collect();
        items.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let mut per: BTreeMap<String, usize> = BTreeMap::new();
        items.retain(|(name, _)| {
            let count = per.entry(name.split(':').next().unwrap_or("").to_owned()).or_default();
            *count += 1;
            *count <= ITEMS_PER_REASON
        });
        range.items = items;
        if let Ok(mut scenes) = self.sink.scenes.lock() {
            scenes.push(range);
        }
    }

    /// The part is over.
    pub fn finish(&mut self) {
        self.close();
    }
}

const ALL_REASONS: [Reason; 10] = [
    Reason::Map,
    Reason::Picture,
    Reason::Background,
    Reason::Sprite,
    Reason::Portrait,
    Reason::Dark,
    Reason::Thin,
    Reason::Flicker,
    Reason::OffModel,
    Reason::Margin,
];

/// `tools/hd-scenes.toml`: the scenes that are accepted as they are, each with its reason.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Exceptions {
    #[serde(default)]
    pub exception: Vec<Exception>,
}

/// An accepted scene: every field given must match (a substring for the part and the item).
#[derive(Debug, Clone, Deserialize)]
pub struct Exception {
    pub part: Option<String>,
    pub map: Option<u8>,
    /// The game modes (`$24` of work RAM, without bit 7) the scene is in.
    pub modes: Option<Vec<u8>>,
    pub reason: Option<String>,
    pub item: Option<String>,
    pub note: String,
}

impl Exceptions {
    /// Reads the file; none is no exception.
    ///
    /// # Errors
    ///
    /// The file is not valid.
    pub fn read(path: &Path) -> anyhow::Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).with_context(|| path.display().to_string()),
            Err(_) => Ok(Self::default()),
        }
    }

    /// The note of the exception that covers `scene`, if one does: it covers a scene when its
    /// reason is one the scene has and, when it names an item, an item the scene has.
    #[must_use]
    pub fn covers(&self, scene: &SceneRange) -> Option<&str> {
        self.exception
            .iter()
            .find(|e| {
                e.part.as_ref().is_none_or(|p| scene.part.contains(p.as_str()))
                    && e.map.is_none_or(|m| m == scene.map)
                    && e.modes.as_ref().is_none_or(|modes| modes.contains(&scene.mode))
                    && e.reason.as_ref().is_none_or(|r| scene.reasons.contains(r))
                    && e.item
                        .as_ref()
                        .is_none_or(|i| scene.items.iter().any(|(n, _)| n.contains(i.as_str())))
            })
            .map(|e| e.note.as_str())
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// The report page: one row per scene, the pictures side by side.
#[must_use]
pub fn html(scenes: &[SceneRange], parts: usize, frames: u64) -> String {
    let open = scenes.iter().filter(|s| s.exception.is_none()).count();
    let mut page = String::new();
    let _ = write!(
        page,
        "<!doctype html><meta charset=utf-8><title>HD scenes</title>\
         <style>body{{font:14px system-ui;background:#16161a;color:#ddd;margin:16px}}\
         .s{{border-top:1px solid #444;padding:10px 0}}img{{width:49%;image-rendering:auto}}\
         code{{color:#9cf}}.x{{opacity:.55}}h1{{font-size:20px}}</style>\
         <h1>HD scenes: {open} uncovered of {} scenes</h1>\
         <p>{parts} parts, {frames} frames played. Left: plain, right: with the HD art.</p>",
        scenes.len()
    );
    for (n, scene) in scenes.iter().enumerate() {
        let class = if scene.exception.is_some() { "s x" } else { "s" };
        let _ = write!(
            page,
            "<div class='{class}'><b>#{n} {}</b> frames {}-{} ({} with a fault), map {}, mode {:02X}, \
             drawn as {}, reasons <code>{}</code>",
            escape(&scene.part),
            scene.first,
            scene.last,
            scene.faulty,
            scene.map,
            scene.mode,
            scene.drawn_as,
            scene.reasons.join(", ")
        );
        if let Some(note) = &scene.exception {
            let _ = write!(page, " <i>exception: {}</i>", escape(note));
        }
        page.push_str("<br><small>");
        for (item, pixels) in scene.items.iter().take(8) {
            let _ = write!(page, "<code>{}</code> {pixels} px; ", escape(item));
        }
        let _ = write!(
            page,
            "</small><br><img src='{}'> <img src='{}'><br>frame {}</div>",
            scene.plain, scene.art, scene.shown
        );
    }
    page
}

/// Writes the report and says what it found; returns whether every scene is covered.
///
/// # Errors
///
/// The files cannot be written.
pub fn write_report(
    dir: &Path,
    scenes: &mut [SceneRange],
    exceptions: &Exceptions,
    parts: usize,
    frames: u64,
    out: &mut dyn std::io::Write,
) -> anyhow::Result<bool> {
    scenes.sort_by(|a, b| a.part.cmp(&b.part).then(a.first.cmp(&b.first)));
    for scene in scenes.iter_mut() {
        scene.exception = exceptions.covers(scene).map(str::to_owned);
    }
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join("index.html"), html(scenes, parts, frames))?;
    std::fs::write(dir.join("scenes.json"), serde_json::to_vec_pretty(&*scenes)?)?;
    std::fs::write(
        dir.join("run.json"),
        serde_json::to_vec(&serde_json::json!({ "parts": parts, "frames": frames }))?,
    )?;
    let open: Vec<&SceneRange> = scenes.iter().filter(|s| s.exception.is_none()).collect();
    let mut by_reason: BTreeMap<&str, (u32, u32)> = BTreeMap::new();
    for scene in scenes.iter() {
        for reason in &scene.reasons {
            let entry = by_reason.entry(reason.as_str()).or_default();
            entry.0 += 1;
            entry.1 += u32::from(scene.exception.is_none());
        }
    }
    writeln!(
        out,
        "hd-scenes: {parts} parts, {frames} frames: {} scenes with a fault, {} uncovered, {} excepted",
        scenes.len(),
        open.len(),
        scenes.len() - open.len()
    )?;
    for (reason, (all, left)) in by_reason {
        writeln!(out, "  {reason:<12} {all:4} scenes, {left:4} uncovered")?;
    }
    for scene in open.iter().take(40) {
        let item = scene.items.first().map_or("", |(name, _)| name.as_str());
        writeln!(
            out,
            "  {} f{}-{} map {} [{}] {item}",
            scene.part,
            scene.first,
            scene.last,
            scene.map,
            scene.reasons.join(",")
        )?;
    }
    writeln!(out, "hd-scenes: report in {}", dir.join("index.html").display())?;
    Ok(open.is_empty())
}

/// Plays the parts `args` choose (every part kind but the battle groups by default) and reports
/// the scenes.
///
/// # Errors
///
/// The pack, the base save or a replay cannot be read, or the report cannot be written.
pub fn run(
    rom: &[u8],
    pack_dir: &Path,
    root: &Path,
    args: &crate::tour::Args,
    out: &mut (dyn std::io::Write + Send),
) -> anyhow::Result<u8> {
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    let mut args = args.clone();
    if args.maps.is_none()
        && args.replays.is_none()
        && !args.menus
        && !args.title
        && args.groups.is_none()
    {
        args.maps = Some(Vec::new());
        args.replays = Some(Vec::new());
        args.menus = true;
        args.title = true;
    }
    let dir = args.out.clone().unwrap_or_else(|| root.join(".work").join("scenes"));
    let pack =
        Arc::new(Pack::open(pack_dir).with_context(|| format!("the pack {}", pack_dir.display()))?);
    let replay_dir = root.join(".work").join("replays");
    let replays = if replay_dir.is_dir() {
        crate::verify_replays::replay_paths(&replay_dir)?
    } else {
        Vec::new()
    };
    let mut jobs = crate::tour::plan_jobs(&args, &[], &replays);
    if args.maps.is_some() {
        for who in WALKERS {
            jobs.push(crate::tour::Job {
                name: format!("walk-{who}"),
                kind: "walk",
                params: String::new(),
                what: crate::tour::Kind::Walk((*who).to_owned()),
            });
        }
    }
    anyhow::ensure!(!jobs.is_empty(), "hd-art scenes: nothing to visit");
    let base_path = match &args.base {
        Some(path) => path.clone(),
        None => crate::tour::find_base_save(root)?,
    };
    let base_save = std::fs::read(&base_path)
        .with_context(|| format!("reading the base save {}", base_path.display()))?;
    let game = crate::save::Game::from_rom(rom)?;
    let sink = Arc::new(Sink::new(dir.clone()));
    let full = std::env::var_os("SCENES_FULL").is_some();
    let mut env = crate::tour_run::Env::new(
        rom,
        Arc::clone(&pack),
        &game,
        base_save,
        if full { 1 } else { STRIDE },
    );
    env.burst = if full { 1 } else { BURST };
    env.limit = std::env::var("SCENES_UNTIL").ok().and_then(|v| v.parse().ok());
    env.gaps = Some(dir.join("gaps"));
    env.scenes = Some(Arc::clone(&sink));
    env.font = Some(Arc::new(ssd_hd::Font::standard(rom).context("the HD text font")?));
    writeln!(out, "hd-scenes: {} parts ({} jobs)", jobs.len(), args.jobs)?;
    let next = AtomicUsize::new(0);
    let frames = AtomicU64::new(0);
    let log = Mutex::new(&mut *out);
    let started = std::time::Instant::now();
    std::thread::scope(|scope| {
        for _ in 0..args.jobs.max(1).min(jobs.len()) {
            scope.spawn(|| {
                loop {
                    let n = next.fetch_add(1, Ordering::Relaxed);
                    let Some(job) = jobs.get(n) else { break };
                    let played = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        crate::tour::play_job(&env, job)
                    }))
                    .unwrap_or_else(|_| Err(anyhow::anyhow!("the part panicked")));
                    let line = match played {
                        Ok(output) => {
                            frames.fetch_add(output.totals.frames, Ordering::Relaxed);
                            format!(
                                "[{}/{}] {} {} frames ({:.0} s)",
                                n + 1,
                                jobs.len(),
                                job.name,
                                output.totals.frames,
                                started.elapsed().as_secs_f64()
                            )
                        }
                        Err(error) => {
                            format!("[{}/{}] {} FAILED: {error:#}", n + 1, jobs.len(), job.name)
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
    let mut scenes = sink.scenes();
    let exceptions = Exceptions::read(&root.join("tools").join("hd-scenes.toml"))?;
    let clean = write_report(
        &dir,
        &mut scenes,
        &exceptions,
        jobs.len(),
        frames.load(Ordering::Relaxed),
        out,
    )?;
    Ok(u8::from(!clean))
}

/// Puts the reports of several runs (`hd-art scenes --out`, one folder each, below `out`) into one:
/// the scenes of every folder with the exceptions of `tools/hd-scenes.toml` applied, the pictures
/// read from the folders they were written to, `index.html` and `scenes.json` in `out`.
///
/// # Errors
///
/// A folder has no report, or the files cannot be written.
pub fn merge(
    root: &Path,
    out_dir: &Path,
    dirs: &[PathBuf],
    out: &mut dyn std::io::Write,
) -> anyhow::Result<u8> {
    let (mut scenes, mut parts, mut frames) = (Vec::new(), 0_usize, 0_u64);
    for dir in dirs {
        let text = std::fs::read(dir.join("scenes.json"))
            .with_context(|| format!("{}: no report", dir.display()))?;
        let mut found: Vec<SceneRange> = serde_json::from_slice(&text)?;
        let name = dir.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        for scene in &mut found {
            for picture in [&mut scene.plain, &mut scene.art] {
                if !picture.is_empty() {
                    *picture = format!("{name}/{picture}");
                }
            }
        }
        scenes.append(&mut found);
        if let Ok(run) = std::fs::read(dir.join("run.json"))
            && let Ok(run) = serde_json::from_slice::<serde_json::Value>(&run)
        {
            parts += run["parts"].as_u64().unwrap_or(0) as usize;
            frames += run["frames"].as_u64().unwrap_or(0);
        }
    }
    let exceptions = Exceptions::read(&root.join("tools").join("hd-scenes.toml"))?;
    let clean = write_report(out_dir, &mut scenes, &exceptions, parts, frames, out)?;
    Ok(u8::from(!clean))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssd_hd::{ItemKey, TallyCount, TallyItem, TallyLayer};

    fn count(layer: TallyLayer, bpp: u8, art: bool, pixels: u64) -> TallyCount {
        TallyCount { layer, bpp, asset: None, tile: 0, pose: None, art, font: false, pixels }
    }

    fn tally(counts: Vec<TallyCount>, items: Vec<TallyItem>) -> FrameTally {
        FrameTally { counts, shapes: Vec::new(), items, uses: Vec::new() }
    }

    fn field() -> Context {
        Context::default()
    }

    fn evaluate_of(tally: &FrameTally, ctx: Context) -> Verdict {
        evaluate(tally, &ArtStats::default(), ctx, &AssetTable::of_ids(&[]), 1, &|_| String::new())
    }

    #[test]
    fn a_map_drawn_from_its_art_is_covered_and_one_that_is_not_is_a_fault() {
        let covered = tally(vec![count(TallyLayer::Bg, 4, true, 80_000)], Vec::new());
        assert!(!evaluate_of(&covered, field()).is_fault());
        let bare = tally(
            vec![count(TallyLayer::Bg, 4, true, 20_000), count(TallyLayer::Bg, 4, false, 60_000)],
            vec![TallyItem {
                key: ItemKey::Char { object: false, bpp: 4, key: 7 },
                pixels: 60_000,
                bbox: (0, 0, 255, 223),
            }],
        );
        let verdict = evaluate_of(&bare, field());
        assert_eq!(verdict.reasons, Reason::Map.bit());
        assert_eq!(verdict.items.len(), 1);
    }

    #[test]
    fn a_few_stray_map_pixels_are_not_a_scene() {
        let almost = tally(
            vec![count(TallyLayer::Bg, 4, true, 80_000), count(TallyLayer::Bg, 4, false, 300)],
            Vec::new(),
        );
        assert!(!evaluate_of(&almost, field()).is_fault());
    }

    #[test]
    fn sprites_without_art_are_a_fault_and_tiny_ones_are_not() {
        let missing = tally(vec![count(TallyLayer::Obj, 4, false, 120)], Vec::new());
        let verdict = evaluate_of(&missing, field());
        assert_eq!(verdict.reasons, Reason::Sprite.bit());
        assert_eq!(verdict.items["sprite:unnamed"], 120);
        let tiny = tally(vec![count(TallyLayer::Obj, 4, false, 6)], Vec::new());
        assert!(!evaluate_of(&tiny, field()).is_fault());
    }

    #[test]
    fn an_exception_covers_the_scenes_it_names_and_no_other() {
        let exceptions: Exceptions = toml::from_str(
            "[[exception]]\nreason = \"map\"\nitem = \"unnamed\"\npart = \"title\"\nnote = \"the intro\"\n",
        )
        .unwrap();
        let scene = |part: &str, reason: &str| SceneRange {
            part: part.to_owned(),
            first: 1,
            last: 2,
            faulty: 2,
            map: 0,
            mode: 7,
            drawn_as: "Bordered".to_owned(),
            reasons: vec![reason.to_owned()],
            items: vec![("map:unnamed".to_owned(), 5)],
            shown: 1,
            plain: String::new(),
            art: String::new(),
            exception: None,
        };
        assert_eq!(exceptions.covers(&scene("title", "map")), Some("the intro"));
        assert_eq!(exceptions.covers(&scene("replay-x", "map")), None);
        assert_eq!(exceptions.covers(&scene("title", "sprite")), None);
    }

    #[test]
    fn a_pose_that_changes_art_with_the_same_pixels_flickers() {
        let pose = |art: Option<u32>| ssd_hd::PoseStat {
            fighter: false,
            field: true,
            matched: art.is_some(),
            recoloured: false,
            visible: 300,
            opaque: 300,
            stripy: 0,
            key: 1,
            palette: 4,
            source: ssd_hd::ArtSource::Exact,
            art,
            org: (100, 80),
            size: (18, 32),
            look: 77,
        };
        let mut tracker = crate::scene_flicker::Tracker::default();
        let mut verdict = Verdict::default();
        tracker.check(&[pose(Some(5))], &mut verdict);
        tracker.check(&[pose(Some(5))], &mut verdict);
        assert!(!verdict.is_fault(), "the same art twice is steady");
        // One change of art (a palette's variant as a fade goes by) is not a flicker
        tracker.check(&[pose(Some(9))], &mut verdict);
        assert!(!verdict.is_fault(), "one change is not a flicker");
        tracker.check(&[pose(Some(9))], &mut verdict);
        tracker.check(&[pose(Some(5))], &mut verdict);
        tracker.check(&[pose(Some(9))], &mut verdict);
        assert_eq!(verdict.reasons, Reason::Flicker.bit(), "back and forth is");
        // Other pixels in the box are an animation, not a flicker
        let mut tracker = crate::scene_flicker::Tracker::default();
        let mut verdict = Verdict::default();
        for (art, look) in [(5, 77), (9, 78), (5, 77)] {
            let mut p = pose(Some(art));
            p.look = look;
            tracker.check(&[p], &mut verdict);
        }
        assert!(!verdict.is_fault());
    }
}
