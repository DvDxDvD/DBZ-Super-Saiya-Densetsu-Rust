//! `menu-test`: the HD menus played headless on the cached ROM, as a player plays them. A scenario
//! starts from a crafted save (its Continue, then the field) or from a frame of a replay, opens the
//! menu with a press of A, makes the HD's choices with keys (the driver of `ssd_game::menu::driver`
//! plays them), and checks what the original and the curtain show after each. `town` starts from the
//! shop's town. `diff` checks that a choice's own presses, replayed as a plain Classic run, end in the
//! same work RAM, and that the Classic timeline of the same choice reaches the same game state.
//! Nothing is written but the report on stdout (the items suite's scratch files are in a temporary
//! directory that it removes at its end).

use crate::save;
use anyhow::{Context, bail};
use ssd_game::data as d;
use ssd_game::menu::driver::{Binding, Curtain, Driver};
use ssd_game::menu::presses::{Next, Outcome};
use ssd_game::scripts;
use ssd_menus::Key;
use ssd_menus::bag::{Bag, Class, Rules, Sidecar};
use ssd_menus::view::Sheet;
use ssd_oracle::{
    CORE_VERSION, HostEvent, Joypad, MODE_ENHANCED, Machine, MemorySaveStore, NullAudio, Recorder,
    Replay, apply_events,
};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The bus address of the first byte of work RAM.
const WRAM_BASE: u32 = 0x7E_0000;
/// Frames a choice may run before the test calls it stuck.
const CHOICE_LIMIT: u32 = 6_000;
/// Idle frames after a comparison point, before the states are compared.
const SETTLE: u32 = 240;
/// Frames a message runs before it takes A (its text is drawn letter by letter).
const MESSAGE_RUN: u32 = 200;

/// Where a run starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Start {
    /// A battery save: its Continue of `slot` (0 to 2), then the field.
    Save { path: PathBuf, slot: usize },
    /// A replay, played for `frames` frames (the field of its map, with the menu closed).
    Replay { path: PathBuf, frames: u64 },
    /// The cheats suite's scenarios: the reference replays of `dir` (the workspace's by default), and the scripted
    /// loss from a battery save (its slot, from `--slot`), when one is given.
    Scenarios { dir: Option<PathBuf>, save: Option<(PathBuf, usize)>, data_out: Option<PathBuf> },
}

/// The command line of `menu-test`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    /// `field` (the scenarios) or `diff` (the comparison with Classic presses).
    pub suite: String,
    pub start: Start,
    /// The scenarios to run (all of the suite when empty).
    pub only: Vec<String>,
}

/// Parses `field|town|diff --save <file.sav> [--slot N] | --replay <file> --frames N [scenario...]`.
///
/// # Errors
///
/// A usage message.
pub fn parse_args(args: &[&str]) -> Result<Args, String> {
    let mut it = args.iter();
    let suite = match it.next() {
        Some(&suite @ ("field" | "town" | "diff" | "items" | "cheats")) => suite.to_owned(),
        _ => return Err("menu-test takes field, town, diff, items or cheats".to_owned()),
    };
    let mut save = None;
    let mut slot = 0;
    let mut replay = None;
    let mut frames = None;
    let mut replays = None;
    let mut data_out = None;
    let mut only = Vec::new();
    while let Some(&arg) = it.next() {
        match arg {
            "--save" => save = it.next().map(PathBuf::from),
            "--replays" => replays = it.next().map(PathBuf::from),
            "--data-out" => data_out = it.next().map(PathBuf::from),
            "--slot" => {
                slot = it
                    .next()
                    .and_then(|n| n.parse::<usize>().ok())
                    .filter(|n| (1..=3).contains(n))
                    .ok_or("--slot takes 1, 2 or 3")?
                    - 1;
            }
            "--replay" => replay = it.next().map(PathBuf::from),
            "--frames" => {
                frames = it.next().and_then(|n| n.parse::<u64>().ok());
                if frames.is_none() {
                    return Err("--frames takes a number of frames".to_owned());
                }
            }
            option if option.starts_with("--") => return Err(format!("unknown option `{option}`")),
            name => only.push(name.to_owned()),
        }
    }
    let start = match (suite.as_str(), save, replay, frames) {
        ("cheats", save, None, None) => {
            Start::Scenarios { dir: replays, save: save.map(|path| (path, slot)), data_out }
        }
        (_, Some(path), None, None) => Start::Save { path, slot },
        (_, None, Some(path), Some(frames)) => Start::Replay { path, frames },
        _ => return Err("give --save <file.sav>, or --replay <file> with --frames N".to_owned()),
    };
    Ok(Args { suite, start, only })
}

/// The checks of one scenario: each is a line of the report, and a failed one fails the suite.
#[derive(Default)]
pub(crate) struct Report {
    lines: Vec<(bool, String)>,
}

impl Report {
    pub(crate) fn check(&mut self, ok: bool, what: &str) {
        self.lines.push((ok, what.to_owned()));
    }

    pub(crate) fn passed(&self) -> bool {
        self.lines.iter().all(|(ok, _)| *ok)
    }
}

/// A run of the menu on the machine, with the driver that plays the HD.
pub(crate) struct Harness {
    pub(crate) machine: Machine,
    pub(crate) driver: Driver,
    /// The joypad of each frame run through the driver, when it is recorded.
    tape: Option<Vec<Joypad>>,
    /// The replay being recorded, when the frames are recorded (their host events too).
    recorder: Option<Recorder>,
}

impl Harness {
    /// A harness of a machine and its driver, with nothing recorded.
    pub(crate) fn new(machine: Machine, driver: Driver) -> Self {
        Self { machine, driver, tape: None, recorder: None }
    }

    /// One frame: the driver's joypad and host events for it (from the live one), the events made on
    /// the machine, and the frame run.
    pub(crate) fn frame(&mut self, live: Joypad) -> anyhow::Result<()> {
        let frame = self.driver.frame(self.machine.wram(), self.machine.sram(), live);
        apply_events(&mut self.machine, &frame.events)?;
        match self.recorder.as_mut() {
            Some(recorder) => {
                recorder.run_frame_with_events(
                    &mut self.machine,
                    frame.pad,
                    &frame.events,
                    &mut NullAudio,
                )?;
            }
            None => self.machine.run_frame(frame.pad, &mut NullAudio)?,
        }
        if let Some(tape) = self.tape.as_mut() {
            tape.push(frame.pad);
        }
        Ok(())
    }

    pub(crate) fn frames(&mut self, count: u32) -> anyhow::Result<()> {
        for _ in 0..count {
            self.frame(Joypad::default())?;
        }
        Ok(())
    }

    /// A press of the player's own: held for two frames, released on the third.
    pub(crate) fn press(&mut self, pad: Joypad) -> anyhow::Result<()> {
        self.frame(pad)?;
        self.frame(pad)?;
        self.frame(Joypad::default())
    }

    /// Frames until the driver has no choice under way.
    pub(crate) fn until_idle(&mut self) -> anyhow::Result<()> {
        for _ in 0..CHOICE_LIMIT {
            if !self.driver.busy() {
                return Ok(());
            }
            self.frame(Joypad::default())?;
        }
        bail!("a choice did not end in {CHOICE_LIMIT} frames (curtain {:?})", self.driver.curtain())
    }

    /// The HD's key, and the frames of its choice.
    pub(crate) fn key(&mut self, key: Key) -> anyhow::Result<()> {
        self.driver.key(key, self.machine.wram());
        self.frame(Joypad::default())?;
        self.until_idle()
    }

    /// The player's A on the field: the menu opens, and the HD shows its command window.
    pub(crate) fn open(&mut self) -> anyhow::Result<()> {
        self.driver.open();
        self.frame(Joypad::default())?;
        self.until_idle()?;
        self.frames(30)
    }

    /// Closes the menu the original keeps after a lifted screen: B presses until the field is free.
    pub(crate) fn close_lifted(&mut self) -> anyhow::Result<()> {
        for _ in 0..12 {
            if self.byte(d::FIELD_STATE) == 0 {
                break;
            }
            self.press(b())?;
            self.frames(40)?;
        }
        Ok(())
    }

    /// A byte of work RAM.
    pub(crate) fn byte(&self, address: u32) -> u8 {
        self.machine.wram()[(address - WRAM_BASE) as usize]
    }

    /// A word of work RAM (little-endian).
    pub(crate) fn word(&self, address: u32) -> u16 {
        u16::from_le_bytes([self.byte(address), self.byte(address + 1)])
    }
}

fn a() -> Joypad {
    Joypad { a: true, ..Joypad::default() }
}

fn b() -> Joypad {
    Joypad { b: true, ..Joypad::default() }
}

/// What a start gives: a machine at the field, and how to make another one there.
struct Starts {
    /// The machine's state at the field.
    state: Vec<u8>,
    /// A machine of the same cartridge (the save in it), before its state is loaded.
    fresh: Box<dyn Fn() -> anyhow::Result<Machine>>,
}

/// Plays the start: the save's Continue or the replay's frames, and keeps the state there.
fn start_of(rom: &[u8], start: &Start) -> anyhow::Result<Starts> {
    let rom = rom.to_vec();
    match start {
        Start::Scenarios { .. } => {
            bail!("menu-test cheats runs its own scenarios: it has no start state")
        }
        Start::Save { path, slot } => {
            let sram =
                std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
            let make = move || {
                let mut saves = MemorySaveStore::default();
                saves.0.insert("sav".to_owned(), sram.clone());
                Ok(Machine::new(rom.clone(), Box::new(saves))?)
            };
            let mut machine = make()?;
            let timeline = scripts::parse_timeline(&save::continue_prefix(*slot))
                .map_err(anyhow::Error::msg)?;
            for pad in timeline {
                machine.run_frame(pad, &mut NullAudio)?;
            }
            let state = machine.save_state();
            Ok(Starts { state, fresh: Box::new(make) })
        }
        Start::Replay { path, frames } => {
            let bytes =
                std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
            let crc = ssd_rom::crc32(&rom);
            let replay = Replay::from_bytes(&bytes, crc, CORE_VERSION)?;
            let make = move || {
                let replay = Replay::from_bytes(&bytes, crc, CORE_VERSION)?;
                Ok(replay.power_on(rom.clone())?)
            };
            let mut machine = make()?;
            for frame in 0..*frames {
                machine.run_frame(replay.input(frame), &mut NullAudio)?;
            }
            let state = machine.save_state();
            Ok(Starts { state, fresh: Box::new(make) })
        }
    }
}

/// A harness at the start: a fresh machine with the start's state, and a driver with the labels. The
/// bag is not bound (the original's list is kept as it is) unless `bag` asks for it.
fn harness(rom: &[u8], starts: &Starts) -> anyhow::Result<Harness> {
    harness_with(rom, starts, false)
}

/// [`harness`], with the bag bound to the save when `bag` is set (as the window binds it).
fn harness_with(rom: &[u8], starts: &Starts, bag: bool) -> anyhow::Result<Harness> {
    let mut machine = (starts.fresh)()?;
    machine.load_state(&starts.state)?;
    let catalogue = ssd_catalogue::Catalogue::from_rom(rom)?;
    let mut driver = Driver::new(ssd_catalogue::menu_labels(&catalogue));
    if !bag {
        driver.set_binding(Binding::Recorded);
    }
    Ok(Harness { machine, driver, tape: None, recorder: None })
}

/// The scenarios of the field suite, by name.
type Scenario = fn(&mut Harness, &mut Report) -> anyhow::Result<()>;

const FIELD_SCENARIOS: [(&str, Scenario); 14] = [
    ("talk", talk),
    ("look", look),
    ("fly-land", fly_land),
    ("fly-menu", fly_menu),
    ("item", item),
    ("menu-cards", menu_cards),
    ("menu-status", menu_status),
    ("menu-order", menu_order),
    ("menu-text", menu_text),
    ("menu-save", menu_save),
    ("cancel-command", cancel_command),
    ("cancel-menu", cancel_menu),
    ("menu-cheats", menu_cheats),
    ("abort", abort),
];

fn talk(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    h.open()?;
    h.key(Key::Confirm)?;
    r.check(h.driver.curtain() == Curtain::Lifted, "Talk lifts the curtain for the message");
    r.check(h.byte(d::MENU_BUSY) == 1, "the message is up (menu busy)");
    // The text runs before it waits for A: a press during the run is not taken.
    h.frames(MESSAGE_RUN)?;
    h.press(a())?;
    h.frames(SETTLE)?;
    r.check(h.byte(d::FIELD_STATE) == 0, "the message closes and the field is free");
    r.check(h.driver.curtain() == Curtain::Down, "the curtain comes down with the field");
    Ok(())
}

fn look(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    h.open()?;
    h.key(Key::Down)?;
    h.key(Key::Confirm)?;
    r.check(h.driver.curtain() == Curtain::Lifted, "Look lifts the curtain for the message");
    r.check(h.byte(d::MENU_BUSY) == 1, "the message is up");
    h.frames(MESSAGE_RUN)?;
    h.press(a())?;
    h.frames(SETTLE)?;
    r.check(h.byte(d::FIELD_STATE) == 0, "the message closes and the field is free");
    Ok(())
}

fn fly_land(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    h.open()?;
    h.key(Key::Down)?;
    h.key(Key::Down)?;
    h.key(Key::Confirm)?;
    r.check(h.driver.curtain() == Curtain::Lifted, "Fly lifts the curtain for the take-off");
    r.check(h.byte(d::MENU_SCREEN) == 1, "the original takes off on its screen 1");
    h.frames(400)?;
    r.check(h.byte(d::TRAVEL_MODE) == 2, "the party flies");
    r.check(h.byte(d::FIELD_STATE) == 0, "the take-off closes the menu");
    r.check(h.driver.curtain() == Curtain::Down, "the curtain comes down");
    h.open()?;
    r.check(h.driver.nav().screen() == ssd_menus::Screen::Command, "flying: the command window");
    h.key(Key::Confirm)?;
    r.check(h.byte(d::MENU_SCREEN) == 5, "Land runs on the original's screen 5");
    r.check(h.driver.curtain() == Curtain::Lifted, "Land lifts the curtain for the landing");
    h.frames(400)?;
    r.check(h.byte(d::TRAVEL_MODE) == 1, "the party walks again");
    r.check(h.byte(d::FIELD_STATE) == 0, "the landing closes the menu");
    Ok(())
}

fn item(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    // The bag is bound, as the window binds it: its keys move the HD cursor over the stacks.
    h.driver.set_binding(Binding::Auto);
    h.frames(1)?;
    h.open()?;
    for _ in 0..3 {
        h.key(Key::Down)?;
    }
    h.key(Key::Confirm)?;
    r.check(h.driver.curtain() == Curtain::Up, "Item keeps the curtain up: the HD item list");
    r.check(h.driver.nav().screen() == ssd_menus::Screen::Items, "the HD shows the item list");
    r.check(h.byte(d::MENU_SCREEN) == 2, "the original is on its Item screen");
    h.key(Key::Down)?;
    h.key(Key::Down)?;
    r.check(h.driver.nav().cursor() == 2, "the HD cursor moves down the list");
    h.key(Key::Cancel)?;
    r.check(h.driver.nav().screen() == ssd_menus::Screen::Command, "cancel goes back");
    r.check(h.driver.nav().cursor() == 3, "the command window keeps the Item row");
    r.check(h.byte(d::MENU_SCREEN) == 0, "the original is back on its command window");
    h.key(Key::Cancel)?;
    r.check(h.driver.curtain() == Curtain::Down, "cancel at the command window closes the menu");
    r.check(h.byte(d::FIELD_STATE) == 0, "the field is free");
    Ok(())
}

/// Opens the Menu list (the fourth row of the field's command window), for the four entries below.
pub(crate) fn menu_list(h: &mut Harness) -> anyhow::Result<()> {
    h.open()?;
    for _ in 0..4 {
        h.key(Key::Down)?;
    }
    h.key(Key::Confirm)?;
    Ok(())
}

/// One entry of the Menu list: its original screen opens under the lifted curtain, and the field
/// comes back when the player has closed it.
fn menu_entry(
    h: &mut Harness,
    r: &mut Report,
    downs: usize,
    screen: u8,
    name: &str,
) -> anyhow::Result<()> {
    menu_list(h)?;
    r.check(h.driver.nav().screen() == ssd_menus::Screen::Menu, "the HD Menu list is up");
    for _ in 0..downs {
        h.key(Key::Down)?;
    }
    h.key(Key::Confirm)?;
    r.check(h.driver.curtain() == Curtain::Lifted, &format!("{name} lifts the curtain"));
    r.check(
        h.byte(d::MENU_SCREEN) == screen,
        &format!("{name} opens the original's screen {screen}"),
    );
    h.frames(SETTLE)?;
    h.close_lifted()?;
    r.check(h.byte(d::FIELD_STATE) == 0, &format!("{name} returns to the field"));
    r.check(h.driver.curtain() == Curtain::Down, &format!("{name}: the curtain is down"));
    Ok(())
}

fn menu_cards(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    menu_entry(h, r, 0, 6, "Cards")
}

fn menu_status(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    menu_entry(h, r, 1, 7, "Status")
}

fn menu_order(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    menu_entry(h, r, 2, 8, "Order")
}

fn menu_text(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    menu_entry(h, r, 3, 9, "Text")
}

pub(crate) fn menu_save(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    menu_list(h)?;
    for _ in 0..4 {
        h.key(Key::Down)?;
    }
    h.key(Key::Confirm)?;
    r.check(h.byte(d::MENU_SCREEN) == 10, "Save opens the original's slot screen");
    r.check(h.driver.curtain() == Curtain::Lifted, "Save lifts the curtain");
    // The slot, the question and the completion each take an A; the last one closes the menu.
    for _ in 0..4 {
        if h.byte(d::MENU_SCREEN) != 10 {
            break;
        }
        h.press(a())?;
        h.frames(60)?;
    }
    h.close_lifted()?;
    r.check(h.byte(d::FIELD_STATE) == 0, "Save returns to the field");
    Ok(())
}

fn cancel_command(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    h.open()?;
    h.key(Key::Cancel)?;
    r.check(h.driver.curtain() == Curtain::Down, "cancel closes the menu");
    r.check(h.byte(d::FIELD_STATE) == 0, "the field is free");
    Ok(())
}

fn cancel_menu(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    menu_list(h)?;
    h.key(Key::Cancel)?;
    r.check(h.driver.nav().screen() == ssd_menus::Screen::Command, "cancel leaves the Menu list");
    r.check(h.byte(d::MENU_SCREEN) == 0, "the original is on its command window");
    r.check(h.driver.nav().cursor() == 4, "the command window keeps the Menu row");
    h.key(Key::Cancel)?;
    r.check(h.driver.curtain() == Curtain::Down, "a second cancel closes the menu");
    Ok(())
}

/// The Menu list's Cheats opens the HD cheats screen: nothing is sent to the original, which keeps its Menu list
/// under it, and B goes back to the Menu list. Options is greyed out and chooses nothing.
fn menu_cheats(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    menu_list(h)?;
    for _ in 0..5 {
        h.key(Key::Down)?;
    }
    h.key(Key::Confirm)?;
    r.check(h.driver.nav().screen() == ssd_menus::Screen::Cheats, "Cheats opens the cheats screen");
    r.check(h.driver.curtain() == Curtain::Up, "the cheats screen is the HD's: the curtain is up");
    r.check(h.byte(d::MENU_SCREEN) == 4, "nothing is sent to the original for Cheats");
    r.check(!h.driver.busy(), "no choice runs for Cheats");
    h.key(Key::Cancel)?;
    r.check(h.driver.nav().screen() == ssd_menus::Screen::Menu, "B goes back to the Menu list");
    r.check(h.driver.nav().cursor() == 5, "the Menu list keeps its cursor on Cheats");
    h.key(Key::Down)?;
    h.key(Key::Confirm)?;
    r.check(h.byte(d::MENU_SCREEN) == 4 && !h.driver.busy(), "nor for Options");
    Ok(())
}

/// The flying command window: Land, Item and Menu; Item and Menu go to the same screens as on foot.
fn fly_menu(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    h.open()?;
    h.key(Key::Down)?;
    h.key(Key::Down)?;
    h.key(Key::Confirm)?;
    h.frames(400)?;
    h.open()?;
    h.key(Key::Down)?;
    h.key(Key::Confirm)?;
    r.check(h.byte(d::MENU_SCREEN) == 2, "flying: Item opens the item list");
    h.key(Key::Cancel)?;
    h.key(Key::Cancel)?;
    r.check(h.driver.curtain() == Curtain::Down, "flying: the menu closes");
    Ok(())
}

/// A macro that cannot reach its state aborts, and the field is playable again: the plan is run by
/// hand with an expectation the Item list never meets.
fn abort(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    use ssd_game::menu::presses::{Button, Player, Step, Until};
    h.open()?;
    let mut player = Player::new(vec![
        Step::Row { row: 3, rows: 5 },
        Step::Press { button: Button::A, until: Until::Screen(9) },
    ]);
    // The plan only reads the menu's state, so the labels are not needed.
    let labels = ssd_menus::Labels::default();
    let outcome = loop {
        let pad = {
            let probe = ssd_game::MenuProbe::new(h.machine.wram(), &labels);
            player.next(&probe)
        };
        match pad {
            Next::Frame(pad) => h.machine.run_frame(pad, &mut NullAudio)?,
            Next::Stage(_) => {}
            Next::Done(outcome) => break outcome,
        }
    };
    r.check(
        matches!(outcome, Outcome::Aborted { .. }),
        "a plan that cannot reach its state aborts and closes the menu",
    );
    r.check(h.byte(d::FIELD_STATE) == 0, "the field is free after the abort");
    let leader = |h: &Harness| (h.word(LEADER_X), h.word(LEADER_Y));
    let before = leader(h);
    for _ in 0..30 {
        h.machine.run_frame(Joypad { right: true, ..Joypad::default() }, &mut NullAudio)?;
    }
    r.check(leader(h) != before, "the party walks again after the abort");
    Ok(())
}

/// The town's command window (screen 13): Talk, Look, Item and Menu, four rows and no Fly. It starts
/// from the shop replay's frame before its town menu opens.
fn town_open(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    h.open()?;
    r.check(h.byte(d::MENU_SCREEN) == 13, "the town's command window is screen 13");
    r.check(
        h.driver.nav().screen() == ssd_menus::Screen::Command,
        "the HD shows the command window",
    );
    for _ in 0..4 {
        h.key(Key::Down)?;
    }
    r.check(h.driver.nav().cursor() == 0, "four rows: Down wraps after Menu, with no Fly");
    Ok(())
}

fn town_item(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    town_open(h, r)?;
    h.key(Key::Down)?;
    h.key(Key::Down)?;
    h.key(Key::Confirm)?;
    r.check(h.driver.curtain() == Curtain::Up, "Item in town opens the HD item list");
    r.check(h.byte(d::MENU_SCREEN) == 2, "the original is on its Item screen");
    h.key(Key::Cancel)?;
    r.check(h.byte(d::MENU_SCREEN) == 13, "cancel brings back the town's command window");
    h.key(Key::Cancel)?;
    r.check(h.driver.curtain() == Curtain::Down && h.byte(d::FIELD_STATE) == 0, "cancel closes it");
    Ok(())
}

fn town_menu(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    town_open(h, r)?;
    h.key(Key::Down)?;
    h.key(Key::Down)?;
    h.key(Key::Down)?;
    h.key(Key::Confirm)?;
    r.check(
        h.driver.nav().screen() == ssd_menus::Screen::Menu,
        "Menu in town opens the HD Menu list",
    );
    h.key(Key::Confirm)?;
    r.check(h.driver.curtain() == Curtain::Lifted && h.byte(d::MENU_SCREEN) == 6, "Cards opens");
    h.close_lifted()?;
    r.check(h.byte(d::FIELD_STATE) == 0, "Cards closes back to the field");
    Ok(())
}

fn town_talk(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    town_open(h, r)?;
    h.key(Key::Confirm)?;
    r.check(
        h.driver.curtain() == Curtain::Lifted,
        "Talk in town lifts the curtain for its message",
    );
    // The shopkeeper's talk runs over several messages: A goes on until the field is free again.
    h.frames(MESSAGE_RUN)?;
    for _ in 0..12 {
        if h.byte(d::FIELD_STATE) == 0 {
            break;
        }
        h.press(a())?;
        h.frames(MESSAGE_RUN)?;
    }
    h.frames(SETTLE)?;
    r.check(h.byte(d::FIELD_STATE) == 0, "the message closes and the field is free");
    Ok(())
}

/// The town scenarios, from the shop replay.
const TOWN_SCENARIOS: [(&str, Scenario); 3] =
    [("town-item", town_item), ("town-menu", town_menu), ("town-talk", town_talk)];

/// The leader's position on the map, x and y words (`re/symbols.toml`, the party's position).
const LEADER_X: u32 = 0x7E_01A2;
const LEADER_Y: u32 = 0x7E_01A4;

/// The field suite: every scenario from the start, and its checks as a report.
fn field_suite(
    rom: &[u8],
    starts: &Starts,
    only: &[String],
    out: &mut String,
) -> anyhow::Result<bool> {
    scenario_suite(rom, starts, &FIELD_SCENARIOS, "field", only, out)
}

/// The scenarios of a suite, each from the start, with its checks as a report.
fn scenario_suite(
    rom: &[u8],
    starts: &Starts,
    scenarios: &[(&str, Scenario)],
    label: &str,
    only: &[String],
    out: &mut String,
) -> anyhow::Result<bool> {
    let mut all_ok = true;
    let mut ran = 0;
    for &(name, scenario) in scenarios {
        if !only.is_empty() && !only.iter().any(|o| o == name) {
            continue;
        }
        ran += 1;
        let mut report = Report::default();
        let mut h = harness(rom, starts)?;
        let result = scenario(&mut h, &mut report);
        if let Err(error) = &result {
            report.check(false, &format!("the scenario failed: {error:#}"));
        }
        let ok = report.passed();
        all_ok &= ok;
        writeln!(out, "{} {name}", if ok { "ok  " } else { "FAIL" })?;
        for (passed, what) in &report.lines {
            if !passed {
                writeln!(out, "       - {what}")?;
            }
        }
    }
    writeln!(
        out,
        "menu-test {label}: {ran} scenarios, {}",
        if all_ok { "all ok" } else { "FAILED" }
    )?;
    Ok(all_ok)
}

/// The game's own state the Classic check compares, as ranges of work RAM: the items, the money,
/// the party, the party's mode and its map. The timers and the random generators are left out.
const GAME_STATE: [(u32, u32); 7] = [
    (0x7E_0100, 0x30),  // item_list
    (0x7E_01B2, 0x03),  // item_count and zeni (a word)
    (0x7E_0190, 0x10),  // party_order
    (0x7E_0B00, 0x120), // party_records
    (0x7E_0D55, 0x01),  // travel_mode
    (0x7E_01A0, 0x01),  // map_id
    (0x7E_01A2, 0x04),  // the leader's position
];

/// The part of work RAM the ranges of `GAME_STATE` cover, as (address, value) pairs.
fn game_state(machine: &Machine) -> Vec<(u32, u8)> {
    let wram = machine.wram();
    GAME_STATE
        .iter()
        .flat_map(|&(start, len)| {
            (start..start + len).map(move |address| (address, wram[(address - WRAM_BASE) as usize]))
        })
        .collect()
}

/// One choice and its checks, against the Classic presses of the same choice.
struct Case {
    name: &'static str,
    /// The choice, played through the driver (its presses are the driver's).
    hd: fn(&mut Harness) -> anyhow::Result<()>,
    /// The same presses as a Classic timeline in the style of the reference scripts (`menus.txt`):
    /// the menu opens with A on the field, Down moves the cursor, A chooses, and the waits let each
    /// screen draw.
    classic: &'static str,
}

/// The choices the differential checks: the command window, the Menu list, the flying window, and
/// the cancels.
const CASES: [Case; 10] = [
    Case {
        name: "talk",
        hd: |h| {
            h.open()?;
            h.key(Key::Confirm)
        },
        classic: "4 a\n120 -\n4 a\n260 -\n",
    },
    Case {
        name: "look",
        hd: |h| {
            h.open()?;
            h.key(Key::Down)?;
            h.key(Key::Confirm)
        },
        classic: "4 a\n120 -\n4 down\n30 -\n4 a\n260 -\n",
    },
    Case {
        name: "item",
        hd: |h| {
            h.open()?;
            for _ in 0..3 {
                h.key(Key::Down)?;
            }
            h.key(Key::Confirm)
        },
        classic: "4 a\n120 -\n4 down\n30 -\n4 down\n30 -\n4 down\n30 -\n4 a\n150 -\n",
    },
    Case {
        name: "cancel-item",
        hd: |h| {
            h.open()?;
            for _ in 0..3 {
                h.key(Key::Down)?;
            }
            h.key(Key::Confirm)?;
            h.key(Key::Cancel)?;
            h.key(Key::Cancel)
        },
        classic: "4 a\n120 -\n4 down\n30 -\n4 down\n30 -\n4 down\n30 -\n4 a\n150 -\n4 b\n120 -\n4 b\n120 -\n",
    },
    Case {
        name: "menu-cards",
        hd: |h| {
            menu_list(h)?;
            h.key(Key::Confirm)
        },
        classic: "4 a\n120 -\n4 down\n30 -\n4 down\n30 -\n4 down\n30 -\n4 down\n30 -\n4 a\n120 -\n4 a\n200 -\n",
    },
    Case {
        name: "menu-status",
        hd: |h| {
            menu_list(h)?;
            h.key(Key::Down)?;
            h.key(Key::Confirm)
        },
        classic: "4 a\n120 -\n4 down\n30 -\n4 down\n30 -\n4 down\n30 -\n4 down\n30 -\n4 a\n120 -\n4 down\n30 -\n4 a\n200 -\n",
    },
    Case {
        name: "menu-text",
        hd: |h| {
            menu_list(h)?;
            for _ in 0..3 {
                h.key(Key::Down)?;
            }
            h.key(Key::Confirm)
        },
        classic: "4 a\n120 -\n4 down\n30 -\n4 down\n30 -\n4 down\n30 -\n4 down\n30 -\n4 a\n120 -\n4 down\n30 -\n4 down\n30 -\n4 down\n30 -\n4 a\n200 -\n",
    },
    Case {
        name: "fly",
        hd: |h| {
            h.open()?;
            h.key(Key::Down)?;
            h.key(Key::Down)?;
            h.key(Key::Confirm)?;
            h.until_idle()
        },
        classic: "4 a\n120 -\n4 down\n30 -\n4 down\n30 -\n4 a\n400 -\n",
    },
    Case {
        name: "land",
        hd: |h| {
            h.open()?;
            h.key(Key::Down)?;
            h.key(Key::Down)?;
            h.key(Key::Confirm)?;
            h.until_idle()?;
            h.frames(400)?;
            h.open()?;
            h.key(Key::Confirm)?;
            h.until_idle()
        },
        classic: "4 a\n120 -\n4 down\n30 -\n4 down\n30 -\n4 a\n400 -\n4 a\n120 -\n4 a\n400 -\n",
    },
    Case {
        name: "cancel-menu",
        hd: |h| {
            menu_list(h)?;
            h.key(Key::Cancel)?;
            h.key(Key::Cancel)
        },
        classic: "4 a\n120 -\n4 down\n30 -\n4 down\n30 -\n4 down\n30 -\n4 down\n30 -\n4 a\n120 -\n4 b\n120 -\n4 b\n120 -\n",
    },
];

/// Checks one choice: the joypad words the driver played for it, replayed on a plain machine with
/// no driver, end in exactly the same work RAM (every byte), frame for frame. So a choice is nothing
/// but presses, and a recorded replay of it is a Classic replay. Returns the replayed machine.
fn tape_check(
    rom: &[u8],
    starts: &Starts,
    case: &Case,
    out: &mut String,
) -> anyhow::Result<(bool, Machine)> {
    let mut played = harness(rom, starts)?;
    played.tape = Some(Vec::new());
    (case.hd)(&mut played)?;
    played.frames(SETTLE)?;
    let tape = played.tape.take().unwrap_or_default();
    let mut replayed = (starts.fresh)()?;
    replayed.load_state(&starts.state)?;
    for pad in &tape {
        replayed.run_frame(*pad, &mut NullAudio)?;
    }
    let (a, b) = (played.machine.wram(), replayed.wram());
    let differing: Vec<usize> = (0..a.len()).filter(|&i| a[i] != b[i]).collect();
    let same_frames = played.machine.frame_count() == replayed.frame_count();
    let ok = differing.is_empty() && same_frames;
    let shown: Vec<String> = differing
        .iter()
        .take(12)
        .map(|&i| format!("{:06X} {:02X}/{:02X}", WRAM_BASE + i as u32, a[i], b[i]))
        .collect();
    let presses = usize::from(tape.first().is_some_and(|pad| pad.a))
        + tape.windows(2).filter(|w| !w[0].a && w[1].a).count();
    writeln!(
        out,
        "{} {} tape: {} frames, {presses} presses of A; replayed as Classic, {} bytes of work RAM differ{}{}",
        if ok { "ok  " } else { "FAIL" },
        case.name,
        tape.len(),
        differing.len(),
        if shown.is_empty() { "" } else { ": " },
        shown.join(" ")
    )?;
    Ok((ok, replayed))
}

/// Checks that the hand-written Classic timeline of a choice reaches the same game state (items,
/// money, party, travel mode, map and position) as the choice's own presses, at the same frame.
fn classic_check(
    rom: &[u8],
    starts: &Starts,
    case: &Case,
    by_tape: Machine,
    out: &mut String,
) -> anyhow::Result<bool> {
    let _ = rom;
    let mut by_hand = (starts.fresh)()?;
    by_hand.load_state(&starts.state)?;
    for pad in scripts::parse_timeline(case.classic).map_err(anyhow::Error::msg)? {
        by_hand.run_frame(pad, &mut NullAudio)?;
    }
    let mut by_tape = by_tape;
    let target = by_tape.frame_count().max(by_hand.frame_count());
    while by_hand.frame_count() < target {
        by_hand.run_frame(Joypad::default(), &mut NullAudio)?;
    }
    while by_tape.frame_count() < target {
        by_tape.run_frame(Joypad::default(), &mut NullAudio)?;
    }
    let (ours, theirs) = (game_state(&by_tape), game_state(&by_hand));
    let differing: Vec<String> = ours
        .iter()
        .zip(theirs.iter())
        .filter(|(x, y)| x != y)
        .take(12)
        .map(|(x, y)| format!("{:06X} {:02X}/{:02X}", x.0, x.1, y.1))
        .collect();
    let ok = differing.is_empty();
    writeln!(
        out,
        "{} {} Classic timeline: the game state at frame {target} {}{}",
        if ok { "ok  " } else { "FAIL" },
        case.name,
        if ok { "is equal" } else { "differs:" },
        if ok { String::new() } else { format!(" {}", differing.join(" ")) }
    )?;
    Ok(ok)
}

/// The differential suite: each choice's presses against its recorded replay, and against its
/// hand-written Classic timeline.
fn diff_suite(
    rom: &[u8],
    starts: &Starts,
    only: &[String],
    out: &mut String,
) -> anyhow::Result<bool> {
    let mut all_ok = true;
    for case in &CASES {
        if !only.is_empty() && !only.iter().any(|o| o == case.name) {
            continue;
        }
        let (tape_ok, replayed) = tape_check(rom, starts, case, out)?;
        let hand_ok = classic_check(rom, starts, case, replayed, out)?;
        all_ok &= tape_ok && hand_ok;
    }
    writeln!(out, "menu-test diff: {}", if all_ok { "all ok" } else { "DIFFERENCES" })?;
    Ok(all_ok)
}

/// Runs the command: prints the report and returns the exit code (0 all ok, 1 a failure).
///
/// # Errors
///
/// The start cannot be played (the save, the replay, the ROM).
pub fn run(rom: &[u8], args: &Args, out: &mut dyn std::io::Write) -> anyhow::Result<u8> {
    if args.suite == "cheats" {
        let mut text = format!("{}\n", header(args));
        let ok = crate::cheats_test::suite(rom, &args.start, &args.only, &mut text)?;
        write!(out, "{text}")?;
        return Ok(u8::from(!ok));
    }
    let starts = start_of(rom, &args.start)?;
    let mut text = format!("{}\n", header(args));
    let ok = match args.suite.as_str() {
        "field" => field_suite(rom, &starts, &args.only, &mut text)?,
        "town" => scenario_suite(rom, &starts, &TOWN_SCENARIOS, "town", &args.only, &mut text)?,
        "items" => {
            let Start::Save { path, .. } = &args.start else {
                bail!("menu-test items takes --save <file.sav>: its crafted starts come from it");
            };
            let base =
                std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
            let ok = items_suite(rom, &base, &args.only, &mut text);
            // Whatever the suite wrote is removed, whether it ran or failed.
            let _ = std::fs::remove_dir_all(scratch_dir());
            ok?
        }
        _ => diff_suite(rom, &starts, &args.only, &mut text)?,
    };
    write!(out, "{text}")?;
    Ok(u8::from(!ok))
}

/// Where the scenario starts, for the report.
#[must_use]
pub fn describe(start: &Start) -> String {
    match start {
        Start::Save { path, slot } => format!("save {} slot {}", path.display(), slot + 1),
        Start::Replay { path, frames } => format!("replay {} at frame {frames}", path.display()),
        Start::Scenarios { dir, save, .. } => format!(
            "scenarios in {}{}",
            dir.as_deref().unwrap_or(Path::new(".work/replays")).display(),
            save.as_ref().map_or_else(String::new, |(path, _)| format!(
                ", scripted loss from {}",
                path.display()
            )),
        ),
    }
}

/// The path of the cached ROM's identifier, for the report's header.
#[must_use]
pub fn header(args: &Args) -> String {
    format!("menu-test {}: from {}", args.suite, describe(&args.start))
}

// --- the bag (suite `items`): Enhanced mode on crafted saves --------------------------------------

/// The Item screen's start: the bag bound to the save with `sidecar` (as the window binds it), with
/// the bag's rules. The field is the start's; the first frame binds the bag.
fn bag_harness(rom: &[u8], starts: &Starts, sidecar: &Sidecar) -> anyhow::Result<Harness> {
    let mut h = harness_with(rom, starts, true)?;
    let catalogue = ssd_catalogue::Catalogue::from_rom(rom)?;
    h.driver = Driver::with_bag(
        ssd_catalogue::menu_labels(&catalogue),
        ssd_menus::bag::Rules::default(),
        sidecar.clone(),
    );
    Ok(h)
}

/// The save item of object `id`, with the attribute byte the game keeps for the object.
fn save_item(rom: &[u8], id: u8) -> anyhow::Result<ssd_save::Item> {
    let catalogue = ssd_catalogue::Catalogue::from_rom(rom)?;
    let item = catalogue.items.get(usize::from(id)).context("no such object")?;
    Ok(ssd_save::Item { id, attr: item.attribute })
}

/// The save `base` (a save file's bytes) with the items `ids` in slot 1, and the first party member
/// at `hp` when given. The checksum and the backup are written as the game writes them.
fn craft_save(rom: &[u8], base: &[u8], ids: &[u8], hp: Option<u16>) -> anyhow::Result<Vec<u8>> {
    let mut file = ssd_save::SaveFile::from_bytes(base)?;
    let mut slot = file.slot(0)?;
    let items = ids.iter().map(|&id| save_item(rom, id)).collect::<anyhow::Result<Vec<_>>>()?;
    slot.set_items(&items)?;
    if let (Some(hp), Some(&first)) = (hp, slot.party().first()) {
        let mut record = slot.record(first)?;
        record.set_hp(hp);
        slot.set_record(first, &record)?;
    }
    file.set_slot(0, &slot)?;
    Ok(file.as_bytes().to_vec())
}

/// The checksum word of slot 1 of a battery save (what the sidecar keys the bag by).
fn slot_checksum(sram: &[u8]) -> anyhow::Result<u16> {
    let raw: &[u8; ssd_save::SLOT_LEN] = sram
        .get(ssd_save::SLOT_BASES[0]..ssd_save::SLOT_BASES[0] + ssd_save::SLOT_LEN)
        .context("a save slot")?
        .try_into()?;
    Ok(ssd_save::checksum(raw))
}

/// The directory of the items suite's scratch files (crafted saves, the recorded replay): the
/// system's temporary directory, emptied when the suite ends.
fn scratch_dir() -> PathBuf {
    std::env::temp_dir().join(format!("ssd-menu-test-{}", std::process::id()))
}

/// A scratch save file of `bytes`, for a scenario's start.
fn scratch_save(name: &str, bytes: &[u8]) -> anyhow::Result<PathBuf> {
    let dir = scratch_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{name}.sav"));
    std::fs::write(&path, bytes)?;
    Ok(path)
}

/// The first party member's HP (the party order's first character, its record in use).
fn first_member_hp(h: &Harness) -> u16 {
    let character = h.byte(d::PARTY_ORDER);
    (0..9)
        .map(|index| d::PARTY_RECORDS + 32 * index)
        .find(|&record| h.byte(record) & 0x80 != 0 && h.byte(record + 0x1C) == character)
        .map_or(0, |record| h.word(record + 3))
}

/// The bag's stacks as (object, count) in bag order, and the original's list (its objects).
fn bag_summary(h: &Harness) -> (Vec<(u8, u16)>, Vec<u8>) {
    let stacks = h
        .driver
        .bag()
        .map_or_else(Vec::new, |bag| bag.stacks().iter().map(|s| (s.object, s.count)).collect());
    let count = usize::from(h.byte(d::ITEM_COUNT)).min(24);
    let list = (0..count).map(|i| h.byte(d::ITEM_LIST + 2 * i as u32)).collect();
    (stacks, list)
}

/// Whether the original's list is the bag's projection: the same objects, in order.
fn list_is_projection(h: &Harness) -> bool {
    let Some(bag) = h.driver.bag() else { return false };
    let objects: Vec<u8> = bag
        .project(None)
        .iter()
        .filter_map(|id| bag.stack(*id))
        .map(|stack| stack.object)
        .collect();
    bag_summary(h).1 == objects
}

/// The original's list as the game writes a pickup, a purchase or a battle drop: an entry appended
/// (object and its attribute byte), the count raised.
fn append_entry(h: &mut Harness, rom: &[u8], object: u8) -> anyhow::Result<()> {
    let count = h.byte(d::ITEM_COUNT);
    let item = save_item(rom, object)?;
    let at = d::ITEM_LIST + 2 * u32::from(count);
    h.machine.write_wram(at, &[item.id, item.attr])?;
    h.machine.write_wram(d::ITEM_COUNT, &[count + 1])?;
    Ok(())
}

/// Opens the bag: the field's Item command (row 3 of the command window), from the closed menu.
fn open_items(h: &mut Harness) -> anyhow::Result<()> {
    h.open()?;
    for _ in 0..3 {
        h.key(Key::Down)?;
    }
    h.key(Key::Confirm)?;
    h.frames(20)
}

/// The shared start of the bag's scenarios: the crafted save and its Continue, the bag bound.
fn bag_start(
    rom: &[u8],
    base: &[u8],
    name: &str,
    ids: &[u8],
    hp: Option<u16>,
) -> anyhow::Result<Starts> {
    let crafted = craft_save(rom, base, ids, hp)?;
    let path = scratch_save(name, &crafted)?;
    start_of(rom, &Start::Save { path, slot: 0 })
}

/// Use Bulma ×2 from a stack of 5: the first member heals 25 per unit (+50), the stack keeps 3, and
/// the original's list is the bag's projection.
fn bag_use(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    let hp_before = first_member_hp(h);
    open_items(h)?;
    r.check(h.driver.bag().is_some(), "the bag is bound to the save");
    r.check(h.driver.nav().screen() == ssd_menus::Screen::Items, "the HD shows the bag");
    h.key(Key::Confirm)?;
    r.check(h.driver.nav().sheet() == Sheet::Choose, "Use or Drop for the stack");
    h.key(Key::Confirm)?;
    r.check(h.driver.nav().sheet() == Sheet::Members, "the use asks for the member");
    h.key(Key::Confirm)?;
    r.check(h.driver.nav().sheet() == Sheet::Quantity, "the quantity follows");
    h.key(Key::Right)?;
    h.key(Key::Confirm)?;
    h.until_idle()?;
    h.frames(SETTLE)?;
    let (stacks, _) = bag_summary(h);
    r.check(
        h.driver.last_outcome() == Some(Outcome::Reached),
        &format!(
            "the plan: {:?} (screen {} step {} busy {})",
            h.driver.last_outcome(),
            h.byte(d::MENU_SCREEN),
            h.byte(d::MENU_STEP),
            h.byte(d::MENU_BUSY)
        ),
    );
    r.check(h.byte(d::FIELD_STATE) == 0, "the menu closed after the uses");
    let hp_after = first_member_hp(h);
    r.check(hp_after == hp_before + 50, &format!("+50 HP: {hp_before} to {hp_after}"));
    r.check(stacks.contains(&(2, 3)), &format!("a stack of 3 Bulma is left: {stacks:?}"));
    r.check(list_is_projection(h), "the original's list is the bag's projection");
    Ok(())
}

/// A pickup while 21 stacks are held lands in the bag: the original's list keeps its 21 entries and
/// the 22nd stack is in the bag.
fn bag_pickup(h: &mut Harness, r: &mut Report, rom: &[u8]) -> anyhow::Result<()> {
    h.frames(2)?;
    r.check(h.driver.bag().map_or(0, |bag| bag.stacks().len()) == 21, "21 stacks are held");
    append_entry(h, rom, 21)?;
    h.frames(2)?;
    let (stacks, list) = bag_summary(h);
    r.check(stacks.len() == 22 && stacks.last() == Some(&(21, 1)), "the pickup is in the bag");
    r.check(list.len() == 21 && !list.contains(&21), "the list still shows 21 entries");
    r.check(list_is_projection(h), "the list is the bag's projection");
    Ok(())
}

/// A purchase adds to a stack the bag holds: the money is paid, the stack grows, the list does not.
fn bag_shop(h: &mut Harness, r: &mut Report, rom: &[u8]) -> anyhow::Result<()> {
    h.frames(2)?;
    let zeni = h.word(d::ZENI);
    append_entry(h, rom, 2)?;
    h.machine.write_wram(d::ZENI, &(zeni - 3).to_le_bytes())?;
    h.frames(2)?;
    let (stacks, _) = bag_summary(h);
    r.check(stacks.contains(&(2, 2)), &format!("the bought Bulma joins its stack: {stacks:?}"));
    r.check(list_is_projection(h), "the list is the bag's projection");
    r.check(h.word(d::ZENI) == zeni - 3, "the price is paid");
    Ok(())
}

/// Drop two Bulma from the stack: the bag and the list follow, and the Scouter stays.
fn bag_drop(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    open_items(h)?;
    h.key(Key::Confirm)?;
    h.key(Key::Down)?;
    h.key(Key::Confirm)?;
    h.key(Key::Right)?;
    h.key(Key::Confirm)?;
    r.check(h.driver.nav().sheet() == Sheet::Confirm, "Drop 2? asks for a Yes");
    h.key(Key::Up)?;
    h.key(Key::Confirm)?;
    h.until_idle()?;
    h.frames(SETTLE)?;
    let (stacks, list) = bag_summary(h);
    r.check(stacks == vec![(16, 1)], &format!("the Bulma is gone, the Scouter stays: {stacks:?}"));
    r.check(list == vec![16], "the list shows the Scouter");
    r.check(h.driver.nav().screen() == ssd_menus::Screen::Items, "the Item screen is back");
    Ok(())
}

/// Save, then Continue: the bag of the save (more stacks than its list) comes back from the sidecar.
fn bag_save_continue(h: &mut Harness, r: &mut Report, rom: &[u8]) -> anyhow::Result<()> {
    h.frames(2)?;
    append_entry(h, rom, 21)?;
    h.frames(2)?;
    menu_save(h, r)?;
    let saved = h.machine.sram().context("the save")?.to_vec();
    let sidecar = h.driver.sidecar().clone();
    r.check(sidecar.saves.len() == 1, "the sidecar keeps the bag of the save");
    let starts = start_of(rom, &Start::Save { path: scratch_save("saved", &saved)?, slot: 0 })?;
    let mut again = bag_harness(rom, &starts, &sidecar)?;
    again.frames(2)?;
    let (stacks, _) = bag_summary(&again);
    r.check(stacks.len() == 22, &format!("Continue restores 22 stacks: {}", stacks.len()));
    r.check(stacks.last() == Some(&(21, 1)), "the stack that the list did not show is back");
    Ok(())
}

/// A Classic-modified save (its checksum no longer matches the sidecar's) rebuilds the bag from its
/// own list.
fn bag_classic_rebuild(
    h: &mut Harness,
    r: &mut Report,
    rom: &[u8],
    base: &[u8],
) -> anyhow::Result<()> {
    h.frames(2)?;
    append_entry(h, rom, 21)?;
    h.frames(2)?;
    menu_save(h, r)?;
    let sidecar = h.driver.sidecar().clone();
    let saved = h.machine.sram().context("the save")?.to_vec();
    let _ = base;
    // A change made by Classic play: slot 1 holds Kami and Puar instead.
    let edited = craft_save(rom, &saved, &[0, 4], None)?;
    let starts = start_of(rom, &Start::Save { path: scratch_save("classic", &edited)?, slot: 0 })?;
    let mut again = bag_harness(rom, &starts, &sidecar)?;
    again.frames(2)?;
    let (stacks, list) = bag_summary(&again);
    r.check(
        stacks == vec![(0, 1), (4, 1)],
        &format!("the bag is rebuilt from the list: {stacks:?}"),
    );
    r.check(list == vec![0, 4], "and the list is the same");
    Ok(())
}

/// A full bag (60 stacks, from the sidecar of the save) shows its notice, and the original's list is at
/// its full count of 24: the original refuses the next pickup by its own check, and nothing is added.
fn bag_full(h: &mut Harness, r: &mut Report, _rom: &[u8]) -> anyhow::Result<()> {
    h.frames(2)?;
    let stacks = h.driver.bag().map_or(0, |bag| bag.stacks().len());
    r.check(
        stacks == 60 && h.driver.bag().is_some_and(Bag::is_full),
        &format!("60 stacks: {stacks}"),
    );
    r.check(h.byte(d::ITEM_COUNT) == 24, "the original's list is full (24 entries)");
    open_items(h)?;
    let notice = h
        .driver
        .scene(h.machine.wram())
        .is_some_and(|scene| scene.texts().any(|(_, _, text)| text.starts_with("Bag full")));
    r.check(notice, "the Item screen says the bag is full");
    // The original adds nothing at 24 entries: no write is made for the refused pickup.
    h.frames(2)?;
    let (after, _) = bag_summary(h);
    r.check(after.len() == 60 && h.byte(d::ITEM_COUNT) == 24, "the 61st stack is refused");
    Ok(())
}

/// The sidecar of a full bag: 60 stacks of the Porunga (each unit its own stack), keyed by the
/// checksum of the crafted start (slot 1 with the first 21 objects).
fn full_sidecar(rom: &[u8], base: &[u8]) -> anyhow::Result<Sidecar> {
    let crafted = craft_save(rom, base, &(0..21).collect::<Vec<u8>>(), None)?;
    let mut bag = Bag::new(Rules::default());
    for _ in 0..60 {
        bag.add(15, 0, Class::of(15, false))
            .map_err(|_| anyhow::anyhow!("the bag is full too soon"))?;
    }
    let mut sidecar = Sidecar::empty();
    sidecar.store(0, slot_checksum(&crafted)?, &bag);
    Ok(sidecar)
}

/// The bag's scenarios from the base save: each builds its own crafted start.
type BagScenario = fn(&mut Harness, &mut Report, &[u8], &[u8]) -> anyhow::Result<()>;

const BAG_SCENARIOS: [(&str, BagScenario); 8] = [
    ("original-items", |h, r, _, _| original_items(h, r)),
    ("use-bulma", |h, r, _, _| bag_use(h, r)),
    ("pickup", |h, r, rom, _| bag_pickup(h, r, rom)),
    ("shop", |h, r, rom, _| bag_shop(h, r, rom)),
    ("drop", |h, r, _, _| bag_drop(h, r)),
    ("save-continue", |h, r, rom, _| bag_save_continue(h, r, rom)),
    ("classic-rebuild", |h, r, rom, base| bag_classic_rebuild(h, r, rom, base)),
    ("bag-full", |h, r, rom, _| bag_full(h, r, rom)),
];

/// The items suite: each bag scenario from its crafted start, with its checks as a report.
fn items_suite(rom: &[u8], base: &[u8], only: &[String], out: &mut String) -> anyhow::Result<bool> {
    let mut all_ok = true;
    let mut ran = 0;
    if only.is_empty() || only.iter().any(|o| o == "round-trip") {
        ran += 1;
        let ok = match round_trip(rom, base, out) {
            Ok(ok) => ok,
            Err(error) => {
                writeln!(out, "FAIL round-trip: {error:#}")?;
                false
            }
        };
        all_ok &= ok;
    }
    for &(name, scenario) in &BAG_SCENARIOS {
        if !only.is_empty() && !only.iter().any(|o| o == name) {
            continue;
        }
        ran += 1;
        let mut report = Report::default();
        let result = bag_scenario(rom, base, name, scenario, &mut report);
        if let Err(error) = &result {
            report.check(false, &format!("the scenario failed: {error:#}"));
        }
        let ok = report.passed();
        all_ok &= ok;
        writeln!(out, "{} {name}", if ok { "ok  " } else { "FAIL" })?;
        for (passed, what) in &report.lines {
            if !passed {
                writeln!(out, "       - {what}")?;
            }
        }
    }
    writeln!(
        out,
        "menu-test items: {ran} scenarios, {}",
        if all_ok { "all ok" } else { "FAILED" }
    )?;
    Ok(all_ok)
}

/// One bag scenario: its start is the base save with the items the scenario needs.
fn bag_scenario(
    rom: &[u8],
    base: &[u8],
    name: &str,
    scenario: BagScenario,
    report: &mut Report,
) -> anyhow::Result<()> {
    let ids: Vec<u8> = match name {
        "use-bulma" => vec![2, 2, 2, 2, 2],
        "original-items" => vec![21, 22, 23],
        "pickup" | "save-continue" | "classic-rebuild" | "bag-full" => (0..21).collect(),
        "shop" => (0..20).collect(),
        "drop" => vec![2, 2, 16],
        _ => Vec::new(),
    };
    let hp = (name == "use-bulma").then_some(30);
    let starts = bag_start(rom, base, name, &ids, hp)?;
    let sidecar = if name == "bag-full" { full_sidecar(rom, base)? } else { Sidecar::empty() };
    let mut h = bag_harness(rom, &starts, &sidecar)?;
    scenario(&mut h, report, rom, base)
}

/// A hash of the state the replay must reproduce: work RAM and the bag (FNV-1a, 64 bits).
fn state_hash(wram: &[u8], bag: &[u8]) -> u64 {
    wram.iter().chain(bag).fold(0xCBF2_9CE4_8422_2325_u64, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01B3)
    })
}

/// The bag's bytes as the replay keeps them (an empty list when the bag is not bound).
fn bag_bytes(driver: &Driver) -> Vec<u8> {
    driver.bag().map_or_else(Vec::new, Bag::to_bytes)
}

/// Records an Enhanced session from power-on (the crafted save in the cartridge, its Continue, the
/// bag's Use of Bulma), then plays the replay file back: the final state hash must be the same. The
/// replay is format 2: its mode and the host events of each frame.
fn round_trip(rom: &[u8], base: &[u8], out: &mut String) -> anyhow::Result<bool> {
    let crafted = craft_save(rom, base, &[2, 2, 2, 2, 2], Some(30))?;
    let catalogue = ssd_catalogue::Catalogue::from_rom(rom)?;
    let labels = ssd_catalogue::menu_labels(&catalogue);
    let mut saves = MemorySaveStore::default();
    saves.0.insert("sav".to_owned(), crafted.clone());
    let machine = Machine::new(rom.to_vec(), Box::new(saves))?;
    let recorder = Recorder::start(&machine)?.in_mode(MODE_ENHANCED);
    let mut h = Harness {
        machine,
        driver: Driver::with_bag(labels.clone(), Rules::default(), Sidecar::empty()),
        tape: None,
        recorder: Some(recorder),
    };
    let continued =
        scripts::parse_timeline(&save::continue_prefix(0)).map_err(anyhow::Error::msg)?;
    for pad in continued {
        h.frame(pad)?;
    }
    h.frames(30)?;
    open_items(&mut h)?;
    h.key(Key::Confirm)?;
    h.key(Key::Confirm)?;
    h.key(Key::Confirm)?;
    h.key(Key::Right)?;
    h.key(Key::Confirm)?;
    h.until_idle()?;
    h.frames(SETTLE)?;
    let recorded = state_hash(h.machine.wram(), &bag_bytes(&h.driver));
    let replay = h.recorder.take().context("the recording")?.into_replay();
    let events: usize = replay.events.iter().map(Vec::len).sum();
    let bytes = replay.to_bytes(CORE_VERSION);
    let path = scratch_dir().join("round-trip.ssdreplay");
    std::fs::create_dir_all(scratch_dir())?;
    std::fs::write(&path, &bytes)?;

    // The file, read back, on a machine of its own, with the bag bound by the replay's events.
    let read = Replay::from_bytes(&std::fs::read(&path)?, ssd_rom::crc32(rom), CORE_VERSION)?;
    let mut played = read.power_on(rom.to_vec())?;
    let mut driver = Driver::with_bag(labels, Rules::default(), Sidecar::empty());
    driver.set_binding(Binding::Recorded);
    for frame in 0..read.frames() {
        let _ = driver.frame(played.wram(), played.sram(), Joypad::default());
        apply_events(&mut played, read.events_at(frame))?;
        for event in read.events_at(frame) {
            if let HostEvent::App { tag, payload } = event {
                driver.replay_event(*tag, payload, played.wram()).map_err(anyhow::Error::msg)?;
            }
        }
        played.run_frame(read.input(frame), &mut NullAudio)?;
    }
    let replayed = state_hash(played.wram(), &bag_bytes(&driver));
    let same = recorded == replayed && read.mode == MODE_ENHANCED && events > 0;
    writeln!(
        out,
        "{} round trip: format 2, mode {}, {} frames, {events} host events; the final state {}",
        if same { "ok  " } else { "FAIL" },
        read.mode,
        read.frames(),
        if same { "is the same" } else { "differs" },
    )?;
    Ok(same)
}

/// Bubbles, Escape and the Dragon Radar are used through the original's own effects: each is used
/// from the bag and the menu comes back to the field. Bubbles and Escape are consumed; the Radar is
/// a keepsake (it cannot be dropped and its effect never removes its entry): it shows its own
/// screen, which only B leaves, and stays in the bag.
fn original_items(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    // The bag binds on the first frame of the start.
    h.frames(1)?;
    for name in ["Bubbles", "Escape", "D.Radar"] {
        let before = bag_summary(h).0.len();
        open_items(h)?;
        for _ in 0..3 {
            h.key(Key::Confirm)?;
        }
        if h.driver.nav().sheet() == Sheet::Members {
            h.key(Key::Confirm)?;
        }
        h.until_idle()?;
        h.frames(SETTLE)?;
        let after = bag_summary(h).0.len();
        let kept = name == "D.Radar";
        r.check(
            (if kept { after == before } else { after + 1 == before })
                && h.byte(d::FIELD_STATE) == 0
                && matches!(h.driver.last_outcome(), Some(Outcome::Reached) | None),
            &format!(
                "{name} is used and {}: {:?} (field {} screen {} step {} busy {} cursor {} row0 {:02x}/{:02x} sheet {:?} bag {:?})",
                if kept { "stays in the bag" } else { "leaves the bag" },
                h.driver.last_outcome(), h.byte(d::FIELD_STATE), h.byte(d::MENU_SCREEN), h.byte(d::MENU_STEP),
                h.byte(d::MENU_BUSY), h.byte(d::MAP_ENTRY_VALUE), h.byte(d::ITEM_LIST), h.byte(d::ITEM_LIST + 1),
                h.driver.nav().sheet(), bag_summary(h).0
            ),
        );
    }
    Ok(())
}
