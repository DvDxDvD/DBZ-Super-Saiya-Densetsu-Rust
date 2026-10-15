//! The twelve reference frames of M6 task 3: the Enhanced picture at 4x, drawn by `--ui-snapshot`
//! from the reference replays (`.work/replays`) and written to `.work/hd/` (untracked). Each PNG's
//! CRC-32 is asserted: the picture is a function of the replay, the ROM and the options, so the file
//! is the same on every run. Needs `SSD_ROM_PATH` (the ROM) and the replays; run with
//! `cargo test -p ssd-app --release --test hd_snapshots -- --ignored`.

use std::path::Path;
use std::process::Command;

/// The frames: the name of the PNG, the replay, the frames played before the picture, and the
/// CRC-32 of the PNG. All twelve took new values on 2026-10-15 for one reason that is not a code
/// change: the GPU path now rounds the texture filtering one step differently (a channel off by
/// one on 0.1 to 0.5 percent of the pixels, never more than two), and the same CRCs come out of the
/// previous recording's own commit. If a CRC moves again with no change to the picture, compare
/// the PNG with the last one pixel by pixel before taking the new value.
const FRAMES: [(&str, &str, u64, u32); 12] = [
    ("01-title", "landing", 1600, 0x60B3_5869),
    ("02-flight", "landing", 500, 0xC8CC_2E4D),
    ("03-field", "story-1", 9500, 0x3059_AECE),
    ("04-message", "landing", 2400, 0x25B6_20FC),
    ("05-command-menu", "story-1", 9900, 0xB8E5_1A62),
    ("06-menu-list", "secret-namek-zfighters", 2700, 0x117F_A86D),
    ("07-menu-status", "secret-namek-zfighters", 3000, 0xE50A_6715),
    ("08-battle-command", "story-1", 8400, 0xF17A_97F7),
    ("09-attack", "story-1", 8900, 0x0169_0AEB),
    ("10-ending-story", "secret-ending-normal", 6000, 0x797C_BD77),
    ("11-ending-credits", "secret-ending-normal", 9000, 0x605A_B5DA),
    ("12-ending-end", "secret-ending-normal", 15000, 0x8009_262D),
];

#[test]
#[ignore = "needs SSD_ROM_PATH and the reference replays"]
fn the_twelve_reference_frames_at_four_times_match_their_crcs() {
    let rom = std::env::var("SSD_ROM_PATH").unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    let replays = root.join(".work").join("replays");
    let out_dir = root.join(".work").join("hd");
    std::fs::create_dir_all(&out_dir).unwrap();
    let data = tempfile::tempdir().unwrap();
    let mut mismatches = Vec::new();
    for (name, replay, frames, crc) in FRAMES {
        let out = out_dir.join(format!("{name}.png"));
        let run = Command::new(env!("CARGO_BIN_EXE_ssd-recompiled"))
            .args(["--rom", &rom, "--replay"])
            .arg(replays.join(format!("{replay}.ssdreplay")))
            .args(["--headless-frames", &frames.to_string()])
            .args(["--hd-scale", "4", "--no-hd-font", "--ui-size", "1170x956", "--ui-snapshot"])
            .arg(&out)
            .env("SSD_DATA_DIR", data.path())
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&run.stderr);
        assert_eq!(run.status.code(), Some(0), "{name}: stderr: {stderr}");
        let actual = ssd_rom::crc32(&std::fs::read(&out).unwrap());
        if actual != crc {
            mismatches
                .push(format!("{name}: the picture's CRC-32 is {actual:08X}, expected {crc:08X}"));
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}
