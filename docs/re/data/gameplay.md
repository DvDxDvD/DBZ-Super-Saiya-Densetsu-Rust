# Gameplay: the text speed, the message engine's waits and the battle's animation

What the Gameplay page of the Options screen (M7, Task 6) needed from the game: which end of the text-speed byte is
fast, when a message waits for the player's A, and where the battle spends its time. Read from banks `$01` and `$05` and
checked headless on the reference replays (`xtask shots ... --watch`, `xtask disasm`). Addresses are bank:address; the
variables are named as in `re/symbols.toml`.

## The text speed (`text_speed`, `$01D0`)

The Text entry of the original Menu keeps the cursor of its list of five choices in `$01D0` (0 to 4; the screen at `01:96AD`
copies the cursor in and out). The message engine (`01:CBE0`, run once a frame) writes the characters of a page with a
handler picked by that byte (the jump table at `01:CCE3`):

| byte | what a frame does |
|---|---|
| 0 | the page-writing step runs three times (most characters) |
| 1 | twice |
| 2 | once, every frame |
| 3 | once every second frame (a countdown of 2 from the table at `01:CD09`) |
| 4 | once every fourth frame (the same table gives 4) |

So **0 is the fastest and 4 the slowest**; the byte counts the other way from a "speed". Two more values are used by the engine
itself while a message runs (4 and 5: the engine saves the player's choice in `$01D1`, puts 4 or 5 in the byte for a special page,
and restores it at the end; 5 writes the whole page at once), and a press of A during a page sets the byte to 0 for the rest of
that message. The Options row shows the five choices slowest first (Slowest, Slow, Normal, Fast, Fastest), the row's index
being `4 - byte`; the default byte is 2 (Normal).

## When a message waits for A (`message_state`, `$071B`)

`$071B` is the state of the message engine: bit 7 a message is up, bits 0 to 3 the state, bit 4 "closes by itself", bits 5 and 6
the instant kinds. Seen on the story replays (`story-1`, frames 2580 to 2993):

| value | state |
|---|---|
| `$80` | the message starts |
| `$81` | writing characters |
| `$82` | a page is written and waits for A, then goes on writing (`01:CDB5`) |
| `$83` | the last page is written and waits for A, then closes (`01:CDDB`) |
| `$85`, `$93`, `$96`, `$90` | choice lines, a timed close, a timed wait (a countdown in `$0726`, one a frame) |

Only `$82` and `$83` wait for a press. The battle's own messages never do: they are timed (`$0726` counts down, 60 frames for the
result message), which is why a battle runs by itself. The press is read from `joy1_new_low` bit 7 (A) in the same frame, so the
host's A is a joypad bit like the player's. The auto-run holds A for three frames once the wait is up and lets go as soon as the
state leaves `$82`/`$83` (checked on `story-1`: from the wait at frame 2616 A comes after the 45 frames of the Short wait and the
message moves on the next frame).

## Where a battle spends its time

The battle is a state machine in the main loop (`05:8336` jumps through the table at `05:834E`, indexed by `battle_phase`,
`$AB`; the code runs once a frame after the machine has waited for scanline 180). One round of the reference battle is:

| `$AB` | what | frames |
|---|---|---|
| 1, 2 | the hand and the choice of cards (the player's A and d-pad are read here) | about 20 with a pad held |
| 3 | `05:894A`: the damage is computed, then a message ("Goku attacks") shows until its countdown ends | 126 |
| 4 | `05:89B8`: the attack animation; the state does not move on until the figures have finished | 295 (87 for a short one) |
| 5 | `05:8A25`: the result message ("19 damage!"), timed | 91 |
| 6, 9 | the turn changes hands | 1 |
| `$0A`, `$0B`, `$13` | the end of the battle (a defeated enemy, the rewards) | 293, 43, many |

So the time is in two kinds of waits: messages (timed through `$0726`, or by A), and the attack animation of state 4, which is
the biggest single wait (more than half of a round). The animation is driven by sprite scripts (the figures' records at
`$7E:3000`, started by `05:93A9`) and counted in frames by many small counters; there is no one wait value to shorten.

### Why the waits are not shortened inside the game

The two random generators (`rng_seed_a`, `rng_seed_b`, `s = s * 5 + $11`) are stepped by the main loop's idle spin, **hundreds
of times in every frame** (449 to 655 steps a frame in the reference battle, the count depends on how long the frame's work took),
and again by the animation code for its cosmetic effects. A wait cut short by a hook would therefore change the number of steps
before the next draw, and with it every later draw: the cards dealt, the hits and the dodges. That is the same as the player
pressing a button on another frame, but it is not "the same battle faster". So the animation speed does not touch the game: it
runs the **same frames**, more of them to one frame on the screen.

### The animation speed (host side)

During the attack animation (`in_battle` = `$FF`, `game_mode` = 4, `$AB` = 4, `$071B` = 0) the host runs 2, 3 or 4 machine frames
for one frame it shows, with the pad of the window frame for all of them, and plays the sound of the first only (the machinery of
the speed hotkey). The machine sees the original's frames in the original's order: the turn order, the numbers and the draws are
unchanged by construction. The player's buttons do nothing in state 4 (the joypad is read only in the card phase and in the
message engine, whose state is 0 here), checked by running the reference battle and 15 story replays with the pad cleared on
every frame of the animation: the machine ends every frame the same (the joypad bytes and their copy at `$0240` apart). The
other long waits (the messages, the end of the battle) are not touched; the messages are what the auto-run and the text speed are for.
