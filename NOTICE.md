# Notices

This project is released under the GNU General Public License, version 3 (see `LICENSE`). It
includes the following third-party work, each under its own terms.

## jgenesis (GPL-3.0)

The SNES emulation core is a vendored copy of selected crates of **jgenesis**
(<https://github.com/jsgroth/jgenesis>, by jsgroth and contributors), taken at commit
`85ffe5fc66fcc196fb3f5dd5340939e6e4f442a2`, in `third_party/jgenesis/`. jgenesis is licensed under
the GPL-3.0; its licence text is `third_party/jgenesis/LICENSE`, and this project, which links it,
is released under the same licence.

The vendored crates are the upstream files, with the changes that are listed one by one, with
their reasons, in `third_party/jgenesis/PATCHES.md` (a deterministic power-on state, runnable
snapshots, a render log that the HD renderer reads, a fast-forward path and similar). Those changes
are released under the GPL-3.0 as well.

## Fonts (SIL Open Font License 1.1)

The fonts in `assets/fonts/` are used unmodified, each with its own licence file:

| Font | Used for | Copyright | Licence file |
|---|---|---|---|
| IBM Plex Sans | the debug panel | (c) 2017 IBM Corp. | `IBMPlexSans-OFL.txt` |
| JetBrains Mono | numbers in the debug panel | (c) 2020 The JetBrains Mono Project Authors | `JetBrainsMono-OFL.txt` |
| DotGothic16 | the Japanese kanji of the Enhanced menus | (c) 2020 The DotGothic16 Project Authors | `DotGothic16-OFL.txt` |
| Share Tech Mono | the HD text of the game and the Enhanced menus | (c) 2012 Carrois Type Design, Ralph du Carrois | `ShareTechMono-OFL.txt` |

The HD art pack (a separate download, under CC BY-NC-SA 4.0, see `LICENSE-ART.md`) holds pictures of
the battle cards whose kanji and numerals are drawn with the glyphs of Kaisei Decol Bold (SIL Open
Font License 1.1, (c) The Kaisei Decol Project Authors). The font itself is not distributed.

## Rust crates

The game and its tools use open-source Rust crates, downloaded by Cargo (`Cargo.lock` lists the
exact versions), among them `wgpu`, `egui`, `sdl3`, `rayon`, `serde`, `clap`, `ab_glyph`, `png`,
`image-webp` and `zip`, under permissive licences (MIT, Apache-2.0 and similar). Their licence texts
are in the crates' own sources.

## The game

*Dragon Ball Z: Super Saiya Densetsu* and the Dragon Ball characters, names and artwork are the
property of their owners (see the disclaimer in `README.md`). This repository contains none of the
game's code, graphics, sound or text: the game runs from a ROM that you dump from a cartridge you
own. The English text of the game is the work of the translators of the **Klepto v1.02** fan
translation, who made the patch that the English version of the ROM comes from; it is not
distributed here either.
