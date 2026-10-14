//! `gfx-log`: plays replays and lists the graphics streams the game loads, where their bytes go
//! and in which scene. The LZ decoder (`00:C559`) and its variant to the staging buffer
//! (`00:C5ED`) are breakpointed: each call names its stream (the source pointer), whose size and
//! consumed length come from the ROM. The DMA channels (`$43xx`), the DMA starts (`$420B`) and
//! the PPU's upload address writes (`$2115-$2117`, `$2121`, `$2102-$2103`) are followed, so a DMA
//! that sends on the bytes of a decoded stream names its destination (VRAM word, CGRAM entry or
//! OAM byte). The other decoders (`00:C707`, `00:C782`, `04:83B2`, `00:FDB5`) are counted only:
//! the title's maps, the Mode 7 flights and the in-place regroups have no stream of their own.
//! The catalogue is what `hd-convert` has to find in the ROM, and what `hd-coverage` checks.

use crate::shared::{read_replay, render_table};
use ssd_catalogue::{RomView, graphics};
use ssd_oracle::{NullAudio, Stopped, Stops};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;

/// The decoder entries, as 24-bit addresses.
const LZ: u32 = 0x00C559;
const LZ_TO_STAGING: u32 = 0x00C5ED;
const COUNTED: [(u32, &str); 4] = [
    (0x00C707, "regroup-in-place"),
    (0x00C782, "unpack-4bit"),
    (0x0483B2, "unpack-variant"),
    (0x00FDB5, "bit-tree"),
];

/// Where `00:C559` puts its output: `$7E:9000`.
const DECODE_BUFFER: u32 = 0x7E_9000;

/// How many decoded buffers stay in view for the DMAs that send them on.
const BUFFER_HISTORY: usize = 64;

/// The command line of `gfx-log`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    /// The replays to play (files, or directories of them); empty: `.work/replays`.
    pub paths: Vec<PathBuf>,
}

/// Parses the arguments after `gfx-log`: replays or directories, and no option.
///
/// # Errors
///
/// An option.
pub fn parse_args(args: &[&str]) -> Result<Args, String> {
    if let Some(option) = args.iter().find(|arg| arg.starts_with('-')) {
        return Err(format!("unknown option `{option}`"));
    }
    Ok(Args { paths: args.iter().map(PathBuf::from).collect() })
}

/// One stream over the replays: how it was loaded and where its bytes went.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Load {
    kind: &'static str,
    consumed: usize,
    decoded: usize,
    loads: u64,
    replays: Vec<String>,
    first: Option<(String, u64)>,
    scenes: Vec<String>,
    destinations: Vec<String>,
}

/// A DMA that sent `size` bytes from `source` to `target` (`VRAM`, `CGRAM`, `OAM` or the
/// register `OTHER`) at `at`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Dma {
    source: u32,
    size: usize,
    target: &'static str,
    at: u16,
}

/// The PPU upload state and the DMA channels, followed through a replay's CPU writes.
#[derive(Default)]
struct Uploads {
    /// The eight registers of each DMA channel (`$43x0` to `$43x7`).
    channels: [[u8; 8]; 8],
    vram_address: u16,
    cgram_address: u16,
    oam_address: u16,
}

impl Uploads {
    /// The DMAs the writes start, in order.
    fn feed(&mut self, writes: &[(u32, u8)]) -> Vec<Dma> {
        let mut dmas = Vec::new();
        for &(address, value) in writes {
            if ((address >> 16) & 0x7F) >= 0x40 {
                continue;
            }
            match address as u16 {
                0x2116 => self.vram_address = (self.vram_address & 0xFF00) | u16::from(value),
                0x2117 => {
                    self.vram_address = (self.vram_address & 0x00FF) | (u16::from(value) << 8);
                }
                0x2121 => self.cgram_address = u16::from(value),
                0x2102 => self.oam_address = (self.oam_address & 0xFF00) | u16::from(value),
                0x2103 => self.oam_address = (self.oam_address & 0x00FF) | (u16::from(value) << 8),
                off @ 0x4300..=0x437F => {
                    self.channels[usize::from((off - 0x4300) >> 4)][usize::from(off & 7)] = value;
                }
                0x420B if value != 0 => {
                    for (channel, registers) in self.channels.iter().enumerate() {
                        if value >> channel & 1 == 0 {
                            continue;
                        }
                        let size = match usize::from(registers[5]) | usize::from(registers[6]) << 8
                        {
                            0 => 0x10000,
                            n => n,
                        };
                        let source = u32::from(registers[2])
                            | u32::from(registers[3]) << 8
                            | u32::from(registers[4]) << 16;
                        let (target, at) = match 0x2100 + u16::from(registers[1]) {
                            0x2118 | 0x2119 => ("VRAM", self.vram_address),
                            0x2122 => ("CGRAM", self.cgram_address),
                            0x2104 => ("OAM", self.oam_address),
                            _ => ("OTHER", 0),
                        };
                        dmas.push(Dma { source, size, target, at });
                        if target == "VRAM" {
                            self.vram_address = self.vram_address.wrapping_add((size / 2) as u16);
                        }
                    }
                }
                _ => {}
            }
        }
        dmas
    }
}

/// The 24-bit pointer at direct page `offset` (low byte first, bank third). The direct page
/// is in bank 0, where the first `$2000` bytes of WRAM are mirrored.
fn pointer(wram: &[u8], dp: u16, offset: u8) -> u32 {
    let byte = |n: u8| u32::from(wram[usize::from(dp.wrapping_add(u16::from(n)) & 0x1FFF)]);
    byte(offset + 2) << 16 | byte(offset + 1) << 8 | byte(offset)
}

/// Plays `replays` and writes the catalogue of the streams they load (see the module
/// documentation): one row per stream.
///
/// # Errors
///
/// A replay cannot be read or played, or the ROM is not a `LoROM` image.
pub fn run(rom: &[u8], replays: &[PathBuf], out: &mut dyn Write) -> anyhow::Result<()> {
    let view = RomView::new(rom)?;
    let mut loads: BTreeMap<u32, Load> = BTreeMap::new();
    let mut counted: BTreeMap<&str, u64> = BTreeMap::new();
    for path in replays {
        let name = path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned());
        let replay = read_replay(path, rom)?;
        let mut machine = replay.power_on(rom.to_vec())?;
        machine.set_cpu_write_log(true);
        machine.set_breakpoint(LZ);
        machine.set_breakpoint(LZ_TO_STAGING);
        for (entry, _) in COUNTED {
            machine.set_breakpoint(entry);
        }
        let stops = Stops { frame_end: true, breakpoint: true, ..Stops::default() };
        let mut uploads = Uploads::default();
        // The decoded buffers of this replay, newest last: the stream key, start and size
        let mut buffers: Vec<(u32, usize, u32)> = Vec::new();
        while machine.frame_count() < replay.frames() {
            let stopped =
                machine.run(&stops, &mut |f| replay.input(f), &mut NullAudio, u64::MAX)?;
            let frame = machine.frame_count();
            let writes = machine.take_cpu_writes();
            for dma in uploads.feed(&writes) {
                let stream = buffers.iter().rev().find(|&&(start, size, _)| {
                    dma.source >= start && (dma.source - start) < size as u32
                });
                if let Some(&(_, _, key)) = stream {
                    let destination = format!("{} {:04X} x{:04X}", dma.target, dma.at, dma.size);
                    let load = loads.entry(key).or_default();
                    if !load.destinations.contains(&destination) {
                        load.destinations.push(destination);
                    }
                }
            }
            let Stopped::Breakpoint { .. } = stopped else { continue };
            let regs = machine.cpu_registers();
            let address = u32::from(regs.pbr) << 16 | u32::from(regs.pc);
            if let Some(&(_, kind)) = COUNTED.iter().find(|(entry, _)| *entry == address) {
                *counted.entry(kind).or_default() += 1;
                continue;
            }
            let (kind, target) =
                if address == LZ { ("LZ", DECODE_BUFFER) } else { ("LZ-staging", 0) };
            let wram = machine.wram();
            let source = pointer(wram, regs.d, 0x83);
            let scene = format!("game_mode {:02X} map {:02X}", wram[0x24], wram[0x1A0]);
            let Some(decoded) = view.offset(source).and_then(|at| graphics::lz_decode(rom, at))
            else {
                continue;
            };
            let target =
                if address == LZ_TO_STAGING { pointer(wram, regs.d, 0x86) } else { target };
            let load = loads.entry(source).or_default();
            load.kind = kind;
            load.consumed = decoded.consumed;
            load.decoded = decoded.data.len();
            load.loads += 1;
            if !load.replays.contains(&name) {
                load.replays.push(name.clone());
            }
            load.first.get_or_insert_with(|| (name.clone(), frame));
            if !load.scenes.contains(&scene) {
                load.scenes.push(scene);
            }
            buffers.push((target, decoded.data.len(), source));
            if buffers.len() > BUFFER_HISTORY {
                buffers.remove(0);
            }
        }
    }

    let header = [
        "stream",
        "kind",
        "consumed",
        "decoded",
        "loads",
        "replays",
        "first",
        "scenes",
        "destinations",
    ];
    let rows: Vec<Vec<String>> = loads
        .iter()
        .map(|(&source, load)| {
            vec![
                format!("{:02X}:{:04X}", source >> 16, source & 0xFFFF),
                load.kind.to_owned(),
                format!("{:04X}", load.consumed),
                format!("{:04X}", load.decoded),
                load.loads.to_string(),
                load.replays.len().to_string(),
                load.first
                    .as_ref()
                    .map_or_else(String::new, |(replay, frame)| format!("{replay} f{frame}")),
                load.scenes.join("; "),
                load.destinations.join("; "),
            ]
        })
        .collect();
    write!(out, "{}", render_table(&header, &rows))?;
    for (kind, calls) in &counted {
        writeln!(out, "gfx-log: {kind} decoder calls {calls}")?;
    }
    writeln!(out, "gfx-log: {} streams over {} replays", loads.len(), replays.len())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dma_start_names_its_source_size_and_target() {
        let mut uploads = Uploads::default();
        let writes = [
            (0x2116, 0x00),
            (0x2117, 0x30),
            (0x4301, 0x18),
            (0x4302, 0x00),
            (0x4303, 0x90),
            (0x4304, 0x7E),
            (0x4305, 0x40),
            (0x4306, 0x00),
            (0x420B, 0x01),
        ];
        let writes: Vec<(u32, u8)> = writes.iter().map(|&(a, v)| (a, v)).collect();
        let dmas = uploads.feed(&writes);
        assert_eq!(dmas, [Dma { source: 0x7E_9000, size: 0x40, target: "VRAM", at: 0x3000 }]);
        assert_eq!(uploads.vram_address, 0x3000 + 0x20, "the address moves by the words sent");
    }

    #[test]
    fn a_direct_page_pointer_reads_three_bytes_from_the_mirror() {
        let mut wram = vec![0_u8; 0x2000];
        wram[0x1083] = 0x34;
        wram[0x1084] = 0x12;
        wram[0x1085] = 0x16;
        assert_eq!(pointer(&wram, 0x1000, 0x83), 0x16_1234);
    }

    #[test]
    fn the_command_line_takes_replays_and_no_option() {
        assert_eq!(parse_args(&["a.ssdreplay"]).unwrap().paths, [PathBuf::from("a.ssdreplay")]);
        assert!(parse_args(&["--all"]).unwrap_err().contains("unknown option"));
    }
}
