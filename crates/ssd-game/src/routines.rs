//! The Rust routines, one file per system or family. Each hooked routine is a
//! [`RoutineFn`](ssd_oracle::RoutineFn) working on the live machine through a
//! [`Ctx`](ssd_oracle::Ctx), over pure logic that the unit tests check. Semantics and contracts:
//! `docs/re/m1-candidates.md`.

pub(crate) mod batch_a;
pub(crate) mod batch_b;
pub(crate) mod batch_c;
pub(crate) mod batch_d;
pub(crate) mod batch_e;
pub(crate) mod batch_f;
pub(crate) mod batch_g;
pub(crate) mod battle_side;
pub(crate) mod bcd;
pub(crate) mod category;
pub(crate) mod entity;
pub(crate) mod rng;
pub(crate) mod stride;

#[cfg(test)]
pub(crate) mod tests {
    use ssd_oracle::Symbols;

    /// The `cycles` of routine `name` in `re/symbols.toml`: its maximum cost.
    pub(crate) fn max_cost(name: &str) -> u64 {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../re/symbols.toml");
        let symbols = Symbols::load(path).unwrap();
        let routine = symbols.routines().iter().find(|r| r.name == name).unwrap();
        routine.cycles.unwrap_or_else(|| panic!("{name}: no cycles"))
    }
}
