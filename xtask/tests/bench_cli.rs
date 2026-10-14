//! `xtask bench` from the command line, where no ROM is needed.

use std::process::{Command, Output};

fn bench(data_dir: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_xtask"))
        .env("SSD_DATA_DIR", data_dir)
        .arg("bench")
        .args(args)
        .output()
        .expect("run xtask")
}

#[test]
fn arguments_are_a_usage_error_that_lists_the_command() {
    let empty = tempfile::tempdir().unwrap();
    for args in [&["--frames", "10"][..], &["600"]] {
        let out = bench(empty.path(), args);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(64), "{args:?}: {stderr}");
        assert!(stderr.contains("bench: takes no arguments"), "{stderr}");
        assert!(stderr.contains("-- bench"), "the usage lists it: {stderr}");
    }
}

#[test]
fn a_missing_rom_cache_is_an_error_not_a_usage_error() {
    let empty = tempfile::tempdir().unwrap();
    let out = bench(empty.path(), &[]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("ROM cache"), "{stderr}");
}
