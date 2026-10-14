//! WRAM freezes: bytes held at a value at every frame boundary (`Machine::set_freezes`).

mod common;

use common::{LEAF, LEAF_COUNTER};
use ssd_oracle::{
    Ctx, Freeze, HookAction, HookEntry, HookMode, HookTable, Joypad, Machine, MemorySaveStore,
    NullAudio, OracleError, Recorder, ReturnKind,
};

const FROZEN: u8 = 0x55;

fn machine() -> Machine {
    Machine::new(common::jsr_leaf_rom(), Box::new(MemorySaveStore::default())).unwrap()
}

fn run(m: &mut Machine, frames: usize) {
    for _ in 0..frames {
        m.run_frame(Joypad::default(), &mut NullAudio).unwrap();
    }
}

fn freeze(address: u32, bytes: &[u8]) -> Freeze {
    Freeze { address, bytes: bytes.to_vec() }
}

/// The leaf of [`common::jsr_leaf_rom`], `INC $10; RTS`.
fn leaf(ctx: &mut Ctx<'_>) {
    let value = ctx.read_dp8(LEAF_COUNTER as u16).wrapping_add(1);
    ctx.write_dp8(LEAF_COUNTER as u16, value);
    ctx.set_nz8(value);
    ctx.set_cost(80);
}

fn leaf_entry(mode: HookMode) -> HookEntry {
    HookEntry {
        name: "leaf".to_owned(),
        key: LEAF,
        mode,
        action: HookAction::Routine { func: leaf, ret: ReturnKind::Rts },
        entry_m8: Some(true),
        entry_x8: Some(false),
        flags_mask: 0xFF,
        verify_sample: 1,
        bound: None,
        budget_frames: None,
        cycles: Some(80),
    }
}

fn error_text(result: Result<(), OracleError>) -> String {
    let error = result.unwrap_err();
    assert!(matches!(error, OracleError::Freeze(_)), "{error}");
    error.to_string()
}

#[test]
fn a_frozen_byte_keeps_its_value_at_every_frame_boundary_while_the_game_writes_it() {
    let mut free = machine();
    let mut seen = Vec::new();
    for _ in 0..6 {
        run(&mut free, 1);
        seen.push(free.wram()[LEAF_COUNTER]);
    }
    assert!(seen.windows(2).all(|w| w[0] != w[1]), "the game changes it every frame: {seen:?}");

    let mut m = machine();
    m.set_cpu_write_log(true);
    m.set_freezes(vec![freeze(0x7E_0000 + LEAF_COUNTER as u32, &[FROZEN])]).unwrap();
    for frame in 0..6 {
        run(&mut m, 1);
        assert_eq!(m.wram()[LEAF_COUNTER], FROZEN, "frame {frame}");
        // The game wrote it all frame long, the freeze is not in its log, and the log stays on.
        let writes = m.take_cpu_writes();
        let game = writes.iter().filter(|&&(address, _)| address == LEAF_COUNTER as u32).count();
        assert!(game > 10, "frame {frame}: {game} writes");
        assert!(writes.iter().all(|&(address, _)| address != 0x7E_0000 + LEAF_COUNTER as u32));
    }
}

#[test]
fn mirrors_and_the_upper_bank_work_and_are_stored_canonically() {
    let mut m = machine();
    m.set_freezes(vec![
        freeze(0x00_0010, &[0xA1]),
        freeze(0x80_0021, &[0xA2, 0xA3]),
        freeze(0x7F_0000, &[0xB1, 0xB2]),
        freeze(0x3F_1FFF, &[0xC1]),
    ])
    .unwrap();
    let addresses: Vec<u32> = m.freezes().iter().map(|f| f.address).collect();
    assert_eq!(addresses, [0x7E_0010, 0x7E_0021, 0x7F_0000, 0x7E_1FFF]);
    run(&mut m, 2);
    let wram = m.wram();
    assert_eq!((wram[0x10], wram[0x21], wram[0x22]), (0xA1, 0xA2, 0xA3));
    assert_eq!((wram[0x1_0000], wram[0x1_0001]), (0xB1, 0xB2));
    assert_eq!(wram[0x1FFF], 0xC1);
}

#[test]
fn freezes_outside_wram_are_an_error_and_change_nothing() {
    let mut m = machine();
    m.set_freezes(vec![freeze(0x7E_0010, &[1])]).unwrap();
    for (what, bad) in [
        ("end of WRAM", freeze(0x7F_FFFF, &[1, 2])),
        ("past WRAM", freeze(0x80_0000 + 0x7F_0000, &[1])),
        ("register space", freeze(0x00_2100, &[1])),
        ("ROM", freeze(0x00_8000, &[1])),
        ("a mirror crossing its end", freeze(0x00_1FFF, &[1, 2])),
        ("no bytes", freeze(0x7E_0000, &[])),
        ("above 24 bits", freeze(0x017E_0000, &[1])),
    ] {
        let text = error_text(m.set_freezes(vec![freeze(0x7E_0020, &[2]), bad]));
        assert!(text.contains("WRAM"), "{what}: {text}");
        assert!(!text.contains("  "), "{what}: no stray spaces: {text}");
        assert_eq!(m.freezes(), [freeze(0x7E_0010, &[1])], "{what}: untouched");
    }
    // The last byte of WRAM is fine.
    m.set_freezes(vec![freeze(0x7F_FFFF, &[1])]).unwrap();
}

#[test]
fn a_machine_with_freezes_cannot_be_recorded() {
    let mut m = machine();
    m.set_freezes(vec![freeze(0x7E_0010, &[1])]).unwrap();
    let error = Recorder::start(&m).map(drop).unwrap_err().to_string();
    assert!(error.contains("freeze"), "{error}");

    m.set_freezes(Vec::new()).unwrap();
    let mut recorder = Recorder::start(&m).unwrap();
    recorder.run_frame(&mut m, Joypad::default(), &mut NullAudio).unwrap();
    m.set_freezes(vec![freeze(0x7E_0010, &[1])]).unwrap();
    let error = recorder.run_frame(&mut m, Joypad::default(), &mut NullAudio).unwrap_err();
    assert!(error.to_string().contains("freeze"), "{error}");
    assert_eq!(recorder.replay().frames(), 1, "nothing recorded for the refused frame");
}

#[test]
fn a_verify_leaf_reading_the_frozen_byte_shows_no_divergence() {
    let mut m = machine();
    m.set_hook_table(Some([leaf_entry(HookMode::Verify)].into_iter().collect::<HookTable>()));
    m.set_freezes(vec![freeze(0x7E_0000 + LEAF_COUNTER as u32, &[FROZEN])]).unwrap();
    run(&mut m, 4);
    let (_, stats) = m.hook_stats().remove(0);
    assert!(stats.verified_ok > 100, "{stats:?}");
    assert_eq!(stats.divergences, 0, "{stats:?}");
    assert_eq!(m.wram()[LEAF_COUNTER], FROZEN);
}

#[test]
fn a_rust_dispatch_that_renders_a_frame_is_frozen_after_its_render() {
    let mut m = machine();
    m.set_hook_table(Some([leaf_entry(HookMode::Rust)].into_iter().collect::<HookTable>()));
    m.set_freezes(vec![freeze(0x7E_0000 + LEAF_COUNTER as u32, &[FROZEN])]).unwrap();
    for frame in 0..40 {
        run(&mut m, 1);
        // A dispatch that renders runs on and writes after the freeze (documented): one more.
        let value = m.wram()[LEAF_COUNTER];
        assert!(value == FROZEN || value == FROZEN + 1, "frame {frame}: {value}");
    }
}

#[test]
fn clearing_the_freezes_restores_normal_behaviour() {
    let mut m = machine();
    m.set_freezes(vec![freeze(0x7E_0000 + LEAF_COUNTER as u32, &[FROZEN])]).unwrap();
    run(&mut m, 2);
    m.set_freezes(Vec::new()).unwrap();
    assert_eq!(m.freezes(), []);
    let mut seen = Vec::new();
    for _ in 0..6 {
        run(&mut m, 1);
        seen.push(m.wram()[LEAF_COUNTER]);
    }
    assert!(seen.windows(2).all(|w| w[0] != w[1]), "the game changes it again: {seen:?}");
}

#[test]
fn freezes_are_not_in_a_save_state_but_stay_across_a_load_and_a_snapshot() {
    let mut m = machine();
    run(&mut m, 2);
    let state = m.save_state();
    m.set_freezes(vec![freeze(0x7E_0000 + LEAF_COUNTER as u32, &[FROZEN])]).unwrap();
    assert_eq!(m.snapshot().freezes(), m.freezes());
    run(&mut m, 1);
    m.load_state(&state).unwrap();
    assert_eq!(m.freezes().len(), 1);
    assert_ne!(m.wram()[LEAF_COUNTER], FROZEN, "the state holds the game's value");
    run(&mut m, 1);
    assert_eq!(m.wram()[LEAF_COUNTER], FROZEN);
}
