//! The two modes of the game and their rulesets (spec §5.8). `Classic` keeps the original's rules
//! and visuals: it is the reference every replay is verified against. `Enhanced` is the game the
//! player plays, with its named differences; the HD menus are the first of them (M4). Only the
//! stub is here: the options of the enhancements arrive with the features that have them.

use ssd_menus::bag::{Rules, SLOTS, STACK};

/// Which rules and visuals the game runs with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// The original's rules and visuals, the reference of the replays.
    #[default]
    Classic,
    /// The enhanced game: the HD menus so far.
    Enhanced,
}

/// The rules of a run: its mode, and the options of its enhancements (spec §5.8): the bag's slots and
/// units per stack (`Rules`, 60 and 99 by default), and whether the cheats are installed (`crate::cheats`:
/// the Enhanced mode has them, Classic never does).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Ruleset {
    mode: Mode,
    bag: Rules,
    cheats: bool,
}

impl Ruleset {
    /// The original's rules.
    #[must_use]
    pub const fn classic() -> Self {
        Self { mode: Mode::Classic, bag: Rules { slots: SLOTS, stack: STACK }, cheats: false }
    }

    /// The enhanced rules, with their default options.
    #[must_use]
    pub const fn enhanced() -> Self {
        Self::enhanced_with(Rules { slots: SLOTS, stack: STACK })
    }

    /// The enhanced rules with the bag of `bag` (the options of `Options`, later). The cheats are installed:
    /// their wrappers run on the cheat block, which is neutral until the Cheats screen writes it.
    #[must_use]
    pub const fn enhanced_with(bag: Rules) -> Self {
        Self { mode: Mode::Enhanced, bag, cheats: true }
    }

    /// Whether the cheats are installed (`crate::cheats::install`).
    #[must_use]
    pub const fn cheats(self) -> bool {
        self.cheats
    }

    /// The bag's rules: its slots and the units of a stack.
    #[must_use]
    pub const fn bag(self) -> Rules {
        self.bag
    }

    /// The mode of the run.
    #[must_use]
    pub const fn mode(self) -> Mode {
        self.mode
    }

    /// Whether the enhancements are on.
    #[must_use]
    pub const fn is_enhanced(self) -> bool {
        matches!(self.mode, Mode::Enhanced)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classic_is_the_default_and_enhanced_is_the_other_mode() {
        assert_eq!(Ruleset::default().mode(), Mode::Classic);
        assert!(!Ruleset::classic().is_enhanced());
        assert!(Ruleset::enhanced().is_enhanced());
        assert_eq!(Ruleset::enhanced().mode(), Mode::Enhanced);
    }

    #[test]
    fn only_the_enhanced_rules_install_the_cheats() {
        assert!(!Ruleset::classic().cheats());
        assert!(Ruleset::enhanced().cheats());
        assert!(Ruleset::enhanced_with(Ruleset::enhanced().bag()).cheats());
    }
}
