//! The battle part of `hd-tour`: makes the game show a chosen enemy group against a chosen party,
//! headless, from a crafted save, and plays a scripted sequence of battle commands, so that every
//! group of the table is seen (the mechanism of the `feat/battle-gallery` branch's
//! `battle-gallery`, ported with the capture left out: the tour tallies the frames itself).
//!
//! How a battle is forced (test-only host actions of this tool; the game, the recompiled
//! routines and the Classic or Enhanced runs never see them):
//! - the party and its levels come from `save-edit` fields applied to a copy of a save, played
//!   from power-on through Continue to the map (a save state of that moment starts every episode);
//! - the party walks on the map while the host keeps the zone's encounter level at its top
//!   (`$0D8B`, `$0D8A`), so the next roll of `02:D1F6` starts a battle;
//! - a breakpoint at the group set-up `01:F692` writes the wanted group into
//!   `encounter_group_index` (`$0EA3`) before the routine reads it, once per episode (the way
//!   the M5 research proved, `docs/re/data/battle.md`; a later form change of the same battle
//!   sets up its own group);
//! - the battle is driven by a seeded pad script: in the command phase (`game_mode` 2) a
//!   weighted random choice of A, directions and B (so every command, card and target comes
//!   up), in the animation phases A pulses to move the messages on; between rounds the host
//!   keeps the party healthy and its KI full (a phase of the plan lets one member fall) and the
//!   enemies near their strength, and at the end of the plan it brings the enemies to 1 HP so
//!   the party defeats them.

use anyhow::{Context, bail};
use ssd_game::scripts;
use ssd_oracle::{Joypad, Machine, MemorySaveStore, NullAudio, Stopped, Stops};

/// The group set-up routine whose entry reads `encounter_group_index`.
const GROUP_SETUP: u32 = 0x01_F692;
const W_GROUP: usize = 0x0EA3;
const W_LEVEL: usize = 0x0D8A;
const W_ZONE: usize = 0x0D8B;
const W_MODE: usize = 0x24;
const W_IN_BATTLE: usize = 0x364;
const W_SCREEN: usize = 0x0D5F;
const W_CURSOR: usize = 0x0D62;
const PARTY: usize = 0x0B00;
const ENEMIES: usize = 0x0E00;

/// One machine from power-on to the map of the crafted save, with the host actions.
pub struct Session {
    pub machine: Machine,
    /// The group the next group set-up gets (cleared by the first one).
    pub group: Option<u8>,
    pub force_encounters: bool,
}

impl Session {
    /// Power on with `sram` as the battery save and play Continue on slot 1.
    ///
    /// # Errors
    ///
    /// The machine cannot be built or run.
    pub fn boot(rom: &[u8], sram: &[u8]) -> anyhow::Result<Self> {
        let mut saves = MemorySaveStore::default();
        saves.0.insert("sav".to_owned(), sram.to_vec());
        let mut machine = Machine::new(rom.to_vec(), Box::new(saves))?;
        machine.set_breakpoint(GROUP_SETUP);
        machine.set_line_log(true);
        let mut session = Self { machine, group: None, force_encounters: false };
        let pads = scripts::parse_timeline(&crate::save::continue_prefix(0))
            .map_err(|e| anyhow::anyhow!("continue prefix: {e}"))?;
        for pad in pads {
            session.step(pad)?;
        }
        Ok(session)
    }

    /// One frame with `pad`. The host actions run at the frame's boundaries and at the group
    /// set-up.
    ///
    /// # Errors
    ///
    /// The core fails.
    pub fn step(&mut self, pad: Joypad) -> anyhow::Result<()> {
        if self.force_encounters && self.mode() == 0 {
            self.poke(W_ZONE, &[1])?;
            self.poke(W_LEVEL, &[16])?;
        }
        let stops = Stops { frame_end: true, breakpoint: true, ..Stops::default() };
        loop {
            match self.machine.run(&stops, &mut |_| pad, &mut NullAudio, u64::MAX)? {
                Stopped::FrameEnd => return Ok(()),
                Stopped::Breakpoint { .. } => {
                    if let Some(group) = self.group.take() {
                        self.poke(W_GROUP, &[group])?;
                    }
                }
                other => bail!("unexpected stop {other:?}"),
            }
        }
    }

    /// Writes WRAM bytes (a test-only host action).
    ///
    /// # Errors
    ///
    /// The bytes are not in work RAM.
    pub fn poke(&mut self, offset: usize, bytes: &[u8]) -> anyhow::Result<()> {
        self.machine.write_wram(0x7E_0000 + offset as u32, bytes)?;
        Ok(())
    }

    #[must_use]
    pub fn wram(&self, offset: usize) -> u8 {
        self.machine.wram()[offset]
    }

    #[must_use]
    pub fn wram16(&self, offset: usize) -> u16 {
        u16::from_le_bytes([self.wram(offset), self.wram(offset + 1)])
    }

    /// `game_mode` without its "just entered" bit.
    #[must_use]
    pub fn mode(&self) -> u8 {
        self.wram(W_MODE) & 0x7F
    }

    /// Whether the battle code is running (`in_battle`).
    #[must_use]
    pub fn in_battle(&self) -> bool {
        self.wram(W_IN_BATTLE) != 0
    }

    /// Walks on the map until a battle starts; returns the frames it took.
    ///
    /// # Errors
    ///
    /// The core fails, or no battle starts within 1 500 frames.
    pub fn walk_into_battle(&mut self) -> anyhow::Result<u64> {
        self.force_encounters = true;
        for frame in 0..1500_u64 {
            let right = (frame / 24) % 2 == 0;
            self.step(Joypad { right, left: !right, ..Joypad::default() })?;
            if matches!(self.mode(), 2 | 4) {
                self.force_encounters = false;
                return Ok(frame + 1);
            }
        }
        bail!("no battle within 1500 frames (mode {})", self.mode())
    }
}

/// The save bytes of the crafted slot 1: `sets` applied to a copy of the save `bytes`.
///
/// # Errors
///
/// The save or a field is bad.
pub fn crafted_bytes(
    game: &crate::save::Game,
    bytes: &[u8],
    sets: &[(String, String)],
) -> anyhow::Result<Vec<u8>> {
    let mut file = ssd_save::SaveFile::from_bytes(bytes)?;
    let mut slot = file.slot(0)?;
    for (field, value) in sets {
        crate::save::apply(&mut slot, Some(game), field, value)
            .with_context(|| format!("--set {field}={value}"))?;
    }
    file.set_slot(0, &slot)?;
    Ok(file.as_bytes().to_vec())
}

// ---------------------------------------------------------------------------------------------
// The pad script and the host's upkeep of a battle

/// xorshift32.
struct Rng(u32);

impl Rng {
    fn next(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    fn below(&mut self, n: u32) -> u32 {
        self.next() % n
    }
}

/// The weighted pad script of the command phase.
pub struct Fuzz {
    rng: Rng,
    key: Joypad,
}

impl Fuzz {
    #[must_use]
    pub fn new(seed: u32) -> Self {
        Self { rng: Rng(seed.max(1)), key: Joypad::default() }
    }

    /// The pad of frame `t` of the episode: `mode` and the menu screen and cursor decide.
    pub fn pad(&mut self, t: u64, mode: u8, screen: u8, cursor: u8) -> Joypad {
        if mode != 2 {
            // Animations and messages: A moves them on.
            return Joypad { a: t % 12 < 2, ..Joypad::default() };
        }
        match t % 8 {
            0 => {
                let mut k = Joypad::default();
                match self.rng.below(100) {
                    0..=59 => k.a = true,
                    60..=71 => k.down = true,
                    72..=79 => k.up = true,
                    80..=83 => k.left = true,
                    84..=87 => k.right = true,
                    88..=91 => k.b = true,
                    _ => {}
                }
                // The first window's Run and Menu end or leave the battle: back up from them.
                if screen == 0x11 && cursor >= 2 {
                    k = Joypad { up: true, ..Joypad::default() };
                }
                self.key = k;
                k
            }
            1 => self.key,
            _ => Joypad::default(),
        }
    }
}

/// What the host does to the records between rounds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    /// Party healthy, enemies near full strength: every command gets played.
    Play,
    /// One party member at a time is left at 1 HP, so an enemy's hit can defeat it.
    LetFall,
    /// The enemies at 1 HP: the party defeats them.
    Finish,
}

fn bp24(s: &Session, record: usize) -> u32 {
    u32::from(s.wram(record + 7))
        | u32::from(s.wram(record + 8)) << 8
        | u32::from(s.wram(record + 9)) << 16
}

/// Between rounds (the command phase): the host's upkeep of the records.
fn upkeep(s: &mut Session, phase: Phase, round: u32, match_bp: bool) -> anyhow::Result<()> {
    let alive = |s: &Session, r: usize| s.wram(r) & 0x40 != 0 && s.wram16(r + 3) > 0;
    let party: Vec<usize> = (0..9).map(|i| PARTY + 0x20 * i).filter(|&r| alive(s, r)).collect();
    let enemies: Vec<usize> = (0..5).map(|i| ENEMIES + 0x20 * i).filter(|&r| alive(s, r)).collect();
    let victim = if party.is_empty() { 0 } else { round as usize % party.len() };
    let enemy_bp = enemies.iter().map(|&r| bp24(s, r)).max().unwrap_or(0);
    for (n, &r) in party.iter().enumerate() {
        let max_hp = s.wram16(r + 0x0A);
        let max_ki = s.wram16(r + 0x0C);
        let hp = if phase == Phase::LetFall && n == victim { 1 } else { max_hp };
        s.poke(r + 3, &hp.to_le_bytes())?;
        s.poke(r + 5, &max_ki.to_le_bytes())?;
        if match_bp && enemy_bp > 0 {
            s.poke(r + 7, &enemy_bp.min(0x00FF_FFFF).to_le_bytes()[..3])?;
        }
    }
    for &r in &enemies {
        let max_hp = s.wram16(r + 0x0A);
        let hp = s.wram16(r + 3);
        match phase {
            Phase::Finish => s.poke(r + 3, &1_u16.to_le_bytes())?,
            _ if hp < max_hp / 2 => s.poke(r + 3, &max_hp.to_le_bytes())?,
            _ => {}
        }
    }
    Ok(())
}

/// Whether the game is in a battle (`game_mode` of the command screen, an attack animation or the
/// last fights).
#[must_use]
pub fn is_battle(s: &Session) -> bool {
    matches!(s.mode(), 2 | 4 | 0x0C)
}

/// Plays a battle that a visit does not want for its own sake (the fight a map starts when it
/// loads): the pad script of the battles, and at each command phase the party healed and the
/// enemies left at 1 HP, so that the party wins.
pub struct Finisher {
    fuzz: Fuzz,
    last: Option<u64>,
    rounds: u32,
}

impl Finisher {
    #[must_use]
    pub fn new(seed: u32) -> Self {
        Self { fuzz: Fuzz::new(seed), last: None, rounds: 0 }
    }

    /// The pad of frame `t` of the battle (the host's upkeep of the records is done here).
    ///
    /// # Errors
    ///
    /// The records cannot be written.
    pub fn pad(&mut self, s: &mut Session, t: u64) -> anyhow::Result<Joypad> {
        if s.mode() == 2
            && s.in_battle()
            && s.wram(W_SCREEN) == 0x11
            && self.last.is_none_or(|last| t >= last + 120)
        {
            self.last = Some(t);
            self.rounds += 1;
            upkeep(s, Phase::Finish, self.rounds, true)?;
        }
        Ok(self.fuzz.pad(t, s.mode(), s.wram(W_SCREEN), s.wram(W_CURSOR)))
    }
}

// ---------------------------------------------------------------------------------------------
// The scenarios

/// One battle: a group against the party of its scenario.
#[derive(Debug, Clone)]
pub struct Episode {
    pub group: u8,
    /// Frames of the command and animation phases before the finish.
    pub frames: u64,
    /// Whether to let party members fall in the last third of the play.
    pub fall: bool,
    pub seed: u32,
}

/// A party (crafted save) and the battles it fights.
#[derive(Debug, Clone)]
pub struct Scenario {
    pub name: String,
    /// `--set` fields of the crafted save.
    pub sets: Vec<(String, String)>,
    /// Bytes written into WRAM on the map before the battle (`offset`, bytes): a fighter's
    /// unit id, for the forms of the story the save cannot hold.
    pub pokes: Vec<(usize, Vec<u8>)>,
    /// Whether the party's BP is set to the strongest enemy's at each round (else the save's).
    pub match_bp: bool,
    pub episodes: Vec<Episode>,
}

/// Plays one episode from the snapshot of the map; `frame` is called after every frame the battle
/// code is running (`t` counts the frames of the episode). Says how it ended.
///
/// # Errors
///
/// The core fails, no battle starts, or `frame` fails.
pub fn run_episode(
    s: &mut Session,
    snapshot: &[u8],
    scenario: &Scenario,
    ep: &Episode,
    frame: &mut dyn FnMut(&Session, u64) -> anyhow::Result<()>,
) -> anyhow::Result<String> {
    s.machine.load_state(snapshot)?;
    s.machine.set_line_log(true);
    s.force_encounters = false;
    for (at, bytes) in &scenario.pokes {
        s.poke(*at, bytes)?;
    }
    s.group = Some(ep.group);
    let took = s.walk_into_battle()?;
    let mut fuzz = Fuzz::new(ep.seed);
    let mut round = 0_u32;
    let mut last_round = u64::MAX;
    let mut idle = 0_u32;
    let mut result = String::from("plan used up");
    for t in 0..ep.frames + 3000 {
        let phase = if t >= ep.frames {
            Phase::Finish
        } else if ep.fall && t >= ep.frames * 2 / 3 {
            Phase::LetFall
        } else {
            Phase::Play
        };
        if s.mode() == 2
            && s.in_battle()
            && s.wram(W_SCREEN) == 0x11
            && (last_round == u64::MAX || t >= last_round + 120)
        {
            last_round = t;
            round += 1;
            upkeep(s, phase, round, scenario.match_bp)?;
        }
        let pad = fuzz.pad(t, s.mode(), s.wram(W_SCREEN), s.wram(W_CURSOR));
        s.step(pad)?;
        if s.in_battle() {
            frame(s, t)?;
        }
        if s.in_battle() || matches!(s.mode(), 2 | 4 | 0x0C) {
            idle = 0;
        } else {
            idle += 1;
            if idle > 120 {
                result = format!("battle over at {t} (mode {})", s.mode());
                break;
            }
        }
    }
    Ok(format!("{result}, {round} rounds, battle entered after {took} frames"))
}

// ---------------------------------------------------------------------------------------------
// The plan

/// `party_unit_ids` of the characters, by character id, and the other rows of the unit table a
/// character fights as in the story (more commands: the techniques of the later chapters).
const PARTY_FORMS: &[(&str, &[u8])] = &[
    ("goku", &[0, 57, 58, 59, 60]),
    ("gohan", &[9, 16, 63, 66, 76, 77, 102]),
    ("piccolo", &[1, 61, 62, 101]),
    ("krillin", &[10, 65, 67]),
    ("tien", &[11, 68, 71]),
    ("yamcha", &[12, 69, 72]),
    ("chiaotzu", &[13, 70, 73]),
    ("vegeta", &[80, 17, 74, 75, 79]),
];

/// Characters of the party, in the order of the plan.
const CHARACTERS: &[&str] = &[
    "goku", "gohan", "piccolo", "krillin", "tien", "yamcha", "chiaotzu", "vegeta", "nail", "dende",
    "tsumuri", "maima",
];

/// The groups the party scenarios fight (a lone strong fighter; several mixed enemies).
const SPARRING: &[u8] = &[20, 77, 36, 56, 83, 21];

fn set(f: &str, v: impl Into<String>) -> (String, String) {
    (f.to_owned(), v.into())
}

/// A party of one at a level that keeps up with the sparring groups.
fn lone(character: &str) -> Vec<(String, String)> {
    let mut sets = vec![set("party", character)];
    // Dende, Nail, Tsumuri and Maima have no level table
    if !matches!(character, "dende" | "nail" | "tsumuri" | "maima") {
        sets.push(set(&format!("{character}.level"), "30"));
    }
    sets
}

/// The scenarios of the default plan: every party fighter (and the forms the story gives it)
/// against the sparring groups, then every group of `groups` against Goku and Piccolo, twelve
/// to a scenario. `frames` is the length of a battle's play.
#[must_use]
pub fn default_plan(groups: &[u8], frames: u64, seed0: u32) -> Vec<Scenario> {
    let mut plan = Vec::new();
    let mut seed = seed0;
    let mut next_seed = || {
        seed = seed.wrapping_mul(2_654_435_761).wrapping_add(12_345);
        seed
    };
    for &name in CHARACTERS {
        let forms = PARTY_FORMS.iter().find(|(n, _)| *n == name).map(|(_, f)| *f);
        let episodes = SPARRING
            .iter()
            .enumerate()
            .map(|(i, &group)| Episode {
                group,
                frames: frames * 3 / 2,
                fall: i % 2 == 0,
                seed: next_seed(),
            })
            .collect();
        plan.push(Scenario {
            name: format!("party-{name}"),
            sets: lone(name),
            pokes: Vec::new(),
            match_bp: true,
            episodes,
        });
        for &unit in forms.unwrap_or(&[]).iter().skip(1) {
            plan.push(Scenario {
                name: format!("party-{name}-unit{unit}"),
                sets: lone(name),
                pokes: vec![(0x0B01, vec![unit])],
                match_bp: true,
                episodes: vec![Episode {
                    group: SPARRING[0],
                    frames,
                    fall: false,
                    seed: next_seed(),
                }],
            });
        }
    }
    let episodes: Vec<Episode> = groups
        .iter()
        .map(|&group| Episode { group, frames, fall: group % 3 == 0, seed: next_seed() })
        .collect();
    for (n, chunk) in episodes.chunks(12).enumerate() {
        plan.push(Scenario {
            name: format!("groups-{n:02}"),
            sets: vec![
                set("party", "goku,piccolo"),
                set("goku.level", "30"),
                set("piccolo.level", "30"),
            ],
            pokes: Vec::new(),
            match_bp: true,
            episodes: chunk.to_vec(),
        });
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pad_script_plays_a_command_phase_and_pulses_a_in_animations() {
        let mut fuzz = Fuzz::new(7);
        // Animations: A for two frames in twelve
        let animation: Vec<bool> = (0..24).map(|t| fuzz.pad(t, 4, 0, 0).a).collect();
        assert_eq!(animation.iter().filter(|&&a| a).count(), 4);
        // The command phase: a key on frame 0 of eight, held on the next, released after
        let mut fuzz = Fuzz::new(7);
        let mut presses = 0;
        for t in 0..800 {
            let pad = fuzz.pad(t, 2, 0, 0);
            let any = pad.a || pad.b || pad.up || pad.down || pad.left || pad.right;
            if t % 8 >= 2 {
                assert!(!any, "released at {t}");
            }
            presses += u32::from(any && t % 8 == 0);
        }
        assert!(presses > 40, "{presses} presses");
    }

    #[test]
    fn a_finisher_without_a_battle_plays_the_pad_script_alone() {
        // No machine here: a finisher's pad is the script's when no command phase is on
        let mut fuzz = Fuzz::new(9);
        let mut expected = Vec::new();
        for t in 0..40 {
            expected.push(fuzz.pad(t, 4, 0, 0));
        }
        assert!(expected.iter().any(|pad| pad.a));
    }

    #[test]
    fn the_first_window_backs_up_from_run_and_menu() {
        let mut fuzz = Fuzz::new(3);
        for t in (0..400).step_by(8) {
            let pad = fuzz.pad(t, 2, 0x11, 3);
            assert!(pad.up && !pad.a && !pad.down, "frame {t}");
        }
    }

    #[test]
    fn the_same_seed_plays_the_same_battle() {
        let play = |seed| {
            let mut fuzz = Fuzz::new(seed);
            (0..200).map(|t| fuzz.pad(t, 2, 0x12, 0)).collect::<Vec<_>>()
        };
        assert_eq!(play(5), play(5));
        assert_ne!(play(5), play(6));
    }

    #[test]
    fn a_plan_has_a_scenario_for_each_party_fighter_and_form() {
        let plan = default_plan(&[3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15], 100, 1);
        let names: Vec<&str> = plan.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"party-goku") && names.contains(&"party-goku-unit57"));
        assert!(names.contains(&"party-maima"));
        let goku = plan.iter().find(|s| s.name == "party-goku").unwrap();
        assert_eq!(goku.episodes.len(), SPARRING.len());
        assert_eq!(goku.episodes[0].frames, 150);
        assert!(plan.iter().all(|s| !s.episodes.is_empty()));
        // Thirteen groups make a scenario of twelve and one of one
        let tail: Vec<usize> = plan
            .iter()
            .filter(|s| s.name.starts_with("groups"))
            .map(|s| s.episodes.len())
            .collect();
        assert_eq!(tail, vec![12, 1]);
        let again = default_plan(&[3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15], 100, 1);
        assert_eq!(plan[0].episodes[1].seed, again[0].episodes[1].seed);
    }
}
