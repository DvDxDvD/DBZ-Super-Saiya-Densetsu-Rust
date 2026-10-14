//! `hd-parity`: plays replays with the PPU's render log on, re-renders every frame with the
//! `ssd-hd` renderer (1x, no art) and compares it with the PPU's own frame, pixel for pixel. The
//! Enhanced picture at scale 1 is compared the same way (it is the same frame by construction).
//! Each replay is also played with the log off, to show the renderer only reads: the machine's
//! state and the frames come out identical. Reports, per replay, the frames checked, the frames
//! and pixels that differ and the first difference (frame, pixel, both colors), the PPU features
//! the replay used, the mean render time, and whether the state and frame checksums agree.

use crate::shared::{read_replay, render_table};
use anyhow::Context;
use ssd_hd::{HdOptions, Look, Renderer, Unsupported};
use ssd_oracle::{Color, FrameLog, NullAudio};
use std::fmt::Write as _;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// The command line of `hd-parity`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    /// Replay files, or directories of them; empty: `.work/replays`.
    pub paths: Vec<PathBuf>,
    /// Stop each replay after this many frames.
    pub frames: Option<u64>,
    /// Replays played at once.
    pub jobs: usize,
}

/// Parses the arguments after `hd-parity`: `[--frames N] [--jobs N] [path...]`.
///
/// # Errors
///
/// An unknown option, or an option without a number.
pub fn parse_args(args: &[&str]) -> Result<Args, String> {
    let mut parsed = Args { paths: Vec::new(), frames: None, jobs: 4 };
    let mut args = args.iter();
    while let Some(&arg) = args.next() {
        match arg {
            "--frames" | "--jobs" => {
                let number = args
                    .next()
                    .and_then(|value| value.parse::<u64>().ok())
                    .filter(|&n| n > 0)
                    .ok_or_else(|| format!("{arg} needs a number"))?;
                if arg == "--frames" {
                    parsed.frames = Some(number);
                } else {
                    parsed.jobs = number as usize;
                }
            }
            option if option.starts_with('-') => return Err(format!("unknown option `{option}`")),
            path => parsed.paths.push(PathBuf::from(path)),
        }
    }
    Ok(parsed)
}

/// The first pixel that differs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Difference {
    pub frame: u64,
    pub x: usize,
    pub y: usize,
    pub ppu: [u8; 3],
    pub hd: [u8; 3],
    /// First and last row of the frame that differs.
    pub rows: (usize, usize),
}

/// Frames of a replay that used a PPU feature.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Features {
    pub mode_0: u64,
    pub mode_1: u64,
    pub mode_7: u64,
    pub mosaic: u64,
    pub windows: u64,
    pub color_math: u64,
    pub sub_screen: u64,
    pub mid_line: u64,
    pub cgram_writes: u64,
    pub vram_writes: u64,
    pub oam_writes: u64,
    pub obsel_changes: u64,
    pub forced_blank_lines: u64,
}

impl Features {
    fn note(&mut self, log: &FrameLog) {
        let entries = &log.entries;
        let any =
            |test: &dyn Fn(&ssd_oracle::LineEntry) -> bool| u64::from(entries.iter().any(test));
        self.mode_0 += any(&|e| e.regs.bg_mode == 0);
        self.mode_1 += any(&|e| e.regs.bg_mode == 1);
        self.mode_7 += any(&|e| e.regs.bg_mode == 7);
        self.mosaic += any(&|e| e.regs.mosaic_bg.contains(&true));
        self.windows += any(&|e| {
            let r = &e.regs;
            r.bg_window_area.iter().flatten().any(|&a| a != 0)
                || r.obj_window_area.iter().any(|&a| a != 0)
                || r.math_window_area.iter().any(|&a| a != 0)
        });
        self.color_math += any(&|e| {
            let r = &e.regs;
            r.color_math != 0
                && (r.backdrop_color_math || r.obj_color_math || r.bg_color_math.contains(&true))
        });
        self.sub_screen += any(&|e| e.regs.sub_screen_layers);
        self.mid_line += any(&|e| e.from_pixel != 0);
        self.cgram_writes += u64::from(!log.cgram_writes.is_empty());
        self.vram_writes += u64::from(!log.vram_writes.is_empty());
        self.oam_writes += u64::from(!log.oam_writes.is_empty());
        self.obsel_changes += u64::from(entries.windows(2).any(|pair| {
            pair[0].sprites.fetch_size != pair[1].sprites.fetch_size
                || pair[0].sprites.fetch_base != pair[1].sprites.fetch_base
        }));
        self.forced_blank_lines += u64::from(entries.iter().any(|e| e.regs.forced_blanking));
    }
}

/// What checking one replay found.
#[derive(Debug, Clone, Default)]
pub struct Outcome {
    pub name: String,
    pub frames: u64,
    pub checked: u64,
    pub differing_frames: u64,
    pub differing_pixels: u64,
    pub first: Option<Difference>,
    /// Frames the renderer refused, and the first reason.
    pub unsupported: u64,
    pub unsupported_first: Option<(u64, Unsupported)>,
    /// Frames whose Enhanced picture at scale 1 is not the PPU's frame, and the pixels that differ.
    pub enhanced_differing_frames: u64,
    pub enhanced_differing_pixels: u64,
    pub features: Features,
    pub render: Duration,
    /// Final machine state checksum and checksum of the chain of frames, log off and on.
    pub plain: (u32, u32),
    pub with_log: (u32, u32),
}

impl Outcome {
    /// Every frame rendered and equal, and the machine unchanged by the log.
    #[must_use]
    pub fn green(&self) -> bool {
        self.checked == self.frames
            && self.differing_frames == 0
            && self.unsupported == 0
            && self.enhanced_differing_frames == 0
            && self.plain == self.with_log
    }
}

fn frame_crc(pixels: &[Color]) -> u32 {
    let bytes: Vec<u8> = pixels.iter().flat_map(|c| [c.r, c.g, c.b, c.a]).collect();
    ssd_rom::crc32(&bytes)
}

/// Plays `path` twice, log off and on.
///
/// # Errors
///
/// The replay cannot be read or played.
pub fn check_replay(rom: &[u8], path: &Path, frame_limit: Option<u64>) -> anyhow::Result<Outcome> {
    let replay = read_replay(path, rom)?;
    let frames = replay.frames().min(frame_limit.unwrap_or(u64::MAX));
    let name = path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    let mut outcome = Outcome { name, frames, ..Outcome::default() };

    // Log off: the reference run
    let mut machine = replay.power_on(rom.to_vec())?;
    let mut chain = 0_u32;
    while machine.frame_count() < frames {
        machine.run_frame_with(&mut |f| replay.input(f), &mut NullAudio)?;
        chain = chain.rotate_left(5) ^ frame_crc(machine.frame().pixels);
    }
    outcome.plain = (ssd_rom::crc32(&machine.save_state()), chain);

    // Log on, every frame re-rendered and compared
    let mut machine = replay.power_on(rom.to_vec())?;
    machine.set_line_log(true);
    let mut renderer = Renderer::new();
    let mut chain = 0_u32;
    while machine.frame_count() < frames {
        machine.run_frame_with(&mut |f| replay.input(f), &mut NullAudio)?;
        let frame_number = machine.frame_count() - 1;
        let ppu = machine.frame();
        chain = chain.rotate_left(5) ^ frame_crc(ppu.pixels);
        let Some(log) = machine.frame_log() else {
            outcome.unsupported += 1;
            outcome.unsupported_first.get_or_insert((frame_number, Unsupported::Incomplete));
            continue;
        };
        outcome.features.note(log);
        let started = Instant::now();
        let rendered = renderer.render(log);
        outcome.render += started.elapsed();
        match rendered {
            Err(reason) => {
                outcome.unsupported += 1;
                outcome.unsupported_first.get_or_insert((frame_number, reason));
            }
            Ok(frame) => {
                outcome.checked += 1;
                let scale_one = HdOptions { scale: 1, look: Look::Crisp };
                match renderer.render_hd(log, scale_one) {
                    Ok(picture) => {
                        let differing = picture
                            .rgba
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .zip(ppu.pixels)
                            .filter(|(hd, ppu)| hd[..3] != [ppu.r, ppu.g, ppu.b])
                            .count() as u64;
                        outcome.enhanced_differing_pixels += differing;
                        outcome.enhanced_differing_frames += u64::from(differing > 0);
                    }
                    Err(_) => outcome.enhanced_differing_frames += 1,
                }
                let same_size = frame.width as u32 == ppu.size.width
                    && frame.height as u32 == ppu.size.height
                    && frame.pixels.len() == ppu.pixels.len();
                if !same_size {
                    outcome.differing_frames += 1;
                    outcome.differing_pixels += frame.pixels.len().max(ppu.pixels.len()) as u64;
                    outcome.first.get_or_insert(Difference {
                        frame: frame_number,
                        x: 0,
                        y: 0,
                        ppu: [0; 3],
                        hd: [0; 3],
                        rows: (0, 0),
                    });
                    continue;
                }
                let mut differing = 0_u64;
                let mut rows = (usize::MAX, 0);
                let mut first_pixel = None;
                for (index, (hd, ppu)) in frame.pixels.iter().zip(ppu.pixels).enumerate() {
                    if hd != ppu {
                        differing += 1;
                        let (x, y) = (index % frame.width, index / frame.width);
                        rows = (rows.0.min(y), rows.1.max(y));
                        first_pixel.get_or_insert((
                            x,
                            y,
                            [ppu.r, ppu.g, ppu.b],
                            [hd.r, hd.g, hd.b],
                        ));
                    }
                }
                if let Some((x, y, ppu_color, hd_color)) = first_pixel {
                    if outcome.first.is_none() {
                        outcome.first = Some(Difference {
                            frame: frame_number,
                            x,
                            y,
                            ppu: ppu_color,
                            hd: hd_color,
                            rows,
                        });
                    }
                    if std::env::var("SSD_HD_EXPLAIN").is_ok_and(|v| v == frame_number.to_string())
                    {
                        explain(log, ppu.pixels, &frame.pixels);
                    }
                }
                outcome.differing_pixels += differing;
                outcome.differing_frames += u64::from(differing > 0);
            }
        }
    }
    outcome.with_log = (ssd_rom::crc32(&machine.save_state()), chain);
    Ok(outcome)
}

/// Debugging aid (`SSD_HD_EXPLAIN=<frame>`): the partial renders of the frame, and the rows and
/// columns that differ.
fn explain(log: &FrameLog, ppu: &[Color], hd: &[Color]) {
    eprintln!("explain: {} entries, {} CGRAM writes", log.entries.len(), log.cgram_writes.len());
    for entry in log.entries.iter().filter(|e| e.from_pixel != 0) {
        let r = &entry.regs;
        eprintln!(
            "partial: line {} from {} blank {} brightness {}",
            entry.scanline, entry.from_pixel, r.forced_blanking, r.brightness
        );
    }
    for y in 0..hd.len() / 256 {
        let row = y * 256..(y + 1) * 256;
        let xs: Vec<usize> = row.clone().filter(|&i| ppu[i] != hd[i]).map(|i| i % 256).collect();
        if let (Some(first), Some(last)) = (xs.first(), xs.last()) {
            let entry = &log.entries.iter().find(|e| usize::from(e.scanline) == y + 1);
            eprintln!(
                "row {y}: {} pixels, x {first}..={last}, ppu #{:02X}{:02X}{:02X} hd #{:02X}{:02X}{:02X}, offset {:?}",
                xs.len(),
                ppu[y * 256 + first].r,
                ppu[y * 256 + first].g,
                ppu[y * 256 + first].b,
                hd[y * 256 + first].r,
                hd[y * 256 + first].g,
                hd[y * 256 + first].b,
                entry.map(|e| e.regs.mosaic_v_offset)
            );
        }
    }
}

/// The replays named by `paths` (files, or directories of `*.ssdreplay`), sorted.
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
pub fn run(rom: &[u8], default_dir: &Path, args: &Args, out: &mut dyn Write) -> anyhow::Result<u8> {
    let paths =
        if args.paths.is_empty() { vec![default_dir.to_path_buf()] } else { args.paths.clone() };
    let files = replay_files(&paths)?;
    anyhow::ensure!(!files.is_empty(), "no replay to check");
    writeln!(out, "hd-parity: {} replays, {} at a time", files.len(), args.jobs.min(files.len()))?;

    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<(usize, anyhow::Result<Outcome>)>> = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..args.jobs.min(files.len()) {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(path) = files.get(index) else { break };
                    let outcome = check_replay(rom, path, args.frames)
                        .with_context(|| format!("checking {}", path.display()));
                    results.lock().expect("a worker panicked").push((index, outcome));
                }
            });
        }
    });
    let mut results = results.into_inner().expect("a worker panicked");
    results.sort_by_key(|(index, _)| *index);

    let mut rows = Vec::new();
    let mut green = true;
    let mut details = String::new();
    let mut total = Outcome::default();
    for (_, outcome) in results {
        let outcome = outcome?;
        green &= outcome.green();
        let per_frame = if outcome.checked == 0 {
            0.0
        } else {
            outcome.render.as_secs_f64() * 1000.0 / outcome.checked as f64
        };
        rows.push(vec![
            outcome.name.clone(),
            outcome.frames.to_string(),
            outcome.checked.to_string(),
            outcome.differing_frames.to_string(),
            outcome.differing_pixels.to_string(),
            outcome.unsupported.to_string(),
            outcome.enhanced_differing_frames.to_string(),
            format!("{per_frame:.2}"),
            if outcome.plain == outcome.with_log { "same" } else { "DIFFERENT" }.to_owned(),
        ]);
        if let Some(first) = outcome.first {
            let (ppu, hd) = (first.ppu, first.hd);
            let _ = writeln!(
                details,
                "  {}: first difference at frame {} pixel ({}, {}): ppu #{:02X}{:02X}{:02X}, hd #{:02X}{:02X}{:02X}",
                outcome.name,
                first.frame,
                first.x,
                first.y,
                ppu[0],
                ppu[1],
                ppu[2],
                hd[0],
                hd[1],
                hd[2]
            );
        }
        if let Some((frame, reason)) = outcome.unsupported_first {
            let _ = writeln!(
                details,
                "  {}: {} frames not rendered, the first is frame {frame}: {reason}",
                outcome.name, outcome.unsupported
            );
        }
        if outcome.plain != outcome.with_log {
            let (off, on) = (outcome.plain, outcome.with_log);
            let _ = writeln!(
                details,
                "  {}: state/frames CRC log off {:08X}/{:08X}, log on {:08X}/{:08X}",
                outcome.name, off.0, off.1, on.0, on.1
            );
        }
        total.frames += outcome.frames;
        total.checked += outcome.checked;
        total.differing_frames += outcome.differing_frames;
        total.differing_pixels += outcome.differing_pixels;
        total.unsupported += outcome.unsupported;
        total.enhanced_differing_frames += outcome.enhanced_differing_frames;
        total.enhanced_differing_pixels += outcome.enhanced_differing_pixels;
        total.render += outcome.render;
        let (t, f) = (&mut total.features, outcome.features);
        t.mode_0 += f.mode_0;
        t.mode_1 += f.mode_1;
        t.mode_7 += f.mode_7;
        t.mosaic += f.mosaic;
        t.windows += f.windows;
        t.color_math += f.color_math;
        t.sub_screen += f.sub_screen;
        t.mid_line += f.mid_line;
        t.cgram_writes += f.cgram_writes;
        t.vram_writes += f.vram_writes;
        t.oam_writes += f.oam_writes;
        t.obsel_changes += f.obsel_changes;
        t.forced_blank_lines += f.forced_blank_lines;
    }
    let header = [
        "replay",
        "frames",
        "checked",
        "differ",
        "pixels",
        "refused",
        "enhanced 1x",
        "ms/frame",
        "state+frames",
    ];
    write!(out, "{}", render_table(&header, &rows))?;
    write!(out, "{details}")?;
    let f = total.features;
    writeln!(
        out,
        "hd-parity: {} frames, {} checked, {} differ ({} pixels), {} refused; mean render {:.2} ms/frame",
        total.frames,
        total.checked,
        total.differing_frames,
        total.differing_pixels,
        total.unsupported,
        total.render.as_secs_f64() * 1000.0 / total.checked.max(1) as f64
    )?;
    writeln!(
        out,
        "hd-parity: enhanced at 1x: {} frames differ ({} pixels)",
        total.enhanced_differing_frames, total.enhanced_differing_pixels
    )?;
    writeln!(
        out,
        "hd-parity: frames using: mode 0 {}, mode 1 {}, mode 7 {}, mosaic {}, windows {}, color math {}, sub screen {}, mid-line writes {}, CGRAM writes {}, VRAM writes {}, OAM writes {}, object-register changes {}, forced blank {}",
        f.mode_0,
        f.mode_1,
        f.mode_7,
        f.mosaic,
        f.windows,
        f.color_math,
        f.sub_screen,
        f.mid_line,
        f.cgram_writes,
        f.vram_writes,
        f.oam_writes,
        f.obsel_changes,
        f.forced_blank_lines
    )?;
    writeln!(out, "hd-parity: {}", if green { "green" } else { "NOT green" })?;
    Ok(u8::from(!green))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_line_takes_limits_and_paths() {
        let args = parse_args(&["--frames", "100", "a.ssdreplay", "--jobs", "2", "dir"]).unwrap();
        assert_eq!(args.frames, Some(100));
        assert_eq!(args.jobs, 2);
        assert_eq!(args.paths, [PathBuf::from("a.ssdreplay"), PathBuf::from("dir")]);
        let default = parse_args(&[]).unwrap();
        assert_eq!((default.frames, default.jobs, default.paths.len()), (None, 4, 0));
    }

    #[test]
    fn a_bad_command_line_says_why() {
        assert!(parse_args(&["--frames"]).unwrap_err().contains("needs a number"));
        assert!(parse_args(&["--jobs", "0"]).unwrap_err().contains("needs a number"));
        assert!(parse_args(&["--fast"]).unwrap_err().contains("unknown option"));
    }

    #[test]
    fn an_outcome_is_green_only_when_every_frame_is_equal_and_the_machine_unchanged() {
        let green = Outcome { frames: 10, checked: 10, ..Outcome::default() };
        assert!(green.green());
        assert!(!Outcome { differing_frames: 1, ..green.clone() }.green());
        assert!(!Outcome { checked: 9, unsupported: 1, ..green.clone() }.green());
        assert!(!Outcome { enhanced_differing_frames: 1, ..green.clone() }.green());
        assert!(!Outcome { with_log: (1, 0), ..green }.green());
    }
}
