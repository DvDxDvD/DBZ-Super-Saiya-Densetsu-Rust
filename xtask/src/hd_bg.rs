//! `hd-art bgscan` and `hd-art bgdump`: which BG layers of the battle the replays draw, by
//! content (the layers the HD backgrounds are made of), and the whole images of those layers.
//! Output goes to stdout or the folder named; nothing here is tracked.

use crate::shared::read_replay;
use anyhow::Context;
use ssd_hd::bgart::layer_key;
use ssd_oracle::NullAudio;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;

const GAME_MODE: usize = 0x24;

/// A kind of layer the battle's backgrounds are: the registers that say it is shown, the tile rows
/// that are its picture, and whether its art fills the pixels it does not draw.
#[derive(Debug, Clone, Copy)]
pub struct Class {
    pub name: &'static str,
    pub bg: usize,
    pub mode: u8,
    pub map_base: u16,
    pub char_base: u16,
    pub size: u8,
    pub rows: [u16; 2],
    pub fill: bool,
}

/// The layers of the battle that are backgrounds: the command screen's picture (BG2), the
/// animation scene's far layer (BG4, the sky and the sea), its near layer (BG3, the ground band) and
/// the single-screen backdrops of the flights and charges (BG4).
pub const CLASSES: [Class; 4] = [
    Class {
        name: "cmd",
        bg: 1,
        mode: 1,
        map_base: 0x6000,
        char_base: 0x3000,
        size: 0,
        rows: [0, 22],
        fill: true,
    },
    Class {
        name: "far",
        bg: 3,
        mode: 0,
        map_base: 0x7000,
        char_base: 0x5000,
        size: 3,
        rows: [0, 64],
        fill: true,
    },
    Class {
        name: "near",
        bg: 2,
        mode: 0,
        map_base: 0x6800,
        char_base: 0x4000,
        size: 1,
        rows: [0, 32],
        fill: false,
    },
    Class {
        name: "dash",
        bg: 3,
        mode: 0,
        map_base: 0x7000,
        char_base: 0x5000,
        size: 0,
        rows: [0, 32],
        fill: true,
    },
];

/// What the frames of one layer content came to.
#[derive(Debug, Default, Clone)]
struct Seen {
    frames: u64,
    mosaic: u64,
    replays: BTreeMap<String, u64>,
    /// The palettes (a hash of the CGRAM) with their frames and the first frame of each.
    palettes: BTreeMap<u64, (u64, String)>,
    /// The palettes of the layer's own colours that lasted at least `PERSIST` frames in a row (not a
    /// fade or a flash): frames in such runs and the first frame of the first run.
    persistent: BTreeMap<u64, (u64, String)>,
    h: (u16, u16),
    v: (u16, u16),
    /// Distinct scroll pairs seen in one frame, at most.
    max_runs: usize,
}

/// Frames in a row a palette must be shown to be a variant of its own and not a fade or a flash.
const PERSIST: u64 = 30;

/// A hash of the colours the layer's own tile map reads (the palette rows of its keyed rows, and the
/// backdrop for a layer that fills).
fn layer_palette_hash(
    log: &ssd_oracle::FrameLog,
    class: &Class,
    planes: usize,
    large: bool,
) -> u64 {
    let (w, _) = ssd_hd::bgart::layer_size(class.size, large);
    let columns = w / if large { 16 } else { 8 };
    let mut rows = std::collections::BTreeSet::new();
    for ty in usize::from(class.rows[0])..usize::from(class.rows[1]) {
        for tx in 0..columns {
            let mut screen = 0;
            if tx >= 32 {
                screen += 1;
            }
            if ty >= 32 {
                screen += if class.size == 2 { 1 } else { 2 };
            }
            let at = usize::from(class.map_base) + screen * 1024 + (ty % 32) * 32 + tx % 32;
            rows.insert(usize::from((log.vram[at & 0x7FFF] >> 10) & 7));
        }
    }
    let mut bytes = Vec::new();
    for row in rows {
        let (first, n) = if class.mode == 0 {
            (class.bg * 32 + row * 4, 4)
        } else if planes == 4 {
            (row * 16, 16)
        } else {
            (row * 4, 4)
        };
        for i in 0..n {
            bytes.extend_from_slice(&log.cgram_start[(first + i) & 255].to_le_bytes());
        }
    }
    if class.fill {
        bytes.extend_from_slice(&log.cgram_start[0].to_le_bytes());
    }
    ssd_hd::pack::tile_key(&bytes)
}

type Table = BTreeMap<(usize, u64), Seen>;

/// What a scan keeps while it plays: the contents seen, and the run of each class's palette.
struct Scanner {
    name: String,
    seen: Table,
    /// Per class: the layer palette now shown, the frames of its run, the content key and the first
    /// frame of the run.
    runs: Vec<Option<(u64, u64, u64, u64)>>,
    last_number: u64,
    /// Where the whole image of a layer is written the first time a palette of it persists.
    dump: Option<std::path::PathBuf>,
}

impl Scanner {
    fn new(name: &str, dump: Option<&std::path::Path>) -> Self {
        Self {
            dump: dump.map(std::path::Path::to_path_buf),
            name: name.to_owned(),
            seen: BTreeMap::new(),
            runs: vec![None; CLASSES.len()],
            last_number: u64::MAX,
        }
    }

    /// One frame of a battle scene (`number` counts the frames played).
    fn feed(&mut self, log: &ssd_oracle::FrameLog, number: u64) {
        let name = &self.name;
        let contiguous = self.last_number != u64::MAX && number == self.last_number + 1;
        self.last_number = number;
        let mut shown = [false; CLASSES.len()];
        for (at, class) in CLASSES.iter().enumerate() {
            let lines: Vec<_> = log
                .entries
                .iter()
                .filter(|e| {
                    let r = &e.regs;
                    !r.forced_blanking
                        && r.bg_mode == class.mode
                        && r.main_bg[class.bg]
                        && r.bg_map_base[class.bg] == class.map_base
                        && r.bg_char_base[class.bg] == class.char_base
                        && r.bg_screen_size[class.bg] == class.size
                })
                .collect();
            let Some(first) = lines.first() else { continue };
            let planes = if class.mode == 0 || class.bg >= 2 { 2 } else { 4 };
            let key = layer_key(
                &log.vram,
                class.map_base,
                class.char_base,
                class.size,
                first.regs.bg_large_tiles[class.bg],
                planes,
                class.rows,
            );
            let entry = self.seen.entry((at, key)).or_default();
            entry.frames += 1;
            entry.mosaic += u64::from(first.regs.mosaic_bg[class.bg] && first.regs.mosaic_size > 0);
            *entry.replays.entry(name.clone()).or_default() += 1;
            let cg: Vec<u8> = log.cgram_start.iter().flat_map(|w| w.to_le_bytes()).collect();
            let hash = ssd_hd::pack::tile_key(&cg);
            let slot = entry.palettes.entry(hash).or_insert((0, format!("{name}:{number}")));
            slot.0 += 1;
            let phash = layer_palette_hash(log, class, planes, first.regs.bg_large_tiles[class.bg]);
            shown[at] = true;
            let continues =
                contiguous && self.runs[at].is_some_and(|(p, _, k, _)| p == phash && k == key);
            if continues {
                if let Some(run) = self.runs[at].as_mut() {
                    run.1 += 1;
                    if run.1 == PERSIST {
                        let slot = entry
                            .persistent
                            .entry(phash)
                            .or_insert((0, format!("{name}:{}", run.3)));
                        slot.0 += PERSIST;
                        if let Some(dir) = &self.dump {
                            let stem = format!("{}-{key:016x}-{phash:016x}", class.name);
                            // (a failed write only loses a dump)
                            let _ = write_layer(log, &first.regs, class, dir, &stem);
                        }
                    } else if run.1 > PERSIST
                        && let Some(slot) = entry.persistent.get_mut(&phash)
                    {
                        slot.0 += 1;
                    }
                }
            } else {
                self.runs[at] = Some((phash, 1, key, number));
            }
            let hs = lines.iter().map(|e| e.regs.bg_h_scroll[class.bg]);
            let vs = lines.iter().map(|e| e.regs.bg_v_scroll[class.bg]);
            let (hmin, hmax) = (hs.clone().min().unwrap_or(0), hs.max().unwrap_or(0));
            let (vmin, vmax) = (vs.clone().min().unwrap_or(0), vs.max().unwrap_or(0));
            if entry.frames == 1 {
                entry.h = (hmin, hmax);
                entry.v = (vmin, vmax);
            }
            entry.h = (entry.h.0.min(hmin), entry.h.1.max(hmax));
            entry.v = (entry.v.0.min(vmin), entry.v.1.max(vmax));
            let mut pairs: Vec<(u16, u16)> = lines
                .iter()
                .map(|e| (e.regs.bg_h_scroll[class.bg], e.regs.bg_v_scroll[class.bg]))
                .collect();
            pairs.sort_unstable();
            pairs.dedup();
            entry.max_runs = entry.max_runs.max(pairs.len());
        }
        for (at, was) in shown.iter().enumerate() {
            if !was {
                self.runs[at] = None;
            }
        }
    }
}

/// Writes the whole image of BG `bg` as `<stem>-src.png` (transparent where the layer is) and
/// `<stem>-fill.png` (over the backdrop) into `dir`, unless they are there.
fn write_layer(
    log: &ssd_oracle::FrameLog,
    regs: &ssd_oracle::LineRegs,
    class: &Class,
    dir: &std::path::Path,
    stem: &str,
) -> anyhow::Result<()> {
    let bg = class.bg;
    std::fs::create_dir_all(dir)?;
    let src = dir.join(format!("{stem}-src.png"));
    if src.exists() {
        return Ok(());
    }
    let (w, h, rgba) = layer_image(&log.vram, &log.cgram_start, regs, bg);
    let backdrop = log.cgram_start[0];
    let back = |shift: u16| (((u32::from(backdrop >> shift) & 31) * 255 + 15) / 31) as u8;
    let mut filled = rgba.clone();
    for px in filled.as_chunks_mut::<4>().0.iter_mut() {
        if px[3] == 0 {
            px.copy_from_slice(&[back(0), back(5), back(10), 255]);
        }
    }
    std::fs::write(
        dir.join(format!("{stem}-fill.png")),
        crate::shots::encode_png(w as u32, h as u32, &filled),
    )?;
    std::fs::write(src, crate::shots::encode_png(w as u32, h as u32, &rgba))?;
    // The CGRAM the layer's pixels read (`ssd_hd::bgart::palette_text`; the entries no pixel of the
    // rows uses are unknown), for the pack's `palette`
    let (_, _, indexes) = layer_indexes(&log.vram, regs, bg, class.rows);
    let mut colours = [ssd_hd::bgart::UNKNOWN_COLOUR; 256];
    for index in indexes.into_iter().flatten() {
        colours[usize::from(index)] = log.cgram_start[usize::from(index)];
    }
    if class.fill {
        colours[0] = log.cgram_start[0];
    }
    std::fs::write(dir.join(format!("{stem}-palette.txt")), ssd_hd::bgart::palette_text(&colours))?;
    Ok(())
}

fn scan_replay(
    rom: &[u8],
    path: &std::path::Path,
    dump: Option<&std::path::Path>,
) -> anyhow::Result<Table> {
    let name = path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    let mut scanner = Scanner::new(&name, dump);
    let replay = read_replay(path, rom)?;
    let mut machine = replay.power_on(rom.to_vec())?;
    machine.set_line_log(true);
    while machine.frame_count() < replay.frames() {
        machine.run_frame_with(&mut |f| replay.input(f), &mut NullAudio)?;
        let number = machine.frame_count() - 1;
        let mode = machine.wram()[GAME_MODE] & 0x7F;
        if !matches!(mode, 0x02 | 0x04 | 0x0C) {
            continue;
        }
        let Some(log) = machine.frame_log() else { continue };
        scanner.feed(log, number);
    }
    Ok(scanner.seen)
}

/// `hd-art bgmaps`: the battle backgrounds each map and encounter zone shows (the backgrounds the
/// replays never reach). From a crafted save on each map a battle is forced in each of the zones 1
/// to 3 and played for `frames` frames; the table is `bgscan`'s, the scenes named
/// `map-<id>-z<zone>`.
pub fn run_maps(
    rom: &[u8],
    root: &std::path::Path,
    frames: u64,
    jobs: usize,
    only: &[u8],
    dump: Option<&std::path::Path>,
    out: &mut dyn Write,
) -> anyhow::Result<u8> {
    use crate::tour_battle::Fuzz;
    let base_path = crate::tour::find_base_save(root)?;
    let base = std::fs::read(&base_path).context("the base save")?;
    let game = crate::save::Game::from_rom(rom)?;
    let work: Vec<(u8, u8)> = (0..48)
        .filter(|id| only.is_empty() || only.contains(id))
        .flat_map(|id| (1..=3).map(move |zone| (id, zone)))
        .collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let results = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..jobs.max(1) {
            scope.spawn(|| {
                loop {
                    let at = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(&(id, zone)) = work.get(at) else { break };
                    let name = format!("map-{id}-z{zone}");
                    let table = (|| -> anyhow::Result<Option<Table>> {
                        let Some(mut session) =
                            crate::tour_visit::boot_free(rom, &game, &base, id, None)?
                        else {
                            return Ok(None);
                        };
                        session.zone = zone;
                        if session.walk_into_battle().is_err() {
                            return Ok(None);
                        }
                        let mut scanner = Scanner::new(&name, dump);
                        let mut fuzz = Fuzz::new(7 + u32::from(id));
                        for t in 0..frames {
                            let pad = fuzz.pad(
                                t,
                                session.mode(),
                                session.wram(0x0D5F),
                                session.wram(0x0D62),
                            );
                            session.step(pad)?;
                            if matches!(session.mode(), 0x02 | 0x04 | 0x0C)
                                && let Some(log) = session.machine.frame_log()
                            {
                                scanner.feed(log, t);
                            }
                        }
                        Ok(Some(scanner.seen))
                    })();
                    match table {
                        Ok(Some(table)) => {
                            eprintln!("bgmaps: {name}");
                            results.lock().expect("results").push(Ok(table));
                        }
                        Ok(None) => eprintln!("bgmaps: {name}: no battle"),
                        Err(error) => eprintln!("bgmaps: {name}: {error:#}"),
                    }
                }
            });
        }
    });
    print_table(merge(results.into_inner().expect("results"))?, out)
}

/// Plays the replays and prints, for each class of background layer, the distinct contents with
/// their frames, replays, palettes, scroll ranges and a frame to take the layer from.
pub fn run(
    rom: &[u8],
    files: &[PathBuf],
    dump: Option<&std::path::Path>,
    out: &mut dyn Write,
) -> anyhow::Result<u8> {
    let next = std::sync::atomic::AtomicUsize::new(0);
    let results = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..4.min(files.len()) {
            scope.spawn(|| {
                loop {
                    let at = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(path) = files.get(at) else { break };
                    let table = scan_replay(rom, path, dump);
                    eprintln!("bgscan: {}", path.display());
                    results.lock().expect("results").push(table);
                }
            });
        }
    });
    print_table(merge(results.into_inner().expect("results"))?, out)
}

/// The tables of the scans put together.
fn merge(tables: Vec<anyhow::Result<Table>>) -> anyhow::Result<Table> {
    let mut all: Table = BTreeMap::new();
    for table in tables {
        for (key, seen) in table.context("a replay")? {
            let entry = all.entry(key).or_default();
            if entry.frames == 0 {
                entry.h = seen.h;
                entry.v = seen.v;
            }
            entry.frames += seen.frames;
            entry.mosaic += seen.mosaic;
            entry.h = (entry.h.0.min(seen.h.0), entry.h.1.max(seen.h.1));
            entry.v = (entry.v.0.min(seen.v.0), entry.v.1.max(seen.v.1));
            entry.max_runs = entry.max_runs.max(seen.max_runs);
            for (k, v) in seen.replays {
                *entry.replays.entry(k).or_default() += v;
            }
            for (k, (n, first)) in seen.palettes {
                let slot = entry.palettes.entry(k).or_insert((0, first));
                slot.0 += n;
            }
            for (k, (n, first)) in seen.persistent {
                let slot = entry.persistent.entry(k).or_insert((0, first));
                slot.0 += n;
            }
        }
    }
    Ok(all)
}

/// Prints the table of a scan.
fn print_table(all: Table, out: &mut dyn Write) -> anyhow::Result<u8> {
    let mut json = Vec::new();
    for ((class, key), seen) in &all {
        let c = &CLASSES[*class];
        let mut palettes: Vec<_> = seen.palettes.iter().collect();
        palettes.sort_by_key(|(_, (n, _))| std::cmp::Reverse(*n));
        let top = palettes.first().map(|(_, (n, first))| (*n, first.clone())).unwrap_or_default();
        writeln!(
            out,
            "{:<4} {key:016x} {:>6} frames ({} mosaic), {} replays, {} palettes (top {} frames at {}), h {:?} v {:?}, up to {} scroll pairs a frame",
            c.name,
            seen.frames,
            seen.mosaic,
            seen.replays.len(),
            seen.palettes.len(),
            top.0,
            top.1,
            seen.h,
            seen.v,
            seen.max_runs
        )?;
        let mut persistent: Vec<_> = seen.persistent.iter().collect();
        persistent.sort_by_key(|(_, (n, _))| std::cmp::Reverse(*n));
        writeln!(
            out,
            "     persistent palettes: {}",
            persistent
                .iter()
                .map(|(h, (n, f))| format!("{h:016x} {n} frames from {f}"))
                .collect::<Vec<_>>()
                .join("; ")
        )?;
        json.push(serde_json::json!({
            "class": c.name, "fill": c.fill, "key": format!("{key:016x}"), "frames": seen.frames, "mosaic": seen.mosaic,
            "replays": seen.replays, "palettes": seen.palettes.len(), "top_frame": top.1, "top_frames": top.0,
            "h": [seen.h.0, seen.h.1], "v": [seen.v.0, seen.v.1], "scroll_pairs": seen.max_runs,
            "palette_frames": palettes.iter().take(6).map(|(_, (n, f))| serde_json::json!([n, f])).collect::<Vec<_>>(),
        }));
    }
    writeln!(out, "JSON {}", serde_json::to_string(&json)?)?;
    Ok(0)
}

/// The whole image of a BG layer (its tile map, not the visible window) as RGBA, 4 bytes a
/// pixel, `(width, height, rgba)`. Colour 0 is transparent.
pub fn layer_image(
    vram: &[u16],
    cgram: &[u16],
    regs: &ssd_oracle::LineRegs,
    bg: usize,
) -> (usize, usize, Vec<u8>) {
    let planes = if regs.bg_mode == 0 || bg >= 2 { 2 } else { 4 };
    let tile_words = planes * 4;
    let large = regs.bg_large_tiles[bg];
    let size = regs.bg_screen_size[bg];
    let (screens_w, screens_h) = match size {
        0 => (1, 1),
        1 => (2, 1),
        2 => (1, 2),
        _ => (2, 2),
    };
    let unit = if large { 16 } else { 8 };
    let (width, height) = (screens_w * 32 * unit, screens_h * 32 * unit);
    let mut rgba = vec![0_u8; width * height * 4];
    let char_base = usize::from(regs.bg_char_base[bg]);
    let map_base = usize::from(regs.bg_map_base[bg]);
    for y in 0..height {
        for x in 0..width {
            let (tx, ty) = (x / unit, y / unit);
            let mut screen = 0;
            if tx >= 32 {
                screen += 1;
            }
            if ty >= 32 {
                screen += if size == 2 { 1 } else { 2 };
            }
            let entry = vram[(map_base + screen * 1024 + (ty % 32) * 32 + tx % 32) & 0x7FFF];
            let (x_flip, y_flip) = (entry & 0x4000 != 0, entry & 0x8000 != 0);
            let (mut ix, mut iy) = (x % unit, y % unit);
            if x_flip {
                ix = unit - 1 - ix;
            }
            if y_flip {
                iy = unit - 1 - iy;
            }
            let tile = usize::from(entry & 0x3FF) + (ix / 8) + 16 * (iy / 8);
            let (col, row) = (ix % 8, iy % 8);
            let at = char_base + tile * tile_words;
            let mut color = 0_usize;
            for plane in (0..planes).step_by(2) {
                let word = vram[(at + row + 4 * plane) & 0x7FFF];
                let bit = 7 - col;
                color |= usize::from((word >> bit) & 1 != 0) << plane;
                color |= usize::from((word >> (bit + 8)) & 1 != 0) << (plane + 1);
            }
            if color == 0 {
                continue;
            }
            let palette = usize::from((entry >> 10) & 7);
            let index = if regs.bg_mode == 0 {
                bg * 32 + palette * 4 + color
            } else if planes == 4 {
                palette * 16 + color
            } else {
                palette * 4 + color
            };
            let word = cgram[index & 255];
            let channel = |shift: u16| (((u32::from(word >> shift) & 31) * 255 + 15) / 31) as u8;
            let o = (y * width + x) * 4;
            rgba[o..o + 4].copy_from_slice(&[channel(0), channel(5), channel(10), 255]);
        }
    }
    (width, height, rgba)
}

/// `hd-art bgdump <replay> <frame>[,<frame>...] <out dir>`: for each frame writes the 1x frame and
/// the whole image of each of its BG layers (transparent colour 0 shown as magenta), and prints
/// the layers' scroll tables.
pub fn dump(
    rom: &[u8],
    replay: &std::path::Path,
    frames: &[u64],
    out_dir: &std::path::Path,
    out: &mut dyn Write,
) -> anyhow::Result<u8> {
    std::fs::create_dir_all(out_dir)?;
    let name = replay.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    let file = read_replay(replay, rom)?;
    let mut machine = file.power_on(rom.to_vec())?;
    machine.set_line_log(true);
    let last = frames.iter().copied().max().unwrap_or(0);
    let mut renderer = ssd_hd::Renderer::new();
    while machine.frame_count() <= last && machine.frame_count() < file.frames() {
        machine.run_frame_with(&mut |f| file.input(f), &mut NullAudio)?;
        let number = machine.frame_count() - 1;
        let Some(log) = machine.frame_log() else { continue };
        let want = frames.contains(&number);
        if !want {
            continue;
        }
        let frame = renderer.render(log);
        let mode = machine.wram()[GAME_MODE];
        writeln!(out, "{name}:{number} game mode {mode:02X}")?;
        if let Ok(frame) = &frame {
            let mut rgba = Vec::with_capacity(frame.pixels.len() * 4);
            for c in &frame.pixels {
                rgba.extend_from_slice(&[c.r, c.g, c.b, 255]);
            }
            std::fs::write(
                out_dir.join(format!("{name}-{number}-frame.png")),
                crate::shots::encode_png(frame.width as u32, frame.height as u32, &rgba),
            )?;
        }
        let Some(first) = log.entries.iter().find(|e| !e.regs.forced_blanking) else { continue };
        for bg in 0..4 {
            let lines: Vec<_> = log
                .entries
                .iter()
                .filter(|e| !e.regs.forced_blanking && (e.regs.main_bg[bg] || e.regs.sub_bg[bg]))
                .collect();
            if lines.is_empty() {
                continue;
            }
            let (w, h, rgba) = layer_image(&log.vram, &log.cgram_start, &lines[0].regs, bg);
            std::fs::write(
                out_dir.join(format!("{name}-{number}-bg{}-src.png", bg + 1)),
                crate::shots::encode_png(w as u32, h as u32, &rgba),
            )?;
            // The same over the backdrop: what the layer and the sky behind it show
            let backdrop = log.cgram_start[0];
            let back = |shift: u16| (((u32::from(backdrop >> shift) & 31) * 255 + 15) / 31) as u8;
            let mut filled = rgba.clone();
            for px in filled.as_chunks_mut::<4>().0.iter_mut() {
                if px[3] == 0 {
                    px.copy_from_slice(&[back(0), back(5), back(10), 255]);
                }
            }
            std::fs::write(
                out_dir.join(format!("{name}-{number}-bg{}-fill.png", bg + 1)),
                crate::shots::encode_png(w as u32, h as u32, &filled),
            )?;
            // scroll runs: (first line, last line, hscroll, vscroll, on the main screen)
            let mut runs: Vec<(u16, u16, u16, u16, bool)> = Vec::new();
            for e in &lines {
                let key = (e.regs.bg_h_scroll[bg], e.regs.bg_v_scroll[bg], e.regs.main_bg[bg]);
                match runs.last_mut() {
                    Some(r) if (r.2, r.3, r.4) == key && r.1 + 1 >= e.scanline => r.1 = e.scanline,
                    _ => runs.push((e.scanline, e.scanline, key.0, key.1, key.2)),
                }
            }
            let r = lines[0].regs;
            let planes = if r.bg_mode == 0 || bg >= 2 { 2 } else { 4 };
            let (_, height) = ssd_hd::bgart::layer_size(r.bg_screen_size[bg], r.bg_large_tiles[bg]);
            let rows = (height / if r.bg_large_tiles[bg] { 16 } else { 8 }) as u16;
            let key = layer_key(
                &log.vram,
                r.bg_map_base[bg],
                r.bg_char_base[bg],
                r.bg_screen_size[bg],
                r.bg_large_tiles[bg],
                planes,
                [0, rows],
            );
            writeln!(
                out,
                "  bg{} {w}x{h} key {key:016x} map {:04X} chr {:04X} large {} size {} mosaic {} lines {}..{} scroll runs {}",
                bg + 1,
                r.bg_map_base[bg],
                r.bg_char_base[bg],
                r.bg_large_tiles[bg],
                r.bg_screen_size[bg],
                r.mosaic_bg[bg],
                lines[0].scanline,
                lines[lines.len() - 1].scanline,
                runs.len()
            )?;
            for run in runs.iter().take(12) {
                writeln!(
                    out,
                    "      lines {}..{} h {} v {} main {}",
                    run.0, run.1, run.2, run.3, run.4
                )?;
            }
        }
        let mut sprites = Vec::new();
        for n in 0..128 {
            let (x, y, tile, attr) = (
                log.oam_low[n * 4],
                log.oam_low[n * 4 + 1],
                log.oam_low[n * 4 + 2],
                log.oam_low[n * 4 + 3],
            );
            let high = log.oam_high[n / 4] >> ((n % 4) * 2) & 3;
            if y < 224 {
                sprites.push(format!(
                    "#{n} x{} y{y} t{tile:02X} pal{} pri{} size{}",
                    u16::from(x) | u16::from(high & 1) << 8,
                    (attr >> 1) & 7,
                    (attr >> 4) & 3,
                    high >> 1
                ));
            }
        }
        writeln!(out, "  objects on screen: {}", sprites.join("; "))?;
        writeln!(
            out,
            "  cgram writes {} vram writes {} tm {:?}/{:?} math {} fixed {:04X} bright {}",
            log.cgram_writes.len(),
            log.vram_writes.len(),
            first.regs.main_bg,
            first.regs.sub_bg,
            first.regs.color_math,
            first.regs.fixed_color,
            first.regs.brightness
        )?;
    }
    Ok(0)
}

/// `hd-art picscan <out dir> [replay|dir...]`: for every frame of the replays, the game mode and
/// the BG layers shown (registers, content key over the whole map, scroll range, mosaic), the
/// frame's loads of VRAM and palette, one JSON line per frame in `<out dir>/<replay>.jsonl` (the
/// research behind the big pictures; nothing here is tracked).
pub fn picscan(rom: &[u8], files: &[PathBuf], out_dir: &std::path::Path) -> anyhow::Result<u8> {
    std::fs::create_dir_all(out_dir)?;
    let next = std::sync::atomic::AtomicUsize::new(0);
    let failed = std::sync::atomic::AtomicBool::new(false);
    std::thread::scope(|scope| {
        for _ in 0..4.min(files.len()) {
            scope.spawn(|| {
                loop {
                    let at = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(path) = files.get(at) else { break };
                    let name = path
                        .file_stem()
                        .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
                    let target = out_dir.join(format!("{name}.jsonl"));
                    match picscan_replay(rom, path, &target) {
                        Ok(()) => eprintln!("picscan: {name}"),
                        Err(error) => {
                            eprintln!("picscan: {name}: {error:#}");
                            failed.store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                    }
                }
            });
        }
    });
    Ok(u8::from(failed.load(std::sync::atomic::Ordering::Relaxed)))
}

fn picscan_replay(
    rom: &[u8],
    path: &std::path::Path,
    target: &std::path::Path,
) -> anyhow::Result<()> {
    let replay = read_replay(path, rom)?;
    let mut machine = replay.power_on(rom.to_vec())?;
    machine.set_line_log(true);
    let mut out = std::io::BufWriter::new(std::fs::File::create(target)?);
    while machine.frame_count() < replay.frames() {
        machine.run_frame_with(&mut |f| replay.input(f), &mut NullAudio)?;
        let number = machine.frame_count() - 1;
        let mode = machine.wram()[GAME_MODE];
        let Some(log) = machine.frame_log() else { continue };
        let lit: Vec<_> = log.entries.iter().filter(|e| !e.regs.forced_blanking).collect();
        let Some(first) = lit.first() else {
            writeln!(out, "{}", serde_json::json!({"f": number, "gm": mode, "blank": true}))?;
            continue;
        };
        let mut layers = Vec::new();
        for bg in 0..4 {
            let shown: Vec<_> =
                lit.iter().filter(|e| e.regs.main_bg[bg] || e.regs.sub_bg[bg]).collect();
            let Some(top) = shown.first() else { continue };
            let regs = &top.regs;
            if regs.bg_mode == 7 {
                continue;
            }
            let planes = if regs.bg_mode == 0 || bg >= 2 { 2 } else { 4 };
            let large = regs.bg_large_tiles[bg];
            let (_, height) = ssd_hd::bgart::layer_size(regs.bg_screen_size[bg], large);
            let rows = (height / if large { 16 } else { 8 }) as u16;
            let key = layer_key(
                &log.vram,
                regs.bg_map_base[bg],
                regs.bg_char_base[bg],
                regs.bg_screen_size[bg],
                large,
                planes,
                [0, rows],
            );
            let key22 = layer_key(
                &log.vram,
                regs.bg_map_base[bg],
                regs.bg_char_base[bg],
                regs.bg_screen_size[bg],
                large,
                planes,
                [0, rows.min(22)],
            );
            let hs = shown.iter().map(|e| e.regs.bg_h_scroll[bg]);
            let vs = shown.iter().map(|e| e.regs.bg_v_scroll[bg]);
            layers.push(serde_json::json!({
                "key22": format!("{key22:016x}"),
                "bg": bg, "mode": regs.bg_mode, "map": regs.bg_map_base[bg],
                "chr": regs.bg_char_base[bg], "size": regs.bg_screen_size[bg], "large": large,
                "key": format!("{key:016x}"), "lines": shown.len(),
                "main": top.regs.main_bg[bg],
                "h": [hs.clone().min(), hs.max()], "v": [vs.clone().min(), vs.max()],
                "mosaic": regs.mosaic_bg[bg] && regs.mosaic_size > 0,
            }));
        }
        let cg: Vec<u8> = log.cgram_start.iter().flat_map(|w| w.to_le_bytes()).collect();
        let m7_lines = lit.iter().filter(|e| e.regs.bg_mode == 7).count();
        let obj_lines = lit.iter().filter(|e| e.regs.main_obj).count();
        writeln!(
            out,
            "{}",
            serde_json::json!({
                "f": number, "gm": mode, "layers": layers, "m7": m7_lines, "obj": obj_lines,
                "vram_writes": log.vram_writes.len(), "cg_writes": log.cgram_writes.len(),
                "pal": format!("{:016x}", ssd_hd::pack::tile_key(&cg)),
                "bright": first.regs.brightness, "math": first.regs.color_math,
                "tm": first.regs.main_bg, "ts": first.regs.sub_bg, "fixed": first.regs.fixed_color,
                "win": first.regs.bg_window_area,
            })
        )?;
    }
    out.flush()?;
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// `hd-art refpal`: the palettes the art of the layers was made in

/// The CGRAM index of each pixel of the rows `rows` of a BG layer's whole image (`None` where the
/// layer's colour is 0, transparent): `(width, height, indexes)`, like [`layer_image`].
fn layer_indexes(
    vram: &[u16],
    regs: &ssd_oracle::LineRegs,
    bg: usize,
    rows: [u16; 2],
) -> (usize, usize, Vec<Option<u16>>) {
    let planes = if regs.bg_mode == 0 || bg >= 2 { 2 } else { 4 };
    let tile_words = planes * 4;
    let large = regs.bg_large_tiles[bg];
    let size = regs.bg_screen_size[bg];
    let screens_w = match size {
        0 | 2 => 1,
        _ => 2,
    };
    let unit = if large { 16 } else { 8 };
    let width = screens_w * 32 * unit;
    let (first, last) = (usize::from(rows[0]) * unit, usize::from(rows[1]) * unit);
    let mut out = vec![None; width * (last - first)];
    let char_base = usize::from(regs.bg_char_base[bg]);
    let map_base = usize::from(regs.bg_map_base[bg]);
    for y in first..last {
        for x in 0..width {
            let (tx, ty) = (x / unit, y / unit);
            let mut screen = 0;
            if tx >= 32 {
                screen += 1;
            }
            if ty >= 32 {
                screen += if size == 2 { 1 } else { 2 };
            }
            let entry = vram[(map_base + screen * 1024 + (ty % 32) * 32 + tx % 32) & 0x7FFF];
            let (x_flip, y_flip) = (entry & 0x4000 != 0, entry & 0x8000 != 0);
            let (mut ix, mut iy) = (x % unit, y % unit);
            if x_flip {
                ix = unit - 1 - ix;
            }
            if y_flip {
                iy = unit - 1 - iy;
            }
            let tile = usize::from(entry & 0x3FF) + (ix / 8) + 16 * (iy / 8);
            let (col, row) = (ix % 8, iy % 8);
            let at = char_base + tile * tile_words;
            let mut color = 0_usize;
            for plane in (0..planes).step_by(2) {
                let word = vram[(at + row + 4 * plane) & 0x7FFF];
                let bit = 7 - col;
                color |= usize::from((word >> bit) & 1 != 0) << plane;
                color |= usize::from((word >> (bit + 8)) & 1 != 0) << (plane + 1);
            }
            if color == 0 {
                continue;
            }
            let palette = usize::from((entry >> 10) & 7);
            let index = if regs.bg_mode == 0 {
                bg * 32 + palette * 4 + color
            } else if planes == 4 {
                palette * 16 + color
            } else {
                palette * 4 + color
            };
            out[(y - first) * width + x] = Some((index & 255) as u16);
        }
    }
    (width, last - first, out)
}

/// The 15-bit colour whose 8-bit channels (the way the layer images expand them) are `rgb`.
fn compress_colour(rgb: [u8; 3]) -> Option<u16> {
    let mut word = 0_u16;
    for (at, &c8) in rgb.iter().enumerate() {
        let c5 = (u32::from(c8) * 31 + 127) / 255;
        if (c5 * 255 + 15) / 31 != u32::from(c8) {
            return None;
        }
        word |= (c5 as u16) << (5 * at);
    }
    Some(word)
}

/// What the palette of a layer came to.
#[derive(Debug, Default)]
pub struct RefPalette {
    pub text: String,
    /// The pixels that voted, and those that agreed with their entry's colour.
    pub votes: u64,
    pub agreed: u64,
    /// The CGRAM entries that no pixel voted for.
    pub unknown: usize,
}

fn palette_of(votes: &[BTreeMap<u16, u64>; 256]) -> RefPalette {
    let mut colours = [ssd_hd::bgart::UNKNOWN_COLOUR; 256];
    let mut result = RefPalette::default();
    for (index, seen) in votes.iter().enumerate() {
        let total: u64 = seen.values().sum();
        match seen.iter().max_by_key(|(_, n)| **n) {
            Some((&colour, &n)) => {
                colours[index] = colour;
                result.votes += total;
                result.agreed += n;
            }
            None => result.unknown += 1,
        }
    }
    result.text = ssd_hd::bgart::palette_text(&colours);
    result
}

/// The palettes by layer id, for the backgrounds and for the maps.
pub type Palettes = (BTreeMap<String, String>, BTreeMap<String, String>);

/// Finds the palette each background and picture layer of `pack` was made in, by playing `files`:
/// in a frame that shows a layer's content (its key), the CGRAM index of each pixel of the layer is
/// known, and the layer's source picture says the colour that pixel had when the art was made, so
/// each index has the colour the pixels of that index agree on. Also each map layer's: the dump
/// of a map whose layer picture is the source (`hd-maps dump`) has the CGRAM it was made with.
///
/// # Errors
///
/// A source picture or a dump cannot be read.
pub fn refpal(
    rom: &[u8],
    pack: &ssd_hd::pack::Pack,
    files: &[PathBuf],
    maps_dump: Option<&std::path::Path>,
    out: &mut dyn Write,
) -> anyhow::Result<Palettes> {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let layers = &pack.bg.layer;
    // The source pictures: the colours of the pixels the art replaces
    let mut sources = Vec::new();
    for layer in layers {
        let source = if layer.source.is_empty() {
            None
        } else {
            let bytes = std::fs::read(pack.dir.join(&layer.source))
                .with_context(|| format!("the source of {}", layer.id))?;
            Some(ssd_hd::png::read_rgba(&bytes).map_err(|e| anyhow::anyhow!("{}: {e}", layer.id))?)
        };
        sources.push(source);
    }
    let done: Vec<AtomicBool> = layers.iter().map(|_| AtomicBool::new(false)).collect();
    let found: std::sync::Mutex<Vec<Option<(RefPalette, String)>>> =
        std::sync::Mutex::new(layers.iter().map(|_| None).collect());
    let next = AtomicUsize::new(0);
    let all_done = || done.iter().all(|d| d.load(Ordering::Relaxed));
    std::thread::scope(|scope| {
        for _ in 0..3.min(files.len().max(1)) {
            scope.spawn(|| {
                while let Some(path) = files.get(next.fetch_add(1, Ordering::Relaxed)) {
                    if all_done() {
                        break;
                    }
                    let name = path
                        .file_stem()
                        .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
                    let Ok(replay) = read_replay(path, rom) else { continue };
                    let Ok(mut machine) = replay.power_on(rom.to_vec()) else { continue };
                    machine.set_line_log(true);
                    while machine.frame_count() < replay.frames() && !all_done() {
                        if machine.run_frame_with(&mut |f| replay.input(f), &mut NullAudio).is_err()
                        {
                            break;
                        }
                        let number = machine.frame_count() - 1;
                        let Some(log) = machine.frame_log() else { continue };
                        for (at, layer) in layers.iter().enumerate() {
                            let Some(source) = &sources[at] else { continue };
                            if done[at].load(Ordering::Relaxed) {
                                continue;
                            }
                            let bg = usize::from(layer.bg);
                            let Some(first) = log.entries.iter().find(|e| {
                                let r = &e.regs;
                                !r.forced_blanking
                                    && r.bg_mode == layer.mode
                                    && (r.main_bg[bg] || r.sub_bg[bg])
                                    && r.bg_map_base[bg] == layer.map_base
                                    && r.bg_char_base[bg] == layer.char_base
                                    && r.bg_screen_size[bg] == layer.size
                            }) else {
                                continue;
                            };
                            let planes = if layer.mode == 0 || bg >= 2 { 2 } else { 4 };
                            let key = layer_key(
                                &log.vram,
                                layer.map_base,
                                layer.char_base,
                                layer.size,
                                first.regs.bg_large_tiles[bg],
                                planes,
                                layer.rows,
                            );
                            if key != layer.key_value() {
                                continue;
                            }
                            // (art made for one palette of the content is read in a frame of that palette)
                            if let Some(want) = layer.palette_value()
                                && ssd_hd::bgart::palette_key(
                                    &log.vram,
                                    &log.cgram_start,
                                    bg,
                                    layer.mode,
                                    layer.map_base,
                                    layer.size,
                                    first.regs.bg_large_tiles[bg],
                                    planes,
                                    layer.rows,
                                    layer.fill,
                                ) != want
                            {
                                continue;
                            }
                            let (w, h, indexes) =
                                layer_indexes(&log.vram, &first.regs, bg, layer.rows);
                            if (source.0 as usize, source.1 as usize) != (w, h) {
                                continue;
                            }
                            let mut votes: [BTreeMap<u16, u64>; 256] =
                                std::array::from_fn(|_| BTreeMap::new());
                            for (n, index) in indexes.iter().enumerate() {
                                let s = &source.2[n * 4..n * 4 + 4];
                                if s[3] == 0 {
                                    continue;
                                }
                                // (the layer's transparent pixels show the backdrop, entry 0)
                                let index = index.unwrap_or(0);
                                if let Some(colour) = compress_colour([s[0], s[1], s[2]]) {
                                    *votes[usize::from(index)].entry(colour).or_default() += 1;
                                }
                            }
                            found.lock().expect("found")[at] =
                                Some((palette_of(&votes), format!("{name}:{number}")));
                            done[at].store(true, Ordering::Relaxed);
                        }
                    }
                    eprintln!("refpal: {}", path.display());
                }
            });
        }
    });
    let found = found.into_inner().expect("found");
    let mut bg = BTreeMap::new();
    for (layer, found) in layers.iter().zip(found) {
        match found {
            Some((palette, from)) => {
                writeln!(
                    out,
                    "refpal: bg {:<28} from {from:<30} {} of {} votes agree, {} entries unknown",
                    layer.id, palette.agreed, palette.votes, palette.unknown
                )?;
                bg.insert(layer.id.clone(), palette.text);
            }
            None => writeln!(out, "refpal: bg {:<28} NOT FOUND in the replays", layer.id)?,
        }
    }
    // The maps: the CGRAM of the dump whose layer picture is the source
    let mut maps = BTreeMap::new();
    if let Some(dump) = maps_dump {
        for layer in &pack.maps.layer {
            if layer.source.is_empty() {
                continue;
            }
            let bytes = std::fs::read(pack.dir.join("art/maps").join(&layer.source))?;
            let (sw, sh, source) =
                ssd_hd::png::read_rgba(&bytes).map_err(|e| anyhow::anyhow!("{e}"))?;
            let mut resolved = false;
            // The maps the index lists first, then every other dump of the same size (a layer is
            // shared by maps that the index may not list all of)
            let mut candidates: Vec<u32> = layer.maps.iter().map(|&id| u32::from(id)).collect();
            let rest: Vec<u32> = (0..64).filter(|id| !candidates.contains(id)).collect();
            candidates.extend(rest);
            for id in candidates {
                let folder = dump.join(format!("map-{id:02}"));
                let size = ssd_hd::png::file_dimensions(&folder.join("layer.png"));
                if !matches!(size, Ok(size) if size == (sw, sh)) {
                    continue;
                }
                let Ok(picture) = std::fs::read(folder.join("layer.png")) else { continue };
                let Ok((w, h, rgba)) = ssd_hd::png::read_rgba(&picture) else { continue };
                if (w, h) != (sw, sh) || rgba != source {
                    continue;
                }
                let words = std::fs::read(folder.join("cgram.bin"))?;
                let mut colours = [ssd_hd::bgart::UNKNOWN_COLOUR; 256];
                for (at, pair) in words.as_chunks::<2>().0.iter().take(256).enumerate() {
                    colours[at] = u16::from_le_bytes(*pair) & 0x7FFF;
                }
                maps.insert(layer.id.clone(), ssd_hd::bgart::palette_text(&colours));
                writeln!(
                    out,
                    "refpal: map {:<12} from the dump of map {id} ({} colours)",
                    layer.id,
                    words.len() / 2
                )?;
                resolved = true;
                break;
            }
            if !resolved {
                writeln!(out, "refpal: map {:<12} NO dump has its picture", layer.id)?;
            }
        }
    }
    Ok((bg, maps))
}
