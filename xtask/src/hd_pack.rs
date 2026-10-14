//! `hd-pack`: merges the approved art of `assets/hd/` into the pack the app loads. The
//! overrides are listed in `assets/hd/hdart.toml` by logical id (`[[art]]` with `id`, `file` and
//! `mode`, the art file under `assets/hd/`); each id must be in the pack's index, and the art must
//! pass the publish rules: an image only under `assets/`, no run of ROM bytes, and no content
//! hash (a 16-digit hexadecimal token), so no tracked file can carry the ROM. `--check` does the
//! checks only; without it the art is copied into the pack and the index is rewritten.

use crate::publish_check::RomIndex;
use anyhow::Context;
use serde::Deserialize;
use ssd_hd::pack::{Mode, Pack};
use std::io::Write;
use std::path::{Path, PathBuf};

/// The overrides file, relative to the repository root.
const OVERRIDES: &str = "assets/hd/hdart.toml";
/// The art folder, relative to the repository root.
const ART: &str = "assets/hd";

/// One override: the asset `id` is drawn from `file`, read as `mode`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Override {
    pub id: String,
    pub file: String,
    pub mode: String,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Default, Deserialize)]
struct Overrides {
    #[serde(default)]
    art: Vec<Override>,
}

/// The command line of `hd-pack`: `[--check]`.
///
/// # Errors
///
/// An option other than `--check`.
pub fn parse_args(args: &[&str]) -> Result<bool, String> {
    match args {
        [] => Ok(false),
        ["--check"] => Ok(true),
        [other, ..] => Err(format!("unknown argument `{other}`")),
    }
}

/// Parses `hdart.toml`.
///
/// # Errors
///
/// The text is not a list of `[[art]]` entries with `id`, `file` and `mode`.
pub fn parse_overrides(text: &str) -> anyhow::Result<Vec<Override>> {
    let parsed: Overrides = toml::from_str(text).context("reading assets/hd/hdart.toml")?;
    Ok(parsed.art)
}

/// The art files under `dir`, recursively, sorted.
fn files_under(dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    if !dir.is_dir() {
        return Ok(files);
    }
    let mut pending = vec![dir.to_path_buf()];
    while let Some(folder) = pending.pop() {
        for entry in
            std::fs::read_dir(&folder).with_context(|| format!("reading {}", folder.display()))?
        {
            let path = entry?.path();
            if path.is_dir() {
                pending.push(path);
            } else {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

/// Whether `data` has a run of 16 hexadecimal digits (a content hash, or a tile key).
fn has_hash(data: &[u8]) -> bool {
    data.windows(16).any(|window| window.iter().all(u8::is_ascii_hexdigit))
}

/// The publish findings of the files under `assets/hd/`: what must not be tracked.
fn findings(root: &Path, rom: &[u8]) -> anyhow::Result<Vec<String>> {
    let files = files_under(&root.join(ART))?;
    let mut found = Vec::new();
    if files.is_empty() {
        return Ok(found);
    }
    // The ROM's windows are only indexed when there is art to look at
    let index = RomIndex::new(rom);
    for path in files {
        let relative = path.strip_prefix(root).unwrap_or(&path).display().to_string();
        let data = std::fs::read(&path).with_context(|| format!("reading {relative}"))?;
        if index.find_run(&data).is_some() {
            found.push(format!("{relative}: contains ROM bytes"));
        }
        if has_hash(&data) {
            found.push(format!("{relative}: contains a content hash"));
        }
    }
    Ok(found)
}

/// Checks the overrides against `pack` and, unless `check_only`, merges them into it. Returns
/// the exit code: 0 when every override is valid, 1 otherwise.
///
/// # Errors
///
/// The overrides cannot be read, the pack cannot be written, or the ROM cannot be scanned.
pub fn run(
    rom: &[u8],
    root: &Path,
    pack_dir: &Path,
    check_only: bool,
    out: &mut dyn Write,
) -> anyhow::Result<u8> {
    let mut problems = findings(root, rom)?;
    let text = match std::fs::read_to_string(root.join(OVERRIDES)) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).context(OVERRIDES),
    };
    let overrides = parse_overrides(&text)?;
    let mut pack = Pack::open(pack_dir)?;
    let mut merged = 0;
    for item in &overrides {
        let mode = match item.mode.as_str() {
            "indexed" => Mode::Indexed,
            "rgba" => Mode::Rgba,
            other => {
                problems.push(format!("{}: mode `{other}` is not indexed or rgba", item.id));
                continue;
            }
        };
        let source = root.join(ART).join(&item.file);
        if !source.is_file() {
            problems.push(format!("{}: no file {}", item.id, item.file));
            continue;
        }
        let Some(position) = pack.index.assets().iter().position(|asset| asset.id == item.id)
        else {
            problems.push(format!("{}: not an asset of the pack (a stale override)", item.id));
            continue;
        };
        if !check_only {
            let target = pack_dir.join("art").join(&item.file);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&source, &target)
                .with_context(|| format!("copying {}", source.display()))?;
            let mut assets = pack.index.assets().to_vec();
            assets[position].art = format!("art/{}", item.file);
            assets[position].mode = mode;
            pack.index = ssd_hd::Index::new(assets);
        }
        merged += 1;
    }
    if !check_only {
        std::fs::write(pack_dir.join("index.bin"), pack.index.encode())
            .with_context(|| format!("writing {}", pack_dir.join("index.bin").display()))?;
    }
    for problem in &problems {
        writeln!(out, "hd-pack: {problem}")?;
    }
    writeln!(
        out,
        "hd-pack: {} overrides {}, {} problems",
        merged,
        if check_only { "checked" } else { "merged" },
        problems.len()
    )?;
    Ok(u8::from(!problems.is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_overrides_name_an_id_a_file_and_a_mode() {
        let text = "[[art]]\nid = \"ui/11:8036/0\"\nfile = \"ui/font.png\"\nmode = \"indexed\"\n";
        let overrides = parse_overrides(text).unwrap();
        assert_eq!(overrides[0].id, "ui/11:8036/0");
        assert_eq!(overrides[0].mode, "indexed");
        assert_eq!(overrides[0].notes, "");
        assert!(parse_overrides("[[art]]\nid = \"x\"\n").is_err(), "a file is required");
        assert!(parse_overrides("").unwrap().is_empty(), "no overrides");
    }

    #[test]
    fn a_content_hash_is_sixteen_hex_digits() {
        assert!(has_hash(b"key = a1b2c3d4e5f60718"));
        assert!(!has_hash(b"portrait/16:8000/0 and 0x1234"));
    }

    #[test]
    fn the_command_line_takes_check_or_nothing() {
        assert_eq!(parse_args(&[]), Ok(false));
        assert_eq!(parse_args(&["--check"]), Ok(true));
        assert!(parse_args(&["--write"]).is_err());
    }
}
