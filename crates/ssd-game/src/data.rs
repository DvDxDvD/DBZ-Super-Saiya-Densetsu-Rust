//! Addresses and sizes of the game's named data, generated at build time from `re/symbols.toml`
//! (`build.rs`): for every `[[variable]]` and `[[table]]` named `some_name`, `SOME_NAME` is the
//! 24-bit bus address of its first byte and `SOME_NAME_SIZE` its size in bytes. [`ALL`] lists
//! them all by name.
//!
//! Routines reach WRAM through the data bank or the direct page, so they use the low 16 bits
//! (`(SOME_NAME & 0xFFFF) as u16`); ROM tables are read with the full address.

include!(concat!(env!("OUT_DIR"), "/data.rs"));

#[cfg(test)]
mod tests {
    use super::*;
    use ssd_oracle::Symbols;

    fn symbols() -> Symbols {
        Symbols::load(concat!(env!("CARGO_MANIFEST_DIR"), "/../../re/symbols.toml")).unwrap()
    }

    #[test]
    fn every_constant_equals_its_symbols_entry() {
        let symbols = symbols();
        let expected: Vec<(&str, u32, u32)> = symbols
            .variables()
            .iter()
            .map(|v| (v.name.as_str(), v.address, v.size))
            .chain(symbols.tables().iter().map(|t| (t.name.as_str(), t.address, t.size)))
            .collect();
        assert_eq!(ALL.len(), expected.len(), "one constant pair per variable and table");
        for (name, address, size) in expected {
            let found = ALL.iter().find(|entry| entry.0 == name);
            assert_eq!(found, Some(&(name, address, size)), "{name}");
        }
    }
    #[test]
    fn constants_are_named_after_their_entries_in_upper_case() {
        let symbols = symbols();
        let variable = |name: &str| symbols.variables().iter().find(|v| v.name == name).unwrap();
        let table = |name: &str| symbols.tables().iter().find(|t| t.name == name).unwrap();
        let rng_seed_a = variable("rng_seed_a");
        assert_eq!((RNG_SEED_A, RNG_SEED_A_SIZE), (rng_seed_a.address, rng_seed_a.size));
        let entity_ids = variable("entity_ids");
        assert_eq!((ENTITY_IDS, ENTITY_IDS_SIZE), (entity_ids.address, entity_ids.size));
        let categories = table("id_category_table");
        assert_eq!(
            (ID_CATEGORY_TABLE, ID_CATEGORY_TABLE_SIZE),
            (categories.address, categories.size)
        );
    }
}
