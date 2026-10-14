//! The emulator core version replays carry: computed from the vendored sources by
//! `build/core_version.rs` (shared with `build.rs`), tested here on temporary trees.

#[path = "../build/core_version.rs"]
mod core_version;

use core_version::{core_version, directories, sources_crc};
use ssd_oracle::{CORE_VERSION, ORACLE_REVISION};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

const COMMIT: &str = "85ffe5fc66fcc196fb3f5dd5340939e6e4f442a2";

/// A vendored tree of `files` (relative path with `/`, contents), with a patch record.
fn tree(files: &[(&str, &str)]) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "PATCHES.md", &format!("# Vendored\n\n- Commit: {COMMIT} (2026-10-06)\n"));
    for (path, contents) in files {
        write(dir.path(), path, contents);
    }
    dir
}

fn write(root: &Path, path: &str, contents: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn crc(dir: &TempDir) -> u32 {
    sources_crc(dir.path()).unwrap()
}

const BASE: &[(&str, &str)] = &[
    ("common/a/Cargo.toml", "[package]\nname = \"a\"\n"),
    ("common/a/src/lib.rs", "pub fn a() {}\n"),
    ("cpu/b/src/core.rs", "fn b() {\n    1\n}\n"),
    ("cpu/b/Cargo.toml", "[package]\nname = \"b\"\n"),
];

#[test]
fn crlf_and_lf_checkouts_have_the_same_version() {
    let lf = tree(BASE);
    let crlf_files: Vec<(&str, String)> =
        BASE.iter().map(|(path, text)| (*path, text.replace('\n', "\r\n"))).collect();
    let crlf = tree(&[]);
    for (path, text) in &crlf_files {
        write(crlf.path(), path, text);
    }
    assert!(fs::read(crlf.path().join("cpu/b/src/core.rs")).unwrap().contains(&b'\r'));
    assert_eq!(crc(&lf), crc(&crlf));
    // The patch record's line endings do not matter either.
    write(crlf.path(), "PATCHES.md", &format!("# Vendored\r\n\r\n- Commit: {COMMIT}\r\n"));
    assert_eq!(core_version(lf.path()), core_version(crlf.path()));
}

#[test]
fn text_that_is_not_source_does_not_change_it() {
    let dir = tree(BASE);
    let before = (crc(&dir), core_version(dir.path()).unwrap());
    write(dir.path(), "PATCHES.md", &format!("# Vendored\n\n- Commit: {COMMIT}\n\nA new patch.\n"));
    write(dir.path(), "common/a/README.md", "notes\n");
    write(dir.path(), "common/a/src/shader.wgsl", "fn main() {}\n");
    write(dir.path(), "LICENSE", "GPL\n");
    write(dir.path(), "cpu/b/Cargo.lock", "# lock\n");
    write(dir.path(), "cpu/b/src/core.rs.md", "# core\n");
    assert_eq!((crc(&dir), core_version(dir.path()).unwrap()), before);
}

#[test]
fn editing_a_rust_file_or_a_crate_manifest_changes_it() {
    let dir = tree(BASE);
    let original = crc(&dir);

    write(dir.path(), "cpu/b/src/core.rs", "fn b() {\n    2\n}\n");
    let edited = crc(&dir);
    assert_ne!(edited, original);

    write(dir.path(), "cpu/b/src/core.rs", "fn b() {\n    1\n}\n");
    assert_eq!(crc(&dir), original, "back to the first text");

    write(dir.path(), "common/a/Cargo.toml", "[package]\nname = \"a\"\nedition = \"2024\"\n");
    assert_ne!(crc(&dir), original);
}

#[test]
fn the_sources_in_nested_directories_and_a_root_manifest_count() {
    let dir = tree(BASE);
    let original = crc(&dir);
    write(dir.path(), "cpu/b/src/deep/er/mod.rs", "mod x;\n");
    let nested = crc(&dir);
    assert_ne!(nested, original);
    write(dir.path(), "Cargo.toml", "[workspace]\n");
    assert_ne!(crc(&dir), nested);
}

#[test]
fn adding_removing_moving_or_swapping_files_changes_it() {
    let dir = tree(BASE);
    let original = crc(&dir);

    write(dir.path(), "common/a/src/extra.rs", "");
    let added = crc(&dir);
    assert_ne!(added, original, "an empty file is still a file");
    fs::remove_file(dir.path().join("common/a/src/extra.rs")).unwrap();
    assert_eq!(crc(&dir), original);

    // The same contents under another name.
    fs::rename(dir.path().join("cpu/b/src/core.rs"), dir.path().join("cpu/b/src/cpu.rs")).unwrap();
    assert_ne!(crc(&dir), original);

    // Two files that exchange their contents; and text moved from one file's end to the next
    // one's start.
    let one = tree(&[("a.rs", "x"), ("b.rs", "y")]);
    let two = tree(&[("a.rs", "y"), ("b.rs", "x")]);
    let split = tree(&[("a.rs", "xy"), ("b.rs", "")]);
    assert_ne!(crc(&one), crc(&two));
    assert_ne!(crc(&one), crc(&split));
}

#[test]
fn the_order_the_files_were_created_in_does_not_matter() {
    let forward = tree(BASE);
    let backward = tree(&[]);
    for (path, text) in BASE.iter().rev() {
        write(backward.path(), path, text);
    }
    assert_eq!(crc(&forward), crc(&backward));
}

#[test]
fn every_directory_of_the_walk_is_listed_for_the_build_script() {
    // Cargo is told about each directory, not just the root: on NTFS, deleting a file in a nested
    // directory changes that directory's modification time only.
    let dir = tree(BASE);
    write(dir.path(), "docs/notes/old.md", "notes\n");
    let relative: Vec<String> = directories(dir.path())
        .unwrap()
        .iter()
        .map(|path| {
            let relative = path.strip_prefix(dir.path()).unwrap();
            relative.to_string_lossy().replace('\\', "/")
        })
        .collect();
    assert_eq!(
        relative,
        [
            "",
            "common",
            "common/a",
            "common/a/src",
            "cpu",
            "cpu/b",
            "cpu/b/src",
            "docs",
            "docs/notes"
        ]
    );
    assert!(directories(&dir.path().join("absent")).is_err());
}

#[test]
fn the_version_is_the_first_12_characters_of_the_commit_and_the_crc() {
    let dir = tree(BASE);
    let version = core_version(dir.path()).unwrap();
    assert_eq!(version, format!("jgenesis {}+{:08X}", &COMMIT[..12], crc(&dir)));

    // A new upstream commit is a new version, with the same sources.
    let other = "0123456789abcdef0123456789abcdef01234567";
    write(dir.path(), "PATCHES.md", &format!("- Commit: {other} (2027-01-01)\n"));
    assert_eq!(
        core_version(dir.path()).unwrap(),
        format!("jgenesis 0123456789ab+{:08X}", crc(&dir))
    );
}

#[test]
fn a_tree_without_sources_or_a_commit_is_an_error() {
    let empty = tree(&[]);
    let error = core_version(empty.path()).unwrap_err();
    assert!(error.contains("no sources"), "{error}");

    let no_commit = tree(BASE);
    for record in ["# Vendored\n", "- Commit: not-a-sha\n", "- Commit: 85ffe5f\n"] {
        write(no_commit.path(), "PATCHES.md", record);
        let error = core_version(no_commit.path()).unwrap_err();
        assert!(error.contains("Commit"), "{record}: {error}");
    }
    fs::remove_file(no_commit.path().join("PATCHES.md")).unwrap();
    assert!(core_version(no_commit.path()).unwrap_err().contains("PATCHES.md"));

    let missing = no_commit.path().join("absent");
    assert!(sources_crc(&missing).is_err());
    assert!(core_version(&missing).is_err());
}

#[test]
fn this_build_carries_the_version_of_the_vendored_sources() {
    let vendored = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../third_party/jgenesis");
    let expected = format!("{} oracle {ORACLE_REVISION}", core_version(&vendored).unwrap());
    assert_eq!(CORE_VERSION, expected);
    assert!(!ORACLE_REVISION.is_empty() && ORACLE_REVISION.bytes().all(|b| b.is_ascii_digit()));
}
