# Graphics: how the game stores, decodes and draws its pictures

Found for milestone M6 (HD graphics and widescreen). Everything here was read from the code of banks `$00`, `$01`, `$03`,
`$04`, `$05` and `$06` and checked by playing the reference replays headless (`xtask disasm`, `shots`, `probe`, and a local
logger that breakpoints the decoders and reads the CPU write log of `$21xx`, `$43xx` and `$420B`; see "How to see it
again"). Certainty: "seen" = a replay shows it, "code" = read in the code only. Routines have no symbols (the symbols file
only holds routines that have a Rust function) and are written as bank:address; tables are named by what they are.
The menu pictures (portraits, window engine) are in `menus.md`; this file is about the whole screen.

## Summary

- Every picture is **one LZ scheme** (below), decoded by two entry points of the same routine, then regrouped into SNES
  bitplanes. A few things are not LZ: the English UI tiles (uncompressed, patched over a decode), a handful of animated
  bits, and the title's tile maps (a bit-tree code). Palettes are 16-colour rows in bank `$09`.
- **All graphics reach VRAM through WRAM**: a decode buffer (`$7E:9000`, or bank `$7F` staging areas), then DMA, queued by
  the game for the NMI or started directly during forced blank. The CPU draws nothing (but the title's first frames); the tile map buffers (`$7E:3000`,
  `$7E:4000`) and the shadow OAM (`$0400`) are built in WRAM and copied.
- Most of the game runs in **BG mode 1 with 256 x 239 lines** (`SETINI` overscan), three BG layers and 8x8 or 16x16
  sprites; only the flight cutscenes and some title scenes use Mode 7, the battle animation scene uses mode 0 with HDMA, and
  mosaic, fixed-colour add/subtract and the brightness register are the only other effects (windows only in the ending).
- Pictures are **content-addressable**: a tile in VRAM has no id the PPU can see, but every tile the game ever uploads
  comes verbatim from a decoded stream, so a hash of the 32 (4 bits) or 16 (2 bits) bytes of a tile identifies it. This is
  what the HD route keys on.

## The decoders

| Routine | What | Seen |
|---|---|---|
| `00:C559` | LZ decoder; the source is the 24-bit pointer in `$83` (word) and `$85` (bank); the output goes to `$7E:9000` and the size is left in `$89` | seen, 14 call sites (field, battle, menus, title, cutscenes) |
| `00:C5ED` | the same decoder with the destination in `$86` (word) and `$88` (bank): every byte is also written through the WRAM port (`$2181-$2183`, `$2180`), and into `$7E:9000` as the match window | seen, 4 call sites (`00:9DBC`, `00:9F23`, `00:A744`, `05:96F8`): battle and cutscene sprite pieces into bank `$7F` |
| `00:C68C` | bitplane regroup of `$7E:9000` (size in `$89`) | seen, called after every `00:C559` of a tile set |
| `00:C707` | the same regroup, in place, on a range of any bank (`$86`, `$88`, `$89`) | seen, after every `00:C5ED` of tiles |
| `00:C782` | 4-bit tiles to one byte per pixel into `$7E:9000` (the Mode 7 cutscene tile sets) | seen, 3 call sites (`00:9DC4`, `00:9F2B`, `00:A74C`) |
| `04:83B2` | a variant of the same unpack to bank `$7F` odd bytes (`$7F:0001+`), 51 blocks of eight rows | seen, one call site (`04:8768`) |
| `00:FDB5` | bit-tree decode: 2048 symbols, one stream bit per step, from a stream at `00:F150` and a node table at `00:FD49`, as the low bytes of words to `$7E:9000`: the **tile maps of the title picture** (two 32 x 32 maps, uploaded to VRAM `$6000`/`$6800`/`$7000` right after) | seen, one call (the title, frame 1760 of the battle replay), see `routines/decode_bit_tree_to_9000.md` |

**The LZ format** (also decoded by the HD menus from the same description in `menus.md`): a 16-bit output size, then groups
of eight items led by a flag byte read least-significant bit first. A set bit is a literal byte. A clear bit is a two-byte
reference: the low nibble of the first byte plus 3 is the length of the copy, and the other twelve bits (the second byte
and the high nibble of the first) are the distance back from the output position; a source before the start of the output
reads as zero. The decoder stops when the output reaches the size, so streams are simply **concatenated** in the ROM: the
next stream starts where the last one ended (bank `$16` is portraits one after the other, bank `$1C` is runs of
sprite pieces). Decoded sizes are always multiples of `$20`.

**Bitplane layout.** The stream holds each 16-byte block as plane 0 (8 bytes) then plane 1 (8 bytes); the regroup
interleaves the two row by row, which is the SNES layout. A 4 bits-a-pixel tile is therefore two such blocks (planes 0/1
then 2/3), 32 bytes; a 2 bits-a-pixel tile is one block, 16 bytes. The tail of `00:C68C` jumps to `00:F400`, which calls
`10:A000`, a hook of the English patch: after the decode of the stream at `08:8000` (the shared UI tile set) it
**overwrites the decoded buffer with `$1800` bytes copied from `11:8036`** (uncompressed, already in SNES layout). Those 384
tiles of 2 bits are the font the English game really uses: Latin letters in the slots of the Japanese kana, digits, the
frame and cursor pieces, the card balls and the kanji of the cards (`names.md`: "the English translation draws its
letters from tiles that are interleaved with the Japanese kana"). The LZ stream of `08:8000` itself still holds the
Japanese set.

## How pictures reach VRAM, CGRAM and OAM

| Path | Where | What |
|---|---|---|
| **VBlank queue** | `$0800` (`slot_block_0800`), drained by `00:8324` in the NMI | records of 8 bytes: a first byte with bit 7 set (and the `VMAIN` value in its low bits), the VRAM word address, a 24-bit source and a size; each is one DMA on channel 2. `find_positive_slot` (`00:8711`) finds a free slot; the column builder (`06:EF00`) and `write_slot_record` (`01:CAAB`, the portraits) fill it. Portraits, field tile columns, animated bits and most uploads after a scene is up go this way |
| **Direct DMA, forced blank** | `00:90E6` (source `$7E:9000`, size `$89`, destination set by the caller), `00:90EC` | the scene loaders call `00:C559`, `00:C68C`, set `$2116` and DMA at once: `00:8844` (a table of tile sets), `00:8C50` (a map's tile set), `00:8F34`, `00:90CF`, `00:9FE2`, `00:A1B5`, `00:AE2D`, `04:875D`, `04:9EFC` |
| **Tile-map buffers** | `$7E:3000` to VRAM `$2C00`, `$7E:4000` to `$6800` (`$0800` bytes each) | the window engine (`menus.md`) writes the BG3 and BG1 tile maps of the field and the battle command screen into them; they are copied whenever a window changes (about every 13 frames in the wander replay, 920 times in 12,000 frames) |
| **Map columns** | `$0880` (64 bytes) to VRAM `$6000`/`$6400` + column, `VMAIN` `$81` (+32 words after each pair: a column) | `06:EF00`-`06:EF9A` builds the 32 tile-map words of one map column and queues it (seen: the field's BG tile map is filled one column at a time, see "The field map") |
| **Face/portrait assembly** | VRAM `$5000`-`$5900` in steps of `$100` bytes | a 32-tile stream (`16:xxxx`, 1024 bytes decoded) is uploaded two chunks of 8 tiles at a time, one chunk pair every 2 frames, so a talking face is built in VRAM while it animates |
| **Battle sprite staging** | bank `$7F` (`$0000`-`$2800`), then DMA of `$100`-`$1000` bytes to VRAM `$0000`-`$7000` | the battle and cutscene sprites are **assembled from small LZ pieces** (`$20`-`$360` bytes decoded each) by `00:C5ED` into `$7F`, regrouped by `00:C707` and uploaded; a pose is a handful of pieces (banks `$1C`-`$1F`, `$0D`, `$10`, `$14`, `$07`, `$0E`) |
| **CPU writes to `$2118`** | 256 words a frame | the title/boot sequence (frames 168-183 of the battle replay); not seen elsewhere |
| **CGRAM** | shadow `$0200` (512 bytes); a working copy `$0300`; flag `copy_done` (`$0715`) | the NMI copies all of `$0200` to CGRAM when the flag is set. Palettes are 16 colours loaded from bank `$09` (the pointers are words in bank `$02`, e.g. `02:A356` for the objects) by `01:C885` into `$0300`+slot x 32, and blended by `05:C116` for fades and flashes |
| **OAM** | shadow `$0400` (512 bytes) and `$0600` (the 32 high-table bytes), 544 bytes in all | copied every NMI by `00:8324`. The NMI also runs the sprite builders (`00:9736` is the field state dispatcher) and `hide_sprites_from_index` (`00:86A0`) fills the rest with Y `$F0` |
| **HDMA** | enabled with `$420C` | `$2101` (`OBSEL`) and `$212C` (`TM`) every frame in the battle animation scene (two bands of the screen with other layers and sprite sizes), `$2105`, `$2121`/`$2122` (a CGRAM gradient) and `$212C` in the Mode 7 flights, `$211B`/`$211E` (Mode 7 scale) in the title |

## What the PPU is set to, scene by scene (seen)

`game_mode` (`$24`) is the main loop's state (`scenes.md`); bit 7 means "just entered". The register values are the last
writes seen in the reference replays.

| Scene | `game_mode` | BG mode / `TM` | Layers and VRAM (word addresses) |
|---|---|---|---|
| Boot, title, intro | `$87` (and `$07`) | 0 and 1 and 7 (`$2105` = 0, 1, 7); `TM` `$00`-`$17` | `BG1SC` `$61`/`$62` (map `$6000`, 64 x 32 or 32 x 64), `BG2SC` `$68`/`$6A` (`$6800`), `BG3SC` `$6E`/`$70` (`$6C00` or `$7000`); BG chars: BG1 `$5000` or `$2000`, BG2 `$4000` or `$2000` (`BG12NBA` `$45`/`$22`), BG3 `$2000` or `$4000` (`BG34NBA` `$02`/`$04`); OBJ chars `$0000` or `$4000`; Mode 7 from frame 157 (with `SETINI` `$04`); the logo is `00:C7DA`, the katakana title line `00:E74A`, the clouds `00:D6C0`, the kana font `00:DF39`, the title backdrop `0A:E400` |
| Story transitions (flights, captions) | `$07` | 7 (`$2105` = 7), `TM` `$01` and `$11` | Mode 7 (8 bits a pixel; the map and the chars share VRAM `$0000`-`$7FFF` as Mode 7 requires, built in `$7F:0000` by `00:C782`/`04:83B2` and uploaded as one `$8000`-byte DMA), BG3 text map `$5C00`, OBJ `OBSEL` `$03`; per-line `$2105`, `$2121`/`$2122` and `$212C` by HDMA |
| **Field** (world, towns, caves, flight) | `$06`, `$00` | 1 with BG3 priority (`$2105` = `$09`), `TM` `$17`, `CGADSUB` `$82` while a menu is open | **BG2 = the map**: map `$6000` (64 x 32), chars `$3000`-`$4B00` (`08:8D4C`, `$3600` bytes, 432 tiles of 4 bits); **BG1** = the party's corner faces: map `$6800` (32 x 32), chars `$5000` (4 bits; `16:xxxx` pictures, built in pieces; seen: the corner faces of the field are BG1 tiles); **BG3** = windows, dialogue and menu text: map `$2C00`, chars `$2000` (the UI tile set, 2 bits); OBJ chars `$0000` (`OBSEL` `$00`: 8 and 16 pixel sprites): `0F:xxxx` sprite sets of `$40`-`$840` bytes each, several per map |
| Battle, command screen | `$02` | the same mode and layers | BG2 = the **battle background** (a fixed 32 x 32 picture: chars `09:83FA`, `$36E0` bytes, to `$3000`; its tile map `0A:E1D0`, `$0800` bytes, to `$6000`), BG1 map `$6800`, BG3 map `$2C00` (HUD, the **cards** and their kanji: 2 bits, chars `$2000`), OBJ = the fighters' sprites assembled in bank `$7F` |
| Battle, animation | `$04` | mode 0 (`$2105` = 0, four 2-bit layers); `TM` `$14`-`$1E` by HDMA | `BG1SC` `$3C` (`$3C00`), `BG2SC` `$60`/`$61` (`$6000`), `BG3SC` `$68`/`$69` (`$6800`), `BG4SC` `$70`/`$73` (`$7000`); chars BG1 `$2000`, BG2 `$3000`, BG3 `$4000`, BG4 `$5000`; the attack's large pictures (`11:C000`, `11:CC45`, `0A:xxxx` of `$800` to `$1680` bytes) streamed through bank `$7F`; the screen's lower part by HDMA |
| Towns, interiors, the shop | `$01` | not measured (its tile set is `08:BF20`) | |
| Menus, status | `$03` | the field's mode and layers | the same as the field with BG2 untouched; windows in the BG1/BG3 buffers (`menus.md`) |
| Minigame (Dende's rescue) | `$0A` | not measured | |
| Last fight stages | `$0C` | not measured; large pictures `12:xxxx` (`$6E0`-`$7C0` bytes) | |
| Ending, staff roll | `$11` | mode 0/1; **windows used** (`W12SEL` `$32`, `WOBJSEL` `$03`, `TMW`) | the only scene with windows |

`TS` (the sub screen) is never enabled. `CGWSEL` is `$00` (fixed colour) or `$30` (the colour math is off, the whole
screen is the main screen); `CGADSUB` is `$82` (subtract the fixed colour from BG2: the field darkens while a menu or a
message is open), `$30`/`$BC` in a few title steps. `MOSAIC` runs `$0F`...`$FF`...`$0F` in 16 steps for every battle
entry and exit. No hi-res mode, no interlace, no direct colour, no offset-per-tile.

## What is stored where (the asset catalogue, as the reference replays use it)

377 distinct LZ streams were met in 13 replays (battle, wander, landing, menus, eight story secrets: the two endings, the
Ginyu duel, Frieza's forced fight, Super Saiyan Frieza, Nail and Porunga, the Namek Z fighters, Piccolo's return and Dende's
rescue), about 340 KB decoded from about 210 KB in the ROM. Counts are streams.

| Class | Where | Seen | Notes |
|---|---|---|---|
| UI tiles (the font, windows, cards, cursor) | `08:8000` (LZ, `$1800`) + `11:8036` (the English overlay, uncompressed) | 1 stream, 50 loads | 384 tiles of 2 bits. VRAM `$2000`. The card balls, the Z, the card kanji and the dot numbers are in it |
| Field tile sets | `08:8D4C` (`$3600`, the outdoor maps), `08:BF20` (`$1B20`, seen at `game_mode` `$01`: towns and interiors), `08:CB70` (`$1660`, a cutscene) | 3 streams | 4 bits, VRAM `$3000`; one set serves a whole map (the columns of the map are tile maps; the tiles are never streamed) |
| Field and flight sprites | `0F:8000`-`0F:D70B` | 28 streams, `$40`-`$840` bytes | 4 bits; VRAM `$0000`-`$1D00`; Goku's ship, Frieza's soldiers and pods, the party's walking frames, the Dragon Balls |
| Object portraits and faces | bank `$16` (17 streams seen, `16:8000` to `16:F1E8`, `$400` bytes decoded each, 32 tiles) | 17 | the picture of an object and the corner faces; `16:8000` and `16:8367` are loaded 573 and 91 times (the animated face of the field); `menus.md` has the pointer table (`02:A4EF`) and the palettes |
| Battle backgrounds | tiles `09:83FA` (`$36E0`) and two more `09:` sets, maps `0A:E1D0`, `0A:DC4D`... | 3 + 6 streams | BG2 of the command screen |
| Large attack and cutscene pictures | `0A:xxxx` (19 streams, to `$1680`), `0E:xxxx` (to `$1760`), `11:C000`, `11:CC45`, `12:xxxx` (6), `13:xxxx` (6) | about 40 | 2 or 4 bits; streamed to VRAM `$3000`-`$6C00` in the animation scene or the cutscenes; `12:xxxx` are the last fight's busts |
| Fighters' pieces (battle, title, endings) | banks `$1C`-`$1F` (167 streams: 88, 21, 24, 34), `$0D` (28), `$10` (24), `$14` (29), `$07` (19), `$0E` (9), `$0C` (2), `$17` (4) | about 300 | pieces of `$20`-`$360` bytes decoded into bank `$7F`; one pose = a few pieces; they are reloaded for every pose (`1C:A57A`.. are loaded 48 to 277 times in one battle) |
| Title, intro, credits | bank `$00` (`C7DA`, `D13B`, `D6C0`, `DF39`, `E74A`, `EB97`, `EE97`, `EFE3`, `F504`), `0A:E400`, `0A:F0A0` | 14 | the logo, the clouds, the katakana title line, the kana font, small sets; with the bit-tree tile maps the title is the only scene whose maps are not stored as LZ |
| Palettes | bank `$09` (16 words each) | not measured | pointer tables in bank `$02`; `01:C885` copies one row into `$0300` + 32 x slot |
| Title set (the text of the title, the intro and the credits) | `00:DF39` (LZ, 256 tiles of 2 bits; VRAM `$2000` onwards for the text) | 2 | white digits (`$80`-`$89`), comma `$8C`, apostrophe `$42`, `©` `$FE`, `/` `$FF` and capitals `$DA`-`$FD`, one 8 x 8 tile each; bold capitals and `.` of two tiles stacked: the top tile `$90`-`$9F`, `$B1`-`$B9` or `$BD` (colour 2), the tile 16 further on (colour 3) |

The title set's white capitals are in VRAM with colour 1 clear: the stream's colour 1 pixels read colour 0 there, so the
binding of the HD text (`ssd_hd::font`) compares them in that form. The bold names keep colour 1 as drawn. Tile indices
are the stream's own; the glyph table is ours, read off the tiles on screen.
| Not LZ | `00:AEA7`, `00:AEBD` (`$16` bytes each: DMA straight from the ROM to VRAM `$6DE5` every second frame of the title, a row of tile-map words), `11:8036` (the English UI overlay), the `00:F150` bit stream | | |

## The field map (code, checked on `wander`)

The field's BG2 is not a stored picture: it is a **map** read through a lookup, and its VRAM tile map holds only the
columns near the camera.

- **Map header.** `00:8E26` (at every map load) reads the map id (`$01A0`) and, through a word table at `06:F2DE`, points at
  a record: the map's width in 256 x 256 blocks (a byte, kept in `$0703`), its height in blocks (`$0702`) and then the grid
  of block ids, one byte per block (pointer in `$0704`-`$0706`). Sizes in blocks run from 1 x 1 (rooms and caves: maps 1,
  6, 7 and 32 to 47: 19 of the 48 maps; maps 3, 18 and 19 are 1 x 2) to 12 x 12 (the Namek maps 9, 11, 16, 20 to 25, 3072 x 3072 pixels); the first map is 9 x 11.
- **Block, metatile, tile.** `06:F089` turns a world pixel (x in `$00`/`$01`, y in `$02`/`$03`) into one tile-map word
  (`$04`/`$05`): the block id picks a 256-byte-aligned definition through a table at `06:F1EE` (bank `$0B`, `$200` bytes
  each: 16 x 16 cells of 16 x 16 pixels, one word each); the word, masked, indexes the map's **metatile table** (pointer in
  `$0707`-`$0709`; four tile-map words per metatile, in a 2 x 2 layout with a row stride of `$40` bytes, stored high byte
  first); the pixel's bit 3 in x and y picks the quarter. A pixel outside the grid (negative, or at or past the limits)
  gives tile 0, palette 0. The routine is pure: it reads only the map pointers and ROM tables.
- **Column streaming.** `06:EF00` builds the 32 tile-map words of the column at the camera's right (or left) edge by calling
  `06:F089` 32 times down the column, and queues the column (`VMAIN` `$81`) to VRAM `$6000`/`$6400` + (x / 8 and 31). The
  load of a map fills 35 columns (`641D`, `641E`, `641F`, then `6000`-`601F`), about 280 pixels, and scrolling adds one
  column as the camera crosses a tile boundary. **So VRAM never holds more than about 35 columns of the map: the
  64-column ring is mostly stale.** Only column uploads were seen in the replays (the ring is 32 rows, 256 pixels, more than the 239 lines); `06:EF9B`
  looks like the row counterpart (code only).
- **Camera.** The NMI (`00:82FB`) writes the BG2 scroll from `$33` (x) and `$35` (y), and BG1's from `$37`/`$39`. The
  follower routines (`follow_step_x`, `follow_target_step`, `06:EDB3`, `06:EE42`) move the camera toward the leader's
  position (`$01A2`, `$01A4`) by at most 4 pixels a frame and settle `$80` behind it in x, which centres a 256-pixel
  view; the camera is **clamped to the map**: x between 0 and (width - 1) x 256, so the original never shows outside the
  map.
- **Objects.** Map objects (NPCs, Dragon Balls, chests) are at `$1000`: 6 bytes each, up to 32 (`$C0` bytes), the first
  byte with bit 7 set when in use, then a word each for x and y in the world. `03:9158` draws them: for each object it
  computes `screen = world - camera` and **skips it unless both coordinates are below 255** (an unsigned compare, so a
  negative one is skipped too), then calls the metasprite builder `00:85F5` with the list that the word table at `02:802E` gives for its
  type (the low 5 bits of the first byte). An object whose origin is outside the 256 x 255 window is never drawn, even if its sprite would reach into it.
- **Metasprite builder `00:85F5`.** Reads a list of 4-byte parts (y offset, tile, attribute, x offset; ended by `$80`) from
  the pointer in `$00`, adds the base position (`$0A`, `$0C`) and tile base (`$12`) and attribute bits (`$10`), writes OAM
  entries at `oam_next_index` (`$55`) and the 9th x bit and size bits into the high table. A part whose y reaches `$F0` or
  more is hidden; **a part whose x is above 255 is hidden**, and a negative x down to -127 is allowed (the 9th bit). It has
  three callers in the traces: `01:C93E` (menu portraits and corner objects), `01:F967` (the two-slot entity pass) and
  `03:91F7` (the map objects above).
- **Party and other entities** (the entity table at `$0E00`, 32 bytes each) are drawn by other code (`01:F9AC` and the
  object update); only the map objects' loop was read.
- **Ported and checked (`ssd-hd::world`, `xtask hd-wide check`).** The lookup of `06:F089` equals the game's BG2 tile map word on
  every visible tile of every field frame. The screen's world position is BG2's scroll registers taken modulo the ring
  (512 x 256 pixels) next to the camera (`$33`, `$35`): the scroll registers and the OAM of a frame are made by the NMI from the
  variables as they stood at the end of the previous frame's run, so those are the ones to read; the row of scanline `n` is
  world y = top + `n`. The builder places a map object's parts with the attribute byte ORed with 1 (the second name table),
  the tile base 0 and no flip; a part is hidden when `ybase + yoff` is not in `0..$F0` or `xbase + xoff` is above 255, and every
  part the port computes is in the OAM of the frame, except in the frames of a map load, where the OAM is cleared while the object
  table is already filled. The frames of a map load (all lines in forced blanking, with VRAM and OAM written) are black; the
  fade-in of a map uses the mosaic on BG2.

## Sprites and the OAM

OBJ sizes follow `OBSEL`: field `$00` (8 x 8 and 16 x 16), title `$02`/`$03`, the battle animation scene changes it by
HDMA. The OAM has 128 entries (a 4-byte low table at `$0400` and a 2-bit-per-entry high table at `$0600`). The 32 sprites
and 34 tiles a line limits matter in busy battle frames; the 239-line frame hides entries with Y `$F0`/`$E0`.

## The frame

The core's frame is **256 x 239** (`SETINI` bit 2, overscan) in 414 of 420 sampled frames; 256 x 224 in the others (a few
title and cutscene steps). Mosaic, brightness (`INIDISP`) and the colour math act on the whole frame; the PPU register
writes are mostly once per frame, except the HDMA scenes above.

## Facts the HD work relies on

1. **A frame can be re-rendered from VRAM, OAM, CGRAM and the per-line registers** (the basis of the HD route). A re-implementation of mode 1 (BG1-BG3, OBJ with priorities and sizes, fixed-colour add and subtract,
   brightness) from `peek_vram`/`peek_cgram`/`peek_oam` and the register writes matched the PPU's frame **pixel for pixel**
   on 489 of 774 sampled frames of six replays: the field, the menus, the battle command screen and most of the title; it
   fails exactly on mode 7, mosaic steps, the battle animation scene (per-line `OBSEL`/`TM`) and the ending.
2. **Every tile in VRAM is a decoded tile**, byte for byte, of a stream listed above, so the converter can index all of them
   by content. Exceptions: tiles the game builds itself (none seen for 4-bit tiles; the digits and HUD text are font tiles).
3. **Portraits are 32 tiles uploaded as two runs of 8** (tiles 0-7 and 16-23): a match must work on a run of tiles, not on
   the whole 32.
4. **The cards are BG3 tiles of 2 bits** (the UI set): the five card kanji, the ball dots, the Z and the numerals are tiles
   of `08:8000`/`11:8036`, with the card's colours in the BG3 palette row, so the translated card glyphs are a tile swap.
5. **The field's world is a map lookup plus an object table**, both read-only reachable: a wider view can be drawn from them
   without touching the emulated machine (map lookup, objects at `$1000`, the metasprite lists of `02:802E`).
6. The map objects are culled on the **origin** at 255 pixels; the camera is clamped to the map; 19 of the 48 maps are a
   single 256 x 256 block.
7. **Title and flights share the Mode 7 plane but not the main loop state**: the title runs in `game_mode` `$87` (Mode 0,
   1 and Mode 7 BG1 alone), the flights in `$07` (Mode 7 BG1 alone, and with Mode 1 caption lines below line 160 in
   the same frame: BG3 text on a backdrop, decided per line). The last fight (`$0C`) also shows the plane. The replay
   that shows them is `battle` (frames 1775-2600).
8. **A frame in the field's mode with a stale map header** (the battle, `$02`/`$04`) keeps the field's variables in work
   RAM: the scene is told by `$24`, not by the map.

## How the HD art finds what it replaces (M6 task 4, `ssd-hd::art`)

The art is keyed by what the game draws, not by addresses, so it follows the game wherever it loads the same pictures.

- **Portraits and faces** are characters of bank `$16` streams: the art of a 32 x 32 picture (a half of a 32-character
  stream, cells in the order of the item screen's grid) is found through the tile binding (the characters of the
  picture's asset on screen), for BG1 (the corner faces) and for object tiles alike. The same drawing is shown in
  several palettes (an object and the faces that share its stream), so the art is chosen by comparing the colours the
  art was made for with the palette row the pixel is drawn with.
- **Battle poses** are assemblies of sprites: the object pieces of one OAM palette (1 the party, 2 the enemy) are one
  fighter, and the pieces of the other palettes form effects as clusters that touch within four pixels. A pose's key is
  the hash of its trimmed palette-index array (a fighter) or of its colours (an effect, whose HDMA gradients change the
  colours line by line). Pieces are decoded whole, the off-screen parts of a sprite included, and a piece wholly off the
  screen is left out; a lower OAM index is on top.
- **What shows**: the art replaces the pose where the original pixels were visible (the PPU's own priorities, windows
  and per-line limits decide), and grows into the pixels next to them, where the pose would be in front of what is
  there. The picture under the sprites is the frame drawn again without them.

## Battle backgrounds (`ssd-hd::bgart`, `hd-art bgscan`)

The battle's backgrounds are BG layers of the two battle scenes, found in the 28 reference replays with `hd-art bgscan` (a
layer's content is a hash of its tile map rows and of the tiles they use; the palette is not part of it). Four kinds, 24
distinct contents:

| Kind | Scene, layer | Contents (frames, of the 107,000 battle frames played) |
|---|---|---|
| `cmd` | command screen (`$02`), BG2, mode 1, map `$6000`, chars `$3000`, one screen; only the first 22 tile rows are the picture (the windows are over the rest, and what is under them changes) | 4 pictures: an ice cave, two Earth shores, Namek (23,000) |
| `far` | animation scene (`$04`, `$0C`), BG4, mode 0, map `$7000`, chars `$5000`, 64 x 64 tiles, wraps both ways | 6: three Namek skies (yellow clouds on lime green, three horizons), an Earth sky over grass, an Earth sky over the sea, a cave (60,000) |
| `near` | same scenes, BG3, map `$6800`, chars `$4000`, 64 x 32 tiles; a band of ground or water, the rest is transparent | 5: dark sea, Namek lake (two), a green plain, a cave (35,000) |
| `dash` | same scenes, BG4, one screen: the backdrops of the flights, charges and transformations (speed lines on a gradient, radial bursts, a diamond pattern) | 9 (15,000) |

Facts the HD binding relies on: the animation scene splits the screen at line 91 or 167 (the layers' scroll registers change on a
line, sometimes in the middle of it), the far and near layers scroll at different speeds (parallax), the scroll registers hold
more than 10 bits (the PPU keeps the low ones: the layer wraps), the sky of the far layer is the backdrop colour (colour 0 of
the layer shows it), and the palettes change from frame to frame (fades and flashes: thousands of CGRAM states in a replay) while
the tiles never change in a battle frame (no VRAM write is logged during the display of any of them). The game also draws plain
flat tiles of these layers that have the content of a glyph of the HD text font, which the text pass must not claim.

## Open

- The party's sprite builder and the battle sprite compositor (which table says which pieces make a pose, in banks
  `$1C`-`$1F`): only the pieces and their upload were traced.
- The palettes: which bank-`$09` rows each scene loads, and the blend tables of `05:C116`.
- The Mode 7 data formats of the flights (the 128 x 128 map and the 8-bit chars): the unpack routines are known, the
  per-scene sources are not.
- Where the faces of the corner HUD are chosen (`menus.md` "Corner faces").
- The last fight stages (`$0C`) and the minigame (`$0A`): the PPU set-up was not captured.
- The title's tile sets by scene step (`00:C7DA` and the bit-tree stream).

## How the HD art finds the field's map (`ssd-hd::mapart`, `xtask hd-maps`)

The field's BG2 can be drawn again from a picture of the whole map because the map is a pure function of data in the ROM
(see "The field map"): the layer's pixel at world `(x, y)` is a tile map word from `06:F089`, a tile of the map's 4-bit set
and a colour of its palette row. The art is keyed by what makes the layer, the size in blocks and the two data pointers (the
block grid and the metatile table, `FieldMap::layout`), not by a VRAM hash: VRAM holds about 35 columns of the layer, and
the same layer is shown in other palettes (`xtask hd-maps dump` found 21 distinct layers in the 48 maps: 4 and 5, 14 and 15,
16, 24 and 25, 22 and 23, 18 and 19, 1, 6 and 7, and 32 to 47 are the same pictures; 8 and 28 to 31 are the same picture with
different data pointers, so they have an entry each). A pixel's world position is the one the wide renderer reads the tile
from (`map_sample`: BG2's scroll taken next to the camera, the mosaic's block start during a fade-in), the same for the
original's window and for the margins of the 16:9 view. The window of a map smaller than the screen, or with the camera not
clamped, shows filler tiles beyond the map's edge; there is no art for them and the coverage tour does not count them.

What moves in a map, over 600 frames of each: no tile of the set changes in any map, and only colours 65, 75 and 76 of the
BG palettes (the shimmer of the water, a cycle of two or three values) change in some; the art adapts to the colour the
game shows at each pixel (`bgart::adapt`), so the water cycles through the art too.

## How to see it again

    cargo run --release -p xtask -- disasm 00C559                 # the decoder (and 00C5ED, 00C68C, 00C707, 06F089, 03:9158, 00:85F5)
    cargo run --release -p xtask -- shots wander --frames 3300 --peek game_mode,map_id    # a field screen
    cargo run --release -p xtask -- shots .work/replays/battle.ssdreplay --every 300 --upto 7200 --sheet 6   # title, flight, field, battle

The uploads, decoder calls and register writes of a replay were logged with a local tool (a breakpoint at each decoder
entry for the source pointer, and the CPU write log reduced to the `$43xx` channel registers, `$420B` and the `$21xx`
state): it is not tracked; the plan turns it into `xtask gfx-log`. The HD prototype that renders a frame from VRAM and
swaps tiles for pack art is local too (`.work/gfx-proto/`, untracked).
