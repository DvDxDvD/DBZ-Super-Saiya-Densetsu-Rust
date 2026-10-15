//! The parts of `hd-tour` that are played by script: the maps (the camera moved over a grid of
//! places on each one), the menus, the title and the intro.

use crate::tour_battle::{Finisher, Session, crafted_bytes, is_battle};
use crate::tour_run::{Env, PartOutput, Recorder};
use ssd_oracle::Joypad;

/// The version of the map grid and of the party each map is visited with.
pub const GRID_VERSION: u32 = 4;
pub const MENUS_VERSION: u32 = 1;
pub const TITLE_VERSION: u32 = 1;

const LEADER_X: usize = 0x1A2;
const LEADER_Y: usize = 0x1A4;
const CAMERA_X: usize = 0x33;
const CAMERA_Y: usize = 0x35;
const BLOCKS_HIGH: usize = 0x702;
const BLOCKS_WIDE: usize = 0x703;

fn set(field: &str, value: impl Into<String>) -> (String, String) {
    (field.to_owned(), value.into())
}

/// The parties the maps are visited with, in turn: the party sprites and the portraits of the
/// story's characters show on the maps they walk.
const PARTIES: [&str; 6] = [
    "goku,piccolo",
    "gohan,krillin,piccolo",
    "vegeta,gohan,krillin",
    "yamcha,tien,chiaotzu",
    "goku,gohan,krillin,vegeta",
    "piccolo,gohan,dende",
];

/// The edits of the base save that put the party of map `id` on that map.
pub(crate) fn map_sets(id: u8) -> Vec<(String, String)> {
    map_sets_with(id, None)
}

/// [`map_sets`] with the party `party` (`goku,piccolo`) instead of the map's own.
pub(crate) fn map_sets_with(id: u8, party: Option<&str>) -> Vec<(String, String)> {
    let party = party.unwrap_or(PARTIES[usize::from(id) % PARTIES.len()]);
    let mut sets = vec![set("party", party)];
    for who in party.split(',') {
        if who != "dende" {
            sets.push(set(&format!("{who}.level"), "20"));
        }
    }
    sets.push(set("flags", "none"));
    sets.push(set("scene", "0"));
    sets.push(set("map", id.to_string()));
    // The leader is where the camera of a window at (32, 40) centres it
    sets.push(set("leader", "160,160"));
    sets.push(set("follower", "32,40"));
    sets
}

/// The distance between the places of a map's grid, a little less than the wide view (372 x 239
/// pixels) so the views overlap.
const STEP_X: u32 = 340;
const STEP_Y: u32 = 200;
/// Frames counted at each place (the map's animations move), after the frames the game needs to
/// draw the new place.
const BURST: u32 = 2;
const SETTLE: u32 = 3;
/// How many scenes a map may start that do not end before its visit stops (each time the free
/// state is loaded again and the visit goes on at the next place), and the frames a scene may take
/// before it is left; the first frames of a scene are counted.
const MAX_STUCK: u32 = 12;
const SCENE_FRAMES: u64 = 300;
const SCENE_LIMIT: u64 = 1500;
/// The frames a map may take from the load to the party being free (the fight some maps start).
const BOOT_LIMIT: u64 = 5000;

/// One place of a map's visit: the camera, the top left of the original's window.
pub type Place = (u16, u16);

/// The blocks of the loaded map, wide and high.
pub(crate) fn blocks_of(session: &Session) -> (u8, u8) {
    (session.wram(BLOCKS_WIDE), session.wram(BLOCKS_HIGH))
}

/// The positions from 0 to `last`, evenly spaced, at most `step` apart.
fn spread(last: u32, step: u32) -> Vec<u32> {
    if last == 0 {
        return vec![0];
    }
    let parts = last.div_ceil(step);
    (0..=parts).map(|n| last * n / parts).collect()
}

/// The places to see a map of `blocks` blocks of 256 pixels (wide, high) from: a grid, row by row
/// from the top left to the last position the game's camera can take (`(blocks - 1) * 256`: it is
/// clamped to the map). The wide views (the window and its margins) of the places cover the whole
/// map, and the outer places are the map's corners and edges.
#[must_use]
pub fn places(blocks: (u8, u8)) -> Vec<Place> {
    let last_x = u32::from(blocks.0.max(1) - 1) * 256;
    let last_y = u32::from(blocks.1.max(1) - 1) * 256;
    let columns = spread(last_x, STEP_X);
    spread(last_y, STEP_Y)
        .into_iter()
        .flat_map(|y| columns.iter().map(move |&x| (x as u16, y as u16)))
        .collect()
}

/// Moves the camera and the party: the camera to `(x, y)` (the top left of the window), the leader
/// to the middle of it. The map object table is drawn from the camera each frame, so the sprites
/// of the new place appear; BG2's tile columns do not (the game streams them in as the camera
/// moves), which the wide renderer does not read (`set_trust_field`).
pub(crate) fn place(session: &mut Session, (x, y): Place) -> anyhow::Result<()> {
    session.poke(LEADER_X, &(x + 128).to_le_bytes())?;
    session.poke(LEADER_Y, &(y + 120).to_le_bytes())?;
    session.poke(CAMERA_X, &x.to_le_bytes())?;
    session.poke(CAMERA_Y, &y.to_le_bytes())?;
    Ok(())
}

/// Whether the game is on the field with the party free to walk.
pub(crate) fn is_free(session: &Session) -> bool {
    let wram = session.machine.wram();
    matches!(session.mode(), 0 | 6) && wram[BLOCKS_WIDE] != 0 && wram[0x25] == 0
}

/// Boots map `id` from a crafted save and plays on, answering what is on the screen, until the
/// party is free on the field (menus closed); `None` when it never is. No frame is drawn.
///
/// # Errors
///
/// The save or the game fails.
pub(crate) fn boot_free(
    rom: &[u8],
    game: &crate::save::Game,
    base_save: &[u8],
    id: u8,
    camera: Option<Place>,
) -> anyhow::Result<Option<Session>> {
    boot_free_with(rom, game, base_save, id, camera, None)
}

/// [`boot_free`] with the party `party` instead of the map's own.
pub(crate) fn boot_free_with(
    rom: &[u8],
    game: &crate::save::Game,
    base_save: &[u8],
    id: u8,
    camera: Option<Place>,
    party: Option<&str>,
) -> anyhow::Result<Option<Session>> {
    let mut sets = map_sets_with(id, party);
    if let Some((x, y)) = camera {
        // The party on the map where the camera of a window at `(x, y)` centres it
        sets.retain(|(field, _)| field != "leader" && field != "follower");
        sets.push(set("leader", format!("{},{}", u32::from(x) + 128, u32::from(y) + 120)));
        sets.push(set("follower", format!("{x},{y}")));
    }
    let sram = crafted_bytes(game, base_save, &sets)?;
    let mut session = Session::boot(rom, &sram)?;
    let mut finisher = Finisher::new(7);
    let mut free = false;
    for t in 0..BOOT_LIMIT {
        if is_free(&session) && t > 0 {
            free = true;
            break;
        }
        let pad = if is_battle(&session) {
            finisher.pad(&mut session, t)?
        } else {
            Joypad { a: t % 24 < 2, b: t % 24 == 12, ..Joypad::default() }
        };
        session.step(pad)?;
    }
    if !free {
        return Ok(None);
    }
    for t in 0..60 {
        session.step(Joypad { b: t % 10 < 3, ..Joypad::default() })?;
    }
    Ok(Some(session))
}

/// Plays up to `limit` frames, answering what is on the screen with A and B, until the party is
/// free; every frame is offered to the recorder (what the player sees between scenes counts).
fn until_free(
    session: &mut Session,
    recorder: &mut Recorder<'_>,
    frame: &mut u64,
    limit: u64,
) -> anyhow::Result<bool> {
    recorder.trust_field(false);
    let mut finisher = Finisher::new(7);
    for t in 0..limit {
        if is_free(session) && t > 0 {
            return Ok(true);
        }
        // A fight is played to its end; a message is answered with A and B
        let pad = if is_battle(session) {
            finisher.pad(session, t)?
        } else {
            Joypad { a: t % 24 < 2, b: t % 24 == 12, ..Joypad::default() }
        };
        session.step(pad)?;
        *frame += 1;
        // The first frames of a scene are what it shows; a scene that goes on is not counted again
        if t < SCENE_FRAMES {
            recorder.feed(&session.machine, *frame);
        } else {
            recorder.skip(&session.machine);
        }
    }
    Ok(is_free(session))
}

/// Presses B for a while: it closes the menu the A of a scene's answers may have opened.
fn close_menus(session: &mut Session, recorder: &mut Recorder<'_>) -> anyhow::Result<()> {
    for t in 0..60 {
        session.step(Joypad { b: t % 10 < 3, ..Joypad::default() })?;
        recorder.skip(&session.machine);
    }
    Ok(())
}

/// Visits map `id`: boots the game on it with a crafted save, gets to the field (answering the
/// messages and fights of the map's own scene), then moves the camera and the party over the
/// grid of places of the map and counts what the game draws at each.
///
/// # Errors
///
/// The save or the game fails.
pub fn play_map(env: &Env<'_>, name: &str, id: u8) -> anyhow::Result<PartOutput> {
    let sram = crafted_bytes(env.game, &env.base_save, &map_sets(id))?;
    let mut session = Session::boot(env.rom, &sram)?;
    let mut recorder = Recorder::new(env, name);
    let mut notes = Vec::new();
    let mut frame = 0_u64;
    if !until_free(&mut session, &mut recorder, &mut frame, BOOT_LIMIT)? {
        notes.push(format!(
            "the party was not free after {BOOT_LIMIT} frames (mode {:02X})",
            session.mode()
        ));
        return Ok(recorder.finish(notes));
    }
    close_menus(&mut session, &mut recorder)?;
    let blocks = (session.wram(BLOCKS_WIDE), session.wram(BLOCKS_HIGH));
    let grid = places(blocks);
    notes.push(format!("{} x {} blocks, {} places", blocks.0, blocks.1, grid.len()));
    // The free state: the party standing on the field, to go back to when a scene does not end
    let free_state = session.machine.save_state();
    let (mut stuck, mut scenes, mut visited) = (0_u32, 0_u32, 0_usize);
    for &at in &grid {
        recorder.trust_field(true);
        let mut seen = true;
        for t in 0..SETTLE + BURST {
            place(&mut session, at)?;
            session.step(Joypad::default())?;
            frame += 1;
            if !matches!(session.mode(), 0 | 6) {
                seen = false;
                break;
            }
            if t < SETTLE {
                recorder.skip(&session.machine);
            } else {
                recorder.count(&session.machine, frame);
            }
        }
        if seen {
            visited += 1;
            continue;
        }
        // A scene took over (a trigger, an encounter): see it through, or leave it
        scenes += 1;
        let before = frame;
        if until_free(&mut session, &mut recorder, &mut frame, SCENE_LIMIT)? {
            notes.push(format!("a scene of {} frames at ({}, {})", frame - before, at.0, at.1));
            close_menus(&mut session, &mut recorder)?;
        } else {
            stuck += 1;
            notes.push(format!(
                "a scene that does not end (mode {:02X}) at ({}, {}): the free state again",
                session.mode(),
                at.0,
                at.1
            ));
            session.machine.load_state(&free_state)?;
            session.machine.set_line_log(true);
            if stuck > MAX_STUCK {
                notes.push("too many scenes that do not end: the visit stops".to_owned());
                break;
            }
        }
        recorder.reset();
    }
    notes.push(format!("{visited} of {} places seen, {scenes} scenes", grid.len()));
    Ok(recorder.finish(notes))
}

/// The map the walk cycles are seen on, and the frames each direction is held (a few steps of
/// every frame of the cycle) and the party rests between them.
const WALK_MAP: u8 = 0;
const WALK_FRAMES: u64 = 90;
const WALK_REST: u64 = 20;

/// The party of the walk gallery of `who`: the character leads, the others follow.
fn walk_party(who: &str) -> String {
    let mut party = vec![who];
    for other in ["goku", "gohan", "krillin", "piccolo", "vegeta"] {
        if other != who && party.len() < 3 {
            party.push(other);
        }
    }
    party.join(",")
}

/// A gallery of the walk cycles of `who`: the party on a map with `who` leading, walking down,
/// right, up and left, every frame offered to the recorder.
///
/// # Errors
///
/// The save or the game fails.
pub fn play_walk(env: &Env<'_>, name: &str, who: &str) -> anyhow::Result<PartOutput> {
    let mut sets = map_sets(WALK_MAP);
    sets.retain(|(field, _)| field != "party");
    sets.push(set("party", walk_party(who)));
    let sram = crafted_bytes(env.game, &env.base_save, &sets)?;
    let mut session = Session::boot(env.rom, &sram)?;
    let mut recorder = Recorder::new(env, name);
    let mut frame = 0_u64;
    recorder.trust_field(false);
    let mut notes = Vec::new();
    if !until_free(&mut session, &mut recorder, &mut frame, BOOT_LIMIT)? {
        notes.push(format!("the party was not free (mode {:02X})", session.mode()));
        return Ok(recorder.finish(notes));
    }
    close_menus(&mut session, &mut recorder)?;
    for (direction, pad) in [
        ("down", Joypad { down: true, ..Joypad::default() }),
        ("right", Joypad { right: true, ..Joypad::default() }),
        ("up", Joypad { up: true, ..Joypad::default() }),
        ("left", Joypad { left: true, ..Joypad::default() }),
    ] {
        for t in 0..WALK_FRAMES + WALK_REST {
            session.step(if t < WALK_FRAMES { pad } else { Joypad::default() })?;
            frame += 1;
            if !matches!(session.mode(), 0 | 6) {
                notes.push(format!(
                    "walking {direction}: a scene took over (mode {:02X})",
                    session.mode()
                ));
                break;
            }
            recorder.feed(&session.machine, frame);
        }
    }
    Ok(recorder.finish(notes))
}

/// The input of the menu walk: Talk, Look, Item, the Menu list (Cards, Status, Order, Text speed,
/// Save) and a flight. Only key presses.
const MENUS_TIMELINE: &str = include_str!("tour_menus.txt");

/// The screens of the map menus the walk must reach (`menu_screen`): the commands, the item list,
/// the Menu list, Cards, Status, Order, the text speed and Save.
const MENU_SCREENS: [u8; 8] = [0, 2, 4, 6, 7, 8, 9, 10];

/// The edits of the base save the menus are walked with: a party of three, four objects and
/// some money.
fn menu_sets() -> Vec<(String, String)> {
    vec![
        set("party", "goku,gohan,piccolo"),
        set("goku.level", "20"),
        set("gohan.level", "20"),
        set("piccolo.level", "20"),
        set("items", "turtle,d.radar,scouter,bulma"),
        set("zeni", "300"),
        set("flags", "none"),
        set("scene", "0"),
    ]
}

/// Walks through the map menus from a crafted save and counts what they show; the notes say which
/// of the menu screens the walk reached.
///
/// # Errors
///
/// The save or the game fails.
pub fn play_menus(env: &Env<'_>, name: &str) -> anyhow::Result<PartOutput> {
    let sram = crafted_bytes(env.game, &env.base_save, &menu_sets())?;
    let mut session = Session::boot(env.rom, &sram)?;
    let pads = ssd_game::scripts::parse_timeline(MENUS_TIMELINE)
        .map_err(|message| anyhow::anyhow!("the menu walk: {message}"))?;
    let mut recorder = Recorder::new(env, name);
    let mut screens = std::collections::BTreeSet::new();
    for (frame, pad) in pads.into_iter().enumerate() {
        session.step(pad)?;
        recorder.feed(&session.machine, frame as u64 + 1);
        // `menu_screen` is not cleared when a menu closes: it counts while a menu is open (the
        // field's state is not free), or on the status page (its own game mode)
        if session.wram(0x25) != 0 || session.mode() == 3 {
            screens.insert(session.wram(0x0D5F));
        }
    }
    let mut notes = vec![format!("menu screens reached: {screens:?}")];
    let missing: Vec<u8> =
        MENU_SCREENS.iter().copied().filter(|screen| !screens.contains(screen)).collect();
    if !missing.is_empty() {
        notes.push(format!("menu screens NOT reached: {missing:?}"));
    }
    Ok(recorder.finish(notes))
}

/// The title screen and the intro: the game from power-on with the menu save in the cartridge, no
/// input for the intro, then Start, Continue and the slot list up to the loaded map.
///
/// # Errors
///
/// The save or the game fails.
pub fn play_title(env: &Env<'_>, name: &str) -> anyhow::Result<PartOutput> {
    let sram = crafted_bytes(env.game, &env.base_save, &menu_sets())?;
    let mut saves = ssd_oracle::MemorySaveStore::default();
    saves.0.insert("sav".to_owned(), sram);
    let mut machine = ssd_oracle::Machine::new(env.rom.to_vec(), Box::new(saves))?;
    machine.set_line_log(true);
    let pads = ssd_game::scripts::parse_timeline(&crate::save::continue_prefix(0))
        .map_err(|message| anyhow::anyhow!("the continue prefix: {message}"))?;
    let mut recorder = Recorder::new(env, name);
    for (frame, pad) in pads.into_iter().enumerate() {
        machine.run_frame(pad, &mut ssd_oracle::NullAudio)?;
        recorder.feed(&machine, frame as u64 + 1);
    }
    Ok(recorder.finish(vec!["power-on, the intro, Start, Continue, the slot list".to_owned()]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_one_block_map_has_one_place_in_the_corner() {
        assert_eq!(places((1, 1)), vec![(0, 0)]);
        assert_eq!(places((0, 0)), vec![(0, 0)]);
    }

    #[test]
    fn the_places_cover_the_map_to_its_last_camera_position() {
        let g = places((9, 11));
        let xs: std::collections::BTreeSet<u16> = g.iter().map(|p| p.0).collect();
        let ys: std::collections::BTreeSet<u16> = g.iter().map(|p| p.1).collect();
        // The camera goes from 0 to (blocks - 1) x 256
        assert_eq!((*xs.first().unwrap(), *xs.last().unwrap()), (0, 8 * 256));
        assert_eq!((*ys.first().unwrap(), *ys.last().unwrap()), (0, 10 * 256));
        assert_eq!(g.len(), xs.len() * ys.len());
        // The views overlap: places are at most a step apart
        let apart = |set: &std::collections::BTreeSet<u16>, limit: u32| {
            let v: Vec<u16> = set.iter().copied().collect();
            v.windows(2).all(|pair| u32::from(pair[1] - pair[0]) <= limit)
        };
        assert!(apart(&xs, STEP_X) && apart(&ys, STEP_Y));
        // Row by row from the top left
        assert_eq!(g[0], (0, 0));
        assert_eq!(g[1].1, 0);
        assert_eq!(*g.last().unwrap(), (8 * 256, 10 * 256));
    }

    #[test]
    fn a_big_map_has_few_places_and_a_row_of_blocks_has_a_row_of_them() {
        // 12 x 12 blocks: the wide view is 372 wide, so a place every 340 pixels
        assert!(places((12, 12)).len() < 200);
        let row = places((9, 1));
        assert!(row.iter().all(|p| p.1 == 0) && row.len() >= 7);
    }

    #[test]
    fn the_party_of_a_map_is_set_on_that_map() {
        let sets = map_sets(7);
        assert!(sets.contains(&set("map", "7")));
        assert!(sets.iter().any(|(f, v)| f == "party" && v == PARTIES[7 % PARTIES.len()]));
        // A character without a level table gets no level
        let dende = map_sets(5);
        assert!(!dende.iter().any(|(f, _)| f == "dende.level"));
        assert!(dende.iter().any(|(f, _)| f == "piccolo.level"));
    }
}
