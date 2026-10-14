//! The HD navigation: the cursors of the HD screens and what a key chooses. Pure: the entries and
//! the cursors are plain values. `ssd-game`'s menu driver turns a choice into the presses the
//! original menu needs, so the original keeps every consequence; this module only moves the HD
//! cursors and reports what was chosen.

use crate::cheats::Row;
use crate::screens::PAGE_SIZE;
use crate::view::{Screen, Sheet, Tab};

/// A navigation key: the host maps its keyboard and gamepad to these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    /// Chooses the entry under the cursor (the SNES A button).
    Confirm,
    /// Goes back: one level up, or closes the menu from the command window (the SNES B button).
    Cancel,
    /// The next tab of the bag (the SNES shoulder buttons).
    Tab,
}

/// An entry of the command window or of the Menu list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    Talk,
    Look,
    Fly,
    Land,
    Item,
    Menu,
    Cards,
    Status,
    Order,
    Text,
    Save,
    /// The cheats screen (M5).
    Cheats,
    /// Shown greyed out until the options exist.
    Options,
}

impl Entry {
    /// The label the window shows.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Talk => "Talk",
            Self::Look => "Look",
            Self::Fly => "Fly",
            Self::Land => "Land",
            Self::Item => "Item",
            Self::Menu => "Menu",
            Self::Cards => "Cards",
            Self::Status => "Status",
            Self::Order => "Order",
            Self::Text => "Text",
            Self::Save => "Save",
            Self::Cheats => "Cheats",
            Self::Options => "Options",
        }
    }

    /// The help line shown while the entry is under the cursor.
    #[must_use]
    pub const fn help(self) -> &'static str {
        match self {
            Self::Talk => "Talk to the person ahead.",
            Self::Look => "Search the ground nearby.",
            Self::Fly => "Take off and fly the map.",
            Self::Land => "Land where the party flies.",
            Self::Item => "Use the items you carry.",
            Self::Menu => "Cards, status and saves.",
            Self::Cards => "Look at the party's cards.",
            Self::Status => "See each fighter's stats.",
            Self::Order => "Change the party order.",
            Self::Text => "Set the text speed.",
            Self::Save => "Save the game in a slot.",
            Self::Cheats => "Set the cheats of this run.",
            Self::Options => "Coming soon.",
        }
    }

    /// Whether the entry does something. Options is shown greyed out until it does.
    #[must_use]
    pub const fn enabled(self) -> bool {
        !matches!(self, Self::Options)
    }
}

/// The rows of the command window, in the original's order: the field's five commands, a town's
/// four (no Fly), and the flying three (Land first).
#[must_use]
pub const fn command_rows(flying: bool, town: bool) -> &'static [Entry] {
    if flying {
        &FLYING_ROWS
    } else if town {
        &TOWN_ROWS
    } else {
        &FIELD_ROWS
    }
}

const FIELD_ROWS: [Entry; 5] = [Entry::Talk, Entry::Look, Entry::Fly, Entry::Item, Entry::Menu];
const TOWN_ROWS: [Entry; 4] = [Entry::Talk, Entry::Look, Entry::Item, Entry::Menu];
const FLYING_ROWS: [Entry; 3] = [Entry::Land, Entry::Item, Entry::Menu];

/// The rows of the Menu list: the original's five (Cards to Save, in its order), then Cheats and
/// Options, which is greyed out.
pub const MENU_ROWS: [Entry; 7] = [
    Entry::Cards,
    Entry::Status,
    Entry::Order,
    Entry::Text,
    Entry::Save,
    Entry::Cheats,
    Entry::Options,
];

/// The rows the original's Menu list has: the first ones of [`MENU_ROWS`].
pub const ORIGINAL_MENU_ROWS: usize = 5;

/// The rows of the item list's left or right column on a page.
const COLUMN: usize = PAGE_SIZE / 2;

/// One stack of the bag's list, as the navigation needs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemRow {
    /// The stack's id in the bag.
    pub id: u32,
    /// Usable here (on the map): Use is offered.
    pub usable: bool,
    /// Can be dropped: Drop is offered.
    pub droppable: bool,
    /// Asks for a party member: the use-on list comes first.
    pub member: bool,
    /// Units in the stack: the most the quantity can be.
    pub count: u16,
}

/// What a key chooses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Nothing: the cursor moved, or the key does nothing on this screen.
    None,
    /// The enabled entry under the cursor is chosen.
    Choose(Entry),
    /// Back: from the command window the menu closes; from a list the command window comes back.
    Back,
    /// Use `units` of the stack `stack` on the party row `member` (`None` when the object needs no
    /// member).
    Use { stack: u32, member: Option<u8>, units: u16 },
    /// Drop `units` of the stack `stack`.
    Drop { stack: u32, units: u16 },
    /// A line for the screen's foot: the choice does nothing, and says why.
    Notice(&'static str),
    /// On the cheats screen, the choice of `row` moves by `by` steps (-1 Left, 1 Right or A).
    Adjust { row: Row, by: i8 },
    /// On the cheats screen, Reset all: every cheat is off.
    ResetAll,
}

/// The cursors of the HD screens: the command window, the Menu list, the bag and the cheats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Nav {
    screen: Screen,
    command: usize,
    menu: usize,
    bag: Bag,
    cheats: usize,
}

/// The cursors and the sheet of the bag's screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Bag {
    tab: Tab,
    /// The cursor: an index of the stacks the tab lists.
    cursor: usize,
    sheet: Sheet,
    /// On Choose: 0 Use, 1 Drop; on Confirm: 0 Yes, 1 No.
    choice: u8,
    /// On Members: the party row.
    member: u8,
    /// On Quantity: the units.
    units: u16,
}

impl Nav {
    /// The screen the HD shows.
    #[must_use]
    pub const fn screen(&self) -> Screen {
        self.screen
    }

    /// The cursor as a view shows it: the row of the command window or of the Menu list, or the row
    /// on the page of the bag.
    #[must_use]
    pub fn cursor(&self) -> u8 {
        let row = match self.screen {
            Screen::Command => self.command,
            Screen::Menu => self.menu,
            Screen::Items => self.bag.cursor % PAGE_SIZE,
            Screen::Cheats => self.cheats,
        };
        u8::try_from(row).unwrap_or(0)
    }

    /// The page of the bag the cursor is on (0 on the other screens).
    #[must_use]
    pub fn page(&self) -> u8 {
        match self.screen {
            Screen::Items => u8::try_from(self.bag.cursor / PAGE_SIZE).unwrap_or(0),
            Screen::Command | Screen::Menu | Screen::Cheats => 0,
        }
    }

    /// The row of the cheats screen under the cursor.
    #[must_use]
    pub fn cheats_row(&self) -> Row {
        Row::ALL[self.cheats.min(Row::ALL.len() - 1)]
    }

    /// The tab of the bag.
    #[must_use]
    pub const fn tab(&self) -> Tab {
        self.bag.tab
    }

    /// The sheet over the bag's list.
    #[must_use]
    pub const fn sheet(&self) -> Sheet {
        self.bag.sheet
    }

    /// The bag's cursor: the index of the stack under it, among the stacks of the tab.
    #[must_use]
    pub const fn bag_cursor(&self) -> usize {
        self.bag.cursor
    }

    /// The choice of the open sheet (Use or Drop, Yes or No).
    #[must_use]
    pub const fn choice(&self) -> u8 {
        self.bag.choice
    }

    /// The party row the use-on list is on.
    #[must_use]
    pub const fn member(&self) -> u8 {
        self.bag.member
    }

    /// The units the quantity selector shows.
    #[must_use]
    pub const fn units(&self) -> u16 {
        self.bag.units
    }

    /// The HD shows `screen` from now on. A list starts at its first row; the command window keeps
    /// the row it had, as the original's does when it comes back from a list.
    pub fn enter(&mut self, screen: Screen) {
        self.screen = screen;
        match screen {
            Screen::Command => {}
            Screen::Menu => self.menu = 0,
            Screen::Items => {
                self.bag.cursor = 0;
                self.bag.sheet = Sheet::Browse;
            }
            Screen::Cheats => self.cheats = 0,
        }
    }

    /// The HD shows `screen` again with its cursor where it was: the Menu list after the cheats, which the
    /// original keeps open under them.
    pub fn resume(&mut self, screen: Screen) {
        self.screen = screen;
    }

    /// A new menu session: the command window at its first row, as the original opens it.
    pub fn open(&mut self) {
        self.enter(Screen::Command);
        self.command = 0;
    }

    /// Shows the bag's `tab` from its first stack.
    pub fn set_tab(&mut self, tab: Tab) {
        self.bag.tab = tab;
        self.bag.cursor = 0;
    }

    /// Back on the bag's list after an action: the sheet closes, the cursor stays where it was.
    pub fn close_sheet(&mut self) {
        self.bag.sheet = Sheet::Browse;
    }

    /// What `key` does on the HD screen: it moves the cursor or says what was chosen. `flying`
    /// and `town` pick the command rows; `items` is the number of entries of the item list.
    pub fn key(&mut self, key: Key, flying: bool, town: bool, items: usize) -> Action {
        match self.screen {
            Screen::Command => {
                let rows = command_rows(flying, town);
                self.command = self.command.min(rows.len() - 1);
                match key {
                    Key::Up => self.command = previous(self.command, rows.len()),
                    Key::Down => self.command = next(self.command, rows.len()),
                    Key::Confirm => return choose(rows[self.command]),
                    Key::Cancel => return Action::Back,
                    Key::Left | Key::Right | Key::Tab => {}
                }
            }
            Screen::Menu => match key {
                Key::Up => self.menu = previous(self.menu, MENU_ROWS.len()),
                Key::Down => self.menu = next(self.menu, MENU_ROWS.len()),
                Key::Confirm => return choose(MENU_ROWS[self.menu]),
                Key::Cancel => return Action::Back,
                Key::Left | Key::Right | Key::Tab => {}
            },
            // The bag is driven by `key_bag`, which has the stacks.
            Screen::Items => {
                let _ = items;
                if key == Key::Cancel && self.bag.sheet == Sheet::Browse {
                    return Action::Back;
                }
            }
            Screen::Cheats => match key {
                Key::Up => self.cheats = previous(self.cheats, Row::ALL.len()),
                Key::Down => self.cheats = next(self.cheats, Row::ALL.len()),
                Key::Left => return adjust(self.cheats_row(), -1),
                // A moves the choice on, and on Reset all it resets.
                Key::Confirm if self.cheats_row() == Row::ResetAll => return Action::ResetAll,
                Key::Right | Key::Confirm => return adjust(self.cheats_row(), 1),
                Key::Cancel => return Action::Back,
                Key::Tab => {}
            },
        }
        Action::None
    }

    /// What `key` does on the bag's screen, with its stacks of the tab as `rows` and the party's
    /// `party` members. The cursors move, a sheet opens or closes, or the choice is reported.
    pub fn key_bag(&mut self, key: Key, rows: &[ItemRow], party: usize) -> Action {
        if self.screen != Screen::Items {
            return Action::None;
        }
        let last = rows.len().saturating_sub(1);
        self.bag.cursor = self.bag.cursor.min(last);
        let Some(row) = rows.get(self.bag.cursor).copied() else {
            return if key == Key::Cancel { Action::Back } else { self.tab_key(key) };
        };
        let bag = &mut self.bag;
        match bag.sheet {
            Sheet::Browse => match key {
                Key::Up => bag.cursor = bag.cursor.saturating_sub(1),
                Key::Down => bag.cursor = (bag.cursor + 1).min(last),
                Key::Left => bag.cursor = bag.cursor.saturating_sub(COLUMN),
                Key::Right => bag.cursor = (bag.cursor + COLUMN).min(last),
                Key::Tab => return self.tab_key(key),
                Key::Cancel => return Action::Back,
                Key::Confirm => {
                    if !row.usable && !row.droppable {
                        return Action::Notice("This cannot be used or dropped here.");
                    }
                    bag.choice = u8::from(!row.usable);
                    bag.sheet = Sheet::Choose;
                }
            },
            Sheet::Choose => match key {
                Key::Up | Key::Down => {
                    if row.usable && row.droppable {
                        bag.choice ^= 1;
                    }
                }
                Key::Cancel => bag.sheet = Sheet::Browse,
                Key::Confirm => {
                    if bag.choice == 0 {
                        if row.member {
                            bag.member = 0;
                            bag.sheet = Sheet::Members;
                        } else {
                            bag.units = 1;
                            bag.sheet = Sheet::Quantity;
                        }
                    } else if row.droppable {
                        bag.units = 1;
                        bag.sheet = Sheet::Quantity;
                    } else {
                        return Action::Notice("This cannot be dropped.");
                    }
                }
                Key::Left | Key::Right | Key::Tab => {}
            },
            Sheet::Members => match key {
                Key::Up => bag.member = bag.member.saturating_sub(1),
                Key::Down => {
                    bag.member = (usize::from(bag.member) + 1).min(party.saturating_sub(1)) as u8;
                }
                Key::Cancel => bag.sheet = Sheet::Choose,
                Key::Confirm => {
                    bag.units = 1;
                    bag.sheet = Sheet::Quantity;
                }
                Key::Left | Key::Right | Key::Tab => {}
            },
            Sheet::Quantity => match key {
                Key::Left => bag.units = bag.units.saturating_sub(1).max(1),
                Key::Right => bag.units = (bag.units + 1).min(row.count.max(1)),
                Key::Cancel => {
                    bag.sheet =
                        if bag.choice == 0 && row.member { Sheet::Members } else { Sheet::Choose };
                }
                Key::Confirm => {
                    let units = bag.units;
                    if bag.choice == 0 {
                        let member = row.member.then_some(bag.member);
                        bag.sheet = Sheet::Browse;
                        return Action::Use { stack: row.id, member, units };
                    }
                    bag.choice = 1;
                    bag.sheet = Sheet::Confirm;
                }
                Key::Up | Key::Down | Key::Tab => {}
            },
            Sheet::Confirm => match key {
                Key::Up | Key::Down | Key::Left | Key::Right => bag.choice ^= 1,
                Key::Cancel => bag.sheet = Sheet::Quantity,
                Key::Confirm => {
                    if bag.choice == 0 {
                        let units = bag.units;
                        bag.sheet = Sheet::Browse;
                        return Action::Drop { stack: row.id, units };
                    }
                    bag.sheet = Sheet::Quantity;
                }
                Key::Tab => {}
            },
        }
        Action::None
    }

    /// The Tab key on the bag: the next tab, and its first stack under the cursor.
    fn tab_key(&mut self, key: Key) -> Action {
        if key == Key::Tab {
            let next = match self.bag.tab {
                Tab::All => Tab::Allies,
                Tab::Allies => Tab::Tools,
                Tab::Tools => Tab::All,
            };
            self.set_tab(next);
        }
        Action::None
    }
}

/// The row above `row` in a list of `rows`, wrapping to the last.
fn previous(row: usize, rows: usize) -> usize {
    if row == 0 { rows - 1 } else { row - 1 }
}

/// The row below `row` in a list of `rows`, wrapping to the first.
fn next(row: usize, rows: usize) -> usize {
    (row + 1) % rows
}

/// The choice of confirming `entry`: a greyed entry chooses nothing.
fn choose(entry: Entry) -> Action {
    if entry.enabled() { Action::Choose(entry) } else { Action::None }
}

/// The change of a cheats row by `by` steps: Reset all has no choices to change.
fn adjust(row: Row, by: i8) -> Action {
    if row.choices() == 0 { Action::None } else { Action::Adjust { row, by } }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack(id: u32, usable: bool, droppable: bool, member: bool, count: u16) -> ItemRow {
        ItemRow { id, usable, droppable, member, count }
    }

    #[test]
    fn the_command_rows_depend_on_flying_and_town() {
        let labels = |rows: &[Entry]| rows.iter().map(|e| e.label()).collect::<Vec<_>>();
        assert_eq!(labels(command_rows(false, false)), ["Talk", "Look", "Fly", "Item", "Menu"]);
        assert_eq!(labels(command_rows(false, true)), ["Talk", "Look", "Item", "Menu"]);
        assert_eq!(labels(command_rows(true, false)), ["Land", "Item", "Menu"]);
        assert_eq!(labels(&MENU_ROWS[..5]), ["Cards", "Status", "Order", "Text", "Save"]);
    }

    #[test]
    fn the_command_cursor_wraps_both_ways() {
        let mut nav = Nav::default();
        nav.open();
        assert_eq!(nav.key(Key::Up, false, false, 0), Action::None);
        assert_eq!(nav.cursor(), 4, "up from the first row goes to Menu");
        assert_eq!(nav.key(Key::Down, false, false, 0), Action::None);
        assert_eq!(nav.cursor(), 0, "down from Menu goes to Talk");
        for _ in 0..2 {
            nav.key(Key::Down, true, false, 0);
        }
        assert_eq!(nav.cursor(), 2, "three rows while flying");
    }

    #[test]
    fn confirm_chooses_the_row_and_cancel_goes_back() {
        let mut nav = Nav::default();
        nav.open();
        nav.key(Key::Down, false, false, 0);
        nav.key(Key::Down, false, false, 0);
        assert_eq!(nav.key(Key::Confirm, false, false, 0), Action::Choose(Entry::Fly));
        assert_eq!(nav.key(Key::Cancel, false, false, 0), Action::Back);
        assert_eq!(nav.screen(), Screen::Command, "a choice does not move the screen by itself");
    }

    #[test]
    fn the_menu_list_opens_the_cheats_and_its_greyed_options_choose_nothing() {
        let mut nav = Nav::default();
        nav.open();
        nav.enter(Screen::Menu);
        for _ in 0..5 {
            nav.key(Key::Down, false, false, 0);
        }
        assert_eq!(nav.cursor(), 5, "Cheats");
        assert_eq!(nav.key(Key::Confirm, false, false, 0), Action::Choose(Entry::Cheats));
        assert!(Entry::Cheats.enabled() && Entry::Save.enabled() && !Entry::Options.enabled());
        assert_eq!(nav.key(Key::Down, false, false, 0), Action::None);
        assert_eq!(nav.cursor(), 6, "Options");
        assert_eq!(nav.key(Key::Confirm, false, false, 0), Action::None);
        assert_eq!(nav.key(Key::Down, false, false, 0), Action::None);
        assert_eq!(nav.cursor(), 0, "the list wraps");
    }

    #[test]
    fn the_cheats_screen_moves_its_rows_and_changes_the_row_under_the_cursor() {
        let mut nav = Nav::default();
        nav.open();
        nav.enter(Screen::Menu);
        for _ in 0..5 {
            nav.key(Key::Down, false, false, 0);
        }
        nav.key(Key::Confirm, false, false, 0);
        nav.enter(Screen::Cheats);
        assert_eq!((nav.cursor(), nav.cheats_row()), (0, Row::Damage));
        assert_eq!(
            nav.key(Key::Right, false, false, 0),
            Action::Adjust { row: Row::Damage, by: 1 }
        );
        assert_eq!(
            nav.key(Key::Left, false, false, 0),
            Action::Adjust { row: Row::Damage, by: -1 }
        );
        assert_eq!(
            nav.key(Key::Confirm, false, false, 0),
            Action::Adjust { row: Row::Damage, by: 1 },
            "A moves a choice on"
        );
        assert_eq!(nav.key(Key::Up, false, false, 0), Action::None);
        assert_eq!(nav.cheats_row(), Row::ResetAll, "up from the first row wraps to Reset all");
        assert_eq!(nav.key(Key::Confirm, false, false, 0), Action::ResetAll);
        assert_eq!(nav.key(Key::Left, false, false, 0), Action::None, "Reset all has no choices");
        assert_eq!(nav.key(Key::Down, false, false, 0), Action::None);
        assert_eq!(nav.cheats_row(), Row::Damage, "down from Reset all wraps to the first row");
        assert_eq!(nav.key(Key::Cancel, false, false, 0), Action::Back);
        nav.enter(Screen::Cheats);
        assert_eq!(nav.cursor(), 0, "a new visit starts at the first row");
    }

    #[test]
    fn the_bag_moves_by_one_and_by_a_column_within_its_stacks() {
        let rows: Vec<ItemRow> = (0..3).map(|i| stack(i, true, true, false, 1)).collect();
        let mut nav = Nav::default();
        nav.enter(Screen::Items);
        assert_eq!(nav.key_bag(Key::Up, &rows, 2), Action::None);
        assert_eq!(nav.cursor(), 0, "the first stack stays first");
        nav.key_bag(Key::Right, &rows, 2);
        assert_eq!(nav.bag_cursor(), 2, "a column jump is clamped to the last stack");
        nav.key_bag(Key::Left, &rows, 2);
        assert_eq!(nav.bag_cursor(), 0);
        let many: Vec<ItemRow> = (0..10).map(|i| stack(i, true, true, false, 1)).collect();
        for _ in 0..9 {
            nav.key_bag(Key::Down, &many, 2);
        }
        assert_eq!((nav.page(), nav.cursor()), (1, 1), "stack 9 is on the second page");
        assert_eq!(nav.key_bag(Key::Cancel, &many, 2), Action::Back);
    }

    #[test]
    fn the_tab_key_cycles_and_puts_the_cursor_on_the_first_stack() {
        let rows: Vec<ItemRow> = (0..3).map(|i| stack(i, true, true, false, 1)).collect();
        let mut nav = Nav::default();
        nav.enter(Screen::Items);
        nav.key_bag(Key::Down, &rows, 2);
        nav.key_bag(Key::Tab, &rows, 2);
        assert_eq!((nav.tab(), nav.bag_cursor()), (Tab::Allies, 0));
        nav.key_bag(Key::Tab, &rows, 2);
        nav.key_bag(Key::Tab, &rows, 2);
        assert_eq!(nav.tab(), Tab::All, "the tabs cycle");
    }

    #[test]
    fn use_on_a_member_then_the_quantity_then_the_use() {
        let rows = [stack(7, true, true, true, 5)];
        let mut nav = Nav::default();
        nav.enter(Screen::Items);
        nav.key_bag(Key::Confirm, &rows, 2);
        assert_eq!(nav.sheet(), Sheet::Choose);
        nav.key_bag(Key::Confirm, &rows, 2);
        assert_eq!(nav.sheet(), Sheet::Members, "Use asks for the member");
        nav.key_bag(Key::Down, &rows, 2);
        nav.key_bag(Key::Down, &rows, 2);
        assert_eq!(nav.member(), 1, "the member list stops at the last member");
        nav.key_bag(Key::Up, &rows, 2);
        nav.key_bag(Key::Confirm, &rows, 2);
        assert_eq!(nav.sheet(), Sheet::Quantity);
        for _ in 0..9 {
            nav.key_bag(Key::Right, &rows, 2);
        }
        assert_eq!(nav.units(), 5, "the quantity stops at the stack's units");
        nav.key_bag(Key::Left, &rows, 2);
        assert_eq!(
            nav.key_bag(Key::Confirm, &rows, 2),
            Action::Use { stack: 7, member: Some(0), units: 4 }
        );
        assert_eq!(nav.sheet(), Sheet::Browse, "the list is back");
    }

    #[test]
    fn a_drop_asks_for_the_quantity_and_then_for_a_yes() {
        let rows = [stack(3, false, true, false, 2)];
        let mut nav = Nav::default();
        nav.enter(Screen::Items);
        nav.key_bag(Key::Confirm, &rows, 2);
        assert_eq!(nav.choice(), 1, "an object not usable here starts on Drop");
        nav.key_bag(Key::Confirm, &rows, 2);
        nav.key_bag(Key::Right, &rows, 2);
        nav.key_bag(Key::Confirm, &rows, 2);
        assert_eq!(nav.sheet(), Sheet::Confirm, "Drop 2? comes after the quantity");
        assert_eq!(nav.choice(), 1, "No is the choice to start on");
        assert_eq!(nav.key_bag(Key::Confirm, &rows, 2), Action::None, "No goes back");
        assert_eq!(nav.sheet(), Sheet::Quantity);
        nav.key_bag(Key::Confirm, &rows, 2);
        nav.key_bag(Key::Up, &rows, 2);
        assert_eq!(nav.key_bag(Key::Confirm, &rows, 2), Action::Drop { stack: 3, units: 2 });
    }

    #[test]
    fn a_pinned_stack_cannot_be_dropped_and_an_unusable_one_says_so() {
        let rows = [stack(9, false, false, false, 1)];
        let mut nav = Nav::default();
        nav.enter(Screen::Items);
        assert_eq!(
            nav.key_bag(Key::Confirm, &rows, 2),
            Action::Notice("This cannot be used or dropped here.")
        );
        let rows = [stack(9, true, false, false, 1)];
        nav.key_bag(Key::Confirm, &rows, 2);
        nav.key_bag(Key::Down, &rows, 2);
        assert_eq!(nav.choice(), 0, "Drop is not offered: the choice stays on Use");
        nav.key_bag(Key::Confirm, &rows, 2);
        assert_eq!(nav.sheet(), Sheet::Quantity);
    }

    #[test]
    fn every_help_line_fits_the_help_window() {
        // The help window holds about 28 characters of the 32 px text on one line.
        let all =
            MENU_ROWS.iter().chain(command_rows(true, false)).chain(command_rows(false, false));
        for entry in all {
            assert!(entry.help().chars().count() <= 28, "{}: {}", entry.label(), entry.help());
        }
    }

    #[test]
    fn entering_a_list_starts_it_at_its_first_row() {
        let mut nav = Nav::default();
        nav.open();
        nav.key(Key::Down, false, false, 0);
        nav.key(Key::Down, false, false, 0);
        nav.enter(Screen::Items);
        assert_eq!(nav.cursor(), 0);
        nav.enter(Screen::Command);
        assert_eq!(nav.cursor(), 2, "the command window keeps its row");
    }
}
