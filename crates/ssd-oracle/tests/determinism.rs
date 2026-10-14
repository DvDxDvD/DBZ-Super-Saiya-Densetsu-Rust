mod common;

use ssd_oracle::{Joypad, Machine, MemorySaveStore, NullAudio};

#[test]
fn two_fresh_machines_are_byte_identical() {
    let mk =
        || Machine::new(common::red_backdrop_rom(), Box::new(MemorySaveStore::default())).unwrap();
    let (mut a, mut b) = (mk(), mk());
    assert_eq!(a.save_state(), b.save_state(), "power-on state must be deterministic");
    for _ in 0..10 {
        a.run_frame(Joypad::default(), &mut NullAudio).unwrap();
        b.run_frame(Joypad::default(), &mut NullAudio).unwrap();
    }
    assert_eq!(a.save_state(), b.save_state());
}
