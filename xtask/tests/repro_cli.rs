//! `xtask repro` from the command line, where no ROM is needed.

use std::process::{Command, Output};

fn repro(data_dir: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_xtask"))
        .env("SSD_DATA_DIR", data_dir)
        .arg("repro")
        .args(args)
        .output()
        .expect("run xtask")
}

#[test]
fn bad_arguments_are_a_usage_error_naming_the_exit_codes() {
    let empty = tempfile::tempdir().unwrap();
    for args in [&[][..], &["--fast"], &["-"], &["a.ssdstate", "b.ssdstate"]] {
        let out = repro(empty.path(), args);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(64), "{args:?}: {stderr}");
        assert!(stderr.contains("usage") && stderr.contains("repro <path.ssdstate>"), "{stderr}");
        assert!(stderr.contains("repro:"), "the message names the mistake: {stderr}");
        assert!(
            stderr.contains("exit 0") && stderr.contains("3 it cannot be reproduced"),
            "{stderr}"
        );
    }
}

#[test]
fn a_missing_rom_cache_is_an_error_not_a_usage_error() {
    let empty = tempfile::tempdir().unwrap();
    let out = repro(empty.path(), &["leaf-1-2.ssdstate"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("ROM cache"), "{stderr}");
}
