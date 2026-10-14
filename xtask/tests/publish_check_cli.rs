//! End-to-end tests of `xtask publish-check` on scratch git repositories.

use std::path::Path;
use std::process::{Command, Output};

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args(["-c", "core.autocrlf=false"])
        .args(args)
        .status()
        .expect("run git");
    assert!(status.success(), "git {args:?} failed");
}

/// A scratch repository whose index holds the given files.
fn scratch_repo(files: &[(&str, &[u8])]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q"]);
    for (name, data) in files {
        std::fs::write(dir.path().join(name), data).unwrap();
        git(dir.path(), &["add", "-f", name]);
    }
    dir
}

/// A data directory whose ROM cache holds `rom`, or no cache at all.
fn data_dir(rom: Option<&[u8]>) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    if let Some(rom) = rom {
        let cache = dir.path().join("cache").join("rom");
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(cache.join("canonical.sfc"), rom).unwrap();
    }
    dir
}

fn publish_check(repo: &Path, data: &Path, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_xtask"))
        .current_dir(repo)
        .env("SSD_DATA_DIR", data)
        .arg("publish-check")
        .args(extra)
        .output()
        .expect("run xtask")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn fake_rom() -> Vec<u8> {
    (0..4096_u32).map(|i| (i * 7 + i / 13) as u8).collect()
}

#[test]
fn rom_scan_off_fails_without_the_flag() {
    let repo = scratch_repo(&[("README.md", b"hello\n")]);
    let out = publish_check(repo.path(), data_dir(None).path(), &[]);
    assert_eq!(out.status.code(), Some(1));
    let stderr = text(&out.stderr);
    assert!(stderr.contains("ROM scan is off"), "{stderr}");
    assert!(stderr.contains("--allow-no-rom"), "{stderr}");
    assert!(!text(&out.stdout).contains("clean"), "{}", text(&out.stdout));
}

#[test]
fn rom_scan_off_passes_with_allow_no_rom() {
    let repo = scratch_repo(&[("README.md", b"hello\n")]);
    let out = publish_check(repo.path(), data_dir(None).path(), &["--allow-no-rom"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(text(&out.stdout).contains("clean (1 files, ROM scan: off)"), "{}", text(&out.stdout));
}

#[test]
fn path_rules_still_apply_with_allow_no_rom() {
    let repo = scratch_repo(&[("game.sfc", b"x")]);
    let out = publish_check(repo.path(), data_dir(None).path(), &["--allow-no-rom"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stdout).contains("game.sfc: ROM-derived or scratch file type"));
}

#[test]
fn rom_scan_on_is_clean_for_unrelated_files() {
    let repo = scratch_repo(&[("README.md", b"hello\n")]);
    let out = publish_check(repo.path(), data_dir(Some(&fake_rom())).path(), &[]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stdout));
    assert!(text(&out.stdout).contains("clean (1 files, ROM scan: on)"));
}

#[test]
fn rom_scan_on_flags_pasted_literals_and_raw_bytes() {
    let rom = fake_rom();
    let table = rom[1000..1020].iter().map(|b| format!("0x{b:02X}, ")).collect::<Vec<_>>().concat();
    let source = format!("const T: &[u8] = &[\n    {table}\n];\n");
    let repo = scratch_repo(&[
        ("table.rs", source.as_bytes()),
        ("dump.dat", &rom[2000..2040]),
        ("ok.rs", b"const T: &[u8] = &[0x01, 0x02];\n"),
    ]);
    let out = publish_check(repo.path(), data_dir(Some(&rom)).path(), &[]);
    assert_eq!(out.status.code(), Some(1));
    let stdout = text(&out.stdout);
    assert!(
        stdout.contains("table.rs: contains ROM bytes as literals (run at line 2)"),
        "{stdout}"
    );
    assert!(stdout.contains("dump.dat: contains ROM bytes at offset 0"), "{stdout}");
    assert!(!stdout.contains("ok.rs"), "{stdout}");
}

#[test]
fn unknown_arguments_are_a_usage_error_with_its_own_exit_code() {
    let repo = scratch_repo(&[("README.md", b"hello\n")]);
    let out = publish_check(repo.path(), data_dir(None).path(), &["--bogus"]);
    assert_eq!(out.status.code(), Some(64), "not 2, the unverified code of verify-replays");
    assert!(text(&out.stderr).contains("usage"));
}

#[test]
fn rom_scan_on_flags_a_table_with_row_comments() {
    let rom = fake_rom();
    let rows = rom[1000..1020]
        .chunks(4)
        .enumerate()
        .map(|(n, row)| {
            let bytes = row.iter().map(|b| format!("0x{b:02X}, ")).collect::<Vec<_>>().concat();
            format!("    {bytes}// row {n}\n")
        })
        .collect::<Vec<_>>()
        .concat();
    let source = format!("const T: &[u8] = &[\n{rows}];\n");
    let repo = scratch_repo(&[("commented.rs", source.as_bytes())]);
    let out = publish_check(repo.path(), data_dir(Some(&rom)).path(), &[]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stdout));
    let stdout = text(&out.stdout);
    assert!(
        stdout.contains("commented.rs: contains ROM bytes as literals (run at line 2)"),
        "{stdout}"
    );
}

#[test]
fn a_rom_cache_without_windows_counts_as_scan_off() {
    let repo = scratch_repo(&[("README.md", b"hello\n")]);
    let empty: &[u8] = &[];
    for (name, cache) in [
        ("empty", empty.to_vec()),
        ("tiny", fake_rom()[..10].to_vec()),
        ("low-entropy", vec![0_u8; 4096]),
    ] {
        let data = data_dir(Some(&cache));
        let out = publish_check(repo.path(), data.path(), &[]);
        assert_eq!(out.status.code(), Some(1), "{name}");
        assert!(text(&out.stderr).contains("--allow-no-rom"), "{name}: {}", text(&out.stderr));
        let out = publish_check(repo.path(), data.path(), &["--allow-no-rom"]);
        assert_eq!(out.status.code(), Some(0), "{name}");
        assert!(text(&out.stdout).contains("ROM scan: off"), "{name}: {}", text(&out.stdout));
    }
}
