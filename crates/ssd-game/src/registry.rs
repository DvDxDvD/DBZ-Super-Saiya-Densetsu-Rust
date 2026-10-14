//! Names to Rust functions, and the hook table built from `re/symbols.toml`.

use crate::routines::{
    batch_a, batch_b, batch_c, batch_d, batch_e, batch_f, batch_g, battle_side, bcd, category,
    entity, rng, stride,
};
use ssd_oracle::{BoundFn, HookAction, HookEntry, HookMode, HookTable, RoutineFn, Symbols};

/// A Rust routine under the name `re/symbols.toml` gives it, with the optional bound function
/// that gives its cost for one call from the entry state ([`BoundFn`]; without one, the `cycles`
/// maximum of the symbols file bounds every call).
#[derive(Debug, Clone, Copy)]
pub struct RegistryEntry {
    pub name: &'static str,
    pub func: RoutineFn,
    pub bound: Option<BoundFn>,
}

impl RegistryEntry {
    /// A routine whose cost is bounded by the static `cycles` of the symbols file.
    const fn new(name: &'static str, func: RoutineFn) -> Self {
        Self { name, func, bound: None }
    }

    /// A routine with a per-call cost bound, registered next to it.
    const fn bounded(name: &'static str, func: RoutineFn, bound: BoundFn) -> Self {
        Self { name, func, bound: Some(bound) }
    }
}

/// Every Rust routine under the name `re/symbols.toml` gives it.
static REGISTRY: [RegistryEntry; 59] = [
    RegistryEntry::new("advance_x_by_6", stride::advance_x_by_6),
    RegistryEntry::new("get_active_side_record", battle_side::get_active_side_record),
    RegistryEntry::new("lookup_id_category", category::lookup_id_category),
    RegistryEntry::new("rng_next_a", rng::rng_next_a),
    RegistryEntry::new("rng_next_b", rng::rng_next_b),
    RegistryEntry::new("entity_id_in_special_set", entity::entity_id_in_special_set),
    RegistryEntry::new("bin24_to_packed_bcd", bcd::bin24_to_packed_bcd),
    RegistryEntry::new("hide_sprites_from_index", batch_a::hide_sprites_from_index),
    RegistryEntry::new("load_palette_from_pointer", batch_a::load_palette_from_pointer),
    RegistryEntry::new("fill_tilemap_4000", batch_a::fill_tilemap_4000),
    RegistryEntry::new("lookup_bank2_pointer", batch_b::lookup_bank2_pointer),
    RegistryEntry::new("fill_buffer_4800", batch_b::fill_buffer_4800),
    RegistryEntry::new("lookup_byte_pair", batch_b::lookup_byte_pair),
    RegistryEntry::new("clear_slot_plus_8", batch_b::clear_slot_plus_8),
    RegistryEntry::new("lookup_0d40_scaled", batch_b::lookup_0d40_scaled),
    RegistryEntry::new("copy_source_block", batch_c::copy_source_block),
    RegistryEntry::new("clear_map_menu_block", batch_c::clear_map_menu_block),
    RegistryEntry::new("fill_word_block", batch_c::fill_word_block),
    RegistryEntry::new("lookup_scaled_entry", batch_c::lookup_scaled_entry),
    RegistryEntry::new("store_map_menu_entry", batch_c::store_map_menu_entry),
    RegistryEntry::new("follow_target_step", batch_c::follow_target_step),
    RegistryEntry::new("record_position_step", batch_c::record_position_step),
    RegistryEntry::bounded("draw_tile_box", batch_d::draw_tile_box, batch_d::draw_tile_box_bound),
    RegistryEntry::bounded(
        "find_positive_slot",
        batch_d::find_positive_slot,
        batch_d::find_positive_slot_bound,
    ),
    RegistryEntry::bounded(
        "scan_record_table",
        batch_d::scan_record_table,
        batch_d::scan_record_table_bound,
    ),
    RegistryEntry::bounded(
        "timer_mask_block",
        batch_d::timer_mask_block,
        batch_d::timer_mask_block_bound,
    ),
    RegistryEntry::bounded(
        "skip_negative_records",
        batch_d::skip_negative_records,
        batch_d::skip_negative_records_bound,
    ),
    RegistryEntry::new("fill_bank7f_pattern", batch_e::fill_bank7f_pattern),
    RegistryEntry::bounded(
        "unpack_bitplanes_to_7f",
        batch_e::unpack_bitplanes_to_7f,
        batch_e::unpack_bitplanes_bound,
    ),
    RegistryEntry::bounded(
        "decode_bit_tree_to_9000",
        batch_e::decode_bit_tree_to_9000,
        batch_e::decode_bit_tree_bound,
    ),
    RegistryEntry::new("set_bit12_block_9000", batch_e::set_bit12_block_9000),
    RegistryEntry::bounded(
        "entity_draw_pass_60_80",
        batch_e::entity_draw_pass_60_80,
        batch_e::entity_pass_bound,
    ),
    RegistryEntry::bounded(
        "blend_palette_slots",
        batch_e::blend_palette_slots,
        batch_e::blend_palette_bound,
    ),
    RegistryEntry::new("bin_to_bcd_from_tens", batch_e::bin_to_bcd_from_tens),
    RegistryEntry::new("store_code_from_low_bits", batch_e::store_code_from_low_bits),
    RegistryEntry::new("split_index_table_lookup", batch_e::split_index_table_lookup),
    RegistryEntry::new("copy_four_words_to_dp", batch_f::copy_four_words_to_dp),
    RegistryEntry::new("empty_return", batch_f::empty_return),
    RegistryEntry::bounded("follow_step_x", batch_f::follow_step_x, batch_f::follow_step_x_bound),
    RegistryEntry::new("fill_tilemap_quad", batch_f::fill_tilemap_quad),
    RegistryEntry::new("write_slot_record", batch_f::write_slot_record),
    RegistryEntry::bounded("fill_word_run", batch_f::fill_word_run, batch_f::fill_word_run_bound),
    RegistryEntry::new("advance_x_and_count", batch_f::advance_x_and_count),
    RegistryEntry::new("lookup_triple_index", batch_f::lookup_triple_index),
    RegistryEntry::bounded(
        "update_travel_and_follow",
        batch_f::update_travel_and_follow,
        batch_f::update_travel_and_follow_bound,
    ),
    RegistryEntry::new("copy_bank9_block", batch_f::copy_bank9_block),
    RegistryEntry::new("add_four_to_dp_word", batch_f::add_four_to_dp_word),
    RegistryEntry::bounded(
        "load_state_1648",
        batch_g::load_state_1648,
        batch_g::load_state_1648_bound,
    ),
    RegistryEntry::new("advance_y_by_20", batch_g::advance_y_by_20),
    RegistryEntry::new("lookup_byte_pair_2", batch_g::lookup_byte_pair_2),
    RegistryEntry::new("derive_state_071a", batch_g::derive_state_071a),
    RegistryEntry::bounded(
        "load_flag_01d6",
        batch_g::load_flag_01d6,
        batch_g::load_flag_01d6_bound,
    ),
    RegistryEntry::bounded(
        "load_counter_1570",
        batch_g::load_counter_1570,
        batch_g::load_counter_1570_bound,
    ),
    RegistryEntry::new("noop_9735", batch_g::noop_9735),
    RegistryEntry::new("clear_48_and_index_1220", batch_g::clear_48_and_index_1220),
    RegistryEntry::new("split_03_nibbles", batch_g::split_03_nibbles),
    RegistryEntry::bounded(
        "divide_24_by_16",
        batch_g::divide_24_by_16,
        batch_g::divide_24_by_16_bound,
    ),
    RegistryEntry::new("adjust_37_toward_ffd1", batch_g::adjust_37_toward_ffd1),
    RegistryEntry::bounded(
        "draw_map_entry_tiles",
        batch_g::draw_map_entry_tiles,
        batch_g::draw_map_entry_tiles_bound,
    ),
];

#[derive(Debug, thiserror::Error)]
pub enum GameError {
    /// A `[[routine]]` of the symbols file names no function of [`registry`].
    #[error("routine `{name}` in the symbols file has no Rust function in ssd-game's registry")]
    UnknownRoutine { name: String },
}

/// The Rust routines by name, the names of `re/symbols.toml`'s `[[routine]]` entries.
#[must_use]
pub fn registry() -> &'static [RegistryEntry] {
    &REGISTRY
}

/// The hook table of every `[[routine]]` of `symbols`, with its Rust function from
/// [`registry`]. `mode_override` replaces the mode of every routine whose mode in `symbols` is
/// not `original`: a routine kept on the original code there (for example one that diverges)
/// stays unhooked whatever the override.
///
/// # Errors
///
/// [`GameError::UnknownRoutine`] for the first routine of `symbols` whose name is not in the
/// registry.
pub fn hook_table(
    symbols: &Symbols,
    mode_override: Option<HookMode>,
) -> Result<HookTable, GameError> {
    #[allow(unused_mut)]
    let mut table = table_from(symbols, mode_override, registry())?;
    #[cfg(feature = "recomp")]
    if std::env::var_os("SSD_RECOMP_ONLY").is_some() {
        // Debugging: only the recompiled routines.
        table = HookTable::new();
    }
    #[cfg(feature = "recomp")]
    if let Some(mode) = mode_override.filter(|mode| *mode != HookMode::Original) {
        add_recompiled(&mut table, mode, 600);
    }
    Ok(table)
}

/// A table of the recompiled functions only (`ssd-recomp-gen`), in `mode`, each with a
/// `Verify` budget of `budget_frames` frames: for fuzzing them against the original.
#[cfg(feature = "recomp")]
#[must_use]
pub fn recompiled_table(mode: HookMode, budget_frames: u64) -> HookTable {
    let mut table = HookTable::new();
    add_recompiled(&mut table, mode, budget_frames);
    table
}

/// Adds every recompiled function (`ssd-recomp-gen`) the table has no routine for, in `mode`:
/// hand-written routines win on the same address.
#[cfg(feature = "recomp")]
fn add_recompiled(table: &mut HookTable, mode: HookMode, budget_frames: u64) {
    // Debugging knobs: `SSD_RECOMP_RANGE=LLLLLL-HHHHHH` keeps the recompiled routines in the
    // range, `SSD_RECOMP_SKIP=A,B,...` drops the listed entries (hexadecimal).
    let range = std::env::var("SSD_RECOMP_RANGE").ok().and_then(|text| {
        let (low, high) = text.split_once('-')?;
        Some((u32::from_str_radix(low, 16).ok()?, u32::from_str_radix(high, 16).ok()?))
    });
    let skip: Vec<u32> = std::env::var("SSD_RECOMP_SKIP")
        .map(|text| {
            text.split(',').filter_map(|p| u32::from_str_radix(p.trim(), 16).ok()).collect()
        })
        .unwrap_or_default();
    // The blocks the original can arrive at are routines only in `Rust` mode (a `Verify` call
    // would compare from the middle of a function).
    let mids: &[ssd_recomp::rt::GenEntry] =
        if mode == HookMode::Rust { ssd_recomp_gen::MID_TABLE } else { &[] };
    for entry in ssd_recomp_gen::TABLE.iter().chain(mids) {
        if table.get(entry.key).is_some()
            || range.is_some_and(|(low, high)| entry.key < low || entry.key > high)
            || skip.contains(&entry.key)
        {
            continue;
        }
        table.insert(HookEntry {
            name: entry.name.to_owned(),
            key: entry.key,
            mode,
            action: HookAction::Routine {
                func: entry.func,
                ret: if entry.long {
                    ssd_oracle::ReturnKind::Rtl
                } else {
                    ssd_oracle::ReturnKind::Rts
                },
            },
            entry_m8: None,
            entry_x8: None,
            flags_mask: entry.flags,
            verify_sample: 1,
            cycles: None,
            bound: Some(entry.bound),
            // Some of them wait for the sound CPU for many frames.
            budget_frames: Some(budget_frames),
        });
    }
}

/// [`hook_table`] over the routines of `registered`.
fn table_from(
    symbols: &Symbols,
    mode_override: Option<HookMode>,
    registered: &[RegistryEntry],
) -> Result<HookTable, GameError> {
    symbols
        .routines()
        .iter()
        .map(|routine| {
            let registered = registered
                .iter()
                .find(|entry| entry.name == routine.name)
                .ok_or_else(|| GameError::UnknownRoutine { name: routine.name.clone() })?;
            let mode = match (routine.mode, mode_override) {
                (HookMode::Original, _) | (_, None) => routine.mode,
                (_, Some(mode)) => mode,
            };
            Ok(HookEntry {
                name: routine.name.clone(),
                key: routine.address,
                mode,
                action: HookAction::Routine { func: registered.func, ret: routine.returns },
                entry_m8: routine.entry_m8,
                entry_x8: routine.entry_x8,
                flags_mask: routine.flags_mask,
                verify_sample: routine.verify_sample,
                cycles: routine.cycles,
                bound: registered.bound,
                budget_frames: routine.budget_frames,
            })
        })
        .collect()
}

/// The warning for the routines of `table` in `Rust` mode without a `cycles` cost, `None` when
/// there are none. Such a routine never runs in Rust: its original code runs (counted in
/// `cost_missing`).
#[must_use]
pub fn missing_cycles_warning(table: &HookTable) -> Option<String> {
    let names: Vec<&str> = table
        .entries()
        .filter(|entry| {
            entry.mode == HookMode::Rust
                && entry.cycles.is_none()
                && entry.bound.is_none()
                && matches!(entry.action, HookAction::Routine { .. })
        })
        .map(|entry| entry.name.as_str())
        .collect();
    (!names.is_empty()).then(|| {
        format!(
            "no `cycles` for {}: their original code runs instead of the Rust routine; measure \
             their costs with --hooks verify and add `cycles` (the maximum over their paths)",
            names.join(", ")
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssd_oracle::ReturnKind;
    use std::collections::HashSet;
    use std::path::Path;

    fn parse(source: &str) -> Symbols {
        Symbols::parse(source, Path::new("symbols.toml")).unwrap()
    }

    fn routine(name: &str, address: &str, mode: &str, extra: &str) -> String {
        format!(
            "[[routine]]\nname = \"{name}\"\naddress = {address}\nreturns = \"rtl\"\n\
             timing = \"pure\"\nmode = \"{mode}\"\n{extra}\n"
        )
    }

    fn function(name: &str) -> RoutineFn {
        registry().iter().find(|entry| entry.name == name).unwrap().func
    }

    #[test]
    fn registry_names_are_unique() {
        let names: HashSet<&str> = registry().iter().map(|entry| entry.name).collect();
        assert_eq!(names.len(), registry().len());
    }

    #[test]
    fn entries_carry_the_symbols_settings_and_the_function() {
        let symbols = parse(&routine(
            "rng_next_a",
            "0x018000",
            "verify",
            "entry_m8 = true\nflags = \"NVZC\"\nverify_sample = 64\ncycles = 280",
        ));
        let table = hook_table(&symbols, None).unwrap();
        assert_eq!(table.len(), 1);
        let entry = table.get(0x01_8000).unwrap();
        assert_eq!(entry.name, "rng_next_a");
        assert_eq!(entry.key, 0x01_8000);
        assert_eq!(entry.mode, HookMode::Verify);
        assert_eq!((entry.entry_m8, entry.entry_x8), (Some(true), None));
        assert_eq!(entry.flags_mask, 0xC3);
        assert_eq!(entry.verify_sample, 64);
        assert_eq!(entry.cycles, Some(280));
        match entry.action {
            HookAction::Routine { func, ret } => {
                assert!(std::ptr::fn_addr_eq(func, function("rng_next_a")));
                assert_eq!(ret, ReturnKind::Rtl);
            }
            HookAction::SyncPoint => panic!("a routine, not a sync point"),
        }
    }

    fn bound_of_six_hundred(_: &ssd_oracle::Ctx<'_>) -> u64 {
        600
    }

    #[test]
    fn a_bound_and_a_budget_reach_the_entry() {
        let symbols = parse(
            &(routine("rng_next_a", "0x018000", "rust", "budget_frames = 7")
                + &routine("rng_next_b", "0x018100", "rust", "cycles = 280")),
        );
        let entries = [
            RegistryEntry::bounded("rng_next_a", function("rng_next_a"), bound_of_six_hundred),
            RegistryEntry::new("rng_next_b", function("rng_next_b")),
        ];
        let table = table_from(&symbols, None, &entries).unwrap();
        let a = table.get(0x01_8000).unwrap();
        assert!(a.bound.is_some_and(|f| std::ptr::fn_addr_eq(f, bound_of_six_hundred as BoundFn)));
        assert_eq!((a.budget_frames, a.budget()), (Some(7), 7));
        let b = table.get(0x01_8100).unwrap();
        assert!(b.bound.is_none());
        assert_eq!((b.budget_frames, b.budget()), (None, 2));
        // A bound stands for `cycles`: only a routine with neither is warned about.
        assert_eq!(missing_cycles_warning(&table), None);
        let bare = parse(&routine("rng_next_b", "0x018100", "rust", ""));
        let table = table_from(&bare, None, &entries).unwrap();
        assert!(missing_cycles_warning(&table).unwrap().contains("rng_next_b"));
    }

    #[test]
    fn an_unknown_name_is_an_error_naming_it() {
        let symbols = parse(
            &(routine("rng_next_a", "0x018000", "verify", "")
                + &routine("no_such_routine", "0x018100", "rust", "")),
        );
        let Err(error) = hook_table(&symbols, None) else { panic!("an unknown name is an error") };
        assert!(matches!(&error, GameError::UnknownRoutine { name } if name == "no_such_routine"));
        assert!(error.to_string().contains("`no_such_routine`"), "{error}");
    }

    #[test]
    fn the_override_applies_to_every_routine_not_kept_on_the_original() {
        use HookMode::{Original, Rust, Verify};
        let symbols = parse(
            &(routine("rng_next_a", "0x018000", "verify", "")
                + &routine("rng_next_b", "0x018100", "rust", "")
                + &routine("advance_x_by_6", "0x018200", "original", "")),
        );
        let modes = |mode_override| {
            let table = hook_table(&symbols, mode_override).unwrap();
            table.entries().map(|entry| entry.mode).collect::<Vec<_>>()
        };
        assert_eq!(modes(None), [Verify, Rust, Original]);
        assert_eq!(modes(Some(Rust)), [Rust, Rust, Original]);
        assert_eq!(modes(Some(Verify)), [Verify, Verify, Original]);
        assert_eq!(modes(Some(Original)), [Original, Original, Original]);
    }

    #[test]
    fn rust_mode_warns_about_routines_without_cycles() {
        let source = routine("rng_next_a", "0x018000", "rust", "cycles = 264")
            + &routine("rng_next_b", "0x018100", "rust", "")
            + &routine("advance_x_by_6", "0x018200", "rust", "");
        let table = |source: &str, mode| hook_table(&parse(source), Some(mode)).unwrap();
        let warning = missing_cycles_warning(&table(&source, HookMode::Rust)).unwrap();
        assert!(warning.contains("rng_next_b") && warning.contains("advance_x_by_6"), "{warning}");
        assert!(!warning.contains("rng_next_a"), "{warning}");

        let all = routine("rng_next_a", "0x018000", "rust", "cycles = 264");
        assert_eq!(missing_cycles_warning(&table(&all, HookMode::Rust)), None);
        // Verify charges nothing, and a routine kept on the original is not hooked.
        assert_eq!(missing_cycles_warning(&table(&source, HookMode::Verify)), None);
        let original = routine("rng_next_b", "0x018100", "original", "");
        assert_eq!(missing_cycles_warning(&table(&original, HookMode::Rust)), None);
    }

    #[test]
    fn the_repository_symbols_give_every_routine_a_cycle_cost() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../re/symbols.toml");
        let table = hook_table(&Symbols::load(path).unwrap(), Some(HookMode::Rust)).unwrap();
        assert_eq!(missing_cycles_warning(&table), None);
    }

    #[test]
    fn the_repository_symbols_name_every_registered_routine() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../re/symbols.toml");
        let symbols = Symbols::load(path).unwrap();
        let table = hook_table(&symbols, None).unwrap();
        let mut names: Vec<&str> = table.entries().map(|entry| entry.name.as_str()).collect();
        let mut registered: Vec<&str> = registry().iter().map(|entry| entry.name).collect();
        names.sort_unstable();
        registered.sort_unstable();
        assert_eq!(names, registered);
    }
}
