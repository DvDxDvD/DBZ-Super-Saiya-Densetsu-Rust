# Changelog

## Unreleased

### New

- A screenshot (F12, or the debug panel's Save screenshot and Save 1x frame) now writes the machine's
  state beside the picture, named the same with `.ssdstate`, so the moment can be played back headless
  (`xtask shots --state-in`). The PNG is deflate-compressed.

## 0.2.0 (2026-10-16)

### New

- **Options screen** (the Menu list's Options entry), saved with the game or kept for this PC as each page says:
  - **Difficulty**: Easy, Normal and Hard presets over the Cheats screen's numbers (Easy and Hard mark the save with
    the CHEATS badge).
  - **Gameplay**: five text speeds (Slowest to Fastest, the same setting as the Menu's Text entry), message auto-run,
    faster battle animations (x2 to x4, the outcome unchanged), random encounters.
  - **Video**: window or fullscreen, scale 2x to 8x, crisp or smooth look, 16:9 or the original 4:3, scanlines, a CRT
    filter and V-sync, applied at once with a Keep or Revert question.
  - **Audio**: volume, stereo or mono.
  - **Controls**: every button and shortcut remappable on the keyboard and on a gamepad.
  - **Accessibility**: larger text, high-contrast menus and a colour-blind-safe cursor.
- **Screenshots**: F12 (remappable) saves the picture as a PNG in `Pictures\DBZ SSD Recompiled\`; the debug panel can
  also save the original 256-pixel frame.
- **Fast-forward**: x8 keeps its full speed on a typical desktop.

### HD art

- About a hundred new character poses: Vegeta's story poses, battle fragments, small figures of the story pictures.
- The town rooms (houses, shops) in HD, with the same furniture drawn the same way in every room.
- Snake Way, King Kai's planet and the Shenron summoning drawn in HD.
- A redrawn Saibaman portrait, closer to the anime.
- The last field effects, the title's Mode 7 planes, the credits frame.

### Fixes

- Talk and Look in front of a character no longer leave the command window on screen for seconds.
- A dialogue portrait is never drawn half from another character's art.
- A field figure is never drawn half from HD art and half from the original's pixels.
- Purple blobs under some characters' feet removed.
- The Shenron summoning scene no longer turns dark with the dragon's body missing.
- A crash of the Remastered mode on King Kai's planet.
- The HD text font no longer switches off; Use in the Item screen works like the original; the map art is drawn right
  after leaving a building.

Known issues and the roadmap are in the README. The Android port is in progress and not part of this release.

## 0.1.0 (2026-10-14)

First public build: the original game verified frame by frame (Classic), the static recompiler, and the Remastered
mode with HD art, 16:9, HD text, the new menus, the bag and the Cheats screen.
