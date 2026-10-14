//! Divergence reports (spec §5.4): the state at the entry of a diverging call, runnable as a
//! regression case, and the divergence's text.

use crate::hooks::hook_key;
use crate::{Divergence, Machine};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

/// Starts the first line of a report's text (see [`ReproHeader`]).
const HEADER_PREFIX: &str = "ssd-repro: key=$";
/// Between the key and the routine name in a header.
const HEADER_ROUTINE: &str = " routine=";
/// Hex digits of a key: 24 bits.
const KEY_DIGITS: usize = 6;

/// What a report's text says about the call, on its first line, for `xtask repro`:
/// `ssd-repro: key=$008040 routine=leaf`. The key is the physical hook key of the routine's
/// entry ([`hook_key`]: the bank bit 7 mirror dropped); the routine is the hook entry's name, as
/// it is (the file names replace some characters), up to the end of the line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReproHeader {
    pub routine: String,
    pub key: u32,
}

impl ReproHeader {
    /// The header on the first line of `text`; `None` if that line is not one (a report of an
    /// older build has none).
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let line = text.lines().next()?;
        let (key, routine) = line.strip_prefix(HEADER_PREFIX)?.split_once(HEADER_ROUTINE)?;
        if key.len() != KEY_DIGITS || routine.is_empty() {
            return None;
        }
        // `from_str_radix` takes a sign: the digits are checked on their own.
        if !key.bytes().all(|digit| digit.is_ascii_hexdigit()) {
            return None;
        }
        Some(Self { routine: routine.to_owned(), key: u32::from_str_radix(key, 16).ok()? })
    }
}

impl fmt::Display for ReproHeader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{HEADER_PREFIX}{:0KEY_DIGITS$X}{HEADER_ROUTINE}{}", self.key, self.routine)
    }
}

/// The text file of a report, next to its state: the state's path with the extension `txt`.
#[must_use]
pub fn text_path(state: &Path) -> PathBuf {
    state.with_extension("txt")
}

/// Saves `divergence` in `dir` (created if missing) as `<routine>-<frame>-<call>.ssdstate`, the
/// [`Machine::save_state`] of `pristine` (the machine at the call's hooked opcode fetch: loaded
/// into a machine with the same ROM and hooks, its first tick reports the call again), and
/// `<routine>-<frame>-<call>.txt`, a [`ReproHeader`] line (the routine, and the key of the
/// opcode fetch `pristine` is at) followed by the divergence's text. In the file names,
/// characters of the routine name other than ASCII letters, digits, `_` and `-` become `_`.
/// Returns the path of the state.
///
/// # Errors
///
/// Returns any error creating the directory or writing the files.
pub fn save(dir: &Path, divergence: &Divergence, pristine: &Machine) -> io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let routine: String = divergence
        .routine
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect();
    let stem = format!("{routine}-{}-{}", divergence.frame, divergence.call_index);
    let state = dir.join(format!("{stem}.ssdstate"));
    std::fs::write(&state, pristine.save_state())?;
    let registers = pristine.cpu_registers();
    let header = ReproHeader {
        routine: divergence.routine.clone(),
        key: hook_key((u32::from(registers.pbr) << 16) | u32::from(registers.pc)),
    };
    std::fs::write(text_path(&state), format!("{header}\n{divergence}"))?;
    Ok(state)
}
