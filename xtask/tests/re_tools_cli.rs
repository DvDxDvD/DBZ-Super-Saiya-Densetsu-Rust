//! The reverse-engineering commands from the command line, where no ROM is needed: a bad command
//! line is a usage error (64) that names the command.

use std::process::Command;

fn run(command: &str, args: &[&str]) -> (Option<i32>, String) {
    let empty = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .env("SSD_DATA_DIR", empty.path())
        .arg(command)
        .args(args)
        .output()
        .expect("run xtask");
    (out.status.code(), String::from_utf8_lossy(&out.stderr).into_owned())
}

#[test]
fn bad_arguments_are_a_usage_error_naming_the_command() {
    let cases: [(&str, &[&str]); 12] = [
        ("disasm", &[]),
        ("disasm", &["a", "b"]),
        ("disasm", &["a", "--len"]),
        ("disasm", &["a", "--bogus"]),
        ("routines", &["--top"]),
        ("routines", &["--bogus"]),
        ("probe", &[]),
        ("probe", &["--address", "05B572"]),
        ("probe", &["--address", "05B572", "--returns", "rts", "--verify"]),
        ("shots", &[]),
        ("shots", &["menus"]),
        ("shots", &["menus", "--every"]),
    ];
    for (command, args) in cases {
        let (code, stderr) = run(command, args);
        assert_eq!(code, Some(64), "{command} {args:?}: {stderr}");
        assert!(stderr.contains(command), "{command} {args:?}: {stderr}");
        assert!(stderr.contains("usage"), "{stderr}");
    }
}

#[test]
fn the_help_describes_the_new_commands() {
    let (code, stderr) = run("nothing", &[]);
    assert_eq!(code, Some(64));
    for command in ["disasm", "routines", "probe", "shots"] {
        assert!(stderr.contains(&format!("xtask -- {command}")), "{command}: {stderr}");
    }
}
