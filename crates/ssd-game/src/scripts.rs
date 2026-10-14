//! Scripted joypad input, frame by frame from power-on: the play-throughs of the real-ROM tests
//! and of the reference replays (`cargo run --release -p xtask -- make-replays`).
//!
//! Each script gives the joypad held during a frame (as [`ssd_oracle::Replay::input`]): frame
//! `f` runs from the tick after frame `f` is rendered to the one that renders frame `f + 1`.
//! The scenes named are where the canonical game is at those frames with this input from
//! power-on (emulation is deterministic); a script is input only, it holds no game data.

use ssd_oracle::Joypad;
use std::sync::OnceLock;

/// Frames of the [`battle_input`] play-through.
pub const BATTLE_FRAMES: u64 = 7_200;
/// Frames of the [`wander_input`] play-through.
pub const WANDER_FRAMES: u64 = 12_000;
/// Frames of the [`menus_input`] play-through.
pub const MENUS_FRAMES: u64 = 6_878;
/// Frames of the [`landing_input`] play-through.
pub const LANDING_FRAMES: u64 = 6_022;
/// Frames of the [`story_1_input`] play-through.
pub const STORY_1_FRAMES: u64 = 48_036;

/// The timeline text of [`menus_input`] (`scripts/menus.txt`).
const MENUS: &str = include_str!("../scripts/menus.txt");
/// The timeline text of [`landing_input`] (`scripts/landing.txt`).
const LANDING: &str = include_str!("../scripts/landing.txt");
/// The timeline text of [`story_1_input`] (`scripts/story-1.txt`).
const STORY_1: &str = include_str!("../scripts/story-1.txt");

/// The joypad of `frame` in a timeline text, expanded once into `cell`.
fn timeline_input(cell: &'static OnceLock<Vec<Joypad>>, text: &str, frame: u64) -> Joypad {
    let pads = cell.get_or_init(|| {
        parse_timeline(text).unwrap_or_else(|message| panic!("a bundled timeline: {message}"))
    });
    usize::try_from(frame).ok().and_then(|f| pads.get(f)).copied().unwrap_or_default()
}

/// The map menu and its screens, from the first map (`scripts/menus.txt`): Talk, Look, Item (no
/// items yet), Menu > Cards, Status, Order, Text speed and Save (slot 01, completed). Frames 0 to
/// 3689 are [`battle_input`]'s, up to the first map.
#[must_use]
pub fn menus_input(frame: u64) -> Joypad {
    static PADS: OnceLock<Vec<Joypad>> = OnceLock::new();
    timeline_input(&PADS, MENUS, frame)
}

/// Taking off, landing on the first island, walking about on foot with Talk and Look, taking off
/// again and flying until a random battle, which A fights (`scripts/landing.txt`). Frames 0 to
/// 3689 are [`battle_input`]'s.
#[must_use]
pub fn landing_input(frame: u64) -> Joypad {
    static PADS: OnceLock<Vec<Joypad>> = OnceLock::new();
    timeline_input(&PADS, LANDING, frame)
}

/// Story 1 (`scripts/story-1.txt`): [`landing_input`] and on from the first island, through the
/// battles met by flying, to the second island, its walk, the Item menu and the first level-up
/// (Goku at level 2). Frames 0 to 3689 are [`battle_input`]'s.
#[must_use]
pub fn story_1_input(frame: u64) -> Joypad {
    static PADS: OnceLock<Vec<Joypad>> = OnceLock::new();
    timeline_input(&PADS, STORY_1, frame)
}

/// A reference replay's script: its file stem, its frames and its input per frame.
pub type ReplayScript = (&'static str, u64, fn(u64) -> Joypad);

/// The scripts the reference replays of `cargo run --release -p xtask -- make-replays` are
/// recorded from.
pub const REPLAYS: [ReplayScript; 5] = [
    ("battle", BATTLE_FRAMES, battle_input),
    ("wander", WANDER_FRAMES, wander_input),
    ("menus", MENUS_FRAMES, menus_input),
    ("landing", LANDING_FRAMES, landing_input),
    ("story-1", STORY_1_FRAMES, story_1_input),
];

/// The joypad of each frame of a timeline text, from frame 0 (frames after its end have all buttons
/// released). One directive or step per line; `#` starts a comment:
///
/// - `@prefix <battle|wander> <n>`: the first `n` frames follow that script (first line only);
/// - `<n> <keys>`: hold the keys for `n` frames; keys are `up down left right a b x y l r start
///   select`, separated by commas, or `-` for none;
/// - `repeat <k>` ... `end`: the steps in between, `k` times (no nesting).
///
/// # Errors
///
/// The text names its first bad line.
pub fn parse_timeline(text: &str) -> Result<Vec<Joypad>, String> {
    let mut frames: Vec<Joypad> = Vec::new();
    let mut block: Option<(u64, Vec<Joypad>)> = None;
    for (index, raw) in text.lines().enumerate() {
        let fail = |message: &str| format!("line {}: {message}", index + 1);
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut words = line.split_whitespace();
        let first = words.next().unwrap_or_default();
        match first {
            "@prefix" => {
                if !frames.is_empty() || block.is_some() {
                    return Err(fail("@prefix comes first"));
                }
                let script: fn(u64) -> Joypad = match words.next() {
                    Some("battle") => battle_input,
                    Some("wander") => wander_input,
                    other => return Err(fail(&format!("unknown prefix script {other:?}"))),
                };
                let n: u64 = words
                    .next()
                    .and_then(|n| n.parse().ok())
                    .ok_or_else(|| fail("@prefix needs a frame count"))?;
                frames.extend((0..n).map(script));
            }
            "repeat" => {
                if block.is_some() {
                    return Err(fail("repeat blocks do not nest"));
                }
                let k: u64 = words
                    .next()
                    .and_then(|n| n.parse().ok())
                    .ok_or_else(|| fail("repeat needs a count"))?;
                block = Some((k, Vec::new()));
            }
            "end" => {
                let (k, steps) = block.take().ok_or_else(|| fail("end without repeat"))?;
                for _ in 0..k {
                    frames.extend(steps.iter().copied());
                }
            }
            count => {
                let n: usize =
                    count.parse().map_err(|_| fail(&format!("`{count}` is not a frame count")))?;
                let mut pad = Joypad::default();
                for key in words.flat_map(|w| w.split(',')).filter(|k| !k.is_empty() && *k != "-") {
                    match key {
                        "up" => pad.up = true,
                        "down" => pad.down = true,
                        "left" => pad.left = true,
                        "right" => pad.right = true,
                        "a" => pad.a = true,
                        "b" => pad.b = true,
                        "x" => pad.x = true,
                        "y" => pad.y = true,
                        "l" => pad.l = true,
                        "r" => pad.r = true,
                        "start" => pad.start = true,
                        "select" => pad.select = true,
                        other => return Err(fail(&format!("unknown key `{other}`"))),
                    }
                }
                let target = block.as_mut().map_or(&mut frames, |(_, steps)| steps);
                target.extend(std::iter::repeat_n(pad, n));
            }
        }
    }
    if block.is_some() {
        return Err("a repeat block is not closed".to_owned());
    }
    Ok(frames)
}

/// Whether `frame` is in the first `len` frames of each `period` from `start` to `end`.
fn pulse(frame: u64, start: u64, end: u64, period: u64, len: u64) -> bool {
    (start..end).contains(&frame) && (frame - start) % period < len
}

/// Into a battle and through it:
/// - frames 0–2399: no input (intro);
/// - 2400–3599: every 64 frames Start, A, A, Down, A (the trace tooling's "mash" cycle):
///   title, new game, prologue and opening scenes, then the first map (~frame 3200);
/// - B at 3600 and 3640 closes any message; A at 3700 opens the map menu, Down at 3730 and
///   3760 moves to Fly, A at 3800 takes off (~frame 3900);
/// - Right held 3900–4499: flight east into a random battle (~4500);
/// - A every 16 frames (4 frames pressed) from 4500: through the battle, won by ~7000.
#[must_use]
pub fn battle_input(frame: u64) -> Joypad {
    let mash = |offset: u64| pulse(frame, 2400 + offset, 3600, 64, 4);
    let at = |start: u64, len: u64| (start..start + len).contains(&frame);
    let battle = pulse(frame, 4500, u64::MAX, 16, 4);
    Joypad {
        start: mash(0),
        a: mash(16) || mash(32) || mash(56) || at(3700, 4) || at(3800, 4) || battle,
        down: mash(48) || at(3730, 4) || at(3760, 4),
        b: at(3600, 6) || at(3640, 6),
        right: (3900..4500).contains(&frame),
        ..Joypad::default()
    }
}

/// [`battle_input`] until the take-off, then wandering the map: from frame 3900 a direction
/// held for 300 frames at a time, Right, Up, Left, Down and again, and from 4500 A every 16
/// frames (4 frames pressed), which also fights the battles met on the way.
#[must_use]
pub fn wander_input(frame: u64) -> Joypad {
    if frame < 3900 {
        return battle_input(frame);
    }
    let direction = (frame - 3900) / 300 % 4;
    Joypad {
        right: direction == 0,
        up: direction == 1,
        left: direction == 2,
        down: direction == 3,
        a: pulse(frame, 4500, u64::MAX, 16, 4),
        ..Joypad::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_timeline_expands_steps_prefixes_and_repeats() {
        let pads = parse_timeline(
            "# comment
@prefix battle 2400
3 -
2 a, start # two keys
repeat 2
1 down
2 -
end
",
        )
        .unwrap();
        assert_eq!(pads.len(), 2400 + 3 + 2 + 6);
        assert_eq!(pads[2399], Joypad::default());
        assert!(pads[2403].a && pads[2403].start && pads[2404].start && !pads[2405].a);
        assert!(pads[2405].down && !pads[2406].down && pads[2408].down);
        assert_eq!(pads[0..2400], (0..2400).map(battle_input).collect::<Vec<_>>()[..]);
        for bad in [
            "x -",
            "3 jump",
            "end",
            "repeat 2
1 a",
            "1 a
@prefix battle 5",
            "@prefix nope 5",
        ] {
            assert!(parse_timeline(bad).is_err(), "{bad:?}");
        }
        assert!(
            parse_timeline(
                "repeat 2
repeat 3
end
end"
            )
            .is_err()
        );
        assert_eq!(parse_timeline("").unwrap(), []);
    }

    #[test]
    fn the_reference_scripts_are_battle_and_wander() {
        let names: Vec<&str> = REPLAYS.iter().map(|&(name, ..)| name).collect();
        assert_eq!(names, ["battle", "wander", "menus", "landing", "story-1"]);
        assert_eq!((REPLAYS[0].1, REPLAYS[1].1), (7_200, 12_000));
        assert_eq!(REPLAYS[1].2(5_000), wander_input(5_000));
    }

    #[test]
    fn the_timeline_scripts_have_their_declared_frames_and_start_as_battle_input() {
        for (text, frames, input) in [
            (MENUS, MENUS_FRAMES, menus_input as fn(u64) -> Joypad),
            (LANDING, LANDING_FRAMES, landing_input),
            (STORY_1, STORY_1_FRAMES, story_1_input),
        ] {
            let pads = parse_timeline(text).unwrap();
            assert_eq!(pads.len() as u64, frames);
            for frame in (0..3_690).step_by(7) {
                assert_eq!(input(frame), battle_input(frame), "frame {frame}");
            }
            assert_eq!(input(frames - 1), pads[pads.len() - 1]);
            assert_eq!(input(frames + 5), Joypad::default(), "released after the end");
        }
        // The first step after the shared prefix: ten frames of nothing, then A.
        assert_eq!(menus_input(3_690), Joypad::default());
        assert!(menus_input(3_700).a && !menus_input(3_704).a);
    }

    #[test]
    fn battle_input_follows_its_script() {
        assert_eq!(battle_input(0), Joypad::default());
        assert_eq!(battle_input(2399), Joypad::default());
        assert!(battle_input(2400).start && battle_input(2403).start && !battle_input(2404).start);
        assert!(battle_input(2416).a && battle_input(2448).down && battle_input(2456).a);
        assert!(battle_input(3600).b && battle_input(3645).b && !battle_input(3646).b);
        assert!(battle_input(3700).a && battle_input(3730).down && battle_input(3800).a);
        assert!(battle_input(3900).right && battle_input(4499).right && !battle_input(4500).right);
        assert!(battle_input(4500).a && battle_input(4503).a && !battle_input(4504).a);
        assert!(battle_input(7188).a && !battle_input(7192).a, "to the end");
    }

    #[test]
    fn wander_input_is_battle_input_until_the_take_off_then_wanders() {
        for frame in 0..3900 {
            assert_eq!(wander_input(frame), battle_input(frame), "frame {frame}");
        }
        let held = |frame: u64| {
            let j = wander_input(frame);
            [j.right, j.up, j.left, j.down]
        };
        assert_eq!(held(3900), [true, false, false, false]);
        assert_eq!(held(4199), [true, false, false, false]);
        assert_eq!(held(4200), [false, true, false, false]);
        assert_eq!(held(4500), [false, false, true, false]);
        assert_eq!(held(4800), [false, false, false, true]);
        assert_eq!(held(5100), [true, false, false, false]);
        assert!(!wander_input(4499).a && wander_input(4500).a && !wander_input(4504).a);
        assert!(wander_input(11_988).a && !wander_input(11_992).a, "to the end");
        let j = wander_input(6000);
        assert!(!j.b && !j.start && !j.select && !j.x && !j.y && !j.l && !j.r);
    }
}
