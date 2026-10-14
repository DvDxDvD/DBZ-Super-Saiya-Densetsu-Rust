//! The object pictures and the item catalogue read from the real ROM.
//!
//! Needs the game's ROM:
//! `SSD_ROM_PATH=... cargo test -p ssd-catalogue --release --test art_real_rom -- --ignored`.
//! The checks are structural (sizes, ranges, the fields of one object); no decoded picture is
//! written down anywhere.

use ssd_catalogue::{Catalogue, RomView, object_portrait};

fn rom() -> Vec<u8> {
    let path = std::env::var("SSD_ROM_PATH").expect("SSD_ROM_PATH");
    let bytes = std::fs::read(path).unwrap();
    let rom = ssd_rom::normalize(&bytes, None).unwrap();
    assert_eq!(ssd_rom::crc32(&rom.bytes), ssd_rom::CANONICAL_CRC32, "not the canonical ROM");
    rom.bytes
}

#[test]
#[ignore = "needs the ROM"]
fn every_object_has_a_picture_of_32_by_32_pixels_and_a_palette() {
    let rom = rom();
    let view = RomView::new(&rom).unwrap();
    for object in 0..40 {
        let portrait =
            object_portrait(view, object).unwrap_or_else(|e| panic!("object {object}: {e}"));
        assert_eq!(portrait.pixels.len(), 32 * 32, "object {object}");
        assert!(portrait.pixels.iter().all(|&index| index < 16), "object {object}");
        assert!(portrait.pixels.iter().any(|&index| index != 0), "object {object}: a picture");
        assert_eq!(portrait.palette.len(), 16, "object {object}");
    }
}

#[test]
#[ignore = "needs the ROM"]
fn bulma_costs_three_pt_and_her_help_is_the_restore_line_of_the_screen() {
    let catalogue = Catalogue::from_rom(&rom()).unwrap();
    let bulma = &catalogue.items[2];
    assert_eq!(bulma.name.text, "Bulma");
    assert_eq!(bulma.price, Some(3));
    assert!(
        bulma.usable_in_battle() && bulma.usable_on_map(),
        "Bulma heals on the map and in battle"
    );
    let lines: Vec<&str> = bulma.help.iter().map(|line| line.text.as_str()).collect();
    assert_eq!(lines, ["Restores 25", "HP."], "the first page of the help text");
    assert!(bulma.effect.is_some(), "Bulma's effect is in the table");
}

#[test]
#[ignore = "needs the ROM"]
fn every_object_has_a_name_and_at_most_three_help_lines() {
    let catalogue = Catalogue::from_rom(&rom()).unwrap();
    assert_eq!(catalogue.items.len(), 40);
    for item in &catalogue.items {
        assert!(!item.name.text.is_empty(), "object {}", item.id);
        assert!(item.help.len() <= 3, "object {}", item.id);
    }
}

/// The picture of object 13 as the original decodes it into work RAM. `SSD_REPLAY` names a replay of
/// the item list (`docs/re/data/menus.md`, the crafted item save): at frame 2680 the cursor is on
/// Oolong, and the last picture decoded is his. The work RAM at `$7E:9000` holds that picture after
/// the decoder's plane conversion, which is checked here byte for byte.
#[test]
#[ignore = "needs the ROM and a replay: SSD_ROM_PATH and SSD_REPLAY"]
fn the_decoded_picture_is_the_one_the_original_writes_to_work_ram() {
    let rom = rom();
    let replay_path = std::env::var("SSD_REPLAY").expect("SSD_REPLAY");
    let bytes = std::fs::read(replay_path).unwrap();
    let replay =
        ssd_oracle::Replay::from_bytes(&bytes, ssd_rom::CANONICAL_CRC32, ssd_oracle::CORE_VERSION)
            .unwrap();
    let mut machine = replay.power_on(rom.clone()).unwrap();
    for frame in 0..2680 {
        machine.run_frame(replay.input(frame), &mut ssd_oracle::NullAudio).unwrap();
    }

    let view = RomView::new(&rom).unwrap();
    let entry = ssd_game::data::OBJECT_PORTRAIT_POINTERS + 3 * 13;
    let pointer = view.pointer(entry, view.u8(entry + 2).unwrap()).unwrap();
    let set = ssd_catalogue::lz_decode(view, pointer).unwrap();
    // The plane conversion of `00:C68C`: each 16-byte block takes the first byte of each plane pair
    // from the first eight bytes and the second from the next eight, interleaved.
    let converted: Vec<u8> = set
        .chunks(16)
        .flat_map(|block| {
            let mut out = [0_u8; 16];
            for k in 0..8 {
                out[2 * k] = block[k];
                out[2 * k + 1] = block[8 + k];
            }
            out
        })
        .collect();
    assert_eq!(&machine.wram()[0x9000..0x9000 + converted.len()], &converted[..]);
}

/// Which half of its set each object shows, read off the original's Item screen (the picture box
/// of a frame matched pixel by pixel against both halves, in the object's palette): objects that
/// share a set take different halves, and not in the order of their numbers.
#[test]
#[ignore = "needs the ROM"]
fn the_halves_are_the_ones_the_original_draws_in_its_picture_box() {
    let rom = rom();
    let view = RomView::new(&rom).unwrap();
    let second: [u8; 16] = [1, 3, 6, 10, 12, 14, 15, 17, 18, 19, 22, 23, 26, 27, 30, 31];
    for object in 0..40_u8 {
        let portrait = object_portrait(view, object).unwrap();
        // The Dragon Balls' two halves look the same; the others are checked
        if object >= 33 && object != 39 {
            continue;
        }
        assert_eq!(portrait.half, u8::from(second.contains(&object)), "object {object}");
    }
}
