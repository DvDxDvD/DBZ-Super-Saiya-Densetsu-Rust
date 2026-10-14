//! The emulator core version that replays carry (`ssd_oracle::CORE_VERSION`), computed from the
//! vendored jgenesis: `jgenesis <upstream commit>+<CRC32 of its sources>`. Shared by `build.rs`
//! and `tests/core_version.rs` (included with `#[path]`), so the function is tested on temporary
//! trees.
//!
//! The sources are every `.rs` file under the vendored root (the build scripts and proc-macro
//! crates included) and every `Cargo.toml` (features and dependencies decide what is compiled).
//! Files are taken in the order of their relative paths, written with `/` and compared as text,
//! so the order the directories list their entries in does not matter; each one enters the
//! checksum as path, length and contents, so a rename, an added or removed file and text moved
//! from one file to the next all change it. Contents are normalised to LF, so a checkout with
//! CRLF endings gets the same version. Documentation (`PATCHES.md`, `LICENSE`) is not a source:
//! editing it never changes the version. A change of the upstream commit does, through the
//! record's `- Commit:` line.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The patch record in the vendored root; its `- Commit:` line names the upstream commit.
const PATCH_RECORD: &str = "PATCHES.md";
/// Characters of the upstream commit in the version.
const COMMIT_CHARS: usize = 12;

/// `jgenesis <first 12 characters of the upstream commit>+<CRC32 of the sources, in hex>` for the
/// vendored root `root`.
pub fn core_version(root: &Path) -> Result<String, String> {
    let record = root.join(PATCH_RECORD);
    let text =
        fs::read_to_string(&record).map_err(|e| format!("reading {}: {e}", record.display()))?;
    let commit = text
        .lines()
        .find_map(|line| line.strip_prefix("- Commit: "))
        .and_then(|rest| rest.split_whitespace().next())
        .filter(|commit| {
            commit.len() >= COMMIT_CHARS && commit.bytes().all(|b| b.is_ascii_hexdigit())
        })
        .ok_or_else(|| format!("{} has no `- Commit: <sha>` line", record.display()))?;
    let crc =
        sources_crc(root).map_err(|e| format!("reading the sources of {}: {e}", root.display()))?;
    Ok(format!("jgenesis {}+{crc:08X}", &commit[..COMMIT_CHARS]))
}

/// The CRC32 of the sources under `root` (see the module documentation).
///
/// # Errors
///
/// If a directory or file cannot be read, or there is no source at all (a wrong root).
pub fn sources_crc(root: &Path) -> io::Result<u32> {
    let mut files = Vec::new();
    collect(root, "", &mut files, &mut Vec::new())?;
    if files.is_empty() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "no sources found"));
    }
    files.sort();
    let crc = crc::Crc::<u32>::new(&crc::CRC_32_ISO_HDLC);
    let mut digest = crc.digest();
    for (relative, path) in &files {
        let bytes = fs::read(path)
            .map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", path.display())))?;
        let text = lf_normalised(&bytes);
        digest.update(relative.as_bytes());
        digest.update(&[0]);
        digest.update(&(text.len() as u64).to_le_bytes());
        digest.update(&text);
    }
    Ok(digest.finalize())
}

/// The root and every directory under it, which the walk of [`sources_crc`] visits, sorted by
/// path. `build.rs` names each one to Cargo: on NTFS, deleting a file from a nested directory
/// changes the modification time of that directory only.
///
/// # Errors
///
/// If a directory cannot be read.
pub fn directories(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut directories = Vec::new();
    collect(root, "", &mut Vec::new(), &mut directories)?;
    directories.sort();
    Ok(directories)
}

/// Adds the sources under `dir` (`relative` is its path from the root, `/`-separated, empty for
/// the root) to `files` as `(relative path, path)`, and `dir` and the directories under it to
/// `directories`.
fn collect(
    dir: &Path,
    relative: &str,
    files: &mut Vec<(String, PathBuf)>,
    directories: &mut Vec<PathBuf>,
) -> io::Result<()> {
    directories.push(dir.to_path_buf());
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().into_string().map_err(|name| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{} in {} is not UTF-8", name.display(), dir.display()),
            )
        })?;
        let relative =
            if relative.is_empty() { name.clone() } else { format!("{relative}/{name}") };
        let kind = entry.file_type()?;
        if kind.is_dir() {
            collect(&entry.path(), &relative, files, directories)?;
        } else if kind.is_file() && is_source(&name) {
            files.push((relative, entry.path()));
        }
    }
    Ok(())
}

/// Whether a file of this name is a source: Rust code, or a crate manifest.
fn is_source(name: &str) -> bool {
    name == "Cargo.toml" || Path::new(name).extension().is_some_and(|extension| extension == "rs")
}

/// `bytes` with every `\r\n` replaced by `\n`.
fn lf_normalised(bytes: &[u8]) -> Vec<u8> {
    let mut text = Vec::with_capacity(bytes.len());
    for (i, &byte) in bytes.iter().enumerate() {
        if !(byte == b'\r' && bytes.get(i + 1) == Some(&b'\n')) {
            text.push(byte);
        }
    }
    text
}
