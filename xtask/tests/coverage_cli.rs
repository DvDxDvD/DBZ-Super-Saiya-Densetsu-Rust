//! `xtask coverage` from the command line, where no ROM is needed.

use std::process::{Command, Output};

fn coverage(data_dir: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_xtask"))
        .env("SSD_DATA_DIR", data_dir)
        .arg("coverage")
        .args(args)
        .output()
        .expect("run xtask")
}

#[test]
fn bad_arguments_are_a_usage_error() {
    let empty = tempfile::tempdir().unwrap();
    for args in [
        &["--hooks", "rust"][..],
        &["--bogus"],
        &["a", "b"],
        &["a", "--fast"],
        &["--exclude"],
        &["--exclude", ""],
        &["--exclude", "rng_next_a,"],
    ] {
        let out = coverage(empty.path(), args);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(64), "{args:?}: {stderr}");
        assert!(stderr.contains("usage") && stderr.contains("coverage [--exclude"), "{stderr}");
        assert!(stderr.contains("coverage:"), "the message names the mistake: {stderr}");
    }
}

#[test]
fn a_routine_to_exclude_must_exist_and_is_checked_before_the_rom() {
    let empty = tempfile::tempdir().unwrap();
    for args in [&["--exclude", "rng_next_c"][..], &["--exclude", "rng_next_a,main_loop_idle"]] {
        let out = coverage(empty.path(), args);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(64), "{args:?}: {stderr}");
        assert!(stderr.contains("is not a routine") && stderr.contains("rng_next_a"), "{stderr}");
    }
}

#[test]
fn a_missing_rom_cache_is_an_error_not_a_usage_error() {
    let empty = tempfile::tempdir().unwrap();
    for args in [&[][..], &["--exclude", "rng_next_a,rng_next_b"]] {
        let out = coverage(empty.path(), args);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(1), "{args:?}: {stderr}");
        assert!(stderr.contains("ROM cache"), "{stderr}");
    }
}
