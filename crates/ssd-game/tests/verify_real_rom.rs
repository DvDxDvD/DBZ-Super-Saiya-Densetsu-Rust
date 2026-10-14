//! Every routine of `re/symbols.toml` checked call by call against the original code of the
//! real game (all in `Verify`), and all of them in `Rust` mode against a pure twin.
//!
//! Needs the game's ROM:
//! `SSD_ROM_PATH=... cargo test -p ssd-game --release --test verify_real_rom -- --ignored --nocapture`.
//! With `SSD_DIVERGENCE_DIR` set, divergence reports are saved there (use `.work/divergences`).

use ssd_game::scripts::{BATTLE_FRAMES, battle_input};
use ssd_oracle::{
    HookMode, HookStats, Joypad, Machine, MemorySaveStore, NullAudio, Symbols, TickEvent, hook_key,
};
use std::sync::Arc;

/// The routine whose true path (a special id) the traces never reached.
const SPECIAL_SET: &str = "entity_id_in_special_set";
/// CPU-busy master cycles of its false path (every id compared): the measured constant.
const SPECIAL_SET_FALSE_PATH: u64 = 256;
/// The ids of its set.
const SPECIAL_IDS: [u8; 5] = [0x10, 0x11, 0x4C, 0x4D, 0x4F];
/// WRAM offset of the id byte of entity slot 0 (`entity_ids`); the routine reads `DBR:offset + Y`.
const ENTITY_ID: u32 = ssd_game::data::ENTITY_IDS & 0xFFFF;

fn symbols() -> Symbols {
    Symbols::load(concat!(env!("CARGO_MANIFEST_DIR"), "/../../re/symbols.toml")).unwrap()
}

/// The canonical game on a fresh machine, without hooks.
fn pure_game() -> Machine {
    let path = std::env::var("SSD_ROM_PATH").expect("SSD_ROM_PATH");
    let bytes = std::fs::read(path).unwrap();
    let rom = ssd_rom::normalize(&bytes, None).unwrap();
    assert_eq!(ssd_rom::crc32(&rom.bytes), ssd_rom::CANONICAL_CRC32, "not the canonical ROM");
    Machine::new(rom.bytes, Box::new(MemorySaveStore::default())).unwrap()
}

/// The canonical game on a fresh machine, every routine of `re/symbols.toml` in `mode`.
fn game_in(mode: HookMode) -> Machine {
    let mut m = pure_game();
    let symbols = symbols();
    m.set_hook_table(Some(ssd_game::hook_table(&symbols, Some(mode)).unwrap()));
    m.set_symbols(Some(Arc::new(symbols)));
    if let Some(dir) = std::env::var_os("SSD_DIVERGENCE_DIR") {
        m.set_divergence_dir(Some(dir.into()));
    }
    m
}

/// The canonical game on a fresh machine, every routine of `re/symbols.toml` in `Verify`.
fn game() -> Machine {
    game_in(HookMode::Verify)
}

/// Runs until frame `frames`, with `input(frame)` held during each frame.
fn run_until(m: &mut Machine, frames: u64, input: impl Fn(u64) -> Joypad) {
    while m.frame_count() < frames {
        m.run_frame(input(m.frame_count()), &mut NullAudio).unwrap();
    }
}

/// The input of the M1a plan: Start held for frames 600–659, X pressed for 6 frames every 120
/// frames from frame 900, Right held for frames 1500–1799. Start lands on the title screen and
/// begins a new game, whose prologue text is still running at frame 3 600: only the RNGs run.
fn scripted_input(frame: u64) -> Joypad {
    Joypad {
        start: (600..660).contains(&frame),
        x: frame >= 900 && (frame - 900) % 120 < 6,
        right: (1500..1800).contains(&frame),
        ..Joypad::default()
    }
}

fn stats(m: &Machine, name: &str) -> HookStats {
    let all = m.hook_stats();
    all.iter().find(|(n, _)| n == name).unwrap_or_else(|| panic!("no routine {name}")).1
}

fn print_table(m: &Machine) {
    eprintln!(
        "{:<26} {:>9} {:>9} {:>4} {:>5} {:>5} {:>9} {:>8} {:>9} {:>9} {:>8} {:>9} {:>9} {:>9} {:>7} {:>8} {:>9}",
        "routine",
        "calls",
        "verified",
        "div",
        "intr",
        "hdma",
        "sampled",
        "mismatch",
        "mean_mclk",
        "cost_bad",
        "no_cost",
        "rust",
        "fallback",
        "over_bound",
        "nested",
        "overflow",
        "reentered"
    );
    for (name, s) in m.hook_stats() {
        eprintln!(
            "{name:<26} {:>9} {:>9} {:>4} {:>5} {:>5} {:>9} {:>8} {:>9.2} {:>9} {:>8} {:>9} {:>9} {:>9} {:>7} {:>8} {:>9}",
            s.calls,
            s.verified_ok,
            s.divergences,
            s.interrupted,
            s.hdma_crossed,
            s.sampled_out,
            s.contract_mismatch,
            s.mean_cpu_cycles,
            s.cost_mismatch,
            s.cost_missing,
            s.rust_runs,
            s.timing_fallback,
            s.cost_over_bound,
            s.nested,
            s.nested_overflow,
            s.reentered
        );
    }
    for divergence in m.divergences() {
        eprint!("{divergence}");
    }
}

/// No divergence, no runaway original, every declared cost exact, none missing and none beyond
/// its bounds; every routine that was called has a verified call. Returns the names of the
/// routines never called.
fn check(m: &Machine) -> Vec<String> {
    print_table(m);
    let mut not_called = Vec::new();
    for (name, s) in m.hook_stats() {
        assert_eq!(s.divergences, 0, "{name} diverged");
        assert_eq!(s.no_return, 0, "{name} did not return");
        assert_eq!(s.cost_mismatch, 0, "{name}: a declared cost differs from the original's");
        assert_eq!(s.instructions_mismatch, 0, "{name}: declared instructions differ");
        assert_eq!(s.cost_missing, 0, "{name}: a call declared no cost");
        assert_eq!(s.cost_over_bound, 0, "{name}: a cost went beyond its bounds");
        assert_eq!(s.nested_overflow, 0, "{name}: nested too deep to be checked");
        if s.calls == 0 {
            eprintln!("note: {name} was not called");
            not_called.push(name);
        } else {
            assert!(s.verified_ok >= 1, "{name}: no call verified");
        }
    }
    not_called
}

#[test]
#[ignore = "needs SSD_ROM_PATH"]
fn scripted_run_matches_the_original() {
    let mut m = game();
    run_until(&mut m, 3_600, scripted_input);
    check(&m);
    // Only the RNGs run before the prologue: the run must still have checked one of them.
    assert!(stats(&m, "rng_next_a").verified_ok >= 1, "rng_next_a: no call verified");
}

#[test]
#[ignore = "needs SSD_ROM_PATH"]
fn every_routine_matches_the_original_through_a_battle() {
    let mut m = game();
    run_until(&mut m, BATTLE_FRAMES, battle_input);
    // `fill_word_run` is run by the menus and the wander script only: the battle script never
    // reaches it.
    assert_eq!(
        check(&m),
        vec!["fill_word_run".to_owned()],
        "every routine but fill_word_run is reached"
    );
    for (name, s) in m.hook_stats() {
        assert_eq!(s.contract_mismatch, 0, "{name}: called outside its entry contract");
    }
}

/// The traces never reached the true path of `entity_id_in_special_set`: at five of its calls,
/// the id it reads is set to each id of the set before the original and the Rust function run.
/// Its false path always costs [`SPECIAL_SET_FALSE_PATH`]; a call costing anything else shows
/// that the original took the true path (each member ends the comparisons at its own
/// place).
#[test]
#[ignore = "needs SSD_ROM_PATH"]
fn special_ids_match_the_original_on_the_true_path() {
    let mut m = game();
    // The play-through until the routine's first call (the battle intro, ~frame 4500).
    while stats(&m, SPECIAL_SET).calls == 0 {
        assert!(m.frame_count() < BATTLE_FRAMES, "{SPECIAL_SET} not reached");
        m.run_frame(battle_input(m.frame_count()), &mut NullAudio).unwrap();
    }
    let key = symbols().routines().iter().find(|r| r.name == SPECIAL_SET).unwrap().address;

    for id in SPECIAL_IDS {
        // Tick to the routine's next arrival (ticking runs every hooked call as original).
        loop {
            assert!(m.frame_count() < BATTLE_FRAMES, "no more calls of {SPECIAL_SET}");
            let input = battle_input(m.frame_count());
            if let TickEvent::Hook(full_pc) = m.tick(input, &mut NullAudio).unwrap()
                && hook_key(full_pc) == key
            {
                break;
            }
        }
        let r = m.cpu_registers();
        let id_address = (u32::from(r.dbr) << 16) + ENTITY_ID + u32::from(r.y);
        m.bus_write(id_address, id);
        assert_eq!(m.bus_read(id_address), id);
        // The tick took the hit; a state saved at a hooked fetch arrives there again.
        let state = m.save_state();
        m.load_state(&state).unwrap();

        let before = stats(&m, SPECIAL_SET);
        m.run_frame(battle_input(m.frame_count()), &mut NullAudio).unwrap();
        let after = stats(&m, SPECIAL_SET);
        assert_eq!(after.divergences, 0, "{SPECIAL_SET} diverged on id {id:02X}");
        assert_eq!(after.cost_mismatch, 0, "{SPECIAL_SET}: wrong cost on id {id:02X}");
        let checked = after.verified_ok - before.verified_ok;
        assert!(checked >= 1, "the call with id {id:02X} was not verified");
        // Total cycles of the calls checked in this frame: the poked one, then false paths.
        let total = |s: &HookStats| (s.mean_cpu_cycles * s.verified_ok as f64).round() as u64;
        let poked = total(&after) - total(&before) - SPECIAL_SET_FALSE_PATH * (checked - 1);
        eprintln!("id {id:02X}: {poked} master cycles ({checked} checked in the frame)");
        assert_ne!(poked, SPECIAL_SET_FALSE_PATH, "id {id:02X} did not take the true path");
    }
    print_table(&m);
}

/// Every routine in `Rust` mode stays on the pure original's timeline through the play-through:
/// after every frame both machines are at the same master cycle with the same registers and
/// WRAM, except dead stack (below S, down to the lowest S the original has reached since the two
/// machines last agreed on those bytes: a routine that pushes and pulls, like
/// `lookup_id_category`, leaves its pushed bytes there, which the Rust function does not write;
/// once the bytes agree again, as the stack is overwritten, the floor rises to S, as lockstep's
/// stack watch does). A dispatch that renders a frame returns after its frame ends; the pure
/// twin then ticks to the same master cycle, fed the same input.
#[test]
#[ignore = "needs SSD_ROM_PATH"]
fn rust_mode_stays_on_the_original_timeline_through_a_battle() {
    let mut original = pure_game();
    let mut hooked = game_in(HookMode::Rust);
    let mut stack_floor = original.cpu_registers().s;
    let mut settling = 0_u8;
    while hooked.frame_count() < BATTLE_FRAMES {
        let frame = hooked.frame_count();
        let input = battle_input(frame);
        hooked.run_frame(input, &mut NullAudio).unwrap();
        while original.frame_count() == frame
            || original.total_master_cycles() < hooked.total_master_cycles()
        {
            original.tick(input, &mut NullAudio).unwrap();
            stack_floor = stack_floor.min(original.cpu_registers().s);
        }
        // A Rust call whose charge spans frames has written all its results at once while the
        // original is still part-way through it, and the frame it ends lands inside an instruction
        // of the original: the two are compared again once their cycles agree (the same exception
        // as `xtask standalone-diff`, which does not compare registers and WRAM then either).
        let gap = original.total_master_cycles().abs_diff(hooked.total_master_cycles());
        if hooked.charge_pending() {
            assert!(gap <= 8, "frame {frame}: master cycles");
            settling = 2;
            continue;
        }
        if settling > 0 && gap != 0 {
            assert!(gap <= 8, "frame {frame}: master cycles");
            settling -= 1;
            continue;
        }
        settling = 0;
        assert_eq!(
            original.total_master_cycles(),
            hooked.total_master_cycles(),
            "frame {frame}: master cycles"
        );
        let (a, b) = (original.cpu_registers(), hooked.cpu_registers());
        assert_eq!(
            (a.a, a.x, a.y, a.s, a.d, a.pbr, a.pc, a.dbr, u8::from(a.p)),
            (b.a, b.x, b.y, b.s, b.d, b.pbr, b.pc, b.dbr, u8::from(b.p)),
            "frame {frame}: registers"
        );
        let dead_stack = usize::from(stack_floor) + 1..=usize::from(a.s);
        let differs = |i: &usize| original.wram()[*i] != hooked.wram()[*i];
        if let Some(i) =
            (0..original.wram().len()).filter(differs).find(|i| !dead_stack.contains(i))
        {
            panic!("frame {frame}: WRAM ${i:05X} differs");
        }
        // The dead bytes that still differ keep the floor below them; none does: it rises to S.
        stack_floor = dead_stack.clone().find(differs).map_or(a.s, |i| (i - 1) as u16);
    }
    print_table(&hooked);
    for (name, s) in hooked.hook_stats() {
        assert_eq!(s.cost_missing, 0, "{name}: a call declared no cost");
        assert_eq!(s.cost_over_bound, 0, "{name}: a cost went beyond its bounds");
        if s.calls > 0 {
            assert!(s.rust_runs > 0, "{name}: never ran in Rust");
        }
    }
}

/// The coverage estimate adds up: a `Verify` pass over the play-through learns the mean
/// instructions of every routine, a `Rust` pass counts its Rust runs at those means, and the
/// instructions it executed itself plus the Rust ones are what the pure original executes by the
/// same master cycle. The routines of a constant length count exactly; the few whose length
/// varies (the BCD conversion, the side record) are counted at their mean, so the total is
/// within a tiny fraction of the pure run's.
#[test]
#[ignore = "needs SSD_ROM_PATH"]
fn the_coverage_estimate_adds_up_to_the_pure_run() {
    let mut verifying = game_in(HookMode::Verify);
    while verifying.frame_count() < BATTLE_FRAMES {
        verifying.run_frame_with(&mut battle_input, &mut NullAudio).unwrap();
    }
    let means: Vec<(String, f64)> =
        verifying.hook_stats().into_iter().map(|(name, s)| (name, s.mean_instructions)).collect();
    // A routine the battle never calls has no mean and adds nothing to either side.
    let unreached = ["fill_word_run"];
    for (name, mean) in &means {
        assert!(
            *mean > 0.0 || unreached.contains(&name.as_str()),
            "{name}: no call was compared, so no mean"
        );
    }

    let mut hooked = game_in(HookMode::Rust);
    hooked.set_mean_instructions(&means);
    while hooked.frame_count() < BATTLE_FRAMES {
        hooked.run_frame_with(&mut battle_input, &mut NullAudio).unwrap();
    }
    let coverage = hooked.coverage();
    print_table(&hooked);
    eprintln!("{coverage:?}: {:.3}% of the instructions run as Rust", coverage.percent());
    assert!(coverage.rust_instructions_estimated > 0 && coverage.percent() < 100.0);

    let mut original = pure_game();
    while original.total_master_cycles() < hooked.total_master_cycles() {
        original.tick(battle_input(original.frame_count()), &mut NullAudio).unwrap();
    }
    assert_eq!(original.total_master_cycles(), hooked.total_master_cycles());
    let estimated = coverage.original_instructions + coverage.rust_instructions_estimated;
    let pure = original.opcode_fetches();
    eprintln!("original + Rust instructions: {estimated}, the pure run's: {pure}");
    // Within 0.001%: only the routines of varying length are estimated.
    assert!(estimated.abs_diff(pure) * 100_000 <= pure, "{estimated} instructions against {pure}");
}
