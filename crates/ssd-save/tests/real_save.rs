//! Round trip on a real save file: `SSD_SAVE_PATH` names it (the tests are ignored without it:
//! `SSD_SAVE_PATH=... cargo test -p ssd-save -- --ignored`). Nothing of the file is checked in.

use ssd_save::{SaveFile, SlotStatus};

fn real_save() -> Option<Vec<u8>> {
    let path = std::env::var_os("SSD_SAVE_PATH")?;
    Some(std::fs::read(path).expect("SSD_SAVE_PATH is readable"))
}

#[test]
#[ignore = "needs SSD_SAVE_PATH, a save file of the game"]
fn every_good_slot_of_a_real_save_survives_a_read_and_write() {
    let bytes = real_save().expect("SSD_SAVE_PATH is set");
    let original = SaveFile::from_bytes(&bytes).unwrap();
    let mut file = original.clone();
    let mut good = 0;
    for i in 0..ssd_save::SLOT_COUNT {
        match original.status(i).unwrap() {
            SlotStatus::Good => {
                good += 1;
                let slot = original.slot(i).unwrap();
                file.set_slot(i, &slot).unwrap();
            }
            SlotStatus::Empty => {}
            other => panic!("slot {i} is {other:?}"),
        }
    }
    assert!(good > 0, "the save has no slot in use");
    assert_eq!(file, original, "writing a slot back changes nothing");
}

#[test]
#[ignore = "needs SSD_SAVE_PATH, a save file of the game"]
fn the_first_slot_decodes_to_a_party() {
    let bytes = real_save().expect("SSD_SAVE_PATH is set");
    let file = SaveFile::from_bytes(&bytes).unwrap();
    let slot = file.slot(0).unwrap();
    let party = slot.party();
    assert!(!party.is_empty(), "a party");
    for id in party {
        let record = slot.record(id).unwrap();
        assert_eq!(record.flags(), 0xC0, "member {id} is in use");
        assert_eq!(record.character_id(), id);
        assert!(record.hp() <= record.max_hp() && record.level() >= 1);
    }
    assert!(slot.zeni() <= ssd_save::ZENI_MAX);
    assert!(slot.items().len() <= ssd_save::ITEM_SLOTS);
}
