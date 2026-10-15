//! `hd-maps`: the field maps as pictures, for the art of the HD remaster of the maps.
//!
//! * `dump` boots each map of the game (from a crafted save, as `hd-tour` does), waits for the
//!   party to be free, and writes the map's BG2 layer as one picture at 1x (`layer.png`: every
//!   8 x 8 cell of the map through the game's own lookup, in the tile set and the colours the
//!   map has), the cells' tile map words (`words.bin`) and what moves in it (`meta.json`: the
//!   tile numbers whose pixels change in VRAM, the palette entries that change). Output goes
//!   under `.work/maps/dump/` and is never tracked.
//! * `shots` draws a place of a map as the Enhanced picture (16:9, at a scale, with the art and without).
//! * `scan` plays replays and lists, per map id, the layouts (the pointers of the map's data) and
//!   tile sets the game shows, to see whether the content of a map changes with the game's state.

use crate::save::Game;
use crate::tour_battle::{Finisher, Session, crafted_bytes, is_battle};
use crate::tour_visit::map_sets;
use anyhow::Context;
use serde::Serialize;
use ssd_hd::world::{FieldState, Rom};
use ssd_oracle::{Joypad, NullAudio};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The number of maps of the game.
pub const MAPS: u8 = 48;

/// What `hd-maps` was asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Args {
    Dump {
        out: Option<PathBuf>,
        maps: Vec<u8>,
        frames: u32,
        jobs: usize,
        base: Option<PathBuf>,
    },
    Scan {
        paths: Vec<PathBuf>,
        every: u64,
    },
    Shots {
        out: Option<PathBuf>,
        pack: Option<PathBuf>,
        scale: u32,
        shots: Vec<Shot>,
        base: Option<PathBuf>,
    },
}

/// A place of a map to draw: the camera is the top left of the original's window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shot {
    pub label: String,
    pub map: u8,
    pub at: (u16, u16),
}

/// Parses the arguments after `hd-maps`.
///
/// # Errors
///
/// An unknown subcommand or option, or a bad number.
pub fn parse_args(args: &[&str]) -> Result<Args, String> {
    let mut iter = args.iter();
    let command = iter.next().copied().ok_or("a subcommand: dump or scan")?;
    let (mut out, mut maps, mut frames, mut jobs, mut base) = (None, Vec::new(), 600, 3, None);
    let mut every = 5_u64;
    let (mut pack, mut scale, mut shots) = (None, 4_u32, Vec::new());
    let mut paths = Vec::new();
    while let Some(&arg) = iter.next() {
        let mut value = |what: &str| iter.next().copied().ok_or(format!("{arg}: {what}"));
        match arg {
            "--out" => out = Some(PathBuf::from(value("a folder")?)),
            "--base" => base = Some(PathBuf::from(value("a save")?)),
            "--maps" => {
                for part in value("map numbers")?.split(',') {
                    let id: u8 = part.trim().parse().map_err(|_| format!("--maps: bad {part}"))?;
                    if id >= MAPS {
                        return Err(format!("--maps: {id} is not a map (0 to {})", MAPS - 1));
                    }
                    maps.push(id);
                }
            }
            "--frames" => {
                frames = value("a number")?.parse().map_err(|_| "--frames: a number".to_owned())?;
            }
            "--jobs" => {
                jobs = value("a number")?.parse().map_err(|_| "--jobs: a number".to_owned())?;
            }
            "--pack" => pack = Some(PathBuf::from(value("a folder")?)),
            "--scale" => {
                scale = value("a number")?.parse().map_err(|_| "--scale: a number".to_owned())?;
                if !(1..=8).contains(&scale) {
                    return Err("--scale: 1 to 8".to_owned());
                }
            }
            "--every" => {
                every = value("a number")?.parse().map_err(|_| "--every: a number".to_owned())?;
                if every == 0 {
                    return Err("--every: at least 1".to_owned());
                }
            }
            other if other.starts_with("--") => return Err(format!("unknown option {other}")),
            other if command == "shots" => shots.push(parse_shot(other)?),
            other => paths.push(PathBuf::from(other)),
        }
    }
    match command {
        "dump" => {
            if maps.is_empty() {
                maps = (0..MAPS).collect();
            }
            Ok(Args::Dump { out, maps, frames, jobs: jobs.max(1), base })
        }
        "scan" => Ok(Args::Scan { paths, every }),
        "shots" if !shots.is_empty() => Ok(Args::Shots { out, pack, scale, shots, base }),
        "shots" => Err("shots: label=map:x,y ...".to_owned()),
        other => Err(format!("unknown subcommand {other}")),
    }
}

/// `label=map:x,y`.
fn parse_shot(text: &str) -> Result<Shot, String> {
    let bad = || format!("{text}: label=map:x,y");
    let (label, rest) = text.split_once('=').ok_or_else(bad)?;
    let (map, at) = rest.split_once(':').ok_or_else(bad)?;
    let (x, y) = at.split_once(',').ok_or_else(bad)?;
    let map: u8 = map.parse().map_err(|_| bad())?;
    if map >= MAPS {
        return Err(format!("{text}: map {map} is not a map"));
    }
    Ok(Shot {
        label: label.to_owned(),
        map,
        at: (x.parse().map_err(|_| bad())?, y.parse().map_err(|_| bad())?),
    })
}

/// What the dump says of one map.
#[derive(Debug, Serialize)]
pub struct MapMeta {
    /// The map asked for and the one the game is on.
    pub asked: u8,
    pub id: u8,
    /// Size in blocks of 256 x 256 pixels.
    pub blocks: (u8, u8),
    /// The map data's pointers in the ROM: `BB:AAAA` of the grid and of the metatile table.
    pub grid: String,
    pub metatiles: String,
    /// The VRAM word address of BG2's tiles.
    pub char_base: u16,
    /// A hash of the cells' words and of the pixels of the tiles they use, and of the colours.
    pub content: String,
    pub palette: String,
    /// The tile numbers whose pixels changed in VRAM during the sample, and the palette entries
    /// (BG colours 0 to 127) that did, with the number of different values each took.
    pub animated_tiles: Vec<u16>,
    pub animated_colours: BTreeMap<u16, usize>,
    /// How many frames were sampled.
    pub sampled: u32,
    /// The cells of the map that show an animated tile.
    pub animated_cells: usize,
    pub cells: usize,
    pub distinct_tiles: usize,
    /// The map objects (NPCs, chests, Dragon Balls) as `[type, x, y]` in world pixels.
    pub objects: Vec<[u16; 3]>,
    /// Notes (the party was not free, a scene that did not end).
    pub notes: Vec<String>,
}

const GAME_MODE: usize = 0x24;
const BLOCKS_WIDE: usize = 0x703;

fn free(session: &Session) -> bool {
    let wram = session.machine.wram();
    matches!(session.mode(), 0 | 6) && wram[BLOCKS_WIDE] != 0 && wram[0x25] == 0
}

fn fnv(hash: &mut u64, bytes: &[u8]) {
    for &byte in bytes {
        *hash = (*hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01B3);
    }
}

/// The 8-bit colour of a 15-bit one, as the HD art's palettes are made.
fn expand(value: u16) -> [u8; 3] {
    let channel = |shift: u16| ((u32::from(value >> shift) & 31) * 255 + 15) / 31;
    [channel(0) as u8, channel(5) as u8, channel(10) as u8]
}

/// A pixel of a 4-bit tile in VRAM: the colour index.
fn tile_pixel(vram: &[u16], char_base: u16, tile: u16, x: usize, y: usize) -> u8 {
    let address = usize::from(char_base.wrapping_add(tile.wrapping_mul(16)) & 0x7FFF);
    let bit = 7 - x;
    let mut color = 0_u8;
    for plane in (0..4).step_by(2) {
        let word = vram[(address + y + 4 * plane) & 0x7FFF];
        color |= u8::from((word >> bit) & 1 != 0) << plane;
        color |= u8::from((word >> (bit + 8)) & 1 != 0) << (plane + 1);
    }
    color
}

/// The map's cells as tile map words, row by row (`width / 8` x `height / 8`).
fn cell_words(rom: Rom<'_>, state: &FieldState) -> Vec<u16> {
    let (width, height) = state.map.size();
    let mut words = Vec::with_capacity((width * height / 64) as usize);
    for y in (0..height).step_by(8) {
        for x in (0..width).step_by(8) {
            words.push(state.map.tile_word(rom, x, y));
        }
    }
    words
}

/// The picture of the cells at 1x: RGBA, `width` x `height`.
fn layer_rgba(words: &[u16], width: usize, vram: &[u16], cgram: &[u16], char_base: u16) -> Vec<u8> {
    let columns = width / 8;
    let rows = words.len() / columns;
    let mut rgba = vec![255_u8; width * rows * 8 * 4];
    for (cell, &word) in words.iter().enumerate() {
        let (cx, cy) = (cell % columns, cell / columns);
        let tile = word & 0x3FF;
        let row = usize::from((word >> 10) & 7);
        for y in 0..8 {
            for x in 0..8 {
                let (sx, sy) = (if word & 0x4000 != 0 { 7 - x } else { x }, {
                    if word & 0x8000 != 0 { 7 - y } else { y }
                });
                let index = tile_pixel(vram, char_base, tile, sx, sy);
                let colour =
                    if index == 0 { cgram[0] } else { cgram[row * 16 + usize::from(index)] };
                let at = ((cy * 8 + y) * width + cx * 8 + x) * 4;
                rgba[at..at + 3].copy_from_slice(&expand(colour));
            }
        }
    }
    rgba
}

/// Boots map `id` and gets to the field with the party free.
fn boot_free(
    rom: &[u8],
    game: &Game,
    base: &[u8],
    id: u8,
    camera: Option<(u16, u16)>,
    notes: &mut Vec<String>,
) -> anyhow::Result<Option<Session>> {
    let mut sets = map_sets(id);
    if let Some((x, y)) = camera {
        // The party starts where the camera of a window at (x, y) centres it: the game loads the
        // map around that place, as it does when the party walks there
        for (field, value) in &mut sets {
            match field.as_str() {
                "leader" => *value = format!("{},{}", x + 128, y + 120),
                "follower" => *value = format!("{x},{y}"),
                _ => {}
            }
        }
    }
    let sram = crafted_bytes(game, base, &sets)?;
    let mut session = Session::boot(rom, &sram)?;
    let mut finisher = Finisher::new(7);
    for t in 0..5000_u64 {
        if free(&session) && t > 0 {
            for t in 0..60 {
                session.step(Joypad { b: t % 10 < 3, ..Joypad::default() })?;
            }
            // The fade-in of the map ends
            for _ in 0..120 {
                session.step(Joypad::default())?;
            }
            return Ok(free(&session).then_some(session));
        }
        let pad = if is_battle(&session) {
            finisher.pad(&mut session, t)?
        } else {
            Joypad { a: t % 24 < 2, b: t % 24 == 12, ..Joypad::default() }
        };
        session.step(pad)?;
    }
    notes.push(format!("the party was not free after 5000 frames (mode {:02X})", session.mode()));
    Ok(None)
}

fn dump_map(
    rom: &[u8],
    game: &Game,
    base: &[u8],
    id: u8,
    frames: u32,
    out: &Path,
) -> anyhow::Result<Option<MapMeta>> {
    let mut notes = Vec::new();
    let Some(mut session) = boot_free(rom, game, base, id, None, &mut notes)? else {
        eprintln!("hd-maps: map {id}: {}", notes.join("; "));
        return Ok(None);
    };
    let rom_view = Rom(rom);
    let state = FieldState::from_wram(session.machine.wram()).context("no map header")?;
    if state.map_id != id {
        notes.push(format!("the game is on map {} instead", state.map_id));
    }
    let char_base = session
        .machine
        .frame_log()
        .and_then(|log| log.entries.iter().find(|e| e.regs.main_bg[1]))
        .map_or(0x3000, |entry| entry.regs.bg_char_base[1]);
    let vram0 = session.machine.vram().to_vec();
    let cgram0 = session.machine.cgram().to_vec();
    let words = cell_words(rom_view, &state);
    let (width, height) = state.map.size();
    let (width, height) = (width as usize, height as usize);

    // What moves: the tile set's pixels and the BG palettes, frame by frame
    let tiles_words = 0x1C00_usize;
    let at = usize::from(char_base);
    let mut tiles_changed = BTreeSet::new();
    let mut colours: BTreeMap<u16, BTreeSet<u16>> = BTreeMap::new();
    for _ in 0..frames {
        session.step(Joypad::default())?;
        if !free(&session) {
            notes.push("a scene took the party during the sample".to_owned());
            break;
        }
        let vram = session.machine.vram();
        for word in 0..tiles_words {
            if vram[(at + word) & 0x7FFF] != vram0[(at + word) & 0x7FFF] {
                tiles_changed.insert((word / 16) as u16);
            }
        }
        let cgram = session.machine.cgram();
        for index in 0..128_usize {
            if cgram[index] != cgram0[index] {
                colours.entry(index as u16).or_default().insert(cgram[index]);
            }
        }
    }

    let objects: Vec<[u16; 3]> = (0..32)
        .filter_map(|n| {
            let o = &session.machine.wram()[0x1000 + n * 6..0x1000 + n * 6 + 6];
            (o[0] & 0x80 != 0).then(|| {
                [
                    u16::from(o[0] & 0x1F),
                    u16::from_le_bytes([o[2], o[3]]),
                    u16::from_le_bytes([o[4], o[5]]),
                ]
            })
        })
        .collect();
    let used: BTreeSet<u16> = words.iter().map(|w| w & 0x3FF).collect();
    let animated_cells = words.iter().filter(|w| tiles_changed.contains(&(*w & 0x3FF))).count();
    let mut content = 0xCBF2_9CE4_8422_2325_u64;
    for word in &words {
        fnv(&mut content, &word.to_le_bytes());
    }
    for tile in &used {
        for word in 0..16 {
            fnv(&mut content, &vram0[(at + usize::from(*tile) * 16 + word) & 0x7FFF].to_le_bytes());
        }
    }
    let mut palette = 0xCBF2_9CE4_8422_2325_u64;
    for colour in &cgram0[..128] {
        fnv(&mut palette, &colour.to_le_bytes());
    }
    let ((gb, ga), (mb, ma)) = state.map.layout();
    let meta = MapMeta {
        asked: id,
        id: state.map_id,
        blocks: state.map.blocks,
        grid: format!("{gb:02X}:{ga:04X}"),
        metatiles: format!("{mb:02X}:{ma:04X}"),
        char_base,
        content: format!("{content:016x}"),
        palette: format!("{palette:016x}"),
        animated_tiles: tiles_changed.iter().copied().collect(),
        animated_colours: colours.iter().map(|(k, v)| (*k, v.len() + 1)).collect(),
        sampled: frames,
        animated_cells,
        cells: words.len(),
        distinct_tiles: used.len(),
        objects,
        notes,
    };
    let folder = out.join(format!("map-{id:02}"));
    std::fs::create_dir_all(&folder)?;
    let rgba = layer_rgba(&words, width, &vram0, &cgram0, char_base);
    ssd_hd::png::write_rgba(&folder.join("layer.png"), width as u32, height as u32, &rgba)?;
    let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    std::fs::write(folder.join("words.bin"), bytes)?;
    let cg: Vec<u8> = cgram0[..128].iter().flat_map(|w| w.to_le_bytes()).collect();
    std::fs::write(folder.join("cgram.bin"), cg)?;
    std::fs::write(folder.join("meta.json"), serde_json::to_string_pretty(&meta)?)?;
    Ok(Some(meta))
}

/// Writes the pictures of the maps.
///
/// # Errors
///
/// A save, the game or a file fails.
#[allow(clippy::too_many_arguments)]
pub fn run_dump(
    rom: &[u8],
    root: &Path,
    out: Option<&Path>,
    maps: &[u8],
    frames: u32,
    jobs: usize,
    base: Option<&Path>,
    log: &mut dyn Write,
) -> anyhow::Result<u8> {
    let out = out.map_or_else(|| root.join(".work").join("maps").join("dump"), Path::to_path_buf);
    let base_path = match base {
        Some(path) => path.to_path_buf(),
        None => crate::tour::find_base_save(root)?,
    };
    let base = std::fs::read(&base_path).with_context(|| format!("{}", base_path.display()))?;
    let game = Game::from_rom(rom)?;
    let queue = Mutex::new(maps.to_vec());
    let results = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..jobs {
            scope.spawn(|| {
                loop {
                    let Some(id) = queue.lock().expect("queue").pop() else { break };
                    let made = dump_map(rom, &game, &base, id, frames, &out);
                    results.lock().expect("results").push((id, made));
                }
            });
        }
    });
    let mut results = results.into_inner().expect("results");
    results.sort_by_key(|(id, _)| *id);
    let mut failed = 0_u8;
    for (id, made) in results {
        match made {
            Ok(Some(m)) => writeln!(
                log,
                "map {id:2} -> {:2} {}x{} blocks grid {} meta {} content {} anim tiles {} colours {} cells {}/{} {}",
                m.id,
                m.blocks.0,
                m.blocks.1,
                m.grid,
                m.metatiles,
                m.content,
                m.animated_tiles.len(),
                m.animated_colours.len(),
                m.animated_cells,
                m.cells,
                m.notes.join("; ")
            )?,
            Ok(None) => {
                failed += 1;
                writeln!(log, "map {id:2}: not reached")?;
            }
            Err(error) => {
                failed += 1;
                writeln!(log, "map {id:2}: {error:#}")?;
            }
        }
    }
    Ok(failed)
}

/// Lists, per map id, the layouts and tile sets replays show.
///
/// # Errors
///
/// A replay cannot be read or played.
pub fn run_scan(
    rom: &[u8],
    files: &[PathBuf],
    every: u64,
    out: &mut dyn Write,
) -> anyhow::Result<u8> {
    let mut seen: BTreeMap<u8, BTreeMap<String, u64>> = BTreeMap::new();
    for file in files {
        let replay = crate::shared::read_replay(file, rom)?;
        let mut machine = replay.power_on(rom.to_vec())?;
        machine.set_line_log(true);
        while machine.frame_count() < replay.frames() {
            machine.run_frame_with(&mut |f| replay.input(f), &mut NullAudio)?;
            if machine.frame_count() % every != 0 {
                continue;
            }
            let wram = machine.wram();
            if !ssd_hd::world::is_field_mode(wram[GAME_MODE]) {
                continue;
            }
            let Some(state) = FieldState::from_wram(wram) else { continue };
            let ((gb, ga), (mb, ma)) = state.map.layout();
            let mut set = 0xCBF2_9CE4_8422_2325_u64;
            for word in 0..0x1C00_usize {
                fnv(&mut set, &machine.vram()[(0x3000 + word) & 0x7FFF].to_le_bytes());
            }
            let key = format!(
                "{}x{} grid {gb:02X}:{ga:04X} meta {mb:02X}:{ma:04X}",
                state.map.blocks.0, state.map.blocks.1
            );
            *seen.entry(state.map_id).or_default().entry(key).or_default() += 1;
            let _ = set;
        }
    }
    for (id, layouts) in seen {
        for (layout, frames) in layouts {
            writeln!(out, "map {id:2}: {layout}  ({frames} samples)")?;
        }
    }
    Ok(0)
}

/// Draws places of maps as the Enhanced picture, with the art of `pack_dir` and without it, to
/// `<out>/<label>-art.png` and `-plain.png`.
///
/// # Errors
///
/// A save, the game, the pack or a file fails.
#[allow(clippy::too_many_arguments)]
pub fn run_shots(
    rom: &[u8],
    root: &Path,
    pack_dir: &Path,
    out: &Path,
    scale: u32,
    shots: &[Shot],
    base: Option<&Path>,
    log: &mut dyn Write,
) -> anyhow::Result<u8> {
    use ssd_hd::world::WideRun;
    use ssd_hd::{HdOptions, Look};
    let base_path = match base {
        Some(path) => path.to_path_buf(),
        None => crate::tour::find_base_save(root)?,
    };
    let base = std::fs::read(&base_path).with_context(|| format!("{}", base_path.display()))?;
    let game = Game::from_rom(rom)?;
    let pack = std::sync::Arc::new(
        ssd_hd::pack::Pack::open(pack_dir)
            .map_err(|e| anyhow::anyhow!("{}: {e}", pack_dir.display()))?,
    );
    std::fs::create_dir_all(out)?;
    let options = HdOptions { scale, look: Look::Crisp };
    let mut failed = 0_u8;
    for shot in shots {
        let mut notes = Vec::new();
        let Some(mut session) = boot_free(rom, &game, &base, shot.map, Some(shot.at), &mut notes)?
        else {
            writeln!(log, "{}: map {} not reached: {}", shot.label, shot.map, notes.join("; "))?;
            failed += 1;
            continue;
        };
        let mut art = WideRun::new(rom);
        art.set_art(Some(std::sync::Arc::clone(&pack)));
        let mut plain = WideRun::new(rom);
        let mut pictures = (None, None);
        for t in 0..6 {
            session.step(Joypad::default())?;
            let wram = session.machine.wram();
            if t < 5 {
                art.skip(wram);
                plain.skip(wram);
            } else {
                let log = session.machine.frame_log();
                pictures = (art.note_hd(wram, log, options), plain.note_hd(wram, log, options));
            }
        }
        let (Some(with), Some(without)) = pictures else {
            writeln!(log, "{}: the frame was not drawn", shot.label)?;
            failed += 1;
            continue;
        };
        let (width, height) = (with.picture.width, with.picture.height);
        for (name, picture) in [("art", &with), ("plain", &without)] {
            let path = out.join(format!("{}-{name}.png", shot.label));
            ssd_hd::png::write_rgba(&path, width, height, &picture.picture.rgba)?;
        }
        writeln!(
            log,
            "{}: map {} at {:?}, {width} x {height}, background pixels from art {}",
            shot.label,
            shot.map,
            shot.at,
            art.art_stats().bg_art
        )?;
    }
    Ok(failed)
}
