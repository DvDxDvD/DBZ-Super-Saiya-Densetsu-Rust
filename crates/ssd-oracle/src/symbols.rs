//! `re/symbols.toml`: names, addresses and hook settings of the game's routines and variables
//! (spec §5.2). Retail addresses live in that file; Rust code refers to routines by name.
//!
//! ```toml
//! [[routine]]
//! name = "example_routine"  # Rust function name in ssd-game's registry
//! address = 0x018000        # physical: banks $00-$1F, offsets $8000-$FFFF
//! returns = "rtl"           # "rts" | "rtl"
//! timing = "pure"           # "pure" | "mmio" | "sync"
//! mode = "verify"           # "original" | "rust" | "verify"
//! entry_m8 = true           # optional
//! entry_x8 = false          # optional
//! flags = "NZ"              # optional: compared flags (default all, "" none); NVMXDIZC
//! verify_sample = 1         # optional, default 1 (every call)
//! cycles = 108              # optional: CPU-busy master cycles charged in Rust mode
//! budget_frames = 2         # optional, default 2: frames a Verify call may take (longer routines)
//!
//! [[table]]                 # a read-only region of the ROM (WRAM data is a variable)
//! name = "example_table"
//! address = 0x0D8200        # physical: banks $00-$1F, offsets $8000-$FFFF; ends in its bank
//! size = 128                # bytes
//!
//! [[variable]]
//! name = "example_flag"
//! address = 0x7E0100
//! size = 1                  # optional, default 1
//!
//! [[sync_point]]            # where lockstep compares machines (first arrival after each NMI)
//! name = "example_idle"
//! address = 0x008100        # physical code address, like a routine's; not a routine's
//! ```
//!
//! Names are unique across routines, tables, variables and sync points. `ssd-game`'s build script
//! turns every table and variable into constants (it reads the same file with its own small
//! struct).

use crate::{HookMode, ReturnKind, hook_key};
use serde::Deserialize;
use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use toml::Spanned;

/// Flag letters of the status register, from bit 7 to bit 0.
const FLAG_LETTERS: &str = "NVMXDIZC";
/// The most frames a routine may ask for as its `Verify` budget (a bound against a typo).
const MAX_BUDGET_FRAMES: u64 = 600;

/// Timing class of a routine (spec §5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Timing {
    /// No MMIO.
    Pure,
    /// Touches hardware registers but needs no time to pass.
    Mmio,
    /// Waits for vblank, DMA, the APU or the joypad, or polls hardware.
    Sync,
}

/// A `[[routine]]` entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Routine {
    /// Name of the Rust function in `ssd-game`'s registry (`[a-z_][a-z0-9_]*`).
    pub name: String,
    /// Physical code address `PBR << 16 | PC` in the ROM: banks `$00–$1F`, offsets
    /// `$8000–$FFFF` (a hook key).
    pub address: u32,
    pub returns: ReturnKind,
    pub timing: Timing,
    pub mode: HookMode,
    /// Required accumulator width at entry (`Some(true)` = 8-bit); `None` accepts both.
    pub entry_m8: Option<bool>,
    /// Required index width at entry (`Some(true)` = 8-bit); `None` accepts both.
    pub entry_x8: Option<bool>,
    /// Flags compared in Verify, bit order NVMXDIZC: those of `flags`, all when it is absent,
    /// none when it is `""`.
    pub flags_mask: u8,
    /// Verify checks one call in `verify_sample` (at least 1).
    pub verify_sample: u32,
    /// CPU-busy master cycles charged per call in Rust mode.
    pub cycles: Option<u64>,
    /// Frames of master cycles the original of a `Verify` call may take before it counts as not
    /// returned (`None`: 2). For routines that legitimately run longer. `Rust` mode is unchanged.
    pub budget_frames: Option<u64>,
}

/// A `[[table]]` entry: a region of the ROM that the game reads as data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    pub name: String,
    /// Physical address of its first byte: banks `$00-$1F`, offsets `$8000-$FFFF`.
    pub address: u32,
    /// Size in bytes (at least 1); the table ends in its bank.
    pub size: u32,
}

/// A `[[variable]]` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variable {
    pub name: String,
    /// 24-bit bus address of its first byte.
    pub address: u32,
    /// Size in bytes (at least 1).
    pub size: u32,
}

/// A `[[sync_point]]` entry: a code address where lockstep compares two machines, at the first
/// arrival after each NMI (see [`Machine::run_until_sync`](crate::Machine::run_until_sync)).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncPoint {
    pub name: String,
    /// Physical code address `PBR << 16 | PC` in the ROM: banks `$00–$1F`, offsets
    /// `$8000–$FFFF` (a hook key).
    pub address: u32,
}

/// The contents of a symbols file, in file order. Names are unique across routines, tables,
/// variables and sync points; routines and sync points each have a hook key of their own; neither
/// tables nor variables overlap each other.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Symbols {
    routines: Vec<Routine>,
    tables: Vec<Table>,
    variables: Vec<Variable>,
    sync_points: Vec<SyncPoint>,
}

#[derive(Debug, thiserror::Error)]
pub enum SymbolsError {
    #[error("cannot read {}: {source}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// The file is not valid TOML, does not follow the schema or breaks a rule; `line` is
    /// 1-based.
    #[error("{}:{line}: {message}", path.display())]
    Invalid { path: PathBuf, line: usize, message: String },
}

impl Symbols {
    /// Reads and checks a symbols file.
    ///
    /// # Errors
    ///
    /// [`SymbolsError::Read`] if the file cannot be read, [`SymbolsError::Invalid`] (with the
    /// file and the line) if it is not valid: TOML syntax, unknown or missing fields, unknown
    /// values of `returns`, `timing` or `mode`, a flag letter outside NVMXDIZC or given twice, a
    /// zero `verify_sample` or `size`, an address that is not 24-bit, a routine, table or sync
    /// point address outside banks `$00–$1F`/offsets `$8000–$FFFF` (or with bank bit 7 set: a
    /// mirror), a table that runs past the end of its bank, a name that is not an identifier
    /// (`[a-z_][a-z0-9_]*`) or is used twice, two routines or two sync points at one address, a
    /// sync point at a routine's address, overlapping tables or overlapping variables.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, SymbolsError> {
        let path = path.as_ref();
        let source = std::fs::read_to_string(path)
            .map_err(|source| SymbolsError::Read { path: path.to_owned(), source })?;
        Self::parse(&source, path)
    }

    /// Parses and checks `source` as [`Symbols::load`] does; `path` names it in errors.
    ///
    /// # Errors
    ///
    /// [`SymbolsError::Invalid`], as for [`Symbols::load`].
    pub fn parse(source: &str, path: &Path) -> Result<Self, SymbolsError> {
        Parser { source, path }.parse()
    }

    #[must_use]
    pub fn routines(&self) -> &[Routine] {
        &self.routines
    }

    #[must_use]
    pub fn tables(&self) -> &[Table] {
        &self.tables
    }

    #[must_use]
    pub fn variables(&self) -> &[Variable] {
        &self.variables
    }

    #[must_use]
    pub fn sync_points(&self) -> &[SyncPoint] {
        &self.sync_points
    }

    /// The routine hooked at a 24-bit address or key (bank bit 7 ignored).
    #[must_use]
    pub fn routine_for_key(&self, key: u32) -> Option<&Routine> {
        let key = hook_key(key);
        self.routines.iter().find(|routine| routine.address == key)
    }

    /// The variable containing the byte at a 24-bit bus address. Low RAM mirrors (`$0000–$1FFF`
    /// of banks `$00–$3F` and `$80–$BF`) are the same bytes as `$7E:0000–$1FFF`.
    #[must_use]
    pub fn variable_at(&self, address: u32) -> Option<&Variable> {
        let address = canonical_address(address);
        self.variables.iter().find(|variable| span(variable).contains(&address))
    }
}

/// Whether `name` matches `[a-z_][a-z0-9_]*` (a Rust function or variable name).
fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|first| first.is_ascii_lowercase() || first == '_')
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// The bytes of `variable` as canonical addresses.
fn span(variable: &Variable) -> Range<u32> {
    let start = canonical_address(variable.address);
    start..start + variable.size
}

/// A 24-bit bus address with low RAM mirrors folded onto bank `$7E`.
pub(crate) fn canonical_address(address: u32) -> u32 {
    let (bank, offset) = ((address >> 16) as u8, address & 0xFFFF);
    if bank & 0x7F <= 0x3F && offset < 0x2000 { 0x7E_0000 | offset } else { address & 0xFF_FFFF }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    #[serde(default)]
    routine: Vec<RawRoutine>,
    #[serde(default)]
    table: Vec<RawTable>,
    #[serde(default)]
    variable: Vec<RawVariable>,
    #[serde(default)]
    sync_point: Vec<RawSyncPoint>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRoutine {
    name: Spanned<String>,
    address: Spanned<u32>,
    returns: RawReturn,
    timing: Timing,
    mode: RawMode,
    entry_m8: Option<bool>,
    entry_x8: Option<bool>,
    flags: Option<Spanned<String>>,
    verify_sample: Option<Spanned<u32>>,
    cycles: Option<u64>,
    budget_frames: Option<Spanned<u64>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTable {
    name: Spanned<String>,
    address: Spanned<u32>,
    size: Spanned<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawVariable {
    name: Spanned<String>,
    address: Spanned<u32>,
    size: Option<Spanned<u32>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSyncPoint {
    name: Spanned<String>,
    address: Spanned<u32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum RawReturn {
    Rts,
    Rtl,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum RawMode {
    Original,
    Rust,
    Verify,
}

struct Parser<'a> {
    source: &'a str,
    path: &'a Path,
}

impl Parser<'_> {
    fn parse(&self) -> Result<Symbols, SymbolsError> {
        let raw: RawFile = toml::from_str(self.source)
            .map_err(|error| self.error(error.span(), error.message().to_owned()))?;
        // Line of each name's first use, for duplicate errors.
        let mut names = HashMap::new();
        let mut routines: Vec<(Routine, usize)> = Vec::new();
        for raw in raw.routine {
            let routine = self.routine(raw, &mut names)?;
            routines.push(routine);
        }
        for (index, (routine, line)) in routines.iter().enumerate() {
            if let Some((first, first_line)) =
                routines[..index].iter().find(|(other, _)| other.address == routine.address)
            {
                return Err(self.error_at(
                    *line,
                    format!(
                        "routine `{}` has the address of `{}` (line {first_line})",
                        routine.name, first.name
                    ),
                ));
            }
        }

        let mut tables: Vec<(Table, usize)> = Vec::new();
        for raw in raw.table {
            let (table, line) = self.table(raw, &mut names)?;
            if let Some((other, other_line)) = tables.iter().find(|(other, _)| {
                let end = |table: &Table| u64::from(table.address) + u64::from(table.size);
                u64::from(other.address) < end(&table) && u64::from(table.address) < end(other)
            }) {
                return Err(self.error_at(
                    line,
                    format!("table `{}` overlaps `{}` (line {other_line})", table.name, other.name),
                ));
            }
            tables.push((table, line));
        }

        let mut variables: Vec<(Variable, usize)> = Vec::new();
        for raw in raw.variable {
            let (variable, line) = self.variable(raw, &mut names)?;
            if let Some((other, other_line)) = variables.iter().find(|(other, _)| {
                let (a, b) = (span(other), span(&variable));
                a.start < b.end && b.start < a.end
            }) {
                return Err(self.error_at(
                    line,
                    format!(
                        "variable `{}` overlaps `{}` (line {other_line})",
                        variable.name, other.name
                    ),
                ));
            }
            variables.push((variable, line));
        }

        // One hook entry per key: a sync point is neither at a routine nor at another sync point.
        let mut sync_points: Vec<(SyncPoint, usize)> = Vec::new();
        for raw in raw.sync_point {
            let line = self.unique_name(&raw.name, &mut names)?;
            let address = self.rom_address(&raw.address, "sync point")?;
            let point = SyncPoint { name: raw.name.into_inner(), address };
            let taken = routines
                .iter()
                .find(|(routine, _)| routine.address == address)
                .map(|(routine, line)| format!("routine `{}` (line {line})", routine.name))
                .or_else(|| {
                    sync_points
                        .iter()
                        .find(|(other, _)| other.address == address)
                        .map(|(other, line)| format!("`{}` (line {line})", other.name))
                });
            if let Some(taken) = taken {
                let message = format!("sync point `{}` has the address of {taken}", point.name);
                return Err(self.error_at(line, message));
            }
            sync_points.push((point, line));
        }

        Ok(Symbols {
            routines: routines.into_iter().map(|(routine, _)| routine).collect(),
            tables: tables.into_iter().map(|(table, _)| table).collect(),
            variables: variables.into_iter().map(|(variable, _)| variable).collect(),
            sync_points: sync_points.into_iter().map(|(point, _)| point).collect(),
        })
    }

    /// A checked routine and the line of its name.
    fn routine(
        &self,
        raw: RawRoutine,
        names: &mut HashMap<String, usize>,
    ) -> Result<(Routine, usize), SymbolsError> {
        let line = self.unique_name(&raw.name, names)?;
        let address = self.rom_address(&raw.address, "routine")?;
        let flags_mask = match &raw.flags {
            None => 0xFF,
            Some(flags) => self.flags_mask(flags)?,
        };
        let verify_sample = match raw.verify_sample {
            None => 1,
            Some(sample) if *sample.get_ref() == 0 => {
                return Err(self.error(Some(sample.span()), "verify_sample must be at least 1"));
            }
            Some(sample) => sample.into_inner(),
        };
        let budget_frames = match raw.budget_frames {
            None => None,
            Some(frames) if !(1..=MAX_BUDGET_FRAMES).contains(frames.get_ref()) => {
                let message = format!("budget_frames must be 1 to {MAX_BUDGET_FRAMES}");
                return Err(self.error(Some(frames.span()), message));
            }
            Some(frames) => Some(frames.into_inner()),
        };
        let routine = Routine {
            name: raw.name.into_inner(),
            address,
            returns: match raw.returns {
                RawReturn::Rts => ReturnKind::Rts,
                RawReturn::Rtl => ReturnKind::Rtl,
            },
            timing: raw.timing,
            mode: match raw.mode {
                RawMode::Original => HookMode::Original,
                RawMode::Rust => HookMode::Rust,
                RawMode::Verify => HookMode::Verify,
            },
            entry_m8: raw.entry_m8,
            entry_x8: raw.entry_x8,
            flags_mask,
            verify_sample,
            cycles: raw.cycles,
            budget_frames,
        };
        Ok((routine, line))
    }

    /// A checked table (overlaps are checked by the caller) and the line of its name.
    fn table(
        &self,
        raw: RawTable,
        names: &mut HashMap<String, usize>,
    ) -> Result<(Table, usize), SymbolsError> {
        let line = self.unique_name(&raw.name, names)?;
        let address = self.rom_address(&raw.address, "table")?;
        let size = *raw.size.get_ref();
        if size == 0 {
            return Err(self.error(Some(raw.size.span()), "size must be at least 1"));
        }
        // The bytes stay in the bank: the next bank's low half is not ROM. (u64: a huge size
        // must not overflow the sum.)
        if u64::from(address & 0xFFFF) + u64::from(size) > 0x1_0000 {
            let message = format!("size {size} does not fit at ${address:06X}");
            return Err(self.error(Some(raw.size.span()), message));
        }
        Ok((Table { name: raw.name.into_inner(), address, size }, line))
    }

    /// A checked variable (overlaps are checked by the caller) and the line of its name.
    fn variable(
        &self,
        raw: RawVariable,
        names: &mut HashMap<String, usize>,
    ) -> Result<(Variable, usize), SymbolsError> {
        let line = self.unique_name(&raw.name, names)?;
        let address = self.address(&raw.address)?;
        let size = match raw.size {
            None => 1,
            Some(size) if *size.get_ref() == 0 => {
                return Err(self.error(Some(size.span()), "size must be at least 1"));
            }
            Some(size) if u64::from(address) + u64::from(*size.get_ref()) > 0x100_0000 => {
                let message = format!("size {} does not fit at ${address:06X}", size.get_ref());
                return Err(self.error(Some(size.span()), message));
            }
            Some(size) => size.into_inner(),
        };
        Ok((Variable { name: raw.name.into_inner(), address, size }, line))
    }

    /// Records `name`, which must be new and not empty; returns its line.
    fn unique_name(
        &self,
        name: &Spanned<String>,
        names: &mut HashMap<String, usize>,
    ) -> Result<usize, SymbolsError> {
        let line = self.line(Some(name.span()));
        if !is_identifier(name.get_ref()) {
            let message =
                format!("name `{}` is not an identifier ([a-z_][a-z0-9_]*)", name.get_ref());
            return Err(self.error_at(line, message));
        }
        if let Some(first) = names.insert(name.get_ref().clone(), line) {
            let message =
                format!("duplicate name `{}` (first used at line {first})", name.get_ref());
            return Err(self.error_at(line, message));
        }
        Ok(line)
    }

    /// The physical address of a `what` (routine, table or sync point) in the ROM: 24-bit, not a bank
    /// bit 7 mirror, banks `$00-$1F`, offsets `$8000-$FFFF`. That is the whole of the game's
    /// image, a 1 MiB `LoROM` of 32 banks of 32 KiB; the range follows its size.
    fn rom_address(&self, raw: &Spanned<u32>, what: &str) -> Result<u32, SymbolsError> {
        let address = self.address(raw)?;
        if hook_key(address) != address {
            let message = format!(
                "address ${address:06X} is a mirror: use the physical address ${:06X}",
                hook_key(address)
            );
            return Err(self.error(Some(raw.span()), message));
        }
        if address >> 16 > 0x1F || address & 0xFFFF < 0x8000 {
            let message = format!(
                "{what} address ${address:06X} is not in the ROM (banks $00-$1F, offsets \
                 $8000-$FFFF)"
            );
            return Err(self.error(Some(raw.span()), message));
        }
        Ok(address)
    }

    fn address(&self, address: &Spanned<u32>) -> Result<u32, SymbolsError> {
        let value = *address.get_ref();
        if value > 0xFF_FFFF {
            let message = format!("address ${value:X} is not a 24-bit address");
            return Err(self.error(Some(address.span()), message));
        }
        Ok(value)
    }

    fn flags_mask(&self, flags: &Spanned<String>) -> Result<u8, SymbolsError> {
        flags.get_ref().chars().try_fold(0_u8, |mask, letter| {
            let bit = FLAG_LETTERS.find(letter).map(|index| 0x80_u8 >> index);
            let message = match bit {
                Some(bit) if mask & bit == 0 => return Ok(mask | bit),
                Some(_) => format!("flag `{letter}` twice in flags"),
                None => format!("unknown flag `{letter}` in flags (letters from {FLAG_LETTERS})"),
            };
            Err(self.error(Some(flags.span()), message))
        })
    }

    fn error(&self, span: Option<Range<usize>>, message: impl Into<String>) -> SymbolsError {
        self.error_at(self.line(span), message.into())
    }

    fn error_at(&self, line: usize, message: String) -> SymbolsError {
        SymbolsError::Invalid { path: self.path.to_owned(), line, message }
    }

    /// 1-based line of the start of `span` (line 1 when the error has no position).
    fn line(&self, span: Option<Range<usize>>) -> usize {
        let start = span.map_or(0, |span| span.start.min(self.source.len()));
        // One more line than there are newlines before `start`.
        self.source.as_bytes()[..start].split(|&byte| byte == b'\n').count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATH: &str = "re/symbols.toml";

    fn parse(source: &str) -> Result<Symbols, SymbolsError> {
        Symbols::parse(source, Path::new(PATH))
    }

    /// The line and message of a parse error.
    fn invalid(source: &str) -> (usize, String) {
        match parse(source) {
            Err(SymbolsError::Invalid { path, line, message }) => {
                assert_eq!(path, Path::new(PATH));
                (line, message)
            }
            other => panic!("expected an invalid file: {other:?}"),
        }
    }

    /// A routine entry with the given name, address and extra lines.
    fn routine(name: &str, address: &str, extra: &str) -> String {
        format!(
            "[[routine]]\nname = \"{name}\"\naddress = {address}\nreturns = \"rts\"\n\
             timing = \"pure\"\nmode = \"verify\"\n{extra}\n"
        )
    }

    fn variable(name: &str, address: &str, size: u32) -> String {
        format!("[[variable]]\nname = \"{name}\"\naddress = {address}\nsize = {size}\n\n")
    }

    fn table(name: &str, address: &str, size: u32) -> String {
        format!("[[table]]\nname = \"{name}\"\naddress = {address}\nsize = {size}\n\n")
    }

    #[test]
    fn parses_every_field_and_the_defaults() {
        let source = "# comment\n\n[[routine]]\nname = \"advance\"\naddress = 0x009000\n\
                      returns = \"rtl\"\ntiming = \"mmio\"\nmode = \"rust\"\nentry_m8 = true\n\
                      entry_x8 = false\nflags = \"NZ\"\nverify_sample = 64\ncycles = 108\nbudget_frames = 6\n\n"
            .to_owned()
            + &routine("plain", "0x05A000", "")
            + &table("lookup", "0x0D8200", 128)
            + &variable("seed", "0x7E0040", 2)
            + "[[variable]]\nname = \"flag\"\naddress = 0x7E0300\n";
        let symbols = parse(&source).unwrap();

        assert_eq!(
            symbols.routines(),
            [
                Routine {
                    name: "advance".into(),
                    address: 0x00_9000,
                    returns: ReturnKind::Rtl,
                    timing: Timing::Mmio,
                    mode: HookMode::Rust,
                    entry_m8: Some(true),
                    entry_x8: Some(false),
                    flags_mask: 0x82,
                    verify_sample: 64,
                    cycles: Some(108),
                    budget_frames: Some(6),
                },
                Routine {
                    name: "plain".into(),
                    address: 0x05_A000,
                    returns: ReturnKind::Rts,
                    timing: Timing::Pure,
                    mode: HookMode::Verify,
                    entry_m8: None,
                    entry_x8: None,
                    flags_mask: 0xFF,
                    verify_sample: 1,
                    cycles: None,
                    budget_frames: None,
                },
            ]
        );
        assert_eq!(
            symbols.tables(),
            [Table { name: "lookup".into(), address: 0x0D_8200, size: 128 }]
        );
        assert_eq!(
            symbols.variables(),
            [
                Variable { name: "seed".into(), address: 0x7E_0040, size: 2 },
                Variable { name: "flag".into(), address: 0x7E_0300, size: 1 },
            ]
        );
    }

    #[test]
    fn flags_map_to_a_mask_in_nvmxdizc_order() {
        for (flags, mask) in [("", 0x00), ("C", 0x01), ("NVZC", 0xC3), ("NVMXDIZC", 0xFF)] {
            let symbols =
                parse(&routine("r", "0x008000", &format!("flags = \"{flags}\""))).unwrap();
            assert_eq!(symbols.routines()[0].flags_mask, mask, "{flags:?}");
        }
        let (line, message) = invalid(&routine("r", "0x008000", "flags = \"NQ\""));
        assert_eq!(line, 7);
        assert!(message.contains("unknown flag `Q`"), "{message}");
        let (line, message) = invalid(&routine("r", "0x008000", "flags = \"NZN\""));
        assert_eq!(line, 7);
        assert!(message.contains("flag `N` twice"), "{message}");
    }

    #[test]
    fn routine_addresses_are_in_the_rom_banks() {
        for address in ["0x008000", "0x00FFFF", "0x1F8000", "0x1FFFFF"] {
            assert!(parse(&routine("r", address, "")).is_ok(), "{address}");
        }
        for address in ["0x007FFF", "0x000000", "0x208000", "0x7E8000", "0x7F0000"] {
            let (line, message) = invalid(&routine("r", address, ""));
            assert_eq!(line, 3, "{address}");
            assert!(message.contains("banks $00-$1F"), "{message}");
        }
    }

    #[test]
    fn names_are_identifiers() {
        for name in ["a", "_x", "rng_next_a", "bcd_2"] {
            assert!(parse(&routine(name, "0x008000", "")).is_ok(), "{name}");
            assert!(parse(&variable(name, "0x7E0000", 1)).is_ok(), "{name}");
        }
        for name in ["", "Leaf", "2nd", "a-b", "a b", "battle::hp", "é"] {
            let (line, message) = invalid(&routine(name, "0x008000", ""));
            assert_eq!(line, 2, "{name:?}");
            assert!(message.contains("not an identifier"), "{message}");
            assert_eq!(invalid(&variable(name, "0x7E0000", 1)).0, 2, "{name:?}");
        }
    }

    #[test]
    fn a_bad_mode_is_reported_with_its_line() {
        let source = routine("first", "0x008000", "")
            + &routine("second", "0x008100", "")
            + "[[routine]]\nname = \"third\"\naddress = 0x008200\nreturns = \"rts\"\n\
               timing = \"pure\"\nmode = \"verfy\"\n";
        let (line, message) = invalid(&source);
        assert_eq!(line, 20, "{message}");
        assert!(message.contains("verfy"), "{message}");

        // Through a file, as the app loads it: the message names the file and the line.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("symbols.toml");
        std::fs::write(&path, &source).unwrap();
        let error = Symbols::load(&path).unwrap_err().to_string();
        let expected = format!("{}:20: ", path.display());
        assert!(error.starts_with(&expected), "{error}");
        assert!(error.contains("verfy"), "{error}");
    }

    #[test]
    fn unknown_values_and_fields_are_rejected() {
        let bad_return = "[[routine]]\nname = \"r\"\naddress = 0x008000\nreturns = \"rti\"\n\
                          timing = \"pure\"\nmode = \"rust\"\n";
        assert_eq!(invalid(bad_return).0, 4);
        let bad_timing = bad_return.replace("rti", "rts").replace("pure", "fast");
        assert_eq!(invalid(&bad_timing).0, 5);
        let (line, message) = invalid(&routine("r", "0x008000", "verify_sampel = 4"));
        assert_eq!(line, 7);
        assert!(message.contains("verify_sampel"), "{message}");
        let (_, message) = invalid("[[routine]]\nname = \"r\"\naddress = 0x008000\n");
        assert!(message.contains("returns"), "missing field: {message}");
        assert_eq!(invalid("[[routines]]\nname = \"r\"\n").0, 1);
        assert_eq!(invalid("[[variable]]\nname = \"v\"\naddress = 0x7E0000\nsize = 0\n").0, 4);
        assert_eq!(invalid(&routine("r", "0x008000", "verify_sample = 0")).0, 7);
        // A budget is 1 to 600 frames.
        for frames in [0, 601] {
            let (line, message) =
                invalid(&routine("r", "0x008000", &format!("budget_frames = {frames}")));
            assert_eq!(line, 7);
            assert!(message.contains("budget_frames must be 1 to 600"), "{message}");
        }
    }

    #[test]
    fn duplicate_names_are_rejected() {
        let source = routine("same", "0x008000", "") + &routine("same", "0x008100", "");
        let (line, message) = invalid(&source);
        assert_eq!(line, 9);
        assert!(message.contains("duplicate name `same` (first used at line 2)"), "{message}");

        let source = variable("v", "0x7E0000", 1) + &variable("v", "0x7E0010", 1);
        assert_eq!(invalid(&source).0, 7);
        // Routines and variables share one namespace.
        let source = routine("both", "0x008000", "") + &variable("both", "0x7E0000", 1);
        assert_eq!(invalid(&source).0, 9);
    }

    #[test]
    fn duplicate_addresses_are_rejected() {
        let source = routine("a", "0x008000", "") + &routine("b", "0x008000", "");
        let (line, message) = invalid(&source);
        assert_eq!(line, 9);
        assert!(message.contains("routine `b` has the address of `a` (line 2)"), "{message}");

        let (line, message) = invalid(&routine("m", "0x808000", ""));
        assert_eq!(line, 3);
        assert!(message.contains("use the physical address $008000"), "{message}");
        assert_eq!(invalid(&routine("big", "0x1000000", "")).0, 3);

        let source = variable("a", "0x7E0010", 2) + &variable("b", "0x7E0011", 1);
        let (line, message) = invalid(&source);
        assert_eq!(line, 7);
        assert!(message.contains("variable `b` overlaps `a` (line 2)"), "{message}");
        // The low RAM mirror is the same memory.
        let source = variable("a", "0x7E0010", 1) + &variable("b", "0x000010", 1);
        assert_eq!(invalid(&source).0, 7);
        let ok = variable("a", "0x7E0010", 2) + &variable("b", "0x7E0012", 1);
        assert!(parse(&ok).is_ok());
    }

    #[test]
    fn tables_are_rom_regions() {
        let source = table("first", "0x0D8200", 128) + &table("second", "0x1FFF00", 0x100);
        let symbols = parse(&source).unwrap();
        assert_eq!(
            symbols.tables(),
            [
                Table { name: "first".into(), address: 0x0D_8200, size: 128 },
                Table { name: "second".into(), address: 0x1F_FF00, size: 0x100 },
            ]
        );
        // A table may fill its bank up to the last byte, and have the size of one byte.
        for (address, size) in [("0x008000", 0x8000), ("0x00FFFF", 1), ("0x1F8000", 16)] {
            assert!(parse(&table("t", address, size)).is_ok(), "{address} {size}");
        }
        for address in ["0x007FFF", "0x000000", "0x208000", "0x7E8000", "0x7E0E01", "0x7F0000"] {
            let (line, message) = invalid(&table("t", address, 2));
            assert_eq!(line, 3, "{address}");
            assert!(message.contains("table address"), "{message}");
            assert!(message.contains("banks $00-$1F"), "{message}");
        }
    }

    #[test]
    fn a_table_is_in_its_bank_and_physical() {
        let (line, message) = invalid(&table("mirror", "0x8D8200", 128));
        assert_eq!(line, 3);
        assert!(message.contains("use the physical address $0D8200"), "{message}");
        assert_eq!(invalid(&table("big", "0x1000000", 1)).0, 3);
        // The table must end in its bank: the next bank's low half is not ROM.
        let (line, message) = invalid(&table("long", "0x0DFF00", 0x101));
        assert_eq!(line, 4);
        assert!(message.contains("size 257 does not fit at $0DFF00"), "{message}");
        assert_eq!(invalid(&table("empty", "0x0D8200", 0)).0, 4);
        let (_, message) = invalid("[[table]]\nname = \"t\"\naddress = 0x0D8200\n");
        assert!(message.contains("size"), "a table has a size: {message}");
        let with_stride = table("t", "0x0D8200", 4).replace("size", "stride = 32\nsize");
        let (line, message) = invalid(&with_stride);
        assert_eq!(line, 4);
        assert!(message.contains("stride"), "{message}");
    }

    #[test]
    fn a_size_is_at_least_one() {
        for (what, source) in
            [("table", table("t", "0x0D8200", 0)), ("variable", variable("v", "0x7E0010", 0))]
        {
            let (line, message) = invalid(&source);
            assert_eq!(line, 4, "{what}");
            assert!(message.contains("size must be at least 1"), "{what}: {message}");
        }
    }

    #[test]
    fn a_huge_size_is_an_error_not_an_overflow() {
        let max = u32::MAX;
        for (what, source) in [
            ("table", table("t", "0x0D8200", max)),
            ("table at the bank end", table("t", "0x0DFFFF", max)),
            ("variable", variable("v", "0x7E0010", max)),
            ("variable at the end", variable("v", "0xFFFFFF", max)),
        ] {
            let (line, message) = invalid(&source);
            assert_eq!(line, 4, "{what}");
            assert!(message.contains("size 4294967295 does not fit at"), "{what}: {message}");
        }
    }

    #[test]
    fn tables_do_not_overlap_each_other() {
        let source = table("a", "0x0D8200", 0x80) + &table("b", "0x0D827F", 2);
        let (line, message) = invalid(&source);
        assert_eq!(line, 7);
        assert!(message.contains("table `b` overlaps `a` (line 2)"), "{message}");
        // Touching is fine, and so is the same offset in another bank.
        let ok =
            table("a", "0x0D8200", 0x80) + &table("b", "0x0D8280", 2) + &table("c", "0x0E8200", 2);
        assert!(parse(&ok).is_ok());
        // Tables are ROM and variables are WRAM: they never overlap each other.
        let ok = table("t", "0x0D8200", 0x80) + &variable("v", "0x7E8200", 2);
        assert!(parse(&ok).is_ok());
    }

    #[test]
    fn names_are_unique_across_routines_tables_and_variables() {
        let source = table("same", "0x0D8200", 4) + &table("same", "0x0D8300", 4);
        let (line, message) = invalid(&source);
        assert_eq!(line, 7);
        assert!(message.contains("duplicate name `same` (first used at line 2)"), "{message}");

        let source = routine("both", "0x008000", "") + &table("both", "0x0D8200", 4);
        let (line, message) = invalid(&source);
        assert_eq!(line, 9);
        assert!(message.contains("duplicate name `both` (first used at line 2)"), "{message}");

        let source = table("both", "0x0D8200", 4) + &variable("both", "0x7E0010", 1);
        let (line, message) = invalid(&source);
        assert_eq!(line, 7);
        assert!(message.contains("duplicate name `both` (first used at line 2)"), "{message}");

        for name in ["Leaf", "2nd", ""] {
            assert_eq!(invalid(&table(name, "0x0D8200", 4)).0, 2, "{name:?}");
        }
    }

    #[test]
    fn lookups_by_key_and_by_address() {
        let source = routine("r", "0x018000", "")
            + &variable("seed", "0x7E0040", 2)
            + &variable("high", "0x7F0000", 1);
        let symbols = parse(&source).unwrap();
        assert_eq!(symbols.routine_for_key(0x81_8000).map(|r| r.name.as_str()), Some("r"));
        assert!(symbols.routine_for_key(0x01_8001).is_none());

        let name = |address| symbols.variable_at(address).map(|v| v.name.as_str());
        assert_eq!(name(0x7E_0040), Some("seed"));
        assert_eq!(name(0x7E_0041), Some("seed"));
        assert_eq!(name(0x80_0041), Some("seed"), "low RAM mirror");
        assert_eq!(name(0x7E_0042), None);
        assert_eq!(name(0x7F_0000), Some("high"));
        assert_eq!(name(0x00_0000), None);
    }

    fn sync_point(name: &str, address: &str) -> String {
        format!("[[sync_point]]\nname = \"{name}\"\naddress = {address}\n\n")
    }

    #[test]
    fn sync_points_are_parsed_in_file_order() {
        let source = sync_point("idle", "0x009000") + &sync_point("menu_wait", "0x1F8000");
        let symbols = parse(&source).unwrap();
        assert_eq!(
            symbols.sync_points(),
            [
                SyncPoint { name: "idle".into(), address: 0x00_9000 },
                SyncPoint { name: "menu_wait".into(), address: 0x1F_8000 },
            ]
        );
        assert_eq!(symbols.routines(), []);
    }

    #[test]
    fn sync_points_are_physical_rom_code_addresses() {
        for address in ["0x007FFF", "0x000000", "0x208000", "0x7E0000"] {
            let (line, message) = invalid(&sync_point("s", address));
            assert_eq!(line, 3, "{address}");
            assert!(message.contains("sync point address"), "{message}");
            assert!(message.contains("banks $00-$1F"), "{message}");
        }
        let (line, message) = invalid(&sync_point("s", "0x809000"));
        assert_eq!(line, 3);
        assert!(message.contains("use the physical address $009000"), "{message}");
        assert_eq!(invalid(&sync_point("s", "0x1000000")).0, 3);
        let with_mode = sync_point("s", "0x009000").replace("\n\n", "\nmode = \"rust\"\n");
        let (line, message) = invalid(&with_mode);
        assert_eq!(line, 4);
        assert!(message.contains("mode"), "{message}");
    }

    #[test]
    fn a_sync_point_has_a_hook_key_of_its_own() {
        // One hook entry per key: not a routine's entry, not another sync point's.
        let source = routine("r", "0x009000", "") + &sync_point("s", "0x009000");
        let (line, message) = invalid(&source);
        assert_eq!(line, 9);
        assert!(
            message.contains("sync point `s` has the address of routine `r` (line 2)"),
            "{message}"
        );
        let source = sync_point("a", "0x009000") + &sync_point("b", "0x009000");
        let (line, message) = invalid(&source);
        assert_eq!(line, 6);
        assert!(message.contains("sync point `b` has the address of `a` (line 2)"), "{message}");
        let ok = routine("r", "0x009000", "") + &sync_point("s", "0x009001");
        assert!(parse(&ok).is_ok());
    }

    #[test]
    fn sync_point_names_are_unique_identifiers() {
        let source = routine("same", "0x009000", "") + &sync_point("same", "0x009100");
        let (line, message) = invalid(&source);
        assert_eq!(line, 9);
        assert!(message.contains("duplicate name `same` (first used at line 2)"), "{message}");
        let source = sync_point("v", "0x009000") + &variable("v", "0x7E0010", 1);
        assert!(invalid(&source).1.contains("duplicate name `v`"));
        for name in ["Idle", "2nd", ""] {
            assert_eq!(invalid(&sync_point(name, "0x009000")).0, 2, "{name:?}");
        }
    }

    #[test]
    fn a_missing_file_is_named() {
        let error = Symbols::load("no/such/symbols.toml").unwrap_err();
        assert!(matches!(error, SymbolsError::Read { .. }));
        assert!(error.to_string().contains("symbols.toml"), "{error}");
    }

    #[test]
    fn the_repository_file_loads() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../re/symbols.toml");
        let symbols = Symbols::load(path).unwrap();
        let names: Vec<&str> = symbols.variables().iter().map(|v| v.name.as_str()).collect();
        for name in [
            "in_battle",
            "travel_mode",
            "map_menu_cursor",
            "rng_seed_a",
            "rng_seed_b",
            "active_side",
            "side_record_a",
            "side_record_b",
            "entity_ids",
        ] {
            assert!(names.contains(&name), "{name} in {names:?}");
        }
        let tables: Vec<&str> = symbols.tables().iter().map(|t| t.name.as_str()).collect();
        for name in [
            "id_category_table",
            "bank2_pointer_table",
            "byte_pair_table",
            "copy_source_offsets",
            "scale_table",
            "position_step_bytes",
            "position_step_words",
        ] {
            assert!(tables.contains(&name), "{name} in {tables:?}");
        }
        let id_category = symbols.tables().iter().find(|t| t.name == "id_category_table").unwrap();
        assert_eq!(id_category.size, 128);
        let sync_points: Vec<&str> =
            symbols.sync_points().iter().map(|s| s.name.as_str()).collect();
        assert_eq!(sync_points, ["main_loop_idle"]);
    }
}
