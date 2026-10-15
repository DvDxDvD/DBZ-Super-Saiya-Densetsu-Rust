//! The Gameplay page's host-side additions against the real game (M7): the message auto-run advances a message that
//! waits for A, and the frames of the battle's attack animation do not read the player's buttons, so running them
//! faster changes nothing the game decides.
//!
//! Needs the game's ROM:
//! `SSD_ROM_PATH=... cargo test -p ssd-game --release --test gameplay_real_rom -- --ignored --nocapture`.

use ssd_game::gameplay::{AutoRun, attack_animation, message_waits_for_a};
use ssd_game::scripts::{battle_input, story_1_input};
use ssd_menus::options::{Values, gameplay as page};
use ssd_oracle::{Joypad, Machine, MemorySaveStore, NullAudio};

/// The bus address of the first byte of work RAM, and the bytes the checks read.
const MESSAGE_STATE: usize = 0x071B;
const BATTLE_PHASE: usize = 0x00AB;
const PARTY_RECORDS: usize = 0x0B00;
const ENEMY_SLOTS: usize = 0x0E00;

fn game() -> Machine {
    let path = std::env::var("SSD_ROM_PATH").expect("SSD_ROM_PATH");
    let bytes = std::fs::read(path).unwrap();
    let rom = ssd_rom::normalize(&bytes, None).unwrap();
    assert_eq!(ssd_rom::crc32(&rom.bytes), ssd_rom::CANONICAL_CRC32, "not the canonical ROM");
    Machine::new(rom.bytes, Box::new(MemorySaveStore::default())).unwrap()
}

fn step(machine: &mut Machine, pad: Joypad) {
    machine.run_frame(pad, &mut NullAudio).unwrap();
}

/// A message that waits for A is closed by the auto-run, and stays up without it.
#[test]
#[ignore = "needs the ROM (SSD_ROM_PATH)"]
fn the_auto_run_closes_a_message_that_waits_for_a() {
    // The reference story: its first message waits for A at about frame 2616 (the script presses A at 2625).
    let to_waiting = || {
        let mut machine = game();
        while !message_waits_for_a(machine.wram()) {
            assert!(
                machine.frame_count() < 3_000,
                "a message waits for A in the first 3000 frames"
            );
            let pad = story_1_input(machine.frame_count());
            step(&mut machine, pad);
        }
        machine
    };
    let mut idle = to_waiting();
    let waiting_at = idle.frame_count();
    let state = idle.wram()[MESSAGE_STATE];
    println!("a message waits for A at frame {waiting_at} (state {state:#04x})");

    // Without the auto-run nobody presses: the message stays for 300 frames.
    for _ in 0..300 {
        step(&mut idle, Joypad::default());
    }
    assert_eq!(idle.wram()[MESSAGE_STATE], state, "the message waits as long as nobody presses");

    // With it, A comes after the wait (Short is 45 frames) and the message moves on.
    let mut values = Values::new();
    values.set(page::AUTO_RUN, 1);
    values.set(page::AUTO_WAIT, 0);
    let mut run = AutoRun::default();
    let mut machine = to_waiting();
    let mut moved_after = None;
    for frame in 0..200 {
        let mut pad = Joypad::default();
        run.apply(machine.wram(), &values, &mut pad);
        step(&mut machine, pad);
        if !message_waits_for_a(machine.wram()) {
            moved_after = Some(frame + 1);
            break;
        }
    }
    let moved = moved_after.expect("the message moved on");
    println!("the auto-run moved the message on after {moved} frames");
    assert!((45..=60).contains(&moved), "after the wait of 45 frames and the press: {moved}");
}

/// What the battle decided at the end of a frame: the phase, the party records and the enemy slots.
fn battle_digest(machine: &Machine) -> (u8, Vec<u8>, Vec<u8>) {
    let wram = machine.wram();
    (
        wram[BATTLE_PHASE],
        wram[PARTY_RECORDS..PARTY_RECORDS + 0x120].to_vec(),
        wram[ENEMY_SLOTS..ENEMY_SLOTS + 0xA0].to_vec(),
    )
}

/// The attack animation does not read the buttons: the reference battle, played with the pad cleared on every
/// frame of the animation, passes through the same phases at the same frames and ends in the same state.
#[test]
#[ignore = "needs the ROM (SSD_ROM_PATH)"]
fn the_attack_animation_ignores_the_pad() {
    let frames = 7_100;
    let mut normal = game();
    let mut cleared = game();
    let mut animation_frames = 0_u32;
    let mut phases = Vec::new();
    for frame in 0..frames {
        let pad = battle_input(frame);
        let in_animation = attack_animation(cleared.wram());
        assert_eq!(
            attack_animation(normal.wram()),
            in_animation,
            "the same machine state at frame {frame}"
        );
        step(&mut normal, pad);
        step(&mut cleared, if in_animation { Joypad::default() } else { pad });
        if in_animation {
            animation_frames += 1;
        }
        assert_eq!(battle_digest(&normal), battle_digest(&cleared), "frame {frame}");
        if phases.last() != Some(&normal.wram()[BATTLE_PHASE]) {
            phases.push(normal.wram()[BATTLE_PHASE]);
        }
    }
    assert_eq!(normal.wram(), cleared.wram(), "the whole work RAM is the same at the end");
    println!(
        "{animation_frames} of {frames} frames are the attack animation; phases seen: {phases:02X?}"
    );
    assert!(animation_frames > 600, "the reference battle has attack animations");
}
