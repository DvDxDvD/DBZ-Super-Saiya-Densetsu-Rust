//! `xtask verify-replays` from the command line, where no ROM is needed.

use std::process::Command;

#[test]
fn bad_arguments_are_a_usage_error_not_an_unverified_run() {
    let empty = tempfile::tempdir().unwrap();
    for args in [&["--hooks", "verify"][..], &["--bogus"], &["a", "b"], &["--hooks"]] {
        let out = Command::new(env!("CARGO_BIN_EXE_xtask"))
            .env("SSD_DATA_DIR", empty.path())
            .arg("verify-replays")
            .args(args)
            .output()
            .expect("run xtask");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(64), "{args:?}: {stderr}");
        assert!(stderr.contains("usage") && stderr.contains("verify-replays"), "{stderr}");
        assert!(stderr.contains("exit 0"), "the help names the exit codes: {stderr}");
    }
}
