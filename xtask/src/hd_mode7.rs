//! `hd-art m7scan` and `hd-art m7dump`: which Mode 7 planes the replays show, by content (the planes the
//! HD plane art is made of), and the whole image of one. Output goes to stdout or the folder named;
//! nothing here is tracked.

use crate::shared::read_replay;
use anyhow::Context;
use ssd_hd::mode7art::plane_key;
use ssd_oracle::{FrameLog, NullAudio};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Write;
use std::path::{Path, PathBuf};

const GAME_MODE: usize = 0x24;

/// The 8-bit colour of a 15-bit one.
fn expand(value: u16) -> [u8; 3] {
    let channel = |shift: u16| (((u32::from(value >> shift) & 31) * 255 + 15) / 31) as u8;
    [channel(0), channel(5), channel(10)]
}

/// Whether some line of the frame shows the Mode 7 plane as its top layer.
fn shows_plane(log: &FrameLog) -> bool {
    log.entries.iter().any(|e| !e.regs.forced_blanking && e.regs.bg_mode == 7 && e.regs.main_bg[0])
}

#[derive(Debug, Default)]
struct Seen {
    frames: u64,
    /// Frames of a plane that load VRAM while displayed.
    writes: u64,
    first: String,
    modes: BTreeMap<u8, u64>,
    palettes: BTreeMap<u64, (u64, String)>,
    /// The ranges of the matrix over the lines, and the lines that show the plane.
    a: (i16, i16),
    d: (i16, i16),
    lines: (u16, u16),
}

/// Plays the replays and prints each Mode 7 plane they show: its key, frames, game modes, palettes
/// and the ranges of the matrix.
pub fn scan(rom: &[u8], files: &[PathBuf], out: &mut dyn Write) -> anyhow::Result<u8> {
    let mut seen: BTreeMap<u64, Seen> = BTreeMap::new();
    for path in files {
        let name = path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned());
        let replay = read_replay(path, rom)?;
        let mut machine = replay.power_on(rom.to_vec())?;
        machine.set_line_log(true);
        while machine.frame_count() < replay.frames() {
            machine.run_frame_with(&mut |f| replay.input(f), &mut NullAudio)?;
            let number = machine.frame_count() - 1;
            let Some(log) = machine.frame_log() else { continue };
            if !shows_plane(log) {
                continue;
            }
            let entry = seen.entry(plane_key(&log.vram)).or_default();
            if entry.frames == 0 {
                entry.first = format!("{name}:{number}");
                entry.a = (i16::MAX, i16::MIN);
                entry.d = (i16::MAX, i16::MIN);
                entry.lines = (u16::MAX, 0);
            }
            entry.frames += 1;
            entry.writes += u64::from(!log.vram_writes.is_empty());
            *entry.modes.entry(machine.wram()[GAME_MODE]).or_default() += 1;
            let cg: Vec<u8> = log.cgram_start.iter().flat_map(|w| w.to_le_bytes()).collect();
            let hash = ssd_hd::pack::tile_key(&cg);
            entry.palettes.entry(hash).or_insert((0, format!("{name}:{number}"))).0 += 1;
            for e in log
                .entries
                .iter()
                .filter(|e| !e.regs.forced_blanking && e.regs.bg_mode == 7 && e.regs.main_bg[0])
            {
                let (a, d) = (e.regs.m7_a as i16, e.regs.m7_d as i16);
                entry.a = (entry.a.0.min(a), entry.a.1.max(a));
                entry.d = (entry.d.0.min(d), entry.d.1.max(d));
                entry.lines = (entry.lines.0.min(e.scanline), entry.lines.1.max(e.scanline));
            }
        }
        eprintln!("m7scan: {}", path.display());
    }
    for (key, s) in &seen {
        writeln!(
            out,
            "plane {key:016X}: {} frames ({} with VRAM writes), first {}, game modes {:?}, lines {}..{}, a {}..{}, d {}..{}",
            s.frames, s.writes, s.first, s.modes, s.lines.0, s.lines.1, s.a.0, s.a.1, s.d.0, s.d.1
        )?;
        for (hash, (frames, first)) in &s.palettes {
            writeln!(out, "    palette {hash:016X}: {frames} frames, first {first}")?;
        }
    }
    if seen.is_empty() {
        writeln!(out, "no frame shows a Mode 7 plane")?;
    }
    Ok(0)
}

/// `hd-art m7dump <replay> <frame> <out dir>`: writes the plane the frame shows as it is at 1x
/// (`plane-src.png`, 1024 x 1024, colour index 0 as the backdrop), its colour indexes (`plane-idx.png`,
/// grey), the palette (`palette.txt`, 256 lines of `RRGGBB`), the key (`key.txt`), the frame
/// (`frame.png`) and the first line of each matrix change.
pub fn dump(
    rom: &[u8],
    replay: &Path,
    frame: u64,
    out_dir: &Path,
    out: &mut dyn Write,
) -> anyhow::Result<u8> {
    std::fs::create_dir_all(out_dir)?;
    let file = read_replay(replay, rom)?;
    let mut machine = file.power_on(rom.to_vec())?;
    machine.set_line_log(true);
    while machine.frame_count() <= frame && machine.frame_count() < file.frames() {
        machine.run_frame_with(&mut |f| file.input(f), &mut NullAudio)?;
    }
    anyhow::ensure!(machine.frame_count() > frame, "the replay has no frame {frame}");
    let log = machine.frame_log().context("no render log")?;
    anyhow::ensure!(shows_plane(log), "frame {frame} shows no Mode 7 plane");
    dump_log(log, machine.wram()[GAME_MODE], out_dir, out)?;
    Ok(0)
}

/// What [`dump`] writes, for the frame `log` (`game_mode`, the game's mode byte, is printed with the
/// key); `shots --gaps` calls it for a saved moment that shows a plane.
pub fn dump_log(
    log: &FrameLog,
    game_mode: u8,
    out_dir: &Path,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(out_dir)?;
    let key = plane_key(&log.vram);
    writeln!(out, "key {key:016X}, game mode {game_mode:02X}")?;
    let backdrop = expand(log.cgram_start[0]);
    let mut src = Vec::with_capacity(1024 * 1024 * 4);
    let mut idx = Vec::with_capacity(1024 * 1024 * 4);
    for y in 0..1024_usize {
        for x in 0..1024_usize {
            let tile = usize::from(log.vram[(y / 8) * 128 + x / 8] & 0xFF);
            let index = (log.vram[(64 * tile + 8 * (y % 8) + x % 8) & 0x7FFF] >> 8) as u8;
            let rgb =
                if index == 0 { backdrop } else { expand(log.cgram_start[usize::from(index)]) };
            src.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
            idx.extend_from_slice(&[index, index, index, 255]);
        }
    }
    std::fs::write(out_dir.join("plane-src.png"), crate::shots::encode_png(1024, 1024, &src))?;
    std::fs::write(out_dir.join("plane-idx.png"), crate::shots::encode_png(1024, 1024, &idx))?;
    let mut palette = String::new();
    for word in &log.cgram_start[..256] {
        let c = expand(*word);
        writeln!(palette, "{:02X}{:02X}{:02X}", c[0], c[1], c[2])?;
    }
    std::fs::write(out_dir.join("palette.txt"), palette)?;
    std::fs::write(out_dir.join("key.txt"), format!("{key:016x}\n"))?;
    let mut renderer = ssd_hd::Renderer::new();
    if let Ok(image) = renderer.render(log) {
        let mut rgba = Vec::with_capacity(image.pixels.len() * 4);
        for c in &image.pixels {
            rgba.extend_from_slice(&[c.r, c.g, c.b, 255]);
        }
        std::fs::write(
            out_dir.join("frame.png"),
            crate::shots::encode_png(image.width as u32, image.height as u32, &rgba),
        )?;
    }
    let mut last = None;
    for e in log.entries.iter().filter(|e| !e.regs.forced_blanking && e.regs.bg_mode == 7) {
        let r = &e.regs;
        let now = (
            r.m7_a,
            r.m7_b,
            r.m7_c,
            r.m7_d,
            r.m7_h_scroll,
            r.m7_v_scroll,
            r.m7_center_x,
            r.m7_center_y,
        );
        if last != Some(now) && e.from_pixel == 0 && e.scanline % 16 == 1 {
            writeln!(
                out,
                "  line {:3}: a {:5} b {:5} c {:5} d {:5} scroll {:5},{:5} centre {:5},{:5} oob {} flip {}{}",
                e.scanline,
                r.m7_a as i16,
                r.m7_b as i16,
                r.m7_c as i16,
                r.m7_d as i16,
                r.m7_h_scroll,
                r.m7_v_scroll,
                r.m7_center_x,
                r.m7_center_y,
                r.m7_out_of_bounds,
                u8::from(r.m7_h_flip),
                u8::from(r.m7_v_flip),
            )?;
        }
        last = Some(now);
    }
    Ok(())
}
