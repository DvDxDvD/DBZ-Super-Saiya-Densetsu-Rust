# Contributing

Thanks for looking. The project is a Rust rewrite of a 1992 game, and the one rule that matters more
than any other is: **nothing from the game goes into this repository.** Not graphics, not text, not
tables, not disassembly, not screenshots of the original, not a save, not a replay. The game is read
from the player's own ROM at run time.

## What may be committed

- Our own code, tools, tests and notes. Research notes (`docs/re/`) describe routines in our own
  words and may quote at most five consecutive instructions of the original code; a short quote is
  not a disassembly.
- A table of 16 bytes or more that comes from the ROM is never pasted into a source file (as
  `0x..` literals or as `b"\x.."`); read it from the ROM at run time and name it by symbol.
- Pictures only in `assets/` (art made for this project) and `docs/media/` (the pictures of the
  README).
- Scratch output goes to `.work/` (never tracked) and is deleted when it is no longer needed.

`cargo xtask publish-check` enforces this (it needs the ROM cached once with
`ssd-recompiled --rom <your rom>`; `--allow-no-rom` skips the ROM scan, for CI). Run it before every
push.

## Before a pull request

```
cargo fmt -p ssd-rom -p ssd-oracle -p ssd-game -p ssd-app -p ssd-catalogue -p ssd-save -p ssd-hd -p ssd-hdpack -p xtask -p ssd-recomp -p ssd-menus -- --check
cargo clippy -p ssd-rom -p ssd-oracle -p ssd-game -p ssd-app -p ssd-catalogue -p ssd-save -p ssd-hd -p ssd-hdpack -p xtask -p ssd-recomp -p ssd-menus --all-targets -- -D warnings
cargo test -p ssd-rom -p ssd-oracle -p ssd-game -p ssd-app -p ssd-catalogue -p ssd-save -p ssd-hd -p ssd-hdpack -p xtask -p ssd-recomp -p ssd-menus
cargo xtask publish-check
```

- Do not run `cargo fmt --all`: the vendored emulator under `third_party/jgenesis` must stay
  byte-identical to upstream, apart from the patches listed in `third_party/jgenesis/PATCHES.md`
  (each patch is its own commit and has an entry there).
- Tests that need the ROM read `SSD_ROM_PATH` and are `#[ignore]`d; run them with
  `cargo test -p ssd-rom -p ssd-oracle -p ssd-game -p ssd-app -p ssd-catalogue -p ssd-save -- --ignored`.
- Anything that can change what the original game does (the emulation core, a Rust routine, the
  recompiler, `re/symbols.toml`) must keep the reference replays identical: see the developer guide
  (`docs/DEVELOPER.md`), "Developer commands", for `make-replays`, `verify-replays` and
  `standalone-diff`. Changes to the Enhanced mode only (menus, cheats, HD drawing, art) and to docs
  and tools need no replay run.
- A change to `crates/ssd-hd` is checked with `cargo run --release -p xtask -- hd-parity`.

## Checking the interface without a window

The tests and the tools never open the game's window or press real keys. Check the UI with an
offscreen snapshot instead, for example
`cargo run -p ssd-app -- --headless-frames 600 --ui-script ui.txt --ui-snapshot ui.png`
(`docs/DEVELOPER.md`, "UI snapshots").

## Style

English in code, comments, docs, commit messages and UI text. No feature that is not asked for, no
speculative generality; when something is a bug, collect the data first.
