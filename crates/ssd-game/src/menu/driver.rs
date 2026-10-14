//! The HD menu driver: the curtain, the HD cursors, the bag, and the plans that carry out what the
//! player chose. It is what the windowed app and the headless tests drive, one frame at a time:
//! [`Driver::key`] gives the HD a key, [`Driver::frame`] says which joypad the next frame runs with and
//! which host events come before it, and [`Driver::scene`] says what the HD draws.
//!
//! The curtain is up (`Up`) while the HD draws the menu, and then the keys belong to the HD and the
//! game gets no buttons. The curtain lifts (`Lifted`) when the original draws the next thing itself
//! (a message after Talk or Look, Fly or Land, Cards, Status, Order, Text, Save), and the game gets the
//! keys again; it comes down (`Down`) once the field is free. The original's menu sits on its first
//! screen while the HD cursor moves: a choice is a plan of presses from the original's own cursor.
//! A use or a drop of the bag stages its stack in the original's list and then presses the original's
//! own keys, one unit at a time, so the original takes the unit and the bag follows the list.

use super::keeper::{BIND_TAG, Keeper, good_checksum};
use super::presses::{Button, Next, Outcome, Player, Step, Until};
use super::{MenuProbe, PORUNGA, SAVE_DONE_STEP, SAVE_LIST_STEP, SAVE_SCREEN};
use crate::cheats::{CHEATS_TAG, block_events, frame_events, hand_events};
use ssd_menus::bag::{Bag, Rules, SavedBag, Sidecar};
use ssd_menus::cheats::{CHARACTERS, CheatSidecar, Cheats, SavedCheats};
use ssd_menus::nav::{Action, Entry, ItemRow, Key, Nav, ORIGINAL_MENU_ROWS, command_rows};
use ssd_menus::view::{
    BagView, CheatsView, Labels, MenuView, ObjectLabel, Screen, Selected, StackView,
};
use ssd_menus::{DrawList, screens};
use ssd_oracle::{HostEvent, Joypad};

/// Where the curtain is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Curtain {
    /// The field: no menu is open.
    Down,
    /// The HD draws the menu, and the keys go to it.
    Up,
    /// The original draws the menu or a message, and the keys go to the game.
    Lifted,
}

/// Where the curtain goes when a plan has reached its state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Arrival {
    /// The HD shows this screen.
    Show(Screen),
    /// The original draws what comes next.
    Lifted,
    /// The menu has closed.
    Closed,
}

/// A plan running, and where it leaves the curtain.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Running {
    player: Player,
    arrival: Arrival,
}

/// How the bag is bound: by the driver itself, from the save and the sidecar, or by a replay's
/// recorded bind event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Binding {
    /// The driver binds the bag when the game has a save or a new game is in the field.
    Auto,
    /// The bag is bound only by the replay's recorded bind (see [`Driver::replay_event`]).
    Recorded,
}

/// What a frame of the menu needs from the host: the joypad of the frame, the host events to make
/// before it, the bag of a save that changed (to be kept in the sidecar file), and the cheats of the save
/// a Save wrote (kept in `saves/ssd.cheats`).
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub pad: Joypad,
    pub events: Vec<HostEvent>,
    pub saved: Option<SavedBag>,
    pub saved_cheats: Option<SavedCheats>,
}

/// Context the menu needs of the game at each frame: its labels and its rules.
/// Work RAM values that the probe reads are read fresh each time they are needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Context {
    /// The party flies: the command window has Land.
    flying: bool,
    /// The party is in a town: the command window has no Fly.
    town: bool,
    /// The party size.
    party: usize,
}

/// The state of the HD menu: everything but the labels, which the probe reads alongside it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct State {
    curtain: Curtain,
    nav: Nav,
    running: Option<Running>,
    last: Option<Outcome>,
    keeper: Keeper,
    sidecar: Sidecar,
    binding: Binding,
    /// The line the bag's screen shows at its foot, until the next key.
    notice: Option<&'static str>,
    /// The cheats the run has: the choices of the Cheats screen, which the cheat block carries.
    cheats: Cheats,
    /// The cheats of the save slots (`saves/ssd.cheats`).
    cheat_sidecar: CheatSidecar,
    /// Whether the cheat wrappers are installed in this run: the cheat block's frame events are made then.
    cheats_installed: bool,
    /// The save the cheats are bound to (the bag's save), so that a new save binds them again.
    cheat_save: Option<(u8, u16)>,
    /// The host events of the cheats' changes, made before the next frame.
    cheat_events: Vec<HostEvent>,
    /// The slot the Save list's cursor is on, kept from the list's step until the Save completes.
    save_slot: Option<u8>,
    /// Whether the Save was at its completion step at the last frame (a completion is seen once).
    save_done: bool,
}

/// The HD menu of a run: the curtain, the HD cursors, the bag and the plan that is running, if one is.
pub struct Driver {
    labels: Labels,
    /// The rules the bag is kept with (kept for [`Driver::reset`]).
    rules: Rules,
    state: State,
}

impl Driver {
    /// A driver for the menus with the names of `labels`, with the field free and the bag unbound.
    #[must_use]
    pub fn new(labels: Labels) -> Self {
        Self::with_bag(labels, Rules::default(), Sidecar::empty())
    }

    /// A driver whose bag has `rules` and binds with the sidecar `sidecar` (the bags of the saves).
    #[must_use]
    pub fn with_bag(labels: Labels, rules: Rules, sidecar: Sidecar) -> Self {
        Self {
            labels,
            rules,
            state: State {
                curtain: Curtain::Down,
                nav: Nav::default(),
                running: None,
                last: None,
                keeper: Keeper::new(rules),
                sidecar,
                binding: Binding::Auto,
                notice: None,
                cheats: Cheats::neutral(),
                cheat_sidecar: CheatSidecar::empty(),
                cheats_installed: false,
                cheat_save: None,
                cheat_events: Vec::new(),
                save_slot: None,
                save_done: false,
            },
        }
    }

    /// The run installs the cheats (its wrappers are in the machine): the cheat block's frame events are
    /// made, and the cheats of the save slots are the ones of `sidecar` (`saves/ssd.cheats`).
    #[must_use]
    pub fn with_cheats(mut self, sidecar: CheatSidecar) -> Self {
        self.state.cheats_installed = true;
        self.state.cheat_sidecar = sidecar;
        self
    }

    /// The machine's state was replaced under the menu (a state loaded, not a frame run): the curtain
    /// comes down, no plan runs, and the bag binds again to what the game now holds (the sidecar's
    /// bag of the save it matches, or a rebuild from the list). The sidecars, the rules, the labels
    /// and the binding mode are kept; the cheats bind again with the bag.
    pub fn reset(&mut self) {
        self.state = State {
            curtain: Curtain::Down,
            nav: Nav::default(),
            running: None,
            last: None,
            keeper: Keeper::new(self.rules),
            sidecar: self.state.sidecar.clone(),
            binding: self.state.binding,
            notice: None,
            cheats: Cheats::neutral(),
            cheat_sidecar: self.state.cheat_sidecar.clone(),
            cheats_installed: self.state.cheats_installed,
            cheat_save: None,
            cheat_events: Vec::new(),
            save_slot: None,
            save_done: false,
        };
    }

    /// The cheats of the run: the choices the Cheats screen shows.
    #[must_use]
    pub fn cheats(&self) -> &Cheats {
        &self.state.cheats
    }

    /// The cheats of the save slots, as the sidecar file keeps them.
    #[must_use]
    pub fn cheat_sidecar(&self) -> &CheatSidecar {
        &self.state.cheat_sidecar
    }

    /// Which save slots carry the CHEATS badge: a good save whose cheats were saved with one on, by its
    /// checksum word (a slot changed by another way has no entry).
    #[must_use]
    pub fn marked_slots(&self, sram: &[u8]) -> [bool; 3] {
        let mut marks = [false; 3];
        for (slot, mark) in (0..3_u8).zip(marks.iter_mut()) {
            if let Some(checksum) = good_checksum(sram, slot) {
                *mark = self.state.cheat_sidecar.marked(slot, checksum);
            }
        }
        marks
    }

    /// How the bag is bound (see [`Binding`]).
    pub fn set_binding(&mut self, binding: Binding) {
        self.state.binding = binding;
    }

    /// The sidecar: the bags of the saves this run knows.
    #[must_use]
    pub fn sidecar(&self) -> &Sidecar {
        &self.state.sidecar
    }

    /// The bag, once it is bound.
    #[must_use]
    pub fn bag(&self) -> Option<&Bag> {
        self.state.keeper.bag()
    }

    /// Where the curtain is.
    #[must_use]
    pub fn curtain(&self) -> Curtain {
        self.state.curtain
    }

    /// The HD cursors.
    #[must_use]
    pub fn nav(&self) -> Nav {
        self.state.nav
    }

    /// Whether a plan is running: the menu is busy with a choice, and the keys wait.
    #[must_use]
    pub fn busy(&self) -> bool {
        self.state.running.is_some()
    }

    /// What the last plan that ended reached, if one has ended.
    #[must_use]
    pub fn last_outcome(&self) -> Option<Outcome> {
        self.state.last
    }

    /// The line the bag's screen shows at its foot, if there is one.
    #[must_use]
    pub fn notice(&self) -> Option<&'static str> {
        self.state.notice
    }

    /// The player opens the menu on the field: a press of A, as a plan. Nothing happens unless the
    /// field is free and no plan runs.
    pub fn open(&mut self) {
        self.state.open();
    }

    /// A key of the HD on the menu of `wram`: the cursor moves, or a choice starts its plan. Ignored
    /// while the curtain is not up, or while a plan runs.
    pub fn key(&mut self, key: Key, wram: &[u8]) {
        let probe = MenuProbe::new(wram, &self.labels);
        self.state.key(key, &probe, &self.labels, wram);
    }

    /// The frame of the menu on the game of `wram` (its work RAM after the frame before it, and the
    /// battery save `sram`): the bag is bound and kept in step with the list, a plan's next press is
    /// chosen, and the live joypad is given to the game when the curtain is not up.
    pub fn frame(&mut self, wram: &[u8], sram: Option<&[u8]>, live: Joypad) -> Frame {
        let probe = MenuProbe::new(wram, &self.labels);
        self.state.frame(&probe, &self.labels, wram, sram, live)
    }

    /// A replay's recorded application event: the bag's bind, applied as it was recorded, or the run's cheats
    /// ([`CHEATS_TAG`]) for the bookkeeping of the saves. The sidecar files are not written by a replay (see
    /// [`Frame`]), so only the run's state changes here. Other tags are not the menus'.
    ///
    /// # Errors
    ///
    /// The event's payload is not a bag, or not a cheats file.
    pub fn replay_event(
        &mut self,
        tag: u8,
        payload: &[u8],
        wram: &[u8],
    ) -> Result<Vec<HostEvent>, String> {
        match tag {
            BIND_TAG => self.state.keeper.bind_recorded(payload, wram, &self.labels),
            CHEATS_TAG => {
                self.state.cheats = Cheats::from_bytes(payload)?;
                Ok(Vec::new())
            }
            _ => Ok(Vec::new()),
        }
    }

    /// The bag's Item screen shows `tab` from its first stack (the shoulder buttons do the same).
    pub fn set_tab(&mut self, tab: ssd_menus::view::Tab) {
        self.state.nav.set_tab(tab);
    }

    /// What the HD draws now: the screen the HD shows, with its cursors, while the curtain is up.
    #[must_use]
    pub fn scene(&self, wram: &[u8]) -> Option<DrawList> {
        if self.state.curtain != Curtain::Up {
            return None;
        }
        let probe = MenuProbe::new(wram, &self.labels);
        self.state.scene(&probe, &self.labels)
    }
}

impl State {
    /// The player's own press opened the menu: see [`Driver::open`].
    fn open(&mut self) {
        if self.curtain != Curtain::Down || self.running.is_some() {
            return;
        }
        // The command window is up once the menu is drawn: its screen (13 in a town) is set a
        // couple of frames after the press.
        let steps = vec![
            Step::Press { button: Button::A, until: Until::Open },
            Step::Wait { until: Until::Ready },
            Step::Pause { frames: SETTLE_WINDOW },
        ];
        self.start(steps, Arrival::Show(Screen::Command));
    }

    /// A key of the HD: see [`Driver::key`].
    fn key(&mut self, key: Key, probe: &MenuProbe<'_>, labels: &Labels, wram: &[u8]) {
        if self.curtain != Curtain::Up || self.running.is_some() {
            return;
        }
        self.notice = None;
        let screen = self.nav.screen();
        let Context { flying, town, party } = context(probe);
        let action = match screen {
            Screen::Items => {
                let rows = self.rows(labels);
                self.nav.key_bag(key, &rows, party)
            }
            _ => self.nav.key(key, flying, town, probe.item_count()),
        };
        match action {
            Action::None => {}
            Action::Notice(text) => self.notice = Some(text),
            // The Cheats screen is the HD's own: nothing is sent to the original, which keeps its Menu list.
            Action::Choose(Entry::Cheats) => self.nav.enter(Screen::Cheats),
            Action::Choose(entry) => {
                if let Some((steps, arrival)) = choice(screen, entry, flying, town) {
                    self.start(steps, arrival);
                }
            }
            Action::Adjust { row, by } => {
                row.step(&mut self.cheats, by);
                self.cheats_changed(wram);
            }
            Action::ResetAll => {
                self.cheats = Cheats::neutral();
                self.cheats_changed(wram);
            }
            Action::Use { stack, member, units } => {
                let object = self.keeper.bag().and_then(|bag| bag.stack(stack)).map(|s| s.object);
                let card = object.is_some_and(|object| {
                    labels.objects.get(usize::from(object)).is_some_and(|label| label.card)
                });
                let kind = match object {
                    _ if card => UseKind::Card,
                    Some(DRAGON_RADAR) => UseKind::Radar,
                    _ => UseKind::Plain,
                };
                let steps = use_steps(stack, member, units, kind, party, flying, town);
                self.start(steps, if card { Arrival::Lifted } else { Arrival::Closed });
            }
            Action::Drop { stack, units } => {
                let steps = drop_steps(stack, units, flying, town);
                self.start(steps, Arrival::Show(Screen::Items));
            }
            Action::Back => match screen {
                Screen::Command => self.start(
                    vec![Step::Press { button: Button::B, until: Until::Closed }],
                    Arrival::Closed,
                ),
                Screen::Menu | Screen::Items => self.start(
                    vec![Step::Press { button: Button::B, until: Until::Command }],
                    Arrival::Show(Screen::Command),
                ),
                // Back to the Menu list, which the original never left: its cursor stays on Cheats.
                Screen::Cheats => self.nav.resume(Screen::Menu),
            },
        }
    }

    /// The cheats changed on the screen: the cheat block is written whole (the choices and the cheats'
    /// runtime bytes, which are zero on the map), and the cards of the hand get the Z corners the choice
    /// asks for. The writes are made before the next frame, with the application event that carries the
    /// choices to a replay.
    fn cheats_changed(&mut self, wram: &[u8]) {
        self.cheat_events.extend(block_events(&self.cheats));
        self.cheat_events.extend(hand_events(&self.cheats, wram));
        self.cheat_events.push(self.cheats_app_event());
    }

    /// The application event that carries the run's cheats to a replay (see [`CHEATS_TAG`]).
    fn cheats_app_event(&self) -> HostEvent {
        HostEvent::App { tag: CHEATS_TAG, payload: self.cheats.to_bytes() }
    }

    /// The cheats of the save the bag is bound to: the settings the sidecar keeps for that save, or all off
    /// for a save without an entry (a new game, or a save changed by another way). The block is written with
    /// them, with the Z corners of the hand.
    fn bind_cheats(&mut self, wram: &[u8]) -> Vec<HostEvent> {
        self.cheat_save = self.keeper.bound_save();
        self.cheats = self
            .cheat_save
            .and_then(|(slot, checksum)| self.cheat_sidecar.entry_for(slot, checksum))
            .map_or_else(Cheats::neutral, |saved| saved.cheats);
        let mut events = block_events(&self.cheats);
        events.extend(hand_events(&self.cheats, wram));
        events.push(self.cheats_app_event());
        events
    }

    /// The bind, the list's keeping and the plan's next frame: see [`Driver::frame`].
    fn frame(
        &mut self,
        probe: &MenuProbe<'_>,
        labels: &Labels,
        wram: &[u8],
        sram: Option<&[u8]>,
        live: Joypad,
    ) -> Frame {
        let mut events = Vec::new();
        let bound_now = !self.keeper.bound() && self.binding == Binding::Auto;
        if bound_now {
            let fresh = probe.field_free() && probe.field_mode();
            events = self.keeper.bind(wram, labels, sram, &self.sidecar, fresh);
        }
        // A bind writes the list as the bag projects it: the list is kept from the next frame on.
        if self.keeper.bound() && !bound_now {
            events.extend(self.keeper.sync(wram, labels, sram, &self.sidecar));
        }
        // The cheats follow the save the bag is bound to: a bind (a new game has none on), or a save copied
        // into the game, gives the cheats of that save.
        if self.keeper.bound() && (bound_now || self.keeper.bound_save() != self.cheat_save) {
            events.extend(self.bind_cheats(wram));
        }
        events.append(&mut self.cheat_events);
        if self.cheats_installed {
            events.extend(frame_events(wram));
        }
        let pad = self.pad(probe, labels, wram, live, &mut events);
        let completed = self.save_completed(probe, sram);
        let bag = self.keeper.saved(sram, wram);
        // The cheats of a Save are written for the slot it wrote, also when the slot's checksum word is the same.
        let written =
            completed.or_else(|| bag.as_ref().map(|&(slot, checksum, _)| (slot, checksum)));
        let saved_cheats = self.store_cheats(written);
        let saved = bag.map(|(slot, checksum, bag)| {
            self.sidecar.store(slot, checksum, &bag);
            SavedBag { slot, checksum, bag }
        });
        Frame { pad, events, saved, saved_cheats }
    }

    /// The Save of the original, followed on work RAM at every frame. While the Save's list shows, its cursor (the
    /// slot, 0 to 2) is kept; the Save completes when the screen reaches its completion step, and then the slot
    /// the list chose is the one written. Returns that slot and its checksum word, once per Save, when `sram`
    /// holds the slot as a good save.
    fn save_completed(&mut self, probe: &MenuProbe<'_>, sram: Option<&[u8]>) -> Option<(u8, u16)> {
        let on_save = probe.screen() == Some(SAVE_SCREEN);
        if on_save && probe.step() == Some(SAVE_LIST_STEP) {
            self.save_slot =
                probe.cursor().filter(|&slot| usize::from(slot) < ssd_save::SLOT_COUNT);
        }
        let done = on_save && probe.step() == Some(SAVE_DONE_STEP);
        let completed = done && !self.save_done;
        self.save_done = done;
        if !completed {
            return None;
        }
        let slot = self.save_slot.take()?;
        Some((slot, good_checksum(sram?, slot)?))
    }

    /// Stores the run's cheats for the save written at `written` (its slot and checksum word), marked when a cheat
    /// is on, and returns that entry. The run keeps the save its bag is bound to: a Save to another slot leaves the
    /// binding (and so the run's cheats) where it was, and only that slot's entry is written.
    fn store_cheats(&mut self, written: Option<(u8, u16)>) -> Option<SavedCheats> {
        let (slot, checksum) = written?;
        self.cheat_sidecar.store(slot, checksum, self.cheats);
        if self.keeper.bound_save() == Some((slot, checksum)) {
            self.cheat_save = Some((slot, checksum));
        }
        self.cheat_sidecar.entry_for(slot, checksum).copied()
    }

    /// The joypad of the next frame of the plan (the live joypad when no plan runs and the curtain
    /// is not up), collecting the host events the plan's stages make.
    fn pad(
        &mut self,
        probe: &MenuProbe<'_>,
        labels: &Labels,
        wram: &[u8],
        live: Joypad,
        events: &mut Vec<HostEvent>,
    ) -> Joypad {
        if self.running.is_none() {
            self.observe(probe);
        }
        if let Some(running) = self.running.as_mut() {
            let arrival = running.arrival;
            loop {
                match running.player.next(probe) {
                    Next::Frame(pad) => return pad,
                    Next::Stage(id) => {
                        events.extend(self.keeper.stage(wram, labels, id));
                    }
                    Next::Done(outcome) => {
                        self.running = None;
                        self.last = Some(outcome);
                        self.arrive(arrival, outcome);
                        break;
                    }
                }
            }
        }
        match self.curtain {
            Curtain::Up => Joypad::default(),
            Curtain::Down | Curtain::Lifted => live,
        }
    }

    /// What the HD draws: see [`Driver::scene`].
    fn scene(&self, probe: &MenuProbe<'_>, labels: &Labels) -> Option<DrawList> {
        if self.nav.screen() == Screen::Cheats {
            return Some(screens::cheats_screen(&self.cheats_view(labels)));
        }
        let Context { flying, town, .. } = context(probe);
        let mut view: MenuView = probe.view_as(
            self.nav.screen(),
            town,
            flying && !town,
            self.nav.cursor(),
            self.nav.page(),
        )?;
        if view.screen == Screen::Items {
            view.bag = self.bag_view(labels);
            view.selected = self.selected(labels);
        }
        Some(screens::scene(&view))
    }

    /// The Cheats screen as it shows: the run's choices, the row under the cursor and the characters' names.
    fn cheats_view(&self, labels: &Labels) -> CheatsView {
        CheatsView {
            cheats: self.cheats,
            row: self.nav.cheats_row(),
            names: labels
                .characters
                .iter()
                .take(usize::from(CHARACTERS))
                .map(|character| character.name.clone())
                .collect(),
        }
    }

    /// The bag's rows of the tab the Item screen shows, as the navigation reads them.
    fn rows(&self, labels: &Labels) -> Vec<ItemRow> {
        let Some(bag) = self.keeper.bag() else { return Vec::new() };
        bag.stacks()
            .iter()
            .filter(|stack| self.nav.tab().shows(ally(labels, stack.object)))
            .map(|stack| {
                let label = label(labels, stack.object);
                ItemRow {
                    id: stack.id,
                    usable: usable_here(label),
                    droppable: droppable(label, stack.class.key),
                    member: label.is_some_and(|label| label.member),
                    count: stack.count,
                }
            })
            .collect()
    }

    /// The bag as the Item screen shows it, once the bag is bound.
    fn bag_view(&self, labels: &Labels) -> Option<BagView> {
        let bag = self.keeper.bag()?;
        let tab = self.nav.tab();
        let stacks: Vec<StackView> = bag
            .stacks()
            .iter()
            .filter(|stack| tab.shows(ally(labels, stack.object)))
            .map(|stack| {
                let label = label(labels, stack.object);
                StackView {
                    id: stack.id,
                    object: stack.object,
                    name: label.map_or_else(|| "?".to_owned(), |label| label.name.clone()),
                    count: stack.count,
                    usable: usable_here(label),
                    droppable: droppable(label, stack.class.key),
                    ally: ally(labels, stack.object),
                }
            })
            .collect();
        let under = stacks.get(self.nav.bag_cursor());
        Some(BagView {
            tab,
            used: bag.stacks().len(),
            slots: usize::from(bag.rules().slots),
            full: bag.is_full(),
            usable_here: under.is_some_and(|stack| stack.usable),
            droppable_here: under.is_some_and(|stack| stack.droppable),
            most: under.map_or(0, |stack| stack.count),
            sheet: self.nav.sheet(),
            choice: self.nav.choice(),
            member: self.nav.member(),
            units: self.nav.units(),
            notice: self.notice,
            stacks,
        })
    }

    /// The object under the Item screen's cursor: its picture, price and help.
    fn selected(&self, labels: &Labels) -> Option<Selected> {
        let bag = self.keeper.bag()?;
        let tab = self.nav.tab();
        let stack = bag
            .stacks()
            .iter()
            .filter(|stack| tab.shows(ally(labels, stack.object)))
            .nth(self.nav.bag_cursor())?;
        let label = label(labels, stack.object)?;
        Some(Selected {
            object: stack.object,
            name: label.name.clone(),
            // Porunga's box shows the wishes he has left (the second byte of his state word)
            price: if stack.object == PORUNGA {
                Some((stack.extra >> 8) as u8)
            } else {
                label.price
            },
            help: label.help.clone(),
            battle: label.battle,
            field: label.field,
        })
    }

    /// Starts a plan that leaves the curtain at `arrival`.
    fn start(&mut self, steps: Vec<Step>, arrival: Arrival) {
        self.running = Some(Running { player: Player::new(steps), arrival });
    }

    /// The curtain follows the menu the player opens or closes, when no plan runs.
    fn observe(&mut self, probe: &MenuProbe<'_>) {
        self.curtain = match self.curtain {
            // A menu that opens always opens on its command window; the screen byte is not set yet
            // then, and it may still hold the last screen of the menu before.
            Curtain::Down if probe.opening() => {
                self.nav.open();
                Curtain::Up
            }
            Curtain::Down => match probe.screen() {
                Some(0 | 13) if probe.menu_open() => {
                    self.nav.open();
                    Curtain::Up
                }
                _ if probe.menu_open() => Curtain::Lifted,
                _ => Curtain::Down,
            },
            Curtain::Up | Curtain::Lifted if probe.field_free() => {
                self.nav = Nav::default();
                Curtain::Down
            }
            Curtain::Up | Curtain::Lifted => self.curtain,
        };
    }

    /// Where a plan that ended leaves the curtain.
    fn arrive(&mut self, arrival: Arrival, outcome: Outcome) {
        self.curtain = match (outcome, arrival) {
            (Outcome::Reached, Arrival::Show(screen)) => {
                self.nav.enter(screen);
                Curtain::Up
            }
            // A plan that could not close the menu leaves the original's menu to the player, with the
            // keys; one that aborted has closed it.
            (Outcome::Reached, Arrival::Lifted) | (Outcome::Stuck { .. }, _) => Curtain::Lifted,
            (Outcome::Reached, Arrival::Closed) | (Outcome::Aborted { .. }, _) => Curtain::Down,
        };
        if self.curtain == Curtain::Down {
            self.nav = Nav::default();
        }
    }
}

/// What the menu of `probe` has: the command window's rows (the town has no Fly, the flying party has
/// Land) and the party's size.
fn context(probe: &MenuProbe<'_>) -> Context {
    let town = probe.town();
    Context { flying: probe.flying() && !town, town, party: probe.party_size() }
}

/// The label of an object, if the labels have it.
fn label(labels: &Labels, object: u8) -> Option<&ObjectLabel> {
    labels.objects.get(usize::from(object))
}

/// Whether a stack of `object` is listed under Allies (a person of the story).
fn ally(labels: &Labels, object: u8) -> bool {
    label(labels, object).is_some_and(|label| label.ally)
}

/// Whether an object can be used on the map: EXT, and not a story helper.
fn usable_here(label: Option<&ObjectLabel>) -> bool {
    label.is_some_and(|label| label.field && !label.story)
}

/// Whether an object can be dropped: not a key item (a Dragon Ball or a story object), and not fixed.
fn droppable(label: Option<&ObjectLabel>, key: bool) -> bool {
    !key && label.is_some_and(|label| !label.fixed)
}

/// The steps and the arrival of choosing `entry` on `screen`, or `None` for an entry with nothing
/// to do (greyed out, or not on this screen). The row of an entry is its row in the original's list.
fn choice(screen: Screen, entry: Entry, flying: bool, town: bool) -> Option<(Vec<Step>, Arrival)> {
    let (row, rows, until, arrival) = match screen {
        Screen::Command => {
            let rows = command_rows(flying, town);
            let row = rows.iter().position(|&e| e == entry)?;
            let (until, arrival) = match entry {
                Entry::Talk | Entry::Look => (Until::Busy, Arrival::Lifted),
                Entry::Fly => (Until::Screen(1), Arrival::Lifted),
                Entry::Land => (Until::Screen(5), Arrival::Lifted),
                Entry::Item => (Until::ItemList, Arrival::Show(Screen::Items)),
                Entry::Menu => (Until::Screen(4), Arrival::Show(Screen::Menu)),
                _ => return None,
            };
            (row, rows.len(), until, arrival)
        }
        Screen::Menu => {
            let (row, until) = match entry {
                Entry::Cards => (0, 6),
                Entry::Status => (1, 7),
                Entry::Order => (2, 8),
                Entry::Text => (3, 9),
                Entry::Save => (4, 10),
                _ => return None,
            };
            (row, ORIGINAL_MENU_ROWS, Until::Screen(until), Arrival::Lifted)
        }
        Screen::Items | Screen::Cheats => return None,
    };
    let steps = vec![
        Step::Row { row: u8::try_from(row).ok()?, rows: u8::try_from(rows).ok()? },
        Step::Press { button: Button::A, until },
    ];
    Some((steps, arrival))
}

/// The row of the Item command in the command window of the field, the town or the flying party.
fn item_row(flying: bool, town: bool) -> u8 {
    command_rows(flying, town).iter().position(|&e| e == Entry::Item).map_or(0, |row| row as u8)
}

/// Frames the original runs after a window is drawn or a menu closes, before it takes the next press
/// (a press in the first frames after a draw is lost).
const SETTLE_WINDOW: u32 = 120;

/// The object id of the Dragon Radar: its effect is a screen of its own, which only B leaves, and
/// which does not consume the object (`01:8C9F`, effect 23 at `01:EBE4`; `docs/re/data/menus.md`).
const DRAGON_RADAR: u8 = 23;

/// What the original does when an object is used, as far as the plan's presses differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UseKind {
    /// A party list or a message, then the effect.
    Plain,
    /// The original's card screen: the curtain lifts there.
    Card,
    /// The effect runs at once on its own screen (the Dragon Radar).
    Radar,
}

/// The steps that open the original's Item list from the field (the menu is closed), then choose its
/// first row: the stack is staged there, and the original's own Use or Drop follows.
fn open_list(flying: bool, town: bool) -> Vec<Step> {
    let rows = command_rows(flying, town).len() as u8;
    vec![
        // The original settles for a moment after a menu closes before it takes the next open.
        Step::Pause { frames: SETTLE_WINDOW },
        Step::Press { button: Button::A, until: Until::Open },
        Step::Wait { until: Until::Ready },
        Step::Pause { frames: SETTLE_WINDOW },
        Step::Row { row: item_row(flying, town), rows },
        Step::Press { button: Button::A, until: Until::ItemList },
    ]
}

/// The steps of using `units` units of the stack `stack`: one round per unit, each from the field
/// (the original closes the menu after an effect). The first round starts on the Item list the HD
/// shows. The original's own effect takes each unit; the bag follows the list.
fn use_steps(
    stack: u32,
    member: Option<u8>,
    units: u16,
    kind: UseKind,
    party: usize,
    flying: bool,
    town: bool,
) -> Vec<Step> {
    let mut steps = Vec::new();
    for unit in 0..units {
        if unit > 0 {
            steps.extend(open_list(flying, town));
        }
        steps.push(Step::Stage { id: stack });
        steps.push(Step::Press { button: Button::A, until: Until::ItemChoice });
        steps.push(Step::Pause { frames: SETTLE_WINDOW });
        steps.push(Step::Row { row: 0, rows: 2 });
        if kind == UseKind::Card {
            // The card screen is the original's: the curtain lifts there.
            steps.push(Step::Press { button: Button::A, until: Until::ItemReply });
            return steps;
        }
        steps.push(Step::Press { button: Button::A, until: Until::UseTaken });
        if kind == UseKind::Radar {
            // The effect runs at once, no message and no party list. Its screen reads A as a change
            // of view, so it is left with B once it is drawn; the object stays in the bag.
            steps.push(Step::Pause { frames: SETTLE_WINDOW });
            steps.push(Step::Press { button: Button::B, until: Until::Closed });
            continue;
        }
        // The original asks for the party member when it needs one (the party list); the member
        // chosen here is the one the HD's use-on list had, and the first one otherwise.
        let row = member.unwrap_or(0);
        steps.push(Step::MemberIf { row, rows: party as u8 });
        // The message after the use waits for A; the effect runs; its result waits for A too. Then the
        // menu closes by itself, or it is left on its command window, which B closes.
        steps.push(Step::Press { button: Button::A, until: Until::ItemEffect });
        steps.push(Step::Press { button: Button::A, until: Until::Closed });
        steps.push(Step::Press { button: Button::B, until: Until::Closed });
    }
    steps
}

/// The steps of dropping `units` units of the stack `stack`: each unit is staged on the list, its
/// Drop is chosen and confirmed with Yes, and the list is back for the next one.
fn drop_steps(stack: u32, units: u16, flying: bool, town: bool) -> Vec<Step> {
    let _ = (flying, town);
    let mut steps = Vec::new();
    for _ in 0..units {
        steps.push(Step::Stage { id: stack });
        steps.push(Step::Row { row: 0, rows: 2 });
        steps.push(Step::Press { button: Button::A, until: Until::ItemChoice });
        steps.push(Step::Pause { frames: SETTLE_WINDOW });
        steps.push(Step::Row { row: 1, rows: 2 });
        steps.push(Step::Press { button: Button::A, until: Until::DropAsk });
        steps.push(Step::Row { row: 0, rows: 2 });
        steps.push(Step::Press { button: Button::A, until: Until::ItemList });
    }
    steps
}

#[cfg(test)]
mod tests {
    use super::super::testing::{FakeMenu, held};
    use super::*;

    /// One frame: the driver gives the joypad (given the live one), and the fake menu runs it.
    fn frame(driver: &mut Driver, menu: &mut FakeMenu, live: Joypad) -> Joypad {
        let wram = menu.wram();
        let pad = driver.frame(&wram, None, live).pad;
        menu.tick(pad);
        pad
    }

    /// Frames until the driver has no plan running.
    fn settle(driver: &mut Driver, menu: &mut FakeMenu) {
        for _ in 0..2_000 {
            frame(driver, menu, Joypad::default());
            if !driver.busy() {
                return;
            }
        }
        panic!("the plan did not end");
    }

    /// A key of the HD, then the frames of its plan.
    fn choose(driver: &mut Driver, menu: &mut FakeMenu, key: Key) {
        driver.key(key, &menu.wram());
        settle(driver, menu);
    }

    /// The player opens the menu on the field with a live press of A.
    fn open_by_player(driver: &mut Driver, menu: &mut FakeMenu) {
        let mut a = Joypad { a: true, ..Joypad::default() };
        for _ in 0..40 {
            frame(driver, menu, a);
            a = Joypad::default();
        }
        assert_eq!(driver.curtain(), Curtain::Up, "the curtain is up on the command window");
    }

    #[test]
    fn the_players_press_opens_the_menu_under_the_curtain_and_the_keys_are_the_hds() {
        let mut driver = Driver::new(Labels::default());
        let mut menu = FakeMenu::default();
        let a = Joypad { a: true, ..Joypad::default() };
        frame(&mut driver, &mut menu, a);
        assert_eq!(driver.curtain(), Curtain::Down, "the press opens the menu on the next frame");
        frame(&mut driver, &mut menu, Joypad::default());
        assert_eq!(driver.curtain(), Curtain::Up);
        assert_eq!(frame(&mut driver, &mut menu, a), Joypad::default(), "live keys go to the HD");
        assert_eq!(driver.nav().screen(), Screen::Command);
    }

    #[test]
    fn a_reset_brings_the_curtain_down_and_unbinds_the_bag() {
        let mut driver = Driver::new(Labels::default());
        let mut menu = FakeMenu::default();
        open_by_player(&mut driver, &mut menu);
        driver.key(Key::Down, &menu.wram());
        driver.reset();
        assert_eq!(driver.curtain(), Curtain::Down);
        assert!(driver.bag().is_none());
        assert!(!driver.busy());
    }

    #[test]
    fn a_menu_that_opens_over_a_stale_screen_byte_still_gets_the_curtain() {
        let mut driver = Driver::new(Labels::default());
        let mut menu = FakeMenu::default();
        // The last menu closed from the Item screen (the Dragon Radar does), so `menu_screen` is 2
        // while the next menu opens.
        menu.stale = true;
        open_by_player(&mut driver, &mut menu);
        assert_eq!(driver.nav().screen(), Screen::Command);
    }

    #[test]
    fn an_item_is_chosen_with_the_hd_cursor_and_cancel_brings_back_its_row() {
        let mut driver = Driver::new(Labels::default());
        let mut menu = FakeMenu::default();
        open_by_player(&mut driver, &mut menu);
        for _ in 0..3 {
            driver.key(Key::Down, &menu.wram());
        }
        assert_eq!(driver.nav().cursor(), 3, "the HD cursor is on Item");
        assert_eq!(menu.cursor, 0, "the original has not moved");
        choose(&mut driver, &mut menu, Key::Confirm);
        assert_eq!((driver.curtain(), driver.nav().screen()), (Curtain::Up, Screen::Items));
        assert_eq!(menu.screen, 2, "the original is on its Item list");
        choose(&mut driver, &mut menu, Key::Cancel);
        assert_eq!(driver.curtain(), Curtain::Up);
        assert_eq!((driver.nav().screen(), driver.nav().cursor()), (Screen::Command, 3));
        assert_eq!(menu.screen, 0);
        choose(&mut driver, &mut menu, Key::Cancel);
        assert_eq!(driver.curtain(), Curtain::Down, "cancel at the command window closes the menu");
        assert_eq!(menu.field, 0);
        assert_eq!(driver.last_outcome(), Some(Outcome::Reached));
    }

    #[test]
    fn talk_lifts_the_curtain_and_the_live_keys_reach_the_game_until_the_field_is_back() {
        let mut driver = Driver::new(Labels::default());
        let mut menu = FakeMenu::default();
        open_by_player(&mut driver, &mut menu);
        choose(&mut driver, &mut menu, Key::Confirm);
        assert_eq!(driver.curtain(), Curtain::Lifted, "the original draws its message");
        assert!(driver.scene(&menu.wram()).is_none(), "the HD draws nothing while lifted");
        let a = Joypad { a: true, ..Joypad::default() };
        assert_eq!(frame(&mut driver, &mut menu, a), a, "the live A dismisses the message");
        frame(&mut driver, &mut menu, Joypad::default());
        assert_eq!(driver.curtain(), Curtain::Down, "the field is free again");
    }

    #[test]
    fn the_menu_list_opens_the_cheats_and_its_greyed_options_do_nothing() {
        let mut driver = Driver::new(Labels::default());
        let mut menu = FakeMenu::default();
        open_by_player(&mut driver, &mut menu);
        for _ in 0..4 {
            driver.key(Key::Down, &menu.wram());
        }
        choose(&mut driver, &mut menu, Key::Confirm);
        assert_eq!((driver.curtain(), driver.nav().screen()), (Curtain::Up, Screen::Menu));
        assert_eq!(menu.screen, 4, "the original's Menu list");
        for _ in 0..5 {
            driver.key(Key::Down, &menu.wram());
        }
        assert_eq!(driver.nav().cursor(), 5, "Cheats");
        choose(&mut driver, &mut menu, Key::Confirm);
        assert_eq!((driver.curtain(), driver.nav().screen()), (Curtain::Up, Screen::Cheats));
        assert_eq!(menu.screen, 4, "nothing was sent to the original: its Menu list stays under");
        assert!(!driver.busy(), "no plan for Cheats");
        choose(&mut driver, &mut menu, Key::Cancel);
        let at = (driver.curtain(), driver.nav().screen(), driver.nav().cursor());
        assert_eq!(at, (Curtain::Up, Screen::Menu, 5), "back on the Menu list, on Cheats");
        assert_eq!(menu.screen, 4);
        driver.key(Key::Down, &menu.wram());
        choose(&mut driver, &mut menu, Key::Confirm);
        assert_eq!((driver.curtain(), driver.nav().screen()), (Curtain::Up, Screen::Menu));
        assert_eq!(menu.screen, 4, "nor for Options");
        assert!(!driver.busy(), "no plan for Options");
    }

    #[test]
    fn the_cheats_screen_writes_its_choices_into_the_cheat_block_before_the_next_frame() {
        use ssd_menus::cheats::{BLOCK_LEN, Multiplier, Setting};
        let mut driver = Driver::new(Labels::default()).with_cheats(CheatSidecar::empty());
        let mut menu = FakeMenu::default();
        open_by_player(&mut driver, &mut menu);
        for _ in 0..4 {
            driver.key(Key::Down, &menu.wram());
        }
        choose(&mut driver, &mut menu, Key::Confirm);
        for _ in 0..5 {
            driver.key(Key::Down, &menu.wram());
        }
        choose(&mut driver, &mut menu, Key::Confirm);
        assert_eq!(driver.nav().screen(), Screen::Cheats);
        driver.key(Key::Right, &menu.wram());
        assert_eq!(driver.cheats().multiplier(Setting::Damage), Multiplier::new(2, 1));
        let frame = driver.frame(&menu.wram(), None, Joypad::default());
        let block = frame.events.iter().find_map(|event| match event {
            HostEvent::Wram { address, bytes } if *address == crate::cheats::BLOCK => {
                Some(bytes.clone())
            }
            _ => None,
        });
        let block = block.expect("the block is written for the change");
        assert_eq!(block.len(), BLOCK_LEN);
        assert_eq!(&block[..2], &[2, 1], "damage x2 is the first pair of the block");
        driver.key(Key::Up, &menu.wram());
        assert_eq!(driver.nav().cheats_row(), ssd_menus::cheats::Row::ResetAll, "the rows wrap");
        driver.key(Key::Confirm, &menu.wram());
        assert!(driver.cheats().is_neutral(), "Reset all turns every cheat off");
        let frame = driver.frame(&menu.wram(), None, Joypad::default());
        assert!(frame.events.iter().any(|event| matches!(
            event,
            HostEvent::Wram { address, bytes } if *address == crate::cheats::BLOCK && bytes[..2] == [1, 1]
        )), "the reset block has the neutral pairs");
    }

    #[test]
    fn cards_lifts_the_curtain_for_the_original_screen() {
        let mut driver = Driver::new(Labels::default());
        let mut menu = FakeMenu::default();
        open_by_player(&mut driver, &mut menu);
        for _ in 0..4 {
            driver.key(Key::Down, &menu.wram());
        }
        choose(&mut driver, &mut menu, Key::Confirm);
        choose(&mut driver, &mut menu, Key::Confirm);
        assert_eq!(driver.curtain(), Curtain::Lifted);
        assert_eq!(menu.screen, 6, "the original's Cards");
    }

    #[test]
    fn fly_takes_off_and_the_take_off_closes_the_menu() {
        let mut driver = Driver::new(Labels::default());
        let mut menu = FakeMenu::default();
        open_by_player(&mut driver, &mut menu);
        driver.key(Key::Down, &menu.wram());
        driver.key(Key::Down, &menu.wram());
        choose(&mut driver, &mut menu, Key::Confirm);
        assert_eq!(driver.curtain(), Curtain::Lifted, "the take-off is the original's");
        for _ in 0..200 {
            frame(&mut driver, &mut menu, Joypad::default());
        }
        assert!(menu.flying, "the party flies");
        assert_eq!(driver.curtain(), Curtain::Down);
    }

    #[test]
    fn without_a_save_or_a_list_the_bag_stays_unbound_and_the_list_is_the_originals() {
        let mut driver = Driver::new(Labels::default());
        let mut menu = FakeMenu::default();
        open_by_player(&mut driver, &mut menu);
        let frame_out = driver.frame(&menu.wram(), None, Joypad::default());
        assert!(frame_out.events.is_empty() && frame_out.saved.is_none());
        assert!(driver.bag().is_none());
    }

    #[test]
    fn held_buttons_are_the_joypad_of_a_press() {
        assert!(held(Button::A).a);
        assert!(!held(Button::B).down);
    }

    #[test]
    fn the_use_plan_stages_the_stack_and_closes_after_each_unit() {
        let steps = use_steps(7, Some(1), 2, UseKind::Plain, 2, false, false);
        let stages = steps.iter().filter(|s| matches!(s, Step::Stage { id: 7 })).count();
        assert_eq!(stages, 2, "each unit is staged");
        let reopens =
            steps.iter().filter(|s| matches!(s, Step::Wait { until: Until::Ready })).count();
        assert_eq!(reopens, 1, "the second unit reopens the menu from the field");
        let closes =
            steps.iter().filter(|s| matches!(s, Step::Press { until: Until::Closed, .. })).count();
        // Each unit ends on a close: A on the result's message, then B on the command window if it stays.
        assert_eq!(closes, 4);
    }

    #[test]
    fn the_drop_plan_confirms_each_unit_and_keeps_the_list_open() {
        let steps = drop_steps(3, 2, false, false);
        let stages = steps.iter().filter(|s| matches!(s, Step::Stage { id: 3 })).count();
        assert_eq!(stages, 2, "each unit is staged");
        assert!(!steps.iter().any(|s| matches!(s, Step::Press { until: Until::Closed, .. })));
    }

    /// A battery save whose `slots` are good saves of the list the fake menu shows (three entries of object 0,
    /// no zeni), each with its mark and its checksum word.
    fn saved_slots(slots: &[usize]) -> Vec<u8> {
        let mut sram = vec![0_u8; ssd_save::SRAM_SIZE];
        for &slot in slots {
            let base = ssd_save::SLOT_BASES[slot];
            let count = base + ssd_save::IMAGE_LEN + (crate::data::ITEM_COUNT - 0x7E_0100) as usize;
            sram[count] = 3;
            sram[base + ssd_save::MARK_OFFSET..][..ssd_save::MARK.len()]
                .copy_from_slice(&ssd_save::MARK);
            let bytes: [u8; ssd_save::SLOT_LEN] =
                sram[base..base + ssd_save::SLOT_LEN].try_into().expect("a slot");
            let at = base + ssd_save::CHECKSUM_OFFSET;
            sram[at..at + 2].copy_from_slice(&ssd_save::checksum(&bytes).to_le_bytes());
        }
        sram
    }

    /// The checksum word of `slot` in `sram`.
    fn checksum_of_slot(sram: &[u8], slot: usize) -> u16 {
        let at = ssd_save::SLOT_BASES[slot] + ssd_save::CHECKSUM_OFFSET;
        u16::from_le_bytes([sram[at], sram[at + 1]])
    }

    /// The game's Save into `slot` (0 to 2) from its list: the list shows with its cursor on the slot, then the
    /// completion, where the cursor is back at 0. Returns the frame of the completion.
    fn save_into(driver: &mut Driver, menu: &mut FakeMenu, sram: &[u8], slot: u8) -> Frame {
        menu.screen = SAVE_SCREEN;
        menu.step = SAVE_LIST_STEP;
        menu.cursor = slot;
        driver.frame(&menu.wram(), Some(sram), Joypad::default());
        menu.step = SAVE_DONE_STEP;
        menu.cursor = 0;
        driver.frame(&menu.wram(), Some(sram), Joypad::default())
    }

    /// A driver with the cheats installed, on the Cheats screen of an open menu (the bag and the cheats are bound
    /// to the battery save `sram` by its first frame).
    fn driver_on_cheats(sram: &[u8]) -> (Driver, FakeMenu) {
        let mut driver = Driver::new(Labels::default()).with_cheats(CheatSidecar::empty());
        let mut menu = FakeMenu::default();
        menu.field = 1;
        menu.screen = 4;
        driver.state.curtain = Curtain::Up;
        driver.state.nav.enter(Screen::Cheats);
        driver.frame(&menu.wram(), Some(sram), Joypad::default());
        (driver, menu)
    }

    #[test]
    fn a_save_of_the_same_data_writes_the_cheats_chosen_since_the_last_save() {
        use ssd_menus::cheats::Row;
        let sram = saved_slots(&[0]);
        let checksum = checksum_of_slot(&sram, 0);
        let (mut driver, mut menu) = driver_on_cheats(&sram);
        driver.key(Key::Right, &menu.wram());
        let damage = *driver.cheats();
        let first = save_into(&mut driver, &mut menu, &sram, 0);
        assert_eq!(
            first.saved_cheats,
            Some(SavedCheats { slot: 0, checksum, marked: true, cheats: damage })
        );
        for _ in 0..20 {
            if driver.nav().cheats_row() == Row::RandomBattles {
                break;
            }
            driver.key(Key::Down, &menu.wram());
        }
        driver.key(Key::Right, &menu.wram());
        let both = *driver.cheats();
        let second = save_into(&mut driver, &mut menu, &sram, 0);
        assert!(second.saved.is_none(), "the bag is unchanged, the data being the same");
        assert_eq!(
            second.saved_cheats,
            Some(SavedCheats { slot: 0, checksum, marked: true, cheats: both }),
            "a save of the same data writes the cheats it is made with"
        );
        assert_eq!(driver.cheat_sidecar().entry_for(0, checksum).map(|s| s.cheats), Some(both));
        assert_eq!(driver.cheat_sidecar().saves.len(), 1, "the slot has one entry, replaced");
    }

    #[test]
    fn a_save_to_another_slot_writes_that_slot_and_the_run_keeps_its_own_save() {
        let sram = saved_slots(&[0, 1]);
        let checksum = checksum_of_slot(&sram, 1);
        let (mut driver, mut menu) = driver_on_cheats(&sram);
        assert_eq!(driver.state.keeper.bound_save(), Some((0, checksum_of_slot(&sram, 0))));
        driver.key(Key::Right, &menu.wram());
        let damage = *driver.cheats();
        let other = save_into(&mut driver, &mut menu, &sram, 1);
        assert_eq!(
            other.saved_cheats,
            Some(SavedCheats { slot: 1, checksum, marked: true, cheats: damage })
        );
        assert!(driver.cheat_sidecar().entry_for(0, checksum_of_slot(&sram, 0)).is_none());
        driver.frame(&menu.wram(), Some(&sram), Joypad::default());
        assert_eq!(*driver.cheats(), damage, "the run keeps its cheats on the save it is bound to");
    }

    #[test]
    fn a_change_of_the_cheats_is_recorded_and_a_replay_gives_them_to_its_run() {
        let mut driver = Driver::new(Labels::default()).with_cheats(CheatSidecar::empty());
        let mut menu = FakeMenu::default();
        menu.field = 1;
        menu.screen = 4;
        driver.state.curtain = Curtain::Up;
        driver.state.nav.enter(Screen::Cheats);
        driver.key(Key::Right, &menu.wram());
        let events = driver.frame(&menu.wram(), None, Joypad::default()).events;
        let payload = events.iter().find_map(|event| match event {
            HostEvent::App { tag, payload } if *tag == CHEATS_TAG => Some(payload.clone()),
            _ => None,
        });
        let payload = payload.expect("the change is recorded as an application event");
        let mut played = Driver::new(Labels::default()).with_cheats(CheatSidecar::empty());
        played.replay_event(CHEATS_TAG, &payload, &menu.wram()).expect("a cheats file");
        assert_eq!(played.cheats(), driver.cheats(), "the replay's run has the recorded cheats");
        assert!(played.replay_event(CHEATS_TAG, &[0xFF; 3], &menu.wram()).is_err());
    }
}
