//! The twelve reference frames of M6 task 3: the Enhanced picture at 4x, drawn by `--ui-snapshot`
//! from the reference replays (`.work/replays`) and written to `.work/hd/` (untracked). Each PNG's
//! CRC-32 is asserted: the picture is a function of the replay, the ROM and the options, so the file
//! is the same on every run. Needs `SSD_ROM_PATH` (the ROM) and the replays; run with
//! `cargo test -p ssd-app --release --test hd_snapshots -- --ignored`.

use std::path::Path;
use std::process::Command;

/// The frames: the name of the PNG, the replay, the frames played before the picture, and the
/// CRC-32 of the PNG.
const FRAMES: [(&str, &str, u64, u32); 12] = [
    ("01-title", "landing", 1600, 0x4AC9_F043),
    ("02-flight", "landing", 500, 0x2C68_EC04),
    ("03-field", "story-1", 9500, 0x5AF6_9BA1),
    ("04-message", "landing", 2400, 0x1D41_EEC1),
    ("05-command-menu", "story-1", 9900, 0x772B_ADF4),
    ("06-menu-list", "secret-namek-zfighters", 2700, 0x2722_7FD2),
    ("07-menu-status", "secret-namek-zfighters", 3000, 0xC7F3_802D),
    ("08-battle-command", "story-1", 8400, 0xE641_D7D9),
    ("09-attack", "story-1", 8900, 0x303F_E058),
    ("10-ending-story", "secret-ending-normal", 6000, 0x8B79_134D),
    ("11-ending-credits", "secret-ending-normal", 9000, 0xA106_E9EA),
    ("12-ending-end", "secret-ending-normal", 15000, 0xDC5D_23C3),
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
        assert_eq!(actual, crc, "{name}: the picture's CRC-32 is {actual:08X}");
    }
}
