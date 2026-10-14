mod common;

use ssd_oracle::{
    AudioSink, Color, FsSaveStore, Joypad, Machine, MemorySaveStore, NullAudio, OracleError,
};

fn machine() -> Machine {
    Machine::new(common::red_backdrop_rom(), Box::new(MemorySaveStore::default())).unwrap()
}

#[test]
fn renders_red_backdrop() {
    let mut m = machine();
    for _ in 0..3 {
        m.run_frame(Joypad::default(), &mut NullAudio).unwrap();
    }
    let f = m.frame();
    assert!(f.size.width >= 256 && f.size.height >= 224);
    let center = f.pixels[(f.size.height / 2 * f.size.width + f.size.width / 2) as usize];
    assert!(center.r >= 200 && center.g <= 30 && center.b <= 30, "center pixel {center:?}");
    assert_eq!(m.frame_count(), 3);
}

#[test]
fn save_state_round_trip_restores_exact_state() {
    // State bytes are deterministic (no hash maps in the core), so they are the sharpest comparison.
    let mut m = machine();
    m.run_frame(Joypad::default(), &mut NullAudio).unwrap();
    let s1 = m.save_state();
    for _ in 0..5 {
        m.run_frame(Joypad::default(), &mut NullAudio).unwrap();
    }
    assert_ne!(m.save_state(), s1, "running frames must change the state");
    m.load_state(&s1).unwrap();
    assert_eq!(m.save_state(), s1);
    assert_eq!(m.frame_count(), 1);
}

#[test]
fn corrupt_state_is_rejected_and_machine_untouched() {
    let mut m = machine();
    m.run_frame(Joypad::default(), &mut NullAudio).unwrap();
    let before = m.save_state();
    assert!(m.load_state(b"definitely not a state").is_err());
    let mut truncated = before.clone();
    truncated.truncate(truncated.len() / 2);
    assert!(m.load_state(&truncated).is_err());
    let mut flipped = before.clone();
    let mid = flipped.len() / 2;
    flipped[mid] ^= 0x5A;
    assert!(m.load_state(&flipped).is_err(), "payload CRC must catch a flipped byte");
    assert_eq!(m.save_state(), before);
    m.run_frame(Joypad::default(), &mut NullAudio).unwrap();
}

#[test]
fn flipped_frame_counter_is_rejected() {
    let mut m = machine();
    m.run_frame(Joypad::default(), &mut NullAudio).unwrap();
    let mut s = m.save_state();
    s[20] ^= 0x01; // frames field: magic 8 + format 4 + crc 4 + rom_crc 4
    assert!(m.load_state(&s).is_err());
}

#[test]
fn older_format_states_are_rejected_with_a_clear_message() {
    let mut m = machine();
    let mut s = m.save_state();
    s[8..12].copy_from_slice(&1_u32.to_le_bytes());
    let err = m.load_state(&s).unwrap_err().to_string();
    assert!(err.contains("format 1"), "{err}");
}

#[test]
fn state_from_another_rom_is_rejected() {
    let mut m = machine();
    let state = m.save_state();
    let mut other_rom = common::red_backdrop_rom();
    other_rom[0x7FC0] = b'X';
    ssd_rom::fix_lorom_checksum(&mut other_rom);
    let mut other = Machine::new(other_rom, Box::new(MemorySaveStore::default())).unwrap();
    assert!(other.load_state(&state).is_err());
    m.load_state(&state).unwrap();
}

/// Counts the samples the core pushes.
#[derive(Default)]
struct CountingAudio(usize);

impl AudioSink for CountingAudio {
    fn push(&mut self, _left: f32, _right: f32) {
        self.0 += 1;
    }
}

#[test]
fn load_state_keeps_the_configured_audio_rate() {
    let mut m = machine();
    // The first frame of a fresh machine yields fewer samples (resampler warm-up): get past it.
    for _ in 0..2 {
        m.run_frame(Joypad::default(), &mut NullAudio).unwrap();
    }
    m.set_audio_frequency(22_050);
    let state = m.save_state();
    m.set_audio_frequency(48_000);
    m.load_state(&state).unwrap();
    // The state was saved at 22.05 kHz, but the host asked for 48 kHz: loading must not revert it.
    let mut audio = CountingAudio::default();
    m.run_frame(Joypad::default(), &mut audio).unwrap();
    let expected = 48_000.0 / m.target_fps();
    let got = audio.0 as f64;
    assert!(
        (expected * 0.9..=expected * 1.1).contains(&got),
        "{got} samples, expected ~{expected}"
    );
}

#[test]
fn frame_is_current_right_after_load_state() {
    let mut a = machine();
    for _ in 0..3 {
        a.run_frame(Joypad::default(), &mut NullAudio).unwrap();
    }
    let state = a.save_state();

    let mut b = machine();
    let f = b.frame();
    assert_eq!((f.size.width, f.size.height), (256, 224), "fresh machine shows the initial frame");
    assert_eq!(f.pixels[f.pixels.len() / 2], Color::BLACK);

    b.load_state(&state).unwrap();
    let f = b.frame();
    let center = f.pixels[(f.size.height / 2 * f.size.width + f.size.width / 2) as usize];
    assert!(center.r >= 200 && center.g <= 30 && center.b <= 30, "center pixel {center:?}");
}

#[test]
fn unreadable_sav_aborts_machine_creation() {
    // A directory where the .sav file should be: reading it fails with an error that is not NotFound.
    // Silently starting with blank SRAM would later overwrite the player's real save.
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("ssd.sav")).unwrap();
    let saves = FsSaveStore::new(dir.path().to_path_buf(), "ssd").unwrap();
    let result = Machine::new(common::red_backdrop_rom(), Box::new(saves));
    assert!(matches!(result, Err(OracleError::Io(_))), "expected OracleError::Io");
}

#[test]
fn missing_sav_is_not_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let saves = FsSaveStore::new(dir.path().to_path_buf(), "ssd").unwrap();
    assert!(Machine::new(common::red_backdrop_rom(), Box::new(saves)).is_ok());
}
