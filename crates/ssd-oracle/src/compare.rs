//! What a routine call leaves behind, and how the original's and the Rust function's outcomes
//! differ (spec §5.4).

use crate::symbols::canonical_address;
use crate::{Machine, Registers, Symbols};
use std::collections::BTreeMap;
use std::fmt;
use std::ops::Range;

/// Differences a [`Divergence`] keeps; the others are only counted.
pub(crate) const KEPT_DIFFERENCES: usize = 256;

/// The state a call left, borrowed from the machine it ran on, so that comparing copies nothing.
#[derive(Debug, Clone)]
pub struct Outcome<'a> {
    pub regs: Registers,
    /// Work RAM, 128 KiB (`$7E:0000` is index 0).
    pub wram: &'a [u8],
    /// Cartridge SRAM, if the cartridge has any.
    pub sram: Option<&'a [u8]>,
    /// Video RAM, 32 Ki words.
    pub vram: &'a [u16],
    /// Palette RAM, 256 words.
    pub cgram: &'a [u16],
    /// OAM low table, 512 bytes.
    pub oam_low: &'a [u8],
    /// OAM high table, 32 bytes.
    pub oam_high: &'a [u8],
    /// The CPU writes the call made, in order, as `(24-bit address, value)` through whatever
    /// mirror the code used.
    pub cpu_writes: &'a [(u32, u8)],
}

impl<'a> Outcome<'a> {
    /// The current state of `machine`, with the CPU writes the call made on it.
    #[must_use]
    pub fn of(machine: &'a Machine, cpu_writes: &'a [(u32, u8)]) -> Self {
        let (oam_low, oam_high) = machine.oam();
        Self {
            regs: machine.cpu_registers(),
            wram: machine.wram(),
            sram: machine.sram(),
            vram: machine.vram(),
            cgram: machine.cgram(),
            oam_low,
            oam_high,
            cpu_writes,
        }
    }
}

/// One way the Rust outcome differs from the original's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Difference {
    /// A CPU register: `A`, `X`, `Y`, `S`, `D`, `DBR`, `PBR`, `PC` (full values), `P` (full
    /// values, reported when a flag in the entry's `flags_mask` differs) or `E` (the emulation
    /// flag, 0 or 1).
    Register { name: &'static str, original: u32, rust: u32 },
    /// A byte of `wram`, `sram`, `vram`, `cgram` or `oam`, by its byte address in that memory:
    /// WRAM offset 0 is `$7E:0000`; VRAM and CGRAM words are two bytes, low byte first (the
    /// low byte of word `w` is byte `2w`); OAM is the low table (`$000–$1FF`) followed by the
    /// high table (`$200–$21F`).
    Memory { space: &'static str, address: u32, original: u8, rust: u8 },
    /// The `index`-th CPU write to an externally observable register, as (bank-`$00` address,
    /// value); `None` when the other call made more such writes. See [`compare`].
    Write { index: usize, original: Option<(u32, u8)>, rust: Option<(u32, u8)> },
}

/// A `Verify` call whose Rust outcome differed from the original's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Divergence {
    /// The hook entry's name.
    pub routine: String,
    /// Index of the call among the routine's dispatched arrivals (0 = the first), sampled out
    /// or not.
    pub call_index: u64,
    /// [`Machine::frame_count`] at the call's entry.
    pub frame: u64,
    /// The first differences, in [`compare`]'s order (at most 256).
    pub differences: Vec<Difference>,
    /// Differences found beyond those kept.
    pub truncated: usize,
    /// The variable each kept WRAM difference falls in, by WRAM offset: its name, plus
    /// `+offset` past its first byte.
    pub labels: BTreeMap<u32, String>,
    /// Other findings about the call, one line each (e.g. its cost going beyond its bounds,
    /// see [`HookStats::cost_over_bound`](crate::HookStats::cost_over_bound)).
    pub notes: Vec<String>,
}

impl Divergence {
    /// A divergence keeping the first 256 `differences` (the rest are counted in `truncated`),
    /// with the WRAM ones named after the `[[variable]]` of `symbols` they fall in.
    #[must_use]
    pub fn new(
        routine: String,
        call_index: u64,
        frame: u64,
        mut differences: Vec<Difference>,
        symbols: Option<&Symbols>,
    ) -> Self {
        let truncated = keep_first(&mut differences);
        let labels = wram_labels(&differences, symbols);
        Self { routine, call_index, frame, differences, truncated, labels, notes: Vec::new() }
    }
}

/// Keeps the first 256 `differences`; returns how many were dropped.
pub(crate) fn keep_first(differences: &mut Vec<Difference>) -> usize {
    let truncated = differences.len().saturating_sub(KEPT_DIFFERENCES);
    differences.truncate(KEPT_DIFFERENCES);
    truncated
}

/// The `[[variable]]` of `symbols` each WRAM difference falls in, by WRAM offset: its name, plus
/// `+offset` past its first byte (none without symbols).
pub(crate) fn wram_labels(
    differences: &[Difference],
    symbols: Option<&Symbols>,
) -> BTreeMap<u32, String> {
    let Some(symbols) = symbols else { return BTreeMap::new() };
    differences
        .iter()
        .filter_map(|difference| match *difference {
            Difference::Memory { space: "wram", address, .. } => {
                let bus_address = 0x7E_0000 + address;
                let variable = symbols.variable_at(bus_address)?;
                let offset = bus_address - canonical_address(variable.address);
                let label = if offset == 0 {
                    variable.name.clone()
                } else {
                    format!("{}+{offset}", variable.name)
                };
                Some((address, label))
            }
            _ => None,
        })
        .collect()
}

/// A difference as text, without a newline: `wram[$7E:0010] counter: $05 != $06`, the original
/// (or reference) value first, WRAM bytes named after their entry of `labels`.
pub(crate) struct DifferenceText<'a> {
    pub difference: &'a Difference,
    pub labels: &'a BTreeMap<u32, String>,
}

impl fmt::Display for DifferenceText<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self.difference {
            Difference::Register { name: "P", original, rust } => write!(
                f,
                "P: ${original:02X} ({}) != ${rust:02X} ({})",
                flag_letters(original),
                flag_letters(rust)
            ),
            Difference::Register { name: name @ ("DBR" | "PBR" | "E"), original, rust } => {
                write!(f, "{name}: ${original:02X} != ${rust:02X}")
            }
            Difference::Register { name, original, rust } => {
                write!(f, "{name}: ${original:04X} != ${rust:04X}")
            }
            Difference::Memory { space: "wram", address, original, rust } => {
                write!(f, "wram[${:02X}:{:04X}]", 0x7E + (address >> 16), address & 0xFFFF)?;
                if let Some(label) = self.labels.get(&address) {
                    write!(f, " {label}")?;
                }
                write!(f, ": ${original:02X} != ${rust:02X}")
            }
            Difference::Memory { space, address, original, rust } => {
                write!(f, "{space}[${address:04X}]: ${original:02X} != ${rust:02X}")
            }
            Difference::Write { index, original, rust } => {
                write!(f, "write {index}: {} != {}", WriteText(original), WriteText(rust))
            }
        }
    }
}

/// A header line, then one line per kept difference (`leaf — wram[$7E:0010] counter: $05 !=
/// $06`, original value first), then the count of the others, then one line per note (`leaf —
/// note`); every line ends with a newline.
impl fmt::Display for Divergence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let routine = &self.routine;
        let total = self.differences.len() + self.truncated;
        let plural = if total == 1 { "" } else { "s" };
        writeln!(
            f,
            "{routine}: call {} at frame {}, {total} difference{plural}",
            self.call_index, self.frame
        )?;
        for difference in &self.differences {
            let text = DifferenceText { difference, labels: &self.labels };
            writeln!(f, "{routine} — {text}")?;
        }
        if self.truncated > 0 {
            writeln!(f, "{routine} — … {} more differences", self.truncated)?;
        }
        for note in &self.notes {
            writeln!(f, "{routine} — {note}")?;
        }
        Ok(())
    }
}

/// The flags of P as letters, upper case when set (`nvMXdiZc`).
fn flag_letters(p: u32) -> String {
    let letter = |(letter, bit): (char, u32)| {
        if (p >> bit) & 1 != 0 { letter } else { letter.to_ascii_lowercase() }
    };
    "NVMXDIZC".chars().zip((0..8).rev()).map(letter).collect()
}

/// `$00:2118 = $01`, or `none`.
struct WriteText(Option<(u32, u8)>);

impl fmt::Display for WriteText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some((address, value)) => {
                write!(f, "${:02X}:{:04X} = ${value:02X}", address >> 16, address & 0xFFFF)
            }
            None => f.write_str("none"),
        }
    }
}

/// Differences of `rust` from `original`, in this order:
///
/// - registers: A, X, Y, S, D, DBR, PBR and PC in full, P under `flags_mask` (bit order
///   NVMXDIZC), E;
/// - WRAM, then SRAM, by address;
/// - unless `hdma_crossed`: VRAM, CGRAM and OAM by byte address, then the CPU writes to the
///   externally observable registers (`$2100–$213F`, `$2140–$2143`, `$2180–$2183`, `$4016`,
///   `$4200`, `$4201`, `$4207–$420D`, `$4300–$437F`; not the CPU's own math registers
///   `$4202–$4206`), compared in order. A write through a mirror is a write to its register:
///   banks `$80–$BF` mirror `$00–$3F`, and `$2144–$217F` repeat the APU ports `$2140–$2143`.
///
/// A call crossed by HDMA (`hdma_crossed`) is compared on registers, WRAM and SRAM only: HDMA
/// writes registers on the original's machine, never on the Rust function's.
#[must_use]
pub fn compare(
    original: &Outcome<'_>,
    rust: &Outcome<'_>,
    flags_mask: u8,
    hdma_crossed: bool,
) -> Vec<Difference> {
    compare_except(original, rust, flags_mask, hdma_crossed, 0..0, usize::MAX)
}

/// [`compare`], except the WRAM bytes at the offsets in `dead_wram`, and stopping at the first
/// `limit` differences (the same ones [`compare`] lists first): a caller that only needs to know
/// whether the outcomes differ passes 1 and does not collect, nor scan, the rest.
pub(crate) fn compare_except(
    original: &Outcome<'_>,
    rust: &Outcome<'_>,
    flags_mask: u8,
    hdma_crossed: bool,
    dead_wram: Range<usize>,
    limit: usize,
) -> Vec<Difference> {
    let mut differences = Vec::new();
    compare_registers(&original.regs, &rust.regs, flags_mask, &mut differences);
    differences.truncate(limit);

    // WRAM on both sides of the dead bytes.
    let end = original.wram.len().min(rust.wram.len());
    let dead_start = dead_wram.start.min(end);
    let dead_end = dead_wram.end.clamp(dead_start, end);
    for range in [0..dead_start, dead_end..end] {
        let (o, r) = (&original.wram[range.clone()], &rust.wram[range.clone()]);
        compare_bytes("wram", range.start as u32, o, r, &mut differences, limit);
    }
    // Both machines have the same cartridge, so both have SRAM or neither does.
    if let (Some(original), Some(rust)) = (original.sram, rust.sram) {
        compare_bytes("sram", 0, original, rust, &mut differences, limit);
    }
    if hdma_crossed || differences.len() >= limit {
        return differences;
    }

    compare_words("vram", original.vram, rust.vram, &mut differences, limit);
    compare_words("cgram", original.cgram, rust.cgram, &mut differences, limit);
    compare_bytes("oam", 0, original.oam_low, rust.oam_low, &mut differences, limit);
    let high_start = original.oam_low.len() as u32;
    compare_bytes("oam", high_start, original.oam_high, rust.oam_high, &mut differences, limit);

    let original_writes: Vec<_> = observable_writes(original.cpu_writes).collect();
    let rust_writes: Vec<_> = observable_writes(rust.cpu_writes).collect();
    for index in 0..original_writes.len().max(rust_writes.len()) {
        if differences.len() >= limit {
            break;
        }
        let (original, rust) =
            (original_writes.get(index).copied(), rust_writes.get(index).copied());
        if original != rust {
            differences.push(Difference::Write { index, original, rust });
        }
    }
    differences
}

fn compare_registers(
    original: &Registers,
    rust: &Registers,
    flags_mask: u8,
    differences: &mut Vec<Difference>,
) {
    type Field = (&'static str, fn(&Registers) -> u32);
    const FIELDS: [Field; 10] = [
        ("A", |r| r.a.into()),
        ("X", |r| r.x.into()),
        ("Y", |r| r.y.into()),
        ("S", |r| r.s.into()),
        ("D", |r| r.d.into()),
        ("DBR", |r| r.dbr.into()),
        ("PBR", |r| r.pbr.into()),
        ("PC", |r| r.pc.into()),
        ("P", |r| u8::from(r.p).into()),
        ("E", |r| r.emulation_mode.into()),
    ];
    for (name, get) in FIELDS {
        let (original, rust) = (get(original), get(rust));
        let mask = if name == "P" { u32::from(flags_mask) } else { u32::MAX };
        if (original ^ rust) & mask != 0 {
            differences.push(Difference::Register { name, original, rust });
        }
    }
}

/// Byte differences, addressed from `start`, added while `differences` holds fewer than
/// `limit`; equal memories (the common case) cost one slice comparison.
fn compare_bytes(
    space: &'static str,
    start: u32,
    original: &[u8],
    rust: &[u8],
    differences: &mut Vec<Difference>,
    limit: usize,
) {
    if differences.len() >= limit || original == rust {
        return;
    }
    let room = limit - differences.len();
    differences.extend(
        original
            .iter()
            .zip(rust)
            .zip(start..)
            .filter(|((o, r), _)| o != r)
            .map(|((&original, &rust), address)| Difference::Memory {
                space,
                address,
                original,
                rust,
            })
            .take(room),
    );
}

/// Differences of word memories, byte by byte (low byte first), up to `limit` in all.
fn compare_words(
    space: &'static str,
    original: &[u16],
    rust: &[u16],
    differences: &mut Vec<Difference>,
    limit: usize,
) {
    if differences.len() >= limit || original == rust {
        return;
    }
    for ((&original, &rust), word) in original.iter().zip(rust).zip(0_u32..) {
        if differences.len() >= limit {
            return;
        }
        let (original, rust) = (original.to_le_bytes(), rust.to_le_bytes());
        compare_bytes(space, 2 * word, &original, &rust, differences, limit);
    }
}

/// The writes of `writes` to externally observable registers, as (bank-`$00` address, value).
fn observable_writes(writes: &[(u32, u8)]) -> impl Iterator<Item = (u32, u8)> + '_ {
    writes.iter().filter_map(|&(address, value)| {
        observable_register(address).map(|register| (u32::from(register), value))
    })
}

/// The externally observable register a 24-bit address reaches, through any mirror (see
/// [`compare`]), as its bank-`$00` address.
fn observable_register(address: u32) -> Option<u16> {
    let bank = (address >> 16) as u8 & 0x7F;
    if bank > 0x3F {
        return None;
    }
    let register = match address as u16 {
        offset @ 0x2144..=0x217F => 0x2140 | (offset & 3),
        offset => offset,
    };
    matches!(
        register,
        0x2100..=0x2143 | 0x2180..=0x2183 | 0x4016 | 0x4200 | 0x4201 | 0x4207..=0x420D | 0x4300..=0x437F
    )
    .then_some(register)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StatusRegister;

    fn registers(p: u8) -> Registers {
        Registers {
            a: 0x1234,
            x: 0x0010,
            y: 0x0020,
            s: 0x1FF0,
            d: 0x0000,
            pbr: 0x00,
            pc: 0x800A,
            dbr: 0x7E,
            p: StatusRegister::from(p),
            emulation_mode: false,
        }
    }

    fn outcome<'a>(regs: Registers, wram: &'a [u8], sram: Option<&'a [u8]>) -> Outcome<'a> {
        Outcome {
            regs,
            wram,
            sram,
            vram: &[],
            cgram: &[],
            oam_low: &[],
            oam_high: &[],
            cpu_writes: &[],
        }
    }

    #[test]
    fn identical_outcomes_have_no_differences() {
        let (wram, sram) = (vec![7_u8; 64], vec![1_u8; 16]);
        let a = outcome(registers(0x30), &wram, Some(&sram));
        let b = outcome(registers(0x30), &wram, Some(&sram));
        assert_eq!(compare(&a, &b, 0xFF, false), []);
    }

    #[test]
    fn registers_differ_in_full_and_flags_under_the_mask() {
        let wram = [0_u8; 4];
        let original = outcome(registers(0x30), &wram, None);
        let mut regs = registers(0x30 | 0x02); // Z set
        regs.a = 0x1235;
        regs.pc = 0x800B;
        regs.emulation_mode = true;
        let rust = outcome(regs, &wram, None);

        assert_eq!(
            compare(&original, &rust, 0xFF, false),
            [
                Difference::Register { name: "A", original: 0x1234, rust: 0x1235 },
                Difference::Register { name: "PC", original: 0x800A, rust: 0x800B },
                Difference::Register { name: "P", original: 0x30, rust: 0x32 },
                Difference::Register { name: "E", original: 0, rust: 1 },
            ]
        );
        let masked = compare(&original, &rust, !0x02, false);
        assert!(
            !masked.iter().any(|d| matches!(d, Difference::Register { name: "P", .. })),
            "Z is not compared: {masked:?}"
        );
        assert_eq!(masked.len(), 3);
    }

    #[test]
    fn memory_differences_are_listed_by_offset() {
        let original_wram = [0_u8, 1, 2, 3, 4, 5];
        let rust_wram = [0_u8, 9, 2, 3, 4, 8];
        let (original_sram, rust_sram) = ([0xAA_u8, 0xBB], [0xAA_u8, 0xBC]);
        let original = outcome(registers(0x30), &original_wram, Some(&original_sram));
        let rust = outcome(registers(0x30), &rust_wram, Some(&rust_sram));

        assert_eq!(
            compare(&original, &rust, 0xFF, false),
            [
                Difference::Memory { space: "wram", address: 1, original: 1, rust: 9 },
                Difference::Memory { space: "wram", address: 5, original: 5, rust: 8 },
                Difference::Memory { space: "sram", address: 1, original: 0xBB, rust: 0xBC },
            ]
        );
    }

    #[test]
    fn a_limit_keeps_the_first_differences_of_the_full_list() {
        // One difference or more in every part of the comparison, in its order.
        let (original_wram, rust_wram) = ([0_u8, 1, 2, 3], [0_u8, 9, 2, 8]);
        let (original_sram, rust_sram) = ([0xAA_u8, 0xBB], [0xAB_u8, 0xBB]);
        let (original_vram, rust_vram) = ([0_u16, 0x1100], [0_u16, 0x1200]);
        let (original_cgram, rust_cgram) = ([0x22_u16], [0x32_u16]);
        let (original_oam, rust_oam) = ([1_u8, 2], [1_u8, 3]);
        let (original_high, rust_high) = ([4_u8], [5_u8]);
        let original = Outcome {
            regs: registers(0x30),
            wram: &original_wram,
            sram: Some(&original_sram),
            vram: &original_vram,
            cgram: &original_cgram,
            oam_low: &original_oam,
            oam_high: &original_high,
            cpu_writes: &[(0x00_2100, 0x0F)],
        };
        let mut regs = registers(0x30);
        regs.a = 0x1235;
        let rust = Outcome {
            regs,
            wram: &rust_wram,
            sram: Some(&rust_sram),
            vram: &rust_vram,
            cgram: &rust_cgram,
            oam_low: &rust_oam,
            oam_high: &rust_high,
            cpu_writes: &[(0x00_2100, 0x0E)],
        };

        let all = compare_except(&original, &rust, 0xFF, false, 0..0, usize::MAX);
        let spaces: Vec<&str> = all
            .iter()
            .map(|difference| match difference {
                Difference::Register { .. } => "register",
                Difference::Memory { space, .. } => space,
                Difference::Write { .. } => "write",
            })
            .collect();
        assert_eq!(
            spaces,
            ["register", "wram", "wram", "sram", "vram", "cgram", "oam", "oam", "write"]
        );
        for limit in 0..=all.len() + 2 {
            let kept = compare_except(&original, &rust, 0xFF, false, 0..0, limit);
            assert_eq!(kept, all[..limit.min(all.len())], "limit {limit}");
        }
        // HDMA crossing the call still leaves out the PPU memories and the writes.
        let crossed = compare_except(&original, &rust, 0xFF, true, 0..0, usize::MAX);
        assert_eq!(crossed, all[..4]);
        assert_eq!(compare_except(&original, &rust, 0xFF, true, 0..0, 2), all[..2]);
    }

    #[test]
    fn dead_wram_is_skipped_and_its_neighbours_are_not() {
        let original_wram = [0_u8; 8];
        let rust_wram = [1_u8; 8];
        let original = outcome(registers(0x30), &original_wram, None);
        let rust = outcome(registers(0x30), &rust_wram, None);
        let addresses = |dead: Range<usize>| -> Vec<u32> {
            compare_except(&original, &rust, 0xFF, false, dead, usize::MAX)
                .into_iter()
                .map(|d| match d {
                    Difference::Memory { address, .. } => address,
                    other => panic!("{other:?}"),
                })
                .collect()
        };
        assert_eq!(addresses(2..5), [0, 1, 5, 6, 7]);
        assert_eq!(addresses(0..0), [0, 1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(addresses(6..100), [0, 1, 2, 3, 4, 5], "clamped to the memory");
    }

    #[test]
    fn observable_registers_and_their_mirrors() {
        for (address, register) in [
            (0x00_2100, Some(0x2100)),
            (0x80_213F, Some(0x213F)),
            (0x3F_2140, Some(0x2140)),
            (0x00_217F, Some(0x2143)),
            (0x00_2180, Some(0x2180)),
            (0x00_2183, Some(0x2183)),
            (0x00_4016, Some(0x4016)),
            (0x00_4200, Some(0x4200)),
            (0x00_4201, Some(0x4201)),
            (0xBF_4207, Some(0x4207)),
            (0x00_420D, Some(0x420D)),
            (0x00_4300, Some(0x4300)),
            (0x00_437F, Some(0x437F)),
            (0x00_2184, None),
            (0x00_4017, None),
            (0x00_4202, None),
            (0x00_4206, None),
            (0x00_420E, None),
            (0x00_4380, None),
            (0x40_2100, None),
            (0x7E_2100, None),
            (0xC0_4200, None),
        ] {
            assert_eq!(observable_register(address), register, "${address:06X}");
        }
    }
}
